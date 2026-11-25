//! Regression tests for storage bugs fixed with the verified storage,
//! expiry and schema-version kernels. Each test fails on the implementation
//! that preceded them.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::{params, Connection};
use tempfile::tempdir;
use trop::database::{get_schema_version, Database, DatabaseConfig};
use trop::{Error, Port, Reservation, ReservationKey};

const PUBLISHED_V1_SCHEMA: &str = include_str!("fixtures/published-v1-schema.sql");

fn open(path: &Path) -> Database {
    Database::open(DatabaseConfig::new(path)).unwrap()
}

fn reservation(path: &str, tag: Option<&str>, port: u16) -> Reservation {
    let key = ReservationKey::new(PathBuf::from(path), tag.map(str::to_owned)).unwrap();
    Reservation::builder(key, Port::try_from(port).unwrap())
        .build()
        .unwrap()
}

fn struct_key(path: &str, tag: Option<&str>) -> ReservationKey {
    ReservationKey {
        path: PathBuf::from(path),
        tag: tag.map(str::to_owned),
    }
}

fn paths(reservations: &[Reservation]) -> Vec<String> {
    let mut paths = reservations
        .iter()
        .map(|reservation| reservation.key().path.to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_validation(result: trop::Result<impl std::fmt::Debug>, field: &str) {
    match result {
        Err(Error::Validation { field: actual, .. }) => assert_eq!(actual, field),
        other => panic!("expected a validation error for {field}, got {other:?}"),
    }
}

fn corruption_details(error: Error) -> String {
    match error {
        Error::DatabaseCorruption { details } => details,
        other => panic!("expected typed database corruption, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

/// `max_age.as_secs() as i64` wrapped for ages of 2^63 seconds or more, so the
/// cutoff landed in the future and every reservation was reported expired.
#[test]
fn huge_max_age_expires_nothing() {
    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    db.create_reservation(&reservation("/fresh", None, 5000))
        .unwrap();
    let old = Reservation::builder(
        ReservationKey::new(PathBuf::from("/old"), None).unwrap(),
        Port::try_from(5001).unwrap(),
    )
    .last_used_at(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
    .build()
    .unwrap();
    db.create_reservation(&old).unwrap();

    for secs in [u64::MAX, 1 << 63, (1 << 63) + 1, i64::MAX as u64] {
        let expired =
            Database::find_expired_reservations(db.connection(), Duration::from_secs(secs))
                .unwrap();
        assert!(expired.is_empty(), "max_age {secs}: {expired:?}");
    }
    // The boundary is unchanged for ordinary ages.
    let expired =
        Database::find_expired_reservations(db.connection(), Duration::from_secs(3600)).unwrap();
    assert_eq!(paths(&expired), ["/old"]);
}

// ---------------------------------------------------------------------------
// Prefix queries
// ---------------------------------------------------------------------------

// The prefix-query tests use Unix path spellings.
#[cfg(unix)]
fn prefix_database(dir: &Path, stored: &[&str]) -> Database {
    let mut db = open(&dir.join("trop.db"));
    for (offset, path) in (0u16..).zip(stored) {
        db.create_reservation(&reservation(path, None, 5000 + offset))
            .unwrap();
    }
    db
}

/// The SQL prefilter used the raw prefix, so a trailing or doubled separator
/// dropped true matches: `/q/b/` omitted `/q/b` itself, and `/a//b` matched
/// nothing.
#[test]
#[cfg(unix)]
fn prefix_query_is_component_wise_whatever_the_prefix_spelling() {
    let dir = tempdir().unwrap();
    let db = prefix_database(
        dir.path(),
        &["/q/b", "/q/b/c", "/q/bc", "/a/b", "/a/b/x/y", "/a/bc"],
    );
    let conn = db.connection();
    let query = |prefix: &str| {
        paths(&Database::get_reservations_by_path_prefix(conn, Path::new(prefix)).unwrap())
    };

    for prefix in ["/q/b", "/q/b/", "/q//b", "/q/./b/", "//q/b"] {
        assert_eq!(query(prefix), ["/q/b", "/q/b/c"], "{prefix}");
    }
    for prefix in ["/a//b", "/a/b/", "/a/b//", "/./a/b"] {
        assert_eq!(query(prefix), ["/a/b", "/a/b/x/y"], "{prefix}");
    }
    assert_eq!(query("/a/b/x"), ["/a/b/x/y"]);
    assert_eq!(
        query("/"),
        ["/a/b", "/a/b/x/y", "/a/bc", "/q/b", "/q/b/c", "/q/bc"]
    );
    // `..` components never match a stored (normalized) path, exactly as
    // `Path::starts_with` decides.
    assert!(query("/a/b/..").is_empty());
    assert!(query("/q/../q/b").is_empty());
    // A relative prefix matches no absolute path; the empty prefix matches
    // every path, as `Path::starts_with("")` does.
    assert!(query("q/b").is_empty());
    assert_eq!(query("").len(), 6);
}

/// `%`, `_` and ASCII case in stored paths and prefixes are plain bytes: the
/// byte-exact `instr` prefilter and the component-wise post-filter keep
/// exactly the true matches.
#[test]
#[cfg(unix)]
fn prefix_query_ignores_like_wildcards_and_case() {
    let dir = tempdir().unwrap();
    let db = prefix_database(
        dir.path(),
        &[
            "/x_y", "/x_y/z", "/xay", "/x%y", "/x%y/z", "/xzzy", "/X_Y", "/p\\q",
        ],
    );
    let conn = db.connection();
    let query = |prefix: &str| {
        paths(&Database::get_reservations_by_path_prefix(conn, Path::new(prefix)).unwrap())
    };
    assert_eq!(query("/x_y"), ["/x_y", "/x_y/z"]);
    assert_eq!(query("/x%y/"), ["/x%y", "/x%y/z"]);
    assert_eq!(query("/X_Y"), ["/X_Y"]);
    assert_eq!(query("/p\\q"), ["/p\\q"]);
}

/// The prefilter compares whole byte strings, so it keeps every true match
/// when stored paths or the prefix contain a NUL.
#[test]
#[cfg(unix)]
fn prefix_query_keeps_matches_with_nul_bytes() {
    let dir = tempdir().unwrap();
    let db = prefix_database(dir.path(), &["/n\0ul", "/n\0ul/x", "/n\0ux", "/n/x\0y"]);
    let conn = db.connection();
    let query = |prefix: &str| {
        paths(&Database::get_reservations_by_path_prefix(conn, Path::new(prefix)).unwrap())
    };
    assert_eq!(query("/n\0ul"), ["/n\0ul", "/n\0ul/x"]);
    assert_eq!(query("/n\0ul/"), ["/n\0ul", "/n\0ul/x"]);
    assert_eq!(query("/n"), ["/n/x\0y"]);
    assert_eq!(query("/n/x\0y"), ["/n/x\0y"]);
}

/// `LIKE` patterns are limited to 50,000 bytes, so a prefix query by a long
/// key failed with "LIKE or GLOB pattern too complex" although the stored
/// path was writable. The `instr` prefilter has no such limit.
#[test]
#[cfg(unix)]
fn prefix_query_by_a_long_key_finds_its_matches() {
    let long = format!("/{}", "a".repeat(60_000));
    let below = format!("{long}/x");
    let sibling = format!("{long}b/x");
    let dir = tempdir().unwrap();
    let db = prefix_database(dir.path(), &[&below, &sibling, "/other"]);
    let found =
        Database::get_reservations_by_path_prefix(db.connection(), Path::new(&long)).unwrap();
    assert_eq!(paths(&found), [below]);
}

/// A prefix that is not valid UTF-8 cannot be a component prefix of any
/// stored (UTF-8) path.
#[test]
#[cfg(unix)]
fn non_utf8_prefix_matches_nothing() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempdir().unwrap();
    let db = prefix_database(dir.path(), &["/tmp/a\u{fffd}", "/tmp/a\u{fffd}/b"]);
    let prefix = Path::new(OsStr::from_bytes(b"/tmp/a\xff"));
    assert!(
        Database::get_reservations_by_path_prefix(db.connection(), prefix)
            .unwrap()
            .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Lossy path keys
// ---------------------------------------------------------------------------

/// `to_string_lossy` aliased distinct non-UTF-8 paths onto one stored key:
/// creating `/tmp/a<FF>` and then `/tmp/a<FE>` deleted the first
/// reservation, and the first key then read back the second key's port.
#[test]
#[cfg(unix)]
fn non_utf8_paths_are_rejected_instead_of_aliased() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    let first = ReservationKey::new(
        PathBuf::from(OsString::from_vec(b"/tmp/a\xff".to_vec())),
        None,
    )
    .unwrap();
    let second = ReservationKey::new(
        PathBuf::from(OsString::from_vec(b"/tmp/a\xfe".to_vec())),
        None,
    )
    .unwrap();
    for (key, port) in [(&first, 5000), (&second, 5001)] {
        let reservation = Reservation::builder(key.clone(), Port::try_from(port).unwrap())
            .build()
            .unwrap();
        assert_validation(db.create_reservation(&reservation), "path");
    }
    assert!(Database::list_all_reservations(db.connection())
        .unwrap()
        .is_empty());

    // A lossy spelling stored by an earlier version is never returned for a
    // non-UTF-8 key.
    db.create_reservation(&reservation("/tmp/a\u{fffd}", None, 5002))
        .unwrap();
    assert!(Database::get_reservation(db.connection(), &first)
        .unwrap()
        .is_none());
    assert!(Database::get_reservation(db.connection(), &second)
        .unwrap()
        .is_none());
    assert_validation(db.delete_reservation(&first), "path");
    assert_eq!(
        Database::list_all_reservations(db.connection())
            .unwrap()
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Write-side validation
// ---------------------------------------------------------------------------

/// Writes were not validated while reads were: a library write of `/a/b/`,
/// a relative path or an untrimmed tag made every later read fail with
/// `DatabaseCorruption`.
#[test]
fn unreadable_keys_are_rejected_before_any_write() {
    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    db.create_reservation(&reservation("/kept", None, 5000))
        .unwrap();

    let bad_keys = [
        (
            ReservationKey::new(PathBuf::from("/a/b/"), None).unwrap(),
            "path",
        ),
        (
            ReservationKey::new(PathBuf::from("relative"), None).unwrap(),
            "path",
        ),
        (
            ReservationKey::new(PathBuf::from(""), None).unwrap(),
            "path",
        ),
        (
            ReservationKey::new(PathBuf::from("/a/../b"), None).unwrap(),
            "path",
        ),
        (
            ReservationKey::new(PathBuf::from("/a/./b"), None).unwrap(),
            "path",
        ),
        (struct_key("/tagged", Some(" web ")), "tag"),
        (struct_key("/tagged", Some("   ")), "tag"),
        (struct_key("/tagged", Some("")), "tag"),
    ];
    for (port, (key, field)) in (5100u16..).zip(&bad_keys) {
        let bad = Reservation::builder(key.clone(), Port::try_from(port).unwrap())
            .build()
            .unwrap();
        assert_validation(db.create_reservation(&bad), field);
        assert_validation(
            Database::create_reservation_simple(db.connection(), &bad),
            field,
        );
        assert_validation(
            db.batch_create_reservations(std::slice::from_ref(&bad)),
            field,
        );
        assert_validation(db.update_last_used(key), field);
        assert_validation(
            Database::update_last_used_simple(db.connection(), key),
            field,
        );
        assert_validation(db.delete_reservation(key), field);
        assert_validation(
            Database::delete_reservation_simple(db.connection(), key),
            field,
        );
        assert_validation(
            db.batch_delete_reservations(std::slice::from_ref(key)),
            field,
        );
        // Lookups find nothing rather than another key's row.
        assert!(Database::get_reservation(db.connection(), key)
            .unwrap()
            .is_none());
    }

    // Nothing was written, and the database still reads and validates.
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(paths(&all), ["/kept"]);
    db.verify_integrity().unwrap();
}

/// A struct-literal `Some("")` tag was stored as `""`, the untagged
/// sentinel, so it silently replaced (or deleted) the untagged reservation at
/// that path.
#[test]
fn empty_tag_never_aliases_the_untagged_reservation() {
    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    db.create_reservation(&reservation("/project", None, 5000))
        .unwrap();

    let aliasing = Reservation::builder(
        struct_key("/project", Some("")),
        Port::try_from(5001).unwrap(),
    )
    .build()
    .unwrap();
    assert_validation(db.create_reservation(&aliasing), "tag");
    assert_validation(
        db.batch_create_reservations(&[reservation("/other", None, 5002), aliasing]),
        "tag",
    );
    assert_validation(
        db.delete_reservation(&struct_key("/project", Some(""))),
        "tag",
    );
    assert!(
        Database::get_reservation(db.connection(), &struct_key("/project", Some("")))
            .unwrap()
            .is_none()
    );

    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].key().tag, None);
    assert_eq!(all[0].port().value(), 5000);
}

/// Lookups match a key's exact text: a corrupt row stored under the exact
/// text of a key that could not be written is still read and reported (as
/// before the write-side checks), while a `Some("")` tag finds nothing
/// rather than the untagged reservation at the same path.
#[test]
fn lookups_report_corrupt_rows_and_never_alias() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("trop.db");
    let mut db = open(&db_path);
    db.create_reservation(&reservation("/project", None, 5000))
        .unwrap();
    {
        let raw = Connection::open(&db_path).unwrap();
        raw.execute(
            "INSERT INTO reservations VALUES ('/a/b/', ' web ', 5001, NULL, NULL, 1, 1)",
            [],
        )
        .unwrap();
    }

    let details = corruption_details(
        Database::get_reservation(db.connection(), &struct_key("/a/b/", Some(" web ")))
            .unwrap_err(),
    );
    assert!(details.contains("field=path"), "{details}");

    // Other keys at those paths have no row.
    assert!(
        Database::get_reservation(db.connection(), &struct_key("/a/b/", None))
            .unwrap()
            .is_none()
    );
    assert!(
        Database::get_reservation(db.connection(), &struct_key("/project", Some("")))
            .unwrap()
            .is_none()
    );
    let untagged = Database::get_reservation(db.connection(), &struct_key("/project", None))
        .unwrap()
        .unwrap();
    assert_eq!(untagged.port().value(), 5000);
}

/// A deserialized reservation can carry identifiers the builder would never
/// produce; the writer refuses them instead of storing an unreadable row.
#[test]
fn untrimmed_project_and_task_are_rejected_before_any_write() {
    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    let template = Reservation::builder(
        ReservationKey::new(PathBuf::from("/p"), None).unwrap(),
        Port::try_from(5000).unwrap(),
    )
    .project(Some("proj".into()))
    .task(Some("task".into()))
    .build()
    .unwrap();
    let json = serde_json::to_value(&template).unwrap();
    for (field, value) in [("project", " proj "), ("project", ""), ("task", "\ttask")] {
        let mut tampered = json.clone();
        tampered[field] = serde_json::Value::String(value.into());
        let tampered: Reservation = serde_json::from_value(tampered).unwrap();
        assert_validation(db.create_reservation(&tampered), field);
    }
    assert!(Database::list_all_reservations(db.connection())
        .unwrap()
        .is_empty());
}

/// Everything the writer accepts reads back unchanged: the key, port,
/// project, task and whole-second timestamps. `sticky` is not persisted.
#[test]
fn written_reservations_round_trip() {
    let dir = tempdir().unwrap();
    let mut db = open(&dir.path().join("trop.db"));
    let created = SystemTime::UNIX_EPOCH + Duration::new(1_700_000_000, 999_999_999);
    let used = SystemTime::UNIX_EPOCH + Duration::new(1_700_000_100, 5);
    let original = Reservation::builder(
        ReservationKey::new(PathBuf::from("/unicode/项目 x"), Some("wéb".into())).unwrap(),
        Port::try_from(65535).unwrap(),
    )
    .project(Some("π".into()))
    .task(Some("t".into()))
    .sticky(true)
    .created_at(created)
    .last_used_at(used)
    .build()
    .unwrap();
    db.create_reservation(&original).unwrap();

    let loaded = Database::get_reservation(db.connection(), original.key())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.key(), original.key());
    assert_eq!(loaded.port(), original.port());
    assert_eq!(loaded.project(), original.project());
    assert_eq!(loaded.task(), original.task());
    assert_eq!(
        loaded.created_at(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    );
    assert_eq!(
        loaded.last_used_at(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_100)
    );
    assert!(original.sticky());
    assert!(!loaded.sticky(), "sticky is not persisted");
}

// ---------------------------------------------------------------------------
// Schema-version metadata
// ---------------------------------------------------------------------------

/// A missing `schema_version` row was treated as an uninitialized database
/// and silently restamped to `2`, although validation reports the same file
/// as corrupt.
#[test]
fn missing_schema_version_row_is_corruption_not_reinitialized() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(open(&path));
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM metadata WHERE key = 'schema_version'", [])
        .unwrap();
    drop(conn);

    let details = corruption_details(Database::open(DatabaseConfig::new(&path)).unwrap_err());
    assert!(details.contains("schema_version"), "{details}");
    let conn = Connection::open(&path).unwrap();
    assert!(get_schema_version(&conn).is_err());
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0, "the corrupt database must not be restamped");
    drop(conn);
    assert!(matches!(
        Database::validate(&DatabaseConfig::new(&path)),
        Err(Error::DatabaseCorruption { .. })
    ));
}

/// A `reservations` table without its `metadata` table was reinitialized
/// around the existing rows.
#[test]
fn missing_metadata_table_with_reservations_is_corruption() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    let mut db = open(&path);
    db.create_reservation(&reservation("/project", None, 5000))
        .unwrap();
    drop(db);
    let conn = Connection::open(&path).unwrap();
    conn.execute("DROP TABLE metadata", []).unwrap();
    drop(conn);

    let details = corruption_details(Database::open(DatabaseConfig::new(&path)).unwrap_err());
    assert!(details.contains("metadata"), "{details}");
    let conn = Connection::open(&path).unwrap();
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'metadata'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "the corrupt database must not be reinitialized");
}

/// A stored version `0` with tables present is not an uninitialized database.
#[test]
fn stored_version_zero_is_not_reinitialized() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(open(&path));
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE metadata SET value = '0' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);

    assert!(matches!(
        Database::open(DatabaseConfig::new(&path)),
        Err(Error::MigrationRequired { found: 0, .. })
    ));
    let conn = Connection::open(&path).unwrap();
    let value: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "0");
}

