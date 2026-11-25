//! Verified kernel: ASCII and UTF-8 text helpers shared by other kernels.
//!
//! - ASCII case folding over bytes (`ascii_upper`) and over `char`s
//!   (`ascii_upper_char`), with the characterization "two strings fold to
//!   the same key exactly when they are equal ignoring ASCII case".
//! - ASCII bridging: a byte string whose bytes are all below `0x80` is exactly
//!   the UTF-8 encoding of the same characters, and conversely a string whose
//!   UTF-8 encoding is all-ASCII consists of ASCII characters.
//! - UTF-8 prefix boundaries: every `char` prefix of a string ends on a
//!   character boundary of its UTF-8 encoding, which lets kernels slice a
//!   `&str` with `split_at` without any possibility of a panic.
//!
//! The UTF-8 model (`encode_utf8`, `is_char_boundary`, ...) is `vstd`'s; the
//! `str`/`String`/`char` operations used here are specified by `vstd`.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// ASCII case folding over bytes
// ---------------------------------------------------------------------------

/// `b` is an ASCII lowercase letter (`a..=z`).
pub open spec fn is_ascii_lower_byte(b: u8) -> bool {
    0x61 <= b <= 0x7A
}

/// `b` is an ASCII uppercase letter (`A..=Z`).
pub open spec fn is_ascii_upper_byte(b: u8) -> bool {
    0x41 <= b <= 0x5A
}

/// ASCII uppercase of one byte: `a..=z` become `A..=Z`, every other byte
/// (including every non-ASCII byte) is unchanged. This is `u8::to_ascii_uppercase`.
pub open spec fn ascii_upper(b: u8) -> u8 {
    if is_ascii_lower_byte(b) {
        (b - 0x20) as u8
    } else {
        b
    }
}

/// Byte-wise ASCII uppercase of a byte string.
pub open spec fn ascii_upper_seq(s: Seq<u8>) -> Seq<u8> {
    s.map_values(|b: u8| ascii_upper(b))
}

/// `a` and `b` are equal when ASCII letters are compared without case
/// (`<[u8]>::eq_ignore_ascii_case`).
pub open spec fn eq_ignore_ascii_case(a: Seq<u8>, b: Seq<u8>) -> bool {
    a.len() == b.len() && forall|i: int|
        0 <= i < a.len() ==> ascii_upper(#[trigger] a[i]) == ascii_upper(b[i])
}

/// Every byte of `s` is ASCII (`< 0x80`).
pub open spec fn is_ascii_bytes(s: Seq<u8>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] < 0x80
}

/// `ascii_upper` changes only ASCII lowercase letters, maps them to the
/// matching uppercase letter, and is idempotent.
pub proof fn lemma_ascii_upper_byte(b: u8)
    ensures
        ascii_upper(b) != b <==> is_ascii_lower_byte(b),
        is_ascii_lower_byte(b) ==> is_ascii_upper_byte(ascii_upper(b)) && ascii_upper(b) == b
            - 0x20,
        !is_ascii_lower_byte(ascii_upper(b)),
        ascii_upper(ascii_upper(b)) == ascii_upper(b),
        (b < 0x80) == (ascii_upper(b) < 0x80),
{
}

/// Folding is idempotent: `ascii_upper_seq(ascii_upper_seq(s)) == ascii_upper_seq(s)`.
pub proof fn lemma_ascii_upper_seq_idempotent(s: Seq<u8>)
    ensures
        ascii_upper_seq(ascii_upper_seq(s)) == ascii_upper_seq(s),
        ascii_upper_seq(s).len() == s.len(),
{
    assert(ascii_upper_seq(ascii_upper_seq(s)) =~= ascii_upper_seq(s));
}

