//! Database schema definitions and SQL constants.
//!
//! This module contains all SQL table definitions, indices, and constants
//! related to the database schema for the trop reservation system.

/// Current schema version for the database.
///
/// This version is stored in the metadata table and is used to ensure
/// compatibility between the database and the application.
pub const CURRENT_SCHEMA_VERSION: i32 = 2;

/// SQL statement to create the metadata table.
///
/// The metadata table stores key-value pairs for database configuration
/// and versioning information.
pub const CREATE_METADATA_TABLE: &str = r"
    CREATE TABLE IF NOT EXISTS metadata (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) STRICT";

/// SQL statement to create the reservations table.
///
/// The reservations table stores all port reservations with their associated
/// metadata. Schema v2 stores an absent tag as the empty string, which is not a
/// valid domain tag, so both tagged and untagged identities participate in the
/// non-null primary key. The path representation remains unchanged pending the
/// separate cross-platform path-storage contract.
pub const CREATE_RESERVATIONS_TABLE: &str = r"
    CREATE TABLE IF NOT EXISTS reservations (
        path TEXT NOT NULL,
        tag TEXT NOT NULL,
        port INTEGER NOT NULL UNIQUE
            CONSTRAINT valid_port CHECK (port BETWEEN 1 AND 65535),
        project TEXT,
        task TEXT,
        created_at INTEGER NOT NULL
            CONSTRAINT valid_created_at CHECK (created_at >= 0),
        last_used_at INTEGER NOT NULL
            CONSTRAINT valid_last_used_at CHECK (last_used_at >= 0),
        PRIMARY KEY (path, tag)
    ) STRICT";

/// SQL statement to create an index on the port column.
///
/// This index speeds up queries for port availability and allocation.
pub const CREATE_PORT_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_reservations_port ON reservations(port)";

/// SQL statement to create an index on the project column.
///
/// This index speeds up filtered lists by project.
pub const CREATE_PROJECT_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_reservations_project ON reservations(project)";

/// SQL statement to create an index on the `last_used_at` column.
///
/// This index speeds up cleanup operations that search for expired reservations.
pub const CREATE_LAST_USED_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS idx_reservations_last_used ON reservations(last_used_at)";

/// SQL statement to select the schema version from the metadata table.
pub const SELECT_SCHEMA_VERSION: &str = "SELECT value FROM metadata WHERE key = 'schema_version'";

/// SQL statement to insert or update the schema version in the metadata table.
pub const INSERT_SCHEMA_VERSION: &str =
    "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?)";

/// SQL statement to insert a reservation.
///
/// Callers delete the same reservation key before inserting when replacement is
/// intended. This must stay as a plain insert so a unique-port conflict with a
/// different reservation key fails instead of deleting that unrelated row.
pub const INSERT_RESERVATION: &str = r"
    INSERT INTO reservations
    (path, tag, port, project, task, created_at, last_used_at)
    VALUES (?, ?, ?, ?, ?, ?, ?)
";

/// SQL statement to delete a reservation by key.
///
/// Used by both single and batch delete operations.
pub const DELETE_RESERVATION: &str = r"
    DELETE FROM reservations
    WHERE path = ? AND tag = ?
";

/// Encodes the domain-level optional tag in schema v2's non-null storage form.
#[must_use]
pub(super) fn encode_tag(tag: Option<&str>) -> &str {
    tag.unwrap_or("")
}
