//! Verified kernel: the JSON rendering of a port map (`reserve-group
//! --format=json`).
//!
//! - `json_escape_char` / `json_escape_body`: `serde_json`'s string escaping
//!   (`\"`, `\\`, `\b`, `\t`, `\n`, `\f`, `\r`, `\u00xx` with lowercase hex
//!   for the other C0 controls; everything else, including DEL, non-ASCII
//!   text and `U+2028`/`U+2029`, as is). Its UTF-8 text is exactly
//!   `serde_json`'s byte-table escaping of the string's UTF-8 text
//!   (`json_escape_bytes`, `lemma_escape_matches_serde_bytes`).
//! - `json_scan_string`: the RFC 8259 §7 grammar for the characters after a
//!   string's opening quote, as a scanner that stops at the first unescaped
//!   quote and decodes every escape (both hex cases; surrogate `\u` escapes
//!   are outside it). The escaped text of every string scans back to exactly
//!   that string, ends at its closing quote whatever follows, and contains
//!   no raw control character (`lemma_json_string_round_trip`), so every key
//!   is a valid JSON string token that names the original tag, and distinct
//!   tags give distinct tokens (`lemma_json_string_injective`).
//! - `json_object_pretty`: `{}` for no entries, otherwise
//!   `{\n  "k": v,\n  ...\n}`, `serde_json`'s `PrettyFormatter` layout for a
//!   flat object, with each value the canonical decimal text of the port.
//! - `read_json_object`: RFC 8259 for a JSON text that is one object whose
//!   values are non-negative integers (`int` without sign, fraction or
//!   exponent), with optional whitespace around every structural character.
//!   The rendered object reads back as exactly its entries, in order, each
//!   port as its number (`lemma_object_reads_back`): the output is valid
//!   JSON and loses nothing.
//! - `render_port_map_pretty`: renders entries in byte order of their keys
//!   (`verified::render::sort_order`, the order of a `BTreeMap<String, _>`),
//!   so for distinct keys the keys strictly increase
//!   (`lemma_sorted_keys_increase`) and the text does not depend on the order
//!   of the input (`lemma_render_order_independent`); the text reads back as
//!   the sorted entries (`lemma_port_map_reads_back`).
//!
//! Production glue: `JsonFormatter` (`trop/src/output/formatters.rs`).
//! Byte equality with `serde_json::to_string_pretty` of the same
//! `BTreeMap<String, u16>` (the layout, and the `ESCAPE` table as transcribed
//! in `json_escape_byte`) is pinned by differential tests, not proved.

use vstd::prelude::*;

use super::render;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

/// The lowercase hex digit of `d` (`0..=15`).
pub open spec fn hex_digit(d: int) -> char {
    render::hex_char(d as u8)
}

/// How `serde_json` writes one character inside a string.
pub open spec fn json_escape_char(c: char) -> Seq<char> {
    if c == '"' {
        seq!['\\', '"']
    } else if c == '\\' {
        seq!['\\', '\\']
    } else if c == '\u{8}' {
        seq!['\\', 'b']
    } else if c == '\t' {
        seq!['\\', 't']
    } else if c == '\n' {
        seq!['\\', 'n']
    } else if c == '\u{c}' {
        seq!['\\', 'f']
    } else if c == '\r' {
        seq!['\\', 'r']
    } else if (c as u32) < 0x20 {
        seq!['\\', 'u', '0', '0', hex_digit((c as u32) as int / 16), hex_digit((c as u32) as int % 16)]
    } else {
        seq![c]
    }
}

/// The escaped characters of a string (without the quotes).
pub open spec fn json_escape_body(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() == 0 {
        Seq::empty()
    } else {
        json_escape_body(s.drop_last()) + json_escape_char(s.last())
    }
}

/// A JSON string token: the escaped characters between quotes.
pub open spec fn json_string(s: Seq<char>) -> Seq<char> {
    seq!['"'] + json_escape_body(s) + seq!['"']
}

// ---------------------------------------------------------------------------
// The RFC 8259 string grammar
// ---------------------------------------------------------------------------

/// The value of a hex digit (`0-9`, `a-f`, `A-F`).
pub open spec fn hex_value(c: char) -> Option<int> {
    if '0' <= c && c <= '9' {
        Some(c as int - 48)
    } else if 'a' <= c && c <= 'f' {
        Some(c as int - 87)
    } else if 'A' <= c && c <= 'F' {
        Some(c as int - 55)
    } else {
        None
    }
}

/// The value of four hex digits.
pub open spec fn hex4_value(t: Seq<char>) -> Option<int> {
    match (hex_value(t[0]), hex_value(t[1]), hex_value(t[2]), hex_value(t[3])) {
        (Some(a), Some(b), Some(c), Some(d)) => Some(((a * 16 + b) * 16 + c) * 16 + d),
        _ => None,
    }
}

/// The character a two-character escape `\x` stands for.
pub open spec fn short_escape(x: char) -> Option<char> {
    if x == '"' {
        Some('"')
    } else if x == '\\' {
        Some('\\')
    } else if x == '/' {
        Some('/')
    } else if x == 'b' {
        Some('\u{8}')
    } else if x == 'f' {
        Some('\u{c}')
    } else if x == 'n' {
        Some('\n')
    } else if x == 'r' {
        Some('\r')
    } else if x == 't' {
        Some('\t')
    } else {
        None
    }
}

/// Prepends a decoded character to a scan result, counting `n` consumed
/// characters.
pub open spec fn scan_cons(c: char, n: nat, rest: Option<(Seq<char>, nat)>) -> Option<
    (Seq<char>, nat),
> {
    match rest {
        Some((v, k)) => Some((seq![c] + v, k + n)),
        None => None,
    }
}

/// RFC 8259 §7: scans the characters after a string's opening quote up to
/// and including its closing quote, which is the first quote not inside an
/// escape. Returns the decoded string and the number of characters consumed,
/// or `None` when the text has a raw control character (`U+0000..=U+001F`),
/// an invalid or surrogate escape, or no closing quote.
pub open spec fn json_scan_string(t: Seq<char>) -> Option<(Seq<char>, nat)>
    decreases t.len(),
{
    if t.len() == 0 {
        None
    } else if t[0] == '"' {
        Some((Seq::empty(), 1))
    } else if (t[0] as u32) < 0x20 {
        None
    } else if t[0] == '\\' {
        if t.len() >= 2 && short_escape(t[1]) is Some {
            scan_cons(short_escape(t[1])->Some_0, 2, json_scan_string(t.subrange(2, t.len() as int)))
        } else if t.len() >= 6 && t[1] == 'u' && hex4_value(t.subrange(2, 6)) is Some && !(0xD800
            <= hex4_value(t.subrange(2, 6))->Some_0 <= 0xDFFF) {
            scan_cons(
                (hex4_value(t.subrange(2, 6))->Some_0 as u32) as char,
                6,
                json_scan_string(t.subrange(6, t.len() as int)),
            )
        } else {
            None
        }
    } else {
        scan_cons(t[0], 1, json_scan_string(t.drop_first()))
    }
}

/// The escape of `c` scans back to `c`, followed by whatever the rest
/// scans to.
proof fn lemma_scan_escape_char(c: char, rest: Seq<char>)
    ensures
        json_scan_string(json_escape_char(c) + rest) == scan_cons(
            c,
            json_escape_char(c).len(),
            json_scan_string(rest),
        ),
{
    let t = json_escape_char(c) + rest;
    let e = json_escape_char(c);
    if e.len() == 1 {
        assert(t.drop_first() =~= rest);
    } else if e.len() == 2 {
        assert(t.subrange(2, t.len() as int) =~= rest);
    } else {
        let v = c as u32;
        assert(t.subrange(6, t.len() as int) =~= rest);
        let h = t.subrange(2, 6);
        assert(h =~= seq!['0', '0', hex_digit((v / 16) as int), hex_digit((v % 16) as int)]);
        assert(hex_value(h[2]) == Some((v / 16) as int));
        assert(hex_value(h[3]) == Some((v % 16) as int));
        assert(hex4_value(h) == Some(v as int));
        assert((v as char) == c) by {
            vstd::utf8::char_u32_cast(c, v);
        }
    }
}

