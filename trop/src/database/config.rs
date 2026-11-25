//! Database configuration and connection parameters.
//!
//! This module provides configuration types for database connections,
//! including path resolution and connection parameters.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};

/// `SQLite`'s busy-timeout limit (`i32::MAX` milliseconds), from the verified
/// configuration kernel.
pub(crate) const MAX_BUSY_TIMEOUT_MILLISECONDS: u64 =
    crate::verified::config::MAX_BUSY_TIMEOUT_MILLIS;
/// The largest whole number of seconds within the limit (derived from it in
/// the verified kernel, which proves it tight).
pub(crate) const MAX_BUSY_TIMEOUT_SECONDS: u64 = crate::verified::config::MAX_LOCK_WAIT_SECONDS;

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
///     .with_busy_timeout(Duration::from_secs(10));
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
    /// let normalized = config
    ///     .path
    ///     .to_string_lossy()
    ///     .replace(std::path::MAIN_SEPARATOR, "/");
    /// assert_eq!(normalized, "/tmp/trop.db");
    /// ```
    #[must_use]
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout: Duration::from_secs(5),
            auto_create: true,
            read_only: false,
        }
    }

    /// Sets the busy timeout duration.
    ///
    /// The busy timeout determines how long the database connection will
    /// wait when encountering a locked database before returning an error.
    /// [`Duration::ZERO`] means do not wait. Values above
    /// 2,147,483,647 milliseconds are rejected when opening the database
    /// because `SQLite` stores this setting as a signed 32-bit millisecond
    /// count.
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

    /// Reject a busy timeout `SQLite` cannot store (more than `i32::MAX`
    /// milliseconds, as `Duration::as_millis` counts them). Decided by the
    /// verified kernel's `busy_timeout_ok` over the duration's parts, which
    /// also proves every configuration-validated `maximum_lock_wait_seconds`
    /// passes.
    pub(super) fn validate_busy_timeout(&self) -> Result<()> {
        if !crate::verified::config::busy_timeout_ok(
            self.busy_timeout.as_secs(),
            self.busy_timeout.subsec_nanos(),
        ) {
            return Err(Error::Validation {
                field: "maximum_lock_wait_seconds".into(),
                message: format!(
                    "Timeout must not exceed {MAX_BUSY_TIMEOUT_MILLISECONDS} milliseconds"
                ),
            });
        }
        Ok(())
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
/// Resolution order:
/// 1. `$TROP_DATA_DIR` if set
/// 2. `~/.trop` on Unix-like systems or `%USERPROFILE%\.trop` on Windows
///
/// This directory contains the database file, config file, and other
/// persistent data.
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined and
/// `TROP_DATA_DIR` is not set.
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
    // Check TROP_DATA_DIR first
    if let Ok(data_dir) = std::env::var("TROP_DATA_DIR") {
        return Ok(PathBuf::from(data_dir));
    }

    // Fall back to ~/.trop
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| Error::Validation {
            field: "home_directory".into(),
            message: "Cannot determine home directory".into(),
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
    use serial_test::serial;

    #[test]
    fn test_config_new() {
        let config = DatabaseConfig::new("/tmp/test.db");
        assert_eq!(config.path, PathBuf::from("/tmp/test.db"));
        assert_eq!(config.busy_timeout, Duration::from_secs(5));
        assert!(config.auto_create);
        assert!(!config.read_only);
    }

    #[test]
    fn test_config_with_busy_timeout() {
        let config = DatabaseConfig::new("/tmp/test.db").with_busy_timeout(Duration::from_secs(10));
        assert_eq!(config.busy_timeout, Duration::from_secs(10));
    }

    #[test]
    fn test_config_accepts_zero_busy_timeout_as_no_wait() {
        let config = DatabaseConfig::new("/tmp/test.db").with_busy_timeout(Duration::ZERO);
        config.validate_busy_timeout().unwrap();
    }

    #[test]
    fn test_config_rejects_busy_timeout_larger_than_sqlite_limit() {
        let config = DatabaseConfig::new("/tmp/test.db")
            .with_busy_timeout(Duration::from_millis(MAX_BUSY_TIMEOUT_MILLISECONDS + 1));
        let error = config.validate_busy_timeout().unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                ref field,
                ref message
            } if field == "maximum_lock_wait_seconds"
                && message.contains(&MAX_BUSY_TIMEOUT_MILLISECONDS.to_string())
        ));
    }

    #[test]
    fn test_busy_timeout_limit_is_exact_at_millisecond_resolution() {
        let accepts = |timeout: Duration| {
            DatabaseConfig::new("/tmp/test.db")
                .with_busy_timeout(timeout)
                .validate_busy_timeout()
                .is_ok()
        };
        assert_eq!(MAX_BUSY_TIMEOUT_MILLISECONDS, 2_147_483_647);
        assert_eq!(MAX_BUSY_TIMEOUT_SECONDS, 2_147_483);
        assert!(accepts(Duration::from_millis(
            MAX_BUSY_TIMEOUT_MILLISECONDS
        )));
        // Sub-millisecond remainders do not count (`as_millis` truncates).
        assert!(accepts(
            Duration::from_millis(MAX_BUSY_TIMEOUT_MILLISECONDS) + Duration::from_nanos(999_999)
        ));
        assert!(!accepts(Duration::from_millis(
            MAX_BUSY_TIMEOUT_MILLISECONDS + 1
        )));
        assert!(!accepts(Duration::MAX));
        assert!(!accepts(Duration::from_secs(u64::MAX / 1000 + 1)));
        // Every whole-second lock wait the configuration accepts is accepted
        // here, and the next second is not.
        assert!(accepts(Duration::from_secs(MAX_BUSY_TIMEOUT_SECONDS)));
        assert!(!accepts(Duration::from_secs(MAX_BUSY_TIMEOUT_SECONDS + 1)));
        assert!(accepts(Duration::ZERO));
        assert!(accepts(Duration::from_nanos(1)));
    }

    #[test]
    fn test_config_read_only() {
        let config = DatabaseConfig::new("/tmp/test.db").read_only();
        assert!(config.read_only);
        assert!(!config.auto_create);
    }

    #[test]
    #[serial]
    fn test_default_data_dir() {
        // This test requires HOME or USERPROFILE to be set
        std::env::remove_var("TROP_DATA_DIR");
        let result = default_data_dir();
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            let dir = result.unwrap();
            assert!(dir.ends_with(".trop"));
        }
    }

    #[test]
    #[serial]
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

    #[test]
    #[serial]
    fn test_data_dir_consistency() {
        // Test that database and config paths use the same data directory

        // Test with custom TROP_DATA_DIR
        std::env::remove_var("TROP_DATA_DIR");
        std::env::set_var("TROP_DATA_DIR", "/custom/trop/data");

        let db_path = resolve_database_path().unwrap();
        let data_dir = default_data_dir().unwrap();
        let config_path = data_dir.join("config.yaml");

        // Both should be in /custom/trop/data
        assert_eq!(db_path, PathBuf::from("/custom/trop/data/trop.db"));
        assert_eq!(config_path, PathBuf::from("/custom/trop/data/config.yaml"));
        assert_eq!(db_path.parent(), config_path.parent());

        // Clean up
        std::env::remove_var("TROP_DATA_DIR");

        // Test with default (HOME-based)
        std::env::remove_var("TROP_DATA_DIR");
        if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
            let db_path = resolve_database_path().unwrap();
            let data_dir = default_data_dir().unwrap();
            let config_path = data_dir.join("config.yaml");

            // Both should be in the same directory
            assert_eq!(db_path.parent(), config_path.parent());
        }
    }
}
