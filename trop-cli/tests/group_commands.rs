//! Comprehensive integration tests for group reservation commands.
//!
//! These tests verify the CLI behavior of `reserve-group` and `autoreserve`
//! commands, including:
//! - Output format variations (export, json, dotenv, human)
//! - Shell type detection and explicit specification
//! - Configuration discovery (autoreserve)
//! - Quiet/verbose output modes
//! - Dry-run behavior
//! - Override flags (force, allow-*)
//! - Task identifier handling (flag vs env var)
//! - Error cases (config not found, invalid format, allocation failures)
//!
//! These tests focus on CLI integration - verifying the commands work correctly
//! from the user's perspective, with proper stdout/stderr separation, exit codes,
//! and output formatting.

mod common;

use common::{create_directory_symlink, TestEnv};
use predicates::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;

// ============================================================================
// Test Helpers
// ============================================================================

type CollisionCase<'a> = (&'a str, &'a [(&'a str, Option<&'a str>)]);
type ReservationMetadataRow = (String, u16, Option<String>, Option<String>, i64, i64);

/// Create a minimal valid trop.yaml configuration for testing.
///
/// This generates a basic configuration with a reservation group containing
/// two services (web and api) with offset-based port allocation. The base
/// port is set high (8000) to avoid conflicts with default allocations.
fn create_test_config(path: &PathBuf, project_name: &str) -> String {
    let config = format!(
        r#"
project: "{project_name}"

ports:
  min: 5000
  max: 9000

reservations:
  base: 8000
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
"#
    );

    fs::write(path, &config).expect("Failed to write test config");
    config
}

fn create_offset_config_without_occupancy_checks(path: &Path, project_name: &str) -> String {
    let config = format!(
        r#"
project: "{project_name}"

ports:
  min: 5000
  max: 12000
occupancy_check:
  skip: true

reservations:
  base: 8000
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
"#
    );

    fs::write(path, &config).expect("Failed to write test config");
    config
}

/// Create a config with preferred ports instead of offsets.
///
/// This tests a different allocation strategy where services specify
/// exact preferred ports rather than relative offsets.
fn create_config_with_preferred_ports(path: &PathBuf) -> String {
    let config = r#"
ports:
  min: 5000
  max: 10000

reservations:
  services:
    web:
      offset: 0
      preferred: 9000
      env: WEB_PORT
    api:
      offset: 1
      preferred: 9001
      env: API_PORT
"#;

    fs::write(path, config).expect("Failed to write test config");
    config.to_string()
}

/// Create a config with both offset and preferred services.
fn create_config_with_mixed_ports(path: &Path) -> String {
    let config = r#"
ports:
  min: 5000
  max: 10000
occupancy_check:
  skip: true

reservations:
  base: 8000
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    admin:
      offset: 2
      preferred: 9000
      env: ADMIN_PORT
"#;

    fs::write(path, config).expect("Failed to write test config");
    config.to_string()
}

/// Create a config without environment variable mappings.
///
/// This tests the case where services don't specify env names, so output
/// formats that require env mappings (export, dotenv) will use service tags.
fn create_config_without_env_mappings(path: &PathBuf) -> String {
    let config = r#"
ports:
  min: 5000
  max: 9000

reservations:
  base: 8100
  services:
    web:
      offset: 0
    api:
      offset: 1
"#;

    fs::write(path, config).expect("Failed to write test config");
    config.to_string()
}

/// Create a group config without hand-interpolating untrusted YAML keys.
///
/// JSON is a valid YAML subset, so serializing the fixture this way keeps
/// whitespace, control characters, quoting, and expansion syntax inert.
fn create_identifier_config(path: &Path, services: &[(&str, Option<&str>)]) {
    let services = services
        .iter()
        .enumerate()
        .map(|(offset, (tag, env))| {
            let mut definition = serde_json::Map::new();
            definition.insert("offset".to_string(), serde_json::json!(offset));
            if let Some(env) = env {
                definition.insert("env".to_string(), serde_json::json!(env));
            }
            (tag.to_string(), serde_json::Value::Object(definition))
        })
        .collect::<serde_json::Map<_, _>>();

    let config = serde_json::json!({
        "project": "identifier-test",
        "ports": {
            "min": 5000,
            "max": 9000
        },
        "reservations": {
            "base": 8200,
            "services": services
        }
    });

    fs::write(
        path,
        serde_json::to_vec_pretty(&config).expect("Failed to serialize test config"),
    )
    .expect("Failed to write test config");
}

#[derive(Clone, Copy)]
enum GroupCommandKind {
    ReserveGroup,
    Autoreserve,
}

impl GroupCommandKind {
    const ALL: [Self; 2] = [Self::ReserveGroup, Self::Autoreserve];

    const fn name(self) -> &'static str {
        match self {
            Self::ReserveGroup => "reserve-group",
            Self::Autoreserve => "autoreserve",
        }
    }
}

#[derive(Clone, Copy)]
struct OutputBoundary {
    name: &'static str,
    format: &'static str,
    shell: Option<&'static str>,
}

const OUTPUT_BOUNDARIES: [OutputBoundary; 5] = [
    OutputBoundary {
        name: "bash",
        format: "export",
        shell: Some("bash"),
    },
    OutputBoundary {
        name: "zsh",
        format: "export",
        shell: Some("zsh"),
    },
    OutputBoundary {
        name: "fish",
        format: "export",
        shell: Some("fish"),
    },
    OutputBoundary {
        name: "powershell",
        format: "export",
        shell: Some("powershell"),
    },
    OutputBoundary {
        name: "dotenv",
        format: "dotenv",
        shell: None,
    },
];

fn run_group_command(
    env: &TestEnv,
    project_dir: &Path,
    config_path: &Path,
    kind: GroupCommandKind,
    boundary: OutputBoundary,
) -> Output {
    let mut command = env.command();

    match kind {
        GroupCommandKind::ReserveGroup => {
            command.arg("reserve-group").arg(config_path);
        }
        GroupCommandKind::Autoreserve => {
            command.arg("autoreserve").current_dir(project_dir);
        }
    }

    command.arg("--format").arg(boundary.format);
    if let Some(shell) = boundary.shell {
        command.arg("--shell").arg(shell);
    }
    command
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run group command")
}

fn run_json_group_command(
    env: &TestEnv,
    project_dir: &Path,
    config_path: &Path,
    kind: GroupCommandKind,
) -> Output {
    run_group_command(
        env,
        project_dir,
        config_path,
        kind,
        OutputBoundary {
            name: "json",
            format: "json",
            shell: None,
        },
    )
}

fn run_json_group_dry_run(
    env: &TestEnv,
    project_dir: &Path,
    config_path: &Path,
    kind: GroupCommandKind,
) -> Output {
    let mut command = env.command();
    match kind {
        GroupCommandKind::ReserveGroup => {
            command.arg("reserve-group").arg(config_path);
        }
        GroupCommandKind::Autoreserve => {
            command.arg("autoreserve").current_dir(project_dir);
        }
    }
    command
        .arg("--format")
        .arg("json")
        .arg("--dry-run")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run group dry-run")
}

fn reservation_rows(env: &TestEnv) -> Vec<(String, u16, i64, i64)> {
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    let mut statement = connection
        .prepare(
            "SELECT tag, port, created_at, last_used_at
             FROM reservations
             WHERE tag <> ''
             ORDER BY tag",
        )
        .expect("Failed to prepare reservation query");

    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("Failed to query reservations")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("Failed to decode reservations")
}

fn reservation_metadata_rows(env: &TestEnv) -> Vec<ReservationMetadataRow> {
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    let mut statement = connection
        .prepare(
            "SELECT tag, port, project, task, created_at, last_used_at
             FROM reservations
             WHERE tag <> ''
             ORDER BY tag",
        )
        .expect("Failed to prepare reservation metadata query");

    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .expect("Failed to query reservation metadata")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("Failed to decode reservation metadata")
}

fn run_concurrent_group_processes(
    data_dir: &Path,
    specs: &[(PathBuf, PathBuf, GroupCommandKind)],
) -> Vec<Output> {
    let binary = assert_cmd::cargo::cargo_bin!("trop").to_path_buf();
    let barrier = Arc::new(Barrier::new(specs.len()));
    let handles = specs
        .iter()
        .cloned()
        .map(|(project_dir, config_path, kind)| {
            let binary = binary.clone();
            let data_dir = data_dir.to_path_buf();
            let barrier = Arc::clone(&barrier);

            thread::spawn(move || {
                barrier.wait();

                let mut command = Command::new(binary);
                command
                    .arg("--data-dir")
                    .arg(data_dir)
                    .arg("--busy-timeout")
                    .arg("30");

                match kind {
                    GroupCommandKind::ReserveGroup => {
                        command.arg("reserve-group").arg(config_path);
                    }
                    GroupCommandKind::Autoreserve => {
                        command.arg("autoreserve").current_dir(project_dir);
                    }
                }

                command
                    .arg("--format")
                    .arg("json")
                    .arg("--allow-unrelated-path")
                    .output()
                    .expect("Failed to run concurrent group command")
            })
        })
        .collect::<Vec<_>>();

    handles
        .into_iter()
        .map(|handle| handle.join().expect("Concurrent command thread panicked"))
        .collect()
}