/// The escaped body begins with the escape of the first character.
proof fn lemma_escape_body_first(s: Seq<char>)
    requires
        s.len() > 0,
    ensures
        json_escape_body(s) == json_escape_char(s[0]) + json_escape_body(s.drop_first()),
    decreases s.len(),
{
    if s.len() == 1 {
        assert(s.drop_last() =~= Seq::<char>::empty());
        assert(s.drop_first() =~= Seq::<char>::empty());
        assert(json_escape_body(s) =~= json_escape_char(s[0]));
    } else {
        lemma_escape_body_first(s.drop_last());
        assert(s.drop_last().drop_first() =~= s.drop_first().drop_last());
        assert(s.drop_first().last() == s.last());
        assert(json_escape_body(s) =~= json_escape_char(s[0]) + json_escape_body(s.drop_first()));
    }
}

/// No escape contains a raw control character.
proof fn lemma_escape_char_clean(c: char)
    ensures
        forall|i: int|
            0 <= i < json_escape_char(c).len() ==> (#[trigger] json_escape_char(c)[i] as u32)
                >= 0x20,
{
    let v = c as u32;
    if v < 0x20 && c != '\u{8}' && c != '\t' && c != '\n' && c != '\u{c}' && c != '\r' {
        assert(hex_digit((v / 16) as int) as u32 >= 0x30);
        assert(hex_digit((v % 16) as int) as u32 >= 0x30);
    }
}

/// The escaped body of a string has no raw control character.
proof fn lemma_escape_body_clean(s: Seq<char>)
    ensures
        forall|i: int|
            0 <= i < json_escape_body(s).len() ==> (#[trigger] json_escape_body(s)[i] as u32)
                >= 0x20,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_escape_body_clean(s.drop_last());
        lemma_escape_char_clean(s.last());
        let a = json_escape_body(s.drop_last());
        let b = json_escape_char(s.last());
        assert forall|i: int| 0 <= i < json_escape_body(s).len() implies (#[trigger] json_escape_body(
            s,
        )[i] as u32) >= 0x20 by {
            if i < a.len() {
                assert(json_escape_body(s)[i] == a[i]);
            } else {
                assert(json_escape_body(s)[i] == b[i - a.len()]);
            }
        }
    }
}

