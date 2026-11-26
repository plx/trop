//! Configuration file discovery and loading.
//!
//! This module handles discovering and loading trop configuration files
//! from various locations with proper precedence.

use crate::config::schema::Config;
use crate::error::{Error, Result};
use crate::verified::config::{
    first_config_level, level_file_precedence, level_load_order, GroupOverlay, LevelFile,
    PROJECT_PRECEDENCE, USER_PRECEDENCE,
};
use crate::PathResolver;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Configuration source with its precedence level.
///
/// Lower precedence values are overridden by higher ones.
///
/// # Examples
///
/// ```
/// use trop::config::ConfigSource;
/// use std::path::PathBuf;
///
/// // User config has lowest precedence
/// let user_config = ConfigSource {
///     path: PathBuf::from("~/.trop/config.yaml"),
///     precedence: 1,
///     config: Default::default(),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ConfigSource {
    /// Path to the configuration file.
    pub path: PathBuf,
    /// Precedence level (higher values take priority).
    pub precedence: u8,
    /// Parsed configuration.
    pub config: Config,
}

/// Reservation-group operation declared by one YAML document.
///
/// `Config` intentionally retains its public `Option<ReservationGroup>` shape,
/// so file loading keeps the distinction between an omitted key and explicit
/// YAML `null` alongside the parsed value until effective merging is complete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ReservationOverlay {
    /// The document omits `reservations` and inherits a lower-precedence group.
    #[default]
    Inherit,
    /// The document supplies a complete replacement group.
    Replace,
    /// The document explicitly clears the effective group with YAML `null`.
    Clear,
}

impl ReservationOverlay {
    /// The verified kernel's code for this overlay (a variant-for-variant
    /// copy).
    pub(crate) const fn code(self) -> GroupOverlay {
        match self {
            Self::Inherit => GroupOverlay::Inherit,
            Self::Replace => GroupOverlay::Replace,
            Self::Clear => GroupOverlay::Clear,
        }
    }

    /// The overlay with the verified kernel's `code` (a variant-for-variant
    /// copy).
    pub(crate) const fn from_code(code: GroupOverlay) -> Self {
        match code {
            GroupOverlay::Inherit => Self::Inherit,
            GroupOverlay::Replace => Self::Replace,
            GroupOverlay::Clear => Self::Clear,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EffectiveConfigSource {
    pub(crate) path: PathBuf,
    pub(crate) precedence: u8,
    pub(crate) config: Config,
    pub(crate) reservations: ReservationOverlay,
}

impl EffectiveConfigSource {
    fn into_public(self) -> ConfigSource {
        ConfigSource {
            path: self.path,
            precedence: self.precedence,
            config: self.config,
        }
    }
}

#[derive(Debug)]
struct ConfigDocument {
    config: Config,
    reservations: ReservationOverlay,
}

#[derive(Debug, Default, Deserialize)]
struct ReservationPresence {
    #[serde(default, deserialize_with = "deserialize_reservation_overlay")]
    reservations: ReservationOverlay,
}

fn deserialize_reservation_overlay<'de, D>(
    deserializer: D,
) -> std::result::Result<ReservationOverlay, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<serde::de::IgnoredAny>::deserialize(deserializer).map(|value| {
        if value.is_some() {
            ReservationOverlay::Replace
        } else {
            ReservationOverlay::Clear
        }
    })
}

/// The file name of a project file of a configuration level.
const fn level_file_name(file: LevelFile) -> &'static str {
    match file {
        LevelFile::Project => "trop.yaml",
        LevelFile::Local => "trop.local.yaml",
    }
}

/// Which project files exist in `dir`: (`trop.yaml`, `trop.local.yaml`).
fn level_files_present(dir: &Path) -> (bool, bool) {
    (
        dir.join(level_file_name(LevelFile::Project)).exists(),
        dir.join(level_file_name(LevelFile::Local)).exists(),
    )
}

/// Loads configuration from various sources.
///
/// # Examples
///
/// ```no_run
/// use trop::config::ConfigLoader;
/// use std::path::Path;
///
/// let sources = ConfigLoader::load_all(Path::new("."), None).unwrap();
/// println!("Found {} configuration sources", sources.len());
/// ```
pub struct ConfigLoader;

impl ConfigLoader {
    /// Discover and load all configuration files.
    ///
    /// Searches for:
    /// 1. User config at `~/.trop/config.yaml` (precedence 1)
    /// 2. Project `trop.yaml` files walking up from `working_dir` (precedence 2)
    /// 3. Project `trop.local.yaml` files (precedence 3)
    ///
    /// The `data_dir` parameter allows overriding where the user config is loaded from.
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration file exists but cannot be read
    /// or parsed.
    pub fn load_all(working_dir: &Path, data_dir: Option<&Path>) -> Result<Vec<ConfigSource>> {
        Self::load_all_effective(working_dir, data_dir).map(|sources| {
            sources
                .into_iter()
                .map(EffectiveConfigSource::into_public)
                .collect()
        })
    }

