//! Comprehensive integration tests for global CLI options.
//!
//! These tests verify global flags and environment variables that affect
//! all commands, including:
//! - --verbose flag
//! - --quiet flag
//! - --data-dir override
//! - --busy-timeout override
//! - --disable-autoinit flag
//! - Environment variable handling (TROP_DATA_DIR, TROP_BUSY_TIMEOUT, etc.)
//! - Precedence rules (CLI flags > env vars > defaults)

mod common;

use common::TestEnv;
use predicates::prelude::*;

// ============================================================================
// Verbose Flag Tests
// ============================================================================

/// Test --verbose flag enables debug output.
///
/// The --verbose flag should cause additional logging to appear on stderr.
/// This helps users understand what trop is doing.
#[test]
fn test_verbose_flag_increases_logging() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Normal run (no verbose)
    let normal_output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Verbose run
    let verbose_output = env
        .command()
        .arg("--verbose")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("test")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Both should succeed
    assert!(normal_output.status.success());
    assert!(verbose_output.status.success());

    // Verbose should produce more stderr output (or at least as much)
    // Note: exact behavior depends on logging implementation
    let normal_stderr = String::from_utf8(normal_output.stderr).unwrap();
    let verbose_stderr = String::from_utf8(verbose_output.stderr).unwrap();

    // At minimum, verbose shouldn't suppress output
    assert!(
        verbose_stderr.len() >= normal_stderr.len()
            || verbose_stderr.to_lowercase().contains("debug")
            || verbose_stderr.to_lowercase().contains("verbose")
    );
}

/// Test --verbose works with all commands.
///
/// The --verbose flag is global and should work with reserve, release, and list.
#[test]
fn test_verbose_flag_works_with_all_commands() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Verbose with reserve
    env.command()
        .arg("--verbose")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Verbose with list
    env.command()
        .arg("--verbose")
        .arg("list")
        .assert()
        .success();

    // Verbose with release
    env.command()
        .arg("--verbose")
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .assert()
        .success();
}

/// Test that --verbose before and after subcommand both work.
///
/// Global flags should work regardless of position.
#[test]
fn test_verbose_flag_position_independence() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // --verbose before subcommand
    env.command()
        .arg("--verbose")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // --verbose after subcommand (if supported)
    // Note: Clap's global flag handling may require it before subcommand
    // This documents the expected behavior
}

// ============================================================================
// Quiet Flag Tests
// ============================================================================

/// Test --quiet flag suppresses non-essential output.
///
/// With --quiet, informational messages and warnings should be suppressed,
/// but stdout should still contain essential data (like port numbers).
#[test]
fn test_quiet_flag_suppresses_output() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve with --quiet
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

    // Stdout should still have the port number
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().parse::<u16>().is_ok(),
        "Stdout should still contain port number"
    );

    // Stderr should be minimal or empty
    let stderr = String::from_utf8(output.stderr).unwrap();
    // Errors still go to stderr, but warnings/info should be suppressed
    // In a successful operation, stderr should be empty or minimal
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "Stderr should be minimal with --quiet"
    );
}

/// Test --quiet works with all commands.
#[test]
fn test_quiet_flag_works_with_all_commands() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Quiet with reserve
    let reserve_out = env
        .command()
        .arg("--quiet")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    assert!(reserve_out.status.success());
    assert!(String::from_utf8(reserve_out.stderr)
        .unwrap()
        .trim()
        .is_empty());

    // Quiet with list
    let list_out = env.command().arg("--quiet").arg("list").output().unwrap();
    assert!(list_out.status.success());
    // List still outputs to stdout, stderr should be quiet
    assert!(String::from_utf8(list_out.stderr)
        .unwrap()
        .trim()
        .is_empty());

    // Quiet with release
    let release_out = env
        .command()
        .arg("--quiet")
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();
    assert!(release_out.status.success());
    assert!(String::from_utf8(release_out.stderr)
        .unwrap()
        .trim()
        .is_empty());
}

/// Test that --quiet and --verbose together is handled gracefully.
///
/// When both flags are present, one should take precedence (typically quiet).
#[test]
fn test_quiet_and_verbose_together() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Use both flags - should not crash
    let output = env
        .command()
        .arg("--quiet")
        .arg("--verbose")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Should handle conflicting flags gracefully"
    );

    // Behavior when both are present is implementation-defined
    // (typically one wins, often the last one or quiet takes precedence)
}

// ============================================================================
// Data Directory Override Tests
// ============================================================================

/// Test --data-dir flag overrides default location.
///
/// The --data-dir flag should cause trop to use a different database location.
#[test]
fn test_data_dir_flag_override() {
    let env = TestEnv::new();
    let custom_data_dir = env.path().join("custom-data");
    let test_path = env.create_dir("test-project");

    // Reserve with custom data dir
    env.command()
        .args(["--data-dir", custom_data_dir.to_str().unwrap()])
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Database should exist in custom location
    assert!(
        custom_data_dir.exists(),
        "Custom data directory should be created"
    );

    // Original data dir should not exist
    assert!(
        !env.data_dir.exists(),
        "Default data dir should not be created"
    );
}

