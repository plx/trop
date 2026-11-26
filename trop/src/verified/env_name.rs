//! Verified kernel: environment-variable name grammar, derivation from
//! service tags, and collision keys.
//!
//! Generated names are written verbatim into `export`/`set -x`/`$env:` shell
//! statements and `.env` files, so the grammar is the injection barrier:
//!
//! - `check_env_name` accepts exactly the portable names
//!   `[A-Za-z_][A-Za-z0-9_]*` of 1 to 255 bytes (`is_portable_env_name`),
//!   reporting `Empty > TooLong > BadFirst > BadRest` in that precedence;
//!   `lemma_portable_is_shell_inert` shows such a name contains no quote,
//!   `$`, backtick, whitespace, control byte, `=`, `;`, or non-ASCII byte.
//! - `derive_env_name` uppercases ASCII letters and maps `-` to `_`
//!   (`derive_spec`), then applies the grammar; non-ASCII tags provably
//!   never derive a name, and two tags derive the same name exactly when they
//!   differ only in ASCII case and `-`/`_`.
//! - `resolve_env_name`: an explicit mapping wins and is validated on its
//!   own, with no fallback to the tag; otherwise the name is derived.
//! - `env_collision_key`: the ASCII-uppercase key under which two names
//!   collide exactly when they are equal ignoring ASCII case.
//!
//! Production glue: `crate::identifier::EnvironmentVariableName`.

use vstd::prelude::*;

use super::text;

verus! {

// ---------------------------------------------------------------------------
// The portable grammar
// ---------------------------------------------------------------------------

/// Maximum length of an environment-variable name, in bytes.
pub const MAX_ENV_NAME_BYTES: usize = 255;

/// A byte that may start a name: `A-Z`, `a-z`, or `_`.
pub open spec fn is_env_head(b: u8) -> bool {
    text::is_ascii_upper_byte(b) || text::is_ascii_lower_byte(b) || b == 0x5F
}

/// A byte that may continue a name: `A-Z`, `a-z`, `_`, or `0-9`.
pub open spec fn is_env_tail(b: u8) -> bool {
    is_env_head(b) || (0x30 <= b <= 0x39)
}

/// The portable grammar `[A-Za-z_][A-Za-z0-9_]*`, 1 to 255 bytes.
pub open spec fn is_portable_env_name(s: Seq<u8>) -> bool {
    &&& 1 <= s.len() <= MAX_ENV_NAME_BYTES
    &&& is_env_head(s[0])
    &&& forall|i: int| 1 <= i < s.len() ==> #[trigger] is_env_tail(s[i])
}

/// Why a name is not portable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvNameError {
    /// The name is empty.
    Empty,
    /// The name is longer than `MAX_ENV_NAME_BYTES` bytes.
    TooLong,
    /// The first byte is not an ASCII letter or `_`.
    BadFirst,
    /// A later byte is not an ASCII letter, digit, or `_`.
    BadRest,
}

