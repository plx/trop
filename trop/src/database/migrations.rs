//! Database schema initialization, compatibility checks, and ordered migrations.

use std::fmt::Write as _;

use rusqlite::{Connection, DatabaseName, OptionalExtension, Transaction, TransactionBehavior};

use crate::error::{Error, Result};

use super::schema::{
    CREATE_LAST_USED_INDEX, CREATE_METADATA_TABLE, CREATE_PORT_INDEX, CREATE_PROJECT_INDEX,
    CREATE_RESERVATIONS_TABLE, CURRENT_SCHEMA_VERSION, INSERT_SCHEMA_VERSION,
    SELECT_SCHEMA_VERSION,
};

const PUBLISHED_SCHEMA_VERSION: i32 = 1;

const CREATE_METADATA_V2_STAGING: &str = r"
    CREATE TABLE metadata_v2 (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) STRICT";

const CREATE_RESERVATIONS_V2_STAGING: &str = r"
    CREATE TABLE reservations_v2 (
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

const EXPECTED_V1_METADATA_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("key", "TEXT", true, 1),
    ColumnSpec::new("value", "TEXT", true, 0),
];

const EXPECTED_V1_RESERVATION_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("path", "TEXT", true, 1),
    ColumnSpec::new("tag", "TEXT", false, 2),
    ColumnSpec::new("port", "INTEGER", true, 0),
    ColumnSpec::new("project", "TEXT", false, 0),
    ColumnSpec::new("task", "TEXT", false, 0),
    ColumnSpec::new("created_at", "INTEGER", true, 0),
    ColumnSpec::new("last_used_at", "INTEGER", true, 0),
];

const EXPECTED_V2_METADATA_COLUMNS: &[ColumnSpec] = EXPECTED_V1_METADATA_COLUMNS;

const EXPECTED_V2_RESERVATION_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("path", "TEXT", true, 1),
    ColumnSpec::new("tag", "TEXT", true, 2),
    ColumnSpec::new("port", "INTEGER", true, 0),
    ColumnSpec::new("project", "TEXT", false, 0),
    ColumnSpec::new("task", "TEXT", false, 0),
    ColumnSpec::new("created_at", "INTEGER", true, 0),
    ColumnSpec::new("last_used_at", "INTEGER", true, 0),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnSpec {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
}

