//! Boundary tests for the configuration glue around the verified
//! configuration kernel (`trop/src/verified/config.rs`): environment-variable
//! text, the lock-wait bound shared with the database, built-in defaults,
//! file placement, and project-file discovery.

use serial_test::serial;
use std::env;
use std::fs;
use std::time::Duration;
use tempfile::TempDir;
use trop::config::{
    Config, ConfigBuilder, ConfigFileKind, ConfigLoader, EnvironmentConfig, OutputFormat,
    PortExclusion,
};
use trop::{Database, DatabaseConfig};

/// Sets an environment variable for the guard's lifetime.
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn new(key: &'static str, value: &str) -> Self {
        let previous = env::var(key).ok();
        env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => env::set_var(self.key, value),
            None => env::remove_var(self.key),
        }
    }
}

fn isolated_builder(dir: &TempDir) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_working_dir(dir.path())
        .with_data_dir(dir.path())
}

#[test]
#[serial]
fn yaml_environment_and_parser_accept_the_same_exclusion_text() {
    for text in ["5000 .. 5010", " 5000..5010 ", "5000..5010", "+5000..05010"] {
        let parsed: PortExclusion = text.parse().unwrap();
        assert_eq!(
            parsed,
            PortExclusion::Range {
                start: 5000,
                end: 5010
            },
            "{text:?}"
        );

        let yaml: Config =
            serde_yaml::from_str(&format!("excluded_ports:\n  - \"{text}\"\n")).unwrap();
        assert_eq!(yaml.excluded_ports, Some(vec![parsed.clone()]), "{text:?}");

        let _guard = EnvGuard::new("TROP_EXCLUDED_PORTS", text);
        let mut config = Config::default();
        EnvironmentConfig::apply_overrides(&mut config).unwrap();
        assert_eq!(config.excluded_ports, Some(vec![parsed]), "{text:?}");
    }
}

#[test]
#[serial]
fn numeric_environment_variables_use_the_unsigned_decimal_syntax() {
    let dir = TempDir::new().unwrap();
    {
        let _days = EnvGuard::new("TROP_EXPIRE_AFTER_DAYS", "+0030");
        let _wait = EnvGuard::new("TROP_BUSY_TIMEOUT", "007");
        let _min = EnvGuard::new("TROP_PORT_MIN", "+6000");
        let config = isolated_builder(&dir).build_effective().unwrap();
        assert_eq!(config.cleanup().expire_after_days, Some(30));
        assert_eq!(config.maximum_lock_wait_seconds(), 7);
        assert_eq!(config.ports().min, 6000);
    }
    for (variable, value) in [
        ("TROP_EXPIRE_AFTER_DAYS", " 30"),
        ("TROP_EXPIRE_AFTER_DAYS", "4294967296"),
        ("TROP_EXPIRE_AFTER_DAYS", "-1"),
        ("TROP_BUSY_TIMEOUT", "5 "),
        ("TROP_BUSY_TIMEOUT", "18446744073709551616"),
        ("TROP_PORT_MIN", "65536"),
        ("TROP_PORT_MAX", ""),
    ] {
        let _guard = EnvGuard::new(variable, value);
        let error = isolated_builder(&dir).build_effective().unwrap_err();
        assert!(
            error.to_string().contains(variable),
            "{variable}={value:?}: {error}"
        );
    }
}

#[test]
#[serial]
fn boolean_and_output_format_variables_ignore_ascii_case_only() {
    let dir = TempDir::new().unwrap();
    {
        let _skip = EnvGuard::new("TROP_DISABLE_AUTOPRUNE", "YeS");
        let _format = EnvGuard::new("TROP_OUTPUT_FORMAT", "JsOn");
        let config = isolated_builder(&dir).build_effective().unwrap();
        assert!(config.disable_autoprune());
        assert_eq!(config.output_format(), OutputFormat::Json);
    }
    for (variable, value) in [
        ("TROP_DISABLE_AUTOPRUNE", " yes"),
        ("TROP_DISABLE_AUTOPRUNE", "o\u{212A}"),
        ("TROP_OUTPUT_FORMAT", "json "),
        ("TROP_OUTPUT_FORMAT", "yaml"),
    ] {
        let _guard = EnvGuard::new(variable, value);
        assert!(
            isolated_builder(&dir).build_effective().is_err(),
            "{variable}={value:?}"
        );
    }
}

