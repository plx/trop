//! Verified kernel: database schema-version classification, the v1-to-v2
//! legacy-row preflight, and table-layout matching.
//!
//! - `parse_i32_decimal` parses the stored `schema_version` text with exactly
//!   the grammar and range of `i32::from_str` (`[+-]?[0-9]+`).
//! - `classify_for_open`, `classify_after_lock` and `classify_for_validation`
//!   decide what opening, the migration's post-lock recheck, and read-only
//!   validation do with the observed metadata. A database is initialized only
//!   when neither the `metadata` nor the `reservations` table exists; a
//!   `metadata` table without exactly one well-formed `schema_version` row is
//!   corruption and is never restamped ("Corruption Detection, But No
//!   Recovery"). A newer version is never opened, migrated or validated; only
//!   the published version 1 migrates (to `CURRENT_SCHEMA_VERSION`, the next
//!   version); opening finds a database current exactly when validation finds
//!   it valid.
//! - `classify_legacy_row` reports which preflight categories a schema-v1 row
//!   violates. A row violates none exactly when the migration copies it
//!   faithfully: the copied row (tag `NULL` becomes `''`) is accepted by the
//!   v2 decoder (`storage::values_decode`) and decodes to the same key
//!   (`classify_legacy_row`'s postcondition), so the preflight blocks every
//!   row that would make the migrated database fail validation, and no
//!   other. The four categories the SQL preflight already counted keep their
//!   exact SQL meaning (`violations_of`).
//! - `columns_match` compares a table's columns with the expected layout;
//!   checking primary-key positions or hidden columns only strengthens it.
//!
//! Production glue: `crate::database::migrations` (`get_schema_version`,
//! `check_schema_compatibility`, the post-lock recheck, the legacy preflight,
//! the v1 layout check) and `crate::database::validation` (schema version,
//! metadata rows, column layouts). Reading SQLite (table existence, rows,
//! storage classes, `pragma_table_info`) stays in the glue.

use super::storage::{Cell, PathPlatform, SqlType};
use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The schema version this client reads and writes.
pub const CURRENT_SCHEMA_VERSION: i32 = 2;

/// The only released predecessor schema version, which migrates to
/// `CURRENT_SCHEMA_VERSION`.
pub const PUBLISHED_SCHEMA_VERSION: i32 = 1;

/// The migration chain is contiguous: the published version migrates to the
/// next version, which is the current one.
pub proof fn lemma_migration_chain_is_contiguous()
    ensures
        PUBLISHED_SCHEMA_VERSION + 1 == CURRENT_SCHEMA_VERSION,
        PUBLISHED_SCHEMA_VERSION < CURRENT_SCHEMA_VERSION,
{
}

// ===========================================================================
// Decimal `i32` text
// ===========================================================================

/// The length of an optional leading sign (`+` or `-`).
pub open spec fn sign_len(s: Seq<u8>) -> int {
    if s.len() > 0 && (s[0] == 43 || s[0] == 45) {
        1
    } else {
        0
    }
}

/// The value `i32::from_str` gives `s`: an optional `+`/`-`, then one or
/// more ASCII digits (leading zeros allowed), in `i32` range; `None` for
/// anything else (empty, a bare sign, any other byte, out of range).
pub open spec fn parsed_i32(s: Seq<u8>) -> Option<int> {
    let d = s.subrange(sign_len(s), s.len() as int);
    let magnitude = super::exclusions::decimal_value(d);
    let value = if s.len() > 0 && s[0] == 45 {
        -magnitude
    } else {
        magnitude
    };
    if d.len() > 0 && super::exclusions::all_digits(d) && i32::MIN <= value <= i32::MAX {
        Some(value)
    } else {
        None
    }
}

/// The value of digits is non-negative.
proof fn lemma_decimal_nonneg(s: Seq<u8>)
    requires
        super::exclusions::all_digits(s),
    ensures
        super::exclusions::decimal_value(s) >= 0,
    decreases s.len(),
{
    if s.len() > 0 {
        assert(super::exclusions::all_digits(s.drop_last())) by {
            assert forall|i: int| 0 <= i < s.drop_last().len() implies super::exclusions::is_digit(
                #[trigger] s.drop_last()[i],
            ) by {
                assert(s.drop_last()[i] == s[i]);
            }
        }
        lemma_decimal_nonneg(s.drop_last());
        assert(super::exclusions::is_digit(s[s.len() - 1]));
    }
}

