//! Integration tests for group reservation operations.
//!
//! This test suite verifies the end-to-end behavior of group reservation
//! functionality, including:
//! - Reserve group with offset-based allocation
//! - Mixed offset and preferred ports
//! - Config discovery for autoreserve
//! - Override flags behavior
//! - Database state verification
//! - Error handling and rollback scenarios

mod common;

use common::database::create_test_database;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use trop::operations::{
    AutoreserveOptions, AutoreservePlan, ReserveGroupOptions, ReserveGroupPlan,
};
use trop::Database;
use trop::{PlanExecutor, ReservationKey};

// ============================================================================
// Test Helpers
// ============================================================================

/// Creates a temporary directory for testing.
///
/// The directory will be automatically cleaned up when dropped.
fn create_temp_dir() -> TempDir {
    TempDir::new().expect("Failed to create temporary directory")
}

/// Creates a test configuration file with the given content.
///
/// Returns the path to the created file.
fn create_config_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let config_path = dir.join(name);
    fs::write(&config_path, content).expect("Failed to write config file");
    config_path
}

/// Creates a simple group reservation configuration with offset-based services.
///
/// Returns a YAML string defining a reservation group with the specified services.
fn simple_offset_config(num_services: usize) -> String {
    let mut services = String::new();
    for i in 0..num_services {
        services.push_str(&format!("    service{i}:\n      offset: {i}\n"));
    }

    format!(
        r#"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
{services}
"#
    )
}

/// Creates a configuration with mixed offset and preferred ports.
fn mixed_allocation_config() -> String {
    r#"
project: test-project
ports:
  min: 5000
  max: 9000
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
    admin:
      preferred: 8080
"#
    .to_string()
}

/// Creates a configuration with only preferred ports (no base port needed).
fn preferred_only_config() -> String {
    r#"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      preferred: 5010
    api:
      preferred: 5020
"#
    .to_string()
}

/// Creates a minimal valid configuration for discovery tests.
fn minimal_config() -> String {
    r#"
project: discovery-test
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
"#
    .to_string()
}

// ============================================================================
// Successful Group Reservation Tests
// ============================================================================

/// Test successful group reservation with offset-based allocation.
///
/// This test verifies the core group reservation functionality:
/// - All services are allocated sequential ports based on offsets
/// - The base port is chosen correctly
/// - Database state reflects all reservations
/// - All reservations share the same path, project, and task
///
/// Invariants tested:
/// - Service ports follow offset pattern: port(service_i) = base + offset_i
/// - All reservations are created atomically
/// - Database consistency after allocation
#[test]
fn test_successful_group_reservation_with_offsets() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(3));
    let mut db = create_test_database();

    // Build and execute the reservation plan
    let options = ReserveGroupOptions::new(config_path.clone());
    let planner = ReserveGroupPlan::new(options).expect("Failed to create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Failed to build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Failed to execute plan");

    // Verify execution succeeded
    assert!(result.success, "Execution should succeed");
    assert!(!result.dry_run, "Should not be dry run");

    // Verify allocated ports were returned
    let allocated_ports = result.allocated_ports.expect("Should have allocated ports");
    assert_eq!(allocated_ports.len(), 3, "Should allocate exactly 3 ports");

    // Verify ports follow the offset pattern
    // All services should have sequential ports starting from some base
    let service0_port = allocated_ports
        .get("service0")
        .expect("service0 should be allocated");
    let service1_port = allocated_ports
        .get("service1")
        .expect("service1 should be allocated");
    let service2_port = allocated_ports
        .get("service2")
        .expect("service2 should be allocated");

    assert_eq!(
        service1_port.value(),
        service0_port.value() + 1,
        "service1 should be base + 1"
    );
    assert_eq!(
        service2_port.value(),
        service0_port.value() + 2,
        "service2 should be base + 2"
    );

    // Verify database state - all reservations should exist
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Failed to list reservations");
    assert_eq!(
        all_reservations.len(),
        3,
        "Database should contain exactly 3 reservations"
    );

    // Verify each reservation has correct attributes
    for reservation in &all_reservations {
        assert_eq!(
            reservation.key().path,
            temp_dir.path(),
            "Reservation path should match config parent directory"
        );
        assert_eq!(
            reservation.project(),
            Some("test-project"),
            "Reservation project should match config"
        );
        assert!(
            reservation.key().tag.is_some(),
            "Reservation should have a tag"
        );

        // Verify the port matches what was allocated
        let tag = reservation.key().tag.as_ref().unwrap();
        let expected_port = allocated_ports
            .get(tag)
            .expect("Tag should exist in allocations");
        assert_eq!(
            reservation.port(),
            *expected_port,
            "Database port should match allocated port for {tag}"
        );
    }
}