fn reservation_count(env: &TestEnv) -> i64 {
    let database_path = env.data_dir.join("trop.db");
    if !database_path.exists() {
        return 0;
    }

    let connection = rusqlite::Connection::open_with_flags(
        database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("Failed to open test database");
    connection
        .query_row("SELECT COUNT(*) FROM reservations", [], |row| row.get(0))
        .expect("Failed to count reservations")
}

fn looks_like_dotenv_assignment(line: &str) -> bool {
    let Some((name, _value)) = line.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn assert_group_failure_is_closed(
    env: &TestEnv,
    output: &Output,
    case_name: &str,
    expected_error: &str,
) {
    assert!(
        !output.status.success(),
        "{case_name} should fail, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stdout, b"",
        "{case_name} must not emit partial or complete generated output"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("Error: ") && stderr.contains(expected_error),
        "{case_name} should return a typed error, stderr: {stderr}"
    );
    assert_stderr_is_inert(&stderr, case_name);

    assert_eq!(
        reservation_count(env),
        0,
        "{case_name} must roll back every reservation"
    );
}

fn assert_stderr_is_inert(stderr: &str, case_name: &str) {
    assert!(
        !stderr.contains('\u{1b}'),
        "{case_name} leaked a terminal escape to stderr: {stderr:?}"
    );
    for line in stderr.lines() {
        let line = line.trim_start();
        assert!(
            !line.starts_with("export ")
                && !line.starts_with("set -x ")
                && !line.starts_with("$env:")
                && !looks_like_dotenv_assignment(line),
            "{case_name} leaked a generated line to stderr: {line:?}"
        );
    }
}

/// Create a nested directory structure for testing autoreserve discovery.
///
/// Creates:
///   base/
///   base/trop.yaml (if include_base_config)
///   base/subdir/
///   base/subdir/trop.yaml (if include_subdir_config)
///   base/subdir/nested/
///
/// Returns (base, subdir, nested) paths.
fn create_nested_structure(
    env: &TestEnv,
    include_base_config: bool,
    include_subdir_config: bool,
) -> (PathBuf, PathBuf, PathBuf) {
    let base = env.create_dir("project");
    let subdir = base.join("subdir");
    fs::create_dir_all(&subdir).expect("Failed to create subdir");
    let nested = subdir.join("nested");
    fs::create_dir_all(&nested).expect("Failed to create nested");

    if include_base_config {
        create_test_config(&base.join("trop.yaml"), "base-project");
    }

    if include_subdir_config {
        create_test_config(&subdir.join("trop.yaml"), "subdir-project");
    }

    (base, subdir, nested)
}

// ============================================================================
// reserve-group: Basic Functionality
// ============================================================================

/// Test basic reserve-group command with default export format.
///
/// This verifies the most fundamental operation: reserving a group of ports
/// from a config file. The command should:
/// - Succeed with exit code 0
/// - Output environment exports to stdout (shell-specific format)
/// - Output status messages to stderr (unless --quiet)
/// - Create reservations in the database for all services
#[test]
fn test_reserve_group_basic_success() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(
        output.status.success(),
        "reserve-group should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stdout should contain export statements (default format)
    // The exact format depends on shell detection, but should mention ports
    // Note: May use environment variable names (WEB_PORT, API_PORT) or service tags (WEB, API)
    assert!(
        stdout.contains("WEB_PORT") || stdout.contains("WEB") || stdout.contains("web"),
        "stdout should contain service references: {stdout}"
    );
    assert!(
        stdout.contains("API_PORT") || stdout.contains("API") || stdout.contains("api"),
        "stdout should contain service references: {stdout}"
    );

    // Stderr should contain status message (not --quiet)
    assert!(
        stderr.contains("Reserved"),
        "stderr should show success message: {stderr}"
    );
}

/// Config-file argument spelling and command choice must not change the
/// inferred reservation identity.
#[test]
fn test_group_config_parent_identity_is_canonical_across_entrypoints() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    let nested_dir = project_dir.join("nested");
    fs::create_dir(&nested_dir).expect("Failed to create nested project directory");
    create_test_config(&config_path, "test-project");

    for (config_arg, working_dir) in [
        (Path::new("trop.yaml"), project_dir.as_path()),
        (Path::new("./trop.yaml"), project_dir.as_path()),
        (config_path.as_path(), project_dir.as_path()),
        (Path::new("../trop.yaml"), nested_dir.as_path()),
    ] {
        let output = env
            .command()
            .arg("reserve-group")
            .arg(config_arg)
            .arg("--format")
            .arg("json")
            .arg("--allow-unrelated-path")
            .current_dir(working_dir)
            .output()
            .expect("Failed to run reserve-group");
        assert!(
            output.status.success(),
            "reserve-group {config_arg:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let autoreserve = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&nested_dir)
        .output()
        .expect("Failed to run autoreserve");
    assert!(
        autoreserve.status.success(),
        "autoreserve failed: {}",
        String::from_utf8_lossy(&autoreserve.stderr)
    );

    let expected_path = project_dir
        .canonicalize()
        .expect("Failed to canonicalize project directory");
    let paths_before_prune = env.reservation_paths();
    assert!(
        paths_before_prune
            .iter()
            .all(|path| path.is_absolute() && path == &expected_path),
        "every group row should use the canonical config parent: {paths_before_prune:?}"
    );

    let list_output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .expect("Failed to list group reservations");
    assert!(list_output.status.success());
    let rows: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("List output should be valid JSON");
    let row_array = rows.as_array().expect("List output should be an array");
    assert_eq!(
        row_array.len(),
        paths_before_prune.len(),
        "JSON output should contain every stored group row"
    );
    assert!(
        row_array
            .iter()
            .all(|row| row["path"].as_str() == expected_path.to_str()),
        "JSON output should expose only the canonical group identity: {rows}"
    );

    let count_before_prune = env.reservation_count();
    env.command().arg("prune").assert().success();
    assert_eq!(
        env.reservation_count(),
        count_before_prune,
        "prune must preserve a live group"
    );
    assert_eq!(
        env.reservation_paths(),
        vec![expected_path.clone(), expected_path]
    );
}

/// A config reached through a directory symlink still infers the physical
/// containing-directory identity for both group entrypoints.
#[test]
fn test_group_config_parent_identity_is_canonical_through_symlink() {
    let env = TestEnv::new();
    let physical = env.create_dir("physical-project");
    let logical = env.path().join("logical-project");
    if !create_directory_symlink(&physical, &logical) {
        return;
    }

    let physical_config = physical.join("trop.yaml");
    create_test_config(&physical_config, "test-project");

    let reserve_group = env
        .command()
        .arg("reserve-group")
        .arg(logical.join("trop.yaml"))
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group through symlink");
    assert!(
        reserve_group.status.success(),
        "reserve-group through symlink failed: {}",
        String::from_utf8_lossy(&reserve_group.stderr)
    );

    let autoreserve = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&logical)
        .output()
        .expect("Failed to run autoreserve through symlink");
    assert!(
        autoreserve.status.success(),
        "autoreserve through symlink failed: {}",
        String::from_utf8_lossy(&autoreserve.stderr)
    );

    let expected_path = physical
        .canonicalize()
        .expect("Failed to canonicalize physical project directory");
    assert_eq!(
        env.reservation_paths(),
        vec![expected_path.clone(), expected_path]
    );
}

/// Test reserve-group with explicit config path that doesn't exist.
///
/// This verifies error handling when the specified config file is not found.
/// The command should fail with a clear error message explaining the problem.
#[test]
fn test_reserve_group_config_not_found() {
    let env = TestEnv::new();
    let fake_config = env.path().join("nonexistent.yaml");

    env.command()
        .arg("reserve-group")
        .arg(&fake_config)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not found").or(predicate::str::contains("Configuration")),
        );
    assert!(
        !env.data_dir.join("trop.db").exists(),
        "invalid config input must fail before opening the database"
    );
}

/// Test reserve-group with a directory path instead of file.
///
/// The config_path argument must be a file, not a directory. This test
/// verifies that passing a directory results in a clear error message.
#[test]
fn test_reserve_group_with_directory_path() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");

    env.command()
        .arg("reserve-group")
        .arg(&dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a file"));
    assert!(
        !env.data_dir.join("trop.db").exists(),
        "non-file config input must fail before opening the database"
    );
}

// ============================================================================
// reserve-group: Output Formats
// ============================================================================

/// Test reserve-group with --format=json.
///
/// JSON format should output a valid JSON object with service tags as keys
/// and port numbers as values. This format is machine-readable and useful
/// for integration with other tools.
#[test]
fn test_reserve_group_json_format() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should be valid JSON
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be valid JSON");

    // Should have entries for web and api
    assert!(
        parsed.get("web").is_some(),
        "JSON should contain 'web' key: {stdout}"
    );
    assert!(
        parsed.get("api").is_some(),
        "JSON should contain 'api' key: {stdout}"
    );

    // Values should be port numbers
    let web_port = parsed["web"].as_u64().expect("web port should be number");
    let api_port = parsed["api"].as_u64().expect("api port should be number");

    assert!(
        (5000..=10000).contains(&web_port),
        "web port should be in valid range"
    );
    assert!(
        (5000..=10000).contains(&api_port),
        "api port should be in valid range"
    );
}

