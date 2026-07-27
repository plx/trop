use std::fs;
use std::path::Path;

use rusqlite::{params, Connection};
use tempfile::tempdir;
use trop::database::{Database, DatabaseConfig};
use trop::Error;

const RESERVATION_COLUMNS: &str = "path, tag, port, project, task, created_at, last_used_at";
const PUBLISHED_V1_SCHEMA: &str = include_str!("fixtures/published-v1-schema.sql");

fn create_database(path: &Path) {
    drop(Database::open(DatabaseConfig::new(path)).unwrap());
}

fn insert_row(conn: &Connection) {
    conn.execute(
        &format!(
            "INSERT INTO reservations ({RESERVATION_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        ),
        params![
            "/project",
            "",
            5000_i64,
            None::<String>,
            None::<String>,
            1_i64,
            2_i64
        ],
    )
    .unwrap();
}

fn corruption_details(error: Error) -> String {
    match error {
        Error::DatabaseCorruption { details } => details,
        other => panic!("expected typed database corruption, got {other:?}"),
    }
}

#[test]
fn database_row_decoding_rejects_every_domain_field_without_unwinding() {
    let cases = [
        (
            "UPDATE reservations SET path = 'relative/path'",
            "field=path",
        ),
        (
            "UPDATE reservations SET path = '/project/../other'",
            "field=path",
        ),
        ("UPDATE reservations SET tag = ' padded '", "field=tag"),
        (
            "PRAGMA ignore_check_constraints=ON;
             UPDATE reservations SET port = 0",
            "field=port",
        ),
        (
            "PRAGMA ignore_check_constraints=ON;
             UPDATE reservations SET port = 70000",
            "field=port",
        ),
        ("UPDATE reservations SET project = ''", "field=project"),
        ("UPDATE reservations SET task = ' padded '", "field=task"),
        (
            "PRAGMA ignore_check_constraints=ON;
             UPDATE reservations SET created_at = -1",
            "field=created_at",
        ),
        (
            "UPDATE reservations SET last_used_at = 9223372036854775807",
            "field=last_used_at",
        ),
    ];

    for (mutation, expected_field) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        create_database(&path);
        let conn = Connection::open(&path).unwrap();
        insert_row(&conn);
        conn.execute_batch(mutation).unwrap();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Database::list_all_reservations(&conn)
        }));
        let error = outcome
            .unwrap_or_else(|_| panic!("mutation unwound instead of returning {expected_field}"))
            .unwrap_err();
        let details = corruption_details(error);
        assert!(details.contains("table=reservations"), "{details}");
        assert!(details.contains(expected_field), "{details}");
        if expected_field == "field=path" {
            assert!(
                details.contains("path=\"relative/path\"")
                    || details.contains("path=\"/project/../other\""),
                "{details}"
            );
        } else {
            assert!(details.contains("path=\"/project\""), "{details}");
        }
        if expected_field != "field=tag" {
            assert!(!details.contains("padded"), "{details}");
        }
    }
}

#[test]
fn database_validation_detects_logical_rows_and_preserves_last_before_created() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    create_database(&path);
    let conn = Connection::open(&path).unwrap();
    insert_row(&conn);
    conn.execute(
        "UPDATE reservations SET created_at = 20, last_used_at = 10",
        [],
    )
    .unwrap();
    drop(conn);

    Database::validate(&DatabaseConfig::new(&path)).unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE reservations SET last_used_at = 9223372036854775807",
        [],
    )
    .unwrap();
    drop(conn);
    let details = corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
    assert!(details.contains("field=last_used_at"), "{details}");
}

#[test]
fn database_validation_rejects_duplicate_keys_and_ports() {
    for duplicate_port in [false, true] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE metadata (
                key TEXT NOT NULL,
                value TEXT NOT NULL
             ) STRICT;
             INSERT INTO metadata VALUES ('schema_version', '2');
             CREATE TABLE reservations (
                path TEXT NOT NULL,
                tag TEXT NOT NULL,
                port INTEGER NOT NULL
                    CONSTRAINT valid_port CHECK (port BETWEEN 1 AND 65535),
                project TEXT,
                task TEXT,
                created_at INTEGER NOT NULL
                    CONSTRAINT valid_created_at CHECK (created_at >= 0),
                last_used_at INTEGER NOT NULL
                    CONSTRAINT valid_last_used_at CHECK (last_used_at >= 0)
             ) STRICT;
             CREATE INDEX idx_reservations_port ON reservations(port);
             CREATE INDEX idx_reservations_project ON reservations(project);
             CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);",
        )
        .unwrap();
        if duplicate_port {
            conn.execute_batch(
                "INSERT INTO reservations VALUES ('/one', '', 5000, NULL, NULL, 1, 1);
                 INSERT INTO reservations VALUES ('/two', '', 5000, NULL, NULL, 1, 1);",
            )
            .unwrap();
        } else {
            conn.execute_batch(
                "INSERT INTO reservations VALUES ('/one', '', 5000, NULL, NULL, 1, 1);
                 INSERT INTO reservations VALUES ('/one', '', 5001, NULL, NULL, 1, 1);",
            )
            .unwrap();
        }
        drop(conn);

        let details =
            corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
        let expected = if duplicate_port {
            "same port"
        } else {
            "duplicate logical"
        };
        assert!(details.contains(expected), "{details}");
    }
}

