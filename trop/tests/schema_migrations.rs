//! Schema migration integration tests using the published-v1 database layout.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;

use rusqlite::{params, Connection};
use tempfile::tempdir;
use trop::database::{check_schema_compatibility, get_schema_version, Database, DatabaseConfig};
use trop::Error;

const PUBLISHED_V1_SCHEMA: &str = include_str!("fixtures/published-v1-schema.sql");

fn create_published_v1_database(conn: &Connection) {
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
}

fn create_published_v1_at(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_published_v1_database(&conn);
}

fn raw_schema_version(path: &Path) -> i32 {
    let conn = Connection::open(path).unwrap();
    get_schema_version(&conn).unwrap()
}

#[test]
fn published_v1_database_migrates_to_v2_and_preserves_rows() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("published-v1.db");
    let conn = Connection::open(&path).unwrap();
    create_published_v1_database(&conn);
    conn.execute(
        "INSERT INTO reservations
         (path, tag, port, project, task, created_at, last_used_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            "/project",
            Option::<&str>::None,
            5000,
            "trop",
            "db-v2",
            10,
            20
        ],
    )
    .unwrap();
    drop(conn);

    let db = Database::open(DatabaseConfig::new(&path)).unwrap();

    assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
    let row = db
        .connection()
        .query_row(
            "SELECT path, tag, port, project, task, created_at, last_used_at
             FROM reservations",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "/project".to_string(),
            String::new(),
            5000,
            "trop".to_string(),
            "db-v2".to_string(),
            10,
            20,
        )
    );
}

#[test]
fn valid_unrepaired_v1_layout_migrates_and_gains_v2_constraints() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("unrepaired-v1.db");
    let conn = Connection::open(&path).unwrap();
    let schema_without_unique_port =
        PUBLISHED_V1_SCHEMA.replace("port INTEGER NOT NULL UNIQUE", "port INTEGER NOT NULL");
    conn.execute_batch(&schema_without_unique_port).unwrap();
    conn.execute_batch(
        "INSERT INTO reservations VALUES ('/untagged', NULL, 5000, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/tagged', 'web', 5001, 'legacy', 'repair', 2, 3);",
    )
    .unwrap();
    drop(conn);

    let db = Database::open(DatabaseConfig::new(&path)).unwrap();

    assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
    assert_eq!(
        db.connection()
            .query_row("SELECT COUNT(*) FROM reservations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        2
    );
    assert!(db
        .connection()
        .execute(
            "INSERT INTO reservations VALUES ('/other', '', 5000, NULL, NULL, 1, 1)",
            [],
        )
        .is_err());
}

#[test]
fn migration_preserves_tagged_rows_boundaries_timestamps_and_metadata() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("semantic-preservation.db");
    let conn = Connection::open(&path).unwrap();
    create_published_v1_database(&conn);
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('fixture_note', 'preserve me')",
        [],
    )
    .unwrap();
    let rows = [
        (
            "/alpha",
            Option::<&str>::None,
            1,
            Option::<&str>::None,
            Option::<&str>::None,
            0,
            0,
        ),
        (
            "/unicode/项目",
            Some("web"),
            65535,
            Some("π"),
            Some("clock-skew"),
            200,
            100,
        ),
    ];
    for row in rows {
        conn.execute(
            "INSERT INTO reservations
             (path, tag, port, project, task, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![row.0, row.1, row.2, row.3, row.4, row.5, row.6],
        )
        .unwrap();
    }
    drop(conn);

    let db = Database::open(DatabaseConfig::new(&path)).unwrap();
    let migrated = db
        .connection()
        .prepare(
            "SELECT path, tag, port, project, task, created_at, last_used_at
             FROM reservations ORDER BY port",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        migrated,
        vec![
            ("/alpha".into(), String::new(), 1, None, None, 0, 0,),
            (
                "/unicode/项目".into(),
                "web".into(),
                65535,
                Some("π".into()),
                Some("clock-skew".into()),
                200,
                100,
            ),
        ]
    );
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT value FROM metadata WHERE key = 'fixture_note'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "preserve me"
    );
}

#[test]
fn fresh_database_initializes_directly_as_v2() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("fresh.db");

    let db = Database::open(DatabaseConfig::new(path)).unwrap();

    assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
}

