//! Regression tests for `execute_migrate` through the public library API.
//!
//! - The plan is a snapshot: a destination reserved, or a source released,
//!   after the plan was built must not be silently overwritten or
//!   resurrected when the plan executes.
//! - A migration whose destination lies inside its source tree keeps every
//!   reservation.
//!
//! Reservation paths here lie below the test's working directory (they need
//! not exist), so the unrelated-path guard permits them; the database lives
//! in a temporary directory.

use std::path::{Path, PathBuf};

use trop::operations::{execute_migrate, MigrateOptions, MigratePlan};
use trop::{Database, DatabaseConfig, Port, Reservation, ReservationKey};

fn base(name: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("trop-migrate-regression")
        .join(name)
}

fn reserve(db: &mut Database, path: &Path, port: u16) {
    let key = ReservationKey::new(path.to_path_buf(), None).unwrap();
    let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&reservation).unwrap();
}

fn port_at(db: &Database, path: &Path) -> Option<u16> {
    let key = ReservationKey::new(path.to_path_buf(), None).unwrap();
    Database::get_reservation(db.connection(), &key)
        .unwrap()
        .map(|reservation| reservation.port().value())
}

fn two_connections() -> (tempfile::TempDir, Database, Database) {
    let directory = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::new(directory.path().join("trop.db"));
    let first = Database::open(config.clone()).unwrap();
    let second = Database::open(config).unwrap();
    (directory, first, second)
}

#[test]
fn destination_reserved_after_planning_is_not_overwritten() {
    let (_directory, mut db, mut other) = two_connections();
    let root = base("late-destination");
    let source = root.join("source");
    let destination = root.join("destination");
    reserve(&mut db, &source, 47_101);

    let mut plan = MigratePlan::new(MigrateOptions::new(source.clone(), destination.clone()));
    plan.build(&db).unwrap();
    assert_eq!(plan.conflict_count(), 0);

    // Another process reserves the destination between planning and
    // execution.
    reserve(&mut other, &destination, 47_102);

    let result = execute_migrate(&plan, &mut db);
    assert!(
        result.is_err(),
        "a migration without --force overwrote a reservation made after planning"
    );
    assert_eq!(port_at(&db, &destination), Some(47_102));
    assert_eq!(port_at(&db, &source), Some(47_101));
}

#[test]
fn source_released_after_planning_is_not_resurrected() {
    let (_directory, mut db, mut other) = two_connections();
    let root = base("late-release");
    let source = root.join("source");
    let destination = root.join("destination");
    reserve(&mut db, &source, 47_111);

    let mut plan = MigratePlan::new(MigrateOptions::new(source.clone(), destination.clone()));
    plan.build(&db).unwrap();

    let key = ReservationKey::new(source.clone(), None).unwrap();
    assert!(other.delete_reservation(&key).unwrap());

    let result = execute_migrate(&plan, &mut db);
    assert!(
        result.is_err(),
        "a migration resurrected a reservation released after planning"
    );
    assert_eq!(port_at(&db, &destination), None);
    assert_eq!(port_at(&db, &source), None);
}

#[test]
fn forced_migration_into_own_subtree_keeps_every_reservation() {
    let (_directory, mut db, _other) = two_connections();
    let a = base("subtree").join("a");
    let ab = a.join("b");
    reserve(&mut db, &a, 47_121);
    reserve(&mut db, &ab, 47_122);

    let options = MigrateOptions::new(a.clone(), ab.clone())
        .with_recursive(true)
        .with_force(true);
    let mut plan = MigratePlan::new(options);
    plan.build(&db).unwrap();
    let result = execute_migrate(&plan, &mut db).unwrap();

    assert_eq!(result.migrated_count, 2);
    assert_eq!(port_at(&db, &a), None);
    assert_eq!(port_at(&db, &ab), Some(47_121));
    assert_eq!(port_at(&db, &ab.join("b")), Some(47_122));
}
