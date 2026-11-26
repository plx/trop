//! Verified kernel: escaping of stored text in diagnostics.
//!
//! `escape_default` computes `escape_text`, the specification of
//! `str::chars().flat_map(char::escape_default)`: `\t`, `\r`, `\n`, `\\`,
//! `\'` and `\"` for those six chars, printable ASCII (`0x20..=0x7E`)
//! unchanged, and `\u{...}` (lowercase hexadecimal, no leading zeros) for
//! everything else. Proved:
//!
//! - every output char is printable ASCII, so no control char, newline,
//!   escape sequence or non-ASCII text from the database reaches a log line
//!   (`lemma_escape_printable`);
//! - quote safety: a reader that honors backslash escapes never sees an
//!   unescaped `"` in the output and is not inside an escape at its end, so
//!   stored text wrapped as `key="..."` cannot close the quotes or forge
//!   further fields (`lemma_escape_quote_safe`);
//! - the escaping is injective, so diagnostics are unambiguous
//!   (`lemma_escape_injective`).
//!
//! Production glue: `escape_text` in `crate::database::operations` (stored
//! paths and tags in corruption details) and `crate::database::validation`
//! (metadata keys). Equality with `char::escape_default` is pinned by an
//! exhaustive test over every `char`.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The lowercase hexadecimal digit for `d` (`0..=15`).
pub open spec fn hex_digit(d: int) -> char {
    if d == 0 {
        '0'
    } else if d == 1 {
        '1'
    } else if d == 2 {
        '2'
    } else if d == 3 {
        '3'
    } else if d == 4 {
        '4'
    } else if d == 5 {
        '5'
    } else if d == 6 {
        '6'
    } else if d == 7 {
        '7'
    } else if d == 8 {
        '8'
    } else if d == 9 {
        '9'
    } else if d == 10 {
        'a'
    } else if d == 11 {
        'b'
    } else if d == 12 {
        'c'
    } else if d == 13 {
        'd'
    } else if d == 14 {
        'e'
    } else {
        'f'
    }
}

/// The lowercase hexadecimal digits of `n`, most significant first, without
/// leading zeros (`0` is `"0"`).
pub open spec fn hex(n: nat) -> Seq<char>
    decreases n,
{
    if n < 16 {
        seq![hex_digit(n as int)]
    } else {
        hex(n / 16).push(hex_digit((n % 16) as int))
    }
}

/// `char::escape_default` of one char.
pub open spec fn escape_char(c: char) -> Seq<char> {
    if c == '\t' {
        seq!['\\', 't']
    } else if c == '\r' {
        seq!['\\', 'r']
    } else if c == '\n' {
        seq!['\\', 'n']
    } else if c == '\\' || c == '\'' || c == '"' {
        seq!['\\', c]
    } else if 0x20 <= c as u32 <= 0x7e {
        seq![c]
    } else {
        seq!['\\', 'u', '{'] + hex(c as u32 as nat) + seq!['}']
    }
}

/// `chars().flat_map(char::escape_default)` of a whole text.
pub open spec fn escape_text(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() == 0 {
        Seq::empty()
    } else {
        escape_char(s[0]) + escape_text(s.drop_first())
    }
}

// ---------------------------------------------------------------------------
// Hex digits
// ---------------------------------------------------------------------------

/// A hexadecimal digit char.
pub open spec fn is_hex_char(c: char) -> bool {
    ('0' <= c <= '9') || ('a' <= c <= 'f')
}

/// Hex digits are distinct, hexadecimal, and printable ASCII.
proof fn lemma_hex_digit(d: int, e: int)
    requires
        0 <= d < 16,
        0 <= e < 16,
    ensures
        is_hex_char(hex_digit(d)),
        hex_digit(d) == hex_digit(e) ==> d == e,
{
}

