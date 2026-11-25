//! Database schema initialization, compatibility checks, and ordered migrations.

use std::fmt::Write as _;
use std::path::Path;

use rusqlite::{Connection, Transaction, TransactionBehavior, MAIN_DB};

use crate::error::{Error, Result};
use crate::verified::schema_version::{
    classify_after_lock, classify_for_open, classify_legacy_row, ColumnFacts, Corruption,
    LegacyRow, LegacyRowViolations, LockedDisposition, MetadataState, OpenDisposition,
};
use crate::verified::storage::{Cell, PathPlatform, SqlType};

use super::operations::{cell_of, OwnedCell};
use super::schema::{
    CREATE_LAST_USED_INDEX, CREATE_METADATA_TABLE, CREATE_PORT_INDEX, CREATE_PROJECT_INDEX,
    CREATE_RESERVATIONS_TABLE, CURRENT_SCHEMA_VERSION, INSERT_SCHEMA_VERSION,
    SELECT_SCHEMA_VERSION,
};

/// The only released predecessor schema, which migrates to the current one.
const PUBLISHED_SCHEMA_VERSION: i32 = crate::verified::schema_version::PUBLISHED_SCHEMA_VERSION;

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

    /// The expected column as the verified layout check sees it.
    const fn facts(&self) -> ColumnFacts<'static> {
        ColumnFacts {
            name: self.name,
            declared_type: self.declared_type,
            not_null: self.not_null,
            primary_key_position: self.primary_key_position,
            hidden: 0,
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

impl ActualColumn {
    /// The column as the verified layout check sees it (`pragma_table_info`
    /// lists no hidden columns).
    fn facts(&self) -> ColumnFacts<'_> {
        ColumnFacts {
            name: &self.name,
            declared_type: &self.declared_type,
            not_null: self.not_null,
            primary_key_position: self.primary_key_position,
            hidden: 0,
        }
    }
}

/// The number of legacy rows in each preflight category.
///
/// The per-row categories come from the verified kernel
/// `classify_legacy_row`: a row is in none of them exactly when the
/// migration copies it faithfully and the v2 row decoder accepts the copy
/// with the same key, so a blocked migration is reported here with every
/// category instead of failing later as generic corruption.
#[derive(Debug, Default, PartialEq, Eq)]
struct LegacyViolations {
    duplicate_logical_keys: i64,
    duplicate_ports: i64,
    empty_tags: i64,
    invalid_ports: i64,
    invalid_timestamps: i64,
    invalid_scalar_types: i64,
    invalid_metadata_scalars: i64,
    invalid_text: i64,
    noncanonical_paths: i64,
    noncanonical_tags: i64,
    noncanonical_projects: i64,
    noncanonical_tasks: i64,
    out_of_range_timestamps: i64,
}