/// Two byte strings fold to the same key exactly when they are equal ignoring
/// ASCII case.
pub proof fn lemma_ascii_upper_seq_eq_iff(a: Seq<u8>, b: Seq<u8>)
    ensures
        (ascii_upper_seq(a) == ascii_upper_seq(b)) <==> eq_ignore_ascii_case(a, b),
{
    if eq_ignore_ascii_case(a, b) {
        assert(ascii_upper_seq(a) =~= ascii_upper_seq(b));
    }
    if ascii_upper_seq(a) == ascii_upper_seq(b) {
        assert(a.len() == ascii_upper_seq(a).len() && b.len() == ascii_upper_seq(b).len());
        assert forall|i: int| 0 <= i < a.len() implies ascii_upper(#[trigger] a[i]) == ascii_upper(
            b[i],
        ) by {
            assert(ascii_upper_seq(a)[i] == ascii_upper_seq(b)[i]);
        }
    }
}

/// `eq_ignore_ascii_case` is an equivalence relation (reflexive, symmetric,
/// transitive): it is equality of the folded keys.
pub proof fn lemma_eq_ignore_ascii_case_equivalence(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    ensures
        eq_ignore_ascii_case(a, a),
        eq_ignore_ascii_case(a, b) ==> eq_ignore_ascii_case(b, a),
        eq_ignore_ascii_case(b, a) && eq_ignore_ascii_case(b, c) ==> eq_ignore_ascii_case(a, c),
{
    lemma_ascii_upper_seq_eq_iff(a, a);
    lemma_ascii_upper_seq_eq_iff(a, b);
    lemma_ascii_upper_seq_eq_iff(b, a);
    lemma_ascii_upper_seq_eq_iff(b, c);
    lemma_ascii_upper_seq_eq_iff(a, c);
}

// ---------------------------------------------------------------------------
// ASCII case folding over chars
// ---------------------------------------------------------------------------

/// ASCII uppercase of one `char`: `a..=z` become `A..=Z`, every other `char`
/// is unchanged. This is `char::to_ascii_uppercase`.
pub open spec fn ascii_upper_char(c: char) -> char {
    if 'a' <= c && c <= 'z' {
        (c as u32 - 32) as char
    } else {
        c
    }
}

/// `char`-wise ASCII uppercase of a string (`str::to_ascii_uppercase`).
pub open spec fn ascii_upper_chars(s: Seq<char>) -> Seq<char> {
    s.map_values(|c: char| ascii_upper_char(c))
}

/// `a` and `b` are equal when ASCII letters are compared without case
/// (`str::eq_ignore_ascii_case`).
pub open spec fn eq_ignore_ascii_case_chars(a: Seq<char>, b: Seq<char>) -> bool {
    a.len() == b.len() && forall|i: int|
        0 <= i < a.len() ==> ascii_upper_char(#[trigger] a[i]) == ascii_upper_char(b[i])
}

/// `ascii_upper_char` changes only ASCII lowercase letters and is idempotent.
pub proof fn lemma_ascii_upper_char(c: char)
    ensures
        ascii_upper_char(c) != c <==> ('a' <= c && c <= 'z'),
        !('a' <= ascii_upper_char(c) && ascii_upper_char(c) <= 'z'),
        ascii_upper_char(ascii_upper_char(c)) == ascii_upper_char(c),
        (c as u32) < 0x80 <==> (ascii_upper_char(c) as u32) < 0x80,
{
    broadcast use vstd::utf8::char_u32_cast, vstd::utf8::char_is_scalar;

    if 'a' <= c && c <= 'z' {
        let u = (c as u32 - 32) as u32;
        assert(vstd::utf8::is_scalar(u));
        vstd::utf8::char_u32_cast((u as char), u);
    }
}

/// Folding is idempotent.
pub proof fn lemma_ascii_upper_chars_idempotent(s: Seq<char>)
    ensures
        ascii_upper_chars(ascii_upper_chars(s)) == ascii_upper_chars(s),
        ascii_upper_chars(s).len() == s.len(),
{
    assert forall|i: int| 0 <= i < s.len() implies #[trigger] ascii_upper_chars(
        ascii_upper_chars(s),
    )[i] == ascii_upper_chars(s)[i] by {
        lemma_ascii_upper_char(s[i]);
    }
    assert(ascii_upper_chars(ascii_upper_chars(s)) =~= ascii_upper_chars(s));
}

/// Two strings fold to the same key exactly when they are equal ignoring
/// ASCII case.
pub proof fn lemma_ascii_upper_chars_eq_iff(a: Seq<char>, b: Seq<char>)
    ensures
        (ascii_upper_chars(a) == ascii_upper_chars(b)) <==> eq_ignore_ascii_case_chars(a, b),
{
    if eq_ignore_ascii_case_chars(a, b) {
        assert(ascii_upper_chars(a) =~= ascii_upper_chars(b));
    }
    if ascii_upper_chars(a) == ascii_upper_chars(b) {
        assert(a.len() == ascii_upper_chars(a).len() && b.len() == ascii_upper_chars(b).len());
        assert forall|i: int| 0 <= i < a.len() implies ascii_upper_char(#[trigger] a[i])
            == ascii_upper_char(b[i]) by {
            assert(ascii_upper_chars(a)[i] == ascii_upper_chars(b)[i]);
        }
    }
}

// ---------------------------------------------------------------------------
// ASCII bridging between bytes and chars
// ---------------------------------------------------------------------------

/// The `char`s whose code points are the given bytes (Latin-1 reading; for
/// ASCII bytes this is the UTF-8 decoding, see `lemma_ascii_bytes_round_trip`).
pub open spec fn chars_of_bytes(bytes: Seq<u8>) -> Seq<char> {
    bytes.map_values(|b: u8| b as char)
}

/// An ASCII byte string is exactly the UTF-8 encoding of the `char`s with the
/// same code points.
pub proof fn lemma_ascii_bytes_round_trip(bytes: Seq<u8>)
    requires
        is_ascii_bytes(bytes),
    ensures
        vstd::utf8::is_ascii_chars(chars_of_bytes(bytes)),
        vstd::utf8::encode_utf8(chars_of_bytes(bytes)) == bytes,
{
    let chars = chars_of_bytes(bytes);
    assert forall|i: int| 0 <= i < chars.len() implies '\0' <= #[trigger] chars[i] <= '\u{7f}' by {
        assert(bytes[i] < 0x80);
    }
    vstd::utf8::is_ascii_chars_encode_utf8(chars);
    assert forall|i: int| 0 <= i < bytes.len() implies #[trigger] vstd::utf8::encode_utf8(chars)[i]
        == bytes[i] by {
        assert(chars[i] as u8 == vstd::utf8::encode_utf8(chars)[i]);
    }
    assert(vstd::utf8::encode_utf8(chars) =~= bytes);
}

/// The first UTF-8 byte of a non-ASCII scalar value is at least `0xC0`.
proof fn lemma_non_ascii_leading_byte(v: u32)
    requires
        vstd::utf8::is_scalar(v),
        v >= 0x80,
    ensures
        vstd::utf8::encode_scalar(v)[0] >= 0xC0,
{
    if vstd::utf8::has_width_2_encoding(v) {
        let x = ((v >> 6) & 0x1F) as u8;
        assert(0xC0u8 | x >= 0xC0u8) by (bit_vector);
    } else if vstd::utf8::has_width_3_encoding(v) {
        let x = ((v >> 12) & 0x0F) as u8;
        assert(0xE0u8 | x >= 0xC0u8) by (bit_vector);
    } else {
        let x = ((v >> 18) & 0x7) as u8;
        assert(0xF0u8 | x >= 0xC0u8) by (bit_vector);
    }
}

/// A string whose UTF-8 encoding is all-ASCII consists of ASCII `char`s,
/// encoded one byte per `char` with the same code point.
pub proof fn lemma_ascii_encoding_is_ascii_chars(s: Seq<char>)
    requires
        is_ascii_bytes(vstd::utf8::encode_utf8(s)),
    ensures
        vstd::utf8::is_ascii_chars(s),
        vstd::utf8::encode_utf8(s).len() == s.len(),
        chars_of_bytes(vstd::utf8::encode_utf8(s)) == s,
    decreases s.len(),
{
    broadcast use vstd::utf8::char_is_scalar, vstd::utf8::char_u32_cast;

    if s.len() > 0 {
        let c = s[0];
        let rest = s.drop_first();
        let enc = vstd::utf8::encode_utf8(s);
        assert(enc == vstd::utf8::encode_scalar(c as u32) + vstd::utf8::encode_utf8(rest));
        if (c as u32) >= 0x80 {
            lemma_non_ascii_leading_byte(c as u32);
            assert(enc[0] == vstd::utf8::encode_scalar(c as u32)[0]);
            assert(false);
        }
        assert(vstd::utf8::encode_scalar(c as u32).len() == 1);
        let enc_rest = vstd::utf8::encode_utf8(rest);
        assert forall|i: int| 0 <= i < enc_rest.len() implies #[trigger] enc_rest[i] < 0x80 by {
            assert(enc_rest[i] == enc[i + 1]);
        }
        lemma_ascii_encoding_is_ascii_chars(rest);
        let u = c as u32;
        assert(enc[0] == (u & 0x7F) as u8);
        assert((u & 0x7F) == u) by (bit_vector)
            requires
                u < 0x80,
        ;
        assert(chars_of_bytes(enc)[0] == c);
        assert(chars_of_bytes(enc) =~= s) by {
            assert forall|i: int| 1 <= i < s.len() implies #[trigger] chars_of_bytes(enc)[i]
                == s[i] by {
                assert(chars_of_bytes(enc)[i] == chars_of_bytes(enc_rest)[i - 1]);
            }
        }
    } else {
        assert(chars_of_bytes(vstd::utf8::encode_utf8(s)) =~= s);
    }
}