/// Concurrent first opens of a new database never see each other's
/// initialization half-done. Reading the metadata in two autocommit
/// statements let an opener see a `reservations` table without a `metadata`
/// table and report the healthy database as corrupt.
#[test]
fn concurrent_first_opens_never_report_corruption() {
    use std::sync::{Arc, Barrier};

    const THREADS: usize = 8;
    const ROUNDS: usize = 200;
    let mut failures = Vec::new();
    for _ in 0..ROUNDS {
        let dir = tempdir().unwrap();
        let path = dir.path().join("trop.db");
        let barrier = Arc::new(Barrier::new(THREADS));
        let openers = (0..THREADS)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let path = path.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Database::open(DatabaseConfig::new(&path)).map(|_| ())
                })
            })
            .collect::<Vec<_>>();
        for opener in openers {
            if let Err(error) = opener.join().unwrap() {
                failures.push(error.to_string());
            }
        }
        assert_eq!(get_schema_version(open(&path).connection()).unwrap(), 2);
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// A truly empty database still initializes.
#[test]
fn empty_database_initializes() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(Connection::open(&path).unwrap());
    let db = open(&path);
    assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
}

// ---------------------------------------------------------------------------
// Legacy (schema v1) preflight
// ---------------------------------------------------------------------------

/// A legacy row: path, tag, port, project, task, `last_used_at`.
type LegacyRowValues<'a> = (
    &'a str,
    Option<&'a str>,
    i64,
    Option<&'a str>,
    Option<&'a str>,
    i64,
);