/// Some byte after the first may not continue a name.
pub open spec fn has_bad_rest(s: Seq<u8>) -> bool {
    exists|i: int| 1 <= i < s.len() && !is_env_tail(#[trigger] s[i])
}

/// The grammar check with its error precedence `Empty > TooLong > BadFirst >
/// BadRest`.
pub open spec fn env_name_verdict(s: Seq<u8>) -> Result<(), EnvNameError> {
    if s.len() == 0 {
        Err(EnvNameError::Empty)
    } else if s.len() > MAX_ENV_NAME_BYTES {
        Err(EnvNameError::TooLong)
    } else if !is_env_head(s[0]) {
        Err(EnvNameError::BadFirst)
    } else if has_bad_rest(s) {
        Err(EnvNameError::BadRest)
    } else {
        Ok(())
    }
}

/// The verdict accepts exactly the portable names.
pub proof fn lemma_verdict_ok_iff_portable(s: Seq<u8>)
    ensures
        env_name_verdict(s) is Ok <==> is_portable_env_name(s),
{
}

/// Bytes that have a meaning to a POSIX shell, fish, PowerShell, or a dotenv
/// parser, or that could change how a terminal renders the output: every
/// control byte (NUL, TAB, LF, CR, ESC, DEL, ...), space, quotes and
/// backtick, `$ = ; & | < > ( ) { } [ ] * ? # \ ! ~ % ^ , . : @ + -`, and
/// every non-ASCII byte.
pub open spec fn is_shell_significant(b: u8) -> bool {
    ||| b < 0x20
    ||| b == 0x7F
    ||| b >= 0x80
    ||| b == 0x20 /* space */
    ||| b == 0x22 /* " */
    ||| b == 0x27 /* ' */
    ||| b == 0x60 /* ` */
    ||| b == 0x24 /* $ */
    ||| b == 0x3D /* = */
    ||| b == 0x3B /* ; */
    ||| b == 0x26 /* & */
    ||| b == 0x7C /* | */
    ||| b == 0x3C /* < */
    ||| b == 0x3E /* > */
    ||| b == 0x28 /* ( */
    ||| b == 0x29 /* ) */
    ||| b == 0x7B /* { */
    ||| b == 0x7D /* } */
    ||| b == 0x5B /* [ */
    ||| b == 0x5D /* ] */
    ||| b == 0x2A /* * */
    ||| b == 0x3F /* ? */
    ||| b == 0x23 /* # */
    ||| b == 0x5C /* \ */
    ||| b == 0x21 /* ! */
    ||| b == 0x7E /* ~ */
    ||| b == 0x25 /* % */
    ||| b == 0x5E /* ^ */
    ||| b == 0x2C /* , */
    ||| b == 0x2E /* . */
    ||| b == 0x3A /* : */
    ||| b == 0x40 /* @ */
    ||| b == 0x2B /* + */
    ||| b == 0x2D /* - */
}

/// Shell-inertness: every byte of a portable name is an ASCII letter, digit,
/// or `_`, so none is shell-significant.
pub proof fn lemma_portable_is_shell_inert(s: Seq<u8>)
    requires
        is_portable_env_name(s),
    ensures
        forall|i: int| 0 <= i < s.len() ==> is_env_tail(#[trigger] s[i]),
        forall|i: int| 0 <= i < s.len() ==> !is_shell_significant(#[trigger] s[i]),
        text::is_ascii_bytes(s),
{
    assert forall|i: int| 0 <= i < s.len() implies is_env_tail(#[trigger] s[i]) by {
        // Byte 0 is a head byte, and the head class is contained in the tail
        // class; later bytes are tail bytes by the grammar.
        if i == 0 {
            assert(is_env_head(s[0]));
        }
    }
}

/// A string whose UTF-8 encoding is a portable name is ASCII, one byte per
/// char, so its bytes and its chars are the same text (and converting the
/// bytes to a `String` is exact).
pub proof fn lemma_portable_is_ascii(s: Seq<char>)
    requires
        is_portable_env_name(vstd::utf8::encode_utf8(s)),
    ensures
        vstd::utf8::is_ascii_chars(s),
        s.len() == vstd::utf8::encode_utf8(s).len(),
        text::chars_of_bytes(vstd::utf8::encode_utf8(s)) == s,
{
    lemma_portable_is_shell_inert(vstd::utf8::encode_utf8(s));
    text::lemma_ascii_encoding_is_ascii_chars(s);
}

// ---------------------------------------------------------------------------
// Derivation from a service tag
// ---------------------------------------------------------------------------

/// One byte of the derivation: `-` becomes `_`, ASCII lowercase becomes
/// uppercase, every other byte is kept.
pub open spec fn derive_byte(b: u8) -> u8 {
    if b == 0x2D {
        0x5F
    } else {
        text::ascii_upper(b)
    }
}

/// The derived name of a tag (before validation).
pub open spec fn derive_spec(tag: Seq<u8>) -> Seq<u8> {
    tag.map_values(|b: u8| derive_byte(b))
}

/// `x` and `y` derive the same byte: they are equal, both separators (`-` or
/// `_`), or the same ASCII letter in either case.
pub open spec fn same_derived_byte(x: u8, y: u8) -> bool {
    ||| x == y
    ||| (x == 0x2D || x == 0x5F) && (y == 0x2D || y == 0x5F)
    ||| (text::is_ascii_lower_byte(x) || text::is_ascii_upper_byte(x)) && text::ascii_upper(x)
        == text::ascii_upper(y)
}

/// Shape of derived names: same length as the tag, no ASCII lowercase letter
/// and no `-`; only `-` and lowercase letters are changed.
pub proof fn lemma_derive_shape(tag: Seq<u8>)
    ensures
        derive_spec(tag).len() == tag.len(),
        forall|i: int|
            0 <= i < tag.len() ==> !text::is_ascii_lower_byte(#[trigger] derive_spec(tag)[i])
                && derive_spec(tag)[i] != 0x2D,
        forall|i: int|
            0 <= i < tag.len() && tag[i] != 0x2D && !text::is_ascii_lower_byte(tag[i])
                ==> #[trigger] derive_spec(tag)[i] == tag[i],
{
}

/// Derivation is idempotent: it is the identity on names it already produced.
pub proof fn lemma_derive_idempotent(tag: Seq<u8>)
    ensures
        derive_spec(derive_spec(tag)) == derive_spec(tag),
{
    assert(derive_spec(derive_spec(tag)) =~= derive_spec(tag));
}

/// Two tags derive the same name exactly when they have the same length and
/// agree byte by byte up to ASCII case and `-`/`_` (so `web-server`,
/// `WEB_SERVER` and `Web_Server` collide).
pub proof fn lemma_derive_collision(a: Seq<u8>, b: Seq<u8>)
    ensures
        derive_spec(a) == derive_spec(b) <==> (a.len() == b.len() && forall|i: int|
            0 <= i < a.len() ==> same_derived_byte(#[trigger] a[i], b[i])),
{
    if a.len() == b.len() && forall|i: int|
        0 <= i < a.len() ==> same_derived_byte(#[trigger] a[i], b[i]) {
        assert(derive_spec(a) =~= derive_spec(b));
    }
    if derive_spec(a) == derive_spec(b) {
        assert(a.len() == derive_spec(a).len() && b.len() == derive_spec(b).len());
        assert forall|i: int| 0 <= i < a.len() implies same_derived_byte(#[trigger] a[i], b[i]) by {
            assert(derive_spec(a)[i] == derive_spec(b)[i]);
        }
    }
}

/// A tag containing a non-ASCII char never derives a portable name: the
/// production `is_ascii` pre-check is redundant with the grammar.
pub proof fn lemma_non_ascii_tag_not_derivable(tag: Seq<char>)
    requires
        !vstd::utf8::is_ascii_chars(tag),
    ensures
        !is_portable_env_name(derive_spec(vstd::utf8::encode_utf8(tag))),
{
    let bytes = vstd::utf8::encode_utf8(tag);
    if is_portable_env_name(derive_spec(bytes)) {
        lemma_portable_is_shell_inert(derive_spec(bytes));
        assert forall|i: int| 0 <= i < bytes.len() implies #[trigger] bytes[i] < 0x80 by {
            assert(derive_spec(bytes)[i] < 0x80);
        }
        text::lemma_ascii_encoding_is_ascii_chars(tag);
    }
}

// ---------------------------------------------------------------------------
// Resolution and collision keys
// ---------------------------------------------------------------------------

/// Why no name could be resolved for a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The explicit mapping is not a portable name.
    InvalidExplicit(EnvNameError),
    /// The unmapped tag does not derive a portable name.
    NonConvertibleTag,
}

/// Resolution: an explicit mapping is validated on its own and used verbatim
/// (never falling back to the tag); otherwise the derived name is used.
pub open spec fn resolve_verdict(tag: Seq<u8>, explicit: Option<Seq<u8>>) -> Result<
    Seq<u8>,
    ResolveError,
> {
    match explicit {
        Some(e) => match env_name_verdict(e) {
            Ok(()) => Ok(e),
            Err(err) => Err(ResolveError::InvalidExplicit(err)),
        },
        None => if is_portable_env_name(derive_spec(tag)) {
            Ok(derive_spec(tag))
        } else {
            Err(ResolveError::NonConvertibleTag)
        },
    }
}

/// With an explicit mapping the outcome does not depend on the tag at all.
pub proof fn lemma_explicit_mapping_ignores_tag(t1: Seq<u8>, t2: Seq<u8>, e: Seq<u8>)
    ensures
        resolve_verdict(t1, Some(e)) == resolve_verdict(t2, Some(e)),
{
}

/// Every resolved name is portable (and so shell-inert).
pub proof fn lemma_resolved_is_portable(tag: Seq<u8>, explicit: Option<Seq<u8>>)
    ensures
        resolve_verdict(tag, explicit) matches Ok(n) ==> is_portable_env_name(n),
{
}

/// For a portable name, the collision key's bytes are the byte-wise ASCII
/// uppercase of the name's bytes, and the key is itself portable.
pub proof fn lemma_collision_key_of_portable(name: Seq<char>)
    requires
        is_portable_env_name(vstd::utf8::encode_utf8(name)),
    ensures
        vstd::utf8::encode_utf8(text::ascii_upper_chars(name)) == text::ascii_upper_seq(
            vstd::utf8::encode_utf8(name),
        ),
        is_portable_env_name(vstd::utf8::encode_utf8(text::ascii_upper_chars(name))),
{
    broadcast use vstd::utf8::char_u32_cast, vstd::utf8::char_is_scalar;

    let bytes = vstd::utf8::encode_utf8(name);
    lemma_portable_is_shell_inert(bytes);
    lemma_portable_is_ascii(name);
    let upper = text::ascii_upper_seq(bytes);
    assert forall|i: int| 0 <= i < upper.len() implies #[trigger] upper[i] < 0x80 by {
        assert(bytes[i] < 0x80);
    }
    text::lemma_ascii_bytes_round_trip(upper);
    assert forall|i: int| 0 <= i < name.len() implies #[trigger] text::ascii_upper_chars(name)[i]
        == text::chars_of_bytes(upper)[i] by {
        let b = bytes[i];
        assert(name[i] == b as char);
        let u = b as u32;
        if text::is_ascii_lower_byte(b) {
            assert(((u - 32) as u32) as char == ((b - 0x20) as u8) as char);
        }
    }
    assert(text::ascii_upper_chars(name) =~= text::chars_of_bytes(upper));
}

/// Collision keys of two portable names are equal exactly when the names are
/// equal ignoring ASCII case, both as chars and as bytes.
pub proof fn lemma_collision_key_eq_iff(a: Seq<char>, b: Seq<char>)
    requires
        is_portable_env_name(vstd::utf8::encode_utf8(a)),
        is_portable_env_name(vstd::utf8::encode_utf8(b)),
    ensures
        (text::ascii_upper_chars(a) == text::ascii_upper_chars(b))
            <==> text::eq_ignore_ascii_case_chars(a, b),
        (text::ascii_upper_chars(a) == text::ascii_upper_chars(b)) <==> text::eq_ignore_ascii_case(
            vstd::utf8::encode_utf8(a),
            vstd::utf8::encode_utf8(b),
        ),
{
    text::lemma_ascii_upper_chars_eq_iff(a, b);
    lemma_collision_key_of_portable(a);
    lemma_collision_key_of_portable(b);
    text::lemma_ascii_upper_seq_eq_iff(vstd::utf8::encode_utf8(a), vstd::utf8::encode_utf8(b));
    lemma_portable_is_ascii(a);
    lemma_portable_is_ascii(b);
    let (bytes_a, bytes_b) = (vstd::utf8::encode_utf8(a), vstd::utf8::encode_utf8(b));
    if text::ascii_upper_seq(bytes_a) == text::ascii_upper_seq(bytes_b) {
        lemma_portable_is_ascii(text::ascii_upper_chars(a));
        lemma_portable_is_ascii(text::ascii_upper_chars(b));
    }
}

// ---------------------------------------------------------------------------
// Executable functions
// ---------------------------------------------------------------------------

/// Whether `b` may start a name.
const fn env_head_byte(b: u8) -> (head: bool)
    ensures
        head == is_env_head(b),
{
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'_')
}

/// Whether `b` may continue a name.
const fn env_tail_byte(b: u8) -> (tail: bool)
    ensures
        tail == is_env_tail(b),
{
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'0'..=b'9')
}

/// One byte of the tag-to-name derivation.
const fn derive_one(b: u8) -> (d: u8)
    ensures
        d == derive_byte(b),
{
    match b {
        b'-' => b'_',
        b'a'..=b'z' => b - 0x20,
        _ => b,
    }
}

/// The grammar check on raw bytes (see `env_name_verdict`).
pub fn check_env_name_bytes(name: &[u8]) -> (r: Result<(), EnvNameError>)
    ensures
        r == env_name_verdict(name@),
        r is Ok <==> is_portable_env_name(name@),
{
    if name.is_empty() {
        return Err(EnvNameError::Empty);
    }
    if name.len() > MAX_ENV_NAME_BYTES {
        return Err(EnvNameError::TooLong);
    }
    if !env_head_byte(name[0]) {
        return Err(EnvNameError::BadFirst);
    }
    let mut i: usize = 1;
    while i < name.len()
        invariant
            1 <= i <= name.len(),
            name.len() <= MAX_ENV_NAME_BYTES,
            is_env_head(name@[0]),
            forall|j: int| 1 <= j < i ==> is_env_tail(#[trigger] name@[j]),
        decreases name.len() - i,
    {
        if !env_tail_byte(name[i]) {
            proof {
                assert(1 <= i < name@.len() && !is_env_tail(name@[i as int]));
                assert(has_bad_rest(name@));
            }
            return Err(EnvNameError::BadRest);
        }
        i += 1;
    }
    Ok(())
}

/// The grammar check on a name's UTF-8 bytes (the bytes written to shells).
pub fn check_env_name(name: &str) -> (r: Result<(), EnvNameError>)
    ensures
        r == env_name_verdict(vstd::utf8::encode_utf8(name@)),
        r is Ok <==> is_portable_env_name(vstd::utf8::encode_utf8(name@)),
        r is Ok ==> vstd::utf8::is_ascii_chars(name@) && text::chars_of_bytes(
            vstd::utf8::encode_utf8(name@),
        ) == name@,
{
    let r = check_env_name_bytes(name.as_bytes());
    proof {
        if r is Ok {
            lemma_portable_is_ascii(name@);
        }
    }
    r
}

/// Derives a portable name from an unmapped service tag: `Some` exactly when
/// the derived bytes (`derive_spec`) are a portable name, which is then
/// returned.
pub fn derive_env_name(tag: &str) -> (r: Option<String>)
    ensures
        r is Some <==> is_portable_env_name(derive_spec(vstd::utf8::encode_utf8(tag@))),
        r matches Some(n) ==> vstd::utf8::encode_utf8(n@) == derive_spec(
            vstd::utf8::encode_utf8(tag@),
        ),
{
    let bytes = tag.as_bytes();
    let mut derived: Vec<u8> = Vec::new();
    let mut i: usize = 0;
    while i < bytes.len()
        invariant
            0 <= i <= bytes.len(),
            derived@ == derive_spec(bytes@.subrange(0, i as int)),
        decreases bytes.len() - i,
    {
        derived.push(derive_one(bytes[i]));
        proof {
            assert(derive_spec(bytes@.subrange(0, i + 1)) =~= derive_spec(
                bytes@.subrange(0, i as int),
            ).push(derive_byte(bytes@[i as int])));
        }
        i += 1;
    }
    proof {
        assert(bytes@.subrange(0, bytes@.len() as int) =~= bytes@);
    }
    match check_env_name_bytes(derived.as_slice()) {
        Ok(()) => {
            proof {
                lemma_portable_is_shell_inert(derived@);
                text::lemma_ascii_bytes_round_trip(derived@);
            }
            Some(text::string_from_ascii(derived.as_slice()))
        },
        Err(_) => None,
    }
}

/// Resolves the name for a service: the explicit mapping when present
/// (validated, never falling back to the tag), otherwise the derived name.
pub fn resolve_env_name(tag: &str, explicit: Option<&str>) -> (r: Result<String, ResolveError>)
    ensures
        r matches Ok(n) ==> resolve_verdict(
            vstd::utf8::encode_utf8(tag@),
            match explicit {
                Some(e) => Some(vstd::utf8::encode_utf8(e@)),
                None => None,
            },
        ) == Ok::<Seq<u8>, ResolveError>(vstd::utf8::encode_utf8(n@)),
        r matches Err(e) ==> resolve_verdict(
            vstd::utf8::encode_utf8(tag@),
            match explicit {
                Some(e) => Some(vstd::utf8::encode_utf8(e@)),
                None => None,
            },
        ) == Err::<Seq<u8>, ResolveError>(e),
        explicit matches Some(e) ==> (r matches Ok(n) ==> n@ == e@),
{
    match explicit {
        Some(name) => match check_env_name(name) {
            Ok(()) => Ok(name.to_owned()),
            Err(error) => Err(ResolveError::InvalidExplicit(error)),
        },
        None => match derive_env_name(tag) {
            Some(name) => Ok(name),
            None => Err(ResolveError::NonConvertibleTag),
        },
    }
}

/// The ASCII-case-insensitive collision key of a name (its ASCII uppercase).
pub fn env_collision_key(name: &str) -> (key: String)
    ensures
        key@ == text::ascii_upper_chars(name@),
        is_portable_env_name(vstd::utf8::encode_utf8(name@)) ==> is_portable_env_name(
            vstd::utf8::encode_utf8(key@),
        ) && vstd::utf8::encode_utf8(key@) == text::ascii_upper_seq(
            vstd::utf8::encode_utf8(name@),
        ),
{
    let key = text::ascii_uppercase(name);
    proof {
        text::lemma_ascii_upper_chars_idempotent(name@);
        if is_portable_env_name(vstd::utf8::encode_utf8(name@)) {
            lemma_collision_key_of_portable(name@);
        }
    }
    key
}

} // verus!