// ---------------------------------------------------------------------------
// UTF-8 prefix boundaries
// ---------------------------------------------------------------------------

/// The number of UTF-8 bytes of a `char` sequence.
pub open spec fn utf8_len(s: Seq<char>) -> nat {
    vstd::utf8::encode_utf8(s).len()
}

/// Splitting a string after `k` chars splits its UTF-8 encoding after
/// `utf8_len(s[..k])` bytes, and that byte offset is a character boundary.
pub proof fn lemma_prefix_boundary(s: Seq<char>, k: int)
    requires
        0 <= k <= s.len(),
    ensures
        vstd::utf8::encode_utf8(s) == vstd::utf8::encode_utf8(s.subrange(0, k))
            + vstd::utf8::encode_utf8(s.subrange(k, s.len() as int)),
        vstd::utf8::is_char_boundary(vstd::utf8::encode_utf8(s), utf8_len(s.subrange(0, k)) as int),
        utf8_len(s.subrange(0, k)) <= utf8_len(s),
    decreases k,
{
    broadcast use vstd::utf8::encode_utf8_valid_utf8;

    assert(s =~= s.subrange(0, k) + s.subrange(k, s.len() as int));
    vstd::utf8::encode_utf8_concat(s.subrange(0, k), s.subrange(k, s.len() as int));
    if k == 0 {
        assert(s.subrange(0, 0) =~= Seq::<char>::empty());
        vstd::utf8::is_char_boundary_start_end_of_seq(vstd::utf8::encode_utf8(s));
    } else {
        let bytes = vstd::utf8::encode_utf8(s);
        let rest = s.drop_first();
        vstd::utf8::encode_utf8_first_scalar(s);
        let w = vstd::utf8::encode_scalar(s[0] as u32).len() as int;
        assert(bytes == vstd::utf8::encode_scalar(s[0] as u32) + vstd::utf8::encode_utf8(rest));
        assert(vstd::utf8::pop_first_scalar(bytes) =~= vstd::utf8::encode_utf8(rest));
        lemma_prefix_boundary(rest, k - 1);
        assert(s.subrange(0, k) =~= seq![s[0]] + rest.subrange(0, k - 1));
        vstd::utf8::encode_utf8_concat(seq![s[0]], rest.subrange(0, k - 1));
        vstd::utf8::encode_utf8_push(Seq::<char>::empty(), s[0]);
        assert(Seq::<char>::empty().push(s[0]) =~= seq![s[0]]);
        assert(vstd::utf8::encode_utf8(Seq::<char>::empty()) =~= Seq::<u8>::empty());
        assert(vstd::utf8::encode_utf8(seq![s[0]]) =~= vstd::utf8::encode_scalar(s[0] as u32));
        assert(utf8_len(s.subrange(0, k)) == w + utf8_len(rest.subrange(0, k - 1)));
        assert(w >= 1);
    }
}

