//! Integration tests for plan generation and dry-run mode.
//!
//! This test suite verifies that:
//! - Plans accurately describe operations before they're executed
//! - Dry-run mode generates plans without modifying the database
//! - Plan actions match the expected operation type (Create, Update, UpdateLastUsed, Delete)
//! - Plan descriptions and warnings are helpful and accurate
//! - The separation of planning and execution phases works correctly
//!
//! The plan-execute pattern is crucial for:
//! - Implementing --dry-run mode for safe previews
//! - Testing operation logic without database side effects
//! - Providing clear feedback about what will happen
//! - Enabling future features like batch operations and rollback

mod common;
use common::create_test_config;

use common::database::create_test_database;
use common::unrelated_path;
use std::path::PathBuf;
use trop::{
    Database, PlanAction, PlanExecutor, Port, ReleaseOptions, ReleasePlan, Reservation,
    ReservationKey, ReserveOptions, ReservePlan,
};

// Port base constants for test organization
const PORT_BASE_NEW_RESERVATION: u16 = 7000;
const PORT_BASE_EXISTING_RESERVATION: u16 = 7010;
const PORT_BASE_RELEASE_PLAN: u16 = 7020;
const PORT_BASE_DRY_RUN_RESERVE: u16 = 7030;
const PORT_BASE_DRY_RUN_RELEASE: u16 = 7040;
const PORT_BASE_NORMAL_EXEC: u16 = 7050;
const PORT_BASE_ERRORS: u16 = 7060;
const PORT_BASE_CONTENT: u16 = 7070;
const PORT_BASE_COMPLEX: u16 = 7100;

// =============================================================================
// Reserve Plan Generation - New Reservations
// =============================================================================

#[test]
fn test_reserve_plan_for_new_reservation_has_create_action() {
    // Tests that planning a new reservation generates a CreateReservation action.
    //
    // This is the most basic case: nothing exists, so we need to create.
    // The plan should have exactly one action of the correct type.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/new"), None).unwrap();
    let port = Port::try_from(PORT_BASE_NEW_RESERVATION).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should have exactly one action
    assert_eq!(plan.actions.len(), 1, "Plan should have one action");

    // Should be CreateReservation
    assert!(
        matches!(plan.actions[0], PlanAction::CreateReservation(_)),
        "Action should be CreateReservation, got: {:?}",
        plan.actions[0]
    );

    // No warnings for a straightforward new reservation
    assert_eq!(plan.warnings.len(), 0, "Should have no warnings");
}