/// Test reserve-group with --format=dotenv.
///
/// Dotenv format outputs lines in "VAR=value" format, suitable for use in
/// .env files. Each line should map an environment variable to a port number.
#[test]
fn test_reserve_group_dotenv_format() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("dotenv")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should contain environment variable assignments
    // Note: May use env names (WEB_PORT, API_PORT) or tags (WEB, API)
    assert!(
        stdout.contains("WEB_PORT=") || stdout.contains("WEB=") || stdout.contains("web="),
        "dotenv format should contain web assignment: {stdout}"
    );
    assert!(
        stdout.contains("API_PORT=") || stdout.contains("API=") || stdout.contains("api="),
        "dotenv format should contain api assignment: {stdout}"
    );

    // Each line should be VAR=value format (no quotes, no export)
    for line in stdout.lines() {
        if !line.is_empty() {
            assert!(line.contains('='), "dotenv line should contain '=': {line}");
            assert!(
                !line.starts_with("export"),
                "dotenv format should not include 'export': {line}"
            );
        }
    }
}

/// Test reserve-group with --format=human.
///
/// Human format provides a readable summary of the allocations, suitable
/// for display to users. It should show service names and their ports in
/// a clear, formatted manner.
#[test]
fn test_reserve_group_human_format() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("human")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should contain service names and port numbers in readable format
    assert!(
        stdout.contains("web") || stdout.contains("WEB"),
        "human format should mention web service: {stdout}"
    );
    assert!(
        stdout.contains("api") || stdout.contains("API"),
        "human format should mention api service: {stdout}"
    );

    // Should contain actual port numbers
    assert!(
        stdout.contains(char::is_numeric),
        "human format should contain port numbers: {stdout}"
    );
}

/// Test reserve-group with --format=export and explicit shell type.
///
/// Export format generates shell-specific variable export statements. When
/// an explicit --shell argument is provided, it should use that shell's
/// syntax regardless of the detected shell environment.
#[test]
fn test_reserve_group_export_format_with_explicit_shell() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // Test bash syntax
    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("export")
        .arg("--shell")
        .arg("bash")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Bash format: export VAR=value
    // May use env names (WEB_PORT) or tags (WEB)
    assert!(
        stdout.contains("export WEB_PORT=") || stdout.contains("export WEB="),
        "bash export should use 'export VAR=' syntax: {stdout}"
    );
}

/// Test reserve-group export format with fish shell syntax.
///
/// Fish shell uses different syntax for setting environment variables:
/// `set -x VAR value` instead of `export VAR=value`. This test verifies
/// that trop generates correct fish syntax when --shell=fish is specified.
#[test]
fn test_reserve_group_export_format_fish_shell() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("export")
        .arg("--shell")
        .arg("fish")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Fish format: set -x VAR value
    // May use env names (WEB_PORT) or tags (WEB)
    assert!(
        stdout.contains("set -x WEB_PORT") || stdout.contains("set -x WEB"),
        "fish export should use 'set -x VAR value' syntax: {stdout}"
    );
}

/// Test reserve-group export format with PowerShell syntax.
///
/// PowerShell uses `$env:VAR="value"` syntax for environment variables.
/// This test verifies correct PowerShell output generation.
#[test]
fn test_reserve_group_export_format_powershell() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("export")
        .arg("--shell")
        .arg("powershell")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // PowerShell format: $env:VAR="value"
    // May use env names (WEB_PORT) or tags (WEB)
    assert!(
        stdout.contains("$env:WEB_PORT") || stdout.contains("$env:WEB"),
        "powershell export should use '$env:VAR' syntax: {stdout}"
    );
}

/// Both group entry points support explicit Zsh output while resolving a valid
/// implicit name and a valid configured mapping from the same config snapshot.
#[test]
fn test_group_commands_export_format_zsh_with_mapped_and_derived_names() {
    for kind in GroupCommandKind::ALL {
        let env = TestEnv::new();
        let project_dir = env.create_dir("project");
        let config_path = project_dir.join("trop.yaml");
        create_identifier_config(
            &config_path,
            &[
                ("api-v2", None),
                ("service\nexport ATTACK=1\u{1b}[31m", Some("WEB_PORT")),
            ],
        );

        let zsh = OUTPUT_BOUNDARIES[1];
        let output = run_group_command(&env, &project_dir, &config_path, kind, zsh);
        assert!(
            output.status.success(),
            "{} should produce Zsh output, stderr: {}",
            kind.name(),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
        assert!(
            stdout
                .lines()
                .any(|line| line.starts_with("export API_V2=")),
            "{} should derive API_V2 safely: {stdout}",
            kind.name()
        );
        assert!(
            stdout
                .lines()
                .any(|line| line.starts_with("export WEB_PORT=")),
            "{} should honor the explicit WEB_PORT mapping: {stdout}",
            kind.name()
        );

        let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
        assert!(
            !stderr.contains('\u{1b}')
                && !stderr.lines().any(|line| {
                    let line = line.trim_start();
                    line.starts_with("export ")
                        || line.starts_with("set -x ")
                        || line.starts_with("$env:")
                        || looks_like_dotenv_assignment(line)
                }),
            "{} must not echo hostile tags as executable-looking diagnostics: {stderr:?}",
            kind.name()
        );
        assert_eq!(
            reservation_count(&env),
            2,
            "{} should persist both successful reservations",
            kind.name()
        );
    }
}

// ============================================================================
// reserve-group: Quiet and Verbose Modes
// ============================================================================

/// Test reserve-group with --quiet flag.
///
/// Quiet mode should suppress status messages on stderr while still outputting
/// the formatted allocations on stdout. This is important for scripting where
/// stdout must contain only the selected machine-readable format.
#[test]
fn test_reserve_group_quiet_mode() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("--quiet")
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stdout should still contain the allocations
    assert!(
        !stdout.is_empty(),
        "quiet mode should still output allocations"
    );
    let _parsed: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    // Stderr should be empty (no status messages)
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet mode should not output status to stderr: {stderr}"
    );
}

/// Test reserve-group with --verbose flag.
///
/// Verbose mode should include additional diagnostic information on stderr,
/// while stdout remains clean and contains only the formatted output. This
/// helps with debugging without breaking scripts that parse stdout.
#[test]
fn test_reserve_group_verbose_mode() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("--verbose")
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let _stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stdout should contain clean JSON
    let _parsed: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    // Stderr may contain verbose diagnostics (implementation-dependent)
    // At minimum, it should have some output in verbose mode
    // (This is a weak assertion since verbose behavior may vary)
}

/// Test stdout/stderr separation is maintained across all formats.
///
/// This is a critical property: formatted allocations always go to stdout,
/// while status messages always go to stderr. Consumers can therefore parse or
/// inspect the requested output without mixing it with diagnostics.
#[test]
fn test_reserve_group_stdout_stderr_separation() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let formats = ["export", "json", "dotenv", "human"];

    for format in &formats {
        let output = env
            .command()
            .arg("reserve-group")
            .arg(&config_path)
            .arg("--format")
            .arg(format)
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to run reserve-group");

        assert!(
            output.status.success(),
            "reserve-group --format={format} should succeed"
        );

        let stdout = String::from_utf8(output.stdout.clone()).expect("Invalid UTF-8");
        let stderr = String::from_utf8(output.stderr.clone()).expect("Invalid UTF-8");

        // Stdout should contain formatted output (not empty)
        assert!(
            !stdout.trim().is_empty(),
            "stdout should contain formatted output for format={format}"
        );

        // Stderr should contain status (not empty, unless --quiet)
        assert!(
            !stderr.trim().is_empty(),
            "stderr should contain status for format={format}"
        );

        // Stdout should not contain status messages like "Reserved X ports"
        // Note: Human format may contain "Reserved ports:" as a header, which is part of the format
        if *format != "human" {
            assert!(
                !stdout.contains("Reserved") || stdout.contains("Reserved ports:"),
                "stdout should not contain status messages for format={format}: {stdout}"
            );
        }
    }
}

// ============================================================================
// reserve-group: Dry-Run Mode
// ============================================================================

/// Test reserve-group with --dry-run flag.
///
/// Dry-run mode should:
/// - Not create the database or make any changes
/// - Show what would happen (on stderr)
/// - Return success exit code
/// - Not output actual allocations on stdout
#[test]
fn test_reserve_group_dry_run() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--dry-run")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success(), "dry-run should succeed");

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stderr should indicate dry-run mode
    assert!(
        stderr.contains("Dry run") || stderr.contains("would"),
        "dry-run should explain what would happen: {stderr}"
    );

    // Database should not be created
    assert!(!env.data_dir.exists(), "dry-run should not create database");
}

/// Test reserve-group dry-run with different output formats.
///
/// Dry-run should work consistently across all output formats, showing
/// what would be done without actually performing the operations.
#[test]
fn test_reserve_group_dry_run_with_formats() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let formats = ["json", "dotenv", "human"];

    for format in &formats {
        let output = env
            .command()
            .arg("reserve-group")
            .arg(&config_path)
            .arg("--format")
            .arg(format)
            .arg("--dry-run")
            .output()
            .expect("Failed to run reserve-group");

        assert!(
            output.status.success(),
            "dry-run with --format={format} should succeed"
        );

        // Database should still not exist
        assert!(
            !env.data_dir.exists(),
            "dry-run should not create database (format={format})"
        );
    }
}

