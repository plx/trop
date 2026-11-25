//! CLI regression tests for the output boundary: shell auto-detection for
//! `--format export` and escaping of tags in `--format human`.

mod common;

use std::fs;
use std::path::Path;

use common::TestEnv;

/// A group configuration whose allocation does not depend on probing the
/// host's sockets.
fn write_config(path: &Path, services: &str) {
    let config = format!(
        r#"
project: "output-boundaries"

ports:
  min: 21000
  max: 21999
occupancy_check:
  skip: true

reservations:
  base: 21000
  services:
{services}"#
    );
    fs::write(path, config).expect("Failed to write config");
}

fn reserve_group_export(env: &TestEnv, config: &Path, shell: Option<&str>) -> std::process::Output {
    let mut command = env.command();
    command
        .arg("reserve-group")
        .arg(config)
        .arg("--format")
        .arg("export")
        .arg("--allow-unrelated-path");
    for variable in ["ZSH_VERSION", "FISH_VERSION", "PSModulePath", "SHELL"] {
        command.env_remove(variable);
    }
    if let Some(shell) = shell {
        command.env("SHELL", shell);
    }
    command.output().expect("Failed to run reserve-group")
}

/// `SHELL=/home/selfish/bin/bash` used to produce fish syntax (the
/// directory name contains "fish"), which bash `eval`s as `set -x ...`.
#[test]
fn export_detects_the_shell_from_the_final_path_component_only() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    let config = dir.join("trop.yaml");
    write_config(
        &config,
        "    web:\n      offset: 0\n      env: WEB_PORT\n    api:\n      offset: 1\n      env: API_PORT\n",
    );

    for shell in ["/home/selfish/bin/bash", "/opt/powershell-addons/bin/bash"] {
        let output = reserve_group_export(&env, &config, Some(shell));
        assert!(
            output.status.success(),
            "SHELL={shell}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(
            lines,
            ["export API_PORT=21001", "export WEB_PORT=21000"],
            "SHELL={shell}"
        );
    }
}

/// With no shell indicator and no `--shell`, export fails before reserving
/// anything instead of guessing bash.
#[test]
fn export_without_a_detectable_shell_fails_before_reserving() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    let config = dir.join("trop.yaml");
    write_config(&config, "    web:\n      offset: 0\n");

    for shell in [None, Some("/bin/tcsh")] {
        let output = reserve_group_export(&env, &config, shell);
        assert!(!output.status.success(), "SHELL={shell:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("--shell"), "{stderr}");
        assert!(output.stdout.is_empty());
        assert_eq!(env.reservation_count(), 0);
    }
}

/// A tag containing a newline used to forge an extra `service: port` line in
/// human output.
#[test]
fn human_output_escapes_tags() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    let config = dir.join("trop.yaml");
    write_config(
        &config,
        "    \"evil\\nweb: 1\":\n      offset: 0\n      env: EVIL_PORT\n",
    );

    let output = env
        .command()
        .arg("reserve-group")
        .arg(&config)
        .arg("--format")
        .arg("human")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run reserve-group");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, ["Reserved ports:", "  \"evil\\nweb: 1\": 21000"]);
}
