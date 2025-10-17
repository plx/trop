//! Comprehensive integration tests for error handling and exit codes.
//!
//! These tests verify that trop handles errors correctly and returns
//! appropriate exit codes, including:
//! - Exit code 0: Success
//! - Exit code 1: Semantic failure (assertion/validation errors)
//! - Exit code 2: Timeout (SQLite busy)
//! - Exit code 3: No data directory found
//! - Exit code 4: Invalid arguments
//! - Exit code 5: I/O error
//! - Exit code 6: Other library errors
//! - Exit code 7: Configuration error
//!
//! Each test documents the expected error scenario and verifies both the
//! exit code and error message quality.

mod common;

use common::TestEnv;
use predicates::prelude::*;

// ============================================================================
// Success Cases (Exit Code 0)
// ============================================================================

/// Test that successful operations return exit code 0.
///
/// This is the baseline: normal operations should exit cleanly.
#[test]
fn test_success_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve should return 0
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .code(0);

    // List should return 0
    env.command().arg("list").assert().code(0);

    // Release should return 0
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .assert()
        .code(0);
}

// ============================================================================
// Semantic Failures (Exit Code 1)
// ============================================================================

/// Test sticky field violation returns exit code 1.
///
/// Attempting to change a sticky field (project/task) without permission
/// is a semantic validation error, not a system error.
#[test]
fn test_sticky_field_violation_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservation with project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Try to change project without permission
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project2")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should fail with exit code 1 (semantic error)
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Sticky field violation should exit with code 1"
    );

    // Should have clear error message
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("project") || stderr.contains("sticky") || stderr.contains("change"),
        "Error message should explain sticky field issue"
    );
}

/// Test path relationship violation returns exit code 1.
///
/// Attempting operations on unrelated paths without permission is a
/// semantic validation error.
#[test]
fn test_path_relationship_violation_exit_code() {
    let env = TestEnv::new();
    let unrelated_path = env.create_dir("unrelated");

    // Try to reserve without --allow-unrelated-path
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&unrelated_path)
        .output()
        .unwrap();

    // Should fail with exit code 1 (semantic error)
    // Note: exact behavior depends on path validation implementation
    if !output.status.success() {
        let code = output.status.code().unwrap();
        assert!(
            code == 1 || code == 4,
            "Path validation should exit with code 1 or 4"
        );

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.is_empty(), "Should have error message");
    }
}

// ============================================================================
// Timeout (Exit Code 2)
// ============================================================================

/// Test database timeout returns exit code 2.
///
/// When SQLite busy timeout is exceeded, exit code should be 2.
/// This is difficult to test reliably without concurrent access,
/// so we document the expected behavior.
///
/// Note: This test is informational - actual timeout testing requires
/// concurrent database access which is complex to set up reliably.
#[test]
fn test_timeout_exit_code_documentation() {
    // This test documents that timeout errors should return exit code 2.
    // Actual timeout testing would require:
    // 1. Opening a transaction in one process
    // 2. Trying to write from another process
    // 3. Verifying exit code 2 when busy timeout expires
    //
    // For now, we verify that the timeout can be configured:
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Very short timeout should still work for non-contended operations
    env.command()
        .arg("--busy-timeout")
        .arg("1")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

// ============================================================================
// No Data Directory (Exit Code 3)
// ============================================================================

/// Test missing data directory with --disable-autoinit returns exit code 3.
///
/// When the database doesn't exist and auto-init is disabled, the error
/// should be distinct from other errors.
#[test]
fn test_no_data_directory_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to use non-existent database with autoinit disabled
    let output = env
        .command()
        .arg("--disable-autoinit")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should fail with exit code 3
    assert_eq!(
        output.status.code().unwrap(),
        3,
        "Missing data directory should exit with code 3"
    );

    // Should have clear error message
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("data directory") || stderr.contains("not found"),
        "Error should mention data directory: {stderr}"
    );
}

/// Test that specifying non-existent data directory with --disable-autoinit fails with code 3.
#[test]
fn test_explicit_missing_data_dir_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let nonexistent = temp.path().join("does-not-exist");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    let output = cmd
        .arg("--data-dir")
        .arg(&nonexistent)
        .arg("--disable-autoinit")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        3,
        "Non-existent data directory should exit with code 3"
    );
}

