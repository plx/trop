//! Integration tests for the configuration system.
//!
//! This test suite validates the complete workflow of the configuration system,
//! including file discovery, merging, environment variable handling, and validation.
//!
//! These tests complement the unit tests in the config module by testing
//! integration scenarios that involve multiple components working together.
//!
//! ## Running Tests
//!
//! Tests that modify environment variables are marked with `#[serial]` to ensure
//! they run sequentially and don't interfere with each other. Environment variables
//! are process-global in Rust, so concurrent access would cause race conditions.
//!
//! The `serial_test` crate handles this automatically - you can run tests normally:
//! ```sh
//! cargo test --test config_integration
//! ```
//!
//! Only environment-dependent tests run serially; other tests run in parallel.

use serial_test::serial;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use trop::config::{
    CleanupConfig, Config, ConfigBuilder, ConfigField, ConfigFileKind, ConfigValueSource,
    OccupancyConfig, OutputFormat, PortConfig, PortExclusion, ReservationGroup, ServiceDefinition,
};
use trop::error::Error;
use trop::operations::{
    AutoreserveOptions, AutoreservePlan, ReserveGroupOptions, ReserveGroupPlan,
};

// ============================================================================
// Test Utilities
// ============================================================================

/// Helper to get path to test fixtures.
fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("configs")
        .join(relative)
}

/// Helper to create a temporary config file.
fn create_temp_config(dir: &Path, filename: &str, content: &str) -> PathBuf {
    let path = dir.join(filename);
    fs::write(&path, content).unwrap();
    path
}

/// RAII guard for setting and restoring environment variables.
///
/// Note: Tests using environment variables should not run in parallel.
/// Use #[serial] attribute or ensure tests clean up properly.
struct EnvGuard {
    key: String,
    old_value: Option<String>,
}

impl EnvGuard {
    fn new(key: &str, value: &str) -> Self {
        let old_value = env::var(key).ok();
        env::set_var(key, value);
        Self {
            key: key.to_string(),
            old_value,
        }
    }

    /// Create a guard that removes the env var (useful for cleanup).
    fn remove(key: &str) -> Self {
        let old_value = env::var(key).ok();
        env::remove_var(key);
        Self {
            key: key.to_string(),
            old_value,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old_value {
            Some(val) => env::set_var(&self.key, val),
            None => env::remove_var(&self.key),
        }
    }
}

/// Helper to clear all TROP_* environment variables before a test.
/// This prevents cross-contamination between tests.
fn clear_trop_env_vars() -> Vec<EnvGuard> {
    let keys = [
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
        "TROP_SKIP_OCCUPANCY_CHECK",
        "TROP_SKIP_IPV4",
        "TROP_SKIP_IPV6",
        "TROP_SKIP_TCP",
        "TROP_SKIP_UDP",
        "TROP_CHECK_ALL_INTERFACES",
        "TROP_EXPIRE_AFTER_DAYS",
        "TROP_CLEANUP_EXPIRE_AFTER_DAYS",
    ];

    keys.iter().map(|k| EnvGuard::remove(k)).collect()
}

// ============================================================================
// Category 1: File Discovery Tests
// ============================================================================

/// Test that the config loader discovers files by walking up the directory tree.
///
/// This validates the fundamental discovery mechanism: starting from a nested
/// directory, the loader should search parent directories until it finds
/// configuration files.
#[test]
fn test_file_discovery_upward_traversal() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path();
    let child = parent.join("nested").join("deeply");
    fs::create_dir_all(&child).unwrap();

    // Create config in parent
    create_temp_config(parent, "trop.yaml", "project: parent-proj\n");

    // Load from child directory - should find parent's config
    let config = ConfigBuilder::new()
        .with_working_dir(&child)
        .skip_env()
        .build()
        .unwrap();

    assert_eq!(config.project, Some("parent-proj".to_string()));
}

/// Test that discovery stops at the first directory containing config files.
///
/// This ensures we don't walk all the way to the filesystem root unnecessarily.
/// Once we find a directory with trop.yaml or trop.local.yaml, we should stop
/// searching parent directories.
#[test]
fn test_file_discovery_stops_at_first_config() {
    let temp = TempDir::new().unwrap();
    let grandparent = temp.path();
    let parent = grandparent.join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();

    // Create configs at multiple levels
    create_temp_config(grandparent, "trop.yaml", "project: grandparent\n");
    create_temp_config(&parent, "trop.yaml", "project: parent\n");

    // Load from child - should find parent's config and stop there
    let config = ConfigBuilder::new()
        .with_working_dir(&child)
        .skip_env()
        .build()
        .unwrap();

    // Should find parent, not grandparent
    assert_eq!(config.project, Some("parent".to_string()));
}

/// Test that trop.local.yaml has precedence over trop.yaml in the same directory.
///
/// When both files exist in the same directory, trop.local.yaml should override
/// settings from trop.yaml. This is important for local development overrides
/// that shouldn't be committed to version control.
#[test]
fn test_file_discovery_local_precedence() {
    let temp = TempDir::new().unwrap();

    create_temp_config(
        temp.path(),
        "trop.yaml",
        "project: base\nmaximum_lock_wait_seconds: 5\n",
    );
    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        "maximum_lock_wait_seconds: 10\n",
    );

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Project from base config
    assert_eq!(config.project, Some("base".to_string()));
    // Lock timeout from local override
    assert_eq!(config.maximum_lock_wait_seconds, Some(10));
}

/// Test loading user config from ~/.trop/config.yaml.
///
/// This test verifies that user-level configuration can be loaded, but requires
/// careful setup since we can't modify the actual user's home directory in tests.
/// Instead, we test the behavior when no user config exists.
#[test]
fn test_file_discovery_no_user_config() {
    let temp = TempDir::new().unwrap();

    // No user config file exists in temp dir, should still work
    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Should get defaults
    assert_eq!(config.ports.as_ref().unwrap().min, 5000);
}

/// Test behavior when no configuration files exist at all.
///
/// The system should fall back to built-in defaults and still produce
/// a valid configuration. This ensures trop works out-of-the-box without
/// requiring any configuration files.
#[test]
fn test_file_discovery_no_configs_uses_defaults() {
    let temp = TempDir::new().unwrap();

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Verify we get all the default values
    assert_eq!(config.ports.as_ref().unwrap().min, 5000);
    assert_eq!(config.ports.as_ref().unwrap().max, Some(7000));
    assert_eq!(config.cleanup.as_ref().unwrap().expire_after_days, Some(30));
    assert_eq!(config.maximum_lock_wait_seconds, Some(5));
    assert_eq!(config.disable_autoinit, Some(false));
    assert_eq!(config.output_format, Some(OutputFormat::Table));
}

// ============================================================================
// Category 2: Merging Tests
// ============================================================================

/// Test the complete precedence chain from defaults through all sources.
///
/// This is the most comprehensive merging test. It validates that the full
/// precedence chain works correctly: defaults → user config → trop.yaml →
/// trop.local.yaml → env → programmatic. Each layer should be able to
/// override the previous one.
#[test]
#[serial]
fn test_merging_complete_precedence_chain() {
    let temp = TempDir::new().unwrap();

    // Layer 1: trop.yaml
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: from-trop-yaml
ports:
  min: 6000
  max: 7000
maximum_lock_wait_seconds: 8
",
    );

    // Layer 2: trop.local.yaml (should override trop.yaml)
    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        r"
maximum_lock_wait_seconds: 12
disable_autoinit: true
",
    );

    // Layer 3: Environment variable (should override files)
    // Note: min=6500 is chosen to be within the file's max=7000 range to avoid validation errors
    let _env = EnvGuard::new("TROP_PORT_MIN", "6500");

    // Layer 4: Programmatic (highest precedence)
    let programmatic = Config {
        output_format: Some(OutputFormat::Json),
        ..Default::default()
    };

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .with_config(programmatic)
        .build()
        .unwrap();

    // Verify each layer's contribution
    assert_eq!(config.project, Some("from-trop-yaml".to_string())); // From trop.yaml
    assert_eq!(config.disable_autoinit, Some(true)); // From trop.local.yaml
    assert_eq!(config.ports.as_ref().unwrap().min, 6500); // From env var
    assert_eq!(config.output_format, Some(OutputFormat::Json)); // From programmatic
    assert_eq!(config.maximum_lock_wait_seconds, Some(12)); // From trop.local.yaml
}

