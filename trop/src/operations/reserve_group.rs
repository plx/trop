//! Reserve group operation planning and execution.
//!
//! This module implements group reservation planning, which reserves multiple
//! related ports based on a configuration file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{
    CleanupConfig, Config, ConfigField, ConfigLoader, ConfigValidator, EffectiveConfig, PortConfig,
};
use crate::database::{check_schema_compatibility, Database};
use crate::error::{Error, Result};
use crate::output::{OutputFormat, ShellType};
use crate::port::allocator::{allocator_from_config, port_span_from_config};
use crate::port::group::{
    GroupAllocationOutcome, GroupAllocationRequest, GroupCleanupCounts, GroupReconciliationPolicy,
    ServiceAllocationRequest,
};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::verified::group::{self as group_kernel, config_service_offset, FormatArg, GroupOutput};
use crate::verified::port_bounds::group_scan_span;
use crate::verified::reserve::{cleanup_policy, CleanupSwitches};
use crate::{PathResolver, Port};
use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, OpenFlags};

use super::cleanup::CleanupOperations;
use super::executor::{ExecutionResult, PlanExecutor};
use super::plan::{OperationPlan, PlanAction};

/// Options for a reserve group operation.
///
/// This struct contains all the parameters needed to plan a group reservation
/// operation from a configuration file.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveGroupOptions {
    /// Path to the configuration file containing the reservation group.
    pub config_path: PathBuf,

    /// Optional task identifier (sticky field).
    pub task: Option<String>,

    /// Authorize unrelated paths, both sticky fields, and atomic replacement
    /// of an incompatible group shape. Allocation integrity checks remain.
    pub force: bool,

    /// Allow operations on unrelated paths without changing other protections.
    pub allow_unrelated_path: bool,

    /// Allow changing only the project field.
    pub allow_project_change: bool,

    /// Allow changing only the task field.
    pub allow_task_change: bool,
}

impl ReserveGroupOptions {
    /// Creates a new `ReserveGroupOptions` with the given config path.
    ///
    /// All optional fields and flags are set to defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::ReserveGroupOptions;
    /// use std::path::PathBuf;
    ///
    /// let options = ReserveGroupOptions::new(PathBuf::from("trop.yaml"));
    /// assert!(!options.force);
    /// ```
    #[must_use]
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            task: None,
            force: false,
            allow_unrelated_path: false,
            allow_project_change: false,
            allow_task_change: false,
        }
    }

    /// Sets the task field.
    #[must_use]
    pub fn with_task(mut self, task: Option<String>) -> Self {
        self.task = task;
        self
    }

    /// Sets the force flag.
    #[must_use]
    pub const fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Sets the `allow_unrelated_path` flag.
    #[must_use]
    pub const fn with_allow_unrelated_path(mut self, allow: bool) -> Self {
        self.allow_unrelated_path = allow;
        self
    }

    /// Sets the `allow_project_change` flag.
    #[must_use]
    pub const fn with_allow_project_change(mut self, allow: bool) -> Self {
        self.allow_project_change = allow;
        self
    }

    /// Sets the `allow_task_change` flag.
    #[must_use]
    pub const fn with_allow_task_change(mut self, allow: bool) -> Self {
        self.allow_task_change = allow;
        self
    }
}

/// A reserve group plan generator.
///
/// This struct is responsible for analyzing a reserve group request and
/// generating a plan that describes what actions to take.
pub struct ReserveGroupPlan {
    options: ReserveGroupOptions,
    config: Config,
    config_path: PathBuf,
    reservation_path: PathBuf,
}

struct GroupConfigPaths {
    source: PathBuf,
    reservation_identity: PathBuf,
}

impl GroupConfigPaths {
    fn resolve(config_path: &Path) -> Result<Self> {
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        let source = resolver.resolve_explicit(config_path)?.into_path_buf();
        let metadata = fs::metadata(&source).map_err(|error| match error.kind() {
            ErrorKind::NotFound => Error::PathNotFound {
                path: source.clone(),
            },
            ErrorKind::PermissionDenied => Error::PermissionDenied {
                path: source.clone(),
            },
            _ => Error::Io(error),
        })?;

        if !metadata.is_file() {
            return Err(Error::InvalidPath {
                path: source,
                reason: "Configuration path is not a file".to_string(),
            });
        }

        let parent = source.parent().ok_or_else(|| Error::InvalidPath {
            path: source.clone(),
            reason: "Configuration file has no parent directory".to_string(),
        })?;
        let reservation_identity = resolver.resolve_implicit(parent)?.into_path_buf();

        Ok(Self {
            source,
            reservation_identity,
        })
    }
}