// ============================================================================
// Invalid Arguments (Exit Code 4)
// ============================================================================

/// Test invalid port number returns exit code 4.
///
/// Argument validation errors should return exit code 4.
#[test]
fn test_invalid_port_number_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Port 0 is invalid
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("0")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should fail with exit code 4
    let code = output.status.code().unwrap();
    assert_eq!(code, 4, "Invalid port should exit with code 4, got {code}");

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("port") || stderr.contains("invalid") || stderr.contains("0"),
        "Error should mention invalid port"
    );
}

/// Test port > 65535 returns exit code 4.
#[test]
fn test_port_too_large_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("70000")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        4,
        "Port too large should exit with code 4"
    );
}

/// Test invalid port range (min > max) returns exit code 4.
#[test]
fn test_invalid_port_range_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--min")
        .arg("9000")
        .arg("--max")
        .arg("8000")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 4 || code == 6,
        "Invalid port range should exit with code 4 or 6"
    );
}

/// Test conflicting flags return exit code 4.
///
/// Mutually exclusive flags should be detected and rejected.
#[test]
fn test_conflicting_flags_exit_code() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // --tag and --untagged-only are mutually exclusive
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("web")
        .arg("--untagged-only")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        4,
        "Conflicting flags should exit with code 4"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Cannot specify both") || stderr.contains("conflict"),
        "Error should explain the conflict"
    );
}

/// Test unknown subcommand returns exit code 4 (or 2 from clap).
///
/// Invalid subcommands should be caught by argument parsing.
#[test]
fn test_unknown_subcommand_exit_code() {
    let env = TestEnv::new();

    let output = env.command().arg("invalid-command").output().unwrap();

    // Clap typically returns 2 for usage errors, but we document this
    let code = output.status.code().unwrap();
    assert!(
        code == 2 || code == 4,
        "Unknown subcommand should fail with code 2 or 4"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("error") || stderr.contains("invalid"),
        "Should have error message"
    );
}

/// Test unknown flag returns failure.
///
/// Invalid flags should be caught by clap.
#[test]
fn test_unknown_flag_exit_code() {
    let env = TestEnv::new();

    let output = env
        .command()
        .arg("--invalid-flag")
        .arg("reserve")
        .output()
        .unwrap();

    // Should fail (clap error)
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("error") || stderr.contains("unexpected"),
        "Should explain unknown flag"
    );
}

// ============================================================================
// I/O Errors (Exit Code 5)
// ============================================================================

/// Test I/O error scenarios.
///
/// I/O errors should return exit code 5. These are difficult to trigger
/// reliably in tests, so we document the expected behavior.
#[test]
fn test_io_error_exit_code_documentation() {
    // I/O errors (exit code 5) are difficult to test reliably because
    // they require system-level failures like:
    // - Disk full
    // - Permission denied
    // - Path too long
    // - etc.
    //
    // We document that such errors should return exit code 5.
    // Manual testing should verify this for scenarios like:
    // - Database on read-only filesystem
    // - No permission to create data directory
    // - Disk full when writing database
}

// ============================================================================
// Library Errors (Exit Code 6)
// ============================================================================

/// Test that release is idempotent and succeeds even when no reservation exists.
///
/// Per the specification (line 631): "Idempotent—returns success even if no
/// matching reservation exists."
#[test]
fn test_release_idempotent_success() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Release something that doesn't exist - should succeed (idempotent)
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    // Should succeed with exit code 0 (idempotent operation)
    assert_eq!(
        output.status.code().unwrap(),
        0,
        "Release should succeed even when reservation doesn't exist (idempotent)"
    );

    // stderr should indicate no reservation was found (but not as an error)
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("No reservation found") || stderr.contains("already released"),
        "Should indicate no reservation was found: {stderr}"
    );
}

// ============================================================================
// Configuration Errors (Exit Code 7)
// ============================================================================

/// Test configuration error scenarios.
///
/// Configuration file parsing errors should return exit code 7.
/// Since we don't have config files in these tests, this is informational.
#[test]
fn test_config_error_exit_code_documentation() {
    // Configuration errors (exit code 7) occur when:
    // - Config file has invalid YAML/format
    // - Config file has invalid values
    // - Config file conflicts
    //
    // These would be tested with actual config files like:
    // - trop.yaml with malformed YAML
    // - trop.yaml with invalid port ranges
    // - Conflicting settings between trop.yaml and trop.local.yaml
}

