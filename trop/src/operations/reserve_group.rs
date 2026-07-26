//! Reserve group operation planning and execution.
//!
//! This module implements group reservation planning, which reserves multiple
//! related ports based on a configuration file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigLoader, ConfigValidator, EffectiveConfig};
use crate::database::Database;
use crate::error::{Error, Result};
use crate::port::group::{
    GroupAllocationRequest, GroupReconciliationPolicy, ServiceAllocationRequest,
};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::{PathResolver, Port};
use rusqlite::Connection;

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

    /// Builds a `GroupAllocationRequest` from the reservation group.
    fn build_group_request(
        &self,
        group: &crate::config::ReservationGroup,
    ) -> Result<GroupAllocationRequest> {
        let mut services = Vec::new();

        for (tag, service_def) in &group.services {
            let preferred = service_def.preferred.map(Port::try_from).transpose()?;

            services.push(ServiceAllocationRequest {
                tag: tag.clone(),
                offset: Some(service_def.offset.unwrap_or(0)),
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
    fn config_with_group_base_as_scan_start(
        &self,
        group: &crate::config::ReservationGroup,
    ) -> Result<Config> {
        let Some(base) = group.base else {
            return Ok(self.config.clone());
        };

        let mut config = self.config.clone();
        let ports = config.ports.as_mut().ok_or_else(|| Error::Validation {
            field: "ports".to_string(),
            message: "Port configuration is required when reservations.base is set".to_string(),
        })?;

        let max = if let Some(max) = ports.max {
            max
        } else if let Some(max_offset) = ports.max_offset {
            ports
                .min
                .checked_add(max_offset)
                .ok_or_else(|| Error::Validation {
                    field: "ports.max_offset".to_string(),
                    message: format!(
                        "Offset {max_offset} would overflow when added to min port {}",
                        ports.min
                    ),
                })?
        } else {
            return Err(Error::Validation {
                field: "ports".to_string(),
                message: "Either max or max_offset must be specified".to_string(),
            });
        };

        if base < ports.min || base > max {
            return Err(Error::Validation {
                field: "reservations.base".to_string(),
                message: format!(
                    "Base port {base} must be within port range {}..{max}",
                    ports.min
                ),
            });
        }

        ports.min = base;
        ports.max = Some(max);
        ports.max_offset = None;

        Ok(config)
    }
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
        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();
        let group = planner.config.reservations.as_ref().unwrap();

        let err = planner
            .config_with_group_base_as_scan_start(group)
            .unwrap_err();

        assert!(matches!(err, Error::Validation { field, .. } if field == "reservations.base"));
    }
}
