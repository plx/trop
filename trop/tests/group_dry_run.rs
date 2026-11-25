//! `ReserveGroupPlan::dry_run` / `AutoreservePlan::dry_run` predict the live
//! outcome without writing.
//!
//! A group dry run builds and executes its plan exactly as a live run does,
//! against a private in-memory copy of the database: it reports the ports
//! (and warnings) a live run on the same snapshot would, fails with the same
//! error, and never writes, creates, or migrates the database file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use trop::operations::{
    AutoreserveOptions, AutoreservePlan, ReserveGroupOptions, ReserveGroupPlan,
};
use trop::{Database, DatabaseConfig, Error, PlanExecutor, Port, Reservation, ReservationKey};

const TIMEOUT: Duration = Duration::from_secs(5);

fn group_config(project: &Path, max: u16, web_offset: u16) -> PathBuf {
    let config_path = project.join("trop.yaml");
    fs::write(
        &config_path,
        format!(
            "project: dry-run-test
ports:
  min: 5000
  max: {max}
occupancy_check:
  skip: true
reservations:
  services:
    web:
      offset: {web_offset}
    api:
      offset: 2
"
        ),
    )
    .unwrap();
    config_path
}

fn planner(config_path: &Path) -> ReserveGroupPlan {
    ReserveGroupPlan::new(
        ReserveGroupOptions::new(config_path.to_path_buf()).with_allow_unrelated_path(true),
    )
    .unwrap()
}

/// Every stored row with its timestamps, in a stable order.
fn rows(db_path: &Path) -> Vec<(String, String, i64, i64, i64)> {
    let connection = rusqlite::Connection::open(db_path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT path, tag, port, created_at, last_used_at FROM reservations
             ORDER BY path, tag",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn live(db_path: &Path, planner: &ReserveGroupPlan) -> trop::Result<trop::ExecutionResult> {
    let mut db = Database::open(DatabaseConfig::new(db_path)).unwrap();
    let tx = db.begin_transaction()?;
    let plan = planner.build_plan(&tx)?;
    let result = PlanExecutor::new(&tx).execute(&plan)?;
    tx.commit()?;
    Ok(result)
}

fn port(value: u16) -> Port {
    Port::try_from(value).unwrap()
}

#[test]
fn dry_run_without_a_database_plans_against_an_empty_one_and_creates_nothing() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("data").join("trop.db");

    let result = planner(&config_path).dry_run(None, TIMEOUT).unwrap();

    assert!(result.dry_run);
    let ports = result.allocated_ports.unwrap();
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert_eq!(ports.get("api"), Some(&port(5002)));
    assert!(!db_path.exists());
    assert!(!dir.path().join("data").exists());
}

#[test]
fn dry_run_predicts_the_live_allocation_and_leaves_the_database_unchanged() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    {
        // Another reservation occupies the first base.
        let mut db = Database::open(DatabaseConfig::new(&db_path)).unwrap();
        let other = dir.path().join("other");
        fs::create_dir(&other).unwrap();
        let reservation = Reservation::builder(
            ReservationKey::new(other, None).unwrap(),
            Port::try_from(5002).unwrap(),
        )
        .build()
        .unwrap();
        db.create_reservation(&reservation).unwrap();
    }
    let before = rows(&db_path);
    let planner = planner(&config_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap();
    assert_eq!(rows(&db_path), before, "a dry run must not write");

    let live = live(&db_path, &planner).unwrap();
    assert_eq!(dry.allocated_ports, live.allocated_ports);
    let ports = live.allocated_ports.unwrap();
    assert_eq!(ports.get("web"), Some(&port(5001)));
    assert_eq!(ports.get("api"), Some(&port(5003)));
}

#[test]
fn dry_run_of_a_stored_group_reports_its_ports_without_refreshing_it() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    let planner = planner(&config_path);
    let first = live(&db_path, &planner).unwrap();
    {
        // Age the rows so that a refresh would show.
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute("UPDATE reservations SET last_used_at = 1000", [])
            .unwrap();
    }
    let before = rows(&db_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap();

    assert_eq!(dry.allocated_ports, first.allocated_ports);
    assert_eq!(rows(&db_path), before, "a dry run must not refresh");
}

#[test]
fn dry_run_fails_exactly_like_the_live_run() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    live(&db_path, &planner(&config_path)).unwrap();
    // The stored group no longer has the configured shape.
    let config_path = group_config(dir.path(), 7000, 1);
    let planner = planner(&config_path);
    let before = rows(&db_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap_err();
    assert_eq!(rows(&db_path), before);
    let live = live(&db_path, &planner).unwrap_err();

    assert!(matches!(dry, Error::ReservationConflict { .. }), "{dry}");
    assert_eq!(dry.to_string(), live.to_string());
    assert_eq!(rows(&db_path), before);
}

