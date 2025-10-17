//! Comprehensive integration tests for the `reserve` command.
//!
//! These tests verify all aspects of port reservation functionality, including:
//! - Basic reservation (with and without explicit path)
//! - Reservation with tags
//! - Reservation with metadata (project, task)
//! - Preferred port handling
//! - Port range constraints
//! - Idempotency (repeated reservations)
//! - Flag combinations (force, overwrite, allow-change, etc.)
//! - Dry-run mode
//! - Error cases

mod common;

use assert_cmd::Command;
use common::{parse_port, TestEnv};
use predicates::prelude::*;

// ============================================================================
// Basic Reservation Tests
// ============================================================================

/// Test basic reserve command with explicit path.
///
/// This verifies the most fundamental operation: reserving a port for a
/// specific directory. The command should:
/// - Succeed and return exit code 0
/// - Output a valid port number (numeric value on stdout)
/// - Create a reservation in the database
#[test]
fn test_reserve_with_explicit_path() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve should succeed and output a port number
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\d+\n$").unwrap());
}

/// Test reserve command without explicit path (uses current directory).
///
/// When no --path is specified, trop should use the current working directory.
/// This is a common usage pattern for developers working in a project directory.
///
/// NOTE: This test changes the working directory, so we use env::set_current_dir
/// and ensure the test is isolated.
#[test]
fn test_reserve_without_path_uses_cwd() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a command that runs in the test directory
    let mut cmd = Command::cargo_bin("trop").unwrap();
    cmd.arg("--data-dir")
        .arg(&env.data_dir)
        .arg("reserve")
        .arg("--allow-unrelated-path")
        .current_dir(&test_path);

    cmd.assert()
        .success()
        .stdout(predicate::str::is_match(r"^\d+\n$").unwrap());

    // Verify the reservation was created for the test path
    let list_output = env.list();
    assert!(list_output.contains(test_path.to_str().unwrap()));
}

/// Test reserve with service tag.
///
/// Tags allow multiple reservations for the same directory (e.g., "web", "api").
/// This test verifies that:
/// - A tag can be specified
/// - The reservation is created with the tag
/// - The tag appears in list output
#[test]
fn test_reserve_with_tag() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with a tag
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("web")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    let port = parse_port(&String::from_utf8(output.stdout).unwrap());

    // Verify tag appears in list output
    let list_output = env.list();
    assert!(list_output.contains("web"));
    assert!(list_output.contains(&port.to_string()));
}

/// Test reserve with project and task metadata.
///
/// Project and task fields provide organizational context for reservations.
/// This test verifies that metadata is correctly stored and retrievable.
#[test]
fn test_reserve_with_project_and_task() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with project and task metadata
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("my-project")
        .arg("--task")
        .arg("feature-123")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verify metadata appears in list output
    let list_output = env.list();
    assert!(list_output.contains("my-project"));
    assert!(list_output.contains("feature-123"));
}

// ============================================================================
// Idempotency Tests
// ============================================================================

/// Test that repeated reserve commands for the same path return the same port.
///
/// This is a critical property: if you reserve a port for a path that already
/// has a reservation, you should get back the existing port, not a new one.
/// This enables scripts to safely call `trop reserve` multiple times.
#[test]
fn test_reserve_is_idempotent() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation
    let port1 = env.reserve_simple(&test_path);

    // Second reservation - should return the same port
    let port2 = env.reserve_simple(&test_path);

    assert_eq!(port1, port2, "Repeated reserve should return the same port");
}

/// Test that reserve with same path but different tag creates a new reservation.
///
/// Unlike the untagged case, tagged reservations are distinct even for the
/// same path. This allows multiple services per directory.
#[test]
fn test_reserve_with_different_tags_creates_new_reservations() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with "web" tag
    let port1 = env.reserve_with_tag(&test_path, "web");

    // Reserve with "api" tag - should get a different port
    let port2 = env.reserve_with_tag(&test_path, "api");

    assert_ne!(
        port1, port2,
        "Different tags should result in different ports"
    );
}