/// Byte offsets of char prefixes are strictly increasing; in particular two
/// prefixes with the same UTF-8 length have the same number of chars.
pub proof fn lemma_prefix_len_injective(s: Seq<char>, i: int, j: int)
    requires
        0 <= i <= s.len(),
        0 <= j <= s.len(),
        utf8_len(s.subrange(0, i)) == utf8_len(s.subrange(0, j)),
    ensures
        i == j,
{
    if i < j {
        vstd::utf8::lemma_encode_utf8_len_strictly_monotonic(s, i, j);
        assert(s.subrange(0, i) == s.subrange(0, i));
    } else if j < i {
        vstd::utf8::lemma_encode_utf8_len_strictly_monotonic(s, j, i);
    }
}

/// `offset` is the UTF-8 byte offset at which char `k` of `s` starts (or
/// the end of the encoding, for `k == s.len()`).
pub open spec fn is_char_offset(s: Seq<char>, k: int, offset: int) -> bool {
    0 <= k <= s.len() && offset == utf8_len(s.subrange(0, k))
}

/// A `&str` whose encoding is the encoding of `s[..k]` holds exactly the
/// chars `s[..k]` (UTF-8 encoding is injective).
proof fn lemma_view_of_prefix(s: Seq<char>, k: int, t: Seq<char>)
    requires
        0 <= k <= s.len(),
        vstd::utf8::encode_utf8(t) == vstd::utf8::encode_utf8(s).subrange(
            0,
            utf8_len(s.subrange(0, k)) as int,
        ),
    ensures
        t == s.subrange(0, k),
{
    lemma_prefix_boundary(s, k);
    assert(vstd::utf8::encode_utf8(t) =~= vstd::utf8::encode_utf8(s.subrange(0, k)));
    vstd::utf8::encode_utf8_decode_utf8(t);
    vstd::utf8::encode_utf8_decode_utf8(s.subrange(0, k));
}

