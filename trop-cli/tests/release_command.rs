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

use assert_cmd::Command;
use common::TestEnv;
use predicates::prelude::*;
use rusqlite::Connection;

fn related_command(env: &TestEnv) -> Command {
    let mut command = env.command();
    command.current_dir(env.path());
    command
}

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
    related_command(&env)
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
    related_command(&env)
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

    // Reserve implicitly so both operations use the canonical CWD identity.
    let reserve_output = related_command(&env)
        .arg("reserve")
        .arg("--allow-unrelated-path")
        .current_dir(&test_path)
        .output()
        .expect("Failed to reserve from the current directory");
    assert!(
        reserve_output.status.success(),
        "Reserve failed: {}",
        String::from_utf8_lossy(&reserve_output.stderr)
    );
    let port: u16 = String::from_utf8(reserve_output.stdout)
        .expect("Invalid UTF-8 in reserve output")
        .trim()
        .parse()
        .expect("Reserve output is not a valid port number");

    // Release from within the directory (using current_dir)
    let mut cmd = related_command(&env);
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
    related_command(&env)
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
    related_command(&env)
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
    related_command(&env)
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

/// Default recursive release removes every tag under a component-aware root.
#[test]
fn test_release_recursive_all_tags_preserves_lexical_prefix_sibling() {
    let env = TestEnv::new();
    let parent = env.create_dir("work/a");
    let child = env.create_dir("work/a/child");
    let lexical_sibling = env.create_dir("work/ab");

    let parent_untagged = env.reserve_simple(&parent);
    let parent_web = env.reserve_with_tag(&parent, "web");
    let child_api = env.reserve_with_tag(&child, "api");
    let sibling = env.reserve_simple(&lexical_sibling);

    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--recursive")
        .assert()
        .success();

    let list_output = env.list();
    assert!(!list_output.contains(&parent_untagged.to_string()));
    assert!(!list_output.contains(&parent_web.to_string()));
    assert!(!list_output.contains(&child_api.to_string()));
    assert!(list_output.contains(&sibling.to_string()));
}

/// Recursive untagged-only release preserves every tagged reservation.
#[test]
fn test_release_recursive_untagged_only() {
    let env = TestEnv::new();
    let parent = env.create_dir("parent");
    let child = env.create_dir("parent/child");

    let parent_untagged = env.reserve_simple(&parent);
    let parent_tagged = env.reserve_with_tag(&parent, "web");
    let child_untagged = env.reserve_simple(&child);
    let child_tagged = env.reserve_with_tag(&child, "api");

    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--untagged-only")
        .arg("--recursive")
        .assert()
        .success();

    let list_output = env.list();
    assert!(!list_output.contains(&parent_untagged.to_string()));
    assert!(!list_output.contains(&child_untagged.to_string()));
    assert!(list_output.contains(&parent_tagged.to_string()));
    assert!(list_output.contains(&child_tagged.to_string()));
}

/// Recursive dry-run reports the live selection without changing any row.
#[test]
fn test_release_recursive_dry_run_uses_component_aware_selection() {
    let env = TestEnv::new();
    let parent = trop::path::normalize::normalize(&env.create_dir("work/a")).unwrap();
    let child = trop::path::normalize::normalize(&env.create_dir("work/a/child")).unwrap();
    let lexical_sibling = trop::path::normalize::normalize(&env.create_dir("work/ab")).unwrap();

    let parent_port = env.reserve_with_tag(&parent, "web");
    let child_port = env.reserve_with_tag(&child, "web");
    let sibling_port = env.reserve_with_tag(&lexical_sibling, "web");

    let output = related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--tag")
        .arg("web")
        .arg("--recursive")
        .arg("--dry-run")
        .output()
        .expect("Failed to run recursive release dry-run");
    assert!(
        output.status.success(),
        "recursive dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("Dry-run output was not UTF-8");
    assert!(stderr.contains(&format!("{}:web", parent.display())));
    assert!(stderr.contains(&format!("{}:web", child.display())));
    assert!(!stderr.contains(&format!("{}:web", lexical_sibling.display())));

    let list_output = env.list();
    assert!(list_output.contains(&parent_port.to_string()));
    assert!(list_output.contains(&child_port.to_string()));
    assert!(list_output.contains(&sibling_port.to_string()));
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
    related_command(&env)
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
    related_command(&env)
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
    related_command(&env)
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
    let output = related_command(&env)
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
    related_command(&env)
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
    let output = related_command(&env)
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
/// Release is idempotent - attempting to release a path with no reservation
/// should succeed with a warning message.
#[test]
fn test_release_nothing_to_release() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to release when no reservation exists - should succeed (idempotent)
    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .assert()
        .success()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No reservation"))
                .or(predicate::str::contains("already released")),
        );
}

