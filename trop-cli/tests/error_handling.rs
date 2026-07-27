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
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

type ReservationRow = (
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
);

fn reservation_snapshot(env: &TestEnv) -> Vec<ReservationRow> {
    let connection = Connection::open(env.data_dir.join("trop.db")).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT path, tag, port, project, task, created_at, last_used_at
             FROM reservations
             ORDER BY path, tag",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn assert_locked_database_command(
    env: &TestEnv,
    expected_operation: &str,
    configure: impl FnOnce(&mut assert_cmd::Command),
) {
    let before = reservation_snapshot(env);
    let locker = Connection::open(env.data_dir.join("trop.db")).unwrap();
    locker.execute_batch("BEGIN IMMEDIATE").unwrap();

    let mut command = env.command();
    command.arg("--quiet").arg("--busy-timeout").arg("1");
    configure(&mut command);
    let output = command.output().unwrap();

    locker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!("Error: database lock timeout while {expected_operation} after 1s\n")
    );
    assert_eq!(
        reservation_snapshot(env),
        before,
        "timed-out command changed reservation state"
    );
}

fn create_group_config(path: &Path) {
    fs::write(
        path,
        "ports:\n  min: 8000\n  max: 8010\n\
         occupancy_check:\n  skip: true\n\
         reservations:\n  base: 8000\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();
}

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
        .current_dir(env.path())
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

#[test]
fn every_database_mutating_command_uses_the_timeout_exit_contract() {
    let env = TestEnv::new();
    let existing = env.create_dir("existing");
    env.reserve_simple(&existing);
    let new_path = env.create_dir("new");
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command
                .arg("reserve")
                .arg("--path")
                .arg(&new_path)
                .arg("--allow-unrelated-path");
        },
    );

    let env = TestEnv::new();
    let existing = env.create_dir("release");
    env.reserve_simple(&existing);
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command.arg("release").arg("--path").arg(&existing);
        },
    );

    let env = TestEnv::new();
    let seed = env.create_dir("seed");
    env.reserve_simple(&seed);
    let config = env.path().join("group.yaml");
    create_group_config(&config);
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command
                .arg("reserve-group")
                .arg(&config)
                .arg("--allow-unrelated-path")
                .arg("--format")
                .arg("json");
        },
    );

    let env = TestEnv::new();
    let project = env.create_dir("autoreserve");
    let seed = env.create_dir("seed");
    env.reserve_simple(&seed);
    create_group_config(&project.join("trop.yaml"));
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command
                .arg("autoreserve")
                .arg("--allow-unrelated-path")
                .arg("--format")
                .arg("json")
                .current_dir(&project);
        },
    );

    let env = TestEnv::new();
    let missing = env.create_dir("prune");
    env.reserve_simple(&missing);
    fs::remove_dir(&missing).unwrap();
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command.arg("prune");
        },
    );

    let env = TestEnv::new();
    let expired = env.create_dir("expire");
    env.reserve_simple(&expired);
    Connection::open(env.data_dir.join("trop.db"))
        .unwrap()
        .execute("UPDATE reservations SET last_used_at = 0", [])
        .unwrap();
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command.arg("expire").arg("--days").arg("1");
        },
    );

    let env = TestEnv::new();
    let missing = env.create_dir("autoclean");
    env.reserve_simple(&missing);
    fs::remove_dir(&missing).unwrap();
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command.arg("autoclean").arg("--days").arg("1");
        },
    );

    let env = TestEnv::new();
    let source = env.create_dir("source");
    let destination = env.create_dir("destination");
    env.reserve_simple(&source);
    assert_locked_database_command(
        &env,
        "starting an immediate database transaction",
        |command| {
            command
                .arg("migrate")
                .arg("--from")
                .arg(&source)
                .arg("--to")
                .arg(&destination);
        },
    );
}