impl ReserveGroupPlan {
    /// Creates a new reserve group plan with the given options.
    ///
    /// This loads the configuration file and validates its contents.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The config source does not exist or is not a regular file
    /// - The config source's containing directory cannot be canonicalized
    /// - The config file cannot be read or parsed
    /// - The config file does not contain a reservation group
    /// - The reservation group is invalid
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{ReserveGroupPlan, ReserveGroupOptions};
    /// use std::path::PathBuf;
    ///
    /// let options = ReserveGroupOptions::new(PathBuf::from("trop.yaml"));
    /// let planner = ReserveGroupPlan::new(options).unwrap();
    /// ```
    pub fn new(options: ReserveGroupOptions) -> Result<Self> {
        let paths = GroupConfigPaths::resolve(&options.config_path)?;
        let config = ConfigLoader::load_file(&paths.source)?;
        Self::from_resolved_config(options, config, paths)
    }

    /// Create a group plan from an already resolved effective configuration.
    ///
    /// The plan retains one cloned snapshot so allocation and output formatting
    /// observe exactly the same validated values.
    ///
    /// # Errors
    ///
    /// Returns an error if the config source does not resolve to a regular
    /// file, its containing directory cannot be canonicalized, or the
    /// effective group configuration is invalid.
    pub fn from_effective(options: ReserveGroupOptions, config: &EffectiveConfig) -> Result<Self> {
        let group_source = config
            .provenance(ConfigField::Reservations)
            .and_then(|provenance| provenance.winner().file_path())
            .unwrap_or(&options.config_path)
            .to_path_buf();
        let mut options = options;
        options.config_path = group_source;

        if config.reservations().is_none() {
            return Err(Error::Validation {
                field: "reservations".to_string(),
                message: "Effective configuration does not contain a reservation group".to_string(),
            });
        }

        Self::from_config(options, config.config().clone())
    }

    /// Creates a plan from an already parsed configuration snapshot.
    pub(super) fn from_config(options: ReserveGroupOptions, config: Config) -> Result<Self> {
        let paths = GroupConfigPaths::resolve(&options.config_path)?;
        Self::from_resolved_config(options, config, paths)
    }

    fn from_resolved_config(
        options: ReserveGroupOptions,
        config: Config,
        paths: GroupConfigPaths,
    ) -> Result<Self> {
        ConfigValidator::validate(&config, true)?;

        Ok(Self {
            options,
            config,
            config_path: paths.source,
            reservation_path: paths.reservation_identity,
        })
    }

    /// Returns the resolved source-file path used by this planner.
    #[must_use]
    pub const fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Returns the exact validated configuration snapshot used by this planner.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Gets the occupancy check configuration from the overall config.
    fn occupancy_config(&self) -> OccupancyCheckConfig {
        if let Some(ref occ_config) = self.config.occupancy_check {
            OccupancyCheckConfig::from(occ_config)
        } else {
            OccupancyCheckConfig::default()
        }
    }

    /// Builds an operation plan for this reserve group request.
    ///
    /// This method performs all validation and determines what actions
    /// are needed. It does NOT modify the database.
    ///
    /// # Note on `_db` parameter
    ///
    /// The `_db` parameter is kept for API consistency with other plan types
    /// (`ReservePlan`, `ReleasePlan`). Group reconciliation and allocation
    /// happen during execution, inside the caller's transaction, so the
    /// database is not inspected while building this plan.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The config does not contain a reservation group
    /// - The reservation group is invalid
    /// - Group allocation validation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{ReserveGroupPlan, ReserveGroupOptions};
    /// use trop::{Database, DatabaseConfig};
    /// use std::path::PathBuf;
    ///
    /// let db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let options = ReserveGroupOptions::new(PathBuf::from("trop.yaml"));
    /// let planner = ReserveGroupPlan::new(options).unwrap();
    /// let plan = planner.build_plan(db.connection()).unwrap();
    /// ```
    pub fn build_plan(&self, _conn: &Connection) -> Result<OperationPlan> {
        // Extract the reservation group from config
        let reservation_group =
            self.config
                .reservations
                .as_ref()
                .ok_or_else(|| Error::Validation {
                    field: "reservations".to_string(),
                    message: "Configuration file does not contain a reservation group".to_string(),
                })?;

        // Validate that we have at least one service
        if reservation_group.services.is_empty() {
            return Err(Error::Validation {
                field: "reservations.services".to_string(),
                message: "Reservation group must contain at least one service".to_string(),
            });
        }

        // Apply the same invocation-path guard as a single reservation. Force
        // is the broad override; the narrow path flag authorizes only this
        // relationship check.
        if !self.options.force && !self.options.allow_unrelated_path {
            Database::validate_path_relationship(&self.reservation_path, false)?;
        }

        // Convert the reservation group to a GroupAllocationRequest
        let request = self.build_group_request(reservation_group)?;

        // Build the plan
        let mut plan = OperationPlan::new(format!(
            "Reserve group of {} services from {}",
            reservation_group.services.len(),
            self.config_path.display()
        ));

        let occupancy_config = self.occupancy_config();
        let full_config = self.config_with_group_base_as_scan_start(reservation_group)?;

        plan = plan.add_action(PlanAction::AllocateGroup {
            request,
            policy: GroupReconciliationPolicy {
                allow_project_change: self.options.allow_project_change,
                allow_task_change: self.options.allow_task_change,
                force: self.options.force,
            },
            full_config,
            occupancy_config,
        });

        Ok(plan)
    }