/// Test that excluded_ports accumulate from multiple sources.
///
/// Unlike most configuration fields which use "last one wins" semantics,
/// excluded_ports is special: it accumulates exclusions from all sources.
/// This allows different layers to each contribute to the exclusion list,
/// which is more useful than replacement.
#[test]
#[serial]
fn test_merging_excluded_ports_accumulation() {
    let temp = TempDir::new().unwrap();

    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
excluded_ports:
  - 5001
  - 5002
",
    );

    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        "excluded_ports:\n  - 5003\n  - \"5010..5020\"\n",
    );

    let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "5030,5040");

    let programmatic = Config {
        excluded_ports: Some(vec![PortExclusion::Single(5050)]),
        ..Default::default()
    };

    // Use empty data dir to isolate from user's global config
    let data_dir = temp.path().join("data");
    fs::create_dir(&data_dir).unwrap();

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .with_data_dir(&data_dir)
        .with_config(programmatic)
        .build()
        .unwrap();

    let excluded = config.excluded_ports.as_ref().unwrap();

    // Should have exclusions from all sources
    // 2 from trop.yaml + 2 from trop.local.yaml + 2 from env + 1 programmatic = 7
    assert_eq!(excluded.len(), 7);
}

/// Test partial configuration merging.
///
/// When multiple sources provide partial configurations, the final result
/// should intelligently merge them. Some fields come from one source,
/// some from another, creating a composite configuration.
#[test]
fn test_merging_partial_configs() {
    let temp = TempDir::new().unwrap();

    // File 1: provides project and ports
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: partial-test
ports:
  min: 6000
  max: 7000
",
    );

    // File 2: provides cleanup and output
    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        r"
cleanup:
  expire_after_days: 45
output_format: csv
",
    );

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Should have fields from both sources
    assert_eq!(config.project, Some("partial-test".to_string()));
    assert_eq!(config.ports.as_ref().unwrap().min, 6000);
    assert_eq!(config.cleanup.as_ref().unwrap().expire_after_days, Some(45));
    assert_eq!(config.output_format, Some(OutputFormat::Csv));
}

/// Test that higher precedence completely replaces simple fields.
///
/// For most fields (unlike excluded_ports), higher precedence should
/// completely replace the value from lower precedence, not merge with it.
/// This test ensures we don't accidentally merge when we should replace.
#[test]
fn test_merging_simple_field_replacement() {
    let temp = TempDir::new().unwrap();

    create_temp_config(temp.path(), "trop.yaml", "maximum_lock_wait_seconds: 5\n");
    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        "maximum_lock_wait_seconds: 10\n",
    );

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Should be 10, not 5, and not some merged value
    assert_eq!(config.maximum_lock_wait_seconds, Some(10));
}

/// Test merging of nested port configuration.
///
/// The PortConfig struct has multiple fields (min, max, max_offset).
/// When merging, we need to properly handle partial updates where one
/// source provides min and another provides max.
#[test]
fn test_merging_port_config_fields() {
    let temp = TempDir::new().unwrap();

    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
ports:
  min: 6000
",
    );

    create_temp_config(
        temp.path(),
        "trop.local.yaml",
        r"
ports:
  min: 6000  # Same min
  max: 8000  # But different max
",
    );

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    assert_eq!(config.ports.as_ref().unwrap().min, 6000);
    assert_eq!(config.ports.as_ref().unwrap().max, Some(8000));
}

// ============================================================================
// Category 3: Environment Variable Tests
// ============================================================================

/// Test that TROP_PROJECT environment variable sets the project name.
///
/// Environment variables should override file-based configuration.
/// This validates the basic env var mechanism works.
#[test]
#[serial]
fn test_env_var_trop_project() {
    let temp = TempDir::new().unwrap();
    create_temp_config(temp.path(), "trop.yaml", "project: from-file\n");

    let _env = EnvGuard::new("TROP_PROJECT", "from-env");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .build()
        .unwrap();

    assert_eq!(config.project, Some("from-env".to_string()));
}

/// Test TROP_PORT_MIN and TROP_PORT_MAX environment variables.
///
/// These env vars should override the port configuration from files.
/// Both variables should work independently and together.
#[test]
#[serial]
fn test_env_var_port_range() {
    let temp = TempDir::new().unwrap();
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
ports:
  min: 5000
  max: 6000
",
    );

    let _env_min = EnvGuard::new("TROP_PORT_MIN", "8000");
    let _env_max = EnvGuard::new("TROP_PORT_MAX", "9000");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .build()
        .unwrap();

    assert_eq!(config.ports.as_ref().unwrap().min, 8000);
    assert_eq!(config.ports.as_ref().unwrap().max, Some(9000));
}

/// Test TROP_EXCLUDED_PORTS with various formats.
///
/// The env var should support:
/// - Single ports: "5001"
/// - Multiple ports: "5001,5002,5003"
/// - Ranges: "5000..5010"
/// - Mixed: "5001,5010..5020,5030"
#[test]
#[serial]
fn test_env_var_excluded_ports_formats() {
    let temp = TempDir::new().unwrap();

    // Use empty data dir to isolate from user's global config
    let data_dir = temp.path().join("data");
    fs::create_dir(&data_dir).unwrap();

    // Single port
    {
        let _guards = clear_trop_env_vars();
        let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "5001");
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .with_data_dir(&data_dir)
            .build()
            .unwrap();
        assert_eq!(config.excluded_ports.as_ref().unwrap().len(), 1);
    }

    // Multiple ports
    {
        let _guards = clear_trop_env_vars();
        let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "5001,5002,5003");
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .with_data_dir(&data_dir)
            .build()
            .unwrap();
        assert_eq!(config.excluded_ports.as_ref().unwrap().len(), 3);
    }

    // Range
    {
        let _guards = clear_trop_env_vars();
        let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "5000..5010");
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .with_data_dir(&data_dir)
            .build()
            .unwrap();
        let excluded = config.excluded_ports.as_ref().unwrap();
        assert_eq!(excluded.len(), 1);
        match &excluded[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(*start, 5000);
                assert_eq!(*end, 5010);
            }
            _ => panic!("Expected range"),
        }
    }

    // Mixed
    {
        let _guards = clear_trop_env_vars();
        let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "5001,5010..5020,5030");
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .with_data_dir(&data_dir)
            .build()
            .unwrap();
        assert_eq!(config.excluded_ports.as_ref().unwrap().len(), 3);
    }
}

/// Test boolean environment variable parsing variations.
///
/// The spec says we should support multiple boolean representations:
/// - true: "true", "1", "yes", "on"
/// - false: "false", "0", "no", "off"
/// Case insensitive.
#[test]
#[serial]
fn test_env_var_boolean_parsing() {
    let temp = TempDir::new().unwrap();

    let test_cases = [
        // (value, expected)
        ("true", true),
        ("True", true),
        ("TRUE", true),
        ("1", true),
        ("yes", true),
        ("YES", true),
        ("on", true),
        ("ON", true),
        ("false", false),
        ("False", false),
        ("FALSE", false),
        ("0", false),
        ("no", false),
        ("NO", false),
        ("off", false),
        ("OFF", false),
    ];

    for (val, expected) in test_cases {
        let _guards = clear_trop_env_vars();
        let _env = EnvGuard::new("TROP_DISABLE_AUTOINIT", val);
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .build()
            .unwrap();
        assert_eq!(
            config.disable_autoinit,
            Some(expected),
            "Failed for value: {val}"
        );
    }
}

/// Test invalid environment variable value error handling.
///
/// When an env var contains an invalid value (e.g., invalid port number,
/// unparseable boolean), we should get a clear error message indicating
/// which env var is problematic and why.
#[test]
#[serial]
fn test_env_var_invalid_values() {
    let temp = TempDir::new().unwrap();

    // Invalid port number
    {
        let _env = EnvGuard::new("TROP_PORT_MIN", "99999"); // Too high
        let result = ConfigBuilder::new().with_working_dir(temp.path()).build();
        assert!(result.is_err());
    }

    // Invalid boolean
    {
        let _env = EnvGuard::new("TROP_DISABLE_AUTOINIT", "maybe");
        let result = ConfigBuilder::new().with_working_dir(temp.path()).build();
        assert!(result.is_err());
    }

    // Invalid excluded port format
    {
        let _env = EnvGuard::new("TROP_EXCLUDED_PORTS", "not-a-port");
        let result = ConfigBuilder::new().with_working_dir(temp.path()).build();
        assert!(result.is_err());
    }
}

