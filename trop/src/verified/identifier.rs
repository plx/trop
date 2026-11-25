//! Verified kernel: Unicode whitespace trimming and identifier validation
//! (tags, projects, tasks).
//!
//! - `trimmed` is the specification of `str::trim`: strip leading, then
//!   trailing, Unicode `White_Space` characters (`is_ws`). `trim` computes
//!   it on a `&str` and returns a sub-slice of its input (no copying, and the
//!   slicing is proved never to panic).
//! - `check_identifier` is the one identifier rule for tags, projects and
//!   tasks: the trimmed value must be non-blank, contain no NUL, and be at
//!   most 255 UTF-8 bytes, with errors reported in exactly that precedence.
//! - `is_canonical` characterizes stored identifiers (non-empty and already
//!   trimmed), and the stored-tag sentinel lemmas prove that the schema-v2
//!   encoding of an optional tag (`None` as the empty string) is injective
//!   and round-trips for every tag the writers can produce.
//!
//! Production glue: `crate::identifier` (`trim_identifier`,
//! `check_identifier`, `is_canonical_identifier`), used by `ReservationKey`,
//! `ReservationBuilder` and `ConfigValidator`.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Unicode whitespace and the trim specification
// ---------------------------------------------------------------------------

/// Unicode's `White_Space` property: exactly these 25 code points. This is
/// the set `char::is_whitespace` (and therefore `str::trim`) recognizes.
pub open spec fn is_ws(c: char) -> bool {
    let u = c as u32;
    (0x09 <= u <= 0x0D) || u == 0x20 || u == 0x85 || u == 0xA0 || u == 0x1680 || (0x2000 <= u
        <= 0x200A) || u == 0x2028 || u == 0x2029 || u == 0x202F || u == 0x205F || u == 0x3000
}

/// The table above is exactly `vstd`'s model of `char::is_whitespace`
/// (`vstd::std_specs::char::is_white_space`), which the executable code
/// calls; the trusted link from that model to the real `std` function is
/// checked exhaustively by a unit test in `crate::identifier`.
pub proof fn lemma_is_ws_is_char_is_whitespace(c: char)
    ensures
        is_ws(c) == vstd::std_specs::char::is_white_space(c),
{
    vstd::utf8::char_u32_cast(c, c as u32);
}

/// Every char of `s` is whitespace (vacuously true for the empty string).
pub open spec fn all_ws(s: Seq<char>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] is_ws(s[i])
}

/// `str::trim_start`: drop leading whitespace.
pub open spec fn trim_start(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() > 0 && is_ws(s[0]) {
        trim_start(s.drop_first())
    } else {
        s
    }
}

/// `str::trim_end`: drop trailing whitespace.
pub open spec fn trim_end(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() > 0 && is_ws(s.last()) {
        trim_end(s.drop_last())
    } else {
        s
    }
}

/// `str::trim`: drop leading and trailing whitespace.
pub open spec fn trimmed(s: Seq<char>) -> Seq<char> {
    trim_end(trim_start(s))
}

/// `(a, b)` delimit the trimmed part of `s`: everything before `a` and from
/// `b` on is whitespace, and (when non-empty) `s[a..b]` starts and ends with a
/// non-whitespace char, so no further trimming is possible.
pub open spec fn are_trim_bounds(s: Seq<char>, a: int, b: int) -> bool {
    &&& 0 <= a <= b <= s.len()
    &&& all_ws(s.subrange(0, a))
    &&& all_ws(s.subrange(b, s.len() as int))
    &&& a < b ==> !is_ws(s[a]) && !is_ws(s[b - 1])
}

/// A stored identifier is canonical: non-empty and equal to its own trim.
pub open spec fn is_canonical(s: Seq<char>) -> bool {
    s.len() > 0 && trimmed(s) == s
}

/// `s` contains a NUL char.
pub open spec fn contains_nul(s: Seq<char>) -> bool {
    exists|i: int| 0 <= i < s.len() && s[i] == '\0'
}

// ---------------------------------------------------------------------------
// Trim lemmas
// ---------------------------------------------------------------------------