    /// Runs this group reservation as a dry run.
    ///
    /// The plan is built and executed exactly as a live run builds and
    /// executes it -- reconciliation, the fresh allocation, the automatic
    /// cleanup and single retry -- but inside a transaction on a private
    /// in-memory copy of the database (`open_dry_run_snapshot`), which is
    /// then discarded. The result therefore reports the ports a live run on
    /// the same snapshot would reserve or refresh, with its warnings, and the
    /// dry run fails with exactly the error such a live run would report.
    /// `database` is the database file, or `None` when none exists yet.
    ///
    /// # Errors
    ///
    /// Returns every error a live run would return, and an error if the
    /// database cannot be read.
    pub fn dry_run(
        &self,
        database: Option<&Path>,
        busy_timeout: Duration,
    ) -> Result<ExecutionResult> {
        let snapshot = open_dry_run_snapshot(database, busy_timeout)?;
        let transaction = snapshot.unchecked_transaction()?;
        let plan = self.build_plan(&transaction)?;
        let mut result = PlanExecutor::new(&transaction).execute(&plan)?;
        // Dropping the transaction rolls the copy back; the copy itself is
        // discarded with `snapshot`.
        drop(transaction);
        result.dry_run = true;
        Ok(result)
    }

    /// Builds a `GroupAllocationRequest` from the reservation group.
    ///
    /// Services are taken in tag order (the configuration stores them in a
    /// `HashMap`), so the request and any validation error it produces are
    /// deterministic. The request's project, task, and tags are validated by
    /// `GroupAllocationRequest::normalized` with the same identifier rules
    /// as a single reservation.
    fn build_group_request(
        &self,
        group: &crate::config::ReservationGroup,
    ) -> Result<GroupAllocationRequest> {
        let mut definitions = group.services.iter().collect::<Vec<_>>();
        definitions.sort_unstable_by_key(|(tag, _)| *tag);

        let mut services = Vec::with_capacity(definitions.len());
        for (tag, service_def) in definitions {
            let preferred = service_def.preferred.map(Port::try_from).transpose()?;

            services.push(ServiceAllocationRequest {
                tag: tag.clone(),
                offset: Some(config_service_offset(service_def.offset)),
                preferred,
            });
        }

        GroupAllocationRequest {
            base_path: self.reservation_path.clone(),
            project: self.config.project.clone(),
            task: self.options.task.clone(),
            services,
        }
        .normalized()
    }

    /// Returns configuration adjusted so `reservations.base` is the group scan start.
    ///
    /// The configured range is resolved exactly as the allocator resolves it
    /// (`port_range_from_config`), and the verified kernel
    /// `verified::port_bounds::group_scan_span` narrows it: the scan runs from
    /// the base to the configured maximum, and the base must lie inside the
    /// configured range. Without a base the configuration is unchanged.
    fn config_with_group_base_as_scan_start(
        &self,
        group: &crate::config::ReservationGroup,
    ) -> Result<Config> {
        if group.base.is_none() {
            return Ok(self.config.clone());
        }
        let ports = self
            .config
            .ports
            .as_ref()
            .ok_or_else(|| Error::Validation {
                field: "ports".to_string(),
                message: "Port configuration is required when reservations.base is set".to_string(),
            })?;
        let configured = port_span_from_config(ports)?;
        let scan = group_scan_span(group.base, configured).ok_or_else(|| Error::Validation {
            field: "reservations.base".to_string(),
            message: format!(
                "Base port {} must be within port range {}..{}",
                group.base.unwrap_or_default(),
                configured.min(),
                configured.max()
            ),
        })?;

        let mut config = self.config.clone();
        config.ports = Some(PortConfig {
            min: scan.min(),
            max: Some(scan.max()),
            max_offset: None,
        });
        Ok(config)
    }
}

