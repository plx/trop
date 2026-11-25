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