proof fn lemma_trim_start_skips(s: Seq<char>, a: int)
    requires
        0 <= a <= s.len(),
        all_ws(s.subrange(0, a)),
        a < s.len() ==> !is_ws(s[a]),
    ensures
        trim_start(s) == s.subrange(a, s.len() as int),
    decreases a,
{
    if a == 0 {
        assert(s.subrange(0, s.len() as int) =~= s);
    } else {
        assert(s.subrange(0, a)[0] == s[0]);
        let t = s.drop_first();
        assert forall|i: int| 0 <= i < a - 1 implies #[trigger] is_ws(t.subrange(0, a - 1)[i]) by {
            assert(t.subrange(0, a - 1)[i] == s.subrange(0, a)[i + 1]);
        }
        lemma_trim_start_skips(t, a - 1);
        assert(t.subrange(a - 1, t.len() as int) =~= s.subrange(a, s.len() as int));
    }
}

proof fn lemma_trim_end_skips(s: Seq<char>, b: int)
    requires
        0 <= b <= s.len(),
        all_ws(s.subrange(b, s.len() as int)),
        b > 0 ==> !is_ws(s[b - 1]),
    ensures
        trim_end(s) == s.subrange(0, b),
    decreases s.len() - b,
{
    if b == s.len() {
        assert(s.subrange(0, b) =~= s);
    } else {
        assert(s.subrange(b, s.len() as int)[s.len() - 1 - b] == s.last());
        let t = s.drop_last();
        assert forall|i: int| 0 <= i < t.len() - b implies #[trigger] is_ws(
            t.subrange(b, t.len() as int)[i],
        ) by {
            assert(t.subrange(b, t.len() as int)[i] == s.subrange(b, s.len() as int)[i]);
        }
        lemma_trim_end_skips(t, b);
        assert(t.subrange(0, b) =~= s.subrange(0, b));
    }
}

/// Characterization: whenever `(a, b)` are trim bounds of `s`,
/// `trimmed(s) == s[a..b]`.
pub proof fn lemma_trimmed_by_bounds(s: Seq<char>, a: int, b: int)
    requires
        are_trim_bounds(s, a, b),
    ensures
        trimmed(s) == s.subrange(a, b),
{
    let n = s.len() as int;
    if a == b {
        assert forall|i: int| 0 <= i < n implies #[trigger] is_ws(s.subrange(0, n)[i]) by {
            if i < a {
                assert(s.subrange(0, a)[i] == s[i]);
            } else {
                assert(s.subrange(b, n)[i - b] == s[i]);
            }
        }
        lemma_trim_start_skips(s, n);
        assert(s.subrange(n, n) =~= Seq::<char>::empty());
        assert(s.subrange(a, b) =~= Seq::<char>::empty());
    } else {
        lemma_trim_start_skips(s, a);
        let t = s.subrange(a, n);
        assert forall|i: int| 0 <= i < t.len() - (b - a) implies #[trigger] is_ws(
            t.subrange(b - a, t.len() as int)[i],
        ) by {
            assert(t.subrange(b - a, t.len() as int)[i] == s.subrange(b, n)[i]);
        }
        lemma_trim_end_skips(t, b - a);
        assert(t.subrange(0, b - a) =~= s.subrange(a, b));
    }
}

