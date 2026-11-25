//! Port bounds on the command line: `--min` / `--max`, the `TROP_PORT_*`
//! environment layer, `reservations.base`, and how a project's `max_offset`
//! combines with overrides.
//!
//! Every command runs with its working directory inside a temporary
//! project, so configuration discovery never reaches the repository.

mod common;

use common::{parse_port, TestEnv};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// A project directory holding `trop.yaml` with `contents`.
fn project(env: &TestEnv, contents: &str) -> PathBuf {
    let dir = env.create_dir("project");
    fs::write(dir.join("trop.yaml"), contents).unwrap();
    dir
}

/// Run `trop <args>` inside `dir` with exactly the given `TROP_PORT_*`
/// variables.
fn trop_in(env: &TestEnv, dir: &Path, args: &[&str], vars: &[(&str, &str)]) -> Output {
    let mut command = env.command();
    command.current_dir(dir);
    for variable in ["TROP_PORT_MIN", "TROP_PORT_MAX", "TROP_PORT_MAX_OFFSET"] {
        command.env_remove(variable);
    }
    for (key, value) in vars {
        command.env(key, value);
    }
    command.args(args).output().unwrap()
}

/// `trop reserve` for the project directory itself, without OS probes.
fn reserve(env: &TestEnv, dir: &Path, args: &[&str], vars: &[(&str, &str)]) -> Output {
    let path = dir.to_str().unwrap();
    let mut all = vec!["reserve", "--path", path, "--skip-occupancy-check"];
    all.extend_from_slice(args);
    trop_in(env, dir, &all, vars)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Regression: `--min` over a project `max_offset` used to re-anchor the
/// offset, so `--min 6000` allocated 6000 although the project range is
/// `5000..=5005`. It is now rejected exactly like the equivalent explicit
/// `max: 5005`.
#[test]
fn min_override_does_not_move_a_project_offset() {
    for contents in [
        "ports:\n  min: 5000\n  max_offset: 5\n",
        "ports:\n  min: 5000\n  max: 5005\n",
    ] {
        let env = TestEnv::new();
        let dir = project(&env, contents);

        let output = reserve(&env, &dir, &["--min", "6000"], &[]);
        assert_eq!(
            output.status.code(),
            Some(7),
            "{contents}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("max must be >= min"),
            "{contents}: {}",
            stderr(&output)
        );

        let output = reserve(&env, &dir, &[], &[("TROP_PORT_MIN", "6010")]);
        assert_eq!(
            output.status.code(),
            Some(7),
            "{contents}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains("max must be >= min"),
            "{contents}: {}",
            stderr(&output)
        );

        // A `--min` inside the range keeps the project's upper bound.
        let output = reserve(&env, &dir, &["--min", "5003"], &[]);
        assert!(output.status.success(), "{contents}: {}", stderr(&output));
        let port = parse_port(&String::from_utf8_lossy(&output.stdout));
        assert!((5003..=5005).contains(&port), "{contents}: port {port}");
    }
}

/// Regression: with both `TROP_PORT_MAX` and `TROP_PORT_MAX_OFFSET` set, the
/// offset used to win silently; it is now a configuration error with the
/// same message as a YAML document.
#[test]
fn environment_with_both_upper_bounds_is_rejected() {
    let env = TestEnv::new();
    let dir = project(&env, "project: bounds\n");
    let output = reserve(
        &env,
        &dir,
        &[],
        &[("TROP_PORT_MAX", "6000"), ("TROP_PORT_MAX_OFFSET", "10")],
    );
    assert_eq!(output.status.code(), Some(7), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Cannot specify both max and max_offset"),
        "{}",
        stderr(&output)
    );
}

/// Regression: `trop validate` used to accept a `reservations.base` outside
/// the document's own range (the error only surfaced in `reserve-group`).
#[test]
fn validate_rejects_group_base_outside_the_range() {
    let env = TestEnv::new();
    let dir = project(
        &env,
        "ports:\n  min: 5000\n  max: 5010\n\
         reservations:\n  base: 9000\n  services:\n    web:\n      offset: 0\n",
    );
    let output = trop_in(&env, &dir, &["validate", "trop.yaml"], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Base port 9000 must be within port range 5000..5010"),
        "{}",
        stderr(&output)
    );

    // The same document with the base in range validates.
    let dir = env.create_dir("valid");
    fs::write(
        dir.join("trop.yaml"),
        "ports:\n  min: 5000\n  max_offset: 10\n\
         reservations:\n  base: 5010\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();
    let output = trop_in(&env, &dir, &["validate", "trop.yaml"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
}

/// Regression: a bad `--min` value was reported as
/// "Invalid arguments: invalid --min value: Invalid arguments: ...".
#[test]
fn reserve_reports_a_bad_bound_once() {
    let env = TestEnv::new();
    let dir = project(&env, "project: bounds\n");
    for (flag, value, message) in [
        (
            "--min",
            "70000",
            "Invalid port number: 70000 is greater than maximum port 65535",
        ),
        (
            "--max",
            "0",
            "Invalid port number: port 0 is reserved and cannot be used",
        ),
    ] {
        let output = reserve(&env, &dir, &[flag, value], &[]);
        assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
        let text = stderr(&output);
        assert!(
            text.contains(&format!(
                "Invalid arguments: invalid {flag} value: {message}"
            )),
            "{text}"
        );
        assert_eq!(text.matches("Invalid arguments").count(), 1, "{text}");
    }
}

/// Regression: `trop scan` accepted `--min` above `--max` (and `--min 0`)
/// and failed later with a configuration error (exit 7). It now rejects
/// them as invalid arguments (exit 4), exactly like `trop reserve`.
#[test]
fn scan_and_reserve_reject_bad_bounds_alike() {
    let env = TestEnv::new();
    let dir = project(&env, "project: bounds\n");
    let reversed = "Invalid port range: min (6000) must be less than or equal to max (5000)";

    let output = trop_in(&env, &dir, &["scan", "--min", "6000", "--max", "5000"], &[]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(stderr(&output).contains(reversed), "{}", stderr(&output));

    let output = reserve(&env, &dir, &["--min", "6000", "--max", "5000"], &[]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
    assert!(stderr(&output).contains(reversed), "{}", stderr(&output));

    for flag in ["--min", "--max"] {
        let output = trop_in(&env, &dir, &["scan", flag, "0"], &[]);
        assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!(
                "invalid {flag} value: Invalid port number: port 0 is reserved and cannot be used"
            )),
            "{}",
            stderr(&output)
        );
    }
}