/// Test all occupancy check environment variables.
///
/// The occupancy check configuration has many flags that can be controlled
/// via environment variables. This test ensures they all work.
#[test]
#[serial]
fn test_env_var_occupancy_checks() {
    let temp = TempDir::new().unwrap();

    let _skip = EnvGuard::new("TROP_SKIP_OCCUPANCY_CHECK", "true");
    let _skip_ip4 = EnvGuard::new("TROP_SKIP_IPV4", "true");
    let _skip_ip6 = EnvGuard::new("TROP_SKIP_IPV6", "false");
    let _skip_tcp = EnvGuard::new("TROP_SKIP_TCP", "true");
    let _skip_udp = EnvGuard::new("TROP_SKIP_UDP", "false");
    let _check_all = EnvGuard::new("TROP_CHECK_ALL_INTERFACES", "true");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .build()
        .unwrap();

    let occ = config.occupancy_check.as_ref().unwrap();
    assert_eq!(occ.skip, Some(true));
    assert_eq!(occ.skip_ip4, Some(true));
    assert_eq!(occ.skip_ip6, Some(false));
    assert_eq!(occ.skip_tcp, Some(true));
    assert_eq!(occ.skip_udp, Some(false));
    assert_eq!(occ.check_all_interfaces, Some(true));
}

/// Test permission flag environment variables.
///
/// All the allow_* permission flags should be controllable via env vars.
#[test]
#[serial]
fn test_env_var_permission_flags() {
    let temp = TempDir::new().unwrap();

    let _unrelated = EnvGuard::new("TROP_ALLOW_UNRELATED_PATH", "true");
    let _change_proj = EnvGuard::new("TROP_ALLOW_CHANGE_PROJECT", "true");
    let _change_task = EnvGuard::new("TROP_ALLOW_CHANGE_TASK", "false");
    let _change = EnvGuard::new("TROP_ALLOW_CHANGE", "true");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .build()
        .unwrap();

    assert_eq!(config.allow_unrelated_path, Some(true));
    assert_eq!(config.allow_change_project, Some(true));
    assert_eq!(config.allow_change_task, Some(false));
    assert_eq!(config.allow_change, Some(true));
}

// ============================================================================
// Category 4: Validation Tests
// ============================================================================

/// Test validation of port range (max must be >= min).
///
/// This is a critical validation rule. If violated, the error message
/// should clearly indicate the problem and suggest how to fix it.
#[test]
fn test_validation_port_range_max_less_than_min() {
    let config = Config {
        ports: Some(PortConfig {
            min: 8000,
            max: Some(5000), // Invalid: max < min
            max_offset: None,
        }),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert!(field.contains("ports"));
            assert!(message.contains("max") || message.contains("min"));
        }
        _ => panic!("Expected validation error, got: {err:?}"),
    }
}

/// Test validation that both max and max_offset cannot be specified.
///
/// This is a mutual exclusion constraint. The user should specify one or
/// the other, not both, as they represent different ways of defining the
/// upper bound.
#[test]
fn test_validation_port_config_both_max_and_offset() {
    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(7000),
            max_offset: Some(2000), // Can't have both!
        }),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of empty project identifier.
