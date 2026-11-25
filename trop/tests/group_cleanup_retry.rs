//! Group allocation runs the automatic cleanup step on exhaustion.
//!
//! The specification's allocation algorithm prunes and/or expires
//! reservations when no port is available and retries once, and adds: "When
//! allocating a port group, we do the same thing". These tests pin that a
//! fresh group whose fallback pattern is exhausted runs exactly one cleanup
//! (respecting `disable_autoprune`/`disable_autoexpire`) and one re-scan, that
//! the cleanup is rolled back when the retry still fails, and that the cleanup
//! never runs when both switches disable it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use trop::operations::{ReserveGroupOptions, ReserveGroupPlan};
use trop::{
    Database, DatabaseConfig, Error, ExecutionResult, PlanExecutor, Port, Reservation,
    ReservationKey,
};

/// Older than the default 30-day expiry.
const STALE: Duration = Duration::from_secs(40 * 24 * 60 * 60);
const FRESH: Duration = Duration::from_secs(0);

/// A two-service group (`web` at offset 0, `api` at offset 1) in a range of
/// exactly two ports, so one foreign reservation exhausts it.
struct Fixture {
    dir: TempDir,
    config_path: PathBuf,
    db: Database,
}

fn fixture(switches: &str) -> Fixture {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let config_path = project.join("trop.yaml");
    fs::write(
        &config_path,
        format!(
            "project: cleanup-test
ports:
  min: 5000
  max: 5001
occupancy_check:
  skip: true
{switches}reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
"
        ),
    )
    .unwrap();
    let db = Database::open(DatabaseConfig::new(dir.path().join("trop.db"))).unwrap();
    Fixture {
        dir,
        config_path,
        db,
    }
}

impl Fixture {
    /// Stores an untagged reservation of `port` for `path`, last used `age`
    /// ago.
    fn reserve(&mut self, path: &Path, port: u16, age: Duration) -> ReservationKey {
        let key = ReservationKey::new(path.to_path_buf(), None).unwrap();
        let used = SystemTime::now() - age;
        let reservation = Reservation::builder(key.clone(), Port::try_from(port).unwrap())
            .created_at(used)
            .last_used_at(used)
            .build()
            .unwrap();
        self.db.create_reservation(&reservation).unwrap();
        key
    }

    /// A directory that exists (so pruning keeps its reservations).
    fn existing_dir(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Runs `reserve-group` on the fixture's configuration.
    fn reserve_group(&self) -> trop::Result<ExecutionResult> {
        let planner = ReserveGroupPlan::new(
            ReserveGroupOptions::new(self.config_path.clone()).with_allow_unrelated_path(true),
        )?;
        let plan = planner.build_plan(self.db.connection())?;
        PlanExecutor::new(self.db.connection()).execute(&plan)
    }

    fn stored(&self, key: &ReservationKey) -> bool {
        Database::get_reservation(self.db.connection(), key)
            .unwrap()
            .is_some()
    }

    fn row_count(&self) -> usize {
        Database::list_all_reservations(self.db.connection())
            .unwrap()
            .len()
    }
}

fn port(value: u16) -> Port {
    Port::try_from(value).unwrap()
}

#[test]
fn group_exhaustion_expires_stale_reservations_and_retries_once() {
    let mut fixture = fixture("");
    let other = fixture.existing_dir("other");
    let stale = fixture.reserve(&other, 5000, STALE);

    let result = fixture
        .reserve_group()
        .expect("automatic expiry must make room for the group");

    let ports = result.allocated_ports.expect("group ports");
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert_eq!(ports.get("api"), Some(&port(5001)));
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("Automatic cleanup")
                && warning.contains("expired 1 reservation")),
        "the retry after cleanup must be reported: {:?}",
        result.warnings
    );
    assert!(!fixture.stored(&stale), "the stale reservation was expired");
    assert_eq!(fixture.row_count(), 2);
}