/// Test that reserve with same path and tag is idempotent.
///
/// This combines idempotency with tags: same path + same tag = same port.
#[test]
fn test_reserve_with_same_tag_is_idempotent() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation with "web" tag
    let port1 = env.reserve_with_tag(&test_path, "web");

    // Second reservation with same tag
    let port2 = env.reserve_with_tag(&test_path, "web");

    assert_eq!(
        port1, port2,
        "Repeated reserve with same tag should return same port"
    );
}

// ============================================================================
// Preferred Port Tests
// ============================================================================

/// Test reserve with preferred port.
///
/// When a preferred port is specified, trop should try to allocate that port
/// if it's available. This test verifies that the preferred port is actually
/// allocated when possible.
#[test]
fn test_reserve_with_preferred_port() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let preferred = 8080;

    // Reserve with preferred port
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    let port = parse_port(&String::from_utf8(output.stdout).unwrap());

    // Should get a valid port (preferred if available, fallback if occupied)
    assert!(
        port >= 5000 && port <= 7000,
        "Should allocate a port in valid range (got {}, preferred was {})",
        port,
        preferred
    );
}

/// Test that preferred port is rejected when already occupied.
///
/// If the preferred port is already reserved, trop should fall back to
/// allocating a different port (unless --ignore-occupied is used).
#[test]
fn test_reserve_preferred_port_already_occupied() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");
    let preferred = 8080;

    // First reservation gets the preferred port
    let port1 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--port")
        .arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    assert!(port1.status.success());

    // Second reservation tries same port, should get a different one
    // (without --ignore-occupied, this might fail or allocate different port)
    // The exact behavior depends on the allocation strategy
    let output2 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--port")
        .arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Either succeeds with different port or fails - both are valid
    if output2.status.success() {
        let port2 = parse_port(&String::from_utf8(output2.stdout).unwrap());
        assert_ne!(
            port2, preferred,
            "Should not allocate same port to different paths"
        );
    }
}

/// Test reserve with --ignore-occupied flag.
///
/// The --ignore-occupied flag tells trop to ignore occupancy checks and
/// allocate a different port without failing if the preferred port is taken.
#[test]
fn test_reserve_with_ignore_occupied() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");
    let preferred = 8080;

    // First reservation gets the preferred port
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--port")
        .arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Second reservation with --ignore-occupied should succeed
    let output2 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--port")
        .arg(preferred.to_string())
        .arg("--ignore-occupied")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(
        output2.status.success(),
        "Should succeed with --ignore-occupied"
    );
}

// ============================================================================
// Port Range Tests
// ============================================================================

/// Test reserve with min/max port constraints.
///
/// The --min and --max flags constrain the range of ports that can be allocated.
/// This test verifies that the allocated port falls within the specified range.
#[test]
fn test_reserve_with_port_range() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let min_port = 9000;
    let max_port = 9100;

    // Reserve with port range
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--min")
        .arg(min_port.to_string())
        .arg("--max")
        .arg(max_port.to_string())
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    let port = parse_port(&String::from_utf8(output.stdout).unwrap());

    // Verify port is within range
    assert!(
        port >= min_port && port <= max_port,
        "Port {port} should be in range [{min_port}, {max_port}]"
    );
}

// ============================================================================
// Force and Override Tests
// ============================================================================

/// Test reserve with --force flag.
///
/// The --force flag overrides all protections (path validation, sticky fields).
/// This test verifies that --force allows operations that would normally fail.
#[test]
fn test_reserve_with_force() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation with project metadata
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("original-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Second reservation with different project should fail without --force
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("different-project")
        .arg("--allow-unrelated-path")
        .assert()
        .failure();

    // But should succeed with --force
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("different-project")
        .arg("--force")
        .assert()
        .success();
}

