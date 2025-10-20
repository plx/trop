//! Integration tests for idempotency and sticky field protection.
//!
//! This test suite verifies that:
//! - Reserve operations are truly idempotent (same inputs → same outputs)
//! - Timestamp updates occur on repeated reservations
//! - Sticky fields (project, task) are protected from accidental changes
//! - Force and specific allow flags correctly override protections
//! - Edge cases like null values and empty strings are handled correctly

mod common;
use common::create_test_config;

use common::database::create_test_database;
use common::{create_reservation, release_reservation};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use trop::{
    Database, PlanAction, PlanExecutor, Port, ReleaseOptions, ReleasePlan, ReservationKey,
    ReserveOptions, ReservePlan,
};

// Port base constants for test organization
const PORT_BASE_IDEMPOTENCY: u16 = 5000;
const PORT_BASE_STICKY_PROJECT: u16 = 5020;
const PORT_BASE_STICKY_TASK: u16 = 5030;
const PORT_BASE_STICKY_COMBINED: u16 = 5040;
const PORT_BASE_RELEASE_IDEMPOTENCY: u16 = 5050;

// =============================================================================
// Idempotency Tests
// =============================================================================

#[test]
fn test_idempotent_reserve_returns_same_port() {
    // Tests that calling reserve multiple times with identical parameters
    // returns the same port every time, verifying the core idempotency guarantee.
    //
    // This is critical because it ensures that running the same reserve command
    // multiple times (e.g., in a build script) will always get the same port back,
    // making the system predictable and safe for automation.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/idempotent"), None).unwrap();
    let port = Port::try_from(PORT_BASE_IDEMPOTENCY).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("test-project".to_string()))
        .with_allow_unrelated_path(true);

    // First reservation
    let plan1 = ReservePlan::new(options.clone(), &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    let result1 = executor.execute(&plan1).unwrap();
    assert!(result1.success);
    assert!(
        result1.port.is_some(),
        "First reservation should allocate a port"
    );

    // Save the port that was actually allocated (might not be the preferred one
    // if it was occupied on the system)
    let allocated_port = result1.port.unwrap();

    // Second reservation with identical parameters
    let plan2 = ReservePlan::new(options.clone(), &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    let result2 = executor.execute(&plan2).unwrap();
    assert!(result2.success);
    assert_eq!(
        result2.port,
        Some(allocated_port),
        "Second reservation should return same port"
    );

    // Third reservation - testing multiple repetitions
    let plan3 = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    let result3 = executor.execute(&plan3).unwrap();
    assert!(result3.success);
    assert_eq!(
        result3.port,
        Some(allocated_port),
        "Third reservation should return same port"
    );

    // All three should return the same port (idempotency)
    assert_eq!(result1.port, result2.port);
    assert_eq!(result2.port, result3.port);

    // Verify only one reservation exists in the database
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(
        all.len(),
        1,
        "Idempotent operations should not create duplicate reservations"
    );
}

#[test]
fn test_idempotent_reserve_updates_timestamp() {
    // Tests that repeated reserve operations update the last_used_at timestamp
    // even though they don't create a new reservation.
    //
    // This is important for automatic cleanup: frequently-used reservations
    // should have recent timestamps and won't be considered expired.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/timestamp"), None).unwrap();
    let port = Port::try_from(PORT_BASE_IDEMPOTENCY + 1).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);

    // First reservation
    let plan1 = ReservePlan::new(options.clone(), &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan1).unwrap();

    // Get initial timestamp
    let reservation1 = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    let timestamp1 = reservation1.last_used_at();

    // Sleep to ensure timestamp difference (Unix timestamps have second precision)
    thread::sleep(Duration::from_secs(2));

    // Second reservation
    let plan2 = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan2).unwrap();

    // Get updated timestamp
    let reservation2 = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    let timestamp2 = reservation2.last_used_at();

    // Timestamp should have been updated
    assert!(
        timestamp2 > timestamp1,
        "Repeated reserve should update last_used_at timestamp"
    );
}