/// A prefix of digits never has a larger value than the whole.
proof fn lemma_decimal_prefix_le(s: Seq<u8>, k: int)
    requires
        super::exclusions::all_digits(s),
        0 <= k <= s.len(),
    ensures
        super::exclusions::decimal_value(s.take(k)) <= super::exclusions::decimal_value(s),
    decreases s.len() - k,
{
    if k < s.len() {
        let t = s.take(k + 1);
        let p = s.take(k);
        assert(super::exclusions::all_digits(p)) by {
            assert forall|i: int| 0 <= i < p.len() implies super::exclusions::is_digit(
                #[trigger] p[i],
            ) by {
                assert(p[i] == s[i]);
            }
        }
        assert(t.drop_last() =~= p);
        assert(t.last() == s[k]);
        assert(super::exclusions::is_digit(s[k]));
        lemma_decimal_nonneg(p);
        lemma_decimal_prefix_le(s, k + 1);
    } else {
        assert(s.take(k) =~= s);
    }
}

/// Parses `s` exactly like `i32::from_str` (see `parsed_i32`). Total.
pub fn parse_i32_decimal(text: &[u8]) -> (r: Option<i32>)
    ensures
        r matches Some(v) ==> parsed_i32(text@) == Some(v as int),
        r is None <==> parsed_i32(text@) is None,
{
    let len = text.len();
    let negative = len > 0 && text[0] == b'-';
    let mut start: usize = 0;
    if len > 0 && (text[0] == b'+' || text[0] == b'-') {
        start = 1;
    }
    let ghost digits = text@.subrange(start as int, len as int);
    if start == len {
        return None;
    }
    // The largest magnitude the sign allows: 2^31 for `-`, 2^31 - 1 otherwise.
    let limit: u64 = if negative {
        2_147_483_648
    } else {
        2_147_483_647
    };
    let mut magnitude: u64 = 0;
    let mut index = start;
    while index < len
        invariant
            start <= index <= len == text@.len(),
            start as int == sign_len(text@),
            negative == (text@.len() > 0 && text@[0] == 45),
            digits == text@.subrange(start as int, len as int),
            super::exclusions::all_digits(digits.take(index - start)),
            magnitude as int == super::exclusions::decimal_value(digits.take(index - start)),
            magnitude <= limit,
            limit == if negative {
                2_147_483_648u64
            } else {
                2_147_483_647u64
            },
        decreases len - index,
    {
        let byte = text[index];
        proof {
            assert(digits[index - start] == byte);
        }
        // Bytes below `b'0'` wrap around to values above 9.
        let digit = byte.wrapping_sub(b'0');
        if digit > 9 {
            proof {
                if super::exclusions::all_digits(digits) {
                    assert(super::exclusions::is_digit(digits[index - start]));
                }
            }
            return None;
        }
        let ghost next = digits.take(index - start + 1);
        proof {
            assert(next.drop_last() =~= digits.take(index - start));
            assert(next.last() == byte);
            assert(super::exclusions::all_digits(next)) by {
                assert forall|j: int| 0 <= j < next.len() implies super::exclusions::is_digit(
                    #[trigger] next[j],
                ) by {
                    if j < index - start {
                        assert(next[j] == digits.take(index - start)[j]);
                    }
                }
            }
        }
        let value = magnitude * 10 + u64::from(digit);
        if value > limit {
            // More digits only increase the magnitude, which is already out
            // of range for this sign.
            proof {
                if super::exclusions::all_digits(digits) {
                    lemma_decimal_prefix_le(digits, index - start + 1);
                }
            }
            return None;
        }
        magnitude = value;
        index += 1;
    }
    proof {
        assert(digits.take(len - start) =~= digits);
    }
    // `magnitude` is within the limit for its sign, so the conversions below
    // succeed.
    let Ok(magnitude) = i64::try_from(magnitude) else {
        return None;
    };
    let signed = if negative {
        -magnitude
    } else {
        magnitude
    };
    i32::try_from(signed).ok()
}

// ===========================================================================
// Schema-version classification
// ===========================================================================

/// What the glue observed about the `metadata` table.
#[derive(Clone, Copy, Debug)]
pub enum MetadataState<'a> {
    /// There is no `metadata` table.
    NoTable,
    /// The table exists but has no `schema_version` row.
    NoVersionRow,
    /// The first `schema_version` row's value, and whether more rows follow.
    VersionRow { value: Cell<'a>, more_rows: bool },
}

/// Why the schema metadata is corrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corruption {
    /// A `reservations` table exists without a `metadata` table.
    MissingMetadataTable,
    /// The `metadata` table has no `schema_version` row.
    MissingVersionRow,
    /// The `metadata` table has more than one `schema_version` row.
    MultipleVersionRows,
    /// The value is not `TEXT`.
    NotText(SqlType),
    /// The value is `TEXT` that is not valid UTF-8.
    NotUtf8,
    /// The value is not a base-10 `i32`.
    NotAnInteger,
}

