//! Configuration file discovery and loading.
//!
//! This module handles discovering and loading trop configuration files
//! from various locations with proper precedence.

use crate::config::schema::Config;
use crate::error::{Error, Result};
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
            Self::load_effective_source(project_file, 2)?;
        }

        match project_file.file_name().and_then(|name| name.to_str()) {
            Some("trop.yaml" | "trop.local.yaml") => {
                let parent = project_file.parent().unwrap_or_else(|| Path::new(""));
                for (name, precedence) in [("trop.yaml", 2), ("trop.local.yaml", 3)] {
                    let path = parent.join(name);
                    if path.exists() {
                        sources.push(Self::load_effective_source(&path, precedence)?);
                    }
                }
            }
            _ => sources.push(Self::load_effective_source(project_file, 2)?),
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

        Self::load_effective_source(&config_path, 1).map(Some)
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
        let mut configs = Vec::new();
        let mut current = PathResolver::new()
            .resolve_implicit(start_dir)?
            .into_path_buf();

        loop {
            let mut found_any = false;

            // Check for trop.yaml
            let trop_yaml = current.join("trop.yaml");
            if trop_yaml.exists() {
                configs.push(Self::load_effective_source(&trop_yaml, 2)?);
                found_any = true;
            }

            // Check for trop.local.yaml (higher precedence)
            let trop_local = current.join("trop.local.yaml");
            if trop_local.exists() {
                configs.push(Self::load_effective_source(&trop_local, 3)?);
                found_any = true;
            }

            // Stop if we found configs or can't go up anymore
            if found_any || !current.pop() {
                break;
            }
        }

        Ok(configs)
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
        let config = serde_yaml::from_str(&contents).map_err(&invalid_yaml)?;
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
