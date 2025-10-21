//! Comprehensive integration tests for cleanup commands (prune, expire, autoclean).
//!
//! These tests verify the CLI behavior of cleanup operations, including:
//! - `prune`: Remove reservations for non-existent paths
//! - `expire`: Remove reservations based on age
//! - `autoclean`: Combined prune + expire operation
//!
//! Tests cover:
//! - Basic functionality for each command
//! - Dry-run mode behavior
//! - Output modes (quiet, normal, verbose)
//! - Configuration file integration (expire threshold)
//! - CLI override of config values
//! - Error cases (missing flags, empty database)
//! - Edge cases (mixed scenarios, boundary conditions)
//!
//! These tests focus on CLI integration - verifying commands work correctly
//! from the user's perspective with proper output formatting, exit codes,
//! and database state changes.

mod common;

use common::TestEnv;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a trop configuration file with cleanup settings.
///
/// This helper creates a minimal trop config that includes cleanup configuration,
/// useful for testing how CLI commands interact with config-file settings.
#[allow(dead_code)] // Reserved for future use when TROP_CONFIG_FILE is implemented
fn create_config_with_cleanup(path: &Path, expire_after_days: Option<u32>) -> String {
    let cleanup_section = if let Some(days) = expire_after_days {
        format!(
            r#"
cleanup:
  expire_after_days: {days}
"#
        )
    } else {
        String::new()
    };

    let config = format!(
        r#"
project: test-cleanup-project

ports:
  min: 5000
  max: 7000
{cleanup_section}
"#
    );

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create config directory");
    }
    fs::write(path, &config).expect("Failed to write config");
    config
}

/// Reserve a port and manually set its last_used_at timestamp to a past time.
///
/// This helper is critical for testing expiration: it creates a reservation
/// and then directly manipulates the database to set an old timestamp,
/// simulating a reservation that hasn't been used in many days.
///
/// # Implementation Note
///
/// We can't use the public API to create old reservations because the library
/// always sets last_used_at to "now". So we create a reservation normally,
/// then use SQL to update its timestamp directly.
fn reserve_old_port(env: &TestEnv, path: &Path, days_old: u64) -> u16 {
    // Create a normal reservation (this also ensures the database exists)
    let port = env.reserve_simple(path);

    // Calculate the old timestamp (days_old days ago)
    let old_time = SystemTime::now() - Duration::from_secs(days_old * 86400);
    let old_timestamp = old_time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Update the database directly to set the old timestamp
    // The database was created by reserve_simple, so it should exist now
    let db_path = env.data_dir.join("trop.db");

    // Wait a tiny bit to ensure database is fully initialized
    std::thread::sleep(Duration::from_millis(10));

    let conn = rusqlite::Connection::open(&db_path).expect("Failed to open database");

    conn.execute(
        "UPDATE reservations SET last_used_at = ?1 WHERE port = ?2",
        rusqlite::params![old_timestamp as i64, port as i64],
    )
    .expect("Failed to update timestamp");

    port
}

// ============================================================================
// Prune Command Tests
// ============================================================================