///
/// Project identifiers cannot be empty or whitespace-only. This prevents
/// confusing database entries and ensures meaningful naming.
#[test]
fn test_validation_empty_project_identifier() {
    let config = Config {
        project: Some(String::new()), // Empty
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());

    // Also test whitespace-only
    let config = Config {
        project: Some("   ".to_string()),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of project field only in trop.yaml files.
///
/// The project field has special semantics and should only appear in
/// trop.yaml or trop.local.yaml files, not in user config. This test
/// validates that constraint.
#[test]
fn test_validation_project_only_in_tropfile() {
    // Programmatic config with project should work (treated as tropfile)
    let config = Config {
        project: Some("test".to_string()),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_ok());
}

/// Test validation of excluded port ranges.
///
/// Excluded port ranges must have start <= end. Invalid ranges should be
/// rejected with a clear error message.
#[test]
fn test_validation_excluded_ports_invalid_range() {
    let config = Config {
        excluded_ports: Some(vec![PortExclusion::Range {
            start: 5010,
            end: 5000, // Invalid: end < start
        }]),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of cleanup configuration.
///
/// The expire_after_days field must be > 0 if specified. Zero or negative
/// values don't make sense for expiration.
#[test]
fn test_validation_cleanup_expire_days_zero() {
    let config = Config {
        cleanup: Some(CleanupConfig {
            expire_after_days: Some(0), // Invalid
        }),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of maximum lock wait timeout.
///
/// The timeout must be > 0. A zero timeout doesn't make sense.
#[test]
fn test_validation_lock_timeout_zero() {
    let config = Config {
        maximum_lock_wait_seconds: Some(0), // Invalid
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of reservation group offset uniqueness.
///
/// Each service in a reservation group must have a unique offset. Duplicate
/// offsets would cause port collisions.
#[test]
fn test_validation_reservation_duplicate_offsets() {
    let mut services = std::collections::HashMap::new();
    services.insert(
        "web".to_string(),
        ServiceDefinition {
            offset: Some(0),
            preferred: None,
            env: None,
        },
    );
    services.insert(
        "api".to_string(),
        ServiceDefinition {
            offset: Some(0), // Duplicate!
            preferred: None,
            env: None,
        },
    );

    let config = Config {
        reservations: Some(ReservationGroup {
            base: Some(5000),
            services,
        }),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of reservation group preferred port uniqueness.
///
/// Preferred ports must be unique across all services in a group.
#[test]
fn test_validation_reservation_duplicate_preferred() {
    let mut services = std::collections::HashMap::new();
    services.insert(
        "web".to_string(),
        ServiceDefinition {
            offset: Some(0),
            preferred: Some(5050),
            env: None,
        },
    );
    services.insert(
        "api".to_string(),
        ServiceDefinition {
            offset: Some(1),
            preferred: Some(5050), // Duplicate!
            env: None,
        },
    );

    let config = Config {
        reservations: Some(ReservationGroup {
            base: Some(5000),
            services,
        }),
        ..Default::default()
    };

    let result = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build();

    assert!(result.is_err());
}

/// Test validation of environment variable names in reservation groups.
///
/// Env var names must:
/// - Start with an ASCII letter or underscore
/// - Contain only ASCII letters, digits, and underscores
/// - Be at most 255 bytes
/// - Be unique within the group under ASCII-case-insensitive comparison
#[test]
fn test_validation_reservation_env_var_names() {
    // Invalid: starts with number
    {
        let mut services = std::collections::HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some("9WEB_PORT".to_string()), // Invalid
            },
        );

        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let result = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build();

        assert!(result.is_err());
    }

    // Invalid: contains invalid characters
    {
        let mut services = std::collections::HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some("WEB-PORT".to_string()), // Hyphen not allowed
            },
        );

        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let result = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build();

        assert!(result.is_err());
    }

    // Invalid: duplicate env var names
    {
        let mut services = std::collections::HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some("PORT".to_string()),
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(1),
                preferred: None,
                env: Some("PORT".to_string()), // Duplicate
            },
        );

        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let result = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build();

        assert!(result.is_err());
    }
}

/// Test the exact portable grammar and byte limit through the full config builder.
#[test]
fn test_validation_reservation_env_var_name_boundaries() {
    for env_name in ["_PRIVATE_PORT".to_string(), format!("A{}", "_".repeat(254))] {
        let services = std::collections::HashMap::from([(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some(env_name),
            },
        )]);
        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        assert!(ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build()
            .is_ok());
    }

    for invalid_name in ["PORT_CAFÉ".to_string(), format!("A{}", "_".repeat(255))] {
        let services = std::collections::HashMap::from([(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some(invalid_name.clone()),
            },
        )]);
        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let error = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build()
            .unwrap_err();
        let diagnostic = error.to_string();
        assert!(!diagnostic.contains(&invalid_name));
        assert!(!diagnostic.contains('\n'));
        assert!(!diagnostic.contains('\r'));
    }
}

/// Test that broad service tags require an explicit portable mapping when
/// ASCII-only derivation cannot produce a valid environment-variable name.
#[test]
fn test_validation_reservation_nonconvertible_tags_require_explicit_mapping() {
    for tag in [
        "api server",
        "api;export ATTACK=1",
        "api$(command)",
        "api\nexport ATTACK=1",
        "123-api",
        "api.café",
        "ß",
    ] {
        let services = std::collections::HashMap::from([(
            tag.to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            },
        )]);
        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let error = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build()
            .unwrap_err();
        let diagnostic = error.to_string();
        assert!(!diagnostic.contains(tag));
        assert!(!diagnostic.contains('\n'));
        assert!(!diagnostic.contains('\r'));
    }

    let services = std::collections::HashMap::from([(
        "api\nexport ATTACK=1".to_string(),
        ServiceDefinition {
            offset: Some(0),
            preferred: None,
            env: Some("_SAFE_API_PORT".to_string()),
        },
    )]);
    let config = Config {
        reservations: Some(ReservationGroup {
            base: Some(5000),
            services,
        }),
        ..Default::default()
    };

    assert!(ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(config)
        .build()
        .is_ok());
}

/// Test every resolved-name collision mode through the full config builder.
#[test]
fn test_validation_reservation_resolved_env_var_collisions() {
    let collision_sets = [
        [("web-server", None), ("web_server", None)],
        [("web", Some("api")), ("api", None)],
        [("web", Some("PORT")), ("api", Some("port"))],
    ];

    for collision_set in collision_sets {
        let services = collision_set
            .into_iter()
            .enumerate()
            .map(|(offset, (tag, env))| {
                (
                    tag.to_string(),
                    ServiceDefinition {
                        offset: Some(u16::try_from(offset).unwrap()),
                        preferred: None,
                        env: env.map(str::to_string),
                    },
                )
            })
            .collect();
        let config = Config {
            reservations: Some(ReservationGroup {
                base: Some(5000),
                services,
            }),
            ..Default::default()
        };

        let error = ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(config)
            .build()
            .unwrap_err();
        assert!(matches!(error, Error::Validation { .. }));
    }
}

// ============================================================================
// Category 5: YAML Parsing Tests
// ============================================================================

/// Test loading a minimal valid configuration file.
///
/// This validates that YAML parsing works for the simplest possible config.
#[test]
fn test_yaml_parsing_minimal_config() {
    // The fixture files have specific names (minimal.yaml, complete.yaml, etc.)
    // but the loader looks for trop.yaml. We need to create a temp dir with trop.yaml
    let temp = TempDir::new().unwrap();
    let fixture_content = fs::read_to_string(fixture_path("valid/minimal.yaml")).unwrap();
    create_temp_config(temp.path(), "trop.yaml", &fixture_content);

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    assert_eq!(config.project, Some("minimal-test".to_string()));
}

/// Test loading a complete configuration with all fields.
///
/// This ensures we can parse a config file that exercises every possible
/// field in the schema.
#[test]
fn test_yaml_parsing_complete_config() {
    let temp = TempDir::new().unwrap();
    let fixture_content = fs::read_to_string(fixture_path("valid/complete.yaml")).unwrap();
    create_temp_config(temp.path(), "trop.yaml", &fixture_content);

    // Use empty data dir to isolate from user's global config
    let data_dir = temp.path().join("data");
    fs::create_dir(&data_dir).unwrap();

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .with_data_dir(&data_dir)
        .skip_env()
        .build()
        .unwrap();

    // Verify all fields were parsed
    assert_eq!(config.project, Some("complete-test".to_string()));
    assert_eq!(config.ports.as_ref().unwrap().min, 6000);
    assert_eq!(config.ports.as_ref().unwrap().max, Some(8000));
    assert_eq!(config.cleanup.as_ref().unwrap().expire_after_days, Some(45));
    assert_eq!(config.maximum_lock_wait_seconds, Some(10));
    assert_eq!(config.output_format, Some(OutputFormat::Json));
    assert_eq!(config.disable_autoinit, Some(false));
    assert_eq!(
        config
            .occupancy_check
            .as_ref()
            .unwrap()
            .check_all_interfaces,
        Some(true)
    );
    assert_eq!(config.excluded_ports.as_ref().unwrap().len(), 3);
}

/// Test parsing port range formats.
///
/// Port ranges can be specified as:
/// - String: "5000..5010"
/// - Object: {start: 5000, end: 5010}
///
///   Both should work.
#[test]
fn test_yaml_parsing_port_range_formats() {
    let temp = TempDir::new().unwrap();
    let fixture_content = fs::read_to_string(fixture_path("valid/with_exclusions.yaml")).unwrap();
    create_temp_config(temp.path(), "trop.yaml", &fixture_content);

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    let excluded = config.excluded_ports.as_ref().unwrap();

    // Should contain both single ports and ranges
    let has_single = excluded
        .iter()
        .any(|e| matches!(e, PortExclusion::Single(_)));
    let has_range = excluded
        .iter()
        .any(|e| matches!(e, PortExclusion::Range { .. }));

    assert!(has_single);
    assert!(has_range);
}

/// Test that unknown fields are rejected.
///
/// The schema uses deny_unknown_fields, so YAML files with unrecognized
/// fields should be rejected. This helps catch typos and outdated configs.
#[test]
fn test_yaml_parsing_unknown_fields_rejected() {
    let temp = TempDir::new().unwrap();
    let fixture_content = fs::read_to_string(fixture_path("invalid/unknown_field.yaml")).unwrap();
    create_temp_config(temp.path(), "trop.yaml", &fixture_content);

    let result = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build();

    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should mention the unknown field (the error comes from serde/yaml parsing)
    // It will be a Validation error from our loader
    assert!(
        matches!(err, Error::Validation { .. }),
        "Expected Validation error, got: {err:?}"
    );
}

/// Test that malformed YAML produces good error messages.
///
/// When YAML syntax is invalid, the error should indicate what went wrong
/// and where (file path, line number if possible).
#[test]
fn test_yaml_parsing_malformed_yaml_error() {
    let temp = TempDir::new().unwrap();
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: test
invalid yaml here: [unclosed bracket
ports:
  min: 5000
",
    );

    let result = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build();

    assert!(result.is_err());
    let err = result.unwrap_err();
    // The loader wraps YAML parse errors as Validation errors
    assert!(matches!(err, Error::Validation { .. }));
}

/// Test optional field handling.
///
/// Most fields in the config schema are optional. This test ensures that
/// omitting fields works correctly and doesn't cause parsing errors.
#[test]
fn test_yaml_parsing_optional_fields() {
    let temp = TempDir::new().unwrap();
    create_temp_config(temp.path(), "trop.yaml", "project: optional-test\n");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    // Project is set
    assert_eq!(config.project, Some("optional-test".to_string()));

    // But we still get defaults for other fields
    assert!(config.ports.is_some()); // From defaults
    assert_eq!(config.ports.unwrap().min, 5000);
}

// ============================================================================
// Category 6: Reservation Group Tests
// ============================================================================

/// Test loading and validating a configuration with reservation groups.
///
/// Reservation groups are a complex feature with many validation rules.
/// This test ensures basic parsing and validation works.
#[test]
fn test_reservation_groups_complete_parsing() {
    let temp = TempDir::new().unwrap();
    let fixture_content = fs::read_to_string(fixture_path("valid/with_reservations.yaml")).unwrap();
    create_temp_config(temp.path(), "trop.yaml", &fixture_content);

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    let reservations = config.reservations.as_ref().unwrap();
    assert_eq!(reservations.base, Some(5050));
    assert_eq!(reservations.services.len(), 4);

    // Check specific services
    let web = reservations.services.get("web").unwrap();
    assert_eq!(web.offset, Some(0));
    assert_eq!(web.preferred, Some(5050));
    assert_eq!(web.env, Some("WEB_PORT".to_string()));

    let api = reservations.services.get("api").unwrap();
    assert_eq!(api.offset, Some(1));
}

/// Test that service definitions can have different field combinations.
///
/// Services can specify:
/// - Just offset
/// - Just preferred (with explicit offset to avoid default 0)
/// - Just env (with explicit offset)
/// - Any combination
///
///   All valid combinations should work.
#[test]
fn test_reservation_groups_service_combinations() {
    let temp = TempDir::new().unwrap();
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: combo-test
reservations:
  base: 5000
  services:
    just_offset:
      offset: 0
    just_preferred:
      offset: 3
      preferred: 6000
    just_env:
      offset: 1
      env: SERVICE_PORT
    all_fields:
      offset: 2
      preferred: 6002
      env: FULL_PORT
",
    );

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build()
        .unwrap();

    let services = &config.reservations.as_ref().unwrap().services;
    assert_eq!(services.len(), 4);

    let just_offset = services.get("just_offset").unwrap();
    assert_eq!(just_offset.offset, Some(0));
    assert_eq!(just_offset.preferred, None);
    assert_eq!(just_offset.env, None);

    let just_preferred = services.get("just_preferred").unwrap();
    assert_eq!(just_preferred.offset, Some(3));
    assert_eq!(just_preferred.preferred, Some(6000));
    assert_eq!(just_preferred.env, None);

    let all_fields = services.get("all_fields").unwrap();
    assert!(all_fields.offset.is_some());
    assert!(all_fields.preferred.is_some());
    assert!(all_fields.env.is_some());
}