#[test]
fn schema_v2_rejects_invalid_keys_ports_and_timestamps_directly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("constraints.db");
    let db = Database::open(DatabaseConfig::new(path)).unwrap();
    let conn = db.connection();

    conn.execute(
        "INSERT INTO reservations VALUES ('/untagged', '', 5000, NULL, NULL, 10, 10)",
        [],
    )
    .unwrap();
    assert!(conn
        .execute(
            "INSERT INTO reservations VALUES ('/untagged', '', 5001, NULL, NULL, 10, 10)",
            [],
        )
        .is_err());
    conn.execute(
        "INSERT INTO reservations VALUES ('/tagged', 'web', 5001, NULL, NULL, 10, 10)",
        [],
    )
    .unwrap();
    assert!(conn
        .execute(
            "INSERT INTO reservations VALUES ('/tagged', 'web', 5002, NULL, NULL, 10, 10)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO reservations VALUES ('/duplicate-port', '', 5000, NULL, NULL, 10, 10)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO reservations VALUES ('/null-tag', NULL, 5002, NULL, NULL, 10, 10)",
            [],
        )
        .is_err());

    for (path, port, created_at, last_used_at) in [
        ("/zero", 0, 10, 10),
        ("/too-large", 65536, 10, 10),
        ("/negative-created", 5003, -1, 10),
        ("/negative-used", 5004, 10, -1),
    ] {
        assert!(
            conn.execute(
                "INSERT INTO reservations VALUES (?1, '', ?2, NULL, NULL, ?3, ?4)",
                params![path, port, created_at, last_used_at],
            )
            .is_err(),
            "v2 accepted invalid row {path}"
        );
    }

    // Clock regressions are allowed; only nonnegative storage is invariant.
    conn.execute(
        "INSERT INTO reservations VALUES ('/clock-skew', '', 5005, NULL, NULL, 20, 10)",
        [],
    )
    .unwrap();
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn invalid_legacy_states_are_aggregated_and_left_unchanged() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("blocked.db");
    let conn = Connection::open(&path).unwrap();
    let schema_without_unique_port =
        PUBLISHED_V1_SCHEMA.replace("port INTEGER NOT NULL UNIQUE", "port INTEGER NOT NULL");
    conn.execute_batch(&schema_without_unique_port).unwrap();
    conn.execute_batch(
        "INSERT INTO reservations VALUES ('/duplicate', NULL, 5000, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/duplicate', NULL, 5001, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/port-a', 'a', 5100, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/port-b', 'b', 5100, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/empty', '', 5101, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/zero', 'zero', 0, NULL, NULL, 1, 1);
         INSERT INTO reservations VALUES ('/time', 'time', 5102, NULL, NULL, -1, -2);
         INSERT INTO reservations VALUES (X'FF', 'blob', 5103, NULL, NULL, 1, 1);
         INSERT INTO metadata VALUES ('invalid_scalar', X'FF');",
    )
    .unwrap();
    drop(conn);

    let error = Database::open(DatabaseConfig::new(&path)).unwrap_err();
    let Error::MigrationBlocked { from, to, details } = error else {
        panic!("expected MigrationBlocked, got {error:?}");
    };
    assert_eq!((from, to), (1, 2));
    for category in [
        "duplicate logical reservation keys=1",
        "duplicate reserved ports=1",
        "empty legacy tags=1",
        "ports outside 1..=65535=1",
        "negative timestamps=1",
        "invalid reservation scalar types=1",
        "invalid metadata scalar types=1",
        "left unchanged",
        "will not choose or discard",
    ] {
        assert!(
            details.contains(category),
            "missing {category:?} in {details:?}"
        );
    }

    let conn = Connection::open(&path).unwrap();
    assert_eq!(get_schema_version(&conn).unwrap(), 1);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM reservations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        8
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name IN ('metadata_v2', 'reservations_v2')",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn read_only_v1_requires_writable_migration_but_read_only_v2_opens() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("read-only.db");
    create_published_v1_at(&path);

    let error = Database::open(DatabaseConfig::new(&path).read_only()).unwrap_err();
    assert!(matches!(
        error,
        Error::MigrationRequired {
            found: 1,
            target: 2,
            ..
        }
    ));
    assert_eq!(raw_schema_version(&path), 1);

    Database::open(DatabaseConfig::new(&path)).unwrap();
    let read_only = Database::open(DatabaseConfig::new(&path).read_only()).unwrap();
    assert_eq!(get_schema_version(read_only.connection()).unwrap(), 2);
}