/// Test group reservation with mixed offset and preferred ports.
///
/// This test verifies that group allocation correctly handles services
/// using different allocation strategies within the same group:
/// - Offset-based services get sequential ports from a base
/// - Preferred-port services get their explicitly requested ports
/// - No conflicts occur between the two strategies
///
/// Invariants tested:
/// - Offset-based services follow pattern allocation
/// - Preferred-port services receive exact requested ports
/// - All allocations coexist without conflicts
/// - Database state is consistent
#[test]
fn test_mixed_offset_and_preferred_ports() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &mixed_allocation_config());
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Failed to create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Failed to build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Failed to execute plan");

    assert!(result.success);

    let allocated_ports = result.allocated_ports.expect("Should have allocated ports");
    assert_eq!(allocated_ports.len(), 3);

    // Verify offset-based services are sequential
    let web_port = allocated_ports.get("web").expect("web should be allocated");
    let api_port = allocated_ports.get("api").expect("api should be allocated");
    assert_eq!(
        api_port.value(),
        web_port.value() + 1,
        "Offset services should be sequential"
    );

    // Verify preferred port service got its requested port
    let admin_port = allocated_ports
        .get("admin")
        .expect("admin should be allocated");
    assert_eq!(
        admin_port.value(),
        8080,
        "Preferred port service should get exact port"
    );

    // Verify no conflicts - admin port should be different from web/api ports
    assert_ne!(
        admin_port.value(),
        web_port.value(),
        "Preferred port should not conflict with offset ports"
    );
    assert_ne!(
        admin_port.value(),
        api_port.value(),
        "Preferred port should not conflict with offset ports"
    );

    // Verify all reservations are in database
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Failed to list reservations");
    assert_eq!(all_reservations.len(), 3);
}

/// Test group reservation with only preferred ports (no base port).
///
/// This test verifies that groups can be allocated without any offset-based
/// services, using only preferred ports:
/// - No base port is needed or calculated
/// - Each service gets its exact requested port
/// - Result correctly indicates no base port
///
/// Invariants tested:
/// - GroupAllocationResult.base_port is None when no offsets used
/// - All preferred ports are allocated successfully
/// - No sequential constraint between services
#[test]
fn test_group_reservation_preferred_ports_only() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &preferred_only_config());
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Failed to create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Failed to build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Failed to execute plan");

    assert!(result.success);

    let allocated_ports = result.allocated_ports.expect("Should have allocated ports");
    assert_eq!(allocated_ports.len(), 2);

    // Verify exact ports were allocated
    assert_eq!(
        allocated_ports.get("web").unwrap().value(),
        5010,
        "Should allocate exact preferred port"
    );
    assert_eq!(
        allocated_ports.get("api").unwrap().value(),
        5020,
        "Should allocate exact preferred port"
    );

    // Note: We can't directly test base_port from ExecutionResult,
    // but we can verify the behavior is correct by checking the allocations
}

/// Test that reservations include project and task fields.
///
/// This test verifies that sticky fields (project, task) from the config
/// and options are correctly propagated to all reservations:
/// - Project field from config is applied
/// - Task field from options is applied
/// - All services in group share same sticky fields
///
/// Invariants tested:
/// - Sticky field consistency across group
/// - Correct propagation from config and options
#[test]
fn test_group_reservation_with_project_and_task() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(2));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path).with_task(Some("development".to_string()));
    let planner = ReserveGroupPlan::new(options).expect("Failed to create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Failed to build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Failed to execute plan");

    assert!(result.success);

    // Verify all reservations have correct sticky fields
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Failed to list reservations");
    for reservation in &all_reservations {
        assert_eq!(
            reservation.project(),
            Some("test-project"),
            "All reservations should have project from config"
        );
        assert_eq!(
            reservation.task(),
            Some("development"),
            "All reservations should have task from options"
        );
    }
}