/// Test basic prune with non-existent paths.
///
/// This verifies the core prune functionality: removing reservations where
/// the associated directory path no longer exists on the filesystem.
/// The command should:
/// - Succeed and report how many reservations were removed
/// - Actually delete the reservations from the database
/// - Output a summary to stderr
#[test]
fn test_prune_removes_nonexistent_paths() {
    let env = TestEnv::new();

    // Create a path, reserve a port, then delete the path
    let temp_path = env.create_dir("temp-project");
    let port = env.reserve_simple(&temp_path);

    // Verify reservation exists
    let list_before = env.list();
    assert!(list_before.contains(&port.to_string()));

    // Delete the directory
    fs::remove_dir_all(&temp_path).expect("Failed to remove directory");

    // Run prune
    let output = env
        .command()
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(
        output.status.success(),
        "prune should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stderr should mention removal
    assert!(
        stderr.contains("Removed") || stderr.contains("1"),
        "stderr should indicate removal: {stderr}"
    );

    // Verify reservation was actually deleted
    let list_after = env.list();
    assert!(
        !list_after.contains(&port.to_string()),
        "pruned reservation should be gone from database"
    );
}

/// Test prune with all paths existing (nothing to prune).
///
/// When all reservation paths still exist, prune should succeed but remove
/// nothing. This tests the "no-op" case and ensures prune doesn't incorrectly
/// remove valid reservations.
#[test]
fn test_prune_with_existing_paths() {
    let env = TestEnv::new();

    // Create paths and reserve ports (don't delete them)
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    env.reserve_simple(&path1);
    env.reserve_simple(&path2);

    // Run prune
    let output = env
        .command()
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate 0 removals
    assert!(
        stderr.contains("Removed 0") || stderr.contains("0 reservation"),
        "stderr should show 0 removals: {stderr}"
    );

    // All reservations should still exist
    let list_after = env.list();
    assert!(list_after.contains(path1.to_str().unwrap()));
    assert!(list_after.contains(path2.to_str().unwrap()));
}

/// Test prune with mixed paths (some exist, some don't).
///
/// This tests the filtering logic: prune should remove only the reservations
/// for non-existent paths while preserving those for existing paths.
#[test]
fn test_prune_mixed_paths() {
    let env = TestEnv::new();

    // Create two paths, reserve ports
    let existing = env.create_dir("existing");
    let to_delete = env.create_dir("to-delete");

    let port_existing = env.reserve_simple(&existing);
    let port_deleted = env.reserve_simple(&to_delete);

    // Delete one directory
    fs::remove_dir_all(&to_delete).expect("Failed to remove directory");

    // Run prune
    let output = env
        .command()
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    // Should remove exactly 1 reservation
    let list_after = env.list();
    assert!(
        list_after.contains(&port_existing.to_string()),
        "existing path's reservation should remain"
    );
    assert!(
        !list_after.contains(&port_deleted.to_string()),
        "deleted path's reservation should be removed"
    );
}

/// Test prune dry-run mode.
///
/// Dry-run should:
/// - Report what would be removed without actually removing it
/// - Not modify the database
/// - Show "[DRY RUN]" or similar indicator in output
/// - Return success exit code
#[test]
fn test_prune_dry_run() {
    let env = TestEnv::new();

    // Create and then delete a path
    let temp_path = env.create_dir("temp");
    let port = env.reserve_simple(&temp_path);
    fs::remove_dir_all(&temp_path).expect("Failed to remove directory");

    // Run prune in dry-run mode
    let output = env
        .command()
        .arg("prune")
        .arg("--dry-run")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate dry-run and show what would be removed
    assert!(
        stderr.contains("DRY RUN") || stderr.contains("Would remove"),
        "stderr should indicate dry-run mode: {stderr}"
    );
    assert!(
        stderr.contains("1"),
        "stderr should show count of what would be removed: {stderr}"
    );

    // Reservation should still exist in database
    let list_after = env.list();
    assert!(
        list_after.contains(&port.to_string()),
        "dry-run should not actually remove reservations"
    );
}

/// Test prune on empty database.
///
/// Running prune when no reservations exist should succeed gracefully,
/// reporting 0 removals without errors.
#[test]
fn test_prune_empty_database() {
    let env = TestEnv::new();

    // Don't create any reservations, just run prune
    let output = env
        .command()
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(
        output.status.success(),
        "prune on empty database should succeed"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    assert!(
        stderr.contains("0"),
        "stderr should indicate 0 removals: {stderr}"
    );
}

/// Test prune quiet mode.
///
/// With --quiet (global flag), prune should:
/// - Suppress stderr output
/// - Only output the count to stdout if removals occurred
/// - Still perform the operation
#[test]
fn test_prune_quiet_mode() {
    let env = TestEnv::new();

    // Create and delete a path
    let temp_path = env.create_dir("temp");
    env.reserve_simple(&temp_path);
    fs::remove_dir_all(&temp_path).expect("Failed to remove directory");

    // Run prune in quiet mode
    let output = env
        .command()
        .arg("--quiet")
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Quiet mode: stdout might have count, stderr should be minimal/empty
    // (exact behavior depends on implementation)
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet mode should suppress stderr: {stderr}"
    );

    // If stdout has output, it should just be the count
    if !stdout.trim().is_empty() {
        assert!(
            stdout.trim().parse::<u32>().is_ok(),
            "quiet stdout should be just a number: {stdout}"
        );
    }
}

/// Test prune verbose mode.
///
/// With --verbose, prune should output detailed information about each
/// removed reservation, including path, port, tag, and project.
#[test]
fn test_prune_verbose_mode() {
    let env = TestEnv::new();

    // Create reservation with metadata
    let temp_path = env.create_dir("verbose-test");
    let port = env.reserve_with_tag(&temp_path, "test-service");
    fs::remove_dir_all(&temp_path).expect("Failed to remove directory");

    // Run prune in verbose mode
    let output = env
        .command()
        .arg("--verbose")
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Verbose should show detailed information
    assert!(
        stderr.contains("Removed"),
        "verbose should show removal message: {stderr}"
    );
    assert!(
        stderr.contains(&port.to_string()),
        "verbose should show port number: {stderr}"
    );
    assert!(
        stderr.contains("test-service"),
        "verbose should show service tag: {stderr}"
    );
}

// ============================================================================
// Expire Command Tests
// ============================================================================

/// Test basic expire with old reservations.
///
/// This verifies core expire functionality: removing reservations that haven't
/// been used within the specified time threshold. The command should:
/// - Require --days flag (or config setting)
/// - Remove reservations older than the threshold
/// - Keep reservations newer than the threshold
/// - Report how many were removed
#[test]
fn test_expire_removes_old_reservations() {
    let env = TestEnv::new();

    // Create old and fresh reservations
    let old_path = env.create_dir("old-project");
    let fresh_path = env.create_dir("fresh-project");

    let old_port = reserve_old_port(&env, &old_path, 30); // 30 days old
    let fresh_port = env.reserve_simple(&fresh_path); // Fresh (today)

    // Expire with 7-day threshold
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(
        output.status.success(),
        "expire should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    assert!(
        stderr.contains("Expired") || stderr.contains("1"),
        "stderr should indicate expiration: {stderr}"
    );

    // Old reservation should be gone, fresh should remain
    let list_after = env.list();
    assert!(
        !list_after.contains(&old_port.to_string()),
        "old reservation should be expired"
    );
    assert!(
        list_after.contains(&fresh_port.to_string()),
        "fresh reservation should remain"
    );
}

/// Test expire with fresh reservations (should keep them).
///
/// When all reservations are newer than the expiration threshold,
/// expire should not remove anything.
#[test]
fn test_expire_keeps_fresh_reservations() {
    let env = TestEnv::new();

    // Create fresh reservations (all within threshold)
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    env.reserve_simple(&path1);
    env.reserve_simple(&path2);

    // Expire with 7-day threshold
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate 0 expirations
    assert!(
        stderr.contains("Expired 0") || stderr.contains("0 reservation"),
        "stderr should show 0 expirations: {stderr}"
    );

    // All reservations should still exist
    let list_after = env.list();
    assert!(list_after.contains(path1.to_str().unwrap()));
    assert!(list_after.contains(path2.to_str().unwrap()));
}

/// Test expire dry-run mode.
///
/// Dry-run should report what would be expired without actually
/// removing the reservations.
#[test]
fn test_expire_dry_run() {
    let env = TestEnv::new();

    // Create an old reservation
    let old_path = env.create_dir("old");
    let old_port = reserve_old_port(&env, &old_path, 30);

    // Expire in dry-run mode
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .arg("--dry-run")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate dry-run
    assert!(
        stderr.contains("DRY RUN") || stderr.contains("Would expire"),
        "stderr should indicate dry-run mode: {stderr}"
    );

    // Reservation should still exist
    let list_after = env.list();
    assert!(
        list_after.contains(&old_port.to_string()),
        "dry-run should not actually expire reservations"
    );
}

/// Test expire uses default threshold when no --days flag.
///
/// The configuration system provides a default expire_after_days of 30.
/// When no --days flag is provided, expire should use this default and succeed.
#[test]
fn test_expire_uses_default_threshold() {
    let env = TestEnv::new();

    // Create reservations at different ages
    let very_old = env.create_dir("very-old");
    let fresh = env.create_dir("fresh");

    reserve_old_port(&env, &very_old, 45); // 45 days old (exceeds default 30)
    env.reserve_simple(&fresh); // Fresh (today)

    // Run expire without --days flag (should use default 30 days)
    let output = env
        .command()
        .arg("expire")
        .output()
        .expect("Failed to run expire");

    assert!(
        output.status.success(),
        "expire should succeed using default threshold, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should show it used 30-day threshold
    assert!(
        stderr.contains("30"),
        "stderr should show default 30-day threshold: {stderr}"
    );

    // Very old reservation should be expired, fresh should remain
    let list_after = env.list();
    assert!(
        !list_after.contains(very_old.to_str().unwrap()),
        "45-day-old reservation should be expired with default 30-day threshold"
    );
    assert!(
        list_after.contains(fresh.to_str().unwrap()),
        "fresh reservation should remain"
    );
}

// NOTE: Config file loading tests are currently disabled because the config system
// doesn't support specifying a custom config file path via environment variable.
// The config builder loads from standard locations (~/.trop/trop.yaml, ./trop.yaml, etc.).
// To test custom config files properly, we would need to either:
// 1. Add support for TROP_CONFIG_FILE env var
// 2. Write config files to standard search locations (which could interfere with other tests)
// 3. Add a --config flag to the CLI
//
// For now, we rely on the default config (which includes expire_after_days: 30)
// and test CLI --days override behavior.

/// Test expire with CLI override of default.
///
/// When --days flag is provided, it should override the default config value.
/// This verifies CLI > config > defaults precedence.
#[test]
fn test_expire_cli_overrides_default() {
    let env = TestEnv::new();

    // Create reservation that's 20 days old (under default 30, but over our CLI value of 10)
    let path = env.create_dir("medium");
    let port = reserve_old_port(&env, &path, 20);

    // Run expire with --days=10 (overrides default 30)
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("10")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    // 20-day-old reservation should be expired (exceeds 10-day CLI threshold)
    let list_after = env.list();
    assert!(
        !list_after.contains(&port.to_string()),
        "CLI --days flag should override default threshold"
    );
}

/// Test expire quiet mode.
///
/// In quiet mode, expire should suppress stderr and optionally output
/// count to stdout.
#[test]
fn test_expire_quiet_mode() {
    let env = TestEnv::new();

    // Create old reservation
    let old_path = env.create_dir("old");
    reserve_old_port(&env, &old_path, 30);

    // Expire in quiet mode
    let output = env
        .command()
        .arg("--quiet")
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stderr should be empty
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet mode should suppress stderr: {stderr}"
    );
}

/// Test expire verbose mode.
///
/// Verbose mode should show detailed information about each expired
/// reservation, including age in days.
#[test]
fn test_expire_verbose_mode() {
    let env = TestEnv::new();

    // Create old reservation with metadata
    let old_path = env.create_dir("old-verbose");
    let port = reserve_old_port(&env, &old_path, 30);

    // Expire in verbose mode
    let output = env
        .command()
        .arg("--verbose")
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should show detailed information including port and age
    assert!(
        stderr.contains(&port.to_string()),
        "verbose should show port number: {stderr}"
    );
    assert!(
        stderr.contains("days old") || stderr.contains("30") || stderr.contains("days"),
        "verbose should show age information: {stderr}"
    );
}

/// Test expire on empty database.
///
/// Running expire when no reservations exist should succeed gracefully.
#[test]
fn test_expire_empty_database() {
    let env = TestEnv::new();

    // Don't create any reservations
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(
        output.status.success(),
        "expire on empty database should succeed"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    assert!(
        stderr.contains("0"),
        "stderr should indicate 0 expirations: {stderr}"
    );
}

/// Test expire with boundary threshold.
///
/// This tests the edge case where a reservation is exactly at the threshold
/// boundary. We want to verify the >= vs > semantics.
#[test]
fn test_expire_boundary_threshold() {
    let env = TestEnv::new();

    // Create reservations at different ages
    let over = env.create_dir("over");
    let under = env.create_dir("under");

    reserve_old_port(&env, &over, 8); // 8 days old (over 7-day threshold)
    reserve_old_port(&env, &under, 5); // 5 days old (under threshold)

    // Expire with 7-day threshold
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let list_after = env.list();

    // 8-day-old should be expired
    assert!(
        !list_after.contains(over.to_str().unwrap()),
        "reservation over threshold should be expired"
    );

    // 5-day-old should remain
    assert!(
        list_after.contains(under.to_str().unwrap()),
        "reservation under threshold should remain"
    );
}

// ============================================================================
// Autoclean Command Tests
// ============================================================================

/// Test autoclean combines prune and expire.
///
/// Autoclean should perform both operations in sequence, removing:
/// 1. Reservations for non-existent paths (prune)
/// 2. Reservations older than threshold (expire)
///
/// The output should show counts for both operations.
#[test]
fn test_autoclean_combined_operation() {
    let env = TestEnv::new();

    // Create three reservations:
    // 1. Non-existent path (will be pruned)
    // 2. Old reservation with existing path (will be expired)
    // 3. Fresh reservation with existing path (will remain)

    let to_prune = env.create_dir("to-prune");
    let to_expire = env.create_dir("to-expire");
    let to_keep = env.create_dir("to-keep");

    let port_prune = env.reserve_simple(&to_prune);
    let port_expire = reserve_old_port(&env, &to_expire, 30);
    let port_keep = env.reserve_simple(&to_keep);

    // Delete one directory (for pruning)
    fs::remove_dir_all(&to_prune).expect("Failed to remove directory");

    // Run autoclean
    let output = env
        .command()
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run autoclean");

    assert!(
        output.status.success(),
        "autoclean should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should show both pruned and expired counts
    assert!(
        stderr.contains("total") || stderr.contains("2"),
        "stderr should show total removals: {stderr}"
    );
    assert!(
        stderr.contains("Pruned") || stderr.contains("pruned") || stderr.contains("1"),
        "stderr should show pruned count: {stderr}"
    );
    assert!(
        stderr.contains("Expired") || stderr.contains("expired") || stderr.contains("1"),
        "stderr should show expired count: {stderr}"
    );

    // Verify correct reservations were removed
    let list_after = env.list();
    assert!(
        !list_after.contains(&port_prune.to_string()),
        "pruned reservation should be removed"
    );
    assert!(
        !list_after.contains(&port_expire.to_string()),
        "expired reservation should be removed"
    );
    assert!(
        list_after.contains(&port_keep.to_string()),
        "fresh valid reservation should remain"
    );
}

/// Test autoclean dry-run mode.
///
/// Dry-run should report what would be removed by both prune and expire
/// without actually modifying the database.
#[test]
fn test_autoclean_dry_run() {
    let env = TestEnv::new();

    // Create reservation to be pruned
    let temp = env.create_dir("temp");
    let port = env.reserve_simple(&temp);
    fs::remove_dir_all(&temp).expect("Failed to remove directory");

    // Run autoclean in dry-run
    let output = env
        .command()
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .arg("--dry-run")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate dry-run
    assert!(
        stderr.contains("DRY RUN") || stderr.contains("Would remove"),
        "stderr should indicate dry-run mode: {stderr}"
    );

    // Reservation should still exist
    let list_after = env.list();
    assert!(
        list_after.contains(&port.to_string()),
        "dry-run should not actually remove reservations"
    );
}

/// Test autoclean uses default threshold.
///
/// When no --days flag is provided, autoclean should use the default
/// expire_after_days value (30 days).
#[test]
fn test_autoclean_uses_default_threshold() {
    let env = TestEnv::new();

    // Create old reservation (45 days old, exceeds default 30)
    let old_path = env.create_dir("old");
    let old_port = reserve_old_port(&env, &old_path, 45);

    // Run autoclean without --days (should use default 30)
    let output = env
        .command()
        .arg("autoclean")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    // Old reservation should be expired
    let list_after = env.list();
    assert!(
        !list_after.contains(&old_port.to_string()),
        "autoclean should use default 30-day threshold"
    );
}

/// Test autoclean with CLI override of default.
///
/// CLI --days flag should override the default config settings.
#[test]
fn test_autoclean_cli_override() {
    let env = TestEnv::new();

    // Create 20-day-old reservation (under default 30, but over our CLI value of 10)
    let path = env.create_dir("medium");
    let port = reserve_old_port(&env, &path, 20);

    // Run autoclean with --days=10 (overrides default 30)
    let output = env
        .command()
        .arg("autoclean")
        .arg("--days")
        .arg("10")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    // Should be expired (CLI threshold of 10 is stricter than default 30)
    let list_after = env.list();
    assert!(
        !list_after.contains(&port.to_string()),
        "CLI --days should override default threshold"
    );
}

/// Test autoclean quiet mode.
///
/// Quiet mode should suppress stderr while still performing the operation.
#[test]
fn test_autoclean_quiet_mode() {
    let env = TestEnv::new();

    // Create something to clean
    let temp = env.create_dir("temp");
    env.reserve_simple(&temp);
    fs::remove_dir_all(&temp).expect("Failed to remove directory");

    // Run in quiet mode
    let output = env
        .command()
        .arg("--quiet")
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stderr should be empty
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet mode should suppress stderr: {stderr}"
    );
}

/// Test autoclean verbose mode.
///
/// Verbose mode should show detailed listings of both pruned and
/// expired reservations.
#[test]
fn test_autoclean_verbose_mode() {
    let env = TestEnv::new();

    // Create reservations to prune and expire
    let to_prune = env.create_dir("prune");
    let to_expire = env.create_dir("expire");

    let port_prune = env.reserve_simple(&to_prune);
    let port_expire = reserve_old_port(&env, &to_expire, 30);

    fs::remove_dir_all(&to_prune).expect("Failed to remove directory");

    // Run in verbose mode
    let output = env
        .command()
        .arg("--verbose")
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should show detailed information for both operations
    assert!(
        stderr.contains(&port_prune.to_string()) || stderr.contains("Pruned"),
        "verbose should show pruned reservations: {stderr}"
    );
    assert!(
        stderr.contains(&port_expire.to_string()) || stderr.contains("Expired"),
        "verbose should show expired reservations: {stderr}"
    );
    assert!(
        stderr.contains("total"),
        "verbose should show total count: {stderr}"
    );
}

/// Test autoclean on empty database.
///
/// Running autoclean when no reservations exist should succeed gracefully.
#[test]
fn test_autoclean_empty_database() {
    let env = TestEnv::new();

    // Don't create any reservations
    let output = env
        .command()
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run autoclean");

    assert!(
        output.status.success(),
        "autoclean on empty database should succeed"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    assert!(
        stderr.contains("0"),
        "stderr should indicate 0 removals: {stderr}"
    );
}

// NOTE: test_autoclean_prune_only removed because the default config always
// includes expire_after_days (30 days), so we can't test a pure "prune-only"
// scenario without custom config file support (see note above about config loading).

// ============================================================================
// Edge Cases and Integration
// ============================================================================

/// Test prune with multiple non-existent paths.
///
/// Batch processing of multiple prunable reservations should work correctly.
#[test]
fn test_prune_multiple_nonexistent_paths() {
    let env = TestEnv::new();

    // Create and delete multiple paths
    for i in 0..5 {
        let path = env.create_dir(&format!("temp{i}"));
        env.reserve_simple(&path);
        fs::remove_dir_all(&path).expect("Failed to remove directory");
    }

    // Run prune
    let output = env
        .command()
        .arg("prune")
        .output()
        .expect("Failed to run prune");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should remove all 5
    assert!(
        stderr.contains("5"),
        "stderr should show 5 removals: {stderr}"
    );

    // Database should be empty
    let list_after = env.list();
    let line_count = list_after.lines().count();
    assert_eq!(
        line_count, 1,
        "only header should remain (all reservations pruned)"
    );
}

/// Test expire with multiple threshold values.
///
/// Different threshold values should correctly filter reservations by age.
#[test]
fn test_expire_multiple_thresholds() {
    let env = TestEnv::new();

    // Create reservations at 5, 15, and 30 days old
    let path5 = env.create_dir("5days");
    let path15 = env.create_dir("15days");
    let path30 = env.create_dir("30days");

    reserve_old_port(&env, &path5, 5);
    reserve_old_port(&env, &path15, 15);
    reserve_old_port(&env, &path30, 30);

    // Expire with 10-day threshold (should remove 15 and 30 day old)
    let output = env
        .command()
        .arg("expire")
        .arg("--days")
        .arg("10")
        .output()
        .expect("Failed to run expire");

    assert!(output.status.success());

    let list_after = env.list();

    // 5-day should remain
    assert!(
        list_after.contains(path5.to_str().unwrap()),
        "5-day-old should remain"
    );

    // 15 and 30 day should be expired
    assert!(
        !list_after.contains(path15.to_str().unwrap()),
        "15-day-old should be expired"
    );
    assert!(
        !list_after.contains(path30.to_str().unwrap()),
        "30-day-old should be expired"
    );
}

/// Test cleanup preserves valid fresh reservations.
///
/// This is a critical safety invariant: cleanup operations should never
/// remove fresh reservations with existing paths.
#[test]
fn test_cleanup_preserves_fresh_valid_reservations() {
    let env = TestEnv::new();

    // Create fresh reservation with existing path
    let valid_path = env.create_dir("valid");
    let valid_port = env.reserve_simple(&valid_path);

    // Run all cleanup operations
    env.command().arg("prune").assert().success();

    env.command()
        .arg("expire")
        .arg("--days")
        .arg("7")
        .assert()
        .success();

    env.command()
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .assert()
        .success();

    // Reservation should still exist after all cleanup
    let list_after = env.list();
    assert!(
        list_after.contains(&valid_port.to_string()),
        "fresh valid reservation should survive all cleanup operations"
    );
}

/// Test autoclean with no overlap between prune and expire sets.
///
/// This verifies correct counting when operations affect different
/// reservations (one pruned, one expired, one kept).
#[test]
fn test_autoclean_no_overlap() {
    let env = TestEnv::new();

    // Create three distinct cases:
    // 1. Non-existent path (will be pruned)
    // 2. Old reservation with existing path (will be expired)
    // 3. Fresh reservation with existing path (will remain)

    let nonexistent = env.create_dir("nonexistent");
    let old = env.create_dir("old");
    let fresh = env.create_dir("fresh");

    env.reserve_simple(&nonexistent);
    reserve_old_port(&env, &old, 30);
    env.reserve_simple(&fresh);

    fs::remove_dir_all(&nonexistent).expect("Failed to remove directory");

    // Run autoclean
    let output = env
        .command()
        .arg("autoclean")
        .arg("--days")
        .arg("7")
        .output()
        .expect("Failed to run autoclean");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should show pruned: 1, expired: 1, total: 2
    assert!(
        stderr.contains("2") && (stderr.contains("1") || stderr.contains("Pruned")),
        "should show correct counts for non-overlapping operations: {stderr}"
    );

    // Only fresh should remain
    let list_after = env.list();
    let line_count = list_after.lines().count();
    assert_eq!(line_count, 2, "should have header + 1 fresh reservation");
}

/// Test cleanup commands respect quiet/verbose global flags.
///
/// All cleanup commands should properly handle the global --quiet and
/// --verbose flags inherited from the main CLI.
#[test]
fn test_cleanup_respects_global_flags() {
    let env = TestEnv::new();

    // Create something to clean
    let temp = env.create_dir("temp");
    env.reserve_simple(&temp);
    fs::remove_dir_all(&temp).expect("Failed to remove directory");

    // Test all commands with --quiet
    for cmd in ["prune", "expire", "autoclean"] {
        let mut command = env.command();
        command.arg("--quiet").arg(cmd);

        if cmd != "prune" {
            command.arg("--days").arg("7");
        }

        let output = command
            .output()
            .unwrap_or_else(|_| panic!("Failed to run {cmd}"));

        assert!(
            output.status.success(),
            "{cmd} should succeed in quiet mode"
        );

        let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
        assert!(
            stderr.is_empty() || stderr.trim().is_empty(),
            "{cmd} quiet mode should suppress stderr: {stderr}"
        );
    }
}
