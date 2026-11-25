//! Read-only physical, schema, and logical database validation.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::verified::schema_version::{
    classify_for_validation, ColumnFacts, Corruption, MetadataState, ValidateDisposition,
};
use crate::verified::sql_schema::{normalize_schema_sql, token_run_contains, tokenize_schema_sql};
use crate::verified::storage::{Cell, SqlType};

use super::migrations::observe_metadata;
use super::operations::row_to_reservation;
use super::schema::{CREATE_METADATA_TABLE, CREATE_RESERVATIONS_TABLE, CURRENT_SCHEMA_VERSION};

const EXPECTED_METADATA_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("key", "TEXT", true, 1),
    ColumnSpec::new("value", "TEXT", true, 0),
];

const EXPECTED_RESERVATION_COLUMNS: &[ColumnSpec] = &[
    ColumnSpec::new("path", "TEXT", true, 1),
    ColumnSpec::new("tag", "TEXT", true, 2),
    ColumnSpec::new("port", "INTEGER", true, 0),
    ColumnSpec::new("project", "TEXT", false, 0),
    ColumnSpec::new("task", "TEXT", false, 0),
    ColumnSpec::new("created_at", "INTEGER", true, 0),
    ColumnSpec::new("last_used_at", "INTEGER", true, 0),
];

