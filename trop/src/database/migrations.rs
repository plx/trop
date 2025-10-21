//! Database schema management and migrations.
//!
//! This module handles database schema initialization, version checking,
//! and migrations.

use rusqlite::Connection;

use crate::error::{Error, Result};

use super::schema::{
    CREATE_LAST_USED_INDEX, CREATE_METADATA_TABLE, CREATE_PORT_INDEX, CREATE_PROJECT_INDEX,
    CREATE_RESERVATIONS_TABLE, CURRENT_SCHEMA_VERSION, INSERT_SCHEMA_VERSION,
    SELECT_SCHEMA_VERSION,
};

/// Initializes the database schema.
///
/// This function creates all tables, indices, and metadata for a fresh
/// database. It should only be called on a database that has not been
/// initialized yet.
///
/// # Errors
///
/// Returns an error if any SQL statement fails to execute.
///
/// # Examples
///
/// ```no_run
/// use rusqlite::Connection;
/// use trop::database::migrations::initialize_schema;
///
/// let conn = Connection::open_in_memory().unwrap();
/// initialize_schema(&conn).unwrap();
/// ```
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    // Create metadata table
    conn.execute(CREATE_METADATA_TABLE, [])?;

    // Create reservations table
    conn.execute(CREATE_RESERVATIONS_TABLE, [])?;

    // Create indices
    conn.execute(CREATE_PORT_INDEX, [])?;
    conn.execute(CREATE_PROJECT_INDEX, [])?;
    conn.execute(CREATE_LAST_USED_INDEX, [])?;

    // Set initial schema version
    conn.execute(INSERT_SCHEMA_VERSION, [CURRENT_SCHEMA_VERSION])?;

    Ok(())
}

/// Gets the current schema version from the database.
///
/// # Errors
///
/// Returns an error if the query fails for reasons other than
/// "no rows returned" (which indicates version 0).
///
/// # Returns
///
/// - `Ok(0)` if the metadata table doesn't exist or has no version
/// - `Ok(version)` if a version is found
/// - `Err(_)` if a database error occurs
pub fn get_schema_version(conn: &Connection) -> Result<i32> {
    match conn.query_row(SELECT_SCHEMA_VERSION, [], |row| {
        let value: String = row.get(0)?;
        value
            .parse::<i32>()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
    }) {
        Ok(version) => Ok(version),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Database exists but no schema - needs initialization
            Ok(0)
        }
        Err(e) => {
            // Check if it's a "no such table" error
            if let rusqlite::Error::SqliteFailure(ref sqlite_err, _) = e {
                if sqlite_err.code == rusqlite::ErrorCode::Unknown {
                    // Table doesn't exist yet
                    return Ok(0);
                }
            }
            Err(e.into())
        }
    }
}

/// Checks schema compatibility and initializes if needed.
///
/// This function:
/// 1. Checks the current schema version
/// 2. If version is 0, initializes the schema
/// 3. If version is older than current, returns an error (migrations needed)
/// 4. If version is newer than current, returns an error (client too old)
/// 5. If version matches, applies any necessary schema fixes
///
/// # Errors
///
/// Returns an error if:
/// - Schema version is incompatible (too old or too new)
/// - Schema initialization fails
/// - Database queries fail
///
/// # Examples
///
/// ```no_run
/// use rusqlite::Connection;
/// use trop::database::migrations::check_schema_compatibility;
///
/// let conn = Connection::open_in_memory().unwrap();
/// check_schema_compatibility(&conn).unwrap();
/// ```
pub fn check_schema_compatibility(conn: &Connection) -> Result<()> {
    let version = get_schema_version(conn)?;

    if version == 0 {
        // Fresh database, initialize it
        initialize_schema(conn)?;
    } else if version < CURRENT_SCHEMA_VERSION {
        // Database is older than current version
        // In the future, we would apply migrations here
        return Err(Error::Validation {
            field: "schema_version".into(),
            message: format!(
                "Database schema version {version} is older than client version {CURRENT_SCHEMA_VERSION}. Migration not yet implemented."
            ),
        });
    } else if version > CURRENT_SCHEMA_VERSION {
        // Database is newer than client can handle
        return Err(Error::Validation {
            field: "schema_version".into(),
            message: format!(
                "Database schema version {version} is newer than client version {CURRENT_SCHEMA_VERSION}. Please upgrade trop."
            ),
        });
    } else {
        // Version matches - apply any necessary schema fixes for databases
        // created before critical bug fixes (e.g., UNIQUE constraint on port)
        apply_schema_fixes_v1(conn)?;
    }

    Ok(())
}