#[test]
fn group_exhaustion_prunes_reservations_of_missing_directories() {
    // Expiry is disabled; pruning alone frees the port.
    let mut fixture = fixture("disable_autoexpire: true\n");
    let missing = fixture.dir.path().join("deleted-worktree");
    let orphan = fixture.reserve(&missing, 5000, FRESH);

    let result = fixture
        .reserve_group()
        .expect("automatic pruning must make room for the group");

    let ports = result.allocated_ports.expect("group ports");
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert_eq!(ports.get("api"), Some(&port(5001)));
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("pruned 1")),
        "{:?}",
        result.warnings
    );
    assert!(!fixture.stored(&orphan));
}

#[test]
fn disabled_expiry_keeps_stale_reservations_and_the_group_fails() {
    // Pruning stays enabled but cannot free a port of an existing directory;
    // expiry, which could, is disabled.
    let mut fixture = fixture("disable_autoexpire: true\n");
    let other = fixture.existing_dir("other");
    let stale = fixture.reserve(&other, 5000, STALE);

    let error = fixture
        .reserve_group()
        .expect_err("a disabled expiry must not free the stale port");

    assert!(
        matches!(
            &error,
            Error::GroupAllocationFailed { attempted: 2, reason }
                if reason.contains("even after automatic cleanup pruned 0 and expired 0")
        ),
        "{error}"
    );
    assert!(fixture.stored(&stale));
}

#[test]
fn both_cleanups_disabled_never_cleans_up() {
    let mut fixture = fixture("disable_autoprune: true\ndisable_autoexpire: true\n");
    let missing = fixture.dir.path().join("deleted-worktree");
    let orphan = fixture.reserve(&missing, 5000, STALE);

    let error = fixture
        .reserve_group()
        .expect_err("no cleanup may run when both are disabled");

    assert!(
        matches!(
            &error,
            Error::GroupAllocationFailed { attempted: 2, reason }
                if reason == "No scanned base can satisfy the complete offset fallback pattern"
        ),
        "{error}"
    );
    assert!(
        fixture.stored(&orphan),
        "neither prune nor expire may remove the row"
    );
}

#[test]
fn group_exhaustion_after_cleanup_fails_and_rolls_the_cleanup_back() {
    let mut fixture = fixture("");
    let stale_dir = fixture.existing_dir("stale");
    let busy_dir = fixture.existing_dir("busy");
    let stale = fixture.reserve(&stale_dir, 5000, STALE);
    let busy = fixture.reserve(&busy_dir, 5001, FRESH);

    let error = fixture
        .reserve_group()
        .expect_err("port 5001 stays reserved after the cleanup");

    assert!(
        matches!(
            &error,
            Error::GroupAllocationFailed { attempted: 2, reason }
                if reason.contains("even after automatic cleanup pruned 0 and expired 1")
        ),
        "{error}"
    );
    // The whole group step, cleanup included, is rolled back.
    assert!(fixture.stored(&stale));
    assert!(fixture.stored(&busy));
    assert_eq!(fixture.row_count(), 2);
}

#[test]
fn group_without_exhaustion_never_cleans_up() {
    let mut fixture = fixture("");
    let other = fixture.existing_dir("other");
    // Stale, but outside the group's range: nothing is exhausted.
    let stale = fixture.reserve(&other, 6000, STALE);

    let result = fixture.reserve_group().expect("the range has room");

    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(fixture.stored(&stale), "cleanup runs only after exhaustion");
}

#[test]
fn group_refresh_never_cleans_up() {
    let mut fixture = fixture("");
    fixture.reserve_group().expect("first reservation");
    let other = fixture.existing_dir("other");
    // A stale row elsewhere: refreshing the stored group allocates nothing.
    let stale = fixture.reserve(&other, 6000, STALE);

    let result = fixture.reserve_group().expect("refresh");

    let ports = result.allocated_ports.expect("group ports");
    assert_eq!(ports.get("web"), Some(&port(5000)));
    assert!(fixture.stored(&stale));
}