/// Every string has trim bounds (unique when the trimmed part is non-empty,
/// see `lemma_trim_bounds_unique`).
pub proof fn lemma_trim_bounds_exist(s: Seq<char>) -> (ab: (int, int))
    ensures
        are_trim_bounds(s, ab.0, ab.1),
    decreases s.len(),
{
    let n = s.len() as int;
    if n == 0 {
        assert(s.subrange(0, 0) =~= Seq::<char>::empty());
        (0, 0)
    } else {
        let t = s.drop_first();
        let (a, b) = lemma_trim_bounds_exist(t);
        if is_ws(s[0]) {
            if a == b {
                assert forall|i: int| 0 <= i < n implies #[trigger] is_ws(s.subrange(0, n)[i]) by {
                    if i > 0 {
                        if i - 1 < a {
                            assert(t.subrange(0, a)[i - 1] == s[i]);
                        } else {
                            assert(t.subrange(b, n - 1)[i - 1 - b] == s[i]);
                        }
                    }
                }
                assert(s.subrange(0, 0) =~= Seq::<char>::empty());
                (0, 0)
            } else {
                assert forall|i: int| 0 <= i < a + 1 implies #[trigger] is_ws(
                    s.subrange(0, a + 1)[i],
                ) by {
                    if i > 0 {
                        assert(t.subrange(0, a)[i - 1] == s[i]);
                    }
                }
                assert(s.subrange(b + 1, n) =~= t.subrange(b, n - 1));
                (a + 1, b + 1)
            }
        } else {
            assert(s.subrange(0, 0) =~= Seq::<char>::empty());
            if a == b {
                assert forall|i: int| 0 <= i < n - 1 implies #[trigger] is_ws(
                    s.subrange(1, n)[i],
                ) by {
                    if i < a {
                        assert(t.subrange(0, a)[i] == s[i + 1]);
                    } else {
                        assert(t.subrange(b, n - 1)[i - b] == s[i + 1]);
                    }
                }
                (0, 1)
            } else {
                assert(s.subrange(b + 1, n) =~= t.subrange(b, n - 1));
                (0, b + 1)
            }
        }
    }
}

/// Trim bounds are unique when the trimmed part is non-empty (when it is
/// empty, every `(a, a)` qualifies and all describe the same empty result).
pub proof fn lemma_trim_bounds_unique(s: Seq<char>, a1: int, b1: int, a2: int, b2: int)
    requires
        are_trim_bounds(s, a1, b1),
        are_trim_bounds(s, a2, b2),
        a1 < b1,
    ensures
        a1 == a2 && b1 == b2,
{
    let n = s.len() as int;
    if a2 > a1 {
        assert(s.subrange(0, a2)[a1] == s[a1]);
    }
    if a2 < a1 {
        if a2 < b2 {
            assert(s.subrange(0, a1)[a2] == s[a2]);
        } else {
            assert(s.subrange(b2, n)[a1 - b2] == s[a1]);
        }
    }
    if b2 < b1 {
        assert(s.subrange(b2, n)[b1 - 1 - b2] == s[b1 - 1]);
    }
    if b2 > b1 {
        assert(s.subrange(b1, n)[b2 - 1 - b1] == s[b2 - 1]);
    }
}

/// The trimmed string is empty exactly when the whole string is whitespace.
pub proof fn lemma_trimmed_empty_iff_all_ws(s: Seq<char>)
    ensures
        trimmed(s).len() == 0 <==> all_ws(s),
        trimmed(s).len() <= s.len(),
{
    let (a, b) = lemma_trim_bounds_exist(s);
    lemma_trimmed_by_bounds(s, a, b);
    let n = s.len() as int;
    if a == b {
        assert forall|i: int| 0 <= i < n implies #[trigger] is_ws(s[i]) by {
            if i < a {
                assert(s.subrange(0, a)[i] == s[i]);
            } else {
                assert(s.subrange(b, n)[i - b] == s[i]);
            }
        }
    } else {
        assert(!is_ws(s[a]));
    }
}

/// `trim` is idempotent: `trimmed(trimmed(s)) == trimmed(s)`.
pub proof fn lemma_trimmed_idempotent(s: Seq<char>)
    ensures
        trimmed(trimmed(s)) == trimmed(s),
{
    let (a, b) = lemma_trim_bounds_exist(s);
    lemma_trimmed_by_bounds(s, a, b);
    let t = s.subrange(a, b);
    assert(t.subrange(0, 0) =~= Seq::<char>::empty());
    if a < b {
        assert(t.subrange(b - a, b - a) =~= Seq::<char>::empty());
        lemma_trimmed_by_bounds(t, 0, b - a);
        assert(t.subrange(0, b - a) =~= t);
    } else {
        lemma_trimmed_by_bounds(t, 0, 0);
    }
}