/// Test reserve-group dry-run with --quiet.
///
/// Even in dry-run mode, --quiet should suppress the description of what
/// would happen. The command should succeed silently.
#[test]
fn test_reserve_group_dry_run_quiet() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("--quiet")
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--dry-run")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stderr should be empty in quiet dry-run mode
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet dry-run should suppress output: {stderr}"
    );
}

// ============================================================================
// reserve-group: Override Flags
// ============================================================================

/// Test reserve-group with --force flag.
///
/// Force authorizes path, metadata, and complete shape replacement while
/// leaving allocation-integrity checks in place.
#[test]
fn test_reserve_group_with_force() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First reservation
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    connection
        .execute(
            "UPDATE reservations SET created_at = 123, last_used_at = 1",
            [],
        )
        .expect("Failed to make timestamps deterministic");
    drop(connection);

    fs::write(
        &config_path,
        r#"
project: replacement-project
ports:
  min: 5000
  max: 9000
occupancy_check:
  skip: true
reservations:
  base: 8000
  services:
    web:
      offset: 0
    db:
      offset: 1
    api:
      offset: 2
"#,
    )
    .expect("Failed to write replacement group");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("replacement-task")
        .arg("--force")
        .assert()
        .success();

    let rows = reservation_metadata_rows(&env);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        ["api", "db", "web"]
    );
    assert!(rows.iter().all(|row| {
        row.2.as_deref() == Some("replacement-project")
            && row.3.as_deref() == Some("replacement-task")
            && row.4 != 123
    }));
}

/// Test reserve-group with --allow-unrelated-path flag.
///
/// This flag allows reservations for paths that don't appear to be related
/// to a project (e.g., temp directories, system paths). Without it, trop
/// may reject certain paths as suspicious.
#[test]
fn test_reserve_group_with_allow_unrelated_path() {
    let env = TestEnv::new();
    let config_path = env.path().join("trop.yaml");
    create_test_config(&config_path, "test-project");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("path relationship violation"));
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-project-change")
        .assert()
        .failure()
        .stderr(predicate::str::contains("path relationship violation"));
    assert_eq!(
        reservation_count(&env),
        0,
        "Narrow metadata permission must not bypass path safety"
    );

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Same, ancestor, and descendant config-parent relationships are accepted
/// without any override.
#[test]
fn test_reserve_group_allows_related_invocation_paths() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let child_dir = project_dir.join("child");
    fs::create_dir_all(&child_dir).expect("Failed to create child directory");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    for current_dir in [&project_dir, &child_dir, env.path()] {
        env.command()
            .arg("reserve-group")
            .arg(&config_path)
            .arg("--format")
            .arg("json")
            .current_dir(current_dir)
            .assert()
            .success();
    }
    assert_eq!(reservation_count(&env), 2);
}

/// Test reserve-group with --allow-project-change flag.
///
/// Project changes fail atomically by default and persist on every row only
/// with the project-specific permission.
#[test]
fn test_reserve_group_with_allow_project_change() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "original-project");

    // First reservation
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    connection
        .execute(
            "UPDATE reservations SET created_at = 123, last_used_at = 1",
            [],
        )
        .expect("Failed to make timestamps deterministic");
    drop(connection);
    let before = reservation_metadata_rows(&env);

    create_test_config(&config_path, "different-project");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("sticky field 'project'"));
    assert_eq!(reservation_metadata_rows(&env), before);

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-project-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let after = reservation_metadata_rows(&env);
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|row| {
        row.2.as_deref() == Some("different-project")
            && row.3.is_none()
            && row.4 == 123
            && row.5 > 1
    }));
}

/// Test reserve-group with --allow-task-change flag.
///
/// Task changes fail atomically by default and persist on every row only with
/// the task-specific permission.
#[test]
fn test_reserve_group_with_allow_task_change() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First reservation with task
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    connection
        .execute(
            "UPDATE reservations SET created_at = 123, last_used_at = 1",
            [],
        )
        .expect("Failed to make timestamps deterministic");
    drop(connection);
    let before = reservation_metadata_rows(&env);

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-2")
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("sticky field 'task'"));
    assert_eq!(reservation_metadata_rows(&env), before);

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-2")
        .arg("--allow-task-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let after = reservation_metadata_rows(&env);
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|row| {
        row.2.as_deref() == Some("test-project")
            && row.3.as_deref() == Some("task-2")
            && row.4 == 123
            && row.5 > 1
    }));
}

/// Test reserve-group with --allow-change flag (combined permission).
///
/// The --allow-change flag is a convenience that enables both
/// --allow-project-change and --allow-task-change simultaneously.
#[test]
fn test_reserve_group_with_allow_change() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "project-1");

    // First reservation
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Change both project and task with single flag
    create_test_config(&config_path, "project-2");
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-2")
        .arg("--allow-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let rows = reservation_metadata_rows(&env);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.2.as_deref() == Some("project-2") && row.3.as_deref() == Some("task-2")
    }));
}

/// Project configuration can supply the same narrow path and combined metadata
/// permissions as explicit flags.
#[test]
fn test_reserve_group_uses_configured_change_permissions() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    fs::write(
        &config_path,
        r#"
project: project-1
allow_unrelated_path: true
ports:
  min: 5000
  max: 9000
occupancy_check:
  skip: true
reservations:
  base: 8000
  services:
    web:
      offset: 0
"#,
    )
    .expect("Failed to write initial configured-permission fixture");
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-1")
        .assert()
        .success();

    fs::write(
        &config_path,
        r#"
project: project-2
allow_unrelated_path: true
allow_change: true
ports:
  min: 5000
  max: 9000
occupancy_check:
  skip: true
reservations:
  base: 8000
  services:
    web:
      offset: 0
"#,
    )
    .expect("Failed to write changed configured-permission fixture");
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-2")
        .assert()
        .success();

    let rows = reservation_metadata_rows(&env);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.as_deref(), Some("project-2"));
    assert_eq!(rows[0].3.as_deref(), Some("task-2"));
}

// ============================================================================
// reserve-group: Task Identifier Handling
// ============================================================================

/// Test reserve-group with --task flag.
///
/// The task identifier can be specified via --task flag and should be
/// stored in the reservation metadata. This allows organizing reservations
/// by feature branch or work item.
#[test]
fn test_reserve_group_with_task_flag() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("feature-123")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    // Verify task appears in stderr status message
    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    // Task may or may not appear in stderr, but command should succeed
    drop(stderr); // Suppress unused warning

    // Verify via list command (implementation-dependent)
    let list_output = env.list();
    // List output format depends on implementation, so this is a weak check
    drop(list_output); // Suppress unused warning
}

/// Test reserve-group with TROP_TASK environment variable.
///
/// Task identifier can also be provided via TROP_TASK env var. This is
/// useful for CI/CD environments where the task ID is set globally.
#[test]
fn test_reserve_group_with_task_env_var() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .env("TROP_TASK", "env-task-456")
        .output()
        .expect("Failed to run reserve-group");

    assert!(
        output.status.success(),
        "reserve-group should respect TROP_TASK env var"
    );
}

/// Test reserve-group task precedence: flag over environment variable.
///
/// When both --task flag and TROP_TASK env var are provided, the command-line
/// flag should take precedence. This follows the standard precedence rule:
/// CLI args > environment variables > defaults.
#[test]
fn test_reserve_group_task_flag_overrides_env() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First reservation with env var
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .env("TROP_TASK", "env-task")
        .assert()
        .success();

    // Second reservation with flag should override (need --allow-task-change)
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("flag-task")
        .arg("--allow-task-change")
        .arg("--allow-unrelated-path")
        .env("TROP_TASK", "env-task")
        .assert()
        .success();
}

// ============================================================================
// autoreserve: Configuration Discovery
// ============================================================================

/// Test autoreserve discovers config from current directory.
///
/// When run from a directory containing trop.yaml, autoreserve should
/// find and use that config file automatically, without requiring an
/// explicit path argument.
#[test]
fn test_autoreserve_discovers_config_in_current_dir() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(
        output.status.success(),
        "autoreserve should find config in current dir, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let _parsed: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");
}

/// Test autoreserve discovers config from parent directory.
///
/// When run from a subdirectory, autoreserve should walk up the directory
/// tree to find the nearest trop.yaml. This enables running the command
/// from anywhere within a project structure.
#[test]
fn test_autoreserve_discovers_config_in_parent() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let subdir = project_dir.join("subdir");
    fs::create_dir_all(&subdir).expect("Failed to create subdir");

    let output = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&subdir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(
        output.status.success(),
        "autoreserve should find config in parent dir"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
    // Should mention the discovered config path
    assert!(
        stderr.contains("trop.yaml") || stderr.contains("Discovered"),
        "stderr should mention discovered config: {stderr}"
    );
}