/// Test the default offset for services with preferred ports.
///
/// Every omitted service offset means zero, so two preferred services that
/// both omit it violate the same offset-uniqueness rule as any other pair.
#[test]
fn test_reservation_groups_preferred_services_share_default_offset_zero() {
    let temp = TempDir::new().unwrap();
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: multi-default
reservations:
  base: 5000
  services:
    web:
      preferred: 5000
    api:
      preferred: 5001
",
    );

    let result = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .skip_env()
        .build();

    let error = result.expect_err("Both omitted offsets should resolve to zero and conflict");
    assert!(
        matches!(
            error,
            Error::Validation {
                ref field,
                ref message
            } if field.starts_with("reservations.services.offset")
                && message == "Duplicate offset: 0"
        ),
        "Expected a precise duplicate-default-offset error, got {error}"
    );
}

// ============================================================================
// Category 7: End-to-End Integration Tests
// ============================================================================

/// Test complete workflow from file discovery through validation.
///
/// This is a comprehensive end-to-end test that exercises the entire
/// configuration system: discovering files in a directory hierarchy,
/// merging them correctly, applying environment variables, and validating
/// the result.
#[test]
fn test_end_to_end_complete_workflow() {
    let fixture_base = fixture_path("hierarchy");
    let child_dir = fixture_base.join("child");

    let config = ConfigBuilder::new()
        .with_working_dir(&child_dir)
        .skip_env()
        .build()
        .unwrap();

    // Should load child/trop.yaml and child/trop.local.yaml
    // Child's project should override parent's
    assert_eq!(config.project, Some("child-project".to_string()));

    // Port config from child's trop.yaml
    assert_eq!(config.ports.as_ref().unwrap().min, 7000);
    assert_eq!(config.ports.as_ref().unwrap().max, Some(8000));

    // Local overrides from trop.local.yaml should apply
    assert_eq!(config.disable_autoinit, Some(true));
    assert_eq!(config.maximum_lock_wait_seconds, Some(15));

    // Excluded ports should accumulate from both files
    let excluded = config.excluded_ports.as_ref().unwrap();
    assert!(excluded.len() >= 2); // At least from both files
}

/// Test workflow with environment variables overriding file config.
///
/// This validates that the complete precedence chain works in a realistic
/// scenario with multiple sources.
#[test]
#[serial]
fn test_end_to_end_with_environment_overrides() {
    let temp = TempDir::new().unwrap();

    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: env-test
ports:
  min: 5000
  max: 6000
disable_autoinit: false
",
    );

    // Environment overrides
    // Note: min=5500 is chosen to be within the file's max=6000 range to avoid validation errors
    let _port = EnvGuard::new("TROP_PORT_MIN", "5500");
    let _flag = EnvGuard::new("TROP_DISABLE_AUTOINIT", "true");
    let _excluded = EnvGuard::new("TROP_EXCLUDED_PORTS", "5500,5600");

    let config = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .build()
        .unwrap();

    // Project from file
    assert_eq!(config.project, Some("env-test".to_string()));

    // Port min from env var
    assert_eq!(config.ports.as_ref().unwrap().min, 5500);

    // Port max from file (not overridden)
    assert_eq!(config.ports.as_ref().unwrap().max, Some(6000));

    // Flag from env var
    assert_eq!(config.disable_autoinit, Some(true));

    // Excluded ports from env var
    assert!(config.excluded_ports.is_some());
}

/// Test that validation runs after merging all sources.
///
/// If the merged result is invalid, we should get a validation error
/// even if individual sources were valid.
#[test]
#[serial]
fn test_end_to_end_validation_after_merge() {
    let _guards = clear_trop_env_vars();
    let temp = TempDir::new().unwrap();

    // File has valid min
    create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
ports:
  min: 5000
  max: 7000
",
    );

    // Env var sets max lower than min, creating invalid state
    let _env = EnvGuard::new("TROP_PORT_MAX", "4000");

    let result = ConfigBuilder::new().with_working_dir(temp.path()).build();

    // Should fail validation
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("TROP_PORT_MAX"),
        "final merged error should name the winning environment source"
    );
}

/// Test configuration with real file system structure.
///
/// This test creates a realistic directory structure with multiple levels
/// and config files, simulating a real project setup.
#[test]
fn test_end_to_end_realistic_directory_structure() {
    let temp = TempDir::new().unwrap();
    let project_root = temp.path();
    let workspace = project_root.join("workspace");
    let service_a = workspace.join("service-a");
    let service_b = workspace.join("service-b");

    fs::create_dir_all(&service_a).unwrap();
    fs::create_dir_all(&service_b).unwrap();

    // Root config
    create_temp_config(
        project_root,
        "trop.yaml",
        r"
project: monorepo
ports:
  min: 5000
  max: 9000
cleanup:
  expire_after_days: 30
",
    );

    // Service A config: discovery stops at first dir with config files,
    // so each service needs its own trop.yaml
    create_temp_config(
        &service_a,
        "trop.yaml",
        r"
excluded_ports:
  - 5050
",
    );

    // Service B config
    create_temp_config(
        &service_b,
        "trop.yaml",
        r"
excluded_ports:
  - 6060
disable_autoinit: true
",
    );

    // Load from service A - discovers service-a/trop.yaml and stops
    let config_a = ConfigBuilder::new()
        .with_working_dir(&service_a)
        .skip_env()
        .build()
        .unwrap();

    // Service A's config only has excluded_ports (no project field)
    assert!(!config_a.excluded_ports.as_ref().unwrap().is_empty());

    // Load from service B
    let config_b = ConfigBuilder::new()
        .with_working_dir(&service_b)
        .skip_env()
        .build()
        .unwrap();

    assert_eq!(config_b.disable_autoinit, Some(true));
}

/// Test that skip_files and skip_env flags work correctly.
///
/// These flags allow users to control which sources are loaded, which is
/// useful for testing and debugging.
#[test]
#[serial]
fn test_end_to_end_skip_flags() {
    let _guards = clear_trop_env_vars();
    let temp = TempDir::new().unwrap();
    create_temp_config(temp.path(), "trop.yaml", "project: skip-test\n");

    let _env = EnvGuard::new("TROP_PROJECT", "from-env");

    // Skip files - should only get env and defaults
    // Programmatic config establishes tropfile context for project field validation
    {
        let programmatic = Config {
            project: None,
            ..Default::default()
        };
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_files()
            .with_config(programmatic)
            .build()
            .unwrap();

        assert_eq!(config.project, Some("from-env".to_string()));
    }

    // Skip env - should only get files and defaults
    {
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_env()
            .build()
            .unwrap();

        assert_eq!(config.project, Some("skip-test".to_string()));
    }

    // Skip both - should only get defaults
    {
        let config = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_files()
            .skip_env()
            .build()
            .unwrap();

        assert!(config.project.is_none()); // No project in defaults
        assert_eq!(config.ports.as_ref().unwrap().min, 5000); // Default
    }
}

