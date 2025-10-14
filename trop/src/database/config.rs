//! Database configuration and connection parameters.
//!
//! This module provides configuration types for database connections,
//! including path resolution and connection parameters.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};

/// Configuration for database connections.
///
/// This struct contains all parameters needed to open and configure
/// a database connection, including the database file path, timeout
/// settings, and access modes.
///
/// # Examples
///
/// ```
/// use trop::database::DatabaseConfig;
/// use std::time::Duration;
///
/// // Create a configuration with default settings
/// let config = DatabaseConfig::new("/tmp/trop.db");
///
/// // Customize the configuration
/// let config = DatabaseConfig::new("/tmp/trop.db")
///     .with_busy_timeout(Duration::from_millis(10000));
/// ```
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Path to the database file.
    pub path: PathBuf,
    /// Busy timeout for database lock contention.
    pub busy_timeout: Duration,
    /// Whether to automatically create the database if it doesn't exist.
    pub auto_create: bool,
    /// Whether to open the database in read-only mode.
    pub read_only: bool,
}

impl DatabaseConfig {
    /// Creates a new database configuration with default settings.
    ///
    /// Default settings:
    /// - `busy_timeout`: 5000ms
    /// - `auto_create`: true
    /// - `read_only`: false
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::database::DatabaseConfig;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// assert_eq!(config.path.to_str().unwrap(), "/tmp/trop.db");
    /// ```
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout: Duration::from_millis(5000),
            auto_create: true,
            read_only: false,
        }
    }

    /// Sets the busy timeout duration.
    ///
    /// The busy timeout determines how long the database connection will
    /// wait when encountering a locked database before returning an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::database::DatabaseConfig;
    /// use std::time::Duration;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db")
    ///     .with_busy_timeout(Duration::from_secs(10));
    /// ```
    #[must_use]
    pub fn with_busy_timeout(mut self, timeout: Duration) -> Self {
        self.busy_timeout = timeout;
        self
    }

    /// Configures the database to be opened in read-only mode.
    ///
    /// When read-only is enabled, `auto_create` is automatically disabled.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::database::DatabaseConfig;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db").read_only();
    /// assert!(config.read_only);
    /// assert!(!config.auto_create);
    /// ```
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.auto_create = false;
        self
    }
}

/// Returns the default data directory for trop.
///
/// The default directory is `~/.trop` on Unix-like systems and
/// `%USERPROFILE%\.trop` on Windows.
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined.
///
/// # Examples
///
/// ```no_run
/// use trop::database::default_data_dir;
///
/// let data_dir = default_data_dir().unwrap();
/// println!("Data directory: {}", data_dir.display());
/// ```
pub fn default_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| {
            Error::Validation {
                field: "home_directory".into(),
                message: "Cannot determine home directory".into(),
            }
        })?;
    Ok(PathBuf::from(home).join(".trop"))
}

/// Resolves the database path using environment variables or defaults.
///
/// The resolution order is:
/// 1. `$TROP_DATA_DIR/trop.db` if the `TROP_DATA_DIR` environment variable is set
/// 2. `~/.trop/trop.db` otherwise
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined and
/// `TROP_DATA_DIR` is not set.
///
/// # Examples
///
/// ```no_run
/// use trop::database::resolve_database_path;
///
/// let db_path = resolve_database_path().unwrap();
/// println!("Database path: {}", db_path.display());
/// ```
pub fn resolve_database_path() -> Result<PathBuf> {
    if let Ok(data_dir) = std::env::var("TROP_DATA_DIR") {
        Ok(PathBuf::from(data_dir).join("trop.db"))
    } else {
        Ok(default_data_dir()?.join("trop.db"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = DatabaseConfig::new("/tmp/test.db");
        assert_eq!(config.path, PathBuf::from("/tmp/test.db"));
        assert_eq!(config.busy_timeout, Duration::from_millis(5000));
        assert!(config.auto_create);
        assert!(!config.read_only);
    }

    #[test]
    fn test_config_with_busy_timeout() {
        let config = DatabaseConfig::new("/tmp/test.db")
            .with_busy_timeout(Duration::from_millis(10000));
        assert_eq!(config.busy_timeout, Duration::from_millis(10000));
    }

    #[test]
    fn test_config_read_only() {
        let config = DatabaseConfig::new("/tmp/test.db").read_only();
        assert!(config.read_only);
        assert!(!config.auto_create);
    }

    #[test]
    fn test_default_data_dir() {
        // This test requires HOME or USERPROFILE to be set
        let result = default_data_dir();
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            let dir = result.unwrap();
            assert!(dir.ends_with(".trop"));
        }
    }

    #[test]
    fn test_resolve_database_path() {
        // Test with default (no TROP_DATA_DIR set)
        std::env::remove_var("TROP_DATA_DIR");
        let result = resolve_database_path();
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            let path = result.unwrap();
            assert!(path.ends_with("trop.db"));
        }

        // Test with TROP_DATA_DIR set
        std::env::set_var("TROP_DATA_DIR", "/custom/data");
        let path = resolve_database_path().unwrap();
        assert_eq!(path, PathBuf::from("/custom/data/trop.db"));

        // Clean up
        std::env::remove_var("TROP_DATA_DIR");
    }
}