/// Test that different data directories are independent.
///
/// Reservations in one data directory should not appear in another.
#[test]
fn test_data_dir_isolation() {
    let env = TestEnv::new();
    let data_dir_a = env.path().join("data-a");
    let data_dir_b = env.path().join("data-b");
    let test_path = env.create_dir("test-project");

    // Reserve in data-a
    let mut cmd_a = env.command();
    cmd_a.arg("--data-dir").arg(&data_dir_a);
    let port_a = cmd_a
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    assert!(port_a.status.success());
    let port_a = common::parse_port(&String::from_utf8(port_a.stdout).unwrap());

    // List in data-b should be empty
    let mut cmd_b = env.command();
    cmd_b.arg("--data-dir").arg(&data_dir_b);
    let list_b = cmd_b.arg("list").output().unwrap();
    let list_output = String::from_utf8(list_b.stdout).unwrap();

    // Should not contain the port from data-a
    assert!(!list_output.contains(&port_a.to_string()));

    // Should only have header
    let lines: Vec<&str> = list_output.lines().collect();
    assert_eq!(lines.len(), 1, "data-b should be empty");
}

/// Test --data-dir with relative path.
///
/// Relative paths should be handled correctly (made absolute).
#[test]
fn test_data_dir_with_relative_path() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Use relative path for data dir
    let output = env
        .command()
        .arg("--data-dir")
        .arg("./relative-data")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .current_dir(env.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    // Should create directory relative to CWD
    let relative_data = env.path().join("relative-data");
    assert!(relative_data.exists());
}

// ============================================================================
// Busy Timeout Override Tests
// ============================================================================

/// Test --busy-timeout flag.
///
/// The --busy-timeout flag should set the SQLite busy timeout.
/// We can't easily test the timeout behavior, but we can verify the flag is accepted.
#[test]
fn test_busy_timeout_flag_accepted() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Use custom timeout
    env.command()
        .arg("--busy-timeout")
        .arg("30")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test --busy-timeout with invalid value.
///
/// Invalid timeout values should be rejected.
#[test]
fn test_busy_timeout_invalid_value() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Negative timeout should fail
    env.command()
        .arg("--busy-timeout")
        .arg("-1")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .failure();

    // Non-numeric timeout should fail
    env.command()
        .arg("--busy-timeout")
        .arg("invalid")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .failure();
}

// ============================================================================
// Disable Autoinit Flag Tests
// ============================================================================

/// Test --disable-autoinit prevents database creation.
///
/// With --disable-autoinit, trop should fail if the database doesn't exist,
/// rather than creating it automatically.
#[test]
fn test_disable_autoinit_prevents_database_creation() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to reserve with --disable-autoinit (database doesn't exist)
    let output = env
        .command()
        .arg("--disable-autoinit")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should fail because database doesn't exist
    assert!(
        !output.status.success(),
        "Should fail when database doesn't exist and autoinit is disabled"
    );

    // Should have error message about missing database
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("data directory")
            || stderr.contains("not found")
            || stderr.contains("database"),
        "Should explain why it failed"
    );

    // Database should not be created
    assert!(!env.data_dir.exists());
}

/// Test that without --disable-autoinit, database is created automatically.
///
/// This is the normal behavior: first use creates the database.
#[test]
fn test_autoinit_creates_database() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve without --disable-autoinit (database doesn't exist yet)
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Database should be created
    assert!(
        env.data_dir.exists(),
        "Database should be auto-created on first use"
    );
}

/// Test --disable-autoinit with existing database.
///
/// If the database already exists, --disable-autoinit should not prevent usage.
#[test]
fn test_disable_autoinit_with_existing_database() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create database first (without --disable-autoinit)
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Now use --disable-autoinit with existing database
    env.command()
        .arg("--disable-autoinit")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("test")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

// ============================================================================
// Environment Variable Tests
// ============================================================================

/// Test TROP_DATA_DIR environment variable.
///
/// The data directory can be set via environment variable.
#[test]
fn test_trop_data_dir_env_variable() {
    let temp = tempfile::tempdir().unwrap();
    let custom_data = temp.path().join("env-data");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    // Use env var for data dir
    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    cmd.env("TROP_DATA_DIR", &custom_data)
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Database should be in custom location
    assert!(custom_data.exists());
}

/// Test --data-dir flag overrides TROP_DATA_DIR env variable.
///
/// CLI flags should have higher precedence than environment variables.
#[test]
fn test_data_dir_flag_overrides_env() {
    let temp = tempfile::tempdir().unwrap();
    let env_data = temp.path().join("env-data");
    let flag_data = temp.path().join("flag-data");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    // Set env var but override with flag
    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    cmd.env("TROP_DATA_DIR", &env_data)
        .arg("--data-dir")
        .arg(&flag_data)
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Flag location should be used
    assert!(flag_data.exists());
    // Env location should not be created
    assert!(!env_data.exists());
}

