//! `trop reserve-group --dry-run` and `trop autoreserve --dry-run` predict the
//! live outcome, and group allocation runs the automatic cleanup step.
//!
//! A group dry run validates the output format and shell, then plans and
//! executes exactly as a live run does against a private in-memory copy of
//! the database: it fails with the live run's exit code, prints the ports the
//! live run would print in the requested format, and never creates or changes
//! the database. Exhausted group allocations prune/expire once and retry, as
//! single reservations do, unless the configuration disables it.

mod common;

use common::TestEnv;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// A project directory holding a two-service group in `5000..=max`.
fn project(env: &TestEnv, max: u16) -> (PathBuf, PathBuf) {
    let project_dir = env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    fs::write(
        &config_path,
        format!(
            "project: dry-run-cli
ports:
  min: 5000
  max: {max}
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
"
        ),
    )
    .unwrap();
    (project_dir, config_path)
}

/// Runs `reserve-group` from the project directory.
fn reserve_group(env: &TestEnv, project_dir: &Path, config: &Path, args: &[&str]) -> Output {
    env.command()
        .current_dir(project_dir)
        .arg("reserve-group")
        .arg(config)
        .arg("--skip-occupancy-check")
        .args(args)
        .output()
        .unwrap()
}

/// Every stored row with its timestamps, in a stable order.
fn rows(env: &TestEnv) -> Vec<(String, String, i64, i64, i64)> {
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT path, tag, port, created_at, last_used_at FROM reservations
             ORDER BY path, tag",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The dry run fails exactly like the live run: same exit code, same
/// diagnostic.
fn assert_same_failure(dry: &Output, live: &Output, what: &str) {
    assert!(
        !dry.status.success(),
        "{what}: dry run succeeded: {}",
        stderr(dry)
    );
    assert!(!live.status.success(), "{what}: live run succeeded");
    assert_eq!(dry.status.code(), live.status.code(), "{what}: exit codes");
    assert_eq!(stderr(dry), stderr(live), "{what}: diagnostics");
}

#[test]
fn dry_run_rejects_an_unknown_shell_like_a_live_run() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);

    let dry = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--shell", "csh", "--dry-run"],
    );
    assert!(
        !env.data_dir.exists(),
        "a dry run never creates the database"
    );
    let live = reserve_group(&env, &project_dir, &config, &["--shell", "csh"]);

    assert_same_failure(&dry, &live, "unknown shell");
    assert!(
        stderr(&dry).contains("unknown shell type"),
        "{}",
        stderr(&dry)
    );
}

#[test]
fn non_export_formats_ignore_the_shell_in_dry_and_live_runs() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);
    let args = ["--format", "json", "--shell", "csh"];

    let dry = reserve_group(
        &env,
        &project_dir,
        &config,
        &[&args[..], &["--dry-run"]].concat(),
    );
    assert!(dry.status.success(), "{}", stderr(&dry));
    let live = reserve_group(&env, &project_dir, &config, &args);
    assert!(live.status.success(), "{}", stderr(&live));
    assert_eq!(stdout(&dry), stdout(&live));
}

#[test]
fn dry_run_prints_the_ports_a_live_run_prints_without_creating_a_database() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);

    for format in ["json", "dotenv", "human"] {
        let dry = reserve_group(
            &env,
            &project_dir,
            &config,
            &["--format", format, "--dry-run"],
        );
        assert!(dry.status.success(), "{format}: {}", stderr(&dry));
        assert!(
            !env.data_dir.exists(),
            "{format}: no database may be created"
        );
        assert!(stderr(&dry).contains("Dry run"), "{}", stderr(&dry));
        assert!(stdout(&dry).contains("5000"), "{format}: {}", stdout(&dry));
    }

    let dry = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--format", "json", "--dry-run"],
    );
    let live = reserve_group(&env, &project_dir, &config, &["--format", "json"]);
    assert!(live.status.success(), "{}", stderr(&live));
    assert_eq!(stdout(&dry), stdout(&live));
    let ports: serde_json::Value = serde_json::from_str(&stdout(&live)).unwrap();
    assert_eq!(ports["web"], 5000);
    assert_eq!(ports["api"], 5001);
}

#[test]
fn dry_run_reports_a_sticky_task_change_like_a_live_run() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);
    let seeded = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--format", "json", "--task", "alpha"],
    );
    assert!(seeded.status.success(), "{}", stderr(&seeded));
    let before = rows(&env);

    let dry = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--format", "json", "--task", "beta", "--dry-run"],
    );
    assert_eq!(rows(&env), before, "a dry run must not write");
    let live = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--format", "json", "--task", "beta"],
    );

    assert_same_failure(&dry, &live, "sticky task");
    assert_eq!(rows(&env), before);
}