/// Test error message quality for common mistakes.
///
/// When users make mistakes, error messages should be helpful and actionable,
/// clearly indicating what's wrong and how to fix it.
#[test]
fn test_end_to_end_error_message_quality() {
    // Test 1: Invalid port in file
    {
        let temp = TempDir::new().unwrap();
        create_temp_config(
            temp.path(),
            "trop.yaml",
            r"
ports:
  min: 70000  # Too high
",
        );

        let result = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_env()
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        // Error should mention the file and the field
        assert!(matches!(
            err,
            Error::Configuration { .. } | Error::Validation { .. }
        ));
    }

    // Test 2: Validation error after merge
    {
        let temp = TempDir::new().unwrap();
        let fixture_content =
            fs::read_to_string(fixture_path("invalid/bad_port_range.yaml")).unwrap();
        create_temp_config(temp.path(), "trop.yaml", &fixture_content);

        let result = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_env()
            .build();

        assert!(result.is_err());
    }

    // Test 3: Unknown field error
    {
        let temp = TempDir::new().unwrap();
        let fixture_content =
            fs::read_to_string(fixture_path("invalid/unknown_field.yaml")).unwrap();
        create_temp_config(temp.path(), "trop.yaml", &fixture_content);

        let result = ConfigBuilder::new()
            .with_working_dir(temp.path())
            .skip_env()
            .build();

        assert!(result.is_err());
    }
}

// ============================================================================
// Category 9: Effective Configuration and Provenance Tests
// ============================================================================

const ALL_EFFECTIVE_FIELDS: &[ConfigField] = &[
    ConfigField::Project,
    ConfigField::DisableAutoinit,
    ConfigField::DisableAutoprune,
    ConfigField::DisableAutoexpire,
    ConfigField::OutputFormat,
    ConfigField::AllowUnrelatedPath,
    ConfigField::AllowChangeProject,
    ConfigField::AllowChangeTask,
    ConfigField::AllowChange,
    ConfigField::MaximumLockWaitSeconds,
    ConfigField::OccupancySkip,
    ConfigField::OccupancySkipIp4,
    ConfigField::OccupancySkipIp6,
    ConfigField::OccupancySkipTcp,
    ConfigField::OccupancySkipUdp,
    ConfigField::OccupancyCheckAllInterfaces,
    ConfigField::PortsMin,
    ConfigField::PortsMax,
    ConfigField::PortsMaxOffset,
    ConfigField::ExcludedPorts,
    ConfigField::CleanupExpireAfterDays,
    ConfigField::Reservations,
];

fn source_layer(source: &ConfigValueSource) -> &'static str {
    match source {
        ConfigValueSource::BuiltIn => "built-in",
        ConfigValueSource::File { kind, .. } => match kind {
            ConfigFileKind::User => "user",
            ConfigFileKind::Project => "project",
            ConfigFileKind::Local => "local",
            _ => "unknown-file",
        },
        ConfigValueSource::Environment { .. } => "environment",
        ConfigValueSource::CommandLine => "cli",
        ConfigValueSource::Programmatic => "programmatic",
        _ => "unknown",
    }
}

/// Semantic built-in defaults retain provenance even when stored as `None`.
#[test]
fn test_effective_config_tracks_absent_built_in_values() {
    let effective = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .build_effective()
        .unwrap();

    for field in [
        ConfigField::Project,
        ConfigField::Reservations,
        ConfigField::ExcludedPorts,
        ConfigField::PortsMaxOffset,
    ] {
        assert_eq!(
            effective.provenance(field).unwrap().winner(),
            &ConfigValueSource::BuiltIn,
            "missing built-in provenance for {field:?}"
        );
    }
}

/// Every field follows the complete documented precedence hierarchy.
#[test]
#[serial]
#[allow(clippy::too_many_lines)]
fn test_effective_config_all_fields_precedence_matrix() {
    let _guards = clear_trop_env_vars();
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    create_temp_config(
        &data_dir,
        "config.yaml",
        r"
disable_autoinit: true
disable_autoprune: true
disable_autoexpire: true
output_format: json
allow_unrelated_path: true
allow_change_project: true
allow_change_task: true
allow_change: true
maximum_lock_wait_seconds: 11
ports:
  min: 5100
  max: 5900
excluded_ports: [5101]
cleanup:
  expire_after_days: 11
occupancy_check:
  skip: true
  skip_ip4: true
  skip_ip6: true
  skip_tcp: true
  skip_udp: true
  check_all_interfaces: true
",
    );
    create_temp_config(
        &project_dir,
        "trop.yaml",
        r"
project: project-layer
disable_autoinit: false
disable_autoprune: false
disable_autoexpire: false
output_format: csv
allow_unrelated_path: false
allow_change_project: false
allow_change_task: false
allow_change: false
maximum_lock_wait_seconds: 22
ports:
  min: 5200
  max: 6000
excluded_ports: [5201]
cleanup:
  expire_after_days: 22
occupancy_check:
  skip: false
  skip_ip4: false
  skip_ip6: false
  skip_tcp: false
  skip_udp: false
  check_all_interfaces: false
reservations:
  services:
    project:
      offset: 0
      env: PROJECT_PORT
",
    );
    create_temp_config(
        &project_dir,
        "trop.local.yaml",
        r"
project: local-layer
disable_autoinit: true
disable_autoprune: true
disable_autoexpire: true
output_format: table
allow_unrelated_path: true
allow_change_project: true
allow_change_task: true
allow_change: true
maximum_lock_wait_seconds: 33
ports:
  min: 5300
  max: 6100
excluded_ports: [5301]
cleanup:
  expire_after_days: 33
occupancy_check:
  skip: true
  skip_ip4: true
  skip_ip6: true
  skip_tcp: true
  skip_udp: true
  check_all_interfaces: true
reservations:
  services:
    local:
      offset: 0
      env: LOCAL_PORT
",
    );

    let _environment = [
        EnvGuard::new("TROP_PROJECT", "environment-layer"),
        EnvGuard::new("TROP_DISABLE_AUTOINIT", "false"),
        EnvGuard::new("TROP_DISABLE_AUTOPRUNE", "false"),
        EnvGuard::new("TROP_DISABLE_AUTOEXPIRE", "false"),
        EnvGuard::new("TROP_OUTPUT_FORMAT", "csv"),
        EnvGuard::new("TROP_ALLOW_UNRELATED_PATH", "false"),
        EnvGuard::new("TROP_ALLOW_PROJECT_CHANGE", "false"),
        EnvGuard::new("TROP_ALLOW_TASK_CHANGE", "false"),
        EnvGuard::new("TROP_ALLOW_CHANGE", "false"),
        EnvGuard::new("TROP_BUSY_TIMEOUT", "44"),
        EnvGuard::new("TROP_PORT_MIN", "5400"),
        EnvGuard::new("TROP_PORT_MAX", "6200"),
        EnvGuard::new("TROP_EXCLUDED_PORTS", "5401"),
        EnvGuard::new("TROP_EXPIRE_AFTER_DAYS", "44"),
        EnvGuard::new("TROP_SKIP_OCCUPANCY_CHECK", "false"),
        EnvGuard::new("TROP_SKIP_IPV4", "false"),
        EnvGuard::new("TROP_SKIP_IPV6", "false"),
        EnvGuard::new("TROP_SKIP_TCP", "false"),
        EnvGuard::new("TROP_SKIP_UDP", "false"),
        EnvGuard::new("TROP_CHECK_ALL_INTERFACES", "false"),
    ];

    let cli = Config {
        project: Some("cli-layer".to_string()),
        ports: Some(PortConfig {
            min: 6500,
            max: Some(6900),
            max_offset: None,
        }),
        excluded_ports: Some(vec![PortExclusion::Single(6501)]),
        cleanup: Some(CleanupConfig {
            expire_after_days: Some(55),
        }),
        occupancy_check: Some(OccupancyConfig {
            skip: Some(false),
            skip_ip4: Some(false),
            skip_ip6: Some(false),
            skip_tcp: Some(false),
            skip_udp: Some(false),
            check_all_interfaces: Some(false),
        }),
        reservations: Some(ReservationGroup {
            base: Some(6500),
            services: HashMap::from([(
                "cli".to_string(),
                ServiceDefinition {
                    offset: Some(0),
                    preferred: None,
                    env: Some("CLI_PORT".to_string()),
                },
            )]),
        }),
        disable_autoinit: Some(false),
        disable_autoprune: Some(false),
        disable_autoexpire: Some(false),
        allow_unrelated_path: Some(false),
        allow_change_project: Some(false),
        allow_change_task: Some(false),
        allow_change: Some(false),
        maximum_lock_wait_seconds: Some(55),
        output_format: Some(OutputFormat::Tsv),
    };
    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(&project_dir)
        .with_cli_config_fields(cli, ALL_EFFECTIVE_FIELDS.iter().copied())
        .build_effective()
        .unwrap();

    assert_eq!(effective.project(), Some("cli-layer"));
    assert_eq!(effective.ports().min, 6500);
    assert_eq!(effective.ports().max, Some(6900));
    assert_eq!(effective.ports().max_offset, None);
    assert_eq!(effective.excluded_ports().len(), 5);
    assert_eq!(effective.cleanup().expire_after_days, Some(55));
    assert_eq!(
        effective.occupancy_check(),
        &OccupancyConfig {
            skip: Some(false),
            skip_ip4: Some(false),
            skip_ip6: Some(false),
            skip_tcp: Some(false),
            skip_udp: Some(false),
            check_all_interfaces: Some(false),
        }
    );
    assert!(effective.reservations().is_some());
    assert!(!effective.disable_autoinit());
    assert!(!effective.disable_autoprune());
    assert!(!effective.disable_autoexpire());
    assert!(!effective.allow_unrelated_path());
    assert!(!effective.allow_project_change());
    assert!(!effective.allow_task_change());
    assert_eq!(effective.maximum_lock_wait_seconds(), 55);
    assert_eq!(effective.output_format(), OutputFormat::Tsv);

    for &field in ALL_EFFECTIVE_FIELDS {
        let provenance = effective.provenance(field).unwrap();
        assert_eq!(
            source_layer(provenance.winner()),
            "cli",
            "wrong winner for {field:?}"
        );

        let actual_layers = provenance
            .contributors()
            .iter()
            .map(source_layer)
            .collect::<Vec<_>>();
        let expected_layers: &[&str] = match field {
            ConfigField::Project => &["built-in", "project", "local", "environment", "cli"],
            ConfigField::Reservations => &["built-in", "project", "local", "cli"],
            _ => &["built-in", "user", "project", "local", "environment", "cli"],
        };
        assert_eq!(actual_layers, expected_layers, "wrong layers for {field:?}");
    }
}