fn blocked_details(rows: &[LegacyRowValues<'_>]) -> String {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
    for (path, tag, port, project, task, last_used) in rows {
        conn.execute(
            "INSERT INTO reservations
             (path, tag, port, project, task, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)",
            params![path, tag, port, project, task, last_used],
        )
        .unwrap();
    }
    drop(conn);

    let error = Database::open(DatabaseConfig::new(&path)).unwrap_err();
    let Error::MigrationBlocked { from, to, details } = error else {
        panic!("expected MigrationBlocked, got {error:?}");
    };
    assert_eq!((from, to), (1, 2));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(get_schema_version(&conn).unwrap(), 1, "v1 left unchanged");
    details
}

/// The v1-to-v2 preflight missed untrimmed tags, projects and tasks,
/// non-canonical paths and out-of-range timestamps; such rows aborted the
/// migration with a generic corruption error instead of the aggregated
/// report.
#[test]
fn legacy_preflight_reports_every_row_the_v2_decoder_would_reject() {
    for (row, category) in [
        (
            ("/padded-tag", Some(" web "), 5000, None, None, 1),
            "blank or untrimmed tags=1",
        ),
        (
            ("/blank-tag", Some("   "), 5000, None, None, 1),
            "blank or untrimmed tags=1",
        ),
        (
            ("/project", None, 5000, Some(" proj "), None, 1),
            "empty or untrimmed project values=1",
        ),
        (
            ("/task", None, 5000, None, Some(""), 1),
            "empty or untrimmed task values=1",
        ),
        (
            ("/trailing/", None, 5000, None, None, 1),
            "paths not in canonical absolute form=1",
        ),
        (
            ("relative", None, 5000, None, None, 1),
            "paths not in canonical absolute form=1",
        ),
        (
            ("/time", None, 5000, None, None, 8_210_266_876_800),
            "timestamps beyond the supported range=1",
        ),
    ] {
        let details = blocked_details(&[row]);
        assert!(
            details.contains(category),
            "missing {category:?} in {details:?}"
        );
        assert!(details.contains("left unchanged"), "{details}");
    }

    // Categories aggregate over rows.
    let details = blocked_details(&[
        ("/a/", Some(" x"), 5000, Some(" p"), Some("t "), 1),
        ("/b/", Some("y "), 5001, None, None, 8_210_266_876_800),
    ]);
    for category in [
        "paths not in canonical absolute form=2",
        "blank or untrimmed tags=2",
        "empty or untrimmed project values=1",
        "empty or untrimmed task values=1",
        "timestamps beyond the supported range=1",
    ] {
        assert!(
            details.contains(category),
            "missing {category:?} in {details:?}"
        );
    }
}

/// Invalid UTF-8 text in a legacy row is reported by the preflight.
#[test]
fn legacy_preflight_reports_invalid_utf8_text() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
    conn.execute(
        "INSERT INTO reservations
         (path, tag, port, project, task, created_at, last_used_at)
         VALUES ('/project', CAST(X'FF' AS TEXT), 5000, NULL, NULL, 1, 1)",
        [],
    )
    .unwrap();
    drop(conn);

    let error = Database::open(DatabaseConfig::new(&path)).unwrap_err();
    let Error::MigrationBlocked { details, .. } = error else {
        panic!("expected MigrationBlocked, got {error:?}");
    };
    assert!(
        details.contains("text values that are not valid UTF-8=1"),
        "{details}"
    );
}

/// Rows the preflight accepts migrate and decode with the same key.
#[test]
fn legacy_rows_that_pass_the_preflight_migrate_and_decode() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
    for (path, tag, port) in [
        ("/", None, 1),
        ("/a", Some("web"), 5000),
        ("/a", None, 5001),
        ("/b/项目", Some("\u{200b}x"), 65535),
    ] {
        conn.execute(
            "INSERT INTO reservations
             (path, tag, port, project, task, created_at, last_used_at)
             VALUES (?1, ?2, ?3, 'p', NULL, 0, 8210266876799)",
            params![path, tag, port],
        )
        .unwrap();
    }
    drop(conn);

    let db = open(&path);
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 4);
    assert!(all
        .iter()
        .any(|r| r.key() == &struct_key("/a", Some("web"))));
    assert!(all.iter().any(|r| r.key() == &struct_key("/a", None)));
}