/// A `&str` whose encoding is the encoding of `s[k..]` holds exactly the
/// chars `s[k..]`.
proof fn lemma_view_of_suffix(s: Seq<char>, k: int, t: Seq<char>)
    requires
        0 <= k <= s.len(),
        vstd::utf8::encode_utf8(t) == vstd::utf8::encode_utf8(s).subrange(
            utf8_len(s.subrange(0, k)) as int,
            utf8_len(s) as int,
        ),
    ensures
        t == s.subrange(k, s.len() as int),
{
    lemma_prefix_boundary(s, k);
    assert(vstd::utf8::encode_utf8(t) =~= vstd::utf8::encode_utf8(s.subrange(k, s.len() as int)));
    vstd::utf8::encode_utf8_decode_utf8(t);
    vstd::utf8::encode_utf8_decode_utf8(s.subrange(k, s.len() as int));
}

// ---------------------------------------------------------------------------
// Executable helpers
// ---------------------------------------------------------------------------

/// The sub-slice of `s` holding its chars `a..b`, located by their UTF-8
/// byte offsets `start` and `end`. Kernel-internal: callers prove the
/// offsets are char offsets, so the slicing cannot panic.
pub(in crate::verified) fn char_slice(s: &str, start: usize, end: usize) -> (t: &str)
    requires
        exists|a: int, b: int|
            a <= b && is_char_offset(s@, a, start as int) && is_char_offset(s@, b, end as int),
    ensures
        forall|a: int, b: int|
            is_char_offset(s@, a, start as int) && is_char_offset(s@, b, end as int) ==> t@
                == s@.subrange(a, b),
{
    let ghost (a, b) = choose|a: int, b: int|
        a <= b && is_char_offset(s@, a, start as int) && is_char_offset(s@, b, end as int);
    proof {
        lemma_prefix_boundary(s@, b);
    }
    let (head, _) = s.split_at(end);
    proof {
        lemma_view_of_prefix(s@, b, head@);
        lemma_prefix_boundary(head@, a);
        assert(head@.subrange(0, a) =~= s@.subrange(0, a));
    }
    let (_, middle) = head.split_at(start);
    proof {
        lemma_view_of_suffix(head@, a, middle@);
        assert(head@.subrange(a, head@.len() as int) =~= s@.subrange(a, b));
        assert forall|a2: int, b2: int|
            is_char_offset(s@, a2, start as int) && is_char_offset(
                s@,
                b2,
                end as int,
            ) implies s@.subrange(a2, b2) == s@.subrange(a, b) by {
            lemma_prefix_len_injective(s@, a, a2);
            lemma_prefix_len_injective(s@, b, b2);
        }
    }
    middle
}

