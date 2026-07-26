//! Per-invocation configuration resolution.
//!
//! The CLI translates command-line arguments into one override layer, resolves
//! the complete configuration once, and then shares the immutable snapshot with
//! every command executed during that invocation.

use crate::error::CliError;
use crate::utils::GlobalOptions;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use trop::config::{Config, ConfigBuilder, ConfigField, ConfigFileKind, EffectiveConfig};
use trop::{Database, DatabaseConfig};

/// Configuration-file discovery profile for a command.
#[derive(Debug, Clone)]
pub(crate) enum ConfigScope {
    /// The command intentionally operates without effective configuration.
    None,
    /// Resolve built-ins, environment, and CLI without reading YAML files.
    ///
    /// Initialization uses this scope so an existing configuration document
    /// can be preserved verbatim even when it is not currently parseable.
    DefaultsEnvironmentCli,
    /// Discover the user and project configuration hierarchy.
    Discover,
    /// Treat one explicitly nominated document as the project configuration.
    ExplicitProject(PathBuf),
}

/// One command-line configuration layer.
#[derive(Debug, Default)]
pub(crate) struct CommandLineConfig {
    pub(crate) config: Config,
    pub(crate) fields: BTreeSet<ConfigField>,
}

impl CommandLineConfig {
    pub(crate) fn record(&mut self, field: ConfigField) {
        self.fields.insert(field);
    }
}

/// Everything needed to resolve configuration for one command.
#[derive(Debug)]
pub(crate) struct ConfigRequest {
    pub(crate) scope: ConfigScope,
    pub(crate) command_line: CommandLineConfig,
    pub(crate) data_dir_override: Option<PathBuf>,
}

impl ConfigRequest {
    pub(crate) fn new(scope: ConfigScope, command_line: CommandLineConfig) -> Self {
        Self {
            scope,
            command_line,
            data_dir_override: None,
        }
    }

    pub(crate) fn with_data_dir_override(mut self, data_dir: Option<PathBuf>) -> Self {
        self.data_dir_override = data_dir;
        self
    }
}

/// Immutable runtime context shared by every command in one process.
#[doc(hidden)]
pub struct InvocationContext {
    global: GlobalOptions,
    data_dir: Option<PathBuf>,
    effective: Option<EffectiveConfig>,
}

impl InvocationContext {
    /// Resolve the requested configuration hierarchy exactly once.
    pub(crate) fn resolve(global: GlobalOptions, request: ConfigRequest) -> Result<Self, CliError> {
        let ConfigRequest {
            scope,
            command_line,
            data_dir_override,
        } = request;
        let data_dir = data_dir_override.or_else(|| global.data_dir.clone());
        let working_dir = std::env::current_dir().map_err(CliError::Io)?;
        let mut builder = ConfigBuilder::new()
            .with_working_dir(working_dir)
            .with_cli_config_fields(command_line.config, command_line.fields);
        if let Some(data_dir) = &data_dir {
            builder = builder.with_data_dir(data_dir);
        }

        let effective = match scope {
            ConfigScope::None => None,
            ConfigScope::DefaultsEnvironmentCli => Some(
                builder
                    .skip_files()
                    .build_effective()
                    .map_err(|error| CliError::Config(error.to_string()))?,
            ),
            ConfigScope::Discover => Some(
                builder
                    .build_effective()
                    .map_err(|error| CliError::Config(error.to_string()))?,
            ),
            ConfigScope::ExplicitProject(path) => Some(
                builder
                    .with_project_file(path)
                    .build_effective()
                    .map_err(|error| CliError::Config(error.to_string()))?,
            ),
        };

        Ok(Self {
            global,
            data_dir,
            effective,
        })
    }

    /// Raw global switches that are deliberately outside the YAML schema.
    pub(crate) const fn global(&self) -> &GlobalOptions {
        &self.global
    }

    /// The one resolved configuration snapshot for this invocation.
    pub(crate) fn effective(&self) -> Result<&EffectiveConfig, CliError> {
        self.effective.as_ref().ok_or_else(|| {
            CliError::Config("this command does not use effective configuration".to_string())
        })
    }

    /// The effective value model consumed by existing library operations.
    pub(crate) fn config(&self) -> Result<&Config, CliError> {
        Ok(self.effective()?.config())
    }

    /// Open the database using the effective autoinit and timeout settings.
    pub(crate) fn open_database(&self) -> Result<Database, CliError> {
        let db_path = self.database_path()?;
        let effective = self.effective()?;

        if !db_path.exists() && effective.disable_autoinit() {
            return Err(CliError::NoDataDirectory);
        }

        let db_config = DatabaseConfig::new(db_path)
            .with_busy_timeout(Duration::from_secs(effective.maximum_lock_wait_seconds()));
        Database::open(db_config).map_err(CliError::from)
    }

    /// Open an already-existing database using the effective lock timeout.
    pub(crate) fn open_existing_database(&self) -> Result<Database, CliError> {
        let db_path = self.database_path()?;
        if !db_path.exists() {
            return Err(CliError::InvalidArguments(
                "Database file not found".to_string(),
            ));
        }

        let db_config = DatabaseConfig::new(db_path).with_busy_timeout(Duration::from_secs(
            self.effective()?.maximum_lock_wait_seconds(),
        ));
        Database::open(db_config).map_err(CliError::from)
    }

    /// Choose the current legacy YAML write target without flattening layers.
    ///
    /// Atomic and comment-preserving mutation is handled by the dedicated YAML
    /// writer remediation. This method only centralizes source selection.
    pub(crate) fn config_file_for_write(&self, force_global: bool) -> Result<PathBuf, CliError> {
        if force_global {
            return Ok(self.data_dir()?.join("config.yaml"));
        }

        let effective = self.effective()?;
        for kind in [ConfigFileKind::Local, ConfigFileKind::Project] {
            if let Some(path) = effective.loaded_file(kind) {
                return Ok(path.to_path_buf());
            }
        }

        Ok(self.data_dir()?.join("config.yaml"))
    }

    /// Resolve the selected data directory for special commands and writers.
    pub(crate) fn data_dir(&self) -> Result<&Path, CliError> {
        self.data_dir.as_deref().ok_or_else(|| {
            CliError::Config("Could not determine the trop data directory".to_string())
        })
    }

    fn database_path(&self) -> Result<PathBuf, CliError> {
        Ok(self.data_dir()?.join("trop.db"))
    }
}