/// An explicitly selected CLI field with `None` preserves the lower value.
#[test]
fn test_effective_cli_none_preserves_lower_value_and_source() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let project_path = create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
project: lower
output_format: json
ports:
  min: 6100
  max: 6200
",
    )
    .canonicalize()
    .unwrap();
    let cli = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: None,
            max_offset: None,
        }),
        ..Default::default()
    };
    let effective = ConfigBuilder::new()
        .with_data_dir(data_dir)
        .with_working_dir(temp.path())
        .skip_env()
        .with_cli_config_fields(
            cli,
            [
                ConfigField::Project,
                ConfigField::OutputFormat,
                ConfigField::PortsMax,
            ],
        )
        .build_effective()
        .unwrap();

    assert_eq!(effective.project(), Some("lower"));
    assert_eq!(effective.output_format(), OutputFormat::Json);
    assert_eq!(effective.ports().max, Some(6200));
    for field in [
        ConfigField::Project,
        ConfigField::OutputFormat,
        ConfigField::PortsMax,
    ] {
        assert_eq!(
            effective.provenance(field).unwrap().winner(),
            &ConfigValueSource::File {
                kind: ConfigFileKind::Project,
                path: project_path.clone(),
            }
        );
    }
}

/// Every participating layer is retained in low-to-high precedence order.
#[test]
#[serial]
fn test_effective_config_tracks_complete_precedence() {
    let _guards = clear_trop_env_vars();
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    let user_path = create_temp_config(&data_dir, "config.yaml", "maximum_lock_wait_seconds: 6\n");
    let project_path =
        create_temp_config(&project_dir, "trop.yaml", "maximum_lock_wait_seconds: 7\n")
            .canonicalize()
            .unwrap();
    let local_path = create_temp_config(
        &project_dir,
        "trop.local.yaml",
        "maximum_lock_wait_seconds: 8\n",
    )
    .canonicalize()
    .unwrap();
    let _environment = EnvGuard::new("TROP_BUSY_TIMEOUT", "9");

    let cli = Config {
        maximum_lock_wait_seconds: Some(10),
        ..Default::default()
    };
    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(&project_dir)
        .with_cli_config_fields(cli, [ConfigField::MaximumLockWaitSeconds])
        .build_effective()
        .unwrap();

    assert_eq!(effective.maximum_lock_wait_seconds(), 10);
    let provenance = effective
        .provenance(ConfigField::MaximumLockWaitSeconds)
        .unwrap();
    assert_eq!(provenance.winner(), &ConfigValueSource::CommandLine);
    assert_eq!(
        provenance.contributors(),
        &[
            ConfigValueSource::BuiltIn,
            ConfigValueSource::File {
                kind: ConfigFileKind::User,
                path: user_path,
            },
            ConfigValueSource::File {
                kind: ConfigFileKind::Project,
                path: project_path,
            },
            ConfigValueSource::File {
                kind: ConfigFileKind::Local,
                path: local_path,
            },
            ConfigValueSource::Environment {
                variable: "TROP_BUSY_TIMEOUT",
            },
            ConfigValueSource::CommandLine,
        ]
    );
}

/// Precise CLI leaves must not replace unrelated nested values.
#[test]
fn test_effective_cli_config_preserves_nested_siblings_and_provenance() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let project_path = create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
ports:
  min: 6100
  max_offset: 500
occupancy_check:
  skip_ip4: true
  skip_tcp: false
",
    )
    .canonicalize()
    .unwrap();

    let cli = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(6400),
            max_offset: None,
        }),
        occupancy_check: Some(OccupancyConfig {
            skip_tcp: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(temp.path())
        .skip_env()
        .with_cli_config_fields(cli, [ConfigField::PortsMax, ConfigField::OccupancySkipTcp])
        .build_effective()
        .unwrap();

    assert_eq!(effective.ports().min, 6100);
    assert_eq!(effective.ports().max, Some(6400));
    assert_eq!(effective.ports().max_offset, None);
    assert_eq!(effective.occupancy_check().skip_ip4, Some(true));
    assert_eq!(effective.occupancy_check().skip_tcp, Some(true));

    assert_eq!(
        effective
            .provenance(ConfigField::PortsMin)
            .unwrap()
            .winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Project,
            path: project_path.clone(),
        }
    );
    assert_eq!(
        effective
            .provenance(ConfigField::OccupancySkipIp4)
            .unwrap()
            .winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Project,
            path: project_path,
        }
    );
    assert_eq!(
        effective
            .provenance(ConfigField::OccupancySkipTcp)
            .unwrap()
            .winner(),
        &ConfigValueSource::CommandLine
    );
    assert_eq!(
        effective
            .provenance(ConfigField::PortsMaxOffset)
            .unwrap()
            .winner(),
        &ConfigValueSource::CommandLine
    );
}

/// Historical environment aliases remain accepted, while canonical names win.
#[test]
#[serial]
fn test_effective_environment_canonical_names_win_over_aliases() {
    let _guards = clear_trop_env_vars();
    let _legacy_timeout = EnvGuard::new("TROP_MAXIMUM_LOCK_WAIT_SECONDS", "17");
    let _legacy_project = EnvGuard::new("TROP_ALLOW_CHANGE_PROJECT", "true");
    let _legacy_task = EnvGuard::new("TROP_ALLOW_CHANGE_TASK", "false");

    let aliases = ConfigBuilder::new().skip_files().build_effective().unwrap();
    assert_eq!(aliases.maximum_lock_wait_seconds(), 17);
    assert!(aliases.allow_project_change());
    assert!(!aliases.allow_task_change());
    assert_eq!(
        aliases
            .provenance(ConfigField::MaximumLockWaitSeconds)
            .unwrap()
            .winner(),
        &ConfigValueSource::Environment {
            variable: "TROP_MAXIMUM_LOCK_WAIT_SECONDS",
        }
    );

    let _canonical_timeout = EnvGuard::new("TROP_BUSY_TIMEOUT", "19");
    let _canonical_project = EnvGuard::new("TROP_ALLOW_PROJECT_CHANGE", "false");
    let _canonical_task = EnvGuard::new("TROP_ALLOW_TASK_CHANGE", "true");
    let canonical = ConfigBuilder::new().skip_files().build_effective().unwrap();

    assert_eq!(canonical.maximum_lock_wait_seconds(), 19);
    assert!(!canonical.allow_project_change());
    assert!(canonical.allow_task_change());
    assert_eq!(
        canonical
            .provenance(ConfigField::MaximumLockWaitSeconds)
            .unwrap()
            .winner(),
        &ConfigValueSource::Environment {
            variable: "TROP_BUSY_TIMEOUT",
        }
    );
    assert_eq!(
        canonical
            .provenance(ConfigField::AllowChangeProject)
            .unwrap()
            .winner(),
        &ConfigValueSource::Environment {
            variable: "TROP_ALLOW_PROJECT_CHANGE",
        }
    );
    assert_eq!(
        canonical
            .provenance(ConfigField::AllowChangeTask)
            .unwrap()
            .winner(),
        &ConfigValueSource::Environment {
            variable: "TROP_ALLOW_TASK_CHANGE",
        }
    );
}