/// Whitespace padding on either side does not change the trimmed value:
/// `trimmed(w1 + t + w2) == trimmed(t)` for all-whitespace `w1`, `w2`.
pub proof fn lemma_trimmed_padding(w1: Seq<char>, t: Seq<char>, w2: Seq<char>)
    requires
        all_ws(w1),
        all_ws(w2),
    ensures
        trimmed(w1 + t + w2) == trimmed(t),
{
    let s = w1 + t + w2;
    let (a, b) = lemma_trim_bounds_exist(t);
    lemma_trimmed_by_bounds(t, a, b);
    let p = w1.len() as int;
    let m = t.len() as int;
    let n = s.len() as int;
    if a < b {
        assert forall|i: int| 0 <= i < p + a implies #[trigger] is_ws(s.subrange(0, p + a)[i]) by {
            if i >= p {
                assert(t.subrange(0, a)[i - p] == s[i]);
            } else {
                assert(w1[i] == s[i]);
            }
        }
        assert forall|i: int| 0 <= i < n - (p + b) implies #[trigger] is_ws(
            s.subrange(p + b, n)[i],
        ) by {
            if b + i < m {
                assert(t.subrange(b, m)[i] == s[p + b + i]);
            } else {
                assert(w2[b + i - m] == s[p + b + i]);
            }
        }
        assert(s[p + a] == t[a]);
        assert(s[p + b - 1] == t[b - 1]);
        lemma_trimmed_by_bounds(s, p + a, p + b);
        assert(s.subrange(p + a, p + b) =~= t.subrange(a, b));
    } else {
        assert forall|i: int| 0 <= i < n implies #[trigger] is_ws(s.subrange(0, n)[i]) by {
            if p <= i < p + m {
                if i - p < a {
                    assert(t.subrange(0, a)[i - p] == s[i]);
                } else {
                    assert(t.subrange(b, m)[i - p - b] == s[i]);
                }
            } else if i >= p + m {
                assert(w2[i - p - m] == s[i]);
            } else {
                assert(w1[i] == s[i]);
            }
        }
        assert(s.subrange(n, n) =~= Seq::<char>::empty());
        lemma_trimmed_by_bounds(s, n, n);
        assert(t.subrange(a, b) =~= Seq::<char>::empty());
    }
}

/// O(1) characterization of canonical identifiers: non-empty and neither the
/// first nor the last char is whitespace.
pub proof fn lemma_canonical_iff_ends(s: Seq<char>)
    ensures
        is_canonical(s) <==> (s.len() > 0 && !is_ws(s[0]) && !is_ws(s.last())),
{
    let n = s.len() as int;
    if n > 0 && !is_ws(s[0]) && !is_ws(s.last()) {
        assert(s.subrange(0, 0) =~= Seq::<char>::empty());
        assert(s.subrange(n, n) =~= Seq::<char>::empty());
        lemma_trimmed_by_bounds(s, 0, n);
        assert(s.subrange(0, n) =~= s);
    }
    if is_canonical(s) {
        let (a, b) = lemma_trim_bounds_exist(s);
        lemma_trimmed_by_bounds(s, a, b);
        assert(s.subrange(a, b).len() == n);
    }
}

/// Every non-empty trimmed value is canonical, so whatever the writers store
/// passes the stored-row decoders.
pub proof fn lemma_trimmed_is_canonical(s: Seq<char>)
    requires
        trimmed(s).len() > 0,
    ensures
        is_canonical(trimmed(s)),
{
    lemma_trimmed_idempotent(s);
}

/// NUL is not whitespace, so the trimmed value contains a NUL exactly when
/// the raw value does.
pub proof fn lemma_nul_survives_trim(s: Seq<char>)
    ensures
        contains_nul(trimmed(s)) == contains_nul(s),
{
    let (a, b) = lemma_trim_bounds_exist(s);
    lemma_trimmed_by_bounds(s, a, b);
    let t = s.subrange(a, b);
    if contains_nul(s) {
        let i = choose|i: int| 0 <= i < s.len() && s[i] == '\0';
        if i < a {
            assert(s.subrange(0, a)[i] == s[i]);
            assert(is_ws(s[i]));
        } else if i >= b {
            assert(s.subrange(b, s.len() as int)[i - b] == s[i]);
            assert(is_ws(s[i]));
        } else {
            assert(t[i - a] == '\0');
        }
    }
    if contains_nul(t) {
        let j = choose|j: int| 0 <= j < t.len() && t[j] == '\0';
        assert(s[a + j] == '\0');
    }
}