/// Runs a planned group allocation: reconciliation and, for a fresh group,
/// the allocation with one automatic cleanup and retry when the fallback
/// pattern is exhausted -- the same step single reservations take.
///
/// The cleanup policy is the verified `verified::reserve::cleanup_policy` of
/// the configuration's `disable_autoprune`/`disable_autoexpire` switches, so
/// a retry is possible unless both are disabled. Expiry uses the configured
/// `cleanup` section, or 30 days without one, as for single reservations.
pub(crate) fn execute_group_allocation(
    conn: &Connection,
    request: &GroupAllocationRequest,
    policy: GroupReconciliationPolicy,
    full_config: &Config,
    occupancy_config: &OccupancyCheckConfig,
) -> Result<GroupAllocationOutcome> {
    let allocator = allocator_from_config(full_config)?;
    let cleanup = cleanup_policy(CleanupSwitches {
        disable_autoprune: full_config.disable_autoprune.unwrap_or(false),
        disable_autoexpire: full_config.disable_autoexpire.unwrap_or(false),
    });
    allocator.allocate_group_with_cleanup(
        conn,
        request,
        occupancy_config,
        policy,
        cleanup.retry_possible(),
        |conn| {
            let default_cleanup = CleanupConfig {
                expire_after_days: Some(30),
            };
            let cleanup_config = full_config.cleanup.as_ref().unwrap_or(&default_cleanup);
            let result = CleanupOperations::automatic_cleanup_in_connection(
                conn,
                cleanup_config,
                cleanup.prune,
                cleanup.expire,
                || {},
            )?;
            Ok(GroupCleanupCounts {
                pruned: result.pruned_count,
                expired: result.expired_count,
            })
        },
    )
}

/// Opens a private, in-memory copy of a database for a dry run.
///
/// - With `database` (an existing database file), the file is opened
///   read-only and copied -- every table with its rows, then its indexes,
///   triggers, and views -- inside one read transaction, so the copy is one
///   consistent snapshot.
/// - Without one, the copy starts empty, as the database a live run would
///   create.
///
/// The copy is then initialized or migrated in memory by the same schema
/// check `Database::open` runs, so it is the database a live run would plan
/// against, and an unsupported schema fails as a live run fails. The file is
/// never written, created, initialized, or migrated. Opening an existing
/// WAL-mode database read-only can leave its `-wal`/`-shm` files beside it;
/// the contents never change.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read, or if its schema
/// is not supported.
#[doc(hidden)]
pub fn open_dry_run_snapshot(
    database: Option<&Path>,
    busy_timeout: Duration,
) -> Result<Connection> {
    let snapshot = Connection::open_in_memory()?;
    if let Some(path) = database {
        let source = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        source.busy_timeout(busy_timeout)?;
        let read = source.unchecked_transaction()?;
        copy_database(&read, &snapshot)?;
        drop(read);
    }
    check_schema_compatibility(&snapshot)?;
    Ok(snapshot)
}

/// Copies every table (with its rows), index, trigger, and view of `source`
/// into the empty database `target`.
fn copy_database(source: &Connection, target: &Connection) -> Result<()> {
    let objects = {
        let mut statement = source.prepare(
            "SELECT type, name, sql FROM sqlite_master \
             WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    // Tables and their rows first, then the objects that refer to them.
    for (kind, name, sql) in &objects {
        if kind == "table" {
            target.execute_batch(sql)?;
            copy_rows(source, target, name)?;
        }
    }
    for (kind, _, sql) in &objects {
        if kind != "table" {
            target.execute_batch(sql)?;
        }
    }
    Ok(())
}

/// Copies every row of `table` from `source` into the same (empty) table of
/// `target`.
fn copy_rows(source: &Connection, target: &Connection, table: &str) -> Result<()> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let mut select = source.prepare(&format!("SELECT * FROM {quoted}"))?;
    let columns = select.column_count();
    let placeholders = vec!["?"; columns].join(", ");
    let mut insert = target.prepare(&format!("INSERT INTO {quoted} VALUES ({placeholders})"))?;
    let mut rows = select.query([])?;
    while let Some(row) = rows.next()? {
        let values = (0..columns)
            .map(|index| row.get::<_, Value>(index))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        insert.execute(params_from_iter(values))?;
    }
    Ok(())
}

/// The `--format` of `reserve-group` and `autoreserve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOutputFormat {
    /// Shell export statements.
    Export,
    /// JSON.
    Json,
    /// Dotenv (`.env`) lines.
    Dotenv,
    /// Human-readable text.
    Human,
}