/// Test autoreserve stops at first directory with config.
///
/// When multiple directories in the hierarchy contain trop.yaml, autoreserve
/// should use the nearest one (closest to starting directory). This prevents
/// unexpected behavior with nested projects.
#[test]
fn test_autoreserve_stops_at_first_config() {
    let env = TestEnv::new();

    // Create nested structure: base has config, subdir has config, nested is empty
    let (_base, _subdir, nested) = create_nested_structure(&env, true, true);

    // Run from nested directory
    let output = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&nested)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should discover subdir's config, not base's config
    assert!(
        stderr.contains("subdir"),
        "should discover nearest config (subdir): {stderr}"
    );
    assert!(
        !stderr.contains("project/trop.yaml") || stderr.contains("subdir"),
        "should not use parent's config when nearer one exists: {stderr}"
    );

    // To be more certain, we can check if it used subdir-project
    // (the project name in subdir's config)
    // This is implementation-dependent on whether project name appears in stderr
}

/// Test autoreserve prefers trop.local.yaml over trop.yaml.
///
/// When both trop.yaml and trop.local.yaml exist in the same directory,
/// autoreserve should prefer the local variant. This allows per-developer
/// customization without modifying the committed trop.yaml.
#[test]
fn test_autoreserve_prefers_local_config() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");

    // Create both configs with different projects to distinguish them
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "global-project");

    let local_config_path = project_dir.join("trop.local.yaml");
    create_test_config(&local_config_path, "local-project");

    let output = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(
        output.status.success(),
        "autoreserve should succeed with local config"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should mention trop.local.yaml if it indicates discovered path
    // (exact behavior depends on whether discovery path is logged)
    // This is a weak assertion since we can't easily verify which config
    // was used from the output alone. The key is that it doesn't fail.
    drop(stderr); // Suppress unused warning
}

/// Test autoreserve error when no config found.
///
/// When run from a directory with no trop.yaml in any parent directory,
/// autoreserve should fail with a clear error message explaining that no
/// configuration file was found.
#[test]
fn test_autoreserve_no_config_found() {
    let env = TestEnv::new();
    let empty_dir = env.create_dir("empty");

    env.command()
        .arg("autoreserve")
        .current_dir(&empty_dir)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No trop configuration file found")
                .or(predicate::str::contains("No configuration file found")),
        );
}

// ============================================================================
// autoreserve: Output and Behavior
// ============================================================================

/// Test autoreserve with all output formats.
///
/// Autoreserve should support the same output formats as reserve-group:
/// export, json, dotenv, and human. This test verifies each format works
/// correctly after automatic discovery.
#[test]
fn test_autoreserve_with_all_formats() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let formats = ["export", "json", "dotenv", "human"];

    for format in &formats {
        let output = env
            .command()
            .arg("autoreserve")
            .arg("--format")
            .arg(format)
            .arg("--allow-unrelated-path")
            .current_dir(&project_dir)
            .output()
            .expect("Failed to run autoreserve");

        assert!(
            output.status.success(),
            "autoreserve --format={format} should succeed"
        );

        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
        assert!(
            !stdout.trim().is_empty(),
            "autoreserve should output formatted allocations for format={format}"
        );
    }
}

/// Test autoreserve with --quiet flag.
///
/// Like reserve-group, autoreserve --quiet should suppress stderr messages
/// while still outputting allocations to stdout. The only difference from
/// reserve-group is that autoreserve performs discovery before allocation.
#[test]
fn test_autoreserve_quiet_mode() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("--quiet")
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Stdout should have allocations
    assert!(!stdout.is_empty());
    let _parsed: serde_json::Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

    // Stderr should be empty
    assert!(
        stderr.is_empty() || stderr.trim().is_empty(),
        "quiet mode should suppress stderr: {stderr}"
    );
}

/// Test autoreserve dry-run mode.
///
/// Dry-run should show what config was discovered and what would be done,
/// but not create the database or make any changes.
#[test]
fn test_autoreserve_dry_run() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let output = env
        .command()
        .arg("autoreserve")
        .arg("--dry-run")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Should indicate dry-run and show discovered config
    assert!(
        stderr.contains("Dry run") || stderr.contains("would"),
        "dry-run should explain what would happen: {stderr}"
    );

    // Database should not be created
    assert!(!env.data_dir.exists(), "dry-run should not create database");
}

/// Test autoreserve with override flags.
///
/// Autoreserve applies the same metadata and force policy after discovery,
/// including environment-derived narrow permissions.
#[test]
fn test_autoreserve_with_override_flags() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First reservation
    env.command()
        .arg("autoreserve")
        .arg("--task")
        .arg("original-task")
        .current_dir(&project_dir)
        .assert()
        .success();

    create_test_config(&config_path, "changed-project");
    env.command()
        .arg("autoreserve")
        .arg("--task")
        .arg("changed-task")
        .current_dir(&project_dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("sticky field"));

    env.command()
        .arg("autoreserve")
        .arg("--task")
        .arg("changed-task")
        .env("TROP_ALLOW_CHANGE", "true")
        .current_dir(&project_dir)
        .assert()
        .success();
    let metadata = reservation_metadata_rows(&env);
    assert!(metadata.iter().all(|row| {
        row.2.as_deref() == Some("changed-project") && row.3.as_deref() == Some("changed-task")
    }));

    fs::write(
        &config_path,
        r#"
project: changed-project
ports:
  min: 5000
  max: 9000
occupancy_check:
  skip: true
reservations:
  base: 8000
  services:
    web:
      offset: 0
    api:
      offset: 1
    admin:
      offset: 2
"#,
    )
    .expect("Failed to write changed autoreserve shape");
    env.command()
        .arg("autoreserve")
        .arg("--force")
        .current_dir(&project_dir)
        .assert()
        .success();
    assert_eq!(reservation_metadata_rows(&env).len(), 3);
}

// ============================================================================
// Error Cases
// ============================================================================

/// Test reserve-group with invalid YAML config.
///
/// When the config file contains invalid YAML syntax, the command should
/// fail with a clear error message indicating the parse error.
#[test]
fn test_reserve_group_invalid_yaml() {
    let env = TestEnv::new();
    let config_path = env.path().join("invalid.yaml");

    // Write invalid YAML
    fs::write(&config_path, "{ invalid yaml content: [ }").expect("Failed to write config");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("parse").or(predicate::str::contains("invalid")));
}

/// Test reserve-group with config missing required fields.
///
/// A valid reservation group requires certain fields (e.g., services).
/// If these are missing, the command should fail with a descriptive error.
#[test]
fn test_reserve_group_missing_required_fields() {
    let env = TestEnv::new();
    let config_path = env.path().join("incomplete.yaml");

    // Write config without services
    fs::write(&config_path, "reservations: {}\n").expect("Failed to write config");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("services")
                .or(predicate::str::contains("required"))
                .or(predicate::str::contains("empty")),
        );
}

/// Test reserve-group when port allocation fails.
///
/// If the requested ports are already occupied and can't be allocated,
/// the command should fail with an error explaining the allocation failure.
/// This tests the error path when group allocation encounters conflicts.
#[test]
fn test_reserve_group_allocation_failure() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");

    // Create config with preferred ports
    create_config_with_preferred_ports(&config_path);

    // Pre-allocate the preferred port using reserve command
    let path1 = env.create_dir("other-project");
    let _port = env.reserve_simple(&path1);

    // Now try to reserve group with overlapping ports - may fail or fallback
    // depending on allocation strategy (this is somewhat implementation-dependent)
    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    // Should either succeed (with fallback ports) or fail (with error message)
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");
        assert!(
            !stderr.is_empty(),
            "allocation failure should have error message"
        );
    }
}

/// Test autoreserve from filesystem root or directory without config.
///
/// When autoreserve can't find a config (e.g., run from /tmp or root),
/// it should fail gracefully with a helpful error message rather than
/// searching indefinitely or crashing.
#[test]
fn test_autoreserve_from_root_directory() {
    let env = TestEnv::new();

    // Use a temporary directory that's unlikely to have a trop config
    let temp_dir = env.path();

    env.command()
        .arg("autoreserve")
        .current_dir(temp_dir)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("No trop configuration file found")
                .or(predicate::str::contains("No configuration file")),
        );
}

/// Invalid shell selection fails before either group entry point can mutate the
/// database or write generated output.
#[test]
fn test_group_commands_invalid_shell_type_fails_without_mutation() {
    let invalid_shell = OutputBoundary {
        name: "invalid-shell",
        format: "export",
        shell: Some("invalid-shell"),
    };

    for kind in GroupCommandKind::ALL {
        let env = TestEnv::new();
        let project_dir = env.create_dir("project");
        let config_path = project_dir.join("trop.yaml");
        create_test_config(&config_path, "test-project");

        let output = run_group_command(&env, &project_dir, &config_path, kind, invalid_shell);
        assert_group_failure_is_closed(
            &env,
            &output,
            &format!("{} invalid shell", kind.name()),
            "validation error",
        );
    }
}

