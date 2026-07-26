//! End-to-end tests for source-aware effective configuration.
//!
//! These tests intentionally exercise configuration through the CLI instead of
//! stopping at parsing or merging. Each fixture uses its own data directory and
//! process environment.

mod common;

use assert_cmd::Command;
use common::TestEnv;
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_ENV_VARS: &[&str] = &[
    "TROP_PROJECT",
    "TROP_PORT_MIN",
    "TROP_PORT_MAX",
    "TROP_PORT_MAX_OFFSET",
    "TROP_EXCLUDED_PORTS",
    "TROP_DISABLE_AUTOINIT",
    "TROP_DISABLE_AUTOPRUNE",
    "TROP_DISABLE_AUTOEXPIRE",
    "TROP_OUTPUT_FORMAT",
    "TROP_ALLOW_UNRELATED_PATH",
    "TROP_ALLOW_PROJECT_CHANGE",
    "TROP_ALLOW_TASK_CHANGE",
    "TROP_ALLOW_CHANGE_PROJECT",
    "TROP_ALLOW_CHANGE_TASK",
    "TROP_ALLOW_CHANGE",
    "TROP_BUSY_TIMEOUT",
    "TROP_MAXIMUM_LOCK_WAIT_SECONDS",
    "TROP_EXPIRE_AFTER_DAYS",
    "TROP_SKIP_OCCUPANCY_CHECK",
    "TROP_SKIP_IPV4",
    "TROP_SKIP_IPV6",
    "TROP_SKIP_TCP",
    "TROP_SKIP_UDP",
    "TROP_CHECK_ALL_INTERFACES",
];

fn isolated_command(data_dir: &Path) -> Command {
    let mut command = Command::cargo_bin("trop").expect("Failed to find trop binary");
    command
        .env("TROP_DATA_DIR", data_dir)
        .arg("--data-dir")
        .arg(data_dir);

    for variable in CONFIG_ENV_VARS {
        command.env_remove(variable);
    }

    command
}

fn canonical_dir(env: &TestEnv, name: &str) -> PathBuf {
    fs::canonicalize(env.create_dir(name)).expect("Failed to canonicalize fixture directory")
}

fn canonical_data_dir(env: &TestEnv) -> PathBuf {
    fs::create_dir_all(&env.data_dir).expect("Failed to create fixture data directory");
    fs::canonicalize(&env.data_dir).expect("Failed to canonicalize fixture data directory")
}

#[test]
fn file_disable_autoinit_prevents_database_creation() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let working_dir = canonical_dir(&env, "work");
    fs::write(data_dir.join("config.yaml"), "disable_autoinit: true\n")
        .expect("Failed to write user configuration");

    let output = isolated_command(&data_dir)
        .current_dir(working_dir)
        .arg("list")
        .output()
        .expect("Failed to run trop list");
    let database_path = data_dir.join("trop.db");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(3),
        "file disable_autoinit must report the missing-data-directory exit code; \
         stdout={}, stderr={stderr}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(
        stderr.contains("Data directory not found"),
        "missing-data-directory diagnostic was not emitted: {stderr}"
    );
    assert!(
        !database_path.exists(),
        "file disable_autoinit must not create {}",
        database_path.display()
    );
}

#[test]
fn file_output_format_controls_list() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let working_dir = canonical_dir(&env, "work");
    fs::write(data_dir.join("config.yaml"), "output_format: json\n")
        .expect("Failed to write user configuration");

    let output = isolated_command(&data_dir)
        .current_dir(working_dir)
        .arg("list")
        .output()
        .expect("Failed to run trop list");

    assert!(
        output.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "file output_format=json must produce JSON, got {:?}: {error}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(parsed, Value::Array(Vec::new()));
}

