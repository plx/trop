//! Verified kernel: schema-v2 stored values (ports, timestamps, paths, tags,
//! whole reservation rows) and the read/write predicates.
//!
//! - Ports and timestamps: `decode_stored_port` accepts exactly
//!   `1..=65535` (the schema's `CHECK` and `is_valid_port`);
//!   `encode_unix_secs` and `decode_unix_secs` share the domain
//!   `0..=MAX_STORABLE_UNIX_SECS` and invert each other, so every encodable
//!   timestamp satisfies `CHECK (... >= 0)` and decodes back unchanged.
//! - Paths: `classify_stored_path` decides `path_verdict`. On Unix a stored
//!   path is storable exactly when it is canonical (`spec_is_canonical_abs`),
//!   i.e. a fixed point of normalization; on Windows native absolute paths
//!   are judged by the platform facts the glue observes, and `/`-rooted
//!   spellings written on Unix by the portable form (no `\`).
//! - Keys: `encode_storage_key` is the write-side key check. It never
//!   transcodes: the stored path text is the key's own UTF-8 text (non-UTF-8
//!   paths are rejected instead of being aliased through U+FFFD), and the
//!   stored tag is `encode_stored_tag` (`None` as `""`). The write predicate
//!   `storable_key` is literally the decoder's predicate
//!   (`lemma_storable_key_iff_decodes`), stored keys are injective
//!   (`lemma_stored_key_injective`), and on Unix stored-text equality is
//!   component equality (`lemma_unix_stored_identity_is_component_identity`).
//! - Rows: `encode_record` is the whole write path for one reservation (an
//!   error is the first failing check in field order, `first_record_error`),
//!   and `row_decodes` is what `row_to_reservation` accepts once the SQL
//!   types are right. Every encoded row decodes to the original fields
//!   (`lemma_encoded_record_round_trips`); a decodable row's key is storable
//!   and its tag re-encodes to the stored text
//!   (`lemma_decoded_row_re_encodes`). The `sticky` flag is not part of the
//!   row: it is not persisted. `StoredKey` and `StoredRow` have private
//!   fields, so every bound key comes from `encode_storage_key` and every
//!   inserted row from `encode_record`; the `UPDATE` statements' other
//!   values (timestamps, and the metadata update's project and task) are
//!   checked by the glue.
//! - Lookups: `lookup_key` is the key's exact text (storable or not), and
//!   among rows whose tag decodes it matches exactly the rows holding the
//!   requested key; there is no lookup when no row can hold the key (no
//!   UTF-8 text, or a `Some("")` tag, which would alias the untagged row).
//! - Prefix queries: `prefix_query` turns a `Path::starts_with` prefix into
//!   the canonical SQL key; with the component-prefix post-filter it selects
//!   exactly the stored paths the `std` model of `starts_with` selects (its
//!   postcondition), and every selected path starts with the key's bytes (by
//!   definition; `lemma_prefix_query_exact`), so the byte-prefix SQL
//!   prefilter never drops a true match.
//!
//! Production glue: `crate::database::operations` (`row_to_reservation`, the
//! write paths, the lookups, `get_reservations_by_path_prefix`, the
//! busy-timeout readback), `crate::database::transaction`,
//! `crate::reservation` (`ReservationKey::storage_key`),
//! `crate::path::storage_platform`. SQL semantics (`=` and `instr` on TEXT,
//! `CHECK`, `UNIQUE`, value storage classes), rusqlite's value mapping,
//! `Path::to_str` and the `std::path` model are trusted.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// Ports
// ===========================================================================

/// Decodes a stored `port` value: exactly the valid ports `1..=65535`, which
/// is also the schema's `CHECK (port BETWEEN 1 AND 65535)`. Total.
pub fn decode_stored_port(value: i64) -> (port: Option<u16>)
    ensures
        port matches Some(p) ==> p as int == value as int,
        port is Some <==> super::port::is_valid_port(value as int),
{
    match u16::try_from(value) {
        Ok(p) if p != 0 => Some(p),
        _ => None,
    }
}

// ===========================================================================
// Timestamps (whole seconds since the Unix epoch)
// ===========================================================================

/// The largest Unix timestamp trop stores and reads back: chrono's maximum
/// `DateTime<Utc>` (`+262142-12-31T23:59:59Z`). A unit test pins it against
/// chrono (`from_timestamp(MAX, 0)` exists and `MAX + 1` does not).
pub const MAX_STORABLE_UNIX_SECS: u64 = 8_210_266_876_799;

/// A storable timestamp: non-negative (the schema's `CHECK (... >= 0)`) and
/// at most `MAX_STORABLE_UNIX_SECS`.
pub open spec fn storable_secs(value: int) -> bool {
    0 <= value <= MAX_STORABLE_UNIX_SECS
}

/// Why a timestamp cannot be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecsEncodeError {
    /// Larger than `i64::MAX` (SQLite integers are signed 64-bit).
    ExceedsI64,
    /// Representable, but beyond `MAX_STORABLE_UNIX_SECS`.
    OutsideSupportedRange,
}

/// Why `secs` cannot be written, or `None` when it is storable: beyond
/// `i64::MAX` first, then beyond `MAX_STORABLE_UNIX_SECS`.
pub open spec fn secs_error(secs: u64) -> Option<SecsEncodeError> {
    if secs as int > i64::MAX as int {
        Some(SecsEncodeError::ExceedsI64)
    } else if secs > MAX_STORABLE_UNIX_SECS {
        Some(SecsEncodeError::OutsideSupportedRange)
    } else {
        None
    }
}

