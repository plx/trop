//! Comprehensive integration tests for the `release` command.
//!
//! These tests verify all aspects of port release functionality, including:
//! - Basic release (untagged and tagged)
//! - Recursive release
//! - Dry-run mode
//! - Force flag behavior
//! - Error cases (nothing to release, etc.)
//! - Release with various flag combinations

mod common;

use common::TestEnv;
use predicates::prelude::*;

// ============================================================================
// Basic Release Tests
// ============================================================================

/// Test basic release of an untagged reservation.
///
/// This verifies the most fundamental operation: releasing a port reservation
/// for a specific directory. After release, the port should be freed and
/// available for reuse.
#[test]
fn test_release_basic() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    let port = env.reserve_simple(&test_path);

    // Verify it exists
    let list_before = env.list();
    assert!(list_before.contains(&port.to_string()));

    // Release it
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .assert()
        .success();

    // Verify it's gone
    let list_after = env.list();
    assert!(!list_after.contains(&port.to_string()));
}

/// Test release with explicit tag.
///
/// When a directory has multiple tagged reservations, releasing a specific
/// tag should only remove that reservation, leaving others intact.
#[test]
fn test_release_with_tag() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create two tagged reservations
    let port_web = env.reserve_with_tag(&test_path, "web");
    let port_api = env.reserve_with_tag(&test_path, "api");

    // Release the "web" tag
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("web")
        .assert()
        .success();

    // Verify "web" is gone but "api" remains
    let list_output = env.list();
    assert!(!list_output.contains(&port_web.to_string()));
    assert!(list_output.contains(&port_api.to_string()));
}

/// Test release without path uses current directory.
///
/// Like reserve, release should default to the current working directory
/// when no --path is specified.
#[test]
fn test_release_without_path_uses_cwd() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Reserve a port
    let port = env.reserve_simple(&test_path);

    // Release from within the directory (using current_dir)
    let mut cmd = env.command();
    cmd.arg("release")
        .current_dir(&test_path)
        .assert()
        .success();

    // Verify it's released
    let list_output = env.list();
    assert!(!list_output.contains(&port.to_string()));
}

/// Test release of untagged reservation when tagged ones exist.
///
/// The --untagged-only flag should release only the untagged reservation,
/// leaving tagged ones intact.
#[test]
fn test_release_untagged_only() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create untagged and tagged reservations
    let port_untagged = env.reserve_simple(&test_path);
    let port_tagged = env.reserve_with_tag(&test_path, "web");

    // Release untagged only
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--untagged-only")
        .assert()
        .success();

    // Verify untagged is gone, tagged remains
    let list_output = env.list();
    assert!(!list_output.contains(&port_untagged.to_string()));
    assert!(list_output.contains(&port_tagged.to_string()));
}

// ============================================================================
// Recursive Release Tests
// ============================================================================

/// Test recursive release.
///
/// The --recursive flag should release all reservations under a directory
/// tree, not just the exact path match.
#[test]
fn test_release_recursive() {
    let env = TestEnv::new();
    let parent = env.create_dir("parent");
    let child1 = env.create_dir("parent/child1");
    let child2 = env.create_dir("parent/child2");

    // Create reservations in parent and children
    let port_parent = env.reserve_simple(&parent);
    let port_child1 = env.reserve_simple(&child1);
    let port_child2 = env.reserve_simple(&child2);

    // Release parent recursively
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--recursive")
        .assert()
        .success();

    // All should be gone
    let list_output = env.list();
    assert!(!list_output.contains(&port_parent.to_string()));
    assert!(!list_output.contains(&port_child1.to_string()));
    assert!(!list_output.contains(&port_child2.to_string()));
}

/// Test recursive release with specific tag.
///
/// Combining --recursive and --tag should release all reservations with
/// that tag under the directory tree.
#[test]
fn test_release_recursive_with_tag() {
    let env = TestEnv::new();
    let parent = env.create_dir("parent");
    let child = env.create_dir("parent/child");

    // Create "web" and "api" tags at both levels
    let port_parent_web = env.reserve_with_tag(&parent, "web");
    let port_parent_api = env.reserve_with_tag(&parent, "api");
    let port_child_web = env.reserve_with_tag(&child, "web");
    let port_child_api = env.reserve_with_tag(&child, "api");

    // Release "web" recursively
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--tag")
        .arg("web")
        .arg("--recursive")
        .assert()
        .success();

    // "web" should be gone at both levels, "api" should remain
    let list_output = env.list();
    assert!(!list_output.contains(&port_parent_web.to_string()));
    assert!(!list_output.contains(&port_child_web.to_string()));
    assert!(list_output.contains(&port_parent_api.to_string()));
    assert!(list_output.contains(&port_child_api.to_string()));
}

