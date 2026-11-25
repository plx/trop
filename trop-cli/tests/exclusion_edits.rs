//! CLI tests for the commands that edit exclusion lists: `exclude`,
//! `compact-exclusions`, and `scan --autoexclude` / `--autocompact`.
//!
//! The decisions these commands make are verified kernels (`trop::port::
//! exclusions` and the `config_rewrite` kernel); these tests pin the glue:
//! which file is written, that only `excluded_ports` changes in it, and the
//! messages and exit codes.

mod common;

use common::TestEnv;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::Output;

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("Failed to read config")
}

fn yaml(path: &Path) -> serde_yaml::Value {
    serde_yaml::from_str(&read(path)).expect("Failed to parse config")
}

fn exclude(env: &TestEnv, dir: &Path, args: &[&str]) -> Output {
    env.command()
        .current_dir(dir)
        .arg("exclude")
        .args(args)
        .output()
        .expect("Failed to run exclude")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The top-level keys of a YAML mapping, in order.
fn keys(value: &serde_yaml::Value) -> Vec<String> {
    value
        .as_mapping()
        .expect("config must be a mapping")
        .keys()
        .map(|key| key.as_str().expect("keys are strings").to_string())
        .collect()
}

// ============================================================================
// Write target
// ============================================================================

/// Regression: `trop exclude` without a project `trop.yaml` silently wrote
/// the global `config.yaml` and exited 0. The spec requires it to fail unless
/// `--global` is given.
#[test]
fn exclude_without_project_config_fails_unless_global() {
    let env = TestEnv::new();
    let dir = env.create_dir("no-project");
    let global = env.data_dir.join("config.yaml");

    let output = exclude(&env, &dir, &["8080"]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(stderr(&output).contains("--global"), "{}", stderr(&output));
    assert!(!global.exists(), "the global config must not be written");
    assert!(!dir.join("trop.yaml").exists());

    let output = exclude(&env, &dir, &["8080", "--global"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        yaml(&global)["excluded_ports"],
        serde_yaml::from_str::<serde_yaml::Value>("[8080]").unwrap()
    );
}

/// With both project files, `trop.local.yaml` (higher precedence) is written.
#[test]
fn exclude_writes_trop_local_yaml_when_present() {
    let env = TestEnv::new();
    let dir = env.create_dir("project");
    fs::write(dir.join("trop.yaml"), "project: demo\n").unwrap();
    fs::write(dir.join("trop.local.yaml"), "excluded_ports:\n  - 6099\n").unwrap();

    let output = exclude(&env, &dir, &["6098"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(read(&dir.join("trop.yaml")), "project: demo\n");
    assert_eq!(
        read(&dir.join("trop.local.yaml")),
        "excluded_ports:\n- 6099\n- 6098\n"
    );
}

// ============================================================================
// Merge semantics
// ============================================================================

/// Regression: overlapping or covered entries used to be appended (`8080`
/// then `8080..8080` left both). The spec: an identical entry is a no-op and
/// a partially overlapping range is merged with the existing one.
#[test]
fn exclude_merges_overlaps_and_skips_covered_entries() {
    let env = TestEnv::new();
    let dir = env.create_dir("merge");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: demo\n").unwrap();

    assert!(exclude(&env, &dir, &["8080..8090"]).status.success());
    let after_first = read(&config);

    // Identical, and covered: no-ops (the file is not rewritten).
    let mut outputs = Vec::new();
    for argument in ["8080..8090", "8085", "8080..8080", "8090"] {
        let output = exclude(&env, &dir, &[argument]);
        assert!(output.status.success(), "{argument}: {output:?}");
        assert_eq!(read(&config), after_first, "{argument}");
        outputs.push((argument, output));
    }
    for (argument, output) in outputs {
        assert!(
            stdout(&output).contains("already excluded"),
            "{argument}: {}",
            stdout(&output)
        );
    }

    // Adjacent: not an overlap, appended.
    assert!(exclude(&env, &dir, &["8091..8095"]).status.success());
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- start: 8080\n  end: 8090\n- start: 8091\n  end: 8095\n"
    );

    // Overlapping both: merged into one range at the first one's position.
    assert!(exclude(&env, &dir, &["8088..8100"]).status.success());
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- start: 8080\n  end: 8100\n"
    );
}

/// Regression: `8080` followed by `8080..8080` used to leave both entries.
#[test]
fn exclude_single_then_one_port_range_is_a_no_op() {
    let env = TestEnv::new();
    let dir = env.create_dir("single");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: demo\n").unwrap();

    assert!(exclude(&env, &dir, &["8080"]).status.success());
    assert!(exclude(&env, &dir, &["8080..8080"]).status.success());
    assert_eq!(read(&config), "project: demo\nexcluded_ports:\n- 8080\n");
}

/// Regression: a parse failure was reported twice over
/// (`Invalid arguments: Invalid port number: Invalid port: abc`).
#[test]
fn exclude_parse_failure_reports_one_message() {
    let env = TestEnv::new();
    let dir = env.create_dir("parse");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: demo\n").unwrap();

    let output = exclude(&env, &dir, &["abc"]);
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let message = stderr(&output);
    assert!(
        message.contains(
            "Invalid arguments: Invalid exclusion 'abc': expected a port (1-65535) or an \
             inclusive range such as 8080..8090"
        ),
        "{message}"
    );
    assert!(!message.contains("Invalid port number"), "{message}");
    assert_eq!(read(&config), "project: demo\n");
}

// ============================================================================
// Key-preserving rewrites
// ============================================================================

/// Regression: rewriting the whole `Config` wrote `reservations: null` (and
/// other nulls) into `trop.local.yaml`, which turned "inherit the group of
/// `trop.yaml`" into "clear it": `autoreserve` then failed.
#[test]
fn exclude_in_local_file_keeps_the_inherited_reservation_group() {
    let env = TestEnv::new();
    let dir = env.create_dir("layered");
    fs::write(
        dir.join("trop.yaml"),
        r#"project: layered
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
"#,
    )
    .unwrap();
    let local = dir.join("trop.local.yaml");
    fs::write(&local, "excluded_ports:\n  - 6099\n").unwrap();

    let output = exclude(&env, &dir, &["6098"]);
    assert!(output.status.success(), "{output:?}");

    let output = env
        .command()
        .current_dir(&dir)
        .arg("autoreserve")
        .arg("--format")
        .arg("json")
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run autoreserve");
    assert!(
        output.status.success(),
        "autoreserve must still see the inherited group: {}",
        stderr(&output)
    );

    let contents = read(&local);
    assert!(!contents.contains("null"), "{contents}");
    assert_eq!(keys(&yaml(&local)), ["excluded_ports"]);
}

/// Every other top-level key keeps its value, its position, and an explicit
/// `null` stays `null`.
#[test]
fn exclude_rewrites_only_excluded_ports() {
    let env = TestEnv::new();
    let dir = env.create_dir("keys");
    let config = dir.join("trop.yaml");
    fs::write(
        &config,
        "project: demo\nexcluded_ports:\n  - \"5000..5002\"\ncleanup: null\nports:\n  min: 5000\n  max: 7000\n",
    )
    .unwrap();

    assert!(exclude(&env, &dir, &["6000"]).status.success());
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- start: 5000\n  end: 5002\n- 6000\ncleanup: null\nports:\n  min: 5000\n  max: 7000\n"
    );
}

/// Regression (found in review): the configuration reader ignores YAML tags
/// on keys. The rewrite used to keep a tagged `excluded_ports` key and append
/// a second one, after which every command failed with "duplicate field".
#[test]
fn exclude_replaces_a_tagged_excluded_ports_key() {
    let env = TestEnv::new();
    let dir = env.create_dir("tagged-key");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: demo\n!foo excluded_ports: [1]\n").unwrap();

    let output = exclude(&env, &dir, &["8080"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- 1\n- 8080\n"
    );

    let output = exclude(&env, &dir, &["9000"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- 1\n- 8080\n- 9000\n"
    );
}

/// A tagged document is read like an untagged one; `exclude` rewrites it and
/// keeps the tag (it used to be refused as "not a YAML mapping").
#[test]
fn exclude_rewrites_a_tagged_document() {
    let env = TestEnv::new();
    let dir = env.create_dir("tagged-root");
    let config = dir.join("trop.yaml");
    fs::write(&config, "--- !foo\nproject: demo\n").unwrap();

    let output = exclude(&env, &dir, &["8080"]);
    assert!(output.status.success(), "{output:?}");
    let output = exclude(&env, &dir, &["9000"]);
    assert!(output.status.success(), "{output:?}");
    match yaml(&config) {
        serde_yaml::Value::Tagged(tagged) => assert_eq!(
            tagged.value,
            serde_yaml::from_str::<serde_yaml::Value>(
                "project: demo\nexcluded_ports: [8080, 9000]\n"
            )
            .unwrap()
        ),
        other => panic!("the tag was dropped: {other:?}"),
    }
}

/// A rewrite that would change how another key reads (the unquoted project
/// name `0x10` would be re-emitted as the number `16`) is refused, and the
/// file is left alone.
#[test]
fn exclude_refuses_a_rewrite_that_changes_other_keys() {
    let env = TestEnv::new();
    let dir = env.create_dir("canonical-scalar");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: 0x10\n").unwrap();

    let output = exclude(&env, &dir, &["8080"]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert!(
        stderr(&output).contains("only excluded_ports replaced"),
        "{}",
        stderr(&output)
    );
    assert!(!stdout(&output).contains("Added"), "{}", stdout(&output));
    assert_eq!(read(&config), "project: 0x10\n");
}

// ============================================================================
// compact-exclusions
// ============================================================================

/// A configuration error is reported once, as `serde_yaml` describes it (not
/// wrapped in a second "validation error for 'configuration'" prefix).
#[test]
fn compact_exclusions_reports_parse_errors_once() {
    let env = TestEnv::new();
    let config = env.path().join("trop.yaml");
    fs::write(&config, "bogus: 1\nexcluded_ports: [2, 1]\n").unwrap();

    let output = env
        .command()
        .arg("compact-exclusions")
        .arg(&config)
        .output()
        .expect("Failed to run compact-exclusions");
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let message = stderr(&output);
    assert!(
        message.contains("Configuration error: Parse error: unknown field `bogus`"),
        "{message}"
    );
    assert!(!message.contains("validation error"), "{message}");
    assert!(!message.contains("Invalid YAML"), "{message}");
    assert_eq!(read(&config), "bogus: 1\nexcluded_ports: [2, 1]\n");
}

/// Regression: the rewrite decision compared entry counts, so a non-canonical
/// list of minimal size was reported as "already optimal".
#[test]
fn compact_exclusions_canonicalizes_lists_of_minimal_size() {
    let env = TestEnv::new();
    let config = env.path().join("trop.yaml");
    fs::write(
        &config,
        "project: demo\nexcluded_ports:\n  - 9000\n  - 8000\n  - \"7000..7000\"\n",
    )
    .unwrap();

    let output = env
        .command()
        .arg("compact-exclusions")
        .arg(&config)
        .output()
        .expect("Failed to run compact-exclusions");
    assert!(output.status.success(), "{output:?}");
    assert!(
        stdout(&output).contains("Compacted 3 exclusions to 3"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        read(&config),
        "project: demo\nexcluded_ports:\n- 7000\n- 8000\n- 9000\n"
    );

    // Now canonical: nothing to do, and the file is not rewritten.
    let before = read(&config);
    let output = env
        .command()
        .arg("compact-exclusions")
        .arg(&config)
        .output()
        .expect("Failed to run compact-exclusions");
    assert!(stdout(&output).contains("already optimal"), "{output:?}");
    assert_eq!(read(&config), before);
}

/// compact-exclusions keeps absent keys absent in `trop.local.yaml`.
#[test]
fn compact_exclusions_writes_no_null_keys() {
    let env = TestEnv::new();
    let local = env.path().join("trop.local.yaml");
    fs::write(&local, "excluded_ports:\n  - 5001\n  - 5000\n").unwrap();

    let output = env
        .command()
        .arg("compact-exclusions")
        .arg(&local)
        .output()
        .expect("Failed to run compact-exclusions");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        read(&local),
        "excluded_ports:\n- start: 5000\n  end: 5001\n"
    );
}

// ============================================================================
// scan --autoexclude / --autocompact
// ============================================================================

fn scan_port(env: &TestEnv, dir: &Path, port: u16, extra: &[&str]) -> Output {
    env.command()
        .current_dir(dir)
        .arg("scan")
        .arg("--min")
        .arg(port.to_string())
        .arg("--max")
        .arg(port.to_string())
        .arg("--skip-udp")
        .arg("--skip-ipv6")
        .arg("--format")
        .arg("json")
        .args(extra)
        .output()
        .expect("Failed to run scan")
}

/// Regression: scan reported the number of candidate ports ("Added 1
/// exclusions") even when nothing was added.
#[test]
fn scan_autoexclude_reports_ports_actually_added() {
    let env = TestEnv::new();
    let dir = env.create_dir("scan");
    let local = dir.join("trop.local.yaml");
    fs::write(dir.join("trop.yaml"), "project: scan\n").unwrap();
    fs::write(&local, "excluded_ports: []\n").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("Failed to bind TCP fixture");
    let port = listener.local_addr().unwrap().port();

    let first = scan_port(&env, &dir, port, &["--autoexclude"]);
    assert!(first.status.success(), "{}", stderr(&first));
    let second = scan_port(&env, &dir, port, &["--autoexclude"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(
        stderr(&first).contains("Added 1 exclusions"),
        "{}",
        stderr(&first)
    );
    assert!(
        stderr(&second).contains("Added 0 exclusions"),
        "{}",
        stderr(&second)
    );
    assert_eq!(read(&local), format!("excluded_ports:\n- {port}\n"));
    drop(listener);
}

/// `--autocompact` compacts the changed list in the same single rewrite.
#[test]
fn scan_autocompact_compacts_after_adding() {
    let env = TestEnv::new();
    let dir = env.create_dir("scan-compact");
    let config = dir.join("trop.yaml");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("Failed to bind TCP fixture");
    let port = listener.local_addr().unwrap().port();
    fs::write(
        &config,
        format!("project: scan\nexcluded_ports:\n  - {}\n", port - 1),
    )
    .unwrap();

    let output = scan_port(&env, &dir, port, &["--autoexclude", "--autocompact"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("Compacted 2 exclusions to 1"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        read(&config),
        format!(
            "project: scan\nexcluded_ports:\n- start: {}\n  end: {port}\n",
            port - 1
        )
    );
    drop(listener);
}

/// Regression: scan decided whether `--autocompact` rewrites by comparing
/// entry counts, so a changed list whose compaction has the same size was
/// left unsorted.
#[test]
fn scan_autocompact_canonicalizes_a_list_of_unchanged_size() {
    let env = TestEnv::new();
    let dir = env.create_dir("scan-canonical");
    let config = dir.join("trop.yaml");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("Failed to bind TCP fixture");
    let port = listener.local_addr().unwrap().port();
    fs::write(
        &config,
        format!(
            "project: scan\nexcluded_ports:\n  - {}\n  - {}\n",
            port + 10,
            port - 10
        ),
    )
    .unwrap();

    let output = scan_port(&env, &dir, port, &["--autoexclude", "--autocompact"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let message = stderr(&output);
    let added = message.find("Added 1 exclusions").expect(&message);
    let compacted = message.find("Compacted 3 exclusions to 3").expect(&message);
    assert!(added < compacted, "{message}");
    assert_eq!(
        read(&config),
        format!(
            "project: scan\nexcluded_ports:\n- {}\n- {port}\n- {}\n",
            port - 10,
            port + 10
        )
    );
    drop(listener);
}

/// "Added N exclusions" is reported only once the file has been written: a
/// refused rewrite reports an error and nothing as added.
#[test]
fn scan_reports_additions_only_after_writing() {
    let env = TestEnv::new();
    let dir = env.create_dir("scan-refused");
    let config = dir.join("trop.yaml");
    fs::write(&config, "project: 0x10\n").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("Failed to bind TCP fixture");
    let port = listener.local_addr().unwrap().port();

    let output = scan_port(&env, &dir, port, &["--autoexclude"]);
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    let message = stderr(&output);
    assert!(message.contains("Failed to rewrite config"), "{message}");
    assert!(!message.contains("Added"), "{message}");
    assert_eq!(read(&config), "project: 0x10\n");
    drop(listener);
}
