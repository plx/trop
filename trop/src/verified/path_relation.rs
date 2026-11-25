//! Verified kernel: relationships between paths (same, ancestor,
//! descendant, unrelated) and component prefixes.
//!
//! - `spec_rel` is the relation between two component sequences: `Same`
//!   when equal, `Ancestor` when the first is a strict prefix of the second,
//!   `Descendant` for the converse, and `Unrelated` otherwise. Its algebra
//!   (exactly one case, flip symmetry, reflexivity, transitivity, the
//!   within/contains partial order) is proved once, for any element type.
//! - `path_relationship` decides `PathRelationship::between`: both paths
//!   are first resolved lexically (`spec_lex_resolve`: `.` dropped, `..`
//!   cancels the preceding name and is kept when there is none to cancel).
//!   A resolved path is a *head* (its root/prefix and any `..` that could not
//!   be resolved) followed by names; paths with different heads (relative
//!   vs. rooted, different prefixes, a different number of unresolvable
//!   `..`) are unrelated (`spec_between`), and otherwise the names decide.
//!   `path_is_within` and `path_contains` decide the derived predicates.
//! - On canonical absolute byte strings (see `super::path`),
//!   `spec_component_prefix` is the byte-level test that agrees with
//!   segment-prefix (`lemma_component_prefix_iff_segs`), and
//!   `spec_rebase` moves a path from one ancestor to another.
//! - `home_display` decides how `trop list` shortens a path below the home
//!   directory: component-wise (like `Path::strip_prefix`), `~` for the home
//!   directory itself and `~/rest` below it, and the shortened form names
//!   exactly the original path (`lemma_home_display_round_trip`).
//!
//! Production glue: `crate::path::relationship` (`PathRelationship::between`
//! and the helpers built on it).

use super::path::CompKind;
use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// The relation on sequences
// ===========================================================================

/// How two paths relate in the hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rel {
    /// The first path is a strict ancestor of the second.
    Ancestor,
    /// The first path is a strict descendant of the second.
    Descendant,
    /// The paths are the same.
    Same,
    /// Neither contains the other.
    Unrelated,
}

/// `a` is a (not necessarily strict) prefix of `b`.
pub open spec fn is_prefix_of<A>(a: Seq<A>, b: Seq<A>) -> bool {
    a.len() <= b.len() && b.subrange(0, a.len() as int) == a
}

/// The relationship between two component sequences.
pub open spec fn spec_rel<A>(a: Seq<A>, b: Seq<A>) -> Rel {
    if a == b {
        Rel::Same
    } else if is_prefix_of(a, b) {
        Rel::Ancestor
    } else if is_prefix_of(b, a) {
        Rel::Descendant
    } else {
        Rel::Unrelated
    }
}

/// The relationship seen from the other side.
pub open spec fn flip(r: Rel) -> Rel {
    match r {
        Rel::Ancestor => Rel::Descendant,
        Rel::Descendant => Rel::Ancestor,
        other => other,
    }
}

/// Exactly one case holds, each characterized independently: `Same` is
/// equality, `Ancestor`/`Descendant` are strict prefixes, and `Unrelated`
/// means neither is a prefix of the other.
pub proof fn lemma_rel_cases<A>(a: Seq<A>, b: Seq<A>)
    ensures
        (spec_rel(a, b) is Same) <==> a == b,
        (spec_rel(a, b) is Ancestor) <==> (is_prefix_of(a, b) && a.len() < b.len()),
        (spec_rel(a, b) is Descendant) <==> (is_prefix_of(b, a) && b.len() < a.len()),
        (spec_rel(a, b) is Unrelated) <==> (!is_prefix_of(a, b) && !is_prefix_of(b, a)),
{
    if is_prefix_of(a, b) && a.len() == b.len() {
        assert(b.subrange(0, b.len() as int) =~= b);
    }
    if is_prefix_of(b, a) && a.len() == b.len() {
        assert(a.subrange(0, a.len() as int) =~= a);
    }
    if a == b {
        assert(a.subrange(0, a.len() as int) =~= a);
    }
}

/// Flip symmetry: `rel(a, b)` is `rel(b, a)` seen from the other side.
pub proof fn lemma_rel_flip<A>(a: Seq<A>, b: Seq<A>)
    ensures
        spec_rel(a, b) == flip(spec_rel(b, a)),
{
    lemma_rel_cases(a, b);
    lemma_rel_cases(b, a);
}

/// Every sequence is the same as itself.
pub proof fn lemma_rel_reflexive<A>(a: Seq<A>)
    ensures
        spec_rel(a, a) == Rel::Same,
{
}

/// Prefix is transitive.
pub proof fn lemma_prefix_transitive<A>(a: Seq<A>, b: Seq<A>, c: Seq<A>)
    requires
        is_prefix_of(a, b),
        is_prefix_of(b, c),
    ensures
        is_prefix_of(a, c),
{
    assert(c.subrange(0, a.len() as int) =~= b.subrange(0, a.len() as int));
}

/// `Ancestor` is transitive.
pub proof fn lemma_rel_ancestor_transitive<A>(a: Seq<A>, b: Seq<A>, c: Seq<A>)
    requires
        spec_rel(a, b) == Rel::Ancestor,
        spec_rel(b, c) == Rel::Ancestor,
    ensures
        spec_rel(a, c) == Rel::Ancestor,
{
    lemma_rel_cases(a, b);
    lemma_rel_cases(b, c);
    lemma_prefix_transitive(a, b, c);
    lemma_rel_cases(a, c);
}

/// "`b` is within `a`" (descendant or same) as a relation on sequences.
pub open spec fn within<A>(b: Seq<A>, a: Seq<A>) -> bool {
    spec_rel(b, a) is Descendant || spec_rel(b, a) is Same
}

/// "`a` contains `b`" (ancestor or same).
pub open spec fn contains<A>(a: Seq<A>, b: Seq<A>) -> bool {
    spec_rel(a, b) is Ancestor || spec_rel(a, b) is Same
}

/// `within` is `contains` with the arguments swapped, it is exactly the
/// prefix order, and it is a partial order (reflexive, antisymmetric,
/// transitive).
pub proof fn lemma_within_partial_order<A>(a: Seq<A>, b: Seq<A>, c: Seq<A>)
    ensures
        within(b, a) <==> contains(a, b),
        within(b, a) <==> is_prefix_of(a, b),
        within(a, a),
        within(a, b) && within(b, a) ==> a == b,
        within(c, b) && within(b, a) ==> within(c, a),
{
    lemma_rel_cases(a, b);
    lemma_rel_cases(b, a);
    lemma_rel_cases(b, c);
    lemma_rel_cases(c, b);
    lemma_rel_cases(a, c);
    lemma_rel_cases(c, a);
    lemma_rel_flip(a, b);
    if a == b {
        assert(b.subrange(0, a.len() as int) =~= a);
    }
    if within(c, b) && within(b, a) {
        lemma_prefix_transitive(a, b, c);
    }
}