// ============================================================================
// Database State Verification Tests
// ============================================================================

/// Test that database state is correct after group operations.
///
/// This test thoroughly verifies database consistency after group allocation:
/// - All reservations exist with correct keys
/// - Path associations are correct (config parent directory)
/// - Project and task associations are correct
/// - Port occupancy is correctly recorded
///
/// Invariants tested:
/// - One reservation per service
/// - Correct ReservationKey structure (path + tag)
/// - Port occupancy prevents re-allocation
#[test]
fn test_database_state_after_group_reservation() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(2));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path).with_task(Some("test-task".to_string()));
    let planner = ReserveGroupPlan::new(options).expect("Failed to create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Failed to build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Failed to execute plan");

    assert!(result.success);

    let allocated_ports = result.allocated_ports.expect("Should have allocated ports");

    // Verify we can retrieve each reservation by its key
    for (tag, port) in &allocated_ports {
        let key = ReservationKey::new(temp_dir.path().to_path_buf(), Some(tag.clone()))
            .expect("Should create valid key");

        let reservation = Database::get_reservation(db.connection(), &key)
            .expect("Database query should succeed")
            .expect("Reservation should exist");

        assert_eq!(reservation.key(), &key, "Key should match");
        assert_eq!(reservation.port(), *port, "Port should match");
        assert_eq!(reservation.project(), Some("test-project"));
        assert_eq!(reservation.task(), Some("test-task"));
    }

    // Verify port occupancy - allocated ports should be marked as reserved
    for port in allocated_ports.values() {
        assert!(
            Database::is_port_reserved(db.connection(), *port)
                .expect("Port occupancy check should succeed"),
            "Allocated port {port} should be reserved in database"
        );
    }
}

/// Test that ports allocated by group operations are not re-allocated.
///
/// This test verifies port occupancy tracking prevents conflicts:
/// - First group allocation succeeds
/// - Second group allocation skips occupied ports
/// - Both groups coexist without port conflicts
///
/// Invariants tested:
/// - Port uniqueness across allocations
/// - Occupancy checking during pattern matching
/// - Database prevents double-allocation
#[test]
fn test_allocated_ports_not_reused() {
    let temp_dir1 = create_temp_dir();
    let temp_dir2 = create_temp_dir();
    let config1_path = create_config_file(temp_dir1.path(), "trop.yaml", &simple_offset_config(2));
    let config2_path = create_config_file(temp_dir2.path(), "trop.yaml", &simple_offset_config(2));
    let mut db = create_test_database();

    // Allocate first group
    let options1 = ReserveGroupOptions::new(config1_path);
    let planner1 = ReserveGroupPlan::new(options1).expect("Failed to create plan");
    let plan1 = planner1
        .build_plan(db.connection())
        .expect("Failed to build plan");
    let mut executor1 = PlanExecutor::new(db.connection());
    let result1 = executor1.execute(&plan1).expect("Failed to execute plan");
    let ports1 = result1.allocated_ports.expect("Should have ports");

    // Allocate second group
    let options2 = ReserveGroupOptions::new(config2_path);
    let planner2 = ReserveGroupPlan::new(options2).expect("Failed to create plan");
    let plan2 = planner2
        .build_plan(db.connection())
        .expect("Failed to build plan");
    let mut executor2 = PlanExecutor::new(db.connection());
    let result2 = executor2.execute(&plan2).expect("Failed to execute plan");
    let ports2 = result2.allocated_ports.expect("Should have ports");

    // Verify no port overlap between the two groups
    for port1 in ports1.values() {
        for port2 in ports2.values() {
            assert_ne!(port1, port2, "Ports should not overlap between groups");
        }
    }

    // Verify both groups are in database
    let all_reservations = Database::list_all_reservations(db.connection()).expect("Should list");
    assert_eq!(
        all_reservations.len(),
        4,
        "Should have 4 total reservations (2 per group)"
    );
}

// ============================================================================
// Config Discovery Tests (Autoreserve)
// ============================================================================