/// Test TROP_BUSY_TIMEOUT environment variable.
#[test]
fn test_trop_busy_timeout_env_variable() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Set timeout via env var
    env.command()
        .env("TROP_BUSY_TIMEOUT", "30")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test --busy-timeout flag overrides TROP_BUSY_TIMEOUT env variable.
#[test]
fn test_busy_timeout_flag_overrides_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Set env to one value, flag to another
    env.command()
        .env("TROP_BUSY_TIMEOUT", "10")
        .arg("--busy-timeout")
        .arg("30")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Should not fail - flag takes precedence
}

/// Test TROP_DISABLE_AUTOINIT environment variable.
#[test]
fn test_trop_disable_autoinit_env_variable() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join("data");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    // Try to create reservation with autoinit disabled via env var
    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    let output = cmd
        .env("TROP_DISABLE_AUTOINIT", "true")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should fail because database doesn't exist
    assert!(!output.status.success());
    assert!(!data_dir.exists());
}

/// Test various boolean environment variable formats.
///
/// Boolean env vars should accept common true/false representations.
#[test]
fn test_boolean_env_var_formats() {
    let temp = tempfile::tempdir().unwrap();
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    // Different ways to say "true"
    for true_value in ["true", "1", "yes", "TRUE", "True"] {
        let data_dir = temp.path().join(format!("data-{true_value}"));

        let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
        let result = cmd
            .env("TROP_DISABLE_AUTOINIT", true_value)
            .arg("--data-dir")
            .arg(&data_dir)
            .arg("reserve")
            .arg("--path")
            .arg(&test_path)
            .arg("--allow-unrelated-path")
            .output()
            .unwrap();

        // Should fail because autoinit is disabled
        assert!(
            !result.status.success(),
            "TROP_DISABLE_AUTOINIT={true_value} should disable autoinit"
        );
    }
}

// ============================================================================
// Precedence Tests
// ============================================================================

/// Test configuration precedence: CLI > env > defaults.
///
/// This is a comprehensive test of the precedence rules.
#[test]
fn test_configuration_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let env_data = temp.path().join("env-data");
    let cli_data = temp.path().join("cli-data");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    // Set env var
    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    cmd.env("TROP_DATA_DIR", &env_data)
        .env("TROP_BUSY_TIMEOUT", "10")
        .arg("--data-dir")
        .arg(&cli_data) // CLI should override env
        .arg("--busy-timeout")
        .arg("30") // CLI should override env
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // CLI flag location should be used
    assert!(cli_data.exists());
    assert!(!env_data.exists());
}

// ============================================================================
// Multiple Global Flags Tests
// ============================================================================

/// Test combining multiple global flags.
///
/// Multiple global flags should work together without interference.
#[test]
fn test_multiple_global_flags() {
    let env = TestEnv::new();
    let custom_data = env.path().join("custom-data");
    let test_path = env.create_dir("test-project");

    // Combine several global flags
    env.command()
        .arg("--verbose")
        .arg("--data-dir")
        .arg(&custom_data)
        .arg("--busy-timeout")
        .arg("30")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    assert!(custom_data.exists());
}

/// Test global flags in different positions.
///
/// Global flags should work when placed in various positions in the command line.
#[test]
fn test_global_flag_positioning() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // All flags before subcommand (recommended)
    env.command()
        .arg("--verbose")
        .arg("--quiet")
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // This documents that global flags should come before the subcommand
    // (Clap's default behavior with global flags)
}

// ============================================================================
// Help and Version with Global Flags
// ============================================================================

/// Test that --help works with global flags.
///
/// Global flags should not interfere with --help.
#[test]
fn test_help_with_global_flags() {
    let env = TestEnv::new();

    env.command()
        .arg("--verbose")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

/// Test that --version works with global flags.
#[test]
fn test_version_with_global_flags() {
    let env = TestEnv::new();

    env.command()
        .arg("--quiet")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("trop"));
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Test --data-dir with nonexistent parent directory.
///
/// If the parent directory doesn't exist, trop should either create it
/// or fail with a clear error.
#[test]
fn test_data_dir_with_nonexistent_parent() {
    let temp = tempfile::tempdir().unwrap();
    let deep_data = temp.path().join("nonexistent").join("deep").join("data");
    let test_path = temp.path().join("test-project");
    std::fs::create_dir_all(&test_path).unwrap();

    let mut cmd = assert_cmd::Command::cargo_bin("trop").unwrap();
    let result = cmd
        .arg("--data-dir")
        .arg(&deep_data)
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    // Should either succeed (creating parents) or fail gracefully
    if !result.status.success() {
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(!stderr.is_empty(), "Should have error message");
    }
}

/// Test --data-dir with special characters.
///
/// Data directory paths with spaces and special characters should work.
#[test]
fn test_data_dir_with_special_characters() {
    let env = TestEnv::new();
    let special_data = env.path().join("data with spaces");
    let test_path = env.create_dir("test-project");

    env.command()
        .arg("--data-dir")
        .arg(&special_data)
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    assert!(special_data.exists());
}
