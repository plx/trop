//! Autoreserve operation planning and execution.
//!
//! This module implements auto-discovery of configuration files and delegation
//! to reserve group logic for batch port reservations.

use std::path::PathBuf;

use crate::config::ConfigLoader;
use crate::error::{Error, Result};
use rusqlite::Connection;

use super::plan::OperationPlan;
use super::reserve_group::{ReserveGroupOptions, ReserveGroupPlan};

/// Options for an autoreserve operation.
///
/// This struct contains all the parameters needed to plan an autoreserve
/// operation, which discovers a config file and reserves the defined group.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct AutoreserveOptions {
    /// Directory to start searching from (typically current working directory).
    pub start_dir: PathBuf,

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

impl AutoreserveOptions {
    /// Creates a new `AutoreserveOptions` with the given start directory.
    ///
    /// All optional fields and flags are set to defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::AutoreserveOptions;
    /// use std::path::PathBuf;
    ///
    /// let options = AutoreserveOptions::new(PathBuf::from("."));
    /// assert!(!options.force);
    /// ```
    #[must_use]
    pub fn new(start_dir: PathBuf) -> Self {
        Self {
            start_dir,
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

/// An autoreserve plan generator.
///
/// This struct is responsible for discovering a configuration file and
/// delegating to `ReserveGroupPlan` for actual reservation planning.
pub struct AutoreservePlan {
    options: AutoreserveOptions,
    discovered_config_path: PathBuf,
}

impl AutoreservePlan {
    /// Creates a new autoreserve plan with the given options.
    ///
    /// This discovers the configuration file by walking up from the start
    /// directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No configuration file is found
    /// - The discovered config file cannot be read or parsed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{AutoreservePlan, AutoreserveOptions};
    /// use std::path::PathBuf;
    ///
    /// let options = AutoreserveOptions::new(PathBuf::from("."));
    /// let planner = AutoreservePlan::new(options).unwrap();
    /// ```
    pub fn new(options: AutoreserveOptions) -> Result<Self> {
        // Discover project configs
        let configs = ConfigLoader::discover_project_configs(&options.start_dir)?;

        // We need at least one config file
        if configs.is_empty() {
            return Err(Error::InvalidPath {
                path: options.start_dir.clone(),
                reason: format!(
                    "No trop configuration file found searching from {}",
                    options.start_dir.display()
                ),
            });
        }

        // Use the highest precedence config (last in the sorted list)
        // ConfigLoader::discover_project_configs returns configs sorted by precedence
        let discovered_config_path = configs
            .iter()
            .max_by_key(|c| c.precedence)
            .map(|c| c.path.clone())
            .ok_or_else(|| Error::InvalidPath {
                path: options.start_dir.clone(),
                reason: "Failed to determine config path".to_string(),
            })?;

        Ok(Self {
            options,
            discovered_config_path,
        })
    }

    /// Builds an operation plan for this autoreserve request.
    ///
    /// This delegates to `ReserveGroupPlan` once the config file is discovered.
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
    /// use trop::operations::{AutoreservePlan, AutoreserveOptions};
    /// use trop::{Database, DatabaseConfig};
    /// use std::path::PathBuf;
    ///
    /// let db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let options = AutoreserveOptions::new(PathBuf::from("."));
    /// let planner = AutoreservePlan::new(options).unwrap();
    /// let plan = planner.build_plan(db.connection()).unwrap();
    /// ```
    pub fn build_plan(&self, conn: &Connection) -> Result<OperationPlan> {
        // Build options for ReserveGroupPlan
        let reserve_group_options = ReserveGroupOptions {
            config_path: self.discovered_config_path.clone(),
            task: self.options.task.clone(),
            force: self.options.force,
            allow_unrelated_path: self.options.allow_unrelated_path,
            allow_project_change: self.options.allow_project_change,
            allow_task_change: self.options.allow_task_change,
        };

        // Delegate to ReserveGroupPlan
        let reserve_group_plan = ReserveGroupPlan::new(reserve_group_options)?;
        reserve_group_plan.build_plan(conn)
    }

    /// Returns the discovered configuration file path.
    #[must_use]
    pub fn discovered_config_path(&self) -> &PathBuf {
        &self.discovered_config_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_config_file(dir: &std::path::Path, name: &str, content: &str) {
        let config_path = dir.join(name);
        fs::write(config_path, content).unwrap();
    }

    #[test]
    fn test_autoreserve_options_new() {
        let options = AutoreserveOptions::new(PathBuf::from("."));
        assert!(!options.force);
        assert!(!options.allow_unrelated_path);
        assert!(options.task.is_none());
    }

    #[test]
    fn test_autoreserve_options_builder() {
        let options = AutoreserveOptions::new(PathBuf::from("."))
            .with_task(Some("dev".to_string()))
            .with_force(true)
            .with_allow_unrelated_path(true);

        assert!(options.force);
        assert!(options.allow_unrelated_path);
        assert_eq!(options.task, Some("dev".to_string()));
    }

    #[test]
    fn test_autoreserve_plan_discovers_config() {
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
        create_test_config_file(temp_dir.path(), "trop.yaml", config_content);

        let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
        let plan = AutoreservePlan::new(options);

        assert!(plan.is_ok());
        let plan = plan.unwrap();
        assert!(plan.discovered_config_path.ends_with("trop.yaml"));
    }

    #[test]
    fn test_autoreserve_plan_prefers_local_config() {
        let temp_dir = TempDir::new().unwrap();

        // Create both trop.yaml and trop.local.yaml
        create_test_config_file(temp_dir.path(), "trop.yaml", "project: main\n");
        create_test_config_file(temp_dir.path(), "trop.local.yaml", "project: local\n");

        let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
        let plan = AutoreservePlan::new(options).unwrap();

        // Should prefer trop.local.yaml (higher precedence)
        assert!(plan.discovered_config_path.ends_with("trop.local.yaml"));
    }

    #[test]
    fn test_autoreserve_plan_no_config_found() {
        let temp_dir = TempDir::new().unwrap();

        let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
        let result = AutoreservePlan::new(options);

        assert!(result.is_err());
        match result {
            Err(Error::InvalidPath { reason, .. }) => {
                assert!(reason.contains("No trop configuration file found"));
            }
            _ => panic!("Expected InvalidPath error"),
        }
    }

    #[test]
    fn test_autoreserve_plan_discovers_from_child_dir() {
        let temp_dir = TempDir::new().unwrap();
        let child_dir = temp_dir.path().join("child");
        fs::create_dir(&child_dir).unwrap();

        // Put config in parent
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
        create_test_config_file(temp_dir.path(), "trop.yaml", config_content);

        // Discover from child - should find parent's config
        let options = AutoreserveOptions::new(child_dir);
        let plan = AutoreservePlan::new(options).unwrap();

        assert!(plan.discovered_config_path.ends_with("trop.yaml"));
    }

    #[test]
    fn test_autoreserve_plan_build_plan() {
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
        create_test_config_file(temp_dir.path(), "trop.yaml", config_content);

        let db = create_test_database();
        let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
        let planner = AutoreservePlan::new(options).unwrap();
        let plan = planner.build_plan(db.connection()).unwrap();

        assert_eq!(plan.actions.len(), 1);
    }

    #[test]
    fn test_autoreserve_discovered_config_path_accessor() {
        let temp_dir = TempDir::new().unwrap();
        create_test_config_file(temp_dir.path(), "trop.yaml", "project: test\n");

        let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
        let planner = AutoreservePlan::new(options).unwrap();

        let path = planner.discovered_config_path();
        assert!(path.ends_with("trop.yaml"));
    }
}