#[test]
fn test_reserve_plan_create_action_contains_correct_reservation() {
    // Tests that the CreateReservation action contains the correct reservation data.
    //
    // The plan action should contain a fully-formed Reservation with all the
    // requested metadata.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/details"), None).unwrap();
    let port = Port::try_from(PORT_BASE_NEW_RESERVATION + 1).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("my-project".to_string()))
        .with_task(Some("my-task".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Extract the reservation from the plan
    if let PlanAction::CreateReservation(ref reservation) = plan.actions[0] {
        assert_eq!(reservation.key(), &key, "Key should match");
        // Port might be fallback if preferred was occupied
        assert!(
            reservation.port().value() >= 5000 && reservation.port().value() <= 7000,
            "Port should be in valid range"
        );
        assert_eq!(
            reservation.project(),
            Some("my-project"),
            "Project should match"
        );
        assert_eq!(reservation.task(), Some("my-task"), "Task should match");
    } else {
        panic!("Expected CreateReservation action");
    }
}

#[test]
fn test_reserve_plan_with_tagged_reservation() {
    // Tests that plans correctly handle tagged reservations.
    //
    // Tags are part of the ReservationKey and should be preserved in the plan.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/tagged"), Some("web".to_string())).unwrap();
    let port = Port::try_from(PORT_BASE_NEW_RESERVATION + 2).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    if let PlanAction::CreateReservation(ref reservation) = plan.actions[0] {
        assert_eq!(
            reservation.key().tag,
            Some("web".to_string()),
            "Tag should be preserved"
        );
    } else {
        panic!("Expected CreateReservation action");
    }
}

#[test]
fn test_reserve_plan_description_is_informative() {
    // Tests that plan descriptions help users understand what will happen.
    //
    // Good descriptions make --dry-run output useful.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/descriptive"), None).unwrap();
    let port = Port::try_from(PORT_BASE_NEW_RESERVATION + 3).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Description should mention what we're doing
    assert!(
        !plan.description.is_empty(),
        "Description should not be empty"
    );
    assert!(
        plan.description.contains("Reserve")
            || plan.description.contains("reserve")
            || plan.description.contains("port"),
        "Description should mention reservation, got: {}",
        plan.description
    );
}

// =============================================================================
// Reserve Plan Generation - Existing Reservations (Idempotent Case)
// =============================================================================

#[test]
fn test_reserve_plan_for_existing_reservation_has_update_last_used_action() {
    // Tests that planning for an existing reservation generates UpdateLastUsed.
    //
    // This is the idempotent case: reservation exists with matching metadata,
    // so we only need to update the timestamp.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/existing"), None).unwrap();
    let port = Port::try_from(PORT_BASE_EXISTING_RESERVATION).unwrap();

    // Create the initial reservation
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
    db.create_reservation(&reservation).unwrap();

    // Generate plan for the same reservation
    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should have exactly one action
    assert_eq!(plan.actions.len(), 1, "Plan should have one action");

    // Should be UpdateLastUsed
    assert!(
        matches!(plan.actions[0], PlanAction::UpdateLastUsed(_)),
        "Action should be UpdateLastUsed for existing reservation, got: {:?}",
        plan.actions[0]
    );

    // Verify it references the correct key
    if let PlanAction::UpdateLastUsed(ref action_key) = plan.actions[0] {
        assert_eq!(action_key, &key, "Should reference the correct key");
    }
}

#[test]
fn test_reserve_plan_idempotent_with_matching_metadata() {
    // Tests that providing the same metadata (project, task) as an existing
    // reservation results in an UpdateLastUsed plan.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/metadata-match"), None).unwrap();
    let port = Port::try_from(PORT_BASE_EXISTING_RESERVATION + 1).unwrap();

    // Create with metadata
    let reservation = Reservation::builder(key.clone(), port)
        .project(Some("project1".to_string()))
        .task(Some("task1".to_string()))
        .build()
        .unwrap();
    db.create_reservation(&reservation).unwrap();

    // Plan with same metadata
    let options = ReserveOptions::new(key, Some(port))
        .with_project(Some("project1".to_string()))
        .with_task(Some("task1".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should be idempotent (UpdateLastUsed)
    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(plan.actions[0], PlanAction::UpdateLastUsed(_)));
}

#[test]
fn test_reserve_plan_fails_with_metadata_mismatch() {
    // Tests that planning fails when metadata doesn't match and no override is set.
    //
    // This is validation happening in the planning phase, before any database
    // modifications.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/metadata-mismatch"), None).unwrap();
    let port = Port::try_from(PORT_BASE_EXISTING_RESERVATION + 2).unwrap();

    // Create with one project
    let reservation = Reservation::builder(key.clone(), port)
        .project(Some("project1".to_string()))
        .build()
        .unwrap();
    db.create_reservation(&reservation).unwrap();

    // Try to plan with different project
    let options = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(options, &create_test_config()).build_plan(db.connection());

    // Should fail during planning
    assert!(result.is_err(), "Plan generation should fail");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::StickyFieldChange { .. }
    ));
}

// =============================================================================
// Release Plan Generation
// =============================================================================

#[test]
fn test_release_plan_for_existing_reservation_has_delete_action() {
    // Tests that planning to release an existing reservation generates DeleteReservation.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/release-existing"), None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE_PLAN).unwrap();

    // Create a reservation
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
    db.create_reservation(&reservation).unwrap();

    // Plan to release it
    let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);

    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    // Should have exactly one action
    assert_eq!(plan.actions.len(), 1, "Plan should have one action");

    // Should be DeleteReservation
    assert!(
        matches!(plan.actions[0], PlanAction::DeleteReservation(_)),
        "Action should be DeleteReservation, got: {:?}",
        plan.actions[0]
    );

    // Verify it references the correct key
    if let PlanAction::DeleteReservation(ref action_key) = plan.actions[0] {
        assert_eq!(action_key, &key, "Should reference the correct key");
    }
}