    pub(crate) fn load_all_effective(
        working_dir: &Path,
        data_dir: Option<&Path>,
    ) -> Result<Vec<EffectiveConfigSource>> {
        let mut sources = Vec::new();

        // Load user config (~/.trop/config.yaml or custom data dir)
        if let Some(user_config) = Self::load_user_config_effective(data_dir)? {
            sources.push(user_config);
        }

        // Walk up directory tree looking for trop.yaml/trop.local.yaml
        let project_configs = Self::discover_project_configs_effective(working_dir)?;
        sources.extend(project_configs);

        // Sort by precedence (higher precedence last for easier processing)
        sources.sort_by_key(|s| s.precedence);

        Ok(sources)
    }

    pub(crate) fn load_with_project_file(
        project_file: &Path,
        data_dir: Option<&Path>,
    ) -> Result<Vec<EffectiveConfigSource>> {
        let mut sources = Vec::new();

        if let Some(user_config) = Self::load_user_config_effective(data_dir)? {
            sources.push(user_config);
        }

        if !project_file.exists() {
            // Preserve the explicit-file contract even when a canonically
            // named sibling happens to exist.
            Self::load_effective_source(project_file, PROJECT_PRECEDENCE)?;
        }

        match project_file.file_name().and_then(|name| name.to_str()) {
            Some("trop.yaml" | "trop.local.yaml") => {
                let parent = project_file.parent().unwrap_or_else(|| Path::new(""));
                let (has_project, has_local) = level_files_present(parent);
                sources.extend(Self::load_level(parent, has_project, has_local)?);
            }
            _ => sources.push(Self::load_effective_source(
                project_file,
                PROJECT_PRECEDENCE,
            )?),
        }
        sources.sort_by_key(|source| source.precedence);
        Ok(sources)
    }