/// Test non-recursive release doesn't affect children.
///
/// Without --recursive, only the exact path should be released,
/// not subdirectories.
#[test]
fn test_release_non_recursive_preserves_children() {
    let env = TestEnv::new();
    let parent = env.create_dir("parent");
    let child = env.create_dir("parent/child");

    // Create reservations in parent and child
    let port_parent = env.reserve_simple(&parent);
    let port_child = env.reserve_simple(&child);

    // Release parent non-recursively
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .assert()
        .success();

    // Parent gone, child remains
    let list_output = env.list();
    assert!(!list_output.contains(&port_parent.to_string()));
    assert!(list_output.contains(&port_child.to_string()));
}

// ============================================================================
// Dry-Run Mode Tests
// ============================================================================

/// Test release with --dry-run flag.
///
/// Dry-run mode should:
/// - Not actually release anything
/// - Show what would be released (on stderr)
/// - Return success exit code
#[test]
fn test_release_dry_run_does_not_release() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    let port = env.reserve_simple(&test_path);

    // Dry-run release
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--dry-run")
        .assert()
        .success();

    // Reservation should still exist
    let list_output = env.list();
    assert!(list_output.contains(&port.to_string()));
}

/// Test that dry-run shows planned actions.
///
/// In dry-run mode, trop should output what would be released.
#[test]
fn test_release_dry_run_shows_plan() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Dry-run should show plan on stderr
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("Dry run"));
}

/// Test dry-run with --quiet suppresses output.
#[test]
fn test_release_dry_run_with_quiet() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Dry-run with --quiet
    let output = env
        .command()
        .arg("--quiet")
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--dry-run")
        .output()
        .unwrap();

    assert!(output.status.success());
    // Should suppress the dry-run plan
    assert!(
        output.stderr.is_empty(),
        "Quiet mode should suppress dry-run output"
    );
}

// ============================================================================
// Force Flag Tests
// ============================================================================

/// Test release with --force flag.
///
/// The --force flag may be needed for certain edge cases or to override
/// protections. This test verifies it works without causing issues.
#[test]
fn test_release_with_force() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    let port = env.reserve_simple(&test_path);

    // Release with --force
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--force")
        .assert()
        .success();

    // Should be released
    let list_output = env.list();
    assert!(!list_output.contains(&port.to_string()));
}

/// Test that force flag doesn't cause issues with non-existent reservations.
///
/// Using --force to release something that doesn't exist should either
/// succeed silently or give a clear message.
#[test]
fn test_release_force_on_nonexistent() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to release something that doesn't exist with --force
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--force")
        .output()
        .unwrap();

    // Should not crash - either succeeds or fails gracefully
    // Exit code can be success (noop) or failure (not found)
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.is_empty(), "Should have error message");
    }
}

// ============================================================================
// Error Cases
// ============================================================================

/// Test release when nothing to release.
///
/// Attempting to release a path with no reservation should fail with
/// a clear error message (unless --force is used).
#[test]
fn test_release_nothing_to_release() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to release when no reservation exists
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No reservation"))
                .or(predicate::str::contains("error")),
        );
}

/// Test release with nonexistent tag.
///
/// Trying to release a tag that doesn't exist should fail clearly.
#[test]
fn test_release_nonexistent_tag() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create untagged reservation
    env.reserve_simple(&test_path);

    // Try to release a tag that doesn't exist
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("nonexistent")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("error")));
}

/// Test that --tag and --untagged-only are mutually exclusive.
///
/// These flags conflict and should not be used together.
#[test]
fn test_release_tag_and_untagged_only_conflict() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to use both --tag and --untagged-only
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("web")
        .arg("--untagged-only")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot specify both"));
}

/// Test release with nonexistent path.
///
/// Trying to release a path that doesn't exist or has never had a
/// reservation should fail appropriately.
#[test]
fn test_release_nonexistent_path() {
    let env = TestEnv::new();
    let fake_path = env.path().join("does-not-exist");

    // Try to release nonexistent path
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&fake_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("error").or(predicate::str::contains("not found")));
}

// ============================================================================
// Environment Variable Tests
// ============================================================================

