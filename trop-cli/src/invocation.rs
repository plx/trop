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
use trop::cli_support::metadata_intent;
use trop::config::{Config, ConfigBuilder, ConfigField, ConfigFileKind, EffectiveConfig};
use trop::{Database, DatabaseConfig, MetadataIntent, PathResolver};

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
    pub(crate) reservation_metadata: Option<ReservationMetadataRequest>,
}

impl CommandLineConfig {
    pub(crate) fn record(&mut self, field: ConfigField) {
        self.fields.insert(field);
    }
}

/// Reservation-only metadata inputs that are not part of the YAML schema.
#[derive(Debug, Default)]
pub(crate) struct ReservationMetadataRequest {
    pub(crate) clear_project: bool,
    pub(crate) task: Option<String>,
    pub(crate) clear_task: bool,
}

/// Metadata intent resolved once for the reserve command.
#[derive(Debug)]
pub(crate) struct ResolvedReservationMetadata {
    pub(crate) project: MetadataIntent,
    pub(crate) task: MetadataIntent,
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
    reservation_metadata: Option<ResolvedReservationMetadata>,
    path_resolver: PathResolver,
    working_dir: PathBuf,
}

impl InvocationContext {
    /// Resolve the requested configuration hierarchy exactly once.
    pub(crate) fn resolve(global: GlobalOptions, request: ConfigRequest) -> Result<Self, CliError> {
        let ConfigRequest {
            scope,
            command_line,
            data_dir_override,
        } = request;
        let CommandLineConfig {
            config: command_line_config,
            fields: command_line_fields,
            reservation_metadata,
        } = command_line;
        let data_dir = data_dir_override.or_else(|| global.data_dir.clone());
        // The user configuration lives in the data directory, so a command
        // that reads configuration files needs one: without it, report the
        // missing data directory (exit 3) rather than a configuration error.
        if data_dir.is_none()
            && matches!(
                scope,
                ConfigScope::Discover | ConfigScope::ExplicitProject(_)
            )
        {
            return Err(crate::utils::undetermined_data_dir_error());
        }
        let path_resolver = PathResolver::new();
        let process_working_dir = std::env::current_dir().map_err(CliError::Io)?;
        let working_dir = path_resolver
            .resolve_implicit(&process_working_dir)
            .map_err(CliError::from)?
            .into_path_buf();
        let mut builder = ConfigBuilder::new()
            .with_working_dir(&working_dir)
            .with_cli_config_fields(command_line_config, command_line_fields);
        if let Some(metadata) = reservation_metadata.as_ref() {
            if metadata.clear_project {
                builder = builder.with_cli_project_clear();
            }
            if metadata.clear_task {
                builder = builder.with_cli_task(None);
            } else if let Some(task) = &metadata.task {
                builder = builder.with_cli_task(Some(task.clone()));
            }
        }
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
        let reservation_metadata = reservation_metadata
            .map(|request| Self::resolve_reservation_metadata(request, effective.as_ref()))
            .transpose()?;

        Ok(Self {
            global,
            data_dir,
            effective,
            reservation_metadata,
            path_resolver,
            working_dir,
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

    /// Return the configured SQLite lock wait when this command uses effective
    /// configuration.
    pub(crate) fn lock_timeout(&self) -> Option<Duration> {
        self.effective
            .as_ref()
            .map(|effective| Duration::from_secs(effective.maximum_lock_wait_seconds()))
    }

    /// The effective value model consumed by existing library operations.
    pub(crate) fn config(&self) -> Result<&Config, CliError> {
        Ok(self.effective()?.config())
    }

    /// Return the centrally resolved metadata intent for `reserve`.
    pub(crate) fn reservation_metadata(&self) -> Result<&ResolvedReservationMetadata, CliError> {
        self.reservation_metadata.as_ref().ok_or_else(|| {
            CliError::Config("this command does not resolve reservation metadata".to_string())
        })
    }

    /// Resolve a reservation path according to how the caller supplied it.
    ///
    /// Explicit CLI/environment values are normalized without following
    /// symlinks. An absent value selects the already-canonicalized invocation
    /// working directory.
    pub(crate) fn resolve_path(&self, path: Option<&Path>) -> Result<PathBuf, CliError> {
        match path {
            Some(path) => self.resolve_explicit_path(path),
            None => Ok(self.working_dir.clone()),
        }
    }

    /// Normalize an explicitly supplied path without following symlinks.
    pub(crate) fn resolve_explicit_path(&self, path: &Path) -> Result<PathBuf, CliError> {
        self.path_resolver
            .resolve_explicit(path)
            .map(trop::path::ResolvedPath::into_path_buf)
            .map_err(CliError::from)
    }

    /// Canonicalize a path regardless of its provenance.
    pub(crate) fn resolve_canonical_path(&self, path: &Path) -> Result<PathBuf, CliError> {
        self.path_resolver
            .resolve_canonical(path)
            .map(trop::path::ResolvedPath::into_path_buf)
            .map_err(CliError::from)
    }

    /// The canonical working directory selected once for this invocation.
    pub(crate) fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    fn resolve_reservation_metadata(
        request: ReservationMetadataRequest,
        effective: Option<&EffectiveConfig>,
    ) -> Result<ResolvedReservationMetadata, CliError> {
        let effective = effective.ok_or_else(|| {
            CliError::Config("reserve requires effective configuration".to_string())
        })?;

        // `--clear-*` clears; otherwise an effective value is set; otherwise
        // the stored value is kept (decided by the verified kernel).
        let project = metadata_intent(request.clear_project, effective.project());
        let task = metadata_intent(request.clear_task, effective.task());

        Ok(ResolvedReservationMetadata { project, task })
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

    /// Validate an existing database through a read-only connection.
    pub(crate) fn validate_existing_database(&self) -> Result<(), CliError> {
        let db_path = self.database_path()?;
        let database_exists = db_path.try_exists().map_err(|error| {
            CliError::from(trop::Error::DatabaseCorruption {
                details: format!(
                    "cannot access the selected database {}: {error}; \
                     verify its path and permissions before retrying. \
                     trop did not initialize or modify the data directory",
                    db_path.display()
                ),
            })
        })?;
        if !database_exists {
            return Err(CliError::from(trop::Error::DatabaseCorruption {
                details: format!(
                    "the selected data directory does not contain {}; \
                     restore a known-good database or recreate disposable reservations. \
                     trop did not initialize or modify the data directory",
                    db_path.display()
                ),
            }));
        }

        let db_config = DatabaseConfig::new(db_path).with_busy_timeout(Duration::from_secs(
            self.effective()?.maximum_lock_wait_seconds(),
        ));
        Database::validate(&db_config).map_err(CliError::from)
    }

    /// Choose the YAML file an exclusion edit writes to, without flattening
    /// layers.
    ///
    /// The verified `select_write_target` kernel decides: the global
    /// `config.yaml` for the global scope; otherwise the loaded
    /// `trop.local.yaml`, then the loaded `trop.yaml`; with neither, the
    /// global file only for `WriteScope::ProjectOrGlobal`. `trop exclude`
    /// uses `WriteScope::Project` (it fails without a project file unless
    /// `--global` is given); `trop scan --autoexclude` falls back to the
    /// global file.
    pub(crate) fn config_file_for_write(
        &self,
        scope: trop::port::exclusions::WriteScope,
    ) -> Result<PathBuf, CliError> {
        use trop::port::exclusions::{select_write_target, WriteScope, WriteTarget};

        let (local, project) = if scope == WriteScope::Global {
            (None, None)
        } else {
            let effective = self.effective()?;
            (
                effective.loaded_file(ConfigFileKind::Local),
                effective.loaded_file(ConfigFileKind::Project),
            )
        };

        match (
            select_write_target(scope, local.is_some(), project.is_some()),
            local,
            project,
        ) {
            (Some(WriteTarget::Local), Some(path), _)
            | (Some(WriteTarget::Project), _, Some(path)) => Ok(path.to_path_buf()),
            (Some(WriteTarget::Global), ..) => Ok(self.data_dir()?.join("config.yaml")),
            // `None`: the project scope found no project file. (The kernel
            // never selects a project-level file that was not loaded.)
            _ => Err(CliError::Config(format!(
                "no project configuration (trop.yaml or trop.local.yaml) found in {} or its \
                 parent directories; create a trop.yaml or use --global",
                self.working_dir().display()
            ))),
        }
    }

    /// Resolve the selected data directory for special commands and writers.
    ///
    /// Fails with exit code 3 when no data directory could be determined.
    pub(crate) fn data_dir(&self) -> Result<&Path, CliError> {
        self.data_dir
            .as_deref()
            .ok_or_else(crate::utils::undetermined_data_dir_error)
    }

    /// The selected database file (`trop.db` in the data directory).
    pub(crate) fn database_path(&self) -> Result<PathBuf, CliError> {
        Ok(self.data_dir()?.join("trop.db"))
    }
}