#[test]
fn database_validation_rejects_malformed_metadata_but_allows_unknown_text_metadata() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    create_database(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute("INSERT INTO metadata VALUES ('unknown', 'preserved')", [])
        .unwrap();
    drop(conn);
    Database::validate(&DatabaseConfig::new(&path)).unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE metadata SET value = 'not-a-number' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let details = corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
    assert!(details.contains("table=metadata"), "{details}");
    assert!(details.contains("field=value"), "{details}");
    assert!(details.contains("key=\"schema_version\""), "{details}");
    assert!(!details.contains("not-a-number"), "{details}");
}

#[test]
fn database_validation_rejects_missing_and_wrong_named_indexes() {
    for replacement in [
        "",
        "CREATE INDEX idx_reservations_project ON reservations(task);",
        "CREATE INDEX idx_reservations_project
         ON reservations(project COLLATE NOCASE DESC);",
        "CREATE INDEX idx_reservations_project
         ON reservations(lower(project));",
    ] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        create_database(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP INDEX idx_reservations_project;")
            .unwrap();
        if !replacement.is_empty() {
            conn.execute_batch(replacement).unwrap();
        }
        drop(conn);

        let details =
            corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
        assert!(details.contains("idx_reservations_project"), "{details}");
        assert!(details.contains("reservations.project"), "{details}");
    }
}

#[test]
fn database_validation_rejects_hidden_columns_and_constraint_text_in_comments() {
    let dir = tempdir().unwrap();
    let hidden_path = dir.path().join("hidden-column.db");
    create_database(&hidden_path);
    let conn = Connection::open(&hidden_path).unwrap();
    conn.execute_batch(
        "ALTER TABLE reservations
         ADD COLUMN hidden_copy TEXT GENERATED ALWAYS AS (path) VIRTUAL;",
    )
    .unwrap();
    drop(conn);
    let details =
        corruption_details(Database::validate(&DatabaseConfig::new(&hidden_path)).unwrap_err());
    assert!(details.contains("required layout"), "{details}");

    let comment_path = dir.path().join("comment-constraints.db");
    let conn = Connection::open(&comment_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
         ) STRICT;
         INSERT INTO metadata VALUES ('schema_version', '2');
         CREATE TABLE reservations (
            path TEXT NOT NULL,
            tag TEXT NOT NULL,
            port INTEGER NOT NULL UNIQUE,
            project TEXT,
            task TEXT,
            created_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            /*
             CONSTRAINT valid_port CHECK (port BETWEEN 1 AND 65535)
             CONSTRAINT valid_created_at CHECK (created_at >= 0)
             CONSTRAINT valid_last_used_at CHECK (last_used_at >= 0)
            */
            PRIMARY KEY (path, tag)
         ) STRICT;
         CREATE INDEX idx_reservations_port ON reservations(port);
         CREATE INDEX idx_reservations_project ON reservations(project);
         CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);",
    )
    .unwrap();
    drop(conn);
    let details =
        corruption_details(Database::validate(&DatabaseConfig::new(&comment_path)).unwrap_err());
    assert!(
        details.contains("missing required constraint valid_port"),
        "{details}"
    );
}