/// Test release with nonexistent tag.
///
/// Release is idempotent - trying to release a tag that doesn't exist should
/// succeed with a warning message.
#[test]
fn test_release_nonexistent_tag() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create untagged reservation
    let port = env.reserve_simple(&test_path);

    // Try to release a tag that doesn't exist - should succeed (idempotent)
    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--tag")
        .arg("nonexistent")
        .assert()
        .success()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No reservation"))
                .or(predicate::str::contains("already released")),
        );

    assert!(env.list().contains(&port.to_string()));
}

/// An untagged-only selector with no match is an idempotent no-op.
#[test]
fn test_release_untagged_only_no_match_preserves_tagged_reservation() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let tagged = env.reserve_with_tag(&test_path, "web");

    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .arg("--untagged-only")
        .assert()
        .success()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No reservation"))
                .or(predicate::str::contains("already released")),
        );

    assert!(env.list().contains(&tagged.to_string()));
}

/// Test that --tag and --untagged-only are mutually exclusive.
///
/// These flags conflict and should not be used together.
#[test]
fn test_release_tag_and_untagged_only_conflict() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Try to use both --tag and --untagged-only
    related_command(&env)
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
/// Release is idempotent - trying to release a path that doesn't exist or has
/// never had a reservation should succeed with a warning message.
#[test]
fn test_release_nonexistent_path() {
    let env = TestEnv::new();
    let fake_path = std::fs::canonicalize(env.path())
        .expect("Failed to canonicalize test root")
        .join("does-not-exist");

    // Try to release nonexistent path - should succeed (idempotent)
    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&fake_path)
        .assert()
        .success()
        .stderr(
            predicate::str::contains("not found")
                .or(predicate::str::contains("No reservation"))
                .or(predicate::str::contains("already released")),
        );
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
    related_command(&env)
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
    related_command(&env)
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
    let output = related_command(&env)
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
    let output = related_command(&env)
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
    let output = related_command(&env)
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

/// Test that the default exact-path release removes every tag at that path.
///
/// Descendant reservations must remain unless `--recursive` is supplied.
#[test]
fn test_release_path_without_filter_removes_all_exact_path_tags_only() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let child_path = env.create_dir("test-project/child");

    let untagged = env.reserve_simple(&test_path);
    let web = env.reserve_with_tag(&test_path, "web");
    let api = env.reserve_with_tag(&test_path, "api");
    let child = env.reserve_with_tag(&child_path, "web");

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .current_dir(env.path())
        .assert()
        .success();

    let list_output = env.list();
    assert!(!list_output.contains(&untagged.to_string()));
    assert!(!list_output.contains(&web.to_string()));
    assert!(!list_output.contains(&api.to_string()));
    assert!(list_output.contains(&child.to_string()));
}

/// Releasing a sideways path is rejected before any reservation is changed.
#[test]
fn test_release_unrelated_path_is_rejected_without_partial_mutation() {
    let env = TestEnv::new();
    let current_path = env.create_dir("current-project");
    let unrelated_path = env.create_dir("unrelated-project");
    let untagged = env.reserve_simple(&unrelated_path);
    let tagged = env.reserve_with_tag(&unrelated_path, "web");

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&unrelated_path)
        .current_dir(&current_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrelated"));

    let list_output = env.list();
    assert!(list_output.contains(&untagged.to_string()));
    assert!(list_output.contains(&tagged.to_string()));
}