/// Test autoreserve discovers config from current directory.
///
/// This test verifies basic config discovery:
/// - Config file in current directory is found
/// - Autoreserve successfully uses discovered config
/// - Same behavior as explicit reserve-group
///
/// Invariants tested:
/// - Discovery finds trop.yaml in start directory
/// - Discovered config is used correctly
#[test]
fn test_autoreserve_discovers_config_in_current_dir() {
    let temp_dir = create_temp_dir();
    create_config_file(temp_dir.path(), "trop.yaml", &minimal_config());
    let mut db = create_test_database();

    let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
    let planner = AutoreservePlan::new(options).expect("Should discover config");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);
    assert!(result.allocated_ports.is_some(), "Should allocate ports");
}

/// Test autoreserve discovers config from parent directory.
///
/// This test verifies upward directory traversal during discovery:
/// - Config in parent directory is found when not in current dir
/// - Discovery stops at first config found
/// - Base path uses config file's parent directory
///
/// Invariants tested:
/// - Upward traversal works correctly
/// - First found config is used (no further traversal)
/// - Path resolution relative to config location
#[test]
fn test_autoreserve_discovers_config_from_parent() {
    let temp_dir = create_temp_dir();
    let child_dir = temp_dir.path().join("child");
    fs::create_dir(&child_dir).expect("Should create child dir");

    // Put config in parent, start discovery from child
    create_config_file(temp_dir.path(), "trop.yaml", &minimal_config());
    let mut db = create_test_database();

    let options = AutoreserveOptions::new(child_dir.clone());
    let planner = AutoreservePlan::new(options).expect("Should discover config");

    // Verify discovered config path is in parent directory
    assert!(
        planner
            .discovered_config_path()
            .starts_with(temp_dir.path()),
        "Discovered config should be in parent directory"
    );

    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);

    // Verify reservations use config's parent directory as base path
    let all_reservations = Database::list_all_reservations(db.connection()).expect("Should list");
    for reservation in &all_reservations {
        assert_eq!(
            reservation.key().path,
            temp_dir.path(),
            "Reservation path should be config parent directory, not discovery start dir"
        );
    }
}

/// Test autoreserve prefers trop.local.yaml over trop.yaml.
///
/// This test verifies config precedence rules:
/// - When both trop.yaml and trop.local.yaml exist, local is preferred
/// - This allows per-developer overrides
///
/// Invariants tested:
/// - Precedence: trop.local.yaml > trop.yaml
/// - Discovery honors precedence rules from ConfigLoader
#[test]
fn test_autoreserve_prefers_local_config() {
    let temp_dir = create_temp_dir();

    // Create both config files with different projects to distinguish them
    let global_config = r#"
project: global-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
"#;

    let local_config = r#"
project: local-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
"#;

    create_config_file(temp_dir.path(), "trop.yaml", global_config);
    create_config_file(temp_dir.path(), "trop.local.yaml", local_config);

    let mut db = create_test_database();
    let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
    let planner = AutoreservePlan::new(options).expect("Should discover config");

    // Verify local config was discovered
    assert!(
        planner
            .discovered_config_path()
            .ends_with("trop.local.yaml"),
        "Should prefer trop.local.yaml over trop.yaml"
    );

    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);

    // Verify reservation uses local config's project
    let all_reservations = Database::list_all_reservations(db.connection()).expect("Should list");
    assert_eq!(all_reservations.len(), 1);
    assert_eq!(
        all_reservations[0].project(),
        Some("local-project"),
        "Should use local config, not global"
    );
}

/// Test autoreserve fails when no config found.
///
/// This test verifies error handling for missing config:
/// - Discovery from directory without config fails
/// - Error message is informative
///
/// Invariants tested:
/// - Discovery fails gracefully when no config exists
/// - Error indicates where discovery was attempted
#[test]
fn test_autoreserve_no_config_found() {
    let temp_dir = create_temp_dir();
    // Don't create any config file

    let options = AutoreserveOptions::new(temp_dir.path().to_path_buf());
    let result = AutoreservePlan::new(options);

    assert!(result.is_err(), "Should fail when no config found");

    match result {
        Err(err) => {
            let err_msg = err.to_string();
            assert!(
                err_msg.contains("No trop configuration file found"),
                "Error should indicate no config found: {err_msg}"
            );
        }
        Ok(_) => panic!("Expected error but got success"),
    }
}

// ============================================================================
// Override Flags Tests
// ============================================================================