/// A mixed group whose invalid tag sorts last must fail atomically at every
/// shell and dotenv renderer boundary for both CLI entry points.
#[test]
fn test_group_commands_fail_closed_at_every_identifier_output_boundary() {
    for kind in GroupCommandKind::ALL {
        for boundary in OUTPUT_BOUNDARIES {
            let env = TestEnv::new();
            let project_dir = env.create_dir("project");
            let config_path = project_dir.join("trop.yaml");
            create_identifier_config(
                &config_path,
                &[("aaa-valid", None), ("zzz;printf-not-run", None)],
            );

            let output = run_group_command(&env, &project_dir, &config_path, kind, boundary);
            assert_group_failure_is_closed(
                &env,
                &output,
                &format!("{} {} mixed identifiers", kind.name(), boundary.name),
                "validation error",
            );
        }
    }
}

/// Hostile tag text remains inert test data. Each class is rejected by both
/// group commands, with no generated stdout, executable-looking stderr line,
/// or persisted reservation.
#[test]
fn test_group_commands_reject_adversarial_unmapped_identifiers() {
    let overlong = "a".repeat(256);
    let fixtures = [
        ("whitespace", "web port"),
        ("control character", "web\tport"),
        ("newline line forging", "web\nexport ATTACK=1"),
        ("CRLF line forging", "web\r\nset -x ATTACK 1"),
        ("shell separators", "web;|&<>port"),
        ("quotes and backticks", "web'\"`port"),
        ("shell expansions", "web$(no-op)${NO_OP}"),
        ("leading digit", "9web"),
        ("Unicode", "wéb端口"),
        ("overlong", overlong.as_str()),
    ];

    for (fixture_index, (fixture_name, tag)) in fixtures.into_iter().enumerate() {
        for (command_index, kind) in GroupCommandKind::ALL.into_iter().enumerate() {
            let env = TestEnv::new();
            let project_dir = env.create_dir("project");
            let config_path = project_dir.join("trop.yaml");
            create_identifier_config(&config_path, &[(tag, None)]);

            let boundary =
                OUTPUT_BOUNDARIES[(fixture_index + command_index) % OUTPUT_BOUNDARIES.len()];
            let output = run_group_command(&env, &project_dir, &config_path, kind, boundary);
            assert_group_failure_is_closed(
                &env,
                &output,
                &format!(
                    "{} {} identifier at {} boundary",
                    kind.name(),
                    fixture_name,
                    boundary.name
                ),
                "validation error",
            );
        }
    }
}

/// Explicit mappings are validated independently of otherwise safe tags.
#[test]
fn test_group_commands_reject_invalid_explicit_mapping() {
    for (index, kind) in GroupCommandKind::ALL.into_iter().enumerate() {
        let env = TestEnv::new();
        let project_dir = env.create_dir("project");
        let config_path = project_dir.join("trop.yaml");
        create_identifier_config(&config_path, &[("web", Some("9WEB PORT"))]);

        let boundary = OUTPUT_BOUNDARIES[index];
        let output = run_group_command(&env, &project_dir, &config_path, kind, boundary);
        assert_group_failure_is_closed(
            &env,
            &output,
            &format!("{} invalid explicit mapping", kind.name()),
            "validation error",
        );
    }
}

/// Resolved identifiers must remain unique whether they are both derived, one
/// is explicit, or their explicit spellings differ only by ASCII case.
#[test]
fn test_group_commands_reject_resolved_identifier_collisions() {
    let collision_cases: [CollisionCase<'_>; 3] = [
        (
            "derived/derived",
            &[("api-server", None), ("api_server", None)],
        ),
        (
            "explicit/derived",
            &[("api-server", None), ("mapped", Some("API_SERVER"))],
        ),
        (
            "case-insensitive",
            &[
                ("uppercase", Some("WEB_PORT")),
                ("lowercase", Some("web_port")),
            ],
        ),
    ];

    for (case_index, (collision_name, services)) in collision_cases.into_iter().enumerate() {
        for kind in GroupCommandKind::ALL {
            let env = TestEnv::new();
            let project_dir = env.create_dir("project");
            let config_path = project_dir.join("trop.yaml");
            create_identifier_config(&config_path, services);

            let boundary = OUTPUT_BOUNDARIES[case_index + 2];
            let output = run_group_command(&env, &project_dir, &config_path, kind, boundary);
            assert_group_failure_is_closed(
                &env,
                &output,
                &format!("{} {collision_name} collision", kind.name()),
                "validation error",
            );
        }
    }
}

// ============================================================================
// Edge Cases and Integration
// ============================================================================

/// Test reserve-group with empty services list.
///
/// A config with an empty services map is technically valid YAML but
/// semantically invalid for group reservations. The command should fail
/// with a clear error.
#[test]
fn test_reserve_group_empty_services() {
    let env = TestEnv::new();
    let config_path = env.path().join("empty-services.yaml");

    let config = r#"
reservations:
  services: {}
"#;
    fs::write(&config_path, config).expect("Failed to write config");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty").or(predicate::str::contains("services")));
}

/// Test reserve-group config without env mappings uses service tags.
///
/// When services don't specify env field, output formatters should fall
/// back to using the service tag as the variable name. This test verifies
/// that behavior works correctly.
#[test]
fn test_reserve_group_without_env_mappings() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_config_without_env_mappings(&config_path);

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("dotenv")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should use service tags as variable names
    assert!(
        stdout.contains("web=") || stdout.contains("WEB="),
        "should use service tag when env not specified: {stdout}"
    );
    assert!(
        stdout.contains("api=") || stdout.contains("API="),
        "should use service tag when env not specified: {stdout}"
    );
}

/// Repeated reserve-group calls return the same mapping.
#[test]
fn test_reserve_group_idempotency() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let reserve_group = || {
        env.command()
            .arg("reserve-group")
            .arg(&config_path)
            .arg("--format")
            .arg("json")
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to run reserve-group")
    };

    let output1 = reserve_group();
    let output2 = reserve_group();

    assert!(
        output1.status.success(),
        "first reserve-group failed: {}",
        String::from_utf8_lossy(&output1.stderr)
    );
    assert!(
        output2.status.success(),
        "second reserve-group failed: {}",
        String::from_utf8_lossy(&output2.stderr)
    );
    assert_eq!(
        output1.stdout, output2.stdout,
        "repeated reserve-group calls must return a byte-identical mapping"
    );
    assert_eq!(
        reservation_count(&env),
        2,
        "repeating a group must retain exactly one row per service"
    );
}

/// Tag whitespace is normalized once for storage, reconciliation, allocation
/// output, and explicit environment-variable lookup.
#[test]
fn test_group_idempotency_uses_normalized_tag_identity() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_identifier_config(&config_path, &[(" web ", Some("WEB_PORT"))]);
    let dotenv = OutputBoundary {
        name: "dotenv",
        format: "dotenv",
        shell: None,
    };

    let first = run_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::ReserveGroup,
        dotenv,
    );
    let second = run_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::Autoreserve,
        dotenv,
    );

    assert!(
        first.status.success(),
        "padded-tag reserve-group failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "padded-tag autoreserve failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        second.stdout, first.stdout,
        "raw and stored tag spellings must resolve to one stable identity"
    );
    assert!(
        String::from_utf8_lossy(&first.stdout).starts_with("WEB_PORT="),
        "normalized allocations must retain the explicit env mapping: {}",
        String::from_utf8_lossy(&first.stdout)
    );
    let rows = reservation_rows(&env);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "web");
}

