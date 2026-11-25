//! Database CRUD operations for reservations.
//!
//! This module implements all create, read, update, and delete operations
//! for port reservations in the database.
//!
//! Every value crosses the storage boundary through the verified storage
//! kernel (`verified::storage`): writes encode a reservation with
//! `encode_record` (keys with `encode_storage_key`), and the row decoder
//! checks paths, tags, ports, identifiers and timestamps with the matching
//! kernel decoders. The kernel proves that the write predicate and the read
//! predicate are the same set, so nothing trop writes is later reported as
//! corruption, and that the stored key is injective. Lookups by key match the
//! key's exact text (`lookup_key`), so they never return another key's row
//! and still report a corrupt row stored under that text.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, Transaction, TransactionBehavior};

use crate::error::{Error, Result};
use crate::path::PathRelationship;
use crate::verified::storage::{
    encode_metadata_update, Cell, KeyError, PathVerdict, RecordError, RecordFields,
    SecsDecodeError, SecsEncodeError, StoredKey, StoredRow, TagDecode,
};
use crate::{Port, PortRange, Reservation, ReservationKey};

use super::connection::Database;
use super::schema::{DELETE_RESERVATION, INSERT_RESERVATION};

/// Whole seconds since the Unix epoch of `time`.
fn secs_since_epoch(time: SystemTime) -> Result<u64> {
    Ok(time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| Error::Validation {
            field: "timestamp".into(),
            message: format!("Invalid timestamp: {error}"),
        })?
        .as_secs())
}

/// The validation error for a timestamp the storage kernel refuses.
fn timestamp_error(error: SecsEncodeError) -> Error {
    Error::Validation {
        field: "timestamp".into(),
        message: match error {
            SecsEncodeError::ExceedsI64 => {
                "timestamp exceeds SQLite's signed 64-bit representation".into()
            }
            SecsEncodeError::OutsideSupportedRange => {
                "timestamp is outside the supported display and serialization range".into()
            }
        },
    }
}

/// Converts a `SystemTime` to Unix epoch seconds for database storage
/// (verified kernel `encode_unix_secs`).
///
/// # Errors
///
/// Returns an error if the time is before the Unix epoch, or beyond the
/// storable range.
pub(super) fn systemtime_to_unix_secs(time: SystemTime) -> Result<i64> {
    crate::verified::storage::encode_unix_secs(secs_since_epoch(time)?).map_err(timestamp_error)
}

/// The validation error for a key the storage kernel refuses to store. It
/// names the same rule the row decoder enforces when reading rows back.
pub(super) fn key_error(error: KeyError) -> Error {
    let (field, message) = match error {
        KeyError::PathNotUtf8 => (
            "path",
            "path is not valid UTF-8, so it has no exact stored text",
        ),
        KeyError::Path(PathVerdict::Empty) => ("path", "path must be nonempty"),
        KeyError::Path(PathVerdict::NotAbsolute) => ("path", "path must be absolute"),
        KeyError::Path(PathVerdict::NotNormal | PathVerdict::Canonical) => (
            "path",
            "path must already be in its absolute lexical-normal form",
        ),
        KeyError::TagNotCanonical => ("tag", "tag must be absent or exact trimmed nonempty text"),
    };
    Error::Validation {
        field: field.into(),
        message: message.into(),
    }
}

/// The validation error for a reservation the storage kernel refuses to
/// store.
fn record_error(error: RecordError) -> Error {
    match error {
        RecordError::Key(error) => key_error(error),
        RecordError::InvalidPort => Error::Validation {
            field: "port".into(),
            message: "port must be an integer in 1..=65535".into(),
        },
        RecordError::ProjectNotCanonical => Error::Validation {
            field: "project".into(),
            message: "project must be absent or exact trimmed nonempty text".into(),
        },
        RecordError::TaskNotCanonical => Error::Validation {
            field: "task".into(),
            message: "task must be absent or exact trimmed nonempty text".into(),
        },
        RecordError::CreatedAt(error) | RecordError::LastUsedAt(error) => timestamp_error(error),
    }
}

/// The row to write for `reservation`: every persisted field checked and
/// encoded by the verified storage kernel (`encode_record`), before any SQL
/// runs. The `sticky` flag is not persisted.
///
/// # Errors
///
/// Returns a validation error for a key, project, task or timestamp that the
/// database could not read back: the first failing field in the order path,
/// tag, port, project, task, creation time, last use. A timestamp before the
/// Unix epoch has no `u64` seconds for the kernel, so it is reported first.
pub(super) fn encode_reservation(reservation: &Reservation) -> Result<StoredRow<'_>> {
    let key = reservation.key();
    let fields = RecordFields {
        path: key.path.to_str(),
        platform: crate::path::storage_platform(&key.path),
        tag: key.tag.as_deref(),
        port: reservation.port().value(),
        project: reservation.project(),
        task: reservation.task(),
        created_secs: secs_since_epoch(reservation.created_at())?,
        last_used_secs: secs_since_epoch(reservation.last_used_at())?,
    };
    crate::verified::storage::encode_record(&fields).map_err(record_error)
}

/// The stored form of a key that is about to be written or deleted.
///
/// # Errors
///
/// Returns a validation error for a key the database could not read back.
pub(super) fn storable_key(key: &ReservationKey) -> Result<StoredKey<'_>> {
    key.storage_key().map_err(key_error)
}

/// A stored SQLite value as the verified kernels see it: valid UTF-8 text,
/// or the storage class otherwise.
pub(super) fn cell_of(value: ValueRef<'_>) -> Cell<'_> {
    match value {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(value) => Cell::Integer(value),
        ValueRef::Real(_) => Cell::Real,
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).map_or(Cell::InvalidText, Cell::Text),
        ValueRef::Blob(_) => Cell::Blob,
    }
}

/// An owned copy of a stored value, for when the row it came from cannot
/// stay borrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OwnedCell {
    Null,
    Integer(i64),
    Real,
    Text(String),
    InvalidText,
    Blob,
}

impl OwnedCell {
    /// Copies a stored value.
    pub(super) fn copy_of(value: ValueRef<'_>) -> Self {
        match cell_of(value) {
            Cell::Null => Self::Null,
            Cell::Integer(value) => Self::Integer(value),
            Cell::Real => Self::Real,
            Cell::Text(text) => Self::Text(text.to_owned()),
            Cell::InvalidText => Self::InvalidText,
            Cell::Blob => Self::Blob,
        }
    }

    /// The value as the verified kernels see it.
    pub(super) fn cell(&self) -> Cell<'_> {
        match self {
            Self::Null => Cell::Null,
            Self::Integer(value) => Cell::Integer(*value),
            Self::Real => Cell::Real,
            Self::Text(text) => Cell::Text(text),
            Self::InvalidText => Cell::InvalidText,
            Self::Blob => Cell::Blob,
        }
    }
}

/// Writes `row`, replacing the row stored under the same key (and only that
/// row: the stored key is injective, and a port held by another key makes
/// the plain `INSERT` fail).
fn replace_row(conn: &Connection, row: &StoredRow<'_>) -> Result<()> {
    conn.execute(DELETE_RESERVATION, params![row.path(), row.tag()])?;
    conn.execute(
        INSERT_RESERVATION,
        params![
            row.path(),
            row.tag(),
            row.port(),
            row.project(),
            row.task(),
            row.created_at(),
            row.last_used_at(),
        ],
    )?;
    Ok(())
}