/// Test allow_unrelated_path flag behavior.
///
/// This test verifies the allow_unrelated_path flag:
/// - When false (default), operations on unrelated paths may be restricted
/// - When true, operations on any path are allowed
///
/// Note: The current implementation doesn't enforce path relatedness for
/// group operations, so this test documents the expected behavior.
///
/// Invariants tested:
/// - Flag is passed through correctly
/// - No unintended side effects from flag
#[test]
fn test_allow_unrelated_path_flag() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(1));
    let mut db = create_test_database();

    // Test with flag enabled
    let options = ReserveGroupOptions::new(config_path).with_allow_unrelated_path(true);
    let planner = ReserveGroupPlan::new(options).expect("Should create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success, "Should succeed with flag enabled");
}

/// Test allow_project_change flag behavior.
///
/// This test verifies the allow_project_change flag:
/// - Flag controls whether project field can be changed on existing reservation
/// - This is a sticky field protection mechanism
///
/// Note: Testing actual enforcement requires existing reservations with
/// different projects, which is a complex scenario.
///
/// Invariants tested:
/// - Flag is passed through options correctly
#[test]
fn test_allow_project_change_flag() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(1));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path).with_allow_project_change(true);
    let planner = ReserveGroupPlan::new(options).expect("Should create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);
}

/// Test allow_task_change flag behavior.
///
/// This test verifies the allow_task_change flag:
/// - Flag controls whether task field can be changed on existing reservation
/// - This is a sticky field protection mechanism
///
/// Invariants tested:
/// - Flag is passed through options correctly
#[test]
fn test_allow_task_change_flag() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(1));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path)
        .with_task(Some("task1".to_string()))
        .with_allow_task_change(true);
    let planner = ReserveGroupPlan::new(options).expect("Should create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);
}

/// Test force flag overrides all protections.
///
/// This test verifies the force flag:
/// - Force flag should override all safety checks
/// - Allows operations that would normally be restricted
///
/// Invariants tested:
/// - Force flag is passed through correctly
/// - Operations succeed with force flag
#[test]
fn test_force_flag_behavior() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(1));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path).with_force(true);
    let planner = ReserveGroupPlan::new(options).expect("Should create plan");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success, "Should succeed with force flag");
}

// ============================================================================
// Error Handling Tests
// ============================================================================

/// Test error when config has no reservations section.
///
/// This test verifies validation of config structure:
/// - Config without reservations section is rejected
/// - Error message is clear and actionable
///
/// Invariants tested:
/// - Required config sections are validated
/// - Planning fails before execution
#[test]
fn test_error_config_without_reservations() {
    let temp_dir = create_temp_dir();
    let config_content = r#"
project: test-project
ports:
  min: 5000
  max: 7000
"#;
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);
    let db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let result = planner.build_plan(db.connection());

    assert!(result.is_err(), "Should fail without reservations");
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("does not contain a reservation group"),
        "Error should mention missing reservations: {err_msg}"
    );
}

/// Test error when reservation group has empty services.
///
/// This test verifies validation of reservation group structure:
/// - Empty services map is rejected
/// - Error occurs during planning
///
/// Invariants tested:
/// - Group must have at least one service
/// - Validation happens before database operations
#[test]
fn test_error_empty_services() {
    let temp_dir = create_temp_dir();
    let config_content = r#"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services: {}
"#;
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);
    let db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let result = planner.build_plan(db.connection());

    assert!(result.is_err(), "Should fail with empty services");
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("at least one service"),
        "Error should mention empty services: {err_msg}"
    );
}

/// Test error when service has neither offset nor preferred port.
///
/// This test verifies service definition validation:
/// - Each service must specify allocation strategy
/// - Error identifies the problematic service
///
/// Invariants tested:
/// - Service validation occurs during planning
/// - Error message includes service tag
#[test]
fn test_error_service_without_allocation_strategy() {
    let temp_dir = create_temp_dir();
    let config_content = r#"
project: test-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web: {}
"#;
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);
    let db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let result = planner.build_plan(db.connection());

    assert!(
        result.is_err(),
        "Should fail when service has no offset or preferred"
    );
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("offset or preferred"),
        "Error should mention missing allocation strategy: {err_msg}"
    );
}

