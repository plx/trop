//! Read-only physical, schema, and logical database validation.

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};

use crate::error::{Error, Result};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualColumn {
    name: String,
    declared_type: String,
    not_null: bool,
    primary_key_position: i64,
    hidden: i64,
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

fn validate_schema_version(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare(
        "SELECT value FROM metadata
         WHERE key = 'schema_version'",
    )?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(Error::corrupt_stored_value(
            "metadata",
            "value",
            "key=\"schema_version\"",
            "exactly one schema_version row is required",
        ));
    };
    let value = text_value(
        row.get_ref(0)?,
        "metadata",
        "value",
        "key=\"schema_version\"",
    )?;
    let version = value.parse::<i32>().map_err(|_| {
        Error::corrupt_stored_value(
            "metadata",
            "value",
            "key=\"schema_version\"",
            "schema_version must be a base-10 signed integer",
        )
    })?;
    if rows.next()?.is_some() {
        return Err(Error::corrupt_stored_value(
            "metadata",
            "value",
            "key=\"schema_version\"",
            "exactly one schema_version row is required",
        ));
    }

    if version > CURRENT_SCHEMA_VERSION {
        return Err(Error::UnsupportedSchemaVersion {
            expected: u32::try_from(CURRENT_SCHEMA_VERSION).unwrap_or_default(),
            found: u32::try_from(version).unwrap_or(u32::MAX),
        });
    }
    if version < CURRENT_SCHEMA_VERSION {
        return Err(Error::MigrationRequired {
            found: version,
            target: CURRENT_SCHEMA_VERSION,
            action: "validation is read-only; reopen the database writable to apply the ordered \
                     migration, or validate a writable migrated copy"
                .into(),
        });
    }

    Ok(())
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

    if schema_versions.len() != 1 {
        return Err(Error::corrupt_stored_value(
            "metadata",
            "value",
            "key=\"schema_version\"",
            "exactly one schema_version row is required",
        ));
    }

    if schema_versions[0].parse::<i32>().is_err() {
        return Err(Error::corrupt_stored_value(
            "metadata",
            "value",
            "key=\"schema_version\"",
            "schema_version must be a base-10 signed integer",
        ));
    }

    Ok(())
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
    let normalized = normalize_schema_sql(&sql);

    for (name, expression) in [
        ("valid_port", "check(portbetween1and65535)"),
        ("valid_created_at", "check(created_at>=0)"),
        ("valid_last_used_at", "check(last_used_at>=0)"),
    ] {
        let required = format!("constraint{name}{expression}");
        if !normalized.contains(&required) {
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
        let expected = normalize_schema_sql(create_sql).replacen("ifnotexists", "", 1);
        if normalize_schema_sql(&actual) != expected {
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

fn column_shapes_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(actual, expected)| {
            actual.name == expected.name
                && actual
                    .declared_type
                    .eq_ignore_ascii_case(expected.declared_type)
                && actual.not_null == expected.not_null
                && actual.hidden == 0
        })
}

fn columns_match(actual: &[ActualColumn], expected: &[ColumnSpec]) -> bool {
    column_shapes_match(actual, expected)
        && actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.primary_key_position == expected.primary_key_position)
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

fn escape_text(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
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

fn normalize_schema_sql(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '"')
        .flat_map(char::to_lowercase)
        .collect()
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