/// Round trip: the escaped text of `s`, then the closing quote, scans back
/// to exactly `s` and stops right after that quote, whatever follows. The
/// escaped text has no raw control character. So `json_string(s)` is a
/// valid JSON string token that decodes to `s`, and distinct strings give
/// distinct tokens.
pub proof fn lemma_json_string_round_trip(s: Seq<char>, rest: Seq<char>)
    ensures
        json_scan_string(json_escape_body(s) + seq!['"'] + rest) == Some(
            (s, json_escape_body(s).len() + 1),
        ),
        forall|i: int|
            0 <= i < json_escape_body(s).len() ==> (#[trigger] json_escape_body(s)[i] as u32)
                >= 0x20,
    decreases s.len(),
{
    lemma_escape_body_clean(s);
    if s.len() == 0 {
        assert(json_escape_body(s) + seq!['"'] + rest =~= seq!['"'] + rest);
        assert(s =~= Seq::<char>::empty());
    } else {
        let tail = s.drop_first();
        lemma_json_string_round_trip(tail, rest);
        lemma_escape_body_first(s);
        let e = json_escape_char(s[0]);
        assert(json_escape_body(s) + seq!['"'] + rest =~= e + (json_escape_body(tail) + seq!['"']
            + rest));
        lemma_scan_escape_char(s[0], json_escape_body(tail) + seq!['"'] + rest);
        assert(seq![s[0]] + tail =~= s);
    }
}

/// Distinct strings have distinct JSON string tokens.
pub proof fn lemma_json_string_injective(a: Seq<char>, b: Seq<char>)
    requires
        json_string(a) == json_string(b),
    ensures
        a == b,
{
    lemma_json_string_round_trip(a, Seq::empty());
    lemma_json_string_round_trip(b, Seq::empty());
    assert(json_string(a).drop_first() =~= json_escape_body(a) + seq!['"'] + Seq::<char>::empty());
    assert(json_string(b).drop_first() =~= json_escape_body(b) + seq!['"'] + Seq::<char>::empty());
}

// ---------------------------------------------------------------------------
// The pretty-printed object
// ---------------------------------------------------------------------------

/// The canonical decimal text of a port, as characters.
pub open spec fn port_text(port: u16) -> Seq<char> {
    super::text::chars_of_bytes(super::exclusions::decimal_text(port as nat))
}

/// One member line: two spaces of indentation, the key, `": "`, the port.
pub open spec fn json_member(entry: (Seq<char>, u16)) -> Seq<char> {
    seq![' ', ' '] + json_string(entry.0) + seq![':', ' '] + port_text(entry.1)
}

/// The member lines joined by `",\n"`.
pub open spec fn json_members(entries: Seq<(Seq<char>, u16)>) -> Seq<char>
    decreases entries.len(),
{
    if entries.len() == 0 {
        Seq::empty()
    } else if entries.len() == 1 {
        json_member(entries[0])
    } else {
        json_members(entries.drop_last()) + seq![',', '\n'] + json_member(entries.last())
    }
}

/// `serde_json`'s pretty layout of a flat object: `{}` when empty, otherwise
/// `{`, newline, the member lines joined by `",\n"`, newline, `}`.
pub open spec fn json_object_pretty(entries: Seq<(Seq<char>, u16)>) -> Seq<char> {
    if entries.len() == 0 {
        seq!['{', '}']
    } else {
        seq!['{', '\n'] + json_members(entries) + seq!['\n', '}']
    }
}

/// What rendering reads from the input: each key's characters and port.
pub open spec fn entry_views(entries: Seq<(&str, u16)>) -> Seq<(Seq<char>, u16)> {
    entries.map_values(|e: (&str, u16)| (e.0@, e.1))
}

/// The UTF-8 bytes of each key: the sort keys (byte order is `str` order).
pub open spec fn key_bytes(entries: Seq<(&str, u16)>) -> Seq<Seq<u8>> {
    entries.map_values(|e: (&str, u16)| vstd::utf8::encode_utf8(e.0@))
}

/// The entries in sorted key order (ties by position).
pub open spec fn sorted_entries(entries: Seq<(&str, u16)>) -> Seq<(Seq<char>, u16)> {
    let order = render::canonical_order(key_bytes(entries));
    Seq::new(entries.len(), |a: int| entry_views(entries)[order[a] as int])
}

/// The rendered port map.
pub open spec fn port_map_text(entries: Seq<(&str, u16)>) -> Seq<char> {
    json_object_pretty(sorted_entries(entries))
}

/// The keys are pairwise distinct (as in a map).
pub open spec fn distinct_keys(entries: Seq<(&str, u16)>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < entries.len() ==> (#[trigger] entries[i]).0@ != (#[trigger] entries[j]).0@
}

/// The keys of `s` strictly increase in byte order.
pub open spec fn strictly_sorted(s: Seq<(Seq<char>, u16)>) -> bool {
    forall|a: int, b: int|
        0 <= a < b < s.len() ==> super::render::lex_lt(
            vstd::utf8::encode_utf8(#[trigger] s[a].0),
            vstd::utf8::encode_utf8(#[trigger] s[b].0),
        )
}

/// The sorted entries are exactly the input entries (each input entry at
/// some position, each position some input entry).
proof fn lemma_sorted_entries_members(entries: Seq<(&str, u16)>)
    requires
        entries.len() <= usize::MAX,
    ensures
        sorted_entries(entries).len() == entries.len(),
        forall|a: int|
            0 <= a < entries.len() ==> entry_views(entries).contains(
                #[trigger] sorted_entries(entries)[a],
            ),
        forall|k: int|
            0 <= k < entries.len() ==> sorted_entries(entries).contains(
                #[trigger] entry_views(entries)[k],
            ),
{
    let keys = key_bytes(entries);
    let order = super::render::canonical_order(keys);
    super::render::lemma_canonical_order_sorted(keys);
    let s = sorted_entries(entries);
    let v = entry_views(entries);
    assert forall|a: int| 0 <= a < entries.len() implies v.contains(#[trigger] s[a]) by {
        assert(s[a] == v[order[a] as int]);
    }
    assert forall|k: int| 0 <= k < entries.len() implies s.contains(#[trigger] v[k]) by {
        assert(super::render::occurs(order, k));
        let a = choose|a: int| 0 <= a < order.len() && order[a] as int == k;
        assert(s[a] == v[k]);
    }
}

/// With distinct keys, the rendered keys strictly increase in byte order
/// (the order of `str`), as in a `BTreeMap<String, _>`.
pub proof fn lemma_sorted_keys_increase(entries: Seq<(&str, u16)>)
    requires
        entries.len() <= usize::MAX,
        distinct_keys(entries),
    ensures
        strictly_sorted(sorted_entries(entries)),
{
    let keys = key_bytes(entries);
    let order = super::render::canonical_order(keys);
    super::render::lemma_canonical_order_sorted(keys);
    assert forall|i: int, j: int| 0 <= i < j < keys.len() implies keys[i] != keys[j] by {
        vstd::utf8::encode_utf8_decode_utf8(entries[i].0@);
        vstd::utf8::encode_utf8_decode_utf8(entries[j].0@);
    }
    let s = sorted_entries(entries);
    assert forall|a: int, b: int| 0 <= a < b < s.len() implies super::render::lex_lt(
        vstd::utf8::encode_utf8(#[trigger] s[a].0),
        vstd::utf8::encode_utf8(#[trigger] s[b].0),
    ) by {
        super::render::lemma_sorted_distinct_keys(keys, order, a, b);
    }
}

/// Strictly sorted sequences that agree before position `a` and hold the
/// same entries agree at `a`.
proof fn lemma_strictly_sorted_agree_at(s1: Seq<(Seq<char>, u16)>, s2: Seq<(Seq<char>, u16)>, a: int)
    requires
        strictly_sorted(s1),
        strictly_sorted(s2),
        forall|x: (Seq<char>, u16)| s1.contains(x) <==> s2.contains(x),
        0 <= a < s1.len(),
        0 <= a < s2.len(),
        forall|b: int| 0 <= b < a ==> s1[b] == s2[b],
    ensures
        s1[a] == s2[a],
{
    let x = s1[a];
    let y = s2[a];
    assert(s1.contains(x));
    let c = choose|c: int| 0 <= c < s2.len() && s2[c] == x;
    assert(s2.contains(y));
    let d = choose|d: int| 0 <= d < s1.len() && s1[d] == y;
    if x != y {
        let kx = vstd::utf8::encode_utf8(x.0);
        let ky = vstd::utf8::encode_utf8(y.0);
        if c < a {
            assert(s1[c] == x);
            super::render::lemma_lex_strict_total(kx, kx);
        }
        if d < a {
            assert(s2[d] == y);
            super::render::lemma_lex_strict_total(ky, ky);
        }
        if c > a && d > a {
            // y = s2[a] precedes x = s2[c], and x = s1[a] precedes y = s1[d].
            super::render::lemma_lex_strict_total(kx, ky);
        }
    }
}

/// Strictly sorted sequences holding the same entries agree before `a`.
proof fn lemma_strictly_sorted_agree_upto(
    s1: Seq<(Seq<char>, u16)>,
    s2: Seq<(Seq<char>, u16)>,
    a: int,
)
    requires
        strictly_sorted(s1),
        strictly_sorted(s2),
        forall|x: (Seq<char>, u16)| s1.contains(x) <==> s2.contains(x),
        0 <= a <= s1.len(),
        a <= s2.len(),
    ensures
        forall|b: int| 0 <= b < a ==> s1[b] == s2[b],
    decreases a,
{
    if a > 0 {
        lemma_strictly_sorted_agree_upto(s1, s2, a - 1);
        lemma_strictly_sorted_agree_at(s1, s2, a - 1);
    }
}

/// Strictly sorted sequences holding the same entries are equal.
proof fn lemma_strictly_sorted_unique(s1: Seq<(Seq<char>, u16)>, s2: Seq<(Seq<char>, u16)>)
    requires
        strictly_sorted(s1),
        strictly_sorted(s2),
        forall|x: (Seq<char>, u16)| s1.contains(x) <==> s2.contains(x),
    ensures
        s1 == s2,
{
    let n = if s1.len() < s2.len() {
        s1.len() as int
    } else {
        s2.len() as int
    };
    lemma_strictly_sorted_agree_upto(s1, s2, n);
    // A longer sequence would hold an entry the shorter one lacks.
    if s1.len() < s2.len() {
        let x = s2[n];
        assert(s2.contains(x));
        let d = choose|d: int| 0 <= d < s1.len() && s1[d] == x;
        assert(s2[d] == x);
        super::render::lemma_lex_strict_total(
            vstd::utf8::encode_utf8(x.0),
            vstd::utf8::encode_utf8(x.0),
        );
    } else if s2.len() < s1.len() {
        let x = s1[n];
        assert(s1.contains(x));
        let d = choose|d: int| 0 <= d < s2.len() && s2[d] == x;
        assert(s1[d] == x);
        super::render::lemma_lex_strict_total(
            vstd::utf8::encode_utf8(x.0),
            vstd::utf8::encode_utf8(x.0),
        );
    }
    assert(s1 =~= s2);
}

/// The text depends only on the map, not on the order of the input: two
/// inputs with distinct keys and the same entries render identically.
pub proof fn lemma_render_order_independent(e1: Seq<(&str, u16)>, e2: Seq<(&str, u16)>)
    requires
        e1.len() <= usize::MAX,
        e2.len() <= usize::MAX,
        distinct_keys(e1),
        distinct_keys(e2),
        forall|x: (Seq<char>, u16)| entry_views(e1).contains(x) <==> entry_views(e2).contains(x),
    ensures
        port_map_text(e1) == port_map_text(e2),
{
    lemma_sorted_entries_members(e1);
    lemma_sorted_entries_members(e2);
    lemma_sorted_keys_increase(e1);
    lemma_sorted_keys_increase(e2);
    let s1 = sorted_entries(e1);
    let s2 = sorted_entries(e2);
    let v1 = entry_views(e1);
    let v2 = entry_views(e2);
    assert forall|x: (Seq<char>, u16)| s1.contains(x) <==> s2.contains(x) by {
        if s1.contains(x) {
            let a = choose|a: int| 0 <= a < s1.len() && s1[a] == x;
            assert(v1.contains(s1[a]));
            assert(v2.contains(x));
            let k = choose|k: int| 0 <= k < v2.len() && v2[k] == x;
            assert(s2.contains(v2[k]));
        }
        if s2.contains(x) {
            let a = choose|a: int| 0 <= a < s2.len() && s2[a] == x;
            assert(v2.contains(s2[a]));
            assert(v1.contains(x));
            let k = choose|k: int| 0 <= k < v1.len() && v1[k] == x;
            assert(s1.contains(v1[k]));
        }
    }
    lemma_strictly_sorted_unique(s1, s2);
}

// ---------------------------------------------------------------------------
// Reading the document back (RFC 8259)
// ---------------------------------------------------------------------------

/// JSON insignificant whitespace (RFC 8259 §2): space, tab, line feed,
/// carriage return.
pub open spec fn is_json_ws(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n' || c == '\r'
}

/// The number of whitespace characters starting at `i`.
pub open spec fn ws_run(t: Seq<char>, i: int) -> nat
    decreases t.len() - i,
{
    if 0 <= i < t.len() && is_json_ws(t[i]) {
        1 + ws_run(t, i + 1)
    } else {
        0
    }
}

/// An ASCII decimal digit.
pub open spec fn is_digit_char(c: char) -> bool {
    '0' <= c && c <= '9'
}

/// The number of digits starting at `i`.
pub open spec fn digit_run(t: Seq<char>, i: int) -> nat
    decreases t.len() - i,
{
    if 0 <= i < t.len() && is_digit_char(t[i]) {
        1 + digit_run(t, i + 1)
    } else {
        0
    }
}

/// The value of the digits `t[i..j]`, most significant first.
pub open spec fn digits_value(t: Seq<char>, i: int, j: int) -> int
    decreases j - i,
{
    if j <= i {
        0
    } else {
        digits_value(t, i, j - 1) * 10 + (t[j - 1] as int - 48)
    }
}

/// RFC 8259 §4 `int` at position `i` (`0`, or a non-zero digit followed by
/// digits): its value and its length. Negative numbers, fractions and
/// exponents are outside this reader (a `.` or `e` after the digits is not
/// a valid continuation, so the object reader rejects them).
pub open spec fn read_uint(t: Seq<char>, i: int) -> Option<(int, nat)> {
    let n = digit_run(t, i);
    if n == 0 || (n > 1 && t[i] == '0') {
        None
    } else {
        Some((digits_value(t, i, i + n), n))
    }
}

/// RFC 8259 §4: one object member at position `i`: a string, optional
/// whitespace, `:`, optional whitespace, an integer, and the whitespace
/// after it. Returns the member and the number of characters it spans.
#[verifier::opaque]
pub open spec fn read_member(t: Seq<char>, i: int) -> Option<((Seq<char>, int), nat)> {
    if !(0 <= i < t.len() && t[i] == '"') {
        None
    } else {
        match json_scan_string(t.subrange(i + 1, t.len() as int)) {
            None => None,
            Some((key, k)) => {
                let colon = i + 1 + k;
                let j = colon + ws_run(t, colon);
                if !(j < t.len() && t[j] == ':') {
                    None
                } else {
                    let v = j + 1 + ws_run(t, j + 1);
                    match read_uint(t, v) {
                        None => None,
                        Some((value, n)) => Some(
                            (
                                (key, value),
                                (1 + k + ws_run(t, colon) + 1 + ws_run(t, j + 1) + n + ws_run(
                                    t,
                                    v + n,
                                )) as nat,
                            ),
                        ),
                    }
                }
            },
        }
    }
}

/// RFC 8259 §4 object members from position `i` (just past the whitespace
/// after `{` or `,`) up to and including the closing `}`: members separated
/// by `,`, with optional whitespace after each `,`. Returns the members in
/// order and the position after the `}`.
#[verifier::opaque]
pub open spec fn read_members(t: Seq<char>, i: int) -> Option<(Seq<(Seq<char>, int)>, int)>
    decreases t.len() - i via read_members_decreases
{
    match read_member(t, i) {
        None => None,
        Some((member, n)) => {
            let w = i + n;
            if 0 <= i && w < t.len() && t[w] == ',' {
                match read_members(t, w + 1 + ws_run(t, w + 1)) {
                    Some((rest, end)) => Some((seq![member] + rest, end)),
                    None => None,
                }
            } else if 0 <= w < t.len() && t[w] == '}' {
                Some((seq![member], w + 1))
            } else {
                None
            }
        },
    }
}

/// A whitespace run ends within the text.
proof fn lemma_ws_run_bounded(t: Seq<char>, i: int)
    requires
        0 <= i <= t.len(),
    ensures
        i + ws_run(t, i) <= t.len(),
    decreases t.len() - i,
{
    if i < t.len() && is_json_ws(t[i]) {
        lemma_ws_run_bounded(t, i + 1);
    }
}

/// `read_members` moves forward within the text.
#[via_fn]
proof fn read_members_decreases(t: Seq<char>, i: int) {
    if let Some((member, n)) = read_member(t, i) {
        let w = i + n;
        if 0 <= i && w < t.len() && t[w] == ',' {
            lemma_ws_run_bounded(t, w + 1);
        }
    }
}

/// RFC 8259: a JSON text that is a single object whose values are
/// non-negative integers, with optional whitespace around it and around
/// every structural character. Returns its members in document order.
#[verifier::opaque]
pub open spec fn read_json_object(t: Seq<char>) -> Option<Seq<(Seq<char>, int)>> {
    let i = ws_run(t, 0) as int;
    if !(i < t.len() && t[i] == '{') {
        None
    } else {
        let j = i + 1 + ws_run(t, i + 1);
        if j < t.len() && t[j] == '}' {
            if j + 1 + ws_run(t, j + 1) == t.len() {
                Some(Seq::empty())
            } else {
                None
            }
        } else {
            match read_members(t, j) {
                Some((members, end)) => if end + ws_run(t, end) == t.len() {
                    Some(members)
                } else {
                    None
                },
                None => None,
            }
        }
    }
}

/// Entries with their ports as JSON numbers.
pub open spec fn as_numbers(entries: Seq<(Seq<char>, u16)>) -> Seq<(Seq<char>, int)> {
    entries.map_values(|e: (Seq<char>, u16)| (e.0, e.1 as int))
}

/// A run of exactly `n` whitespace characters at `i`.
proof fn lemma_ws_run_exact(t: Seq<char>, i: int, n: nat)
    requires
        0 <= i,
        i + n <= t.len(),
        forall|x: int| i <= x < i + n ==> is_json_ws(#[trigger] t[x]),
        i + n == t.len() || !is_json_ws(t[i + n]),
    ensures
        ws_run(t, i) == n,
    decreases n,
{
    if n > 0 {
        lemma_ws_run_exact(t, i + 1, (n - 1) as nat);
    }
}

/// The canonical decimal text of a port, inside a text and followed by a
/// non-digit, reads back as that port.
proof fn lemma_read_port(t: Seq<char>, i: int, port: u16)
    requires
        0 <= i,
        i + port_text(port).len() < t.len(),
        t.subrange(i, i + port_text(port).len()) == port_text(port),
        !is_digit_char(t[i + port_text(port).len()]),
    ensures
        read_uint(t, i) == Some((port as int, port_text(port).len())),
{
    let b = super::exclusions::decimal_text(port as nat);
    super::render::lemma_decimal_canonical(port as nat);
    let d = port_text(port);
    assert(d.len() == b.len());
    assert forall|x: int| 0 <= x < b.len() implies #[trigger] t[i + x] == b[x] as char by {
        assert(t[i + x] == t.subrange(i, i + d.len())[x]);
    }
    lemma_digit_run_exact(t, i, b, b.len());
    lemma_digits_value(t, i, b, b.len());
    if b.len() > 1 {
        assert(t[i + 0] == b[0] as char);
        assert(t[i] == b[0] as char);
    }
}

/// The digits `b` at `i`, followed by a non-digit, are one digit run.
proof fn lemma_digit_run_exact(t: Seq<char>, i: int, b: Seq<u8>, m: nat)
    requires
        0 <= i,
        m <= b.len(),
        i + b.len() < t.len(),
        super::exclusions::all_digits(b),
        forall|x: int| 0 <= x < b.len() ==> #[trigger] t[i + x] == b[x] as char,
        !is_digit_char(t[i + b.len()]),
    ensures
        digit_run(t, i + b.len() - m) == m,
    decreases m,
{
    if m > 0 {
        lemma_digit_run_exact(t, i, b, (m - 1) as nat);
        let x = b.len() - m;
        assert(super::exclusions::is_digit(b[x]));
        assert(t[i + x] == b[x] as char);
    }
}

/// The digits `b[..m]` at `i` have `b[..m]`'s decimal value.
proof fn lemma_digits_value(t: Seq<char>, i: int, b: Seq<u8>, m: nat)
    requires
        0 <= i,
        m <= b.len(),
        forall|x: int| 0 <= x < b.len() ==> #[trigger] t[i + x] == b[x] as char,
    ensures
        digits_value(t, i, i + m) == super::exclusions::decimal_value(b.subrange(0, m as int)),
    decreases m,
{
    if m > 0 {
        lemma_digits_value(t, i, b, (m - 1) as nat);
        assert(b.subrange(0, m as int).drop_last() =~= b.subrange(0, m - 1));
        assert(t[i + m - 1] == b[m - 1] as char);
    } else {
        assert(b.subrange(0, 0) =~= Seq::<u8>::empty());
    }
    if m == b.len() {
        assert(b.subrange(0, m as int) =~= b);
    }
}

/// The member lines, first line first.
proof fn lemma_members_first(es: Seq<(Seq<char>, u16)>)
    requires
        es.len() > 0,
    ensures
        json_members(es) == if es.len() == 1 {
            json_member(es[0])
        } else {
            json_member(es[0]) + (seq![',', '\n'] + json_members(es.drop_first()))
        },
    decreases es.len(),
{
    if es.len() > 2 {
        let init = es.drop_last();
        let tail = es.drop_first();
        lemma_members_first(init);
        assert(init[0] == es[0]);
        assert(init.drop_first() =~= tail.drop_last());
        assert(tail.last() == es.last());
        assert(json_members(tail) == json_members(tail.drop_last()) + seq![',', '\n']
            + json_member(tail.last()));
        assert(json_members(es) == json_members(init) + seq![',', '\n'] + json_member(es.last()));
        assert(json_members(es) =~= json_member(es[0]) + (seq![',', '\n'] + json_members(tail)));
    } else if es.len() == 2 {
        assert(es.drop_last() =~= seq![es[0]]);
        assert(es.drop_first() =~= seq![es[1]]);
        assert(json_members(es.drop_last()) == json_member(es[0]));
        assert(es.last() == es[1]);
        assert(json_members(es) == json_members(es.drop_last()) + seq![',', '\n'] + json_member(
            es.last(),
        ));
        assert(json_members(es.drop_first()) == json_member(es[1]));
        assert(json_members(es) =~= json_member(es[0]) + (seq![',', '\n'] + json_members(
            es.drop_first(),
        )));
    }
}

/// Where the parts of a member line sit: two spaces, the quoted key, `:`,
/// a space, the digits.
proof fn lemma_member_layout(key: Seq<char>, port: u16)
    ensures
        json_member((key, port)).len() == json_escape_body(key).len() + 6 + port_text(port).len(),
        json_member((key, port))[2] == '"',
        json_member((key, port)).subrange(3, json_member((key, port)).len() as int)
            == json_escape_body(key) + seq!['"'] + (seq![':', ' '] + port_text(port)),
{
    let m = json_member((key, port));
    assert(m =~= seq![' ', ' ', '"'] + (json_escape_body(key) + seq!['"'] + (seq![':', ' ']
        + port_text(port))));
    assert(m.subrange(3, m.len() as int) =~= json_escape_body(key) + seq!['"'] + (seq![':', ' ']
        + port_text(port)));
}

/// The suffix of `a + b` after `a` is `b`.
proof fn lemma_suffix<T>(a: Seq<T>, b: Seq<T>)
    ensures
        (a + b).subrange(a.len() as int, (a + b).len() as int) == b,
{
    assert((a + b).subrange(a.len() as int, (a + b).len() as int) =~= b);
}

/// A member line after a prefix, split before its key's characters and
/// before the `:`.
proof fn lemma_member_split(p: Seq<char>, key: Seq<char>, port: u16, rest: Seq<char>)
    ensures
        p + json_member((key, port)) + rest == (p + seq![' ', ' ', '"']) + (json_escape_body(key)
            + seq!['"'] + (seq![':', ' '] + port_text(port) + rest)),
        p + json_member((key, port)) + rest == (p + seq![' ', ' '] + json_string(key)) + (seq![
            ':',
            ' ',
        ] + port_text(port) + rest),
{
    let esc = json_escape_body(key);
    let tail = seq![':', ' '] + port_text(port) + rest;
    assert(p + json_member((key, port)) + rest =~= (p + seq![' ', ' ', '"']) + (esc + seq!['"']
        + tail));
    assert(p + json_member((key, port)) + rest =~= (p + seq![' ', ' '] + json_string(key)) + tail);
}

/// The text after a member's opening quote, and after its key.
proof fn lemma_after_quote(p: Seq<char>, key: Seq<char>, port: u16, rest: Seq<char>)
    ensures
        (p + json_member((key, port)) + rest).subrange(
            p.len() + 3int,
            (p + json_member((key, port)) + rest).len() as int,
        ) == json_escape_body(key) + seq!['"'] + (seq![':', ' '] + port_text(port) + rest),
        (p + json_member((key, port)) + rest).subrange(
            p.len() + 4int + json_escape_body(key).len(),
            (p + json_member((key, port)) + rest).len() as int,
        ) == seq![':', ' '] + port_text(port) + rest,
        (p + json_member((key, port)) + rest)[p.len() + 2int] == '"',
        (p + json_member((key, port)) + rest).len() > p.len() + 4int + json_escape_body(key).len(),
{
    let esc = json_escape_body(key);
    let tail = seq![':', ' '] + port_text(port) + rest;
    let t = p + json_member((key, port)) + rest;
    lemma_member_split(p, key, port, rest);
    let a = p + seq![' ', ' ', '"'];
    let b = p + seq![' ', ' '] + json_string(key);
    lemma_suffix(a, esc + seq!['"'] + tail);
    lemma_suffix(b, tail);
    assert(t[p.len() + 2int] == a[p.len() + 2int]);
}

/// Whitespace runs read the same in a suffix.
proof fn lemma_ws_run_shift(t: Seq<char>, c: int, k: int)
    requires
        0 <= c <= t.len(),
        0 <= k,
    ensures
        ws_run(t, c + k) == ws_run(t.subrange(c, t.len() as int), k),
    decreases t.len() - c - k,
{
    let u = t.subrange(c, t.len() as int);
    if c + k < t.len() {
        assert(t[c + k] == u[k]);
        lemma_ws_run_shift(t, c, k + 1);
    }
}

/// Digit runs read the same in a suffix.
proof fn lemma_digit_run_shift(t: Seq<char>, c: int, k: int)
    requires
        0 <= c <= t.len(),
        0 <= k,
    ensures
        digit_run(t, c + k) == digit_run(t.subrange(c, t.len() as int), k),
    decreases t.len() - c - k,
{
    let u = t.subrange(c, t.len() as int);
    if c + k < t.len() {
        assert(t[c + k] == u[k]);
        lemma_digit_run_shift(t, c, k + 1);
    }
}

/// Digit values read the same in a suffix.
proof fn lemma_digits_value_shift(t: Seq<char>, c: int, k: int, n: nat)
    requires
        0 <= c <= t.len(),
        0 <= k,
        c + k + n <= t.len(),
    ensures
        digits_value(t, c + k, c + k + n) == digits_value(t.subrange(c, t.len() as int), k, k + n),
    decreases n,
{
    if n > 0 {
        lemma_digits_value_shift(t, c, k, (n - 1) as nat);
        assert(t[c + k + n - 1] == t.subrange(c, t.len() as int)[k + n - 1]);
    }
}

/// Integers read the same in a suffix.
proof fn lemma_read_uint_shift(t: Seq<char>, c: int, k: int)
    requires
        0 <= c <= t.len(),
        0 <= k,
    ensures
        read_uint(t, c + k) == read_uint(t.subrange(c, t.len() as int), k),
{
    let u = t.subrange(c, t.len() as int);
    lemma_digit_run_shift(t, c, k);
    let n = digit_run(t, c + k);
    if n > 0 {
        lemma_digit_run_bounded(t, c + k);
        lemma_digits_value_shift(t, c, k, n);
        assert(t[c + k] == u[k]);
    }
}

/// A digit run ends within the text.
proof fn lemma_digit_run_bounded(t: Seq<char>, i: int)
    requires
        0 <= i,
    ensures
        digit_run(t, i) > 0 ==> i + digit_run(t, i) <= t.len(),
    decreases t.len() - i,
{
    if 0 <= i < t.len() && is_digit_char(t[i]) {
        lemma_digit_run_bounded(t, i + 1);
    }
}

/// What follows a member's key: `: `, the digits, `trail` whitespace, and
/// a structural character.
proof fn lemma_value_facts(port: u16, rest: Seq<char>, trail: nat)
    requires
        trail < rest.len(),
        !is_digit_char(rest[0]),
        forall|x: int| 0 <= x < trail ==> is_json_ws(#[trigger] rest[x]),
        !is_json_ws(rest[trail as int]),
    ensures
        ({
            let tail = seq![':', ' '] + port_text(port) + rest;
            &&& tail[0] == ':'
            &&& ws_run(tail, 0) == 0
            &&& ws_run(tail, 1) == 1
            &&& read_uint(tail, 2) == Some((port as int, port_text(port).len()))
            &&& ws_run(tail, 2int + port_text(port).len()) == trail
        }),
{
    let digits = port_text(port);
    let tail = seq![':', ' '] + digits + rest;
    super::render::lemma_decimal_canonical(port as nat);
    assert(super::exclusions::is_digit(super::exclusions::decimal_text(port as nat)[0]));
    assert(tail[2] == digits[0]);
    lemma_ws_run_exact(tail, 0, 0);
    lemma_ws_run_exact(tail, 1, 1);
    let e: int = 2int + digits.len();
    assert(tail.subrange(2, e) =~= digits);
    assert(tail[e] == rest[0]);
    lemma_read_port(tail, 2, port);
    assert forall|x: int| e <= x < e + trail implies is_json_ws(#[trigger] tail[x]) by {
        assert(tail[x] == rest[x - e]);
    }
    assert(tail[e + trail] == rest[trail as int]);
    lemma_ws_run_exact(tail, e, trail);
}

/// The facts `read_member` checks at position `i` of a text, given what
/// follows the opening quote there: the escaped key, its closing quote, `: `,
/// the digits, `trail` whitespace, and a structural character.
proof fn lemma_member_facts_at(
    t: Seq<char>,
    i: int,
    key: Seq<char>,
    port: u16,
    rest: Seq<char>,
    trail: nat,
)
    requires
        trail < rest.len(),
        !is_digit_char(rest[0]),
        forall|x: int| 0 <= x < trail ==> is_json_ws(#[trigger] rest[x]),
        !is_json_ws(rest[trail as int]),
        0 <= i,
        i + 2 + json_escape_body(key).len() < t.len(),
        t[i] == '"',
        t.subrange(i + 1, t.len() as int) == json_escape_body(key) + seq!['"'] + (seq![':', ' ']
            + port_text(port) + rest),
        t.subrange(i + 2 + json_escape_body(key).len(), t.len() as int) == seq![':', ' ']
            + port_text(port) + rest,
    ensures
        ({
            let colon = i + 2 + json_escape_body(key).len();
            &&& json_scan_string(t.subrange(i + 1, t.len() as int)) == Some(
                (key, json_escape_body(key).len() + 1),
            )
            &&& t[colon] == ':'
            &&& ws_run(t, colon) == 0
            &&& ws_run(t, colon + 1) == 1
            &&& read_uint(t, colon + 2) == Some((port as int, port_text(port).len()))
            &&& ws_run(t, colon + 2 + port_text(port).len()) == trail
        }),
{
    let esc = json_escape_body(key);
    let digits = port_text(port);
    let tail = seq![':', ' '] + digits + rest;
    let colon: int = i + 2 + esc.len();
    lemma_json_string_round_trip(key, tail);
    let u = t.subrange(colon, t.len() as int);
    lemma_value_facts(port, rest, trail);
    assert(t[colon] == u[0]);
    lemma_ws_run_shift(t, colon, 0);
    lemma_ws_run_shift(t, colon, 1);
    lemma_read_uint_shift(t, colon, 2);
    lemma_ws_run_shift(t, colon, 2int + digits.len());
}

/// The facts `read_member` checks, for a member line after any prefix,
/// followed by `trail` whitespace and then a structural character.
proof fn lemma_member_facts(p: Seq<char>, key: Seq<char>, port: u16, rest: Seq<char>, trail: nat)
    requires
        trail < rest.len(),
        !is_digit_char(rest[0]),
        forall|x: int| 0 <= x < trail ==> is_json_ws(#[trigger] rest[x]),
        !is_json_ws(rest[trail as int]),
    ensures
        ({
            let t = p + json_member((key, port)) + rest;
            let i = p.len() + 2int;
            let colon = i + 2 + json_escape_body(key).len();
            &&& 0 <= i < t.len() && t[i] == '"'
            &&& json_scan_string(t.subrange(i + 1, t.len() as int)) == Some(
                (key, json_escape_body(key).len() + 1),
            )
            &&& colon < t.len() && t[colon] == ':'
            &&& ws_run(t, colon) == 0
            &&& ws_run(t, colon + 1) == 1
            &&& read_uint(t, colon + 2) == Some((port as int, port_text(port).len()))
            &&& ws_run(t, colon + 2 + port_text(port).len()) == trail
        }),
{
    lemma_after_quote(p, key, port, rest);
    lemma_member_facts_at(p + json_member((key, port)) + rest, p.len() + 2int, key, port, rest, trail);
}

/// A member line after any prefix, followed by `trail` whitespace and then
/// a structural character, reads back as the member, spanning the line
/// (without its two-space indentation) and the whitespace.
#[verifier::spinoff_prover]
proof fn lemma_read_member(p: Seq<char>, key: Seq<char>, port: u16, rest: Seq<char>, trail: nat)
    requires
        trail < rest.len(),
        !is_digit_char(rest[0]),
        forall|x: int| 0 <= x < trail ==> is_json_ws(#[trigger] rest[x]),
        !is_json_ws(rest[trail as int]),
    ensures
        read_member(p + json_member((key, port)) + rest, p.len() + 2int) == Some(
            ((key, port as int), (json_escape_body(key).len() + 4 + port_text(port).len()
                + trail) as nat),
        ),
{
    reveal(read_member);
    lemma_member_facts(p, key, port, rest, trail);
}

/// The member lines start with the first member's indentation and quote.
proof fn lemma_members_start(es: Seq<(Seq<char>, u16)>)
    requires
        es.len() > 0,
    ensures
        json_members(es).len() >= 3,
        json_members(es)[0] == ' ' && json_members(es)[1] == ' ' && json_members(es)[2] == '"',
{
    lemma_members_first(es);
    let m = json_member(es[0]);
    assert(m[0] == ' ' && m[1] == ' ' && m[2] == '"');
    if es.len() > 1 {
        let whole = m + (seq![',', '\n'] + json_members(es.drop_first()));
        assert(whole[0] == m[0] && whole[1] == m[1] && whole[2] == m[2]);
    }
}

/// Regrouping the text around the first member line.
proof fn lemma_regroup(p: Seq<char>, m: Seq<char>, ms: Seq<char>)
    ensures
        p + (m + (seq![',', '\n'] + ms)) + seq!['\n', '}'] == p + m + (seq![',', '\n'] + ms
            + seq!['\n', '}']),
        p + (m + (seq![',', '\n'] + ms)) + seq!['\n', '}'] == (p + m + seq![',', '\n']) + ms
            + seq!['\n', '}'],
{
    assert(p + (m + (seq![',', '\n'] + ms)) + seq!['\n', '}'] =~= p + m + (seq![',', '\n'] + ms
        + seq!['\n', '}']));
    assert(p + (m + (seq![',', '\n'] + ms)) + seq!['\n', '}'] =~= (p + m + seq![',', '\n']) + ms
        + seq!['\n', '}']);
}

/// The member lines after any prefix, then `\n}`, read back as the entries,
/// ending at the end of the text.
#[verifier::spinoff_prover]
proof fn lemma_read_members(p: Seq<char>, es: Seq<(Seq<char>, u16)>)
    requires
        es.len() > 0,
    ensures
        read_members(p + json_members(es) + seq!['\n', '}'], p.len() + 2int) == Some(
            (as_numbers(es), (p + json_members(es) + seq!['\n', '}']).len() as int),
        ),
    decreases es.len(),
{
    reveal(read_members);
    let t = p + json_members(es) + seq!['\n', '}'];
    let key = es[0].0;
    let port = es[0].1;
    let member = json_member((key, port));
    assert(es[0] == (key, port));
    lemma_members_first(es);
    let i: int = p.len() + 2int;
    if es.len() == 1 {
        let rest = seq!['\n', '}'];
        assert(json_members(es) == member);
        lemma_read_member(p, key, port, rest, 1);
        assert(as_numbers(es) =~= seq![(key, port as int)]);
    } else {
        let tail = es.drop_first();
        let ms = json_members(tail);
        let rest = seq![',', '\n'] + ms + seq!['\n', '}'];
        let p2 = p + member + seq![',', '\n'];
        lemma_regroup(p, member, ms);
        lemma_read_member(p, key, port, rest, 0);
        let w: int = p.len() as int + member.len();
        assert(t[w] == rest[0]);
        lemma_members_start(tail);
        assert(t[w + 1] == '\n' && t[w + 2] == ' ' && t[w + 3] == ' ' && t[w + 4] == '"') by {
            assert(t[w + 1] == rest[1]);
            assert(t[w + 2] == ms[0]);
            assert(t[w + 3] == ms[1]);
            assert(t[w + 4] == ms[2]);
        }
        lemma_ws_run_exact(t, w + 1, 3);
        lemma_read_members(p2, tail);
        assert(as_numbers(es) =~= seq![(key, port as int)] + as_numbers(tail));
    }
}

/// The rendered object is a valid JSON text (RFC 8259) that reads back as
/// exactly the entries, in order, each port as its number.
pub proof fn lemma_object_reads_back(es: Seq<(Seq<char>, u16)>)
    ensures
        read_json_object(json_object_pretty(es)) == Some(as_numbers(es)),
{
    reveal(read_json_object);
    let t = json_object_pretty(es);
    if es.len() == 0 {
        assert(t =~= seq!['{', '}']);
        lemma_ws_run_exact(t, 0, 0);
        lemma_ws_run_exact(t, 1, 0);
        lemma_ws_run_exact(t, 2, 0);
        assert(as_numbers(es) =~= Seq::<(Seq<char>, int)>::empty());
    } else {
        let p = seq!['{', '\n'];
        lemma_members_first(es);
        assert(t =~= p + json_members(es) + seq!['\n', '}']);
        assert(t[1] == '\n' && t[2] == ' ' && t[3] == ' ' && t[4] == '"');
        lemma_ws_run_exact(t, 0, 0);
        lemma_ws_run_exact(t, 1, 3);
        lemma_read_members(p, es);
        lemma_ws_run_exact(t, t.len() as int, 0);
    }
}

/// The port map text reads back (RFC 8259) as the entries in key order:
/// for distinct keys, exactly the map, as the keys strictly increase.
pub proof fn lemma_port_map_reads_back(entries: Seq<(&str, u16)>)
    ensures
        read_json_object(port_map_text(entries)) == Some(as_numbers(sorted_entries(entries))),
{
    lemma_object_reads_back(sorted_entries(entries));
}

// ---------------------------------------------------------------------------
// serde_json's byte-level escaping
// ---------------------------------------------------------------------------

/// How `serde_json` escapes one byte of a string's UTF-8 text: its `ESCAPE`
/// table (`format_escaped_str_contents`) and `write_char_escape`. `"`, `\`,
/// backspace, tab, line feed, form feed and carriage return get two-byte
/// escapes, every other byte below `0x20` becomes `\u00` and two lowercase
/// hex digits, and every other byte (DEL and the bytes of non-ASCII text
/// included) is copied.
pub open spec fn json_escape_byte(b: u8) -> Seq<u8> {
    if b == 0x22 {
        seq![0x5Cu8, 0x22u8]
    } else if b == 0x5C {
        seq![0x5Cu8, 0x5Cu8]
    } else if b == 0x08 {
        seq![0x5Cu8, 0x62u8]
    } else if b == 0x09 {
        seq![0x5Cu8, 0x74u8]
    } else if b == 0x0A {
        seq![0x5Cu8, 0x6Eu8]
    } else if b == 0x0C {
        seq![0x5Cu8, 0x66u8]
    } else if b == 0x0D {
        seq![0x5Cu8, 0x72u8]
    } else if b < 0x20 {
        seq![
            0x5Cu8,
            0x75u8,
            0x30u8,
            0x30u8,
            super::render::hex_byte(b / 16),
            super::render::hex_byte(b % 16),
        ]
    } else {
        seq![b]
    }
}

/// `serde_json`'s escaping of a whole UTF-8 text, byte by byte.
pub open spec fn json_escape_bytes(s: Seq<u8>) -> Seq<u8>
    decreases s.len(),
{
    if s.len() == 0 {
        Seq::empty()
    } else {
        json_escape_bytes(s.drop_last()) + json_escape_byte(s.last())
    }
}

/// Byte escaping distributes over concatenation.
proof fn lemma_escape_bytes_concat(a: Seq<u8>, b: Seq<u8>)
    ensures
        json_escape_bytes(a + b) == json_escape_bytes(a) + json_escape_bytes(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(json_escape_bytes(b) =~= Seq::<u8>::empty());
        assert(json_escape_bytes(a) + json_escape_bytes(b) =~= json_escape_bytes(a));
    } else {
        lemma_escape_bytes_concat(a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        assert(json_escape_bytes(a + b) =~= json_escape_bytes(a) + json_escape_bytes(b));
    }
}

/// Bytes of `0x80` and above (all bytes of non-ASCII text) are copied.
proof fn lemma_escape_bytes_high(s: Seq<u8>)
    requires
        forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] >= 0x80,
    ensures
        json_escape_bytes(s) == s,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_escape_bytes_high(s.drop_last());
        assert(s.drop_last() + seq![s.last()] =~= s);
    }
}

/// Every byte of the UTF-8 encoding of a non-ASCII scalar value is at least
/// `0x80`.
proof fn lemma_encode_scalar_high(v: u32)
    requires
        v >= 0x80,
    ensures
        forall|i: int|
            0 <= i < vstd::utf8::encode_scalar(v).len() ==> #[trigger] vstd::utf8::encode_scalar(
                v,
            )[i] >= 0x80,
{
    let a = ((v >> 6) & 0x1F) as u8;
    let b = ((v >> 12) & 0x0F) as u8;
    let c = ((v >> 18) & 0x7) as u8;
    let d = (v & 0x3F) as u8;
    let e = ((v >> 6) & 0x3F) as u8;
    let f = ((v >> 12) & 0x3F) as u8;
    assert((0xC0u8 | a) >= 0x80) by (bit_vector);
    assert((0xE0u8 | b) >= 0x80) by (bit_vector);
    assert((0xF0u8 | c) >= 0x80) by (bit_vector);
    assert((0x80u8 | d) >= 0x80) by (bit_vector);
    assert((0x80u8 | e) >= 0x80) by (bit_vector);
    assert((0x80u8 | f) >= 0x80) by (bit_vector);
}

/// The UTF-8 encoding of an ASCII character is its code point.
proof fn lemma_encode_ascii_char(c: char)
    requires
        (c as u32) < 0x80,
    ensures
        vstd::utf8::encode_utf8(seq![c]) == seq![(c as u32) as u8],
{
    let v = c as u32;
    assert(v & 0x7F == v) by (bit_vector)
        requires
            v < 0x80,
    ;
    lemma_encode_one(c);
}

/// The UTF-8 encoding of one character is the encoding of its scalar value.
proof fn lemma_encode_one(c: char)
    ensures
        vstd::utf8::encode_utf8(seq![c]) == vstd::utf8::encode_scalar(c as u32),
{
    let one = seq![c];
    assert(one[0] == c);
    assert(one.drop_first() =~= Seq::<char>::empty());
    assert(vstd::utf8::encode_utf8(Seq::<char>::empty()) == Seq::<u8>::empty());
    assert(vstd::utf8::encode_utf8(one) == vstd::utf8::encode_scalar(one[0] as u32)
        + vstd::utf8::encode_utf8(one.drop_first()));
    assert(vstd::utf8::encode_scalar(c as u32) + Seq::<u8>::empty() =~= vstd::utf8::encode_scalar(
        c as u32,
    ));
}

/// The escape of an ASCII character is the Latin-1 reading of the byte
/// escape of its code point (the escape is ASCII).
proof fn lemma_escape_ascii_char(c: char)
    requires
        (c as u32) < 0x80,
    ensures
        json_escape_char(c) == super::text::chars_of_bytes(json_escape_byte((c as u32) as u8)),
        super::text::is_ascii_bytes(json_escape_byte((c as u32) as u8)),
{
    let v = c as u32;
    let b = v as u8;
    let e = json_escape_byte(b);
    if v < 0x20 {
        assert(super::render::hex_byte(b / 16) < 0x80);
        assert(super::render::hex_byte(b % 16) < 0x80);
    }
    assert(json_escape_char(c) =~= super::text::chars_of_bytes(e));
}

/// One character: the UTF-8 text of its escape is `serde_json`'s byte
/// escaping of its UTF-8 text.
proof fn lemma_escape_char_bytes(c: char)
    ensures
        vstd::utf8::encode_utf8(json_escape_char(c)) == json_escape_bytes(
            vstd::utf8::encode_utf8(seq![c]),
        ),
{
    let v = c as u32;
    if v < 0x80 {
        lemma_encode_ascii_char(c);
        lemma_escape_ascii_char(c);
        super::text::lemma_ascii_bytes_round_trip(json_escape_byte(v as u8));
        let one = seq![v as u8];
        assert(one.drop_last() =~= Seq::<u8>::empty());
        assert(one.last() == v as u8);
        assert(json_escape_bytes(Seq::<u8>::empty()) == Seq::<u8>::empty());
        assert(json_escape_bytes(one) == json_escape_bytes(one.drop_last()) + json_escape_byte(
            one.last(),
        ));
        assert(json_escape_bytes(one) =~= json_escape_byte(v as u8));
    } else {
        assert(json_escape_char(c) == seq![c]);
        lemma_encode_one(c);
        lemma_encode_scalar_high(v);
        lemma_escape_bytes_high(vstd::utf8::encode_scalar(v));
    }
}

/// The escaping is `serde_json`'s byte-level escaping: the UTF-8 text of
/// `json_escape_body(s)` is `json_escape_bytes` of the UTF-8 text of `s`.
/// (Escaping per character equals escaping per byte because every escaped
/// byte is ASCII and every byte of non-ASCII text is `0x80` or above.)
pub proof fn lemma_escape_matches_serde_bytes(s: Seq<char>)
    ensures
        vstd::utf8::encode_utf8(json_escape_body(s)) == json_escape_bytes(
            vstd::utf8::encode_utf8(s),
        ),
    decreases s.len(),
{
    if s.len() == 0 {
        assert(vstd::utf8::encode_utf8(s) =~= Seq::<u8>::empty());
        assert(json_escape_body(s) =~= Seq::<char>::empty());
    } else {
        let init = s.drop_last();
        let c = s.last();
        lemma_escape_matches_serde_bytes(init);
        lemma_escape_char_bytes(c);
        vstd::utf8::encode_utf8_concat(json_escape_body(init), json_escape_char(c));
        assert(init + seq![c] =~= s);
        vstd::utf8::encode_utf8_concat(init, seq![c]);
        lemma_escape_bytes_concat(vstd::utf8::encode_utf8(init), vstd::utf8::encode_utf8(seq![c]));
    }
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// `hex_digit(d)`, computed.
fn hex_digit_of(d: u32) -> (c: char)
    requires
        d < 16,
    ensures
        c == hex_digit(d as int),
{
    let b: u8 = u8::try_from(d).unwrap_or_default();
    if b < 10 {
        (b'0' + b) as char
    } else {
        (b'a' + b - 10) as char
    }
}

/// Appends the escape of `c`.
fn push_escaped(out: &mut String, c: char)
    ensures
        final(out)@ == old(out)@ + json_escape_char(c),
{
    let ghost start = out@;
    let v = c as u32;
    if c == '"' {
        out.push('\\');
        out.push('"');
    } else if c == '\\' {
        out.push('\\');
        out.push('\\');
    } else if c == '\u{8}' {
        out.push('\\');
        out.push('b');
    } else if c == '\t' {
        out.push('\\');
        out.push('t');
    } else if c == '\n' {
        out.push('\\');
        out.push('n');
    } else if c == '\u{c}' {
        out.push('\\');
        out.push('f');
    } else if c == '\r' {
        out.push('\\');
        out.push('r');
    } else if v < 0x20 {
        out.push('\\');
        out.push('u');
        out.push('0');
        out.push('0');
        out.push(hex_digit_of(v / 16));
        out.push(hex_digit_of(v % 16));
    } else {
        out.push(c);
    }
    proof {
        assert(out@ =~= start + json_escape_char(c));
    };
}

/// Appends the JSON string token of `s`.
#[verifier::spinoff_prover]
fn push_json_string(out: &mut String, s: &str)
    ensures
        final(out)@ == old(out)@ + json_string(s@),
{
    let ghost start = out@;
    out.push('"');
    let mut chars = s.chars();
    let ghost n = s@.len() as int;
    let ghost mut k: int = 0;
    let mut done = false;
    while !done
        invariant
            0 <= k <= n,
            n == s@.len(),
            vstd::std_specs::iter::IteratorSpec::remaining(&chars) == s@.subrange(k, n),
            out@ == start + seq!['"'] + json_escape_body(s@.subrange(0, k)),
            done ==> k == n,
        decreases n - k + if done {
            0int
        } else {
            1int
        },
    {
        match chars.next() {
            Some(c) => {
                proof {
                    let next = s@.subrange(0, k + 1);
                    assert(next.drop_last() =~= s@.subrange(0, k));
                    assert(next.last() == c);
                }
                push_escaped(out, c);
                proof {
                    k = k + 1;
                };
            },
            None => {
                done = true;
            },
        }
    }
    out.push('"');
    proof {
        assert(s@.subrange(0, n) =~= s@);
        assert(out@ =~= start + json_string(s@));
    };
}

/// Appends the canonical decimal text of `port`.
fn push_port(out: &mut String, port: u16)
    ensures
        final(out)@ == old(out)@ + port_text(port),
{
    let digits = render::u16_decimal_string(port);
    out.push_str(digits.as_str());
}

/// Renders a port map as `serde_json`'s `to_string_pretty` renders a
/// `BTreeMap<String, u16>`: the entries in byte order of their keys (ties
/// by position), each key a JSON string token and each port its canonical
/// decimal text. Total.
pub fn render_port_map_pretty(entries: &[(&str, u16)]) -> (out: String)
    ensures
        out@ == port_map_text(entries@),
{
    let n = entries.len();
    let mut keys: Vec<&[u8]> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            n == entries@.len(),
            0 <= i <= n,
            keys@.len() == i,
            forall|a: int| 0 <= a < i ==> (#[trigger] keys@[a])@ == key_bytes(entries@)[a],
        decreases n - i,
    {
        keys.push(entries[i].0.as_bytes());
        i += 1;
    }
    proof {
        assert(render::keys_view(keys@) =~= key_bytes(entries@));
    }
    let order = render::sort_order(keys.as_slice());
    let ghost sorted = sorted_entries(entries@);
    let ghost views = entry_views(entries@);
    proof {
        render::lemma_canonical_order(key_bytes(entries@), order@);
    }
    let mut out = String::new();
    if n == 0 {
        out.push('{');
        out.push('}');
        proof {
            assert(out@ =~= port_map_text(entries@));
        }
        return out;
    }
    out.push('{');
    out.push('\n');
    let mut j: usize = 0;
    while j < n
        invariant
            n == entries@.len() == order@.len() == sorted.len() == views.len(),
            n > 0,
            0 <= j <= n,
            sorted == sorted_entries(entries@),
            views == entry_views(entries@),
            order@ == render::canonical_order(key_bytes(entries@)),
            forall|a: int| 0 <= a < n ==> (#[trigger] order@[a] as int) < n,
            out@ == seq!['{', '\n'] + json_members(sorted.subrange(0, j as int)),
        decreases n - j,
    {
        let index = order[j];
        if j > 0 {
            out.push(',');
            out.push('\n');
        }
        let (key, port) = entries[index];
        let ghost before = out@;
        out.push(' ');
        out.push(' ');
        push_json_string(&mut out, key);
        out.push(':');
        out.push(' ');
        push_port(&mut out, port);
        proof {
            assert(sorted[j as int] == views[index as int]);
            assert(views[index as int] == (key@, port));
            let next = sorted.subrange(0, j + 1);
            assert(next.drop_last() =~= sorted.subrange(0, j as int));
            assert(next.last() == (key@, port));
            assert(out@ =~= before + json_member((key@, port)));
            if j == 0 {
                assert(json_members(next) == json_member(next[0]));
                assert(sorted.subrange(0, 0) =~= Seq::<(Seq<char>, u16)>::empty());
            }
            assert(out@ =~= seq!['{', '\n'] + json_members(next));
        }
        j += 1;
    }
    out.push('\n');
    out.push('}');
    proof {
        assert(sorted.subrange(0, n as int) =~= sorted);
        assert(out@ =~= port_map_text(entries@));
    }
    out
}

} // verus!