#[test]
fn dry_run_simulates_the_automatic_cleanup_and_retry() {
    let dir = TempDir::new().unwrap();
    // Range 5000..=5002 fits only base 5000 for offsets 0 and 2.
    let config_path = group_config(dir.path(), 5002, 0);
    let db_path = dir.path().join("trop.db");
    {
        let mut db = Database::open(DatabaseConfig::new(&db_path)).unwrap();
        let other = dir.path().join("other");
        fs::create_dir(&other).unwrap();
        let used = SystemTime::now() - Duration::from_secs(40 * 24 * 60 * 60);
        let reservation = Reservation::builder(
            ReservationKey::new(other, None).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .created_at(used)
        .last_used_at(used)
        .build()
        .unwrap();
        db.create_reservation(&reservation).unwrap();
    }
    let before = rows(&db_path);
    let planner = planner(&config_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap();

    assert_eq!(rows(&db_path), before, "the cleanup ran only on the copy");
    let ports = dry.allocated_ports.clone().unwrap();
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert!(
        dry.warnings
            .iter()
            .any(|warning| warning.contains("Automatic cleanup")),
        "{:?}",
        dry.warnings
    );
    let live = live(&db_path, &planner).unwrap();
    assert_eq!(dry.allocated_ports, live.allocated_ports);
}

#[test]
fn autoreserve_dry_run_delegates_to_the_group_dry_run() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    group_config(&project, 7000, 0);

    let planner = AutoreservePlan::new(
        AutoreserveOptions::new(project.clone()).with_allow_unrelated_path(true),
    )
    .unwrap();
    let result = planner.dry_run(None, TIMEOUT).unwrap();

    assert!(result.dry_run);
    let ports = result.allocated_ports.unwrap();
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert_eq!(ports.get("api"), Some(&port(5002)));
}

#[test]
fn dry_run_refuses_an_unsupported_schema_like_the_live_run() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    {
        // A database from a newer trop: a live run refuses it, and so must a
        // dry run.
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata VALUES ('schema_version', '99');",
            )
            .unwrap();
    }
    let planner = planner(&config_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap_err();
    let live = Database::open(DatabaseConfig::new(&db_path)).unwrap_err();

    assert!(
        matches!(dry, Error::UnsupportedSchemaVersion { .. }),
        "{dry}"
    );
    assert_eq!(dry.to_string(), live.to_string());
}

const PUBLISHED_V1_SCHEMA: &str = include_str!("fixtures/published-v1-schema.sql");

/// The stored schema version, read without changing the file.
fn schema_version(db_path: &Path) -> i32 {
    let connection =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    trop::database::get_schema_version(&connection).unwrap()
}

/// A schema-v1 database (the published 0.1.0 layout) is migrated only in
/// the dry run's private copy: the dry run predicts the ports the live run
/// (which migrates the file) then reserves, and the file's bytes and schema
/// version are unchanged by the dry run.
#[test]
fn dry_run_migrates_a_v1_database_only_in_memory() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    {
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
        // One untagged v1 row (NULL tag) holding the first base.
        connection
            .execute(
                "INSERT INTO reservations
                 (path, tag, port, project, task, created_at, last_used_at)
                 VALUES ('/elsewhere', NULL, 5000, 'other', NULL, 1700000000, 1700000000)",
                [],
            )
            .unwrap();
    }
    let before = fs::read(&db_path).unwrap();
    let planner = planner(&config_path);

    let dry = planner.dry_run(Some(&db_path), TIMEOUT).unwrap();

    assert_eq!(
        fs::read(&db_path).unwrap(),
        before,
        "a dry run never migrates or writes the file"
    );
    assert_eq!(schema_version(&db_path), 1);
    let ports = dry.allocated_ports.clone().unwrap();
    assert_eq!(ports.get("web"), Some(&port(5001)));
    assert_eq!(ports.get("api"), Some(&port(5003)));

    let live = live(&db_path, &planner).unwrap();
    assert_eq!(dry.allocated_ports, live.allocated_ports);
    assert_eq!(
        schema_version(&db_path),
        2,
        "the live run migrates the file"
    );
}

/// A busy timeout `SQLite` cannot store (above `i32::MAX` milliseconds) is
/// rejected by the dry run with the live run's validation error, with or
/// without a database file. It used to panic inside rusqlite for an
/// existing database, and to be ignored without one.
#[test]
fn dry_run_rejects_a_busy_timeout_sqlite_cannot_store_like_a_live_run() {
    let dir = TempDir::new().unwrap();
    let config_path = group_config(dir.path(), 7000, 0);
    let db_path = dir.path().join("trop.db");
    Database::open(DatabaseConfig::new(&db_path)).unwrap();
    let planner = planner(&config_path);
    let limit = Duration::from_millis(i32::MAX.unsigned_abs().into());
    let too_long = limit + Duration::from_millis(1);

    let live = Database::open(DatabaseConfig::new(&db_path).with_busy_timeout(too_long))
        .unwrap_err()
        .to_string();
    for database in [Some(db_path.as_path()), None] {
        let dry = planner.dry_run(database, too_long).unwrap_err();
        assert!(matches!(dry, Error::Validation { .. }), "{dry}");
        assert_eq!(dry.to_string(), live);
    }
    // Sub-millisecond excess does not count, as `Duration::as_millis` rounds
    // down; `Database::open` accepts it too.
    let just_within = limit + Duration::from_nanos(999_999);
    Database::open(DatabaseConfig::new(&db_path).with_busy_timeout(just_within)).unwrap();
    for database in [Some(db_path.as_path()), None] {
        planner.dry_run(database, just_within).unwrap();
        planner.dry_run(database, limit).unwrap();
        planner.dry_run(database, Duration::ZERO).unwrap();
    }
}

