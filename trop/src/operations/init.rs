//! Database initialization operations.
//!
//! This module provides functionality for explicitly initializing the trop
//! data directory and database, with support for dry-run mode and optional
//! configuration file creation.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::{Database, DatabaseConfig};

/// Options for database initialization.
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Data directory to initialize.
    pub data_dir: PathBuf,
    /// Overwrite existing database if it exists.
    pub overwrite: bool,
    /// Create a default configuration file.
    pub create_config: bool,
    /// Maximum time to wait for `SQLite` lock acquisition.
    pub busy_timeout: Duration,
}

impl InitOptions {
    /// Creates new initialization options.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            overwrite: false,
            create_config: false,
            busy_timeout: Duration::from_secs(5),
        }
    }

    /// Sets whether to overwrite existing database.
    #[must_use]
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Sets whether to create default configuration file.
    #[must_use]
    pub fn with_create_config(mut self, create_config: bool) -> Self {
        self.create_config = create_config;
        self
    }

    /// Sets the maximum time to wait for `SQLite` lock acquisition.
    #[must_use]
    pub const fn with_busy_timeout(mut self, busy_timeout: Duration) -> Self {
        self.busy_timeout = busy_timeout;
        self
    }
}

/// Result of initialization operation.
// One flag per independent step that was carried out.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct InitResult {
    /// Whether the data directory was created.
    pub data_dir_created: bool,
    /// Whether the database was created or recreated.
    pub database_created: bool,
    /// Whether an existing database was replaced (`--overwrite` with a
    /// database present).
    pub database_replaced: bool,
    /// Whether a configuration file was created.
    pub config_created: bool,
    /// Path to the data directory.
    pub data_dir: PathBuf,
}

/// What an initialization will do, decided before anything changes.
///
/// [`init_database`] carries out exactly this plan, and a dry run reports
/// it, so both agree on every step (and on refusing to replace an existing
/// database without `overwrite`, see [`plan_init`]).
// One flag per independent step (and the one observed fact it reports).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitPlan {
    /// The data directory.
    pub data_dir: PathBuf,
    /// The data directory does not exist and will be created.
    pub create_data_dir: bool,
    /// An existing database will be removed (with its `-wal`, `-shm` and
    /// `-journal` files) and replaced by a new one.
    pub replace_database: bool,
    /// The default configuration file will be written.
    pub create_config: bool,
    /// A configuration file exists and will be kept as it is.
    pub config_exists: bool,
}

impl InitPlan {
    /// The database file (`trop.db` in the data directory). A database is
    /// created (or recreated) by every plan.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE)
    }

    /// The configuration file (`config.yaml` in the data directory).
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join(CONFIG_FILE)
    }
}

/// The database file name.
const DATABASE_FILE: &str = "trop.db";
/// The user configuration file name.
const CONFIG_FILE: &str = "config.yaml";
/// The files `SQLite` keeps beside a database (write-ahead log, its shared
/// memory index, and the rollback journal). A stale one must not survive
/// next to a newly created database.
const DATABASE_SIDE_FILES: [&str; 3] = ["trop.db-wal", "trop.db-shm", "trop.db-journal"];

/// Default user configuration template.
///
/// Every setting is commented out and shows the built-in default (lists
/// show an example), so the file changes nothing until edited; uncommenting
/// any setting gives a valid user configuration. Project-only settings
/// (`project`, `reservations`) belong in a project's `trop.yaml`.
const DEFAULT_CONFIG_TEMPLATE: &str = r##"# trop user configuration
#
# Settings here apply to every project. Each setting is commented out and
# shows the built-in default; remove the leading "# " from its lines to set
# it. Project settings (project, reservations) belong in a project's
# trop.yaml instead.

# Ports trop allocates from (default 5000 through 7000)
# ports:
#   min: 5000
#   max: 7000

# Ports trop never allocates, as single ports or "start..end" ranges
# (default none; these lines are an example)
# excluded_ports:
#   - 5432
#   - "6000..6010"

# Seconds to wait for the database lock (default 5)
# maximum_lock_wait_seconds: 5