#[test]
fn test_idempotent_reserve_action_is_update_last_used() {
    // Tests that the plan for an idempotent reserve operation contains
    // an UpdateLastUsed action, not a CreateReservation action.
    //
    // This verifies that the planning logic correctly identifies existing
    // reservations and generates the appropriate minimal plan.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/plan-action"), None).unwrap();
    let port = Port::try_from(PORT_BASE_IDEMPOTENCY + 2).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);

    // First reservation - should create
    let plan1 = ReservePlan::new(options.clone(), &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    assert_eq!(plan1.actions.len(), 1);
    assert!(
        matches!(plan1.actions[0], PlanAction::CreateReservation(_)),
        "First plan should create new reservation"
    );

    // Execute the creation
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan1).unwrap();

    // Second reservation - should only update timestamp
    let plan2 = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    assert_eq!(plan2.actions.len(), 1);
    assert!(
        matches!(plan2.actions[0], PlanAction::UpdateLastUsed(_)),
        "Subsequent plans should only update timestamp"
    );
}

#[test]
fn test_idempotent_reserve_with_different_tags() {
    // Tests that reservations with the same path but different tags are
    // independent and each maintains its own idempotency.
    //
    // This verifies that the ReservationKey (path + tag) properly distinguishes
    // between different reservations.

    let mut db = create_test_database();
    let path = PathBuf::from("/test/multi-tag");

    let key_web = ReservationKey::new(path.clone(), Some("web".to_string())).unwrap();
    let key_api = ReservationKey::new(path.clone(), Some("api".to_string())).unwrap();
    let key_none = ReservationKey::new(path, None).unwrap();

    let port_web = Port::try_from(PORT_BASE_IDEMPOTENCY + 10).unwrap();
    let port_api = Port::try_from(PORT_BASE_IDEMPOTENCY + 11).unwrap();
    let port_none = Port::try_from(PORT_BASE_IDEMPOTENCY + 12).unwrap();

    // Reserve all three
    for (key, port) in [
        (key_web.clone(), port_web),
        (key_api.clone(), port_api),
        (key_none.clone(), port_none),
    ] {
        let opts = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(opts, &create_test_config())
            .build_plan(db.connection())
            .unwrap();
        let mut executor = PlanExecutor::new(db.connection());
        executor.execute(&plan).unwrap();
    }

    // Re-reserve each one - should be idempotent
    for (key, expected_port) in [
        (key_web, port_web),
        (key_api, port_api),
        (key_none, port_none),
    ] {
        let opts = ReserveOptions::new(key, Some(expected_port)).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(opts, &create_test_config())
            .build_plan(db.connection())
            .unwrap();

        // Should be UpdateLastUsed, not CreateReservation
        assert_eq!(plan.actions.len(), 1);
        assert!(matches!(plan.actions[0], PlanAction::UpdateLastUsed(_)));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();
        assert_eq!(result.port, Some(expected_port));
    }

    // All three should still exist
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 3);
}

// =============================================================================
// Sticky Field Protection - Project Field
// =============================================================================

#[test]
fn test_cannot_change_project_without_permission() {
    // Tests that attempting to change the project field on an existing reservation
    // fails with StickyFieldChange error when no override flags are set.
    //
    // This protects against accidental configuration changes that might indicate
    // a mistake in the reservation request (e.g., wrong directory, copy-paste error).

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/sticky-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT).unwrap();

    // Create initial reservation with project "project1"
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to change project to "project2" without permission
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should fail to change project field");
    let err = result.unwrap_err();
    assert!(
        matches!(err, trop::Error::StickyFieldChange { .. }),
        "Should be StickyFieldChange error, got: {err:?}"
    );

    // Verify the error message mentions the field name
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("project"),
        "Error message should mention 'project'"
    );
}

#[test]
fn test_can_change_project_with_force_flag() {
    // Tests that the --force flag allows changing the project field.
    //
    // Force is the master override - it bypasses all sticky field protection.
    // This is useful when you know what you're doing and need to update metadata.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/force-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 1).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change project with force flag
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_force(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "Force flag should allow planning project change"
    );

    // Note: Currently the implementation validates that force allows the change,
    // but doesn't actually update sticky fields (generates UpdateLastUsed not UpdateReservation).
    // This test verifies that the validation passes with force, which is the current behavior.
}