#[test]
fn test_release_plan_for_nonexistent_reservation_has_no_actions() {
    // Tests that planning to release a non-existent reservation results in
    // an empty plan with a warning (idempotent release).

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/release-nonexistent"), None).unwrap();

    let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);

    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    // Should have no actions
    assert_eq!(
        plan.actions.len(),
        0,
        "Plan should have no actions for nonexistent reservation"
    );

    // Should have a warning
    assert_eq!(
        plan.warnings.len(),
        1,
        "Should have a warning about nothing to release"
    );

    assert!(
        plan.warnings[0].contains("No reservation found")
            || plan.warnings[0].contains("already released"),
        "Warning should explain nothing to release, got: {}",
        plan.warnings[0]
    );
}

#[test]
fn test_release_plan_description_is_informative() {
    // Tests that release plan descriptions are helpful.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/release-desc"), None).unwrap();

    let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);

    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    assert!(
        !plan.description.is_empty(),
        "Description should not be empty"
    );
    assert!(
        plan.description.contains("Release") || plan.description.contains("release"),
        "Description should mention release, got: {}",
        plan.description
    );
}

// =============================================================================
// Dry-Run Mode - Reserve Operations
// =============================================================================

#[test]
fn test_dry_run_reserve_does_not_create_reservation() {
    // Tests that executing a reserve plan in dry-run mode doesn't modify the database.
    //
    // This is the core promise of dry-run: you can see what would happen
    // without any side effects.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-reserve"), None).unwrap();
    let port = Port::try_from(PORT_BASE_DRY_RUN_RESERVE).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Execute in dry-run mode
    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    // Result should indicate dry-run
    assert!(result.dry_run, "Result should indicate dry-run mode");
    assert!(result.success, "Dry-run should succeed");

    // Database should be unchanged
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(
        reservation.is_none(),
        "Dry-run should not create reservation"
    );

    // Verify count is still zero
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 0, "Database should be unchanged");
}

#[test]
fn test_dry_run_reserve_returns_port_in_result() {
    // Tests that dry-run results still indicate what port would be reserved.
    //
    // Even though nothing is created, the result should show what would happen.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-port"), None).unwrap();
    let port = Port::try_from(PORT_BASE_DRY_RUN_RESERVE + 1).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    // Should report the port that would be reserved (might be fallback if preferred was occupied)
    assert!(result.port.is_some(), "Dry-run should report a port");
    let allocated_port = result.port.unwrap();
    assert!(
        allocated_port.value() >= 5000 && allocated_port.value() <= 7000,
        "Port should be in valid range"
    );
}

#[test]
fn test_dry_run_idempotent_reserve_does_not_update_timestamp() {
    // Tests that dry-run doesn't update timestamps on existing reservations.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-timestamp"), None).unwrap();
    let port = Port::try_from(PORT_BASE_DRY_RUN_RESERVE + 2).unwrap();

    // Create initial reservation
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
    db.create_reservation(&reservation).unwrap();

    // Get initial timestamp
    let initial = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    let initial_timestamp = initial.last_used_at();

    // Plan idempotent reserve
    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Execute in dry-run
    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    executor.execute(&plan).unwrap();

    // Timestamp should be unchanged
    let after = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    let after_timestamp = after.last_used_at();

    assert_eq!(
        after_timestamp, initial_timestamp,
        "Dry-run should not update timestamp"
    );
}

