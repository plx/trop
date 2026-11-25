//! Regression tests: opening and validation report a schema-version verdict
//! of the verified `schema_version` kernel with one message table, so the
//! same corrupt or newer metadata gives the same error text either way.
//! (Before, the two had separate, drifted tables: for example
//! "schema_version must be TEXT, found INTEGER" when opening and
//! "expected TEXT, found INTEGER" when validating.)

use std::path::Path;

use rusqlite::Connection;
use tempfile::tempdir;
use trop::database::{Database, DatabaseConfig};
use trop::Error;

/// A current database whose `metadata` table is replaced by a loosely
/// typed one holding `schema_version = <value>` (an SQL expression).
fn database_with_version_cell(path: &Path, value_sql: &str) {
    drop(Database::open(DatabaseConfig::new(path)).unwrap());
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&format!(
        "DROP TABLE metadata;
         CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value);
         INSERT INTO metadata VALUES ('schema_version', {value_sql});"
    ))
    .unwrap();
}

fn corruption_details(result: trop::Result<impl std::fmt::Debug>, what: &str) -> String {
    match result {
        Err(Error::DatabaseCorruption { details }) => details,
        other => panic!("{what}: expected typed database corruption, got {other:?}"),
    }
}

#[test]
fn open_and_validate_report_the_same_schema_version_corruption() {
    let cases = [
        ("NULL", "expected TEXT, found NULL"),
        ("2", "expected TEXT, found INTEGER"),
        ("2.5", "expected TEXT, found REAL"),
        ("X'32'", "expected TEXT, found BLOB"),
        ("CAST(X'FF' AS TEXT)", "stored TEXT is not valid UTF-8"),
        ("'two'", "schema_version must be a base-10 signed integer"),
        ("' 2'", "schema_version must be a base-10 signed integer"),
    ];
    for (value_sql, reason) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        database_with_version_cell(&path, value_sql);

        let opened = corruption_details(
            Database::open(DatabaseConfig::new(&path)).map(|_| ()),
            value_sql,
        );
        let validated =
            corruption_details(Database::validate(&DatabaseConfig::new(&path)), value_sql);
        assert_eq!(opened, validated, "{value_sql}");
        assert!(opened.contains(reason), "{value_sql}: {opened}");
        assert!(opened.contains("table=metadata"), "{value_sql}: {opened}");
        // Regression: the key used to be prefixed twice
        // (`key=key="schema_version"`).
        assert!(
            opened.contains("field=value, key=\"schema_version\": "),
            "{value_sql}: {opened}"
        );
    }
}

/// The row-count verdicts (`MissingVersionRow`, `MultipleVersionRows`) also
/// come from the one message table, opening and validating alike.
#[test]
fn open_and_validate_report_the_same_schema_version_row_count_corruption() {
    let cases = [
        (
            "missing row",
            "DROP TABLE metadata;
             CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value);",
        ),
        (
            "two rows",
            "DROP TABLE metadata;
             CREATE TABLE metadata (key TEXT NOT NULL, value);
             INSERT INTO metadata VALUES ('schema_version', '2');
             INSERT INTO metadata VALUES ('schema_version', '2');",
        ),
    ];
    for (what, sql) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        drop(Database::open(DatabaseConfig::new(&path)).unwrap());
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();

        let opened =
            corruption_details(Database::open(DatabaseConfig::new(&path)).map(|_| ()), what);
        let validated = corruption_details(Database::validate(&DatabaseConfig::new(&path)), what);
        assert_eq!(opened, validated, "{what}");
        assert!(
            opened.starts_with(
                "table=metadata, field=value, key=\"schema_version\": \
                 exactly one schema_version row is required."
            ),
            "{what}: {opened}"
        );
    }
}

/// Regression: metadata row diagnostics named the key twice
/// (`key=key="other"`); they now read like the schema version's.
#[test]
fn metadata_row_diagnostics_name_the_key_once() {
    let cases = [
        (
            "INSERT INTO metadata VALUES ('other', CAST(X'FF' AS TEXT));",
            "table=metadata, field=value, key=\"other\": stored TEXT is not valid UTF-8.",
        ),
        (
            "INSERT INTO metadata VALUES (CAST(X'FF' AS TEXT), 'x');",
            "table=metadata, field=key, key=<invalid UTF-8>: stored TEXT is not valid UTF-8.",
        ),
    ];
    for (sql, expected) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        drop(Database::open(DatabaseConfig::new(&path)).unwrap());
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();

        let validated = corruption_details(Database::validate(&DatabaseConfig::new(&path)), sql);
        assert!(validated.starts_with(expected), "{sql}: {validated}");
    }
}

#[test]
fn open_and_validate_report_the_same_newer_schema_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    database_with_version_cell(&path, "'999'");

    let opened = Database::open(DatabaseConfig::new(&path)).map(|_| ());
    let validated = Database::validate(&DatabaseConfig::new(&path));
    match (opened, validated) {
        (
            Err(Error::UnsupportedSchemaVersion {
                expected: e1,
                found: f1,
            }),
            Err(Error::UnsupportedSchemaVersion {
                expected: e2,
                found: f2,
            }),
        ) => {
            assert_eq!((e1, f1), (e2, f2));
            assert_eq!(f1, 999);
        }
        other => panic!("expected unsupported schema versions, got {other:?}"),
    }
}

#[test]
fn opening_reports_a_missing_metadata_table() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(Database::open(DatabaseConfig::new(&path)).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE metadata;")
        .unwrap();

    let opened = corruption_details(
        Database::open(DatabaseConfig::new(&path)).map(|_| ()),
        "missing metadata",
    );
    assert!(
        opened.starts_with(
            "table=metadata, field=value, key=\"schema_version\": \
             the metadata table is missing although a reservations table exists."
        ),
        "{opened}"
    );
}