/// Applies schema fixes for version 1 databases.
///
/// This function checks if the port column has a UNIQUE constraint and
/// adds it if missing. This is necessary for databases created before
/// the concurrency bug fix that added the UNIQUE constraint.
///
/// The migration approach is conservative:
/// - First checks if any duplicate ports exist
/// - If duplicates exist, returns an error requiring manual cleanup
/// - If no duplicates, recreates the table with the UNIQUE constraint
///
/// # Errors
///
/// Returns an error if:
/// - Duplicate ports are found in the database
/// - The migration SQL fails to execute
fn apply_schema_fixes_v1(conn: &Connection) -> Result<()> {
    // Simple read-only check using pragma_data_version (always succeeds on readonly)
    // If we can write, test with an actual write operation
    let is_readonly = conn.execute("BEGIN IMMEDIATE", []).is_err();

    // If we started a transaction, roll it back
    let _ = conn.execute("ROLLBACK", []);

    if is_readonly {
        // Can't modify a read-only database, skip migration
        return Ok(());
    }

    // Check if port column already has UNIQUE constraint
    // Use sqlite_master instead of pragma functions to avoid issues
    let has_unique_constraint: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master
             WHERE type = 'table'
             AND name = 'reservations'
             AND sql LIKE '%port INTEGER NOT NULL UNIQUE%'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if has_unique_constraint {
        // Already has the constraint, nothing to do
        return Ok(());
    }

    // Check for duplicate ports before migration
    let duplicate_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM (
            SELECT port FROM reservations GROUP BY port HAVING COUNT(*) > 1
        )",
        [],
        |row| row.get(0),
    )?;

    if duplicate_count > 0 {
        return Err(Error::DatabaseCorruption {
            details: format!(
                "Database contains {duplicate_count} duplicate port allocation(s). \
                 Please run 'trop cleanup' to remove duplicates before upgrading, \
                 or delete the database and recreate your reservations."
            ),
        });
    }

    // Recreate table with UNIQUE constraint
    // SQLite doesn't support adding UNIQUE constraints to existing columns,
    // so we need to recreate the table
    conn.execute_batch(
        "BEGIN TRANSACTION;

         -- Create new table with UNIQUE constraint
         CREATE TABLE reservations_new (
             path TEXT NOT NULL,
             tag TEXT,
             port INTEGER NOT NULL UNIQUE,
             project TEXT,
             task TEXT,
             created_at INTEGER NOT NULL,
             last_used_at INTEGER NOT NULL,
             PRIMARY KEY (path, tag)
         );

         -- Copy data from old table
         INSERT INTO reservations_new
         SELECT path, tag, port, project, task, created_at, last_used_at
         FROM reservations;

         -- Drop old table
         DROP TABLE reservations;

         -- Rename new table
         ALTER TABLE reservations_new RENAME TO reservations;

         -- Recreate indices
         CREATE INDEX IF NOT EXISTS idx_reservations_port ON reservations(port);
         CREATE INDEX IF NOT EXISTS idx_reservations_project ON reservations(project);
         CREATE INDEX IF NOT EXISTS idx_reservations_last_used ON reservations(last_used_at);

         COMMIT;",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_connection() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn test_initialize_schema() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        // Verify metadata table exists and has version
        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);

        // Verify reservations table exists
        let count: i32 = conn
            .query_row("SELECT COUNT(*) FROM reservations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_get_schema_version_uninitialized() {
        let conn = create_test_connection();
        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, 0);
    }

    #[test]
    fn test_get_schema_version_initialized() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_check_schema_compatibility_fresh_database() {
        let conn = create_test_connection();

        // Should initialize the schema
        check_schema_compatibility(&conn).unwrap();

        // Verify it was initialized
        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_check_schema_compatibility_current_version() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        // Should succeed with current version
        check_schema_compatibility(&conn).unwrap();
    }

    #[test]
    fn test_check_schema_compatibility_newer_version() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        // Manually set a newer version
        conn.execute(
            "UPDATE metadata SET value = '999' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        // Should fail with version too new
        let result = check_schema_compatibility(&conn);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("newer than client"));
    }

    #[test]
    fn test_check_schema_compatibility_older_version() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        // Manually set an older version (if current version > 1)
        if CURRENT_SCHEMA_VERSION > 1 {
            conn.execute(
                "UPDATE metadata SET value = '0' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();

            // Should fail with version too old
            let result = check_schema_compatibility(&conn);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.to_string().contains("older than client"));
        }
    }

    #[test]
    fn test_schema_creates_all_indices() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        // Query for index existence
        let index_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_reservations_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // We should have 3 indices (port, project, last_used)
        assert_eq!(index_count, 3);
    }
}