#[test]
fn test_dry_run_with_metadata_works() {
    // Tests that dry-run handles reservations with metadata correctly.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-metadata"), None).unwrap();
    let port = Port::try_from(PORT_BASE_DRY_RUN_RESERVE + 3).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("test-project".to_string()))
        .with_task(Some("test-task".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    assert!(result.success, "Dry-run with metadata should succeed");

    // Database should be unchanged
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(reservation.is_none(), "Dry-run should not create anything");
}

// =============================================================================
// Dry-Run Mode - Release Operations
// =============================================================================

#[test]
fn test_dry_run_release_does_not_delete_reservation() {
    // Tests that dry-run release doesn't actually delete anything.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-release"), None).unwrap();
    let port = Port::try_from(PORT_BASE_DRY_RUN_RELEASE).unwrap();

    // Create a reservation
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
    db.create_reservation(&reservation).unwrap();

    // Plan to release it
    let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    // Execute in dry-run
    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    assert!(result.dry_run, "Result should indicate dry-run");
    assert!(result.success, "Dry-run release should succeed");

    // Reservation should still exist
    let still_there = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(
        still_there.is_some(),
        "Dry-run release should not delete reservation"
    );
}

#[test]
fn test_dry_run_release_of_nonexistent_reservation() {
    // Tests that dry-run release of nonexistent reservation completes successfully.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-release-none"), None).unwrap();

    let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    assert!(
        result.success,
        "Dry-run should succeed even for nonexistent"
    );
    assert!(result.dry_run, "Should be marked as dry-run");
}

// =============================================================================
// Plan Execution - Normal Mode
// =============================================================================

#[test]
fn test_normal_execution_creates_reservation() {
    // Tests that executing a plan in normal mode (not dry-run) actually
    // modifies the database.
    //
    // This is the contrast to dry-run: normal execution should have side effects.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/normal-exec"), None).unwrap();
    let port = Port::try_from(PORT_BASE_NORMAL_EXEC).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Execute normally (not dry-run)
    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();

    assert!(!result.dry_run, "Should not be dry-run");
    assert!(result.success, "Execution should succeed");

    // Database should be modified
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(
        reservation.is_some(),
        "Normal execution should create reservation"
    );
}

#[test]
fn test_normal_execution_deletes_reservation() {
    // Tests that normal release execution actually deletes.

    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/normal-release"), None).unwrap();
    let port = Port::try_from(PORT_BASE_NORMAL_EXEC + 1).unwrap();

    // Create a reservation
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
    db.create_reservation(&reservation).unwrap();

    // Release normally
    let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();

    assert!(result.success, "Release should succeed");
    assert!(!result.dry_run, "Should not be dry-run");

    // Reservation should be gone
    let reservation = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(reservation.is_none(), "Reservation should be deleted");
}

// =============================================================================
// Plan Generation Error Cases
// =============================================================================

#[test]
fn test_plan_generation_uses_automatic_allocation_when_no_port() {
    // Tests that planning a new reservation without a port uses automatic allocation.
    //
    // With Phase 6, automatic allocation is used when no port is specified.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/no-port"), None).unwrap();

    // No port specified - should use automatic allocation
    let options = ReserveOptions::new(key.clone(), None).with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Should successfully create a plan with an allocated port
    assert_eq!(plan.actions.len(), 1);
    assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));

    // The allocated port should be within the configured range (5000-7000)
    if let PlanAction::CreateReservation(ref reservation) = plan.actions[0] {
        assert!(reservation.port().value() >= 5000);
        assert!(reservation.port().value() <= 7000);
        assert_eq!(reservation.key(), &key);
    }
}

#[test]
fn test_plan_generation_fails_for_path_violation() {
    // Tests that path validation errors occur during planning.

    let db = create_test_database();

    let unrelated = unrelated_path("plan_test");

    let key = ReservationKey::new(unrelated, None).unwrap();
    let port = Port::try_from(PORT_BASE_ERRORS).unwrap();

    // Don't allow unrelated path
    let options = ReserveOptions::new(key, Some(port));

    let result = ReservePlan::new(options, &create_test_config()).build_plan(db.connection());

    assert!(result.is_err(), "Should fail on path violation");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::PathRelationshipViolation { .. }
    ));
}

// =============================================================================
// Plan Content and Validation
// =============================================================================

#[test]
fn test_plan_is_empty_method() {
    // Tests the convenience method for checking if a plan has actions.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/empty"), None).unwrap();

    // Plan for nonexistent release has no actions
    let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .unwrap();

    assert!(plan.is_empty(), "Release of nonexistent should be empty");
    assert_eq!(plan.len(), 0, "Length should be 0");
}

#[test]
fn test_plan_is_not_empty_for_create() {
    // Tests that plans with actions are not empty.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/not-empty"), None).unwrap();
    let port = Port::try_from(PORT_BASE_CONTENT).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    assert!(!plan.is_empty(), "Create plan should not be empty");
    assert_eq!(plan.len(), 1, "Should have one action");
}

#[test]
fn test_plan_action_descriptions_are_helpful() {
    // Tests that individual action descriptions make sense.
    //
    // These descriptions are used in logging and dry-run output.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/action-desc"), None).unwrap();
    let port = Port::try_from(PORT_BASE_CONTENT + 1).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Get action description
    let description = plan.actions[0].description();

    assert!(!description.is_empty(), "Description should not be empty");
    assert!(
        description.contains("reservation") || description.contains("Reservation"),
        "Description should mention reservation"
    );
    assert!(
        description.contains("7071") || description.contains("port"),
        "Description should mention port"
    );
}