/// The value of a well-typed version cell, or why it is corrupt (type, then
/// encoding, then syntax).
pub open spec fn cell_version(value: Cell) -> Result<i32, Corruption> {
    match value {
        Cell::Text(t) => match parsed_i32(super::storage::text_bytes(t)) {
            Some(v) => Ok(v as i32),
            None => Err(Corruption::NotAnInteger),
        },
        Cell::InvalidText => Err(Corruption::NotUtf8),
        _ => Err(Corruption::NotText(super::storage::sql_type(value))),
    }
}

/// The stored version as opening reads it: a missing table or row, then the
/// first row's type and encoding, then a second row, then the syntax.
pub open spec fn open_version(meta: MetadataState) -> Result<i32, Corruption> {
    match meta {
        MetadataState::NoTable => Err(Corruption::MissingMetadataTable),
        MetadataState::NoVersionRow => Err(Corruption::MissingVersionRow),
        MetadataState::VersionRow { value, more_rows } => match value {
            Cell::Text(_) => if more_rows {
                Err(Corruption::MultipleVersionRows)
            } else {
                cell_version(value)
            },
            _ => cell_version(value),
        },
    }
}

/// The stored version as validation reads it: the same checks, with the
/// syntax checked before a second row.
pub open spec fn validation_version(meta: MetadataState) -> Result<i32, Corruption> {
    match meta {
        MetadataState::NoTable => Err(Corruption::MissingMetadataTable),
        MetadataState::NoVersionRow => Err(Corruption::MissingVersionRow),
        MetadataState::VersionRow { value, more_rows } => match cell_version(value) {
            Ok(v) => if more_rows {
                Err(Corruption::MultipleVersionRows)
            } else {
                Ok(v)
            },
            Err(c) => Err(c),
        },
    }
}

/// Both readers accept exactly the same metadata, with the same version.
pub proof fn lemma_readers_agree(meta: MetadataState)
    ensures
        open_version(meta) is Ok <==> validation_version(meta) is Ok,
        open_version(meta) is Ok ==> open_version(meta) == validation_version(meta),
{
}

/// What opening a database does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenDisposition {
    /// Neither table exists: create the current schema.
    Initialize,
    /// The schema is current.
    UpToDate,
    /// A newer schema: refuse.
    Future(i32),
    /// An older (or nonsensical) version with no migration: refuse.
    NoMigrationPath(i32),
    /// The published version: migrate (when writable).
    MigrateFromPublished,
    /// The metadata is corrupt: refuse, change nothing.
    Corrupt(Corruption),
}

/// The disposition of a stored version.
pub open spec fn version_disposition(v: i32) -> OpenDisposition {
    if v > CURRENT_SCHEMA_VERSION {
        OpenDisposition::Future(v)
    } else if v == CURRENT_SCHEMA_VERSION {
        OpenDisposition::UpToDate
    } else if v == PUBLISHED_SCHEMA_VERSION {
        OpenDisposition::MigrateFromPublished
    } else {
        OpenDisposition::NoMigrationPath(v)
    }
}

/// What opening a database does, given its metadata and whether a
/// `reservations` table exists.
pub open spec fn open_disposition(meta: MetadataState, reservations_table: bool) -> OpenDisposition {
    match meta {
        MetadataState::NoTable => if reservations_table {
            OpenDisposition::Corrupt(Corruption::MissingMetadataTable)
        } else {
            OpenDisposition::Initialize
        },
        _ => match open_version(meta) {
            Ok(v) => version_disposition(v),
            Err(c) => OpenDisposition::Corrupt(c),
        },
    }
}

/// Text that decodes as `i32`, decoded. Total.
fn version_of_text(t: &str) -> (r: Result<i32, Corruption>)
    ensures
        r == cell_version(Cell::Text(t)),
{
    match parse_i32_decimal(t.as_bytes()) {
        Some(v) => Ok(v),
        None => Err(Corruption::NotAnInteger),
    }
}

/// The version of a version cell, or why it is corrupt. Total.
fn version_of_cell(value: Cell) -> (r: Result<i32, Corruption>)
    ensures
        r == cell_version(value),
{
    match value {
        Cell::Text(t) => version_of_text(t),
        Cell::InvalidText => Err(Corruption::NotUtf8),
        _ => Err(Corruption::NotText(super::storage::cell_type(value))),
    }
}