/// Encodes whole seconds since the epoch as the stored `INTEGER`. Total.
pub fn encode_unix_secs(secs: u64) -> (r: Result<i64, SecsEncodeError>)
    ensures
        r matches Ok(v) ==> v as int == secs as int,
        r is Ok <==> storable_secs(secs as int),
        r matches Err(e) ==> (e == SecsEncodeError::ExceedsI64 <==> secs as int > i64::MAX as int),
        r matches Err(e) ==> secs_error(secs) == Some(e),
        r is Ok <==> secs_error(secs) is None,
{
    match i64::try_from(secs) {
        Err(_) => Err(SecsEncodeError::ExceedsI64),
        Ok(value) => {
            if secs <= MAX_STORABLE_UNIX_SECS {
                Ok(value)
            } else {
                Err(SecsEncodeError::OutsideSupportedRange)
            }
        },
    }
}

/// Why a stored timestamp is corrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecsDecodeError {
    /// Negative.
    Negative,
    /// Beyond `MAX_STORABLE_UNIX_SECS`.
    OutsideSupportedRange,
}

/// Decodes a stored timestamp. Total.
pub fn decode_unix_secs(value: i64) -> (r: Result<u64, SecsDecodeError>)
    ensures
        r matches Ok(s) ==> s as int == value as int,
        r is Ok <==> storable_secs(value as int),
        r matches Err(e) ==> (e == SecsDecodeError::Negative <==> value < 0),
{
    match u64::try_from(value) {
        Err(_) => Err(SecsDecodeError::Negative),
        Ok(secs) => {
            if secs <= MAX_STORABLE_UNIX_SECS {
                Ok(secs)
            } else {
                Err(SecsDecodeError::OutsideSupportedRange)
            }
        },
    }
}

/// Every storable timestamp satisfies the schema's `CHECK (... >= 0)` and
/// fits both the stored `INTEGER` (`i64`) and the decoded `u64`. Together
/// with `encode_unix_secs` and `decode_unix_secs` (each succeeds exactly on
/// `storable_secs` and preserves the value), the codec round-trips both
/// ways and the write domain equals the read domain.
pub proof fn lemma_storable_secs_fit_the_schema(secs: int)
    requires
        storable_secs(secs),
    ensures
        secs >= 0,
        secs <= i64::MAX,
        secs <= u64::MAX,
{
}

// ===========================================================================
// Paths
// ===========================================================================

/// How a stored path text is judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathVerdict {
    /// The canonical stored form.
    Canonical,
    /// The empty string.
    Empty,
    /// Not absolute.
    NotAbsolute,
    /// Absolute, but not in canonical (lexically normal) form.
    NotNormal,
}

/// What the platform contributes to judging a stored path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPlatform {
    /// Unix: `/` is the only root, and the byte model decides everything.
    Unix,
    /// Windows (and other non-Unix targets): whether `std` considers the
    /// path absolute, and whether it is a fixed point of component
    /// resolution there. Paths `std` does not consider absolute may still be
    /// `/`-rooted spellings written on Unix.
    // Only the non-Unix glue constructs it.
    #[cfg_attr(unix, allow(dead_code))]
    Native { absolute: bool, normal: bool },
}

/// The verdict on a stored path's bytes.
pub open spec fn path_verdict(s: Seq<u8>, platform: PathPlatform) -> PathVerdict {
    if s.len() == 0 {
        PathVerdict::Empty
    } else {
        match platform {
            PathPlatform::Unix => {
                if s[0] != 0x2F {
                    PathVerdict::NotAbsolute
                } else if super::path::spec_is_canonical_abs(s, false) {
                    PathVerdict::Canonical
                } else {
                    PathVerdict::NotNormal
                }
            },
            PathPlatform::Native { absolute, normal } => {
                if absolute {
                    if normal {
                        PathVerdict::Canonical
                    } else {
                        PathVerdict::NotNormal
                    }
                } else if s[0] != 0x2F {
                    PathVerdict::NotAbsolute
                } else if super::path::spec_is_canonical_abs(s, true) {
                    PathVerdict::Canonical
                } else {
                    PathVerdict::NotNormal
                }
            },
        }
    }
}

/// A path that may be stored (and is accepted when read back).
pub open spec fn storable_path(s: Seq<u8>, platform: PathPlatform) -> bool {
    path_verdict(s, platform) == PathVerdict::Canonical
}

/// Judges a stored path's bytes. Total.
pub fn classify_stored_path(s: &[u8], platform: PathPlatform) -> (verdict: PathVerdict)
    ensures
        verdict == path_verdict(s@, platform),
{
    if s.is_empty() {
        return PathVerdict::Empty;
    }
    match platform {
        PathPlatform::Unix => {
            if s[0] != b'/' {
                PathVerdict::NotAbsolute
            } else if super::path::is_canonical_abs_path(s, false) {
                PathVerdict::Canonical
            } else {
                PathVerdict::NotNormal
            }
        },
        PathPlatform::Native { absolute, normal } => {
            if absolute {
                if normal {
                    PathVerdict::Canonical
                } else {
                    PathVerdict::NotNormal
                }
            } else if s[0] != b'/' {
                PathVerdict::NotAbsolute
            } else if super::path::is_canonical_abs_path(s, true) {
                PathVerdict::Canonical
            } else {
                PathVerdict::NotNormal
            }
        },
    }
}

/// On Unix the storable paths are exactly the canonical absolute strings,
/// which are exactly the fixed points of normalization (for any working
/// directory).
pub proof fn lemma_unix_storable_is_canonical(s: Seq<u8>, cwd: Seq<u8>)
    ensures
        storable_path(s, PathPlatform::Unix) <==> super::path::spec_is_canonical_abs(s, false),
        storable_path(s, PathPlatform::Unix) <==> super::path::spec_normalize(s, cwd) == Ok::<
            Seq<u8>,
            super::path::NormError,
        >(s),
{
    super::path::lemma_canonical_iff_fixpoint(s, cwd);
    if super::path::spec_is_canonical_abs(s, false) {
        super::path::lemma_canonical_render(s, false);
    }
}