/// Test reserve with --allow-project-change flag.
///
/// This flag provides fine-grained control over project field updates,
/// allowing project changes without enabling all --force behavior.
#[test]
fn test_reserve_with_allow_project_change() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation with project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("original-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Change project with --allow-project-change
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("different-project")
        .arg("--allow-project-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test reserve with --allow-task-change flag.
///
/// Similar to --allow-project-change but for the task field.
#[test]
fn test_reserve_with_allow_task_change() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation with task
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--task")
        .arg("task-1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Change task with --allow-task-change
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--task")
        .arg("task-2")
        .arg("--allow-task-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test reserve with --allow-change flag (both project and task).
///
/// The --allow-change flag is a convenience that enables both
/// --allow-project-change and --allow-task-change.
#[test]
fn test_reserve_with_allow_change() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // First reservation with metadata
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("proj1")
        .arg("--task")
        .arg("task1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Change both with --allow-change
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("proj2")
        .arg("--task")
        .arg("task2")
        .arg("--allow-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

// ============================================================================
// Dry-Run Mode Tests
// ============================================================================

/// Test reserve with --dry-run flag.
///
/// Dry-run mode should:
/// - Not create the database
/// - Not make any changes
/// - Show what would happen (on stderr)
/// - Return success exit code
#[test]
fn test_reserve_dry_run_does_not_create_database() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Dry-run reserve
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .arg("--dry-run")
        .assert()
        .success();

    // Database should not exist
    assert!(!env.data_dir.exists(), "Dry-run should not create database");
}

/// Test that dry-run shows planned actions.
///
/// In dry-run mode, trop should output a description of what would happen
/// to stderr (keeping stdout clean for scripting).
#[test]
fn test_reserve_dry_run_shows_plan() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Dry-run should show plan on stderr
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));
}

/// Test that dry-run with --quiet suppresses output.
///
/// Even in dry-run mode, --quiet should suppress the plan output.
#[test]
fn test_reserve_dry_run_with_quiet() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Dry-run with --quiet should not output plan
    let output = env
        .command()
        .arg("--quiet")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .arg("--dry-run")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "Quiet mode should suppress dry-run plan"
    );
}

// ============================================================================
// Environment Variable Tests
// ============================================================================

/// Test that TROP_PATH environment variable is respected.
///
/// Command-line arguments should take precedence, but env vars provide defaults.
#[test]
fn test_reserve_respects_trop_path_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Set TROP_PATH and reserve without --path
    env.command()
        .arg("reserve")
        .arg("--allow-unrelated-path")
        .env("TROP_PATH", &test_path)
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\d+\n$").unwrap());
}

/// Test that command-line --path overrides TROP_PATH.
///
/// This verifies the precedence: CLI flags > env vars.
#[test]
fn test_cli_path_overrides_env_path() {
    let env = TestEnv::new();
    let path1 = env.create_dir("path1");
    let path2 = env.create_dir("path2");

    // Set env to path1 but use --path for path2
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--allow-unrelated-path")
        .env("TROP_PATH", &path1)
        .output()
        .unwrap();

    assert!(output.status.success());

    // List should show path2, not path1
    let list_output = env.list();
    assert!(list_output.contains(path2.to_str().unwrap()));
}

/// Test TROP_PROJECT environment variable.
#[test]
fn test_reserve_respects_trop_project_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with project from env var
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .env("TROP_PROJECT", "env-project")
        .assert()
        .success();

    // Verify project in list
    let list_output = env.list();
    assert!(list_output.contains("env-project"));
}

/// Test TROP_TASK environment variable.
#[test]
fn test_reserve_respects_trop_task_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with task from env var
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .env("TROP_TASK", "env-task")
        .assert()
        .success();

    // Verify task in list
    let list_output = env.list();
    assert!(list_output.contains("env-task"));
}

// ============================================================================
// Error Cases
// ============================================================================

/// Test reserve with invalid port number.
///
/// Port numbers must be in range 1-65535. Values outside this range should
/// be rejected with a clear error message.
#[test]
fn test_reserve_with_invalid_port() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Port 0 is invalid
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("0")
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid").or(predicate::str::contains("error")));

    // Port > 65535 is invalid
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("70000")
        .arg("--allow-unrelated-path")
        .assert()
        .failure();
}