/// Decides `open_disposition`. Total.
pub fn classify_for_open(meta: MetadataState, reservations_table: bool) -> (d: OpenDisposition)
    ensures
        d == open_disposition(meta, reservations_table),
{
    let version = match meta {
        MetadataState::NoTable => {
            return if reservations_table {
                OpenDisposition::Corrupt(Corruption::MissingMetadataTable)
            } else {
                OpenDisposition::Initialize
            };
        },
        MetadataState::NoVersionRow => Err(Corruption::MissingVersionRow),
        MetadataState::VersionRow { value, more_rows } => match value {
            Cell::Text(_) if more_rows => Err(Corruption::MultipleVersionRows),
            _ => version_of_cell(value),
        },
    };
    let v = match version {
        Err(c) => return OpenDisposition::Corrupt(c),
        Ok(v) => v,
    };
    if v > CURRENT_SCHEMA_VERSION {
        return OpenDisposition::Future(v);
    }
    if v == CURRENT_SCHEMA_VERSION {
        OpenDisposition::UpToDate
    } else if v == PUBLISHED_SCHEMA_VERSION {
        OpenDisposition::MigrateFromPublished
    } else {
        OpenDisposition::NoMigrationPath(v)
    }
}

/// Only a database with neither table is initialized: any `metadata` table
/// (with or without a version row) or a `reservations` table alone prevents
/// it, so corrupt metadata is never silently restamped.
pub proof fn lemma_initialize_only_empty(meta: MetadataState, reservations_table: bool)
    ensures
        open_disposition(meta, reservations_table) == OpenDisposition::Initialize <==> (
        meta is NoTable && !reservations_table),
{
}

/// A `metadata` table without exactly one integer `schema_version` row is
/// corruption.
pub proof fn lemma_malformed_metadata_is_corrupt(meta: MetadataState, reservations_table: bool)
    requires
        !(meta is NoTable),
        open_version(meta) is Err,
    ensures
        open_disposition(meta, reservations_table) is Corrupt,
{
}

/// A newer stored version is refused by opening, by the post-lock recheck
/// and by validation; it is never migrated or written.
pub proof fn lemma_future_always_refused(meta: MetadataState, reservations_table: bool)
    requires
        open_version(meta) matches Ok(v) && v > CURRENT_SCHEMA_VERSION,
    ensures
        open_disposition(meta, reservations_table) is Future,
        lock_disposition(meta, reservations_table) is Future,
        validate_disposition(meta) is Future,
{
    lemma_readers_agree(meta);
}

/// What the migration does after acquiring the write lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockedDisposition {
    /// Another client already migrated: nothing to do.
    AlreadyCurrent,
    /// A newer schema appeared: refuse.
    Future(i32),
    /// The version is no longer the published one: refuse.
    Changed(i32),
    /// Still the published version: migrate.
    Proceed,
    /// The metadata is corrupt: refuse.
    Corrupt(Corruption),
}

/// The post-lock recheck, as a function of the observed metadata.
pub open spec fn lock_disposition(meta: MetadataState, reservations_table: bool) -> LockedDisposition {
    match open_disposition(meta, reservations_table) {
        OpenDisposition::Initialize => LockedDisposition::Changed(0),
        OpenDisposition::UpToDate => LockedDisposition::AlreadyCurrent,
        OpenDisposition::Future(v) => LockedDisposition::Future(v),
        OpenDisposition::NoMigrationPath(v) => LockedDisposition::Changed(v),
        OpenDisposition::MigrateFromPublished => LockedDisposition::Proceed,
        OpenDisposition::Corrupt(c) => LockedDisposition::Corrupt(c),
    }
}

/// Decides `lock_disposition`. Total.
pub fn classify_after_lock(meta: MetadataState, reservations_table: bool) -> (d: LockedDisposition)
    ensures
        d == lock_disposition(meta, reservations_table),
{
    match classify_for_open(meta, reservations_table) {
        OpenDisposition::Initialize => LockedDisposition::Changed(0),
        OpenDisposition::UpToDate => LockedDisposition::AlreadyCurrent,
        OpenDisposition::Future(v) => LockedDisposition::Future(v),
        OpenDisposition::NoMigrationPath(v) => LockedDisposition::Changed(v),
        OpenDisposition::MigrateFromPublished => LockedDisposition::Proceed,
        OpenDisposition::Corrupt(c) => LockedDisposition::Corrupt(c),
    }
}

/// The migration proceeds after the lock only from a state opening would
/// migrate: exactly the published version.
pub proof fn lemma_proceed_only_from_published(meta: MetadataState, reservations_table: bool)
    ensures
        lock_disposition(meta, reservations_table) == LockedDisposition::Proceed <==> (
        open_disposition(meta, reservations_table) == OpenDisposition::MigrateFromPublished),
        lock_disposition(meta, reservations_table) == LockedDisposition::Proceed ==> open_version(
            meta,
        ) == Ok::<i32, Corruption>(PUBLISHED_SCHEMA_VERSION),
{
}

