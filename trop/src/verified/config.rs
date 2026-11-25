//! Verified kernel: configuration parsing, validation, layering, placement,
//! and discovery decisions.
//!
//! - Environment-variable text: unsigned decimal numbers (one parser for
//!   every numeric `TROP_*` variable, deciding the kernel's decimal syntax;
//!   that this is the syntax of the unsigned `from_str` implementations is
//!   trusted and pinned by differential tests), boolean words, and
//!   output-format keywords. YAML numbers are parsed by `serde_yaml`.
//! - The `reservations` group rules (`check_config_group`): accepted exactly
//!   when every service is well formed and tags, defaulted offsets, preferred
//!   ports, and resolved environment-variable names are pairwise distinct;
//!   acceptance does not depend on the order of the services, and the
//!   reported error is the first in the order given.
//! - Database lock-wait bounds: the configured seconds and a `Duration`'s
//!   milliseconds against `SQLite`'s signed 32-bit millisecond limit.
//! - Built-in defaults, boolean flags, and the permission derivation.
//! - Layering: the per-leaf overlay of configuration layers (exec), lemmas
//!   about spec-only models of its fold, of accumulated exclusions and of
//!   the reservation-group overlay (no exec code is checked against these
//!   models), file placement rules, the choice of the project-file level
//!   over an existence oracle, and the load order of that level's files.
//!
//! Production glue: `crate::config` (`environment`, `validator`, `merger`,
//! `effective`, `builder`, `loader`, `schema`) and `crate::database::config`.

use super::env_name;
use super::exclusions::parse_unsigned;
use super::identifier;
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Unsigned decimal numbers
// ---------------------------------------------------------------------------

/// Parses a `u32` in the kernel's unsigned decimal syntax and range
/// (`is_unsigned_text` with `u32::MAX`). That this is `u32::from_str`'s
/// syntax is trusted, pinned by differential tests.
pub fn parse_u32_decimal(s: &[u8]) -> (r: Option<u32>)
    ensures
        r matches Some(v) ==> super::exclusions::is_unsigned_text(s@, v as int, u32::MAX as int),
        r is None ==> forall|v: int| !super::exclusions::is_unsigned_text(s@, v, u32::MAX as int),
{
    match parse_unsigned(s, 0xFFFF_FFFF) {
        Some(v) => u32::try_from(v).ok(),
        None => None,
    }
}

/// Parses a `u64` in the kernel's unsigned decimal syntax and range
/// (`is_unsigned_text` with `u64::MAX`). That this is `u64::from_str`'s
/// syntax is trusted, pinned by differential tests.
pub fn parse_u64_decimal(s: &[u8]) -> (r: Option<u64>)
    ensures
        r matches Some(v) ==> super::exclusions::is_unsigned_text(s@, v as int, u64::MAX as int),
        r is None ==> forall|v: int| !super::exclusions::is_unsigned_text(s@, v, u64::MAX as int),
{
    parse_unsigned(s, 0xFFFF_FFFF_FFFF_FFFF)
}

/// `k` ASCII zeros.
pub open spec fn zeros(k: nat) -> Seq<u8> {
    Seq::new(k, |i: int| 48u8)
}

/// Leading zeros do not change the value of a digit string.
proof fn lemma_leading_zeros_value(k: nat, t: Seq<u8>)
    ensures
        super::exclusions::decimal_value(zeros(k) + t) == super::exclusions::decimal_value(t),
    decreases t.len(), k,
{
    if t.len() == 0 {
        assert(zeros(k) + t =~= zeros(k));
        if k > 0 {
            assert(zeros(k).drop_last() =~= zeros((k - 1) as nat));
            lemma_leading_zeros_value((k - 1) as nat, t);
            assert(zeros((k - 1) as nat) + t =~= zeros((k - 1) as nat));
        }
    } else {
        assert((zeros(k) + t).drop_last() =~= zeros(k) + t.drop_last());
        assert((zeros(k) + t).last() == t.last());
        lemma_leading_zeros_value(k, t.drop_last());
    }
}

/// Completeness: every value up to `max`, written in decimal with any number
/// of leading zeros and an optional `+`, is accepted with exactly that value.
pub proof fn lemma_decimal_text_accepted(n: nat, k: nat, max: int)
    requires
        n <= max,
    ensures
        super::exclusions::is_unsigned_text(
            zeros(k) + super::exclusions::decimal_text(n),
            n as int,
            max,
        ),
        super::exclusions::is_unsigned_text(
            seq![43u8] + zeros(k) + super::exclusions::decimal_text(n),
            n as int,
            max,
        ),
{
    let t = super::exclusions::decimal_text(n);
    super::exclusions::lemma_decimal_text(n);
    lemma_leading_zeros_value(k, t);
    let z = zeros(k) + t;
    assert(super::exclusions::all_digits(z)) by {
        assert forall|i: int| 0 <= i < z.len() implies super::exclusions::is_digit(
            #[trigger] z[i],
        ) by {
            if i >= k {
                assert(z[i] == t[i - k]);
            }
        }
    }
    assert(super::exclusions::unsigned_digits(z) == z) by {
        assert(super::exclusions::is_digit(z[0]));
    }
    let p = seq![43u8] + z;
    assert(seq![43u8] + zeros(k) + t =~= p);
    assert(super::exclusions::unsigned_digits(p) =~= z);
}

// ---------------------------------------------------------------------------
// Boolean words
// ---------------------------------------------------------------------------

/// `"true"`.
pub open spec fn word_true() -> Seq<u8> {
    seq![0x74u8, 0x72u8, 0x75u8, 0x65u8]
}

/// `"1"`.
pub open spec fn word_one() -> Seq<u8> {
    seq![0x31u8]
}

/// `"yes"`.
pub open spec fn word_yes() -> Seq<u8> {
    seq![0x79u8, 0x65u8, 0x73u8]
}

/// `"on"`.
pub open spec fn word_on() -> Seq<u8> {
    seq![0x6Fu8, 0x6Eu8]
}

/// `"false"`.
pub open spec fn word_false() -> Seq<u8> {
    seq![0x66u8, 0x61u8, 0x6Cu8, 0x73u8, 0x65u8]
}

/// `"0"`.
pub open spec fn word_zero() -> Seq<u8> {
    seq![0x30u8]
}

/// `"no"`.
pub open spec fn word_no() -> Seq<u8> {
    seq![0x6Eu8, 0x6Fu8]
}

/// `"off"`.
pub open spec fn word_off() -> Seq<u8> {
    seq![0x6Fu8, 0x66u8, 0x66u8]
}

/// `s` is `true`, `1`, `yes`, or `on`, ignoring ASCII case.
pub open spec fn is_true_word(s: Seq<u8>) -> bool {
    ||| super::text::eq_ignore_ascii_case(s, word_true())
    ||| super::text::eq_ignore_ascii_case(s, word_one())
    ||| super::text::eq_ignore_ascii_case(s, word_yes())
    ||| super::text::eq_ignore_ascii_case(s, word_on())
}

/// `s` is `false`, `0`, `no`, or `off`, ignoring ASCII case.
pub open spec fn is_false_word(s: Seq<u8>) -> bool {
    ||| super::text::eq_ignore_ascii_case(s, word_false())
    ||| super::text::eq_ignore_ascii_case(s, word_zero())
    ||| super::text::eq_ignore_ascii_case(s, word_no())
    ||| super::text::eq_ignore_ascii_case(s, word_off())
}

/// The boolean a word denotes, if any.
pub open spec fn bool_word_value(s: Seq<u8>) -> Option<bool> {
    if is_true_word(s) {
        Some(true)
    } else if is_false_word(s) {
        Some(false)
    } else {
        None
    }
}

/// ASCII uppercase of one byte (`u8::to_ascii_uppercase`).
const fn fold_byte(b: u8) -> (r: u8)
    ensures
        r == super::text::ascii_upper(b),
{
    if b'a' <= b && b <= b'z' {
        b - 0x20
    } else {
        b
    }
}

/// Byte-wise case-insensitive equality with a 1-byte word.
proof fn lemma_eq_len1(s: Seq<u8>, w: Seq<u8>)
    requires
        s.len() == 1,
        w.len() == 1,
    ensures
        super::text::eq_ignore_ascii_case(s, w) <==> super::text::ascii_upper(s[0])
            == super::text::ascii_upper(w[0]),
{
}

/// Byte-wise case-insensitive equality with a 2-byte word.
proof fn lemma_eq_len2(s: Seq<u8>, w: Seq<u8>)
    requires
        s.len() == 2,
        w.len() == 2,
    ensures
        super::text::eq_ignore_ascii_case(s, w) <==> (super::text::ascii_upper(s[0])
            == super::text::ascii_upper(w[0]) && super::text::ascii_upper(s[1])
            == super::text::ascii_upper(w[1])),
{
}

/// Byte-wise case-insensitive equality with a 3-byte word.
proof fn lemma_eq_len3(s: Seq<u8>, w: Seq<u8>)
    requires
        s.len() == 3,
        w.len() == 3,
    ensures
        super::text::eq_ignore_ascii_case(s, w) <==> (super::text::ascii_upper(s[0])
            == super::text::ascii_upper(w[0]) && super::text::ascii_upper(s[1])
            == super::text::ascii_upper(w[1]) && super::text::ascii_upper(s[2])
            == super::text::ascii_upper(w[2])),
{
}

