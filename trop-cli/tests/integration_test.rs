//! Basic integration tests for the trop CLI.
//!
//! These are "tracer bullet" tests that verify the CLI compiles and basic
//! functionality works. Comprehensive testing is handled elsewhere.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

/// Test that the CLI binary exists and responds to --version.
#[test]
fn test_version() {
    Command::cargo_bin("trop")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("trop"));
}

/// Test that the CLI binary responds to --help.
#[test]
fn test_help() {
    Command::cargo_bin("trop")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Manage ephemeral port reservations",
        ));
}

/// Test basic reserve command - happy path.
#[test]
fn test_reserve_basic() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // Reserve a port
    let output = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--path")
        .arg(temp.path())
        .arg("--allow-unrelated-path")
        .assert()
        .success()
        .get_output()
        .clone();

    // Should output a port number
    let stdout = String::from_utf8(output.stdout).unwrap();
    let port: u16 = stdout
        .trim()
        .parse()
        .expect("Should output a valid port number");
    assert!(port > 0);
}

/// Test basic list command - empty database.
#[test]
fn test_list_empty() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // List should show header even with no reservations
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir")
        .arg(&data_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("PORT"));
}

/// Test reserve then list.
#[test]
fn test_reserve_then_list() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");
    let path = temp.path();

    // Reserve a port
    let output = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--path")
        .arg(path)
        .arg("--allow-unrelated-path")
        .assert()
        .success()
        .get_output()
        .clone();

    let port_str = String::from_utf8(output.stdout).unwrap();
    let port = port_str.trim();

    // List should show the reservation
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir")
        .arg(&data_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains(port));
}

/// Test idempotent reserve.
#[test]
fn test_reserve_idempotent() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");
    let path = temp.path();

    // First reservation
    let output1 = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--path")
        .arg(path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    let port1 = String::from_utf8(output1.stdout).unwrap();

    // Second reservation - should return same port
    let output2 = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--path")
        .arg(path)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    let port2 = String::from_utf8(output2.stdout).unwrap();

    assert_eq!(port1, port2);
}

/// Test list JSON format.
#[test]
fn test_list_json_format() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // List in JSON format (should be valid even if empty)
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--format")
        .arg("json")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["));
}
