//! Database connection management.
//!
//! This module provides the main database connection type with proper
//! initialization and PRAGMA settings for optimal `SQLite` configuration.

use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{sqlite_error_is_lock_contention, Error, Result};

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
    /// Opens an existing database read-only and validates its physical,
    /// schema, and logical integrity.
    ///
    /// This entry point never initializes or migrates a database and never
    /// changes its journal mode.
    ///
    /// # Errors
    ///
    /// Returns a typed compatibility, corruption, I/O, or database error.
    pub fn validate(config: &DatabaseConfig) -> Result<()> {
        let timeout = config.busy_timeout;
        config.validate_busy_timeout()?;
        (|| {
            let conn = Connection::open_with_flags(
                &config.path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(timeout)?;
            super::validation::validate_current_database(&conn)
        })()
        .map_err(|error: Error| {
            error.classify_sqlite_lock(timeout, "opening and validating the database")
        })
    }

    /// Opens a database connection with the given configuration.
    ///
    /// This function will:
    /// - Create the parent directory if `auto_create` is enabled
    /// - Open the database with appropriate flags
    /// - Configure busy timeout
    /// - Set WAL mode for concurrent access
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
        let timeout = config.busy_timeout;
        config.validate_busy_timeout()?;
        Self::open_inner(config).map_err(|error| {
            error.classify_sqlite_lock(timeout, "opening, configuring, or migrating the database")
        })
    }

    fn open_inner(config: DatabaseConfig) -> Result<Self> {
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

        // Set pragmas for optimal operation (skip for read-only databases)
        if config.read_only {
            conn.busy_timeout(config.busy_timeout)?;
        } else {
            enable_wal_with_retry(&conn, config.busy_timeout)?;
            conn.execute_batch("PRAGMA synchronous = NORMAL")?;
        }

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

    /// Begins an IMMEDIATE transaction for atomic operations.
    ///
    /// IMMEDIATE transactions acquire a write lock immediately, preventing
    /// other writers from starting. This ensures serialized execution and
    /// prevents race conditions in port allocation.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction cannot be started.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    /// let tx = db.begin_transaction().unwrap();
    /// // Perform operations...
    /// tx.commit().unwrap();
    /// ```
    pub fn begin_transaction(&mut self) -> Result<Transaction<'_>> {
        let timeout = self.config.busy_timeout;
        self.conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(Error::from)
            .map_err(|error| {
                error.classify_sqlite_lock(timeout, "starting an immediate database transaction")
            })
    }

    /// Returns the configured maximum wait for `SQLite` lock contention.
    #[must_use]
    pub const fn busy_timeout(&self) -> Duration {
        self.config.busy_timeout
    }
}

fn enable_wal_with_retry(conn: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    let result = enable_wal_with_retry_inner(conn, timeout);
    let restore_result = set_busy_timeout(conn, timeout);

    match (result, restore_result) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn enable_wal_with_retry_inner(conn: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        set_busy_timeout(conn, remaining)?;

        match conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(_) => return Ok(()),
            Err(error) if sqlite_error_is_lock_contention(&error) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(error);
                }

                // Simultaneous journal-mode upgrades can deadlock while both
                // connections hold read locks, causing SQLite to bypass its
                // busy handler. Drop the completed statement, yield briefly,
                // and retry within the caller's configured timeout.
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(error) => return Err(error),
        }
    }
}

fn set_busy_timeout(conn: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    conn.execute_batch(&format!("PRAGMA busy_timeout = {}", timeout.as_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Barrier};
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
        let busy_timeout: i64 = db
            .connection()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout, 5000);
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
    fn database_open_rejects_timeout_above_sqlite_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        let timeout = Duration::from_millis(i32::MAX as u64 + 1);

        let error =
            Database::open(DatabaseConfig::new(&path).with_busy_timeout(timeout)).unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                ref field,
                ref message
            } if field == "maximum_lock_wait_seconds"
                && message.contains("2147483647 milliseconds")
        ));
        assert!(
            !path.exists(),
            "invalid timeout must be rejected before creating a database"
        );
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
        let result = db
            .connection()
            .execute("CREATE TABLE test (id INTEGER)", []);
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

    #[test]
    fn database_open_maps_wal_configuration_contention_to_lock_timeout() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        drop(Database::open(DatabaseConfig::new(&path)).unwrap());

        let locker = Connection::open(&path).unwrap();
        locker
            .pragma_update(None, "journal_mode", "DELETE")
            .unwrap();
        locker.execute_batch("BEGIN IMMEDIATE").unwrap();

        let error = Database::open(DatabaseConfig::new(&path).with_busy_timeout(Duration::ZERO))
            .unwrap_err();
        assert!(matches!(
            error,
            Error::LockTimeout {
                timeout,
                ref operation
            } if timeout == Duration::ZERO
                && operation == "opening, configuring, or migrating the database"
        ));

        locker.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn begin_transaction_maps_expired_contention_to_lock_timeout() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        drop(Database::open(DatabaseConfig::new(&path)).unwrap());

        let locker = Connection::open(&path).unwrap();
        locker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut database =
            Database::open(DatabaseConfig::new(&path).with_busy_timeout(Duration::ZERO)).unwrap();

        let error = database.begin_transaction().unwrap_err();
        assert!(matches!(
            error,
            Error::LockTimeout {
                timeout,
                ref operation
            } if timeout == Duration::ZERO
                && operation == "starting an immediate database transaction"
        ));

        locker.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn transaction_acquired_before_timeout_succeeds_without_sleeping() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        drop(Database::open(DatabaseConfig::new(&path)).unwrap());

        let locker = Connection::open(&path).unwrap();
        locker.execute_batch("BEGIN IMMEDIATE").unwrap();

        let started = Arc::new(Barrier::new(2));
        let contender_started = Arc::clone(&started);
        let contender_path = path.clone();
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let contender = std::thread::spawn(move || {
            let mut database = Database::open(
                DatabaseConfig::new(contender_path).with_busy_timeout(Duration::from_secs(2)),
            )
            .unwrap();
            contender_started.wait();
            let result = database
                .begin_transaction()
                .and_then(|transaction| transaction.commit().map_err(Error::from));
            result_sender.send(result).unwrap();
        });

        started.wait();
        assert!(
            result_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "contender should remain blocked while the write lock is held"
        );
        locker.execute_batch("ROLLBACK").unwrap();

        result_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("contender did not finish after lock release")
            .unwrap();
        contender.join().unwrap();
    }

    #[test]
    fn malformed_database_is_not_misclassified_as_lock_timeout() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("malformed.db");
        std::fs::write(&path, b"not a sqlite database").unwrap();

        let error = Database::open(DatabaseConfig::new(&path).with_busy_timeout(Duration::ZERO))
            .unwrap_err();
        assert!(matches!(error, Error::Database(_)));
    }
}