/// Byte-wise case-insensitive equality with a 4-byte word.
proof fn lemma_eq_len4(s: Seq<u8>, w: Seq<u8>)
    requires
        s.len() == 4,
        w.len() == 4,
    ensures
        super::text::eq_ignore_ascii_case(s, w) <==> (super::text::ascii_upper(s[0])
            == super::text::ascii_upper(w[0]) && super::text::ascii_upper(s[1])
            == super::text::ascii_upper(w[1]) && super::text::ascii_upper(s[2])
            == super::text::ascii_upper(w[2]) && super::text::ascii_upper(s[3])
            == super::text::ascii_upper(w[3])),
{
}

/// Byte-wise case-insensitive equality with a 5-byte word.
proof fn lemma_eq_len5(s: Seq<u8>, w: Seq<u8>)
    requires
        s.len() == 5,
        w.len() == 5,
    ensures
        super::text::eq_ignore_ascii_case(s, w) <==> (super::text::ascii_upper(s[0])
            == super::text::ascii_upper(w[0]) && super::text::ascii_upper(s[1])
            == super::text::ascii_upper(w[1]) && super::text::ascii_upper(s[2])
            == super::text::ascii_upper(w[2]) && super::text::ascii_upper(s[3])
            == super::text::ascii_upper(w[3]) && super::text::ascii_upper(s[4])
            == super::text::ascii_upper(w[4])),
{
}

/// Parses a boolean word: `true`, `1`, `yes`, `on` are `true`; `false`, `0`,
/// `no`, `off` are `false` (ASCII case ignored); anything else, including
/// surrounding whitespace, is rejected.
pub fn parse_bool_word(s: &[u8]) -> (r: Option<bool>)
    ensures
        r == bool_word_value(s@),
{
    proof {
        lemma_bool_word_lengths(s@);
    }
    match s.len() {
        1 => {
            proof {
                lemma_eq_len1(s@, word_one());
                lemma_eq_len1(s@, word_zero());
            }
            match fold_byte(s[0]) {
                b'1' => Some(true),
                b'0' => Some(false),
                _ => None,
            }
        },
        2 => {
            proof {
                lemma_eq_len2(s@, word_on());
                lemma_eq_len2(s@, word_no());
            }
            match (fold_byte(s[0]), fold_byte(s[1])) {
                (b'O', b'N') => Some(true),
                (b'N', b'O') => Some(false),
                _ => None,
            }
        },
        3 => {
            proof {
                lemma_eq_len3(s@, word_yes());
                lemma_eq_len3(s@, word_off());
            }
            match (fold_byte(s[0]), fold_byte(s[1]), fold_byte(s[2])) {
                (b'Y', b'E', b'S') => Some(true),
                (b'O', b'F', b'F') => Some(false),
                _ => None,
            }
        },
        4 => {
            proof {
                lemma_eq_len4(s@, word_true());
            }
            match (fold_byte(s[0]), fold_byte(s[1]), fold_byte(s[2]), fold_byte(s[3])) {
                (b'T', b'R', b'U', b'E') => Some(true),
                _ => None,
            }
        },
        5 => {
            proof {
                lemma_eq_len5(s@, word_false());
            }
            match (
                fold_byte(s[0]),
                fold_byte(s[1]),
                fold_byte(s[2]),
                fold_byte(s[3]),
                fold_byte(s[4]),
            ) {
                (b'F', b'A', b'L', b'S', b'E') => Some(false),
                _ => None,
            }
        },
        _ => None,
    }
}

/// A word that matches a boolean word has that word's length.
proof fn lemma_bool_word_lengths(s: Seq<u8>)
    ensures
        super::text::eq_ignore_ascii_case(s, word_true()) ==> s.len() == 4,
        super::text::eq_ignore_ascii_case(s, word_one()) ==> s.len() == 1,
        super::text::eq_ignore_ascii_case(s, word_yes()) ==> s.len() == 3,
        super::text::eq_ignore_ascii_case(s, word_on()) ==> s.len() == 2,
        super::text::eq_ignore_ascii_case(s, word_false()) ==> s.len() == 5,
        super::text::eq_ignore_ascii_case(s, word_zero()) ==> s.len() == 1,
        super::text::eq_ignore_ascii_case(s, word_no()) ==> s.len() == 2,
        super::text::eq_ignore_ascii_case(s, word_off()) ==> s.len() == 3,
{
}

/// `eq_ignore_ascii_case` is an equivalence: it relates exactly the strings
/// with the same ASCII-uppercase key.
proof fn lemma_eq_ignore_case_trans(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    requires
        super::text::eq_ignore_ascii_case(a, b),
    ensures
        super::text::eq_ignore_ascii_case(a, c) == super::text::eq_ignore_ascii_case(b, c),
{
    super::text::lemma_ascii_upper_seq_eq_iff(a, b);
    super::text::lemma_ascii_upper_seq_eq_iff(a, c);
    super::text::lemma_ascii_upper_seq_eq_iff(b, c);
}

/// The two word lists are disjoint, so the parse is a function; and the
/// result depends only on the text up to ASCII case.
pub proof fn lemma_bool_words(s: Seq<u8>, t: Seq<u8>)
    ensures
        !(is_true_word(s) && is_false_word(s)),
        super::text::eq_ignore_ascii_case(s, t) ==> bool_word_value(s) == bool_word_value(t),
        bool_word_value(word_true()) == Some(true),
        bool_word_value(word_false()) == Some(false),
{
    lemma_bool_word_lengths(s);
    if is_true_word(s) && is_false_word(s) {
        if s.len() == 1 {
            assert(super::text::ascii_upper(word_one()[0]) != super::text::ascii_upper(
                word_zero()[0],
            ));
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_one()[0]));
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_zero()[0]));
        } else if s.len() == 2 {
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_on()[0]));
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_no()[0]));
        } else {
            assert(s.len() == 3);
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_yes()[0]));
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(word_off()[0]));
        }
    }
    if super::text::eq_ignore_ascii_case(s, t) {
        lemma_eq_ignore_case_trans(s, t, word_true());
        lemma_eq_ignore_case_trans(s, t, word_one());
        lemma_eq_ignore_case_trans(s, t, word_yes());
        lemma_eq_ignore_case_trans(s, t, word_on());
        lemma_eq_ignore_case_trans(s, t, word_false());
        lemma_eq_ignore_case_trans(s, t, word_zero());
        lemma_eq_ignore_case_trans(s, t, word_no());
        lemma_eq_ignore_case_trans(s, t, word_off());
    }
    assert(super::text::eq_ignore_ascii_case(word_true(), word_true()));
    assert(super::text::eq_ignore_ascii_case(word_false(), word_false()));
    lemma_bool_word_lengths(word_false());
}

// ---------------------------------------------------------------------------
// Output formats
// ---------------------------------------------------------------------------

/// A list output format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormatCode {
    /// `json`.
    Json,
    /// `csv`.
    Csv,
    /// `tsv`.
    Tsv,
    /// `table`.
    Table,
}

/// The keyword naming a format (what its `Display` prints).
pub open spec fn keyword(f: OutputFormatCode) -> Seq<u8> {
    match f {
        OutputFormatCode::Json => seq![0x6Au8, 0x73u8, 0x6Fu8, 0x6Eu8],
        OutputFormatCode::Csv => seq![0x63u8, 0x73u8, 0x76u8],
        OutputFormatCode::Tsv => seq![0x74u8, 0x73u8, 0x76u8],
        OutputFormatCode::Table => seq![0x74u8, 0x61u8, 0x62u8, 0x6Cu8, 0x65u8],
    }
}

/// The format `s` names, ignoring ASCII case.
pub open spec fn output_format_value(s: Seq<u8>) -> Option<OutputFormatCode> {
    if super::text::eq_ignore_ascii_case(s, keyword(OutputFormatCode::Json)) {
        Some(OutputFormatCode::Json)
    } else if super::text::eq_ignore_ascii_case(s, keyword(OutputFormatCode::Csv)) {
        Some(OutputFormatCode::Csv)
    } else if super::text::eq_ignore_ascii_case(s, keyword(OutputFormatCode::Tsv)) {
        Some(OutputFormatCode::Tsv)
    } else if super::text::eq_ignore_ascii_case(s, keyword(OutputFormatCode::Table)) {
        Some(OutputFormatCode::Table)
    } else {
        None
    }
}