// ---------------------------------------------------------------------------
// The identifier rule
// ---------------------------------------------------------------------------

/// Maximum length of an identifier, in UTF-8 bytes, after trimming.
pub const MAX_IDENTIFIER_BYTES: usize = 255;

/// Why an identifier was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    /// Empty or only whitespace.
    Blank,
    /// The trimmed value contains a NUL char.
    ContainsNul,
    /// The trimmed value is longer than `MAX_IDENTIFIER_BYTES` UTF-8 bytes.
    TooLong,
}

/// The identifier rule, with its error precedence `Blank > ContainsNul >
/// TooLong`: the accepted value is the trimmed input.
pub open spec fn identifier_verdict(s: Seq<char>) -> Result<Seq<char>, IdentifierError> {
    let t = trimmed(s);
    if t.len() == 0 {
        Err(IdentifierError::Blank)
    } else if contains_nul(t) {
        Err(IdentifierError::ContainsNul)
    } else if super::text::utf8_len(t) > MAX_IDENTIFIER_BYTES {
        Err(IdentifierError::TooLong)
    } else {
        Ok(t)
    }
}

/// `s` is an acceptable identifier.
pub open spec fn is_valid_identifier(s: Seq<char>) -> bool {
    identifier_verdict(s) is Ok
}

/// Validating a value that already passed (its stored, trimmed form) passes
/// again with the same result, and that result is canonical.
pub proof fn lemma_revalidation(s: Seq<char>)
    requires
        is_valid_identifier(s),
    ensures
        identifier_verdict(trimmed(s)) == identifier_verdict(s),
        is_canonical(trimmed(s)),
{
    lemma_trimmed_idempotent(s);
}

/// An accepted identifier contains no NUL anywhere, not even in the
/// whitespace that trimming removed.
pub proof fn lemma_valid_identifier_has_no_nul(s: Seq<char>)
    requires
        is_valid_identifier(s),
    ensures
        !contains_nul(s),
{
    lemma_nul_survives_trim(s);
}

// ---------------------------------------------------------------------------
// Schema-v2 stored-tag sentinel
// ---------------------------------------------------------------------------

/// A domain tag is valid when absent, or canonical when present.
pub open spec fn is_valid_tag(tag: Option<Seq<char>>) -> bool {
    match tag {
        None => true,
        Some(t) => is_canonical(t),
    }
}

/// Schema v2 stores an absent tag as the empty string.
pub open spec fn encode_stored_tag(tag: Option<Seq<char>>) -> Seq<char> {
    match tag {
        None => Seq::empty(),
        Some(t) => t,
    }
}

/// Decoding a stored tag: `""` is untagged, canonical text is that tag, and
/// anything else is corrupt (`None`).
pub open spec fn decode_stored_tag(stored: Seq<char>) -> Option<Option<Seq<char>>> {
    if stored.len() == 0 {
        Some(None)
    } else if is_canonical(stored) {
        Some(Some(stored))
    } else {
        None
    }
}

/// Every valid tag round-trips through the stored encoding.
pub proof fn lemma_stored_tag_round_trip(tag: Option<Seq<char>>)
    requires
        is_valid_tag(tag),
    ensures
        decode_stored_tag(encode_stored_tag(tag)) == Some(tag),
{
}

/// The stored encoding is injective on valid tags: in particular a tagged key
/// never aliases the untagged key at the same path.
pub proof fn lemma_stored_tag_injective(a: Option<Seq<char>>, b: Option<Seq<char>>)
    requires
        is_valid_tag(a),
        is_valid_tag(b),
        encode_stored_tag(a) == encode_stored_tag(b),
    ensures
        a == b,
{
    lemma_stored_tag_round_trip(a);
    lemma_stored_tag_round_trip(b);
}