#[test]
fn concurrent_migrators_serialize_and_both_observe_v2() {
    let dir = tempdir().unwrap();
    for iteration in 0..50 {
        let path = dir.path().join(format!("concurrent-{iteration}.db"));
        let conn = Connection::open(&path).unwrap();
        create_published_v1_database(&conn);
        conn.execute(
            "INSERT INTO reservations VALUES ('/project', NULL, 5000, NULL, NULL, 1, 1)",
            [],
        )
        .unwrap();
        drop(conn);

        let barrier = Arc::new(Barrier::new(3));
        let handles = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    Database::open(DatabaseConfig::new(path))
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        for handle in handles {
            let db = handle.join().unwrap().unwrap_or_else(|error| {
                panic!("concurrent migration iteration {iteration} failed: {error:?}")
            });
            assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
            assert_eq!(
                db.connection()
                    .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                5000
            );
        }

        let conn = Connection::open(&path).unwrap();
        assert_eq!(get_schema_version(&conn).unwrap(), 2);
        assert_eq!(
            conn.query_row(
                "SELECT tag FROM reservations WHERE path = '/project'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            ""
        );
    }
}

#[test]
fn migration_handles_active_wal_and_reader_snapshot() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("active-wal.db");
    let writer = Connection::open(&path).unwrap();
    let journal_mode: String = writer
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "wal");
    writer
        .execute_batch("PRAGMA wal_autocheckpoint = 0")
        .unwrap();
    create_published_v1_database(&writer);
    writer
        .execute(
            "INSERT INTO reservations VALUES ('/first', NULL, 5000, NULL, NULL, 1, 1)",
            [],
        )
        .unwrap();

    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT COUNT(*) FROM reservations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    writer
        .execute(
            "INSERT INTO reservations VALUES ('/second', 'web', 5001, NULL, NULL, 2, 2)",
            [],
        )
        .unwrap();

    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    assert!(PathBuf::from(wal_name).exists());

    let migrated = Database::open(DatabaseConfig::new(&path)).unwrap();
    assert_eq!(get_schema_version(migrated.connection()).unwrap(), 2);
    assert_eq!(
        migrated
            .connection()
            .query_row("SELECT COUNT(*) FROM reservations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        2
    );
    reader.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn simulated_sqlite_full_rolls_back_to_complete_v1() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sqlite-full.db");
    let conn = Connection::open(&path).unwrap();
    create_published_v1_database(&conn);
    for index in 0..100 {
        conn.execute(
            "INSERT INTO reservations
             (path, tag, port, project, task, created_at, last_used_at)
             VALUES (?1, NULL, ?2, NULL, NULL, 1, 1)",
            params![
                format!("/project/{index}/{}", "x".repeat(1024)),
                5000 + index
            ],
        )
        .unwrap();
    }
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap();
    let effective_limit: i64 = conn
        .query_row(
            &format!("PRAGMA max_page_count = {page_count}"),
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(effective_limit, page_count);

    let error = check_schema_compatibility(&conn).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Database(rusqlite::Error::SqliteFailure(ref sqlite, _))
                if sqlite.code == rusqlite::ErrorCode::DiskFull
        ),
        "expected SQLITE_FULL, got {error:?}"
    );
    assert_eq!(get_schema_version(&conn).unwrap(), 1);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE name IN ('metadata_v2', 'reservations_v2')",
            [],
            |row| row.get::<_, i64>(0)
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

#[test]
fn migration_is_idempotent_and_creates_no_backup_sidecar() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("idempotent.db");
    create_published_v1_at(&path);

    Database::open(DatabaseConfig::new(&path)).unwrap();
    let schema_after_first = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT group_concat(sql, '\n') FROM sqlite_schema
             WHERE sql IS NOT NULL ORDER BY name",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    Database::open(DatabaseConfig::new(&path)).unwrap();
    let schema_after_second = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT group_concat(sql, '\n') FROM sqlite_schema
             WHERE sql IS NOT NULL ORDER BY name",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();

    assert_eq!(schema_after_first, schema_after_second);
    assert!(
        dir.path().read_dir().unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("bak")),
        "migration must not imply a persistent downgrade backup"
    );
}

#[test]
fn published_v1_compatibility_check_rejects_v2_without_mutation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("old-client.db");
    let db = Database::open(DatabaseConfig::new(&path)).unwrap();
    drop(db);

    // This is the exact future-version branch shipped in trop 0.1.0 at
    // eaceea6bc196fc5f787e2320bf6016e1a6f6bf88.
    let old_client = Connection::open(&path).unwrap();
    let before = old_client.total_changes();
    let version = get_schema_version(&old_client).unwrap();
    let old_result = if version > 1 {
        Err(format!(
            "Database schema version {version} is newer than client version 1. Please upgrade trop."
        ))
    } else {
        Ok(())
    };

    assert!(old_result
        .unwrap_err()
        .contains("newer than client version 1"));
    assert_eq!(old_client.total_changes(), before);
    assert_eq!(get_schema_version(&old_client).unwrap(), 2);
}
