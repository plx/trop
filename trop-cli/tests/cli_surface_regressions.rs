//! Regression tests for the command-line surface: exit codes, verbosity,
//! data-directory selection, table escaping, `init`, list filters, `scan`'s
//! occupancy policy, and consistent argument validation.
//!
//! Each test names the behavior that failed before the verified
//! `verified::cli` kernel took over the decision.

mod common;

use std::fs;
use std::net::TcpListener;
use std::path::Path;

use common::TestEnv;

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A bare `trop` command in a scratch directory with the home and data
/// directory variables controlled by the test.
fn trop_without_dirs(cwd: &Path) -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("trop").expect("trop binary");
    command
        .current_dir(cwd)
        .env_remove("TROP_DATA_DIR")
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .env_remove("TROP_LOG_MODE");
    command
}

// ============================================================================
// (a) Usage errors exit 4; 2 is reserved for lock timeouts
// ============================================================================

/// `trop scan --min 70000` used to exit 2 (clap's usage code, which the
/// specification reserves for lock timeouts) while `trop reserve --min 70000`
/// exited 4. Both are invalid arguments now, with the same message.
#[test]
fn out_of_range_port_arguments_exit_4_like_reserve() {
    let env = TestEnv::new();
    let scan = env
        .command()
        .args(["scan", "--min", "70000"])
        .output()
        .unwrap();
    let reserve = env
        .command()
        .args(["reserve", "--min", "70000", "--allow-unrelated-path"])
        .output()
        .unwrap();
    assert_eq!(scan.status.code(), Some(4), "{scan:?}");
    assert_eq!(reserve.status.code(), Some(4), "{reserve:?}");
    assert_eq!(stderr(&scan), stderr(&reserve));
    assert!(
        stderr(&scan).contains("70000 is greater than maximum port 65535"),
        "{}",
        stderr(&scan)
    );

    for args in [
        &["assert-port", "70000"][..],
        &["port-info", "70000"],
        &["assert-port", "0"],
        &["port-info", "abc"],
    ] {
        let output = env.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(4), "{args:?}: {output:?}");
    }
}

/// Every clap usage error exits 4 (not 2); `--help` and `--version` exit 0.
#[test]
fn usage_errors_exit_4_and_help_exits_0() {
    let env = TestEnv::new();
    for args in [
        &["bogus"][..],
        &[],
        &["--no-such-flag", "list"],
        &["list", "--format", "xml"],
        &["release", "--tag", "web", "--untagged-only", "--no-such"],
        &["port-info", "5050", "--skip-tcp"],
    ] {
        let output = env.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(4), "{args:?}: {output:?}");
        assert!(!stderr(&output).is_empty(), "{args:?}");
    }
    for args in [&["--help"][..], &["--version"], &["list", "--help"]] {
        let output = env.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(!stdout(&output).is_empty(), "{args:?}");
    }
}

// ============================================================================
// (b) One verbosity for the logger and the commands
// ============================================================================

/// `TROP_LOG_MODE` had no effect: commands only looked at the flags.
#[test]
fn trop_log_mode_quiet_silences_command_summaries() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    env.reserve_simple(&dir);

    let normal = env.command().arg("prune").output().unwrap();
    assert!(normal.status.success(), "{normal:?}");
    assert!(!stderr(&normal).is_empty(), "prune reports a summary");

    let quiet = env
        .command()
        .arg("prune")
        .env("TROP_LOG_MODE", "quiet")
        .output()
        .unwrap();
    assert!(quiet.status.success(), "{quiet:?}");
    assert_eq!(stderr(&quiet), "", "TROP_LOG_MODE=quiet must silence prune");

    // A flag overrides the environment.
    let verbose = env
        .command()
        .args(["--verbose", "prune"])
        .env("TROP_LOG_MODE", "quiet")
        .output()
        .unwrap();
    assert!(!stderr(&verbose).is_empty(), "{verbose:?}");
}