/// Every tag accepted by the identifier rule is stored as a valid tag, so the
/// reader decodes exactly what the writer stored.
pub proof fn lemma_validated_tag_round_trips(s: Seq<char>)
    requires
        is_valid_identifier(s),
    ensures
        is_valid_tag(Some(trimmed(s))),
        decode_stored_tag(encode_stored_tag(Some(trimmed(s)))) == Some(Some(trimmed(s))),
{
    lemma_trimmed_is_canonical(s);
}

// ---------------------------------------------------------------------------
// Executable functions
// ---------------------------------------------------------------------------

/// `all_ws(s[lo..k])` extends by one whitespace char.
proof fn lemma_all_ws_push(s: Seq<char>, lo: int, k: int)
    requires
        0 <= lo <= k < s.len(),
        all_ws(s.subrange(lo, k)),
        is_ws(s[k]),
    ensures
        all_ws(s.subrange(lo, k + 1)),
{
    assert forall|i: int| 0 <= i < k + 1 - lo implies #[trigger] is_ws(
        s.subrange(lo, k + 1)[i],
    ) by {
        if i < k - lo {
            assert(s.subrange(lo, k + 1)[i] == s.subrange(lo, k)[i]);
        }
    }
}

/// One pass over `s`: its trimmed sub-slice, and whether `s` contains a NUL.
fn scan(s: &str) -> (r: (&str, bool))
    ensures
        r.0@ == trimmed(s@),
        r.1 == contains_nul(s@),
{
    let bytes = s.as_bytes();
    let total = bytes.len();
    let mut chars = s.chars();
    let ghost n = s@.len() as int;
    let ghost mut k: int = 0;
    let ghost mut a: int = 0;
    let ghost mut b: int = 0;
    let mut pos: usize = 0;
    let mut start: usize = 0;
    let mut end: usize = 0;
    let mut found = false;
    let mut has_nul = false;
    // Bytes left in `s` means chars left in `chars`: the loop consumes one
    // char per iteration and stops exactly at the end of the string.
    while pos < total
        invariant
            0 <= k <= n,
            n == s@.len(),
            vstd::std_specs::iter::IteratorSpec::remaining(&chars) == s@.subrange(k, n),
            total == super::text::utf8_len(s@),
            super::text::is_char_offset(s@, k, pos as int),
            super::text::is_char_offset(s@, a, start as int),
            super::text::is_char_offset(s@, b, end as int),
            !found ==> a == 0 && b == 0 && all_ws(s@.subrange(0, k)),
            found ==> 0 <= a < b <= k && all_ws(s@.subrange(0, a)) && !is_ws(s@[a]) && !is_ws(
                s@[b - 1],
            ) && all_ws(s@.subrange(b, k)),
            has_nul == (exists|i: int| 0 <= i < k && s@[i] == '\0'),
        decreases total - pos,
    {
        proof {
            super::text::lemma_prefix_boundary(s@, k);
            if k == n {
                assert(s@.subrange(0, k) =~= s@);
            }
        }
        match chars.next() {
            Some(c) => {
                let width = c.len_utf8();
                proof {
                    assert(s@.subrange(0, k + 1) =~= s@.subrange(0, k).push(c));
                    vstd::utf8::encode_utf8_push(s@.subrange(0, k), c);
                    super::text::lemma_prefix_boundary(s@, k + 1);
                }
                let next = pos + width;
                if c.is_whitespace() {
                    proof {
                        lemma_all_ws_push(s@, if found { b } else { 0 }, k);
                    };
                } else {
                    if !found {
                        found = true;
                        start = pos;
                        proof {
                            a = k;
                        };
                    }
                    end = next;
                    proof {
                        b = k + 1;
                        assert(s@.subrange(b, k + 1) =~= Seq::<char>::empty());
                    };
                }
                if c == '\0' {
                    has_nul = true;
                }
                proof {
                    k = k + 1;
                }
                pos = next;
            },
            None => {
                // Unreachable: bytes remain, so chars remain.
                proof {
                    assert(false);
                };
            },
        }
    }
    proof {
        super::text::lemma_prefix_boundary(s@, k);
        assert(s@.subrange(0, n) =~= s@);
        super::text::lemma_prefix_len_injective(s@, k, n);
        assert(s@.subrange(0, k) =~= s@);
        if !found {
            assert(s@.subrange(0, 0) =~= Seq::<char>::empty());
        }
        lemma_trimmed_by_bounds(s@, a, b);
    }
    (super::text::char_slice(s, start, end), has_nul)
}