/// What read-only validation reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidateDisposition {
    /// The schema is current.
    Valid,
    /// A newer schema.
    Future(i32),
    /// An older version (any version below the current one).
    MigrationRequired(i32),
    /// The metadata is corrupt.
    Corrupt(Corruption),
}

/// Read-only validation's verdict on the metadata.
pub open spec fn validate_disposition(meta: MetadataState) -> ValidateDisposition {
    match validation_version(meta) {
        Err(c) => ValidateDisposition::Corrupt(c),
        Ok(v) => if v > CURRENT_SCHEMA_VERSION {
            ValidateDisposition::Future(v)
        } else if v < CURRENT_SCHEMA_VERSION {
            ValidateDisposition::MigrationRequired(v)
        } else {
            ValidateDisposition::Valid
        },
    }
}

/// Decides `validate_disposition`. Total.
pub fn classify_for_validation(meta: MetadataState) -> (d: ValidateDisposition)
    ensures
        d == validate_disposition(meta),
{
    let version = match meta {
        MetadataState::NoTable => Err(Corruption::MissingMetadataTable),
        MetadataState::NoVersionRow => Err(Corruption::MissingVersionRow),
        MetadataState::VersionRow { value, more_rows } => match version_of_cell(value) {
            Ok(_) if more_rows => Err(Corruption::MultipleVersionRows),
            other => other,
        },
    };
    let v = match version {
        Err(c) => return ValidateDisposition::Corrupt(c),
        Ok(v) => v,
    };
    if v > CURRENT_SCHEMA_VERSION {
        return ValidateDisposition::Future(v);
    }
    if v < CURRENT_SCHEMA_VERSION {
        return ValidateDisposition::MigrationRequired(v);
    }
    ValidateDisposition::Valid
}

/// Opening finds a database current exactly when validation finds it valid,
/// for every state of the metadata (missing table or row, duplicate rows,
/// wrong types, bad text) and either state of the `reservations` table.
pub proof fn lemma_open_current_iff_valid(meta: MetadataState, reservations_table: bool)
    ensures
        open_disposition(meta, reservations_table) == OpenDisposition::UpToDate <==> validate_disposition(
            meta,
        ) == ValidateDisposition::Valid,
{
    lemma_readers_agree(meta);
}

// ===========================================================================
// The v1-to-v2 legacy preflight
// ===========================================================================

/// One schema-v1 `reservations` row, cell by cell, and the platform's facts
/// about its path text (`Unix` when the path is not text).
#[derive(Clone, Copy, Debug)]
pub struct LegacyRow<'a> {
    /// `path`.
    pub path: Cell<'a>,
    /// The platform's facts about the path text.
    pub path_platform: PathPlatform,
    /// `tag`.
    pub tag: Cell<'a>,
    /// `port`.
    pub port: Cell<'a>,
    /// `project`.
    pub project: Cell<'a>,
    /// `task`.
    pub task: Cell<'a>,
    /// `created_at`.
    pub created_at: Cell<'a>,
    /// `last_used_at`.
    pub last_used_at: Cell<'a>,
}

/// The preflight categories one legacy row violates.
// One flag per reported category: the glue adds each to its own counter.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegacyRowViolations {
    /// `tag = ''` (it would alias the untagged `NULL` tag).
    pub empty_tag: bool,
    /// `typeof(port) <> 'integer' OR port NOT BETWEEN 1 AND 65535`.
    pub invalid_port: bool,
    /// `typeof(created_at) <> 'integer' OR typeof(last_used_at) <> 'integer'
    /// OR created_at < 0 OR last_used_at < 0`.
    pub negative_timestamp: bool,
    /// `typeof(path) <> 'text' OR typeof(tag|project|task) NOT IN ('null',
    /// 'text')`.
    pub invalid_scalar_type: bool,
    /// A text value (path, tag, project, task) that is not valid UTF-8.
    pub invalid_text: bool,
    /// Path text that is not a storable (canonical absolute) path.
    pub noncanonical_path: bool,
    /// Non-empty tag text that is blank or untrimmed.
    pub noncanonical_tag: bool,
    /// Project text that is empty, blank or untrimmed.
    pub noncanonical_project: bool,
    /// Task text that is empty, blank or untrimmed.
    pub noncanonical_task: bool,
    /// An integer timestamp above `MAX_STORABLE_UNIX_SECS`.
    pub timestamp_out_of_range: bool,
}

/// The cell is `NULL` or `TEXT`.
pub open spec fn null_or_text(c: Cell) -> bool {
    super::storage::sql_type(c) == SqlType::Null || super::storage::sql_type(c) == SqlType::Text
}