/// The reservations whose stored path starts with `prefix`, component-wise.
///
/// The verified kernel `prefix_query` turns the prefix into its canonical
/// key: on stored (canonical) paths, "has the key as a component prefix"
/// (`canonical_is_component_prefix`, the post-filter) is exactly
/// `Path::starts_with(prefix)`, and every such path starts with the key's
/// bytes, so the SQL prefilter `instr(path, key) = 1` (exactly the stored
/// texts whose bytes start with the key's: no wildcards, case folding, NUL
/// truncation or pattern-length limit) never drops a match.
#[cfg(unix)]
fn reservations_under(conn: &Connection, prefix: &Path) -> Result<Vec<Reservation>> {
    use std::os::unix::ffi::OsStrExt;

    use crate::verified::storage::PrefixQuery;

    let key = match crate::verified::storage::prefix_query(prefix.as_os_str().as_bytes()) {
        PrefixQuery::Nothing => return Ok(Vec::new()),
        PrefixQuery::Everything => None,
        // A component prefix of a UTF-8 path ends before a `/` or at the
        // end, so it is UTF-8 itself: a key that is not matches nothing.
        PrefixQuery::Under(key) => match String::from_utf8(key) {
            Ok(key) => Some(key),
            Err(_) => return Ok(Vec::new()),
        },
    };
    let mut stmt = conn.prepare(SELECT_BY_PATH_PREFIX)?;
    let mut rows = stmt.query([key.as_deref().unwrap_or("")])?;
    let mut reservations = Vec::new();
    while let Some(row) = rows.next()? {
        let reservation = row_to_reservation(row)?;
        let stored = reservation.key().path.as_os_str().as_bytes();
        let selected = key.as_ref().map_or(true, |key| {
            crate::verified::path_relation::canonical_is_component_prefix(key.as_bytes(), stored)
        });
        if selected {
            reservations.push(reservation);
        }
    }
    Ok(reservations)
}

/// The reservations whose stored path starts with `prefix`, component-wise.
///
/// Outside Unix, stored paths may be native (`C:\...`) or `/`-rooted
/// spellings written on Unix, so no single text key prefilters both: every
/// row's stored path text is tested with [`Path::starts_with`], and only the
/// rows that match are decoded (so, as with the Unix prefilter, a corrupt
/// row elsewhere does not fail the query). A decoded path is the stored
/// text itself, so no further filtering is needed.
#[cfg(not(unix))]
fn reservations_under(conn: &Connection, prefix: &Path) -> Result<Vec<Reservation>> {
    let mut stmt = conn.prepare(LIST_RESERVATIONS)?;
    let mut rows = stmt.query([])?;
    let mut reservations = Vec::new();
    while let Some(row) = rows.next()? {
        let matches = match row.get_ref(0)? {
            ValueRef::Text(bytes) => {
                std::str::from_utf8(bytes).is_ok_and(|text| Path::new(text).starts_with(prefix))
            }
            _ => false,
        };
        if matches {
            reservations.push(row_to_reservation(row)?);
        }
    }
    Ok(reservations)
}

/// Converts stored Unix epoch seconds to a representable `SystemTime`
/// (verified kernel `decode_unix_secs`; the platform's `SystemTime` range is
/// checked here).
fn unix_secs_to_systemtime(secs: i64, field: &str, key: &str) -> Result<SystemTime> {
    let seconds = crate::verified::storage::decode_unix_secs(secs).map_err(|error| {
        Error::corrupt_stored_value(
            "reservations",
            field,
            key,
            match error {
                SecsDecodeError::Negative => "timestamp must be nonnegative",
                SecsDecodeError::OutsideSupportedRange => {
                    "timestamp is outside the supported display and serialization range"
                }
            },
        )
    })?;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| {
            Error::corrupt_stored_value(
                "reservations",
                field,
                key,
                "timestamp is outside this platform's representable SystemTime range",
            )
        })
}

/// Helper function to deserialize a reservation from a database row.
///
/// Expects row fields in this order: path, tag, port, project, task, `created_at`, `last_used_at`
///
/// Each value is decoded by the verified storage kernel; the checks together
/// are `verified::storage::row_decodes`, which accepts exactly the rows the
/// write path produces.
pub(super) fn row_to_reservation(row: &rusqlite::Row<'_>) -> Result<Reservation> {
    let key_context = reservation_key_context(row);
    let path = required_text(row.get_ref(0)?, "path", &key_context)?;
    let path = validate_stored_path(path, &key_context)?;
    let tag = decode_stored_tag(row.get_ref(1)?, &key_context)?;
    // Both parts were decoded exactly: the tag is canonical, so no trimming
    // or other normalization applies.
    let key = ReservationKey { path, tag };

    let port_value = required_integer(row.get_ref(2)?, "port", &key_context)?;
    let port = crate::verified::storage::decode_stored_port(port_value)
        .and_then(|port| Port::try_from(port).ok())
        .ok_or_else(|| {
            Error::corrupt_stored_value(
                "reservations",
                "port",
                &key_context,
                "port must be an integer in 1..=65535",
            )
        })?;

    let project = optional_identifier(row.get_ref(3)?, "project", &key_context)?;
    let task = optional_identifier(row.get_ref(4)?, "task", &key_context)?;
    let created_secs = required_integer(row.get_ref(5)?, "created_at", &key_context)?;
    let last_used_secs = required_integer(row.get_ref(6)?, "last_used_at", &key_context)?;
    let created_at = unix_secs_to_systemtime(created_secs, "created_at", &key_context)?;
    let last_used_at = unix_secs_to_systemtime(last_used_secs, "last_used_at", &key_context)?;

    Reservation::builder(key, port)
        .project(project)
        .task(task)
        .created_at(created_at)
        .last_used_at(last_used_at)
        .build()
        .map_err(|error| {
            Error::corrupt_stored_value("reservations", &error.field, &key_context, &error.message)
        })
}

/// Checks a stored path with the verified storage kernel
/// (`classify_stored_path`), the same predicate the write path applies.
fn validate_stored_path(path: &str, key: &str) -> Result<PathBuf> {
    let verdict = crate::verified::storage::classify_stored_path(
        path.as_bytes(),
        crate::path::storage_platform(Path::new(path)),
    );
    let reason = match verdict {
        PathVerdict::Canonical => return Ok(PathBuf::from(path)),
        PathVerdict::Empty => "path must be nonempty",
        PathVerdict::NotAbsolute => "path must be absolute",
        PathVerdict::NotNormal => "path must already be in its absolute lexical-normal form",
    };
    Err(Error::corrupt_stored_value(
        "reservations",
        "path",
        key,
        reason,
    ))
}