/// Parses an output-format keyword (`json`, `csv`, `tsv`, `table`),
/// ignoring ASCII case.
pub fn parse_output_format(s: &[u8]) -> (r: Option<OutputFormatCode>)
    ensures
        r == output_format_value(s@),
        forall|f: OutputFormatCode|
            r == Some(f) <==> super::text::eq_ignore_ascii_case(s@, #[trigger] keyword(f)),
{
    proof {
        lemma_keywords_distinct(s@);
    }
    match s.len() {
        3 => {
            proof {
                lemma_eq_len3(s@, keyword(OutputFormatCode::Csv));
                lemma_eq_len3(s@, keyword(OutputFormatCode::Tsv));
            }
            match (fold_byte(s[0]), fold_byte(s[1]), fold_byte(s[2])) {
                (b'C', b'S', b'V') => Some(OutputFormatCode::Csv),
                (b'T', b'S', b'V') => Some(OutputFormatCode::Tsv),
                _ => None,
            }
        },
        4 => {
            proof {
                lemma_eq_len4(s@, keyword(OutputFormatCode::Json));
            }
            match (fold_byte(s[0]), fold_byte(s[1]), fold_byte(s[2]), fold_byte(s[3])) {
                (b'J', b'S', b'O', b'N') => Some(OutputFormatCode::Json),
                _ => None,
            }
        },
        5 => {
            proof {
                lemma_eq_len5(s@, keyword(OutputFormatCode::Table));
            }
            match (
                fold_byte(s[0]),
                fold_byte(s[1]),
                fold_byte(s[2]),
                fold_byte(s[3]),
                fold_byte(s[4]),
            ) {
                (b'T', b'A', b'B', b'L', b'E') => Some(OutputFormatCode::Table),
                _ => None,
            }
        },
        _ => None,
    }
}

/// Text matching a keyword has its length, and at most one keyword matches.
proof fn lemma_keywords_distinct(s: Seq<u8>)
    ensures
        forall|f: OutputFormatCode|
            super::text::eq_ignore_ascii_case(s, #[trigger] keyword(f)) ==> s.len() == keyword(
                f,
            ).len(),
        forall|f: OutputFormatCode, g: OutputFormatCode|
            super::text::eq_ignore_ascii_case(s, #[trigger] keyword(f))
                && super::text::eq_ignore_ascii_case(
                s,
                #[trigger] keyword(g),
            ) ==> f == g,
{
    assert forall|f: OutputFormatCode, g: OutputFormatCode|
        super::text::eq_ignore_ascii_case(s, #[trigger] keyword(f))
            && super::text::eq_ignore_ascii_case(
            s,
            #[trigger] keyword(g),
        ) implies f == g by {
        let (kf, kg) = (keyword(f), keyword(g));
        assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(kf[0]));
        assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(kg[0]));
        if s.len() == 3 {
            assert(super::text::ascii_upper(s[0]) == super::text::ascii_upper(kf[0]));
        }
    }
}

/// Keywords are injective, and every format's keyword parses back to it:
/// `parse_output_format(display(f)) == Some(f)`.
pub proof fn lemma_output_format_round_trip(f: OutputFormatCode, g: OutputFormatCode)
    ensures
        keyword(f) == keyword(g) ==> f == g,
        output_format_value(keyword(f)) == Some(f),
{
    lemma_keywords_distinct(keyword(f));
    assert(super::text::eq_ignore_ascii_case(keyword(f), keyword(f)));
    if keyword(f) == keyword(g) {
        assert(super::text::eq_ignore_ascii_case(keyword(f), keyword(g)));
    }
}

/// The keyword of a format, as the text its `Display` writes.
pub fn output_format_keyword(f: OutputFormatCode) -> (k: &'static str)
    ensures
        vstd::utf8::encode_utf8(k@) == keyword(f),
{
    broadcast use vstd::string::group_string_axioms;

    let k = match f {
        OutputFormatCode::Json => {
            let k = "json";
            proof {
                reveal_strlit("json");
                assert(k@ =~= seq!['j', 's', 'o', 'n']);
            }
            k
        },
        OutputFormatCode::Csv => {
            let k = "csv";
            proof {
                reveal_strlit("csv");
                assert(k@ =~= seq!['c', 's', 'v']);
            }
            k
        },
        OutputFormatCode::Tsv => {
            let k = "tsv";
            proof {
                reveal_strlit("tsv");
                assert(k@ =~= seq!['t', 's', 'v']);
            }
            k
        },
        OutputFormatCode::Table => {
            let k = "table";
            proof {
                reveal_strlit("table");
                assert(k@ =~= seq!['t', 'a', 'b', 'l', 'e']);
            }
            k
        },
    };
    proof {
        let bytes = keyword(f);
        assert(super::text::is_ascii_bytes(bytes));
        super::text::lemma_ascii_bytes_round_trip(bytes);
        assert(k@ =~= super::text::chars_of_bytes(bytes));
    }
    k
}

// ---------------------------------------------------------------------------
// Reservation groups (`reservations` in trop.yaml)
// ---------------------------------------------------------------------------

/// One configured service of a `reservations` group, as written.
pub struct ServiceEntry<'a> {
    /// The tag (the key of the `services` map), untrimmed.
    pub tag: &'a str,
    /// The configured offset; omitted means 0.
    pub offset: Option<u16>,
    /// The preferred port.
    pub preferred: Option<u16>,
    /// The explicit environment-variable name.
    pub env: Option<&'a str>,
}

/// The UTF-8 bytes of an optional string.
pub open spec fn opt_utf8(e: Option<&str>) -> Option<Seq<u8>> {
    match e {
        Some(x) => Some(vstd::utf8::encode_utf8(x@)),
        None => None,
    }
}

/// The tag a service is known by: its trimmed tag. Uniqueness, allocation,
/// and the derived environment-variable name all use it.
pub open spec fn tag_key(e: ServiceEntry) -> Seq<char> {
    identifier::trimmed(e.tag@)
}

/// The offset a service falls back to (an omitted offset is 0).
pub open spec fn effective_offset(e: ServiceEntry) -> u16 {
    match e.offset {
        Some(o) => o,
        None => 0,
    }
}

/// The environment-variable name a service exports: its explicit name, or
/// the name derived from its trimmed tag.
pub open spec fn resolved_name(e: ServiceEntry) -> Result<Seq<u8>, env_name::ResolveError> {
    env_name::resolve_verdict(vstd::utf8::encode_utf8(tag_key(e)), opt_utf8(e.env))
}

/// The ASCII-case-insensitive collision key of the resolved name.
pub open spec fn env_key(e: ServiceEntry) -> Seq<u8> {
    match resolved_name(e) {
        Ok(n) => super::text::ascii_upper_seq(n),
        Err(_) => Seq::empty(),
    }
}

/// The explicit name's grammar violation, if any.
pub open spec fn explicit_env_error(e: ServiceEntry) -> Option<env_name::EnvNameError> {
    match opt_utf8(e.env) {
        Some(b) => match env_name::env_name_verdict(b) {
            Ok(()) => None,
            Err(err) => Some(err),
        },
        None => None,
    }
}

/// A service on its own is valid: its tag is a valid identifier, its
/// preferred port (if any) is not 0, and it resolves to a portable
/// environment-variable name.
pub open spec fn service_valid(e: ServiceEntry) -> bool {
    &&& identifier::identifier_verdict(e.tag@) is Ok
    &&& e.preferred != Some(0u16)
    &&& resolved_name(e) is Ok
}

/// Two services collide: same trimmed tag, same (defaulted) offset, same
/// preferred port, or resolved names equal ignoring ASCII case.
pub open spec fn services_collide(a: ServiceEntry, b: ServiceEntry) -> bool {
    ||| tag_key(a) == tag_key(b)
    ||| effective_offset(a) == effective_offset(b)
    ||| (a.preferred is Some && a.preferred == b.preferred)
    ||| env_key(a) == env_key(b)
}

/// A valid group: the base (if any) is not port 0, every service is valid,
/// and no two services collide.
pub open spec fn config_group_valid(base: Option<u16>, s: Seq<ServiceEntry>) -> bool {
    &&& base != Some(0u16)
    &&& forall|i: int| 0 <= i < s.len() ==> #[trigger] service_valid(s[i])
    &&& forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j ==> !#[trigger] services_collide(s[i], s[j])
}

/// Why a `reservations` group is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupIssue {
    /// The base is port 0.
    InvalidBase,
    /// A tag is not a valid identifier.
    Tag {
        /// Service index.
        index: usize,
        /// The rule the tag breaks.
        issue: identifier::IdentifierError,
    },
    /// A trimmed tag repeats an earlier service's.
    DuplicateTag {
        /// Service index.
        index: usize,
    },
    /// A (defaulted) offset repeats an earlier service's.
    DuplicateOffset {
        /// Service index.
        index: usize,
        /// The repeated offset.
        offset: u16,
    },
    /// A preferred port is 0.
    InvalidPreferred {
        /// Service index.
        index: usize,
    },
    /// A preferred port repeats an earlier service's.
    DuplicatePreferred {
        /// Service index.
        index: usize,
        /// The repeated port.
        port: u16,
    },
    /// An explicit environment-variable name is not portable.
    InvalidEnv {
        /// Service index.
        index: usize,
        /// The grammar rule the name breaks.
        error: env_name::EnvNameError,
    },
    /// An unmapped tag derives no portable environment-variable name.
    NonConvertibleTag {
        /// Service index.
        index: usize,
    },
    /// A resolved name equals an earlier service's, ignoring ASCII case.
    DuplicateEnv {
        /// Service index.
        index: usize,
    },
}

