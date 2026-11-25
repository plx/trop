//! Database connection management.
//!
//! This module provides the main database connection type with proper
//! initialization and PRAGMA settings for optimal `SQLite` configuration.

use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::error::{sqlite_error_is_lock_contention, Error, Result};
use crate::verified::wal::{enable_wal, WalAnswer, WalOutcome};

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

/// Switches the connection to WAL journal mode (verified kernel
/// `enable_wal`): `PRAGMA journal_mode = WAL` is retried only on lock
/// contention and only within `timeout`, each attempt runs with the
/// remaining time as its busy timeout, pauses last at most 10 ms, and the
/// busy timeout is restored to `timeout` on every exit. The journal mode
/// SQLite reports must be `wal` (`memory` is accepted for an in-memory
/// database, which cannot use WAL and cannot be shared).
fn enable_wal_with_retry(conn: &Connection, timeout: Duration) -> rusqlite::Result<()> {
    let started = Instant::now();
    // An in-memory (or temporary) database has no file name.
    let in_memory = conn.path().map_or(true, str::is_empty);
    let (outcome, _) = enable_wal(
        timeout.as_nanos(),
        in_memory,
        &|| started.elapsed().as_nanos(),
        &|millis| conn.execute_batch(&format!("PRAGMA busy_timeout = {millis}")),
        &|| match conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(mode) => WalAnswer::Mode(mode.into_bytes()),
            // Simultaneous journal-mode upgrades can deadlock while both
            // connections hold read locks, causing SQLite to bypass its busy
            // handler, so contention is retried here within the timeout.
            Err(error) if sqlite_error_is_lock_contention(&error) => WalAnswer::Contention(error),
            Err(error) => WalAnswer::Failed(error),
        },
        &|nanos| std::thread::sleep(Duration::from_nanos(nanos)),
    );
    match outcome {
        WalOutcome::Enabled => Ok(()),
        WalOutcome::NotWal(mode) => Err(journal_mode_not_wal(&mode)),
        WalOutcome::TimedOut(error)
        | WalOutcome::Failed(error)
        | WalOutcome::BusyTimeoutFailed(error)
        | WalOutcome::RestoreFailed(error) => Err(error),
    }
}

/// The error for a database that stayed in another journal mode: trop needs
/// WAL for safe concurrent access.
fn journal_mode_not_wal(mode: &[u8]) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(format!(
            "cannot enable WAL journal mode: SQLite kept journal mode \"{}\"; \
             trop requires WAL for concurrent access to an on-disk database",
            String::from_utf8_lossy(mode)
        )),
    )
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

    #[test]
    fn database_that_cannot_use_wal_is_rejected_instead_of_silently_continuing() {
        // SQLite keeps a temporary database (empty file name) in DELETE mode
        // and reports "delete" from `PRAGMA journal_mode = WAL`. Before the
        // reported mode was checked, the switch counted as a success.
        let conn = Connection::open("").unwrap();
        let error = enable_wal_with_retry(&conn, Duration::from_secs(1)).unwrap_err();
        assert!(
            error.to_string().contains("journal mode \"delete\""),
            "{error}"
        );
        // The busy timeout is restored even though the switch failed.
        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout, 1000);

        let error = Database::open(DatabaseConfig::new("")).unwrap_err();
        assert!(
            matches!(error, Error::Database(ref sqlite) if sqlite.to_string().contains("WAL")),
            "{error:?}"
        );
    }

    #[test]
    fn in_memory_database_keeps_memory_journal_mode() {
        let db = Database::open(DatabaseConfig::new(":memory:")).unwrap();
        let journal_mode: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "memory");
    }

    #[test]
    fn file_database_reports_wal_after_enabling() {
        let dir = tempdir().unwrap();
        let conn = Connection::open(dir.path().join("wal.db")).unwrap();
        enable_wal_with_retry(&conn, Duration::from_millis(250)).unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "wal");
        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout, 250);
    }

    /// Drives the verified retry loop with scripted oracles and records the
    /// effects, as the glue's closures would perform them.
    fn scripted_run(
        timeout_nanos: u128,
        answers: &[&str],
        clock_step: u128,
    ) -> (WalOutcome<String>, Vec<String>) {
        use std::cell::{Cell, RefCell};
        let effects = RefCell::new(Vec::new());
        let now = Cell::new(0_u128);
        let next = Cell::new(0_usize);
        let (outcome, _) = enable_wal(
            timeout_nanos,
            false,
            &|| {
                let reading = now.get();
                now.set(reading + clock_step);
                reading
            },
            &|millis| {
                effects.borrow_mut().push(format!("busy {millis}"));
                Ok(())
            },
            &|| {
                let answer = answers[next.get().min(answers.len() - 1)];
                next.set(next.get() + 1);
                effects.borrow_mut().push(format!("attempt {answer}"));
                match answer {
                    "busy" => WalAnswer::Contention("busy".to_string()),
                    "error" => WalAnswer::Failed("error".to_string()),
                    mode => WalAnswer::Mode(mode.as_bytes().to_vec()),
                }
            },
            &|nanos| {
                effects.borrow_mut().push(format!("pause {nanos}"));
                now.set(now.get() + u128::from(nanos));
            },
        );
        (outcome, effects.into_inner())
    }

    #[test]
    fn wal_retry_pauses_at_most_ten_milliseconds_and_restores_the_timeout_last() {
        // 25 ms budget, a clock that advances 1 ms per reading.
        let (outcome, effects) = scripted_run(25_000_000, &["busy", "busy", "WAL"], 1_000_000);
        assert!(matches!(outcome, WalOutcome::Enabled));
        assert_eq!(
            effects,
            [
                "busy 25",
                "attempt busy",
                "pause 10000000",
                "busy 13",
                "attempt busy",
                "pause 10000000",
                "busy 1",
                "attempt WAL",
                "busy 25",
            ]
        );
    }

    #[test]
    fn wal_retry_gives_up_at_the_timeout_and_still_restores() {
        let (outcome, effects) = scripted_run(12_000_000, &["busy"], 1_000_000);
        assert!(matches!(outcome, WalOutcome::TimedOut(ref e) if e == "busy"));
        assert_eq!(
            effects,
            [
                "busy 12",
                "attempt busy",
                "pause 10000000",
                "busy 0",
                "attempt busy",
                "busy 12",
            ]
        );
        // A zero timeout never pauses.
        let (outcome, effects) = scripted_run(0, &["busy"], 0);
        assert!(matches!(outcome, WalOutcome::TimedOut(_)));
        assert_eq!(effects, ["busy 0", "attempt busy", "busy 0"]);
    }

    #[test]
    fn wal_retry_stops_at_the_first_other_error_or_mode() {
        let (outcome, effects) = scripted_run(25_000_000, &["error"], 0);
        assert!(matches!(outcome, WalOutcome::Failed(ref e) if e == "error"));
        assert_eq!(effects, ["busy 25", "attempt error", "busy 25"]);
        let (outcome, _) = scripted_run(25_000_000, &["delete"], 0);
        assert!(matches!(outcome, WalOutcome::NotWal(ref m) if m == b"delete"));
        let (outcome, _) = scripted_run(25_000_000, &["memory"], 0);
        assert!(matches!(outcome, WalOutcome::NotWal(_)));
    }
}
