//! Integration tests for reservation operations.

mod common;
use common::create_test_config;

use common::database::create_test_database;
use std::path::PathBuf;
use trop::Database;
use trop::{Port, ReleaseOptions, ReleasePlan, ReservationKey, ReserveOptions, ReservePlan};

#[test]
fn test_reserve_and_release_cycle() {
    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();
    let port = Port::try_from(8080).unwrap();

    // Reserve a port
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("test-project".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(reserve_opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    assert_eq!(plan.actions.len(), 1);

    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();
    assert!(result.success);
    assert!(result.port.is_some(), "Should allocate a port");

    // Save the actually allocated port (might not be the preferred one if occupied)
    let allocated_port = result.port.unwrap();

    // Verify reservation exists
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(reservation.is_some());
    assert_eq!(reservation.as_ref().unwrap().port(), allocated_port);
    assert_eq!(
        reservation.as_ref().unwrap().project(),
        Some("test-project")
    );

    // Release the port
    let release_opts = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);

    let plan = ReleasePlan::new(release_opts)
        .build_plan(db.connection())
        .unwrap();
    assert_eq!(plan.actions.len(), 1);

    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();
    assert!(result.success);

    // Verify reservation is gone
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(reservation.is_none());
}

#[test]
fn test_idempotent_reserve() {
    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();
    let port = Port::try_from(8080).unwrap();

    let reserve_opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("test-project".to_string()))
        .with_allow_unrelated_path(true);

    // First reservation
    let plan = ReservePlan::new(reserve_opts.clone(), &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();
    assert!(result.success);
    assert!(
        result.port.is_some(),
        "First reservation should allocate a port"
    );

    // Save the actually allocated port (might not be the preferred one if occupied)
    let allocated_port = result.port.unwrap();

    // Second reservation with same parameters
    let plan2 = ReservePlan::new(reserve_opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should update timestamp, not create new reservation
    assert_eq!(plan2.actions.len(), 1);
    assert!(matches!(
        plan2.actions[0],
        trop::PlanAction::UpdateLastUsed(_)
    ));

    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan2).unwrap();
    assert!(result.success);
    assert_eq!(
        result.port,
        Some(allocated_port),
        "Second reservation should return same port"
    );

    // Verify only one reservation exists
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 1);
}

#[test]
fn test_sticky_field_protection() {
    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();
    let port = Port::try_from(8080).unwrap();

    // Create initial reservation with project
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(reserve_opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = trop::PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to change project without force
    let reserve_opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(reserve_opts2, &create_test_config()).build_plan(db.connection());
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::StickyFieldChange { .. }
    ));

    // Change project with force flag
    let reserve_opts3 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_force(true)
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(reserve_opts3, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan);
    assert!(result.is_ok());
}

#[test]
fn test_dry_run_mode() {
    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();
    let port = Port::try_from(8080).unwrap();

    let reserve_opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("test-project".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(reserve_opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Execute in dry-run mode
    let mut executor = trop::PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    assert!(result.success);
    assert!(result.dry_run);
    assert!(
        result.port.is_some(),
        "Should have planned to allocate a port"
    );

    // Verify no reservation was created
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(reservation.is_none());
}

#[test]
fn test_release_idempotent() {
    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();

    // Release non-existent reservation
    let release_opts = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);

    let plan = ReleasePlan::new(release_opts.clone())
        .build_plan(db.connection())
        .unwrap();

    // Should have no actions, just a warning
    assert_eq!(plan.actions.len(), 0);
    assert_eq!(plan.warnings.len(), 1);
    assert!(plan.warnings[0].contains("No reservation found"));

    let mut executor = trop::PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();
    assert!(result.success);
}

#[test]
fn test_multiple_tagged_reservations() {
    let db = create_test_database();
    let path = PathBuf::from("/test/project");

    // Create multiple reservations for same path with different tags
    let key1 = ReservationKey::new(path.clone(), Some("web".to_string())).unwrap();
    let key2 = ReservationKey::new(path.clone(), Some("api".to_string())).unwrap();
    let key3 = ReservationKey::new(path, None).unwrap();

    let port1 = Port::try_from(8080).unwrap();
    let port2 = Port::try_from(8081).unwrap();
    let port3 = Port::try_from(8082).unwrap();

    // Reserve all three
    for (key, port) in [(key1, port1), (key2, port2), (key3, port3)] {
        let opts = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(opts, &create_test_config())
            .build_plan(db.connection())
            .unwrap();
        let mut executor = trop::PlanExecutor::new(db.connection());
        executor.execute(&plan).unwrap();
    }

    // Verify all three exist
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 3);
}