/// The digits of `n` are hex chars, there is at least one, and there is
/// exactly one when `n < 16`.
proof fn lemma_hex_shape(n: nat)
    ensures
        hex(n).len() >= 1,
        hex(n).len() == 1 <==> n < 16,
        forall|i: int| 0 <= i < hex(n).len() ==> is_hex_char(#[trigger] hex(n)[i]),
    decreases n,
{
    if n < 16 {
        lemma_hex_digit(n as int, n as int);
    } else {
        lemma_hex_shape(n / 16);
        lemma_hex_digit((n % 16) as int, (n % 16) as int);
        let h = hex(n);
        assert forall|i: int| 0 <= i < h.len() implies is_hex_char(#[trigger] h[i]) by {
            if i < h.len() - 1 {
                assert(h[i] == hex(n / 16)[i]);
            }
        }
    }
}

/// Different numbers have different digits.
proof fn lemma_hex_injective(a: nat, b: nat)
    requires
        hex(a) == hex(b),
    ensures
        a == b,
    decreases a,
{
    lemma_hex_shape(a);
    lemma_hex_shape(b);
    if a < 16 {
        assert(b < 16);
        assert(hex(a) == seq![hex_digit(a as int)]);
        assert(hex(b) == seq![hex_digit(b as int)]);
        assert(seq![hex_digit(a as int)][0] == hex_digit(a as int));
        assert(seq![hex_digit(b as int)][0] == hex_digit(b as int));
        assert(hex_digit(a as int) == hex_digit(b as int));
        lemma_hex_digit(a as int, b as int);
    } else {
        assert(b >= 16);
        let ha = hex(a);
        let hb = hex(b);
        assert(ha == hex(a / 16).push(hex_digit((a % 16) as int)));
        assert(hb == hex(b / 16).push(hex_digit((b % 16) as int)));
        assert(ha.last() == hb.last());
        lemma_hex_digit((a % 16) as int, (b % 16) as int);
        assert(ha.drop_last() =~= hex(a / 16));
        assert(hb.drop_last() =~= hex(b / 16));
        assert(a % 16 == b % 16);
        lemma_hex_injective(a / 16, b / 16);
        assert(a / 16 == b / 16);
        assert(a == 16 * (a / 16) + a % 16) by (nonlinear_arith);
        assert(b == 16 * (b / 16) + b % 16) by (nonlinear_arith);
    }
}

// ---------------------------------------------------------------------------
// Structure of the escaping
// ---------------------------------------------------------------------------

/// Escaping distributes over concatenation.
pub proof fn lemma_escape_concat(a: Seq<char>, b: Seq<char>)
    ensures
        escape_text(a + b) == escape_text(a) + escape_text(b),
    decreases a.len(),
{
    if a.len() == 0 {
        assert(a + b =~= b);
    } else {
        assert((a + b).drop_first() =~= a.drop_first() + b);
        lemma_escape_concat(a.drop_first(), b);
    }
}

/// Escaping one more char appends its escape.
proof fn lemma_escape_push(s: Seq<char>, c: char)
    ensures
        escape_text(s.push(c)) == escape_text(s) + escape_char(c),
{
    let one = seq![c];
    lemma_escape_concat(s, one);
    assert(s + one =~= s.push(c));
    assert(one[0] == c);
    assert(one.drop_first() =~= Seq::<char>::empty());
    assert(escape_text(one.drop_first()) == Seq::<char>::empty());
    assert(escape_text(one) == escape_char(c) + escape_text(one.drop_first()));
    assert(escape_text(one) =~= escape_char(c));
}

/// Printable ASCII.
pub open spec fn is_printable(c: char) -> bool {
    0x20 <= c as u32 <= 0x7e
}

/// Every char of one escape is printable ASCII, and the escape is never
/// empty.
proof fn lemma_escape_char_printable(c: char)
    ensures
        escape_char(c).len() >= 1,
        forall|i: int| 0 <= i < escape_char(c).len() ==> is_printable(#[trigger] escape_char(c)[i]),
{
    if !(c == '\t' || c == '\r' || c == '\n' || c == '\\' || c == '\'' || c == '"' || (0x20
        <= c as u32 <= 0x7e)) {
        let h = hex(c as u32 as nat);
        lemma_hex_shape(c as u32 as nat);
        let e = escape_char(c);
        assert forall|i: int| 0 <= i < e.len() implies is_printable(#[trigger] e[i]) by {
            if 3 <= i < 3 + h.len() {
                assert(e[i] == h[i - 3]);
                assert(is_hex_char(h[i - 3]));
            }
        }
    }
}

/// Every output char is printable ASCII (`0x20..=0x7E`): no control char,
/// newline, escape sequence, NUL or non-ASCII char survives.
pub proof fn lemma_escape_printable(s: Seq<char>)
    ensures
        forall|i: int| 0 <= i < escape_text(s).len() ==> is_printable(#[trigger] escape_text(s)[i]),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_escape_char_printable(s[0]);
        lemma_escape_printable(s.drop_first());
        let e = escape_char(s[0]);
        let r = escape_text(s.drop_first());
        let t = escape_text(s);
        assert(t == e + r);
        assert forall|i: int| 0 <= i < t.len() implies is_printable(#[trigger] t[i]) by {
            if i < e.len() {
                assert(t[i] == e[i]);
            } else {
                assert(t[i] == r[i - e.len()]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Quote safety
// ---------------------------------------------------------------------------

/// A reader of a `"`-delimited string that honors backslash escapes, reading
/// `t` from inside the string (`escaped`: right after an escaping
/// backslash). `None` once an unescaped `"` has closed the string;
/// otherwise `Some(e)`, where `e` says whether the reader ends right after
/// an escaping backslash.
pub open spec fn scan_quoted(escaped: bool, t: Seq<char>) -> Option<bool>
    decreases t.len(),
{
    if t.len() == 0 {
        Some(escaped)
    } else if escaped {
        scan_quoted(false, t.drop_first())
    } else if t[0] == '\\' {
        scan_quoted(true, t.drop_first())
    } else if t[0] == '"' {
        None
    } else {
        scan_quoted(false, t.drop_first())
    }
}

/// Reading a concatenation reads the parts in turn.
proof fn lemma_scan_concat(escaped: bool, a: Seq<char>, b: Seq<char>)
    ensures
        scan_quoted(escaped, a + b) == (match scan_quoted(escaped, a) {
            None => None,
            Some(e) => scan_quoted(e, b),
        }),
    decreases a.len(),
{
    if a.len() == 0 {
        assert(a + b =~= b);
    } else {
        assert((a + b).drop_first() =~= a.drop_first() + b);
        assert((a + b)[0] == a[0]);
        if escaped {
            lemma_scan_concat(false, a.drop_first(), b);
        } else if a[0] == '\\' {
            lemma_scan_concat(true, a.drop_first(), b);
        } else if a[0] != '"' {
            lemma_scan_concat(false, a.drop_first(), b);
        }
    }
}

/// Reading hex digits inside the string changes nothing.
proof fn lemma_scan_hex(n: nat)
    ensures
        scan_quoted(false, hex(n)) == Some(false),
    decreases n,
{
    reveal_with_fuel(scan_quoted, 3);
    lemma_hex_shape(n);
    if n < 16 {
        let h = hex(n);
        assert(h.drop_first() =~= Seq::<char>::empty());
    } else {
        lemma_scan_hex(n / 16);
        let d = hex_digit((n % 16) as int);
        lemma_hex_digit((n % 16) as int, (n % 16) as int);
        lemma_scan_concat(false, hex(n / 16), seq![d]);
        assert(hex(n / 16) + seq![d] =~= hex(n));
        assert(seq![d].drop_first() =~= Seq::<char>::empty());
    }
}

/// One escape, read inside the string, leaves the reader inside the string
/// and not after a backslash.
proof fn lemma_scan_escape_char(c: char)
    ensures
        scan_quoted(false, escape_char(c)) == Some(false),
{
    reveal_with_fuel(scan_quoted, 5);
    let e = escape_char(c);
    if c == '\t' || c == '\r' || c == '\n' || c == '\\' || c == '\'' || c == '"' {
        assert(e.drop_first().drop_first() =~= Seq::<char>::empty());
    } else if 0x20 <= c as u32 <= 0x7e {
        assert(e.drop_first() =~= Seq::<char>::empty());
    } else {
        let h = hex(c as u32 as nat);
        let head = seq!['\\', 'u', '{'];
        assert(head.drop_first().drop_first().drop_first() =~= Seq::<char>::empty());
        assert(scan_quoted(false, head) == Some(false));
        lemma_scan_hex(c as u32 as nat);
        assert(seq!['}'].drop_first() =~= Seq::<char>::empty());
        lemma_scan_concat(false, head, h);
        lemma_scan_concat(false, head + h, seq!['}']);
    }
}

/// Quote safety: reading the escaped text inside a `"`-delimited string
/// never meets an unescaped `"` and does not end right after an escaping
/// backslash, so the closing quote the caller appends is the one that ends
/// the string.
pub proof fn lemma_escape_quote_safe(s: Seq<char>)
    ensures
        scan_quoted(false, escape_text(s)) == Some(false),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_scan_escape_char(s[0]);
        lemma_escape_quote_safe(s.drop_first());
        lemma_scan_concat(false, escape_char(s[0]), escape_text(s.drop_first()));
    }
}

// ---------------------------------------------------------------------------
// Injectivity
// ---------------------------------------------------------------------------

/// No escape is a proper prefix of another (with any continuation): the
/// escapes form a prefix code, and different chars have different escapes.
proof fn lemma_escape_prefix_code(c1: char, r1: Seq<char>, c2: char, r2: Seq<char>)
    requires
        escape_char(c1) + r1 == escape_char(c2) + r2,
    ensures
        c1 == c2,
        r1 == r2,
{
    let e1 = escape_char(c1);
    let e2 = escape_char(c2);
    let t = e1 + r1;
    lemma_escape_char_printable(c1);
    lemma_escape_char_printable(c2);
    assert(t[0] == e1[0]);
    assert(t[0] == e2[0]);
    let unicode1 = !(c1 == '\t' || c1 == '\r' || c1 == '\n' || c1 == '\\' || c1 == '\'' || c1
        == '"' || (0x20 <= c1 as u32 <= 0x7e));
    let unicode2 = !(c2 == '\t' || c2 == '\r' || c2 == '\n' || c2 == '\\' || c2 == '\'' || c2
        == '"' || (0x20 <= c2 as u32 <= 0x7e));
    if e1.len() >= 2 && e2.len() >= 2 {
        assert(t[1] == e1[1]);
        assert(t[1] == e2[1]);
    }
    if unicode1 && unicode2 {
        let n1 = c1 as u32 as nat;
        let n2 = c2 as u32 as nat;
        let h1 = hex(n1);
        let h2 = hex(n2);
        lemma_hex_shape(n1);
        lemma_hex_shape(n2);
        let l1 = h1.len() as int;
        let l2 = h2.len() as int;
        if l1 < l2 {
            assert(t[3 + l1] == e1[3 + l1]);
            assert(t[3 + l1] == e2[3 + l1]);
            assert(e2[3 + l1] == h2[l1]);
            assert(is_hex_char(h2[l1]));
        } else if l2 < l1 {
            assert(t[3 + l2] == e2[3 + l2]);
            assert(t[3 + l2] == e1[3 + l2]);
            assert(e1[3 + l2] == h1[l2]);
            assert(is_hex_char(h1[l2]));
        } else {
            assert forall|i: int| 0 <= i < l1 implies h1[i] == h2[i] by {
                assert(t[3 + i] == e1[3 + i]);
                assert(t[3 + i] == e2[3 + i]);
            }
            assert(h1 =~= h2);
            lemma_hex_injective(n1, n2);
            vstd::utf8::char_u32_cast(c1, c1 as u32);
            vstd::utf8::char_u32_cast(c2, c2 as u32);
        }
    }
    assert(e1 == e2);
    assert(r1 =~= t.subrange(e1.len() as int, t.len() as int));
    assert(r2 =~= (e2 + r2).subrange(e2.len() as int, t.len() as int));
}

/// The escaping is injective: equal diagnostics come from equal stored text.
pub proof fn lemma_escape_injective(a: Seq<char>, b: Seq<char>)
    requires
        escape_text(a) == escape_text(b),
    ensures
        a == b,
    decreases a.len(),
{
    if a.len() == 0 && b.len() > 0 {
        lemma_escape_char_printable(b[0]);
        assert(escape_text(b).len() >= 1);
    } else if a.len() > 0 && b.len() == 0 {
        lemma_escape_char_printable(a[0]);
        assert(escape_text(a).len() >= 1);
    } else if a.len() > 0 {
        lemma_escape_prefix_code(
            a[0],
            escape_text(a.drop_first()),
            b[0],
            escape_text(b.drop_first()),
        );
        lemma_escape_injective(a.drop_first(), b.drop_first());
        assert(a =~= seq![a[0]] + a.drop_first());
        assert(b =~= seq![b[0]] + b.drop_first());
    } else {
        assert(a =~= b);
    }
}

// ---------------------------------------------------------------------------
// Executable functions
// ---------------------------------------------------------------------------

/// The hex digit char for `d < 16` (`hex_digit`).
fn hex_digit_char(d: u32) -> (c: char)
    requires
        d < 16,
    ensures
        c == hex_digit(d as int),
{
    match d {
        0 => '0',
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        _ => 'f',
    }
}

/// Appends the hex digits of `n`.
fn push_hex(out: &mut String, n: u32)
    ensures
        final(out)@ == old(out)@ + hex(n as nat),
    decreases n,
{
    if n >= 16 {
        push_hex(out, n / 16);
    }
    out.push(hex_digit_char(n % 16));
    proof {
        if n >= 16 {
            assert(hex(n as nat) == hex((n / 16) as nat).push(hex_digit((n % 16) as int)));
        }
    };
}

/// Appends `char::escape_default(c)`.
fn push_escaped(out: &mut String, c: char)
    ensures
        final(out)@ == old(out)@ + escape_char(c),
{
    let ghost start = out@;
    match c {
        '\t' | '\r' | '\n' => {
            out.push('\\');
            out.push(
                match c {
                    '\t' => 't',
                    '\r' => 'r',
                    _ => 'n',
                },
            );
        },
        '\\' | '\'' | '"' => {
            out.push('\\');
            out.push(c);
        },
        ' '..='~' => out.push(c),
        _ => {
            out.push('\\');
            out.push('u');
            out.push('{');
            let ghost mid = out@;
            push_hex(out, c as u32);
            out.push('}');
            proof {
                assert(mid == start + seq!['\\', 'u', '{']);
            };
        },
    }
    proof {
        assert(out@ =~= start + escape_char(c));
    };
}

/// `value.chars().flat_map(char::escape_default).collect::<String>()`
/// (`escape_text`). Total.
pub fn escape_default(value: &str) -> (escaped: String)
    ensures
        escaped@ == escape_text(value@),
{
    let mut escaped = String::new();
    let mut chars = value.chars();
    let ghost n = value@.len() as int;
    let ghost mut k: int = 0;
    let mut done = false;
    proof {
        assert(value@.subrange(0, 0) =~= Seq::<char>::empty());
    }
    while !done
        invariant
            0 <= k <= n,
            n == value@.len(),
            vstd::std_specs::iter::IteratorSpec::remaining(&chars) == value@.subrange(k, n),
            escaped@ == escape_text(value@.subrange(0, k)),
            done ==> k == n,
        decreases n - k + if done {
            0int
        } else {
            1int
        },
    {
        match chars.next() {
            Some(c) => {
                push_escaped(&mut escaped, c);
                proof {
                    assert(value@.subrange(0, k + 1) =~= value@.subrange(0, k).push(c));
                    lemma_escape_push(value@.subrange(0, k), c);
                    k = k + 1;
                };
            },
            None => {
                done = true;
            },
        }
    }
    proof {
        assert(value@.subrange(0, n) =~= value@);
    }
    escaped
}

} // verus!