/// The text of a text cell, `None` otherwise.
pub open spec fn cell_text(c: Cell) -> Option<Seq<char>> {
    match c {
        Cell::Text(t) => Some(t@),
        _ => None,
    }
}

/// The tag the migration writes: `COALESCE(tag, '')`.
pub open spec fn migrated_tag(c: Cell) -> Seq<char> {
    match c {
        Cell::Text(t) => t@,
        _ => Seq::empty(),
    }
}

/// The v1 key's tag: `NULL` is untagged, text is that tag.
pub open spec fn legacy_tag(c: Cell) -> Option<Seq<char>> {
    cell_text(c)
}

/// The integer of an integer cell (0 otherwise).
pub open spec fn cell_int(c: Cell) -> int {
    match c {
        Cell::Integer(v) => v as int,
        _ => 0,
    }
}

/// The text cell is not a canonical identifier.
pub open spec fn noncanonical_text(c: Cell) -> bool {
    c matches Cell::Text(t) && !super::identifier::is_canonical(t@)
}

/// The integer cell lies above the storable range.
pub open spec fn above_max(c: Cell) -> bool {
    c matches Cell::Integer(v) && v > super::storage::MAX_STORABLE_UNIX_SECS
}

/// The cell is `''`.
pub open spec fn empty_text(c: Cell) -> bool {
    c matches Cell::Text(t) && t@.len() == 0
}

/// The cell is a valid port integer.
pub open spec fn valid_port_cell(c: Cell) -> bool {
    c matches Cell::Integer(p) && super::port::is_valid_port(p as int)
}

/// The cell is a non-negative integer.
pub open spec fn nonneg_int(c: Cell) -> bool {
    c matches Cell::Integer(v) && v >= 0
}

/// The cell is path text that is not storable.
pub open spec fn unstorable_path_text(c: Cell, platform: PathPlatform) -> bool {
    c matches Cell::Text(p) && !super::storage::storable_path(super::storage::text_bytes(p), platform)
}

/// The cell is non-empty text that is not canonical.
pub open spec fn padded_tag_text(c: Cell) -> bool {
    c matches Cell::Text(t) && t@.len() > 0 && !super::identifier::is_canonical(t@)
}

/// The exact meaning of each category.
pub open spec fn violations_of(r: LegacyRow) -> LegacyRowViolations {
    LegacyRowViolations {
        empty_tag: empty_text(r.tag),
        invalid_port: !valid_port_cell(r.port),
        negative_timestamp: !(nonneg_int(r.created_at) && nonneg_int(r.last_used_at)),
        invalid_scalar_type: super::storage::sql_type(r.path) != SqlType::Text || !null_or_text(
            r.tag,
        ) || !null_or_text(r.project) || !null_or_text(r.task),
        invalid_text: r.path is InvalidText || r.tag is InvalidText || r.project is InvalidText
            || r.task is InvalidText,
        noncanonical_path: unstorable_path_text(r.path, r.path_platform),
        noncanonical_tag: padded_tag_text(r.tag),
        noncanonical_project: noncanonical_text(r.project),
        noncanonical_task: noncanonical_text(r.task),
        timestamp_out_of_range: above_max(r.created_at) || above_max(r.last_used_at),
    }
}

/// No category is violated.
pub open spec fn no_violations(v: LegacyRowViolations) -> bool {
    !v.empty_tag && !v.invalid_port && !v.negative_timestamp && !v.invalid_scalar_type
        && !v.invalid_text && !v.noncanonical_path && !v.noncanonical_tag && !v.noncanonical_project
        && !v.noncanonical_task && !v.timestamp_out_of_range
}

/// The migration copies the row faithfully: every value already has its v2
/// storage class (so the copy changes no value), the copied row (tag `NULL`
/// becomes `''`) is accepted by the v2 decoder, and it decodes to the same
/// key (a v1 `''` tag would decode as untagged).
pub open spec fn migrates_faithfully(r: LegacyRow) -> bool {
    &&& r.path is Text
    &&& r.tag is Null || r.tag is Text
    &&& r.port is Integer
    &&& r.project is Null || r.project is Text
    &&& r.task is Null || r.task is Text
    &&& r.created_at is Integer
    &&& r.last_used_at is Integer
    &&& super::storage::values_decode(
        super::storage::text_bytes(r.path->Text_0),
        r.path_platform,
        migrated_tag(r.tag),
        cell_int(r.port),
        cell_text(r.project),
        cell_text(r.task),
        cell_int(r.created_at),
        cell_int(r.last_used_at),
    )
    &&& super::identifier::decode_stored_tag(migrated_tag(r.tag)) == Some(legacy_tag(r.tag))
}