#[test]
fn dry_run_rejects_an_overlong_task_like_a_live_run() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);
    let task = "t".repeat(300);

    let dry = reserve_group(&env, &project_dir, &config, &["--task", &task, "--dry-run"]);
    let live = reserve_group(&env, &project_dir, &config, &["--task", &task]);

    assert_same_failure(&dry, &live, "overlong task");
    assert!(stderr(&live).contains("'task'"), "{}", stderr(&live));
    assert!(stderr(&live).contains("255 bytes"), "{}", stderr(&live));
    assert!(!env.data_dir.join("trop.db").exists() || rows(&env).is_empty());
}

#[test]
fn dry_run_from_an_unrelated_directory_fails_like_a_live_run() {
    let env = TestEnv::new();
    let (_, config) = project(&env, 9000);
    let elsewhere = env.create_dir("elsewhere");

    let dry = reserve_group(
        &env,
        &elsewhere,
        &config,
        &["--format", "json", "--dry-run"],
    );
    let live = reserve_group(&env, &elsewhere, &config, &["--format", "json"]);

    assert_same_failure(&dry, &live, "unrelated path");
}

#[test]
fn dry_run_with_autoinit_disabled_fails_like_a_live_run_without_a_database() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);
    let run = |dry_run: bool| {
        let mut command = env.command();
        command
            .current_dir(&project_dir)
            .arg("--disable-autoinit")
            .arg("reserve-group")
            .arg(&config)
            .arg("--skip-occupancy-check")
            .args(["--format", "json"]);
        if dry_run {
            command.arg("--dry-run");
        }
        command.output().unwrap()
    };

    let dry = run(true);
    let live = run(false);

    assert_same_failure(&dry, &live, "disabled autoinit");
    assert!(!env.data_dir.join("trop.db").exists());
}

#[test]
fn autoreserve_dry_run_reports_the_stored_group_without_refreshing_it() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 9000);
    let seeded = reserve_group(&env, &project_dir, &config, &["--format", "dotenv"]);
    assert!(seeded.status.success(), "{}", stderr(&seeded));
    {
        let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
        connection
            .execute("UPDATE reservations SET last_used_at = 2000", [])
            .unwrap();
    }
    let before = rows(&env);

    let dry = env
        .command()
        .current_dir(&project_dir)
        .args([
            "autoreserve",
            "--skip-occupancy-check",
            "--format",
            "dotenv",
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(dry.status.success(), "{}", stderr(&dry));
    assert_eq!(stdout(&dry), stdout(&seeded));
    assert!(
        stderr(&dry).contains("Discovered config"),
        "{}",
        stderr(&dry)
    );
    assert_eq!(rows(&env), before, "a dry run must not refresh");
}

/// Stores a reservation of port 5000 for an existing unrelated directory and
/// makes it older than the default 30-day expiry.
fn stale_reservation_on_5000(env: &TestEnv) {
    let other = env.create_dir("other");
    let reserved = env
        .command()
        .current_dir(&env.temp_path)
        .arg("reserve")
        .arg("--path")
        .arg(&other)
        .args(["--port", "5000", "--skip-occupancy-check"])
        .output()
        .unwrap();
    assert!(reserved.status.success(), "{}", stderr(&reserved));
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    connection
        .execute(
            "UPDATE reservations SET created_at = strftime('%s', 'now') - 3456000,
             last_used_at = strftime('%s', 'now') - 3456000",
            [],
        )
        .unwrap();
}

#[test]
fn reserve_group_expires_a_stale_reservation_and_retries() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 5001);
    stale_reservation_on_5000(&env);

    let dry = reserve_group(
        &env,
        &project_dir,
        &config,
        &["--format", "json", "--dry-run"],
    );
    assert!(dry.status.success(), "{}", stderr(&dry));
    assert_eq!(rows(&env).len(), 1, "the dry run cleaned up only its copy");

    let live = reserve_group(&env, &project_dir, &config, &["--format", "json"]);
    assert!(live.status.success(), "{}", stderr(&live));
    assert!(
        stderr(&live).contains("Automatic cleanup"),
        "{}",
        stderr(&live)
    );
    assert_eq!(stdout(&dry), stdout(&live));
    let ports: serde_json::Value = serde_json::from_str(&stdout(&live)).unwrap();
    assert_eq!(ports["web"], 5000);
    assert_eq!(ports["api"], 5001);
    assert_eq!(rows(&env).len(), 2, "the stale reservation was expired");
}

#[test]
fn reserve_group_respects_disabled_automatic_cleanup() {
    let env = TestEnv::new();
    let (project_dir, config) = project(&env, 5001);
    stale_reservation_on_5000(&env);
    let before = rows(&env);

    let output = env
        .command()
        .current_dir(&project_dir)
        .env("TROP_DISABLE_AUTOPRUNE", "true")
        .env("TROP_DISABLE_AUTOEXPIRE", "true")
        .arg("reserve-group")
        .arg(&config)
        .args(["--skip-occupancy-check", "--format", "json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("complete offset fallback pattern"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("automatic cleanup"),
        "{}",
        stderr(&output)
    );
    assert_eq!(rows(&env), before, "no cleanup may run");
}
