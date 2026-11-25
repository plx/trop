//! Regression tests for `migrate` and `release` across the path hierarchy.
//!
//! - A migration whose destination lies inside its source tree keeps every
//!   reservation (it used to delete a row it had just moved).
//! - A destination that is itself being moved away is not a conflict, so
//!   `migrate --from P --to P` and re-rooting a subtree onto its parent
//!   succeed without `--force`.
//! - The unrelated-path guard covers every reservation a recursive release
//!   deletes and every source and destination of a migration.

mod common;

use common::TestEnv;
use rusqlite::Connection;
use std::path::Path;

/// The port reserved at `path` (untagged), read straight from the database.
fn port_at(env: &TestEnv, path: &Path) -> Option<u16> {
    let connection = Connection::open(env.data_dir.join("trop.db")).unwrap();
    connection
        .query_row(
            "SELECT port FROM reservations WHERE path = ?1 AND tag = ''",
            [path.to_str().unwrap()],
            |row| row.get(0),
        )
        .ok()
}

fn run_in(env: &TestEnv, cwd: &Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
    env.command()
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to run trop")
}

#[test]
fn migrate_into_own_subtree_with_force_keeps_every_reservation() {
    let env = TestEnv::new();
    let a = env.create_dir("w/a");
    let b = env.create_dir("w/a/b");
    let port_a = env.reserve_simple(&a);
    let port_b = env.reserve_simple(&b);

    let output = run_in(
        &env,
        &env.temp_path,
        &[
            "migrate".as_ref(),
            "--from".as_ref(),
            a.as_os_str(),
            "--to".as_ref(),
            b.as_os_str(),
            "--recursive".as_ref(),
            "--force".as_ref(),
        ],
    );
    assert!(output.status.success(), "{output:?}");

    assert_eq!(port_at(&env, &a), None);
    assert_eq!(
        port_at(&env, &b),
        Some(port_a),
        "the moved /w/a row was lost"
    );
    assert_eq!(port_at(&env, &b.join("b")), Some(port_b));
    assert_eq!(env.reservation_count(), 2);
}

#[test]
fn migrate_onto_itself_succeeds_without_force() {
    let env = TestEnv::new();
    let project = env.create_dir("w/project");
    let port = env.reserve_simple(&project);

    let output = run_in(
        &env,
        &env.temp_path,
        &[
            "migrate".as_ref(),
            "--from".as_ref(),
            project.as_os_str(),
            "--to".as_ref(),
            project.as_os_str(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &project), Some(port));
    assert_eq!(env.reservation_count(), 1);
}

#[test]
fn migrate_subtree_onto_parent_succeeds_without_force() {
    let env = TestEnv::new();
    let a = env.create_dir("w/a");
    let ab = env.create_dir("w/a/b");
    let abb = env.create_dir("w/a/b/b");
    let port_ab = env.reserve_simple(&ab);
    let port_abb = env.reserve_simple(&abb);

    let output = run_in(
        &env,
        &env.temp_path,
        &[
            "migrate".as_ref(),
            "--from".as_ref(),
            ab.as_os_str(),
            "--to".as_ref(),
            a.as_os_str(),
            "--recursive".as_ref(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &a), Some(port_ab));
    assert_eq!(port_at(&env, &ab), Some(port_abb));
    assert_eq!(port_at(&env, &abb), None);
}

#[test]
fn recursive_release_of_an_ancestor_refuses_a_sideways_reservation() {
    let env = TestEnv::new();
    let root = env.create_dir("w");
    let cwd = env.create_dir("w/a");
    let sideways = env.create_dir("w/b");
    let sideways_port = env.reserve_simple(&sideways);

    let release = |extra: &[&str]| {
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "release".as_ref(),
            "--path".as_ref(),
            root.as_os_str(),
            "--recursive".as_ref(),
        ];
        args.extend(extra.iter().map(|arg| -> &std::ffi::OsStr { arg.as_ref() }));
        run_in(&env, &cwd, &args)
    };

    let output = release(&[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("path relationship violation"),
        "{output:?}"
    );
    assert_eq!(port_at(&env, &sideways), Some(sideways_port));

    let output = release(&["--allow-unrelated-path"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &sideways), None);

    let port = env.reserve_simple(&sideways);
    assert_eq!(port_at(&env, &sideways), Some(port));
    let output = release(&["--force"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &sideways), None);
}

#[test]
fn recursive_release_below_the_working_directory_is_permitted() {
    let env = TestEnv::new();
    let root = env.create_dir("w");
    let child = env.create_dir("w/a/child");
    env.reserve_simple(&child);

    let output = run_in(
        &env,
        &root,
        &[
            "release".as_ref(),
            "--path".as_ref(),
            root.as_os_str(),
            "--recursive".as_ref(),
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &child), None);
}

#[test]
fn migrate_guards_sources_and_destinations() {
    let env = TestEnv::new();
    let cwd = env.create_dir("w/a");
    let sideways = env.create_dir("w/b");
    let below = cwd.join("moved");
    let port = env.reserve_simple(&sideways);

    // The source is sideways of the working directory.
    let migrate = |from: &Path, to: &Path, extra: &[&str]| {
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "migrate".as_ref(),
            "--from".as_ref(),
            from.as_os_str(),
            "--to".as_ref(),
            to.as_os_str(),
        ];
        args.extend(extra.iter().map(|arg| -> &std::ffi::OsStr { arg.as_ref() }));
        run_in(&env, &cwd, &args)
    };
    let output = migrate(&sideways, &below, &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(port_at(&env, &sideways), Some(port));

    let output = migrate(&sideways, &below, &["--allow-unrelated-path"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &below), Some(port));

    // The destination is sideways of the working directory.
    let output = migrate(&below, &sideways, &[]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(port_at(&env, &below), Some(port));

    let output = migrate(&below, &sideways, &["--force"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(port_at(&env, &sideways), Some(port));
    assert_eq!(port_at(&env, &below), None);
}