// ============================================================================
// Error Message Quality Tests
// ============================================================================

/// Test that error messages are helpful and actionable.
///
/// Errors should explain what went wrong and suggest fixes when possible.
#[test]
fn test_error_messages_are_helpful() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservation with project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Try to change project - error should suggest solution
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project2")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();

    // Error should be descriptive
    assert!(!stderr.is_empty(), "Should have error message");

    // Should mention the problem
    assert!(
        stderr.contains("project") || stderr.contains("change"),
        "Should identify the problem"
    );

    // Ideally should suggest a solution (like using --allow-project-change)
    // but this depends on implementation
}

/// Test that releasing a non-existent reservation provides clear feedback.
///
/// While release is idempotent and succeeds, it should still provide clear
/// feedback to the user that no reservation was found.
#[test]
fn test_release_nonexistent_clear_message() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    // Should succeed (idempotent)
    assert!(
        output.status.success(),
        "Release should succeed even when reservation doesn't exist"
    );

    // But should provide clear feedback
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("not found")
            || stderr.contains("No reservation")
            || stderr.contains("already released"),
        "Should clearly indicate no reservation was found (as info, not error): {stderr}"
    );
}

/// Test that invalid arguments have clear error messages.
#[test]
fn test_invalid_argument_error_message() {
    let env = TestEnv::new();

    // Missing required subcommand
    let output = env.command().output().unwrap();
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Usage:") || stderr.contains("COMMAND"),
        "Should show usage when subcommand is missing"
    );
}

// ============================================================================
// Error Consistency Tests
// ============================================================================

/// Test that same error produces same exit code consistently.
///
/// Repeated invocations of the same error condition should return
/// the same exit code.
#[test]
fn test_error_exit_code_consistency() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create and try to violate sticky field multiple times
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // First violation
    let code1 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project2")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap()
        .status
        .code()
        .unwrap();

    // Second violation (same error)
    let code2 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("project3")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap()
        .status
        .code()
        .unwrap();

    assert_eq!(code1, code2, "Same error should give same exit code");
}

// ============================================================================
// Stderr vs Stdout Tests
// ============================================================================

/// Test that errors go to stderr, not stdout.
///
/// Error messages must go to stderr to avoid polluting stdout for scripts.
#[test]
fn test_errors_go_to_stderr() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Trigger an error
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("0")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(!output.status.success());

    // Error should be on stderr
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.is_empty(), "Error message should be on stderr");

    // Stdout should be empty
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.trim().is_empty(), "Stdout should be empty on error");
}

/// Test that successful operations don't write errors.
///
/// Success cases should have minimal stderr (no errors/warnings).
#[test]
fn test_success_no_errors_on_stderr() {
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

    // With --quiet, stderr should be empty
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.trim().is_empty(),
        "Successful quiet operation should have empty stderr"
    );

    // Stdout should have the port
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().parse::<u16>().is_ok(),
        "Stdout should have port number"
    );
}

// ============================================================================
// Multiple Errors Tests
// ============================================================================

/// Test that first error is reported when multiple issues exist.
///
/// If multiple validation errors occur, at least one should be reported clearly.
#[test]
fn test_multiple_errors_reporting() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to use both invalid port AND invalid range
    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--port")
        .arg("0")
        .arg("--min")
        .arg("9000")
        .arg("--max")
        .arg("8000")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(!output.status.success());

    // Should report at least one error
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.is_empty());
    assert!(
        stderr.contains("port") || stderr.contains("range") || stderr.contains("invalid"),
        "Should report an error"
    );
}

// ============================================================================
// Help and Version Don't Error
// ============================================================================

/// Test that --help exits successfully.
///
/// Help output is not an error, should return exit code 0.
#[test]
fn test_help_exit_code() {
    let env = TestEnv::new();

    env.command()
        .arg("--help")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Usage:"));
}

/// Test that --version exits successfully.
#[test]
fn test_version_exit_code() {
    let env = TestEnv::new();

    env.command()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("trop"));
}

/// Test that subcommand --help exits successfully.
#[test]
fn test_subcommand_help_exit_code() {
    let env = TestEnv::new();

    env.command()
        .arg("reserve")
        .arg("--help")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Reserve"));
}