#[test]
fn database_validation_preserves_quoted_schema_tokens_during_comparison() {
    for port_constraint in [
        r#"CONSTRAINT valid_port CHECK (port BETWEEN 1 AND "655 35")"#,
        r#"CONSTRAINT "valid_ port" CHECK (port BETWEEN 1 AND 65535)"#,
        r#"CONSTRAINT valid_port CHECK ("portbetween1and65535")"#,
    ] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("quoted-token.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE metadata (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
             ) STRICT;
             INSERT INTO metadata VALUES ('schema_version', '2');
             CREATE TABLE reservations (
                path TEXT NOT NULL,
                tag TEXT NOT NULL,
                port INTEGER NOT NULL UNIQUE
                    {port_constraint},
                project TEXT,
                task TEXT,
                created_at INTEGER NOT NULL
                    CONSTRAINT valid_created_at CHECK (created_at >= 0),
                last_used_at INTEGER NOT NULL
                    CONSTRAINT valid_last_used_at CHECK (last_used_at >= 0),
                PRIMARY KEY (path, tag)
             ) STRICT;
             CREATE INDEX idx_reservations_port ON reservations(port);
             CREATE INDEX idx_reservations_project ON reservations(project);
             CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);"
        ))
        .unwrap();

        if port_constraint.contains("655 35") {
            conn.execute(
                "INSERT INTO reservations VALUES ('/project', '', 70000, NULL, NULL, 1, 1)",
                [],
            )
            .expect("the malformed quoted bound demonstrates ineffective enforcement");
            conn.execute("DELETE FROM reservations", []).unwrap();
        } else if port_constraint.contains("portbetween") {
            conn.execute(
                "INSERT INTO reservations VALUES ('/project', '', 5000, NULL, NULL, 1, 1)",
                [],
            )
            .expect_err("the fused quoted token must not enforce the canonical port predicate");
        }
        drop(conn);

        let details =
            corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
        assert!(
            details.contains("valid_port")
                || details.contains("does not exactly match the required definition"),
            "{details}"
        );
    }
}

#[test]
fn database_validation_rejects_missing_check_constraints() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
         ) STRICT;
         INSERT INTO metadata VALUES ('schema_version', '2');
         CREATE TABLE reservations (
            path TEXT NOT NULL,
            tag TEXT NOT NULL,
            port INTEGER NOT NULL UNIQUE,
            project TEXT,
            task TEXT,
            created_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            PRIMARY KEY (path, tag)
         ) STRICT;
         CREATE INDEX idx_reservations_port ON reservations(port);
         CREATE INDEX idx_reservations_project ON reservations(project);
         CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);",
    )
    .unwrap();
    drop(conn);

    let details = corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
    assert!(details.contains("valid_port"), "{details}");
}

#[test]
fn database_validation_is_read_only_and_does_not_modify_database_contents() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    create_database(&path);
    let before = fs::read(&path).unwrap();
    let wal = path.with_extension("db-wal");
    let wal_before = wal.exists().then(|| fs::read(&wal).unwrap());

    Database::validate(&DatabaseConfig::new(&path)).unwrap();

    assert_eq!(fs::read(&path).unwrap(), before);
    let wal_after = wal.exists().then(|| fs::read(&wal).unwrap());
    match wal_before {
        Some(before) => assert_eq!(
            wal_after,
            Some(before),
            "read-only validation must not modify pending database content"
        ),
        None => assert!(
            wal_after.as_ref().is_none_or(Vec::is_empty),
            "read-only validation must not add database content"
        ),
    }
    assert!(
        !path.with_extension("db-journal").exists(),
        "read-only validation must not start a rollback journal"
    );
}

#[test]
fn database_validation_classifies_random_bytes_as_physical_corruption() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    fs::write(&path, b"not a sqlite database").unwrap();

    let details = corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err());
    assert!(
        details.contains("physical database corruption"),
        "{details}"
    );
}

#[test]
fn database_validation_is_read_only_for_missing_legacy_and_future_databases() {
    let dir = tempdir().unwrap();

    let missing = dir.path().join("missing.db");
    assert!(Database::validate(&DatabaseConfig::new(&missing)).is_err());
    assert!(
        !missing.exists(),
        "read-only validation must not initialize a missing database"
    );

    let legacy = dir.path().join("legacy.db");
    let conn = Connection::open(&legacy).unwrap();
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
    drop(conn);
    let legacy_before = fs::read(&legacy).unwrap();
    assert!(matches!(
        Database::validate(&DatabaseConfig::new(&legacy)),
        Err(Error::MigrationRequired {
            found: 1,
            target: 2,
            ..
        })
    ));
    assert_eq!(
        fs::read(&legacy).unwrap(),
        legacy_before,
        "read-only validation must not migrate a legacy database"
    );

    let future = dir.path().join("future.db");
    create_database(&future);
    let conn = Connection::open(&future).unwrap();
    conn.execute(
        "UPDATE metadata SET value = '3' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let future_before = fs::read(&future).unwrap();
    assert!(matches!(
        Database::validate(&DatabaseConfig::new(&future)),
        Err(Error::UnsupportedSchemaVersion {
            expected: 2,
            found: 3
        })
    ));
    assert_eq!(
        fs::read(&future).unwrap(),
        future_before,
        "read-only validation must not rewrite a future database"
    );
}