/// The dry run's stand-in for the live run's directory creation: it accepts
/// an existing directory or missing directories under one, and rejects (with
/// an I/O error, the live run's error class) a database whose directory or
/// any ancestor of it is not a directory -- creating nothing either way.
#[test]
fn dry_run_checks_that_the_database_directory_can_be_created() {
    use trop::operations::reserve_group::check_database_directory_creatable;

    let dir = TempDir::new().unwrap();
    let file = dir.path().join("plain-file");
    fs::write(&file, "not a directory").unwrap();

    check_database_directory_creatable(&dir.path().join("trop.db")).unwrap();
    let nested = dir.path().join("a").join("b").join("trop.db");
    check_database_directory_creatable(&nested).unwrap();
    assert!(!dir.path().join("a").exists(), "nothing is created");
    check_database_directory_creatable(Path::new("trop.db")).unwrap();

    for db_path in [
        file.join("trop.db"),
        file.join("data").join("trop.db"),
        file.join("data").join("deeper").join("trop.db"),
    ] {
        let dry = check_database_directory_creatable(&db_path).unwrap_err();
        assert!(matches!(dry, Error::Io(_)), "{dry}");
        assert!(dry.to_string().contains("is not a directory"), "{dry}");
        // The live open fails with the same error class.
        let live = Database::open(DatabaseConfig::new(&db_path)).unwrap_err();
        assert!(matches!(live, Error::Io(_)), "{live}");
    }
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "not a directory",
        "the obstruction is left alone"
    );
}

/// `check_database_directory_creatable` is trusted glue that models the live
/// run's `create_dir_all` with an `lstat` walk. Pin it differentially on the
/// cases where such a walk is most likely to diverge from `mkdir`: symlinks
/// (dangling, to a file, to a directory, in a loop) and `..` after a file.
/// Each case builds the same tree twice; the dry check runs on one copy and
/// the live open (which creates the directory) on the other, and the two
/// must agree on success.
#[cfg(unix)]
#[test]
fn dry_directory_check_agrees_with_the_live_open_on_symlinks_and_dot_dot() {
    use std::os::unix::fs::symlink;
    use trop::operations::reserve_group::check_database_directory_creatable;

    fn tree(root: &Path) {
        fs::write(root.join("plain-file"), "not a directory").unwrap();
        fs::create_dir(root.join("realdir")).unwrap();
        symlink(root.join("missing-target"), root.join("dangling")).unwrap();
        symlink(root.join("plain-file"), root.join("link-to-file")).unwrap();
        symlink(root.join("realdir"), root.join("link-to-dir")).unwrap();
        symlink(root.join("loop-b"), root.join("loop-a")).unwrap();
        symlink(root.join("loop-a"), root.join("loop-b")).unwrap();
    }

    // (database path below the tree root, whether the live run succeeds)
    let cases: [(&str, bool); 12] = [
        ("dangling/trop.db", false),
        ("dangling/data/trop.db", false),
        ("link-to-file/trop.db", false),
        ("link-to-file/data/trop.db", false),
        ("link-to-dir/trop.db", true),
        ("link-to-dir/new/trop.db", true),
        ("loop-a/trop.db", false),
        ("loop-a/data/trop.db", false),
        ("plain-file/../data/trop.db", false),
        ("realdir/../data/trop.db", true),
        ("missing/../data/trop.db", true),
        ("realdir/./new/trop.db", true),
    ];
    for (relative, live_succeeds) in cases {
        let dry_root = TempDir::new().unwrap();
        let live_root = TempDir::new().unwrap();
        tree(dry_root.path());
        tree(live_root.path());

        let dry = check_database_directory_creatable(&dry_root.path().join(relative));
        let live = Database::open(DatabaseConfig::new(live_root.path().join(relative)));
        assert_eq!(
            live.is_ok(),
            live_succeeds,
            "{relative}: unexpected live result {:?}",
            live.as_ref().err()
        );
        assert_eq!(
            dry.is_ok(),
            live.is_ok(),
            "{relative}: dry {:?}, live {:?}",
            dry.as_ref().err(),
            live.as_ref().err()
        );
        if let Err(error) = &dry {
            assert!(matches!(error, Error::Io(_)), "{relative}: {error}");
        }
        assert!(
            !dry_root.path().join("data").exists()
                && !dry_root.path().join("missing").exists()
                && !dry_root.path().join("realdir").join("new").exists(),
            "{relative}: the dry check created something"
        );
    }
}
