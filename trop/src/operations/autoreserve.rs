//! Autoreserve operation planning and execution.
//!
//! This module implements auto-discovery of configuration files and delegation
//! to reserve group logic for batch port reservations.

use std::path::PathBuf;

use crate::config::{Config, ConfigField, ConfigFileKind, ConfigLoader, EffectiveConfig};
use crate::error::{Error, Result};
use crate::verified::group::{first_present, select_highest_precedence};
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

    fn into_reserve_group_options(self, config_path: PathBuf) -> ReserveGroupOptions {
        ReserveGroupOptions {
            config_path,
            task: self.task,
            force: self.force,
            allow_unrelated_path: self.allow_unrelated_path,
            allow_project_change: self.allow_project_change,
            allow_task_change: self.allow_task_change,
        }
    }
}

/// An autoreserve plan generator.
///
/// This struct is responsible for discovering a configuration file and
/// delegating to `ReserveGroupPlan` for actual reservation planning.
pub struct AutoreservePlan {
    reserve_group: ReserveGroupPlan,
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
        let mut configs = ConfigLoader::discover_project_configs(&options.start_dir)?;

        // Use the highest-precedence config (the last one on ties, as
        // `Iterator::max_by_key` would); the verified kernel
        // `verified::group::select_highest_precedence` decides. No config
        // file at all is an error.
        let precedences = configs
            .iter()
            .map(|config| config.precedence)
            .collect::<Vec<_>>();
        let Some(selected) = select_highest_precedence(&precedences) else {
            return Err(Error::InvalidPath {
                path: options.start_dir.clone(),
                reason: format!(
                    "No trop configuration file found searching from {}",
                    options.start_dir.display()
                ),
            });
        };
        let selected_config = configs.swap_remove(selected);
        let reserve_group_options = options.into_reserve_group_options(selected_config.path);
        let reserve_group =
            ReserveGroupPlan::from_config(reserve_group_options, selected_config.config)?;

        Ok(Self { reserve_group })
    }

    /// Create an autoreserve plan from one resolved effective configuration.
    ///
    /// The reservation group's winning source identifies the discovered
    /// project file. If no group supplied a file source, the highest-precedence
    /// loaded project file is retained for diagnostics.
    ///
    /// # Errors
    ///
    /// Returns an error if no project configuration file participated in the
    /// effective configuration or if the effective values are invalid.
    pub fn from_effective(options: AutoreserveOptions, config: &EffectiveConfig) -> Result<Self> {
        // Candidates in order: the reservation group's winning file, then the
        // loaded local file, then the loaded project file. The verified kernel
        // `verified::group::first_present` picks the first one present.
        let candidates = [
            config
                .provenance(ConfigField::Reservations)
                .and_then(|provenance| provenance.winner().file_path()),
            config.loaded_file(ConfigFileKind::Local),
            config.loaded_file(ConfigFileKind::Project),
        ];
        let present = candidates.map(|candidate| candidate.is_some());
        let discovered_config_path = first_present(&present)
            .and_then(|index| candidates[index])
            .ok_or_else(|| Error::InvalidPath {
                path: options.start_dir.clone(),
                reason: format!(
                    "No trop configuration file found searching from {}",
                    options.start_dir.display()
                ),
            })?
            .to_path_buf();

        let reserve_group_options = options.into_reserve_group_options(discovered_config_path);
        let reserve_group = ReserveGroupPlan::from_effective(reserve_group_options, config)?;

        Ok(Self { reserve_group })
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
        self.reserve_group.build_plan(conn)
    }

    /// Returns the discovered configuration file path.
    #[must_use]
    pub fn discovered_config_path(&self) -> &PathBuf {
        self.reserve_group.config_path()
    }

    /// Returns the exact validated configuration snapshot used by this planner.
    #[must_use]
    pub const fn config(&self) -> &Config {
        self.reserve_group.config()
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
        assert!(plan.discovered_config_path().ends_with("trop.yaml"));
    }

    #[test]
    fn test_autoreserve_config_accessor_is_creation_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("trop.yaml");
        create_test_config_file(
            temp_dir.path(),
            "trop.yaml",
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
        let planner = AutoreservePlan::new(
            AutoreserveOptions::new(temp_dir.path().to_path_buf()).with_allow_unrelated_path(true),
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
    fn test_autoreserve_plan_prefers_local_config() {
        let temp_dir = TempDir::new().unwrap();

        // Create both trop.yaml and trop.local.yaml
        create_test_config_file(temp_dir.path(), "trop.yaml", "project: main\n");
        create_test_config_file(temp_dir.path(), "trop.local.yaml", "project: local\n");

        let options =
            AutoreserveOptions::new(temp_dir.path().to_path_buf()).with_allow_unrelated_path(true);
        let plan = AutoreservePlan::new(options).unwrap();

        // Should prefer trop.local.yaml (higher precedence)
        assert!(plan.discovered_config_path().ends_with("trop.local.yaml"));
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

        assert!(plan.discovered_config_path().ends_with("trop.yaml"));
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
        let options =
            AutoreserveOptions::new(temp_dir.path().to_path_buf()).with_allow_unrelated_path(true);
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

    #[test]
    fn test_precedence_selection_matches_max_by_key() {
        // The verified selection agrees with `Iterator::max_by_key`, which
        // returns the last of several equally high precedences.
        for precedences in [
            vec![],
            vec![2],
            vec![2, 3],
            vec![3, 2],
            vec![2, 2],
            vec![1, 3, 3, 2],
            vec![u8::MAX, 0, u8::MAX],
        ] {
            let expected = precedences
                .iter()
                .enumerate()
                .max_by_key(|(_, precedence)| **precedence)
                .map(|(index, _)| index);
            assert_eq!(select_highest_precedence(&precedences), expected);
        }
    }

    #[test]
    fn test_first_present_candidate_wins() {
        assert_eq!(first_present(&[]), None);
        assert_eq!(first_present(&[false, false, false]), None);
        assert_eq!(first_present(&[false, true, true]), Some(1));
        assert_eq!(first_present(&[true, false, true]), Some(0));
        assert_eq!(first_present(&[false, false, true]), Some(2));
    }
}