#[test]
fn test_can_change_project_with_allow_project_change_flag() {
    // Tests that the --allow-project-change flag allows changing just the project field.
    //
    // This is more granular than --force, allowing project changes while still
    // protecting task and other fields.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/allow-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 2).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change project with specific allow flag
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_allow_project_change(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "allow_project_change flag should allow planning project change"
    );
}

#[test]
fn test_can_keep_same_project_value() {
    // Tests that providing the same project value as the existing reservation
    // is allowed (not considered a "change").
    //
    // This is important for idempotency: if your configuration says project=X
    // and the reservation already has project=X, it should succeed without
    // requiring any special flags.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/same-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 3).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Reserve again with same project - should succeed
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "Should allow keeping the same project value"
    );
}

#[test]
fn test_cannot_change_project_from_some_to_none() {
    // Tests that changing project from a value to None is blocked.
    //
    // This transition (Some → None) is considered a change and requires permission,
    // preventing accidental removal of metadata.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/project-to-none"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 4).unwrap();

    // Create with project
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to remove project (change to None)
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(None)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should block change from Some to None");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::StickyFieldChange { .. }
    ));
}

#[test]
fn test_cannot_change_project_from_none_to_some() {
    // Tests that changing project from None to a value is blocked.
    //
    // This transition (None → Some) is also considered a change requiring permission.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/none-to-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 5).unwrap();

    // Create with no project
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(None)
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to add project
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should block change from None to Some");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::StickyFieldChange { .. }
    ));
}

#[test]
fn test_can_keep_project_as_none() {
    // Tests that keeping project as None when it's already None is allowed.
    //
    // If both old and new values are None, there's no change to block.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/keep-none-project"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_PROJECT + 6).unwrap();

    // Create with no project
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(None)
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Reserve again with no project - should succeed
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(None)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_ok(), "Should allow keeping project as None");
}

// =============================================================================
// Sticky Field Protection - Task Field
// =============================================================================

#[test]
fn test_cannot_change_task_without_permission() {
    // Tests that attempting to change the task field fails without override flags.
    //
    // Task field protection works identically to project field protection.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/sticky-task"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_TASK).unwrap();

    // Create with task
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to change task
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_task(Some("task2".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should fail to change task field");
    let err = result.unwrap_err();
    assert!(matches!(err, trop::Error::StickyFieldChange { .. }));

    let err_msg = err.to_string();
    assert!(err_msg.contains("task"), "Error should mention 'task'");
}

#[test]
fn test_can_change_task_with_force_flag() {
    // Tests that --force allows changing the task field.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/force-task"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_TASK + 1).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change task with force
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task2".to_string()))
        .with_force(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "Force flag should allow planning task change"
    );
}

#[test]
fn test_can_change_task_with_allow_task_change_flag() {
    // Tests that --allow-task-change allows changing just the task field.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/allow-task"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_TASK + 2).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change task with specific allow flag
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task2".to_string()))
        .with_allow_task_change(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "allow_task_change flag should allow planning task change"
    );
}

#[test]
fn test_can_keep_same_task_value() {
    // Tests that keeping the same task value is allowed without special flags.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/same-task"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_TASK + 3).unwrap();

    // Create initial reservation
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Reserve again with same task
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_ok(), "Should allow keeping the same task value");
}

// =============================================================================
// Sticky Field Protection - Combined Project and Task
// =============================================================================

#[test]
fn test_cannot_change_both_project_and_task() {
    // Tests that attempting to change both sticky fields at once is blocked.
    //
    // The validation should fail on the first field checked, preventing
    // any partial updates.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/both-sticky"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_COMBINED).unwrap();

    // Create with both fields
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to change both
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_task(Some("task2".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should fail when changing both fields");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::StickyFieldChange { .. }
    ));
}

#[test]
fn test_can_change_both_with_force() {
    // Tests that --force allows changing both sticky fields.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/both-force"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_COMBINED + 1).unwrap();

    // Create with both fields
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change both with force
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_task(Some("task2".to_string()))
        .with_force(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "Force should allow planning changes to both fields"
    );
}

#[test]
fn test_can_change_both_with_individual_flags() {
    // Tests that setting both allow flags enables changing both fields.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/both-allow"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_COMBINED + 2).unwrap();

    // Create with both fields
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Change both with individual allow flags
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_task(Some("task2".to_string()))
        .with_allow_project_change(true)
        .with_allow_task_change(true)
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    assert!(
        result.is_ok(),
        "Individual flags should allow planning changes to both fields"
    );
}

#[test]
fn test_can_change_project_but_not_task_with_selective_flags() {
    // Tests that allow_project_change only allows changing project,
    // not task, demonstrating flag independence.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/selective"), None).unwrap();
    let port = Port::try_from(PORT_BASE_STICKY_COMBINED + 3).unwrap();

    // Create with both fields
    let opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan).unwrap();

    // Try to change both but only allow project change
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_task(Some("task2".to_string()))
        .with_allow_project_change(true) // Only project allowed
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(opts2, &create_test_config()).build_plan(db.connection());

    // Should fail because task change is not allowed
    assert!(result.is_err(), "Should fail on task change");
    let err = result.unwrap_err();
    assert!(matches!(err, trop::Error::StickyFieldChange { .. }));

    let err_msg = err.to_string();
    assert!(err_msg.contains("task"), "Error should be about task field");
}