/// Every successful normalization result is storable on Unix, so no key the
/// CLI builds through normalization is ever rejected or later reported as
/// corrupt.
pub proof fn lemma_normalized_is_storable(p: Seq<u8>, cwd: Seq<u8>)
    requires
        super::path::spec_normalize(p, cwd) is Ok,
    ensures
        storable_path(super::path::spec_normalize(p, cwd)->Ok_0, PathPlatform::Unix),
{
    super::path::lemma_normalize_canonical(p, cwd);
    let s = super::path::spec_normalize(p, cwd)->Ok_0;
    super::path::lemma_canonical_render(s, false);
}

/// The `/`-rooted portable form a Windows reader accepts is also storable on
/// Unix, so a database written on Windows with such spellings reads on Unix.
pub proof fn lemma_slash_rooted_storable_on_unix(
    s: Seq<u8>,
    absolute: bool,
    normal: bool,
)
    requires
        !absolute,
        storable_path(s, PathPlatform::Native { absolute, normal }),
    ensures
        storable_path(s, PathPlatform::Unix),
{
    super::path::lemma_canonical_backslash_narrows(s);
}

// ===========================================================================
// Tags
// ===========================================================================

/// How a stored tag text decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagDecode {
    /// `""`: the untagged sentinel.
    Untagged,
    /// Canonical text: the tag itself.
    Tagged,
    /// Anything else (blank or untrimmed non-empty text).
    Corrupt,
}

/// Decodes a stored tag text (`identifier::decode_stored_tag`). Total.
pub fn decode_stored_tag(stored: &str) -> (r: TagDecode)
    ensures
        r == TagDecode::Untagged <==> super::identifier::decode_stored_tag(stored@) == Some(
            None::<Seq<char>>,
        ),
        r == TagDecode::Tagged <==> super::identifier::decode_stored_tag(stored@) == Some(
            Some(stored@),
        ),
        r == TagDecode::Corrupt <==> super::identifier::decode_stored_tag(stored@) is None,
{
    if stored.is_empty() {
        TagDecode::Untagged
    } else if super::identifier::is_canonical_identifier(stored) {
        TagDecode::Tagged
    } else {
        TagDecode::Corrupt
    }
}

/// The UTF-8 bytes of a text (what `str::as_bytes` returns).
pub open spec fn text_bytes(s: &str) -> Seq<u8> {
    vstd::utf8::encode_utf8(s@)
}

/// The view of an optional text.
pub open spec fn opt_chars(o: Option<&str>) -> Option<Seq<char>> {
    match o {
        Some(s) => Some(s@),
        None => None,
    }
}

/// An optional identifier (project, task) is absent or canonical.
pub open spec fn opt_canonical(o: Option<Seq<char>>) -> bool {
    match o {
        Some(s) => super::identifier::is_canonical(s),
        None => true,
    }
}

/// The stored tag texts the decoder accepts are exactly the encodings of
/// valid tags, and decoding inverts encoding: the tag codec is a bijection
/// between valid tags and accepted stored texts.
pub proof fn lemma_tag_codec_bijection(stored: Seq<char>, tag: Option<Seq<char>>)
    ensures
        super::identifier::decode_stored_tag(stored) == Some(tag) <==> (
        super::identifier::is_valid_tag(tag) && super::identifier::encode_stored_tag(tag)
            == stored),
{
    if super::identifier::is_valid_tag(tag) {
        super::identifier::lemma_stored_tag_round_trip(tag);
    }
}

// ===========================================================================
// Storage keys
// ===========================================================================

/// A key that may be written: its path is storable and its tag is absent or
/// canonical.
pub open spec fn storable_key(path: Seq<u8>, platform: PathPlatform, tag: Option<Seq<char>>) -> bool {
    storable_path(path, platform) && super::identifier::is_valid_tag(tag)
}

/// Why a key cannot be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The path is not valid UTF-8 (it has no exact stored text).
    PathNotUtf8,
    /// The path is not storable (never `PathVerdict::Canonical`).
    Path(PathVerdict),
    /// The tag is present but blank or untrimmed.
    TagNotCanonical,
}

/// A key's stored form: the `path` and `tag` columns. The fields are
/// private and only `encode_storage_key` constructs it, so every value is the
/// encoding of a storable key: code outside this module cannot write a
/// hand-built key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredKey<'a> {
    path: &'a str,
    tag: &'a str,
}

