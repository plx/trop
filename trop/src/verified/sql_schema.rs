//! Verified kernel: the schema-SQL tokenizer and normalizer used to compare
//! the live database schema with the expected one.
//!
//! `tokenize_schema_sql` computes the specification tokenizer `tokenize`
//! over the SQL text's UTF-8 bytes. It skips ASCII whitespace and reads, in
//! order of precedence at each position:
//!
//! - a bare word: a maximal run of ASCII letters and digits, `_`, `$` and
//!   non-ASCII bytes, ASCII-lowercased (kind `w`);
//! - a double-quoted identifier with `""` escapes. When it is closed and its
//!   decoded text is exactly the allowed identifier (the table's own name),
//!   it reads as that bare word (kind `w`), otherwise as its raw text
//!   (kind `q`);
//! - a `'...'` or `` `...` `` literal with doubled-delimiter escapes (kind
//!   `'` or `` ` ``), a `[...]` identifier (kind `[`), a `--` line comment or
//!   a `/* */` block comment (kind `c`), each kept as raw text and running to
//!   the end of the input when unterminated;
//! - otherwise an operator of one byte, or of two (`<= >= != == -> <> ||`)
//!   or three (`->>`) bytes (kind `s`).
//!
//! Proved:
//!
//! - the tokenizer is total and terminates on every byte string; every
//!   index is in bounds, so the old `expect("peeked ...")` sites have no
//!   counterpart that could panic;
//! - `normalize_schema_sql` is `encode_tokens(tokenize(..))`, where every
//!   token is written as `kind`, the decimal byte length of its value, `:`,
//!   the value, and `;`. The encoding is injective
//!   (`lemma_encode_tokens_injective`), so two normalized texts are equal
//!   exactly when the token streams are (`lemma_normalized_eq_iff`): the
//!   exact-definition check accepts precisely the texts whose tokens match;
//! - `token_run_contains` is exactly "the needle's tokens occur as a
//!   contiguous run of the haystack's tokens". Text inside a comment or a
//!   quoted literal is part of one token there, so it can never satisfy a
//!   required-constraint check (the old substring search over the encoded
//!   stream could be satisfied that way);
//! - a closed quoted token's raw text is its delimiter, its decoded text with
//!   every delimiter doubled, and its delimiter again
//!   (`lemma_closed_quote_round_trip`);
//! - the tokens are the input with whitespace between tokens removed: every
//!   input byte is either whitespace outside the tokens or part of exactly
//!   one token, whose value is its raw text, ASCII-lowercased for bare words
//!   (`lemma_tokens_cover_input`). So two texts normalize equally only if
//!   they differ just in whitespace between tokens, the ASCII case of bare
//!   words, and the quoting of the allowed identifier;
//! - conversely, inserting ASCII whitespace at any position the tokenizer
//!   reaches (a token boundary, or whitespace between tokens) leaves the
//!   tokens unchanged (`lemma_space_between_tokens`).
//!
//! Byte-level tokenizing agrees with the historical `char`-level tokenizer on
//! every `&str`: every delimiter and operator is ASCII, every byte of a
//! non-ASCII UTF-8 sequence is `>= 0x80` (a word byte, and never a
//! delimiter), and token lengths were byte lengths. The glue's differential
//! test against the historical implementation pins this.
//!
//! Production glue: `validate_required_constraints` and
//! `validate_exact_table_definitions` in `crate::database::validation`.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ---------------------------------------------------------------------------
// Byte classes and token kinds
// ---------------------------------------------------------------------------

/// The token kind of a bare word (and of the allowed quoted identifier).
pub const KIND_WORD: u8 = b'w';

/// The token kind of any other double-quoted identifier.
pub const KIND_QUOTED: u8 = b'q';

/// The token kind of a `[...]` identifier.
pub const KIND_BRACKET: u8 = b'[';

/// The token kind of a `--` or `/* */` comment.
pub const KIND_COMMENT: u8 = b'c';

/// The token kind of an operator or other punctuation.
pub const KIND_SYMBOL: u8 = b's';

/// ASCII whitespace as `u8::is_ascii_whitespace` defines it: space, tab, line
/// feed, form feed and carriage return (not vertical tab).
pub open spec fn is_sql_space(b: u8) -> bool {
    b == 0x20 || b == 0x09 || b == 0x0A || b == 0x0C || b == 0x0D
}

/// A byte of a bare word: ASCII letters and digits, `_`, `$`, and every
/// non-ASCII byte.
pub open spec fn is_word_byte(b: u8) -> bool {
    (0x30 <= b <= 0x39) || (0x41 <= b <= 0x5A) || (0x61 <= b <= 0x7A) || b == 0x5F || b == 0x24
        || b >= 0x80
}

/// The two-byte operators `<= >= != == <> -> ||`.
pub open spec fn is_two_byte_operator(a: u8, b: u8) -> bool {
    ((a == b'<' || a == b'>' || a == b'!' || a == b'=') && b == b'=') || ((a == b'<' || a
        == b'-') && b == b'>') || (a == b'|' && b == b'|')
}

/// ASCII lowercase of one byte (`u8::to_ascii_lowercase`).
pub open spec fn ascii_lower(b: u8) -> u8 {
    if 0x41 <= b <= 0x5A {
        (b + 0x20) as u8
    } else {
        b
    }
}

/// Byte-wise ASCII lowercase.
pub open spec fn lower_seq(s: Seq<u8>) -> Seq<u8> {
    s.map_values(|b: u8| ascii_lower(b))
}

// ---------------------------------------------------------------------------
// The specification tokenizer
// ---------------------------------------------------------------------------

/// One past the end of the run of word bytes starting at `i`.
pub open spec fn word_end(s: Seq<u8>, i: int) -> int
    decreases s.len() - i,
{
    if 0 <= i < s.len() && is_word_byte(s[i]) {
        word_end(s, i + 1)
    } else {
        i
    }
}

/// Reading a token quoted by `d` from position `k` (just after the opening
/// delimiter): one past its end, and whether a closing delimiter ended it.
/// A doubled delimiter stands for one delimiter in the text.
pub open spec fn quote_scan(s: Seq<u8>, k: int, d: u8) -> (int, bool)
    decreases s.len() - k,
{
    if k < 0 || k >= s.len() {
        (s.len() as int, false)
    } else if s[k] == d {
        if k + 1 < s.len() && s[k + 1] == d {
            quote_scan(s, k + 2, d)
        } else {
            (k + 1, true)
        }
    } else {
        quote_scan(s, k + 1, d)
    }
}

/// The decoded text of the token quoted by `d` from position `k`: each
/// doubled delimiter becomes one.
pub open spec fn quote_body(s: Seq<u8>, k: int, d: u8) -> Seq<u8>
    decreases s.len() - k,
{
    if k < 0 || k >= s.len() {
        Seq::empty()
    } else if s[k] == d {
        if k + 1 < s.len() && s[k + 1] == d {
            seq![d] + quote_body(s, k + 2, d)
        } else {
            Seq::empty()
        }
    } else {
        seq![s[k]] + quote_body(s, k + 1, d)
    }
}

/// One past the first `b` at or after `k`, or the end of the input.
pub open spec fn through_byte(s: Seq<u8>, k: int, b: u8) -> int
    decreases s.len() - k,
{
    if k < 0 || k >= s.len() {
        s.len() as int
    } else if s[k] == b {
        k + 1
    } else {
        through_byte(s, k + 1, b)
    }
}

/// One past the first `*/` starting at or after `k`, or the end of the input.
pub open spec fn block_comment_end(s: Seq<u8>, k: int) -> int
    decreases s.len() - k,
{
    if k < 0 || k >= s.len() {
        s.len() as int
    } else if s[k] == b'*' && k + 1 < s.len() && s[k + 1] == b'/' {
        k + 2
    } else {
        block_comment_end(s, k + 1)
    }
}

