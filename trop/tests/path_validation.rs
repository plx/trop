//! Integration tests for path relationship validation.
//!
//! This test suite verifies that:
//! - Ancestor and descendant paths are allowed (hierarchical relationships)
//! - Unrelated paths are blocked by default
//! - The --force and --allow-unrelated-path flags correctly override restrictions
//! - Path validation applies to both reserve and release operations
//! - Edge cases like current directory, parent directory, and root paths work correctly
//!
//! Path relationships are a key safety feature: they prevent accidentally
//! modifying reservations from completely unrelated projects when you run
//! trop commands from the wrong directory.

mod common;

use common::database::create_test_database;
use common::{create_reservation, unrelated_path};
use std::env;
use std::path::PathBuf;
use trop::{
    PlanExecutor, Port, ReleaseOptions, ReleasePlan, ReservationKey, ReserveOptions, ReservePlan,
};

// Port base constants for test organization
const PORT_BASE_RESERVE_ALLOWED: u16 = 6000;
const PORT_BASE_RESERVE_BLOCKED: u16 = 6020;
const PORT_BASE_RESERVE_OVERRIDE: u16 = 6030;
const PORT_BASE_RELEASE: u16 = 6040;
const PORT_BASE_MULTIPLE: u16 = 6060;
const PORT_BASE_EDGE: u16 = 6080;

// =============================================================================
// Reserve Path Validation - Allowed Relationships
// =============================================================================

#[test]
fn test_reserve_current_directory_allowed() {
    // Tests that reserving for the current working directory is always allowed.
    //
    // The current directory is considered a valid target because you're explicitly
    // running the command from there, showing clear intent.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let key = ReservationKey::new(cwd, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_ALLOWED).unwrap();

    // Should succeed without any special flags
    let options = ReserveOptions::new(key, Some(port));

    let result = ReservePlan::new(options).build_plan(&db);

    assert!(
        result.is_ok(),
        "Current directory should always be allowed: {:?}",
        result.err()
    );
}

#[test]
fn test_reserve_subdirectory_allowed() {
    // Tests that reserving for a subdirectory of the current directory is allowed.
    //
    // This is a descendant relationship: if you're in /home/user/project,
    // you should be able to reserve for /home/user/project/backend.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let subdir = cwd.join("subdirectory");
    let key = ReservationKey::new(subdir, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_ALLOWED + 1).unwrap();

    let options = ReserveOptions::new(key, Some(port));

    let result = ReservePlan::new(options).build_plan(&db);

    assert!(
        result.is_ok(),
        "Subdirectory should be allowed: {:?}",
        result.err()
    );
}