/// Selects the output of `reserve-group` and `autoreserve`.
///
/// JSON, dotenv, and human output need no shell, so `shell` is ignored for
/// them. Export output uses `shell` when it is given -- an unknown name is an
/// error, never replaced by detection -- and otherwise the detected shell.
/// The verified kernel `verified::group::select_group_output` decides, and it
/// parses `shell` or detects the shell only in those cases.
///
/// # Errors
///
/// Returns an error for the export format when `shell` names no supported
/// shell, or when no shell is given and detection fails.
pub fn select_group_output(format: GroupOutputFormat, shell: Option<&str>) -> Result<OutputFormat> {
    let format = match format {
        GroupOutputFormat::Export => FormatArg::Export,
        GroupOutputFormat::Json => FormatArg::Json,
        GroupOutputFormat::Dotenv => FormatArg::Dotenv,
        GroupOutputFormat::Human => FormatArg::Human,
    };
    let parse = shell.map(|name| move || ShellType::from_string(name));
    Ok(
        match group_kernel::select_group_output(format, parse, ShellType::detect)? {
            GroupOutput::Export(shell) => OutputFormat::Export(shell),
            GroupOutput::Json => OutputFormat::Json,
            GroupOutput::Dotenv => OutputFormat::Dotenv,
            GroupOutput::Human => OutputFormat::Human,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ReservationGroup, ServiceDefinition};
    use crate::database::test_util::create_test_database;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_config_file(dir: &TempDir, content: &str) -> PathBuf {
        let config_path = dir.path().join("trop.yaml");
        fs::write(&config_path, content).unwrap();
        config_path
    }

    #[test]
    fn test_reserve_group_options_new() {
        let options = ReserveGroupOptions::new(PathBuf::from("trop.yaml"));
        assert!(!options.force);
        assert!(!options.allow_unrelated_path);
        assert!(options.task.is_none());
    }

    #[test]
    fn test_reserve_group_options_builder() {
        let options = ReserveGroupOptions::new(PathBuf::from("trop.yaml"))
            .with_task(Some("dev".to_string()))
            .with_force(true)
            .with_allow_unrelated_path(true);

        assert!(options.force);
        assert!(options.allow_unrelated_path);
        assert_eq!(options.task, Some("dev".to_string()));
    }

    #[test]
    fn test_reserve_group_plan_new_with_valid_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
";
        let config_path = create_test_config_file(&temp_dir, config_content);

        let options = ReserveGroupOptions::new(config_path);
        let plan = ReserveGroupPlan::new(options);

        assert!(plan.is_ok());
    }

    #[test]
    fn test_reserve_group_config_accessor_is_creation_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = create_test_config_file(
            &temp_dir,
            r"
project: snapshot-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
      env: SNAPSHOT_PORT
",
        );
        let planner = ReserveGroupPlan::new(
            ReserveGroupOptions::new(config_path.clone()).with_allow_unrelated_path(true),
        )
        .unwrap();

        fs::write(&config_path, "this: [is not valid yaml").unwrap();

        assert_eq!(
            planner.config().project.as_deref(),
            Some("snapshot-project")
        );
        assert_eq!(
            planner
                .config()
                .reservations
                .as_ref()
                .unwrap()
                .services
                .get("web")
                .unwrap()
                .env
                .as_deref(),
            Some("SNAPSHOT_PORT")
        );

        let db = create_test_database();
        assert!(planner.build_plan(db.connection()).is_ok());
    }

    #[test]
    fn test_reserve_group_plan_new_without_reservations() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test-project
