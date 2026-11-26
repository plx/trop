//! Regression tests for the verified cleanup kernels (`verified::cleanup`,
//! `verified::sort`) through the public library API.
//!
//! - Automatic expiration that is enabled must expire with the 30-day
//!   default whenever no threshold is configured, whether the `cleanup`
//!   section is missing or present without `expire_after_days` (before the
//!   fix, `cleanup: Some(CleanupConfig { expire_after_days: None })` reported
//!   automatic expiration as enabled but expired nothing), for single
//!   reservations and for groups (`ReserveGroupPlan::new` loads one
//!   `trop.yaml` without built-in defaults, so `cleanup: {}` reaches the
//!   group's automatic cleanup as exactly such a section).
//! - Revalidation: every planned removal ends in exactly one bucket, and the
//!   reported counts add up, for rows that vanish, change, or survive a
//!   guarded delete.
//! - The removal order: prune candidates in scan order, then expiration
//!   candidates oldest first with ties in scan order.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;
use trop::config::{CleanupConfig, Config, OccupancyConfig, PortConfig};
use trop::operations::{
    CleanupOperations, ReserveGroupOptions, ReserveGroupPlan, ReserveOptions, ReservePlan,
};
use trop::{
    Database, DatabaseConfig, Error, ExecutionResult, PlanExecutor, Port, Reservation,
    ReservationKey,
};

const DAY: u64 = 24 * 60 * 60;

fn port(value: u16) -> Port {
    Port::try_from(value).unwrap()
}

/// A database in a temporary directory, with existing directories to
/// reserve (so pruning keeps their reservations).
struct Fixture {
    dir: TempDir,
    db: Database,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let db = Database::open(DatabaseConfig::new(dir.path().join("trop.db"))).unwrap();
        Self { dir, db }
    }

    fn existing_dir(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Stores a reservation of `port` for `path` (tagged `tag`), last used
    /// `age` ago.
    fn store(&mut self, path: &Path, tag: Option<&str>, port_value: u16, age: Duration) {
        self.store_at(path, tag, port_value, SystemTime::now() - age);
    }

    /// Stores a reservation of `port` for `path` (tagged `tag`), last used at
    /// `used`.
    fn store_at(&mut self, path: &Path, tag: Option<&str>, port_value: u16, used: SystemTime) {
        let key = ReservationKey::new(path.to_path_buf(), tag.map(str::to_string)).unwrap();
        let reservation = Reservation::builder(key, port(port_value))
            .created_at(used)
            .last_used_at(used)
            .build()
            .unwrap();
        self.db.create_reservation(&reservation).unwrap();
    }

    fn count(&self) -> usize {
        Database::list_all_reservations(self.db.connection())
            .unwrap()
            .len()
    }
}

/// A one-port range (`5600`) with occupancy checks off.
fn one_port_config(cleanup: Option<CleanupConfig>) -> Config {
    Config {
        ports: Some(PortConfig {
            min: 5600,
            max: Some(5600),
            max_offset: None,
        }),
        occupancy_check: Some(OccupancyConfig {
            skip: Some(true),
            ..Default::default()
        }),
        cleanup,
        ..Default::default()
    }
}

/// Reserves a fresh key in the exhausted one-port range: the only port is
/// held by a 40-day-old reservation at an existing directory, so only
/// automatic expiration can free it.
fn reserve_after_stale_blocker(cleanup: Option<CleanupConfig>) -> trop::Result<Option<Port>> {
    let mut fixture = Fixture::new();
    let stale = fixture.existing_dir("stale");
    fixture.store(&stale, None, 5600, Duration::from_secs(40 * DAY));
    let target = fixture.existing_dir("target");
    let config = one_port_config(cleanup);
    let options = ReserveOptions::new(ReservationKey::new(target, None).unwrap(), None)
        .with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &config).build_plan(fixture.db.connection())?;
    let result = PlanExecutor::new(fixture.db.connection()).execute(&plan)?;
    assert_eq!(
        fixture.count(),
        1,
        "the stale blocker must have been expired"
    );
    Ok(result.port)
}