#[test]
fn test_multiple_plans_can_be_generated_without_execution() {
    // Tests that we can generate many plans without executing any of them.
    //
    // This demonstrates the separation of planning and execution phases.

    let db = create_test_database();

    let plans: Vec<_> = (0..10)
        .map(|i| {
            let key = ReservationKey::new(PathBuf::from(format!("/test/plan-{i}")), None).unwrap();
            let port = Port::try_from(PORT_BASE_CONTENT + 10 + i as u16).unwrap();

            let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

            ReservePlan::new(options, &create_test_config())
                .build_plan(db.connection())
                .unwrap()
        })
        .collect();

    // All plans should have been generated successfully
    assert_eq!(plans.len(), 10);

    // Database should still be empty (no execution happened)
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 0, "Generating plans should not modify database");

    // All plans should have create actions
    for plan in plans {
        assert_eq!(plan.len(), 1);
        assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));
    }
}

#[test]
fn test_plan_execution_result_contains_useful_information() {
    // Tests that execution results contain information about what happened.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/result-info"), None).unwrap();
    let port = Port::try_from(PORT_BASE_CONTENT + 20).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).unwrap();

    // Result should indicate success
    assert!(result.success, "Should indicate success");

    // Should indicate not dry-run
    assert!(!result.dry_run, "Should indicate normal execution");

    // Should include the port (might be fallback if preferred was occupied)
    assert!(result.port.is_some(), "Should report a port");
    let allocated_port = result.port.unwrap();
    assert!(
        allocated_port.value() >= 5000 && allocated_port.value() <= 7000,
        "Port should be in valid range"
    );
}

#[test]
fn test_dry_run_result_indicates_dry_run_mode() {
    // Tests that dry-run results are clearly marked.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/dry-run-marked"), None).unwrap();
    let port = Port::try_from(PORT_BASE_CONTENT + 21).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).unwrap();

    // Should be clearly marked as dry-run
    assert!(result.dry_run, "Result must indicate dry-run mode");
    assert!(result.success, "Dry-run should succeed");
}

// =============================================================================
// Complex Planning Scenarios
// =============================================================================

#[test]
fn test_planning_with_all_metadata_fields() {
    // Tests that plans correctly capture all metadata fields.

    let db = create_test_database();
    let key =
        ReservationKey::new(PathBuf::from("/test/all-metadata"), Some("web".to_string())).unwrap();
    let port = Port::try_from(PORT_BASE_COMPLEX).unwrap();

    let options = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("full-project".to_string()))
        .with_task(Some("full-task".to_string()))
        .with_allow_unrelated_path(true);

    let plan = ReservePlan::new(options, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    // Extract and verify the reservation
    if let PlanAction::CreateReservation(ref reservation) = plan.actions[0] {
        assert_eq!(reservation.key(), &key);
        // Port might be fallback if preferred was occupied
        assert!(
            reservation.port().value() >= 5000 && reservation.port().value() <= 7000,
            "Port should be in valid range"
        );
        assert_eq!(reservation.project(), Some("full-project"));
        assert_eq!(reservation.task(), Some("full-task"));
    } else {
        panic!("Expected CreateReservation");
    }
}

#[test]
fn test_planning_sequence_reserve_then_release() {
    // Tests that we can plan a sequence of operations.
    //
    // This demonstrates how plans can be generated, inspected, and then
    // executed in sequence.

    let db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/sequence"), None).unwrap();
    let port = Port::try_from(PORT_BASE_COMPLEX + 10).unwrap();

    // Plan 1: Reserve
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    let reserve_plan = ReservePlan::new(reserve_opts, &create_test_config())
        .build_plan(db.connection())
        .unwrap();

    assert_eq!(reserve_plan.len(), 1);
    assert!(matches!(
        reserve_plan.actions[0],
        PlanAction::CreateReservation(_)
    ));

    // Execute reserve
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&reserve_plan).unwrap();

    // Plan 2: Release
    let release_opts = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    let release_plan = ReleasePlan::new(release_opts)
        .build_plan(db.connection())
        .unwrap();

    assert_eq!(release_plan.len(), 1);
    assert!(matches!(
        release_plan.actions[0],
        PlanAction::DeleteReservation(_)
    ));

    // Execute release
    let mut executor = PlanExecutor::new(db.connection());
    executor.execute(&release_plan).unwrap();

    // Verify final state
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 0);
}