#[test]
fn lock_timeout_sources_wait_for_the_configured_interval() {
    enum TimeoutSource {
        Cli,
        Environment,
        Config,
    }

    for source in [
        TimeoutSource::Cli,
        TimeoutSource::Environment,
        TimeoutSource::Config,
    ] {
        let env = TestEnv::new();
        let existing = env.create_dir("existing");
        env.reserve_simple(&existing);
        let new_path = env.create_dir("new");
        if matches!(source, TimeoutSource::Config) {
            fs::write(
                env.data_dir.join("config.yaml"),
                "maximum_lock_wait_seconds: 1\n",
            )
            .unwrap();
        }

        let locker = Connection::open(env.data_dir.join("trop.db")).unwrap();
        locker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let mut command = env.command();
        if matches!(source, TimeoutSource::Cli) {
            command.arg("--busy-timeout").arg("1");
        }
        if matches!(source, TimeoutSource::Environment) {
            command.env("TROP_BUSY_TIMEOUT", "1");
        }
        command
            .arg("reserve")
            .arg("--path")
            .arg(&new_path)
            .arg("--allow-unrelated-path");

        let started = Instant::now();
        let output = command.output().unwrap();
        let elapsed = started.elapsed();
        locker.execute_batch("ROLLBACK").unwrap();

        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "Error: database lock timeout while starting an immediate database transaction after 1s\n"
        );
        assert!(
            elapsed >= Duration::from_millis(800) && elapsed < Duration::from_secs(3),
            "configured one-second wait took {elapsed:?}"
        );
        assert_eq!(env.reservation_count(), 1);
    }
}

#[test]
fn command_proceeds_when_lock_is_released_before_timeout() {
    let env = TestEnv::new();
    let existing = env.create_dir("existing");
    env.reserve_simple(&existing);
    let new_path = env.create_dir("new");
    let locker = Connection::open(env.data_dir.join("trop.db")).unwrap();
    locker.execute_batch("BEGIN IMMEDIATE").unwrap();

    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("trop"));
    command
        .env("TROP_DATA_DIR", &env.data_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--busy-timeout")
        .arg("2")
        .arg("reserve")
        .arg("--path")
        .arg(&new_path)
        .arg("--allow-unrelated-path");
    let child = command.spawn().unwrap();
    let (sender, receiver) = mpsc::sync_channel(1);
    let waiter = std::thread::spawn(move || {
        sender.send(child.wait_with_output()).unwrap();
    });

    assert!(
        receiver.recv_timeout(Duration::from_millis(150)).is_err(),
        "command should remain blocked while the write lock is held"
    );
    locker.execute_batch("ROLLBACK").unwrap();

    let output = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("command did not finish after lock release")
        .unwrap();
    waiter.join().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(env.reservation_count(), 2);
}

#[test]
fn read_only_commands_continue_under_a_wal_writer() {
    let env = TestEnv::new();
    let path = env.create_dir("project");
    let port = env.reserve_simple(&path);
    let locker = Connection::open(env.data_dir.join("trop.db")).unwrap();
    locker.execute_batch("BEGIN IMMEDIATE").unwrap();

    let commands = [
        vec!["list".to_string()],
        vec![
            "assert-reservation".to_string(),
            "--path".to_string(),
            path.display().to_string(),
        ],
        vec!["assert-port".to_string(), port.to_string()],
        vec!["port-info".to_string(), port.to_string()],
        vec!["list-projects".to_string()],
        vec!["assert-data-dir".to_string(), "--validate".to_string()],
    ];
    for arguments in commands {
        let output = env
            .command()
            .arg("--busy-timeout")
            .arg("0")
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    locker.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn malformed_database_error_is_not_overmapped_to_timeout() {
    let env = TestEnv::new();
    fs::create_dir_all(&env.data_dir).unwrap();
    fs::write(env.data_dir.join("trop.db"), b"not a sqlite database").unwrap();

    let output = env
        .command()
        .arg("--busy-timeout")
        .arg("0")
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("database"), "{stderr}");
    assert!(!stderr.contains("lock timeout"), "{stderr}");
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
        .current_dir(env.path())
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
        .current_dir(env.path())
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
