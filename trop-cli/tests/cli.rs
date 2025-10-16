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