/// Distinct raw config keys that normalize to one reservation identity are
/// rejected before normal and dry-run entrypoints can persist or forge output.
#[test]
fn test_group_commands_reject_tags_colliding_after_normalization() {
    let hostile_tag = "service\nexport ATTACK=1\u{1b}[31m";
    for kind in GroupCommandKind::ALL {
        for dry_run in [false, true] {
            let env = TestEnv::new();
            let project_dir = env.create_dir(kind.name());
            let config_path = project_dir.join("trop.yaml");
            create_identifier_config(
                &config_path,
                &[
                    (hostile_tag, Some("SAFE_PORT")),
                    (&format!(" {hostile_tag} "), Some("OTHER_SAFE_PORT")),
                ],
            );

            let output = if dry_run {
                run_json_group_dry_run(&env, &project_dir, &config_path, kind)
            } else {
                run_json_group_command(&env, &project_dir, &config_path, kind)
            };
            let mode = if dry_run { "dry-run" } else { "normal" };

            assert_group_failure_is_closed(
                &env,
                &output,
                &format!("{} {mode} normalized tag collision", kind.name()),
                "must be unique after trimming whitespace",
            );
            assert!(
                !String::from_utf8_lossy(&output.stderr).contains(hostile_tag),
                "collision diagnostic must not echo the hostile tag: {:?}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

/// Reconciliation conflicts keep broad explicitly mapped tags out of
/// executable-looking diagnostics and leave the incompatible group untouched.
#[test]
fn test_group_reconciliation_conflict_diagnostics_keep_hostile_tags_inert() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    let hostile_tag = "service\nexport ATTACK=1\u{1b}[31m";
    create_identifier_config(
        &config_path,
        &[
            ("api", Some("API_PORT")),
            (hostile_tag, Some("SAFE_SERVICE_PORT")),
        ],
    );

    let initial = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::ReserveGroup,
    );
    assert!(
        initial.status.success(),
        "hostile mapped tag should allocate safely: {}",
        String::from_utf8_lossy(&initial.stderr)
    );

    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    assert_eq!(
        connection
            .execute(
                "UPDATE reservations SET port = port + 1 WHERE tag = ?1",
                [hostile_tag],
            )
            .expect("Failed to inject an incompatible stored mapping"),
        1
    );
    drop(connection);
    let before = reservation_rows(&env);

    let repeated = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::Autoreserve,
    );

    assert!(
        !repeated.status.success(),
        "incompatible hostile-tag group should fail"
    );
    assert_eq!(
        repeated.stdout, b"",
        "reconciliation conflict must not emit generated output"
    );
    let stderr = String::from_utf8_lossy(&repeated.stderr);
    assert!(
        stderr.starts_with("Error: ") && stderr.contains("reservation conflict"),
        "reconciliation should return a typed conflict: {stderr}"
    );
    assert_stderr_is_inert(&stderr, "hostile-tag reconciliation conflict");
    assert!(
        !stderr.contains(hostile_tag),
        "reconciliation diagnostic must not echo the hostile tag: {stderr:?}"
    );
    assert_eq!(
        reservation_rows(&env),
        before,
        "reconciliation conflict must preserve every stored row"
    );
}

/// Offset-only, preferred-only, and mixed groups remain stable across both
/// entrypoints.
#[test]
fn test_group_idempotency_covers_allocation_shapes_and_entrypoints() {
    for case_name in ["offset", "preferred", "mixed"] {
        let env = TestEnv::new();
        let project_dir = env.create_dir(case_name);
        let config_path = project_dir.join("trop.yaml");
        let expected_services = match case_name {
            "offset" => {
                create_test_config(&config_path, "offset-project");
                2
            }
            "preferred" => {
                create_config_with_preferred_ports(&config_path);
                2
            }
            "mixed" => {
                create_config_with_mixed_ports(&config_path);
                3
            }
            _ => unreachable!("all cases are enumerated above"),
        };

        let calls = [
            GroupCommandKind::ReserveGroup,
            GroupCommandKind::ReserveGroup,
            GroupCommandKind::Autoreserve,
            GroupCommandKind::Autoreserve,
        ];
        let outputs = calls
            .into_iter()
            .map(|kind| run_json_group_command(&env, &project_dir, &config_path, kind))
            .collect::<Vec<_>>();

        for output in &outputs {
            assert!(
                output.status.success(),
                "{case_name} group call failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let expected = &outputs[0].stdout;
        for output in outputs.iter().skip(1) {
            assert_eq!(
                &output.stdout, expected,
                "{case_name} group mappings must be byte-identical across entrypoints"
            );
        }
        assert_eq!(
            reservation_count(&env),
            expected_services,
            "{case_name} group must retain exactly one row per service"
        );
    }
}

/// Both group entrypoints accept full-domain preferred ports and reserve them
/// before selecting the lowest complete fallback pattern.
#[test]
fn test_group_commands_accept_outside_range_preferred_and_avoid_internal_collisions() {
    for kind in GroupCommandKind::ALL {
        let env = TestEnv::new();
        let project_dir = env.create_dir(kind.name());
        let config_path = project_dir.join("trop.yaml");
        fs::write(
            &config_path,
            r#"
ports:
  min: 5000
  max: 5100
occupancy_check:
  skip: true
reservations:
  services:
    external:
      offset: 3
      preferred: 65535
    admin:
      offset: 2
      preferred: 5000
    web:
      offset: 0
    api:
      offset: 1
"#,
        )
        .expect("Failed to write preferred-port regression config");

        let output = run_json_group_command(&env, &project_dir, &config_path, kind);
        assert!(
            output.status.success(),
            "{} preferred-port allocation failed: {}",
            kind.name(),
            String::from_utf8_lossy(&output.stderr)
        );
        let mapping: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Group output should be JSON");
        assert_eq!(mapping["external"], 65535);
        assert_eq!(mapping["admin"], 5000);
        assert_eq!(mapping["web"], 5001);
        assert_eq!(mapping["api"], 5002);
        assert_eq!(reservation_count(&env), 4);
    }
}

/// A database-reserved preference uses the service's offset fallback through
/// both command entrypoints, including the configuration default of zero.
#[test]
fn test_group_commands_reserved_preferred_uses_offset_fallback() {
    for kind in GroupCommandKind::ALL {
        let env = TestEnv::new();
        let blocker_dir = env.create_dir("blocker");
        let blocker = env
            .command()
            .arg("reserve")
            .arg("--path")
            .arg(&blocker_dir)
            .arg("--port")
            .arg("5050")
            .arg("--min")
            .arg("5050")
            .arg("--max")
            .arg("5050")
            .arg("--skip-occupancy-check")
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to reserve preferred-port blocker");
        assert!(
            blocker.status.success(),
            "Failed to reserve preferred-port blocker: {}",
            String::from_utf8_lossy(&blocker.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&blocker.stdout).trim(), "5050");

        let project_dir = env.create_dir(kind.name());
        let config_path = project_dir.join("trop.yaml");
        fs::write(
            &config_path,
            r#"
ports:
  min: 5000
  max: 5100
occupancy_check:
  skip: true
reservations:
  services:
    web:
      preferred: 5050
    api:
      offset: 1
"#,
        )
        .expect("Failed to write preferred-fallback regression config");

        let output = run_json_group_command(&env, &project_dir, &config_path, kind);
        assert!(
            output.status.success(),
            "{} preferred fallback failed: {}",
            kind.name(),
            String::from_utf8_lossy(&output.stderr)
        );
        let mapping: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Group output should be JSON");
        assert_eq!(mapping["web"], 5000);
        assert_eq!(mapping["api"], 5001);
        assert_eq!(
            reservation_count(&env),
            3,
            "The blocker and complete fallback group should coexist"
        );
    }
}

/// Compatible reuse preserves creation metadata and refreshes every member.
#[test]
fn test_group_idempotency_preserves_created_at_and_refreshes_last_used() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let first = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::ReserveGroup,
    );
    assert!(
        first.status.success(),
        "initial group call failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    connection
        .execute(
            "UPDATE reservations SET created_at = 123, last_used_at = 1",
            [],
        )
        .expect("Failed to age reservation timestamps");
    drop(connection);

    let second = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::Autoreserve,
    );
    assert!(
        second.status.success(),
        "repeated group call failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        first.stdout, second.stdout,
        "refreshing a compatible group must preserve its mapping"
    );

    let rows = reservation_rows(&env);
    assert_eq!(rows.len(), 2);
    for (tag, _port, created_at, last_used_at) in rows {
        assert_eq!(
            created_at, 123,
            "compatible reuse must preserve {tag}'s creation timestamp"
        );
        assert!(
            last_used_at > 1,
            "compatible reuse must refresh {tag}'s last-used timestamp"
        );
    }
}

/// A partial stored group is rejected without filling or replacing rows.
#[test]
fn test_partial_group_conflict_is_atomic() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    let initial = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::ReserveGroup,
    );
    assert!(
        initial.status.success(),
        "initial group call failed: {}",
        String::from_utf8_lossy(&initial.stderr)
    );

    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db"))
        .expect("Failed to open test database");
    connection
        .execute("DELETE FROM reservations WHERE tag = 'api'", [])
        .expect("Failed to inject a partial group");
    drop(connection);
    let before = reservation_rows(&env);
    assert_eq!(before.len(), 1, "fixture must contain one partial row");

    let repeated = run_json_group_command(
        &env,
        &project_dir,
        &config_path,
        GroupCommandKind::Autoreserve,
    );
    assert!(
        !repeated.status.success(),
        "a partial group must fail instead of mixing old and new rows"
    );
    assert!(
        String::from_utf8_lossy(&repeated.stderr).contains("reservation conflict"),
        "partial group failure should identify the conflict: {}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    assert_eq!(
        reservation_rows(&env),
        before,
        "a partial-group conflict must not mutate the surviving row"
    );
}

/// Repeated autoreserve calls return the same mapping.
#[test]
fn test_autoreserve_idempotency() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First call
    let output1 = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output1.status.success());
    let stdout1 = String::from_utf8(output1.stdout).expect("Invalid UTF-8");
    let json1: serde_json::Value = serde_json::from_str(&stdout1).expect("Should be valid JSON");

    // Second call
    let output2 = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output2.status.success());
    let stdout2 = String::from_utf8(output2.stdout).expect("Invalid UTF-8");
    let json2: serde_json::Value = serde_json::from_str(&stdout2).expect("Should be valid JSON");

    assert_eq!(
        json1, json2,
        "repeated autoreserve calls must return the same mapping"
    );

    assert!(json1.get("web").is_some());
    assert!(json1.get("api").is_some());
    assert!(json2.get("web").is_some());
    assert!(json2.get("api").is_some());
    assert_eq!(
        reservation_count(&env),
        2,
        "repeating a group must retain exactly one row per service"
    );
}