/// Test error when preferred port is outside allowed range.
///
/// This test verifies port range validation:
/// - Preferred ports must be within configured range
/// - Error occurs during execution (allocation)
/// - No partial allocations occur
///
/// Invariants tested:
/// - Port range boundaries are enforced
/// - Validation catches out-of-range ports
#[test]
fn test_error_preferred_port_out_of_range() {
    let temp_dir = create_temp_dir();
    let config_content = r#"
project: test-project
ports:
  min: 5000
  max: 5010
reservations:
  services:
    web:
      preferred: 9000
"#;
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan);

    assert!(
        result.is_err(),
        "Should fail when preferred port is out of range"
    );

    // Verify no reservations were created (atomicity)
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Should be able to list");
    assert_eq!(
        all_reservations.len(),
        0,
        "No reservations should be created on failure"
    );
}

/// Test error when preferred port is already occupied.
///
/// This test verifies port availability checking:
/// - Preferred ports must be available
/// - Occupancy is checked before allocation
/// - Clear error when port is unavailable
///
/// Invariants tested:
/// - Port occupancy prevents conflicts
/// - Error indicates which port is unavailable
#[test]
fn test_error_preferred_port_occupied() {
    let temp_dir = create_temp_dir();
    let mut db = create_test_database();

    // First, reserve a port that we'll try to use as preferred
    let first_config = r#"
project: first-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    existing:
      offset: 0
"#;
    let first_path = create_config_file(temp_dir.path(), "first.yaml", first_config);
    let first_options = ReserveGroupOptions::new(first_path);
    let first_planner = ReserveGroupPlan::new(first_options).expect("Should create planner");
    let first_plan = first_planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut first_executor = PlanExecutor::new(db.connection());
    let first_result = first_executor
        .execute(&first_plan)
        .expect("First allocation should succeed");
    let first_ports = first_result.allocated_ports.expect("Should have ports");
    let occupied_port = first_ports.get("existing").expect("Should have port");

    // Now try to allocate a group with a preferred port that's already taken
    let second_config = format!(
        r#"
project: second-project
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      preferred: {}
"#,
        occupied_port.value()
    );
    let second_path = create_config_file(temp_dir.path(), "second.yaml", &second_config);
    let second_options = ReserveGroupOptions::new(second_path);
    let second_planner = ReserveGroupPlan::new(second_options).expect("Should create planner");
    let second_plan = second_planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut second_executor = PlanExecutor::new(db.connection());
    let result = second_executor.execute(&second_plan);

    assert!(
        result.is_err(),
        "Should fail when preferred port is occupied"
    );

    // Verify only first group's reservations exist
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Should be able to list");
    assert_eq!(
        all_reservations.len(),
        1,
        "Only first group should be allocated"
    );
}

/// Test error when no base port available for offset pattern.
///
/// This test verifies pattern matching exhaustion:
/// - When no valid base port exists, allocation fails
/// - Error indicates pattern matching failure
/// - No partial allocations occur
///
/// Invariants tested:
/// - Pattern matching handles port exhaustion
/// - Error message is informative
#[test]
fn test_error_no_base_port_for_pattern() {
    let temp_dir = create_temp_dir();
    let mut db = create_test_database();

    // Create a very small range
    let config_content = r#"
project: test-project
ports:
  min: 5100
  max: 5101
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
    db:
      offset: 2
"#;

    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);

    // First, occupy port 5100 so the pattern can't start there
    // Need a separate directory to avoid path conflicts
    let blocking_dir = create_temp_dir();
    let blocking_config = r#"
project: blocking
ports:
  min: 5100
  max: 5101
reservations:
  services:
    blocker:
      preferred: 5100
"#;
    let blocking_path = create_config_file(blocking_dir.path(), "trop.yaml", blocking_config);
    let blocking_options = ReserveGroupOptions::new(blocking_path);
    let blocking_planner = ReserveGroupPlan::new(blocking_options).expect("Should create planner");
    let blocking_plan = blocking_planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut blocking_executor = PlanExecutor::new(db.connection());
    blocking_executor
        .execute(&blocking_plan)
        .expect("Blocking allocation should succeed");

    // Now try to allocate the pattern - should fail
    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");
    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan);

    assert!(
        result.is_err(),
        "Should fail when no base port available for pattern"
    );
    let err = result.unwrap_err();
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("base port") || err_msg.contains("pattern"),
        "Error should mention pattern matching failure: {err_msg}"
    );
}

// ============================================================================
// Dry-Run Tests
// ============================================================================