impl<'a> StoredKey<'a> {
    /// The stored path.
    pub closed spec fn spec_path(self) -> &'a str {
        self.path
    }

    /// The stored tag.
    pub closed spec fn spec_tag(self) -> &'a str {
        self.tag
    }

    /// The stored path: the key's own text.
    pub fn path(&self) -> (path: &'a str)
        ensures
            path == self.spec_path(),
    {
        self.path
    }

    /// The stored tag: the tag, or `""` for no tag.
    pub fn tag(&self) -> (tag: &'a str)
        ensures
            tag == self.spec_tag(),
    {
        self.tag
    }
}

/// The stored tag column for a tag (exec form of `encode_stored_tag`).
fn encode_tag_text(tag: Option<&str>) -> (stored: &str)
    ensures
        stored@ == super::identifier::encode_stored_tag(opt_chars(tag)),
{
    let Some(t) = tag else {
        proof {
            reveal_strlit("");
        }
        return "";
    };
    t
}

/// The error the write-side key check reports, in the order UTF-8, path,
/// tag; `None` when the key is storable.
pub open spec fn key_error(path: Option<&str>, platform: PathPlatform, tag: Option<&str>) -> Option<
    KeyError,
> {
    match path {
        None => Some(KeyError::PathNotUtf8),
        Some(p) => {
            if !storable_path(text_bytes(p), platform) {
                Some(KeyError::Path(path_verdict(text_bytes(p), platform)))
            } else if !super::identifier::is_valid_tag(opt_chars(tag)) {
                Some(KeyError::TagNotCanonical)
            } else {
                None
            }
        },
    }
}

/// The write-side key check and encoding. `path` is the key path's text
/// (`None` when it is not valid UTF-8), `platform` the platform's facts about
/// it. Total. It succeeds exactly for storable keys; the stored path is then
/// the key's own text, unchanged (no lossy transcoding), and the stored tag
/// is `encode_stored_tag(tag)`. Errors follow the order UTF-8, path, tag.
pub fn encode_storage_key<'a>(path: Option<&'a str>, platform: PathPlatform, tag: Option<&'a str>) -> (r:
    Result<StoredKey<'a>, KeyError>)
    ensures
        r is Ok <==> (path is Some && storable_key(
            text_bytes(path->Some_0),
            platform,
            opt_chars(tag),
        )),
        r matches Ok(k) ==> path == Some(k.spec_path()) && k.spec_tag()@
            == super::identifier::encode_stored_tag(opt_chars(tag)),
        r matches Err(e) ==> key_error(path, platform, tag) == Some(e),
        r is Ok <==> key_error(path, platform, tag) is None,
{
    let Some(p) = path else {
        return Err(KeyError::PathNotUtf8);
    };
    let verdict = classify_stored_path(p.as_bytes(), platform);
    if !matches!(verdict, PathVerdict::Canonical) {
        return Err(KeyError::Path(verdict));
    }
    if let Some(t) = tag {
        if !super::identifier::is_canonical_identifier(t) {
            return Err(KeyError::TagNotCanonical);
        }
    }
    Ok(StoredKey { path: p, tag: encode_tag_text(tag) })
}

/// Read/write agreement for keys: the stored `(path, tag)` columns the
/// decoder accepts are exactly the encodings of storable keys, and the
/// decoded tag is the one that was encoded. Anything trop writes it reads
/// back, and anything it reads back it could have written.
pub proof fn lemma_storable_key_iff_decodes(
    path: Seq<u8>,
    platform: PathPlatform,
    stored_tag: Seq<char>,
    tag: Option<Seq<char>>,
)
    ensures
        (storable_key(path, platform, tag) && super::identifier::encode_stored_tag(tag)
            == stored_tag) <==> (storable_path(path, platform)
            && super::identifier::decode_stored_tag(stored_tag) == Some(tag)),
{
    lemma_tag_codec_bijection(stored_tag, tag);
}

/// Injectivity: storable keys with the same stored columns are the same key.
/// So `DELETE`/`INSERT`/`SELECT ... WHERE path = ? AND tag = ?` with one
/// key's stored form never touch another key's row; in particular a tagged
/// key never aliases the untagged key at the same path.
pub proof fn lemma_stored_key_injective(
    p1: Seq<u8>,
    platform1: PathPlatform,
    t1: Option<Seq<char>>,
    p2: Seq<u8>,
    platform2: PathPlatform,
    t2: Option<Seq<char>>,
)
    requires
        storable_key(p1, platform1, t1),
        storable_key(p2, platform2, t2),
        p1 == p2,
        super::identifier::encode_stored_tag(t1) == super::identifier::encode_stored_tag(t2),
    ensures
        t1 == t2,
{
    super::identifier::lemma_stored_tag_injective(t1, t2);
}

/// On Unix, two storable keys have the same stored columns exactly when they
/// have the same path components and the same tag: the database's text
/// identity agrees with `ReservationKey` equality (`PathBuf` compares
/// components) on everything trop stores. (`/a/b` and `/a/b/` are equal
/// in memory, but only `/a/b` is storable.)
pub proof fn lemma_unix_stored_identity_is_component_identity(
    p1: Seq<u8>,
    t1: Option<Seq<char>>,
    p2: Seq<u8>,
    t2: Option<Seq<char>>,
)
    requires
        storable_key(p1, PathPlatform::Unix, t1),
        storable_key(p2, PathPlatform::Unix, t2),
    ensures
        (p1 == p2 && super::identifier::encode_stored_tag(t1)
            == super::identifier::encode_stored_tag(t2)) <==> (super::path::segs(p1)
            == super::path::segs(p2) && t1 == t2),
{
    super::path::lemma_canonical_injective(p1, p2, false);
    if super::identifier::encode_stored_tag(t1) == super::identifier::encode_stored_tag(t2) {
        super::identifier::lemma_stored_tag_injective(t1, t2);
    }
}

// ===========================================================================
// Lookups
// ===========================================================================

/// The stored columns a lookup by key matches (`WHERE path = ? AND tag = ?`).
/// Unlike `StoredKey`, the key need not be storable: a row stored under a
/// non-storable key's exact text is corrupt, and a lookup that finds it
/// reports the corruption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookupKey<'a> {
    path: &'a str,
    tag: &'a str,
}