/// One past the end of the operator starting at `i`.
pub open spec fn operator_end(s: Seq<u8>, i: int) -> int {
    if i + 1 < s.len() && is_two_byte_operator(s[i], s[i + 1]) {
        if s[i] == b'-' && s[i + 1] == b'>' && i + 2 < s.len() && s[i + 2] == b'>' {
            i + 3
        } else {
            i + 2
        }
    } else {
        i + 1
    }
}

/// A token: its kind and its value.
// Only specifications use this alias. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint.
#[allow(dead_code)]
pub type Token = (u8, Seq<u8>);

/// The token starting at the non-whitespace position `i`, and one past its
/// end.
pub open spec fn scan_token(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>) -> (Token, int) {
    let c = s[i];
    if is_word_byte(c) {
        let e = word_end(s, i);
        ((KIND_WORD, lower_seq(s.subrange(i, e))), e)
    } else if c == b'"' {
        let e = quote_scan(s, i + 1, c).0;
        let decoded = quote_body(s, i + 1, c);
        if quote_scan(s, i + 1, c).1 && allowed == Some(decoded) {
            ((KIND_WORD, lower_seq(decoded)), e)
        } else {
            ((KIND_QUOTED, s.subrange(i, e)), e)
        }
    } else if c == b'\'' || c == b'`' {
        let e = quote_scan(s, i + 1, c).0;
        ((c, s.subrange(i, e)), e)
    } else if c == b'[' {
        let e = through_byte(s, i + 1, b']');
        ((KIND_BRACKET, s.subrange(i, e)), e)
    } else if c == b'-' && i + 1 < s.len() && s[i + 1] == b'-' {
        let e = through_byte(s, i + 2, b'\n');
        ((KIND_COMMENT, s.subrange(i, e)), e)
    } else if c == b'/' && i + 1 < s.len() && s[i + 1] == b'*' {
        let e = block_comment_end(s, i + 2);
        ((KIND_COMMENT, s.subrange(i, e)), e)
    } else {
        let e = operator_end(s, i);
        ((KIND_SYMBOL, s.subrange(i, e)), e)
    }
}

/// The tokens of `s` from position `i` on.
pub open spec fn lex(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>) -> Seq<Token>
    decreases s.len() - i,
    via lex_decreases
{
    if i < 0 || i >= s.len() {
        Seq::empty()
    } else if is_sql_space(s[i]) {
        lex(s, i + 1, allowed)
    } else {
        seq![scan_token(s, i, allowed).0] + lex(s, scan_token(s, i, allowed).1, allowed)
    }
}

#[via_fn]
proof fn lex_decreases(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>) {
    if 0 <= i < s.len() && !is_sql_space(s[i]) {
        lemma_scan_token_bounds(s, i, allowed);
    }
}

/// The tokens of `s`. `allowed` is the one double-quoted identifier that
/// reads as a bare word.
pub open spec fn tokenize(s: Seq<u8>, allowed: Option<Seq<u8>>) -> Seq<Token> {
    lex(s, 0, allowed)
}

// ---------------------------------------------------------------------------
// Scanning bounds
// ---------------------------------------------------------------------------