#[test]
fn test_reserve_parent_directory_allowed() {
    // Tests that reserving for a parent directory is allowed.
    //
    // This is an ancestor relationship: if you're in /home/user/project/backend,
    // you should be able to reserve for /home/user/project.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();

    // Get parent directory (if it exists)
    if let Some(parent) = cwd.parent() {
        let key = ReservationKey::new(parent.to_path_buf(), None).unwrap();
        let port = Port::try_from(PORT_BASE_RESERVE_ALLOWED + 2).unwrap();

        let options = ReserveOptions::new(key, Some(port));

        let result = ReservePlan::new(options).build_plan(&db);

        assert!(
            result.is_ok(),
            "Parent directory should be allowed: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_reserve_nested_subdirectory_allowed() {
    // Tests that deeply nested subdirectories are allowed.
    //
    // The hierarchical relationship extends to any depth.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let nested = cwd.join("a").join("b").join("c").join("d");
    let key = ReservationKey::new(nested, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_ALLOWED + 3).unwrap();

    let options = ReserveOptions::new(key, Some(port));

    let result = ReservePlan::new(options).build_plan(&db);

    assert!(
        result.is_ok(),
        "Nested subdirectory should be allowed: {:?}",
        result.err()
    );
}

#[test]
fn test_reserve_ancestor_directory_allowed() {
    // Tests that any ancestor in the path hierarchy is allowed.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();

    // Walk up the directory tree testing ancestors
    let mut current = cwd.as_path();
    let mut tested_count = 0;

    while let Some(parent) = current.parent() {
        let key = ReservationKey::new(parent.to_path_buf(), None).unwrap();
        let port = Port::try_from(PORT_BASE_RESERVE_ALLOWED + 10 + tested_count).unwrap();

        let options = ReserveOptions::new(key, Some(port));
        let result = ReservePlan::new(options).build_plan(&db);

        assert!(result.is_ok(), "Ancestor {:?} should be allowed", parent);

        current = parent;
        tested_count += 1;

        // Limit iterations to prevent infinite loops
        if tested_count >= 10 {
            break;
        }
    }

    assert!(tested_count > 0, "Should have tested at least one ancestor");
}

// =============================================================================
// Reserve Path Validation - Blocked Relationships
// =============================================================================

#[test]
fn test_reserve_unrelated_absolute_path_blocked() {
    // Tests that an absolute path unrelated to the current directory is blocked.
    //
    // This is the core protection: prevents accidentally operating on
    // unrelated projects.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();

    // Create a path that's definitely unrelated by using a different root structure
    let unrelated = unrelated_path("completely/unrelated/path");

    // Verify it's actually unrelated (not an ancestor or descendant)
    assert!(
        !cwd.starts_with(&unrelated) && !unrelated.starts_with(&cwd),
        "Test path should be unrelated to cwd"
    );

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_BLOCKED).unwrap();

    // Should fail without allow flag
    let options = ReserveOptions::new(key, Some(port));

    let result = ReservePlan::new(options).build_plan(&db);

    assert!(result.is_err(), "Unrelated path should be blocked");
    let err = result.unwrap_err();
    assert!(
        matches!(err, trop::Error::PathRelationshipViolation { .. }),
        "Should be PathRelationshipViolation, got: {err:?}"
    );

    // Error message should be informative
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("path relationship") || err_msg.contains("unrelated"),
        "Error message should mention path relationship"
    );
}

#[test]
fn test_reserve_sibling_directory_blocked() {
    // Tests that sibling directories are blocked.
    //
    // If you're in /home/user/project-a, you shouldn't be able to
    // modify /home/user/project-b without explicit permission.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();

    // Get a sibling by going to parent and adding a different child
    if let Some(parent) = cwd.parent() {
        let sibling = parent.join("completely_different_sibling_directory_12345");

        // Verify it's a sibling (same parent, but not ancestor/descendant of cwd)
        assert!(!cwd.starts_with(&sibling) && !sibling.starts_with(&cwd));

        let key = ReservationKey::new(sibling, None).unwrap();
        let port = Port::try_from(PORT_BASE_RESERVE_BLOCKED + 1).unwrap();

        let options = ReserveOptions::new(key, Some(port));

        let result = ReservePlan::new(options).build_plan(&db);

        assert!(result.is_err(), "Sibling directory should be blocked");
        assert!(matches!(
            result.unwrap_err(),
            trop::Error::PathRelationshipViolation { .. }
        ));
    }
}

#[test]
fn test_reserve_cousin_path_blocked() {
    // Tests that "cousin" paths (sharing a grandparent but not parent) are blocked.
    //
    // Example: from /a/b/c, cannot access /a/d/e without permission.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();

    // Create a cousin: go up two levels, then down a different path
    if let Some(grandparent) = cwd.parent().and_then(|p| p.parent()) {
        let cousin = grandparent
            .join("different_branch_12345")
            .join("cousin_path");

        // Verify it's unrelated
        assert!(!cwd.starts_with(&cousin) && !cousin.starts_with(&cwd));

        let key = ReservationKey::new(cousin, None).unwrap();
        let port = Port::try_from(PORT_BASE_RESERVE_BLOCKED + 2).unwrap();

        let options = ReserveOptions::new(key, Some(port));

        let result = ReservePlan::new(options).build_plan(&db);

        assert!(result.is_err(), "Cousin path should be blocked");
        assert!(matches!(
            result.unwrap_err(),
            trop::Error::PathRelationshipViolation { .. }
        ));
    }
}