impl<'a> LookupKey<'a> {
    /// The `path` column matched.
    pub closed spec fn spec_path(self) -> &'a str {
        self.path
    }

    /// The `tag` column matched.
    pub closed spec fn spec_tag(self) -> &'a str {
        self.tag
    }

    /// The `path` column matched: the key's own text.
    pub fn path(&self) -> (path: &'a str)
        ensures
            path == self.spec_path(),
    {
        self.path
    }

    /// The `tag` column matched: the tag, or `""` for no tag.
    pub fn tag(&self) -> (tag: &'a str)
        ensures
            tag == self.spec_tag(),
    {
        self.tag
    }
}

/// A row whose `path` and `tag` columns hold `stored_path` and `stored_tag`
/// is matched by the lookup `k` (none matches nothing).
pub open spec fn lookup_matches(k: Option<LookupKey>, stored_path: Seq<char>, stored_tag: Seq<char>) -> bool {
    match k {
        Some(k) => k.spec_path()@ == stored_path && k.spec_tag()@ == stored_tag,
        None => false,
    }
}

/// A row whose `path` and `tag` columns hold `stored_path` and `stored_tag`
/// decodes to the key `(path, tag)` (as far as the key columns go).
pub open spec fn holds_key(
    stored_path: Seq<char>,
    stored_tag: Seq<char>,
    path: Seq<char>,
    tag: Option<Seq<char>>,
) -> bool {
    stored_path == path && super::identifier::decode_stored_tag(stored_tag) == Some(tag)
}

/// The lookup for the key `(path, tag)`: `path` is the key path's text
/// (`None` when it is not valid UTF-8), `tag` its tag. Total.
///
/// Exactness: among rows whose tag column decodes, the lookup matches
/// exactly the rows holding the requested key, so it never returns another
/// key's row (a `Some("")` tag never aliases the untagged sentinel) and
/// never misses the key's own row. There is no lookup (`None`) exactly when
/// no row can hold the key: the path has no exact text, or the tag is
/// `Some("")`, which no stored tag decodes to. Otherwise the lookup is the
/// key's exact text, storable or not, so a corrupt row stored under that text
/// is still found and reported.
pub fn lookup_key<'a>(path: Option<&'a str>, tag: Option<&'a str>) -> (r: Option<LookupKey<'a>>)
    ensures
        r is None <==> (path is None || opt_chars(tag) == Some(Seq::<char>::empty())),
        r matches Some(k) ==> path == Some(k.spec_path()) && k.spec_tag()@
            == super::identifier::encode_stored_tag(opt_chars(tag)),
        forall|stored_path: Seq<char>, stored_tag: Seq<char>|
            super::identifier::decode_stored_tag(stored_tag) is Some ==> (#[trigger] lookup_matches(
                r,
                stored_path,
                stored_tag,
            ) <==> (path is Some && holds_key(
                stored_path,
                stored_tag,
                path->Some_0@,
                opt_chars(tag),
            ))),
{
    let Some(p) = path else {
        proof {
            assert forall|stored_path: Seq<char>, stored_tag: Seq<char>|
                super::identifier::decode_stored_tag(stored_tag) is Some implies (
                #[trigger] lookup_matches(None, stored_path, stored_tag) <==> (path is Some
                    && holds_key(stored_path, stored_tag, path->Some_0@, opt_chars(tag)))) by {}
        }
        return None;
    };
    if let Some(t) = tag {
        if t.is_empty() {
            proof {
                assert forall|stored_path: Seq<char>, stored_tag: Seq<char>|
                    super::identifier::decode_stored_tag(stored_tag) is Some implies (
                    #[trigger] lookup_matches(None, stored_path, stored_tag) <==> (path is Some
                        && holds_key(stored_path, stored_tag, path->Some_0@, opt_chars(tag)))) by {
                    // No stored tag decodes to `Some("")`: `""` is untagged.
                    if super::identifier::decode_stored_tag(stored_tag) == Some(opt_chars(tag)) {
                        assert(stored_tag.len() > 0);
                    }
                }
            }
            return None;
        }
    }
    let k = LookupKey { path: p, tag: encode_tag_text(tag) };
    proof {
        assert forall|stored_path: Seq<char>, stored_tag: Seq<char>|
            super::identifier::decode_stored_tag(stored_tag) is Some implies (
            #[trigger] lookup_matches(Some(k), stored_path, stored_tag) <==> (path is Some
                && holds_key(stored_path, stored_tag, path->Some_0@, opt_chars(tag)))) by {
            lemma_tag_codec_bijection(stored_tag, opt_chars(tag));
        }
    }
    Some(k)
}

// ===========================================================================
// Whole rows
// ===========================================================================

/// A reservation's persisted fields as the write path hands them over.
/// `sticky` is deliberately absent: it is not persisted.
#[derive(Clone, Copy, Debug)]
pub struct RecordFields<'a> {
    /// The key path's text (`None` when it is not valid UTF-8).
    pub path: Option<&'a str>,
    /// The platform's facts about the key path.
    pub platform: PathPlatform,
    /// The key's tag.
    pub tag: Option<&'a str>,
    /// The port.
    pub port: u16,
    /// The project identifier.
    pub project: Option<&'a str>,
    /// The task identifier.
    pub task: Option<&'a str>,
    /// Creation time, whole seconds since the epoch.
    pub created_secs: u64,
    /// Last use, whole seconds since the epoch.
    pub last_used_secs: u64,
}

/// A schema-v2 `reservations` row with correctly typed SQL values, as the
/// write path produces it. The fields are private and only `encode_record`
/// constructs it, so every value is the encoding of a storable record: code
/// outside this module cannot write a hand-built row.
#[derive(Clone, Copy, Debug)]
pub struct StoredRow<'a> {
    path: &'a str,
    tag: &'a str,
    port: i64,
    project: Option<&'a str>,
    task: Option<&'a str>,
    created_at: i64,
    last_used_at: i64,
}

