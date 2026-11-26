//! CLI regression tests for the shared identifier rule on reservation tags.
//!
//! `--tag` values follow the same rule as project and task identifiers: after
//! trimming Unicode whitespace they must be non-empty, contain no NUL, and be
//! at most 255 UTF-8 bytes. Violations are invalid arguments (exit code 4)
//! and persist nothing. Rows stored before the rule applied to tags stay
//! readable and addressable.

mod common;

use common::TestEnv;

/// Regression: `trop reserve --tag <300 chars>` used to succeed and store a
/// tag longer than the 255-byte limit every other identifier obeys.
#[test]
fn reserve_rejects_overlong_tag_as_invalid_argument() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let overlong = "a".repeat(300);

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&project)
        .arg("--tag")
        .arg(&overlong)
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("tag"), "{stderr}");
    assert!(stderr.contains("255 bytes"), "{stderr}");
    assert!(
        !stderr.contains(&overlong),
        "diagnostic must not echo the tag"
    );
    assert_eq!(env.reservation_count(), 0);
}

/// The limit counts UTF-8 bytes of the trimmed tag: 128 two-byte characters
/// (256 bytes) are rejected even though they are only 128 characters.
#[test]
fn reserve_rejects_tag_over_255_utf8_bytes() {
    let env = TestEnv::new();
    let project = env.create_dir("project");

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&project)
        .arg("--tag")
        .arg("é".repeat(128))
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(env.reservation_count(), 0);
}

/// Whitespace around a maximal tag is trimmed before the limit applies, and
/// the trimmed tag is what gets stored.
#[test]
fn reserve_accepts_255_byte_tag_after_trimming() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let maximal = "t".repeat(255);

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&project)
        .arg("--tag")
        .arg(format!(" \u{3000}{maximal}\t "))
        .arg("--port")
        .arg("47213")
        .arg("--ignore-occupied")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    let stored: String = connection
        .query_row("SELECT tag FROM reservations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, maximal);
}

/// A whitespace-only tag is still rejected as an invalid argument.
#[test]
fn reserve_rejects_blank_tag() {
    let env = TestEnv::new();
    let project = env.create_dir("project");

    let output = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&project)
        .arg("--tag")
        .arg(" \u{3000} ")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(env.reservation_count(), 0);
}

/// Reservations whose tags an earlier trop stored without a length limit stay
/// usable: listing, unrelated reservations, migration, and release by tag all
/// still work. Only NEW reservations must satisfy the identifier rule.
#[test]
fn legacy_overlong_tag_rows_stay_usable() {
    let env = TestEnv::new();
    let project = env.create_dir("project");
    let destination = env.temp_path.join("moved");
    let other = env.create_dir("other");
    let legacy_tag = "t".repeat(300);

    // Create the database with an untagged reservation, then add the legacy
    // row at the same stored path, as an earlier trop could have written it.
    let output = env
        .command()
        .current_dir(&env.temp_path)
        .args(["reserve", "--port", "47231", "--ignore-occupied"])
        .arg("--allow-unrelated-path")
        .arg("--path")
        .arg(&project)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    let stored_path: String = connection
        .query_row("SELECT path FROM reservations", [], |row| row.get(0))
        .unwrap();
    connection
        .execute(
            "INSERT INTO reservations (path, tag, port, project, task, created_at, last_used_at)
             VALUES (?1, ?2, 47232, NULL, NULL, 1, 2)",
            rusqlite::params![stored_path, legacy_tag],
        )
        .unwrap();
    drop(connection);

    let output = env
        .command()
        .current_dir(&env.temp_path)
        .arg("list")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains(&legacy_tag));

    // Unrelated reservations are unaffected by the legacy row.
    let output = env
        .command()
        .current_dir(&env.temp_path)
        .args(["reserve", "--port", "47233", "--ignore-occupied"])
        .arg("--allow-unrelated-path")
        .arg("--path")
        .arg(&other)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    // A new reservation with the legacy tag text is still rejected.
    let output = env
        .command()
        .current_dir(&env.temp_path)
        .args(["reserve", "--ignore-occupied", "--allow-unrelated-path"])
        .arg("--path")
        .arg(&project)
        .arg("--tag")
        .arg(&legacy_tag)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4), "{output:?}");

    // Migration rebuilds the legacy key without re-validating it.
    let output = env
        .command()
        .current_dir(&env.temp_path)
        .arg("migrate")
        .arg("--from")
        .arg(&project)
        .arg("--to")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(env.reservation_count(), 3);

    // The legacy reservation can still be released by its tag.
    let output = env
        .command()
        .current_dir(&env.temp_path)
        .args(["release", "--allow-unrelated-path"])
        .arg("--path")
        .arg(&destination)
        .arg("--tag")
        .arg(&legacy_tag)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(env.reservation_count(), 2);
    let connection = rusqlite::Connection::open(env.data_dir.join("trop.db")).unwrap();
    let remaining_tags: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM reservations WHERE tag <> ''",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_tags, 0);
}