/// The UTF-8 length of `s` in bytes. (`str::len` is specified with a
/// truncating cast; the byte slice's length carries the `usize` bound.)
fn utf8_byte_len(s: &str) -> (len: usize)
    ensures
        len == super::text::utf8_len(s@),
{
    let bytes = s.as_bytes();
    bytes.len()
}

/// `str::trim`: the sub-slice of `s` without leading and trailing Unicode
/// whitespace.
pub fn trim(s: &str) -> (t: &str)
    ensures
        t@ == trimmed(s@),
{
    scan(s).0
}

/// Applies the identifier rule (see `identifier_verdict`): returns the
/// trimmed value, or the first violated condition in the order
/// `Blank > ContainsNul > TooLong`.
pub fn check_identifier(s: &str) -> (r: Result<&str, IdentifierError>)
    ensures
        r matches Ok(t) ==> identifier_verdict(s@) == Ok::<Seq<char>, IdentifierError>(t@),
        r matches Err(e) ==> identifier_verdict(s@) == Err::<Seq<char>, IdentifierError>(e),
        r is Ok <==> is_valid_identifier(s@),
        r matches Ok(t) ==> t@ == trimmed(s@) && is_canonical(t@) && !contains_nul(s@)
            && super::text::utf8_len(t@) <= MAX_IDENTIFIER_BYTES,
{
    let (t, has_nul) = scan(s);
    proof {
        lemma_nul_survives_trim(s@);
    }
    if t.is_empty() {
        Err(IdentifierError::Blank)
    } else if has_nul {
        Err(IdentifierError::ContainsNul)
    } else if utf8_byte_len(t) > MAX_IDENTIFIER_BYTES {
        Err(IdentifierError::TooLong)
    } else {
        proof {
            lemma_trimmed_is_canonical(s@);
        }
        Ok(t)
    }
}

/// Whether `s` is a canonical stored identifier: non-empty and already
/// trimmed (the check stored-row decoders apply to tags, projects and tasks).
pub fn is_canonical_identifier(s: &str) -> (canonical: bool)
    ensures
        canonical == is_canonical(s@),
{
    let t = trim(s);
    let canonical = !t.is_empty() && utf8_byte_len(t) == utf8_byte_len(s);
    proof {
        let (a, b) = lemma_trim_bounds_exist(s@);
        lemma_trimmed_by_bounds(s@, a, b);
        let n = s@.len() as int;
        if canonical {
            // The trimmed slice has the UTF-8 length of the whole string, so
            // nothing was trimmed: a == 0 and b == n.
            super::text::lemma_prefix_boundary(s@, a);
            super::text::lemma_prefix_boundary(s@, b);
            let tail = s@.subrange(a, n);
            super::text::lemma_prefix_boundary(tail, b - a);
            assert(tail.subrange(0, b - a) =~= s@.subrange(a, b));
            assert(tail.subrange(b - a, n - a) =~= s@.subrange(b, n));
            assert(s@.subrange(0, 0) =~= Seq::<char>::empty());
            assert(s@.subrange(0, n) =~= s@);
            super::text::lemma_prefix_boundary(s@, 0);
            super::text::lemma_prefix_boundary(s@, n);
            super::text::lemma_prefix_len_injective(s@, a, 0);
            super::text::lemma_prefix_len_injective(s@, b, n);
            assert(s@.subrange(a, b) =~= s@);
        }
    }
    canonical
}

} // verus!