/// `--verbose --quiet` resolved to verbose for the logger but quiet for the
/// commands; it is quiet everywhere now.
#[test]
fn quiet_wins_over_verbose() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    env.reserve_simple(&dir);
    let output = env
        .command()
        .args(["--verbose", "--quiet", "prune"])
        .env("TROP_LOG_MODE", "verbose")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(stderr(&output), "");
}

/// An invalid `TROP_LOG_MODE` is diagnosed (with control characters
/// escaped) and otherwise ignored.
#[test]
fn invalid_trop_log_mode_is_diagnosed_with_escaping() {
    let env = TestEnv::new();
    let output = env
        .command()
        .arg("list")
        .env("TROP_LOG_MODE", "loud\n\u{1b}[2J")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = stderr(&output);
    assert!(
        text.contains("WARN: ignoring invalid TROP_LOG_MODE value `\"loud\\n\\x1b[2J\"`"),
        "{text}"
    );
    assert!(!text.contains('\u{1b}'));

    // Padding stays visible; an empty value counts as unset (no warning).
    let padded = env
        .command()
        .arg("list")
        .env("TROP_LOG_MODE", " quiet")
        .output()
        .unwrap();
    assert!(
        stderr(&padded).contains("TROP_LOG_MODE value ` quiet`:"),
        "{padded:?}"
    );
    let empty = env
        .command()
        .arg("list")
        .env("TROP_LOG_MODE", "")
        .output()
        .unwrap();
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(stderr(&empty), "");

    let quiet = env
        .command()
        .args(["--quiet", "list"])
        .env("TROP_LOG_MODE", "loud")
        .output()
        .unwrap();
    assert_eq!(stderr(&quiet), "");
}

// ============================================================================
// (c), (d) Data-directory selection
// ============================================================================

/// `trop show-data-dir` with HOME and USERPROFILE unset panicked (exit
/// 101); every command that needs a data directory now reports it missing
/// with exit 3.
#[test]
fn no_home_reports_no_data_directory_without_panicking() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        &["show-data-dir"][..],
        &["list"],
        &["init"],
        &["assert-data-dir"],
    ] {
        let output = trop_without_dirs(temp.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{args:?}: {output:?}");
        let text = stderr(&output);
        assert!(!text.contains("panicked"), "{text}");
        assert!(text.contains("HOME"), "{text}");
    }
    // Commands that need no data directory still work.
    let output = trop_without_dirs(temp.path())
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    // The directory was not touched.
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

/// `HOME=''` and `HOME=relhome` used to select `.trop` and `relhome/.trop`
/// relative to the current directory; an unusable HOME is now skipped in
/// favor of USERPROFILE, and otherwise the data directory is missing.
#[test]
fn empty_or_relative_home_is_not_used() {
    let temp = tempfile::tempdir().unwrap();
    for home in ["", "relhome", "."] {
        let output = trop_without_dirs(temp.path())
            .env("HOME", home)
            .arg("show-data-dir")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3), "HOME={home:?}: {output:?}");
    }
    #[cfg(unix)]
    {
        let profile = temp.path().join("profile");
        let output = trop_without_dirs(temp.path())
            .env("HOME", "relhome")
            .env("USERPROFILE", &profile)
            .arg("show-data-dir")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            stdout(&output),
            format!("{}\n", profile.join(".trop").display())
        );
    }
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

// ============================================================================
// (e) Table cells are escaped
// ============================================================================