/// A word ends within the input, not before it starts.
pub proof fn lemma_word_end_bounds(s: Seq<u8>, i: int)
    requires
        0 <= i <= s.len(),
    ensures
        i <= word_end(s, i) <= s.len(),
        forall|j: int| i <= j < word_end(s, i) ==> is_word_byte(#[trigger] s[j]),
        word_end(s, i) < s.len() ==> !is_word_byte(s[word_end(s, i)]),
    decreases s.len() - i,
{
    if i < s.len() && is_word_byte(s[i]) {
        lemma_word_end_bounds(s, i + 1);
    }
}

/// A quoted token ends within the input, after its start.
pub proof fn lemma_quote_scan_bounds(s: Seq<u8>, k: int, d: u8)
    requires
        0 <= k <= s.len(),
    ensures
        k <= quote_scan(s, k, d).0 <= s.len(),
        quote_scan(s, k, d).1 ==> k < quote_scan(s, k, d).0 && s[quote_scan(s, k, d).0 - 1] == d,
        !quote_scan(s, k, d).1 ==> quote_scan(s, k, d).0 == s.len(),
    decreases s.len() - k,
{
    if k < s.len() {
        if s[k] == d {
            if k + 1 < s.len() && s[k + 1] == d {
                lemma_quote_scan_bounds(s, k + 2, d);
            }
        } else {
            lemma_quote_scan_bounds(s, k + 1, d);
        }
    }
}

/// `through_byte` ends within the input, after its start (unless at the end).
pub proof fn lemma_through_byte_bounds(s: Seq<u8>, k: int, b: u8)
    requires
        0 <= k <= s.len(),
    ensures
        k <= through_byte(s, k, b) <= s.len(),
        k < s.len() ==> k < through_byte(s, k, b),
    decreases s.len() - k,
{
    if k < s.len() && s[k] != b {
        lemma_through_byte_bounds(s, k + 1, b);
    }
}

/// A block comment ends within the input.
pub proof fn lemma_block_comment_end_bounds(s: Seq<u8>, k: int)
    requires
        0 <= k <= s.len(),
    ensures
        k <= block_comment_end(s, k) <= s.len(),
        k < s.len() ==> k < block_comment_end(s, k),
    decreases s.len() - k,
{
    if k < s.len() && !(s[k] == b'*' && k + 1 < s.len() && s[k + 1] == b'/') {
        lemma_block_comment_end_bounds(s, k + 1);
    }
}

/// Every token is non-empty and ends within the input.
pub proof fn lemma_scan_token_bounds(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>)
    requires
        0 <= i < s.len(),
    ensures
        i < scan_token(s, i, allowed).1 <= s.len(),
{
    let c = s[i];
    if is_word_byte(c) {
        lemma_word_end_bounds(s, i);
        lemma_word_end_bounds(s, i + 1);
    } else if c == b'"' || c == b'\'' || c == b'`' {
        lemma_quote_scan_bounds(s, i + 1, c);
    } else if c == b'[' {
        lemma_through_byte_bounds(s, i + 1, b']');
    } else if c == b'-' && i + 1 < s.len() && s[i + 1] == b'-' {
        lemma_through_byte_bounds(s, i + 2, b'\n');
    } else if c == b'/' && i + 1 < s.len() && s[i + 1] == b'*' {
        lemma_block_comment_end_bounds(s, i + 2);
    }
}

// ---------------------------------------------------------------------------
// Quoted tokens
// ---------------------------------------------------------------------------

/// `x` with every `d` doubled: how a quoted token writes its text.
pub open spec fn double_delims(x: Seq<u8>, d: u8) -> Seq<u8>
    decreases x.len(),
{
    if x.len() == 0 {
        Seq::empty()
    } else {
        (if x[0] == d {
            seq![d, d]
        } else {
            seq![x[0]]
        }) + double_delims(x.drop_first(), d)
    }
}

/// A closed quoted token is exactly its decoded text with every delimiter
/// doubled, followed by the closing delimiter: the decoding is the inverse
/// of SQL's quoting.
pub proof fn lemma_closed_quote_round_trip(s: Seq<u8>, k: int, d: u8)
    requires
        0 <= k <= s.len(),
        quote_scan(s, k, d).1,
    ensures
        s.subrange(k, quote_scan(s, k, d).0) == double_delims(quote_body(s, k, d), d) + seq![d],
    decreases s.len() - k,
{
    lemma_quote_scan_bounds(s, k, d);
    let e = quote_scan(s, k, d).0;
    let body = quote_body(s, k, d);
    if s[k] == d {
        if k + 1 < s.len() && s[k + 1] == d {
            lemma_closed_quote_round_trip(s, k + 2, d);
            lemma_quote_scan_bounds(s, k + 2, d);
            assert(body == seq![d] + quote_body(s, k + 2, d));
            assert(body.drop_first() =~= quote_body(s, k + 2, d));
            assert(double_delims(body, d) == seq![d, d] + double_delims(quote_body(s, k + 2, d), d));
            assert(s.subrange(k, e) =~= seq![d, d] + s.subrange(k + 2, e));
        } else {
            assert(body =~= Seq::<u8>::empty());
            assert(s.subrange(k, e) =~= seq![d]);
        }
    } else {
        lemma_closed_quote_round_trip(s, k + 1, d);
        lemma_quote_scan_bounds(s, k + 1, d);
        assert(body == seq![s[k]] + quote_body(s, k + 1, d));
        assert(body.drop_first() =~= quote_body(s, k + 1, d));
        assert(s.subrange(k, e) =~= seq![s[k]] + s.subrange(k + 1, e));
    }
}

// ---------------------------------------------------------------------------
// The encoding
// ---------------------------------------------------------------------------

/// An ASCII decimal digit.
pub open spec fn is_digit(b: u8) -> bool {
    0x30 <= b <= 0x39
}

/// The decimal text of `n`, without leading zeros (`0` is `"0"`).
pub open spec fn decimal(n: nat) -> Seq<u8>
    decreases n,
{
    if n < 10 {
        seq![(0x30 + n) as u8]
    } else {
        decimal(n / 10) + seq![(0x30 + n % 10) as u8]
    }
}

/// The number a string of decimal digits denotes.
pub open spec fn decimal_value(ds: Seq<u8>) -> int
    decreases ds.len(),
{
    if ds.len() == 0 {
        0
    } else {
        decimal_value(ds.drop_last()) * 10 + (ds.last() - 0x30)
    }
}

/// One token's encoding: its kind, the decimal byte length of its value,
/// `:`, the value, and `;`.
pub open spec fn encode_token(t: Token) -> Seq<u8> {
    seq![t.0] + decimal(t.1.len()) + seq![b':'] + t.1 + seq![b';']
}

/// The encoding of a token stream: its tokens' encodings, in order.
pub open spec fn encode_tokens(ts: Seq<Token>) -> Seq<u8>
    decreases ts.len(),
{
    if ts.len() == 0 {
        Seq::empty()
    } else {
        encode_token(ts[0]) + encode_tokens(ts.drop_first())
    }
}

/// The normalized form of SQL text: the encoding of its tokens.
pub open spec fn normalized(s: Seq<u8>, allowed: Option<Seq<u8>>) -> Seq<u8> {
    encode_tokens(tokenize(s, allowed))
}

/// `decimal(n)` is a non-empty string of digits denoting `n`.
pub proof fn lemma_decimal(n: nat)
    ensures
        decimal(n).len() >= 1,
        forall|j: int| 0 <= j < decimal(n).len() ==> is_digit(#[trigger] decimal(n)[j]),
        decimal_value(decimal(n)) == n,
    decreases n,
{
    if n >= 10 {
        lemma_decimal(n / 10);
        let ds = decimal(n);
        let q = (n / 10) as nat;
        let r = n % 10;
        assert(ds.drop_last() =~= decimal(q));
        assert(ds.last() == (0x30 + r) as u8);
        assert(decimal_value(ds) == decimal_value(decimal(q)) * 10 + r);
        assert(n == q * 10 + r) by (nonlinear_arith)
            requires
                q == n / 10,
                r == n % 10,
        ;
    } else {
        let ds = decimal(n);
        assert(ds.drop_last() =~= Seq::<u8>::empty());
        assert(ds.last() == (0x30 + n) as u8);
        assert(decimal_value(ds) == decimal_value(ds.drop_last()) * 10 + (ds.last() - 0x30));
        assert(decimal_value(Seq::<u8>::empty()) == 0);
    }
}

/// Distinct numbers have distinct decimal texts.
pub proof fn lemma_decimal_injective(a: nat, b: nat)
    requires
        decimal(a) == decimal(b),
    ensures
        a == b,
{
    lemma_decimal(a);
    lemma_decimal(b);
}

/// A decimal text followed by `:` can be split in only one way.
pub proof fn lemma_decimal_colon_split(a: nat, b: nat, r1: Seq<u8>, r2: Seq<u8>)
    requires
        decimal(a) + seq![b':'] + r1 == decimal(b) + seq![b':'] + r2,
    ensures
        a == b,
        r1 == r2,
{
    let da = decimal(a);
    let db = decimal(b);
    let x = da + seq![b':'] + r1;
    let y = db + seq![b':'] + r2;
    lemma_decimal(a);
    lemma_decimal(b);
    if da.len() < db.len() {
        assert(x[da.len() as int] == b':');
        assert(y[da.len() as int] == db[da.len() as int]);
        assert(is_digit(db[da.len() as int]));
    } else if db.len() < da.len() {
        assert(y[db.len() as int] == b':');
        assert(x[db.len() as int] == da[db.len() as int]);
        assert(is_digit(da[db.len() as int]));
    }
    assert(da.len() == db.len());
    assert(da =~= x.subrange(0, da.len() as int));
    assert(db =~= y.subrange(0, db.len() as int));
    lemma_decimal_injective(a, b);
    assert(r1 =~= x.subrange(da.len() as int + 1, x.len() as int));
    assert(r2 =~= y.subrange(db.len() as int + 1, y.len() as int));
}

/// One token's encoding, followed by anything, can be read back in only one
/// way.
pub proof fn lemma_encode_token_split(x: Token, y: Token, ex: Seq<u8>, ey: Seq<u8>)
    requires
        encode_token(x) + ex == encode_token(y) + ey,
    ensures
        x == y,
        ex == ey,
{
    let a = encode_token(x) + ex;
    let b = encode_token(y) + ey;
    assert(a[0] == x.0);
    assert(b[0] == y.0);
    let tx = x.1 + seq![b';'] + ex;
    let ty = y.1 + seq![b';'] + ey;
    assert(a.drop_first() =~= decimal(x.1.len()) + seq![b':'] + tx);
    assert(b.drop_first() =~= decimal(y.1.len()) + seq![b':'] + ty);
    lemma_decimal_colon_split(x.1.len(), y.1.len(), tx, ty);
    assert(x.1 =~= tx.subrange(0, x.1.len() as int));
    assert(y.1 =~= ty.subrange(0, y.1.len() as int));
    assert(ex =~= tx.subrange(x.1.len() as int + 1, tx.len() as int));
    assert(ey =~= ty.subrange(y.1.len() as int + 1, ty.len() as int));
}

/// Only the empty token stream encodes to the empty text.
pub proof fn lemma_encode_tokens_empty(ts: Seq<Token>)
    ensures
        (encode_tokens(ts).len() == 0) == (ts.len() == 0),
{
    if ts.len() > 0 {
        let first = encode_token(ts[0]);
        assert(first[0] == ts[0].0);
        assert(encode_tokens(ts) == first + encode_tokens(ts.drop_first()));
    }
}

/// The encoding is injective: the token stream can be read back from it.
pub proof fn lemma_encode_tokens_injective(xs: Seq<Token>, ys: Seq<Token>)
    requires
        encode_tokens(xs) == encode_tokens(ys),
    ensures
        xs == ys,
    decreases xs.len(),
{
    lemma_encode_tokens_empty(xs);
    lemma_encode_tokens_empty(ys);
    if xs.len() == 0 || ys.len() == 0 {
        assert(xs =~= ys);
    } else {
        assert(encode_tokens(xs) == encode_token(xs[0]) + encode_tokens(xs.drop_first()));
        assert(encode_tokens(ys) == encode_token(ys[0]) + encode_tokens(ys.drop_first()));
        lemma_encode_token_split(
            xs[0],
            ys[0],
            encode_tokens(xs.drop_first()),
            encode_tokens(ys.drop_first()),
        );
        lemma_encode_tokens_injective(xs.drop_first(), ys.drop_first());
        assert(xs =~= seq![xs[0]] + xs.drop_first());
        assert(ys =~= seq![ys[0]] + ys.drop_first());
    }
}

/// Two SQL texts normalize equally exactly when their token streams are
/// equal.
pub proof fn lemma_normalized_eq_iff(
    a: Seq<u8>,
    allowed_a: Option<Seq<u8>>,
    b: Seq<u8>,
    allowed_b: Option<Seq<u8>>,
)
    ensures
        normalized(a, allowed_a) == normalized(b, allowed_b) <==> tokenize(a, allowed_a)
            == tokenize(b, allowed_b),
{
    if normalized(a, allowed_a) == normalized(b, allowed_b) {
        lemma_encode_tokens_injective(tokenize(a, allowed_a), tokenize(b, allowed_b));
    }
}

/// Appending a token appends its encoding.
pub proof fn lemma_encode_tokens_push(ts: Seq<Token>, t: Token)
    ensures
        encode_tokens(ts.push(t)) == encode_tokens(ts) + encode_token(t),
    decreases ts.len(),
{
    if ts.len() == 0 {
        assert(ts.push(t).drop_first() =~= Seq::<Token>::empty());
    } else {
        lemma_encode_tokens_push(ts.drop_first(), t);
        assert(ts.push(t).drop_first() =~= ts.drop_first().push(t));
    }
}

// ---------------------------------------------------------------------------
// Coverage: the tokens are the input, less the whitespace between them
// ---------------------------------------------------------------------------

/// The spans `(start, end)` of the tokens of `s` from position `i` on, in
/// the order `lex` produces the tokens.
pub open spec fn lex_spans(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>) -> Seq<(int, int)>
    decreases s.len() - i,
    via lex_spans_decreases
{
    if i < 0 || i >= s.len() {
        Seq::empty()
    } else if is_sql_space(s[i]) {
        lex_spans(s, i + 1, allowed)
    } else {
        seq![(i, scan_token(s, i, allowed).1)] + lex_spans(s, scan_token(s, i, allowed).1, allowed)
    }
}

#[via_fn]
proof fn lex_spans_decreases(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>) {
    if 0 <= i < s.len() && !is_sql_space(s[i]) {
        lemma_scan_token_bounds(s, i, allowed);
    }
}

/// A token's value is determined by its raw text: a bare word is its raw
/// text (word bytes only) in ASCII lowercase; the allowed quoted identifier
/// is the allowed name, quoted with doubled `"`, in ASCII lowercase; every
/// other token is exactly its raw text, which does not start with a word
/// byte.
pub open spec fn token_text(t: Token, raw: Seq<u8>, allowed: Option<Seq<u8>>) -> bool {
    if t.0 == KIND_WORD {
        ||| (raw.len() >= 1 && (forall|k: int| 0 <= k < raw.len() ==> is_word_byte(#[trigger] raw[k]))
            && t.1 == lower_seq(raw))
        ||| (allowed matches Some(a) && raw == seq![b'"'] + double_delims(a, b'"') + seq![b'"']
            && t.1 == lower_seq(a))
    } else {
        raw.len() >= 1 && !is_word_byte(raw[0]) && t.1 == raw
    }
}

/// Position `p` lies in none of the spans.
pub open spec fn uncovered(spans: Seq<(int, int)>, p: int) -> bool {
    forall|k: int| 0 <= k < spans.len() ==> !((#[trigger] spans[k]).0 <= p < spans[k].1)
}

/// The spans tile `s[i..]`: in order, non-empty, within the input, and
/// every byte outside them is ASCII whitespace.
pub open spec fn tiles(s: Seq<u8>, i: int, spans: Seq<(int, int)>) -> bool {
    &&& forall|k: int| 0 <= k < spans.len() ==> i <= (#[trigger] spans[k]).0 < spans[k].1 <= s.len()
    &&& forall|k: int| 0 <= k < spans.len() - 1 ==> (#[trigger] spans[k]).1 <= spans[k + 1].0
    &&& forall|p: int|
        i <= p < s.len() && uncovered(spans, p)
            ==> is_sql_space(#[trigger] s[p])
}

/// One token's value is determined by its raw text (see `token_text`).
pub proof fn lemma_scan_token_text(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>)
    requires
        0 <= i < s.len(),
    ensures
        token_text(scan_token(s, i, allowed).0, s.subrange(i, scan_token(s, i, allowed).1), allowed),
{
    lemma_scan_token_bounds(s, i, allowed);
    let (t, e) = scan_token(s, i, allowed);
    let raw = s.subrange(i, e);
    let c = s[i];
    if is_word_byte(c) {
        lemma_word_end_bounds(s, i);
        assert forall|k: int| 0 <= k < raw.len() implies is_word_byte(#[trigger] raw[k]) by {
            assert(raw[k] == s[i + k]);
        }
    } else if c == b'"' {
        lemma_quote_scan_bounds(s, i + 1, c);
        if quote_scan(s, i + 1, c).1 && allowed == Some(quote_body(s, i + 1, c)) {
            lemma_closed_quote_round_trip(s, i + 1, c);
            let body = quote_body(s, i + 1, c);
            let tail = s.subrange(i + 1, e);
            assert(tail == double_delims(body, c) + seq![c]);
            assert(raw =~= seq![c] + tail);
            assert(seq![c] + tail =~= seq![c] + double_delims(body, c) + seq![c]);
        }
    }
}

/// The tokens of `s[i..]` are the input less the whitespace between them:
/// their spans tile it, and each token's value is determined by its raw
/// text (`token_text`). So two texts with equal token streams differ only in
/// whitespace between tokens, the ASCII case of bare words, and the quoting
/// of the allowed identifier.
pub proof fn lemma_tokens_cover_input(s: Seq<u8>, i: int, allowed: Option<Seq<u8>>)
    requires
        0 <= i <= s.len(),
    ensures
        lex_spans(s, i, allowed).len() == lex(s, i, allowed).len(),
        tiles(s, i, lex_spans(s, i, allowed)),
        forall|k: int|
            0 <= k < lex(s, i, allowed).len() ==> token_text(
                #[trigger] lex(s, i, allowed)[k],
                s.subrange(lex_spans(s, i, allowed)[k].0, lex_spans(s, i, allowed)[k].1),
                allowed,
            ),
    decreases s.len() - i,
{
    let spans = lex_spans(s, i, allowed);
    let ts = lex(s, i, allowed);
    if i == s.len() {
        assert(spans =~= Seq::<(int, int)>::empty());
    } else if is_sql_space(s[i]) {
        lemma_tokens_cover_input(s, i + 1, allowed);
        let rest = lex_spans(s, i + 1, allowed);
        assert(spans == rest);
        assert forall|p: int|
            i <= p < s.len() && uncovered(spans, p)
                implies is_sql_space(#[trigger] s[p]) by {
            if p > i {
                assert(i + 1 <= p);
            }
        }
    } else {
        lemma_scan_token_bounds(s, i, allowed);
        let (t, e) = scan_token(s, i, allowed);
        lemma_tokens_cover_input(s, e, allowed);
        lemma_scan_token_text(s, i, allowed);
        let rest = lex_spans(s, e, allowed);
        let rest_ts = lex(s, e, allowed);
        assert(spans == seq![(i, e)] + rest);
        assert(ts == seq![t] + rest_ts);
        assert forall|k: int| 0 <= k < spans.len() implies i <= (#[trigger] spans[k]).0 < spans[k].1
            <= s.len() by {
            if k > 0 {
                assert(spans[k] == rest[k - 1]);
            }
        }
        assert forall|k: int| 0 <= k < spans.len() - 1 implies (#[trigger] spans[k]).1 <= spans[k
            + 1].0 by {
            if k > 0 {
                assert(spans[k] == rest[k - 1]);
                assert(spans[k + 1] == rest[k]);
            } else {
                assert(spans[1] == rest[0]);
            }
        }
        assert forall|p: int|
            i <= p < s.len() && uncovered(spans, p)
                implies is_sql_space(#[trigger] s[p]) by {
            assert(!(spans[0].0 <= p < spans[0].1));
            assert forall|k: int| 0 <= k < rest.len() implies !((#[trigger] rest[k]).0 <= p < rest[k].1) by {
                assert(spans[k + 1] == rest[k]);
            }
            assert(uncovered(rest, p));
        }
        assert forall|k: int| 0 <= k < ts.len() implies token_text(
            #[trigger] ts[k],
            s.subrange(spans[k].0, spans[k].1),
            allowed,
        ) by {
            if k > 0 {
                assert(ts[k] == rest_ts[k - 1]);
                assert(spans[k] == rest[k - 1]);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Robustness: whitespace between tokens does not matter
// ---------------------------------------------------------------------------

/// `s` with the byte `w` inserted before position `i`.
pub open spec fn insert_byte(s: Seq<u8>, i: int, w: u8) -> Seq<u8> {
    s.subrange(0, i) + seq![w] + s.subrange(i, s.len() as int)
}

/// The lexer, started at `j`, reaches position `i` (a token boundary or a
/// whitespace position).
pub open spec fn visits(s: Seq<u8>, j: int, i: int, allowed: Option<Seq<u8>>) -> bool
    decreases s.len() - j,
    via visits_decreases
{
    if j == i {
        true
    } else if j > i || j < 0 || j >= s.len() {
        false
    } else if is_sql_space(s[j]) {
        visits(s, j + 1, i, allowed)
    } else {
        visits(s, scan_token(s, j, allowed).1, i, allowed)
    }
}

#[via_fn]
proof fn visits_decreases(s: Seq<u8>, j: int, i: int, allowed: Option<Seq<u8>>) {
    if 0 <= j < s.len() && !is_sql_space(s[j]) {
        lemma_scan_token_bounds(s, j, allowed);
    }
}

/// The bytes of `insert_byte(s, i, w)`.
pub proof fn lemma_insert_byte(s: Seq<u8>, i: int, w: u8)
    requires
        0 <= i <= s.len(),
    ensures
        insert_byte(s, i, w).len() == s.len() + 1,
        insert_byte(s, i, w)[i] == w,
        forall|p: int| 0 <= p < i ==> #[trigger] insert_byte(s, i, w)[p] == s[p],
        forall|p: int| i <= p < s.len() ==> #[trigger] insert_byte(s, i, w)[p + 1] == s[p],
{
    let t = insert_byte(s, i, w);
    assert forall|p: int| i <= p < s.len() implies #[trigger] t[p + 1] == s[p] by {
        assert(t[p + 1] == s.subrange(i, s.len() as int)[p - i]);
    }
}

// Shifting: after the inserted byte, every scanner reads the same bytes one
// position later.

proof fn lemma_word_end_shift(s: Seq<u8>, i: int, w: u8, k: int)
    requires
        0 <= i <= k <= s.len(),
    ensures
        word_end(insert_byte(s, i, w), k + 1) == word_end(s, k) + 1,
    decreases s.len() - k,
{
    lemma_insert_byte(s, i, w);
    if k < s.len() && is_word_byte(s[k]) {
        lemma_word_end_shift(s, i, w, k + 1);
    }
}

proof fn lemma_quote_shift(s: Seq<u8>, i: int, w: u8, k: int, d: u8)
    requires
        0 <= i <= k <= s.len(),
    ensures
        quote_scan(insert_byte(s, i, w), k + 1, d) == (
            quote_scan(s, k, d).0 + 1,
            quote_scan(s, k, d).1,
        ),
        quote_body(insert_byte(s, i, w), k + 1, d) == quote_body(s, k, d),
    decreases s.len() - k,
{
    lemma_insert_byte(s, i, w);
    if k < s.len() {
        if s[k] == d {
            if k + 1 < s.len() && s[k + 1] == d {
                lemma_quote_shift(s, i, w, k + 2, d);
            }
        } else {
            lemma_quote_shift(s, i, w, k + 1, d);
        }
    }
}

proof fn lemma_through_byte_shift(s: Seq<u8>, i: int, w: u8, k: int, b: u8)
    requires
        0 <= i <= k <= s.len(),
    ensures
        through_byte(insert_byte(s, i, w), k + 1, b) == through_byte(s, k, b) + 1,
    decreases s.len() - k,
{
    lemma_insert_byte(s, i, w);
    if k < s.len() && s[k] != b {
        lemma_through_byte_shift(s, i, w, k + 1, b);
    }
}

proof fn lemma_block_comment_end_shift(s: Seq<u8>, i: int, w: u8, k: int)
    requires
        0 <= i <= k <= s.len(),
    ensures
        block_comment_end(insert_byte(s, i, w), k + 1) == block_comment_end(s, k) + 1,
    decreases s.len() - k,
{
    lemma_insert_byte(s, i, w);
    if k < s.len() && !(s[k] == b'*' && k + 1 < s.len() && s[k + 1] == b'/') {
        lemma_block_comment_end_shift(s, i, w, k + 1);
    }
}

proof fn lemma_scan_token_shift(s: Seq<u8>, i: int, w: u8, k: int, allowed: Option<Seq<u8>>)
    requires
        0 <= i <= k < s.len(),
    ensures
        scan_token(insert_byte(s, i, w), k + 1, allowed) == (
            scan_token(s, k, allowed).0,
            scan_token(s, k, allowed).1 + 1,
        ),
{
    let t = insert_byte(s, i, w);
    lemma_insert_byte(s, i, w);
    lemma_scan_token_bounds(s, k, allowed);
    let e = scan_token(s, k, allowed).1;
    let c = s[k];
    assert(t[k + 1] == c);
    if k + 1 < s.len() {
        assert(t[k + 2] == s[k + 1]);
    }
    if k + 2 < s.len() {
        assert(t[k + 3] == s[k + 2]);
    }
    if is_word_byte(c) {
        lemma_word_end_shift(s, i, w, k);
    } else if c == b'"' || c == b'\'' || c == b'`' {
        lemma_quote_shift(s, i, w, k + 1, c);
    } else if c == b'[' {
        lemma_through_byte_shift(s, i, w, k + 1, b']');
    } else if c == b'-' && k + 1 < s.len() && s[k + 1] == b'-' {
        lemma_through_byte_shift(s, i, w, k + 2, b'\n');
    } else if c == b'/' && k + 1 < s.len() && s[k + 1] == b'*' {
        lemma_block_comment_end_shift(s, i, w, k + 2);
    }
    assert(t.subrange(k + 1, e + 1) =~= s.subrange(k, e));
}

proof fn lemma_lex_shift(s: Seq<u8>, i: int, w: u8, k: int, allowed: Option<Seq<u8>>)
    requires
        0 <= i <= k <= s.len(),
    ensures
        lex(insert_byte(s, i, w), k + 1, allowed) == lex(s, k, allowed),
    decreases s.len() - k,
{
    let t = insert_byte(s, i, w);
    lemma_insert_byte(s, i, w);
    if k < s.len() {
        assert(t[k + 1] == s[k]);
        if is_sql_space(s[k]) {
            lemma_lex_shift(s, i, w, k + 1, allowed);
        } else {
            lemma_scan_token_bounds(s, k, allowed);
            lemma_scan_token_shift(s, i, w, k, allowed);
            lemma_lex_shift(s, i, w, scan_token(s, k, allowed).1, allowed);
        }
    }
}

// Locality: before the inserted whitespace, every scanner that stops at or
// before it reads the same bytes and stops at the same place.

proof fn lemma_word_end_local(s: Seq<u8>, i: int, w: u8, k: int)
    requires
        0 <= k <= i < s.len(),
        word_end(s, k) <= i,
        is_sql_space(w),
    ensures
        word_end(insert_byte(s, i, w), k) == word_end(s, k),
    decreases i - k,
{
    lemma_insert_byte(s, i, w);
    if k < i {
        if is_word_byte(s[k]) {
            lemma_word_end_local(s, i, w, k + 1);
        }
    } else {
        lemma_word_end_bounds(s, k);
        if is_word_byte(s[k]) {
            lemma_word_end_bounds(s, k + 1);
        }
    }
}

proof fn lemma_quote_local(s: Seq<u8>, i: int, w: u8, k: int, d: u8)
    requires
        0 <= k <= i < s.len(),
        quote_scan(s, k, d).0 <= i,
        is_sql_space(w),
        !is_sql_space(d),
    ensures
        quote_scan(insert_byte(s, i, w), k, d) == quote_scan(s, k, d),
        quote_body(insert_byte(s, i, w), k, d) == quote_body(s, k, d),
    decreases i - k,
{
    lemma_insert_byte(s, i, w);
    lemma_quote_scan_bounds(s, k, d);
    if s[k] == d {
        if k + 1 < s.len() && s[k + 1] == d {
            lemma_quote_scan_bounds(s, k + 2, d);
            lemma_quote_local(s, i, w, k + 2, d);
        } else if k + 1 < i {
            assert(insert_byte(s, i, w)[k + 1] == s[k + 1]);
        }
    } else {
        lemma_quote_scan_bounds(s, k + 1, d);
        lemma_quote_local(s, i, w, k + 1, d);
    }
}

proof fn lemma_through_byte_local(s: Seq<u8>, i: int, w: u8, k: int, b: u8)
    requires
        0 <= k <= i < s.len(),
        through_byte(s, k, b) <= i,
    ensures
        through_byte(insert_byte(s, i, w), k, b) == through_byte(s, k, b),
    decreases i - k,
{
    lemma_insert_byte(s, i, w);
    lemma_through_byte_bounds(s, k, b);
    if s[k] != b {
        lemma_through_byte_bounds(s, k + 1, b);
        lemma_through_byte_local(s, i, w, k + 1, b);
    }
}

proof fn lemma_block_comment_end_local(s: Seq<u8>, i: int, w: u8, k: int)
    requires
        0 <= k <= i < s.len(),
        block_comment_end(s, k) <= i,
    ensures
        block_comment_end(insert_byte(s, i, w), k) == block_comment_end(s, k),
    decreases i - k,
{
    lemma_insert_byte(s, i, w);
    lemma_block_comment_end_bounds(s, k);
    if !(s[k] == b'*' && k + 1 < s.len() && s[k + 1] == b'/') {
        lemma_block_comment_end_bounds(s, k + 1);
        lemma_block_comment_end_local(s, i, w, k + 1);
    } else {
        assert(insert_byte(s, i, w)[k + 1] == s[k + 1]);
    }
}

proof fn lemma_scan_token_local(s: Seq<u8>, i: int, w: u8, j: int, allowed: Option<Seq<u8>>)
    requires
        0 <= j < i < s.len(),
        scan_token(s, j, allowed).1 <= i,
        is_sql_space(w),
    ensures
        scan_token(insert_byte(s, i, w), j, allowed) == scan_token(s, j, allowed),
{
    let t = insert_byte(s, i, w);
    lemma_insert_byte(s, i, w);
    lemma_scan_token_bounds(s, j, allowed);
    let e = scan_token(s, j, allowed).1;
    let c = s[j];
    assert(t[j] == c);
    if j + 1 < i {
        assert(t[j + 1] == s[j + 1]);
    }
    if j + 2 < i {
        assert(t[j + 2] == s[j + 2]);
    }
    if is_word_byte(c) {
        lemma_word_end_local(s, i, w, j);
    } else if c == b'"' || c == b'\'' || c == b'`' {
        lemma_quote_scan_bounds(s, j + 1, c);
        lemma_quote_local(s, i, w, j + 1, c);
    } else if c == b'[' {
        lemma_through_byte_local(s, i, w, j + 1, b']');
    } else if c == b'-' && s[j + 1] == b'-' {
        lemma_through_byte_bounds(s, j + 2, b'\n');
        assert(t[j + 1] == s[j + 1]);
        lemma_through_byte_local(s, i, w, j + 2, b'\n');
    } else if c == b'/' && s[j + 1] == b'*' {
        lemma_block_comment_end_bounds(s, j + 2);
        assert(t[j + 1] == s[j + 1]);
        lemma_block_comment_end_local(s, i, w, j + 2);
    }
    assert(t.subrange(j, e) =~= s.subrange(j, e));
}

proof fn lemma_lex_local(s: Seq<u8>, i: int, w: u8, j: int, allowed: Option<Seq<u8>>)
    requires
        0 <= j <= i < s.len(),
        visits(s, j, i, allowed),
        is_sql_space(w),
    ensures
        lex(insert_byte(s, i, w), j, allowed) == lex(s, j, allowed),
    decreases i - j,
{
    let t = insert_byte(s, i, w);
    lemma_insert_byte(s, i, w);
    if j == i {
        lemma_lex_shift(s, i, w, i, allowed);
    } else if is_sql_space(s[j]) {
        lemma_lex_local(s, i, w, j + 1, allowed);
    } else {
        lemma_scan_token_bounds(s, j, allowed);
        let e = scan_token(s, j, allowed).1;
        lemma_visits_bound(s, e, i, allowed);
        lemma_scan_token_local(s, i, w, j, allowed);
        lemma_lex_local(s, i, w, e, allowed);
    }
}

/// A visited position is not before the start.
proof fn lemma_visits_bound(s: Seq<u8>, j: int, i: int, allowed: Option<Seq<u8>>)
    requires
        visits(s, j, i, allowed),
    ensures
        j <= i,
    decreases s.len() - j,
{
    if j != i && 0 <= j < s.len() && j < i {
        if is_sql_space(s[j]) {
            lemma_visits_bound(s, j + 1, i, allowed);
        } else {
            lemma_scan_token_bounds(s, j, allowed);
            lemma_visits_bound(s, scan_token(s, j, allowed).1, i, allowed);
        }
    }
}

/// Inserting an ASCII whitespace byte at a position the lexer reaches (a
/// token boundary, or inside whitespace between tokens) does not change the
/// tokens. With `lemma_tokens_cover_input`, this makes the exact-definition
/// check precisely "the same tokens, however they are spaced".
pub proof fn lemma_space_between_tokens(s: Seq<u8>, i: int, w: u8, allowed: Option<Seq<u8>>)
    requires
        0 <= i < s.len(),
        visits(s, 0, i, allowed),
        is_sql_space(w),
    ensures
        tokenize(insert_byte(s, i, w), allowed) == tokenize(s, allowed),
{
    lemma_lex_local(s, i, w, 0, allowed);
}

// ---------------------------------------------------------------------------
// Token runs
// ---------------------------------------------------------------------------

/// `needle` occurs as a contiguous run of `hay`.
pub open spec fn contains_run(hay: Seq<Token>, needle: Seq<Token>) -> bool {
    exists|k: int|
        0 <= k && k + needle.len() <= hay.len() && #[trigger] hay.subrange(k, k + needle.len())
            == needle
}

// ---------------------------------------------------------------------------
// Executable tokenizer
// ---------------------------------------------------------------------------

/// A token: its kind (`w`, `q`, `'`, `` ` ``, `[`, `c` or `s`) and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaToken {
    /// The token's kind.
    pub kind: u8,
    /// The token's value: the lowercased word, or the raw text.
    pub value: Vec<u8>,
}

impl SchemaToken {
    /// The token as the specification sees it.
    pub open spec fn spec_token(&self) -> Token {
        (self.kind, self.value@)
    }
}

/// The specification view of a token sequence.
pub open spec fn token_views(ts: Seq<SchemaToken>) -> Seq<Token> {
    ts.map_values(|t: SchemaToken| t.spec_token())
}

/// The specification view of an optional byte slice.
pub open spec fn opt_view(o: Option<&[u8]>) -> Option<Seq<u8>> {
    match o {
        Some(x) => Some(x@),
        None => None,
    }
}

const fn is_space_byte(b: u8) -> (r: bool)
    ensures
        r == is_sql_space(b),
{
    b == b' ' || b == b'\t' || b == b'\n' || b == 0x0C || b == b'\r'
}

const fn is_word(b: u8) -> (r: bool)
    ensures
        r == is_word_byte(b),
{
    (b'0' <= b && b <= b'9') || (b'A' <= b && b <= b'Z') || (b'a' <= b && b <= b'z') || b
        == b'_' || b == b'$' || b >= 0x80
}

const fn two_byte_operator(a: u8, b: u8) -> (r: bool)
    ensures
        r == is_two_byte_operator(a, b),
{
    ((a == b'<' || a == b'>' || a == b'!' || a == b'=') && b == b'=') || ((a == b'<' || a
        == b'-') && b == b'>') || (a == b'|' && b == b'|')
}

const fn lower_byte(b: u8) -> (r: u8)
    ensures
        r == ascii_lower(b),
{
    if b'A' <= b && b <= b'Z' {
        b + 0x20
    } else {
        b
    }
}

fn scan_word_end(s: &[u8], i: usize) -> (e: usize)
    requires
        i <= s.len(),
    ensures
        e == word_end(s@, i as int),
{
    let mut j = i;
    while j < s.len() && is_word(s[j])
        invariant
            i <= j <= s.len(),
            word_end(s@, j as int) == word_end(s@, i as int),
        decreases s.len() - j,
    {
        j += 1;
    }
    j
}

/// Reads a token quoted by `d` from `k`: one past its end, whether it was
/// closed, and its decoded text.
fn scan_quoted(s: &[u8], k: usize, d: u8) -> (r: (usize, bool, Vec<u8>))
    requires
        k <= s.len(),
    ensures
        (r.0 as int, r.1) == quote_scan(s@, k as int, d),
        r.2@ == quote_body(s@, k as int, d),
{
    let mut j = k;
    let mut decoded: Vec<u8> = Vec::new();
    loop
        invariant
            k <= j <= s.len(),
            quote_scan(s@, j as int, d) == quote_scan(s@, k as int, d),
            decoded@ + quote_body(s@, j as int, d) == quote_body(s@, k as int, d),
        decreases s.len() - j,
    {
        if j >= s.len() {
            proof {
                assert(decoded@ + Seq::<u8>::empty() =~= decoded@);
            }
            return (s.len(), false, decoded);
        }
        if s[j] == d {
            if j + 1 < s.len() && s[j + 1] == d {
                proof {
                    assert(decoded@.push(d) + quote_body(s@, j + 2, d) =~= decoded@ + (seq![d]
                        + quote_body(s@, j + 2, d)));
                }
                decoded.push(d);
                j += 2;
            } else {
                proof {
                    assert(decoded@ + Seq::<u8>::empty() =~= decoded@);
                }
                return (j + 1, true, decoded);
            }
        } else {
            proof {
                assert(decoded@.push(s@[j as int]) + quote_body(s@, j + 1, d) =~= decoded@ + (
                seq![s@[j as int]] + quote_body(s@, j + 1, d)));
            }
            decoded.push(s[j]);
            j += 1;
        }
    }
}

fn scan_through_byte(s: &[u8], k: usize, b: u8) -> (e: usize)
    requires
        k <= s.len(),
    ensures
        e == through_byte(s@, k as int, b),
{
    let mut j = k;
    while j < s.len()
        invariant
            k <= j <= s.len(),
            through_byte(s@, j as int, b) == through_byte(s@, k as int, b),
        decreases s.len() - j,
    {
        if s[j] == b {
            return j + 1;
        }
        j += 1;
    }
    s.len()
}

fn scan_block_comment_end(s: &[u8], k: usize) -> (e: usize)
    requires
        k <= s.len(),
    ensures
        e == block_comment_end(s@, k as int),
{
    let mut j = k;
    while j < s.len()
        invariant
            k <= j <= s.len(),
            block_comment_end(s@, j as int) == block_comment_end(s@, k as int),
        decreases s.len() - j,
    {
        if s[j] == b'*' && j + 1 < s.len() && s[j + 1] == b'/' {
            return j + 2;
        }
        j += 1;
    }
    s.len()
}

fn scan_operator_end(s: &[u8], i: usize) -> (e: usize)
    requires
        i < s.len(),
    ensures
        e == operator_end(s@, i as int),
{
    if i + 1 < s.len() && two_byte_operator(s[i], s[i + 1]) {
        if s[i] == b'-' && s[i + 1] == b'>' && i + 2 < s.len() && s[i + 2] == b'>' {
            i + 3
        } else {
            i + 2
        }
    } else {
        i + 1
    }
}

/// `bytes[start..end]`.
fn copy_range(bytes: &[u8], start: usize, end: usize) -> (copy: Vec<u8>)
    requires
        start <= end <= bytes.len(),
    ensures
        copy@ == bytes@.subrange(start as int, end as int),
{
    let mut copy: Vec<u8> = Vec::new();
    let mut j = start;
    while j < end
        invariant
            start <= j <= end <= bytes.len(),
            copy@ == bytes@.subrange(start as int, j as int),
        decreases end - j,
    {
        copy.push(bytes[j]);
        proof {
            assert(bytes@.subrange(start as int, j + 1) =~= bytes@.subrange(start as int, j as int).push(
                bytes@[j as int],
            ));
        }
        j += 1;
    }
    copy
}

/// `bytes[start..end]`, ASCII-lowercased.
fn lower_range(bytes: &[u8], start: usize, end: usize) -> (lower: Vec<u8>)
    requires
        start <= end <= bytes.len(),
    ensures
        lower@ == lower_seq(bytes@.subrange(start as int, end as int)),
{
    let mut lower: Vec<u8> = Vec::new();
    let mut j = start;
    while j < end
        invariant
            start <= j <= end <= bytes.len(),
            lower@ == lower_seq(bytes@.subrange(start as int, j as int)),
        decreases end - j,
    {
        lower.push(lower_byte(bytes[j]));
        proof {
            assert(lower_seq(bytes@.subrange(start as int, j + 1)) =~= lower_seq(
                bytes@.subrange(start as int, j as int),
            ).push(ascii_lower(bytes@[j as int])));
        }
        j += 1;
    }
    lower
}

/// The token at the non-whitespace position `pos`, and one past its end.
fn scan_token_exec(sql: &[u8], pos: usize, allowed: Option<&[u8]>) -> (r: (SchemaToken, usize))
    requires
        pos < sql.len(),
    ensures
        (r.0.spec_token(), r.1 as int) == scan_token(sql@, pos as int, opt_view(allowed)),
{
    proof {
        lemma_scan_token_bounds(sql@, pos as int, opt_view(allowed));
    }
    let first = sql[pos];
    if is_word(first) {
        let end = scan_word_end(sql, pos);
        (SchemaToken { kind: KIND_WORD, value: lower_range(sql, pos, end) }, end)
    } else if first == b'"' {
        let (end, closed, decoded) = scan_quoted(sql, pos + 1, first);
        let is_allowed = match allowed {
            Some(name) => closed && super::text::bytes_equal(decoded.as_slice(), name),
            None => false,
        };
        if is_allowed {
            let len = decoded.len();
            (SchemaToken { kind: KIND_WORD, value: lower_range(decoded.as_slice(), 0, len) }, end)
        } else {
            (SchemaToken { kind: KIND_QUOTED, value: copy_range(sql, pos, end) }, end)
        }
    } else if first == b'\'' || first == b'`' {
        let (end, _, _) = scan_quoted(sql, pos + 1, first);
        (SchemaToken { kind: first, value: copy_range(sql, pos, end) }, end)
    } else if first == b'[' {
        let end = scan_through_byte(sql, pos + 1, b']');
        (SchemaToken { kind: KIND_BRACKET, value: copy_range(sql, pos, end) }, end)
    } else if first == b'-' && pos + 1 < sql.len() && sql[pos + 1] == b'-' {
        let end = scan_through_byte(sql, pos + 2, b'\n');
        (SchemaToken { kind: KIND_COMMENT, value: copy_range(sql, pos, end) }, end)
    } else if first == b'/' && pos + 1 < sql.len() && sql[pos + 1] == b'*' {
        let end = scan_block_comment_end(sql, pos + 2);
        (SchemaToken { kind: KIND_COMMENT, value: copy_range(sql, pos, end) }, end)
    } else {
        let end = scan_operator_end(sql, pos);
        (SchemaToken { kind: KIND_SYMBOL, value: copy_range(sql, pos, end) }, end)
    }
}

/// The tokens of `sql` (see `tokenize`). Total: every byte string is
/// tokenized, without panics.
pub fn tokenize_schema_sql(sql: &[u8], allowed: Option<&[u8]>) -> (tokens: Vec<SchemaToken>)
    ensures
        token_views(tokens@) == tokenize(sql@, opt_view(allowed)),
{
    let ghost a = opt_view(allowed);
    let mut tokens: Vec<SchemaToken> = Vec::new();
    let mut i: usize = 0;
    while i < sql.len()
        invariant
            i <= sql.len(),
            a == opt_view(allowed),
            token_views(tokens@) + lex(sql@, i as int, a) == tokenize(sql@, a),
        decreases sql.len() - i,
    {
        if is_space_byte(sql[i]) {
            i += 1;
        } else {
            let (token, end) = scan_token_exec(sql, i, allowed);
            proof {
                lemma_scan_token_bounds(sql@, i as int, a);
                let t = token.spec_token();
                assert(lex(sql@, i as int, a) == seq![t] + lex(sql@, end as int, a));
                assert(token_views(tokens@.push(token)) =~= token_views(tokens@).push(t));
                assert(token_views(tokens@).push(t) + lex(sql@, end as int, a) =~= token_views(
                    tokens@,
                ) + (seq![t] + lex(sql@, end as int, a)));
            }
            tokens.push(token);
            i = end;
        }
    }
    proof {
        assert(token_views(tokens@) + Seq::<Token>::empty() =~= token_views(tokens@));
    }
    tokens
}

/// Appends the decimal text of `n`.
fn push_decimal(out: &mut Vec<u8>, n: usize)
    ensures
        final(out)@ == old(out)@ + decimal(n as nat),
    decreases n,
{
    if n >= 10 {
        push_decimal(out, n / 10);
    }
    let digit = u8::try_from(n % 10).unwrap_or_default();
    proof {
        if n >= 10 {
            assert(old(out)@ + decimal((n / 10) as nat) + seq![(0x30 + n % 10) as u8] =~= old(
                out,
            )@ + decimal(n as nat));
        } else {
            assert(n % 10 == n);
        }
    }
    out.push(b'0' + digit);
}

/// Appends `bytes`.
fn push_bytes(out: &mut Vec<u8>, bytes: &[u8])
    ensures
        final(out)@ == old(out)@ + bytes@,
{
    let mut j: usize = 0;
    while j < bytes.len()
        invariant
            j <= bytes.len(),
            out@ == old(out)@ + bytes@.subrange(0, j as int),
        decreases bytes.len() - j,
    {
        out.push(bytes[j]);
        proof {
            assert(bytes@.subrange(0, j + 1) =~= bytes@.subrange(0, j as int).push(bytes@[j as int]));
        }
        j += 1;
    }
    proof {
        assert(bytes@.subrange(0, bytes@.len() as int) =~= bytes@);
    };
}

/// The encoding of `tokens` (see `encode_tokens`).
pub fn encode_schema_tokens(tokens: &[SchemaToken]) -> (out: Vec<u8>)
    ensures
        out@ == encode_tokens(token_views(tokens@)),
{
    let mut out: Vec<u8> = Vec::new();
    let mut i: usize = 0;
    while i < tokens.len()
        invariant
            i <= tokens.len(),
            out@ == encode_tokens(token_views(tokens@.subrange(0, i as int))),
        decreases tokens.len() - i,
    {
        let token = &tokens[i];
        out.push(token.kind);
        push_decimal(&mut out, token.value.len());
        out.push(b':');
        push_bytes(&mut out, token.value.as_slice());
        out.push(b';');
        proof {
            let prefix = token_views(tokens@.subrange(0, i as int));
            lemma_encode_tokens_push(prefix, token.spec_token());
            assert(token_views(tokens@.subrange(0, i + 1)) =~= prefix.push(token.spec_token()));
        }
        i += 1;
    }
    proof {
        assert(tokens@.subrange(0, tokens@.len() as int) =~= tokens@);
    }
    out
}

/// The normalized form of `sql` (see `normalized`): equal for two texts
/// exactly when their token streams are equal (`lemma_normalized_eq_iff`).
pub fn normalize_schema_sql(sql: &[u8], allowed: Option<&[u8]>) -> (out: Vec<u8>)
    ensures
        out@ == normalized(sql@, opt_view(allowed)),
{
    let tokens = tokenize_schema_sql(sql, allowed);
    encode_schema_tokens(tokens.as_slice())
}

fn tokens_equal(a: &SchemaToken, b: &SchemaToken) -> (equal: bool)
    ensures
        equal == (a.spec_token() == b.spec_token()),
{
    a.kind == b.kind && super::text::bytes_equal(a.value.as_slice(), b.value.as_slice())
}

/// Whether `needle` occurs at position `k` of `hay`.
fn run_at(hay: &[SchemaToken], needle: &[SchemaToken], k: usize) -> (found: bool)
    requires
        k + needle.len() <= hay.len(),
    ensures
        found == (token_views(hay@).subrange(k as int, k + needle.len()) == token_views(needle@)),
{
    let ghost h = token_views(hay@);
    let ghost n = token_views(needle@);
    let mut j: usize = 0;
    while j < needle.len()
        invariant
            k + needle.len() <= hay.len(),
            j <= needle.len(),
            h == token_views(hay@),
            n == token_views(needle@),
            forall|q: int| 0 <= q < j ==> h[k + q] == n[q],
        decreases needle.len() - j,
    {
        if !tokens_equal(&hay[k + j], &needle[j]) {
            proof {
                assert(h.subrange(k as int, k + needle.len())[j as int] != n[j as int]);
            }
            return false;
        }
        j += 1;
    }
    proof {
        assert(h.subrange(k as int, k + needle.len()) =~= n);
    }
    true
}

/// Whether the tokens of `needle` occur as a contiguous run of the tokens of
/// `hay` (see `contains_run`). An empty needle occurs everywhere.
pub fn token_run_contains(hay: &[SchemaToken], needle: &[SchemaToken]) -> (found: bool)
    ensures
        found == contains_run(token_views(hay@), token_views(needle@)),
{
    let ghost h = token_views(hay@);
    let ghost n = token_views(needle@);
    if needle.len() > hay.len() {
        return false;
    }
    let last = hay.len() - needle.len();
    let mut k: usize = 0;
    loop
        invariant
            k <= last,
            last == hay.len() - needle.len(),
            needle.len() <= hay.len(),
            h == token_views(hay@),
            n == token_views(needle@),
            forall|q: int| 0 <= q < k ==> #[trigger] h.subrange(q, q + n.len()) != n,
        decreases last - k,
    {
        if run_at(hay, needle, k) {
            proof {
                assert(h.subrange(k as int, k + n.len()) == n);
            }
            return true;
        }
        if k == last {
            return false;
        }
        k += 1;
    }
}

} // verus!
