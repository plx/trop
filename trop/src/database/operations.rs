//! Database CRUD operations for reservations.
//!
//! This module implements all create, read, update, and delete operations
//! for port reservations in the database.

use std::env;
use std::path::Path;
use std::time::{Duration, SystemTime};

use rusqlite::{params, Connection, TransactionBehavior};

use crate::error::{Error, Result};
use crate::path::PathRelationship;
use crate::{Port, PortRange, Reservation, ReservationKey};

use super::connection::Database;
use super::schema::{DELETE_RESERVATION, INSERT_RESERVATION};

/// Converts a `SystemTime` to Unix epoch seconds for database storage.
///
/// # Errors
///
/// Returns an error if the time is before the Unix epoch.
#[allow(clippy::cast_possible_wrap)]
pub(super) fn systemtime_to_unix_secs(time: SystemTime) -> Result<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| crate::error::Error::Validation {
            field: "timestamp".into(),
            message: format!("Invalid timestamp: {e}"),
        })
        .map(|d| d.as_secs() as i64)
}

/// Converts Unix epoch seconds from the database to a `SystemTime`.
#[allow(clippy::cast_sign_loss)]
pub(super) fn unix_secs_to_systemtime(secs: i64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64)
}

/// Helper function to deserialize a reservation from a database row.
///
/// Expects row fields in this order: path, tag, port, project, task, `created_at`, `last_used_at`
fn row_to_reservation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reservation> {
    let path: String = row.get(0)?;
    let tag: Option<String> = row.get(1)?;
    let port_value: u16 = row.get(2)?;
    let project: Option<String> = row.get(3)?;
    let task: Option<String> = row.get(4)?;
    let created_secs: i64 = row.get(5)?;
    let last_used_secs: i64 = row.get(6)?;

    let key = ReservationKey::new(path.into(), tag)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let port = Port::try_from(port_value)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

    let created_at = unix_secs_to_systemtime(created_secs);
    let last_used_at = unix_secs_to_systemtime(last_used_secs);

    Reservation::builder(key, port)
        .project(project)
        .task(task)
        .created_at(created_at)
        .last_used_at(last_used_at)
        .build()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

// SQL statements for CRUD operations
const SELECT_RESERVATION: &str = r"
    SELECT port, project, task, created_at, last_used_at
    FROM reservations
    WHERE path = ? AND tag IS ?
";

const UPDATE_LAST_USED: &str = r"
    UPDATE reservations
    SET last_used_at = ?
    WHERE path = ? AND tag IS ?
";

const LIST_RESERVATIONS: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    ORDER BY path, tag
";

const SELECT_RESERVED_PORTS: &str = r"
    SELECT DISTINCT port
    FROM reservations
    WHERE port >= ? AND port <= ?
    ORDER BY port
";

const SELECT_BY_PATH_PREFIX: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE path LIKE ? || '%'
    ORDER BY path, tag
";

const SELECT_EXPIRED: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE last_used_at < ?
    ORDER BY last_used_at
";

const CHECK_PORT_RESERVED: &str = r"
    SELECT COUNT(*) FROM reservations WHERE port = ?
";

const SELECT_BY_PORT: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE port = ?
";

impl Database {
    /// Creates or updates a reservation in the database.
    ///
    /// This operation uses a transaction with IMMEDIATE mode to ensure
    /// atomicity and prevent conflicts.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transaction cannot be started
    /// - The insert fails
    /// - The transaction cannot be committed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Reservation, ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    ///
    /// db.create_reservation(&reservation).unwrap();
    /// ```
    pub fn create_reservation(&mut self, reservation: &Reservation) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // For NULL tags, explicitly delete first to ensure replacement works
        // (INSERT OR REPLACE doesn't work with NULL in PRIMARY KEY due to NULL != NULL)
        tx.execute(
            DELETE_RESERVATION,
            params![reservation.key().path_as_string(), reservation.key().tag],
        )?;

        let created_secs = systemtime_to_unix_secs(reservation.created_at())?;
        let last_used_secs = systemtime_to_unix_secs(reservation.last_used_at())?;

        tx.execute(
            INSERT_RESERVATION,
            params![
                reservation.key().path_as_string(),
                reservation.key().tag,
                reservation.port().value(),
                reservation.project(),
                reservation.task(),
                created_secs,
                last_used_secs,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Creates or updates a reservation using an existing connection or transaction.
    ///
    /// This method is intended for use within an existing transaction context.
    /// Unlike `create_reservation`, it does not create its own transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Reservation, ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    ///
    /// let tx = db.begin_transaction().unwrap();
    /// Database::create_reservation_simple(&tx, &reservation).unwrap();
    /// tx.commit().unwrap();
    /// ```
    pub fn create_reservation_simple(conn: &Connection, reservation: &Reservation) -> Result<()> {
        // For NULL tags, explicitly delete first to ensure replacement works
        // (INSERT OR REPLACE doesn't work with NULL in PRIMARY KEY due to NULL != NULL)
        conn.execute(
            DELETE_RESERVATION,
            params![reservation.key().path_as_string(), reservation.key().tag],
        )?;

        let created_secs = systemtime_to_unix_secs(reservation.created_at())?;
        let last_used_secs = systemtime_to_unix_secs(reservation.last_used_at())?;

        conn.execute(
            INSERT_RESERVATION,
            params![
                reservation.key().path_as_string(),
                reservation.key().tag,
                reservation.port().value(),
                reservation.project(),
                reservation.task(),
                created_secs,
                last_used_secs,
            ],
        )?;

        Ok(())
    }

    /// Retrieves a reservation from the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails (other than "not found").
    ///
    /// # Returns
    ///
    /// - `Ok(Some(reservation))` if the reservation exists
    /// - `Ok(None)` if the reservation doesn't exist
    /// - `Err(_)` if a database error occurs
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    /// ```
    pub fn get_reservation(conn: &Connection, key: &ReservationKey) -> Result<Option<Reservation>> {
        let mut stmt = conn.prepare(SELECT_RESERVATION)?;

        match stmt.query_row(params![key.path_as_string(), key.tag], |row| {
            let port_value: u16 = row.get(0)?;
            let port = Port::try_from(port_value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            let project: Option<String> = row.get(1)?;
            let task: Option<String> = row.get(2)?;
            let created_secs: i64 = row.get(3)?;
            let last_used_secs: i64 = row.get(4)?;

            let created_at = unix_secs_to_systemtime(created_secs);
            let last_used_at = unix_secs_to_systemtime(last_used_secs);

            Reservation::builder(key.clone(), port)
                .project(project)
                .task(task)
                .created_at(created_at)
                .last_used_at(last_used_at)
                .build()
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        }) {
            Ok(reservation) => Ok(Some(reservation)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Updates the `last_used_at` timestamp for a reservation.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction or update fails.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the reservation was found and updated
    /// - `Ok(false)` if the reservation was not found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let updated = db.update_last_used(&key).unwrap();
    /// ```
    pub fn update_last_used(&mut self, key: &ReservationKey) -> Result<bool> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let now = systemtime_to_unix_secs(SystemTime::now())?;

        let rows_affected = tx.execute(
            UPDATE_LAST_USED,
            params![now, key.path_as_string(), key.tag],
        )?;

        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// Deletes a reservation from the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction or delete fails.
    ///
    /// # Returns
    ///
    /// - `Ok(true)` if the reservation was found and deleted
    /// - `Ok(false)` if the reservation was not found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let deleted = db.delete_reservation(&key).unwrap();
    /// ```
    pub fn delete_reservation(&mut self, key: &ReservationKey) -> Result<bool> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let rows_affected =
            tx.execute(DELETE_RESERVATION, params![key.path_as_string(), key.tag])?;

        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// Updates the last used timestamp for a reservation (without creating a transaction).
    ///
    /// This method is intended for use within an existing transaction.
    /// For standalone use, use `update_last_used` instead.
    ///
    /// # Errors
    ///
    /// Returns an error if the timestamp conversion fails or the database update fails.
    pub fn update_last_used_simple(conn: &Connection, key: &ReservationKey) -> Result<bool> {
        let now = systemtime_to_unix_secs(SystemTime::now())?;
        let rows_affected = conn.execute(
            UPDATE_LAST_USED,
            params![now, key.path_as_string(), key.tag],
        )?;
        Ok(rows_affected > 0)
    }

    /// Deletes a reservation from the database (without creating a transaction).
    ///
    /// This method is intended for use within an existing transaction.
    /// For standalone use, use `delete_reservation` instead.
    ///
    /// # Errors
    ///
    /// Returns an error if the database deletion fails.
    pub fn delete_reservation_simple(conn: &Connection, key: &ReservationKey) -> Result<bool> {
        let rows_affected =
            conn.execute(DELETE_RESERVATION, params![key.path_as_string(), key.tag])?;
        Ok(rows_affected > 0)
    }

    /// Lists all reservations in the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails or if any reservation
    /// cannot be deserialized.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let reservations = Database::list_all_reservations(db.connection()).unwrap();
    /// for reservation in reservations {
    ///     println!("{:?}", reservation);
    /// }
    /// ```
    pub fn list_all_reservations(conn: &Connection) -> Result<Vec<Reservation>> {
        let mut stmt = conn.prepare(LIST_RESERVATIONS)?;

        let reservations = stmt
            .query_map([], row_to_reservation)?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(reservations)
    }

    /// Gets all reserved ports within a given range.
    ///
    /// This query is useful for finding which ports in a range are
    /// already allocated, which can help with port selection algorithms.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Port, PortRange};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5100).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    ///
    /// let reserved = Database::get_reserved_ports(db.connection(), &range).unwrap();
    /// ```
    pub fn get_reserved_ports(conn: &Connection, range: &PortRange) -> Result<Vec<Port>> {
        let mut stmt = conn.prepare(SELECT_RESERVED_PORTS)?;

        let ports = stmt
            .query_map(params![range.min().value(), range.max().value()], |row| {
                let port_value: u16 = row.get(0)?;
                Port::try_from(port_value)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(ports)
    }

    /// Gets all reservations whose paths start with the given prefix.
    ///
    /// This is useful for finding all reservations under a directory tree.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use std::path::Path;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let prefix = Path::new("/home/user/projects");
    /// let reservations = Database::get_reservations_by_path_prefix(db.connection(), prefix).unwrap();
    /// ```
    pub fn get_reservations_by_path_prefix(
        conn: &Connection,
        prefix: &Path,
    ) -> Result<Vec<Reservation>> {
        let mut stmt = conn.prepare(SELECT_BY_PATH_PREFIX)?;

        let reservations = stmt
            .query_map([prefix.to_string_lossy().to_string()], row_to_reservation)?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(reservations)
    }

    /// Finds reservations that haven't been used within the specified duration.
    ///
    /// This is useful for cleanup operations to find stale reservations.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use std::time::Duration;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let max_age = Duration::from_secs(86400 * 7); // 7 days
    /// let expired = Database::find_expired_reservations(db.connection(), max_age).unwrap();
    /// ```
    pub fn find_expired_reservations(
        conn: &Connection,
        max_age: Duration,
    ) -> Result<Vec<Reservation>> {
        let now_secs = systemtime_to_unix_secs(SystemTime::now())?;
        #[allow(clippy::cast_possible_wrap)]
        let max_age_secs = max_age.as_secs() as i64;
        let cutoff = now_secs.saturating_sub(max_age_secs);

        let mut stmt = conn.prepare(SELECT_EXPIRED)?;

        let reservations = stmt
            .query_map([cutoff], row_to_reservation)?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(reservations)
    }

    /// Checks if a specific port is reserved.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::Port;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let port = Port::try_from(8080).unwrap();
    /// let is_reserved = Database::is_port_reserved(db.connection(), port).unwrap();
    /// ```
    pub fn is_port_reserved(conn: &Connection, port: Port) -> Result<bool> {
        let count: i32 =
            conn.query_row(CHECK_PORT_RESERVED, params![port.value()], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Gets a reservation by port number.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails (other than "not found").
    ///
    /// # Returns
    ///
    /// - `Ok(Some(reservation))` if a reservation exists for this port
    /// - `Ok(None)` if no reservation exists for this port
    /// - `Err(_)` if a database error occurs
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::Port;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Database::get_reservation_by_port(db.connection(), port).unwrap();
    /// ```
    pub fn get_reservation_by_port(conn: &Connection, port: Port) -> Result<Option<Reservation>> {
        let mut stmt = conn.prepare_cached(SELECT_BY_PORT)?;
        let mut rows = stmt.query_map(params![port.value()], row_to_reservation)?;

        match rows.next() {
            Some(Ok(reservation)) => Ok(Some(reservation)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Gets all reserved ports in a range.
    ///
    /// This is an alias for `get_reserved_ports` with the same behavior,
    /// provided for consistency with the CLI command naming.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Port, PortRange};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5100).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    ///
    /// let reserved = Database::get_reserved_ports_in_range(db.connection(), &range).unwrap();
    /// ```
    pub fn get_reserved_ports_in_range(conn: &Connection, range: &PortRange) -> Result<Vec<Port>> {
        // This is the same as get_reserved_ports - we just provide both names
        Self::get_reserved_ports(conn, range)
    }

    /// Gets all unique project identifiers from reservations.
    ///
    /// Returns a sorted list of distinct non-null project values.
    /// Projects with NULL values are excluded from the result.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// let projects = Database::list_projects(db.connection()).unwrap();
    /// for project in projects {
    ///     println!("{}", project);
    /// }
    /// ```
    pub fn list_projects(conn: &Connection) -> Result<Vec<String>> {
        let query = "SELECT DISTINCT project FROM reservations
                     WHERE project IS NOT NULL
                     ORDER BY project";

        let mut stmt = conn.prepare(query)?;

        let projects = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(projects)
    }

    /// Verifies database integrity using PRAGMA `integrity_check`.
    ///
    /// This is compatible with existing transaction patterns as it's a read-only operation.
    ///
    /// # Errors
    ///
    /// Returns an error if the integrity check fails or detects corruption.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// db.verify_integrity().unwrap();
    /// ```
    pub fn verify_integrity(&mut self) -> Result<()> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;

        if result == "ok" {
            Ok(())
        } else {
            Err(Error::DatabaseCorruption {
                details: format!("Integrity check failed: {result}"),
            })
        }
    }

    /// Validates path relationship for database operations.
    ///
    /// This method checks if the operation on `target_path` from the current
    /// working directory is allowed. By default, ancestor and descendant paths
    /// are allowed (hierarchical relationships), but unrelated paths require
    /// the `allow_unrelated` flag.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The current working directory cannot be determined
    /// - The paths are unrelated and `allow_unrelated` is false
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use std::path::Path;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let db = Database::open(config).unwrap();
    ///
    /// // Check if we can operate on a path
    /// let target = Path::new("/home/user/project");
    /// let result = Database::validate_path_relationship(target, false);
    /// ```
    pub fn validate_path_relationship(target_path: &Path, allow_unrelated: bool) -> Result<()> {
        let current_dir = env::current_dir()?;
        let relationship = PathRelationship::between(target_path, &current_dir);

        if !relationship.is_allowed_without_force() && !allow_unrelated {
            return Err(Error::PathRelationshipViolation {
                details: relationship.description(target_path, &current_dir),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::{create_test_database, create_test_reservation};
    use std::path::PathBuf;

    #[test]
    fn test_create_reservation() {
        let mut db = create_test_database();
        let reservation = create_test_reservation("/test/path", 5000);

        db.create_reservation(&reservation).unwrap();

        // Verify it was created
        let loaded = Database::get_reservation(db.connection(), reservation.key()).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().port(), reservation.port());
    }

    #[test]
    fn test_get_reservation_not_found() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/nonexistent"), None).unwrap();

        let result = Database::get_reservation(db.connection(), &key).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update_last_used() {
        let mut db = create_test_database();
        let reservation = create_test_reservation("/test/path", 5000);

        db.create_reservation(&reservation).unwrap();

        // Wait a bit to ensure timestamp changes (need at least 1 second for Unix timestamp precision)
        std::thread::sleep(std::time::Duration::from_secs(2));

        let updated = db.update_last_used(reservation.key()).unwrap();
        assert!(updated);

        // Verify timestamp was updated
        let loaded = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        assert!(loaded.last_used_at() > reservation.last_used_at());
    }

    #[test]
    fn test_update_last_used_not_found() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/nonexistent"), None).unwrap();

        let updated = db.update_last_used(&key).unwrap();
        assert!(!updated);
    }

    #[test]
    fn test_delete_reservation() {
        let mut db = create_test_database();
        let reservation = create_test_reservation("/test/path", 5000);

        db.create_reservation(&reservation).unwrap();

        let deleted = db.delete_reservation(reservation.key()).unwrap();
        assert!(deleted);

        // Verify it was deleted
        let loaded = Database::get_reservation(db.connection(), reservation.key()).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_delete_reservation_not_found() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/nonexistent"), None).unwrap();

        let deleted = db.delete_reservation(&key).unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_list_all_reservations() {
        let mut db = create_test_database();

        // Create multiple reservations
        let r1 = create_test_reservation("/path1", 5000);
        let r2 = create_test_reservation("/path2", 5001);
        let r3 = create_test_reservation("/path3", 5002);

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();
        db.create_reservation(&r3).unwrap();

        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 3);

        // Verify they're sorted by path
        assert_eq!(all[0].key().path, PathBuf::from("/path1"));
        assert_eq!(all[1].key().path, PathBuf::from("/path2"));
        assert_eq!(all[2].key().path, PathBuf::from("/path3"));
    }

    #[test]
    fn test_list_all_reservations_empty() {
        let db = create_test_database();
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_reservation_with_optional_fields() {
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("my-project".to_string()))
            .task(Some("my-task".to_string()))
            .build()
            .unwrap();

        db.create_reservation(&reservation).unwrap();

        let loaded = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.project(), Some("my-project"));
        assert_eq!(loaded.task(), Some("my-task"));
    }

    #[test]
    fn test_reservation_with_tag() {
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/path"), Some("web".to_string())).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();

        db.create_reservation(&reservation).unwrap();

        let loaded = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.key().tag, Some("web".to_string()));
    }

    #[test]
    fn test_replace_reservation() {
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port1 = Port::try_from(5000).unwrap();
        let port2 = Port::try_from(5001).unwrap();

        // Create initial reservation
        let r1 = Reservation::builder(key.clone(), port1).build().unwrap();
        db.create_reservation(&r1).unwrap();

        // Replace with new port
        let r2 = Reservation::builder(key.clone(), port2).build().unwrap();
        db.create_reservation(&r2).unwrap();

        // Should have the new port
        let loaded = Database::get_reservation(db.connection(), &key)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.port(), port2);

        // Should still have only one reservation
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_get_reserved_ports() {
        let mut db = create_test_database();

        // Create reservations with different ports
        db.create_reservation(&create_test_reservation("/path1", 5000))
            .unwrap();
        db.create_reservation(&create_test_reservation("/path2", 5005))
            .unwrap();
        db.create_reservation(&create_test_reservation("/path3", 5010))
            .unwrap();
        db.create_reservation(&create_test_reservation("/path4", 5020))
            .unwrap();

        // Query for ports in range 5000-5010
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        let reserved = Database::get_reserved_ports(db.connection(), &range).unwrap();
        assert_eq!(reserved.len(), 3);
        assert_eq!(reserved[0].value(), 5000);
        assert_eq!(reserved[1].value(), 5005);
        assert_eq!(reserved[2].value(), 5010);
    }

    #[test]
    fn test_get_reservations_by_path_prefix() {
        let mut db = create_test_database();

        // Create reservations with different paths
        db.create_reservation(&create_test_reservation("/home/user/project1", 5000))
            .unwrap();
        db.create_reservation(&create_test_reservation("/home/user/project2", 5001))
            .unwrap();
        db.create_reservation(&create_test_reservation("/opt/project3", 5002))
            .unwrap();

        // Query for /home/user prefix
        let prefix = Path::new("/home/user");
        let reservations =
            Database::get_reservations_by_path_prefix(db.connection(), prefix).unwrap();

        assert_eq!(reservations.len(), 2);
        assert!(reservations[0]
            .key()
            .path
            .to_string_lossy()
            .starts_with("/home/user"));
        assert!(reservations[1]
            .key()
            .path
            .to_string_lossy()
            .starts_with("/home/user"));
    }

    #[test]
    fn test_find_expired_reservations() {
        let mut db = create_test_database();

        // Create a reservation with old last_used_at
        let old_time = SystemTime::now() - Duration::from_secs(200);
        let key = ReservationKey::new(PathBuf::from("/old/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let old_reservation = Reservation::builder(key, port)
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&old_reservation).unwrap();

        // Create a fresh reservation
        db.create_reservation(&create_test_reservation("/fresh/path", 5001))
            .unwrap();

        // Find expired (older than 100 seconds)
        let expired =
            Database::find_expired_reservations(db.connection(), Duration::from_secs(100)).unwrap();

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].key().path, PathBuf::from("/old/path"));
    }

    #[test]
    fn test_is_port_reserved() {
        let mut db = create_test_database();

        let port1 = Port::try_from(5000).unwrap();
        let port2 = Port::try_from(5001).unwrap();

        // Port not reserved initially
        assert!(!Database::is_port_reserved(db.connection(), port1).unwrap());

        // Create reservation
        db.create_reservation(&create_test_reservation("/path", 5000))
            .unwrap();

        // Port should now be reserved
        assert!(Database::is_port_reserved(db.connection(), port1).unwrap());

        // Different port still not reserved
        assert!(!Database::is_port_reserved(db.connection(), port2).unwrap());
    }

    #[test]
    fn test_validate_path_relationship_ancestor() {
        use std::env;

        let _db = create_test_database();
        let cwd = env::current_dir().unwrap();

        // Ancestor path (parent of cwd) should be allowed
        if let Some(parent) = cwd.parent() {
            let result = Database::validate_path_relationship(parent, false);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_validate_path_relationship_descendant() {
        use std::env;

        let _db = create_test_database();
        let cwd = env::current_dir().unwrap();

        // Descendant path (child of cwd) should be allowed
        let child = cwd.join("subdir");
        let result = Database::validate_path_relationship(&child, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_relationship_same() {
        use std::env;

        let _db = create_test_database();
        let cwd = env::current_dir().unwrap();

        // Same path should be allowed
        let result = Database::validate_path_relationship(&cwd, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_relationship_unrelated_denied() {
        let _db = create_test_database();

        // Create a path that's definitely unrelated to the current directory
        let unrelated = Path::new("/unrelated/path/xyz");

        // Should fail without allow_unrelated
        let result = Database::validate_path_relationship(unrelated, false);
        assert!(result.is_err());

        // Check that it's the right error type
        match result {
            Err(Error::PathRelationshipViolation { .. }) => {} // Expected
            _ => panic!("Expected PathRelationshipViolation error"),
        }
    }

    #[test]
    fn test_validate_path_relationship_unrelated_allowed() {
        let _db = create_test_database();

        // Create a path that's definitely unrelated to the current directory
        let unrelated = Path::new("/unrelated/path/xyz");

        // Should succeed with allow_unrelated
        let result = Database::validate_path_relationship(unrelated, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_projects_empty() {
        let db = create_test_database();
        let projects = Database::list_projects(db.connection()).unwrap();
        assert_eq!(projects.len(), 0);
    }

    #[test]
    fn test_list_projects_single() {
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/path1"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("project-a".to_string()))
            .build()
            .unwrap();

        db.create_reservation(&reservation).unwrap();

        let projects = Database::list_projects(db.connection()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0], "project-a");
    }

    #[test]
    fn test_list_projects_multiple() {
        let mut db = create_test_database();

        // Create reservations with different projects
        let r1 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .project(Some("zebra".to_string()))
        .build()
        .unwrap();

        let r2 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
            Port::try_from(5001).unwrap(),
        )
        .project(Some("alpha".to_string()))
        .build()
        .unwrap();

        let r3 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path3"), None).unwrap(),
            Port::try_from(5002).unwrap(),
        )
        .project(Some("beta".to_string()))
        .build()
        .unwrap();

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();
        db.create_reservation(&r3).unwrap();

        let projects = Database::list_projects(db.connection()).unwrap();
        assert_eq!(projects.len(), 3);
        // Should be sorted alphabetically
        assert_eq!(projects[0], "alpha");
        assert_eq!(projects[1], "beta");
        assert_eq!(projects[2], "zebra");
    }

    #[test]
    fn test_list_projects_duplicates() {
        let mut db = create_test_database();

        // Create multiple reservations with same project
        let r1 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .project(Some("same-project".to_string()))
        .build()
        .unwrap();

        let r2 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
            Port::try_from(5001).unwrap(),
        )
        .project(Some("same-project".to_string()))
        .build()
        .unwrap();

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();

        let projects = Database::list_projects(db.connection()).unwrap();
        // Should only return distinct values
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0], "same-project");
    }

    #[test]
    fn test_list_projects_excludes_null() {
        let mut db = create_test_database();

        // Create reservation with project
        let r1 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .project(Some("has-project".to_string()))
        .build()
        .unwrap();

        // Create reservation without project (NULL)
        let r2 = Reservation::builder(
            ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
            Port::try_from(5001).unwrap(),
        )
        .build()
        .unwrap();

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();

        let projects = Database::list_projects(db.connection()).unwrap();
        // Should only return non-NULL projects
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0], "has-project");
    }
}