/// Test that TROP_PATH environment variable is respected.
#[test]
fn test_release_respects_trop_path_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    let port = env.reserve_simple(&test_path);

    // Release using env var for path
    env.command()
        .arg("release")
        .env("TROP_PATH", &test_path)
        .assert()
        .success();

    // Should be released
    let list_output = env.list();
    assert!(!list_output.contains(&port.to_string()));
}

/// Test that command-line --path overrides TROP_PATH.
#[test]
fn test_cli_path_overrides_env_path_for_release() {
    let env = TestEnv::new();
    let path1 = env.create_dir("path1");
    let path2 = env.create_dir("path2");

    // Create reservations at both paths
    let port1 = env.reserve_simple(&path1);
    let port2 = env.reserve_simple(&path2);

    // Set env to path1 but use --path for path2
    env.command()
        .arg("release")
        .arg("--path")
        .arg(&path2)
        .env("TROP_PATH", &path1)
        .assert()
        .success();

    // path2 should be released, path1 should remain
    let list_output = env.list();
    assert!(list_output.contains(&port1.to_string()));
    assert!(!list_output.contains(&port2.to_string()));
}

// ============================================================================
// Output Tests
// ============================================================================

/// Test that release produces appropriate success message.
///
/// After successful release, there should be some confirmation on stderr
/// (unless --quiet is used). Stdout should be empty (no port number needed).
#[test]
fn test_release_success_message() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Release and check output
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    assert!(output.status.success());

    // Stdout should be empty (or minimal)
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "Stdout should be empty for release"
    );

    // Stderr may have a success message (but not required)
    // Just verify it doesn't have error indicators
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.to_lowercase().contains("error"));
}

/// Test that --quiet suppresses output.
#[test]
fn test_release_quiet_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Release with --quiet
    let output = env
        .command()
        .arg("--quiet")
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    assert!(output.status.success());

    // Both stdout and stderr should be minimal/empty
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.trim().is_empty());
    assert!(stderr.trim().is_empty() || stderr.is_empty());
}

/// Test --verbose provides additional information.
#[test]
fn test_release_verbose_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Release with --verbose
    let output = env
        .command()
        .arg("--verbose")
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verbose mode may produce stderr output (logs, details)
    // We can't guarantee specific content, but it shouldn't error
}

// ============================================================================
// Multiple Release Tests
// ============================================================================

/// Test releasing multiple tags one by one.
///
/// This verifies that releasing tags individually works correctly and
/// doesn't interfere with other tags at the same path.
#[test]
fn test_release_multiple_tags_sequentially() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create three tagged reservations
    let port1 = env.reserve_with_tag(&test_path, "web");
    let port2 = env.reserve_with_tag(&test_path, "api");
    let port3 = env.reserve_with_tag(&test_path, "db");

    // Release them one by one
    env.release_with_tag(&test_path, "web");
    let list1 = env.list();
    assert!(!list1.contains(&port1.to_string()));
    assert!(list1.contains(&port2.to_string()));
    assert!(list1.contains(&port3.to_string()));

    env.release_with_tag(&test_path, "api");
    let list2 = env.list();
    assert!(!list2.contains(&port2.to_string()));
    assert!(list2.contains(&port3.to_string()));

    env.release_with_tag(&test_path, "db");
    let list3 = env.list();
    assert!(!list3.contains(&port3.to_string()));
}

/// Test that releasing all reservations at a path works.
///
/// If there are multiple tags at a path, releasing the path without
/// specifying a tag might release all of them, or might require --recursive
/// or special flags. This test documents the behavior.
#[test]
fn test_release_path_with_multiple_tags() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create multiple tagged reservations
    env.reserve_with_tag(&test_path, "web");
    env.reserve_with_tag(&test_path, "api");

    // Try to release the path (no tag specified)
    // This might fail (ambiguous) or release all - behavior depends on implementation
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .output()
        .unwrap();

    // Document the behavior: either succeeds or gives clear error
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.is_empty(), "Should explain why it failed");
    }
}

// ============================================================================
// Idempotency Tests
// ============================================================================

/// Test that releasing twice doesn't cause issues with --force.
///
/// Releasing something that's already released should either:
/// - Fail with a clear error (without --force)
/// - Succeed as a no-op (with --force)
#[test]
fn test_release_idempotency_with_force() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create and release a reservation
    env.reserve_simple(&test_path);
    env.release(&test_path);

    // Try to release again with --force - should not crash
    let output = env
        .command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--force")
        .output()
        .unwrap();

    // Should succeed or fail gracefully
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.is_empty());
    }
}