// =============================================================================
// Reserve Path Validation - Override Mechanisms
// =============================================================================

#[test]
fn test_reserve_unrelated_path_with_allow_flag() {
    // Tests that --allow-unrelated-path allows operations on unrelated paths.
    //
    // This flag provides explicit opt-in for cross-project operations.

    let mut db = create_test_database();

    let unrelated = unrelated_path("unrelated/path");

    let key = ReservationKey::new(unrelated, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_OVERRIDE).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

    let result = ReservePlan::new(options.clone()).build_plan(&db);

    assert!(
        result.is_ok(),
        "Unrelated path should be allowed with flag: {:?}",
        result.err()
    );

    // Also verify we can execute the plan
    let plan = result.unwrap();
    let mut executor = PlanExecutor::new(&mut db);
    let exec_result = executor.execute(&plan);

    assert!(
        exec_result.is_ok(),
        "Execution should succeed with allow flag"
    );
}

#[test]
fn test_reserve_unrelated_path_with_force_flag() {
    // Tests that --force also allows unrelated paths.
    //
    // Force is a master override that bypasses all safety checks.

    let mut db = create_test_database();

    let unrelated = unrelated_path("forced/path");

    let key = ReservationKey::new(unrelated, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_OVERRIDE + 1).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_force(true);

    let result = ReservePlan::new(options.clone()).build_plan(&db);

    assert!(
        result.is_ok(),
        "Force flag should allow unrelated paths: {:?}",
        result.err()
    );

    // Verify execution
    let plan = result.unwrap();
    let mut executor = PlanExecutor::new(&mut db);
    let exec_result = executor.execute(&plan);

    assert!(exec_result.is_ok(), "Execution should succeed with force");
}

#[test]
fn test_reserve_force_overrides_all_path_checks() {
    // Tests that force flag works for any path, even obviously wrong ones.
    //
    // This verifies force is truly a master override.

    let db = create_test_database();

    // Use root directory or C:\ - definitely unrelated
    let unrelated = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };

    let key = ReservationKey::new(unrelated, None).unwrap();
    let port = Port::try_from(PORT_BASE_RESERVE_OVERRIDE + 2).unwrap();

    let options = ReserveOptions::new(key, Some(port)).with_force(true);

    let result = ReservePlan::new(options).build_plan(&db);

    assert!(
        result.is_ok(),
        "Force should override even root path restrictions"
    );
}

// =============================================================================
// Release Path Validation - Same Rules Apply
// =============================================================================

#[test]
fn test_release_current_directory_allowed() {
    // Tests that releasing from the current directory is allowed.
    //
    // Path validation rules apply identically to release operations.

    let mut db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let key = ReservationKey::new(cwd, None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE).unwrap();

    // Create a reservation first (with allow flag since we're in a temp dir)
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    create_reservation(&mut db, reserve_opts).unwrap();

    // Release should work without special flags
    let release_opts = ReleaseOptions::new(key);
    let result = ReleasePlan::new(release_opts).build_plan(&db);

    assert!(result.is_ok(), "Current directory release should succeed");
}

#[test]
fn test_release_subdirectory_allowed() {
    // Tests that releasing a subdirectory reservation is allowed.

    let mut db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let subdir = cwd.join("subdir");
    let key = ReservationKey::new(subdir, None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE + 1).unwrap();

    // Create reservation
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    create_reservation(&mut db, reserve_opts).unwrap();

    // Release subdirectory
    let release_opts = ReleaseOptions::new(key);
    let result = ReleasePlan::new(release_opts).build_plan(&db);

    assert!(result.is_ok(), "Subdirectory release should succeed");
}

#[test]
fn test_release_unrelated_path_blocked() {
    // Tests that releasing an unrelated path is blocked by default.

    let db = create_test_database();

    let unrelated = unrelated_path("release_test");

    let key = ReservationKey::new(unrelated, None).unwrap();

    let release_opts = ReleaseOptions::new(key);
    let result = ReleasePlan::new(release_opts).build_plan(&db);

    assert!(result.is_err(), "Unrelated path release should be blocked");
    assert!(matches!(
        result.unwrap_err(),
        trop::Error::PathRelationshipViolation { .. }
    ));
}