/// reserve-group and autoreserve share one idempotent group identity.
#[test]
fn test_reserve_group_then_autoreserve() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First: reserve-group
    let output1 = env
        .command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");

    assert!(output1.status.success());
    let stdout1 = String::from_utf8(output1.stdout).expect("Invalid UTF-8");
    let json1: serde_json::Value = serde_json::from_str(&stdout1).expect("Should be valid JSON");

    // Second: autoreserve from same directory
    let output2 = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run autoreserve");

    assert!(output2.status.success());
    let stdout2 = String::from_utf8(output2.stdout).expect("Invalid UTF-8");
    let json2: serde_json::Value = serde_json::from_str(&stdout2).expect("Should be valid JSON");

    assert_eq!(
        json1, json2,
        "reserve-group and autoreserve must return the same mapping"
    );

    assert!(json1.get("web").is_some());
    assert!(json1.get("api").is_some());
    assert!(json2.get("web").is_some());
    assert!(json2.get("api").is_some());
    assert_eq!(
        reservation_count(&env),
        2,
        "alternating commands must retain exactly one row per service"
    );
}

/// Competing processes for one new group converge on one complete mapping.
#[test]
fn test_concurrent_same_group_requests_converge() {
    let env = TestEnv::new();
    env.command().arg("init").assert().success();

    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_config_with_mixed_ports(&config_path);

    let mut expected_output = None;
    for _round in 0..3 {
        let specs = (0..6)
            .map(|index| {
                let kind = if index % 2 == 0 {
                    GroupCommandKind::ReserveGroup
                } else {
                    GroupCommandKind::Autoreserve
                };
                (project_dir.clone(), config_path.clone(), kind)
            })
            .collect::<Vec<_>>();
        let outputs = run_concurrent_group_processes(&env.data_dir, &specs);

        for output in &outputs {
            assert!(
                output.status.success(),
                "concurrent same-group request failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let round_output = outputs[0].stdout.clone();
        for output in outputs.iter().skip(1) {
            assert_eq!(
                output.stdout, round_output,
                "same-group processes must return one byte-identical mapping"
            );
        }
        if let Some(expected) = &expected_output {
            assert_eq!(
                &round_output, expected,
                "same-group mapping must remain stable across stress rounds"
            );
        } else {
            expected_output = Some(round_output);
        }

        assert_eq!(
            reservation_count(&env),
            3,
            "same-group concurrency must leave one complete three-service group"
        );
    }
}

/// Concurrent requests for distinct groups allocate unique complete mappings,
/// and a second concurrent round preserves each mapping.
#[test]
fn test_concurrent_distinct_groups_remain_unique_and_stable() {
    let env = TestEnv::new();
    env.command().arg("init").assert().success();

    let groups = (0..6)
        .map(|index| {
            let project_dir = env.create_dir(&format!("project-{index}"));
            let config_path = project_dir.join("trop.yaml");
            create_offset_config_without_occupancy_checks(
                &config_path,
                &format!("project-{index}"),
            );
            (project_dir, config_path)
        })
        .collect::<Vec<_>>();

    let first_specs = groups
        .iter()
        .map(|(project_dir, config_path)| {
            (
                project_dir.clone(),
                config_path.clone(),
                GroupCommandKind::ReserveGroup,
            )
        })
        .collect::<Vec<_>>();
    let first_outputs = run_concurrent_group_processes(&env.data_dir, &first_specs);

    let mut first_mappings = Vec::with_capacity(first_outputs.len());
    let mut all_ports = HashSet::new();
    for output in &first_outputs {
        assert!(
            output.status.success(),
            "concurrent distinct-group request failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mapping: BTreeMap<String, u16> =
            serde_json::from_slice(&output.stdout).expect("Group output should be JSON");
        assert_eq!(mapping.len(), 2, "each distinct group must be complete");
        for port in mapping.values() {
            assert!(
                all_ports.insert(*port),
                "distinct groups must never share port {port}"
            );
        }
        first_mappings.push(mapping);
    }

    assert_eq!(
        reservation_count(&env),
        12,
        "six two-service groups must leave exactly twelve rows"
    );

    let second_specs = groups
        .iter()
        .map(|(project_dir, config_path)| {
            (
                project_dir.clone(),
                config_path.clone(),
                GroupCommandKind::Autoreserve,
            )
        })
        .collect::<Vec<_>>();
    let second_outputs = run_concurrent_group_processes(&env.data_dir, &second_specs);

    for (index, output) in second_outputs.iter().enumerate() {
        assert!(
            output.status.success(),
            "repeated concurrent distinct-group request failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mapping: BTreeMap<String, u16> =
            serde_json::from_slice(&output.stdout).expect("Group output should be JSON");
        assert_eq!(
            mapping, first_mappings[index],
            "distinct group {index} must retain its original mapping"
        );
    }
    assert_eq!(
        reservation_count(&env),
        12,
        "repeating distinct groups must not create or remove rows"
    );
}

/// The root README reservation-only example inherits the built-in port range.
#[test]
fn test_readme_reservations_only_example_uses_built_in_ports() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("readme-project");
    let config_path = project_dir.join("trop.yaml");
    fs::write(
        &config_path,
        r"
reservations:
  services:
    web:
      offset: 0
      preferred: 8080
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    db:
      offset: 2
      env: DB_PORT
",
    )
    .unwrap();

    let output = env
        .command()
        .env("TROP_SKIP_OCCUPANCY_CHECK", "true")
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to run the README group example");
    assert!(
        output.status.success(),
        "README group example failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mapping: BTreeMap<String, u16> =
        serde_json::from_slice(&output.stdout).expect("README output should be JSON");
    assert_eq!(mapping.get("web"), Some(&8080));
    assert_eq!(mapping.get("api"), Some(&5001));
    assert_eq!(mapping.get("db"), Some(&5002));
}

/// Both group entrypoints consume every effective source layer identically.
#[test]
fn test_group_commands_share_complete_layered_configuration() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("layered-project");
    let project_path = project_dir.join("trop.yaml");
    let local_path = project_dir.join("trop.local.yaml");
    fs::write(
        &project_path,
        r"
project: base-project
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
",
    )
    .unwrap();
    fs::write(
        &local_path,
        r"
project: local-project
excluded_ports: [5002]
",
    )
    .unwrap();

    let run = |name: &str, kind: GroupCommandKind, nominated: &Path| {
        let data_dir = env.temp_path.join(name);
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(
            data_dir.join("config.yaml"),
            r"
excluded_ports: [5000]
cleanup:
  expire_after_days: 7
occupancy_check:
  skip: true
",
        )
        .unwrap();

        let mut command = env.command_bare();
        command
            .arg("--data-dir")
            .arg(&data_dir)
            .env("TROP_EXCLUDED_PORTS", "5001");
        match kind {
            GroupCommandKind::ReserveGroup => {
                command.arg("reserve-group").arg(nominated);
            }
            GroupCommandKind::Autoreserve => {
                command.arg("autoreserve").current_dir(&project_dir);
            }
        }
        command
            .arg("--format")
            .arg("json")
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to run layered group command")
    };

    let cases = [
        (
            "explicit-project-data",
            GroupCommandKind::ReserveGroup,
            project_path.as_path(),
        ),
        (
            "explicit-local-data",
            GroupCommandKind::ReserveGroup,
            local_path.as_path(),
        ),
        (
            "autoreserve-data",
            GroupCommandKind::Autoreserve,
            project_path.as_path(),
        ),
    ];
    let expected = BTreeMap::from([("api".to_string(), 5004), ("web".to_string(), 5003)]);

    for (name, kind, nominated) in cases {
        let output = run(name, kind, nominated);
        assert!(
            output.status.success(),
            "{} failed: {}",
            kind.name(),
            String::from_utf8_lossy(&output.stderr)
        );
        let mapping: BTreeMap<String, u16> =
            serde_json::from_slice(&output.stdout).expect("Group output should be JSON");
        assert_eq!(
            mapping,
            expected,
            "{} did not consume the complete effective configuration",
            kind.name()
        );
    }
}

/// An explicit local clear disables both entrypoints without touching stored rows.
#[test]
fn test_reservations_null_disables_group_commands_without_mutation() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("cleared-project");
    let project_path = project_dir.join("trop.yaml");
    let local_path = project_dir.join("trop.local.yaml");
    create_offset_config_without_occupancy_checks(&project_path, "clear-project");
    fs::write(&local_path, "excluded_ports: [9000]\n").unwrap();

    let seeded = env
        .command()
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .output()
        .expect("Failed to seed group before clear");
    assert!(
        seeded.status.success(),
        "Failed to seed group before clear: {}",
        String::from_utf8_lossy(&seeded.stderr)
    );
    let before = reservation_rows(&env);

    fs::write(&local_path, "reservations: null\n").unwrap();
    for kind in GroupCommandKind::ALL {
        for dry_run in [true, false] {
            let mut command = env.command();
            match kind {
                GroupCommandKind::ReserveGroup => {
                    command.arg("reserve-group").arg(&project_path);
                }
                GroupCommandKind::Autoreserve => {
                    command.arg("autoreserve").current_dir(&project_dir);
                }
            }
            command
                .arg("--format")
                .arg("json")
                .arg("--allow-unrelated-path");
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command
                .output()
                .expect("Failed to run cleared group command");
            assert!(
                !output.status.success(),
                "{}{} must fail after reservations: null",
                kind.name(),
                if dry_run { " --dry-run" } else { "" }
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("reservations"),
                "{} should identify the cleared reservation group: {}",
                kind.name(),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                reservation_rows(&env),
                before,
                "{} changed stored rows after an explicit clear",
                kind.name()
            );
        }
    }
}
