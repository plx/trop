//! `trop reserve --dry-run` predicts the live outcome.
//!
//! A dry run plans exactly as a live run plans (on a read-only connection,
//! inside a transaction that is rolled back) and reports what the live run
//! would: the same failures with the same exit codes and messages, and the
//! same port on stdout. It never changes the database, and it never creates
//! one.

mod common;

use assert_cmd::Command;
use common::{parse_port, TestEnv};
use std::path::Path;
use std::process::Output;

/// One stored row, timestamps included (so that a refresh would show).
#[derive(Debug, PartialEq, Eq)]
struct Row {
    path: String,
    tag: String,
    port: i64,
    project: Option<String>,
    task: Option<String>,
    created_at: i64,
    last_used_at: i64,
}

/// Every stored row, in a stable order.
fn snapshot(env: &TestEnv) -> Vec<Row> {
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT path, tag, port, project, task, created_at, last_used_at
             FROM reservations ORDER BY path, tag",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok(Row {
                path: row.get(0)?,
                tag: row.get(1)?,
                port: row.get(2)?,
                project: row.get(3)?,
                task: row.get(4)?,
                created_at: row.get(5)?,
                last_used_at: row.get(6)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// Makes every stored timestamp old, so that a refresh would be visible.
fn age_timestamps(env: &TestEnv) {
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    connection
        .execute(
            "UPDATE reservations SET created_at = 1000, last_used_at = 2000",
            [],
        )
        .unwrap();
}

/// `trop reserve --path <path>` plus `args`, run from the test's own
/// temporary directory.
fn reserve(env: &TestEnv, path: &Path, args: &[&str]) -> Output {
    let mut command: Command = env.command();
    command
        .current_dir(&env.temp_path)
        .arg("reserve")
        .arg("--path")
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// Asserts that a dry run failed exactly as the live run did.
fn assert_same_failure(dry: &Output, live: &Output) {
    assert!(!live.status.success(), "live run unexpectedly succeeded");
    assert_eq!(
        dry.status.code(),
        live.status.code(),
        "dry run: {}\nlive run: {}",
        stderr(dry),
        stderr(live)
    );
    assert_eq!(stderr(dry), stderr(live));
    assert!(stdout(dry).is_empty(), "a failed dry run printed a port");
}

/// Regression: a sticky project change used to "succeed" as a dry run
/// (exit 0) although the live run exits 1.
#[test]
fn dry_run_rejects_a_sticky_project_change_like_a_live_run() {
    let env = TestEnv::new();
    let path = env.create_dir("sticky");
    let first = reserve(&env, &path, &["--project", "p1"]);
    assert!(first.status.success(), "{}", stderr(&first));
    age_timestamps(&env);
    let before = snapshot(&env);

    let dry = reserve(&env, &path, &["--project", "p2", "--dry-run"]);
    assert_eq!(snapshot(&env), before, "a dry run changed the database");
    let live = reserve(&env, &path, &["--project", "p2"]);
    assert_same_failure(&dry, &live);
    assert_eq!(snapshot(&env), before);
}

/// Regression: an unrelated path used to "succeed" as a dry run (exit 0)
/// although the live run exits 1.
#[test]
fn dry_run_rejects_an_unrelated_path_like_a_live_run() {
    let env = TestEnv::new();
    let related = env.create_dir("related");
    let first = reserve(&env, &related, &[]);
    assert!(first.status.success(), "{}", stderr(&first));
    let before = snapshot(&env);

    let sideways = Path::new("/nonexistent/sideways");
    let dry = reserve(&env, sideways, &["--dry-run"]);
    assert_eq!(snapshot(&env), before, "a dry run changed the database");
    let live = reserve(&env, sideways, &[]);
    assert_same_failure(&dry, &live);
    assert_eq!(snapshot(&env), before);
}

/// Regression: a dry run used to print no port. It now prints the port the
/// live run reports: the stored port of an existing reservation, or the
/// port a new reservation would get.
#[test]
fn dry_run_prints_the_port_a_live_run_reports() {
    let env = TestEnv::new();
    let existing = env.create_dir("existing");
    let first = reserve(&env, &existing, &[]);
    assert!(first.status.success(), "{}", stderr(&first));
    let existing_port = parse_port(&stdout(&first));
    age_timestamps(&env);
    let before = snapshot(&env);

    // An existing reservation: the dry run reports its port, refreshes
    // nothing, and the live run agrees.
    let dry = reserve(&env, &existing, &["--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert_eq!(parse_port(&stdout(&dry)), existing_port);
    assert!(stderr(&dry).contains("Dry run - would perform the following actions:"));
    assert!(stderr(&dry).contains("Update last_used_at timestamp"));
    assert_eq!(
        snapshot(&env),
        before,
        "a dry run refreshed the reservation"
    );

    // A new reservation: the dry run reports the port the live run then
    // takes, and creates nothing.
    let fresh = env.create_dir("fresh");
    let dry = reserve(&env, &fresh, &["--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    let predicted = parse_port(&stdout(&dry));
    assert!(stderr(&dry).contains(&format!("on port {predicted}")));
    assert_eq!(snapshot(&env), before, "a dry run created a reservation");
    let live = reserve(&env, &fresh, &[]);
    assert!(live.status.success(), "{}", stderr(&live));
    assert_eq!(parse_port(&stdout(&live)), predicted);

    let live = reserve(&env, &existing, &[]);
    assert_eq!(parse_port(&stdout(&live)), existing_port);
}

/// `--quiet` silences the plan, never the port.
#[test]
fn quiet_dry_run_prints_only_the_port() {
    let env = TestEnv::new();
    let path = env.create_dir("quiet");
    let first = reserve(&env, &path, &[]);
    assert!(first.status.success(), "{}", stderr(&first));
    let port = parse_port(&stdout(&first));

    let mut command = env.command();
    let output = command
        .current_dir(&env.temp_path)
        .arg("--quiet")
        .arg("reserve")
        .arg("--path")
        .arg(&path)
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    assert_eq!(parse_port(&stdout(&output)), port);
}

/// A dry run never creates the data directory or the database; it says that
/// no database exists.
#[test]
fn dry_run_without_a_database_creates_nothing() {
    let env = TestEnv::new();
    let path = env.create_dir("nothing");
    let output = reserve(&env, &path, &["--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("No database exists yet"));
    assert!(
        !env.data_dir.exists(),
        "a dry run created the data directory"
    );
}

/// With auto-init disabled a live run refuses to create the database, so
/// the dry run fails the same way instead of claiming that a live run would
/// create it.
#[test]
fn dry_run_without_a_database_and_auto_init_fails_like_a_live_run() {
    let env = TestEnv::new();
    let path = env.create_dir("noinit");
    let dry = reserve(&env, &path, &["--disable-autoinit", "--dry-run"]);
    let live = reserve(&env, &path, &["--disable-autoinit"]);
    assert_same_failure(&dry, &live);
    assert!(!stderr(&dry).contains("a live run would create it"));
    assert!(
        !env.data_dir.join("trop.db").exists(),
        "a run with auto-init disabled created the database"
    );
}

/// An exhausted range with automatic cleanup enabled defers allocation to
/// execution. The dry run cannot clean up, so it says that it cannot predict
/// the port or the outcome, prints no port, and changes nothing.
#[test]
fn dry_run_of_a_deferred_allocation_says_it_cannot_predict_the_outcome() {
    let env = TestEnv::new();
    let range = ["--min", "5000", "--max", "5000", "--skip-occupancy-check"];
    let holder = env.create_dir("holder");
    let first = reserve(&env, &holder, &range);
    assert!(first.status.success(), "{}", stderr(&first));
    let before = snapshot(&env);

    let other = env.create_dir("other");
    let mut args = range.to_vec();
    args.push("--dry-run");
    let dry = reserve(&env, &other, &args);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(stdout(&dry).is_empty(), "{}", stdout(&dry));
    assert!(stderr(&dry).contains("automatic cleanup after exhaustion"));
    assert!(
        stderr(&dry).contains("cannot predict the port"),
        "{}",
        stderr(&dry)
    );
    assert_eq!(snapshot(&env), before, "a dry run changed the database");
}

/// A database file that has never been initialized is not initialized by a
/// dry run (a read-only connection cannot): the dry run says so and
/// succeeds, and the live run then initializes it.
#[test]
fn dry_run_on_an_uninitialized_database_leaves_it_alone() {
    let env = TestEnv::new();
    std::fs::create_dir_all(&env.data_dir).unwrap();
    let database = env.data_dir.join("trop.db");
    std::fs::write(&database, b"").unwrap();
    let path = env.create_dir("uninitialized");

    let dry = reserve(&env, &path, &["--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(stdout(&dry).is_empty());
    assert!(
        stderr(&dry).contains("is not initialized yet; a live run would initialize it first"),
        "{}",
        stderr(&dry)
    );
    assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);

    let live = reserve(&env, &path, &[]);
    assert!(live.status.success(), "{}", stderr(&live));
    parse_port(&stdout(&live));
}

/// A database at an older schema version is not migrated by a dry run: the
/// dry run says that a live run would attempt the migration first, and
/// leaves the file untouched.
#[test]
fn dry_run_on_a_database_needing_migration_leaves_it_alone() {
    let env = TestEnv::new();
    std::fs::create_dir_all(&env.data_dir).unwrap();
    let database = env.data_dir.join("trop.db");
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO metadata (key, value) VALUES ('schema_version', '1');",
            )
            .unwrap();
    }
    let before = std::fs::read(&database).unwrap();
    let path = env.create_dir("old-schema");

    let dry = reserve(&env, &path, &["--dry-run"]);
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert!(stdout(&dry).is_empty());
    assert!(
        stderr(&dry).contains("has schema version 1 and needs a migration to version"),
        "{}",
        stderr(&dry)
    );
    assert_eq!(std::fs::read(&database).unwrap(), before);
}