ports:
  min: 5000
  max: 7000
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let db = create_test_database();

        let options = ReserveGroupOptions::new(config_path).with_allow_unrelated_path(true);
        let plan = ReserveGroupPlan::new(options).unwrap();
        let result = plan.build_plan(db.connection());

        assert!(result.is_err());
        match result {
            Err(Error::Validation { field, .. }) => {
                assert_eq!(field, "reservations");
            }
            _ => panic!("Expected validation error for missing reservations"),
        }
    }

    #[test]
    fn test_reserve_group_plan_build_plan_simple() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let db = create_test_database();

        let options = ReserveGroupOptions::new(config_path).with_allow_unrelated_path(true);
        let planner = ReserveGroupPlan::new(options).unwrap();
        let plan = planner.build_plan(db.connection()).unwrap();

        assert_eq!(plan.actions.len(), 1);
        match &plan.actions[0] {
            PlanAction::AllocateGroup { request, .. } => {
                assert_eq!(request.services.len(), 2);
                assert_eq!(request.project, Some("test-project".to_string()));
            }
            _ => panic!("Expected AllocateGroup action"),
        }
    }

    #[test]
    fn test_reserve_group_plan_empty_services() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services: {}
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let db = create_test_database();

        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();
        let result = planner.build_plan(db.connection());

        assert!(result.is_err());
        match result {
            Err(Error::Validation { field, .. }) => {
                assert_eq!(field, "reservations.services");
            }
            _ => panic!("Expected validation error for empty services"),
        }
    }

    #[test]
    fn test_build_group_request_with_offsets() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(1),
                preferred: None,
                env: None,
            },
        );

        let group = ReservationGroup {
            base: Some(5000),
            services,
        };

        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test
ports:
  min: 5000
  max: 7000
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();

        let request = planner.build_group_request(&group).unwrap();

        assert_eq!(request.services.len(), 2);
        assert_eq!(request.project, Some("test".to_string()));
    }

    #[test]
    fn test_build_group_request_with_preferred() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: None,
                preferred: Some(8080),
                env: None,
            },
        );

        let group = ReservationGroup {
            base: None,
            services,
        };

        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test
ports:
  min: 5000
  max: 7000
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();

        let request = planner.build_group_request(&group).unwrap();

        assert_eq!(request.services.len(), 1);
        assert_eq!(
            request.services[0].preferred,
            Some(Port::try_from(8080).unwrap())
        );
        assert_eq!(
            request.services[0].offset,
            Some(0),
            "A preferred service still receives the specified default offset"
        );
    }

    #[test]
    fn test_build_group_request_service_without_offset_or_preferred() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: None,
                preferred: None,
                env: None,
            },
        );

        let group = ReservationGroup {
            base: None,
            services,
        };

        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test
ports:
  min: 5000
  max: 7000
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();

        let request = planner.build_group_request(&group).unwrap();

        assert_eq!(request.services.len(), 1);
        assert_eq!(request.services[0].tag, "web");
        assert_eq!(request.services[0].offset, Some(0));
        assert_eq!(request.services[0].preferred, None);
    }

    #[test]
    fn test_group_base_becomes_scan_start() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test
ports:
  min: 5000
  max: 7000
reservations:
  base: 6500
  services:
    web:
      offset: 0
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();
        let group = planner.config.reservations.as_ref().unwrap();

        let config = planner.config_with_group_base_as_scan_start(group).unwrap();
        let ports = config.ports.unwrap();

        assert_eq!(ports.min, 6500);
        assert_eq!(ports.max, Some(7000));
        assert_eq!(ports.max_offset, None);
    }

    #[test]
    fn test_group_base_outside_range_is_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test
ports:
  min: 5000
  max: 6000
reservations:
  base: 6500
  services:
    web:
      offset: 0