/// `str::to_ascii_uppercase`, computed char by char.
pub fn ascii_uppercase(s: &str) -> (upper: String)
    ensures
        upper@ == ascii_upper_chars(s@),
{
    let mut upper = String::new();
    let mut chars = s.chars();
    let ghost n = s@.len() as int;
    let ghost mut k: int = 0;
    let mut done = false;
    while !done
        invariant
            0 <= k <= n,
            n == s@.len(),
            vstd::std_specs::iter::IteratorSpec::remaining(&chars) == s@.subrange(k, n),
            upper@ == ascii_upper_chars(s@.subrange(0, k)),
            done ==> k == n,
        decreases n - k + if done { 0int } else { 1int },
    {
        match chars.next() {
            Some(c) => {
                upper.push(c.to_ascii_uppercase());
                proof {
                    assert(s@.subrange(0, k + 1) =~= s@.subrange(0, k).push(c));
                    assert(ascii_upper_chars(s@.subrange(0, k + 1)) =~= ascii_upper_chars(
                        s@.subrange(0, k),
                    ).push(ascii_upper_char(c)));
                    k = k + 1;
                };
            },
            None => {
                done = true;
            },
        }
    }
    proof {
        assert(s@.subrange(0, n) =~= s@);
    }
    upper
}

/// `u8::to_ascii_uppercase`.
pub const fn upper_byte(b: u8) -> (u: u8)
    ensures
        u == ascii_upper(b),
{
    if b'a' <= b && b <= b'z' {
        b - 0x20
    } else {
        b
    }
}

/// `<[u8]>::eq_ignore_ascii_case`: equal lengths and byte-wise equal after
/// ASCII uppercasing.
pub fn eq_ignore_ascii_case_bytes(a: &[u8], b: &[u8]) -> (r: bool)
    ensures
        r == eq_ignore_ascii_case(a@, b@),
{
    if a.len() != b.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < a.len()
        invariant
            0 <= i <= a.len() == b.len(),
            forall|j: int| 0 <= j < i ==> ascii_upper(#[trigger] a@[j]) == ascii_upper(b@[j]),
        decreases a.len() - i,
    {
        if upper_byte(a[i]) != upper_byte(b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// The string whose `char`s have the given bytes as code points. For ASCII
/// input this is the exact inverse of UTF-8 encoding (see
/// `lemma_ascii_bytes_round_trip`).
pub fn string_from_ascii(bytes: &[u8]) -> (text: String)
    ensures
        text@ == chars_of_bytes(bytes@),
{
    let mut text = String::new();
    let mut i: usize = 0;
    while i < bytes.len()
        invariant
            0 <= i <= bytes.len(),
            text@ == chars_of_bytes(bytes@.subrange(0, i as int)),
        decreases bytes.len() - i,
    {
        text.push(bytes[i] as char);
        proof {
            assert(chars_of_bytes(bytes@.subrange(0, i + 1)) =~= chars_of_bytes(
                bytes@.subrange(0, i as int),
            ).push(bytes@[i as int] as char));
        }
        i += 1;
    }
    proof {
        assert(bytes@.subrange(0, bytes@.len() as int) =~= bytes@);
    }
    text
}

} // verus!