# Days after which an unused reservation expires (default 30)
# cleanup:
#   expire_after_days: 30

# Format of trop list output, one of table, json, csv or tsv (default table)
# output_format: table
"##;

/// Plans an initialization from the current state of the data directory.
///
/// The decision is made by the formally verified kernel
/// `verified::cli::plan_init` (see `docs/verification.md`): an existing
/// database is refused unless `overwrite` is set; otherwise the plan creates
/// the data directory if it is missing, replaces an existing database only
/// with `overwrite`, and writes the default configuration only when
/// `create_config` is set and no configuration file exists. Those steps are
/// the only ones that reach that state, and no step is taken whose effect
/// already holds.
///
/// # Errors
///
/// Returns a validation error for the `database` field if the database
/// exists and `overwrite` is not set; [`init_database`] fails with the same
/// error.
pub fn plan_init(options: &InitOptions) -> Result<InitPlan> {
    use crate::verified::cli::{self as kernel, InitFacts};

    let data_dir = options.data_dir.clone();
    let db_path = data_dir.join(DATABASE_FILE);
    let config_exists = data_dir.join(CONFIG_FILE).exists();
    let facts = InitFacts {
        dir_exists: data_dir.exists(),
        db_exists: db_path.exists(),
        config_exists,
        overwrite: options.overwrite,
        with_config: options.create_config,
    };
    match kernel::plan_init(facts) {
        kernel::InitPlan::RefuseExistingDatabase => Err(Error::Validation {
            field: "database".into(),
            message: format!(
                "Database already exists at {}. Use --overwrite to replace it.",
                db_path.display()
            ),
        }),
        kernel::InitPlan::Proceed(steps) => Ok(InitPlan {
            data_dir,
            create_data_dir: steps.create_dir,
            replace_database: steps.remove_db,
            create_config: steps.write_config,
            config_exists,
        }),
    }
}