/// The tag migration (`COALESCE(tag, '')`) keeps the v1 key exactly when
/// the v1 tag is `NULL` (it becomes the untagged sentinel `''`) or canonical
/// text; a v1 `''` tag would turn into the untagged key, which is why empty
/// legacy tags block the migration.
pub proof fn lemma_legacy_tag_migration(c: Cell)
    requires
        null_or_text(c),
        !(c is InvalidText),
    ensures
        (super::identifier::decode_stored_tag(migrated_tag(c)) == Some(legacy_tag(c))) <==> (
        c is Null || (!empty_text(c) && !padded_tag_text(c))),
        c is Null ==> super::identifier::decode_stored_tag(migrated_tag(c)) == Some(
            None::<Seq<char>>,
        ),
{
    if let Cell::Text(t) = c {
        if t@.len() == 0 {
            assert(super::identifier::decode_stored_tag(t@) == Some(None::<Seq<char>>));
        }
    }
}

/// Whether an optional-identifier cell is non-canonical text. Total.
fn text_cell_noncanonical(c: Cell) -> (r: bool)
    ensures
        r == noncanonical_text(c),
{
    match c {
        Cell::Text(t) => !super::identifier::is_canonical_identifier(t),
        _ => false,
    }
}

/// Whether a cell is `NULL` or `TEXT`. Total.
fn cell_null_or_text(c: Cell) -> (r: bool)
    ensures
        r == null_or_text(c),
{
    matches!(c, Cell::Null | Cell::Text(_) | Cell::InvalidText)
}

/// Whether an integer cell lies above the storable range. Total.
fn cell_above_max(c: Cell) -> (r: bool)
    ensures
        r == above_max(c),
{
    match c {
        Cell::Integer(v) => match u64::try_from(v) {
            Ok(u) => u > super::storage::MAX_STORABLE_UNIX_SECS,
            Err(_) => false,
        },
        _ => false,
    }
}

/// The preflight categories a legacy row violates. Total. Each flag has its
/// exact meaning (`violations_of`), and a row violates no category exactly
/// when the migration copies it faithfully, so the preflight blocks every
/// row that would make the migrated database fail validation and no other.
pub fn classify_legacy_row(r: &LegacyRow) -> (v: LegacyRowViolations)
    ensures
        v == violations_of(*r),
        no_violations(v) <==> migrates_faithfully(*r),
{
    let empty_tag = match r.tag {
        Cell::Text(t) => t.is_empty(),
        _ => false,
    };
    let invalid_port = match r.port {
        Cell::Integer(p) => super::storage::decode_stored_port(p).is_none(),
        _ => true,
    };
    let negative_timestamp = match (r.created_at, r.last_used_at) {
        (Cell::Integer(c), Cell::Integer(l)) => c < 0 || l < 0,
        _ => true,
    };
    let invalid_scalar_type = !matches!(r.path, Cell::Text(_) | Cell::InvalidText)
        || !cell_null_or_text(r.tag) || !cell_null_or_text(r.project) || !cell_null_or_text(r.task);
    let invalid_text = matches!(r.path, Cell::InvalidText) || matches!(r.tag, Cell::InvalidText)
        || matches!(r.project, Cell::InvalidText) || matches!(r.task, Cell::InvalidText);
    let noncanonical_path = match r.path {
        Cell::Text(p) => !matches!(
            super::storage::classify_stored_path(p.as_bytes(), r.path_platform),
            super::storage::PathVerdict::Canonical
        ),
        _ => false,
    };
    let noncanonical_tag = match r.tag {
        Cell::Text(t) => !t.is_empty() && !super::identifier::is_canonical_identifier(t),
        _ => false,
    };
    let noncanonical_project = text_cell_noncanonical(r.project);
    let noncanonical_task = text_cell_noncanonical(r.task);
    let timestamp_out_of_range = cell_above_max(r.created_at) || cell_above_max(r.last_used_at);
    let v = LegacyRowViolations {
        empty_tag,
        invalid_port,
        negative_timestamp,
        invalid_scalar_type,
        invalid_text,
        noncanonical_path,
        noncanonical_tag,
        noncanonical_project,
        noncanonical_task,
        timestamp_out_of_range,
    };
    proof {
        lemma_clean_iff_faithful(*r);
    }
    v
}

