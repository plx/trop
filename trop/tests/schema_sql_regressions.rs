//! Regression tests for the verified schema-SQL tokenizer
//! (`verified::sql_schema`) as `Database::validate` uses it.

use rusqlite::Connection;
use tempfile::tempdir;
use trop::database::{Database, DatabaseConfig};
use trop::Error;

/// The normalized token streams of the three required constraints, as the
/// old substring search compared them.
const ENCODED_CONSTRAINTS: &str = "\
    w10:constraint;w10:valid_port;w5:check;s1:(;w4:port;w7:between;w1:1;w3:and;w5:65535;s1:);\
    w10:constraint;w16:valid_created_at;w5:check;s1:(;w10:created_at;s2:>=;w1:0;s1:);\
    w10:constraint;w18:valid_last_used_at;w5:check;s1:(;w12:last_used_at;s2:>=;w1:0;s1:);";

fn corruption_details(error: Error) -> String {
    match error {
        Error::DatabaseCorruption { details } => details,
        other => panic!("expected typed database corruption, got {other:?}"),
    }
}

/// A schema-v2 database whose reservations table has no CHECK constraints
/// but carries `extra` (a comment or a column default) inside its
/// definition.
fn database_without_constraints(extra_column_clause: &str, trailing: &str) -> String {
    format!(
        "CREATE TABLE metadata (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
         ) STRICT;
         INSERT INTO metadata VALUES ('schema_version', '2');
         CREATE TABLE reservations (
            path TEXT NOT NULL,
            tag TEXT NOT NULL,
            port INTEGER NOT NULL UNIQUE,
            project TEXT {extra_column_clause},
            task TEXT,
            created_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            {trailing}
            PRIMARY KEY (path, tag)
         ) STRICT;
         CREATE INDEX idx_reservations_port ON reservations(port);
         CREATE INDEX idx_reservations_project ON reservations(project);
         CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);"
    )
}

fn validation_details(schema: &str) -> String {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(schema).unwrap();
    drop(conn);
    corruption_details(Database::validate(&DatabaseConfig::new(&path)).unwrap_err())
}

#[test]
fn encoded_constraint_text_in_a_comment_does_not_satisfy_a_required_constraint() {
    // Before the token-aligned check, the comment's raw text contained the
    // encoded constraint tokens, so the substring search reported all three
    // constraints present and validation failed later, for the wrong reason.
    let details = validation_details(&database_without_constraints(
        "",
        &format!("/* {ENCODED_CONSTRAINTS} */"),
    ));
    assert!(
        details.contains("missing required constraint valid_port"),
        "{details}"
    );
}

#[test]
fn encoded_constraint_text_in_a_quoted_literal_does_not_satisfy_a_required_constraint() {
    let details = validation_details(&database_without_constraints(
        &format!("DEFAULT '{ENCODED_CONSTRAINTS}'"),
        "",
    ));
    assert!(
        details.contains("missing required constraint valid_port"),
        "{details}"
    );
}

#[test]
fn freshly_created_database_validates() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    drop(Database::open(DatabaseConfig::new(&path)).unwrap());
    Database::validate(&DatabaseConfig::new(&path)).unwrap();
}

#[test]
fn whitespace_and_keyword_case_do_not_affect_the_exact_definition_check() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trop.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "create table \"metadata\" (key text primary key not null, value text not null) strict;
         insert into metadata values ('schema_version', '2');
         CREATE   TABLE reservations(path TEXT NOT NULL,tag TEXT NOT NULL,
            port INTEGER NOT NULL UNIQUE CONSTRAINT valid_port CHECK(port BETWEEN 1 AND 65535),
            project TEXT,task TEXT,
            created_at INTEGER NOT NULL CONSTRAINT valid_created_at CHECK(created_at>=0),
            last_used_at INTEGER NOT NULL CONSTRAINT valid_last_used_at CHECK(last_used_at>=0),
            PRIMARY KEY(path,tag)) STRICT;
         CREATE INDEX idx_reservations_port ON reservations(port);
         CREATE INDEX idx_reservations_project ON reservations(project);
         CREATE INDEX idx_reservations_last_used ON reservations(last_used_at);",
    )
    .unwrap();
    drop(conn);
    Database::validate(&DatabaseConfig::new(&path)).unwrap();
}