#[test]
fn test_release_unrelated_path_with_allow_flag() {
    // Tests that --allow-unrelated-path enables releasing unrelated paths.

    let mut db = create_test_database();

    let unrelated = unrelated_path("release_allowed");

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE + 10).unwrap();

    // Create reservation
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_allow_unrelated_path(true);
    create_reservation(&mut db, reserve_opts).unwrap();

    // Release with allow flag
    let release_opts = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    let result = ReleasePlan::new(release_opts.clone()).build_plan(&db);

    assert!(
        result.is_ok(),
        "Release should succeed with allow flag: {:?}",
        result.err()
    );

    // Execute the release
    let plan = result.unwrap();
    let mut executor = PlanExecutor::new(&mut db);
    let exec_result = executor.execute(&plan);

    assert!(exec_result.is_ok(), "Release execution should succeed");
}

#[test]
fn test_release_unrelated_path_with_force_flag() {
    // Tests that --force enables releasing unrelated paths.

    let mut db = create_test_database();

    let unrelated = unrelated_path("release_forced");

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_RELEASE + 11).unwrap();

    // Create reservation
    let reserve_opts = ReserveOptions::new(key.clone(), Some(port)).with_force(true);
    create_reservation(&mut db, reserve_opts).unwrap();

    // Release with force
    let release_opts = ReleaseOptions::new(key).with_force(true);
    let result = ReleasePlan::new(release_opts).build_plan(&db);

    assert!(
        result.is_ok(),
        "Release should succeed with force: {:?}",
        result.err()
    );

    // Execute the release
    let plan = result.unwrap();
    let mut executor = PlanExecutor::new(&mut db);
    let exec_result = executor.execute(&plan);

    assert!(exec_result.is_ok(), "Release execution should succeed");
}

// =============================================================================
// Path Validation with Multiple Reservations
// =============================================================================

#[test]
fn test_multiple_related_paths_all_allowed() {
    // Tests that multiple hierarchically-related paths can all be reserved.
    //
    // This verifies that path validation doesn't get confused when
    // operating on multiple paths in the same hierarchy.

    let mut db = create_test_database();
    let cwd = env::current_dir().unwrap();

    let paths = vec![
        cwd.clone(),
        cwd.join("frontend"),
        cwd.join("backend"),
        cwd.join("backend").join("api"),
    ];

    // Reserve all of them
    for (i, path) in paths.iter().enumerate() {
        let key = ReservationKey::new(path.clone(), None).unwrap();
        let port = Port::try_from(PORT_BASE_MULTIPLE + i as u16).unwrap();

        let options = ReserveOptions::new(key, Some(port));
        let result = create_reservation(&mut db, options);

        assert!(
            result.is_ok(),
            "Should be able to reserve related path {:?}",
            path
        );
    }

    // Verify all were created
    let all = db.list_all_reservations().unwrap();
    assert_eq!(all.len(), paths.len());
}

#[test]
fn test_mixed_related_and_unrelated_paths() {
    // Tests that related paths work while unrelated paths are still blocked,
    // demonstrating that validation is applied per-operation, not globally.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let subdir = cwd.join("allowed");

    let unrelated = unrelated_path("blocked");

    // Related path should work
    let key1 = ReservationKey::new(subdir, None).unwrap();
    let port1 = Port::try_from(PORT_BASE_MULTIPLE + 10).unwrap();

    let opts1 = ReserveOptions::new(key1, Some(port1));
    let result1 = ReservePlan::new(opts1).build_plan(&db);

    assert!(result1.is_ok(), "Related path should work");

    // Unrelated path should fail
    let key2 = ReservationKey::new(unrelated, None).unwrap();
    let port2 = Port::try_from(PORT_BASE_MULTIPLE + 11).unwrap();

    let opts2 = ReserveOptions::new(key2, Some(port2));
    let result2 = ReservePlan::new(opts2).build_plan(&db);

    assert!(result2.is_err(), "Unrelated path should still be blocked");
}