/// No violation exactly when the migration is faithful.
proof fn lemma_clean_iff_faithful(r: LegacyRow)
    ensures
        no_violations(violations_of(r)) <==> migrates_faithfully(r),
{
    let v = violations_of(r);
    if no_violations(v) {
        assert(r.path is Text);
        match r.tag {
            Cell::Text(t) => {
                assert(t@.len() > 0);
                assert(super::identifier::is_canonical(t@));
            },
            _ => {},
        }
    }
    if migrates_faithfully(r) {
        match r.tag {
            Cell::Text(t) => {
                if t@.len() == 0 {
                    assert(super::identifier::decode_stored_tag(t@) == Some(
                        None::<Seq<char>>,
                    ));
                }
            },
            _ => {},
        }
    }
}

// ===========================================================================
// Table layouts
// ===========================================================================

/// One column of a table (`pragma_table_info` / `pragma_table_xinfo`), or
/// one expected column (`hidden` 0).
#[derive(Clone, Copy, Debug)]
pub struct ColumnFacts<'a> {
    /// The column name.
    pub name: &'a str,
    /// The declared type.
    pub declared_type: &'a str,
    /// `NOT NULL`.
    pub not_null: bool,
    /// The 1-based position in the primary key, or 0.
    pub primary_key_position: i64,
    /// `pragma_table_xinfo`'s `hidden` (0 for an ordinary column).
    pub hidden: i64,
}

/// One column matches its expected column: the same name (bytes), the same
/// declared type ignoring ASCII case, the same `NOT NULL`, not hidden (when
/// checked), and the same primary-key position (when checked).
pub open spec fn column_matches(
    actual: ColumnFacts,
    expected: ColumnFacts,
    check_pk: bool,
    check_hidden: bool,
) -> bool {
    &&& super::storage::text_bytes(actual.name) == super::storage::text_bytes(expected.name)
    &&& super::text::eq_ignore_ascii_case(
        super::storage::text_bytes(actual.declared_type),
        super::storage::text_bytes(expected.declared_type),
    )
    &&& actual.not_null == expected.not_null
    &&& check_hidden ==> actual.hidden == 0
    &&& check_pk ==> actual.primary_key_position == expected.primary_key_position
}

/// The table has exactly the expected columns, in order.
pub open spec fn layout_matches(
    actual: Seq<ColumnFacts>,
    expected: Seq<ColumnFacts>,
    check_pk: bool,
    check_hidden: bool,
) -> bool {
    actual.len() == expected.len() && forall|i: int|
        0 <= i < actual.len() ==> #[trigger] column_matches(
            actual[i],
            expected[i],
            check_pk,
            check_hidden,
        )
}

/// Decides `layout_matches`. Total.
pub fn columns_match(
    actual: &[ColumnFacts],
    expected: &[ColumnFacts],
    check_pk: bool,
    check_hidden: bool,
) -> (r: bool)
    ensures
        r == layout_matches(actual@, expected@, check_pk, check_hidden),
{
    if actual.len() != expected.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < actual.len()
        invariant
            actual.len() == expected.len(),
            0 <= i <= actual.len(),
            forall|j: int|
                0 <= j < i ==> #[trigger] column_matches(
                    actual@[j],
                    expected@[j],
                    check_pk,
                    check_hidden,
                ),
        decreases actual.len() - i,
    {
        let a = &actual[i];
        let e = &expected[i];
        let ok = super::text::bytes_equal(a.name.as_bytes(), e.name.as_bytes())
            && super::text::bytes_eq_ignore_ascii_case(
            a.declared_type.as_bytes(),
            e.declared_type.as_bytes(),
        ) && a.not_null == e.not_null && (!check_hidden || a.hidden == 0) && (!check_pk
            || a.primary_key_position == e.primary_key_position);
        proof {
            assert(*a == actual@[i as int]);
            assert(*e == expected@[i as int]);
            assert(ok == column_matches(*a, *e, check_pk, check_hidden));
        }
        if !ok {
            return false;
        }
        i += 1;
    }
    true
}

/// Checking the primary key (or hidden columns) only strengthens the match.
pub proof fn lemma_more_checks_only_strengthen(
    actual: Seq<ColumnFacts>,
    expected: Seq<ColumnFacts>,
    check_pk: bool,
    check_hidden: bool,
)
    requires
        layout_matches(actual, expected, check_pk, check_hidden),
    ensures
        layout_matches(actual, expected, false, check_hidden),
        layout_matches(actual, expected, check_pk, false),
{
    assert forall|i: int| 0 <= i < actual.len() implies #[trigger] column_matches(
        actual[i],
        expected[i],
        false,
        check_hidden,
    ) by {
        assert(column_matches(actual[i], expected[i], check_pk, check_hidden));
    }
    assert forall|i: int| 0 <= i < actual.len() implies #[trigger] column_matches(
        actual[i],
        expected[i],
        check_pk,
        false,
    ) by {
        assert(column_matches(actual[i], expected[i], check_pk, check_hidden));
    }
}

} // verus!