const REQUIRED_NAMED_INDEXES: &[(&str, &str)] = &[
    ("idx_reservations_port", "port"),
    ("idx_reservations_project", "project"),
    ("idx_reservations_last_used", "last_used_at"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnSpec {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
}

impl ColumnSpec {
    const fn new(
        name: &'static str,
        declared_type: &'static str,
        not_null: bool,
        primary_key_position: i64,
    ) -> Self {
        Self {
            name,
            declared_type,
            not_null,
            primary_key_position,
        }
    }

    /// The expected column as the verified layout check sees it.
    const fn facts(&self) -> ColumnFacts<'static> {
        ColumnFacts {
            name: self.name,
            declared_type: self.declared_type,
            not_null: self.not_null,
            primary_key_position: self.primary_key_position,
            hidden: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualColumn {
    name: String,
    declared_type: String,
    not_null: bool,
    primary_key_position: i64,
    hidden: i64,
}

impl ActualColumn {
    /// The column as the verified layout check sees it.
    fn facts(&self) -> ColumnFacts<'_> {
        ColumnFacts {
            name: &self.name,
            declared_type: &self.declared_type,
            not_null: self.not_null,
            primary_key_position: self.primary_key_position,
            hidden: self.hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexTerm {
    name: Option<String>,
    descending: bool,
    collation: String,
}

/// Validates the current database without changing connection or persistent
/// state.
pub(super) fn validate_current_database(conn: &Connection) -> Result<()> {
    match validate_current_database_inner(conn) {
        Err(Error::Database(error)) if is_physical_corruption(&error) => {
            Err(Error::DatabaseCorruption {
                details: format!(
                    "SQLite reported physical database corruption: {error}. \
                     Make a copy before recovery; restore a known-good database \
                     or delete the disposable database and recreate its \
                     reservations. trop did not modify or repair the stored data"
                ),
            })
        }
        result => result,
    }
}

fn validate_current_database_inner(conn: &Connection) -> Result<()> {
    let metadata = table_columns(conn, "metadata")?;
    if !metadata.iter().any(|column| column.name == "key")
        || !metadata.iter().any(|column| column.name == "value")
    {
        return Err(Error::DatabaseCorruption {
            details: "schema metadata cannot be read because its key/value columns are missing; \
                      restore a known-good database or recreate disposable reservations. \
                      trop did not modify or repair the stored data"
                .into(),
        });
    }

    // Compatibility is decided before enforcing the current schema layout so
    // a valid legacy or future database receives its specific typed error.
    validate_schema_version(conn)?;

    let reservations = table_columns(conn, "reservations")?;
    if !column_shapes_match(&metadata, EXPECTED_METADATA_COLUMNS)
        || !column_shapes_match(&reservations, EXPECTED_RESERVATION_COLUMNS)
        || !table_is_strict(conn, "metadata")?
        || !table_is_strict(conn, "reservations")?
    {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 table columns or STRICT status do not match the required layout; \
                      restore a known-good database or recreate disposable reservations. \
                      trop did not modify or repair the stored data"
                .into(),
        });
    }

    validate_metadata_rows(conn)?;
    validate_reservation_rows(conn)?;
    validate_logical_uniqueness(conn)?;

    if !columns_match(&metadata, EXPECTED_METADATA_COLUMNS)
        || !columns_match(&reservations, EXPECTED_RESERVATION_COLUMNS)
    {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 primary-key structure does not match the required layout; \
                      restore a known-good database or recreate disposable reservations. \
                      trop did not modify or repair the stored data"
                .into(),
        });
    }

    validate_required_constraints(conn)?;
    validate_exact_table_definitions(conn)?;
    validate_indexes(conn)?;
    validate_foreign_keys(conn)?;
    validate_physical_integrity(conn)?;
    Ok(())
}

/// Checks the stored schema version (verified kernel
/// `classify_for_validation`, which agrees with opening: a database opens as
/// current exactly when it validates).
fn validate_schema_version(conn: &Connection) -> Result<()> {
    let observed = observe_metadata(conn)?;
    schema_version_verdict(observed.state())
}

/// Validation's verdict on observed schema metadata (verified kernel
/// `classify_for_validation`), as an error for anything but a current,
/// well-formed version.
fn schema_version_verdict(state: MetadataState<'_>) -> Result<()> {
    match classify_for_validation(state) {
        ValidateDisposition::Valid => Ok(()),
        ValidateDisposition::Future(version) => Err(Error::UnsupportedSchemaVersion {
            expected: u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or_default(),
            found: u32::try_from(version).unwrap_or(u32::MAX),
        }),
        ValidateDisposition::MigrationRequired(version) => Err(Error::MigrationRequired {
            found: version,
            target: CURRENT_SCHEMA_VERSION,
            action: "validation is read-only; reopen the database writable to apply the ordered \
                     migration, or validate a writable migrated copy"
                .into(),
        }),
        ValidateDisposition::Corrupt(corruption) => {
            let kind;
            let reason = match corruption {
                Corruption::MissingMetadataTable => "the metadata table is missing",
                Corruption::MissingVersionRow | Corruption::MultipleVersionRows => {
                    "exactly one schema_version row is required"
                }
                Corruption::NotText(sql_type) => {
                    kind = format!("expected TEXT, found {}", sql_type_name(sql_type));
                    &kind
                }
                Corruption::NotUtf8 => "stored TEXT is not valid UTF-8",
                Corruption::NotAnInteger => "schema_version must be a base-10 signed integer",
            };
            Err(Error::corrupt_stored_value(
                "metadata",
                "value",
                "key=\"schema_version\"",
                reason,
            ))
        }
    }
}

/// The name `typeof` reports for a storage class, in upper case.
const fn sql_type_name(sql_type: SqlType) -> &'static str {
    match sql_type {
        SqlType::Null => "NULL",
        SqlType::Integer => "INTEGER",
        SqlType::Real => "REAL",
        SqlType::Text => "TEXT",
        SqlType::Blob => "BLOB",
    }
}

fn validate_metadata_rows(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare("SELECT key, value FROM metadata ORDER BY key")?;
    let mut rows = statement.query([])?;
    let mut schema_versions = Vec::new();

    while let Some(row) = rows.next()? {
        let key_ref = row.get_ref(0)?;
        let key_context = metadata_key_context(key_ref);
        let key = text_value(key_ref, "metadata", "key", &key_context)?;
        let value_ref = row.get_ref(1)?;
        let value = text_value(value_ref, "metadata", "value", &key_context)?;
        if key == "schema_version" {
            schema_versions.push(value.to_owned());
        }
    }

    // This scan is a separate read from `validate_schema_version`'s, so the
    // `schema_version` rows it saw are classified again by the same verified
    // kernel instead of being re-checked by hand.
    let state = match schema_versions.as_slice() {
        [] => MetadataState::NoVersionRow,
        [first, rest @ ..] => MetadataState::VersionRow {
            value: Cell::Text(first),
            more_rows: !rest.is_empty(),
        },
    };
    schema_version_verdict(state)
}

fn validate_reservation_rows(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare(
        "SELECT path, tag, port, project, task, created_at, last_used_at
         FROM reservations
         ORDER BY path, tag",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        row_to_reservation(row)?;
    }
    Ok(())
}

fn validate_logical_uniqueness(conn: &Connection) -> Result<()> {
    let duplicate_keys = count_query(
        conn,
        "SELECT COUNT(*) FROM (
            SELECT path, tag FROM reservations
            GROUP BY path, tag HAVING COUNT(*) > 1
         )",
    )?;
    if duplicate_keys > 0 {
        return Err(Error::corrupt_stored_value(
            "reservations",
            "primary_key",
            "<multiple reservation keys>",
            "duplicate logical path/tag identities are stored",
        ));
    }

    let duplicate_ports = count_query(
        conn,
        "SELECT COUNT(*) FROM (
            SELECT port FROM reservations
            GROUP BY port HAVING COUNT(*) > 1
         )",
    )?;
    if duplicate_ports > 0 {
        return Err(Error::corrupt_stored_value(
            "reservations",
            "port",
            "<multiple reservation keys>",
            "the same port is stored for more than one reservation",
        ));
    }

    Ok(())
}