impl<'a> StoredRow<'a> {
    /// `path`.
    pub closed spec fn spec_path(self) -> &'a str {
        self.path
    }

    /// `tag`.
    pub closed spec fn spec_tag(self) -> &'a str {
        self.tag
    }

    /// `port`.
    pub closed spec fn spec_port(self) -> i64 {
        self.port
    }

    /// `project`.
    pub closed spec fn spec_project(self) -> Option<&'a str> {
        self.project
    }

    /// `task`.
    pub closed spec fn spec_task(self) -> Option<&'a str> {
        self.task
    }

    /// `created_at`.
    pub closed spec fn spec_created_at(self) -> i64 {
        self.created_at
    }

    /// `last_used_at`.
    pub closed spec fn spec_last_used_at(self) -> i64 {
        self.last_used_at
    }

    /// `path` (TEXT).
    pub fn path(&self) -> (path: &'a str)
        ensures
            path == self.spec_path(),
    {
        self.path
    }

    /// `tag` (TEXT, `""` for no tag).
    pub fn tag(&self) -> (tag: &'a str)
        ensures
            tag == self.spec_tag(),
    {
        self.tag
    }

    /// `port` (INTEGER).
    pub fn port(&self) -> (port: i64)
        ensures
            port == self.spec_port(),
    {
        self.port
    }

    /// `project` (TEXT or NULL).
    pub fn project(&self) -> (project: Option<&'a str>)
        ensures
            project == self.spec_project(),
    {
        self.project
    }

    /// `task` (TEXT or NULL).
    pub fn task(&self) -> (task: Option<&'a str>)
        ensures
            task == self.spec_task(),
    {
        self.task
    }

    /// `created_at` (INTEGER).
    pub fn created_at(&self) -> (created_at: i64)
        ensures
            created_at == self.spec_created_at(),
    {
        self.created_at
    }

    /// `last_used_at` (INTEGER).
    pub fn last_used_at(&self) -> (last_used_at: i64)
        ensures
            last_used_at == self.spec_last_used_at(),
    {
        self.last_used_at
    }
}

/// Why a reservation cannot be written, in the order the fields are checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// The key cannot be stored.
    Key(KeyError),
    /// Port 0 (unreachable through `Port`).
    InvalidPort,
    /// The project is present but blank or untrimmed.
    ProjectNotCanonical,
    /// The task is present but blank or untrimmed.
    TaskNotCanonical,
    /// The creation time cannot be stored.
    CreatedAt(SecsEncodeError),
    /// The last-use time cannot be stored.
    LastUsedAt(SecsEncodeError),
}

/// A reservation that may be written.
pub open spec fn record_storable(f: RecordFields) -> bool {
    &&& f.path is Some
    &&& storable_key(text_bytes(f.path->Some_0), f.platform, opt_chars(f.tag))
    &&& super::port::is_valid_port(f.port as int)
    &&& opt_canonical(opt_chars(f.project))
    &&& opt_canonical(opt_chars(f.task))
    &&& storable_secs(f.created_secs as int)
    &&& storable_secs(f.last_used_secs as int)
}

/// The first check a record fails, in the field order the write path checks
/// them: the key (UTF-8, path, tag), port, project, task, creation time, last
/// use; `None` when the record is storable.
pub open spec fn first_record_error(f: RecordFields) -> Option<RecordError> {
    match key_error(f.path, f.platform, f.tag) {
        Some(e) => Some(RecordError::Key(e)),
        None => if f.port == 0 {
            Some(RecordError::InvalidPort)
        } else if !opt_canonical(opt_chars(f.project)) {
            Some(RecordError::ProjectNotCanonical)
        } else if !opt_canonical(opt_chars(f.task)) {
            Some(RecordError::TaskNotCanonical)
        } else {
            match secs_error(f.created_secs) {
                Some(e) => Some(RecordError::CreatedAt(e)),
                None => match secs_error(f.last_used_secs) {
                    Some(e) => Some(RecordError::LastUsedAt(e)),
                    None => None,
                },
            }
        },
    }
}

/// `row` is the stored form of `f`.
pub open spec fn encodes(f: RecordFields, row: StoredRow) -> bool {
    &&& f.path == Some(row.spec_path())
    &&& row.spec_tag()@ == super::identifier::encode_stored_tag(opt_chars(f.tag))
    &&& row.spec_port() as int == f.port as int
    &&& row.spec_project() == f.project
    &&& row.spec_task() == f.task
    &&& row.spec_created_at() as int == f.created_secs as int
    &&& row.spec_last_used_at() as int == f.last_used_secs as int
}

/// What `row_to_reservation` accepts once every column has its SQL type,
/// stated on the column values: a storable path, a decodable tag, a valid
/// port, canonical optional identifiers and storable timestamps.
pub open spec fn values_decode(
    path: Seq<u8>,
    platform: PathPlatform,
    tag: Seq<char>,
    port: int,
    project: Option<Seq<char>>,
    task: Option<Seq<char>>,
    created_at: int,
    last_used_at: int,
) -> bool {
    &&& storable_path(path, platform)
    &&& super::identifier::decode_stored_tag(tag) is Some
    &&& super::port::is_valid_port(port)
    &&& opt_canonical(project)
    &&& opt_canonical(task)
    &&& storable_secs(created_at)
    &&& storable_secs(last_used_at)
}

/// What `row_to_reservation` accepts for a correctly typed row.
pub open spec fn row_decodes(row: StoredRow, platform: PathPlatform) -> bool {
    values_decode(
        text_bytes(row.spec_path()),
        platform,
        row.spec_tag()@,
        row.spec_port() as int,
        opt_chars(row.spec_project()),
        opt_chars(row.spec_task()),
        row.spec_created_at() as int,
        row.spec_last_used_at() as int,
    )
}

/// The tag a decodable row decodes to.
pub open spec fn decoded_tag(row: StoredRow) -> Option<Seq<char>> {
    super::identifier::decode_stored_tag(row.spec_tag()@)->Some_0
}

/// The write path for one reservation: checks every persisted field and
/// returns the row to insert. Total. It succeeds exactly for storable
/// records, the row is their encoding, and an error is the first failing
/// check in the field order path, tag, port, project, task, created, last
/// used (`first_record_error`). (The glue rejects a timestamp before the
/// Unix epoch, which has no `u64` seconds, before calling this.)
pub fn encode_record<'a>(f: &RecordFields<'a>) -> (r: Result<StoredRow<'a>, RecordError>)
    ensures
        r is Ok <==> record_storable(*f),
        r is Ok <==> first_record_error(*f) is None,
        r matches Ok(row) ==> encodes(*f, row),
        r matches Err(e) ==> first_record_error(*f) == Some(e),
{
    let key = match encode_storage_key(f.path, f.platform, f.tag) {
        Ok(key) => key,
        Err(e) => return Err(RecordError::Key(e)),
    };
    if f.port == 0 {
        return Err(RecordError::InvalidPort);
    }
    if let Some(project) = f.project {
        if !super::identifier::is_canonical_identifier(project) {
            return Err(RecordError::ProjectNotCanonical);
        }
    }
    if let Some(task) = f.task {
        if !super::identifier::is_canonical_identifier(task) {
            return Err(RecordError::TaskNotCanonical);
        }
    }
    let created_at = match encode_unix_secs(f.created_secs) {
        Ok(v) => v,
        Err(e) => return Err(RecordError::CreatedAt(e)),
    };
    let last_used_at = match encode_unix_secs(f.last_used_secs) {
        Ok(v) => v,
        Err(e) => return Err(RecordError::LastUsedAt(e)),
    };
    Ok(
        StoredRow {
            path: key.path,
            tag: key.tag,
            port: i64::from(f.port),
            project: f.project,
            task: f.task,
            created_at,
            last_used_at,
        },
    )
}

/// Round trip, write then read: the row written for a storable record is
/// accepted by the decoder and decodes to exactly the record's fields (the
/// path text, the tag, port, project, task and whole-second timestamps).
/// The `sticky` flag is not among the persisted fields, so a reservation
/// reads back with `sticky() == false`.
pub proof fn lemma_encoded_record_round_trips(f: RecordFields, row: StoredRow)
    requires
        record_storable(f),
        encodes(f, row),
    ensures
        row_decodes(row, f.platform),
        decoded_tag(row) == opt_chars(f.tag),
        Some(row.spec_path()) == f.path,
        row.spec_port() as int == f.port as int,
        opt_chars(row.spec_project()) == opt_chars(f.project),
        opt_chars(row.spec_task()) == opt_chars(f.task),
        row.spec_created_at() as int == f.created_secs as int,
        row.spec_last_used_at() as int == f.last_used_secs as int,
{
    super::identifier::lemma_stored_tag_round_trip(opt_chars(f.tag));
}

/// Read then write: a row the decoder accepts has a storable key, and its
/// tag re-encodes to the stored tag text (the tag codec bijection). The
/// port and timestamp facts restate `row_decodes`. This lemma does not state
/// `encodes(f, row)` or `record_storable(f)` for a decoded record `f`, and
/// says nothing about project and task beyond `row_decodes`.
pub proof fn lemma_decoded_row_re_encodes(row: StoredRow, platform: PathPlatform)
    requires
        row_decodes(row, platform),
    ensures
        storable_key(text_bytes(row.spec_path()), platform, decoded_tag(row)),
        super::identifier::encode_stored_tag(decoded_tag(row)) == row.spec_tag()@,
        super::port::is_valid_port(row.spec_port() as int),
        storable_secs(row.spec_created_at() as int),
        storable_secs(row.spec_last_used_at() as int),
{
    lemma_tag_codec_bijection(row.spec_tag()@, decoded_tag(row));
}

// ===========================================================================
// SQL cell values (storage classes)
// ===========================================================================

/// An SQLite storage class (`typeof`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlType {
    /// `NULL`.
    Null,
    /// `INTEGER`.
    Integer,
    /// `REAL`.
    Real,
    /// `TEXT`.
    Text,
    /// `BLOB`.
    Blob,
}

/// One stored value as the decoders see it. `Text` is valid UTF-8 text;
/// `InvalidText` is a `TEXT` value that is not valid UTF-8.
#[derive(Clone, Copy, Debug)]
pub enum Cell<'a> {
    /// `NULL`.
    Null,
    /// An `INTEGER`.
    Integer(i64),
    /// A `REAL`.
    Real,
    /// Valid UTF-8 `TEXT`.
    Text(&'a str),
    /// `TEXT` that is not valid UTF-8.
    InvalidText,
    /// A `BLOB`.
    Blob,
}

/// The storage class of a cell (`typeof`).
pub open spec fn sql_type(c: Cell) -> SqlType {
    match c {
        Cell::Null => SqlType::Null,
        Cell::Integer(_) => SqlType::Integer,
        Cell::Real => SqlType::Real,
        Cell::Text(_) | Cell::InvalidText => SqlType::Text,
        Cell::Blob => SqlType::Blob,
    }
}

/// The storage class of a cell (`typeof`). Total.
pub fn cell_type(c: Cell) -> (t: SqlType)
    ensures
        t == sql_type(c),
{
    match c {
        Cell::Null => SqlType::Null,
        Cell::Integer(_) => SqlType::Integer,
        Cell::Real => SqlType::Real,
        Cell::Text(_) | Cell::InvalidText => SqlType::Text,
        Cell::Blob => SqlType::Blob,
    }
}

// ===========================================================================
// Prefix queries
// ===========================================================================

/// The `std` model of `Path::starts_with(prefix)` for a stored canonical
/// path `s` on Unix: the empty prefix has no components and matches
/// everything; any other relative prefix starts with a non-root component and
/// matches no absolute path; an absolute prefix matches when its components
/// after the root (`segs`, which drops empty and `.` pieces but keeps `..`)
/// are a prefix of the path's.
pub open spec fn starts_with_model(s: Seq<u8>, prefix: Seq<u8>) -> bool {
    if prefix.len() == 0 {
        true
    } else if !super::path::is_abs(prefix) {
        false
    } else {
        super::path_relation::is_prefix_of(super::path::segs(prefix), super::path::segs(s))
    }
}

/// What a prefix query selects.
// Only the Unix glue runs prefix queries through the kernel.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrefixQuery {
    /// Every stored path.
    Everything,
    /// No stored path.
    Nothing,
    /// The stored paths that have this canonical key as a component prefix.
    Under(Vec<u8>),
}