/// Recursive release applies the same guard to its requested root.
#[test]
fn test_release_recursive_unrelated_path_is_rejected_without_mutation() {
    let env = TestEnv::new();
    let current_path = env.create_dir("current-project");
    let unrelated_path = env.create_dir("unrelated-project");
    let child_path = env.create_dir("unrelated-project/child");
    let parent = env.reserve_simple(&unrelated_path);
    let child = env.reserve_simple(&child_path);

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&unrelated_path)
        .arg("--recursive")
        .current_dir(&current_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrelated"));

    let list_output = env.list();
    assert!(list_output.contains(&parent.to_string()));
    assert!(list_output.contains(&child.to_string()));
}

/// The explicit unrelated-path permission bypasses the relationship guard.
#[test]
fn test_release_allow_unrelated_path_releases_target() {
    let env = TestEnv::new();
    let current_path = env.create_dir("current-project");
    let unrelated_path = env.create_dir("unrelated-project");
    let untagged = env.reserve_simple(&unrelated_path);
    let tagged = env.reserve_with_tag(&unrelated_path, "web");

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&unrelated_path)
        .arg("--allow-unrelated-path")
        .current_dir(&current_path)
        .assert()
        .success();

    let list_output = env.list();
    assert!(!list_output.contains(&untagged.to_string()));
    assert!(!list_output.contains(&tagged.to_string()));
}

/// The effective configuration permission also bypasses the path guard.
#[test]
fn test_release_honors_allow_unrelated_path_environment_config() {
    let env = TestEnv::new();
    let current_path = env.create_dir("current-project");
    let unrelated_path = env.create_dir("unrelated-project");
    let port = env.reserve_simple(&unrelated_path);

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&unrelated_path)
        .env("TROP_ALLOW_UNRELATED_PATH", "true")
        .current_dir(&current_path)
        .assert()
        .success();

    assert!(!env.list().contains(&port.to_string()));
}

/// A late delete failure rolls back every exact-path deletion.
#[test]
fn test_release_exact_path_deletion_is_atomic() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let untagged = env.reserve_simple(&test_path);
    let tagged = env.reserve_with_tag(&test_path, "web");

    let connection =
        Connection::open(env.data_dir.join("trop.db")).expect("Failed to open test database");
    connection
        .execute_batch(
            "
            CREATE TRIGGER fail_tagged_release
            BEFORE DELETE ON reservations
            WHEN OLD.tag = 'web'
            BEGIN
                SELECT RAISE(ABORT, 'forced late exact release failure');
            END;
            ",
        )
        .expect("Failed to install release failure trigger");
    drop(connection);

    env.command()
        .arg("release")
        .arg("--path")
        .arg(&test_path)
        .current_dir(env.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "forced late exact release failure",
        ));

    let list_output = env.list();
    assert!(list_output.contains(&untagged.to_string()));
    assert!(list_output.contains(&tagged.to_string()));
}

/// A late recursive delete failure rolls back every earlier deletion.
#[test]
fn test_release_recursive_deletion_is_atomic() {
    let env = TestEnv::new();
    let parent = env.create_dir("atomic-parent");
    let child = env.create_dir("atomic-parent/child");
    let parent_port = env.reserve_simple(&parent);
    let child_port = env.reserve_with_tag(&child, "fail");

    let connection =
        Connection::open(env.data_dir.join("trop.db")).expect("Failed to open test database");
    connection
        .execute_batch(
            "
            CREATE TRIGGER fail_late_recursive_release
            BEFORE DELETE ON reservations
            WHEN OLD.tag = 'fail'
            BEGIN
                SELECT RAISE(ABORT, 'forced late recursive release failure');
            END;
            ",
        )
        .expect("Failed to install recursive release failure trigger");
    drop(connection);

    related_command(&env)
        .arg("release")
        .arg("--path")
        .arg(&parent)
        .arg("--recursive")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "forced late recursive release failure",
        ));

    let list_output = env.list();
    assert!(
        list_output.contains(&parent_port.to_string()),
        "an earlier recursive deletion committed despite a later failure"
    );
    assert!(list_output.contains(&child_port.to_string()));
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