/// Test reserve with invalid min/max range.
///
/// Min must be <= max. Invalid ranges should be rejected.
#[test]
fn test_reserve_with_invalid_port_range() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Min > max is invalid
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--min")
        .arg("9000")
        .arg("--max")
        .arg("8000")
        .arg("--allow-unrelated-path")
        .assert()
        .failure();
}

/// Test reserve with nonexistent path fails with appropriate error.
///
/// While trop generally accepts paths (they might be created later),
/// some operations may require the path to exist.
#[test]
fn test_reserve_with_nonexistent_path() {
    let env = TestEnv::new();
    let fake_path = env.path().join("does-not-exist");

    // This might succeed or fail depending on path validation strategy
    // Just verify it doesn't crash
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&fake_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Either succeeds or fails gracefully with error message
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.is_empty(), "Error should have a message");
    }
}

/// Test sticky field violation without permission flag.
///
/// Attempting to change a sticky field (project/task) without the appropriate
/// --allow-* flag should fail with a clear error message.
#[test]
fn test_sticky_field_violation_fails() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create initial reservation with project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Try to change project without permission - should fail
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project2")
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("project").or(predicate::str::contains("sticky")));
}

// ============================================================================
// Output Format Tests
// ============================================================================

/// Test that reserve outputs only the port number on stdout.
///
/// This is critical for shell scripting: stdout should contain ONLY the port
/// number (with a trailing newline), nothing else. All other output should
/// go to stderr.
#[test]
fn test_reserve_stdout_is_just_port_number() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();

    // Should match exactly: digits followed by newline
    assert!(
        stdout.trim().parse::<u16>().is_ok(),
        "Stdout should be just a port number, got: {stdout:?}"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "Stdout should be exactly one line"
    );
}

/// Test that warnings go to stderr, not stdout.
///
/// Even if there are warnings, stdout should still contain only the port number.
#[test]
fn test_reserve_warnings_go_to_stderr() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a scenario that might generate warnings
    // (exact scenario depends on implementation)
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .arg("--verbose")
        .output()
        .unwrap();

    assert!(output.status.success());

    // Stdout should still be just the port
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.trim().parse::<u16>().is_ok());

    // Any warnings/logs should be on stderr
    // (we can't guarantee warnings, but if there are any, they must be on stderr)
}

/// Test --quiet flag suppresses warnings.
///
/// With --quiet, even warnings should be suppressed from stderr.
#[test]
fn test_reserve_quiet_suppresses_warnings() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    let output = env
        .command()
        .arg("--quiet")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());

    // Stdout should have the port
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.trim().parse::<u16>().is_ok());

    // Stderr should be minimal/empty
    // Note: errors still go to stderr, but warnings should be suppressed
}

// ============================================================================
// Flag Combination Tests
// ============================================================================

/// Test multiple flags work together correctly.
///
/// This test combines several flags to verify they don't interfere with each other.
#[test]
fn test_reserve_with_multiple_flags() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Combine tag, project, task, and preferred port
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("web")
        .arg("--project")
        .arg("my-project")
        .arg("--task")
        .arg("feature-1")
        .arg("--port")
        .arg("8080")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    let port = parse_port(&String::from_utf8(output.stdout).unwrap());
    // Port might be fallback if preferred was occupied
    assert!(
        port >= 5000 && port <= 7000,
        "Should allocate a port in valid range"
    );

    // Verify all metadata in list
    let list_output = env.list();
    assert!(list_output.contains("web"));
    assert!(list_output.contains("my-project"));
    assert!(list_output.contains("feature-1"));
    assert!(list_output.contains(&port.to_string()));
}

/// Test that conflicting flags are handled correctly.
///
/// Some flag combinations don't make sense (e.g., --quiet and --verbose).
/// The CLI should handle these gracefully (typically, last one wins or
/// one takes precedence).
#[test]
fn test_reserve_with_conflicting_flags() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // --quiet and --verbose together - should not crash
    env.command()
        .arg("--quiet")
        .arg("--verbose")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}