fn validate_required_constraints(conn: &Connection) -> Result<()> {
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'table' AND name = 'reservations'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::DatabaseCorruption {
            details: "schema v2 reservations table is missing".into(),
        })?;
    // Verified kernel `token_run_contains`: the constraint's tokens must
    // occur as a contiguous run of the table's tokens. Text inside a comment
    // or a quoted literal is part of one token, so it never matches.
    let tokens = tokenize_schema_sql(sql.as_bytes(), None);

    for (name, expression) in [
        ("valid_port", "CHECK (port BETWEEN 1 AND 65535)"),
        ("valid_created_at", "CHECK (created_at >= 0)"),
        ("valid_last_used_at", "CHECK (last_used_at >= 0)"),
    ] {
        let required =
            tokenize_schema_sql(format!("CONSTRAINT {name} {expression}").as_bytes(), None);
        if !token_run_contains(&tokens, &required) {
            return Err(Error::DatabaseCorruption {
                details: format!(
                    "schema v2 reservations table is missing required constraint {name}; \
                     restore a known-good database or recreate disposable reservations. \
                     trop did not modify or repair the stored data"
                ),
            });
        }
    }

    Ok(())
}

fn validate_exact_table_definitions(conn: &Connection) -> Result<()> {
    for (table, create_sql) in [
        ("metadata", CREATE_METADATA_TABLE),
        ("reservations", CREATE_RESERVATIONS_TABLE),
    ] {
        let actual: String = conn.query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )?;
        let expected_sql = create_sql.replacen("IF NOT EXISTS", "", 1);
        // Verified kernel `normalize_schema_sql`: its encoding is injective,
        // so the normalized texts are equal exactly when the token streams
        // are (whitespace between tokens and the case of bare words aside).
        let allowed = Some(table.as_bytes());
        let expected = normalize_schema_sql(expected_sql.as_bytes(), allowed);
        if normalize_schema_sql(actual.as_bytes(), allowed) != expected {
            return Err(Error::DatabaseCorruption {
                details: format!(
                    "schema v2 table {table} does not exactly match the required definition; \
                     restore a known-good database or recreate disposable reservations. \
                     trop did not modify or repair the stored data"
                ),
            });
        }
    }
    Ok(())
}

fn validate_indexes(conn: &Connection) -> Result<()> {
    for (index, column) in REQUIRED_NAMED_INDEXES {
        let descriptor = conn
            .query_row(
                "SELECT \"unique\", origin, partial
                 FROM pragma_index_list('reservations')
                 WHERE name = ?1",
                [index],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        if descriptor.as_ref() != Some(&(0, "c".to_string(), 0))
            || index_key_terms(conn, index)?
                != [IndexTerm {
                    name: Some((*column).to_string()),
                    descending: false,
                    collation: "BINARY".to_string(),
                }]
        {
            return Err(Error::DatabaseCorruption {
                details: format!(
                    "schema v2 required index {index} must be a non-unique, non-partial index \
                     over reservations.{column}; restore a known-good database or recreate \
                     disposable reservations. trop did not modify or repair the stored data"
                ),
            });
        }
    }

    if !has_unique_index(conn, &["path", "tag"], "pk")? || !has_unique_index(conn, &["port"], "u")?
    {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 is missing its path/tag primary-key or global port uniqueness \
                      structure; restore a known-good database or recreate disposable \
                      reservations. trop did not modify or repair the stored data"
                .into(),
        });
    }

    Ok(())
}