/// A tag, project, or task with a newline or tab printed one reservation
/// across several lines (or with extra columns).
#[test]
fn list_table_keeps_each_reservation_on_one_line() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    let output = env
        .command()
        .args(["reserve", "--allow-unrelated-path", "--path"])
        .arg(&dir)
        .args(["--tag", "x\ny", "--project", "evil\nproj", "--task", "t\tx"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let output = env.command().arg("list").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let table = stdout(&output);
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 2, "{table}");
    assert_eq!(lines[1].split('\t').count(), 7, "{table}");
    assert!(
        lines[1].contains("\"x\\ny\"\t\"evil\\nproj\"\t\"t\\tx\""),
        "{table}"
    );

    let output = env.command().arg("list-projects").output().unwrap();
    assert_eq!(stdout(&output), "\"evil\\nproj\"\n");

    let port = lines[1].split('\t').next().unwrap();
    let output = env.command().args(["port-info", port]).output().unwrap();
    let info = stdout(&output);
    assert!(info.contains("Tag: \"x\\ny\"\n"), "{info}");
    assert!(info.contains("Project: \"evil\\nproj\"\n"), "{info}");
    assert!(info.contains("Task: \"t\\tx\"\n"), "{info}");
}

/// `trop list` shortens paths under `HOME` component by component, and shows
/// the home directory itself as `~/` (as it always has); a sibling that
/// merely shares the home directory's text as a prefix is not shortened.
#[test]
fn list_shortens_home_paths_component_wise() {
    let env = TestEnv::new();
    let home = env.create_dir("home");
    let below = env.create_dir("home/src");
    let sibling = env.create_dir("homestead");
    for path in [&home, &below, &sibling] {
        let output = env
            .command()
            .env("HOME", &home)
            .args(["reserve", "--allow-unrelated-path", "--path"])
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let output = env
        .command()
        .env("HOME", &home)
        .arg("list")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let table = stdout(&output);
    let paths: Vec<&str> = table
        .lines()
        .skip(1)
        .map(|line| line.split('\t').nth(1).unwrap())
        .collect();
    assert!(paths.contains(&"~/"), "{table}");
    assert!(paths.contains(&"~/src"), "{table}");
    assert!(
        paths.contains(&sibling.display().to_string().as_str()),
        "{table}"
    );
}

// ============================================================================
// (f) init
// ============================================================================

/// `init --dry-run` printed "ERROR: Database already exists" but exited 0
/// while the live run exited 6.
#[test]
fn init_dry_run_fails_exactly_like_the_live_run() {
    let env = TestEnv::new();
    let init = |extra: &[&str]| {
        env.command_bare()
            .arg("init")
            .arg("--data-dir")
            .arg(&env.data_dir)
            .args(extra)
            .output()
            .unwrap()
    };
    let first = init(&["--dry-run"]);
    assert!(first.status.success(), "{first:?}");
    assert!(!env.data_dir.exists(), "a dry run changes nothing");
    assert!(init(&[]).status.success());

    let dry = init(&["--dry-run"]);
    let live = init(&[]);
    assert_eq!(dry.status.code(), Some(6), "{dry:?}");
    assert_eq!(live.status.code(), Some(6), "{live:?}");
    assert_eq!(stderr(&dry), stderr(&live));

    let dry = init(&["--dry-run", "--overwrite"]);
    assert!(dry.status.success(), "{dry:?}");
    assert!(stdout(&dry).contains("Remove existing database"));
}

/// When the data directory's path is a file, `init --dry-run` reported "Data
/// directory already exists" and exited 0 while the live run failed. Both
/// now fail the same way (exit 6, same message) and change nothing.
#[test]
fn init_dry_run_fails_like_the_live_run_when_the_path_is_a_file() {
    let env = TestEnv::new();
    let file = env.create_dir("parent").join("notadir");
    fs::write(&file, b"x").unwrap();
    for data_dir in [file.clone(), file.join("trop")] {
        let init = |extra: &[&str]| {
            env.command_bare()
                .arg("init")
                .arg("--data-dir")
                .arg(&data_dir)
                .args(extra)
                .output()
                .unwrap()
        };
        let dry = init(&["--dry-run"]);
        let live = init(&[]);
        assert_eq!(dry.status.code(), Some(6), "{dry:?}");
        assert_eq!(live.status.code(), Some(6), "{live:?}");
        assert_eq!(stderr(&dry), stderr(&live));
        assert!(stderr(&live).contains("is not a directory"), "{live:?}");
        assert!(!stdout(&dry).contains("already exists"), "{dry:?}");
    }
    assert_eq!(fs::read(&file).unwrap(), b"x");
}

/// `init --overwrite` on a fresh directory reported "Recreated database".
#[test]
fn init_overwrite_on_a_fresh_directory_reports_created() {
    let env = TestEnv::new();
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = stdout(&output);
    assert!(text.contains("  - Created database"), "{text}");
    assert!(!text.contains("Recreated"), "{text}");

    let again = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .output()
        .unwrap();
    assert!(
        stdout(&again).contains("  - Recreated database"),
        "{again:?}"
    );
}

/// The default `config.yaml` written by `init --with-config` used a
/// nonexistent `port_range` key; uncommenting it broke every command. The
/// file as written and with its settings uncommented both load.
#[test]
fn init_config_template_loads_as_written_and_uncommented() {
    let env = TestEnv::new();
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--with-config")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let config_path = env.data_dir.join("config.yaml");
    let template = fs::read_to_string(&config_path).unwrap();
    assert!(!template.contains("port_range"));

    let list = env.command().arg("list").output().unwrap();
    assert!(list.status.success(), "{list:?}");

    let uncommented: String = template
        .lines()
        .map(|line| {
            let rest = line.strip_prefix("# ").unwrap_or(line);
            let body = rest.trim_start();
            let setting = body.starts_with("- ")
                || body.split_once(':').is_some_and(|(key, _)| {
                    !key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                });
            if setting && line.starts_with("# ") {
                format!("{rest}\n")
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    assert!(uncommented.contains("\nports:\n"), "{uncommented}");
    fs::write(&config_path, uncommented).unwrap();
    let list = env.command().arg("list").output().unwrap();
    assert!(list.status.success(), "{list:?}");
}

// ============================================================================
// (g) list filters compare canonical identifiers
// ============================================================================

/// `list --filter-tag ' web'` found nothing although `assert-reservation
/// --tag ' web'` found the reservation (stored tags are trimmed).
#[test]
fn list_filters_compare_trimmed_values() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    let output = env
        .command()
        .args(["reserve", "--allow-unrelated-path", "--path"])
        .arg(&dir)
        .args(["--tag", "web", "--project", "proj"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    for (flag, value, rows) in [
        ("--filter-tag", " web", 1),
        ("--filter-tag", "web\t", 1),
        ("--filter-tag", "we", 0),
        ("--filter-tag", " ", 0),
        ("--filter-project", "  proj ", 1),
        ("--filter-project", "", 0),
    ] {
        let output = env
            .command()
            .args(["list", "--format", "json", flag, value])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let rows_found: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            rows_found.as_array().unwrap().len(),
            rows,
            "{flag} {value:?}"
        );
    }

    let assert = env
        .command()
        .args(["assert-reservation", "--path"])
        .arg(&dir)
        .args(["--tag", " web"])
        .output()
        .unwrap();
    assert!(assert.status.success(), "{assert:?}");
}

// ============================================================================
// (h) scan ignores a configured global skip
// ============================================================================

/// A configured `occupancy_check.skip: true` disabled every probe in `scan`
/// (which has no skip flag), and scan reported no occupied ports.
#[test]
fn scan_probes_despite_a_configured_global_skip() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind TCP fixture");
    let port = listener.local_addr().unwrap().port();
    fs::write(
        project.join("trop.yaml"),
        format!(
            "ports:\n  min: {}\n  max: {port}\noccupancy_check:\n  skip: true\n  skip_udp: true\n  skip_ip6: true\n",
            port - 1
        ),
    )
    .unwrap();

    let output = env
        .command()
        .current_dir(&project)
        .args(["scan", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .any(|row| row["port"] == port),
        "the listener on {port} must be reported: {rows}"
    );
    drop(listener);
}

/// With every probe disabled by protocol or family switches, scan says the
/// ports were not checked instead of reporting that none is occupied.
#[test]
fn scan_without_any_probe_reports_not_checked() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    fs::write(
        project.join("trop.yaml"),
        "ports:\n  min: 40000\n  max: 40001\noccupancy_check:\n  skip_tcp: true\n",
    )
    .unwrap();
    let output = env
        .command()
        .current_dir(&project)
        .args(["scan", "--skip-udp", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(stdout(&output).is_empty(), "{output:?}");
    assert!(
        stderr(&output).contains("ports were not checked"),
        "{output:?}"
    );
}

// ============================================================================
// (i), (n) The same invalid value exits the same way everywhere
// ============================================================================

/// A blank `--tag` exited 6 in assert-reservation but 4 in reserve.
#[test]
fn blank_tag_is_an_invalid_argument_everywhere() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    for args in [
        &["reserve", "--allow-unrelated-path"][..],
        &["release", "--allow-unrelated-path"],
        &["assert-reservation"],
    ] {
        let output = env
            .command()
            .args(args)
            .arg("--path")
            .arg(&dir)
            .args(["--tag", "  "])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{args:?}: {output:?}");
        assert!(
            stderr(&output).contains("tag must be non-empty after trimming whitespace"),
            "{args:?}: {output:?}"
        );
    }
}

/// An invalid `--task` exited 7 for reserve but 6 for reserve-group and
/// autoreserve.
#[test]
fn invalid_task_exits_like_reserve_for_group_commands() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let config = project.join("trop.yaml");
    fs::write(
        &config,
        "ports:\n  min: 21000\n  max: 21999\noccupancy_check:\n  skip: true\nreservations:\n  base: 21000\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();

    let reserve = env
        .command()
        .current_dir(&project)
        .args(["reserve", "--task", " ", "--allow-unrelated-path"])
        .output()
        .unwrap();
    let group = env
        .command()
        .current_dir(&project)
        .arg("reserve-group")
        .arg(&config)
        .args(["--task", " ", "--allow-unrelated-path"])
        .output()
        .unwrap();
    let auto = env
        .command()
        .current_dir(&project)
        .args(["autoreserve", "--task", " ", "--allow-unrelated-path"])
        .output()
        .unwrap();
    assert_eq!(reserve.status.code(), Some(7), "{reserve:?}");
    assert_eq!(group.status.code(), Some(7), "{group:?}");
    assert_eq!(auto.status.code(), Some(7), "{auto:?}");
    assert_eq!(stderr(&group), stderr(&reserve));
    assert_eq!(stderr(&auto), stderr(&reserve));
    assert_eq!(env.reservation_count(), 0);
}

/// An invalid `TROP_TASK` (no `--task` flag) is reported as coming from
/// `TROP_TASK` by all three commands, with the same message and exit code;
/// the group commands must not blame the command line for it.
#[test]
fn invalid_trop_task_is_attributed_to_the_environment_everywhere() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let config = project.join("trop.yaml");
    fs::write(
        &config,
        "ports:\n  min: 21000\n  max: 21999\noccupancy_check:\n  skip: true\nreservations:\n  base: 21000\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();

    let run = |args: &[&std::ffi::OsStr]| {
        env.command()
            .current_dir(&project)
            .env("TROP_TASK", "   ")
            .args(args)
            .arg("--allow-unrelated-path")
            .output()
            .unwrap()
    };
    let reserve = run(&["reserve".as_ref()]);
    let group = run(&["reserve-group".as_ref(), config.as_os_str()]);
    let auto = run(&["autoreserve".as_ref()]);
    assert_eq!(reserve.status.code(), Some(7), "{reserve:?}");
    assert!(
        stderr(&reserve).contains("'task (TROP_TASK)'"),
        "{}",
        stderr(&reserve)
    );
    assert_eq!(group.status.code(), Some(7), "{group:?}");
    assert_eq!(auto.status.code(), Some(7), "{auto:?}");
    assert_eq!(stderr(&group), stderr(&reserve));
    assert_eq!(stderr(&auto), stderr(&reserve));
    assert_eq!(env.reservation_count(), 0);
}
