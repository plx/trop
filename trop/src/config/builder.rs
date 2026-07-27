//! Configuration builder pattern.
//!
//! This module provides a builder for constructing configurations programmatically,
//! integrating file loading, merging, environment variables, and validation.

use crate::config::environment::EnvironmentConfig;
use crate::config::loader::{ConfigLoader, ReservationOverlay};
use crate::config::schema::{CleanupConfig, Config, OccupancyConfig, OutputFormat, PortConfig};
use crate::config::validator::ConfigValidator;
use crate::config::{ConfigField, ConfigFileKind, ConfigValueSource, EffectiveConfig};
use crate::error::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Builder for loading and constructing configuration.
///
/// # Examples
///
/// ```no_run
/// use trop::config::ConfigBuilder;
/// use std::path::Path;
///
/// let config = ConfigBuilder::new()
///     .with_working_dir(Path::new("."))
///     .build()
///     .unwrap();
/// ```
pub struct ConfigBuilder {
    working_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    skip_env: bool,
    skip_files: bool,
    additional_config: Option<Config>,
    cli_config: Option<(Config, BTreeSet<ConfigField>)>,
    project_file: Option<PathBuf>,
}

impl ConfigBuilder {
    /// Create a new configuration builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            working_dir: None,
            data_dir: None,
            skip_env: false,
            skip_files: false,
            additional_config: None,
            cli_config: None,
            project_file: None,
        }
    }

    /// Set the working directory for config discovery.
    ///
    /// Configuration files will be discovered starting from this directory
    /// and walking up the directory tree.
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Set the data directory for user config loading.
    ///
    /// This overrides the default data directory (`~/.trop` or `$TROP_DATA_DIR`)
    /// when loading the user configuration file.
    #[must_use]
    pub fn with_data_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.data_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Skip loading configuration files.
    ///
    /// Useful for testing or when you want to build configuration programmatically.
    #[must_use]
    pub fn skip_files(mut self) -> Self {
        self.skip_files = true;
        self
    }

    /// Skip environment variable overrides.
    ///
    /// Useful for testing or when you want deterministic configuration.
    #[must_use]
    pub fn skip_env(mut self) -> Self {
        self.skip_env = true;
        self
    }

    /// Add additional configuration to merge (highest precedence).
    ///
    /// This configuration will be merged last, giving it the highest precedence
    /// after environment variables.
    #[must_use]
    pub fn with_config(mut self, config: Config) -> Self {
        self.additional_config = Some(config);
        self
    }

    /// Add command-line configuration at the highest documented precedence.
    ///
    /// Present fields are inferred from `config`. Call
    /// [`Self::with_cli_config_fields`] when a command needs to identify exact
    /// nested leaf fields, such as overriding `ports.max` without replacing
    /// `ports.min`.
    #[must_use]
    pub fn with_cli_config(mut self, config: Config) -> Self {
        let fields = ConfigField::present_in(&config);
        self.cli_config = Some((config, fields));
        self
    }

    /// Add precise command-line configuration fields.
    ///
    /// Only fields listed in `explicit_fields` participate in the override.
    /// This preserves lower-precedence sibling values for partial nested CLI
    /// options.
    #[must_use]
    pub fn with_cli_config_fields<I>(mut self, config: Config, explicit_fields: I) -> Self
    where
        I: IntoIterator<Item = ConfigField>,
    {
        self.cli_config = Some((config, explicit_fields.into_iter().collect()));
        self
    }

    /// Load an explicitly nominated project configuration file.
    ///
    /// User configuration is still loaded normally, but automatic project-file
    /// discovery is replaced by this exact file.
    #[must_use]
    pub fn with_project_file(mut self, path: impl AsRef<Path>) -> Self {
        self.project_file = Some(path.as_ref().to_path_buf());
        self
    }

    /// Build the final configuration.
    ///
    /// Performs the following steps:
    /// 1. Starts with default configuration
    /// 2. Loads and merges file-based configurations (if not skipped)
    /// 3. Applies environment variable overrides (if not skipped)
    /// 4. Applies additional configuration (if provided)
    /// 5. Applies command-line configuration (if provided)
    /// 6. Validates the final configuration
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Configuration files cannot be read or parsed
    /// - Environment variables contain invalid values
    /// - The final configuration fails validation
    pub fn build(self) -> Result<Config> {
        self.build_effective().map(EffectiveConfig::into_config)
    }

    /// Build a merged configuration together with field provenance.
    ///
    /// This follows the same merge and validation behavior as [`Self::build`]
    /// while retaining each field's winning and contributing sources.
    ///
    /// # Errors
    ///
    /// Returns an error if a source cannot be read or parsed, a source contains
    /// fields invalid for its kind, an environment value is invalid, or the
    /// merged result fails validation.
    pub fn build_effective(self) -> Result<EffectiveConfig> {
        let mut sources = Vec::new();

        if !self.skip_files {
            sources = if let Some(project_file) = self.project_file.as_deref() {
                ConfigLoader::load_with_project_file(project_file, self.data_dir.as_deref())?
            } else {
                let working_dir = self
                    .working_dir
                    .as_deref()
                    .unwrap_or_else(|| Path::new("."));
                ConfigLoader::load_all_effective(working_dir, self.data_dir.as_deref())?
            };
        }

        let is_tropfile = self.additional_config.is_some()
            || self.cli_config.is_some()
            || self.project_file.is_some()
            || sources.iter().any(|source| source.precedence >= 2);

        let mut effective = EffectiveConfig::from_defaults(Self::default_config());

        for source in sources {
            let kind = match source.precedence {
                1 => ConfigFileKind::User,
                3 => ConfigFileKind::Local,
                _ => ConfigFileKind::Project,
            };
            let value_source = ConfigValueSource::File {
                kind,
                path: source.path.clone(),
            };
            let reservation_overlay = if kind == ConfigFileKind::User
                && source.reservations == ReservationOverlay::Clear
            {
                // Existing generated global files can contain null for
                // absent optional fields. User config cannot own a group,
                // so this is inert rather than a project-level clear.
                ReservationOverlay::Inherit
            } else {
                source.reservations
            };

            ConfigValidator::validate_source_document(
                &source.config,
                &value_source,
                reservation_overlay != ReservationOverlay::Inherit,
            )?;
            effective.record_file(kind, source.path);
            effective.merge_config(&source.config, &value_source);
            if reservation_overlay == ReservationOverlay::Clear {
                effective.clear_reservations(&value_source);
            }
        }

        if !self.skip_env {
            let mut changes = Vec::new();
            EnvironmentConfig::apply_overrides_with(effective.config_mut(), |field, variable| {
                changes.push((field, variable));
            })?;
            for (field, variable) in changes {
                effective.record(field, ConfigValueSource::Environment { variable });
            }
        }

        if let Some(additional) = self.additional_config {
            effective.merge_config(&additional, &ConfigValueSource::Programmatic);
        }

        if let Some((cli_config, fields)) = self.cli_config {
            effective.apply_precise(&cli_config, fields, &ConfigValueSource::CommandLine);
        }

        effective.validate(is_tropfile)?;
        Ok(effective)
    }

    /// Create default configuration.
    ///
    /// Returns a configuration with all defaults matching the specification:
    /// - Port range: 5000-7000
    /// - Expire after: 30 days
    /// - Lock timeout: 5 seconds
    /// - All auto behaviors enabled
    /// - All permission flags disabled
    /// - All occupancy checks enabled
    /// - Output format: table
    fn default_config() -> Config {
        Config {
            project: None,
            ports: Some(PortConfig {
                min: 5000,
                max: Some(7000),
                max_offset: None,
            }),
            excluded_ports: None,
            cleanup: Some(CleanupConfig {
                expire_after_days: Some(30),
            }),
            occupancy_check: Some(OccupancyConfig {
                skip: Some(false),
                skip_ip4: Some(false),
                skip_ip6: Some(false),
                skip_tcp: Some(false),
                skip_udp: Some(false),
                check_all_interfaces: Some(false),
            }),
            reservations: None,
            disable_autoinit: Some(false),
            disable_autoprune: Some(false),
            disable_autoexpire: Some(false),
            allow_unrelated_path: Some(false),
            allow_change_project: Some(false),
            allow_change_task: Some(false),
            allow_change: Some(false),
            maximum_lock_wait_seconds: Some(5),
            output_format: Some(OutputFormat::Table),
        }
    }
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::PortExclusion;
    use crate::config::ConfigMerger;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_builder_default() {
        let config = ConfigBuilder::new().skip_files().build().unwrap();

        // Check all defaults
        assert_eq!(config.ports.as_ref().unwrap().min, 5000);
        assert_eq!(config.ports.as_ref().unwrap().max, Some(7000));
        assert_eq!(config.cleanup.as_ref().unwrap().expire_after_days, Some(30));
        assert_eq!(config.maximum_lock_wait_seconds, Some(5));
        assert_eq!(config.disable_autoinit, Some(false));
        assert_eq!(config.output_format, Some(OutputFormat::Table));
    }

    #[test]
    fn test_builder_skip_files() {
        let config = ConfigBuilder::new().skip_files().build().unwrap();
        assert!(config.project.is_none());
    }

    #[test]
    fn test_builder_with_working_dir() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("trop.yaml"), "project: test-project\n").unwrap();

        let config = ConfigBuilder::new()
            .with_working_dir(temp_dir.path())
            .skip_env()
            .build()
            .unwrap();

        assert_eq!(config.project, Some("test-project".to_string()));
    }

    #[test]
    fn test_builder_with_additional_config() {
        let additional = Config {
            project: Some("override".to_string()),
            ..Default::default()
        };

        let config = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(additional)
            .build()
            .unwrap();

        assert_eq!(config.project, Some("override".to_string()));
    }

    #[test]
    fn test_builder_validates() {
        let invalid = Config {
            project: Some(String::new()), // Empty project name
            ..Default::default()
        };

        let result = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(invalid)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_precedence_order() {
        let temp_dir = TempDir::new().unwrap();

        // File says project=file
        fs::write(temp_dir.path().join("trop.yaml"), "project: file\n").unwrap();

        // Additional config says project=override
        let additional = Config {
            project: Some("override".to_string()),
            ..Default::default()
        };

        let config = ConfigBuilder::new()
            .with_working_dir(temp_dir.path())
            .skip_env()
            .with_config(additional)
            .build()
            .unwrap();

        // Additional config should win
        assert_eq!(config.project, Some("override".to_string()));
    }

    #[test]
    fn test_builder_merges_excluded_ports() {
        // Test that ConfigMerger properly accumulates excluded_ports
        // by using two programmatic configs to avoid user config interference

        // First config excludes 5001
        let config1 = Config {
            excluded_ports: Some(vec![PortExclusion::Single(5001)]),
            ..Default::default()
        };

        // Second config excludes 5002
        let config2 = Config {
            excluded_ports: Some(vec![PortExclusion::Single(5002)]),
            ..Default::default()
        };

        // Merge them programmatically
        let mut merged = config1;
        ConfigMerger::merge_into(&mut merged, &config2);

        // Both should be present
        assert_eq!(merged.excluded_ports.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_default_config_matches_spec() {
        let defaults = ConfigBuilder::default_config();

        // Port range
        let ports = defaults.ports.unwrap();
        assert_eq!(ports.min, 5000);
        assert_eq!(ports.max, Some(7000));

        // Cleanup
        let cleanup = defaults.cleanup.unwrap();
        assert_eq!(cleanup.expire_after_days, Some(30));

        // Lock timeout
        assert_eq!(defaults.maximum_lock_wait_seconds, Some(5));

        // Auto behaviors
        assert_eq!(defaults.disable_autoinit, Some(false));
        assert_eq!(defaults.disable_autoprune, Some(false));
        assert_eq!(defaults.disable_autoexpire, Some(false));

        // Permissions
        assert_eq!(defaults.allow_unrelated_path, Some(false));
        assert_eq!(defaults.allow_change_project, Some(false));
        assert_eq!(defaults.allow_change_task, Some(false));
        assert_eq!(defaults.allow_change, Some(false));

        // Occupancy checks
        let occ = defaults.occupancy_check.unwrap();
        assert_eq!(occ.skip, Some(false));
        assert_eq!(occ.skip_ip4, Some(false));
        assert_eq!(occ.skip_ip6, Some(false));
        assert_eq!(occ.skip_tcp, Some(false));
        assert_eq!(occ.skip_udp, Some(false));
        assert_eq!(occ.check_all_interfaces, Some(false));

        // Output format
        assert_eq!(defaults.output_format, Some(OutputFormat::Table));
    }

    #[test]
    fn test_builder_with_data_dir() {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().join("custom_data");
        std::fs::create_dir_all(&data_dir).unwrap();

        // Create a config file in the custom data directory
        fs::write(data_dir.join("config.yaml"), "excluded_ports:\n  - 9999\n").unwrap();

        // Build config with custom data directory
        let config = ConfigBuilder::new()
            .with_data_dir(&data_dir)
            .skip_env()
            .build()
            .unwrap();

        // Verify the exclusion from custom data dir config was loaded
        assert!(config.excluded_ports.is_some());
        let exclusions = config.excluded_ports.unwrap();
        assert_eq!(exclusions.len(), 1);
        assert_eq!(exclusions[0], PortExclusion::Single(9999));
    }

    #[test]
    fn test_builder_accepts_max_offset_over_defaults() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("trop.yaml"),
            "ports:\n  min: 8000\n  max_offset: 25\n",
        )
        .unwrap();

        let config = ConfigBuilder::new()
            .with_working_dir(temp_dir.path())
            .skip_env()
            .build()
            .unwrap();

        let ports = config.ports.unwrap();
        assert_eq!(ports.min, 8000);
        assert_eq!(ports.max, None);
        assert_eq!(ports.max_offset, Some(25));
    }
}