fn has_unique_index(conn: &Connection, columns: &[&str], origin: &str) -> Result<bool> {
    let mut statement = conn.prepare(
        "SELECT name FROM pragma_index_list('reservations')
         WHERE \"unique\" = 1 AND origin = ?1 AND partial = 0",
    )?;
    let names = statement
        .query_map([origin], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for name in names {
        let expected = columns
            .iter()
            .map(|column| IndexTerm {
                name: Some((*column).to_string()),
                descending: false,
                collation: "BINARY".to_string(),
            })
            .collect::<Vec<_>>();
        if index_key_terms(conn, &name)? == expected {
            return Ok(true);
        }
    }
    Ok(false)
}

fn index_key_terms(conn: &Connection, index: &str) -> Result<Vec<IndexTerm>> {
    let mut statement = conn.prepare(
        "SELECT name, \"desc\", coll
         FROM pragma_index_xinfo(?1)
         WHERE key = 1
         ORDER BY seqno",
    )?;
    let terms = statement
        .query_map([index], |row| {
            Ok(IndexTerm {
                name: row.get(0)?,
                descending: row.get::<_, i64>(1)? != 0,
                collation: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(terms)
}

fn validate_foreign_keys(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare("PRAGMA foreign_key_check")?;
    if statement.query([])?.next()?.is_some() {
        return Err(Error::DatabaseCorruption {
            details: "schema v2 foreign-key check failed; restore a known-good database or \
                      recreate disposable reservations. trop did not modify or repair the \
                      stored data"
                .into(),
        });
    }
    Ok(())
}

fn validate_physical_integrity(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare("PRAGMA integrity_check")?;
    let messages = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if messages.as_slice() != ["ok"] {
        return Err(Error::DatabaseCorruption {
            details: format!(
                "SQLite physical integrity check failed: {}. Make a copy before recovery; \
                 restore a known-good database or delete the disposable database and recreate \
                 its reservations. trop did not modify or repair the stored data",
                messages.join("; ")
            ),
        });
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<ActualColumn>> {
    let mut statement = conn.prepare(
        "SELECT name, type, \"notnull\", pk, hidden
         FROM pragma_table_xinfo(?1)
         ORDER BY cid",
    )?;
    let columns = statement
        .query_map([table], |row| {
            Ok(ActualColumn {
                name: row.get(0)?,
                declared_type: row.get(1)?,
                not_null: row.get::<_, i64>(2)? != 0,
                primary_key_position: row.get(3)?,
                hidden: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(columns)
}

/// The column layout check (verified kernel `columns_match`): equal lengths,
/// and position by position the same name, the same declared type ignoring
/// ASCII case, the same `NOT NULL`, no hidden column, and (with `check_pk`)
/// the same primary-key position.
fn layout_matches(actual: &[ActualColumn], expected: &[ColumnSpec], check_pk: bool) -> bool {
    let actual = actual.iter().map(ActualColumn::facts).collect::<Vec<_>>();
    let expected = expected.iter().map(ColumnSpec::facts).collect::<Vec<_>>();
    crate::verified::schema_version::columns_match(&actual, &expected, check_pk, true)
}

fn column_shapes_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    layout_matches(actual, expected, false)
}

fn columns_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    layout_matches(actual, expected, true)
}

fn table_is_strict(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT strict FROM pragma_table_list WHERE schema = 'main' AND name = ?1",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some_and(|strict| strict == 1))
}

fn count_query(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
}

fn text_value<'a>(value: ValueRef<'a>, table: &str, field: &str, key: &str) -> Result<&'a str> {
    let ValueRef::Text(bytes) = value else {
        return Err(Error::corrupt_stored_value(
            table,
            field,
            key,
            &format!("expected TEXT, found {}", value_kind(value)),
        ));
    };
    std::str::from_utf8(bytes).map_err(|_| {
        Error::corrupt_stored_value(table, field, key, "stored TEXT is not valid UTF-8")
    })
}

fn metadata_key_context(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).map_or_else(
            |_| "key=<invalid UTF-8>".to_string(),
            |key| format!("key=\"{}\"", escape_text(key)),
        ),
        other => format!("key=<{}>", value_kind(other)),
    }
}

/// `char::escape_default` of every char (verified kernel `escape_default`:
/// printable ASCII only, no unescaped `"`, injective).
fn escape_text(value: &str) -> String {
    crate::verified::escape::escape_default(value)
}

fn value_kind(value: ValueRef<'_>) -> &'static str {
    match value {
        ValueRef::Null => "NULL",
        ValueRef::Integer(_) => "INTEGER",
        ValueRef::Real(_) => "REAL",
        ValueRef::Text(_) => "TEXT",
        ValueRef::Blob(_) => "BLOB",
    }
}

fn is_physical_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(
                sqlite.code,
                rusqlite::ErrorCode::DatabaseCorrupt
                    | rusqlite::ErrorCode::NotADatabase
            )
    )
}

#[cfg(test)]
mod tests {
    //! Glue tests for the verified schema-SQL kernel: the byte-level
    //! tokenizer is differential-tested against the historical `char`-level
    //! implementation (kept here verbatim as the oracle), and the token-run
    //! containment check is pinned at its boundaries.

    use std::fmt::Write as _;

    use super::{normalize_schema_sql, token_run_contains, tokenize_schema_sql};

    // ---- The historical char-level normalizer (differential oracle) ----

    fn legacy_normalize_schema_sql(sql: &str, allowed_quoted_identifier: Option<&str>) -> String {
        let mut normalized = String::with_capacity(sql.len());
        let mut characters = sql.chars().peekable();

        while let Some(character) = characters.next() {
            if character.is_ascii_whitespace() {
                continue;
            }

            if is_schema_word_character(character) {
                let mut word = String::from(character);
                while characters
                    .peek()
                    .is_some_and(|next| is_schema_word_character(*next))
                {
                    word.push(characters.next().expect("peeked word character"));
                }
                push_schema_token(&mut normalized, 'w', &word.to_ascii_lowercase());
                continue;
            }

            match character {
                '"' => {
                    let (raw, decoded, closed) =
                        read_quoted_schema_token(&mut characters, '"', true);
                    if closed && allowed_quoted_identifier == Some(decoded.as_str()) {
                        push_schema_token(&mut normalized, 'w', &decoded.to_ascii_lowercase());
                    } else {
                        push_schema_token(&mut normalized, 'q', &raw);
                    }
                }
                '\'' | '`' => {
                    let (raw, _, _) = read_quoted_schema_token(&mut characters, character, true);
                    push_schema_token(&mut normalized, character, &raw);
                }
                '[' => {
                    let mut raw = String::from(character);
                    for quoted in characters.by_ref() {
                        raw.push(quoted);
                        if quoted == ']' {
                            break;
                        }
                    }
                    push_schema_token(&mut normalized, '[', &raw);
                }
                '-' if characters.peek() == Some(&'-') => {
                    let mut comment = String::from(character);
                    comment.push(characters.next().expect("peeked comment delimiter"));
                    for content in characters.by_ref() {
                        comment.push(content);
                        if content == '\n' {
                            break;
                        }
                    }
                    push_schema_token(&mut normalized, 'c', &comment);
                }
                '/' if characters.peek() == Some(&'*') => {
                    let mut comment = String::from(character);
                    comment.push(characters.next().expect("peeked comment delimiter"));
                    while let Some(content) = characters.next() {
                        comment.push(content);
                        if content == '*' && characters.peek() == Some(&'/') {
                            comment.push(characters.next().expect("peeked comment delimiter"));
                            break;
                        }
                    }
                    push_schema_token(&mut normalized, 'c', &comment);
                }
                _ => {
                    let mut operator = String::from(character);
                    if characters
                        .peek()
                        .is_some_and(|next| is_two_character_sql_operator(character, *next))
                    {
                        operator.push(characters.next().expect("peeked operator"));
                        if operator == "->" && characters.peek() == Some(&'>') {
                            operator.push(characters.next().expect("peeked operator"));
                        }
                    }
                    push_schema_token(&mut normalized, 's', &operator);
                }
            }
        }

        normalized
    }

    fn is_schema_word_character(character: char) -> bool {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '$') || !character.is_ascii()
    }

    fn is_two_character_sql_operator(first: char, second: char) -> bool {
        matches!(
            (first, second),
            ('<' | '>' | '!' | '=', '=') | ('<' | '-', '>') | ('|', '|')
        )
    }

    fn read_quoted_schema_token<I>(
        characters: &mut std::iter::Peekable<I>,
        delimiter: char,
        doubled_delimiter_escape: bool,
    ) -> (String, String, bool)
    where
        I: Iterator<Item = char>,
    {
        let mut raw = String::from(delimiter);
        let mut decoded = String::new();
        while let Some(character) = characters.next() {
            raw.push(character);
            if character == delimiter {
                if doubled_delimiter_escape && characters.peek() == Some(&delimiter) {
                    raw.push(characters.next().expect("peeked delimiter"));
                    decoded.push(delimiter);
                } else {
                    return (raw, decoded, true);
                }
            } else {
                decoded.push(character);
            }
        }
        (raw, decoded, false)
    }

    fn push_schema_token(normalized: &mut String, kind: char, value: &str) {
        write!(normalized, "{kind}{}:{value};", value.len())
            .expect("writing to String cannot fail");
    }

    // ---- Helpers ----

    fn normalized(sql: &str, allowed: Option<&str>) -> String {
        String::from_utf8(normalize_schema_sql(
            sql.as_bytes(),
            allowed.map(str::as_bytes),
        ))
        .expect("the normalized form of UTF-8 text is UTF-8")
    }

    fn contains(hay: &str, needle: &str) -> bool {
        token_run_contains(
            &tokenize_schema_sql(hay.as_bytes(), None),
            &tokenize_schema_sql(needle.as_bytes(), None),
        )
    }

    /// A small deterministic xorshift generator (no test-only dependency).
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next_u64() % (n as u64)).expect("below n")
        }
    }

    /// Characters that exercise every tokenizer branch, including non-ASCII
    /// text (2-, 3- and 4-byte UTF-8), control characters, vertical tab
    /// (not ASCII whitespace), and every delimiter and operator byte.
    const ALPHABET: &[char] = &[
        'a', 'Z', 'q', '0', '9', '_', '$', ' ', '\t', '\n', '\r', '\x0B', '\x0C', '\0', '"', '\'',
        '`', '[', ']', '-', '/', '*', '<', '>', '!', '=', '|', '(', ')', ',', ';', ':', '.', '+',
        '%', 'é', 'É', '€', '😀', '\u{85}', '\u{a0}',
    ];

    fn random_sql(rng: &mut Rng) -> String {
        let len = rng.below(24);
        (0..len)
            .map(|_| ALPHABET[rng.below(ALPHABET.len())])
            .collect()
    }

    // ---- Differential tests ----

    #[test]
    fn kernel_normalizer_matches_the_char_level_normalizer_on_random_text() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..200_000 {
            let sql = random_sql(&mut rng);
            let allowed_text = random_sql(&mut rng);
            for allowed in [
                None,
                Some("metadata"),
                Some("reservations"),
                Some(allowed_text.as_str()),
                Some(""),
            ] {
                assert_eq!(
                    normalized(&sql, allowed),
                    legacy_normalize_schema_sql(&sql, allowed),
                    "sql={sql:?} allowed={allowed:?}"
                );
            }
        }
    }

    #[test]
    fn kernel_normalizer_matches_the_char_level_normalizer_on_edge_cases() {
        let cases = [
            "",
            " \t\r\n\x0C",
            "\x0B",
            "\"",
            "\"\"",
            "\"\"\"\"",
            "\"a\"\"b\"",
            "\"metadata\"",
            "\"Metadata\"",
            "\"metadata",
            "'it''s'",
            "'unterminated",
            "`a``b`",
            "[a]]b",
            "[unterminated",
            "-",
            "--",
            "-- comment\nnext",
            "-->",
            "->",
            "->>",
            "->>>",
            "<=>",
            "<>=",
            "!==",
            "||||",
            "/",
            "/*",
            "/*/",
            "/**/",
            "/* a */ b",
            "/* never closed *",
            "CamelCase_Word$1 éÉ€😀",
            "a\u{85}b\u{a0}c",
            "CREATE TABLE \"reservations\" (port INTEGER NOT NULL)",
        ];
        for sql in cases {
            for allowed in [None, Some("metadata"), Some("a\"b"), Some("")] {
                assert_eq!(
                    normalized(sql, allowed),
                    legacy_normalize_schema_sql(sql, allowed),
                    "sql={sql:?} allowed={allowed:?}"
                );
            }
        }
    }

    #[test]
    fn kernel_normalizer_matches_the_schema_definitions() {
        for (table, sql) in [
            ("metadata", super::CREATE_METADATA_TABLE),
            ("reservations", super::CREATE_RESERVATIONS_TABLE),
        ] {
            assert_eq!(
                normalized(sql, Some(table)),
                legacy_normalize_schema_sql(sql, Some(table))
            );
        }
    }

    #[test]
    fn normalizer_encodes_each_token_with_its_byte_length() {
        assert_eq!(normalized("", None), "");
        assert_eq!(normalized("  ", None), "");
        assert_eq!(normalized("Port", None), "w4:port;");
        assert_eq!(normalized("é", None), "w2:é;");
        assert_eq!(normalized("->>", None), "s3:->>;");
        assert_eq!(normalized("\"a\"\"b\"", Some("a\"b")), "w3:a\"b;");
        assert_eq!(normalized("\"a\"\"b\"", None), "q6:\"a\"\"b\";");
        // Length prefixes keep the encoding injective: these two inputs
        // differ in token boundaries, so their normalized forms differ.
        assert_ne!(normalized("'a;w1:b'", None), normalized("'a' b", None));
        let long = "x".repeat(1234);
        assert_eq!(normalized(&long, None), format!("w1234:{long};"));
    }

    // ---- Token-run containment ----

    #[test]
    fn token_run_containment_is_aligned_to_token_boundaries() {
        let constraint = "CONSTRAINT valid_port CHECK (port BETWEEN 1 AND 65535)";
        assert!(contains(constraint, constraint));
        assert!(contains(
            &format!("port INTEGER NOT NULL {constraint}, project TEXT"),
            constraint
        ));
        assert!(contains(
            "constraint   VALID_PORT check(port between 1 and 65535)",
            constraint
        ));
        // The constraint text inside a comment or a quoted literal is one
        // token there, so it does not occur as a run of tokens.
        assert!(!contains(&format!("/* {constraint} */"), constraint));
        assert!(!contains(&format!("-- {constraint}\n"), constraint));
        assert!(!contains(&format!("'{constraint}'"), constraint));
        // Nor does the encoded token stream of the constraint, which the old
        // substring search over normalized text accepted.
        let encoded = normalized(constraint, None);
        assert!(!contains(&format!("/* {encoded} */"), constraint));
        assert!(!contains(&format!("'{encoded}'"), constraint));
        // A run must be contiguous and complete.
        assert!(!contains(
            "CONSTRAINT valid_port /* x */ CHECK (port BETWEEN 1 AND 65535)",
            constraint
        ));
        assert!(!contains(
            "CONSTRAINT valid_port CHECK (port BETWEEN 1 AND 6553)",
            constraint
        ));
        assert!(!contains("CONSTRAINT valid_port", constraint));
        // A bare word does not match inside a longer word.
        assert!(!contains("valid_ports", "valid_port"));
        // Boundaries: the empty needle occurs everywhere, including in empty
        // text; nothing non-empty occurs in empty text.
        assert!(contains("", ""));
        assert!(contains("a", ""));
        assert!(!contains("", "a"));
        assert!(contains("a b", "b"));
        assert!(contains("a b", "a"));
    }
}