#[test]
fn automatic_expiration_without_a_threshold_uses_the_thirty_day_default() {
    // A `cleanup` section without `expire_after_days` (library callers can
    // build one; an empty YAML section merges over the built-in default).
    let port_of_reserve = reserve_after_stale_blocker(Some(CleanupConfig {
        expire_after_days: None,
    }))
    .expect("enabled automatic expiration must expire a 40-day-old reservation");
    assert_eq!(port_of_reserve, Some(port(5600)));
}

#[test]
fn automatic_expiration_agrees_for_missing_and_empty_cleanup_sections() {
    for cleanup in [
        None,
        Some(CleanupConfig {
            expire_after_days: None,
        }),
        Some(CleanupConfig {
            expire_after_days: Some(30),
        }),
    ] {
        assert_eq!(
            reserve_after_stale_blocker(cleanup.clone()).unwrap(),
            Some(port(5600)),
            "cleanup section {cleanup:?}"
        );
    }
}

/// Reserves a two-service group (`web` at offset 0, `api` at offset 1) in
/// the two-port range `5700`-`5701`, whose first port is held by a
/// 40-day-old reservation at an existing directory, so only automatic
/// expiration can make room. `cleanup_section` is the configuration file's
/// `cleanup` section (empty for none). Returns the result and whether the
/// stale blocker is still stored.
fn reserve_group_after_stale_blocker(
    cleanup_section: &str,
) -> (trop::Result<ExecutionResult>, bool) {
    let mut fixture = Fixture::new();
    let project = fixture.existing_dir("project");
    let config_path = project.join("trop.yaml");
    fs::write(
        &config_path,
        format!(
            "project: cleanup-regression
ports:
  min: 5700
  max: 5701
occupancy_check:
  skip: true
{cleanup_section}reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
"
        ),
    )
    .unwrap();
    let stale = fixture.existing_dir("stale");
    fixture.store(&stale, None, 5700, Duration::from_secs(40 * DAY));
    let stale_key = ReservationKey::new(stale, None).unwrap();

    let result = ReserveGroupPlan::new(
        ReserveGroupOptions::new(config_path).with_allow_unrelated_path(true),
    )
    .and_then(|planner| planner.build_plan(fixture.db.connection()))
    .and_then(|plan| PlanExecutor::new(fixture.db.connection()).execute(&plan));
    let stale_stored = Database::get_reservation(fixture.db.connection(), &stale_key)
        .unwrap()
        .is_some();
    (result, stale_stored)
}

#[test]
fn group_automatic_expiration_without_a_threshold_uses_the_thirty_day_default() {
    // `cleanup: {}` loads as `Some(CleanupConfig { expire_after_days: None })`.
    let (result, stale_stored) = reserve_group_after_stale_blocker("cleanup: {}\n");
    let result = result.expect("enabled automatic expiration must make room for the group");
    let ports = result.allocated_ports.expect("group ports");
    assert_eq!(ports.get("web"), Some(&port(5700)));
    assert_eq!(ports.get("api"), Some(&port(5701)));
    assert!(!stale_stored, "the stale blocker must have been expired");
}

#[test]
fn group_automatic_expiration_agrees_for_missing_and_empty_cleanup_sections() {
    for section in ["", "cleanup: {}\n", "cleanup:\n  expire_after_days: 30\n"] {
        let (result, stale_stored) = reserve_group_after_stale_blocker(section);
        assert!(result.is_ok(), "cleanup section {section:?}: {result:?}");
        assert!(!stale_stored, "cleanup section {section:?}");
    }
    // A 60-day threshold keeps the 40-day-old blocker: the group fails.
    let (result, stale_stored) =
        reserve_group_after_stale_blocker("cleanup:\n  expire_after_days: 60\n");
    assert!(
        matches!(result, Err(Error::GroupAllocationFailed { .. })),
        "{result:?}"
    );
    assert!(stale_stored);
}