/// Initializes the trop data directory and database.
///
/// This function creates the data directory if needed, initializes the database,
/// and optionally creates a default configuration file. It carries out the
/// plan from [`plan_init`].
///
/// # Errors
///
/// Returns an error if:
/// - The data directory cannot be created
/// - The database cannot be initialized
/// - The configuration file cannot be written
/// - Overwrite is false and the database already exists
///
/// # Examples
///
/// ```no_run
/// use trop::operations::init::{InitOptions, init_database};
/// use std::path::PathBuf;
///
/// let options = InitOptions::new(PathBuf::from("/tmp/trop-test"))
///     .with_overwrite(false)
///     .with_create_config(true);
///
/// let result = init_database(&options).unwrap();
/// println!("Database created: {}", result.database_created);
/// ```
pub fn init_database(options: &InitOptions) -> Result<InitResult> {
    let plan = plan_init(options)?;

    // 1. Create the data directory if it doesn't exist.
    if plan.create_data_dir {
        fs::create_dir_all(&plan.data_dir)?;
    }

    // 2. Remove an existing database (and its side files) when replacing it.
    let db_path = plan.database_path();
    if plan.replace_database {
        fs::remove_file(&db_path)?;
        for side_file in DATABASE_SIDE_FILES {
            match fs::remove_file(plan.data_dir.join(side_file)) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
    }

    // 3. Initialize the database (this creates the schema).
    let db_config = DatabaseConfig::new(&db_path).with_busy_timeout(options.busy_timeout);
    let _db = Database::open(db_config)?;

    // 4. Optionally create the default configuration file (never replacing
    // an existing one).
    if plan.create_config {
        fs::write(plan.config_path(), DEFAULT_CONFIG_TEMPLATE)?;
    }

    Ok(InitResult {
        data_dir_created: plan.create_data_dir,
        database_created: true,
        database_replaced: plan.replace_database,
        config_created: plan.create_config,
        data_dir: plan.data_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_options_busy_timeout_override() {
        let options =
            InitOptions::new(PathBuf::from("/tmp/trop")).with_busy_timeout(Duration::from_secs(17));

        assert_eq!(options.busy_timeout, Duration::from_secs(17));
    }

    #[test]
    fn test_init_fresh_directory() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        let options = InitOptions::new(data_dir.clone());
        let result = init_database(&options).unwrap();

        assert!(result.data_dir_created);
        assert!(result.database_created);
        assert!(!result.config_created);
        assert!(data_dir.exists());
        assert!(data_dir.join("trop.db").exists());
    }

    #[test]
    fn test_init_existing_directory() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().to_path_buf();

        let options = InitOptions::new(data_dir.clone());
        let result = init_database(&options).unwrap();

        assert!(!result.data_dir_created); // Directory already existed
        assert!(result.database_created);
        assert!(data_dir.join("trop.db").exists());
    }

    #[test]
    fn test_init_with_config() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        let options = InitOptions::new(data_dir.clone()).with_create_config(true);
        let result = init_database(&options).unwrap();

        assert!(result.data_dir_created);
        assert!(result.database_created);
        assert!(result.config_created);
        assert!(data_dir.join("config.yaml").exists());

        // Verify config content
        let config_content = fs::read_to_string(data_dir.join("config.yaml")).unwrap();
        assert!(config_content.contains("# trop user configuration"));
    }

    #[test]
    fn test_init_fails_without_overwrite() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        // Create first time
        let options = InitOptions::new(data_dir.clone());
        init_database(&options).unwrap();

        // Try to create again without overwrite
        let options = InitOptions::new(data_dir.clone());
        let result = init_database(&options);

        assert!(result.is_err());
        match result {
            Err(Error::Validation { field, message }) => {
                assert_eq!(field, "database");
                assert!(message.contains("already exists"));
                assert!(message.contains("--overwrite"));
            }
            _ => panic!("Expected validation error"),
        }
    }

    #[test]
    fn test_init_with_overwrite() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        // Create first time
        let options = InitOptions::new(data_dir.clone());
        init_database(&options).unwrap();

        // Create again with overwrite
        let options = InitOptions::new(data_dir.clone()).with_overwrite(true);
        let result = init_database(&options).unwrap();

        assert!(!result.data_dir_created); // Directory already existed
        assert!(result.database_created);
        assert!(data_dir.join("trop.db").exists());
    }

    #[test]
    fn test_plan_matches_the_live_run() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        // Fresh directory: create everything; `overwrite` changes nothing.
        for overwrite in [false, true] {
            let options = InitOptions::new(data_dir.clone())
                .with_overwrite(overwrite)
                .with_create_config(true);
            let plan = plan_init(&options).unwrap();
            assert!(plan.create_data_dir);
            assert!(
                !plan.replace_database,
                "a fresh directory is never replaced"
            );
            assert!(plan.create_config);
            assert!(!plan.config_exists);
        }
        let result =
            init_database(&InitOptions::new(data_dir.clone()).with_overwrite(true)).unwrap();
        assert!(result.data_dir_created);
        assert!(result.database_created);
        assert!(!result.database_replaced);

        // Existing database: the plan refuses exactly when the live run does,
        // with the same error.
        let options = InitOptions::new(data_dir.clone());
        let planned = plan_init(&options).unwrap_err().to_string();
        let live = init_database(&options).unwrap_err().to_string();
        assert_eq!(planned, live);
        assert!(planned.contains("already exists"), "{planned}");

        let options = InitOptions::new(data_dir.clone()).with_overwrite(true);
        let plan = plan_init(&options).unwrap();
        assert!(!plan.create_data_dir);
        assert!(plan.replace_database);
        assert!(!plan.create_config);
        let result = init_database(&options).unwrap();
        assert!(result.database_replaced);
        assert!(!result.data_dir_created);
    }

    /// Regression: `--overwrite` removed only `trop.db`, leaving stale
    /// `-wal`/`-shm` files beside the new database.
    #[test]
    fn test_overwrite_removes_database_side_files() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");
        init_database(&InitOptions::new(data_dir.clone())).unwrap();
        for side_file in DATABASE_SIDE_FILES {
            fs::write(data_dir.join(side_file), b"stale").unwrap();
        }

        init_database(&InitOptions::new(data_dir.clone()).with_overwrite(true)).unwrap();

        for side_file in DATABASE_SIDE_FILES {
            let path = data_dir.join(side_file);
            assert!(
                !path.exists() || fs::read(&path).unwrap() != b"stale",
                "stale {side_file} survived"
            );
        }
        // The recreated database is usable.
        let db = Database::open(DatabaseConfig::new(data_dir.join("trop.db"))).unwrap();
        drop(db);
    }

    /// The template documents real settings with their real defaults: the
    /// file as written is an empty configuration, and uncommenting every
    /// setting gives a valid user configuration with those defaults.
    #[test]
    fn test_default_config_template_uncomments_to_valid_defaults() {
        use crate::config::{ConfigBuilder, DEFAULT_MAX_PORT, DEFAULT_MIN_PORT};

        // Regression: the template documented a nonexistent `port_range` key
        // with a wrong 49152-65535 default, and a `reservations` example
        // that a user configuration may not contain.
        assert!(!DEFAULT_CONFIG_TEMPLATE.contains("port_range"));
        assert!(!DEFAULT_CONFIG_TEMPLATE.contains("49152"));
        assert!(!DEFAULT_CONFIG_TEMPLATE.contains("# reservations:"));
        assert!(DEFAULT_CONFIG_TEMPLATE.contains(&format!("#   min: {DEFAULT_MIN_PORT}")));
        assert!(DEFAULT_CONFIG_TEMPLATE.contains(&format!("#   max: {DEFAULT_MAX_PORT}")));

        // A commented setting line is "# " followed by optional indentation,
        // then a lowercase key and ':', or a "- " list item.
        let uncomment = |line: &str| -> Option<String> {
            let rest = line.strip_prefix("# ")?;
            let body = rest.trim_start();
            let is_setting = body.starts_with("- ")
                || body.split_once(':').is_some_and(|(key, _)| {
                    !key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                });
            is_setting.then(|| rest.to_string())
        };
        let uncommented: String = DEFAULT_CONFIG_TEMPLATE
            .lines()
            .map(|line| uncomment(line).unwrap_or_else(|| line.to_string()) + "\n")
            .collect();
        assert!(uncommented.contains("\nports:\n  min: "), "{uncommented}");
        assert!(uncommented.contains("\nmaximum_lock_wait_seconds: 5\n"));
        assert!(uncommented.contains("\noutput_format: table\n"));

        let build = |contents: &str| {
            let temp = TempDir::new().unwrap();
            fs::write(temp.path().join("config.yaml"), contents).unwrap();
            let work = TempDir::new().unwrap();
            ConfigBuilder::new()
                .with_data_dir(temp.path())
                .with_working_dir(work.path())
                .skip_env()
                .build_effective()
        };
        let defaults = build("").unwrap();
        let as_written = build(DEFAULT_CONFIG_TEMPLATE).unwrap();
        assert_eq!(as_written.config(), defaults.config());

        let effective = build(&uncommented).unwrap_or_else(|error| {
            panic!("uncommented template must load: {error}\n{uncommented}")
        });
        let ports = effective.ports();
        assert_eq!(ports.min, DEFAULT_MIN_PORT);
        assert_eq!(ports.max, Some(DEFAULT_MAX_PORT));
        assert_eq!(effective.maximum_lock_wait_seconds(), 5);
        assert_eq!(
            effective.config().excluded_ports.as_ref().map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn test_init_config_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let data_dir = temp.path().join("trop");

        // Create config file manually
        fs::create_dir_all(&data_dir).unwrap();
        let config_path = data_dir.join("config.yaml");
        fs::write(&config_path, "custom config").unwrap();

        // Initialize with create_config = true
        let options = InitOptions::new(data_dir.clone()).with_create_config(true);
        let result = init_database(&options).unwrap();

        assert!(!result.config_created); // Should not overwrite existing config

        // Verify config wasn't changed
        let config_content = fs::read_to_string(&config_path).unwrap();
        assert_eq!(config_content, "custom config");
    }
}