// =============================================================================
// Edge Cases and Boundary Conditions
// =============================================================================

#[test]
fn test_path_validation_with_tagged_reservations() {
    // Tests that path validation works correctly with tagged reservations.
    //
    // The tag doesn't affect path relationship checking - only the path matters.

    let db = create_test_database();
    let cwd = env::current_dir().unwrap();
    let subdir = cwd.join("tagged");

    // Reserve with tag - should work
    let key = ReservationKey::new(subdir, Some("web".to_string())).unwrap();
    let port = Port::try_from(PORT_BASE_EDGE).unwrap();

    let options = ReserveOptions::new(key, Some(port));
    let result = ReservePlan::new(options).build_plan(&db);

    assert!(
        result.is_ok(),
        "Tagged reservation should respect path validation"
    );
}

#[test]
fn test_path_validation_applies_before_sticky_field_check() {
    // Tests the order of validation: path relationship is checked before
    // sticky field protection.
    //
    // This ensures that unrelated paths are rejected early, even if they
    // would fail sticky field checks later.

    let mut db = create_test_database();

    let unrelated = unrelated_path("validation_order");

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_EDGE + 10).unwrap();

    // Create initial reservation with allow flag
    let opts1 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);
    create_reservation(&mut db, opts1).unwrap();

    // Try to reserve again with different project, no allow flag
    // Should fail on path validation, not sticky field check
    let opts2 = ReserveOptions::new(key, Some(port)).with_project(Some("project2".to_string()));

    let result = ReservePlan::new(opts2).build_plan(&db);

    assert!(result.is_err(), "Should fail validation");

    // Should be path relationship error, not sticky field error
    let err = result.unwrap_err();
    assert!(
        matches!(err, trop::Error::PathRelationshipViolation { .. }),
        "Should fail on path validation first, got: {err:?}"
    );
}

#[test]
fn test_allow_unrelated_path_does_not_affect_sticky_fields() {
    // Tests that --allow-unrelated-path only affects path validation,
    // not sticky field protection.
    //
    // Each protection mechanism is independent and has its own override flags.

    let mut db = create_test_database();

    let unrelated = unrelated_path("independent_flags");

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_EDGE + 11).unwrap();

    // Create with project
    let opts1 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_allow_unrelated_path(true);
    create_reservation(&mut db, opts1).unwrap();

    // Try to change project with only allow_unrelated_path
    let opts2 = ReserveOptions::new(key, Some(port))
        .with_project(Some("project2".to_string()))
        .with_allow_unrelated_path(true); // Path flag set, but not project flag

    let result = ReservePlan::new(opts2).build_plan(&db);

    // Should fail on sticky field check, not path check
    assert!(result.is_err(), "Should fail on sticky field check");
    let err = result.unwrap_err();
    assert!(
        matches!(err, trop::Error::StickyFieldChange { .. }),
        "Should be sticky field error, got: {err:?}"
    );
}

#[test]
fn test_force_flag_overrides_both_path_and_sticky_checks() {
    // Tests that --force is truly a master override for all protections.

    let mut db = create_test_database();

    let unrelated = unrelated_path("force_all");

    let key = ReservationKey::new(unrelated.clone(), None).unwrap();
    let port = Port::try_from(PORT_BASE_EDGE + 12).unwrap();

    // Create with project
    let opts1 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project1".to_string()))
        .with_force(true);
    create_reservation(&mut db, opts1).unwrap();

    // Change project and use unrelated path - both should work with force
    let opts2 = ReserveOptions::new(key.clone(), Some(port))
        .with_project(Some("project2".to_string()))
        .with_force(true);

    let result = ReservePlan::new(opts2.clone()).build_plan(&db);

    assert!(
        result.is_ok(),
        "Force should override all protections: {:?}",
        result.err()
    );

    // The planning succeeds, which means force allows both path and sticky field changes.
    // Note: Actual field updates would require UpdateReservation action, which is not
    // currently generated by the reserve logic - this is expected current behavior.
}
