//! Database schema definitions and SQL constants.
//!
//! This module contains all SQL table definitions, indices, and constants
//! related to the database schema for the trop reservation system.

/// Current schema version for the database.
///
/// This version is stored in the metadata table and is used to ensure
/// compatibility between the database and the application.
pub const CURRENT_SCHEMA_VERSION: i32 = 1;

/// SQL statement to create the metadata table.
///
/// The metadata table stores key-value pairs for database configuration
/// and versioning information.
pub const CREATE_METADATA_TABLE: &str = r"
    CREATE TABLE IF NOT EXISTS metadata (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    )";

/// SQL statement to create the reservations table.
///
/// The reservations table stores all port reservations with their associated
/// metadata. The primary key is the combination of (path, tag) to ensure
/// uniqueness of reservations. The port column has a UNIQUE constraint to
/// prevent duplicate port allocations under concurrent load.
pub const CREATE_RESERVATIONS_TABLE: &str = r"
    CREATE TABLE IF NOT EXISTS reservations (
        path TEXT NOT NULL,
        tag TEXT,
        port INTEGER NOT NULL UNIQUE,
        project TEXT,
        task TEXT,
        created_at INTEGER NOT NULL,
        last_used_at INTEGER NOT NULL,
        PRIMARY KEY (path, tag)
    )";

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

/// SQL statement to insert or replace a reservation.
///
/// Used by both single and batch create operations.
pub const INSERT_RESERVATION: &str = r"
    INSERT OR REPLACE INTO reservations
    (path, tag, port, project, task, created_at, last_used_at)
    VALUES (?, ?, ?, ?, ?, ?, ?)
";

/// SQL statement to delete a reservation by key.
///
/// Used by both single and batch delete operations.
pub const DELETE_RESERVATION: &str = r"
    DELETE FROM reservations
    WHERE path = ? AND tag IS ?
";