/// Test dry-run mode doesn't modify database.
///
/// This test verifies dry-run behavior:
/// - Plan is validated but not executed
/// - No database modifications occur
/// - Result indicates dry-run mode
///
/// Invariants tested:
/// - Dry-run is side-effect free
/// - Database state is unchanged
/// - Result correctly indicates dry-run
#[test]
fn test_dry_run_no_database_changes() {
    let temp_dir = create_temp_dir();
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", &simple_offset_config(2));
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    // Execute in dry-run mode
    let mut executor = PlanExecutor::new(db.connection()).dry_run();
    let result = executor.execute(&plan).expect("Dry-run should succeed");

    assert!(result.success, "Dry-run should succeed");
    assert!(result.dry_run, "Result should indicate dry-run mode");

    // Verify no reservations were created
    let all_reservations =
        Database::list_all_reservations(db.connection()).expect("Should be able to list");
    assert_eq!(
        all_reservations.len(),
        0,
        "Dry-run should not create reservations"
    );
}

// ============================================================================
// Complex Scenario Tests
// ============================================================================

/// Test group allocation with large offset gaps.
///
/// This test verifies handling of sparse offset patterns:
/// - Services with large gaps between offsets
/// - Pattern matching finds suitable base port
/// - All services get correct offsets from base
///
/// Invariants tested:
/// - Pattern matching works with sparse patterns
/// - Offset arithmetic is correct for large values
/// - No overflow in port calculations
#[test]
fn test_large_offset_gaps() {
    let temp_dir = create_temp_dir();
    let config_content = r#"
project: test-project
ports:
  min: 5000
  max: 15000
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 100
    admin:
      offset: 1000
"#;
    let config_path = create_config_file(temp_dir.path(), "trop.yaml", config_content);
    let mut db = create_test_database();

    let options = ReserveGroupOptions::new(config_path);
    let planner = ReserveGroupPlan::new(options).expect("Should create planner");
    let plan = planner
        .build_plan(db.connection())
        .expect("Should build plan");

    let mut executor = PlanExecutor::new(db.connection());
    let result = executor.execute(&plan).expect("Should execute");

    assert!(result.success);

    let allocated_ports = result.allocated_ports.expect("Should have ports");
    let web_port = allocated_ports.get("web").expect("Should have web");
    let api_port = allocated_ports.get("api").expect("Should have api");
    let admin_port = allocated_ports.get("admin").expect("Should have admin");

    // Verify offset relationships
    assert_eq!(
        api_port.value(),
        web_port.value() + 100,
        "API should be base + 100"
    );
    assert_eq!(
        admin_port.value(),
        web_port.value() + 1000,
        "Admin should be base + 1000"
    );
}

/// Test multiple sequential group allocations.
///
/// This test verifies that multiple group operations work correctly:
/// - Each group gets distinct ports
/// - Pattern matching skips already-allocated ports
/// - Database maintains all reservations
///
/// Invariants tested:
/// - No port conflicts across groups
/// - Each group is independently valid
/// - Database handles multiple groups
#[test]
fn test_multiple_sequential_group_allocations() {
    let mut db = create_test_database();
    let mut temp_dirs = Vec::new();

    // Allocate three different groups sequentially, each in its own directory
    for i in 0..3 {
        let temp_dir = create_temp_dir();
        let config_content = format!(
            r"
project: project-{i}
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
    api:
      offset: 1
"
        );
        let config_path = create_config_file(temp_dir.path(), "trop.yaml", &config_content);

        let options = ReserveGroupOptions::new(config_path);
        let planner = ReserveGroupPlan::new(options).expect("Should create planner");
        let plan = planner
            .build_plan(db.connection())
            .expect("Should build plan");

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).expect("Should execute");

        assert!(result.success, "Group {i} should succeed");

        // Keep temp_dir alive
        temp_dirs.push(temp_dir);
    }

    // Verify all reservations exist
    let all_reservations = Database::list_all_reservations(db.connection()).expect("Should list");
    assert_eq!(
        all_reservations.len(),
        6,
        "Should have 6 total reservations (3 groups × 2 services)"
    );

    // Verify all ports are unique
    let mut all_ports = std::collections::HashSet::new();
    for reservation in &all_reservations {
        let port = reservation.port();
        assert!(
            all_ports.insert(port),
            "Port {port} should only be allocated once"
        );
    }
}
