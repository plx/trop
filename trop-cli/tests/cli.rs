//! Integration tests for the trop CLI.
//!
//! These tests verify that the CLI binary behaves correctly, including
//! argument parsing, help text, and version output.

use assert_cmd::Command;
use predicates::prelude::*;

/// Test that the binary runs without arguments and displays help/error.
#[test]
fn test_cli_no_arguments() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    // With clap subcommands required, no arguments should fail and show usage
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("Usage:"));
}

/// Test that the --version flag displays version information.
#[test]
fn test_cli_version_flag() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("--version");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("trop"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

/// Test that the -V short flag also displays version information.
#[test]
fn test_cli_version_short_flag() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("-V");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("trop"))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

/// Test that the --help flag displays help text.
#[test]
fn test_cli_help_flag() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains(
            "Manage ephemeral port reservations",
        ));
}

/// Test that the -h short flag also displays help text.
#[test]
fn test_cli_help_short_flag() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("-h");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

/// Test that an invalid subcommand produces an error.
#[test]
fn test_cli_invalid_subcommand() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("invalid-command");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

/// Test that an invalid flag produces an error.
#[test]
fn test_cli_invalid_flag() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("--invalid-flag");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

/// Test that the short `-v` flag is not supported (verbose is long-only).
#[test]
fn test_cli_short_verbose_flag_not_supported() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("-v").arg("list");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

/// Test that the short `-q` flag is not supported (quiet is long-only).
#[test]
fn test_cli_short_quiet_flag_not_supported() {
    let mut cmd = Command::cargo_bin("trop").expect("Failed to find trop binary");

    cmd.arg("-q").arg("list");

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

// ── --force flag availability on mutating commands (gh issue: #53) ──

/// Helper: assert that `trop <subcommand> --help` mentions `--force`.
fn assert_help_contains_force(subcommand: &str) {
    Command::cargo_bin("trop")
        .expect("Failed to find trop binary")
        .args([subcommand, "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--force"));
}

/// Helper: assert that `trop <subcommand> --help` mentions `--dry-run`.
fn assert_help_contains_dry_run(subcommand: &str) {
    Command::cargo_bin("trop")
        .expect("Failed to find trop binary")
        .args([subcommand, "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn test_prune_help_shows_force() {
    assert_help_contains_force("prune");
}

#[test]
fn test_expire_help_shows_force() {
    assert_help_contains_force("expire");
}

#[test]
fn test_autoclean_help_shows_force() {
    assert_help_contains_force("autoclean");
}

#[test]
fn test_init_help_shows_force() {
    assert_help_contains_force("init");
}

#[test]
fn test_reserve_help_shows_force() {
    assert_help_contains_force("reserve");
}

#[test]
fn test_release_help_shows_force() {
    assert_help_contains_force("release");
}

#[test]
fn test_reserve_group_help_shows_force() {
    assert_help_contains_force("reserve-group");
}

#[test]
fn test_autoreserve_help_shows_force() {
    assert_help_contains_force("autoreserve");
}

#[test]
fn test_exclude_help_shows_force() {
    assert_help_contains_force("exclude");
}

#[test]
fn test_compact_exclusions_help_shows_force() {
    assert_help_contains_force("compact-exclusions");
}

#[test]
fn test_migrate_help_shows_force() {
    assert_help_contains_force("migrate");
}

// Verify --dry-run on mutating commands that support it (all except exclude).
#[test]
fn test_prune_help_shows_dry_run() {
    assert_help_contains_dry_run("prune");
}

#[test]
fn test_expire_help_shows_dry_run() {
    assert_help_contains_dry_run("expire");
}

#[test]
fn test_autoclean_help_shows_dry_run() {
    assert_help_contains_dry_run("autoclean");
}

#[test]
fn test_init_help_shows_dry_run() {
    assert_help_contains_dry_run("init");
}

#[test]
fn test_exclude_help_does_not_show_dry_run() {
    Command::cargo_bin("trop")
        .expect("Failed to find trop binary")
        .args(["exclude", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run").not());
}
