//! Database connection management.
//!
//! This module provides the main database connection type with proper
//! initialization and PRAGMA settings for optimal `SQLite` configuration.

use rusqlite::{Connection, OpenFlags};

use crate::error::Result;

use super::config::DatabaseConfig;

/// A database connection wrapper with configuration.
///
/// This type manages a `SQLite` connection with appropriate PRAGMA settings
/// for concurrent access and performance.
///
/// # Examples
///
/// ```no_run
/// use trop::database::{Database, DatabaseConfig};
///
/// let config = DatabaseConfig::new("/tmp/trop.db");
/// let db = Database::open(config).unwrap();
/// ```
#[derive(Debug)]
pub struct Database {
    pub(super) conn: Connection,
    #[allow(dead_code)]
    config: DatabaseConfig,
}

impl Database {
    /// Opens a database connection with the given configuration.
    ///
    /// This function will:
    /// - Create the parent directory if `auto_create` is enabled
    /// - Open the database with appropriate flags
    /// - Set WAL mode for concurrent access
    /// - Configure busy timeout
    /// - Initialize or verify the database schema
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The database file cannot be opened
    /// - The parent directory cannot be created
    /// - PRAGMA settings cannot be applied
    /// - Schema initialization or verification fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    /// ```
    pub fn open(config: DatabaseConfig) -> Result<Self> {
        // Ensure parent directory exists if auto-creating
        if config.auto_create && !config.path.exists() {
            if let Some(parent) = config.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // Determine open flags based on configuration
        let flags = if config.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else if config.auto_create {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };

        // Open the connection
        let conn = Connection::open_with_flags(&config.path, flags)?;

        // Set pragmas for optimal operation
        // Note: PRAGMA journal_mode returns a result, so we use query_row
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL")?;
        conn.execute_batch(&format!(
            "PRAGMA busy_timeout = {}",
            config.busy_timeout.as_millis()
        ))?;

        // Check and initialize schema (will be implemented in migrations module)
        super::migrations::check_schema_compatibility(&conn)?;

        Ok(Self { conn, config })
    }

    /// Returns a reference to the underlying `SQLite` connection.
    ///
    /// This provides access to the raw connection for advanced operations.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    /// let conn = db.connection();
    /// ```
    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Returns a mutable reference to the underlying `SQLite` connection.
    ///
    /// This provides mutable access to the raw connection for operations
    /// that require mutability, such as transactions.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    /// let conn = db.connection_mut();
    /// ```
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_open() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let config = DatabaseConfig::new(&path);

        let db = Database::open(config).unwrap();
        assert!(path.exists());

        // Verify pragmas are set correctly
        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");
    }

    #[test]
    fn test_database_auto_create_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("subdir").join("test.db");
        let config = DatabaseConfig::new(&path);

        assert!(!path.parent().unwrap().exists());

        let _db = Database::open(config).unwrap();
        assert!(path.exists());
        assert!(path.parent().unwrap().exists());
    }

    #[test]
    fn test_database_read_only() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        // Create database first
        {
            let config = DatabaseConfig::new(&path);
            Database::open(config).unwrap();
        }

        // Open in read-only mode
        let config = DatabaseConfig::new(&path).read_only();
        let db = Database::open(config).unwrap();

        // Verify we can read but not write
        let result = db.connection().execute("CREATE TABLE test (id INTEGER)", []);
        assert!(result.is_err());
    }

    #[test]
    fn test_database_connection_accessors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let config = DatabaseConfig::new(&path);

        let mut db = Database::open(config).unwrap();

        // Test immutable accessor
        let _conn = db.connection();

        // Test mutable accessor
        let _conn_mut = db.connection_mut();
    }
}
