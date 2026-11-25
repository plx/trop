//! Verified kernel: rendering of reservation-group output, deterministic
//! ordering, and escaping of user text for human-readable output.
//!
//! - Sequence vocabulary: `split_on` (exactly `str::split` with a single
//!   separator), `join_with`, and `lemma_split_join` (splitting a join of
//!   separator-free parts recovers the parts).
//! - `u16_decimal` / `u16_decimal_string`: the canonical decimal text of a
//!   port (digits only, no leading zero, 1 to 5 bytes), which parses back
//!   to the same value.
//! - `sort_order`: a stable sort by byte-lexicographic key (`lex_lt`, the
//!   order of Rust's `str`/`String` comparison), returning the unique
//!   permutation that lists the keys in increasing order (ties by position).
//! - `render_group`: the export/dotenv block for a reservation group. It
//!   orders entries by tag, fails closed on the first entry (in tag order)
//!   that has no portable name or whose name collides, ignoring ASCII case,
//!   with an earlier one, and otherwise renders one line per entry joined
//!   by `\n`. Every line is a single shell/dotenv statement that parses back
//!   to exactly its name and port (`lemma_block`). Neither the text nor
//!   which entry an error reports depends on the order of the input
//!   (`lemma_group_order_independent`, `lemma_group_failure_order_independent`).
//! - `escape_display_field`: escapes control characters, Unicode bidi
//!   controls and the line/paragraph separators in user text shown in
//!   human-readable output, injectively (`unescape_field` inverts it) and
//!   as the identity on text without them; `tab_row` joins escaped fields
//!   with tabs, one line with one column per field.
//!
//! Production glue: `crate::output` (`formatters`, `display_field`,
//! `tab_row`).

use vstd::prelude::*;

use super::env_name;
use super::shell;
use super::text;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ---------------------------------------------------------------------------
// Sequence vocabulary: split, join, first occurrence
// ---------------------------------------------------------------------------

/// No element of `s` is `x`.
pub open spec fn free_of<T>(s: Seq<T>, x: T) -> bool {
    forall|i: int| 0 <= i < s.len() ==> s[i] != x
}

/// The fields of `s` between occurrences of `sep`: exactly `str::split` with
/// a one-character pattern (`""` gives `[""]`, `"a,"` gives `["a", ""]`).
pub open spec fn split_on<T>(s: Seq<T>, sep: T) -> Seq<Seq<T>>
    decreases s.len(),
{
    if s.len() == 0 {
        seq![Seq::<T>::empty()]
    } else {
        let init = split_on(s.drop_last(), sep);
        if s.last() == sep {
            init.push(Seq::<T>::empty())
        } else {
            init.update(init.len() - 1, init.last().push(s.last()))
        }
    }
}

/// The parts separated by `sep` (`[].join` is empty).
pub open spec fn join_with<T>(parts: Seq<Seq<T>>, sep: T) -> Seq<T>
    decreases parts.len(),
{
    if parts.len() == 0 {
        Seq::<T>::empty()
    } else if parts.len() == 1 {
        parts[0]
    } else {
        join_with(parts.drop_last(), sep).push(sep) + parts.last()
    }
}

/// The index of the first `x` in `s`, or `s.len()` when there is none.
pub open spec fn first_index<T>(s: Seq<T>, x: T) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else if s[0] == x {
        0
    } else {
        1 + first_index(s.drop_first(), x)
    }
}

/// `split_on` always returns at least one field.
pub proof fn lemma_split_nonempty<T>(s: Seq<T>, sep: T)
    ensures
        split_on(s, sep).len() >= 1,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_split_nonempty(s.drop_last(), sep);
    }
}

/// A separator-free sequence is a single field.
pub proof fn lemma_split_free<T>(s: Seq<T>, sep: T)
    requires
        free_of(s, sep),
    ensures
        split_on(s, sep) == seq![s],
    decreases s.len(),
{
    if s.len() > 0 {
        let init = s.drop_last();
        assert(free_of(init, sep));
        lemma_split_free(init, sep);
        assert(s.last() != sep);
        assert(init.push(s.last()) =~= s);
        assert(split_on(s, sep) =~= seq![s]);
    } else {
        assert(s =~= Seq::<T>::empty());
    }
}

/// Appending a separator and then a separator-free tail appends exactly one
/// field.
pub proof fn lemma_split_append<T>(a: Seq<T>, t: Seq<T>, sep: T)
    requires
        free_of(t, sep),
    ensures
        split_on(a.push(sep) + t, sep) == split_on(a, sep).push(t),
    decreases t.len(),
{
    if t.len() == 0 {
        assert(a.push(sep) + t =~= a.push(sep));
        assert((a.push(sep)).drop_last() =~= a);
        assert(t =~= Seq::<T>::empty());
    } else {
        let t0 = t.drop_last();
        assert(free_of(t0, sep));
        lemma_split_append(a, t0, sep);
        let whole = a.push(sep) + t;
        assert(whole.drop_last() =~= a.push(sep) + t0);
        assert(whole.last() == t.last());
        assert(t0.push(t.last()) =~= t);
        lemma_split_nonempty(a, sep);
        assert(split_on(whole, sep) =~= split_on(a, sep).push(t));
    }
}

