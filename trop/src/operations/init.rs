//! Database initialization operations.
//!
//! This module provides functionality for explicitly initializing the trop
//! data directory and database, with support for dry-run mode and optional
//! configuration file creation.

use std::fs;
use std::path::PathBuf;

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
}

impl InitOptions {
    /// Creates new initialization options.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            overwrite: false,
            create_config: false,
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
}

/// Result of initialization operation.
#[derive(Debug)]
pub struct InitResult {
    /// Whether the data directory was created.
    pub data_dir_created: bool,
    /// Whether the database was created or recreated.
    pub database_created: bool,
    /// Whether a configuration file was created.
    pub config_created: bool,
    /// Path to the data directory.
    pub data_dir: PathBuf,
}

/// Default minimal configuration template.
const DEFAULT_CONFIG_TEMPLATE: &str = r"# Trop Configuration File
# See documentation for available options

# Port allocation range (default: 49152-65535)
# port_range:
#   min: 49152
#   max: 65535

# Maximum lock wait time in seconds (default: 5)
# maximum_lock_wait_seconds: 5

# Group reservations (example)
# reservations:
#   path: .  # Base path for group reservations
#   services:
#     web:
#       port: 3000
#       env: PORT
#     api:
#       port: 3001
#       env: API_PORT
";

/// Initializes the trop data directory and database.
///
/// This function creates the data directory if needed, initializes the database,
/// and optionally creates a default configuration file.
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
    let mut result = InitResult {
        data_dir_created: false,
        database_created: false,
        config_created: false,
        data_dir: options.data_dir.clone(),
    };

    // 1. Create data directory if it doesn't exist
    if !options.data_dir.exists() {
        fs::create_dir_all(&options.data_dir)?;
        result.data_dir_created = true;
    }

    // 2. Determine database path
    let db_path = options.data_dir.join("trop.db");

    // 3. Check if database already exists
    let db_exists = db_path.exists();

    if db_exists && !options.overwrite {
        return Err(Error::Validation {
            field: "database".into(),
            message: format!(
                "Database already exists at {}. Use --overwrite to replace it.",
                db_path.display()
            ),
        });
    }

    // 4. Remove existing database if overwriting
    if db_exists && options.overwrite {
        fs::remove_file(&db_path)?;
    }

    // 5. Initialize database (this will create schema)
    let db_config = DatabaseConfig::new(&db_path);
    let mut _db = Database::open(db_config)?;
    result.database_created = true;

    // 6. Optionally create default configuration file
    if options.create_config {
        let config_path = options.data_dir.join("config.yaml");

        // Only create if it doesn't exist
        if !config_path.exists() {
            fs::write(&config_path, DEFAULT_CONFIG_TEMPLATE)?;
            result.config_created = true;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        assert!(config_content.contains("Trop Configuration File"));
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