/// The first problem with service `i` given the services before it, in the
/// validator's order: tag rule, duplicate tag, duplicate offset, preferred
/// port 0, duplicate preferred port, invalid explicit name, non-convertible
/// tag, duplicate resolved name.
pub open spec fn service_issue(s: Seq<ServiceEntry>, i: int) -> Option<GroupIssue> {
    let e = s[i];
    let index = i as usize;
    match identifier::identifier_verdict(e.tag@) {
        Err(issue) => Some(GroupIssue::Tag { index, issue }),
        Ok(t) => if exists|j: int| 0 <= j < i && tag_key(#[trigger] s[j]) == t {
            Some(GroupIssue::DuplicateTag { index })
        } else if exists|j: int|
            0 <= j < i && effective_offset(#[trigger] s[j]) == effective_offset(e) {
            Some(GroupIssue::DuplicateOffset { index, offset: effective_offset(e) })
        } else if e.preferred == Some(0u16) {
            Some(GroupIssue::InvalidPreferred { index })
        } else if e.preferred is Some && exists|j: int|
            0 <= j < i && (#[trigger] s[j]).preferred == e.preferred {
            Some(GroupIssue::DuplicatePreferred { index, port: e.preferred->Some_0 })
        } else if explicit_env_error(e) is Some {
            Some(GroupIssue::InvalidEnv { index, error: explicit_env_error(e)->Some_0 })
        } else if resolved_name(e) is Err {
            Some(GroupIssue::NonConvertibleTag { index })
        } else if exists|j: int| 0 <= j < i && env_key(#[trigger] s[j]) == env_key(e) {
            Some(GroupIssue::DuplicateEnv { index })
        } else {
            None
        },
    }
}

/// `e` is the issue of service `i`, and every earlier service passed.
pub open spec fn first_group_issue(s: Seq<ServiceEntry>, i: int, e: GroupIssue) -> bool {
    &&& 0 <= i < s.len()
    &&& forall|k: int| 0 <= k < i ==> #[trigger] service_issue(s, k) is None
    &&& service_issue(s, i) == Some(e)
}

/// The service index a per-service issue names (-1 for `InvalidBase`).
pub open spec fn group_issue_index(e: GroupIssue) -> int {
    match e {
        GroupIssue::InvalidBase => -1,
        GroupIssue::Tag { index, .. } => index as int,
        GroupIssue::DuplicateTag { index } => index as int,
        GroupIssue::DuplicateOffset { index, .. } => index as int,
        GroupIssue::InvalidPreferred { index } => index as int,
        GroupIssue::DuplicatePreferred { index, .. } => index as int,
        GroupIssue::InvalidEnv { index, .. } => index as int,
        GroupIssue::NonConvertibleTag { index } => index as int,
        GroupIssue::DuplicateEnv { index } => index as int,
    }
}