/// Splitting a join of separator-free parts recovers exactly the parts.
pub proof fn lemma_split_join<T>(parts: Seq<Seq<T>>, sep: T)
    requires
        parts.len() >= 1,
        forall|i: int| 0 <= i < parts.len() ==> free_of(#[trigger] parts[i], sep),
    ensures
        split_on(join_with(parts, sep), sep) == parts,
    decreases parts.len(),
{
    if parts.len() == 1 {
        lemma_split_free(parts[0], sep);
        assert(parts =~= seq![parts[0]]);
    } else {
        let init = parts.drop_last();
        assert forall|i: int| 0 <= i < init.len() implies free_of(#[trigger] init[i], sep) by {
            assert(init[i] == parts[i]);
        }
        lemma_split_join(init, sep);
        assert(free_of(parts.last(), sep));
        lemma_split_append(join_with(init, sep), parts.last(), sep);
        assert(init.push(parts.last()) =~= parts);
    }
}

/// Every element of `s` satisfies `p`.
pub open spec fn all_satisfy<T>(s: Seq<T>, p: spec_fn(T) -> bool) -> bool {
    forall|i: int| 0 <= i < s.len() ==> p(#[trigger] s[i])
}

/// A join contains only elements of its parts and the separator: if they
/// all satisfy `p`, so does every element of the join.
pub proof fn lemma_join_all<T>(parts: Seq<Seq<T>>, sep: T, p: spec_fn(T) -> bool)
    requires
        p(sep),
        forall|i: int| 0 <= i < parts.len() ==> all_satisfy(#[trigger] parts[i], p),
    ensures
        all_satisfy(join_with(parts, sep), p),
    decreases parts.len(),
{
    if parts.len() > 1 {
        let init = parts.drop_last();
        assert forall|i: int| 0 <= i < init.len() implies all_satisfy(#[trigger] init[i], p) by {
            assert(init[i] == parts[i]);
        }
        lemma_join_all(init, sep, p);
        assert(all_satisfy(parts.last(), p));
        let head = join_with(init, sep);
        let j = head.push(sep) + parts.last();
        assert forall|i: int| 0 <= i < j.len() implies p(#[trigger] j[i]) by {
            if i < head.len() {
                assert(j[i] == head[i]);
            } else if i > head.len() {
                assert(j[i] == parts.last()[i - head.len() - 1]);
            }
        }
    } else if parts.len() == 1 {
        assert(all_satisfy(parts[0], p));
    }
}

/// The first `x` of `a + [x] + t`, for an `x`-free `a`, is at `a.len()`.
pub proof fn lemma_first_index_at<T>(a: Seq<T>, x: T, t: Seq<T>)
    requires
        free_of(a, x),
    ensures
        first_index(a.push(x) + t, x) == a.len(),
    decreases a.len(),
{
    let s = a.push(x) + t;
    if a.len() == 0 {
        assert(s[0] == x);
    } else {
        assert(s[0] == a[0]);
        assert(free_of(a.drop_first(), x));
        lemma_first_index_at(a.drop_first(), x, t);
        assert(s.drop_first() =~= a.drop_first().push(x) + t);
    }
}

// ---------------------------------------------------------------------------
// Executable byte helpers
// ---------------------------------------------------------------------------

/// Appends `s` to `out`.
pub fn push_bytes(out: &mut Vec<u8>, s: &[u8])
    ensures
        final(out)@ == old(out)@ + s@,
{
    let ghost start = out@;
    let mut i: usize = 0;
    while i < s.len()
        invariant
            0 <= i <= s.len(),
            out@ == start + s@.subrange(0, i as int),
        decreases s.len() - i,
    {
        out.push(s[i]);
        proof {
            assert(s@.subrange(0, i + 1) =~= s@.subrange(0, i as int).push(s@[i as int]));
        }
        i += 1;
    }
    proof {
        assert(s@.subrange(0, s@.len() as int) =~= s@);
    };
}

// ---------------------------------------------------------------------------
// Decimal port text
// ---------------------------------------------------------------------------

/// `decimal_text(v)` (`exclusions`) is canonical: digits only, the first
/// digit is not `0` unless `v == 0`, at most 5 digits for a `u16`, and it
/// parses back to `v` (`super::exclusions::lemma_decimal_text`).
pub proof fn lemma_decimal_canonical(v: nat)
    ensures
        super::exclusions::decimal_text(v).len() >= 1,
        super::exclusions::all_digits(super::exclusions::decimal_text(v)),
        super::exclusions::decimal_value(super::exclusions::decimal_text(v)) == v,
        v >= 1 ==> super::exclusions::decimal_text(v)[0] != b'0',
        v < 10 ==> super::exclusions::decimal_text(v).len() == 1,
        v < 100 ==> super::exclusions::decimal_text(v).len() <= 2,
        v < 1000 ==> super::exclusions::decimal_text(v).len() <= 3,
        v < 10000 ==> super::exclusions::decimal_text(v).len() <= 4,
        v < 100000 ==> super::exclusions::decimal_text(v).len() <= 5,
    decreases v,
{
    super::exclusions::lemma_decimal_text(v);
    if v >= 10 {
        lemma_decimal_canonical(v / 10);
        assert(super::exclusions::decimal_text(v)[0] == super::exclusions::decimal_text(v / 10)[0]);
    }
}

/// Appends the canonical decimal text of `v`.
pub fn push_decimal(out: &mut Vec<u8>, v: u16)
    ensures
        final(out)@ == old(out)@ + super::exclusions::decimal_text(v as nat),
    decreases v,
{
    if v >= 10 {
        push_decimal(out, v / 10);
    }
    let d = u8::try_from(v % 10).unwrap_or_default();
    out.push(b'0' + d);
    proof {
        assert(super::exclusions::decimal_text(v as nat) == if v < 10 {
            seq![(v + 48) as u8]
        } else {
            super::exclusions::decimal_text((v / 10) as nat).push((v % 10 + 48) as u8)
        });
        if v < 10 {
            assert(seq![(v + 48) as u8] =~= Seq::<u8>::empty().push((v + 48) as u8));
        }
    };
}

/// The canonical decimal text of `v` (what `v.to_string()` produces).
pub fn u16_decimal(v: u16) -> (out: Vec<u8>)
    ensures
        out@ == super::exclusions::decimal_text(v as nat),
{
    let mut out = Vec::new();
    push_decimal(&mut out, v);
    proof {
        assert(Seq::<u8>::empty() + super::exclusions::decimal_text(v as nat) =~= super::exclusions::decimal_text(
            v as nat,
        ));
    }
    out
}

/// The canonical decimal text of `v` as a `String` (what `v.to_string()`
/// produces): its UTF-8 bytes are `decimal_text(v)`.
pub fn u16_decimal_string(v: u16) -> (text: String)
    ensures
        vstd::utf8::encode_utf8(text@) == super::exclusions::decimal_text(v as nat),
        text@ == text::chars_of_bytes(super::exclusions::decimal_text(v as nat)),
{
    let digits = u16_decimal(v);
    proof {
        lemma_decimal_canonical(v as nat);
        assert forall|i: int| 0 <= i < digits@.len() implies #[trigger] digits@[i] < 0x80 by {
            assert(super::exclusions::is_digit(digits@[i]));
        }
        text::lemma_ascii_bytes_round_trip(digits@);
    }
    text::string_from_ascii(digits.as_slice())
}

// ---------------------------------------------------------------------------
// Byte-lexicographic order and the stable sort
// ---------------------------------------------------------------------------

/// Byte-lexicographic order (a proper prefix comes first). For UTF-8 text
/// this is the order of Rust's `str`/`String` comparison.
pub open spec fn lex_lt(a: Seq<u8>, b: Seq<u8>) -> bool
    decreases a.len(),
{
    if a.len() == 0 {
        b.len() > 0
    } else if b.len() == 0 {
        false
    } else if a[0] != b[0] {
        a[0] < b[0]
    } else {
        lex_lt(a.drop_first(), b.drop_first())
    }
}

/// `lex_lt` is a strict total order: irreflexive, asymmetric, total on
/// distinct sequences.
pub proof fn lemma_lex_strict_total(a: Seq<u8>, b: Seq<u8>)
    ensures
        !lex_lt(a, a),
        lex_lt(a, b) ==> !lex_lt(b, a),
        a != b ==> lex_lt(a, b) || lex_lt(b, a),
    decreases a.len(),
{
    if a.len() > 0 {
        lemma_lex_strict_total(a.drop_first(), a.drop_first());
        if b.len() > 0 {
            lemma_lex_strict_total(a.drop_first(), b.drop_first());
            if a[0] == b[0] && a != b {
                assert(a =~= seq![a[0]] + a.drop_first());
                assert(b =~= seq![b[0]] + b.drop_first());
            }
        }
    } else if b.len() == 0 {
        assert(a =~= b);
    }
}

/// `lex_lt` is transitive.
pub proof fn lemma_lex_transitive(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    requires
        lex_lt(a, b),
        lex_lt(b, c),
    ensures
        lex_lt(a, c),
    decreases a.len(),
{
    if a.len() > 0 && b.len() > 0 && c.len() > 0 && a[0] == b[0] && b[0] == c[0] {
        lemma_lex_transitive(a.drop_first(), b.drop_first(), c.drop_first());
    }
}

/// Entry `i` sorts before entry `j`: a smaller key, or the same key and an
/// earlier position (a stable sort).
pub open spec fn key_before(keys: Seq<Seq<u8>>, i: int, j: int) -> bool {
    lex_lt(keys[i], keys[j]) || (keys[i] == keys[j] && i < j)
}

/// `key_before` is a strict total order on positions.
pub proof fn lemma_key_before_order(keys: Seq<Seq<u8>>, i: int, j: int, k: int)
    ensures
        !key_before(keys, i, i),
        key_before(keys, i, j) ==> !key_before(keys, j, i),
        i != j ==> key_before(keys, i, j) || key_before(keys, j, i),
        key_before(keys, i, j) && key_before(keys, j, k) ==> key_before(keys, i, k),
{
    lemma_lex_strict_total(keys[i], keys[j]);
    lemma_lex_strict_total(keys[i], keys[i]);
    if lex_lt(keys[i], keys[j]) && lex_lt(keys[j], keys[k]) {
        lemma_lex_transitive(keys[i], keys[j], keys[k]);
    }
}

/// Position `k` appears in `order`.
pub open spec fn occurs(order: Seq<usize>, k: int) -> bool {
    exists|a: int| 0 <= a < order.len() && order[a] as int == k
}

/// `order` lists every position of `keys` exactly once, in increasing
/// `key_before` order (by key, ties by position).
pub open spec fn is_sorted_order(keys: Seq<Seq<u8>>, order: Seq<usize>) -> bool {
    &&& order.len() == keys.len()
    &&& forall|a: int| 0 <= a < order.len() ==> (#[trigger] order[a] as int) < keys.len()
    &&& forall|a: int, b: int|
        0 <= a < b < order.len() ==> key_before(
            keys,
            #[trigger] order[a] as int,
            #[trigger] order[b] as int,
        )
    &&& forall|k: int| 0 <= k < keys.len() ==> #[trigger] occurs(order, k)
}

/// In a sorted order, positions follow `key_before`.
pub proof fn lemma_sorted_positions(keys: Seq<Seq<u8>>, order: Seq<usize>, a: int, b: int)
    requires
        is_sorted_order(keys, order),
        0 <= a < order.len(),
        0 <= b < order.len(),
    ensures
        key_before(keys, order[a] as int, order[b] as int) <==> a < b,
        order[a] == order[b] <==> a == b,
{
    lemma_key_before_order(keys, order[a] as int, order[b] as int, 0);
    if a < b {
        assert(key_before(keys, order[a] as int, order[b] as int));
    } else if b < a {
        assert(key_before(keys, order[b] as int, order[a] as int));
    }
}

/// Sorted orders agree on a position, given that they agree before it.
proof fn lemma_sorted_order_agree_at(keys: Seq<Seq<u8>>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_sorted_order(keys, o1),
        is_sorted_order(keys, o2),
        0 <= a < o1.len(),
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    ensures
        o1[a] == o2[a],
{
    let x = o1[a] as int;
    let y = o2[a] as int;
    assert(occurs(o1, y));
    let b = choose|b: int| 0 <= b < o1.len() && o1[b] as int == y;
    assert(occurs(o2, x));
    let c = choose|c: int| 0 <= c < o2.len() && o2[c] as int == x;
    if x != y {
        if b < a {
            assert(o2[b] == o1[b]);
            lemma_sorted_positions(keys, o2, b, a);
        }
        if c < a {
            assert(o1[c] == o2[c]);
            lemma_sorted_positions(keys, o1, c, a);
        }
        lemma_sorted_positions(keys, o1, a, b);
        lemma_sorted_positions(keys, o2, a, c);
        lemma_key_before_order(keys, x, y, 0);
    }
}

/// Sorted orders agree on every position below `a`.
proof fn lemma_sorted_order_agree_upto(keys: Seq<Seq<u8>>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_sorted_order(keys, o1),
        is_sorted_order(keys, o2),
        0 <= a <= o1.len(),
    ensures
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    decreases a,
{
    if a > 0 {
        lemma_sorted_order_agree_upto(keys, o1, o2, a - 1);
        lemma_sorted_order_agree_at(keys, o1, o2, a - 1);
    }
}

/// The sorted order is unique: the output order is a function of the keys.
pub proof fn lemma_sorted_order_unique(keys: Seq<Seq<u8>>, o1: Seq<usize>, o2: Seq<usize>)
    requires
        is_sorted_order(keys, o1),
        is_sorted_order(keys, o2),
    ensures
        o1 == o2,
{
    lemma_sorted_order_agree_upto(keys, o1, o2, o1.len() as int);
    assert(o1 =~= o2);
}

/// The sorted order of `keys` (unique, see `lemma_sorted_order_unique`).
pub open spec fn canonical_order(keys: Seq<Seq<u8>>) -> Seq<usize> {
    choose|o: Seq<usize>| is_sorted_order(keys, o)
}

/// Any sorted order is the canonical one.
pub proof fn lemma_canonical_order(keys: Seq<Seq<u8>>, o: Seq<usize>)
    requires
        is_sorted_order(keys, o),
    ensures
        canonical_order(keys) == o,
        is_sorted_order(keys, canonical_order(keys)),
{
    lemma_sorted_order_unique(keys, o, canonical_order(keys));
}

/// Where position `i` is inserted into the sorted prefix `old`: the first
/// index (from `p` on) whose key is strictly above `kv[i]`, or `old.len()`.
spec fn insertion_point(kv: Seq<Seq<u8>>, old: Seq<usize>, i: int, p: int) -> int
    decreases old.len() - p,
{
    if p >= old.len() || lex_lt(kv[i], kv[old[p] as int]) {
        p
    } else {
        insertion_point(kv, old, i, p + 1)
    }
}

/// The insertion point is past every key at most `kv[i]` and before the
/// first key above it.
proof fn lemma_insertion_point(kv: Seq<Seq<u8>>, old: Seq<usize>, i: int, p: int)
    requires
        0 <= p <= old.len(),
        forall|x: int| 0 <= x < p ==> !lex_lt(kv[i], kv[#[trigger] old[x] as int]),
    ensures
        p <= insertion_point(kv, old, i, p) <= old.len(),
        forall|x: int|
            0 <= x < insertion_point(kv, old, i, p) ==> !lex_lt(kv[i], kv[#[trigger] old[x] as int]),
        insertion_point(kv, old, i, p) < old.len() ==> lex_lt(
            kv[i],
            kv[old[insertion_point(kv, old, i, p)] as int],
        ),
    decreases old.len() - p,
{
    if p < old.len() && !lex_lt(kv[i], kv[old[p] as int]) {
        lemma_insertion_point(kv, old, i, p + 1);
    }
}

/// A sorted order of the first `i` positions, built by insertion (the
/// ghost counterpart of `sort_order`).
proof fn lemma_sorted_prefix(kv: Seq<Seq<u8>>, i: int) -> (o: Seq<usize>)
    requires
        0 <= i <= kv.len() <= usize::MAX,
    ensures
        o.len() == i,
        forall|a: int| 0 <= a < i ==> (#[trigger] o[a] as int) < i,
        forall|a: int, b: int|
            0 <= a < b < i ==> key_before(kv, #[trigger] o[a] as int, #[trigger] o[b] as int),
        forall|k: int| 0 <= k < i ==> #[trigger] occurs(o, k),
    decreases i,
{
    if i == 0 {
        Seq::empty()
    } else {
        let j = i - 1;
        let old = lemma_sorted_prefix(kv, j);
        lemma_insertion_point(kv, old, j, 0);
        let p = insertion_point(kv, old, j, 0);
        let o = old.insert(p, j as usize);
        assert forall|a: int, b: int| 0 <= a < b < i implies key_before(
            kv,
            #[trigger] o[a] as int,
            #[trigger] o[b] as int,
        ) by {
            lemma_insert_sorted(kv, old, p, j as usize, a, b);
        }
        assert forall|a: int| 0 <= a < i implies (#[trigger] o[a] as int) < i by {
            if a < p {
                assert(o[a] == old[a]);
            } else if a > p {
                assert(o[a] == old[a - 1]);
            }
        }
        assert forall|k: int| 0 <= k < i implies #[trigger] occurs(o, k) by {
            if k == j {
                assert(o[p] as int == k);
            } else {
                assert(occurs(old, k));
                let x = choose|x: int| 0 <= x < old.len() && old[x] as int == k;
                if x < p {
                    assert(o[x] == old[x]);
                } else {
                    assert(o[x + 1] == old[x]);
                }
            }
        }
        o
    }
}

/// Every key sequence of a length a slice can have has a sorted order, so
/// `canonical_order` is that (unique) sorted order.
pub proof fn lemma_canonical_order_sorted(keys: Seq<Seq<u8>>)
    requires
        keys.len() <= usize::MAX,
    ensures
        is_sorted_order(keys, canonical_order(keys)),
{
    let o = lemma_sorted_prefix(keys, keys.len() as int);
    lemma_canonical_order(keys, o);
}

/// With pairwise distinct keys (such as the keys of a map), the sorted keys
/// strictly increase.
pub proof fn lemma_sorted_distinct_keys(keys: Seq<Seq<u8>>, order: Seq<usize>, a: int, b: int)
    requires
        is_sorted_order(keys, order),
        forall|i: int, j: int| 0 <= i < j < keys.len() ==> keys[i] != keys[j],
        0 <= a < b < order.len(),
    ensures
        lex_lt(keys[order[a] as int], keys[order[b] as int]),
{
    assert(key_before(keys, order[a] as int, order[b] as int));
    lemma_sorted_positions(keys, order, a, b);
    if keys[order[a] as int] == keys[order[b] as int] {
        if order[a] < order[b] {
            assert(keys[order[a] as int] != keys[order[b] as int]);
        } else {
            assert(keys[order[b] as int] != keys[order[a] as int]);
        }
    }
}

/// The keys of a slice of byte slices.
pub open spec fn keys_view(keys: Seq<&[u8]>) -> Seq<Seq<u8>> {
    keys.map_values(|k: &[u8]| k@)
}

/// `lex_lt(a, b)`, computed.
pub fn lex_less(a: &[u8], b: &[u8]) -> (r: bool)
    ensures
        r == lex_lt(a@, b@),
{
    let mut i: usize = 0;
    while i < a.len() && i < b.len()
        invariant
            0 <= i <= a.len(),
            i <= b.len(),
            lex_lt(a@, b@) == lex_lt(
                a@.subrange(i as int, a@.len() as int),
                b@.subrange(i as int, b@.len() as int),
            ),
        decreases a.len() - i,
    {
        let ghost sa = a@.subrange(i as int, a@.len() as int);
        let ghost sb = b@.subrange(i as int, b@.len() as int);
        proof {
            assert(sa[0] == a@[i as int]);
            assert(sb[0] == b@[i as int]);
        }
        if a[i] != b[i] {
            return a[i] < b[i];
        }
        proof {
            assert(sa.drop_first() =~= a@.subrange(i + 1, a@.len() as int));
            assert(sb.drop_first() =~= b@.subrange(i + 1, b@.len() as int));
        }
        i += 1;
    }
    i == a.len() && i < b.len()
}

/// Inserting the newest position `i` before the first strictly larger key
/// keeps the order sorted.
proof fn lemma_insert_sorted(kv: Seq<Seq<u8>>, old: Seq<usize>, p: int, i: usize, a: int, b: int)
    requires
        0 <= p <= old.len() == i < kv.len(),
        forall|x: int| 0 <= x < i ==> (#[trigger] old[x] as int) < i,
        forall|x: int, y: int|
            0 <= x < y < i ==> key_before(kv, #[trigger] old[x] as int, #[trigger] old[y] as int),
        forall|x: int| 0 <= x < p ==> !lex_lt(kv[i as int], kv[#[trigger] old[x] as int]),
        p < i ==> lex_lt(kv[i as int], kv[old[p] as int]),
        0 <= a < b < i + 1,
    ensures
        key_before(kv, old.insert(p, i)[a] as int, old.insert(p, i)[b] as int),
{
    let o = old.insert(p, i);
    if a < p && b < p {
        assert(o[a] == old[a] && o[b] == old[b]);
    } else if a > p && b > p {
        assert(o[a] == old[a - 1] && o[b] == old[b - 1]);
    } else if a < p && b > p {
        assert(o[a] == old[a] && o[b] == old[b - 1]);
    } else if b == p {
        // `old[a]` has a key at most `kv[i]`, and an earlier position.
        assert(o[a] == old[a] && o[b] == i);
        lemma_lex_strict_total(kv[i as int], kv[old[a] as int]);
    } else {
        // a == p: every later key is above `kv[i]`.
        assert(o[a] == i && o[b] == old[b - 1]);
        let q = old[p] as int;
        let r = old[b - 1] as int;
        if b - 1 > p {
            assert(key_before(kv, q, r));
            if lex_lt(kv[q], kv[r]) {
                lemma_lex_transitive(kv[i as int], kv[q], kv[r]);
            }
        }
    }
}

/// The stable sort of positions by key: the unique sorted order.
pub fn sort_order(keys: &[&[u8]]) -> (order: Vec<usize>)
    ensures
        is_sorted_order(keys_view(keys@), order@),
        order@ == canonical_order(keys_view(keys@)),
{
    let ghost kv = keys_view(keys@);
    let n = keys.len();
    let mut order: Vec<usize> = Vec::new();
    let ghost mut pos: Seq<int> = Seq::empty();
    let mut i: usize = 0;
    while i < n
        invariant
            n == keys@.len() == kv.len(),
            kv == keys_view(keys@),
            0 <= i <= n,
            order@.len() == i,
            pos.len() == i,
            forall|a: int| 0 <= a < i ==> (#[trigger] order@[a] as int) < i,
            forall|a: int, b: int|
                0 <= a < b < i ==> key_before(
                    kv,
                    #[trigger] order@[a] as int,
                    #[trigger] order@[b] as int,
                ),
            forall|k: int| 0 <= k < i ==> 0 <= #[trigger] pos[k] < i && order@[pos[k]] as int == k,
        decreases n - i,
    {
        let mut p: usize = 0;
        while p < i && !lex_less(keys[i], keys[order[p]])
            invariant
                n == keys@.len() == kv.len(),
                kv == keys_view(keys@),
                0 <= i < n,
                0 <= p <= i,
                order@.len() == i,
                forall|a: int| 0 <= a < i ==> (#[trigger] order@[a] as int) < i,
                forall|a: int| 0 <= a < p ==> !lex_lt(kv[i as int], kv[#[trigger] order@[a] as int]),
            decreases i - p,
        {
            p += 1;
        }
        let ghost old_order = order@;
        proof {
            assert(kv[i as int] == keys@[i as int]@);
            if p < i {
                assert(kv[old_order[p as int] as int] == keys@[old_order[p as int] as int]@);
            }
        }
        order.insert(p, i);
        proof {
            let o = order@;
            assert(o == old_order.insert(p as int, i));
            assert forall|a: int, b: int| 0 <= a < b < i + 1 implies key_before(
                kv,
                #[trigger] o[a] as int,
                #[trigger] o[b] as int,
            ) by {
                lemma_insert_sorted(kv, old_order, p as int, i, a, b);
            }
            let new_pos = Seq::new(
                (i + 1) as nat,
                |k: int|
                    if k == i {
                        p as int
                    } else if pos[k] >= p {
                        pos[k] + 1
                    } else {
                        pos[k]
                    },
            );
            assert forall|k: int| 0 <= k < i + 1 implies 0 <= #[trigger] new_pos[k] < i + 1
                && o[new_pos[k]] as int == k by {
                if k < i {
                    assert(old_order[pos[k]] as int == k);
                }
            }
            assert forall|a: int| 0 <= a < i + 1 implies (#[trigger] o[a] as int) < i + 1 by {
                if a < p {
                    assert(o[a] == old_order[a]);
                } else if a > p {
                    assert(o[a] == old_order[a - 1]);
                }
            }
            pos = new_pos;
        }
        i += 1;
    }
    proof {
        assert forall|k: int| 0 <= k < n implies #[trigger] occurs(order@, k) by {
            assert(order@[pos[k]] as int == k);
        }
        lemma_canonical_order(kv, order@);
    }
    order
}

// ---------------------------------------------------------------------------
// Assignment lines and blocks
// ---------------------------------------------------------------------------

/// How a group is rendered: shell export statements or dotenv lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignmentStyle {
    /// `export N=P` / `set -x N P` / `$env:N="P"`.
    Export(shell::ShellKind),
    /// `N=P`.
    Dotenv,
}

/// The line assigning `port` to `name`.
pub open spec fn assignment_line(style: AssignmentStyle, name: Seq<u8>, port: u16) -> Seq<u8> {
    let value = super::exclusions::decimal_text(port as nat);
    match style {
        AssignmentStyle::Export(sh) => shell::export_line_spec(sh, name, value),
        AssignmentStyle::Dotenv => name + seq![b'='] + value,
    }
}

/// The lines of a block, in order.
pub open spec fn block_lines(style: AssignmentStyle, assigns: Seq<(Seq<u8>, u16)>) -> Seq<
    Seq<u8>,
> {
    assigns.map_values(|x: (Seq<u8>, u16)| assignment_line(style, x.0, x.1))
}

/// The block: the lines joined by `\n`, with no trailing newline.
pub open spec fn block_spec(style: AssignmentStyle, assigns: Seq<(Seq<u8>, u16)>) -> Seq<u8> {
    join_with(block_lines(style, assigns), b'\n')
}

/// Model dotenv reader: a line of word bytes (so no quote, `$`, `#`, space
/// or control byte) whose key, before the first `=`, is portable.
pub open spec fn dotenv_assignment(line: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    let k = first_index(line, b'=');
    if (forall|i: int| 0 <= i < line.len() ==> shell::is_word_byte(#[trigger] line[i])) && k
        < line.len() && env_name::is_portable_env_name(line.subrange(0, k)) {
        Some((line.subrange(0, k), line.subrange(k + 1, line.len() as int)))
    } else {
        None
    }
}

/// The assignment a line performs, under the model of its consumer.
pub open spec fn parse_assignment_line(style: AssignmentStyle, line: Seq<u8>) -> Option<
    (Seq<u8>, Seq<u8>),
> {
    match style {
        AssignmentStyle::Export(sh) => shell::parse_export_line(sh, line),
        AssignmentStyle::Dotenv => dotenv_assignment(line),
    }
}

/// An ASCII byte other than `\n` and `\r`.
pub open spec fn is_line_byte(b: u8) -> bool {
    b < 0x80 && b != b'\n' && b != b'\r'
}

/// An ASCII byte other than `\r`.
pub open spec fn is_block_byte(b: u8) -> bool {
    b < 0x80 && b != b'\r'
}

/// A port's decimal text is an inert value.
pub proof fn lemma_decimal_inert(port: u16)
    ensures
        shell::is_inert_value(super::exclusions::decimal_text(port as nat)),
{
    let t = super::exclusions::decimal_text(port as nat);
    lemma_decimal_canonical(port as nat);
    assert forall|i: int| 0 <= i < t.len() implies #[trigger] shell::is_value_byte(t[i]) by {
        assert(super::exclusions::is_digit(t[i]));
    }
    assert(super::exclusions::is_digit(t[0]));
}

/// Every line is a single ASCII statement without `\n` or `\r`, and its
/// consumer reads it as the assignment of exactly the port's decimal text
/// to the name.
pub proof fn lemma_assignment_line(style: AssignmentStyle, name: Seq<u8>, port: u16)
    requires
        env_name::is_portable_env_name(name),
    ensures
        parse_assignment_line(style, assignment_line(style, name, port)) == Some(
            (name, super::exclusions::decimal_text(port as nat)),
        ),
        forall|i: int|
            0 <= i < assignment_line(style, name, port).len() ==> is_line_byte(
                #[trigger] assignment_line(style, name, port)[i],
            ),
        assignment_line(style, name, port).len() > 0,
{
    let value = super::exclusions::decimal_text(port as nat);
    let line = assignment_line(style, name, port);
    lemma_decimal_inert(port);
    match style {
        AssignmentStyle::Export(sh) => {
            shell::lemma_export_round_trip(sh, name, value);
            shell::lemma_export_line_ascii(sh, name, value);
            assert forall|i: int| 0 <= i < line.len() implies is_line_byte(#[trigger] line[i]) by {
                assert(line[i] < 0x80);
            }
        },
        AssignmentStyle::Dotenv => {
            env_name::lemma_portable_is_shell_inert(name);
            assert forall|i: int| 0 <= i < line.len() implies shell::is_word_byte(
                #[trigger] line[i],
            ) by {
                if i < name.len() {
                    assert(line[i] == name[i]);
                    shell::lemma_word_bytes_are_inert(name[i]);
                } else if i > name.len() {
                    assert(line[i] == value[i - name.len() - 1]);
                    assert(shell::is_value_byte(value[i - name.len() - 1]));
                }
            }
            assert forall|i: int| 0 <= i < line.len() implies is_line_byte(#[trigger] line[i]) by {
                shell::lemma_word_bytes_are_inert(line[i]);
            }
            assert forall|i: int| 0 <= i < name.len() implies name[i] != b'=' by {
                assert(env_name::is_env_tail(name[i]));
            }
            lemma_first_index_at(name, b'=', value);
            assert(name.push(b'=') + value =~= line);
            assert(line.subrange(0, name.len() as int) =~= name);
            assert(line.subrange(name.len() as int + 1, line.len() as int) =~= value);
        },
    }
}

/// The block's lines are exactly the assignment lines: splitting on `\n`
/// recovers them in order (no line is merged or split), each is read as
/// exactly its assignment, and the block is ASCII without `\r`. An empty
/// group renders as the empty string.
pub proof fn lemma_block(style: AssignmentStyle, assigns: Seq<(Seq<u8>, u16)>)
    requires
        forall|a: int|
            0 <= a < assigns.len() ==> env_name::is_portable_env_name(#[trigger] assigns[a].0),
    ensures
        assigns.len() == 0 ==> block_spec(style, assigns).len() == 0,
        assigns.len() > 0 ==> split_on(block_spec(style, assigns), b'\n') == block_lines(
            style,
            assigns,
        ),
        forall|a: int|
            0 <= a < assigns.len() ==> parse_assignment_line(
                style,
                #[trigger] block_lines(style, assigns)[a],
            ) == Some((assigns[a].0, super::exclusions::decimal_text(assigns[a].1 as nat))),
        all_satisfy(block_spec(style, assigns), |b: u8| is_block_byte(b)),
{
    let lines = block_lines(style, assigns);
    let ok = |b: u8| is_block_byte(b);
    assert forall|a: int| 0 <= a < lines.len() implies free_of(#[trigger] lines[a], b'\n')
        && all_satisfy(lines[a], ok) && parse_assignment_line(style, lines[a]) == Some(
        (assigns[a].0, super::exclusions::decimal_text(assigns[a].1 as nat)),
    ) by {
        lemma_assignment_line(style, assigns[a].0, assigns[a].1);
        assert forall|i: int| 0 <= i < lines[a].len() implies lines[a][i] != b'\n' && ok(
            #[trigger] lines[a][i],
        ) by {
            assert(is_line_byte(lines[a][i]));
        }
    }
    if assigns.len() > 0 {
        lemma_split_join(lines, b'\n');
    }
    lemma_join_all(lines, b'\n', ok);
}

// ---------------------------------------------------------------------------
// Group rendering
// ---------------------------------------------------------------------------

/// One service of a group: its tag (the sort key), its resolved
/// environment-variable name (`None` when resolution failed), and its port.
pub struct GroupEntry {
    /// The service tag's UTF-8 bytes.
    pub tag: Vec<u8>,
    /// The resolved name's bytes, or `None` when it could not be resolved.
    pub name: Option<Vec<u8>>,
    /// The reserved port.
    pub port: u16,
}

/// Why a group cannot be rendered. Positions index the input slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupError {
    /// Entry `source` has no portable name.
    Unresolved {
        /// The failing entry.
        source: usize,
    },
    /// Entry `source`'s name equals, ignoring ASCII case, the name of entry
    /// `earlier`, which sorts before it.
    Collision {
        /// The failing entry.
        source: usize,
        /// The entry it collides with.
        earlier: usize,
    },
}

/// The tags of a group, as sort keys.
pub open spec fn tag_keys(entries: Seq<GroupEntry>) -> Seq<Seq<u8>> {
    entries.map_values(|e: GroupEntry| e.tag@)
}

/// An entry's name when it is present and portable.
pub open spec fn entry_name(e: GroupEntry) -> Option<Seq<u8>> {
    match e.name {
        Some(n) => if env_name::is_portable_env_name(n@) {
            Some(n@)
        } else {
            None
        },
        None => None,
    }
}

/// The name of an entry known to resolve.
pub open spec fn name_of(e: GroupEntry) -> Seq<u8> {
    match entry_name(e) {
        Some(n) => n,
        None => Seq::empty(),
    }
}

/// Every entry has a portable name, and no two entries' names are equal
/// ignoring ASCII case (`PowerShell` and Windows read environment names
/// case-insensitively, so such names would overwrite each other).
pub open spec fn group_resolves(entries: Seq<GroupEntry>) -> bool {
    &&& forall|i: int| 0 <= i < entries.len() ==> #[trigger] entry_name(entries[i]) is Some
    &&& forall|i: int, j: int|
        0 <= i < entries.len() && 0 <= j < entries.len() && i != j
            ==> !text::eq_ignore_ascii_case(
            name_of(#[trigger] entries[i]),
            name_of(#[trigger] entries[j]),
        )
}

/// Every entry that sorts before entry `k` resolves, and their names are
/// pairwise distinct ignoring ASCII case: `k` is the first failure.
pub open spec fn fine_before(entries: Seq<GroupEntry>, k: int) -> bool {
    let keys = tag_keys(entries);
    &&& forall|j: int|
        0 <= j < entries.len() && key_before(keys, j, k) ==> #[trigger] entry_name(
            entries[j],
        ) is Some
    &&& forall|i: int, j: int|
        0 <= i < entries.len() && 0 <= j < entries.len() && i != j && key_before(keys, i, k)
            && key_before(keys, j, k) ==> !text::eq_ignore_ascii_case(
            name_of(#[trigger] entries[i]),
            name_of(#[trigger] entries[j]),
        )
}

/// The group's assignments (name, port) in tag order.
pub open spec fn sorted_assignments(entries: Seq<GroupEntry>) -> Seq<(Seq<u8>, u16)> {
    let order = canonical_order(tag_keys(entries));
    Seq::new(
        entries.len(),
        |a: int| (name_of(entries[order[a] as int]), entries[order[a] as int].port),
    )
}

/// When position `a` of a sorted order is the first failure (everything
/// before it is fine), entry `order[a]` satisfies `fine_before`.
proof fn lemma_first_failure(es: Seq<GroupEntry>, order: Seq<usize>, a: int)
    requires
        is_sorted_order(tag_keys(es), order),
        0 <= a < order.len(),
        forall|x: int| 0 <= x < a ==> #[trigger] entry_name(es[order[x] as int]) is Some,
        forall|x: int, y: int|
            0 <= x < y < a ==> !text::eq_ignore_ascii_case(
                name_of(#[trigger] es[order[x] as int]),
                name_of(#[trigger] es[order[y] as int]),
            ),
    ensures
        fine_before(es, order[a] as int),
{
    let keys = tag_keys(es);
    let k = order[a] as int;
    assert forall|j: int| 0 <= j < es.len() && key_before(keys, j, k) implies #[trigger] entry_name(
        es[j],
    ) is Some by {
        assert(occurs(order, j));
        let x = choose|x: int| 0 <= x < order.len() && order[x] as int == j;
        lemma_sorted_positions(keys, order, x, a);
    }
    assert forall|i: int, j: int|
        0 <= i < es.len() && 0 <= j < es.len() && i != j && key_before(keys, i, k) && key_before(
            keys,
            j,
            k,
        ) implies !text::eq_ignore_ascii_case(
        name_of(#[trigger] es[i]),
        name_of(#[trigger] es[j]),
    ) by {
        assert(occurs(order, i));
        assert(occurs(order, j));
        let x = choose|x: int| 0 <= x < order.len() && order[x] as int == i;
        let y = choose|y: int| 0 <= y < order.len() && order[y] as int == j;
        lemma_sorted_positions(keys, order, x, a);
        lemma_sorted_positions(keys, order, y, a);
        lemma_sorted_positions(keys, order, x, y);
        if y < x {
            text::lemma_eq_ignore_ascii_case_equivalence(
                name_of(es[i]),
                name_of(es[j]),
                name_of(es[j]),
            );
        }
    }
}

/// When every position of a sorted order is fine, the whole group resolves.
proof fn lemma_all_resolve(es: Seq<GroupEntry>, order: Seq<usize>)
    requires
        is_sorted_order(tag_keys(es), order),
        forall|x: int| 0 <= x < order.len() ==> #[trigger] entry_name(es[order[x] as int]) is Some,
        forall|x: int, y: int|
            0 <= x < y < order.len() ==> !text::eq_ignore_ascii_case(
                name_of(#[trigger] es[order[x] as int]),
                name_of(#[trigger] es[order[y] as int]),
            ),
    ensures
        group_resolves(es),
{
    let keys = tag_keys(es);
    assert forall|i: int| 0 <= i < es.len() implies #[trigger] entry_name(es[i]) is Some by {
        assert(occurs(order, i));
    }
    assert forall|i: int, j: int|
        0 <= i < es.len() && 0 <= j < es.len() && i != j implies !text::eq_ignore_ascii_case(
        name_of(#[trigger] es[i]),
        name_of(#[trigger] es[j]),
    ) by {
        assert(occurs(order, i));
        assert(occurs(order, j));
        let x = choose|x: int| 0 <= x < order.len() && order[x] as int == i;
        let y = choose|y: int| 0 <= y < order.len() && order[y] as int == j;
        lemma_sorted_positions(keys, order, x, y);
        if y < x {
            text::lemma_eq_ignore_ascii_case_equivalence(
                name_of(es[i]),
                name_of(es[j]),
                name_of(es[j]),
            );
        }
    }
}

/// The byte slices of the entries' tags.
fn tag_slices(entries: &[GroupEntry]) -> (tags: Vec<&[u8]>)
    ensures
        keys_view(tags@) == tag_keys(entries@),
{
    let n = entries.len();
    let mut tags: Vec<&[u8]> = Vec::new();
    let mut t: usize = 0;
    while t < n
        invariant
            n == entries@.len(),
            0 <= t <= n,
            tags@.len() == t,
            forall|j: int| 0 <= j < t ==> (#[trigger] tags@[j])@ == entries@[j].tag@,
        decreases n - t,
    {
        tags.push(entries[t].tag.as_slice());
        t += 1;
    }
    assert(keys_view(tags@) =~= tag_keys(entries@));
    tags
}

/// Finds the first failure in tag order, if any (see `render_group`).
fn first_group_failure(entries: &[GroupEntry], order: &[usize]) -> (r: Option<GroupError>)
    requires
        is_sorted_order(tag_keys(entries@), order@),
    ensures
        r is None <==> group_resolves(entries@),
        r is None ==> forall|x: int|
            0 <= x < order@.len() ==> #[trigger] entry_name(entries@[order@[x] as int]) is Some,
        r matches Some(GroupError::Unresolved { source }) ==> source < entries@.len()
            && entry_name(entries@[source as int]) is None && fine_before(entries@, source as int),
        r matches Some(GroupError::Collision { source, earlier }) ==> source < entries@.len()
            && earlier < entries@.len() && key_before(
            tag_keys(entries@),
            earlier as int,
            source as int,
        ) && entry_name(entries@[source as int]) is Some && entry_name(
            entries@[earlier as int],
        ) is Some && text::eq_ignore_ascii_case(
            name_of(entries@[earlier as int]),
            name_of(entries@[source as int]),
        ) && fine_before(entries@, source as int),
{
    let count = entries.len();
    let ghost es = entries@;
    let ghost keys = tag_keys(es);
    let mut pos: usize = 0;
    while pos < count
        invariant
            count == es.len() == order@.len(),
            es == entries@,
            keys == tag_keys(es),
            is_sorted_order(keys, order@),
            0 <= pos <= count,
            forall|x: int| 0 <= x < pos ==> #[trigger] entry_name(es[order@[x] as int]) is Some,
            forall|x: int, y: int|
                0 <= x < y < pos ==> !text::eq_ignore_ascii_case(
                    name_of(#[trigger] es[order@[x] as int]),
                    name_of(#[trigger] es[order@[y] as int]),
                ),
        decreases count - pos,
    {
        let source = order[pos];
        let Some(name) = &entries[source].name else {
            proof {
                lemma_first_failure(es, order@, pos as int);
            }
            return Some(GroupError::Unresolved { source });
        };
        if env_name::check_env_name_bytes(name.as_slice()).is_err() {
            proof {
                lemma_first_failure(es, order@, pos as int);
            }
            return Some(GroupError::Unresolved { source });
        }
        let mut prior: usize = 0;
        while prior < pos
            invariant
                count == es.len() == order@.len(),
                es == entries@,
                keys == tag_keys(es),
                is_sorted_order(keys, order@),
                0 <= prior <= pos < count,
                source == order@[pos as int],
                entry_name(es[source as int]) == Some(name@),
                forall|x: int| 0 <= x < pos ==> #[trigger] entry_name(es[order@[x] as int]) is Some,
                forall|x: int, y: int|
                    0 <= x < y < pos ==> !text::eq_ignore_ascii_case(
                        name_of(#[trigger] es[order@[x] as int]),
                        name_of(#[trigger] es[order@[y] as int]),
                    ),
                forall|y: int|
                    0 <= y < prior ==> !text::eq_ignore_ascii_case(
                        name_of(#[trigger] es[order@[y] as int]),
                        name@,
                    ),
            decreases pos - prior,
        {
            let earlier = order[prior];
            proof {
                assert(entry_name(es[order@[prior as int] as int]) is Some);
            }
            if let Some(other) = &entries[earlier].name {
                if text::eq_ignore_ascii_case_bytes(other.as_slice(), name.as_slice()) {
                    proof {
                        lemma_sorted_positions(keys, order@, prior as int, pos as int);
                        lemma_first_failure(es, order@, pos as int);
                    }
                    return Some(GroupError::Collision { source, earlier });
                }
            }
            prior += 1;
        }
        proof {
            assert forall|x: int, y: int| 0 <= x < y < pos + 1 implies !text::eq_ignore_ascii_case(
                name_of(#[trigger] es[order@[x] as int]),
                name_of(#[trigger] es[order@[y] as int]),
            ) by {
                if y == pos {
                    assert(name_of(es[order@[y] as int]) == name@);
                }
            }
        }
        pos += 1;
    }
    proof {
        lemma_all_resolve(es, order@);
    }
    None
}

/// Renders a group's export or dotenv block, failing closed.
///
/// - `Ok(text)` exactly when every entry has a portable name and no two
///   names are equal ignoring ASCII case. `text` is then the block of the
///   entries' assignments in tag order (`block_spec`): its lines (split on
///   `\n`) are exactly the assignment lines, each read by its consumer as
///   exactly its name and port (`lemma_block`).
/// - Otherwise the error names the first entry in tag order that fails: no
///   portable name (`Unresolved`), or a name colliding with an earlier
///   entry's (`Collision`); an entry's own resolution failure is reported
///   before any collision of it. Nothing is rendered on error. The entry an
///   error names is the group's unique first failure (`first_failure_at`,
///   `lemma_first_failure_unique`).
pub fn render_group(style: AssignmentStyle, entries: &[GroupEntry]) -> (r: Result<
    String,
    GroupError,
>)
    ensures
        is_sorted_order(tag_keys(entries@), canonical_order(tag_keys(entries@))),
        r is Ok <==> group_resolves(entries@),
        r matches Ok(text) ==> vstd::utf8::encode_utf8(text@) == block_spec(
            style,
            sorted_assignments(entries@),
        ),
        r matches Ok(text) ==> entries@.len() > 0 ==> split_on(
            vstd::utf8::encode_utf8(text@),
            b'\n',
        ) == block_lines(style, sorted_assignments(entries@)),
        r matches Ok(text) ==> entries@.len() == 0 ==> text@.len() == 0,
        r matches Err(GroupError::Unresolved { source }) ==> source < entries@.len() && entry_name(
            entries@[source as int],
        ) is None && fine_before(entries@, source as int),
        r matches Err(GroupError::Collision { source, earlier }) ==> source < entries@.len()
            && earlier < entries@.len() && key_before(
            tag_keys(entries@),
            earlier as int,
            source as int,
        ) && entry_name(entries@[source as int]) is Some && entry_name(
            entries@[earlier as int],
        ) is Some && text::eq_ignore_ascii_case(
            name_of(entries@[earlier as int]),
            name_of(entries@[source as int]),
        ) && fine_before(entries@, source as int),
        r matches Err(e) ==> first_failure_at(entries@, error_source(e)),
{
    let n = entries.len();
    let ghost es = entries@;
    let ghost keys = tag_keys(es);
    let tags = tag_slices(entries);
    let order = sort_order(tags.as_slice());
    proof {
        lemma_canonical_order(keys, order@);
    }
    if let Some(error) = first_group_failure(entries, order.as_slice()) {
        proof {
            if let GroupError::Collision { source, earlier } = error {
                assert(entry_name(es[earlier as int]) is Some);
            }
        }
        return Err(error);
    }
    let ghost assigns = sorted_assignments(es);
    let ghost lines = block_lines(style, assigns);
    let mut out: Vec<u8> = Vec::new();
    let mut c: usize = 0;
    while c < n
        invariant
            n == es.len() == order@.len() == assigns.len() == lines.len(),
            es == entries@,
            keys == tag_keys(es),
            is_sorted_order(keys, order@),
            order@ == canonical_order(keys),
            assigns == sorted_assignments(es),
            lines == block_lines(style, assigns),
            forall|x: int| 0 <= x < n ==> #[trigger] entry_name(es[order@[x] as int]) is Some,
            0 <= c <= n,
            out@ == join_with(lines.subrange(0, c as int), b'\n'),
        decreases n - c,
    {
        let k = order[c];
        proof {
            assert(entry_name(es[order@[c as int] as int]) is Some);
            assert(assigns[c as int] == (name_of(es[k as int]), es[k as int].port));
        }
        if let Some(name) = &entries[k].name {
            if c > 0 {
                out.push(b'\n');
            }
            let ghost before = out@;
            match style {
                AssignmentStyle::Export(sh) => {
                    let digits = u16_decimal(entries[k].port);
                    shell::push_export_line(&mut out, sh, name.as_slice(), digits.as_slice());
                },
                AssignmentStyle::Dotenv => {
                    push_bytes(&mut out, name.as_slice());
                    out.push(b'=');
                    push_decimal(&mut out, entries[k].port);
                },
            }
            proof {
                assert(out@ =~= before + lines[c as int]);
                let prefix = lines.subrange(0, c + 1);
                if c == 0 {
                    assert(prefix =~= seq![lines[0]]);
                } else {
                    assert(prefix.drop_last() =~= lines.subrange(0, c as int));
                }
            };
        }
        c += 1;
    }
    proof {
        assert(lines.subrange(0, n as int) =~= lines);
        assert forall|x: int| 0 <= x < assigns.len() implies env_name::is_portable_env_name(
            #[trigger] assigns[x].0,
        ) by {
            assert(entry_name(es[order@[x] as int]) is Some);
        }
        lemma_block(style, assigns);
        assert forall|i: int| 0 <= i < out@.len() implies #[trigger] out@[i] < 0x80 by {
            assert((|b: u8| is_block_byte(b))(out@[i]));
        }
        text::lemma_ascii_bytes_round_trip(out@);
    }
    let text_out = text::string_from_ascii(out.as_slice());
    Ok(text_out)
}

// ---------------------------------------------------------------------------
// Escaping user text for human-readable output
// ---------------------------------------------------------------------------

/// A control character: C0 (`U+0000..=U+001F`, including `\t`, `\n`, `\r`
/// and ESC), DEL (`U+007F`), or C1 (`U+0080..=U+009F`, including CSI and
/// NEL).
pub open spec fn is_control_char(c: char) -> bool {
    (c as u32) < 0x20 || (0x7F <= (c as u32) && (c as u32) <= 0x9F)
}

/// A layout control: a Unicode `Bidi_Control` character (ALM `U+061C`,
/// LRM and RLM `U+200E`/`U+200F`, the embeddings and overrides
/// `U+202A..=U+202E`, the isolates `U+2066..=U+2069`), or the line or
/// paragraph separator (`U+2028`, `U+2029`, the only characters of
/// categories `Zl` and `Zp`). Bidi controls reorder the rest of a line in
/// bidi-aware terminals; the separators end a line for readers such as
/// Python's `str.splitlines` and JavaScript.
pub open spec fn is_layout_control(c: char) -> bool {
    let v = c as u32;
    v == 0x061C || v == 0x200E || v == 0x200F || (0x2028 <= v && v <= 0x202E) || (0x2066 <= v
        && v <= 0x2069)
}

/// A char that display fields escape: a control character or a layout
/// control.
pub open spec fn is_unsafe_char(c: char) -> bool {
    is_control_char(c) || is_layout_control(c)
}

/// A field that is printed as is: no control character or layout control,
/// and not starting with `"` (so it can never be mistaken for an escaped
/// field).
pub open spec fn is_clean_field(s: Seq<char>) -> bool {
    &&& forall|i: int| 0 <= i < s.len() ==> !is_unsafe_char(#[trigger] s[i])
    &&& !(s.len() > 0 && s[0] == '"')
}

/// The ASCII byte of hex digit `d` (`0-9`, `a-f`).
pub open spec fn hex_byte(d: u8) -> u8 {
    if d < 10 {
        (d + 48) as u8
    } else {
        (d + 87) as u8
    }
}

/// The hex digit `d` as a char.
pub open spec fn hex_char(d: u8) -> char {
    hex_byte(d) as char
}

/// The value of a hex digit char (meaningful for `0-9` and `a-f`).
pub open spec fn hex_value(c: char) -> int {
    if '0' <= c && c <= '9' {
        c as int - 48
    } else {
        c as int - 87
    }
}

/// The four hex digits of a layout control's code point (all are below
/// `U+10000`), most significant first.
pub open spec fn layout_digits(c: char) -> Seq<char> {
    let v = c as u32;
    seq![
        hex_char(((v / 4096) % 16) as u8),
        hex_char(((v / 256) % 16) as u8),
        hex_char(((v / 16) % 16) as u8),
        hex_char((v % 16) as u8),
    ]
}

/// How one char is written inside an escaped field: `\"`, `\\`, `\n`,
/// `\r`, `\t`, `\xHH` for any other control character, `\u{HHHH}` for a
/// layout control, otherwise itself.
pub open spec fn escape_char(c: char) -> Seq<char> {
    if c == '"' {
        seq!['\\', '"']
    } else if c == '\\' {
        seq!['\\', '\\']
    } else if c == '\n' {
        seq!['\\', 'n']
    } else if c == '\r' {
        seq!['\\', 'r']
    } else if c == '\t' {
        seq!['\\', 't']
    } else if is_control_char(c) {
        seq!['\\', 'x', hex_char(((c as u32) / 16) as u8), hex_char(((c as u32) % 16) as u8)]
    } else if is_layout_control(c) {
        seq!['\\', 'u', '{'] + layout_digits(c) + seq!['}']
    } else {
        seq![c]
    }
}

/// The escaped body of a field.
pub open spec fn escape_body(s: Seq<char>) -> Seq<char>
    decreases s.len(),
{
    if s.len() == 0 {
        Seq::empty()
    } else {
        escape_body(s.drop_last()) + escape_char(s.last())
    }
}

/// How a field is displayed: as is when clean, otherwise quoted and
/// escaped.
pub open spec fn display_field_spec(s: Seq<char>) -> Seq<char> {
    if is_clean_field(s) {
        s
    } else {
        seq!['"'] + escape_body(s) + seq!['"']
    }
}

/// Decodes an escaped body (the inverse of `escape_body`).
pub open spec fn unescape_body(t: Seq<char>) -> Seq<char>
    decreases t.len(),
{
    if t.len() == 0 {
        Seq::empty()
    } else if t[0] == '\\' && t.len() >= 2 {
        if t[1] == 'n' {
            seq!['\n'] + unescape_body(t.subrange(2, t.len() as int))
        } else if t[1] == 'r' {
            seq!['\r'] + unescape_body(t.subrange(2, t.len() as int))
        } else if t[1] == 't' {
            seq!['\t'] + unescape_body(t.subrange(2, t.len() as int))
        } else if t[1] == 'x' && t.len() >= 4 {
            seq![(hex_value(t[2]) * 16 + hex_value(t[3])) as char] + unescape_body(
                t.subrange(4, t.len() as int),
            )
        } else if t[1] == 'u' && t.len() >= 8 {
            seq![
                (hex_value(t[3]) * 4096 + hex_value(t[4]) * 256 + hex_value(t[5]) * 16 + hex_value(
                    t[6],
                )) as char,
            ] + unescape_body(t.subrange(8, t.len() as int))
        } else {
            seq![t[1]] + unescape_body(t.subrange(2, t.len() as int))
        }
    } else {
        seq![t[0]] + unescape_body(t.drop_first())
    }
}

/// Decodes a displayed field (the inverse of `display_field_spec`).
pub open spec fn unescape_field(t: Seq<char>) -> Seq<char> {
    if t.len() >= 2 && t[0] == '"' && t.last() == '"' {
        unescape_body(t.subrange(1, t.len() - 1))
    } else {
        t
    }
}

/// Hex digits decode to their value and are printable ASCII.
proof fn lemma_hex_char(d: u8)
    requires
        d < 16,
    ensures
        hex_value(hex_char(d)) == d,
        hex_char(d) != '\\' && !is_unsafe_char(hex_char(d)),
        (hex_char(d) as u32) < 0x80,
{
    assert((hex_byte(d) as char) as u32 == hex_byte(d) as u32);
}

/// Every escape sequence decodes to its char, whatever follows it.
proof fn lemma_unescape_unit(c: char, rest: Seq<char>)
    ensures
        unescape_body(escape_char(c) + rest) == seq![c] + unescape_body(rest),
{
    broadcast use vstd::utf8::char_u32_cast;

    let t = escape_char(c) + rest;
    if c == '"' || c == '\\' || c == '\n' || c == '\r' || c == '\t' {
        assert(t.subrange(2, t.len() as int) =~= rest);
    } else if is_control_char(c) {
        let v = c as u32;
        let hi = (v / 16) as u8;
        let lo = (v % 16) as u8;
        lemma_hex_char(hi);
        lemma_hex_char(lo);
        assert(t[2] == hex_char(hi) && t[3] == hex_char(lo));
        assert(t.subrange(4, t.len() as int) =~= rest);
        assert(hi as int * 16 + lo as int == v as int);
        assert((v as int) as char == c);
    } else if is_layout_control(c) {
        let v = c as u32;
        let d3 = ((v / 4096) % 16) as u8;
        let d2 = ((v / 256) % 16) as u8;
        let d1 = ((v / 16) % 16) as u8;
        let d0 = (v % 16) as u8;
        lemma_hex_char(d3);
        lemma_hex_char(d2);
        lemma_hex_char(d1);
        lemma_hex_char(d0);
        assert(t[1] == 'u' && t[3] == hex_char(d3) && t[4] == hex_char(d2) && t[5] == hex_char(d1)
            && t[6] == hex_char(d0));
        assert(t.subrange(8, t.len() as int) =~= rest);
        lemma_layout_digits_value(v);
        assert((v as int) as char == c);
    } else {
        assert(t.drop_first() =~= rest);
    }
}

/// The four hex digits of a code point below `U+10000` add up to it.
proof fn lemma_layout_digits_value(v: u32)
    requires
        v < 0x10000,
    ensures
        ((v / 4096) % 16) as u8 as int * 4096 + ((v / 256) % 16) as u8 as int * 256 + ((v / 16)
            % 16) as u8 as int * 16 + (v % 16) as u8 as int == v as int,
{
    let q1 = v / 16;
    let q2 = q1 / 16;
    let q3 = q2 / 16;
    assert(v == 16 * q1 + v % 16);
    assert(q1 == 16 * q2 + q1 % 16);
    assert(q2 == 16 * q3 + q2 % 16);
    assert(q3 < 16);
    assert(v / 256 == q2) by {
        vstd::arithmetic::div_mod::lemma_div_denominator(v as int, 16, 16);
    }
    assert(v / 4096 == q3) by {
        vstd::arithmetic::div_mod::lemma_div_denominator(v as int, 256, 16);
    }
}

/// `escape_body` distributes over a leading char.
proof fn lemma_escape_body_cons(c: char, s: Seq<char>)
    ensures
        escape_body(seq![c] + s) == escape_char(c) + escape_body(s),
    decreases s.len(),
{
    if s.len() == 0 {
        assert(seq![c] + s =~= seq![c]);
        assert(seq![c].drop_last() =~= Seq::<char>::empty());
        assert(escape_char(c) + escape_body(s) =~= escape_char(c));
        assert(escape_body(Seq::<char>::empty()) + escape_char(c) =~= escape_char(c));
    } else {
        lemma_escape_body_cons(c, s.drop_last());
        assert((seq![c] + s).drop_last() =~= seq![c] + s.drop_last());
        assert((seq![c] + s).last() == s.last());
        assert(escape_char(c) + escape_body(s.drop_last()) + escape_char(s.last()) =~= escape_char(c)
            + (escape_body(s.drop_last()) + escape_char(s.last())));
    }
}

/// Decoding inverts escaping.
pub proof fn lemma_escape_body_round_trip(s: Seq<char>)
    ensures
        unescape_body(escape_body(s)) == s,
    decreases s.len(),
{
    if s.len() > 0 {
        let c = s[0];
        let rest = s.drop_first();
        assert(s =~= seq![c] + rest);
        lemma_escape_body_cons(c, rest);
        lemma_unescape_unit(c, escape_body(rest));
        lemma_escape_body_round_trip(rest);
    } else {
        assert(escape_body(s) =~= Seq::<char>::empty());
    }
}

/// An escaped body has no control character or layout control.
pub proof fn lemma_escape_body_clean(s: Seq<char>)
    ensures
        forall|i: int|
            0 <= i < escape_body(s).len() ==> !is_unsafe_char(#[trigger] escape_body(s)[i]),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_escape_body_clean(s.drop_last());
        let c = s.last();
        let e = escape_char(c);
        let v = c as u32;
        if is_control_char(c) && !(c == '"' || c == '\\' || c == '\n' || c == '\r' || c == '\t') {
            lemma_hex_char((v / 16) as u8);
            lemma_hex_char((v % 16) as u8);
        } else if is_layout_control(c) {
            lemma_hex_char(((v / 4096) % 16) as u8);
            lemma_hex_char(((v / 256) % 16) as u8);
            lemma_hex_char(((v / 16) % 16) as u8);
            lemma_hex_char((v % 16) as u8);
        }
        assert forall|i: int| 0 <= i < e.len() implies !is_unsafe_char(#[trigger] e[i]) by {}
        let b = escape_body(s.drop_last());
        assert forall|i: int| 0 <= i < escape_body(s).len() implies !is_unsafe_char(
            #[trigger] escape_body(s)[i],
        ) by {
            if i < b.len() {
                assert(escape_body(s)[i] == b[i]);
            } else {
                assert(escape_body(s)[i] == e[i - b.len()]);
            }
        }
    }
}

/// The display form of a field: identity on clean fields; otherwise quoted
/// and escaped. It never contains a control character or layout control (so
/// no `\n`, `\r`, `\t`, ESC, line separator or bidi override can split a
/// line, shift a column, reorder text or reach the terminal), and
/// `unescape_field` recovers the field, so distinct fields display
/// differently.
pub proof fn lemma_display_field(s: Seq<char>)
    ensures
        is_clean_field(s) ==> display_field_spec(s) == s,
        forall|i: int|
            0 <= i < display_field_spec(s).len() ==> !is_unsafe_char(
                #[trigger] display_field_spec(s)[i],
            ),
        unescape_field(display_field_spec(s)) == s,
        display_field_spec(s).len() == 0 <==> s.len() == 0,
{
    let d = display_field_spec(s);
    if !is_clean_field(s) {
        let body = escape_body(s);
        lemma_escape_body_clean(s);
        lemma_escape_body_round_trip(s);
        assert forall|i: int| 0 <= i < d.len() implies !is_unsafe_char(#[trigger] d[i]) by {
            if 0 < i < d.len() - 1 {
                assert(d[i] == body[i - 1]);
            }
        }
        assert(d.subrange(1, d.len() - 1) =~= body);
    }
}

/// Distinct fields are displayed differently.
pub proof fn lemma_display_field_injective(a: Seq<char>, b: Seq<char>)
    requires
        display_field_spec(a) == display_field_spec(b),
    ensures
        a == b,
{
    lemma_display_field(a);
    lemma_display_field(b);
}

/// Hex digit `d` (`d < 16`) as a char.
fn hex_char_of(d: u32) -> (c: char)
    requires
        d < 16,
    ensures
        c == hex_char(d as u8),
{
    let b: u8 = u8::try_from(d).unwrap_or_default();
    if b < 10 {
        (b'0' + b) as char
    } else {
        (b'a' + b - 10) as char
    }
}

/// Appends the escape of `c` (see `escape_char`).
fn push_escaped(out: &mut String, c: char)
    ensures
        final(out)@ == old(out)@ + escape_char(c),
{
    let ghost start = out@;
    if c == '"' {
        out.push('\\');
        out.push('"');
    } else if c == '\\' {
        out.push('\\');
        out.push('\\');
    } else if c == '\n' {
        out.push('\\');
        out.push('n');
    } else if c == '\r' {
        out.push('\\');
        out.push('r');
    } else if c == '\t' {
        out.push('\\');
        out.push('t');
    } else if is_control(c) {
        let v = c as u32;
        out.push('\\');
        out.push('x');
        out.push(hex_char_of(v / 16));
        out.push(hex_char_of(v % 16));
    } else if is_layout(c) {
        let v = c as u32;
        out.push('\\');
        out.push('u');
        out.push('{');
        out.push(hex_char_of(v / 4096 % 16));
        out.push(hex_char_of(v / 256 % 16));
        out.push(hex_char_of(v / 16 % 16));
        out.push(hex_char_of(v % 16));
        out.push('}');
    } else {
        out.push(c);
    }
    proof {
        assert(out@ =~= start + escape_char(c));
    };
}

/// Whether `c` is a control character (see `is_control_char`).
const fn is_control(c: char) -> (r: bool)
    ensures
        r == is_control_char(c),
{
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
}

/// Whether `c` is a layout control (see `is_layout_control`).
const fn is_layout(c: char) -> (r: bool)
    ensures
        r == is_layout_control(c),
{
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Escapes user text for human-readable output (see `display_field_spec`
/// and `lemma_display_field`).
pub fn escape_display_field(s: &str) -> (out: String)
    ensures
        out@ == display_field_spec(s@),
        is_clean_field(s@) ==> out@ == s@,
        forall|i: int| 0 <= i < out@.len() ==> !is_unsafe_char(#[trigger] out@[i]),
        unescape_field(out@) == s@,
{
    let mut body = String::new();
    let mut clean = true;
    let mut chars = s.chars();
    let ghost n = s@.len() as int;
    let ghost mut k: int = 0;
    let mut done = false;
    while !done
        invariant
            0 <= k <= n,
            n == s@.len(),
            vstd::std_specs::iter::IteratorSpec::remaining(&chars) == s@.subrange(k, n),
            body@ == escape_body(s@.subrange(0, k)),
            clean == is_clean_field(s@.subrange(0, k)),
            done ==> k == n,
        decreases n - k + if done {
            0int
        } else {
            1int
        },
    {
        match chars.next() {
            Some(c) => {
                let first = body.is_empty();
                let control = is_control(c) || is_layout(c);
                proof {
                    let pre = s@.subrange(0, k);
                    let next = s@.subrange(0, k + 1);
                    assert(next =~= pre.push(c));
                    assert(next.drop_last() =~= pre);
                    assert(next[k] == c);
                    assert(first <==> k == 0) by {
                        if k > 0 {
                            lemma_escape_char_nonempty(pre);
                        } else {
                            assert(pre =~= Seq::<char>::empty());
                        }
                    }
                    if k > 0 {
                        assert(next[0] == pre[0]);
                    }
                    if clean && !control && !(c == '"' && first) {
                        assert forall|i: int| 0 <= i < next.len() implies !is_unsafe_char(
                            #[trigger] next[i],
                        ) by {
                            if i < k {
                                assert(next[i] == pre[i]);
                            }
                        }
                        assert(is_clean_field(next));
                    } else if !clean {
                        if pre.len() > 0 && pre[0] == '"' {
                            assert(next[0] == '"');
                        } else {
                            let i = choose|i: int|
                                0 <= i < pre.len() && is_unsafe_char(#[trigger] pre[i]);
                            assert(next[i] == pre[i]);
                        }
                        assert(!is_clean_field(next));
                    } else {
                        assert(!is_clean_field(next));
                    }
                }
                if control || (c == '"' && first) {
                    clean = false;
                }
                push_escaped(&mut body, c);
                proof {
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
        lemma_display_field(s@);
    }
    if clean {
        s.to_owned()
    } else {
        let mut out = String::new();
        out.push('"');
        out.push_str(body.as_str());
        out.push('"');
        proof {
            assert(out@ =~= seq!['"'] + escape_body(s@) + seq!['"']);
        }
        out
    }
}

/// The escaped body of a non-empty string is non-empty.
proof fn lemma_escape_char_nonempty(s: Seq<char>)
    requires
        s.len() > 0,
    ensures
        escape_body(s).len() > 0,
{
    assert(escape_char(s.last()).len() > 0);
}

// ---------------------------------------------------------------------------
// Independence from input order
// ---------------------------------------------------------------------------

/// What rendering reads from an entry: its tag, its portable name (if
/// any), and its port.
pub open spec fn entry_view(e: GroupEntry) -> (Seq<u8>, Option<Seq<u8>>, u16) {
    (e.tag@, entry_name(e), e.port)
}

/// The views of a group's entries, in input order.
pub open spec fn entry_views(entries: Seq<GroupEntry>) -> Seq<(Seq<u8>, Option<Seq<u8>>, u16)> {
    entries.map_values(|e: GroupEntry| entry_view(e))
}

/// Tags strictly increase along `s`.
pub open spec fn strictly_sorted_views(s: Seq<(Seq<u8>, Option<Seq<u8>>, u16)>) -> bool {
    forall|a: int, b: int| 0 <= a < b < s.len() ==> lex_lt(#[trigger] s[a].0, #[trigger] s[b].0)
}

/// Two sequences with strictly increasing tags and the same elements are
/// equal.
pub proof fn lemma_strictly_sorted_views_unique(
    s1: Seq<(Seq<u8>, Option<Seq<u8>>, u16)>,
    s2: Seq<(Seq<u8>, Option<Seq<u8>>, u16)>,
)
    requires
        strictly_sorted_views(s1),
        strictly_sorted_views(s2),
        forall|x: (Seq<u8>, Option<Seq<u8>>, u16)| s1.contains(x) <==> s2.contains(x),
    ensures
        s1 == s2,
    decreases s1.len(),
{
    if s1.len() == 0 {
        if s2.len() > 0 {
            assert(s2.contains(s2[0]));
        }
        assert(s1 =~= s2);
    } else {
        assert(s1.contains(s1[0]));
        assert(s2.len() > 0);
        assert(s2.contains(s2[0]));
        let j = choose|j: int| 0 <= j < s2.len() && s2[j] == s1[0];
        let i = choose|i: int| 0 <= i < s1.len() && s1[i] == s2[0];
        if i > 0 && j > 0 {
            assert(lex_lt(s1[0].0, s1[i].0));
            assert(lex_lt(s2[0].0, s2[j].0));
            lemma_lex_strict_total(s1[0].0, s2[0].0);
        }
        assert(s1[0] == s2[0]);
        let t1 = s1.drop_first();
        let t2 = s2.drop_first();
        assert forall|x: (Seq<u8>, Option<Seq<u8>>, u16)| t1.contains(x) <==> t2.contains(x) by {
            if t1.contains(x) {
                let k = choose|k: int| 0 <= k < t1.len() && t1[k] == x;
                assert(s1[k + 1] == x);
                assert(lex_lt(s1[0].0, s1[k + 1].0));
                lemma_lex_strict_total(x.0, x.0);
                assert(s2.contains(x));
                let m = choose|m: int| 0 <= m < s2.len() && s2[m] == x;
                assert(m != 0);
                assert(t2[m - 1] == x);
            }
            if t2.contains(x) {
                let k = choose|k: int| 0 <= k < t2.len() && t2[k] == x;
                assert(s2[k + 1] == x);
                assert(lex_lt(s2[0].0, s2[k + 1].0));
                lemma_lex_strict_total(x.0, x.0);
                assert(s1.contains(x));
                let m = choose|m: int| 0 <= m < s1.len() && s1[m] == x;
                assert(m != 0);
                assert(t1[m - 1] == x);
            }
        }
        assert forall|a: int, b: int| 0 <= a < b < t1.len() implies lex_lt(
            #[trigger] t1[a].0,
            #[trigger] t1[b].0,
        ) by {
            assert(t1[a] == s1[a + 1] && t1[b] == s1[b + 1]);
        }
        assert forall|a: int, b: int| 0 <= a < b < t2.len() implies lex_lt(
            #[trigger] t2[a].0,
            #[trigger] t2[b].0,
        ) by {
            assert(t2[a] == s2[a + 1] && t2[b] == s2[b + 1]);
        }
        lemma_strictly_sorted_views_unique(t1, t2);
        assert(s1 =~= seq![s1[0]] + t1);
        assert(s2 =~= seq![s2[0]] + t2);
    }
}

/// The entries' views in tag order.
pub open spec fn sorted_views(entries: Seq<GroupEntry>) -> Seq<(Seq<u8>, Option<Seq<u8>>, u16)> {
    let order = canonical_order(tag_keys(entries));
    Seq::new(entries.len(), |a: int| entry_view(entries[order[a] as int]))
}

/// With distinct tags, the views in tag order strictly increase and contain
/// exactly the entries' views.
proof fn lemma_sorted_views(entries: Seq<GroupEntry>)
    requires
        is_sorted_order(tag_keys(entries), canonical_order(tag_keys(entries))),
        forall|i: int, j: int|
            0 <= i < j < entries.len() ==> (#[trigger] entries[i]).tag@ != (#[trigger] entries[j]).tag@,
    ensures
        strictly_sorted_views(sorted_views(entries)),
        forall|x: (Seq<u8>, Option<Seq<u8>>, u16)|
            sorted_views(entries).contains(x) <==> entry_views(entries).contains(x),
{
    let keys = tag_keys(entries);
    let order = canonical_order(keys);
    let s = sorted_views(entries);
    let v = entry_views(entries);
    assert forall|i: int, j: int| 0 <= i < j < keys.len() implies keys[i] != keys[j] by {
        assert(keys[i] == entries[i].tag@ && keys[j] == entries[j].tag@);
    }
    assert forall|a: int, b: int| 0 <= a < b < s.len() implies lex_lt(
        #[trigger] s[a].0,
        #[trigger] s[b].0,
    ) by {
        lemma_sorted_distinct_keys(keys, order, a, b);
    }
    assert forall|x: (Seq<u8>, Option<Seq<u8>>, u16)| s.contains(x) <==> v.contains(x) by {
        if s.contains(x) {
            let a = choose|a: int| 0 <= a < s.len() && s[a] == x;
            assert(v[order[a] as int] == x);
        }
        if v.contains(x) {
            let k = choose|k: int| 0 <= k < v.len() && v[k] == x;
            assert(occurs(order, k));
            let a = choose|a: int| 0 <= a < order.len() && order[a] as int == k;
            assert(s[a] == x);
        }
    }
}

/// Output order never depends on input order: two groups with distinct
/// tags and the same entries (as seen by rendering), listed in any order,
/// have the same sorted assignments, so `render_group` produces the same
/// text for both. This is why `HashMap` iteration order cannot leak into
/// the output. (`lemma_group_failure_order_independent` covers the error
/// case.)
pub proof fn lemma_group_order_independent(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>)
    requires
        e1.len() <= usize::MAX,
        e2.len() <= usize::MAX,
        distinct_tags(e1),
        distinct_tags(e2),
        same_views(e1, e2),
    ensures
        sorted_views(e1) == sorted_views(e2),
        sorted_assignments(e1) == sorted_assignments(e2),
{
    lemma_canonical_order_sorted(tag_keys(e1));
    lemma_canonical_order_sorted(tag_keys(e2));
    lemma_sorted_views(e1);
    lemma_sorted_views(e2);
    lemma_strictly_sorted_views_unique(sorted_views(e1), sorted_views(e2));
    let s1 = sorted_assignments(e1);
    let s2 = sorted_assignments(e2);
    assert forall|a: int| 0 <= a < s1.len() implies s1[a] == s2[a] by {
        assert(sorted_views(e1)[a] == sorted_views(e2)[a]);
    }
    assert(s1 =~= s2);
}

/// Entry `k` is the group's first failure in tag order: every entry that
/// sorts before it is fine (`fine_before`), and it has no portable name or
/// its name equals, ignoring ASCII case, the name of an entry that sorts
/// before it. Every error of `render_group` names such an entry.
pub open spec fn first_failure_at(entries: Seq<GroupEntry>, k: int) -> bool {
    &&& 0 <= k < entries.len()
    &&& fine_before(entries, k)
    &&& (entry_name(entries[k]) is None || exists|j: int|
        0 <= j < entries.len() && key_before(tag_keys(entries), j, k) && #[trigger] entry_name(
            entries[j],
        ) is Some && text::eq_ignore_ascii_case(name_of(entries[j]), name_of(entries[k])))
}

/// The entry a group error is about.
pub open spec fn error_source(e: GroupError) -> int {
    match e {
        GroupError::Unresolved { source } => source as int,
        GroupError::Collision { source, .. } => source as int,
    }
}

/// A failing entry cannot sort before a first failure.
proof fn lemma_failure_not_before(es: Seq<GroupEntry>, a: int, b: int)
    requires
        first_failure_at(es, a),
        first_failure_at(es, b),
        key_before(tag_keys(es), a, b),
    ensures
        false,
{
    let keys = tag_keys(es);
    assert(entry_name(es[a]) is Some);
    let j = choose|j: int|
        0 <= j < es.len() && key_before(keys, j, a) && #[trigger] entry_name(es[j]) is Some
            && text::eq_ignore_ascii_case(name_of(es[j]), name_of(es[a]));
    lemma_key_before_order(keys, j, a, b);
    lemma_key_before_order(keys, a, b, a);
    assert(j != a);
    assert(key_before(keys, j, b));
    assert(!text::eq_ignore_ascii_case(name_of(es[j]), name_of(es[a])));
}

/// A group has at most one first failure, so the error `render_group`
/// reports is determined by the group.
pub proof fn lemma_first_failure_unique(es: Seq<GroupEntry>, k1: int, k2: int)
    requires
        first_failure_at(es, k1),
        first_failure_at(es, k2),
    ensures
        k1 == k2,
{
    let keys = tag_keys(es);
    if k1 != k2 {
        lemma_key_before_order(keys, k1, k2, k1);
        if key_before(keys, k1, k2) {
            lemma_failure_not_before(es, k1, k2);
        } else {
            lemma_failure_not_before(es, k2, k1);
        }
    }
}

/// Tags are pairwise distinct.
pub open spec fn distinct_tags(es: Seq<GroupEntry>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < es.len() ==> (#[trigger] es[i]).tag@ != (#[trigger] es[j]).tag@
}

/// The two groups have the same entries as seen by rendering.
pub open spec fn same_views(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>) -> bool {
    forall|x: (Seq<u8>, Option<Seq<u8>>, u16)|
        entry_views(e1).contains(x) <==> entry_views(e2).contains(x)
}

/// The entry of `e2` that looks like `e1[i]`.
spec fn counterpart(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>, i: int) -> int {
    choose|m: int| 0 <= m < e2.len() && entry_view(#[trigger] e2[m]) == entry_view(e1[i])
}

proof fn lemma_counterpart(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>, i: int)
    requires
        same_views(e1, e2),
        0 <= i < e1.len(),
    ensures
        0 <= counterpart(e1, e2, i) < e2.len(),
        entry_view(e2[counterpart(e1, e2, i)]) == entry_view(e1[i]),
{
    assert(entry_views(e1)[i] == entry_view(e1[i]));
    assert(entry_views(e1).contains(entry_view(e1[i])));
    let m = choose|m: int| 0 <= m < e2.len() && entry_views(e2)[m] == entry_view(e1[i]);
    assert(entry_view(e2[m]) == entry_view(e1[i]));
}

/// With distinct tags, `key_before` is the order of the tags.
proof fn lemma_key_before_tags(es: Seq<GroupEntry>, i: int, j: int)
    requires
        distinct_tags(es),
        0 <= i < es.len(),
        0 <= j < es.len(),
    ensures
        key_before(tag_keys(es), i, j) == lex_lt(es[i].tag@, es[j].tag@),
        es[i].tag@ == es[j].tag@ <==> i == j,
{
    lemma_lex_strict_total(es[i].tag@, es[j].tag@);
    if i < j {
        assert(es[i].tag@ != es[j].tag@);
    } else if j < i {
        assert(es[j].tag@ != es[i].tag@);
    }
}

/// Whether a group resolves does not depend on input order.
proof fn lemma_group_resolves_transfers(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>)
    requires
        distinct_tags(e1),
        distinct_tags(e2),
        same_views(e1, e2),
        same_views(e2, e1),
        group_resolves(e1),
    ensures
        group_resolves(e2),
{
    assert forall|i: int| 0 <= i < e2.len() implies #[trigger] entry_name(e2[i]) is Some by {
        lemma_counterpart(e2, e1, i);
        assert(entry_name(e1[counterpart(e2, e1, i)]) is Some);
    }
    assert forall|i: int, j: int|
        0 <= i < e2.len() && 0 <= j < e2.len() && i != j implies !text::eq_ignore_ascii_case(
        name_of(#[trigger] e2[i]),
        name_of(#[trigger] e2[j]),
    ) by {
        lemma_counterpart(e2, e1, i);
        lemma_counterpart(e2, e1, j);
        let a = counterpart(e2, e1, i);
        let b = counterpart(e2, e1, j);
        lemma_key_before_tags(e2, i, j);
        lemma_key_before_tags(e1, a, b);
        assert(name_of(e1[a]) == name_of(e2[i]));
        assert(name_of(e1[b]) == name_of(e2[j]));
    }
}

/// A first failure of `e1` is a first failure of `e2` at the entry that
/// looks the same.
proof fn lemma_failure_transfers(e1: Seq<GroupEntry>, e2: Seq<GroupEntry>, k: int)
    requires
        distinct_tags(e1),
        distinct_tags(e2),
        same_views(e1, e2),
        same_views(e2, e1),
        first_failure_at(e1, k),
    ensures
        first_failure_at(e2, counterpart(e1, e2, k)),
{
    let keys1 = tag_keys(e1);
    let keys2 = tag_keys(e2);
    lemma_counterpart(e1, e2, k);
    let m = counterpart(e1, e2, k);
    assert forall|j: int| 0 <= j < e2.len() && key_before(keys2, j, m) implies #[trigger] entry_name(
        e2[j],
    ) is Some by {
        lemma_counterpart(e2, e1, j);
        let a = counterpart(e2, e1, j);
        lemma_key_before_tags(e2, j, m);
        lemma_key_before_tags(e1, a, k);
        assert(entry_name(e1[a]) is Some);
    }
    assert forall|i: int, j: int|
        0 <= i < e2.len() && 0 <= j < e2.len() && i != j && key_before(keys2, i, m) && key_before(
            keys2,
            j,
            m,
        ) implies !text::eq_ignore_ascii_case(
        name_of(#[trigger] e2[i]),
        name_of(#[trigger] e2[j]),
    ) by {
        lemma_counterpart(e2, e1, i);
        lemma_counterpart(e2, e1, j);
        let a = counterpart(e2, e1, i);
        let b = counterpart(e2, e1, j);
        lemma_key_before_tags(e2, i, m);
        lemma_key_before_tags(e2, j, m);
        lemma_key_before_tags(e2, i, j);
        lemma_key_before_tags(e1, a, k);
        lemma_key_before_tags(e1, b, k);
        lemma_key_before_tags(e1, a, b);
        assert(name_of(e1[a]) == name_of(e2[i]));
        assert(name_of(e1[b]) == name_of(e2[j]));
    }
    assert(fine_before(e2, m));
    if entry_name(e1[k]) is Some {
        let j = choose|j: int|
            0 <= j < e1.len() && key_before(keys1, j, k) && #[trigger] entry_name(e1[j]) is Some
                && text::eq_ignore_ascii_case(name_of(e1[j]), name_of(e1[k]));
        lemma_counterpart(e1, e2, j);
        let n = counterpart(e1, e2, j);
        lemma_key_before_tags(e1, j, k);
        lemma_key_before_tags(e2, n, m);
        assert(name_of(e2[n]) == name_of(e1[j]));
        assert(name_of(e2[m]) == name_of(e1[k]));
        assert(entry_name(e2[n]) is Some);
    }
}

/// Errors do not depend on input order either: for two groups with
/// distinct tags and the same entries (as seen by rendering), listed in any
/// order, either both resolve or neither does, and their first failures
/// (the entries `render_group` reports) look the same. Since the glue
/// reports a failure by its tag, the error message is the same too.
pub proof fn lemma_group_failure_order_independent(
    e1: Seq<GroupEntry>,
    e2: Seq<GroupEntry>,
    k1: int,
    k2: int,
)
    requires
        distinct_tags(e1),
        distinct_tags(e2),
        same_views(e1, e2),
    ensures
        group_resolves(e1) <==> group_resolves(e2),
        first_failure_at(e1, k1) && first_failure_at(e2, k2) ==> entry_view(e1[k1]) == entry_view(
            e2[k2],
        ),
{
    assert(same_views(e2, e1));
    if group_resolves(e1) {
        lemma_group_resolves_transfers(e1, e2);
    }
    if group_resolves(e2) {
        lemma_group_resolves_transfers(e2, e1);
    }
    if first_failure_at(e1, k1) && first_failure_at(e2, k2) {
        lemma_failure_transfers(e1, e2, k1);
        lemma_counterpart(e1, e2, k1);
        lemma_first_failure_unique(e2, counterpart(e1, e2, k1), k2);
    }
}

// ---------------------------------------------------------------------------
// Tab-separated rows of display fields
// ---------------------------------------------------------------------------

/// A row of fields, each in display form, separated by tabs.
pub open spec fn tab_row_spec(fields: Seq<Seq<char>>) -> Seq<char> {
    join_with(fields.map_values(|f: Seq<char>| display_field_spec(f)), '\t')
}

/// A row is one line with exactly one column per field: splitting it on
/// tabs gives the fields' display forms (each of which decodes back to its
/// field), and it contains no newline or carriage return.
pub proof fn lemma_tab_row(fields: Seq<Seq<char>>)
    ensures
        fields.len() > 0 ==> split_on(tab_row_spec(fields), '\t') == fields.map_values(
            |f: Seq<char>| display_field_spec(f),
        ),
        free_of(tab_row_spec(fields), '\n'),
        free_of(tab_row_spec(fields), '\r'),
{
    let shown = fields.map_values(|f: Seq<char>| display_field_spec(f));
    let not_line_break = |c: char| c != '\n' && c != '\r';
    assert forall|i: int| 0 <= i < shown.len() implies free_of(#[trigger] shown[i], '\t')
        && all_satisfy(shown[i], not_line_break) by {
        lemma_display_field(fields[i]);
        assert forall|j: int| 0 <= j < shown[i].len() implies shown[i][j] != '\t' && not_line_break(
            #[trigger] shown[i][j],
        ) by {
            assert(!is_unsafe_char(shown[i][j]));
        }
    }
    if fields.len() > 0 {
        lemma_split_join(shown, '\t');
    }
    lemma_join_all(shown, '\t', not_line_break);
}

/// Renders a tab-separated row of display fields (see `lemma_tab_row`).
pub fn tab_row(fields: &[&str]) -> (row: String)
    ensures
        row@ == tab_row_spec(fields@.map_values(|f: &str| f@)),
{
    let ghost views = fields@.map_values(|f: &str| f@);
    let ghost shown = views.map_values(|f: Seq<char>| display_field_spec(f));
    let mut row = String::new();
    let mut i: usize = 0;
    while i < fields.len()
        invariant
            0 <= i <= fields.len(),
            views == fields@.map_values(|f: &str| f@),
            shown == views.map_values(|f: Seq<char>| display_field_spec(f)),
            row@ == join_with(shown.subrange(0, i as int), '\t'),
        decreases fields.len() - i,
    {
        if i > 0 {
            row.push('\t');
        }
        let ghost before = row@;
        let field = escape_display_field(fields[i]);
        row.push_str(field.as_str());
        proof {
            assert(views[i as int] == fields@[i as int]@);
            let prefix = shown.subrange(0, i + 1);
            if i == 0 {
                assert(prefix =~= seq![shown[0]]);
            } else {
                assert(prefix.drop_last() =~= shown.subrange(0, i as int));
            }
        }
        i += 1;
    }
    proof {
        assert(shown.subrange(0, fields@.len() as int) =~= shown);
    }
    row
}

} // verus!