/// The stored canonical path `s` is selected by the query `q`.
pub open spec fn query_selects(q: PrefixQuery, s: Seq<u8>) -> bool {
    match q {
        PrefixQuery::Everything => true,
        PrefixQuery::Nothing => false,
        PrefixQuery::Under(key) => super::path_relation::spec_component_prefix(key@, s),
    }
}

/// The query for `Path::starts_with(prefix)` over stored paths. Total.
// Only the Unix glue calls this byte-level entry point.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn prefix_query(prefix: &[u8]) -> (q: PrefixQuery)
    ensures
        q matches PrefixQuery::Under(key) ==> super::path::spec_is_canonical_abs(key@, false)
            && super::path::segs(key@) == super::path::segs(prefix@),
        forall|s: Seq<u8>|
            super::path::spec_is_canonical_abs(s, false) ==> (#[trigger] starts_with_model(
                s,
                prefix@,
            ) <==> query_selects(q, s)),
{
    if prefix.is_empty() {
        return PrefixQuery::Everything;
    }
    if prefix[0] != b'/' {
        return PrefixQuery::Nothing;
    }
    if super::path::has_parent_segment(prefix) {
        proof {
            let i = choose|i: int|
                0 <= i < super::path::segs(prefix@).len() && super::path::is_parent_seg(
                    #[trigger] super::path::segs(prefix@)[i],
                );
            assert forall|s: Seq<u8>|
                super::path::spec_is_canonical_abs(s, false) implies !#[trigger] starts_with_model(
                s,
                prefix@,
            ) by {
                super::path::lemma_canonical_render(s, false);
                if starts_with_model(s, prefix@) {
                    assert(super::path::segs(s)[i] == super::path::segs(prefix@)[i]);
                    assert(super::path::is_normal_seg(super::path::segs(s)[i]));
                }
            }
        }
        return PrefixQuery::Nothing;
    }
    // Normalizing a `..`-free absolute path only re-spells it (it never
    // fails), and its result is the canonical key.
    let Ok(key) = super::path::normalize_unix(prefix, prefix) else {
        proof {
            super::path::lemma_parent_free_segs(prefix@);
        }
        return PrefixQuery::Nothing;
    };
    proof {
        lemma_prefix_key(prefix@, key@);
    }
    PrefixQuery::Under(key)
}