// =============================================================================
// Release Idempotency
// =============================================================================

#[test]
fn test_release_is_idempotent() {
    // Tests that releasing a reservation multiple times doesn't cause errors.
    //
    // The first release deletes the reservation; subsequent releases should
    // succeed with a warning (not an error) because the desired state is achieved.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/release-idempotent"), None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE_IDEMPOTENCY).unwrap();

    // Create a reservation
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    create_reservation(&mut db, reserve_opts, &create_test_config()).unwrap();

    // Verify it exists
    assert!(Database::get_reservation(db.connection(), &key)
        .unwrap()
        .is_some());

    // First release
    let release_opts = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
    let plan1 = ReleasePlan::new(release_opts.clone())
        .build_plan(db.connection())
        .unwrap();
    let mut executor = PlanExecutor::new(db.connection());
    let result1 = executor.execute(&plan1).unwrap();
    assert!(result1.success);

    // Verify it's gone
    assert!(Database::get_reservation(db.connection(), &key)
        .unwrap()
        .is_none());

    // Second release - should succeed idempotently
    let plan2 = ReleasePlan::new(release_opts)
        .build_plan(db.connection())
        .unwrap();

    // Plan should have no actions, just a warning
    assert_eq!(
        plan2.actions.len(),
        0,
        "Second release should have no actions"
    );
    assert_eq!(plan2.warnings.len(), 1, "Should have a warning");
    assert!(plan2.warnings[0].contains("No reservation found"));

    let mut executor = PlanExecutor::new(db.connection());
    let result2 = executor.execute(&plan2).unwrap();
    assert!(result2.success, "Second release should succeed");
}

#[test]
fn test_reserve_after_release_creates_new_reservation() {
    // Tests the full lifecycle: reserve → release → reserve again.
    //
    // After a release, reserving again should create a new reservation,
    // not try to update a non-existent one.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/lifecycle"), None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE_IDEMPOTENCY + 1).unwrap();

    // First reservation
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);
    create_reservation(&mut db, reserve_opts, &create_test_config()).unwrap();

    // Release
    let release_opts = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
    release_reservation(&mut db, release_opts).unwrap();

    // Reserve again - should create new reservation
    let reserve_opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string())) // Different project is OK now
        .with_allow_unrelated_path(true);

    let plan2 = ReservePlan::new(reserve_opts2, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should be CreateReservation, not UpdateLastUsed
    assert_eq!(plan2.actions.len(), 1);
    assert!(
        matches!(plan2.actions[0], PlanAction::CreateReservation(_)),
        "After release, should create new reservation"
    );

    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&plan2).unwrap();

    // Verify new reservation exists with new project
    let reservation = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    assert_eq!(reservation.project(), Some("project2"));
}