impl ColumnSpec {
    const fn new(
        name: &'static str,
        declared_type: &'static str,
        not_null: bool,
        primary_key_position: i64,
    ) -> Self {
        Self {
            name,
            declared_type,
            not_null,
            primary_key_position,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualColumn {
    name: String,
    declared_type: String,
    not_null: bool,
    primary_key_position: i64,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LegacyViolations {
    duplicate_logical_keys: i64,
    duplicate_ports: i64,
    empty_tags: i64,
    invalid_ports: i64,
    invalid_timestamps: i64,
    invalid_scalar_types: i64,
    invalid_metadata_scalars: i64,
}

impl LegacyViolations {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    fn recovery_details(&self) -> String {
        let mut details = String::from(
            "schema v1 cannot be migrated safely because the following legacy \
             invariants are violated:",
        );
        for (label, count) in [
            (
                "duplicate logical reservation keys",
                self.duplicate_logical_keys,
            ),
            ("duplicate reserved ports", self.duplicate_ports),
            ("empty legacy tags", self.empty_tags),
            ("ports outside 1..=65535", self.invalid_ports),
            ("negative timestamps", self.invalid_timestamps),
            (
                "invalid reservation scalar types",
                self.invalid_scalar_types,
            ),
            (
                "invalid metadata scalar types",
                self.invalid_metadata_scalars,
            ),
        ] {
            if count > 0 {
                let _ = write!(details, " {label}={count};");
            }
        }
        details.push_str(
            " the original schema-v1 database was left unchanged. Make a copy, \
             inspect the reported categories with sqlite3, and either correct \
             every conflict manually or delete the disposable database and \
             recreate reservations. trop will not choose or discard legacy rows.",
        );
        details
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationPhase {
    Locked,
    PreflightComplete,
    StagingTablesCreated,
    RowsCopied,
    OldTablesDropped,
    StagingTablesRenamed,
    VersionUpdated,
    BeforeCommit,
    AfterCommit,
}

/// Initializes a fresh database directly at the current schema version.
///
/// Initialization is atomic: a failed statement leaves the database
/// uninitialized rather than exposing a partial schema.
///
/// # Errors
///
/// Returns a typed migration error when the database is read-only, or a
/// database error when schema creation, validation, or commit fails.
pub fn initialize_schema(conn: &Connection) -> Result<()> {
    if conn.is_readonly(DatabaseName::Main)? {
        return Err(Error::MigrationRequired {
            found: 0,
            target: CURRENT_SCHEMA_VERSION,
            action: "open or copy the database to a writable location before initialization".into(),
        });
    }

    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    initialize_schema_in_transaction(&tx)?;
    validate_schema_v2(&tx)?;
    tx.commit()?;
    Ok(())
}

fn initialize_schema_in_transaction(conn: &Connection) -> Result<()> {
    conn.execute(CREATE_METADATA_TABLE, [])?;
    conn.execute(CREATE_RESERVATIONS_TABLE, [])?;
    conn.execute(CREATE_PORT_INDEX, [])?;
    conn.execute(CREATE_PROJECT_INDEX, [])?;
    conn.execute(CREATE_LAST_USED_INDEX, [])?;
    conn.execute(INSERT_SCHEMA_VERSION, [CURRENT_SCHEMA_VERSION])?;
    Ok(())
}

/// Gets the schema version stored in the metadata table.
///
/// Version zero denotes an uninitialized database.
///
/// # Errors
///
/// Returns a database error when schema metadata exists but cannot be read as
/// the integer version expected by trop.
pub fn get_schema_version(conn: &Connection) -> Result<i32> {
    match conn.query_row(SELECT_SCHEMA_VERSION, [], |row| {
        let value: String = row.get(0)?;
        value
            .parse::<i32>()
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    }) {
        Ok(version) => Ok(version),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
        Err(error) => {
            if let rusqlite::Error::SqliteFailure(ref sqlite_error, _) = error {
                if sqlite_error.code == rusqlite::ErrorCode::Unknown {
                    return Ok(0);
                }
            }
            Err(error.into())
        }
    }
}

/// Initializes, migrates, or verifies a database for this client.
///
/// Schema v1 is the only released predecessor. Its migration to v2 is ordered,
/// fail-closed, and protected by one immediate transaction.
///
/// # Errors
///
/// Returns a typed compatibility error for read-only legacy databases,
/// unsupported future versions, and legacy data that cannot be migrated
/// without choosing or discarding rows. Database and integrity failures are
/// also returned without leaving a partially migrated schema.
pub fn check_schema_compatibility(conn: &Connection) -> Result<()> {
    check_schema_compatibility_with_observer(conn, |_| Ok(()))
}

fn check_schema_compatibility_with_observer(
    conn: &Connection,
    mut observer: impl FnMut(MigrationPhase) -> Result<()>,
) -> Result<()> {
    let version = get_schema_version(conn)?;

    if version == 0 {
        return initialize_schema(conn);
    }
    if version == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    if version > CURRENT_SCHEMA_VERSION {
        return Err(Error::UnsupportedSchemaVersion {
            expected: u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or_default(),
            found: u32::try_from(version).unwrap_or(u32::MAX),
        });
    }
    if version != PUBLISHED_SCHEMA_VERSION {
        return Err(Error::MigrationRequired {
            found: version,
            target: CURRENT_SCHEMA_VERSION,
            action: "no ordered migration is available for this older schema version".into(),
        });
    }
    if conn.is_readonly(DatabaseName::Main)? {
        return Err(Error::MigrationRequired {
            found: version,
            target: CURRENT_SCHEMA_VERSION,
            action: "copy the database to a writable location or reopen it read-write; \
                     schema-v1 data was not modified"
                .into(),
        });
    }

    let original_synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    conn.pragma_update(None, "synchronous", "FULL")?;

    let migration_result = migrate_v1_to_v2(conn, &mut observer);
    let restore_result = conn.pragma_update(None, "synchronous", original_synchronous);

    match (migration_result, restore_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn migrate_v1_to_v2(
    conn: &Connection,
    observer: &mut impl FnMut(MigrationPhase) -> Result<()>,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    observer(MigrationPhase::Locked)?;

    // A second migrator may have waited for the first one's commit.
    let locked_version = get_schema_version(&tx)?;
    if locked_version == CURRENT_SCHEMA_VERSION {
        tx.rollback()?;
        return Ok(());
    }
    if locked_version > CURRENT_SCHEMA_VERSION {
        return Err(Error::UnsupportedSchemaVersion {
            expected: u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or_default(),
            found: u32::try_from(locked_version).unwrap_or(u32::MAX),
        });
    }
    if locked_version != PUBLISHED_SCHEMA_VERSION {
        return Err(Error::MigrationRequired {
            found: locked_version,
            target: CURRENT_SCHEMA_VERSION,
            action: "the schema version changed while waiting for the migration lock".into(),
        });
    }

    validate_published_v1_layout(&tx)?;
    let violations = collect_legacy_violations(&tx)?;
    if !violations.is_empty() {
        return Err(Error::MigrationBlocked {
            from: PUBLISHED_SCHEMA_VERSION,
            to: CURRENT_SCHEMA_VERSION,
            details: violations.recovery_details(),
        });
    }
    observer(MigrationPhase::PreflightComplete)?;

    let reservation_count: i64 =
        tx.query_row("SELECT COUNT(*) FROM reservations", [], |row| row.get(0))?;
    let metadata_count: i64 =
        tx.query_row("SELECT COUNT(*) FROM metadata", [], |row| row.get(0))?;

    tx.execute(CREATE_METADATA_V2_STAGING, [])?;
    tx.execute(CREATE_RESERVATIONS_V2_STAGING, [])?;
    observer(MigrationPhase::StagingTablesCreated)?;

    tx.execute(
        "INSERT INTO metadata_v2 (key, value)
         SELECT key, value FROM metadata",
        [],
    )?;
    tx.execute(
        "INSERT INTO reservations_v2
         (path, tag, port, project, task, created_at, last_used_at)
         SELECT path, COALESCE(tag, ''), port, project, task, created_at, last_used_at
         FROM reservations",
        [],
    )?;
    observer(MigrationPhase::RowsCopied)?;

    ensure_copied_count(&tx, "metadata_v2", metadata_count)?;
    ensure_copied_count(&tx, "reservations_v2", reservation_count)?;

    tx.execute("DROP TABLE reservations", [])?;
    tx.execute("DROP TABLE metadata", [])?;
    observer(MigrationPhase::OldTablesDropped)?;

    tx.execute("ALTER TABLE reservations_v2 RENAME TO reservations", [])?;
    tx.execute("ALTER TABLE metadata_v2 RENAME TO metadata", [])?;
    observer(MigrationPhase::StagingTablesRenamed)?;

    tx.execute(CREATE_PORT_INDEX, [])?;
    tx.execute(CREATE_PROJECT_INDEX, [])?;
    tx.execute(CREATE_LAST_USED_INDEX, [])?;

    let updated = tx.execute(
        "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
        [CURRENT_SCHEMA_VERSION],
    )?;
    if updated != 1 {
        return Err(Error::MigrationBlocked {
            from: PUBLISHED_SCHEMA_VERSION,
            to: CURRENT_SCHEMA_VERSION,
            details: "schema-version metadata was not updated exactly once; \
                      the migration was rolled back"
                .into(),
        });
    }
    observer(MigrationPhase::VersionUpdated)?;

    validate_schema_v2(&tx)?;
    observer(MigrationPhase::BeforeCommit)?;
    tx.commit()?;
    observer(MigrationPhase::AfterCommit)?;
    Ok(())
}

fn ensure_copied_count(conn: &Connection, table: &str, expected: i64) -> Result<()> {
    let actual: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get(0),
    )?;
    if actual != 1 {
        return Err(Error::MigrationBlocked {
            from: PUBLISHED_SCHEMA_VERSION,
            to: CURRENT_SCHEMA_VERSION,
            details: format!("migration staging table {table} is missing; migration rolled back"),
        });
    }

    let sql = format!("SELECT COUNT(*) FROM {table}");
    let copied: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
    if copied != expected {
        return Err(Error::MigrationBlocked {
            from: PUBLISHED_SCHEMA_VERSION,
            to: CURRENT_SCHEMA_VERSION,
            details: format!(
                "migration copied {copied} of {expected} rows into {table}; migration rolled back"
            ),
        });
    }
    Ok(())
}

fn validate_published_v1_layout(conn: &Connection) -> Result<()> {
    let metadata = table_columns(conn, "metadata")?;
    let reservations = table_columns(conn, "reservations")?;
    let staging_objects: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE name IN ('metadata_v2', 'reservations_v2')",
        [],
        |row| row.get(0),
    )?;

    let mut mismatches = Vec::new();
    if !columns_match(&metadata, EXPECTED_V1_METADATA_COLUMNS) {
        mismatches.push("metadata table does not match the published-v1 layout");
    }
    if !columns_match(&reservations, EXPECTED_V1_RESERVATION_COLUMNS) {
        mismatches.push("reservations table does not match the published-v1 layout");
    }
    if staging_objects > 0 {
        mismatches.push("reserved migration staging objects already exist");
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(Error::MigrationBlocked {
            from: PUBLISHED_SCHEMA_VERSION,
            to: CURRENT_SCHEMA_VERSION,
            details: format!(
                "{}; the original database was left unchanged. Restore the \
                 published-v1 layout from a copy or delete the disposable database.",
                mismatches.join("; ")
            ),
        })
    }
}

fn collect_legacy_violations(conn: &Connection) -> Result<LegacyViolations> {
    Ok(LegacyViolations {
        duplicate_logical_keys: count_query(
            conn,
            "SELECT COUNT(*) FROM (
                SELECT path, COALESCE(tag, '') AS normalized_tag
                FROM reservations
                GROUP BY path, normalized_tag
                HAVING COUNT(*) > 1
             )",
        )?,
        duplicate_ports: count_query(
            conn,
            "SELECT COUNT(*) FROM (
                SELECT port FROM reservations
                GROUP BY port HAVING COUNT(*) > 1
             )",
        )?,
        empty_tags: count_query(conn, "SELECT COUNT(*) FROM reservations WHERE tag = ''")?,
        invalid_ports: count_query(
            conn,
            "SELECT COUNT(*) FROM reservations
             WHERE typeof(port) <> 'integer' OR port NOT BETWEEN 1 AND 65535",
        )?,
        invalid_timestamps: count_query(
            conn,
            "SELECT COUNT(*) FROM reservations
             WHERE typeof(created_at) <> 'integer'
                OR typeof(last_used_at) <> 'integer'
                OR created_at < 0
                OR last_used_at < 0",
        )?,
        invalid_scalar_types: count_query(
            conn,
            "SELECT COUNT(*) FROM reservations
             WHERE typeof(path) <> 'text'
                OR typeof(tag) NOT IN ('null', 'text')
                OR typeof(project) NOT IN ('null', 'text')
                OR typeof(task) NOT IN ('null', 'text')",
        )?,
        invalid_metadata_scalars: count_query(
            conn,
            "SELECT COUNT(*) FROM metadata
             WHERE typeof(key) <> 'text' OR typeof(value) <> 'text'",
        )?,
    })
}

fn count_query(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
}

fn validate_schema_v2(conn: &Connection) -> Result<()> {
    let metadata = table_columns(conn, "metadata")?;
    let reservations = table_columns(conn, "reservations")?;
    if !columns_match(&metadata, EXPECTED_V2_METADATA_COLUMNS)
        || !columns_match(&reservations, EXPECTED_V2_RESERVATION_COLUMNS)
        || !table_is_strict(conn, "metadata")?
        || !table_is_strict(conn, "reservations")?
    {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 table layout does not match the required strict schema".into(),
        });
    }

    for index in [
        "idx_reservations_port",
        "idx_reservations_project",
        "idx_reservations_last_used",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema
                WHERE type = 'index' AND name = ?1 AND tbl_name = 'reservations'
             )",
            [index],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::DatabaseCorruption {
                details: format!("schema v2 is missing required index {index}"),
            });
        }
    }

    let schema_version_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM metadata
         WHERE key = 'schema_version' AND value = ?1",
        [CURRENT_SCHEMA_VERSION],
        |row| row.get(0),
    )?;
    if schema_version_rows != 1 {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 must contain exactly one current schema-version value".into(),
        });
    }

    let invalid_rows = count_query(
        conn,
        "SELECT COUNT(*) FROM reservations
         WHERE typeof(path) <> 'text'
            OR typeof(tag) <> 'text'
            OR typeof(port) <> 'integer'
            OR port NOT BETWEEN 1 AND 65535
            OR typeof(project) NOT IN ('null', 'text')
            OR typeof(task) NOT IN ('null', 'text')
            OR typeof(created_at) <> 'integer'
            OR created_at < 0
            OR typeof(last_used_at) <> 'integer'
            OR last_used_at < 0",
    )?;
    if invalid_rows > 0 {
        return Err(Error::DatabaseCorruption {
            details: format!("schema v2 contains {invalid_rows} row(s) violating v2 constraints"),
        });
    }

    let duplicate_keys = count_query(
        conn,
        "SELECT COUNT(*) FROM (
            SELECT path, tag FROM reservations
            GROUP BY path, tag HAVING COUNT(*) > 1
         )",
    )?;
    let duplicate_ports = count_query(
        conn,
        "SELECT COUNT(*) FROM (
            SELECT port FROM reservations
            GROUP BY port HAVING COUNT(*) > 1
         )",
    )?;
    if duplicate_keys > 0 || duplicate_ports > 0 {
        return Err(Error::DatabaseCorruption {
            details: format!(
                "schema v2 uniqueness failure: duplicate keys={duplicate_keys}, \
                 duplicate ports={duplicate_ports}"
            ),
        });
    }

    let mut foreign_keys = conn.prepare("PRAGMA foreign_key_check")?;
    if foreign_keys.query([])?.next()?.is_some() {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 foreign-key check failed".into(),
        });
    }

    let mut integrity = conn.prepare("PRAGMA integrity_check")?;
    let messages = integrity
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if messages.as_slice() != ["ok"] {
        return Err(Error::DatabaseCorruption {
            details: format!("schema v2 integrity check failed: {}", messages.join("; ")),
        });
    }

    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<ActualColumn>> {
    let mut statement = conn.prepare(
        "SELECT name, type, \"notnull\", pk
         FROM pragma_table_info(?1)
         ORDER BY cid",
    )?;
    let columns = statement
        .query_map([table], |row| {
            Ok(ActualColumn {
                name: row.get(0)?,
                declared_type: row.get(1)?,
                not_null: row.get::<_, i64>(2)? != 0,
                primary_key_position: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(columns)
}

fn columns_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.name == expected.name
                && actual
                    .declared_type
                    .eq_ignore_ascii_case(expected.declared_type)
                && actual.not_null == expected.not_null
                && actual.primary_key_position == expected.primary_key_position
        })
}

fn table_is_strict(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some_and(|strict| strict == 1))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;

    const PUBLISHED_V1_SCHEMA: &str = include_str!("../../tests/fixtures/published-v1-schema.sql");
    const INTERRUPT_DATABASE_ENV: &str = "TROP_TEST_MIGRATION_INTERRUPT_DATABASE";
    const INTERRUPT_PHASE_ENV: &str = "TROP_TEST_MIGRATION_INTERRUPT_PHASE";
    const INTERRUPT_EXIT_CODE: i32 = 86;

    fn create_test_connection() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    fn create_published_v1_schema(conn: &Connection) {
        conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO reservations
             (path, tag, port, project, task, created_at, last_used_at)
             VALUES ('/published', NULL, 5000, 'trop', 'migration', 10, 20)",
            [],
        )
        .unwrap();
    }

    fn phase_name(phase: MigrationPhase) -> &'static str {
        match phase {
            MigrationPhase::Locked => "locked",
            MigrationPhase::PreflightComplete => "preflight",
            MigrationPhase::StagingTablesCreated => "staging-created",
            MigrationPhase::RowsCopied => "rows-copied",
            MigrationPhase::OldTablesDropped => "old-tables-dropped",
            MigrationPhase::StagingTablesRenamed => "staging-renamed",
            MigrationPhase::VersionUpdated => "version-updated",
            MigrationPhase::BeforeCommit => "before-commit",
            MigrationPhase::AfterCommit => "after-commit",
        }
    }

    #[test]
    fn initializes_schema_v2_atomically() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();

        assert_eq!(get_schema_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
        validate_schema_v2(&conn).unwrap();
    }

    #[test]
    fn uninitialized_database_reports_version_zero() {
        let conn = create_test_connection();
        assert_eq!(get_schema_version(&conn).unwrap(), 0);
    }

    #[test]
    fn current_schema_is_compatible() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();
        check_schema_compatibility(&conn).unwrap();
    }

    #[test]
    fn future_schema_uses_typed_compatibility_error() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();
        conn.execute(
            "UPDATE metadata SET value = '999' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        assert!(matches!(
            check_schema_compatibility(&conn),
            Err(Error::UnsupportedSchemaVersion {
                expected: 2,
                found: 999
            })
        ));
    }

    #[test]
    fn injected_failure_at_every_precommit_phase_restores_complete_v1() {
        let phases = [
            MigrationPhase::Locked,
            MigrationPhase::PreflightComplete,
            MigrationPhase::StagingTablesCreated,
            MigrationPhase::RowsCopied,
            MigrationPhase::OldTablesDropped,
            MigrationPhase::StagingTablesRenamed,
            MigrationPhase::VersionUpdated,
            MigrationPhase::BeforeCommit,
        ];

        for target in phases {
            let conn = create_test_connection();
            create_published_v1_schema(&conn);
            let result = check_schema_compatibility_with_observer(&conn, |phase| {
                if phase == target {
                    Err(Error::Validation {
                        field: "migration_failpoint".into(),
                        message: phase_name(phase).into(),
                    })
                } else {
                    Ok(())
                }
            });

            assert!(result.is_err(), "phase {} must fail", phase_name(target));
            assert_eq!(get_schema_version(&conn).unwrap(), 1);
            assert_eq!(
                count_query(&conn, "SELECT COUNT(*) FROM reservations").unwrap(),
                1
            );
            assert_eq!(
                count_query(
                    &conn,
                    "SELECT COUNT(*) FROM sqlite_schema
                     WHERE name IN ('metadata_v2', 'reservations_v2')"
                )
                .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        }
    }

    #[test]
    fn migration_commit_uses_full_synchronous_and_restores_connection_setting() {
        let conn = create_test_connection();
        create_published_v1_schema(&conn);
        conn.pragma_update(None, "synchronous", "NORMAL").unwrap();

        check_schema_compatibility_with_observer(&conn, |phase| {
            if phase == MigrationPhase::BeforeCommit {
                let synchronous: i64 = conn
                    .query_row("PRAGMA synchronous", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(synchronous, 2, "migration commit must use FULL");
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(
            conn.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1,
            "connection setting must return to NORMAL"
        );
    }

    #[test]
    fn migration_interrupt_helper() {
        let Ok(database) = std::env::var(INTERRUPT_DATABASE_ENV) else {
            return;
        };
        let target = std::env::var(INTERRUPT_PHASE_ENV).unwrap();
        let conn = Connection::open(database).unwrap();
        let result = check_schema_compatibility_with_observer(&conn, |phase| {
            if phase_name(phase) == target {
                std::process::exit(INTERRUPT_EXIT_CODE);
            }
            Ok(())
        });
        panic!("migration helper did not interrupt at {target}: {result:?}");
    }

    #[test]
    fn process_interruption_recovers_as_complete_v1_or_complete_v2() {
        let phases = [
            MigrationPhase::Locked,
            MigrationPhase::StagingTablesCreated,
            MigrationPhase::RowsCopied,
            MigrationPhase::OldTablesDropped,
            MigrationPhase::StagingTablesRenamed,
            MigrationPhase::VersionUpdated,
            MigrationPhase::BeforeCommit,
            MigrationPhase::AfterCommit,
        ];

        for target in phases {
            let dir = tempdir().unwrap();
            let path = dir.path().join(format!("{}.db", phase_name(target)));
            {
                let conn = Connection::open(&path).unwrap();
                create_published_v1_schema(&conn);
            }

            let status = Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("database::migrations::tests::migration_interrupt_helper")
                .arg("--nocapture")
                .env(INTERRUPT_DATABASE_ENV, &path)
                .env(INTERRUPT_PHASE_ENV, phase_name(target))
                .status()
                .unwrap();
            assert_eq!(
                status.code(),
                Some(INTERRUPT_EXIT_CODE),
                "unexpected helper status for {}: {status}",
                phase_name(target)
            );

            let conn = Connection::open(&path).unwrap();
            let expected_version = if target == MigrationPhase::AfterCommit {
                2
            } else {
                1
            };
            assert_eq!(
                get_schema_version(&conn).unwrap(),
                expected_version,
                "wrong recovered version after {}",
                phase_name(target)
            );
            assert_eq!(
                count_query(
                    &conn,
                    "SELECT COUNT(*) FROM sqlite_schema
                     WHERE name IN ('metadata_v2', 'reservations_v2')"
                )
                .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
        }
    }
}
