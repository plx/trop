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

/// Reservation tags follow the same rule as project and task identifiers.
#[test]
fn reservation_key_tags_follow_the_identifier_rule() {
    let path = PathBuf::from("/project");
    assert!(ReservationKey::new(path.clone(), Some("a".repeat(256))).is_err());
    assert!(ReservationKey::new(path.clone(), Some("web\0".to_string())).is_err());
    assert!(ReservationKey::new(path.clone(), Some(" \u{2003} ".to_string())).is_err());
    let key = ReservationKey::new(path, Some(format!(" {} ", "a".repeat(255)))).unwrap();
    assert_eq!(key.tag.map(|tag| tag.len()), Some(255));

    let error: Error = ReservationKey::new(PathBuf::from("/project"), Some("a".repeat(256)))
        .unwrap_err()
        .into();
    assert!(matches!(error, Error::Validation { ref field, .. } if field == "tag"));
}

/// Rows written before tags were limited to 255 bytes must stay readable:
/// stored keys are rebuilt with the lenient `ReservationKey::from_stored`
/// (canonical form only), never with the write-side `ReservationKey::new`.
///
/// Ignored until database row decoding (`trop/src/database/operations.rs`,
/// owned by the storage work package) switches its `ReservationKey::new`
/// call to `ReservationKey::from_stored`.
#[test]
#[ignore = "pending: database row decoding must call ReservationKey::from_stored"]
fn legacy_rows_with_overlong_tags_remain_readable() {
    use rusqlite::{params, Connection};
    use trop::database::{Database, DatabaseConfig};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(Database::open(DatabaseConfig::new(&path)).unwrap());
    let conn = Connection::open(&path).unwrap();
    let legacy_tag = "t".repeat(300);
    conn.execute(
        "INSERT INTO reservations (path, tag, port, project, task, created_at, last_used_at)
         VALUES (?1, ?2, ?3, NULL, NULL, 1, 2)",
        params!["/project", legacy_tag, 5000_i64],
    )
    .unwrap();

    let reservations = Database::list_all_reservations(&conn).unwrap();
    assert_eq!(reservations.len(), 1);
    assert_eq!(
        reservations[0].key().tag.as_deref(),
        Some(legacy_tag.as_str())
    );
}
