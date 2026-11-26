//! Regression tests for the shared identifier rules in configuration
//! validation and reservation keys.

use std::collections::HashMap;
use std::path::PathBuf;

use trop::config::schema::{ReservationGroup, ServiceDefinition};
use trop::config::{Config, ConfigValidator};
use trop::{Error, ReservationKey};

fn group_config(tags: &[(&str, Option<&str>)]) -> Config {
    let services = tags
        .iter()
        .enumerate()
        .map(|(offset, (tag, env))| {
            (
                (*tag).to_string(),
                ServiceDefinition {
                    offset: Some(u16::try_from(offset).unwrap()),
                    preferred: None,
                    env: env.map(str::to_string),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    Config {
        project: Some("project".to_string()),
        reservations: Some(ReservationGroup {
            base: Some(5000),
            services,
        }),
        ..Config::default()
    }
}

fn validation_message(result: trop::Result<()>) -> (String, String) {
    match result {
        Err(Error::Validation { field, message }) => (field, message),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// Regression: the implicit environment-variable name used to be derived
/// from the untrimmed tag, so ` web ` was rejected as non-convertible even
/// though allocation and output use the trimmed tag `web` (which derives
/// `WEB`).
#[test]
fn implicit_env_name_is_derived_from_the_trimmed_tag() {
    for tag in [" web ", "\tweb", "web\u{3000}", "\u{a0}api-v2\u{2009}"] {
        let config = group_config(&[(tag, None)]);
        assert!(
            ConfigValidator::validate(&config, true).is_ok(),
            "{tag:?} should derive a name from its trimmed form"
        );
    }
}

/// Trimmed tags still collide with their untrimmed spellings, both as tags
/// and as derived environment-variable names.
#[test]
fn trimmed_tag_derivations_still_collide() {
    let config = group_config(&[(" web ", None), ("WEB", None)]);
    let (field, message) = validation_message(ConfigValidator::validate(&config, true));
    assert_eq!(field, "reservations.services.env");
    assert!(message.contains("unique"), "{message}");
}

/// Interior whitespace is not trimmed and still cannot be derived.
#[test]
fn interior_whitespace_still_blocks_derivation() {
    let config = group_config(&[(" web port ", None)]);
    let (field, _) = validation_message(ConfigValidator::validate(&config, true));
    assert_eq!(field, "reservations.services.tag");

    // An explicit mapping makes such a tag usable.
    let config = group_config(&[(" web port ", Some("WEB_PORT"))]);
    assert!(ConfigValidator::validate(&config, true).is_ok());
}

/// The identifier length limit counts UTF-8 bytes, and the message says so.
#[test]
fn identifier_length_limit_is_reported_in_bytes() {
    // 128 two-byte characters: 128 characters but 256 bytes.
    let config = Config {
        project: Some("é".repeat(128)),
        ..Config::default()
    };
    let (field, message) = validation_message(ConfigValidator::validate(&config, true));
    assert_eq!(field, "project");
    assert_eq!(message, "Cannot exceed 255 bytes");

    let config = Config {
        project: Some(format!(" {} ", "é".repeat(127))),
        ..Config::default()
    };
    assert!(ConfigValidator::validate(&config, true).is_ok());
}

/// Identifier errors keep their precedence: blank, then NUL, then length.
#[test]
fn identifier_error_precedence() {
    let cases = [
        (
            " \u{3000} ".to_string(),
            "Cannot be empty or only whitespace",
        ),
        (
            format!("\0{}", "a".repeat(300)),
            "Cannot contain null bytes",
        ),
        ("a".repeat(256), "Cannot exceed 255 bytes"),
    ];
    for (project, expected) in cases {
        let config = Config {
            project: Some(project),
            ..Config::default()
        };
        let (_, message) = validation_message(ConfigValidator::validate(&config, true));
        assert_eq!(message, expected);
    }
}

/// Tags of new reservations follow the same rule as project and task
/// identifiers, and report it with the same messages.
#[test]
fn validated_reservation_key_tags_follow_the_identifier_rule() {
    let path = PathBuf::from("/project");
    let cases = [
        ("a".repeat(256), "Cannot exceed 255 bytes"),
        ("é".repeat(128), "Cannot exceed 255 bytes"),
        ("web\0".to_string(), "Cannot contain null bytes"),
        (
            format!("\0{}", "a".repeat(300)),
            "Cannot contain null bytes",
        ),
    ];
    for (tag, expected) in cases {
        let error = ReservationKey::new_validated(path.clone(), Some(tag.clone())).unwrap_err();
        assert_eq!(error.field, "tag");
        assert_eq!(error.message, expected);
        // A project with the same text fails for the same reason.
        let config = Config {
            project: Some(tag),
            ..Config::default()
        };
        let (_, message) = validation_message(ConfigValidator::validate(&config, true));
        assert_eq!(message, expected);
    }
    let error =
        ReservationKey::new_validated(path.clone(), Some(" \u{2003} ".to_string())).unwrap_err();
    assert_eq!(
        error.message,
        "tag must be non-empty after trimming whitespace"
    );

    let key = ReservationKey::new_validated(path, Some(format!(" {} ", "a".repeat(255)))).unwrap();
    assert_eq!(key.tag.map(|tag| tag.len()), Some(255));

    let error: Error =
        ReservationKey::new_validated(PathBuf::from("/project"), Some("a".repeat(256)))
            .unwrap_err()
            .into();
    assert!(matches!(error, Error::Validation { ref field, .. } if field == "tag"));
}

/// Inserts a row with `tag` directly, as an earlier trop (which did not limit
/// tag length) could have written it.
fn insert_legacy_row(conn: &rusqlite::Connection, path: &str, tag: &str, port: i64) {
    conn.execute(
        "INSERT INTO reservations (path, tag, port, project, task, created_at, last_used_at)
         VALUES (?1, ?2, ?3, NULL, NULL, 1, 2)",
        rusqlite::params![path, tag, port],
    )
    .unwrap();
}

/// Rows written before tags of new reservations were limited to 255 bytes
/// must stay readable, addressable, and valid: stored keys are rebuilt with
/// the lenient `ReservationKey::new`, never with `new_validated`.
#[test]
fn legacy_rows_with_overlong_tags_remain_readable() {
    use trop::database::{Database, DatabaseConfig};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(Database::open(DatabaseConfig::new(&path)).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let legacy_tag = "t".repeat(300);
    insert_legacy_row(&conn, "/project", &legacy_tag, 5000);
    insert_legacy_row(&conn, "/project", "nul\0tag", 5001);

    let reservations = Database::list_all_reservations(&conn).unwrap();
    assert_eq!(reservations.len(), 2);
    let mut tags = reservations
        .iter()
        .map(|reservation| reservation.key().tag.clone().unwrap())
        .collect::<Vec<_>>();
    tags.sort();
    assert_eq!(tags, vec!["nul\0tag".to_string(), legacy_tag.clone()]);

    // Lookups by the legacy tag still find the row.
    let key = ReservationKey::new(PathBuf::from("/project"), Some(legacy_tag)).unwrap();
    let found = Database::get_reservation(&conn, &key).unwrap().unwrap();
    assert_eq!(found.port().value(), 5000);
    drop(conn);

    // Whole-database validation and reopening still accept the rows.
    Database::validate(&DatabaseConfig::new(&path)).unwrap();
    let db = Database::open(DatabaseConfig::new(&path)).unwrap();
    db.verify_integrity().unwrap();
}

/// A published-v1 database holding a legacy overlong tag still migrates to
/// the current schema: migration validates every row through the same
/// lenient stored-row decoding.
#[test]
fn v1_database_with_overlong_tag_migrates() {
    use trop::database::{get_schema_version, Database, DatabaseConfig};

    const PUBLISHED_V1_SCHEMA: &str = include_str!("fixtures/published-v1-schema.sql");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("published-v1.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(PUBLISHED_V1_SCHEMA).unwrap();
    let legacy_tag = "v".repeat(300);
    insert_legacy_row(&conn, "/project", &legacy_tag, 5000);
    drop(conn);

    let db = Database::open(DatabaseConfig::new(&path)).unwrap();
    assert_eq!(get_schema_version(db.connection()).unwrap(), 2);
    let reservations = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(reservations.len(), 1);
    assert_eq!(
        reservations[0].key().tag.as_deref(),
        Some(legacy_tag.as_str())
    );
}
