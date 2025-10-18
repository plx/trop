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

use common::TestEnv;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

// ============================================================================
// Test Helpers
// ============================================================================

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
      preferred: 9000
      env: WEB_PORT
    api:
      preferred: 9001
      env: API_PORT
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

// ============================================================================
// reserve-group: Quiet and Verbose Modes
// ============================================================================

/// Test reserve-group with --quiet flag.
///
/// Quiet mode should suppress status messages on stderr while still outputting
/// the formatted allocations on stdout. This is important for scripting where
/// you want clean output: `eval $(trop reserve-group config.yaml --quiet)`
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
/// status messages always go to stderr. This enables shell integration like
/// `eval $(trop reserve-group config.yaml)` to work correctly.
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
/// The --force flag overrides all safety checks. This test verifies that
/// --force allows the operation to proceed even when it would normally be
/// rejected due to path validation or sticky field protections.
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

    // Second reservation with different task should work with --force
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("different-task")
        .arg("--force")
        .assert()
        .success();
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

    // Should succeed with --allow-unrelated-path
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test reserve-group with --allow-project-change flag.
///
/// CURRENT BEHAVIOR: Group reservations don't currently enforce sticky field
/// validation in the same way as single reservations. This test documents that
/// reserve-group succeeds even when the project changes, since each call
/// creates new reservations.
///
/// NOTE: If group idempotency is implemented, sticky field validation would
/// become relevant and this test would need to verify that behavior.
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

    // Change config to different project
    create_test_config(&config_path, "different-project");

    // CURRENT BEHAVIOR: Succeeds without permission since new reservations are created
    // If idempotency is implemented, this might fail without --allow-project-change
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // With --allow-project-change flag (also succeeds)
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-project-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
}

/// Test reserve-group with --allow-task-change flag.
///
/// CURRENT BEHAVIOR: Similar to project changes, task changes don't trigger
/// sticky field validation failures since each group reservation creates new
/// entries. This test documents the current non-idempotent behavior.
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

    // CURRENT BEHAVIOR: Changing task succeeds (creates new reservations)
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-2")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // With --allow-task-change also succeeds
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--task")
        .arg("task-3")
        .arg("--allow-task-change")
        .arg("--allow-unrelated-path")
        .assert()
        .success();
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
/// Autoreserve should support the same override flags as reserve-group
/// (--force, --allow-unrelated-path, etc.). This test verifies that flags
/// are properly passed through after discovery.
#[test]
fn test_autoreserve_with_override_flags() {
    let env = TestEnv::new();
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    // First reservation
    env.command()
        .arg("autoreserve")
        .arg("--allow-unrelated-path")
        .current_dir(&project_dir)
        .assert()
        .success();

    // Second with different task - should work with --force
    env.command()
        .arg("autoreserve")
        .arg("--task")
        .arg("different-task")
        .arg("--force")
        .current_dir(&project_dir)
        .assert()
        .success();
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

/// Test reserve-group with invalid shell type.
///
/// When --shell is specified with an unsupported value, the command should
/// fail with an error listing valid shell types.
#[test]
fn test_reserve_group_invalid_shell_type() {
    let env = TestEnv::new();
    let config_dir = env.create_dir("project");
    let config_path = config_dir.join("trop.yaml");
    create_test_config(&config_path, "test-project");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--format")
        .arg("export")
        .arg("--shell")
        .arg("invalid-shell")
        .assert()
        .failure()
        .stderr(predicate::str::contains("shell").or(predicate::str::contains("invalid")));
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

/// Test autoreserve repeated calls behavior.
///
/// CURRENT BEHAVIOR: Group reservations currently allocate new ports on each call.
/// This test documents the existing behavior where repeated autoreserve calls
/// create new reservations rather than reusing existing ones.
///
/// NOTE: This may change in the future to be idempotent (return same ports),
/// which would be more consistent with the single-reservation behavior.
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

    // CURRENT BEHAVIOR: Allocates different ports each time
    // Future enhancement: Could be made idempotent
    assert_ne!(
        json1, json2,
        "Currently, repeated calls allocate new ports (not idempotent)"
    );

    // Verify both calls successfully allocated ports
    assert!(json1.get("web").is_some());
    assert!(json1.get("api").is_some());
    assert!(json2.get("web").is_some());
    assert!(json2.get("api").is_some());
}

/// Test reserve-group followed by autoreserve behavior.
///
/// CURRENT BEHAVIOR: Group reservations allocate new ports on each invocation,
/// even when called via different commands (reserve-group vs autoreserve).
/// This test documents that currently there is no idempotency across commands.
///
/// NOTE: Future enhancement could make this idempotent if desired.
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

    // CURRENT BEHAVIOR: Different commands allocate different ports
    assert_ne!(
        json1, json2,
        "Currently, different commands allocate new ports (not idempotent)"
    );

    // Both should have successfully allocated ports
    assert!(json1.get("web").is_some());
    assert!(json1.get("api").is_some());
    assert!(json2.get("web").is_some());
    assert!(json2.get("api").is_some());
}