#[test]
fn automatic_expiration_keeps_a_configured_threshold() {
    // A 60-day threshold keeps the 40-day-old blocker, so the range stays
    // exhausted.
    let error = reserve_after_stale_blocker(Some(CleanupConfig {
        expire_after_days: Some(60),
    }))
    .unwrap_err();
    assert!(
        matches!(error, Error::PortExhausted { .. }),
        "unexpected error: {error}"
    );
}

#[test]
fn removal_order_is_prune_then_oldest_expired_with_scan_order_ties() {
    let mut fixture = Fixture::new();
    let missing = fixture.dir.path().join("missing");
    let existing = fixture.existing_dir("existing");
    // Two prune candidates (a missing path; scan order is path, then tag),
    // the newer one first in scan order; the older one is also expired, and
    // prune takes precedence.
    fixture.store(&missing, Some("a"), 5000, Duration::from_secs(DAY));
    fixture.store(&missing, Some("b"), 5001, Duration::from_secs(50 * DAY));
    // Expiration candidates: `c` and `d` last used at the same second (a
    // tie keeps scan order, not port or insertion order), `e` older.
    let tie = SystemTime::now() - Duration::from_secs(20 * DAY);
    fixture.store_at(&existing, Some("d"), 5002, tie);
    fixture.store_at(&existing, Some("c"), 5003, tie);
    fixture.store(&existing, Some("e"), 5004, Duration::from_secs(25 * DAY));
    // Fresh: kept.
    fixture.store(&existing, Some("f"), 5005, Duration::from_secs(DAY));

    let config = CleanupConfig {
        expire_after_days: Some(7),
    };
    let tags = |reservations: &[Reservation]| -> Vec<String> {
        reservations
            .iter()
            .map(|reservation| reservation.key().tag.clone().unwrap())
            .collect()
    };
    let preview = CleanupOperations::autoclean(&mut fixture.db, &config, true).unwrap();
    assert_eq!(
        tags(&preview.removed_reservations),
        ["a", "b", "e", "c", "d"]
    );
    assert_eq!(tags(&preview.pruned_reservations), ["a", "b"]);
    assert_eq!(tags(&preview.expired_reservations), ["e", "c", "d"]);
    assert_eq!(tags(&preview.preserved_reservations), ["f"]);
    assert_eq!(preview.considered_count, 6);
    assert_eq!(
        preview.pruned_count + preview.expired_count,
        preview.total_removed
    );
    assert_eq!(
        preview.preserved_count + preview.total_removed,
        preview.considered_count
    );

    // The live run removes and reports exactly the preview.
    let live = CleanupOperations::autoclean(&mut fixture.db, &config, false).unwrap();
    assert_eq!(live.removed_reservations, preview.removed_reservations);
    assert_eq!(live.pruned_reservations, preview.pruned_reservations);
    assert_eq!(live.expired_reservations, preview.expired_reservations);
    assert_eq!(fixture.count(), 1);
}

/// Prunes three missing paths `/fate/a`, `/fate/b`, `/fate/c` after
/// installing `trigger_sql`, and returns the result.
fn prune_with_trigger(trigger_sql: &str) -> (Fixture, trop::operations::PruneResult) {
    let mut fixture = Fixture::new();
    for (index, name) in ["a", "b", "c"].into_iter().enumerate() {
        let path = PathBuf::from(format!("/trop-cleanup-fate/{name}"));
        fixture.store(
            &path,
            None,
            5100 + u16::try_from(index).unwrap(),
            Duration::from_secs(DAY),
        );
    }
    fixture.db.connection().execute_batch(trigger_sql).unwrap();
    let result = CleanupOperations::prune(&mut fixture.db, false).unwrap();
    assert_eq!(
        result.preserved_count + result.removed_count,
        result.considered_count
    );
    assert_eq!(result.considered_count, 3);
    (fixture, result)
}