/// Appending at least one component makes a strict descendant.
pub proof fn lemma_rel_append<A>(a: Seq<A>, s: Seq<A>)
    requires
        s.len() > 0,
    ensures
        spec_rel(a, a + s) == Rel::Ancestor,
        spec_rel(a + s, a) == Rel::Descendant,
{
    assert((a + s).subrange(0, a.len() as int) =~= a);
    lemma_rel_cases(a, a + s);
    lemma_rel_flip(a, a + s);
}

// ===========================================================================
// Lexical resolution and the relationship between `Path`s
// ===========================================================================

/// A `std::path::Component` as data: its kind and an equality key (the
/// component's bytes; for Windows prefixes, a key that compares like the
/// parsed prefix).
pub struct PathToken {
    /// The component kind.
    pub kind: CompKind,
    /// The equality key.
    pub name: Vec<u8>,
}

/// The view of a token.
pub open spec fn tok_view(t: PathToken) -> (CompKind, Seq<u8>) {
    (t.kind, t.name@)
}

/// The view of a token list.
pub open spec fn tok_views(ts: Seq<PathToken>) -> Seq<(CompKind, Seq<u8>)> {
    ts.map_values(|t: PathToken| tok_view(t))
}

/// The kinds of a token list.
pub open spec fn kinds_of(ts: Seq<(CompKind, Seq<u8>)>) -> Seq<CompKind> {
    ts.map_values(|t: (CompKind, Seq<u8>)| t.0)
}

/// Lexical resolution, as the indices of the surviving components: `.` is
/// dropped; `..` cancels a preceding named component and is kept when there
/// is none (at the root, at the start of a relative path, after a drive
/// prefix, or after another kept `..`). The first kept `..` is exactly where
/// the strict resolution `super::path::spec_resolve` (which backs
/// `resolve_components`) fails, so the two agree whenever the strict
/// resolution succeeds (`lemma_lex_resolve_agrees_with_strict`). A kept `..` is part of the
/// path's head, never treated as a name (see `spec_between`).
pub open spec fn spec_lex_resolve(ks: Seq<CompKind>) -> Seq<int>
    decreases ks.len(),
{
    if ks.len() == 0 {
        Seq::empty()
    } else {
        let st = spec_lex_resolve(ks.drop_last());
        match ks.last() {
            CompKind::Cur => st,
            CompKind::Parent => if st.len() > 0 && ks[st.last()] is Normal {
                st.drop_last()
            } else {
                st.push(ks.len() - 1)
            },
            _ => st.push(ks.len() - 1),
        }
    }
}

/// The elements of `xs` at the indices `idx`.
pub open spec fn select<A>(xs: Seq<A>, idx: Seq<int>) -> Seq<A> {
    idx.map_values(|i: int| xs[i])
}

/// The lexically resolved token sequence.
pub open spec fn lex_resolved(ts: Seq<(CompKind, Seq<u8>)>) -> Seq<(CompKind, Seq<u8>)> {
    select(ts, spec_lex_resolve(kinds_of(ts)))
}

/// The number of leading tokens of `ts`, from index `i` on, that are not
/// named components.
pub open spec fn head_len_from(ts: Seq<(CompKind, Seq<u8>)>, i: int) -> nat
    decreases ts.len() - i,
{
    if 0 <= i < ts.len() && !(ts[i].0 is Normal) {
        1 + head_len_from(ts, i + 1)
    } else {
        0
    }
}

/// The length of the head of `ts`: its leading run of components that are
/// not named. For a resolved path (`lex_resolved`) the head is the root
/// and/or prefix followed by the `..` components that could not be resolved
/// (`lemma_lex_resolved_shape`).
pub open spec fn head_len(ts: Seq<(CompKind, Seq<u8>)>) -> nat {
    head_len_from(ts, 0)
}

/// `x` and `y` have the same head: the same anchor (both relative, both
/// rooted, the same prefix) and the same unresolvable `..` components.
pub open spec fn same_head(x: Seq<(CompKind, Seq<u8>)>, y: Seq<(CompKind, Seq<u8>)>) -> bool {
    head_len(x) == head_len(y) && x.take(head_len(x) as int) == y.take(head_len(y) as int)
}

/// The relation between resolved token sequences: `spec_rel` when the
/// heads agree (so only the names after the head can differ), `Unrelated`
/// otherwise.
pub open spec fn spec_lexical_rel(x: Seq<(CompKind, Seq<u8>)>, y: Seq<(CompKind, Seq<u8>)>) -> Rel {
    if same_head(x, y) {
        spec_rel(x, y)
    } else {
        Rel::Unrelated
    }
}

/// `PathRelationship::between(a, b)`: the relation between the lexically
/// resolved component sequences.
pub open spec fn spec_between(a: Seq<(CompKind, Seq<u8>)>, b: Seq<(CompKind, Seq<u8>)>) -> Rel {
    spec_lexical_rel(lex_resolved(a), lex_resolved(b))
}

/// `PathRelationship::is_within(a, b)`: `a` is `b` or lies below it.
pub open spec fn between_within(a: Seq<(CompKind, Seq<u8>)>, b: Seq<(CompKind, Seq<u8>)>) -> bool {
    spec_between(a, b) is Descendant || spec_between(a, b) is Same
}

/// `PathRelationship::contains(a, b)`: `a` is `b` or lies above it.
pub open spec fn between_contains(a: Seq<(CompKind, Seq<u8>)>, b: Seq<(CompKind, Seq<u8>)>) -> bool {
    spec_between(a, b) is Ancestor || spec_between(a, b) is Same
}

/// The shape `std` gives a component list: roots and prefixes occur only as
/// a leading run.
pub open spec fn anchors_lead(ks: Seq<CompKind>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < ks.len() && super::path::is_anchor(#[trigger] ks[j]) ==> super::path::is_anchor(
            #[trigger] ks[i],
        )
}

/// No named component is followed by a component of another kind.
pub open spec fn names_closed(ks: Seq<CompKind>) -> bool {
    forall|i: int, j: int| 0 <= i < j < ks.len() && #[trigger] ks[i] is Normal ==> #[trigger] ks[j] is Normal
}

/// Every component after the head of `x` is a named component.
pub open spec fn names_after_head(x: Seq<(CompKind, Seq<u8>)>) -> bool {
    forall|j: int| head_len(x) <= j < x.len() ==> #[trigger] x[j].0 is Normal
}

// ---------------------------------------------------------------------------
// Lemmas
// ---------------------------------------------------------------------------