#[test]
fn largest_accepted_lock_wait_opens_the_database() {
    let dir = TempDir::new().unwrap();
    let accepted = Config {
        maximum_lock_wait_seconds: Some(2_147_483),
        ..Default::default()
    };
    let config = isolated_builder(&dir)
        .skip_env()
        .with_config(accepted)
        .build_effective()
        .unwrap();
    let timeout = Duration::from_secs(config.maximum_lock_wait_seconds());
    Database::open(DatabaseConfig::new(dir.path().join("trop.db")).with_busy_timeout(timeout))
        .unwrap();

    let rejected = Config {
        maximum_lock_wait_seconds: Some(2_147_484),
        ..Default::default()
    };
    let error = isolated_builder(&dir)
        .skip_env()
        .with_config(rejected)
        .build_effective()
        .unwrap_err();
    assert!(error.to_string().contains("2147483 seconds"), "{error}");
}

#[test]
fn built_in_defaults_are_the_documented_values() {
    let dir = TempDir::new().unwrap();
    let config = isolated_builder(&dir).skip_env().build_effective().unwrap();
    assert_eq!(config.cleanup().expire_after_days, Some(30));
    assert_eq!(config.maximum_lock_wait_seconds(), 5);
    assert_eq!(config.output_format(), OutputFormat::Table);
    assert!(!config.disable_autoinit());
    assert!(!config.disable_autoprune());
    assert!(!config.disable_autoexpire());
    assert!(!config.allow_unrelated_path());
    assert!(!config.allow_project_change());
    assert!(!config.allow_task_change());
}

#[test]
fn allow_change_grants_both_permissions_and_specific_flags_grant_their_own() {
    let dir = TempDir::new().unwrap();
    let build = |change, project, task| {
        isolated_builder(&dir)
            .skip_env()
            .with_config(Config {
                allow_change: change,
                allow_change_project: project,
                allow_change_task: task,
                ..Default::default()
            })
            .build_effective()
            .unwrap()
    };
    let flags = [None, Some(false), Some(true)];
    for change in flags {
        for project in flags {
            for task in flags {
                let config = build(change, project, task);
                let on = |flag: Option<bool>| flag == Some(true);
                assert_eq!(config.allow_project_change(), on(change) || on(project));
                assert_eq!(config.allow_task_change(), on(change) || on(task));
            }
        }
    }
}

#[test]
fn user_file_reservations_null_is_inert_and_a_user_group_is_rejected() {
    let dir = TempDir::new().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("trop.yaml"),
        "reservations:\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();

    // The user file's `reservations: null` does not clear the project group
    // (it is lower precedence anyway, and inert).
    fs::write(dir.path().join("config.yaml"), "reservations: null\n").unwrap();
    let config = ConfigBuilder::new()
        .with_working_dir(&project)
        .with_data_dir(dir.path())
        .skip_env()
        .build_effective()
        .unwrap();
    assert!(config.reservations().is_some());

    fs::write(
        dir.path().join("config.yaml"),
        "reservations:\n  services:\n    api:\n      offset: 1\n",
    )
    .unwrap();
    let error = ConfigBuilder::new()
        .with_working_dir(&project)
        .with_data_dir(dir.path())
        .skip_env()
        .build_effective()
        .unwrap_err();
    assert!(
        error.to_string().contains("reservations are only valid"),
        "{error}"
    );
}

#[test]
fn discovery_stops_at_the_nearest_level_with_either_file() {
    let dir = TempDir::new().unwrap();
    let child = dir.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(dir.path().join("trop.yaml"), "project: parent\n").unwrap();
    // Only the private file at the nearest level: the parent's trop.yaml is
    // not loaded.
    fs::write(child.join("trop.local.yaml"), "project: child-local\n").unwrap();

    let sources = ConfigLoader::discover_project_configs(&child).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].precedence, 3);
    assert_eq!(sources[0].config.project.as_deref(), Some("child-local"));

    let config = ConfigBuilder::new()
        .with_working_dir(&child)
        .with_data_dir(dir.path())
        .skip_env()
        .build_effective()
        .unwrap();
    assert_eq!(config.project(), Some("child-local"));
    assert_eq!(config.loaded_files().len(), 1);
    assert_eq!(config.loaded_files()[0].kind(), ConfigFileKind::Local);

    // Both files at one level: trop.yaml first (lower precedence).
    fs::write(child.join("trop.yaml"), "project: child\n").unwrap();
    let sources = ConfigLoader::discover_project_configs(&child).unwrap();
    let precedences: Vec<u8> = sources.iter().map(|source| source.precedence).collect();
    assert_eq!(precedences, [2, 3]);
}