/// Project-only fields are rejected from user config with the exact source path.
#[test]
fn test_effective_source_validation_names_invalid_user_file() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    let user_path = create_temp_config(&data_dir, "config.yaml", "project: forbidden\n");

    let error = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(&project_dir)
        .skip_env()
        .build_effective()
        .unwrap_err();
    let message = error.to_string();

    assert!(message.contains("project"), "{message}");
    assert!(
        message.contains(&user_path.display().to_string()),
        "{message}"
    );
}

/// Effective-aware group planners share one merged snapshot and source path.
#[test]
fn test_effective_project_file_feeds_group_planners() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let project_path = create_temp_config(
        temp.path(),
        "chosen.yaml",
        r"
project: effective-group
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
",
    );

    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_project_file(&project_path)
        .skip_env()
        .build_effective()
        .unwrap();

    assert_eq!(effective.ports().min, 5000);
    assert_eq!(effective.ports().max, Some(7000));
    assert_eq!(
        effective.loaded_file(ConfigFileKind::Project),
        Some(project_path.as_path())
    );
    assert_eq!(
        effective
            .provenance(ConfigField::Reservations)
            .unwrap()
            .winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Project,
            path: project_path.clone(),
        }
    );

    let group = ReserveGroupPlan::from_effective(
        ReserveGroupOptions::new(project_path.clone()),
        &effective,
    )
    .unwrap();
    assert_eq!(group.config(), effective.config());

    let autoreserve = AutoreservePlan::from_effective(
        AutoreserveOptions::new(temp.path().to_path_buf()),
        &effective,
    )
    .unwrap();
    assert_eq!(autoreserve.discovered_config_path(), &project_path);
    assert_eq!(autoreserve.config(), effective.config());
}

/// Canonically named explicit group files compose their sibling project layers.
#[test]
fn test_effective_explicit_group_file_loads_project_and_local_siblings() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    let project_path = create_temp_config(
        &project_dir,
        "trop.yaml",
        r"
project: base-project
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
",
    );
    let local_path = create_temp_config(
        &project_dir,
        "trop.local.yaml",
        r"
project: local-project
excluded_ports: [5001]
",
    );

    for nominated_path in [&project_path, &local_path] {
        let effective = ConfigBuilder::new()
            .with_data_dir(&data_dir)
            .with_project_file(nominated_path)
            .skip_env()
            .build_effective()
            .unwrap();

        assert_eq!(effective.project(), Some("local-project"));
        assert_eq!(effective.excluded_ports(), &[PortExclusion::Single(5001)]);
        assert!(effective
            .reservations()
            .unwrap()
            .services
            .contains_key("web"));
        assert_eq!(
            effective.loaded_file(ConfigFileKind::Project),
            Some(project_path.as_path())
        );
        assert_eq!(
            effective.loaded_file(ConfigFileKind::Local),
            Some(local_path.as_path())
        );
        assert_eq!(
            effective
                .provenance(ConfigField::Reservations)
                .unwrap()
                .winner(),
            &ConfigValueSource::File {
                kind: ConfigFileKind::Project,
                path: project_path.clone(),
            }
        );

        let planner = ReserveGroupPlan::from_effective(
            ReserveGroupOptions::new(nominated_path.clone()),
            &effective,
        )
        .unwrap();
        assert_eq!(planner.config_path(), &project_path);
    }
}

/// Reservation overlays distinguish inheritance, atomic replacement, and clear.
#[test]
fn test_effective_reservation_overlay_contract() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let project_path = create_temp_config(
        temp.path(),
        "trop.yaml",
        r"
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
",
    );
    let local_path = create_temp_config(temp.path(), "trop.local.yaml", "excluded_ports: [5001]\n");
    let project_path = project_path.canonicalize().unwrap();
    let local_path = local_path.canonicalize().unwrap();

    let inherited = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(temp.path())
        .skip_env()
        .build_effective()
        .unwrap();
    assert!(inherited
        .reservations()
        .unwrap()
        .services
        .contains_key("web"));
    assert_eq!(
        inherited
            .provenance(ConfigField::Reservations)
            .unwrap()
            .winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Project,
            path: project_path.clone(),
        }
    );

    fs::write(
        &local_path,
        r"
reservations:
  services:
    api:
      offset: 0
      env: API_PORT
",
    )
    .unwrap();
    let replaced = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(temp.path())
        .skip_env()
        .build_effective()
        .unwrap();
    let services = &replaced.reservations().unwrap().services;
    assert_eq!(services.len(), 1);
    assert!(services.contains_key("api"));
    assert_eq!(
        replaced
            .provenance(ConfigField::Reservations)
            .unwrap()
            .winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Local,
            path: local_path.clone(),
        }
    );
    let replaced_planner =
        ReserveGroupPlan::from_effective(ReserveGroupOptions::new(project_path.clone()), &replaced)
            .unwrap();
    assert_eq!(replaced_planner.config_path(), &local_path);

    fs::write(&local_path, "reservations: null\n").unwrap();
    let cleared = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(temp.path())
        .skip_env()
        .build_effective()
        .unwrap();
    assert!(cleared.reservations().is_none());
    let provenance = cleared
        .provenance(ConfigField::Reservations)
        .expect("an explicit clear must retain provenance");
    assert_eq!(
        provenance.winner(),
        &ConfigValueSource::File {
            kind: ConfigFileKind::Local,
            path: local_path.clone(),
        }
    );
    assert_eq!(
        provenance.contributors(),
        &[
            ConfigValueSource::BuiltIn,
            ConfigValueSource::File {
                kind: ConfigFileKind::Project,
                path: project_path,
            },
            ConfigValueSource::File {
                kind: ConfigFileKind::Local,
                path: local_path,
            },
        ]
    );
}

/// Arbitrarily named explicit files remain standalone project sources.
#[test]
fn test_effective_arbitrary_project_file_does_not_infer_siblings() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    create_temp_config(temp.path(), "trop.yaml", "project: sibling\n");
    create_temp_config(temp.path(), "trop.local.yaml", "reservations: null\n");
    let chosen_path = create_temp_config(
        temp.path(),
        "chosen.yaml",
        r"
project: chosen
reservations:
  services:
    chosen:
      offset: 0
      env: CHOSEN_PORT
",
    );

    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_project_file(&chosen_path)
        .skip_env()
        .build_effective()
        .unwrap();

    assert_eq!(effective.project(), Some("chosen"));
    assert!(effective
        .reservations()
        .unwrap()
        .services
        .contains_key("chosen"));
    assert!(effective.loaded_file(ConfigFileKind::Local).is_none());
    assert_eq!(
        effective.loaded_file(ConfigFileKind::Project),
        Some(chosen_path.as_path())
    );
}

/// A generated user-wide null is inert because global config cannot own groups.
#[test]
fn test_effective_user_config_reservations_null_is_inert() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    create_temp_config(&data_dir, "config.yaml", "reservations: null\n");
    let project_path = create_temp_config(
        &project_dir,
        "trop.yaml",
        r"
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
",
    )
    .canonicalize()
    .unwrap();

    let effective = ConfigBuilder::new()
        .with_data_dir(&data_dir)
        .with_working_dir(&project_dir)
        .skip_env()
        .build_effective()
        .unwrap();

    assert!(effective
        .reservations()
        .unwrap()
        .services
        .contains_key("web"));
    assert_eq!(
        effective
            .provenance(ConfigField::Reservations)
            .unwrap()
            .contributors(),
        &[
            ConfigValueSource::BuiltIn,
            ConfigValueSource::File {
                kind: ConfigFileKind::Project,
                path: project_path,
            },
        ]
    );
}