fn paths(reservations: &[Reservation]) -> Vec<String> {
    reservations
        .iter()
        .map(|reservation| reservation.key().path.display().to_string())
        .collect()
}

#[test]
fn a_row_removed_by_an_earlier_delete_counts_as_removed() {
    // Deleting `a` also deletes `b`: `b` is observed gone and still reported
    // as removed by this invocation.
    let (fixture, result) = prune_with_trigger(
        "CREATE TEMP TRIGGER cascade_a_to_b AFTER DELETE ON reservations
         WHEN OLD.path = '/trop-cleanup-fate/a'
         BEGIN DELETE FROM reservations WHERE path = '/trop-cleanup-fate/b'; END;",
    );
    assert_eq!(result.removed_count, 3);
    assert_eq!(
        paths(&result.removed_reservations),
        [
            "/trop-cleanup-fate/a",
            "/trop-cleanup-fate/b",
            "/trop-cleanup-fate/c"
        ]
    );
    assert_eq!(fixture.count(), 0);
}

#[test]
fn a_row_the_guarded_delete_leaves_in_place_is_preserved() {
    // The guarded delete of `b` is silently skipped, so it matched nothing
    // and the row is still present: preserved, not reported as removed.
    let (fixture, result) = prune_with_trigger(
        "CREATE TEMP TRIGGER keep_b BEFORE DELETE ON reservations
         WHEN OLD.path = '/trop-cleanup-fate/b'
         BEGIN SELECT RAISE(IGNORE); END;",
    );
    assert_eq!(result.removed_count, 2);
    assert_eq!(
        paths(&result.removed_reservations),
        ["/trop-cleanup-fate/a", "/trop-cleanup-fate/c"]
    );
    assert_eq!(
        paths(&result.preserved_reservations),
        ["/trop-cleanup-fate/b"]
    );
    assert_eq!(fixture.count(), 1);
}

#[test]
fn a_row_gone_after_a_skipped_delete_counts_as_removed() {
    // The guarded delete of `b` is skipped, but the trigger removed the row
    // itself: the delete matched nothing and the re-read finds it gone.
    let (fixture, result) = prune_with_trigger(
        "CREATE TEMP TRIGGER remove_b_then_skip BEFORE DELETE ON reservations
         WHEN OLD.path = '/trop-cleanup-fate/b'
         BEGIN
             DELETE FROM reservations WHERE path = '/trop-cleanup-fate/b';
             SELECT RAISE(IGNORE);
         END;",
    );
    assert_eq!(result.removed_count, 3);
    assert_eq!(result.preserved_reservations, Vec::<Reservation>::new());
    assert_eq!(fixture.count(), 0);
}

#[test]
fn a_row_changed_before_its_delete_is_preserved_with_its_current_value() {
    // Deleting `a` refreshes `b`'s last use: `b` no longer matches the
    // planned snapshot, so it is preserved, with its current row.
    let (fixture, result) = prune_with_trigger(
        "CREATE TEMP TRIGGER touch_b AFTER DELETE ON reservations
         WHEN OLD.path = '/trop-cleanup-fate/a'
         BEGIN
             UPDATE reservations SET last_used_at = last_used_at + 1
             WHERE path = '/trop-cleanup-fate/b';
         END;",
    );
    assert_eq!(
        paths(&result.removed_reservations),
        ["/trop-cleanup-fate/a", "/trop-cleanup-fate/c"]
    );
    assert_eq!(
        paths(&result.preserved_reservations),
        ["/trop-cleanup-fate/b"]
    );
    let stored = Database::list_all_reservations(fixture.db.connection()).unwrap();
    assert_eq!(result.preserved_reservations, stored);
}