/// The head never exceeds the sequence, consists of non-named components,
/// and is followed by a named component (if anything).
pub proof fn lemma_head_len_bound(ts: Seq<(CompKind, Seq<u8>)>, i: int)
    requires
        0 <= i <= ts.len(),
    ensures
        i + head_len_from(ts, i) <= ts.len(),
        forall|j: int| i <= j < i + head_len_from(ts, i) ==> !(#[trigger] ts[j].0 is Normal),
        i + head_len_from(ts, i) < ts.len() ==> ts[i + head_len_from(ts, i)].0 is Normal,
    decreases ts.len() - i,
{
    if i < ts.len() && !(ts[i].0 is Normal) {
        lemma_head_len_bound(ts, i + 1);
    }
}

/// Lexical resolution keeps indices in bounds, in increasing order.
pub proof fn lemma_lex_resolve_bounds(ks: Seq<CompKind>)
    ensures
        forall|j: int|
            0 <= j < spec_lex_resolve(ks).len() ==> 0 <= #[trigger] spec_lex_resolve(ks)[j]
                < ks.len(),
        forall|j: int, l: int|
            0 <= j < l < spec_lex_resolve(ks).len() ==> spec_lex_resolve(ks)[j]
                < spec_lex_resolve(ks)[l],
    decreases ks.len(),
{
    if ks.len() > 0 {
        let prev = ks.drop_last();
        lemma_lex_resolve_bounds(prev);
        let st = spec_lex_resolve(prev);
        let r = spec_lex_resolve(ks);
        assert(r == st || r == st.drop_last() || r == st.push(ks.len() - 1));
        assert forall|j: int| 0 <= j < r.len() implies 0 <= #[trigger] r[j] < ks.len() by {
            if j < st.len() {
                assert(r[j] == st[j]);
            }
        }
        assert forall|j: int, l: int| 0 <= j < l < r.len() implies r[j] < r[l] by {
            if l < st.len() {
                assert(r[j] == st[j]);
                assert(r[l] == st[l]);
            } else {
                assert(r[j] == st[j]);
            }
        }
    }
}

/// When the strict resolution of `super::path` succeeds (the normalizer
/// accepts the path), lexical resolution agrees with it.
pub proof fn lemma_lex_resolve_agrees_with_strict(ks: Seq<CompKind>)
    requires
        super::path::spec_resolve(ks) is Some,
    ensures
        spec_lex_resolve(ks) == super::path::spec_resolve(ks)->Some_0,
    decreases ks.len(),
{
    if ks.len() > 0 {
        let prev = ks.drop_last();
        assert(super::path::spec_resolve(prev) is Some);
        lemma_lex_resolve_agrees_with_strict(prev);
    }
}