";
        let config_path = create_test_config_file(&temp_dir, config_content);

        // Validation of the document now rejects the base (it used to be
        // accepted until the plan was built) ...
        let err = ReserveGroupPlan::new(ReserveGroupOptions::new(config_path))
            .err()
            .expect("an out-of-range base is a validation error");
        assert!(
            matches!(&err, Error::Validation { field, message }
                if field == "reservations.base"
                    && message == "Base port 6500 must be within port range 5000..6000"),
            "{err:?}"
        );

        // ... and the scan-start narrowing still refuses it on its own.
        let planner = planner_with_ports(&temp_dir, "  min: 5000\n  max: 6000");
        let err = planner
            .config_with_group_base_as_scan_start(&group_with_base(Some(6500)))
            .unwrap_err();
        assert!(
            matches!(&err, Error::Validation { field, message }
                if field == "reservations.base"
                    && message == "Base port 6500 must be within port range 5000..6000"),
            "{err:?}"
        );
    }

    fn planner_with_ports(dir: &TempDir, ports: &str) -> ReserveGroupPlan {
        let config_content = format!("project: test\nports:\n{ports}\n");
        let config_path = create_test_config_file(dir, &config_content);
        ReserveGroupPlan::new(ReserveGroupOptions::new(config_path)).unwrap()
    }

    fn group_with_base(base: Option<u16>) -> ReservationGroup {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            },
        );
        ReservationGroup { base, services }
    }

    /// Regression: the group task follows the same identifier rules as a
    /// single reservation's task (no NUL bytes, at most 255 bytes).
    #[test]
    fn test_reserve_group_validates_task_like_reserve() {
        let temp_dir = TempDir::new().unwrap();
        let config_content = r"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
";
        let config_path = create_test_config_file(&temp_dir, config_content);
        let db = create_test_database();

        for (task, message) in [
            ("x".repeat(300), "Cannot exceed 255 bytes"),
            ("dev\0task".to_string(), "Cannot contain null bytes"),
        ] {
            let options = ReserveGroupOptions::new(config_path.clone())
                .with_allow_unrelated_path(true)
                .with_task(Some(task.clone()));
            let planner = ReserveGroupPlan::new(options).unwrap();
            let single = ConfigValidator::validate_runtime_identifier("task", &task)
                .expect_err("single reserve rejects this task");

            match planner.build_plan(db.connection()) {
                Err(Error::Validation {
                    field,
                    message: actual,
                }) => {
                    assert_eq!(field, "task");
                    assert_eq!(actual, message);
                    assert!(
                        matches!(&single, Error::Validation { message, .. } if *message == actual),
                        "group and single-reservation task errors differ: {single:?}"
                    );
                }
                other => panic!("expected a task validation error, got {other:?}"),
            }
        }

        // A 255-byte task (after trimming) is accepted, trimmed.
        let task = "t".repeat(255);
        let options = ReserveGroupOptions::new(config_path)
            .with_allow_unrelated_path(true)
            .with_task(Some(format!(" {task} ")));
        let plan = ReserveGroupPlan::new(options)
            .unwrap()
            .build_plan(db.connection())
            .unwrap();
        match &plan.actions[0] {
            PlanAction::AllocateGroup { request, .. } => {
                assert_eq!(request.task.as_deref(), Some(task.as_str()));
            }
            _ => panic!("Expected AllocateGroup action"),
        }
    }

    /// Regression: services are built in tag order, not `HashMap` order, so
    /// the request (and the first validation error) is deterministic.
    #[test]
    fn test_build_group_request_orders_services_by_tag() {
        let tags = [
            "kappa", "alpha", "zeta", "delta", "beta", "theta", "eta", "gamma", "iota", "epsilon",
        ];
        let mut services = HashMap::new();
        for (offset, tag) in (0u16..).zip(tags) {
            services.insert(
                tag.to_string(),
                ServiceDefinition {
                    offset: Some(offset),
                    preferred: None,
                    env: None,
                },
            );
        }
        let group = ReservationGroup {
            base: None,
            services,
        };

        let temp_dir = TempDir::new().unwrap();
        let planner = planner_with_ports(&temp_dir, "  min: 5000\n  max: 7000");
        let request = planner.build_group_request(&group).unwrap();

        let mut expected = tags.map(str::to_string).to_vec();
        expected.sort();
        let actual = request
            .services
            .iter()
            .map(|service| service.tag.clone())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_group_scan_bounds_at_range_edges() {
        let temp_dir = TempDir::new().unwrap();

        // No base: the configuration is unchanged (max_offset kept).
        let planner = planner_with_ports(&temp_dir, "  min: 5000\n  max_offset: 100");
        let config = planner
            .config_with_group_base_as_scan_start(&group_with_base(None))
            .unwrap();
        let ports = config.ports.unwrap();
        assert_eq!(
            (ports.min, ports.max, ports.max_offset),
            (5000, None, Some(100))
        );

        // Base at either edge of a max_offset range: max comes from the
        // configured min, never from the base.
        for base in [5000, 5100] {
            let config = planner
                .config_with_group_base_as_scan_start(&group_with_base(Some(base)))
                .unwrap();
            let ports = config.ports.unwrap();
            assert_eq!(
                (ports.min, ports.max, ports.max_offset),
                (base, Some(5100), None)
            );
        }

        // One past either edge is rejected.
        for base in [4999, 5101] {
            let err = planner
                .config_with_group_base_as_scan_start(&group_with_base(Some(base)))
                .unwrap_err();
            assert!(
                matches!(&err, Error::Validation { field, message }
                    if field == "reservations.base"
                        && *message == format!("Base port {base} must be within port range 5000..5100")),
                "unexpected error: {err:?}"
            );
        }

        // The whole port domain.
        let planner = planner_with_ports(&temp_dir, "  min: 1\n  max: 65535");
        let config = planner
            .config_with_group_base_as_scan_start(&group_with_base(Some(65535)))
            .unwrap();
        let ports = config.ports.unwrap();
        assert_eq!((ports.min, ports.max), (65535, Some(65535)));
    }

    #[test]
    fn test_group_scan_bounds_reports_missing_ports() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = create_test_config_file(&temp_dir, "project: test\n");
        let planner = ReserveGroupPlan::new(ReserveGroupOptions::new(config_path)).unwrap();

        let err = planner
            .config_with_group_base_as_scan_start(&group_with_base(Some(5000)))
            .unwrap_err();
        assert!(matches!(err, Error::Validation { field, .. } if field == "ports"));
    }

    /// Non-export formats never consult the shell, so an unknown `--shell`
    /// is ignored for them; export parses an explicit shell (and never falls
    /// back to detection when it is unknown).
    #[test]
    fn group_output_selection_consults_the_shell_only_for_export() {
        for (format, expected) in [
            (GroupOutputFormat::Json, OutputFormat::Json),
            (GroupOutputFormat::Dotenv, OutputFormat::Dotenv),
            (GroupOutputFormat::Human, OutputFormat::Human),
        ] {
            assert_eq!(select_group_output(format, Some("csh")).unwrap(), expected);
            assert_eq!(select_group_output(format, None).unwrap(), expected);
        }

        assert_eq!(
            select_group_output(GroupOutputFormat::Export, Some("ZSH")).unwrap(),
            OutputFormat::Export(ShellType::Zsh)
        );
        assert_eq!(
            select_group_output(GroupOutputFormat::Export, Some("pwsh")).unwrap(),
            OutputFormat::Export(ShellType::PowerShell)
        );
        let error = select_group_output(GroupOutputFormat::Export, Some("csh")).unwrap_err();
        assert!(matches!(error, Error::Validation { ref field, .. } if field == "shell"));

        // Without an explicit shell, export uses exactly what detection says.
        let detected = ShellType::detect().map(OutputFormat::Export);
        let selected = select_group_output(GroupOutputFormat::Export, None);
        assert_eq!(selected.ok(), detected.ok());
    }

    /// A dry-run snapshot holds every row of the source and keeps its
    /// constraints and indexes; writing to it never reaches the file.
    #[test]
    fn dry_run_snapshot_copies_rows_constraints_and_indexes() {
        use crate::{Reservation, ReservationKey};

        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("trop.db");
        {
            let mut db = Database::open(crate::DatabaseConfig::new(&db_path)).unwrap();
            for (index, port) in [5000_u16, 5001, 65535].into_iter().enumerate() {
                let key = ReservationKey::new(
                    PathBuf::from(format!("/snapshot/{index}")),
                    (index > 0).then(|| format!("tag{index}")),
                )
                .unwrap();
                let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                    .project(Some("p".to_string()))
                    .build()
                    .unwrap();
                db.create_reservation(&reservation).unwrap();
            }
        }
        let original = Database::list_all_reservations(
            Database::open(crate::DatabaseConfig::new(&db_path).read_only())
                .unwrap()
                .connection(),
        )
        .unwrap();

        let snapshot =
            open_dry_run_snapshot(Some(&db_path), std::time::Duration::from_secs(1)).unwrap();

        assert_eq!(
            Database::list_all_reservations(&snapshot).unwrap(),
            original
        );
        let indexes: i64 = snapshot
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND sql IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 3);
        // The UNIQUE port constraint survives the copy.
        let duplicate = snapshot.execute(
            "INSERT INTO reservations (path, tag, port, created_at, last_used_at)
             VALUES ('/other', '', 5000, 0, 0)",
            [],
        );
        assert!(duplicate.is_err());
        snapshot.execute("DELETE FROM reservations", []).unwrap();
        drop(snapshot);
        let after = Database::list_all_reservations(
            Database::open(crate::DatabaseConfig::new(&db_path).read_only())
                .unwrap()
                .connection(),
        )
        .unwrap();
        assert_eq!(after, original, "the file never changes");
    }

    /// Without a database file the snapshot is a freshly initialized empty
    /// database, and nothing is created on disk.
    #[test]
    fn dry_run_snapshot_without_a_database_is_empty_and_initialized() {
        let dir = TempDir::new().unwrap();
        let snapshot = open_dry_run_snapshot(None, std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(Database::list_all_reservations(&snapshot).unwrap(), []);
        assert_eq!(
            crate::database::get_schema_version(&snapshot).unwrap(),
            crate::database::get_schema_version(create_test_database().connection()).unwrap()
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