impl LegacyViolations {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Counts one row's categories.
    fn add_row(&mut self, row: LegacyRowViolations) {
        self.empty_tags += i64::from(row.empty_tag);
        self.invalid_ports += i64::from(row.invalid_port);
        self.invalid_timestamps += i64::from(row.negative_timestamp);
        self.invalid_scalar_types += i64::from(row.invalid_scalar_type);
        self.invalid_text += i64::from(row.invalid_text);
        self.noncanonical_paths += i64::from(row.noncanonical_path);
        self.noncanonical_tags += i64::from(row.noncanonical_tag);
        self.noncanonical_projects += i64::from(row.noncanonical_project);
        self.noncanonical_tasks += i64::from(row.noncanonical_task);
        self.out_of_range_timestamps += i64::from(row.timestamp_out_of_range);
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
            ("text values that are not valid UTF-8", self.invalid_text),
            (
                "paths not in canonical absolute form",
                self.noncanonical_paths,
            ),
            ("blank or untrimmed tags", self.noncanonical_tags),
            (
                "empty or untrimmed project values",
                self.noncanonical_projects,
            ),
            ("empty or untrimmed task values", self.noncanonical_tasks),
            (
                "timestamps beyond the supported range",
                self.out_of_range_timestamps,
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
    if conn.is_readonly(MAIN_DB)? {
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
/// Version zero denotes an uninitialized database: neither the `metadata`
/// nor the `reservations` table exists.
///
/// # Errors
///
/// Returns a database corruption error when the schema metadata is corrupt:
/// a `metadata` table without exactly one integer `schema_version` row, or a
/// `reservations` table without a `metadata` table. Such a database is never
/// treated as uninitialized.
pub fn get_schema_version(conn: &Connection) -> Result<i32> {
    let observed = observe_metadata(conn)?;
    match classify_for_open(observed.state(), observed.reservations_table) {
        OpenDisposition::Initialize => Ok(0),
        OpenDisposition::UpToDate => Ok(CURRENT_SCHEMA_VERSION),
        OpenDisposition::MigrateFromPublished => Ok(PUBLISHED_SCHEMA_VERSION),
        OpenDisposition::Future(version) | OpenDisposition::NoMigrationPath(version) => Ok(version),
        OpenDisposition::Corrupt(corruption) => Err(version_corruption(corruption)),
    }
}

/// What the glue observed about the schema metadata: the facts the verified
/// schema-version kernel classifies.
pub(super) struct ObservedMetadata {
    version: ObservedVersion,
    /// Whether a `reservations` table exists (only looked up, and only
    /// relevant, when there is no `metadata` table).
    reservations_table: bool,
}

enum ObservedVersion {
    NoTable,
    NoRow,
    Row { value: OwnedCell, more_rows: bool },
}

impl ObservedMetadata {
    pub(super) fn state(&self) -> MetadataState<'_> {
        match &self.version {
            ObservedVersion::NoTable => MetadataState::NoTable,
            ObservedVersion::NoRow => MetadataState::NoVersionRow,
            ObservedVersion::Row { value, more_rows } => MetadataState::VersionRow {
                value: value.cell(),
                more_rows: *more_rows,
            },
        }
    }
}

/// Reads the `schema_version` rows, and whether a `reservations` table
/// exists when the `metadata` table does not.
pub(super) fn observe_metadata(conn: &Connection) -> Result<ObservedMetadata> {
    let version = match conn.prepare(SELECT_SCHEMA_VERSION) {
        Err(error) if is_missing_metadata_table(&error) => ObservedVersion::NoTable,
        Err(error) => return Err(error.into()),
        Ok(mut statement) => {
            let mut rows = statement.query([])?;
            match rows.next()? {
                None => ObservedVersion::NoRow,
                Some(row) => {
                    let value = OwnedCell::copy_of(row.get_ref(0)?);
                    // A second row is looked for only once the first value
                    // is readable text, as the classification requires.
                    let more_rows = matches!(value, OwnedCell::Text(_)) && rows.next()?.is_some();
                    ObservedVersion::Row { value, more_rows }
                }
            }
        }
    };
    let reservations_table =
        matches!(version, ObservedVersion::NoTable) && table_exists(conn, "reservations")?;
    Ok(ObservedMetadata {
        version,
        reservations_table,
    })
}

/// Whether the main database has a table named `table` (table names are
/// case-insensitive in SQLite).
fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1 COLLATE NOCASE",
        [table],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// The corruption error for schema metadata the kernel classified as
/// corrupt.
fn version_corruption(corruption: Corruption) -> Error {
    let reason = match corruption {
        Corruption::MissingMetadataTable => {
            "the metadata table is missing although a reservations table exists"
        }
        Corruption::MissingVersionRow | Corruption::MultipleVersionRows => {
            "exactly one schema_version row is required"
        }
        Corruption::NotText(SqlType::Null) => "schema_version must be TEXT, found NULL",
        Corruption::NotText(SqlType::Integer) => "schema_version must be TEXT, found INTEGER",
        Corruption::NotText(SqlType::Real) => "schema_version must be TEXT, found REAL",
        Corruption::NotText(SqlType::Blob) => "schema_version must be TEXT, found BLOB",
        // Text cells are never reported as `NotText`.
        Corruption::NotText(SqlType::Text) => "schema_version must be TEXT",
        Corruption::NotUtf8 => "schema_version TEXT is not valid UTF-8",
        Corruption::NotAnInteger => "schema_version must be a base-10 signed integer",
    };
    Error::corrupt_stored_value("metadata", "value", "key=\"schema_version\"", reason)
}

/// The error for a schema newer than this client.
fn unsupported_version(version: i32) -> Error {
    Error::UnsupportedSchemaVersion {
        expected: u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or_default(),
        found: u32::try_from(version).unwrap_or(u32::MAX),
    }
}

fn is_missing_metadata_table(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite, Some(message))
            if sqlite.code == rusqlite::ErrorCode::Unknown
                && message.contains("no such table")
                && message.contains("metadata")
    )
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
/// without choosing or discarding rows. Corrupt schema metadata (a
/// `metadata` table without exactly one integer `schema_version` row, or
/// reservations without a `metadata` table) is reported as database
/// corruption and never reinitialized: only a database with neither table is
/// initialized. Database and integrity failures are also returned without
/// leaving a partially migrated schema.
pub fn check_schema_compatibility(conn: &Connection) -> Result<()> {
    check_schema_compatibility_with_observer(conn, |_| Ok(()))
}

fn check_schema_compatibility_with_observer(
    conn: &Connection,
    mut observer: impl FnMut(MigrationPhase) -> Result<()>,
) -> Result<()> {
    // The verified kernel decides; only a database with neither table is
    // initialized, and corrupt metadata is reported, never restamped.
    let metadata = observe_metadata(conn)?;
    match classify_for_open(metadata.state(), metadata.reservations_table) {
        OpenDisposition::Initialize => return initialize_schema(conn),
        OpenDisposition::UpToDate => return Ok(()),
        OpenDisposition::Future(version) => return Err(unsupported_version(version)),
        OpenDisposition::NoMigrationPath(version) => {
            return Err(Error::MigrationRequired {
                found: version,
                target: CURRENT_SCHEMA_VERSION,
                action: "no ordered migration is available for this older schema version".into(),
            });
        }
        OpenDisposition::Corrupt(corruption) => return Err(version_corruption(corruption)),
        OpenDisposition::MigrateFromPublished => {}
    }
    if conn.is_readonly(MAIN_DB)? {
        return Err(Error::MigrationRequired {
            found: PUBLISHED_SCHEMA_VERSION,
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

    // A second migrator may have waited for the first one's commit. The
    // verified kernel proceeds only from the published version.
    let metadata = observe_metadata(&tx)?;
    match classify_after_lock(metadata.state(), metadata.reservations_table) {
        LockedDisposition::AlreadyCurrent => {
            tx.rollback()?;
            return Ok(());
        }
        LockedDisposition::Future(version) => return Err(unsupported_version(version)),
        LockedDisposition::Changed(version) => {
            return Err(Error::MigrationRequired {
                found: version,
                target: CURRENT_SCHEMA_VERSION,
                action: "the schema version changed while waiting for the migration lock".into(),
            });
        }
        LockedDisposition::Corrupt(corruption) => return Err(version_corruption(corruption)),
        LockedDisposition::Proceed => {}
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
    let mut violations = LegacyViolations {
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
        invalid_metadata_scalars: count_query(
            conn,
            "SELECT COUNT(*) FROM metadata
             WHERE typeof(key) <> 'text' OR typeof(value) <> 'text'",
        )?,
        ..LegacyViolations::default()
    };

    // Every per-row category is decided by the verified kernel.
    let mut statement = conn.prepare(
        "SELECT path, tag, port, project, task, created_at, last_used_at
         FROM reservations",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let path = cell_of(row.get_ref(0)?);
        let path_platform = match path {
            Cell::Text(text) => crate::path::storage_platform(Path::new(text)),
            _ => PathPlatform::Unix,
        };
        let legacy = LegacyRow {
            path,
            path_platform,
            tag: cell_of(row.get_ref(1)?),
            port: cell_of(row.get_ref(2)?),
            project: cell_of(row.get_ref(3)?),
            task: cell_of(row.get_ref(4)?),
            created_at: cell_of(row.get_ref(5)?),
            last_used_at: cell_of(row.get_ref(6)?),
        };
        violations.add_row(classify_legacy_row(&legacy));
    }
    Ok(violations)
}

fn count_query(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
}

fn validate_schema_v2(conn: &Connection) -> Result<()> {
    super::validation::validate_current_database(conn)
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

/// Whether the columns match the published-v1 layout, including primary-key
/// positions (verified kernel `columns_match`).
fn columns_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    let actual = actual.iter().map(ActualColumn::facts).collect::<Vec<_>>();
    let expected = expected.iter().map(ColumnSpec::facts).collect::<Vec<_>>();
    crate::verified::schema_version::columns_match(&actual, &expected, true, false)
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

    /// The verified schema-version parser against `i32::from_str`, the parser
    /// it replaced, on the syntax's edge cases and on every short string over
    /// a sign/digit/other alphabet.
    #[test]
    fn version_parser_matches_i32_from_str() {
        use crate::verified::schema_version::parse_i32_decimal;
        for text in [
            "",
            "+",
            "-",
            "0",
            "-0",
            "+0",
            "02",
            "+2",
            "-2",
            " 2",
            "2 ",
            "2\n",
            "1_000",
            "2147483647",
            "2147483648",
            "-2147483648",
            "-2147483649",
            "+2147483647",
            "0000000000000000000002147483647",
            "99999999999999999999",
            "\u{ff12}",
            "1e3",
            "--1",
            "+-1",
        ] {
            assert_eq!(
                parse_i32_decimal(text.as_bytes()),
                text.parse::<i32>().ok(),
                "{text:?}"
            );
        }
        let alphabet = ['+', '-', '0', '1', '9', 'a', ' '];
        let mut frontier = vec![String::new()];
        for _ in 0..5 {
            let mut next = Vec::new();
            for prefix in &frontier {
                for c in alphabet {
                    let text = format!("{prefix}{c}");
                    assert_eq!(
                        parse_i32_decimal(text.as_bytes()),
                        text.parse::<i32>().ok(),
                        "{text:?}"
                    );
                    next.push(text);
                }
            }
            frontier = next;
        }
        for value in [i32::MIN, -1, 0, 1, 2, i32::MAX] {
            assert_eq!(parse_i32_decimal(value.to_string().as_bytes()), Some(value));
        }
    }

    /// A metadata table without a version row is corrupt, not
    /// uninitialized (it used to read as version 0 and be restamped).
    #[test]
    fn metadata_without_version_row_is_corrupt() {
        let conn = create_test_connection();
        initialize_schema(&conn).unwrap();
        conn.execute("DELETE FROM metadata", []).unwrap();
        assert!(matches!(
            get_schema_version(&conn),
            Err(Error::DatabaseCorruption { .. })
        ));
        assert!(matches!(
            check_schema_compatibility(&conn),
            Err(Error::DatabaseCorruption { .. })
        ));
        assert_eq!(
            count_query(&conn, "SELECT COUNT(*) FROM metadata").unwrap(),
            0
        );

        // Duplicate rows and malformed values are reported the same way.
        let conn = create_test_connection();
        conn.execute_batch(
            "CREATE TABLE metadata (key TEXT NOT NULL, value);
             INSERT INTO metadata VALUES ('schema_version', '2');
             INSERT INTO metadata VALUES ('schema_version', '2');",
        )
        .unwrap();
        assert!(matches!(
            get_schema_version(&conn),
            Err(Error::DatabaseCorruption { details }) if details.contains("exactly one")
        ));
        for value in ["'x'", "NULL", "2", "X'32'"] {
            let conn = create_test_connection();
            conn.execute_batch(&format!(
                "CREATE TABLE metadata (key TEXT NOT NULL, value);
                 INSERT INTO metadata VALUES ('schema_version', {value});"
            ))
            .unwrap();
            assert!(
                matches!(
                    get_schema_version(&conn),
                    Err(Error::DatabaseCorruption { .. })
                ),
                "{value}"
            );
        }
    }

    /// The column layout check through the verified kernel: names exact,
    /// types without ASCII case, `NOT NULL` and primary-key positions exact.
    #[test]
    fn layout_check_compares_every_column_attribute() {
        let column = |name: &str, declared_type: &str, not_null, pk| ActualColumn {
            name: name.into(),
            declared_type: declared_type.into(),
            not_null,
            primary_key_position: pk,
        };
        let exact = vec![
            column("key", "text", true, 1),
            column("value", "TeXt", true, 0),
        ];
        assert!(columns_match(&exact, EXPECTED_V1_METADATA_COLUMNS));
        for wrong in [
            vec![column("key", "TEXT", true, 1)],
            vec![
                column("Key", "TEXT", true, 1),
                column("value", "TEXT", true, 0),
            ],
            vec![
                column("key", "TEXTS", true, 1),
                column("value", "TEXT", true, 0),
            ],
            vec![
                column("key", "TEXT", false, 1),
                column("value", "TEXT", true, 0),
            ],
            vec![
                column("key", "TEXT", true, 0),
                column("value", "TEXT", true, 0),
            ],
            vec![
                column("key", "TEXT", true, 1),
                column("value", "TEXT", true, 0),
                column("x", "TEXT", false, 0),
            ],
        ] {
            assert!(
                !columns_match(&wrong, EXPECTED_V1_METADATA_COLUMNS),
                "{wrong:?}"
            );
        }
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