/// Lexical resolution keeps a `..` exactly when the strict resolution
/// (`super::path::spec_resolve`, which backs `resolve_components`) fails: a
/// path's head contains an unresolvable `..` if and only if
/// `resolve_components` rejects it.
pub proof fn lemma_lex_resolve_keeps_parent_iff_strict_fails(ks: Seq<CompKind>)
    ensures
        (super::path::spec_resolve(ks) is None) <==> (exists|j: int|
            0 <= j < spec_lex_resolve(ks).len() && (ks[#[trigger] spec_lex_resolve(ks)[j]] is Parent)),
    decreases ks.len(),
{
    if ks.len() > 0 {
        let n = ks.len() as int;
        let prev = ks.drop_last();
        lemma_lex_resolve_keeps_parent_iff_strict_fails(prev);
        lemma_lex_resolve_bounds(prev);
        let st = spec_lex_resolve(prev);
        let r = spec_lex_resolve(ks);
        assert forall|j: int| 0 <= j < st.len() implies ks[#[trigger] st[j]] == prev[st[j]] by {
            assert(0 <= st[j] < prev.len());
        }
        let pops = ks.last() is Parent && st.len() > 0 && ks[st.last()] is Normal;
        if super::path::spec_resolve(prev) is None {
            let j = choose|j: int| 0 <= j < st.len() && (prev[#[trigger] st[j]] is Parent);
            assert(ks[st[j]] is Parent);
            if pops {
                assert(j != st.len() - 1);
                assert(r == st.drop_last());
            }
            assert(0 <= j < r.len() && r[j] == st[j]);
            assert(ks[r[j]] is Parent);
        } else {
            lemma_lex_resolve_agrees_with_strict(prev);
            assert(forall|j: int| 0 <= j < st.len() ==> !(prev[#[trigger] st[j]] is Parent));
            if ks.last() is Parent && !pops {
                assert(super::path::spec_resolve(ks) is None);
                assert(r == st.push(n - 1));
                assert(r[r.len() - 1] == n - 1);
                assert(ks[r[r.len() - 1]] is Parent);
            } else {
                assert(super::path::spec_resolve(ks) is Some);
                assert forall|j: int| 0 <= j < r.len() implies !(ks[#[trigger] r[j]] is Parent) by {
                    if j < st.len() {
                        assert(r[j] == st[j]);
                    } else {
                        assert(r[j] == n - 1);
                    }
                }
            }
        }
    }
}

/// For `std`-shaped input (anchors only as a leading run), no named
/// component of the resolved kinds is followed by one of another kind.
proof fn lemma_lex_resolve_closed(ks: Seq<CompKind>)
    requires
        anchors_lead(ks),
    ensures
        names_closed(super::path::select_kinds(ks, spec_lex_resolve(ks))),
    decreases ks.len(),
{
    if ks.len() > 0 {
        let n = ks.len() as int;
        let prev = ks.drop_last();
        assert(anchors_lead(prev)) by {
            assert forall|i: int, j: int|
                0 <= i < j < prev.len() && super::path::is_anchor(
                    #[trigger] prev[j],
                ) implies super::path::is_anchor(#[trigger] prev[i]) by {
                assert(prev[i] == ks[i] && prev[j] == ks[j]);
            }
        }
        lemma_lex_resolve_closed(prev);
        lemma_lex_resolve_bounds(prev);
        let st = spec_lex_resolve(prev);
        let sp = super::path::select_kinds(prev, st);
        let so = super::path::select_kinds(ks, st);
        assert(so =~= sp) by {
            assert forall|j: int| 0 <= j < st.len() implies so[j] == sp[j] by {
                assert(0 <= st[j] < prev.len());
            }
        }
        let r = spec_lex_resolve(ks);
        let x = super::path::select_kinds(ks, r);
        let k = ks.last();
        if k is Cur {
            assert(x =~= so);
        } else if k is Parent && st.len() > 0 && ks[st.last()] is Normal {
            assert(x =~= so.drop_last());
            assert forall|i: int, j: int|
                0 <= i < j < x.len() && #[trigger] x[i] is Normal implies #[trigger] x[j] is Normal by {
                assert(x[i] == sp[i] && x[j] == sp[j]);
            }
        } else {
            assert(r == st.push(n - 1));
            assert(x =~= so.push(k));
            // Nothing kept so far is named when a non-named component is
            // pushed: for `..` the top is not named (and names are closed);
            // for an anchor, every earlier component is an anchor.
            assert forall|i: int| 0 <= i < sp.len() && !(k is Normal) implies !(
            #[trigger] sp[i] is Normal) by {
                if k is Parent {
                    assert(sp[sp.len() - 1] == ks[st.last()]);
                    if i < sp.len() - 1 && sp[i] is Normal {
                        assert(sp[sp.len() - 1] is Normal);
                    }
                } else {
                    assert(super::path::is_anchor(ks[n - 1]));
                    assert(0 <= st[i] < prev.len());
                    assert(super::path::is_anchor(ks[st[i]]));
                }
            }
            assert forall|i: int, j: int|
                0 <= i < j < x.len() && #[trigger] x[i] is Normal implies #[trigger] x[j] is Normal by {
                assert(x[i] == sp[i]);
                if j < sp.len() {
                    assert(x[j] == sp[j]);
                } else {
                    assert(x[j] == k);
                }
            }
        }
    }
}

/// The shape of a resolved `std`-shaped path: a head (root and/or prefix,
/// then the `..` components that could not be resolved) followed by named
/// components only.
pub proof fn lemma_lex_resolved_shape(ts: Seq<(CompKind, Seq<u8>)>)
    requires
        anchors_lead(kinds_of(ts)),
    ensures
        names_after_head(lex_resolved(ts)),
{
    let ks = kinds_of(ts);
    let idx = spec_lex_resolve(ks);
    let x = lex_resolved(ts);
    lemma_lex_resolve_closed(ks);
    lemma_lex_resolve_bounds(ks);
    lemma_head_len_bound(x, 0);
    let sel = super::path::select_kinds(ks, idx);
    assert forall|j: int| 0 <= j < x.len() implies #[trigger] x[j].0 == sel[j] by {
        assert(0 <= idx[j] < ks.len());
    }
    let h = head_len(x) as int;
    assert forall|j: int| h <= j < x.len() implies #[trigger] x[j].0 is Normal by {
        assert(x[h].0 == sel[h]);
        assert(x[j].0 == sel[j]);
        if j > h {
            assert(sel[h] is Normal);
        }
    }
}

/// A leading anchor survives lexical resolution in first position.
pub proof fn lemma_lex_resolve_keeps_first_anchor(ks: Seq<CompKind>)
    requires
        ks.len() > 0,
        super::path::is_anchor(ks[0]),
    ensures
        spec_lex_resolve(ks).len() > 0,
        spec_lex_resolve(ks)[0] == 0,
    decreases ks.len(),
{
    if ks.len() == 1 {
        assert(ks.drop_last() =~= Seq::<CompKind>::empty());
    } else {
        let prev = ks.drop_last();
        assert(prev[0] == ks[0]);
        lemma_lex_resolve_keeps_first_anchor(prev);
        let st = spec_lex_resolve(prev);
        if ks.last() is Parent && st.len() > 0 && ks[st.last()] is Normal {
            if st.len() == 1 {
                assert(st.last() == 0);
            }
        }
    }
}

/// Paths are related to themselves as `Same`.
pub proof fn lemma_between_reflexive(a: Seq<(CompKind, Seq<u8>)>)
    ensures
        spec_between(a, a) == Rel::Same,
{
}

/// Flip symmetry of `between`.
pub proof fn lemma_between_flip(a: Seq<(CompKind, Seq<u8>)>, b: Seq<(CompKind, Seq<u8>)>)
    ensures
        spec_between(a, b) == flip(spec_between(b, a)),
{
    lemma_rel_flip(lex_resolved(a), lex_resolved(b));
}

/// `Ancestor` is transitive for `between`.
pub proof fn lemma_between_ancestor_transitive(
    a: Seq<(CompKind, Seq<u8>)>,
    b: Seq<(CompKind, Seq<u8>)>,
    c: Seq<(CompKind, Seq<u8>)>,
)
    requires
        spec_between(a, b) == Rel::Ancestor,
        spec_between(b, c) == Rel::Ancestor,
    ensures
        spec_between(a, c) == Rel::Ancestor,
{
    lemma_rel_ancestor_transitive(lex_resolved(a), lex_resolved(b), lex_resolved(c));
}

/// `between` is `Same` exactly when the resolved sequences are equal.
pub proof fn lemma_between_same_iff(a: Seq<(CompKind, Seq<u8>)>, b: Seq<(CompKind, Seq<u8>)>)
    ensures
        (spec_between(a, b) is Same) <==> lex_resolved(a) == lex_resolved(b),
{
    lemma_rel_cases(lex_resolved(a), lex_resolved(b));
}

/// `Ancestor` and `Descendant` mean what they say: for `std`-shaped paths
/// the deeper resolved path is the shallower one followed by at least one
/// named component (never by a `..`, a root or a prefix).
pub proof fn lemma_between_strict_extension_is_named(
    a: Seq<(CompKind, Seq<u8>)>,
    b: Seq<(CompKind, Seq<u8>)>,
)
    requires
        anchors_lead(kinds_of(a)),
        anchors_lead(kinds_of(b)),
    ensures
        spec_between(a, b) is Ancestor ==> is_prefix_of(lex_resolved(a), lex_resolved(b))
            && lex_resolved(a).len() < lex_resolved(b).len() && (forall|i: int|
            lex_resolved(a).len() <= i < lex_resolved(b).len() ==> (#[trigger] lex_resolved(
                b,
            )[i]).0 is Normal),
        spec_between(a, b) is Descendant ==> is_prefix_of(lex_resolved(b), lex_resolved(a))
            && lex_resolved(b).len() < lex_resolved(a).len() && (forall|i: int|
            lex_resolved(b).len() <= i < lex_resolved(a).len() ==> (#[trigger] lex_resolved(
                a,
            )[i]).0 is Normal),
{
    let x = lex_resolved(a);
    let y = lex_resolved(b);
    lemma_lex_resolved_shape(a);
    lemma_lex_resolved_shape(b);
    lemma_head_len_bound(x, 0);
    lemma_head_len_bound(y, 0);
    lemma_rel_cases(x, y);
}

/// `within` and `contains` built on `between` (what `is_within` and
/// `contains` compute) are converses of each other, and `within` is a
/// partial order on paths up to lexical resolution: reflexive, antisymmetric
/// (mutually within means the same resolved path), and transitive. It holds
/// exactly when the heads agree and the resolved `b` is a prefix of the
/// resolved `a`.
pub proof fn lemma_between_within_partial_order(
    a: Seq<(CompKind, Seq<u8>)>,
    b: Seq<(CompKind, Seq<u8>)>,
    c: Seq<(CompKind, Seq<u8>)>,
)
    ensures
        between_within(b, a) <==> between_contains(a, b),
        between_within(b, a) <==> same_head(lex_resolved(a), lex_resolved(b)) && is_prefix_of(
            lex_resolved(a),
            lex_resolved(b),
        ),
        between_within(a, a),
        between_within(a, b) && between_within(b, a) ==> lex_resolved(a) == lex_resolved(b),
        between_within(c, b) && between_within(b, a) ==> between_within(c, a),
{
    let x = lex_resolved(a);
    let y = lex_resolved(b);
    let z = lex_resolved(c);
    lemma_within_partial_order(x, y, z);
    lemma_within_partial_order(y, x, z);
    lemma_between_flip(a, b);
}

/// A path with no anchor (relative, including the empty path) is unrelated
/// to an anchored path (one starting at the root, or with a prefix): never
/// its ancestor or descendant.
pub proof fn lemma_between_relative_vs_rooted(
    a: Seq<(CompKind, Seq<u8>)>,
    b: Seq<(CompKind, Seq<u8>)>,
)
    requires
        forall|i: int| 0 <= i < a.len() ==> !super::path::is_anchor(#[trigger] a[i].0),
        b.len() > 0,
        super::path::is_anchor(b[0].0),
    ensures
        spec_between(a, b) == Rel::Unrelated,
        spec_between(b, a) == Rel::Unrelated,
{
    let ka = kinds_of(a);
    let kb = kinds_of(b);
    lemma_lex_resolve_bounds(ka);
    let ra = lex_resolved(a);
    let rb = lex_resolved(b);
    assert(kb[0] == b[0].0);
    lemma_lex_resolve_keeps_first_anchor(kb);
    assert(rb[0] == b[0]);
    assert(head_len(rb) >= 1);
    lemma_head_len_bound(ra, 0);
    lemma_head_len_bound(rb, 0);
    if head_len(ra) == head_len(rb) {
        assert(ra[0] == a[spec_lex_resolve(ka)[0]]);
        assert(ra.take(head_len(ra) as int)[0] == ra[0]);
        assert(rb.take(head_len(rb) as int)[0] == rb[0]);
    }
    assert(!same_head(ra, rb));
    assert(!same_head(rb, ra));
}

// ---------------------------------------------------------------------------
// Executable relationship
// ---------------------------------------------------------------------------

/// Lexically resolves a token list; returns the surviving indices.
fn lex_resolve_tokens(ts: &[PathToken]) -> (kept: Vec<usize>)
    ensures
        super::path::usize_ints(kept@) == spec_lex_resolve(kinds_of(tok_views(ts@))),
        forall|j: int| 0 <= j < kept.len() ==> #[trigger] kept[j] < ts.len(),
{
    let ghost ks = kinds_of(tok_views(ts@));
    let mut stack: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < ts.len()
        invariant
            i <= ts.len(),
            ks == kinds_of(tok_views(ts@)),
            spec_lex_resolve(ks.subrange(0, i as int)) == super::path::usize_ints(stack@),
            forall|j: int| 0 <= j < stack.len() ==> #[trigger] stack[j] < i,
        decreases ts.len() - i,
    {
        proof {
            assert(ks.subrange(0, i + 1).drop_last() =~= ks.subrange(0, i as int));
            assert(ks[i as int] == ts@[i as int].kind);
            assert(forall|j: int| 0 <= j < ts.len() ==> #[trigger] ks[j] == ts@[j].kind);
        }
        let ghost before = stack@;
        match ts[i].kind {
            CompKind::Cur => {},
            CompKind::Parent => {
                let n = stack.len();
                if n > 0 && matches!(ts[stack[n - 1]].kind, CompKind::Normal) {
                    stack.pop();
                    proof {
                        assert(super::path::usize_ints(stack@) =~= super::path::usize_ints(
                            before,
                        ).drop_last());
                    };
                } else {
                    stack.push(i);
                    proof {
                        assert(super::path::usize_ints(stack@) =~= super::path::usize_ints(
                            before,
                        ).push(i as int));
                    };
                }
            },
            _ => {
                stack.push(i);
                proof {
                    assert(super::path::usize_ints(stack@) =~= super::path::usize_ints(
                        before,
                    ).push(i as int));
                };
            },
        }
        i += 1;
    }
    proof {
        assert(ks.subrange(0, ts.len() as int) =~= ks);
    }
    stack
}

/// Whether two component kinds are equal.
fn kind_eq(a: CompKind, b: CompKind) -> (eq: bool)
    ensures
        eq == (a == b),
{
    matches!(
        (a, b),
        (CompKind::Prefix, CompKind::Prefix) | (CompKind::Root, CompKind::Root) | (
            CompKind::Cur,
            CompKind::Cur,
        ) | (CompKind::Parent, CompKind::Parent) | (CompKind::Normal, CompKind::Normal)
    )
}

/// Whether two tokens are equal (same kind, same key bytes).
pub fn token_eq(a: &PathToken, b: &PathToken) -> (eq: bool)
    ensures
        eq == (tok_view(*a) == tok_view(*b)),
{
    if !kind_eq(a.kind, b.kind) || a.name.len() != b.name.len() {
        proof {
            if tok_view(*a) == tok_view(*b) {
                assert(a.name@.len() == b.name@.len());
            }
        }
        return false;
    }
    let mut k: usize = 0;
    while k < a.name.len()
        invariant
            k <= a.name.len(),
            a.name.len() == b.name.len(),
            forall|j: int| 0 <= j < k ==> a.name@[j] == b.name@[j],
        decreases a.name.len() - k,
    {
        if a.name[k] != b.name[k] {
            return false;
        }
        k += 1;
    }
    proof {
        assert(a.name@ =~= b.name@);
    }
    true
}

/// The head length of the resolved sequence `select(ts, idx)`.
fn count_head(ts: &[PathToken], idx: &[usize]) -> (count: usize)
    requires
        forall|j: int| 0 <= j < idx.len() ==> #[trigger] idx[j] < ts.len(),
    ensures
        count as int == head_len(select(tok_views(ts@), super::path::usize_ints(idx@))),
{
    let ghost x = select(tok_views(ts@), super::path::usize_ints(idx@));
    let mut i: usize = 0;
    while i < idx.len() && !matches!(ts[idx[i]].kind, CompKind::Normal)
        invariant
            i <= idx.len(),
            x == select(tok_views(ts@), super::path::usize_ints(idx@)),
            forall|j: int| 0 <= j < idx.len() ==> #[trigger] idx[j] < ts.len(),
            head_len_from(x, 0) == i + head_len_from(x, i as int),
        decreases idx.len() - i,
    {
        proof {
            assert(x[i as int] == tok_view(ts@[idx@[i as int] as int]));
        }
        i += 1;
    }
    proof {
        if i < idx.len() {
            assert(x[i as int] == tok_view(ts@[idx@[i as int] as int]));
        }
    }
    i
}

/// The length of the longest common prefix of `select(a, ia)` and
/// `select(b, ib)`.
fn common_prefix_len(a: &[PathToken], ia: &[usize], b: &[PathToken], ib: &[usize]) -> (n: usize)
    requires
        forall|j: int| 0 <= j < ia.len() ==> #[trigger] ia[j] < a.len(),
        forall|j: int| 0 <= j < ib.len() ==> #[trigger] ib[j] < b.len(),
    ensures
        ({
            let x = select(tok_views(a@), super::path::usize_ints(ia@));
            let y = select(tok_views(b@), super::path::usize_ints(ib@));
            &&& n <= x.len()
            &&& n <= y.len()
            &&& forall|j: int| 0 <= j < n ==> x[j] == y[j]
            &&& n < x.len() && n < y.len() ==> x[n as int] != y[n as int]
        }),
{
    let ghost x = select(tok_views(a@), super::path::usize_ints(ia@));
    let ghost y = select(tok_views(b@), super::path::usize_ints(ib@));
    let mut i: usize = 0;
    while i < ia.len() && i < ib.len() && token_eq(&a[ia[i]], &b[ib[i]])
        invariant
            i <= ia.len(),
            i <= ib.len(),
            x == select(tok_views(a@), super::path::usize_ints(ia@)),
            y == select(tok_views(b@), super::path::usize_ints(ib@)),
            forall|j: int| 0 <= j < ia.len() ==> #[trigger] ia[j] < a.len(),
            forall|j: int| 0 <= j < ib.len() ==> #[trigger] ib[j] < b.len(),
            forall|j: int| 0 <= j < i ==> x[j] == y[j],
        decreases ia.len() - i,
    {
        proof {
            assert(x[i as int] == tok_view(a@[ia@[i as int] as int]));
            assert(y[i as int] == tok_view(b@[ib@[i as int] as int]));
        }
        i += 1;
    }
    proof {
        if i < ia.len() && i < ib.len() {
            assert(x[i as int] == tok_view(a@[ia@[i as int] as int]));
            assert(y[i as int] == tok_view(b@[ib@[i as int] as int]));
        }
    }
    i
}

/// Decides `spec_between`: the relationship between two paths given as
/// component tokens, after lexical resolution, with heads compared.
pub fn path_relationship(a: &[PathToken], b: &[PathToken]) -> (r: Rel)
    ensures
        r == spec_between(tok_views(a@), tok_views(b@)),
{
    let ia = lex_resolve_tokens(a);
    let ib = lex_resolve_tokens(b);
    let ghost x = select(tok_views(a@), super::path::usize_ints(ia@));
    let ghost y = select(tok_views(b@), super::path::usize_ints(ib@));
    proof {
        assert(x == lex_resolved(tok_views(a@)));
        assert(y == lex_resolved(tok_views(b@)));
        lemma_head_len_bound(x, 0);
        lemma_head_len_bound(y, 0);
        lemma_rel_cases(x, y);
    }
    let ca = count_head(a, &ia);
    let cb = count_head(b, &ib);
    let common = common_prefix_len(a, &ia, b, &ib);
    proof {
        if ca == cb {
            if common >= ca {
                assert(x.take(ca as int) =~= y.take(cb as int));
            } else {
                assert(x.take(ca as int)[common as int] == x[common as int]);
                assert(y.take(cb as int)[common as int] == y[common as int]);
            }
        }
        if common == x.len() {
            assert(y.subrange(0, x.len() as int) =~= x);
        }
        if is_prefix_of(x, y) && common < x.len() {
            assert(y.subrange(0, x.len() as int)[common as int] == y[common as int]);
        }
        if common == y.len() {
            assert(x.subrange(0, y.len() as int) =~= y);
        }
        if is_prefix_of(y, x) && common < y.len() {
            assert(x.subrange(0, y.len() as int)[common as int] == x[common as int]);
        }
        if common == x.len() && common == y.len() {
            assert(x =~= y);
        }
    }
    if ca != cb || common < ca {
        return Rel::Unrelated;
    }
    let na = ia.len();
    let nb = ib.len();
    if common == na && common == nb {
        Rel::Same
    } else if common == na {
        Rel::Ancestor
    } else if common == nb {
        Rel::Descendant
    } else {
        Rel::Unrelated
    }
}

/// Decides `between_within`: `path` is `dir` or lies below it.
pub fn path_is_within(path: &[PathToken], dir: &[PathToken]) -> (within: bool)
    ensures
        within == between_within(tok_views(path@), tok_views(dir@)),
{
    matches!(path_relationship(path, dir), Rel::Descendant | Rel::Same)
}

/// Decides `between_contains`: `path` is `other` or lies above it.
pub fn path_contains(path: &[PathToken], other: &[PathToken]) -> (contains: bool)
    ensures
        contains == between_contains(tok_views(path@), tok_views(other@)),
{
    matches!(path_relationship(path, other), Rel::Ancestor | Rel::Same)
}

// ===========================================================================
// Canonical absolute byte strings (vocabulary for stored paths)
// ===========================================================================

/// `p` is a component prefix of `s` on byte strings: a byte prefix that ends
/// at a component boundary (`/a/b` is not a component prefix of `/a/bc`;
/// `/` is a component prefix of every absolute path).
pub open spec fn spec_component_prefix(p: Seq<u8>, s: Seq<u8>) -> bool {
    is_prefix_of(p, s) && (p.len() == s.len() || p == seq![b'/'] || s[p.len() as int] == b'/')
}

/// Decides `spec_component_prefix(p, s)`: `p` is a byte prefix of `s` that
/// ends at a component boundary. Total. On canonical absolute strings this
/// is segment prefix, i.e. `Path::starts_with` (`lemma_component_prefix_iff_segs`).
// Only the Unix glue (stored-path prefix queries) calls this.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn canonical_is_component_prefix(p: &[u8], s: &[u8]) -> (r: bool)
    ensures
        r == spec_component_prefix(p@, s@),
{
    let n = p.len();
    if n > s.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < n
        invariant
            n == p.len() <= s.len(),
            0 <= i <= n,
            forall|j: int| 0 <= j < i ==> p@[j] == s@[j],
        decreases n - i,
    {
        if p[i] != s[i] {
            proof {
                assert(s@.subrange(0, n as int)[i as int] == s@[i as int]);
            }
            return false;
        }
        i += 1;
    }
    proof {
        assert(s@.subrange(0, n as int) =~= p@);
    }
    let boundary = n == s.len() || (n == 1 && p[0] == b'/') || s[n] == b'/';
    proof {
        if n == 1 && p@[0] == b'/' {
            assert(p@ =~= seq![b'/']);
        }
        if p@ == seq![b'/'] {
            assert(p@[0] == b'/');
        }
    }
    boundary
}

/// `render_tail` distributes over concatenation.
pub proof fn lemma_render_tail_concat(a: Seq<Seq<u8>>, b: Seq<Seq<u8>>)
    ensures
        super::path::render_tail(a + b) == super::path::render_tail(a) + super::path::render_tail(
            b,
        ),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(super::path::render_tail(b) =~= Seq::<u8>::empty());
        assert(super::path::render_tail(a) + super::path::render_tail(b)
            =~= super::path::render_tail(a));
    } else {
        lemma_render_tail_concat(a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        assert(super::path::render_tail(a + b) =~= super::path::render_tail(a)
            + super::path::render_tail(b));
    }
}

/// Segment prefix implies the byte-level component prefix.
proof fn lemma_segs_prefix_to_component_prefix(p: Seq<u8>, s: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(p, false),
        super::path::spec_is_canonical_abs(s, false),
        is_prefix_of(super::path::segs(p), super::path::segs(s)),
    ensures
        spec_component_prefix(p, s),
{
    super::path::lemma_canonical_render(p, false);
    super::path::lemma_canonical_render(s, false);
    let sp = super::path::segs(p);
    let ss = super::path::segs(s);
    if sp.len() == 0 {
        assert(p == seq![b'/']);
        assert(s.subrange(0, 1) =~= seq![b'/']);
    } else {
        let rest = ss.subrange(sp.len() as int, ss.len() as int);
        assert(ss =~= sp + rest);
        lemma_render_tail_concat(sp, rest);
        assert(p == super::path::render_tail(sp));
        if rest.len() > 0 {
            assert(s == super::path::render_tail(sp) + super::path::render_tail(rest));
            assert(s.subrange(0, p.len() as int) =~= p);
            super::path::lemma_render_tail_first(rest);
            assert(s[p.len() as int] == super::path::render_tail(rest)[0]);
        } else {
            assert(rest =~= Seq::<Seq<u8>>::empty());
            assert(ss =~= sp);
            assert(s.subrange(0, p.len() as int) =~= p);
        }
    }
}

/// The byte-level component prefix implies segment prefix.
proof fn lemma_component_prefix_to_segs_prefix(p: Seq<u8>, s: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(p, false),
        super::path::spec_is_canonical_abs(s, false),
        spec_component_prefix(p, s),
    ensures
        is_prefix_of(super::path::segs(p), super::path::segs(s)),
{
    super::path::lemma_canonical_render(p, false);
    super::path::lemma_canonical_render(s, false);
    let sp = super::path::segs(p);
    let ss = super::path::segs(s);
    if p.len() == s.len() {
        assert(s.subrange(0, s.len() as int) =~= s);
        assert(sp.subrange(0, sp.len() as int) =~= sp);
    } else if p == seq![b'/'] {
        assert(sp.len() == 0) by {
            if sp.len() > 0 {
                assert(super::path::render(sp) == super::path::render_tail(sp));
                assert(super::path::is_normal_seg(sp[sp.len() - 1]));
                assert(super::path::render_tail(sp).len() >= 2);
            }
        }
        assert(ss.subrange(0, 0) =~= sp);
    } else {
        let r = s.subrange(p.len() as int + 1, s.len() as int);
        assert(s =~= p + seq![b'/'] + r);
        super::path::lemma_segs_concat(p, r);
        assert(ss.subrange(0, sp.len() as int) =~= sp);
    }
}

/// On canonical absolute strings, the byte-level component-prefix test is
/// exactly segment-sequence prefix (`Path::starts_with`).
pub proof fn lemma_component_prefix_iff_segs(p: Seq<u8>, s: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(p, false),
        super::path::spec_is_canonical_abs(s, false),
    ensures
        spec_component_prefix(p, s) <==> is_prefix_of(super::path::segs(p), super::path::segs(s)),
{
    if is_prefix_of(super::path::segs(p), super::path::segs(s)) {
        lemma_segs_prefix_to_component_prefix(p, s);
    }
    if spec_component_prefix(p, s) {
        lemma_component_prefix_to_segs_prefix(p, s);
    }
}

/// Completeness of a byte-prefix prefilter: a segment-level descendant (or
/// the path itself) of a canonical `p` starts with the bytes of `p`.
pub proof fn lemma_segs_prefix_implies_byte_prefix(p: Seq<u8>, s: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(p, false),
        super::path::spec_is_canonical_abs(s, false),
        is_prefix_of(super::path::segs(p), super::path::segs(s)),
    ensures
        is_prefix_of(p, s),
{
    lemma_component_prefix_iff_segs(p, s);
}

/// `/` is a component prefix of every canonical absolute path.
pub proof fn lemma_component_prefix_root(s: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(s, false),
    ensures
        spec_component_prefix(seq![b'/'], s),
{
    super::path::lemma_canonical_render(s, false);
    assert(s.subrange(0, 1) =~= seq![b'/']);
}

/// On canonical strings the segment relation is readable from the bytes:
/// `Same` is byte equality and `Ancestor` is a strict component prefix.
pub proof fn lemma_canonical_rel(a: Seq<u8>, b: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(a, false),
        super::path::spec_is_canonical_abs(b, false),
    ensures
        (spec_rel(super::path::segs(a), super::path::segs(b)) is Same) <==> a == b,
        (spec_rel(super::path::segs(a), super::path::segs(b)) is Ancestor) <==> (
        spec_component_prefix(a, b) && a != b),
        (spec_rel(super::path::segs(a), super::path::segs(b)) is Descendant) <==> (
        spec_component_prefix(b, a) && a != b),
{
    super::path::lemma_canonical_render(a, false);
    super::path::lemma_canonical_render(b, false);
    lemma_component_prefix_iff_segs(a, b);
    lemma_component_prefix_iff_segs(b, a);
    lemma_rel_cases(super::path::segs(a), super::path::segs(b));
}

// ===========================================================================
// Rebasing (vocabulary for migrations)
// ===========================================================================

/// Moves `x` from under `from` to under `to`: `to ++ x[|from|..]`, or `None`
/// when `from` is not a prefix of `x`.
pub open spec fn spec_rebase<A>(x: Seq<A>, from: Seq<A>, to: Seq<A>) -> Option<Seq<A>> {
    if is_prefix_of(from, x) {
        Some(to + x.subrange(from.len() as int, x.len() as int))
    } else {
        None
    }
}

/// Rebasing lands under `to`, rebasing back restores the original, and
/// rebasing onto the same base is the identity.
pub proof fn lemma_rebase_round_trip<A>(x: Seq<A>, from: Seq<A>, to: Seq<A>)
    requires
        is_prefix_of(from, x),
    ensures
        is_prefix_of(to, spec_rebase(x, from, to)->Some_0),
        spec_rebase(spec_rebase(x, from, to)->Some_0, to, from) == Some(x),
        spec_rebase(x, from, from) == Some(x),
{
    let r = to + x.subrange(from.len() as int, x.len() as int);
    assert(r.subrange(0, to.len() as int) =~= to);
    assert(r.subrange(to.len() as int, r.len() as int) =~= x.subrange(
        from.len() as int,
        x.len() as int,
    ));
    assert(from + x.subrange(from.len() as int, x.len() as int) =~= x);
}

/// Rebasing with fixed bases is injective.
pub proof fn lemma_rebase_injective<A>(x: Seq<A>, y: Seq<A>, from: Seq<A>, to: Seq<A>)
    requires
        is_prefix_of(from, x),
        is_prefix_of(from, y),
        spec_rebase(x, from, to) == spec_rebase(y, from, to),
    ensures
        x == y,
{
    lemma_rebase_round_trip(x, from, to);
    lemma_rebase_round_trip(y, from, to);
}

/// Rebasing preserves the relationship between any two paths under `from`.
pub proof fn lemma_rebase_preserves_rel<A>(x: Seq<A>, y: Seq<A>, from: Seq<A>, to: Seq<A>)
    requires
        is_prefix_of(from, x),
        is_prefix_of(from, y),
    ensures
        spec_rel(spec_rebase(x, from, to)->Some_0, spec_rebase(y, from, to)->Some_0) == spec_rel(
            x,
            y,
        ),
{
    let n = from.len() as int;
    let tx = x.subrange(n, x.len() as int);
    let ty = y.subrange(n, y.len() as int);
    let rx = to + tx;
    let ry = to + ty;
    assert(from + tx =~= x);
    assert(from + ty =~= y);
    lemma_rel_cases(x, y);
    lemma_rel_cases(rx, ry);
    // Both pairs share a common prefix, so each relation reduces to the
    // relation between the tails.
    assert(is_prefix_of(x, y) <==> is_prefix_of(rx, ry)) by {
        if is_prefix_of(x, y) {
            assert(ry.subrange(0, rx.len() as int) =~= to + y.subrange(n, x.len() as int));
            assert(y.subrange(n, x.len() as int) =~= y.subrange(0, x.len() as int).subrange(
                n,
                x.len() as int,
            ));
        }
        if is_prefix_of(rx, ry) {
            assert(y.subrange(0, x.len() as int) =~= from + ry.subrange(
                to.len() as int,
                rx.len() as int,
            ));
            assert(ry.subrange(to.len() as int, rx.len() as int) =~= ry.subrange(
                0,
                rx.len() as int,
            ).subrange(to.len() as int, rx.len() as int));
        }
    }
    assert(is_prefix_of(y, x) <==> is_prefix_of(ry, rx)) by {
        if is_prefix_of(y, x) {
            assert(rx.subrange(0, ry.len() as int) =~= to + x.subrange(n, y.len() as int));
            assert(x.subrange(n, y.len() as int) =~= x.subrange(0, y.len() as int).subrange(
                n,
                y.len() as int,
            ));
        }
        if is_prefix_of(ry, rx) {
            assert(x.subrange(0, y.len() as int) =~= from + rx.subrange(
                to.len() as int,
                ry.len() as int,
            ));
            assert(rx.subrange(to.len() as int, ry.len() as int) =~= rx.subrange(
                0,
                ry.len() as int,
            ).subrange(to.len() as int, ry.len() as int));
        }
    }
    assert(x == y <==> rx == ry) by {
        if rx == ry {
            assert(tx =~= rx.subrange(to.len() as int, rx.len() as int));
            assert(ty =~= ry.subrange(to.len() as int, ry.len() as int));
        }
    }
}

/// Rebasing a canonical path between canonical bases gives a canonical
/// path (so normalizing the result again is a no-op).
pub proof fn lemma_rebase_canonical(x: Seq<Seq<u8>>, from: Seq<Seq<u8>>, to: Seq<Seq<u8>>)
    requires
        super::path::all_normal(x),
        super::path::all_normal(to),
        is_prefix_of(from, x),
    ensures
        super::path::all_normal(spec_rebase(x, from, to)->Some_0),
        super::path::spec_is_canonical_abs(
            super::path::render(spec_rebase(x, from, to)->Some_0),
            false,
        ),
        super::path::segs(super::path::render(spec_rebase(x, from, to)->Some_0)) == spec_rebase(
            x,
            from,
            to,
        )->Some_0,
{
    let r = spec_rebase(x, from, to)->Some_0;
    let n = from.len() as int;
    assert forall|i: int| 0 <= i < r.len() implies super::path::is_normal_seg(#[trigger] r[i]) by {
        if i < to.len() {
            assert(r[i] == to[i]);
        } else {
            assert(r[i] == x[i - to.len() + n]);
        }
    }
    super::path::lemma_render_canonical(r);
}

// ===========================================================================
// Home-relative display (vocabulary for `trop list`)
// ===========================================================================

/// How `trop list` shows a path relative to the home directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeDisplay {
    /// The path is the home directory: show `~`.
    Home,
    /// The path lies strictly below the home directory: show `~/` followed
    /// by the path's components from `rest_start` on.
    Below {
        /// Index of the first component after the home directory's.
        rest_start: usize,
    },
    /// The path is not under the home directory (or there is no usable home
    /// directory): show it in full.
    Elsewhere,
}

/// `home` (non-empty) is a component prefix of `path`.
pub open spec fn under_home<A>(path: Seq<A>, home: Seq<A>) -> bool {
    home.len() > 0 && is_prefix_of(home, path)
}

/// Decides how to show `path` given `home`, component-wise: the components
/// are compared as tokens, so `/home/al` is never a prefix of
/// `/home/alice/x`. An empty `home` never shortens anything.
pub fn home_display(path: &[PathToken], home: &[PathToken]) -> (r: HomeDisplay)
    ensures
        r == HomeDisplay::Home <==> under_home(tok_views(path@), tok_views(home@)) && path@.len()
            == home@.len(),
        r matches HomeDisplay::Below { rest_start } ==> rest_start == home@.len() && rest_start
            < path@.len() && under_home(tok_views(path@), tok_views(home@)),
        r == HomeDisplay::Elsewhere <==> !under_home(tok_views(path@), tok_views(home@)),
{
    let n = home.len();
    if n == 0 || n > path.len() {
        return HomeDisplay::Elsewhere;
    }
    let mut k: usize = 0;
    while k < n
        invariant
            n == home@.len() <= path@.len(),
            0 <= k <= n,
            forall|j: int| 0 <= j < k ==> tok_view(#[trigger] path@[j]) == tok_view(home@[j]),
        decreases n - k,
    {
        if !token_eq(&path[k], &home[k]) {
            proof {
                assert(tok_views(path@).subrange(0, n as int)[k as int] == tok_view(path@[k as int]));
                assert(tok_views(home@)[k as int] == tok_view(home@[k as int]));
            }
            return HomeDisplay::Elsewhere;
        }
        k += 1;
    }
    proof {
        assert(tok_views(path@).subrange(0, n as int) =~= tok_views(home@));
    }
    if n == path.len() {
        HomeDisplay::Home
    } else {
        HomeDisplay::Below { rest_start: n }
    }
}

/// The shortened display is a faithful name: home's components followed by
/// the rest give back the path (so expanding `~` restores it), and two paths
/// under the same home with the same rest are the same path.
pub proof fn lemma_home_display_round_trip<A>(path: Seq<A>, other: Seq<A>, home: Seq<A>)
    requires
        under_home(path, home),
        under_home(other, home),
    ensures
        home + path.subrange(home.len() as int, path.len() as int) == path,
        path.subrange(home.len() as int, path.len() as int) == other.subrange(
            home.len() as int,
            other.len() as int,
        ) ==> path == other,
{
    assert(home + path.subrange(home.len() as int, path.len() as int) =~= path);
    assert(home + other.subrange(home.len() as int, other.len() as int) =~= other);
}

} // verus!