/// Decodes a stored tag with the verified storage kernel: `""` is untagged,
/// canonical text is the tag, anything else is corrupt.
fn decode_stored_tag(value: ValueRef<'_>, key: &str) -> Result<Option<String>> {
    let tag = text_value(value, "tag", key)?;
    match crate::verified::storage::decode_stored_tag(tag) {
        TagDecode::Untagged => Ok(None),
        TagDecode::Tagged => Ok(Some(tag.to_owned())),
        TagDecode::Corrupt => Err(Error::corrupt_stored_value(
            "reservations",
            "tag",
            key,
            "tag must be the empty untagged sentinel or exact trimmed nonempty text",
        )),
    }
}

fn optional_identifier(value: ValueRef<'_>, field: &str, key: &str) -> Result<Option<String>> {
    if value == ValueRef::Null {
        return Ok(None);
    }
    let identifier = text_value(value, field, key)?;
    if !crate::identifier::is_canonical_identifier(identifier) {
        return Err(Error::corrupt_stored_value(
            "reservations",
            field,
            key,
            "optional identifier must be NULL or exact trimmed nonempty text",
        ));
    }
    Ok(Some(identifier.to_owned()))
}

fn required_integer(value: ValueRef<'_>, field: &str, key: &str) -> Result<i64> {
    if let ValueRef::Integer(value) = value {
        Ok(value)
    } else {
        Err(Error::corrupt_stored_value(
            "reservations",
            field,
            key,
            &format!("expected INTEGER, found {}", value_kind(value)),
        ))
    }
}

fn required_text<'a>(value: ValueRef<'a>, field: &str, key: &str) -> Result<&'a str> {
    text_value(value, field, key)
}

fn text_value<'a>(value: ValueRef<'a>, field: &str, key: &str) -> Result<&'a str> {
    let ValueRef::Text(bytes) = value else {
        return Err(Error::corrupt_stored_value(
            "reservations",
            field,
            key,
            &format!("expected TEXT, found {}", value_kind(value)),
        ));
    };
    std::str::from_utf8(bytes).map_err(|_| {
        Error::corrupt_stored_value("reservations", field, key, "stored TEXT is not valid UTF-8")
    })
}

fn reservation_key_context(row: &rusqlite::Row<'_>) -> String {
    let path = row
        .get_ref(0)
        .map_or_else(|_| "<unreadable>".to_string(), describe_key_text);
    let tag = row.get_ref(1).map_or_else(
        |_| "<unreadable>".to_string(),
        |value| match value {
            ValueRef::Text([]) => "<untagged>".to_string(),
            other => describe_key_text(other),
        },
    );
    format!("path={path}, tag={tag}")
}

fn describe_key_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).map_or_else(
            |_| "<invalid UTF-8>".to_string(),
            |text| format!("\"{}\"", escape_text(text)),
        ),
        other => format!("<{}>", value_kind(other)),
    }
}

/// `char::escape_default` of every char (verified kernel `escape_default`:
/// printable ASCII only, no unescaped `"`, injective).
fn escape_text(value: &str) -> String {
    crate::verified::escape::escape_default(value)
}

