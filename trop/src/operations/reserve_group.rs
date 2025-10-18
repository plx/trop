//! Reserve group operation planning and execution.
//!
//! This module implements group reservation planning, which reserves multiple
//! related ports based on a configuration file.

use std::path::PathBuf;

use crate::config::{Config, ConfigLoader};
use crate::database::Database;
use crate::error::{Error, Result};
use crate::port::group::{GroupAllocationRequest, ServiceAllocationRequest};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::Port;

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

    /// Force flag - overrides all protections.
    pub force: bool,

    /// Allow operations on unrelated paths.
    pub allow_unrelated_path: bool,

    /// Allow changing the project field.
    pub allow_project_change: bool,

    /// Allow changing the task field.
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
    base_path: PathBuf,
}

impl ReserveGroupPlan {
    /// Creates a new reserve group plan with the given options.
    ///
    /// This loads the configuration file and validates its contents.
    ///
    /// # Errors
    ///
    /// Returns an error if:
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
        // Load the configuration file
        let config = ConfigLoader::load_file(&options.config_path)?;

        // Get the base path (parent directory of the config file)
        let base_path = options
            .config_path
            .parent()
            .ok_or_else(|| Error::InvalidPath {
                path: options.config_path.clone(),
                reason: "Config file has no parent directory".to_string(),
            })?
            .to_path_buf();

        Ok(Self {
            options,
            config,
            base_path,
        })
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
    /// (`ReservePlan`, `ReleasePlan`). Group allocation happens during execution
    /// (not during planning), so the database isn't needed at plan-building time.
    /// This matches the signature expected by the operations system.
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
    /// let plan = planner.build_plan(&db).unwrap();
    /// ```
    pub fn build_plan(&self, _db: &Database) -> Result<OperationPlan> {
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

        // Convert the reservation group to a GroupAllocationRequest
        let request = self.build_group_request(reservation_group)?;

        // Build the plan
        let mut plan = OperationPlan::new(format!(
            "Reserve group of {} services from {}",
            reservation_group.services.len(),
            self.options.config_path.display()
        ));

        let occupancy_config = self.occupancy_config();

        plan = plan.add_action(PlanAction::AllocateGroup {
            request,
            full_config: self.config.clone(),
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
            // Validate service has either offset or preferred
            if service_def.offset.is_none() && service_def.preferred.is_none() {
                return Err(Error::Validation {
                    field: format!("reservations.services.{tag}"),
                    message: "Service must have either offset or preferred port".to_string(),
                });
            }

            let preferred = service_def.preferred.map(Port::try_from).transpose()?;

            services.push(ServiceAllocationRequest {
                tag: tag.clone(),
                offset: service_def.offset,
                preferred,
            });
        }

        Ok(GroupAllocationRequest {
            base_path: self.base_path.clone(),
            project: self.config.project.clone(),
            task: self.options.task.clone(),
            services,
        })
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

        let options = ReserveGroupOptions::new(config_path);
        let plan = ReserveGroupPlan::new(options).unwrap();
        let result = plan.build_plan(&db);

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

        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).unwrap();
        let plan = planner.build_plan(&db).unwrap();

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
        let result = planner.build_plan(&db);

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

        let result = planner.build_group_request(&group);

        assert!(result.is_err());
        match result {
            Err(Error::Validation { field, .. }) => {
                assert!(field.contains("web"));
            }
            _ => panic!("Expected validation error"),
        }
    }
}