#[test]
fn file_allow_unrelated_path_reaches_reserve() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let caller = canonical_dir(&env, "caller");
    let target = canonical_dir(&env, "target");
    fs::write(
        data_dir.join("config.yaml"),
        r"allow_unrelated_path: true
ports:
  min: 45123
  max: 45123
occupancy_check:
  skip: true
",
    )
    .expect("Failed to write user configuration");

    let output = isolated_command(&data_dir)
        .current_dir(&caller)
        .arg("reserve")
        .arg("--path")
        .arg(&target)
        .output()
        .expect("Failed to run trop reserve");

    assert!(
        output.status.success(),
        "file allow_unrelated_path must authorize the reservation: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "45123");

    let connection =
        rusqlite::Connection::open(data_dir.join("trop.db")).expect("Failed to open test database");
    let persisted_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM reservations", [], |row| row.get(0))
        .expect("Failed to count persisted reservations");
    assert_eq!(
        persisted_count, 1,
        "reserve must persist exactly one reservation"
    );

    let persisted: Option<(String, u16)> = connection
        .query_row("SELECT path, port FROM reservations", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
        .expect("Failed to query persisted reservation");

    assert_eq!(
        persisted,
        Some((target.display().to_string(), 45123)),
        "reserve must persist exactly the configured reservation"
    );
}

#[test]
fn invalid_user_project_reports_source() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let working_dir = canonical_dir(&env, "work");
    let config_path = data_dir.join("config.yaml");
    fs::write(&config_path, "project: forbidden-global\n")
        .expect("Failed to write user configuration");

    let output = isolated_command(&data_dir)
        .current_dir(working_dir)
        .arg("list")
        .output()
        .expect("Failed to run trop list");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "project is invalid in user config {}; command unexpectedly succeeded with stdout {}",
        config_path.display(),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        stderr.contains("project"),
        "diagnostic must identify the invalid project field: {stderr}"
    );
    assert!(
        stderr.contains(config_path.to_string_lossy().as_ref()),
        "diagnostic must identify the exact source path {}: {stderr}",
        config_path.display()
    );
}

#[test]
fn cli_override_can_repair_lower_precedence_port_range() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let working_dir = canonical_dir(&env, "work");
    fs::write(
        working_dir.join("trop.yaml"),
        "ports:\n  min: 7000\n  max: 8000\n",
    )
    .expect("Failed to write project configuration");

    let output = isolated_command(&data_dir)
        .current_dir(&working_dir)
        .env("TROP_PORT_MAX", "6000")
        .arg("reserve")
        .arg("--max")
        .arg("9000")
        .arg("--skip-occupancy-check")
        .output()
        .expect("Failed to run trop reserve");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "the highest-precedence CLI value must repair the invalid intermediate \
         file/environment range: {stderr}"
    );
    let port: u16 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("reserve output was not a port");
    assert!(
        (7000..=9000).contains(&port),
        "CLI-repaired range produced port {port}"
    );
}

#[test]
fn validate_reports_invalid_field_source() {
    let env = TestEnv::new();
    let data_dir = canonical_data_dir(&env);
    let working_dir = canonical_dir(&env, "work");
    let config_path = working_dir.join("config.yaml");
    fs::write(&config_path, "project: forbidden-global\n")
        .expect("Failed to write configuration under validation");

    let output = isolated_command(&data_dir)
        .current_dir(working_dir)
        .arg("validate")
        .arg(&config_path)
        .output()
        .expect("Failed to run trop validate");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "validate must reject project in a user config"
    );
    assert!(
        stderr.contains("project") && stderr.contains(config_path.to_string_lossy().as_ref()),
        "validate diagnostic must identify the field and exact source: {stderr}"
    );
}

#[test]
fn init_validates_effective_busy_timeout_before_mutation() {
    let env = TestEnv::new();
    let fallback_data_dir = canonical_data_dir(&env);
    let init_data_dir = env.path().join("init-target");

    let output = isolated_command(&fallback_data_dir)
        .env("TROP_BUSY_TIMEOUT", "not-a-number")
        .arg("init")
        .arg("--data-dir")
        .arg(&init_data_dir)
        .output()
        .expect("Failed to run trop init");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "init must reject an invalid effective busy timeout"
    );
    assert!(
        stderr.contains("TROP_BUSY_TIMEOUT"),
        "init diagnostic must identify the invalid environment source: {stderr}"
    );
    assert!(
        !init_data_dir.join("trop.db").exists(),
        "init must validate configuration before creating the database"
    );
}