    /// Load user configuration file.
    ///
    /// If `data_dir` is provided, loads from `{data_dir}/config.yaml`.
    /// Otherwise uses the default data directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    fn load_user_config_effective(
        data_dir: Option<&Path>,
    ) -> Result<Option<EffectiveConfigSource>> {
        let config_path = if let Some(dir) = data_dir {
            dir.join("config.yaml")
        } else {
            Self::user_config_path()?
        };

        if !config_path.exists() {
            return Ok(None);
        }

        Self::load_effective_source(&config_path, USER_PRECEDENCE).map(Some)
    }

    /// Discover project configurations by walking up directories.
    ///
    /// The discovery root is treated as inferred context: it is normalized to
    /// an absolute path and canonicalized before parent traversal begins.
    ///
    /// Stops at the first directory containing either trop.yaml or trop.local.yaml.
    ///
    /// # Errors
    ///
    /// Returns an error if the discovery root cannot be canonicalized or if any
    /// discovered file cannot be read or parsed.
    pub fn discover_project_configs(start_dir: &Path) -> Result<Vec<ConfigSource>> {
        Self::discover_project_configs_effective(start_dir).map(|sources| {
            sources
                .into_iter()
                .map(EffectiveConfigSource::into_public)
                .collect()
        })
    }

    fn discover_project_configs_effective(start_dir: &Path) -> Result<Vec<EffectiveConfigSource>> {
        let start = PathResolver::new()
            .resolve_implicit(start_dir)?
            .into_path_buf();
        // Level 0 is the start directory and level `k + 1` the parent of
        // level `k`, up to the root (the directories `PathBuf::pop` visits).
        let levels: Vec<&Path> = start.ancestors().collect();
        let has_files = |level: usize| level_files_present(levels[level]);

        // The verified kernel picks the nearest level holding either file.
        match first_config_level(levels.len(), &has_files) {
            Some((level, has_project, has_local)) => {
                Self::load_level(levels[level], has_project, has_local)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Loads the project files present in `dir`, in the verified kernel's
    /// load order: `trop.yaml` (project precedence) before `trop.local.yaml`
    /// (local precedence), each only when reported present.
    fn load_level(
        dir: &Path,
        has_project: bool,
        has_local: bool,
    ) -> Result<Vec<EffectiveConfigSource>> {
        level_load_order(has_project, has_local)
            .into_iter()
            .map(|file| {
                Self::load_effective_source(
                    &dir.join(level_file_name(file)),
                    level_file_precedence(file),
                )
            })
            .collect()
    }

    /// Load and parse a YAML configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the YAML is invalid.
    pub fn load_file(path: &Path) -> Result<Config> {
        Self::load_document(path).map(|document| document.config)
    }

    fn load_effective_source(path: &Path, precedence: u8) -> Result<EffectiveConfigSource> {
        let document = Self::load_document(path)?;
        Ok(EffectiveConfigSource {
            path: path.to_path_buf(),
            precedence,
            config: document.config,
            reservations: document.reservations,
        })
    }

    fn load_document(path: &Path) -> Result<ConfigDocument> {
        let contents = fs::read_to_string(path).map_err(|e| Error::InvalidPath {
            path: path.to_path_buf(),
            reason: format!("Failed to read configuration file: {e}"),
        })?;

        let invalid_yaml = |error: serde_yaml::Error| Error::Validation {
            field: format!("{}", path.display()),
            message: format!("Invalid YAML: {error}"),
        };
        let config = serde_yaml::from_str(&contents).map_err(invalid_yaml)?;
        let presence: ReservationPresence =
            serde_yaml::from_str(&contents).map_err(invalid_yaml)?;

        Ok(ConfigDocument {
            config,
            reservations: presence.reservations,
        })
    }

    /// Get user config directory path.
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    fn user_config_path() -> Result<PathBuf> {
        let data_dir = crate::database::default_data_dir()?;
        Ok(data_dir.join("config.yaml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_load_nonexistent_file() {
        let result = ConfigLoader::load_file(Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("bad.yaml");
        fs::write(&config_path, "invalid: yaml: syntax:").unwrap();

        let result = ConfigLoader::load_file(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_valid_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yaml");
        fs::write(&config_path, "project: test-project\n").unwrap();

        let config = ConfigLoader::load_file(&config_path).unwrap();
        assert_eq!(config.project, Some("test-project".to_string()));
    }

    #[test]
    fn test_discover_no_configs() {
        let temp_dir = TempDir::new().unwrap();
        let configs = ConfigLoader::discover_project_configs(temp_dir.path()).unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn test_discover_trop_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let trop_yaml = temp_dir.path().join("trop.yaml");
        fs::write(&trop_yaml, "project: test\n").unwrap();

        let configs = ConfigLoader::discover_project_configs(temp_dir.path()).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].precedence, 2);
    }

    #[test]
    fn test_discover_both_trop_files() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("trop.yaml"), "project: main\n").unwrap();
        fs::write(temp_dir.path().join("trop.local.yaml"), "project: local\n").unwrap();

        let configs = ConfigLoader::discover_project_configs(temp_dir.path()).unwrap();
        assert_eq!(configs.len(), 2);

        // Find trop.yaml and trop.local.yaml
        let trop_yaml = configs.iter().find(|c| c.precedence == 2).unwrap();
        let trop_local = configs.iter().find(|c| c.precedence == 3).unwrap();

        assert_eq!(trop_yaml.config.project, Some("main".to_string()));
        assert_eq!(trop_local.config.project, Some("local".to_string()));
    }

    #[test]
    fn test_discover_stops_at_first_config() {
        let temp_dir = TempDir::new().unwrap();
        let child = temp_dir.path().join("child");
        fs::create_dir(&child).unwrap();

        // Put config in parent
        fs::write(temp_dir.path().join("trop.yaml"), "project: parent\n").unwrap();

        // Discover from child - should find parent's config and stop
        let configs = ConfigLoader::discover_project_configs(&child).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].config.project, Some("parent".to_string()));
    }

    /// A level's files are loaded in the verified kernel's order: exactly the
    /// files present, `trop.yaml` (precedence 2) before `trop.local.yaml`
    /// (precedence 3), both by discovery and for a nominated project file.
    #[test]
    fn test_level_files_load_in_kernel_order() {
        for (project, local) in [(false, false), (true, false), (false, true), (true, true)] {
            let temp_dir = TempDir::new().unwrap();
            let dir = temp_dir.path();
            if project {
                fs::write(dir.join("trop.yaml"), "project: main\n").unwrap();
            }
            if local {
                fs::write(dir.join("trop.local.yaml"), "project: local\n").unwrap();
            }
            let mut expected = Vec::new();
            if project {
                expected.push((dir.join("trop.yaml"), 2, Some("main".to_string())));
            }
            if local {
                expected.push((dir.join("trop.local.yaml"), 3, Some("local".to_string())));
            }
            let summary = |sources: Vec<EffectiveConfigSource>| -> Vec<_> {
                sources
                    .into_iter()
                    .map(|source| (source.path, source.precedence, source.config.project))
                    .collect()
            };

            let discovered = ConfigLoader::discover_project_configs_effective(dir).unwrap();
            let canonical = dir.canonicalize().unwrap();
            let rebase = |entries: Vec<(PathBuf, u8, Option<String>)>| -> Vec<_> {
                entries
                    .into_iter()
                    .map(|(path, precedence, project)| {
                        (
                            canonical.join(path.file_name().unwrap()),
                            precedence,
                            project,
                        )
                    })
                    .collect()
            };
            assert_eq!(
                summary(discovered),
                rebase(expected.clone()),
                "{project} {local}"
            );

            for nominated in ["trop.yaml", "trop.local.yaml"] {
                let file = dir.join(nominated);
                if !file.exists() {
                    continue;
                }
                let user_data = dir.join("no-user-config");
                let loaded = ConfigLoader::load_with_project_file(&file, Some(&user_data)).unwrap();
                assert_eq!(summary(loaded), expected, "{nominated} {project} {local}");
            }
        }
    }

    #[test]
    fn test_load_all_sorts_by_precedence() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("trop.yaml"), "project: main\n").unwrap();
        fs::write(temp_dir.path().join("trop.local.yaml"), "project: local\n").unwrap();

        let sources = ConfigLoader::load_all(temp_dir.path(), None).unwrap();

        // Should be sorted by precedence (lowest to highest)
        for i in 1..sources.len() {
            assert!(sources[i - 1].precedence <= sources[i].precedence);
        }
    }
}