/// No service before `n` has an issue, so services `0..n` are valid and
/// pairwise collision-free.
proof fn lemma_group_prefix_valid(s: Seq<ServiceEntry>, n: int)
    requires
        0 <= n <= s.len(),
        forall|k: int| 0 <= k < n ==> #[trigger] service_issue(s, k) is None,
    ensures
        forall|i: int| 0 <= i < n ==> #[trigger] service_valid(s[i]),
        forall|i: int, j: int|
            0 <= i < n && 0 <= j < n && i != j ==> !#[trigger] services_collide(s[i], s[j]),
{
    assert forall|i: int| 0 <= i < n implies #[trigger] service_valid(s[i]) by {
        assert(service_issue(s, i) is None);
    }
    assert forall|i: int, j: int|
        0 <= i < n && 0 <= j < n && i != j implies !#[trigger] services_collide(s[i], s[j]) by {
        let lo = if i < j {
            i
        } else {
            j
        };
        let hi = if i < j {
            j
        } else {
            i
        };
        assert(service_issue(s, hi) is None);
        let t = identifier::identifier_verdict(s[hi].tag@)->Ok_0;
        assert(t == tag_key(s[hi]));
        if tag_key(s[lo]) == tag_key(s[hi]) {
            assert(exists|k: int| 0 <= k < hi && tag_key(#[trigger] s[k]) == t);
        }
        if effective_offset(s[lo]) == effective_offset(s[hi]) {
            assert(exists|k: int|
                0 <= k < hi && effective_offset(#[trigger] s[k]) == effective_offset(s[hi]));
        }
        if s[lo].preferred is Some && s[lo].preferred == s[hi].preferred {
            assert(exists|k: int| 0 <= k < hi && (#[trigger] s[k]).preferred == s[hi].preferred);
        }
        if env_key(s[lo]) == env_key(s[hi]) {
            assert(exists|k: int| 0 <= k < hi && env_key(#[trigger] s[k]) == env_key(s[hi]));
        }
    }
}

/// A valid group has no service issue.
proof fn lemma_valid_group_has_no_issue(base: Option<u16>, s: Seq<ServiceEntry>)
    requires
        config_group_valid(base, s),
    ensures
        forall|k: int| 0 <= k < s.len() ==> #[trigger] service_issue(s, k) is None,
{
    assert forall|k: int| 0 <= k < s.len() implies #[trigger] service_issue(s, k) is None by {
        assert(service_valid(s[k]));
        if exists|j: int| 0 <= j < k && tag_key(#[trigger] s[j]) == tag_key(s[k]) {
            let j = choose|j: int| 0 <= j < k && tag_key(#[trigger] s[j]) == tag_key(s[k]);
            assert(services_collide(s[j], s[k]));
        }
        if exists|j: int|
            0 <= j < k && effective_offset(#[trigger] s[j]) == effective_offset(s[k]) {
            let j = choose|j: int|
                0 <= j < k && effective_offset(#[trigger] s[j]) == effective_offset(s[k]);
            assert(services_collide(s[j], s[k]));
        }
        if s[k].preferred is Some && exists|j: int|
            0 <= j < k && (#[trigger] s[j]).preferred == s[k].preferred {
            let j = choose|j: int| 0 <= j < k && (#[trigger] s[j]).preferred == s[k].preferred;
            assert(services_collide(s[j], s[k]));
        }
        if exists|j: int| 0 <= j < k && env_key(#[trigger] s[j]) == env_key(s[k]) {
            let j = choose|j: int| 0 <= j < k && env_key(#[trigger] s[j]) == env_key(s[k]);
            assert(services_collide(s[j], s[k]));
        }
    }
}

/// UTF-8 encoding is injective.
pub proof fn lemma_utf8_injective(a: Seq<char>, b: Seq<char>)
    ensures
        (vstd::utf8::encode_utf8(a) == vstd::utf8::encode_utf8(b)) == (a == b),
{
    vstd::utf8::encode_utf8_decode_utf8(a);
    vstd::utf8::encode_utf8_decode_utf8(b);
}

/// Whether some earlier trimmed tag equals `t`.
fn earlier_tag(tags: &[&str], t: &str) -> (found: bool)
    ensures
        found == exists|j: int| 0 <= j < tags@.len() && (#[trigger] tags@[j])@ == t@,
{
    let mut j: usize = 0;
    while j < tags.len()
        invariant
            0 <= j <= tags@.len(),
            forall|k: int| 0 <= k < j ==> (#[trigger] tags@[k])@ != t@,
        decreases tags@.len() - j,
    {
        let other = tags[j];
        let same = super::sticky::bytes_eq(other.as_bytes(), t.as_bytes());
        proof {
            lemma_utf8_injective(other@, t@);
        }
        if same {
            return true;
        }
        j += 1;
    }
    false
}

/// Whether some service before `i` has (defaulted) offset `offset`.
fn earlier_offset(s: &[ServiceEntry], i: usize, offset: u16) -> (found: bool)
    requires
        i <= s@.len(),
    ensures
        found == exists|j: int| 0 <= j < i && effective_offset(#[trigger] s@[j]) == offset,
{
    let mut j: usize = 0;
    while j < i
        invariant
            0 <= j <= i <= s@.len(),
            forall|k: int| 0 <= k < j ==> effective_offset(#[trigger] s@[k]) != offset,
        decreases i - j,
    {
        if super::group::config_service_offset(s[j].offset) == offset {
            return true;
        }
        j += 1;
    }
    false
}

/// Whether some service before `i` has preferred port `port`.
fn earlier_preferred(s: &[ServiceEntry], i: usize, port: u16) -> (found: bool)
    requires
        i <= s@.len(),
    ensures
        found == exists|j: int| 0 <= j < i && (#[trigger] s@[j]).preferred == Some(port),
{
    let mut j: usize = 0;
    while j < i
        invariant
            0 <= j <= i <= s@.len(),
            forall|k: int| 0 <= k < j ==> (#[trigger] s@[k]).preferred != Some(port),
        decreases i - j,
    {
        if let Some(other) = s[j].preferred {
            if other == port {
                return true;
            }
        }
        j += 1;
    }
    false
}

/// Whether some earlier collision key equals `key`.
fn earlier_key(keys: &[String], key: &String) -> (found: bool)
    ensures
        found == exists|j: int| 0 <= j < keys@.len() && (#[trigger] keys@[j])@ == key@,
{
    let mut j: usize = 0;
    while j < keys.len()
        invariant
            0 <= j <= keys@.len(),
            forall|k: int| 0 <= k < j ==> (#[trigger] keys@[k])@ != key@,
        decreases keys@.len() - j,
    {
        if keys[j] == *key {
            return true;
        }
        j += 1;
    }
    false
}

/// The facts recorded for the services before `i`: their trimmed tags and
/// their collision keys (whose UTF-8 bytes are `env_key`).
pub open spec fn recorded(
    s: Seq<ServiceEntry>,
    tags: Seq<&str>,
    keys: Seq<String>,
    i: int,
) -> bool {
    &&& tags.len() == i
    &&& keys.len() == i
    &&& forall|k: int| 0 <= k < i ==> (#[trigger] tags[k])@ == tag_key(s[k])
    &&& forall|k: int|
        0 <= k < i ==> vstd::utf8::encode_utf8((#[trigger] keys[k])@) == env_key(s[k])
}

/// The issue of service `i`, given the facts of the services before it; on
/// success, records service `i`'s facts.
fn check_service<'a>(
    s: &[ServiceEntry<'a>],
    i: usize,
    tags: &mut Vec<&'a str>,
    keys: &mut Vec<String>,
) -> (r: Option<GroupIssue>)
    requires
        i < s@.len(),
        recorded(s@, old(tags)@, old(keys)@, i as int),
    ensures
        r == service_issue(s@, i as int),
        r is None ==> recorded(s@, final(tags)@, final(keys)@, i + 1),
{
    let e = &s[i];
    let ghost se = s@[i as int];
    let t = match identifier::check_identifier(e.tag) {
        Ok(t) => t,
        Err(issue) => return Some(GroupIssue::Tag { index: i, issue }),
    };
    proof {
        assert(t@ == tag_key(se));
    }
    if earlier_tag(tags.as_slice(), t) {
        proof {
            let j = choose|j: int| 0 <= j < tags@.len() && (#[trigger] tags@[j])@ == t@;
            assert(tag_key(s@[j]) == t@);
        }
        return Some(GroupIssue::DuplicateTag { index: i });
    }
    proof {
        assert forall|j: int| 0 <= j < i implies tag_key(#[trigger] s@[j]) != t@ by {
            assert(tags@[j]@ == tag_key(s@[j]));
        }
    }
    let offset = super::group::config_service_offset(e.offset);
    if earlier_offset(s, i, offset) {
        return Some(GroupIssue::DuplicateOffset { index: i, offset });
    }
    if let Some(port) = e.preferred {
        if port == 0 {
            return Some(GroupIssue::InvalidPreferred { index: i });
        }
        if earlier_preferred(s, i, port) {
            return Some(GroupIssue::DuplicatePreferred { index: i, port });
        }
    }
    if let Some(explicit) = e.env {
        if let Err(error) = env_name::check_env_name(explicit) {
            return Some(GroupIssue::InvalidEnv { index: i, error });
        }
    }
    let name = match env_name::resolve_env_name(t, e.env) {
        Ok(name) => name,
        Err(env_name::ResolveError::NonConvertibleTag) => {
            return Some(GroupIssue::NonConvertibleTag { index: i });
        },
        // Unreachable: an invalid explicit name was reported above.
        Err(env_name::ResolveError::InvalidExplicit(error)) => {
            return Some(GroupIssue::InvalidEnv { index: i, error });
        },
    };
    proof {
        env_name::lemma_resolved_is_portable(
            vstd::utf8::encode_utf8(tag_key(se)),
            opt_utf8(se.env),
        );
    }
    let key = env_name::env_collision_key(name.as_str());
    proof {
        assert(vstd::utf8::encode_utf8(key@) == env_key(se));
    }
    if earlier_key(keys.as_slice(), &key) {
        proof {
            let j = choose|j: int| 0 <= j < keys@.len() && (#[trigger] keys@[j])@ == key@;
            assert(env_key(s@[j]) == env_key(se));
        }
        return Some(GroupIssue::DuplicateEnv { index: i });
    }
    proof {
        assert forall|j: int| 0 <= j < i implies env_key(#[trigger] s@[j]) != env_key(se) by {
            lemma_utf8_injective(keys@[j]@, key@);
        }
    }
    tags.push(t);
    keys.push(key);
    None
}

/// Checks a `reservations` group, with its services in the order given (the
/// glue sorts them by tag, so the reported issue is deterministic).
///
/// Succeeds exactly for a valid group (`config_group_valid`). Otherwise
/// reports `InvalidBase` for base 0, or else the first service with an issue
/// (`service_issue`).
pub fn check_config_group(base: Option<u16>, services: &[ServiceEntry]) -> (r: Result<
    (),
    GroupIssue,
>)
    ensures
        r is Ok <==> config_group_valid(base, services@),
        r matches Err(e) ==> match e {
            GroupIssue::InvalidBase => base == Some(0u16),
            _ => base != Some(0u16) && first_group_issue(services@, group_issue_index(e), e),
        },
{
    if let Some(0) = base {
        return Err(GroupIssue::InvalidBase);
    }
    let mut tags: Vec<&str> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut i: usize = 0;
    while i < services.len()
        invariant
            0 <= i <= services@.len(),
            base != Some(0u16),
            recorded(services@, tags@, keys@, i as int),
            forall|k: int| 0 <= k < i ==> #[trigger] service_issue(services@, k) is None,
        decreases services@.len() - i,
    {
        if let Some(e) = check_service(services, i, &mut tags, &mut keys) {
            assert(group_issue_index(e) == i);
            proof {
                if config_group_valid(base, services@) {
                    lemma_valid_group_has_no_issue(base, services@);
                }
            }
            return Err(e);
        }
        i += 1;
    }
    proof {
        lemma_group_prefix_valid(services@, services@.len() as int);
    }
    Ok(())
}

/// Acceptance does not depend on the order of the services: a group is valid
/// exactly when any reordering of it is. (`p` is the reordering: service `i`
/// of `t` is service `p[i]` of `s`.)
pub proof fn lemma_config_group_permutation(
    base: Option<u16>,
    s: Seq<ServiceEntry>,
    t: Seq<ServiceEntry>,
    p: Seq<int>,
)
    requires
        t.len() == s.len(),
        p.len() == s.len(),
        forall|i: int| 0 <= i < t.len() ==> 0 <= #[trigger] p[i] < s.len() && t[i] == s[p[i]],
        p.no_duplicates(),
    ensures
        config_group_valid(base, s) == config_group_valid(base, t),
{
    let n = s.len() as int;
    // `p` is injective from `0..n` into `0..n`, so it hits every index.
    vstd::set_lib::lemma_int_range(0, n);
    p.unique_seq_to_set();
    let range = vstd::set_lib::set_int_range(0, n);
    assert(p.to_set().subset_of(range)) by {
        assert forall|x: int| #[trigger] p.to_set().contains(x) implies range.contains(x) by {
            assert(p.contains(x));
            let i = choose|i: int| 0 <= i < p.len() && p[i] == x;
            assert(0 <= p[i] < n);
        }
    }
    vstd::set_lib::lemma_subset_equality(p.to_set(), range);
    assert forall|k: int| 0 <= k < n implies #[trigger] p.contains(k) by {
        assert(range.contains(k));
        assert(p.to_set().contains(k));
    }
    if config_group_valid(base, s) {
        assert forall|i: int| 0 <= i < t.len() implies #[trigger] service_valid(t[i]) by {
            assert(service_valid(s[p[i]]));
        }
        assert forall|i: int, j: int|
            0 <= i < t.len() && 0 <= j < t.len() && i != j implies !#[trigger] services_collide(
            t[i],
            t[j],
        ) by {
            assert(p[i] != p[j]);
            assert(!services_collide(s[p[i]], s[p[j]]));
        }
    }
    if config_group_valid(base, t) {
        assert forall|k: int| 0 <= k < s.len() implies #[trigger] service_valid(s[k]) by {
            assert(p.contains(k));
            let i = choose|i: int| 0 <= i < n && #[trigger] p[i] == k;
            assert(service_valid(t[i]));
        }
        assert forall|k: int, l: int|
            0 <= k < s.len() && 0 <= l < s.len() && k != l implies !#[trigger] services_collide(
            s[k],
            s[l],
        ) by {
            assert(p.contains(k) && p.contains(l));
            let i = choose|i: int| 0 <= i < n && #[trigger] p[i] == k;
            let j = choose|j: int| 0 <= j < n && #[trigger] p[j] == l;
            assert(!services_collide(t[i], t[j]));
        }
    }
}

/// Consequences of validity: omitted offsets default to 0, so at most one
/// service omits its offset; every service exports a portable name; and
/// every service's tag key is a canonical identifier.
pub proof fn lemma_config_group_consequences(base: Option<u16>, s: Seq<ServiceEntry>)
    requires
        config_group_valid(base, s),
    ensures
        forall|i: int, j: int|
            0 <= i < s.len() && 0 <= j < s.len() && i != j && (#[trigger] s[i]).offset is None
                ==> (#[trigger] s[j]).offset is Some,
        forall|i: int|
            0 <= i < s.len() ==> (#[trigger] resolved_name(s[i])) is Ok
                && env_name::is_portable_env_name(resolved_name(s[i])->Ok_0),
        forall|i: int| 0 <= i < s.len() ==> identifier::is_canonical(#[trigger] tag_key(s[i])),
{
    assert forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j && (#[trigger] s[i]).offset
            is None implies (#[trigger] s[j]).offset is Some by {
        assert(!services_collide(s[i], s[j]));
    }
    assert forall|i: int| 0 <= i < s.len() implies (#[trigger] resolved_name(s[i])) is Ok
        && env_name::is_portable_env_name(resolved_name(s[i])->Ok_0) by {
        assert(service_valid(s[i]));
        env_name::lemma_resolved_is_portable(
            vstd::utf8::encode_utf8(tag_key(s[i])),
            opt_utf8(s[i].env),
        );
    }
    assert forall|i: int| 0 <= i < s.len() implies identifier::is_canonical(
        #[trigger] tag_key(s[i]),
    ) by {
        assert(service_valid(s[i]));
        identifier::lemma_trimmed_is_canonical(s[i].tag@);
    }
}

/// `r` is what reserve-group's request validation receives for the
/// configured service `e` (`build_group_request`): its tag as written (the
/// request validation trims it), its defaulted offset, and its preferred
/// port.
pub open spec fn requested_as(e: ServiceEntry, r: super::group::RawService) -> bool {
    &&& r.tag@ == e.tag@
    &&& r.offset == Some(effective_offset(e))
    &&& r.preferred == e.preferred
}

/// Every non-empty configuration group that validates passes reserve-group's
/// request validation (`super::group::group_request_valid`, which
/// `validate_group_request` accepts exactly): every tag passes the
/// identifier rule, every service has an offset, no preferred port is 0,
/// and trimmed tags, offsets, and preferred ports are pairwise distinct.
pub proof fn lemma_valid_config_group_is_valid_request(
    base: Option<u16>,
    s: Seq<ServiceEntry>,
    r: Seq<super::group::RawService>,
)
    requires
        config_group_valid(base, s),
        s.len() > 0,
        r.len() == s.len(),
        forall|i: int| 0 <= i < s.len() ==> #[trigger] requested_as(s[i], r[i]),
    ensures
        super::group::group_request_valid(r),
{
    assert forall|i: int| 0 <= i < r.len() implies #[trigger] super::group::request_service_ok(
        r[i],
    ) by {
        assert(service_valid(s[i]) && requested_as(s[i], r[i]));
    }
    assert forall|i: int, j: int| 0 <= i < r.len() && 0 <= j < r.len() && i != j implies !(
    #[trigger] super::group::requests_clash(r[i], r[j])) by {
        assert(!services_collide(s[i], s[j]));
        assert(requested_as(s[i], r[i]) && requested_as(s[j], r[j]));
        assert(super::group::tag_key(r[i]) == tag_key(s[i]));
        assert(super::group::tag_key(r[j]) == tag_key(s[j]));
    }
}

// ---------------------------------------------------------------------------
// Database lock-wait bounds
// ---------------------------------------------------------------------------

/// `SQLite`'s busy-timeout limit: it stores the timeout as a signed 32-bit
/// millisecond count, so at most `i32::MAX` milliseconds.
pub const MAX_BUSY_TIMEOUT_MILLIS: u64 = 0x7FFF_FFFF;

/// The largest whole number of seconds within the limit, derived from it.
pub const MAX_LOCK_WAIT_SECONDS: u64 = MAX_BUSY_TIMEOUT_MILLIS / 1000;

/// A lock wait of `seconds` whole seconds fits `SQLite`'s limit.
pub open spec fn lock_wait_fits(seconds: int) -> bool {
    seconds * 1000 <= i32::MAX
}

/// The milliseconds of a duration of `secs` seconds and `nanos` nanoseconds,
/// as `Duration::as_millis` computes them.
pub open spec fn duration_millis(secs: int, nanos: int) -> int {
    secs * 1000 + nanos / 1_000_000
}

/// A busy timeout of `secs` seconds and `nanos` nanoseconds fits `SQLite`'s
/// limit.
pub open spec fn busy_timeout_fits(secs: int, nanos: int) -> bool {
    duration_millis(secs, nanos) <= i32::MAX
}

/// The limit is `i32::MAX` milliseconds; the seconds limit is the largest
/// whole number of seconds within it (`2147483`), so it is tight.
pub proof fn lemma_lock_wait_limits()
    ensures
        MAX_BUSY_TIMEOUT_MILLIS == i32::MAX as u64,
        MAX_LOCK_WAIT_SECONDS == 2147483,
        lock_wait_fits(MAX_LOCK_WAIT_SECONDS as int),
        !lock_wait_fits(MAX_LOCK_WAIT_SECONDS as int + 1),
        forall|s: int| #[trigger] lock_wait_fits(s) <==> s <= MAX_LOCK_WAIT_SECONDS,
{
    assert forall|s: int| #[trigger] lock_wait_fits(s) <==> s <= MAX_LOCK_WAIT_SECONDS by {
        if s <= 2147483 {
            assert(s * 1000 <= 2147483000) by (nonlinear_arith)
                requires
                    s <= 2147483,
            ;
        } else {
            assert(s * 1000 >= 2147484000) by (nonlinear_arith)
                requires
                    s >= 2147484,
            ;
        }
    }
}

/// Whether a configured lock wait of `seconds` fits `SQLite`'s limit.
pub const fn lock_wait_seconds_ok(seconds: u64) -> (ok: bool)
    ensures
        ok == lock_wait_fits(seconds as int),
{
    proof {
        lemma_lock_wait_limits();
    }
    seconds <= MAX_LOCK_WAIT_SECONDS
}

/// Whether a busy timeout of `secs` seconds and `subsec_nanos` nanoseconds
/// (a `Duration`'s parts) fits `SQLite`'s limit. Total: every input is
/// decided without overflow.
pub fn busy_timeout_ok(secs: u64, subsec_nanos: u32) -> (ok: bool)
    ensures
        ok == busy_timeout_fits(secs as int, subsec_nanos as int),
{
    proof {
        lemma_lock_wait_limits();
    }
    if secs > MAX_LOCK_WAIT_SECONDS {
        proof {
            assert(subsec_nanos / 1_000_000 >= 0);
        }
        return false;
    }
    proof {
        assert(secs * 1000 <= 2147483000) by (nonlinear_arith)
            requires
                secs <= 2147483,
        ;
    }
    secs * 1000 + u64::from(subsec_nanos / 1_000_000) <= MAX_BUSY_TIMEOUT_MILLIS
}

/// Every lock wait the configuration accepts converts (as whole seconds) to a
/// busy timeout the database accepts, and every shorter timeout is accepted
/// too (so a remaining wait never exceeds the limit).
pub proof fn lemma_lock_wait_is_valid_busy_timeout(seconds: int, secs: int, nanos: int)
    requires
        0 <= seconds,
        lock_wait_fits(seconds),
        0 <= secs,
        0 <= nanos,
        duration_millis(secs, nanos) <= duration_millis(seconds, 0),
    ensures
        busy_timeout_fits(seconds, 0),
        busy_timeout_fits(secs, nanos),
{
}

// ---------------------------------------------------------------------------
// Built-in defaults, flags, and permissions
// ---------------------------------------------------------------------------

/// Default `cleanup.expire_after_days`.
pub const DEFAULT_EXPIRE_AFTER_DAYS: u32 = 30;

/// Default `maximum_lock_wait_seconds`.
pub const DEFAULT_LOCK_WAIT_SECONDS: u64 = 5;

/// Default `output_format`.
pub const DEFAULT_OUTPUT_FORMAT: OutputFormatCode = OutputFormatCode::Table;

/// Whether `cleanup.expire_after_days` is valid (at least one day).
pub const fn expire_after_days_ok(days: u32) -> (ok: bool)
    ensures
        ok == (days >= 1),
{
    days != 0
}

/// The built-in defaults satisfy every rule: the expiry is at least one day
/// and the lock wait fits `SQLite`'s limit (the built-in port range is
/// `super::port_bounds::lemma_defaults_valid`).
pub proof fn lemma_builtin_defaults_valid()
    ensures
        DEFAULT_EXPIRE_AFTER_DAYS >= 1,
        lock_wait_fits(DEFAULT_LOCK_WAIT_SECONDS as int),
        busy_timeout_fits(DEFAULT_LOCK_WAIT_SECONDS as int, 0),
{
}

/// A boolean setting is on: explicitly `true`. Unset means off.
pub open spec fn flag_on(flag: Option<bool>) -> bool {
    flag == Some(true)
}

/// Whether a boolean setting is on (unset means off, the safe default).
pub const fn flag_enabled(flag: Option<bool>) -> (on: bool)
    ensures
        on == flag_on(flag),
{
    match flag {
        Some(value) => value,
        None => false,
    }
}

/// The effective `(project change, task change)` permissions:
/// `allow_change` grants both, and each specific flag grants its own.
pub const fn derive_permissions(
    allow_change: Option<bool>,
    allow_change_project: Option<bool>,
    allow_change_task: Option<bool>,
) -> (r: (bool, bool))
    ensures
        r.0 == (flag_on(allow_change) || flag_on(allow_change_project)),
        r.1 == (flag_on(allow_change) || flag_on(allow_change_task)),
{
    let change = flag_enabled(allow_change);
    (change || flag_enabled(allow_change_project), change || flag_enabled(allow_change_task))
}

/// `a` grants no more than `b`.
pub open spec fn flag_le(a: Option<bool>, b: Option<bool>) -> bool {
    flag_on(a) ==> flag_on(b)
}

/// The derivation is monotone (turning a flag on never revokes a
/// permission), denies by default, and treats an explicit `false` exactly
/// like an unset flag.
pub proof fn lemma_permissions(
    c1: Option<bool>,
    p1: Option<bool>,
    t1: Option<bool>,
    c2: Option<bool>,
    p2: Option<bool>,
    t2: Option<bool>,
)
    requires
        flag_le(c1, c2),
        flag_le(p1, p2),
        flag_le(t1, t2),
    ensures
        (flag_on(c1) || flag_on(p1)) ==> (flag_on(c2) || flag_on(p2)),
        (flag_on(c1) || flag_on(t1)) ==> (flag_on(c2) || flag_on(t2)),
        !flag_on(None) && !flag_on(Some(false)),
{
}

// ---------------------------------------------------------------------------
// Layering: the per-leaf overlay
// ---------------------------------------------------------------------------

/// One layer's value for a leaf over the value below it: the layer's own
/// value when it sets one, otherwise the value below.
pub open spec fn overlay<T>(lower: Option<T>, layer: Option<T>) -> Option<T> {
    if layer is Some {
        layer
    } else {
        lower
    }
}

/// Lays one layer's leaf over the value below. Returns the new value and
/// whether this layer set it (the layer is then the leaf's provenance).
pub fn overlay_leaf<T: Copy>(lower: Option<T>, layer: Option<T>) -> (r: (Option<T>, bool))
    ensures
        r.0 == overlay(lower, layer),
        r.1 == layer is Some,
{
    match layer {
        Some(value) => (Some(value), true),
        None => (lower, false),
    }
}

/// The overlay is a monoid with `None` as its identity: layering a layer that
/// sets nothing changes nothing, and how consecutive layers are grouped does
/// not matter.
pub proof fn lemma_overlay_monoid<T>(a: Option<T>, b: Option<T>, c: Option<T>)
    ensures
        overlay(None, a) == a,
        overlay(a, None) == a,
        overlay(overlay(a, b), c) == overlay(a, overlay(b, c)),
{
}

/// The value of a leaf after laying `layers` (lowest precedence first) over
/// `start`.
pub open spec fn fold_overlay<T>(start: Option<T>, layers: Seq<Option<T>>) -> Option<T>
    decreases layers.len(),
{
    if layers.len() == 0 {
        start
    } else {
        overlay(fold_overlay(start, layers.drop_last()), layers.last())
    }
}

/// Precedence, in the model: the fold's value for a leaf is the value of
/// the highest-precedence layer that sets it (`w`, which is also the last
/// layer whose overlay reported "set"), or the starting value (the built-in
/// default) when no layer sets it. Production layering and its provenance
/// bookkeeping are not checked against `fold_overlay`.
pub proof fn lemma_fold_overlay_precedence<T>(start: Option<T>, layers: Seq<Option<T>>, w: int)
    ensures
        0 <= w < layers.len() && layers[w] is Some && (forall|j: int|
            w < j < layers.len() ==> #[trigger] layers[j] is None) ==> fold_overlay(start, layers)
            == layers[w],
        (forall|j: int| 0 <= j < layers.len() ==> #[trigger] layers[j] is None) ==> fold_overlay(
            start,
            layers,
        ) == start,
    decreases layers.len(),
{
    if layers.len() > 0 {
        let init = layers.drop_last();
        if 0 <= w < layers.len() && layers[w] is Some && (forall|j: int|
            w < j < layers.len() ==> #[trigger] layers[j] is None) {
            if w < layers.len() - 1 {
                assert(layers.last() is None);
                assert forall|j: int| w < j < init.len() implies #[trigger] init[j] is None by {
                    assert(layers[j] is None);
                }
                lemma_fold_overlay_precedence(start, init, w);
            }
        }
        if forall|j: int| 0 <= j < layers.len() ==> #[trigger] layers[j] is None {
            assert(layers.last() is None);
            assert forall|j: int| 0 <= j < init.len() implies #[trigger] init[j] is None by {
                assert(layers[j] is None);
            }
            lemma_fold_overlay_precedence(start, init, w);
        }
    }
}

/// Folding is compositional: laying `a` and then `b` equals laying `a + b`,
/// so merging layers in groups (e.g. files, then overrides) gives the same
/// result as merging them one by one.
pub proof fn lemma_fold_overlay_append<T>(start: Option<T>, a: Seq<Option<T>>, b: Seq<Option<T>>)
    ensures
        fold_overlay(start, a + b) == fold_overlay(fold_overlay(start, a), b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        assert((a + b).drop_last() =~= a + b.drop_last());
        lemma_fold_overlay_append(start, a, b.drop_last());
    }
}

// ---------------------------------------------------------------------------
// Layering: accumulated exclusions
// ---------------------------------------------------------------------------

/// The `excluded_ports` list after laying `layers` (lowest precedence
/// first): each layer's list is appended (`Vec::extend`), never replaced.
pub open spec fn accumulate_exclusions(
    layers: Seq<Seq<super::exclusions::ExclusionEntry>>,
) -> Seq<super::exclusions::ExclusionEntry>
    decreases layers.len(),
{
    if layers.len() == 0 {
        Seq::empty()
    } else {
        accumulate_exclusions(layers.drop_last()) + layers.last()
    }
}

/// Exclusions accumulate, in the model: `accumulate_exclusions` excludes
/// exactly the ports some layer excludes (the union of the layers' sets),
/// whatever the order. The production `Vec::extend` sites are not checked
/// against it.
pub proof fn lemma_accumulated_exclusions_union(
    layers: Seq<Seq<super::exclusions::ExclusionEntry>>,
    p: int,
)
    ensures
        super::exclusions::covers(accumulate_exclusions(layers), p) <==> exists|i: int|
            0 <= i < layers.len() && super::exclusions::covers(#[trigger] layers[i], p),
    decreases layers.len(),
{
    if layers.len() > 0 {
        let init = layers.drop_last();
        lemma_accumulated_exclusions_union(init, p);
        super::exclusions::lemma_covers_add(accumulate_exclusions(init), layers.last(), p);
        if exists|i: int|
            0 <= i < layers.len() && super::exclusions::covers(#[trigger] layers[i], p) {
            let i = choose|i: int|
                0 <= i < layers.len() && super::exclusions::covers(#[trigger] layers[i], p);
            if i < init.len() {
                assert(super::exclusions::covers(init[i], p));
            }
        }
        if exists|i: int| 0 <= i < init.len() && super::exclusions::covers(#[trigger] init[i], p) {
            let i = choose|i: int|
                0 <= i < init.len() && super::exclusions::covers(#[trigger] init[i], p);
            assert(super::exclusions::covers(layers[i], p));
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration files: kinds, placement, and the reservation group
// ---------------------------------------------------------------------------

/// The precedence of the user configuration file (`config.yaml`).
pub const USER_PRECEDENCE: u8 = 1;

/// The precedence of a project file (`trop.yaml`, or a nominated file).
pub const PROJECT_PRECEDENCE: u8 = 2;

/// The precedence of a private project file (`trop.local.yaml`).
pub const LOCAL_PRECEDENCE: u8 = 3;

/// The role of a configuration file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// The user configuration file.
    User,
    /// A project file.
    Project,
    /// A private project file.
    Local,
}

/// The role of a file loaded at precedence `p`: 1 is the user file, 3 the
/// private project file, anything else a project file.
pub const fn kind_for_precedence(p: u8) -> (k: FileKind)
    ensures
        k == (if p == USER_PRECEDENCE {
            FileKind::User
        } else if p == LOCAL_PRECEDENCE {
            FileKind::Local
        } else {
            FileKind::Project
        }),
        k == FileKind::User <==> p == USER_PRECEDENCE,
{
    match p {
        USER_PRECEDENCE => FileKind::User,
        LOCAL_PRECEDENCE => FileKind::Local,
        _ => FileKind::Project,
    }
}

/// What a configuration document does to the effective reservation group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupOverlay {
    /// The document omits `reservations`: the group below stays.
    Inherit,
    /// The document supplies a group: it replaces the group below.
    Replace,
    /// The document sets `reservations: null`: the group below is removed.
    Clear,
}

/// Why a document may not appear in its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementError {
    /// The user configuration file names a project.
    ProjectInUserFile,
    /// The user configuration file declares reservations.
    ReservationsInUserFile,
}

/// Checks where a document may appear, and returns its effective group
/// overlay: the user configuration file may name neither a project nor a
/// reservation group (its `reservations: null` is inert); project files may
/// contain both.
pub const fn check_placement(user_file: bool, has_project: bool, overlay: GroupOverlay) -> (r:
    Result<GroupOverlay, PlacementError>)
    ensures
        user_file && has_project ==> r == Err::<GroupOverlay, PlacementError>(
            PlacementError::ProjectInUserFile,
        ),
        user_file && !has_project && overlay == GroupOverlay::Replace ==> r == Err::<
            GroupOverlay,
            PlacementError,
        >(PlacementError::ReservationsInUserFile),
        user_file && !has_project && overlay != GroupOverlay::Replace ==> r == Ok::<
            GroupOverlay,
            PlacementError,
        >(GroupOverlay::Inherit),
        !user_file ==> r == Ok::<GroupOverlay, PlacementError>(overlay),
        user_file && r is Ok ==> r == Ok::<GroupOverlay, PlacementError>(GroupOverlay::Inherit),
{
    if !user_file {
        return Ok(overlay);
    }
    if has_project {
        return Err(PlacementError::ProjectInUserFile);
    }
    match overlay {
        GroupOverlay::Replace => Err(PlacementError::ReservationsInUserFile),
        GroupOverlay::Inherit | GroupOverlay::Clear => Ok(GroupOverlay::Inherit),
    }
}

/// The layer whose group is in effect after laying the documents' group
/// overlays in order (lowest precedence first), or `None` for no group.
pub open spec fn group_winner(overlays: Seq<GroupOverlay>) -> Option<int>
    decreases overlays.len(),
{
    if overlays.len() == 0 {
        None
    } else {
        match overlays.last() {
            GroupOverlay::Inherit => group_winner(overlays.drop_last()),
            GroupOverlay::Replace => Some(overlays.len() - 1),
            GroupOverlay::Clear => None,
        }
    }
}

/// In the model, the effective group is the group of the last document
/// that mentions `reservations`, unless that document cleared it:
/// `Some(i)` exactly when document `i` replaced the group and every later
/// document inherits it. The builder's replace/clear loop is not checked
/// against `group_winner`.
pub proof fn lemma_group_winner(overlays: Seq<GroupOverlay>, i: int)
    ensures
        group_winner(overlays) == Some(i) <==> (0 <= i < overlays.len() && overlays[i]
            == GroupOverlay::Replace && forall|j: int|
            i < j < overlays.len() ==> #[trigger] overlays[j] == GroupOverlay::Inherit),
        group_winner(overlays) matches Some(w) ==> 0 <= w < overlays.len(),
    decreases overlays.len(),
{
    if overlays.len() > 0 {
        let init = overlays.drop_last();
        lemma_group_winner(init, i);
        let n = overlays.len() - 1;
        if overlays.last() == GroupOverlay::Inherit {
            if 0 <= i < overlays.len() && overlays[i] == GroupOverlay::Replace && forall|j: int|
                i < j < overlays.len() ==> #[trigger] overlays[j] == GroupOverlay::Inherit {
                assert(i < n);
                assert forall|j: int| i < j < init.len() implies #[trigger] init[j]
                    == GroupOverlay::Inherit by {
                    assert(overlays[j] == GroupOverlay::Inherit);
                }
            }
            if group_winner(init) == Some(i) {
                assert(init[i] == overlays[i]);
                assert forall|j: int| i < j < overlays.len() implies #[trigger] overlays[j]
                    == GroupOverlay::Inherit by {
                    if j < n {
                        assert(init[j] == GroupOverlay::Inherit);
                    }
                }
            }
        } else if overlays.last() == GroupOverlay::Clear {
            if 0 <= i < overlays.len() && overlays[i] == GroupOverlay::Replace && forall|j: int|
                i < j < overlays.len() ==> #[trigger] overlays[j] == GroupOverlay::Inherit {
                assert(i != n);
                assert(overlays[n] == GroupOverlay::Inherit);
            }
        } else {
            if 0 <= i < overlays.len() && overlays[i] == GroupOverlay::Replace && forall|j: int|
                i < j < overlays.len() ==> #[trigger] overlays[j] == GroupOverlay::Inherit {
                if i < n {
                    assert(overlays[n] == GroupOverlay::Inherit);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Project-file discovery
// ---------------------------------------------------------------------------

/// The oracle reported neither `trop.yaml` nor `trop.local.yaml` at any of
/// the levels `0..k` (level 0 is the starting directory, level `k + 1` the
/// parent of level `k`).
pub open spec fn no_config_below<F: Fn(usize) -> (bool, bool)>(has_files: F, k: int) -> bool {
    forall|j: usize| j < k ==> #[trigger] has_files.ensures((j,), (false, false))
}

/// Finds the nearest directory holding a project file: the first of `levels`
/// directories (from the starting directory up to the root) at which the
/// oracle reports `trop.yaml` or `trop.local.yaml`, with what it reported.
/// It loads nothing; `level_load_order` orders the level's files.
pub fn first_config_level<F: Fn(usize) -> (bool, bool)>(levels: usize, has_files: &F) -> (r:
    Option<(usize, bool, bool)>)
    requires
        forall|k: usize| k < levels ==> #[trigger] has_files.requires((k,)),
    ensures
        r matches Some((k, project, local)) ==> {
            &&& k < levels
            &&& project || local
            &&& has_files.ensures((k,), (project, local))
            &&& no_config_below(*has_files, k as int)
        },
        r is None ==> no_config_below(*has_files, levels as int),
{
    let mut k: usize = 0;
    while k < levels
        invariant
            k <= levels,
            forall|j: usize| j < levels ==> #[trigger] has_files.requires((j,)),
            no_config_below(*has_files, k as int),
        decreases levels - k,
    {
        let (project, local) = has_files(k);
        if project || local {
            return Some((k, project, local));
        }
        proof {
            assert forall|j: usize| j < k + 1 implies #[trigger] has_files.ensures(
                (j,),
                (false, false),
            ) by {
                if j == k {
                    assert(has_files.ensures((k,), (project, local)));
                }
            }
        }
        k += 1;
    }
    None
}

/// A check of the constants: user < project < local. The order in which a
/// level's files are loaded is `level_load_order`'s.
pub proof fn lemma_precedence_order()
    ensures
        USER_PRECEDENCE < PROJECT_PRECEDENCE < LOCAL_PRECEDENCE,
{
}

/// A project file of a configuration level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelFile {
    /// The level's `trop.yaml`.
    Project,
    /// The level's `trop.local.yaml`.
    Local,
}

/// The precedence a level file is loaded at.
pub open spec fn spec_level_file_precedence(file: LevelFile) -> u8 {
    match file {
        LevelFile::Project => PROJECT_PRECEDENCE,
        LevelFile::Local => LOCAL_PRECEDENCE,
    }
}

/// The precedence a level file is loaded at: `trop.yaml` is a project file
/// and `trop.local.yaml` a private project file
/// (`lemma_level_file_kinds`).
pub const fn level_file_precedence(file: LevelFile) -> (p: u8)
    ensures
        p == spec_level_file_precedence(file),
{
    match file {
        LevelFile::Project => PROJECT_PRECEDENCE,
        LevelFile::Local => LOCAL_PRECEDENCE,
    }
}

/// A level file's precedence gives it its role: by `kind_for_precedence`'s
/// specification, `trop.yaml` is read as a project file and
/// `trop.local.yaml` as a private project file, never as the user file.
pub proof fn lemma_level_file_kinds()
    ensures
        spec_level_file_precedence(LevelFile::Project) != USER_PRECEDENCE,
        spec_level_file_precedence(LevelFile::Project) != LOCAL_PRECEDENCE,
        spec_level_file_precedence(LevelFile::Local) == LOCAL_PRECEDENCE,
{
}

/// The files of a configuration level to load, in load order: exactly the
/// files present (`trop.yaml` when `has_project`, `trop.local.yaml` when
/// `has_local`), each once, in strictly ascending precedence. So the level's
/// `trop.yaml` is loaded before its `trop.local.yaml`, and the later, higher
/// precedence file overlays the earlier one.
pub fn level_load_order(has_project: bool, has_local: bool) -> (order: Vec<LevelFile>)
    ensures
        order@.contains(LevelFile::Project) <==> has_project,
        order@.contains(LevelFile::Local) <==> has_local,
        order@.len() == (if has_project {
            1int
        } else {
            0int
        }) + (if has_local {
            1int
        } else {
            0int
        }),
        forall|i: int, j: int|
            0 <= i < j < order@.len() ==> spec_level_file_precedence(#[trigger] order@[i])
                < spec_level_file_precedence(#[trigger] order@[j]),
{
    let mut order: Vec<LevelFile> = Vec::new();
    if has_project {
        order.push(LevelFile::Project);
    }
    if has_local {
        order.push(LevelFile::Local);
    }
    proof {
        if has_project {
            assert(order@[0] == LevelFile::Project);
        }
        if has_local {
            assert(order@[order@.len() - 1] == LevelFile::Local);
        }
        if !has_project {
            assert forall|i: int| 0 <= i < order@.len() implies order@[i] != LevelFile::Project by {}
        }
        if !has_local {
            assert forall|i: int| 0 <= i < order@.len() implies order@[i] != LevelFile::Local by {}
        }
    }
    order
}

} // verus!