fn value_kind(value: ValueRef<'_>) -> &'static str {
    match value {
        ValueRef::Null => "NULL",
        ValueRef::Integer(_) => "INTEGER",
        ValueRef::Real(_) => "REAL",
        ValueRef::Text(_) => "TEXT",
        ValueRef::Blob(_) => "BLOB",
    }
}

// SQL statements for CRUD operations
const SELECT_RESERVATION: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE path = ? AND tag = ?
";

const UPDATE_LAST_USED: &str = r"
    UPDATE reservations
    SET last_used_at = ?
    WHERE path = ? AND tag = ?
";

const UPDATE_METADATA_AND_LAST_USED: &str = r"
    UPDATE reservations
    SET project = ?, task = ?, last_used_at = ?
    WHERE path = ? AND tag = ?
";

const DELETE_RESERVATION_IF_UNCHANGED: &str = r"
    DELETE FROM reservations
    WHERE path = ?
      AND tag = ?
      AND port = ?
      AND project IS ?
      AND task IS ?
      AND created_at = ?
      AND last_used_at = ?
";

const LIST_RESERVATIONS: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    ORDER BY path, tag
";

const SELECT_RESERVED_PORTS: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE port >= ? AND port <= ?
    ORDER BY port
";

// Only the Unix prefix query binds a text key.
#[cfg_attr(not(unix), allow(dead_code))]
const SELECT_BY_PATH_PREFIX: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE instr(path, ?) = 1
    ORDER BY path, tag
";

const SELECT_BY_EXACT_PATH: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE path = ?
    ORDER BY tag
";

const SELECT_TAGGED_BY_EXACT_PATH: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE path = ? AND tag <> ''
    ORDER BY tag
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
    /// Executes an operation inside a `SQLite` savepoint.
    pub(crate) fn with_savepoint<T>(
        conn: &Connection,
        name: &str,
        operation: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        conn.execute_batch(&format!("SAVEPOINT {name}"))?;

        match operation(conn) {
            Ok(value) => {
                conn.execute_batch(&format!("RELEASE {name}"))?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute_batch(&format!("ROLLBACK TO {name}"));
                let _ = conn.execute_batch(&format!("RELEASE {name}"));
                Err(error)
            }
        }
    }

    /// Executes a mutating operation under one owning `IMMEDIATE`
    /// transaction, or under a savepoint when the caller already owns a
    /// transaction.
    ///
    /// This lets library entry points preserve the same serialization boundary
    /// as the CLI without attempting to nest `SQLite` transactions.
    pub(crate) fn with_immediate_transaction_or_savepoint<T>(
        conn: &Connection,
        savepoint_name: &str,
        lock_operation: &str,
        operation: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let timeout_millis =
            conn.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))?;
        // SQLite treats negative busy timeouts as zero (verified kernel
        // `busy_timeout_millis`).
        let timeout = Duration::from_millis(crate::verified::storage::busy_timeout_millis(
            timeout_millis,
        ));

        if !conn.is_autocommit() {
            return Self::with_savepoint(conn, savepoint_name, operation).map_err(|error| {
                error.classify_sqlite_lock(
                    timeout,
                    format!("executing {lock_operation} in a caller-owned transaction"),
                )
            });
        }

        let transaction = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
            .map_err(Error::from)
            .map_err(|error| {
                error.classify_sqlite_lock(
                    timeout,
                    format!("starting an immediate transaction for {lock_operation}"),
                )
            })?;
        match operation(&transaction) {
            Ok(value) => {
                transaction.commit().map_err(Error::from).map_err(|error| {
                    error.classify_sqlite_lock(
                        timeout,
                        format!("committing the transaction for {lock_operation}"),
                    )
                })?;
                Ok(value)
            }
            Err(error) => {
                Err(error.classify_sqlite_lock(timeout, format!("executing {lock_operation}")))
            }
        }
    }

    /// Creates or updates a reservation in the database.
    ///
    /// This operation uses a transaction with IMMEDIATE mode to ensure
    /// atomicity and prevent conflicts.
    ///
    /// The reservation is checked before any SQL runs: its key, project, task
    /// and timestamps must be storable exactly as the database reads rows
    /// back (see [`Error::Validation`]). The `sticky` flag is not persisted.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The reservation cannot be stored: a path that is not valid UTF-8 or
    ///   not in canonical absolute form, a blank or untrimmed tag, project or
    ///   task, or a timestamp outside the storable range (a validation error;
    ///   nothing is written)
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
        let timeout = self.busy_timeout();
        self.create_reservation_inner(reservation).map_err(|error| {
            error.classify_sqlite_lock(timeout, "creating or updating a reservation")
        })
    }

    fn create_reservation_inner(&mut self, reservation: &Reservation) -> Result<()> {
        let row = encode_reservation(reservation)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        replace_row(&tx, &row)?;
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
    /// Returns a validation error, before any SQL runs, if the reservation
    /// cannot be stored (as for [`Database::create_reservation`]), or an
    /// error if the insert fails.
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
        let row = encode_reservation(reservation)?;
        Self::with_savepoint(conn, "trop_create_reservation", |conn| {
            replace_row(conn, &row)
        })
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
    /// - `Ok(None)` if the reservation doesn't exist, including when no
    ///   stored row can hold the key (a path that is not valid UTF-8, or a
    ///   `Some("")` tag, which would otherwise alias the untagged reservation)
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
        // The verified kernel `lookup_key` matches exactly the rows holding
        // this key: the key's own text, never a lossy or aliasing spelling.
        // A key no row can hold (a path that is not UTF-8, or a `Some("")`
        // tag) has no reservation; a corrupt row stored under a non-storable
        // key's exact text is still read and reported.
        let Some(lookup) =
            crate::verified::storage::lookup_key(key.path.to_str(), key.tag.as_deref())
        else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(SELECT_RESERVATION)?;
        let mut rows = stmt.query(params![lookup.path(), lookup.tag()])?;
        rows.next()?.map(row_to_reservation).transpose()
    }

    /// Updates the `last_used_at` timestamp for a reservation.
    ///
    /// # Errors
    ///
    /// Returns a validation error, before any SQL runs, if the key cannot be
    /// stored (as for [`Database::create_reservation`]), or an error if the
    /// transaction or update fails.
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
        let timeout = self.busy_timeout();
        self.update_last_used_inner(key).map_err(|error| {
            error.classify_sqlite_lock(timeout, "updating reservation access time")
        })
    }

    fn update_last_used_inner(&mut self, key: &ReservationKey) -> Result<bool> {
        let stored = storable_key(key)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let now = systemtime_to_unix_secs(SystemTime::now())?;

        let rows_affected =
            tx.execute(UPDATE_LAST_USED, params![now, stored.path(), stored.tag()])?;

        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// Deletes a reservation from the database.
    ///
    /// # Errors
    ///
    /// Returns a validation error, before any SQL runs, if the key cannot be
    /// stored (as for [`Database::create_reservation`]), or an error if the
    /// transaction or delete fails.
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
        let timeout = self.busy_timeout();
        self.delete_reservation_inner(key)
            .map_err(|error| error.classify_sqlite_lock(timeout, "deleting a reservation"))
    }

    fn delete_reservation_inner(&mut self, key: &ReservationKey) -> Result<bool> {
        let stored = storable_key(key)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let rows_affected = tx.execute(DELETE_RESERVATION, params![stored.path(), stored.tag()])?;

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
    /// Returns a validation error, before any SQL runs, if the key cannot be
    /// stored, or an error if the timestamp conversion fails or the database
    /// update fails.
    pub fn update_last_used_simple(conn: &Connection, key: &ReservationKey) -> Result<bool> {
        let stored = storable_key(key)?;
        let now = systemtime_to_unix_secs(SystemTime::now())?;
        let rows_affected =
            conn.execute(UPDATE_LAST_USED, params![now, stored.path(), stored.tag()])?;
        Ok(rows_affected > 0)
    }

    /// Atomically updates sticky metadata and the last-used timestamp for one
    /// reservation without changing its key, port, or creation timestamp.
    pub(crate) fn update_metadata_and_last_used_simple(
        conn: &Connection,
        key: &ReservationKey,
        project: Option<&str>,
        task: Option<&str>,
        last_used_at: SystemTime,
    ) -> Result<bool> {
        // Every bound value comes from the verified storage kernel: the key
        // columns from `encode_storage_key`, the new project, task and last
        // use from `encode_metadata_update`, which checks them exactly as
        // `encode_record` does. A last use before the epoch has no `u64`
        // seconds for the kernel, so it is reported here, after the key and
        // before the project and task (a last use past the storable range is
        // the kernel's, reported after them).
        let stored = storable_key(key)?;
        let last_used_secs = secs_since_epoch(last_used_at)?;
        let update =
            encode_metadata_update(stored, project, task, last_used_secs).map_err(record_error)?;
        let rows_affected = conn.execute(
            UPDATE_METADATA_AND_LAST_USED,
            params![
                update.project(),
                update.task(),
                update.last_used_at(),
                update.path(),
                update.tag()
            ],
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
    /// Returns a validation error, before any SQL runs, if the key cannot be
    /// stored, or an error if the database deletion fails.
    pub fn delete_reservation_simple(conn: &Connection, key: &ReservationKey) -> Result<bool> {
        let stored = storable_key(key)?;
        let rows_affected =
            conn.execute(DELETE_RESERVATION, params![stored.path(), stored.tag()])?;
        Ok(rows_affected > 0)
    }

    /// Deletes a reservation only if every persisted field still matches the
    /// supplied snapshot.
    ///
    /// Cleanup uses this guard inside its owning transaction so a filesystem
    /// decision or expiration predicate can never be applied to a refreshed
    /// or replaced row with the same key.
    pub(crate) fn delete_reservation_if_unchanged(
        conn: &Connection,
        reservation: &Reservation,
    ) -> Result<bool> {
        let row = encode_reservation(reservation)?;
        let rows_affected = conn.execute(
            DELETE_RESERVATION_IF_UNCHANGED,
            params![
                row.path(),
                row.tag(),
                row.port(),
                row.project(),
                row.task(),
                row.created_at(),
                row.last_used_at(),
            ],
        )?;
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
        let mut rows = stmt.query([])?;
        let mut reservations = Vec::new();
        while let Some(row) = rows.next()? {
            reservations.push(row_to_reservation(row)?);
        }
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
        let mut rows = stmt.query(params![range.min().value(), range.max().value()])?;
        let mut ports = Vec::new();
        while let Some(row) = rows.next()? {
            ports.push(row_to_reservation(row)?.port());
        }
        Ok(ports)
    }

    /// Gets all reservations whose paths start with the given prefix.
    ///
    /// This is useful for finding all reservations under a directory tree.
    /// Matching is component-wise, like [`Path::starts_with`]: `/a/b` selects
    /// `/a/b` and `/a/b/c` but not `/a/bc`, and redundant separators or `.`
    /// components in the prefix (`/a/b/`, `/a//b`, `/a/./b`) do not change
    /// the result.
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
        reservations_under(conn, prefix)
    }

    /// Returns every tagged and untagged reservation at one exact path.
    ///
    /// Reservations below the path are deliberately excluded.
    pub(crate) fn get_reservations_by_exact_path(
        conn: &Connection,
        path: &Path,
    ) -> Result<Vec<Reservation>> {
        // A path with no UTF-8 text has no exact stored text, so no row; any
        // other path is matched by its own text (a corrupt row stored under a
        // non-storable path's text is read and reported).
        let Some(stored) = path.to_str() else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare_cached(SELECT_BY_EXACT_PATH)?;
        let mut rows = stmt.query([stored])?;
        let mut reservations = Vec::new();
        while let Some(row) = rows.next()? {
            reservations.push(row_to_reservation(row)?);
        }
        Ok(reservations)
    }

    /// Returns every tagged reservation at one exact path.
    ///
    /// Untagged reservations and reservations below the path are deliberately
    /// excluded. Group reconciliation uses this exact set to distinguish a
    /// complete stored group from partial or changed service sets.
    pub(crate) fn get_tagged_reservations_by_exact_path(
        conn: &Connection,
        path: &Path,
    ) -> Result<Vec<Reservation>> {
        // A path with no UTF-8 text has no exact stored text, so no row; any
        // other path is matched by its own text (a corrupt row stored under a
        // non-storable path's text is read and reported).
        let Some(stored) = path.to_str() else {
            return Ok(Vec::new());
        };
        let mut stmt = conn.prepare_cached(SELECT_TAGGED_BY_EXACT_PATH)?;
        let mut rows = stmt.query([stored])?;
        let mut reservations = Vec::new();
        while let Some(row) = rows.next()? {
            reservations.push(row_to_reservation(row)?);
        }
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
        // Exact for every stored timestamp, including maximum ages beyond
        // `i64::MAX` seconds (verified kernel `expiry_cutoff`).
        let cutoff = crate::verified::expiry::expiry_cutoff(now_secs, max_age.as_secs());

        let mut stmt = conn.prepare(SELECT_EXPIRED)?;
        let mut rows = stmt.query([cutoff])?;
        let mut reservations = Vec::new();
        while let Some(row) = rows.next()? {
            reservations.push(row_to_reservation(row)?);
        }
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
        let mut rows = stmt.query(params![port.value()])?;
        rows.next()?.map(row_to_reservation).transpose()
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
        let projects = Self::list_all_reservations(conn)?
            .into_iter()
            .filter_map(|reservation| reservation.project().map(ToOwned::to_owned))
            .collect::<BTreeSet<_>>();
        Ok(projects.into_iter().collect())
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
    pub fn verify_integrity(&self) -> Result<()> {
        super::validation::validate_current_database(&self.conn)
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
    /// - The paths are unrelated both lexically and physically, and
    ///   `allow_unrelated` is false
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
        let lexical_relationship = PathRelationship::between(target_path, &current_dir);
        // Preserve lexical hierarchy for explicit paths. When that comparison
        // is unrelated, canonicalize each path's existing portion and restore
        // any nonexistent target suffix before comparing them physically.
        // This keeps canonical inferred identities compatible with the
        // platform's process-CWD spelling (notably Windows verbatim prefixes
        // and short names) without requiring the target itself to exist.
        let physically_related = || {
            let (canonical_target_root, target_remainder) =
                crate::path::canonicalize::canonicalize_existing(target_path).ok()?;
            let canonical_target = match target_remainder {
                Some(path) => canonical_target_root.join(path),
                None => canonical_target_root,
            };
            let canonical_current = crate::path::canonicalize::canonicalize(&current_dir).ok()?;
            Some(
                PathRelationship::between(&canonical_target, &canonical_current)
                    .is_allowed_without_force(),
            )
        };

        if !allow_unrelated
            && !lexical_relationship.is_allowed_without_force()
            && !physically_related().unwrap_or(false)
        {
            return Err(Error::PathRelationshipViolation {
                details: lexical_relationship.description(target_path, &current_dir),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::{create_test_database, create_test_reservation};
    use rusqlite::types::Value;
    use std::path::PathBuf;

    #[cfg(feature = "property-tests")]
    use proptest::prelude::*;

    // -----------------------------------------------------------------------
    // Glue and trusted-assumption checks for the verified storage kernels
    // -----------------------------------------------------------------------

    /// Every string over `alphabet` of length at most `max_len`.
    fn generated_strings(alphabet: &[char], max_len: usize) -> Vec<String> {
        let mut all = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..max_len {
            let mut next = Vec::new();
            for prefix in &frontier {
                for &c in alphabet {
                    let mut s = prefix.clone();
                    s.push(c);
                    next.push(s);
                }
            }
            all.extend(next.iter().cloned());
            frontier = next;
        }
        all
    }

    /// The trusted chrono bound behind `MAX_STORABLE_UNIX_SECS`: the largest
    /// timestamp chrono can represent (the write path used to ask chrono).
    #[test]
    fn max_storable_secs_is_chronos_maximum() {
        let max = crate::verified::storage::MAX_STORABLE_UNIX_SECS;
        let max = i64::try_from(max).unwrap();
        assert!(chrono::DateTime::<chrono::Utc>::from_timestamp(max, 0).is_some());
        assert!(chrono::DateTime::<chrono::Utc>::from_timestamp(max + 1, 0).is_none());
        assert_eq!(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp(), max);
        // Every non-negative timestamp up to it is representable (chrono's
        // range is an interval).
        assert!(chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).is_some());
    }

    /// The write path's timestamp conversion at its boundaries.
    #[test]
    fn timestamp_codec_boundaries() {
        use crate::verified::storage::{
            decode_unix_secs, encode_unix_secs, MAX_STORABLE_UNIX_SECS,
        };
        let max = i64::try_from(MAX_STORABLE_UNIX_SECS).unwrap();
        assert_eq!(encode_unix_secs(0), Ok(0));
        assert_eq!(encode_unix_secs(MAX_STORABLE_UNIX_SECS), Ok(max));
        assert_eq!(
            encode_unix_secs(MAX_STORABLE_UNIX_SECS + 1),
            Err(SecsEncodeError::OutsideSupportedRange)
        );
        assert_eq!(
            encode_unix_secs(i64::MAX as u64),
            Err(SecsEncodeError::OutsideSupportedRange)
        );
        assert_eq!(
            encode_unix_secs(i64::MAX as u64 + 1),
            Err(SecsEncodeError::ExceedsI64)
        );
        assert_eq!(encode_unix_secs(u64::MAX), Err(SecsEncodeError::ExceedsI64));
        assert_eq!(decode_unix_secs(0), Ok(0));
        assert_eq!(decode_unix_secs(max), Ok(MAX_STORABLE_UNIX_SECS));
        assert_eq!(decode_unix_secs(-1), Err(SecsDecodeError::Negative));
        assert_eq!(decode_unix_secs(i64::MIN), Err(SecsDecodeError::Negative));
        assert_eq!(
            decode_unix_secs(max + 1),
            Err(SecsDecodeError::OutsideSupportedRange)
        );
        assert_eq!(
            decode_unix_secs(i64::MAX),
            Err(SecsDecodeError::OutsideSupportedRange)
        );
        let past = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert!(matches!(
            systemtime_to_unix_secs(past),
            Err(Error::Validation { .. })
        ));
    }

    #[test]
    fn stored_port_boundaries() {
        use crate::verified::storage::decode_stored_port;
        for (value, expected) in [
            (i64::MIN, None),
            (-1, None),
            (0, None),
            (1, Some(1)),
            (65535, Some(65535)),
            (65536, None),
            (i64::MAX, None),
        ] {
            assert_eq!(decode_stored_port(value), expected, "{value}");
        }
    }

    #[test]
    fn busy_timeout_readback_clamps_negative_values() {
        use crate::verified::storage::busy_timeout_millis;
        assert_eq!(busy_timeout_millis(i64::MIN), 0);
        assert_eq!(busy_timeout_millis(-1), 0);
        assert_eq!(busy_timeout_millis(0), 0);
        assert_eq!(busy_timeout_millis(5000), 5000);
        assert_eq!(busy_timeout_millis(i64::MAX), i64::MAX as u64);
    }

    /// The expiry cutoff against an independent 128-bit reference, at the
    /// edges of the `i64` and `u64` ranges.
    #[test]
    fn expiry_cutoff_matches_wide_arithmetic() {
        use crate::verified::expiry::expiry_cutoff;
        let nows = [i64::MIN, -1, 0, 1, 1_700_000_000, i64::MAX];
        let ages = [
            0,
            1,
            1_700_000_000,
            i64::MAX as u64,
            i64::MAX as u64 + 1,
            u64::MAX,
        ];
        for now in nows {
            for age in ages {
                let exact = i128::from(now) - i128::from(age);
                let expected = i64::try_from(exact).unwrap_or(i64::MIN);
                let cutoff = expiry_cutoff(now, age);
                assert_eq!(cutoff, expected, "now {now}, age {age}");
                for t in [i64::MIN, -1, 0, 1, now, i64::MAX] {
                    let expired = i128::from(now) - i128::from(t) > i128::from(age);
                    assert_eq!(t < cutoff, expired, "now {now}, age {age}, t {t}");
                }
            }
        }
    }

    /// The read predicate is unchanged: on Unix the kernel's stored-path
    /// verdict agrees with the previous decoder (`PathBuf::is_absolute`, then
    /// a `resolve_components` round trip) on every short string over the
    /// path-relevant alphabet.
    #[test]
    #[cfg(unix)]
    fn stored_path_verdict_matches_previous_decoder() {
        for input in generated_strings(&['/', '.', 'a', '\\'], 7) {
            let path = PathBuf::from(&input);
            let previous = if input.is_empty() {
                PathVerdict::Empty
            } else if !path.is_absolute() {
                PathVerdict::NotAbsolute
            } else if crate::path::normalize::resolve_components(&path)
                .ok()
                .and_then(|normalized| normalized.to_str().map(|value| value == input))
                .unwrap_or(false)
            {
                PathVerdict::Canonical
            } else {
                PathVerdict::NotNormal
            };
            assert_eq!(
                crate::verified::storage::classify_stored_path(
                    input.as_bytes(),
                    crate::path::storage_platform(&path)
                ),
                previous,
                "{input:?}"
            );
        }
    }

    /// The Windows-portable stored-path predicate the decoder used before the
    /// storage kernel (`slash_rooted_path_is_lexically_normal`), kept as a
    /// reference for the kernel's slash-rooted branch, which Linux CI never
    /// reaches through the glue.
    fn previous_slash_rooted_path_is_lexically_normal(path: &str) -> bool {
        path == "/"
            || path.strip_prefix('/').is_some_and(|suffix| {
                !suffix.is_empty()
                    && !suffix.contains('\\')
                    && suffix
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
            })
    }

    #[test]
    fn slash_rooted_branch_matches_previous_predicate() {
        use crate::verified::storage::{classify_stored_path, PathPlatform};
        for input in generated_strings(&['/', '.', 'a', '\\'], 7) {
            assert_eq!(
                crate::verified::path::is_canonical_abs_path(input.as_bytes(), true),
                previous_slash_rooted_path_is_lexically_normal(&input),
                "{input:?}"
            );
            // A path `std` does not consider absolute is judged by that form.
            if !input.is_empty() && input.starts_with('/') {
                let platform = PathPlatform::Native {
                    absolute: false,
                    normal: false,
                };
                assert_eq!(
                    classify_stored_path(input.as_bytes(), platform) == PathVerdict::Canonical,
                    previous_slash_rooted_path_is_lexically_normal(&input),
                    "{input:?}"
                );
            }
        }
    }

    /// The trusted link between the escaping kernel and
    /// `char::escape_default`, for every `char`.
    #[test]
    fn escape_kernel_matches_std_for_every_char() {
        for code in 0..=u32::from(char::MAX) {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let text = c.to_string();
            let expected: String = c.escape_default().collect();
            assert_eq!(escape_text(&text), expected, "U+{code:04X}");
        }
        let mixed = "pa\"th\\\n\u{1b}[31m\u{0}é\u{10ffff}'";
        assert_eq!(
            escape_text(mixed),
            mixed
                .chars()
                .flat_map(char::escape_default)
                .collect::<String>()
        );
    }

    fn decode_values(values: &[Value]) -> Result<Reservation> {
        let conn = Connection::open_in_memory().unwrap();
        let mut statement = conn.prepare("SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7").unwrap();
        let mut rows = statement
            .query(rusqlite::params_from_iter(values.iter()))
            .unwrap();
        row_to_reservation(rows.next().unwrap().unwrap())
    }

    #[test]
    fn test_overflowing_stored_timestamp_is_typed_corruption_without_unwind() {
        let values = [
            Value::Text("/project".into()),
            Value::Text(String::new()),
            Value::Integer(5000),
            Value::Null,
            Value::Null,
            Value::Integer(1),
            Value::Integer(i64::MAX),
        ];
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_values(&values)));
        let error = outcome
            .expect("stored values must never unwind")
            .unwrap_err();
        assert!(matches!(
            error,
            Error::DatabaseCorruption { details }
                if details.contains("table=reservations")
                    && details.contains("field=last_used_at")
                    && details.contains("path=\"/project\"")
        ));
    }

    #[test]
    fn test_each_sqlite_scalar_type_mismatch_is_typed_without_unwind_or_blob_leak() {
        let valid = [
            Value::Text("/project".into()),
            Value::Text(String::new()),
            Value::Integer(5000),
            Value::Null,
            Value::Null,
            Value::Integer(1),
            Value::Integer(2),
        ];
        for (index, replacement, expected_field) in [
            (0, Value::Blob(b"path-secret".to_vec()), "field=path"),
            (1, Value::Integer(7), "field=tag"),
            (2, Value::Real(5000.0), "field=port"),
            (3, Value::Blob(b"project-secret".to_vec()), "field=project"),
            (4, Value::Integer(9), "field=task"),
            (5, Value::Text("1".into()), "field=created_at"),
            (
                6,
                Value::Blob(b"timestamp-secret".to_vec()),
                "field=last_used_at",
            ),
        ] {
            let mut values = valid.clone();
            values[index] = replacement;
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_values(&values)));
            let error = outcome
                .expect("stored values must never unwind")
                .unwrap_err();
            let Error::DatabaseCorruption { details } = error else {
                panic!("expected typed corruption, got {error:?}");
            };
            assert!(details.contains(expected_field), "{details}");
            assert!(details.contains("found"), "{details}");
            assert!(!details.contains("secret"), "{details}");
        }
    }

    #[cfg(feature = "property-tests")]
    fn sqlite_value_strategy() -> impl Strategy<Value = Value> {
        prop_oneof![
            Just(Value::Null),
            any::<i64>().prop_map(Value::Integer),
            any::<f64>().prop_map(Value::Real),
            any::<String>().prop_map(Value::Text),
            proptest::collection::vec(any::<u8>(), 0..64).prop_map(Value::Blob),
        ]
    }

    #[cfg(feature = "property-tests")]
    proptest! {
        #[test]
        fn arbitrary_sqlite_scalars_never_unwind(
            values in proptest::collection::vec(sqlite_value_strategy(), 7)
        ) {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode_values(&values)
            }));
            prop_assert!(outcome.is_ok());
        }
    }

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
    fn test_delete_reservation_if_unchanged_preserves_refreshed_row() {
        let mut db = create_test_database();
        let reservation = create_test_reservation("/test/refreshed", 5000);
        db.create_reservation(&reservation).unwrap();
        let snapshot = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();

        let transaction = db.begin_transaction().unwrap();
        transaction
            .execute(
                "UPDATE reservations
                 SET last_used_at = last_used_at + 1
                 WHERE path = ? AND tag = ?",
                params![
                    snapshot.key().storage_key().unwrap().path(),
                    snapshot.key().storage_key().unwrap().tag()
                ],
            )
            .unwrap();

        assert!(!Database::delete_reservation_if_unchanged(&transaction, &snapshot).unwrap());
        assert!(Database::get_reservation(&transaction, snapshot.key())
            .unwrap()
            .is_some());
        transaction.commit().unwrap();
    }

    /// The metadata update validates the key, project and task before any
    /// SQL, exactly as the row decoder will read them back.
    #[test]
    fn metadata_update_validates_before_any_write() {
        let db = create_test_database();
        let reservation = create_test_reservation("/test/metadata", 5000);
        Database::create_reservation_simple(db.connection(), &reservation).unwrap();
        let before = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        let now = SystemTime::now();
        let update = |key: &ReservationKey, project: Option<&str>, task: Option<&str>| {
            Database::update_metadata_and_last_used_simple(db.connection(), key, project, task, now)
        };
        let field = |result: Result<bool>| match result {
            Err(Error::Validation { field, .. }) => field,
            other => panic!("expected a validation error, got {other:?}"),
        };

        let unstorable = [
            (
                ReservationKey {
                    path: PathBuf::from("/test/metadata/"),
                    tag: None,
                },
                "path",
            ),
            (
                ReservationKey {
                    path: PathBuf::from("relative"),
                    tag: None,
                },
                "path",
            ),
            (
                ReservationKey {
                    path: PathBuf::from("/test/metadata"),
                    tag: Some(" web ".into()),
                },
                "tag",
            ),
            (
                ReservationKey {
                    path: PathBuf::from("/test/metadata"),
                    tag: Some(String::new()),
                },
                "tag",
            ),
        ];
        for (key, expected) in &unstorable {
            assert_eq!(field(update(key, Some("p"), Some("t"))), *expected);
        }
        for (project, task, expected) in [
            (Some(" p "), None, "project"),
            (Some(""), None, "project"),
            (None, Some("t "), "task"),
            (Some("p"), Some("   "), "task"),
            // The key is checked first, then the project, then the task.
            (Some(" p "), Some(" t "), "project"),
        ] {
            assert_eq!(field(update(reservation.key(), project, task)), expected);
        }
        let key = ReservationKey {
            path: PathBuf::from("/test/metadata/"),
            tag: None,
        };
        assert_eq!(field(update(&key, Some(" p "), None)), "path");

        // Nothing changed, and the row still decodes.
        let after = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        assert_eq!(after.project(), before.project());
        assert_eq!(after.task(), before.task());
        assert_eq!(after.last_used_at(), before.last_used_at());

        // A storable update goes through.
        assert!(update(reservation.key(), Some("p"), Some("t")).unwrap());
        let updated = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        assert_eq!(updated.project(), Some("p"));
        assert_eq!(updated.task(), Some("t"));
    }

    /// The metadata update's last use goes through the same timestamp codec
    /// as inserts (`encode_metadata_update`): the storable boundaries are
    /// written and read back exactly, one second beyond is rejected before
    /// any write, and clearing project and task writes NULLs.
    #[test]
    fn metadata_update_last_use_boundaries() {
        use crate::verified::storage::MAX_STORABLE_UNIX_SECS;
        use std::time::{Duration, UNIX_EPOCH};

        let db = create_test_database();
        let reservation = create_test_reservation("/test/metadata", 5000);
        Database::create_reservation_simple(db.connection(), &reservation).unwrap();
        let key = reservation.key();
        let update = |project: Option<&str>, task: Option<&str>, secs: u64| {
            Database::update_metadata_and_last_used_simple(
                db.connection(),
                key,
                project,
                task,
                UNIX_EPOCH + Duration::from_secs(secs),
            )
        };
        let stored_last_use = || -> i64 {
            db.connection()
                .query_row(
                    "SELECT last_used_at FROM reservations WHERE path = '/test/metadata'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        };

        for secs in [0, 1, MAX_STORABLE_UNIX_SECS] {
            assert!(update(Some("p"), Some("t"), secs).unwrap(), "{secs}");
            assert_eq!(stored_last_use(), i64::try_from(secs).unwrap());
            assert!(Database::get_reservation(db.connection(), key)
                .unwrap()
                .is_some());
        }
        match update(Some("p"), Some("t"), MAX_STORABLE_UNIX_SECS + 1) {
            Err(Error::Validation { field, .. }) => assert_eq!(field, "timestamp"),
            other => panic!("expected a timestamp validation error, got {other:?}"),
        }
        // The project is checked before the last use.
        match update(Some(" p"), None, MAX_STORABLE_UNIX_SECS + 1) {
            Err(Error::Validation { field, .. }) => assert_eq!(field, "project"),
            other => panic!("expected a project validation error, got {other:?}"),
        }
        assert_eq!(
            stored_last_use(),
            i64::try_from(MAX_STORABLE_UNIX_SECS).unwrap()
        );

        assert!(update(None, None, 7).unwrap());
        let cleared = Database::get_reservation(db.connection(), key)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.project(), None);
        assert_eq!(cleared.task(), None);
        assert_eq!(stored_last_use(), 7);
    }

    /// Pins the error order for a last use before the epoch: after the key,
    /// before the project and task (it has no `u64` seconds to hand the
    /// kernel). Nothing is written.
    #[test]
    fn metadata_update_reports_a_pre_epoch_last_use_after_the_key() {
        use std::time::{Duration, UNIX_EPOCH};

        let db = create_test_database();
        let reservation = create_test_reservation("/test/metadata", 5000);
        Database::create_reservation_simple(db.connection(), &reservation).unwrap();
        let before = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        let pre_epoch = UNIX_EPOCH - Duration::from_secs(1);
        let field = |key: &ReservationKey, project: Option<&str>, task: Option<&str>| {
            match Database::update_metadata_and_last_used_simple(
                db.connection(),
                key,
                project,
                task,
                pre_epoch,
            ) {
                Err(Error::Validation { field, .. }) => field,
                other => panic!("expected a validation error, got {other:?}"),
            }
        };
        let unstorable_key = ReservationKey {
            path: PathBuf::from("/test/metadata/"),
            tag: None,
        };

        assert_eq!(field(&unstorable_key, Some(" p"), None), "path");
        assert_eq!(field(reservation.key(), Some(" p"), None), "timestamp");
        assert_eq!(field(reservation.key(), None, Some(" t")), "timestamp");
        assert_eq!(field(reservation.key(), Some("p"), Some("t")), "timestamp");
        assert_eq!(
            Database::get_reservation(db.connection(), reservation.key())
                .unwrap()
                .unwrap(),
            before
        );
    }

    /// Exact-path lookups match the path's own text: a corrupt row stored
    /// under a non-storable path's text is reported, and a path that is not
    /// UTF-8 (no exact text) finds nothing.
    #[test]
    fn exact_path_lookups_report_corrupt_rows() {
        let db = create_test_database();
        db.connection()
            .execute(
                "INSERT INTO reservations VALUES ('/a/b/', 'web', 5001, NULL, NULL, 1, 1)",
                [],
            )
            .unwrap();
        for result in [
            Database::get_reservations_by_exact_path(db.connection(), Path::new("/a/b/")),
            Database::get_tagged_reservations_by_exact_path(db.connection(), Path::new("/a/b/")),
        ] {
            assert!(matches!(result, Err(Error::DatabaseCorruption { .. })));
        }
        assert_eq!(
            Database::get_reservations_by_exact_path(db.connection(), Path::new("/a/b"))
                .unwrap()
                .len(),
            0
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(std::ffi::OsStr::from_bytes(b"/a/b/\xff"));
            assert_eq!(
                Database::get_reservations_by_exact_path(db.connection(), path)
                    .unwrap()
                    .len(),
                0
            );
        }
    }

    /// The guarded delete encodes its snapshot with the verified kernel: a
    /// snapshot the database could not hold is a validation error, and
    /// nothing is deleted.
    #[test]
    fn guarded_delete_validates_its_snapshot() {
        let db = create_test_database();
        let reservation = create_test_reservation("/test/guarded", 5000);
        Database::create_reservation_simple(db.connection(), &reservation).unwrap();
        let stored = Database::get_reservation(db.connection(), reservation.key())
            .unwrap()
            .unwrap();
        let rebuilt = |key: ReservationKey, project: Option<&str>| {
            Reservation::builder(key, stored.port())
                .created_at(stored.created_at())
                .last_used_at(stored.last_used_at())
                .build()
                .map(|reservation| {
                    // Bypass the builder's checks, as deserialization can.
                    let mut value = serde_json::to_value(&reservation).unwrap();
                    value["project"] = project.map_or(serde_json::Value::Null, Into::into);
                    serde_json::from_value::<Reservation>(value).unwrap()
                })
                .unwrap()
        };
        let snapshots = [
            (
                rebuilt(
                    ReservationKey {
                        path: PathBuf::from("/test/guarded/"),
                        tag: None,
                    },
                    None,
                ),
                "path",
            ),
            (
                rebuilt(
                    ReservationKey {
                        path: PathBuf::from("/test/guarded"),
                        tag: Some(String::new()),
                    },
                    None,
                ),
                "tag",
            ),
            (rebuilt(reservation.key().clone(), Some(" p ")), "project"),
        ];
        for (snapshot, expected) in &snapshots {
            match Database::delete_reservation_if_unchanged(db.connection(), snapshot) {
                Err(Error::Validation { field, .. }) => assert_eq!(field, *expected),
                other => panic!("expected a validation error, got {other:?}"),
            }
        }
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            1
        );

        // The exact snapshot still deletes.
        assert!(Database::delete_reservation_if_unchanged(db.connection(), &stored).unwrap());
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            0
        );
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
    fn test_create_reservation_port_conflict_does_not_replace_other_key() {
        let mut db = create_test_database();

        let port = Port::try_from(5000).unwrap();
        let key1 = ReservationKey::new(PathBuf::from("/path1"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/path2"), None).unwrap();
        let r1 = Reservation::builder(key1.clone(), port).build().unwrap();
        let r2 = Reservation::builder(key2.clone(), port).build().unwrap();

        db.create_reservation(&r1).unwrap();
        assert!(db.create_reservation(&r2).is_err());

        let loaded1 = Database::get_reservation(db.connection(), &key1)
            .unwrap()
            .unwrap();
        assert_eq!(loaded1.port(), port);
        assert!(Database::get_reservation(db.connection(), &key2)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_create_reservation_simple_port_conflict_preserves_original_key() {
        let mut db = create_test_database();

        let old_port = Port::try_from(5000).unwrap();
        let conflicting_port = Port::try_from(5001).unwrap();
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let other_key = ReservationKey::new(PathBuf::from("/other"), None).unwrap();
        let original = Reservation::builder(key.clone(), old_port).build().unwrap();
        let other = Reservation::builder(other_key, conflicting_port)
            .build()
            .unwrap();
        let conflicting_update = Reservation::builder(key.clone(), conflicting_port)
            .build()
            .unwrap();

        db.create_reservation(&original).unwrap();
        db.create_reservation(&other).unwrap();

        assert!(Database::create_reservation_simple(db.connection(), &conflicting_update).is_err());

        let loaded = Database::get_reservation(db.connection(), &key)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.port(), old_port);
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
    fn test_get_reservations_by_path_prefix_is_component_aware() {
        let mut db = create_test_database();

        db.create_reservation(&create_test_reservation("/projects/foo", 5000))
            .unwrap();
        db.create_reservation(&create_test_reservation("/projects/foo/child", 5001))
            .unwrap();
        db.create_reservation(&create_test_reservation("/projects/foobar", 5002))
            .unwrap();

        let reservations =
            Database::get_reservations_by_path_prefix(db.connection(), Path::new("/projects/foo"))
                .unwrap();

        let paths = reservations
            .iter()
            .map(|reservation| reservation.key().path.as_path())
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&Path::new("/projects/foo")));
        assert!(paths.contains(&Path::new("/projects/foo/child")));
        assert!(!paths.contains(&Path::new("/projects/foobar")));
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
    fn test_validate_path_relationship_canonical_same() {
        let _db = create_test_database();
        let cwd = env::current_dir().unwrap();
        let canonical_cwd = crate::path::canonicalize::canonicalize(&cwd).unwrap();

        let result = Database::validate_path_relationship(&canonical_cwd, false);
        assert!(
            result.is_ok(),
            "canonical and process spellings of the current directory must be related: {result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_path_relationship_canonical_nonexistent_descendant() {
        use std::os::unix::fs::symlink;

        let _db = create_test_database();
        let cwd = env::current_dir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let cwd_link = temp.path().join("cwd-link");
        symlink(&cwd, &cwd_link).unwrap();
        let nonexistent_descendant = cwd_link.join("does-not-exist");

        let result = Database::validate_path_relationship(&nonexistent_descendant, false);
        assert!(
            result.is_ok(),
            "a nonexistent descendant reached through a canonical equivalent must be related: {result:?}"
        );
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