/// The canonical key of a `..`-free absolute prefix has the prefix's
/// segments, and selecting by it is exactly the `starts_with` model.
proof fn lemma_prefix_key(prefix: Seq<u8>, key: Seq<u8>)
    requires
        super::path::is_abs(prefix),
        !super::path::has_parent_seg_spec(super::path::segs(prefix)),
        super::path::spec_normalize(prefix, prefix) == Ok::<Seq<u8>, super::path::NormError>(key),
    ensures
        super::path::spec_is_canonical_abs(key, false),
        super::path::segs(key) == super::path::segs(prefix),
        forall|s: Seq<u8>|
            super::path::spec_is_canonical_abs(s, false) ==> (#[trigger] starts_with_model(
                s,
                prefix,
            ) <==> super::path_relation::spec_component_prefix(key, s)),
{
    super::path::lemma_parent_free_segs(prefix);
    super::path::lemma_normalize_canonical(prefix, prefix);
    assert forall|s: Seq<u8>| super::path::spec_is_canonical_abs(s, false) implies (
    #[trigger] starts_with_model(s, prefix) <==> super::path_relation::spec_component_prefix(
        key,
        s,
    )) by {
        super::path_relation::lemma_component_prefix_iff_segs(key, s);
    }
}

/// Every path an `Under(key)` query selects starts with the key's bytes.
/// This holds by the definition of `spec_component_prefix` (the proof is
/// empty). Together with the SQL prefilter `instr(path, key) = 1` (trusted:
/// true exactly when the stored text's UTF-8 bytes start with the key's
/// bytes; no wildcards, case folding, NUL truncation or pattern-length
/// limit), the prefilter never drops a selected path. That `query_selects`
/// is exactly the `starts_with` model on stored canonical paths is
/// `prefix_query`'s postcondition, not this lemma. The post-filter is
/// `canonical_is_component_prefix`.
pub proof fn lemma_prefix_query_exact(q: PrefixQuery, s: Seq<u8>)
    requires
        query_selects(q, s),
    ensures
        q matches PrefixQuery::Under(key) ==> super::path_relation::is_prefix_of(key@, s),
{
}

// ===========================================================================
// Connection settings
// ===========================================================================

/// The busy timeout `PRAGMA busy_timeout` reports, as a non-negative number
/// of milliseconds: SQLite treats negative timeouts as zero. Total.
pub fn busy_timeout_millis(reported: i64) -> (millis: u64)
    ensures
        millis as int == if reported < 0 {
            0
        } else {
            reported as int
        },
{
    u64::try_from(reported).unwrap_or(0)
}

} // verus!
