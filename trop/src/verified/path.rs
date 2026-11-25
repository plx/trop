//! Verified kernel: lexical path normalization (tilde handling, component
//! resolution, canonical absolute form).
//!
//! Two models of a path live here:
//!
//! - **Token level** (all platforms): a path is the sequence of
//!   `std::path::Component` kinds (`CompKind`). `resolve_component_kinds`
//!   resolves `.` and `..` with a stack (`spec_resolve`).
//! - **Byte level** (Unix): a path is its `OsStr` byte string. `pieces`
//!   splits it at every `/`, `segs` drops the empty and `.` pieces (the
//!   model of `Path::components` without the root), `resolve_segs` resolves
//!   `..`, and `render` spells a segment list as `/a/b/c`.
//!   `normalize_unix` computes `spec_normalize`; its outputs are exactly
//!   the strings accepted by `is_canonical_abs_path`
//!   (`spec_is_canonical_abs`), it is idempotent, and on canonical strings
//!   byte equality coincides with segment equality.
//!
//! Production glue: `crate::path::normalize` (`resolve_components`,
//! `expand_tilde`, `normalize`), `crate::path::types` (`NormalizedPath::new`),
//! `crate::path::canonicalize` (`canonicalize_existing`), and
//! `crate::path::resolver` (`PathResolver::resolve`).

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// Token level: `.` / `..` resolution over component kinds
// ===========================================================================

/// The kind of one `std::path::Component`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompKind {
    /// A Windows path prefix (`C:`, `\\server\share`, ...).
    Prefix,
    /// The root directory (`/`, or `\` on Windows).
    Root,
    /// `.`
    Cur,
    /// `..`
    Parent,
    /// Any other (named) component.
    Normal,
}

/// Whether `k` anchors a path (a prefix or the root).
pub open spec fn is_anchor(k: CompKind) -> bool {
    k is Prefix || k is Root
}

/// Stack resolution of `.` and `..` over component kinds, as the indices of
/// the components that survive (in order), or `None` when some `..` has no
/// named component to cancel (it would escape the root, or climb above the
/// start of a relative path).
///
/// Anchors and named components are pushed, `.` is dropped, and `..` pops
/// the top of the stack only when that top is a named component.
pub open spec fn spec_resolve(ks: Seq<CompKind>) -> Option<Seq<int>>
    decreases ks.len(),
{
    if ks.len() == 0 {
        Some(Seq::empty())
    } else {
        match spec_resolve(ks.drop_last()) {
            None => None,
            Some(st) => match ks.last() {
                CompKind::Cur => Some(st),
                CompKind::Parent => if st.len() > 0 && ks[st.last()] is Normal {
                    Some(st.drop_last())
                } else {
                    None
                },
                _ => Some(st.push(ks.len() - 1)),
            },
        }
    }
}

/// The indices `0, 1, ..., n - 1`.
pub open spec fn index_range(n: int) -> Seq<int> {
    Seq::new(n as nat, |i: int| i)
}

/// `idx` is a strictly increasing list of indices into a sequence of length
/// `n`.
pub open spec fn increasing_indices(idx: Seq<int>, n: int) -> bool {
    (forall|j: int| 0 <= j < idx.len() ==> 0 <= #[trigger] idx[j] < n) && (forall|j: int, l: int|
        0 <= j < l < idx.len() ==> idx[j] < idx[l])
}

/// The kinds selected by `idx`.
pub open spec fn select_kinds(ks: Seq<CompKind>, idx: Seq<int>) -> Seq<CompKind> {
    idx.map_values(|i: int| ks[i])
}

/// The error of `resolve_component_kinds`: some `..` has nothing to cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapesRoot {
    /// A `..` reached the root (or the start of a relative path).
    TooManyParents,
}

/// `spec_resolve` fails on every extension of a failing prefix.
pub proof fn lemma_resolve_none_extends(ks: Seq<CompKind>, n: int)
    requires
        0 <= n <= ks.len(),
        spec_resolve(ks.subrange(0, n)) is None,
    ensures
        spec_resolve(ks) is None,
    decreases ks.len() - n,
{
    if n < ks.len() {
        assert(ks.subrange(0, n + 1).drop_last() =~= ks.subrange(0, n));
        lemma_resolve_none_extends(ks, n + 1);
    } else {
        assert(ks.subrange(0, n) =~= ks);
    }
}

/// Facts about every successful resolution: the kept indices are strictly
/// increasing and in bounds, select only anchors and named components, keep
/// every anchor, and are no more numerous than the input.
pub proof fn lemma_resolve_well_formed(ks: Seq<CompKind>)
    requires
        spec_resolve(ks) is Some,
    ensures
        ({
            let st = spec_resolve(ks)->Some_0;
            &&& increasing_indices(st, ks.len() as int)
            &&& forall|j: int| 0 <= j < st.len() ==> #[trigger] ks[st[j]] is Normal || is_anchor(
                ks[st[j]],
            )
            &&& forall|i: int| 0 <= i < ks.len() && is_anchor(#[trigger] ks[i]) ==> st.contains(i)
            &&& st.len() <= ks.len()
        }),
    decreases ks.len(),
{
    if ks.len() > 0 {
        let prev = ks.drop_last();
        lemma_resolve_well_formed(prev);
        let pst = spec_resolve(prev)->Some_0;
        let st = spec_resolve(ks)->Some_0;
        let last = ks.len() - 1;
        assert(forall|i: int| 0 <= i < prev.len() ==> prev[i] == ks[i]);
        match ks.last() {
            CompKind::Cur => {},
            CompKind::Parent => {
                assert(st =~= pst.drop_last());
                assert forall|i: int| 0 <= i < ks.len() && is_anchor(#[trigger] ks[i]) implies
                    st.contains(i) by {
                    assert(i < last);
                    assert(pst.contains(i));
                    let j = choose|j: int| 0 <= j < pst.len() && pst[j] == i;
                    if j == pst.len() - 1 {
                        assert(ks[pst.last()] is Normal);
                    }
                    assert(st[j] == i);
                }
            },
            _ => {
                assert(st =~= pst.push(last));
                assert forall|i: int| 0 <= i < ks.len() && is_anchor(#[trigger] ks[i]) implies
                    st.contains(i) by {
                    if i < last {
                        assert(pst.contains(i));
                        let j = choose|j: int| 0 <= j < pst.len() && pst[j] == i;
                        assert(st[j] == i);
                    } else {
                        assert(st[st.len() - 1] == i);
                    }
                }
            },
        }
    }
}

/// Resolving a sequence with no `.` or `..` keeps every component.
pub proof fn lemma_resolve_identity(ks: Seq<CompKind>)
    requires
        forall|i: int| 0 <= i < ks.len() ==> !(#[trigger] ks[i] is Cur) && !(ks[i] is Parent),
    ensures
        spec_resolve(ks) == Some(index_range(ks.len() as int)),
    decreases ks.len(),
{
    if ks.len() > 0 {
        lemma_resolve_identity(ks.drop_last());
        assert(index_range(ks.len() as int) =~= index_range(ks.len() - 1).push(ks.len() - 1));
    } else {
        assert(index_range(0) =~= Seq::<int>::empty());
    }
}

/// Resolution is idempotent: resolving the surviving components again keeps
/// all of them.
pub proof fn lemma_resolve_idempotent(ks: Seq<CompKind>)
    requires
        spec_resolve(ks) is Some,
    ensures
        ({
            let st = spec_resolve(ks)->Some_0;
            spec_resolve(select_kinds(ks, st)) == Some(index_range(st.len() as int))
        }),
{
    lemma_resolve_well_formed(ks);
    let st = spec_resolve(ks)->Some_0;
    let sel = select_kinds(ks, st);
    assert forall|i: int| 0 <= i < sel.len() implies !(#[trigger] sel[i] is Cur) && !(
    sel[i] is Parent) by {
        assert(sel[i] == ks[st[i]]);
    }
    lemma_resolve_identity(sel);
}

/// The kept indices as integers.
pub open spec fn usize_ints(v: Seq<usize>) -> Seq<int> {
    v.map_values(|i: usize| i as int)
}

/// Resolves `.` and `..` over component kinds; returns the indices of the
/// surviving components.
///
/// `Ok(kept)` exactly when `spec_resolve` succeeds, and then `kept` is its
/// result (see `lemma_resolve_well_formed` and `lemma_resolve_idempotent`
/// for its properties).
pub fn resolve_component_kinds(kinds: &[CompKind]) -> (r: Result<Vec<usize>, EscapesRoot>)
    ensures
        r matches Ok(kept) ==> spec_resolve(kinds@) == Some(usize_ints(kept@)),
        r is Err <==> spec_resolve(kinds@) is None,
{
    let mut stack: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < kinds.len()
        invariant
            i <= kinds.len(),
            spec_resolve(kinds@.subrange(0, i as int)) == Some(usize_ints(stack@)),
            forall|j: int| 0 <= j < stack.len() ==> stack[j] < i,
        decreases kinds.len() - i,
    {
        proof {
            assert(kinds@.subrange(0, i + 1).drop_last() =~= kinds@.subrange(0, i as int));
        }
        match kinds[i] {
            CompKind::Cur => {},
            CompKind::Parent => {
                let n = stack.len();
                if n > 0 && matches!(kinds[stack[n - 1]], CompKind::Normal) {
                    let ghost before = stack@;
                    stack.pop();
                    proof {
                        assert(usize_ints(stack@) =~= usize_ints(before).drop_last());
                    };
                } else {
                    proof {
                        lemma_resolve_none_extends(kinds@, i + 1);
                    }
                    return Err(EscapesRoot::TooManyParents);
                }
            },
            _ => {
                let ghost before = stack@;
                stack.push(i);
                proof {
                    assert(usize_ints(stack@) =~= usize_ints(before).push(i as int));
                };
            },
        }
        i += 1;
    }
    proof {
        assert(kinds@.subrange(0, kinds.len() as int) =~= kinds@);
    }
    Ok(stack)
}

// ===========================================================================
// Byte level (Unix): pieces, segments, resolution, rendering
// ===========================================================================

/// `s` starts with `/` (Unix `Path::is_absolute`).
pub open spec fn is_abs(s: Seq<u8>) -> bool {
    s.len() > 0 && s[0] == b'/'
}

/// No byte of `x` equals `c`.
pub open spec fn no_byte(x: Seq<u8>, c: u8) -> bool {
    forall|j: int| 0 <= j < x.len() ==> #[trigger] x[j] != c
}

/// `s` split at every `/` (like `str::split('/')`): `k` slashes give
/// `k + 1` pieces, some possibly empty.
pub open spec fn pieces(s: Seq<u8>) -> Seq<Seq<u8>>
    decreases s.len(),
{
    if s.len() == 0 {
        seq![Seq::<u8>::empty()]
    } else {
        let p = pieces(s.drop_last());
        if s.last() == b'/' {
            p.push(Seq::<u8>::empty())
        } else {
            p.update(p.len() - 1, p.last().push(s.last()))
        }
    }
}

/// An empty piece (from `//` or a leading/trailing `/`) or `.`.
pub open spec fn is_skipped(x: Seq<u8>) -> bool {
    x.len() == 0 || x == seq![b'.']
}

/// The piece `..`.
pub open spec fn is_parent_seg(x: Seq<u8>) -> bool {
    x == seq![b'.', b'.']
}

/// A named path segment: non-empty, not `.` or `..`, and free of `/`.
pub open spec fn is_normal_seg(x: Seq<u8>) -> bool {
    !is_skipped(x) && !is_parent_seg(x) && no_byte(x, b'/')
}

/// Every element of `st` is a named segment.
pub open spec fn all_normal(st: Seq<Seq<u8>>) -> bool {
    forall|i: int| 0 <= i < st.len() ==> is_normal_seg(#[trigger] st[i])
}

/// No element of `ps` contains `/`.
pub open spec fn all_slash_free(ps: Seq<Seq<u8>>) -> bool {
    forall|i: int| 0 <= i < ps.len() ==> no_byte(#[trigger] ps[i], b'/')
}

/// `ps` without its empty and `.` pieces, in order.
pub open spec fn keep(ps: Seq<Seq<u8>>) -> Seq<Seq<u8>>
    decreases ps.len(),
{
    if ps.len() == 0 {
        ps
    } else if is_skipped(ps.last()) {
        keep(ps.drop_last())
    } else {
        keep(ps.drop_last()).push(ps.last())
    }
}

/// The segments of `s`: its pieces without empty and `.` pieces (`..`
/// segments are kept). For an absolute `s` (starting with `/`) this models
/// Unix `Path::components()` minus the leading root. For a relative `s`,
/// `std` would also yield a leading `CurDir` for `./...`, which `segs`
/// drops; `normalize_unix` only takes the segments of the absolute joined
/// path, so the production glue relies on the model for absolute strings
/// only.
pub open spec fn segs(s: Seq<u8>) -> Seq<Seq<u8>> {
    keep(pieces(s))
}

/// One stack step: `..` pops (and fails on an empty stack), empty and `.`
/// pieces are ignored, anything else is pushed.
pub open spec fn step(st: Seq<Seq<u8>>, x: Seq<u8>) -> Option<Seq<Seq<u8>>> {
    if is_parent_seg(x) {
        if st.len() == 0 {
            None
        } else {
            Some(st.drop_last())
        }
    } else if is_skipped(x) {
        Some(st)
    } else {
        Some(st.push(x))
    }
}

/// Applies `step` for every element of `ps`, starting from the stack `st`.
pub open spec fn resolve_from(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>) -> Option<Seq<Seq<u8>>>
    decreases ps.len(),
{
    if ps.len() == 0 {
        Some(st)
    } else {
        match resolve_from(st, ps.drop_last()) {
            None => None,
            Some(t) => step(t, ps.last()),
        }
    }
}

/// Lexical `..` resolution of an absolute segment list: `None` when a `..`
/// would climb above the root.
pub open spec fn resolve_segs(ss: Seq<Seq<u8>>) -> Option<Seq<Seq<u8>>> {
    resolve_from(Seq::empty(), ss)
}

/// `/st[0]/st[1]/.../st[n-1]` (empty for an empty list).
pub open spec fn render_tail(st: Seq<Seq<u8>>) -> Seq<u8>
    decreases st.len(),
{
    if st.len() == 0 {
        Seq::empty()
    } else {
        render_tail(st.drop_last()) + seq![b'/'] + st.last()
    }
}

/// The absolute spelling of a segment list: `/` for the empty list,
/// otherwise `/st[0]/.../st[n-1]` (no trailing `/`).
pub open spec fn render(st: Seq<Seq<u8>>) -> Seq<u8> {
    if st.len() == 0 {
        seq![b'/']
    } else {
        render_tail(st)
    }
}

/// The path that normalization resolves: `p` itself when absolute,
/// otherwise `cwd/p`.
pub open spec fn joined(p: Seq<u8>, cwd: Seq<u8>) -> Seq<u8> {
    if is_abs(p) {
        p
    } else {
        cwd + seq![b'/'] + p
    }
}

/// Why byte-level normalization failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormError {
    /// A `..` would climb above the root.
    EscapesRoot,
    /// The path is relative and the base directory is not absolute.
    BaseNotAbsolute,
}

/// Unix lexical normalization of `p` against the working directory `cwd`:
/// make it absolute, take its segments, resolve `..`, and render the result.
pub open spec fn spec_normalize(p: Seq<u8>, cwd: Seq<u8>) -> Result<Seq<u8>, NormError> {
    if !is_abs(p) && !is_abs(cwd) {
        Err(NormError::BaseNotAbsolute)
    } else {
        match resolve_segs(segs(joined(p, cwd))) {
            Some(st) => Ok(render(st)),
            None => Err(NormError::EscapesRoot),
        }
    }
}

/// A segment allowed in a canonical path: named, and free of `\` when
/// `backslash_forbidden`.
pub open spec fn seg_ok(x: Seq<u8>, backslash_forbidden: bool) -> bool {
    is_normal_seg(x) && (!backslash_forbidden || no_byte(x, b'\\'))
}

/// The canonical absolute spelling: `/`, or `/` followed by `/`-separated
/// segments that are all non-empty, not `.` or `..` (and contain no `\`
/// when `backslash_forbidden`). No empty segment means no `//` and no
/// trailing `/`.
pub open spec fn spec_is_canonical_abs(s: Seq<u8>, backslash_forbidden: bool) -> bool {
    s == seq![b'/'] || (s.len() > 1 && s[0] == b'/' && forall|i: int|
        1 <= i < pieces(s).len() ==> seg_ok(#[trigger] pieces(s)[i], backslash_forbidden))
}

// ---------------------------------------------------------------------------
// Lemmas: pieces
// ---------------------------------------------------------------------------

/// There is always at least one piece.
pub proof fn lemma_pieces_len(s: Seq<u8>)
    ensures
        pieces(s).len() >= 1,
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_pieces_len(s.drop_last());
    }
}

/// Pieces never contain `/`.
pub proof fn lemma_pieces_slash_free(s: Seq<u8>)
    ensures
        all_slash_free(pieces(s)),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_pieces_slash_free(s.drop_last());
        lemma_pieces_len(s.drop_last());
        let p = pieces(s.drop_last());
        if s.last() != b'/' {
            assert forall|i: int| 0 <= i < pieces(s).len() implies no_byte(
                #[trigger] pieces(s)[i],
                b'/',
            ) by {
                if i == p.len() - 1 {
                    let x = p.last().push(s.last());
                    assert(pieces(s)[i] == x);
                    assert forall|j: int| 0 <= j < x.len() implies #[trigger] x[j] != b'/' by {
                        if j < p.last().len() {
                            assert(x[j] == p.last()[j]);
                        }
                    }
                }
            }
        }
    }
}

/// A `/`-free string is a single piece.
pub proof fn lemma_pieces_single(x: Seq<u8>)
    requires
        no_byte(x, b'/'),
    ensures
        pieces(x) == seq![x],
    decreases x.len(),
{
    if x.len() == 0 {
        assert(x =~= Seq::<u8>::empty());
    } else {
        let y = x.drop_last();
        assert(no_byte(y, b'/')) by {
            assert forall|j: int| 0 <= j < y.len() implies #[trigger] y[j] != b'/' by {
                assert(y[j] == x[j]);
            }
        }
        lemma_pieces_single(y);
        assert(y.push(x.last()) =~= x);
        assert(seq![y].update(0, y.push(x.last())) =~= seq![x]);
    }
}

/// Splitting distributes over a `/`: `pieces(a/b) == pieces(a) ++ pieces(b)`.
pub proof fn lemma_pieces_concat(a: Seq<u8>, b: Seq<u8>)
    ensures
        pieces(a + seq![b'/'] + b) == pieces(a) + pieces(b),
    decreases b.len(),
{
    let x = a + seq![b'/'] + b;
    if b.len() == 0 {
        assert(x.drop_last() =~= a);
        assert(pieces(b) =~= seq![Seq::<u8>::empty()]);
        assert(pieces(x) =~= pieces(a) + pieces(b));
    } else {
        let b1 = b.drop_last();
        let c = b.last();
        assert(x.drop_last() =~= a + seq![b'/'] + b1);
        assert(x.last() == c);
        lemma_pieces_concat(a, b1);
        lemma_pieces_len(b1);
        let pa = pieces(a);
        let pb = pieces(b1);
        if c == b'/' {
            assert(pieces(x) =~= pa + pieces(b));
        } else {
            assert((pa + pb).last() == pb.last());
            assert(pieces(x) =~= pa + pieces(b));
        }
    }
}

/// Rendering then splitting gives back the segments (after a leading empty
/// piece).
pub proof fn lemma_render_tail_pieces(st: Seq<Seq<u8>>)
    requires
        all_slash_free(st),
    ensures
        pieces(render_tail(st)) == seq![Seq::<u8>::empty()] + st,
    decreases st.len(),
{
    if st.len() == 0 {
        assert(render_tail(st) =~= Seq::<u8>::empty());
        assert(seq![Seq::<u8>::empty()] + st =~= seq![Seq::<u8>::empty()]);
    } else {
        let st1 = st.drop_last();
        assert(all_slash_free(st1)) by {
            assert forall|i: int| 0 <= i < st1.len() implies no_byte(#[trigger] st1[i], b'/') by {
                assert(st1[i] == st[i]);
            }
        }
        lemma_render_tail_pieces(st1);
        assert(no_byte(st[st.len() - 1], b'/'));
        lemma_pieces_single(st.last());
        lemma_pieces_concat(render_tail(st1), st.last());
        assert(seq![Seq::<u8>::empty()] + st1 + seq![st.last()] =~= seq![Seq::<u8>::empty()]
            + st);
    }
}

/// `render_tail` of a list whose last element grew by one byte.
proof fn lemma_render_tail_extend_last(q: Seq<Seq<u8>>, c: u8)
    requires
        q.len() > 0,
    ensures
        render_tail(q.update(q.len() - 1, q.last().push(c))) == render_tail(q).push(c),
{
    let q2 = q.update(q.len() - 1, q.last().push(c));
    assert(q2.drop_last() =~= q.drop_last());
    assert(q2.last() == q.last().push(c));
    assert(render_tail(q2) =~= render_tail(q).push(c));
}

/// Splitting then joining is the identity:
/// `s == pieces(s)[0] ++ render_tail(pieces(s)[1..])`.
pub proof fn lemma_split_join(s: Seq<u8>)
    ensures
        s == pieces(s)[0] + render_tail(pieces(s).drop_first()),
    decreases s.len(),
{
    lemma_pieces_len(s);
    if s.len() == 0 {
        assert(pieces(s).drop_first() =~= Seq::<Seq<u8>>::empty());
        assert(s =~= pieces(s)[0] + render_tail(pieces(s).drop_first()));
    } else {
        let s1 = s.drop_last();
        let c = s.last();
        lemma_split_join(s1);
        lemma_pieces_len(s1);
        let p = pieces(s1);
        if c == b'/' {
            let q = pieces(s);
            assert(q =~= p.push(Seq::<u8>::empty()));
            let qd = q.drop_first();
            assert(qd =~= p.drop_first().push(Seq::<u8>::empty()));
            assert(qd.drop_last() =~= p.drop_first());
            assert(qd.last() == Seq::<u8>::empty());
            assert(render_tail(qd) =~= render_tail(p.drop_first()) + seq![b'/']
                + Seq::<u8>::empty());
            assert(s =~= s1.push(c));
            assert(s =~= q[0] + render_tail(q.drop_first()));
        } else {
            let q = pieces(s);
            if p.len() == 1 {
                assert(q =~= seq![p[0].push(c)]);
                assert(q.drop_first() =~= Seq::<Seq<u8>>::empty());
                assert(p.drop_first() =~= Seq::<Seq<u8>>::empty());
                assert(s =~= s1.push(c));
                assert(s =~= q[0] + render_tail(q.drop_first()));
            } else {
                let pd = p.drop_first();
                assert(q.drop_first() =~= pd.update(pd.len() - 1, pd.last().push(c)));
                lemma_render_tail_extend_last(pd, c);
                assert(q[0] == p[0]);
                assert(s =~= s1.push(c));
                assert(s =~= q[0] + render_tail(q.drop_first()));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lemmas: keep, resolution
// ---------------------------------------------------------------------------

/// `keep` distributes over concatenation.
pub proof fn lemma_keep_concat(a: Seq<Seq<u8>>, b: Seq<Seq<u8>>)
    ensures
        keep(a + b) == keep(a) + keep(b),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
        assert(keep(a) + keep(b) =~= keep(a));
    } else {
        lemma_keep_concat(a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        if is_skipped(b.last()) {
        } else {
            assert(keep(a) + keep(b) =~= (keep(a) + keep(b.drop_last())).push(b.last()));
        }
    }
}

/// `keep` is the identity on lists without empty or `.` pieces.
pub proof fn lemma_keep_identity(ps: Seq<Seq<u8>>)
    requires
        forall|i: int| 0 <= i < ps.len() ==> !is_skipped(#[trigger] ps[i]),
    ensures
        keep(ps) == ps,
    decreases ps.len(),
{
    if ps.len() > 0 {
        assert(!is_skipped(ps[ps.len() - 1]));
        lemma_keep_identity(ps.drop_last());
        assert(ps.drop_last().push(ps.last()) =~= ps);
    }
}

/// `keep` keeps no empty or `.` piece, and preserves `/`-freedom.
pub proof fn lemma_keep_props(ps: Seq<Seq<u8>>)
    ensures
        forall|i: int| 0 <= i < keep(ps).len() ==> !is_skipped(#[trigger] keep(ps)[i]),
        all_slash_free(ps) ==> all_slash_free(keep(ps)),
    decreases ps.len(),
{
    if ps.len() > 0 {
        let p1 = ps.drop_last();
        lemma_keep_props(p1);
        if all_slash_free(ps) {
            assert(all_slash_free(p1)) by {
                assert forall|i: int| 0 <= i < p1.len() implies no_byte(#[trigger] p1[i], b'/') by {
                    assert(p1[i] == ps[i]);
                }
            }
            assert(no_byte(ps[ps.len() - 1], b'/'));
        }
    }
}

/// The segments of the empty string and of the bare root are empty.
pub proof fn lemma_segs_trivial()
    ensures
        segs(Seq::<u8>::empty()) == Seq::<Seq<u8>>::empty(),
        segs(seq![b'/']) == Seq::<Seq<u8>>::empty(),
        keep(seq![Seq::<u8>::empty()]) == Seq::<Seq<u8>>::empty(),
        pieces(Seq::<u8>::empty()) == seq![Seq::<u8>::empty()],
        pieces(seq![b'/']) == seq![Seq::<u8>::empty(), Seq::<u8>::empty()],
{
    let e = Seq::<u8>::empty();
    let ee = Seq::<Seq<u8>>::empty();
    assert(pieces(e) == seq![e]);
    assert(seq![e].drop_last() =~= ee);
    assert(seq![e].last() == e);
    assert(keep(ee) == ee);
    assert(keep(seq![e]) == keep(ee));
    let root = seq![b'/'];
    assert(root.drop_last() =~= e);
    assert(root.last() == b'/');
    assert(pieces(root) == pieces(e).push(e));
    assert(seq![e].push(e) =~= seq![e, e]);
    assert(seq![e, e].drop_last() =~= seq![e]);
    assert(seq![e, e].last() == e);
    assert(keep(seq![e, e]) == keep(seq![e]));
}

/// Segments distribute over a `/`: `segs(a/b) == segs(a) ++ segs(b)`.
pub proof fn lemma_segs_concat(a: Seq<u8>, b: Seq<u8>)
    ensures
        segs(a + seq![b'/'] + b) == segs(a) + segs(b),
{
    lemma_pieces_concat(a, b);
    lemma_keep_concat(pieces(a), pieces(b));
}

/// Empty and `.` pieces do not affect resolution.
pub proof fn lemma_resolve_keep(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>)
    ensures
        resolve_from(st, keep(ps)) == resolve_from(st, ps),
    decreases ps.len(),
{
    if ps.len() > 0 {
        lemma_resolve_keep(st, ps.drop_last());
        if !is_skipped(ps.last()) {
            assert(keep(ps).drop_last() =~= keep(ps.drop_last()));
        } else {
            assert(!is_parent_seg(ps.last()));
        }
    }
}

/// Resolving a concatenation resolves the first part, then the second.
pub proof fn lemma_resolve_concat(st: Seq<Seq<u8>>, a: Seq<Seq<u8>>, b: Seq<Seq<u8>>)
    ensures
        resolve_from(st, a + b) == (match resolve_from(st, a) {
            None => None,
            Some(t) => resolve_from(t, b),
        }),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        lemma_resolve_concat(st, a, b.drop_last());
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
    }
}

/// Resolution of `/`-free pieces from a stack of named segments yields named
/// segments only.
pub proof fn lemma_resolve_normal(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>)
    requires
        all_normal(st),
        all_slash_free(ps),
        resolve_from(st, ps) is Some,
    ensures
        all_normal(resolve_from(st, ps)->Some_0),
    decreases ps.len(),
{
    if ps.len() > 0 {
        let p1 = ps.drop_last();
        assert(all_slash_free(p1)) by {
            assert forall|i: int| 0 <= i < p1.len() implies no_byte(#[trigger] p1[i], b'/') by {
                assert(p1[i] == ps[i]);
            }
        }
        lemma_resolve_normal(st, p1);
        let t = resolve_from(st, p1)->Some_0;
        let r = resolve_from(st, ps)->Some_0;
        assert(no_byte(ps[ps.len() - 1], b'/'));
        assert forall|i: int| 0 <= i < r.len() implies is_normal_seg(#[trigger] r[i]) by {
            if i < t.len() {
                assert(r[i] == t[i]);
            }
        }
    }
}

/// Resolving named segments only pushes them.
pub proof fn lemma_resolve_named(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>)
    requires
        all_normal(ps),
    ensures
        resolve_from(st, ps) == Some(st + ps),
    decreases ps.len(),
{
    if ps.len() > 0 {
        let p1 = ps.drop_last();
        assert(all_normal(p1)) by {
            assert forall|i: int| 0 <= i < p1.len() implies is_normal_seg(#[trigger] p1[i]) by {
                assert(p1[i] == ps[i]);
            }
        }
        lemma_resolve_named(st, p1);
        assert(is_normal_seg(ps[ps.len() - 1]));
        assert((st + p1).push(ps.last()) =~= st + ps);
    } else {
        assert(st + ps =~= st);
    }
}

// ---------------------------------------------------------------------------
// Lemmas: canonical form
// ---------------------------------------------------------------------------

/// A non-empty `render_tail` starts with `/`.
pub proof fn lemma_render_tail_first(st: Seq<Seq<u8>>)
    requires
        st.len() > 0,
    ensures
        render_tail(st).len() > 0,
        render_tail(st)[0] == b'/',
    decreases st.len(),
{
    let st1 = st.drop_last();
    if st1.len() > 0 {
        lemma_render_tail_first(st1);
    } else {
        assert(render_tail(st1) =~= Seq::<u8>::empty());
    }
}

/// `render` of named segments is canonical, and its segments are exactly the
/// rendered list.
pub proof fn lemma_render_canonical(st: Seq<Seq<u8>>)
    requires
        all_normal(st),
    ensures
        spec_is_canonical_abs(render(st), false),
        segs(render(st)) == st,
        is_abs(render(st)),
{
    lemma_segs_trivial();
    if st.len() > 0 {
        let s = render(st);
        assert(all_slash_free(st)) by {
            assert forall|i: int| 0 <= i < st.len() implies no_byte(#[trigger] st[i], b'/') by {
                assert(is_normal_seg(st[i]));
            }
        }
        lemma_render_tail_pieces(st);
        let e = Seq::<u8>::empty();
        assert(pieces(s) == seq![e] + st);
        let st1 = st.drop_last();
        assert(is_normal_seg(st[st.len() - 1]));
        assert(s == render_tail(st1) + seq![b'/'] + st.last());
        if st1.len() == 0 {
            assert(render_tail(st1) =~= e);
            assert(s[0] == b'/');
        } else {
            lemma_render_tail_first(st1);
            assert(s[0] == render_tail(st1)[0]);
        }
        assert forall|i: int| 1 <= i < pieces(s).len() implies seg_ok(
            #[trigger] pieces(s)[i],
            false,
        ) by {
            assert(pieces(s)[i] == st[i - 1]);
        }
        lemma_keep_concat(seq![e], st);
        assert forall|i: int| 0 <= i < st.len() implies !is_skipped(#[trigger] st[i]) by {
            assert(is_normal_seg(st[i]));
        }
        lemma_keep_identity(st);
        assert(seq![e] + st =~= pieces(s));
        assert(Seq::<Seq<u8>>::empty() + st =~= st);
    } else {
        assert(render(st) =~= seq![b'/']);
    }
}

/// Every canonical string is the rendering of its segments, which are named.
pub proof fn lemma_canonical_render(s: Seq<u8>, backslash_forbidden: bool)
    requires
        spec_is_canonical_abs(s, backslash_forbidden),
    ensures
        all_normal(segs(s)),
        render(segs(s)) == s,
        is_abs(s),
{
    lemma_segs_trivial();
    if s == seq![b'/'] {
    } else {
        let e = Seq::<u8>::empty();
        let x = s.subrange(1, s.len() as int);
        assert(s =~= e + seq![b'/'] + x);
        lemma_pieces_concat(e, x);
        lemma_pieces_len(x);
        assert(pieces(e) =~= seq![e]);
        let t = pieces(x);
        assert(pieces(s) =~= seq![e] + t);
        assert forall|i: int| 0 <= i < t.len() implies is_normal_seg(#[trigger] t[i]) by {
            assert(t[i] == pieces(s)[i + 1]);
            assert(seg_ok(pieces(s)[i + 1], backslash_forbidden));
        }
        assert forall|i: int| 0 <= i < t.len() implies !is_skipped(#[trigger] t[i]) by {
            assert(is_normal_seg(t[i]));
        }
        lemma_keep_concat(seq![e], t);
        lemma_keep_identity(t);
        assert(segs(s) =~= t);
        lemma_split_join(s);
        assert(pieces(s).drop_first() =~= t);
        assert(pieces(s)[0] == e);
        assert(s =~= render_tail(t));
    }
}

// ---------------------------------------------------------------------------
// Theorems: normalization
// ---------------------------------------------------------------------------

/// Normalization succeeds with a canonical absolute string whose segments are
/// the resolved segments of the joined path.
pub proof fn lemma_normalize_canonical(p: Seq<u8>, cwd: Seq<u8>)
    requires
        spec_normalize(p, cwd) is Ok,
    ensures
        spec_is_canonical_abs(spec_normalize(p, cwd)->Ok_0, false),
        segs(spec_normalize(p, cwd)->Ok_0) == resolve_segs(segs(joined(p, cwd)))->Some_0,
{
    let j = joined(p, cwd);
    lemma_pieces_slash_free(j);
    lemma_keep_props(pieces(j));
    lemma_resolve_normal(Seq::empty(), segs(j));
    lemma_render_canonical(resolve_segs(segs(j))->Some_0);
}

/// Canonical strings are fixed points of normalization, whatever the working
/// directory.
pub proof fn lemma_normalize_fixpoint(s: Seq<u8>, cwd: Seq<u8>)
    requires
        spec_is_canonical_abs(s, false),
    ensures
        spec_normalize(s, cwd) == Ok::<Seq<u8>, NormError>(s),
{
    lemma_canonical_render(s, false);
    lemma_resolve_named(Seq::empty(), segs(s));
    assert(Seq::<Seq<u8>>::empty() + segs(s) =~= segs(s));
}

/// Characterization: a string is canonical exactly when normalization maps
/// it to itself.
pub proof fn lemma_canonical_iff_fixpoint(s: Seq<u8>, cwd: Seq<u8>)
    ensures
        spec_is_canonical_abs(s, false) <==> spec_normalize(s, cwd) == Ok::<Seq<u8>, NormError>(s),
{
    if spec_is_canonical_abs(s, false) {
        lemma_normalize_fixpoint(s, cwd);
    }
    if spec_normalize(s, cwd) == Ok::<Seq<u8>, NormError>(s) {
        lemma_normalize_canonical(s, cwd);
    }
}

/// Normalization is idempotent, even across different working directories.
pub proof fn lemma_normalize_idempotent(p: Seq<u8>, cwd1: Seq<u8>, cwd2: Seq<u8>)
    requires
        spec_normalize(p, cwd1) is Ok,
    ensures
        spec_normalize(spec_normalize(p, cwd1)->Ok_0, cwd2) == spec_normalize(p, cwd1),
{
    lemma_normalize_canonical(p, cwd1);
    lemma_normalize_fixpoint(spec_normalize(p, cwd1)->Ok_0, cwd2);
}

/// The working directory only matters for relative paths.
pub proof fn lemma_normalize_absolute_ignores_cwd(p: Seq<u8>, cwd1: Seq<u8>, cwd2: Seq<u8>)
    requires
        is_abs(p),
    ensures
        spec_normalize(p, cwd1) == spec_normalize(p, cwd2),
{
}

/// On canonical strings, byte equality and segment equality coincide (so
/// text keys and component-wise path equality agree).
pub proof fn lemma_canonical_injective(a: Seq<u8>, b: Seq<u8>, backslash_forbidden: bool)
    requires
        spec_is_canonical_abs(a, backslash_forbidden),
        spec_is_canonical_abs(b, backslash_forbidden),
    ensures
        segs(a) == segs(b) <==> a == b,
{
    lemma_canonical_render(a, backslash_forbidden);
    lemma_canonical_render(b, backslash_forbidden);
}

/// Forbidding `\` only narrows the canonical form.
pub proof fn lemma_canonical_backslash_narrows(s: Seq<u8>)
    requires
        spec_is_canonical_abs(s, true),
    ensures
        spec_is_canonical_abs(s, false),
{
}

// ---------------------------------------------------------------------------
// Theorems: normalization introduces no new bytes
// ---------------------------------------------------------------------------

/// No element of `ps` contains the byte `b`.
pub open spec fn all_without(ps: Seq<Seq<u8>>, b: u8) -> bool {
    forall|i: int| 0 <= i < ps.len() ==> no_byte(#[trigger] ps[i], b)
}

/// Pieces of a string without `b` do not contain `b`.
proof fn lemma_pieces_without(s: Seq<u8>, b: u8)
    requires
        no_byte(s, b),
    ensures
        all_without(pieces(s), b),
    decreases s.len(),
{
    if s.len() > 0 {
        let s1 = s.drop_last();
        lemma_no_byte_push(s1, s.last(), b);
        assert(s1.push(s.last()) =~= s);
        lemma_pieces_without(s1, b);
        lemma_pieces_len(s1);
        let p = pieces(s1);
        if s.last() != b'/' {
            lemma_no_byte_push(p.last(), s.last(), b);
            assert forall|i: int| 0 <= i < pieces(s).len() implies no_byte(
                #[trigger] pieces(s)[i],
                b,
            ) by {
                if i < p.len() - 1 {
                    assert(pieces(s)[i] == p[i]);
                }
            }
        } else {
            assert forall|i: int| 0 <= i < pieces(s).len() implies no_byte(
                #[trigger] pieces(s)[i],
                b,
            ) by {
                if i < p.len() {
                    assert(pieces(s)[i] == p[i]);
                }
            }
        }
    }
}

/// `keep` and resolution only select existing elements, so they introduce no
/// byte.
proof fn lemma_keep_resolve_without(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>, b: u8)
    requires
        all_without(st, b),
        all_without(ps, b),
    ensures
        all_without(keep(ps), b),
        resolve_from(st, ps) matches Some(t) ==> all_without(t, b),
    decreases ps.len(),
{
    if ps.len() > 0 {
        let p1 = ps.drop_last();
        assert(all_without(p1, b)) by {
            assert forall|i: int| 0 <= i < p1.len() implies no_byte(#[trigger] p1[i], b) by {
                assert(p1[i] == ps[i]);
            }
        }
        lemma_keep_resolve_without(st, p1, b);
        assert(no_byte(ps[ps.len() - 1], b));
        let k1 = keep(p1);
        assert forall|i: int| 0 <= i < keep(ps).len() implies no_byte(
            #[trigger] keep(ps)[i],
            b,
        ) by {
            if i < k1.len() {
                assert(keep(ps)[i] == k1[i]);
            }
        }
        if let Some(t1) = resolve_from(st, p1) {
            if let Some(t) = resolve_from(st, ps) {
                assert forall|i: int| 0 <= i < t.len() implies no_byte(#[trigger] t[i], b) by {
                    if i < t1.len() {
                        assert(t[i] == t1[i]);
                    }
                }
            }
        }
    }
}

/// Rendering adds only `/` bytes.
proof fn lemma_render_without(st: Seq<Seq<u8>>, b: u8)
    requires
        b != b'/',
        all_without(st, b),
    ensures
        no_byte(render_tail(st), b),
        no_byte(render(st), b),
    decreases st.len(),
{
    if st.len() > 0 {
        let st1 = st.drop_last();
        assert(all_without(st1, b)) by {
            assert forall|i: int| 0 <= i < st1.len() implies no_byte(#[trigger] st1[i], b) by {
                assert(st1[i] == st[i]);
            }
        }
        lemma_render_without(st1, b);
        let x = render_tail(st1);
        let y = st.last();
        assert(no_byte(y, b));
        let r = x + seq![b'/'] + y;
        assert forall|j: int| 0 <= j < r.len() implies #[trigger] r[j] != b by {
            if j < x.len() {
                assert(r[j] == x[j]);
            } else if j == x.len() {
                assert(r[j] == b'/');
            } else {
                assert(r[j] == y[j - x.len() - 1]);
            }
        }
    } else {
        assert(render_tail(st) =~= Seq::<u8>::empty());
        assert(render(st) =~= seq![b'/']);
    }
}

/// Normalization introduces no byte other than `/`: a byte that occurs in
/// neither the path nor (for a relative path) the working directory does not
/// occur in the result. So a NUL never appears from nowhere, and a path
/// without `\` normalizes to the portable form that forbids `\`.
pub proof fn lemma_normalize_introduces_no_bytes(p: Seq<u8>, cwd: Seq<u8>, b: u8)
    requires
        b != b'/',
        no_byte(p, b),
        is_abs(p) || no_byte(cwd, b),
        spec_normalize(p, cwd) is Ok,
    ensures
        no_byte(spec_normalize(p, cwd)->Ok_0, b),
        b == b'\\' ==> spec_is_canonical_abs(spec_normalize(p, cwd)->Ok_0, true),
{
    let j = joined(p, cwd);
    if !is_abs(p) {
        let sep = seq![b'/'];
        assert forall|k: int| 0 <= k < j.len() implies #[trigger] j[k] != b by {
            if k < cwd.len() {
                assert(j[k] == cwd[k]);
            } else if k == cwd.len() {
                assert(j[k] == b'/');
            } else {
                assert(j[k] == p[k - cwd.len() - 1]);
            }
        }
    }
    lemma_pieces_without(j, b);
    let e = Seq::<Seq<u8>>::empty();
    lemma_keep_resolve_without(e, pieces(j), b);
    lemma_keep_resolve_without(e, segs(j), b);
    let st = resolve_segs(segs(j))->Some_0;
    lemma_render_without(st, b);
    lemma_normalize_canonical(p, cwd);
    let out = render(st);
    assert(out == spec_normalize(p, cwd)->Ok_0);
    if b == b'\\' && out != seq![b'/'] {
        lemma_pieces_without(out, b);
        assert forall|i: int| 1 <= i < pieces(out).len() implies seg_ok(
            #[trigger] pieces(out)[i],
            true,
        ) by {
            assert(seg_ok(pieces(out)[i], false));
            assert(no_byte(pieces(out)[i], b));
        }
    }
}

// ---------------------------------------------------------------------------
// Executable byte-level normalization
// ---------------------------------------------------------------------------

/// The byte strings held by a list of buffers.
pub open spec fn views(v: Seq<Vec<u8>>) -> Seq<Seq<u8>> {
    v.map_values(|x: Vec<u8>| x@)
}

/// Whether `s` is an absolute Unix path (starts with `/`).
// Only the Unix glue calls this byte-level entry point.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn is_absolute_unix(s: &[u8]) -> (absolute: bool)
    ensures
        absolute == is_abs(s@),
{
    !s.is_empty() && s[0] == b'/'
}

/// A copy of `s[start..end]`.
fn copy_range(s: &[u8], start: usize, end: usize) -> (v: Vec<u8>)
    requires
        start <= end <= s.len(),
    ensures
        v@ == s@.subrange(start as int, end as int),
{
    let mut v: Vec<u8> = Vec::new();
    let mut k = start;
    while k < end
        invariant
            start <= k <= end <= s.len(),
            v@ == s@.subrange(start as int, k as int),
        decreases end - k,
    {
        v.push(s[k]);
        proof {
            assert(v@ =~= s@.subrange(start as int, k + 1));
        }
        k += 1;
    }
    v
}

/// Applies `step` for the piece `s[start..end]` to `stack`.
fn apply_piece(stack: &mut Vec<Vec<u8>>, s: &[u8], start: usize, end: usize) -> (ok: bool)
    requires
        start <= end <= s.len(),
    ensures
        ok ==> step(views(old(stack)@), s@.subrange(start as int, end as int)) == Some(
            views(final(stack)@),
        ),
        !ok ==> step(views(old(stack)@), s@.subrange(start as int, end as int)) is None,
{
    let ghost piece = s@.subrange(start as int, end as int);
    let ghost before = views(stack@);
    let len = end - start;
    if len == 2 && s[start] == b'.' && s[start + 1] == b'.' {
        proof {
            assert(piece =~= seq![b'.', b'.']);
        }
        if stack.is_empty() {
            return false;
        }
        stack.pop();
        proof {
            assert(views(stack@) =~= before.drop_last());
        }
        true
    } else if len == 0 || (len == 1 && s[start] == b'.') {
        proof {
            if len == 1 {
                assert(piece =~= seq![b'.']);
            }
            assert(piece.len() != 2);
            assert(views(stack@) == before);
        }
        true
    } else {
        proof {
            if piece.len() == 2 {
                assert(piece[0] == s@[start as int]);
                assert(piece[1] == s@[start + 1]);
                assert(seq![b'.', b'.'][0] == b'.');
                assert(seq![b'.', b'.'][1] == b'.');
            }
            if piece.len() == 1 {
                assert(piece[0] == s@[start as int]);
                assert(seq![b'.'][0] == b'.');
            }
        }
        let seg = copy_range(s, start, end);
        stack.push(seg);
        proof {
            assert(views(stack@) =~= before.push(piece));
        }
        true
    }
}

/// One unfolding of `resolve_from`.
proof fn lemma_resolve_unfold(st: Seq<Seq<u8>>, ps: Seq<Seq<u8>>)
    requires
        ps.len() > 0,
    ensures
        resolve_from(st, ps) == (match resolve_from(st, ps.drop_last()) {
            None => None,
            Some(t) => step(t, ps.last()),
        }),
{
}

/// Applies `step` for every piece of `s` to `stack`.
fn resolve_into(stack: &mut Vec<Vec<u8>>, s: &[u8]) -> (ok: bool)
    ensures
        ok ==> resolve_from(views(old(stack)@), pieces(s@)) == Some(views(final(stack)@)),
        !ok ==> resolve_from(views(old(stack)@), pieces(s@)) is None,
{
    let ghost st0 = views(stack@);
    let n = s.len();
    let mut start: usize = 0;
    let mut i: usize = 0;
    let mut failed = false;
    proof {
        lemma_segs_trivial();
        assert(s@.subrange(0, 0) =~= Seq::<u8>::empty());
        assert(seq![Seq::<u8>::empty()].drop_last() =~= Seq::<Seq<u8>>::empty());
        assert(s@.subrange(0, 0) =~= s@.subrange(0, 0));
    }
    while i < n
        invariant
            n == s.len(),
            start <= i <= n,
            pieces(s@.subrange(0, i as int)).len() >= 1,
            pieces(s@.subrange(0, i as int)).last() == s@.subrange(start as int, i as int),
            !failed ==> resolve_from(st0, pieces(s@.subrange(0, i as int)).drop_last()) == Some(
                views(stack@),
            ),
            failed ==> resolve_from(st0, pieces(s@.subrange(0, i as int)).drop_last()) is None,
        decreases n - i,
    {
        let ghost p = pieces(s@.subrange(0, i as int));
        if s[i] == b'/' {
            if !failed {
                let ok = apply_piece(stack, s, start, i);
                if !ok {
                    failed = true;
                }
            }
            proof {
                lemma_scan_slash(s@, i as int, start as int, false);
                lemma_resolve_unfold(st0, p);
            }
            start = i + 1;
        } else {
            proof {
                lemma_scan_byte(s@, i as int, start as int, false);
            };
        }
        i += 1;
    }
    proof {
        assert(s@.subrange(0, n as int) =~= s@);
        lemma_resolve_unfold(st0, pieces(s@));
    }
    if failed {
        return false;
    }
    apply_piece(stack, s, start, n)
}

/// Renders a stack of segments as `/` or `/a/b/...`.
fn render_segments(st: &[Vec<u8>]) -> (out: Vec<u8>)
    ensures
        out@ == render(views(st@)),
{
    let mut out: Vec<u8> = Vec::new();
    if st.is_empty() {
        out.push(b'/');
        proof {
            assert(out@ =~= seq![b'/']);
        }
        return out;
    }
    let mut j: usize = 0;
    proof {
        assert(views(st@).subrange(0, 0) =~= Seq::<Seq<u8>>::empty());
        assert(out@ =~= Seq::<u8>::empty());
    }
    while j < st.len()
        invariant
            j <= st.len(),
            out@ == render_tail(views(st@).subrange(0, j as int)),
        decreases st.len() - j,
    {
        let ghost before = out@;
        out.push(b'/');
        let seg = &st[j];
        let mut k: usize = 0;
        proof {
            assert(seg@.subrange(0, 0) =~= Seq::<u8>::empty());
            assert(out@ =~= before + seq![b'/'] + seg@.subrange(0, 0));
        }
        while k < seg.len()
            invariant
                k <= seg.len(),
                out@ == before + seq![b'/'] + seg@.subrange(0, k as int),
            decreases seg.len() - k,
        {
            out.push(seg[k]);
            proof {
                assert(out@ =~= before + seq![b'/'] + seg@.subrange(0, k + 1));
            }
            k += 1;
        }
        proof {
            let v = views(st@).subrange(0, j + 1);
            assert(v.drop_last() =~= views(st@).subrange(0, j as int));
            assert(v.last() == st@[j as int]@);
            assert(seg@.subrange(0, seg.len() as int) =~= seg@);
        }
        j += 1;
    }
    proof {
        assert(views(st@).subrange(0, st.len() as int) =~= views(st@));
    }
    out
}

/// Unix lexical normalization: computes `spec_normalize` of `path`
/// against the working directory `cwd` (ignored when `path` is absolute).
///
/// Total. Its `Ok` results are canonical (`lemma_normalize_canonical`),
/// fixed points of normalization (`lemma_normalize_idempotent`), and the
/// canonical strings are exactly its fixed points
/// (`lemma_canonical_iff_fixpoint`).
// Only the Unix glue calls this byte-level entry point.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn normalize_unix(path: &[u8], cwd: &[u8]) -> (r: Result<Vec<u8>, NormError>)
    ensures
        r matches Ok(out) ==> spec_normalize(path@, cwd@) == Ok::<Seq<u8>, NormError>(out@),
        r matches Err(e) ==> spec_normalize(path@, cwd@) == Err::<Seq<u8>, NormError>(e),
{
    let ghost e = Seq::<Seq<u8>>::empty();
    proof {
        lemma_resolve_keep(e, pieces(joined(path@, cwd@)));
        if !is_abs(path@) {
            lemma_pieces_concat(cwd@, path@);
            lemma_resolve_concat(e, pieces(cwd@), pieces(path@));
        }
    }
    let absolute = is_absolute_unix(path);
    if !absolute && !is_absolute_unix(cwd) {
        return Err(NormError::BaseNotAbsolute);
    }
    let mut stack: Vec<Vec<u8>> = Vec::new();
    proof {
        assert(views(stack@) =~= e);
    }
    if !absolute && !resolve_into(&mut stack, cwd) {
        return Err(NormError::EscapesRoot);
    }
    if !resolve_into(&mut stack, path) {
        return Err(NormError::EscapesRoot);
    }
    Ok(render_segments(&stack))
}

/// Checks a single piece `s[start..end]` (known to be `/`-free, with
/// `has_backslash` telling whether it contains `\`).
fn piece_is_ok(
    s: &[u8],
    start: usize,
    end: usize,
    has_backslash: bool,
    backslash_forbidden: bool,
) -> (ok: bool)
    requires
        start <= end <= s.len(),
        no_byte(s@.subrange(start as int, end as int), b'/'),
        has_backslash == !no_byte(s@.subrange(start as int, end as int), b'\\'),
    ensures
        ok == seg_ok(s@.subrange(start as int, end as int), backslash_forbidden),
{
    let ghost piece = s@.subrange(start as int, end as int);
    let len = end - start;
    let single_dot = len == 1 && s[start] == b'.';
    let double_dot = len == 2 && s[start] == b'.' && s[start + 1] == b'.';
    proof {
        if len == 1 {
            assert(piece[0] == s@[start as int]);
            assert(seq![b'.'][0] == b'.');
            if single_dot {
                assert(piece =~= seq![b'.']);
            }
        }
        if len == 2 {
            assert(piece[0] == s@[start as int]);
            assert(piece[1] == s@[start + 1]);
            assert(seq![b'.', b'.'][0] == b'.');
            assert(seq![b'.', b'.'][1] == b'.');
            if double_dot {
                assert(piece =~= seq![b'.', b'.']);
            }
        }
    }
    !(len == 0 || single_dot || double_dot || (backslash_forbidden && has_backslash))
}

/// Every completed piece of `q` after the leading one (all but the first and
/// the last) is an allowed segment.
pub open spec fn completed_ok(q: Seq<Seq<u8>>, backslash_forbidden: bool) -> bool {
    forall|j: int| 1 <= j < q.len() - 1 ==> seg_ok(#[trigger] q[j], backslash_forbidden)
}

/// Scanning a `/`: the current piece is completed and an empty one starts.
proof fn lemma_scan_slash(s: Seq<u8>, i: int, start: int, backslash_forbidden: bool)
    requires
        0 <= start <= i < s.len(),
        s[i] == b'/',
        pieces(s.subrange(0, i)).len() >= 1,
        pieces(s.subrange(0, i)).last() == s.subrange(start, i),
    ensures
        pieces(s.subrange(0, i + 1)) == pieces(s.subrange(0, i)).push(Seq::<u8>::empty()),
        pieces(s.subrange(0, i + 1)).last() == s.subrange(i + 1, i + 1),
        pieces(s.subrange(0, i + 1)).drop_last() == pieces(s.subrange(0, i)),
        pieces(s.subrange(0, i)).len() >= 2 ==> completed_ok(
            pieces(s.subrange(0, i + 1)),
            backslash_forbidden,
        ) == (completed_ok(
            pieces(s.subrange(0, i)),
            backslash_forbidden,
        ) && seg_ok(s.subrange(start, i), backslash_forbidden)),
{
    let p = pieces(s.subrange(0, i));
    let q = pieces(s.subrange(0, i + 1));
    assert(s.subrange(0, i + 1).drop_last() =~= s.subrange(0, i));
    assert(q == p.push(Seq::<u8>::empty()));
    assert(q.drop_last() =~= p);
    assert(s.subrange(i + 1, i + 1) =~= Seq::<u8>::empty());
    assert(forall|j: int| 0 <= j < p.len() ==> q[j] == p[j]);
    if completed_ok(p, backslash_forbidden) && seg_ok(s.subrange(start, i), backslash_forbidden) {
        assert forall|j: int| 1 <= j < q.len() - 1 implies seg_ok(
            #[trigger] q[j],
            backslash_forbidden,
        ) by {
            if j < p.len() - 1 {
                assert(q[j] == p[j]);
            }
        }
    }
    if !completed_ok(p, backslash_forbidden) {
        let j = choose|j: int|
            1 <= j < p.len() - 1 && !seg_ok(#[trigger] p[j], backslash_forbidden);
        assert(q[j] == p[j]);
    }
}

/// Scanning a byte other than `/`: it extends the current piece.
proof fn lemma_scan_byte(s: Seq<u8>, i: int, start: int, backslash_forbidden: bool)
    requires
        0 <= start <= i < s.len(),
        s[i] != b'/',
        pieces(s.subrange(0, i)).len() >= 1,
        pieces(s.subrange(0, i)).last() == s.subrange(start, i),
    ensures
        pieces(s.subrange(0, i + 1)).len() == pieces(s.subrange(0, i)).len(),
        pieces(s.subrange(0, i + 1)).last() == s.subrange(start, i + 1),
        pieces(s.subrange(0, i + 1)).drop_last() == pieces(s.subrange(0, i)).drop_last(),
        completed_ok(pieces(s.subrange(0, i + 1)), backslash_forbidden) == completed_ok(
            pieces(s.subrange(0, i)),
            backslash_forbidden,
        ),
        no_byte(s.subrange(start, i + 1), b'/') == no_byte(s.subrange(start, i), b'/'),
        no_byte(s.subrange(start, i + 1), b'\\') == (no_byte(s.subrange(start, i), b'\\') && s[i]
            != b'\\'),
{
    let p = pieces(s.subrange(0, i));
    let q = pieces(s.subrange(0, i + 1));
    let cur = s.subrange(start, i);
    let ext = s.subrange(start, i + 1);
    assert(s.subrange(0, i + 1).drop_last() =~= s.subrange(0, i));
    assert(q == p.update(p.len() - 1, p.last().push(s[i])));
    assert(ext =~= cur.push(s[i]));
    assert(q.drop_last() =~= p.drop_last());
    assert(forall|j: int| 0 <= j < p.len() - 1 ==> q[j] == p[j]);
    lemma_no_byte_push(cur, s[i], b'/');
    lemma_no_byte_push(cur, s[i], b'\\');
}

/// `no_byte` of a sequence extended by one byte.
proof fn lemma_no_byte_push(x: Seq<u8>, c: u8, b: u8)
    ensures
        no_byte(x.push(c), b) == (no_byte(x, b) && c != b),
{
    let y = x.push(c);
    if no_byte(x, b) && c != b {
        assert forall|j: int| 0 <= j < y.len() implies #[trigger] y[j] != b by {
            if j < x.len() {
                assert(y[j] == x[j]);
            }
        }
    }
    if !no_byte(x, b) {
        let j = choose|j: int| 0 <= j < x.len() && #[trigger] x[j] == b;
        assert(y[j] == x[j]);
    }
    if c == b {
        assert(y[x.len() as int] == c);
    }
}

/// Whether `s` is in canonical absolute form (`spec_is_canonical_abs`).
///
/// Total. With `backslash_forbidden == false` it accepts exactly the fixed
/// points of `normalize_unix` (`lemma_canonical_iff_fixpoint`).
// Only the Unix glue calls this byte-level entry point.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn is_canonical_abs_path(s: &[u8], backslash_forbidden: bool) -> (canonical: bool)
    ensures
        canonical == spec_is_canonical_abs(s@, backslash_forbidden),
{
    let n = s.len();
    if n == 0 || s[0] != b'/' {
        proof {
            if s@ == seq![b'/'] {
                assert(s@[0] == b'/');
            }
        }
        return false;
    }
    if n == 1 {
        proof {
            assert(s@ =~= seq![b'/']);
        }
        return true;
    }
    let mut ok = true;
    let mut start: usize = 1;
    let mut i: usize = 1;
    let mut has_backslash = false;
    proof {
        lemma_segs_trivial();
        assert(s@.subrange(0, 1) =~= seq![b'/']);
        assert(s@.subrange(1, 1) =~= Seq::<u8>::empty());
    }
    while i < n
        invariant
            n == s.len(),
            n > 1,
            s@[0] == b'/',
            1 <= start <= i <= n,
            pieces(s@.subrange(0, i as int)).len() >= 2,
            pieces(s@.subrange(0, i as int)).last() == s@.subrange(start as int, i as int),
            ok == completed_ok(pieces(s@.subrange(0, i as int)), backslash_forbidden),
            no_byte(s@.subrange(start as int, i as int), b'/'),
            has_backslash == !no_byte(s@.subrange(start as int, i as int), b'\\'),
        decreases n - i,
    {
        if s[i] == b'/' {
            let piece_ok = piece_is_ok(s, start, i, has_backslash, backslash_forbidden);
            proof {
                lemma_scan_slash(s@, i as int, start as int, backslash_forbidden);
            }
            ok = ok && piece_ok;
            start = i + 1;
            has_backslash = false;
        } else {
            proof {
                lemma_scan_byte(s@, i as int, start as int, backslash_forbidden);
            }
            if s[i] == b'\\' {
                has_backslash = true;
            }
        }
        i += 1;
    }
    let last_ok = piece_is_ok(s, start, n, has_backslash, backslash_forbidden);
    proof {
        assert(s@.subrange(0, n as int) =~= s@);
        assert(s@ != seq![b'/']) by {
            assert(seq![b'/'].len() == 1);
        }
    }
    ok && last_ok
}

// ===========================================================================
// Tilde expansion
// ===========================================================================

/// A path separator: `/`, and also `\` when `backslash_is_sep` (Windows).
pub open spec fn is_sep(c: u8, backslash_is_sep: bool) -> bool {
    c == b'/' || (backslash_is_sep && c == b'\\')
}

/// How a path string uses a leading `~`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TildeForm {
    /// No tilde expansion applies; the path is used as written.
    NotTilde,
    /// Exactly `~`: the home directory.
    HomeOnly,
    /// `~` followed by separators; the path continues at `rest_start`, after
    /// every leading separator.
    HomeRelative {
        /// Index of the first byte after the separators that follow `~`.
        rest_start: usize,
    },
    /// `~name...` (another user's home), which is not supported.
    UserSyntax,
}

/// Classifies the leading `~` of `s`.
///
/// On Unix (`backslash_is_sep == false`) a `\` is an ordinary filename byte,
/// so `~\x` is a literal relative name (`NotTilde`), like a quoted tilde
/// prefix in a POSIX shell. For `HomeRelative` every separator after `~` is
/// skipped, so the rest never starts with a separator (on Unix it is never
/// absolute and the expansion cannot leave the home directory).
pub fn classify_tilde(s: &[u8], backslash_is_sep: bool) -> (form: TildeForm)
    ensures
        (form is NotTilde) <==> (s.len() == 0 || s[0] != b'~' || (!backslash_is_sep && s.len()
            >= 2 && s[1] == b'\\')),
        (form is HomeOnly) <==> s@ == seq![b'~'],
        (form is HomeRelative) <==> (s.len() >= 2 && s[0] == b'~' && is_sep(
            s[1],
            backslash_is_sep,
        )),
        (form is UserSyntax) <==> (s.len() >= 2 && s[0] == b'~' && !is_sep(s[1], backslash_is_sep)
            && s[1] != b'\\'),
        form matches TildeForm::HomeRelative { rest_start } ==> 2 <= rest_start <= s.len() && (
        forall|j: int| 1 <= j < rest_start ==> is_sep(#[trigger] s[j], backslash_is_sep)) && (
        rest_start == s.len() || !is_sep(s[rest_start as int], backslash_is_sep)),
{
    let n = s.len();
    if n == 0 || s[0] != b'~' {
        return TildeForm::NotTilde;
    }
    if n == 1 {
        proof {
            assert(s@ =~= seq![b'~']);
        }
        return TildeForm::HomeOnly;
    }
    proof {
        assert(s@ != seq![b'~']) by {
            assert(seq![b'~'].len() == 1);
        }
    }
    let c = s[1];
    if c == b'/' || (backslash_is_sep && c == b'\\') {
        let mut k: usize = 2;
        while k < n && (s[k] == b'/' || (backslash_is_sep && s[k] == b'\\'))
            invariant
                2 <= k <= n,
                n == s.len(),
                forall|j: int| 1 <= j < k ==> is_sep(#[trigger] s[j], backslash_is_sep),
            decreases n - k,
        {
            k += 1;
        }
        TildeForm::HomeRelative { rest_start: k }
    } else if c == b'\\' {
        TildeForm::NotTilde
    } else {
        TildeForm::UserSyntax
    }
}

/// `home` joined with the relative `rest` the way `PathBuf::push` joins a
/// relative path on Unix: a `/` is inserted unless `home` is empty or
/// already ends with `/`.
pub open spec fn spec_tilde_join(home: Seq<u8>, rest: Seq<u8>) -> Seq<u8> {
    if home.len() > 0 && home.last() != b'/' {
        home + seq![b'/'] + rest
    } else {
        home + rest
    }
}

/// Containment: the expansion's segments are the home directory's segments
/// followed by the rest's, and a relative rest keeps the home directory's
/// absoluteness (so `~//etc` stays under the home directory).
pub proof fn lemma_tilde_join_segments(home: Seq<u8>, rest: Seq<u8>)
    ensures
        segs(spec_tilde_join(home, rest)) == segs(home) + segs(rest),
        !is_abs(rest) ==> is_abs(spec_tilde_join(home, rest)) == is_abs(home),
{
    let e = Seq::<u8>::empty();
    lemma_segs_trivial();
    if home.len() > 0 && home.last() != b'/' {
        lemma_segs_concat(home, rest);
    } else if home.len() == 0 {
        assert(home + rest =~= rest);
        assert(Seq::<Seq<u8>>::empty() + segs(rest) =~= segs(rest));
    } else {
        let h1 = home.drop_last();
        assert(home + rest =~= h1 + seq![b'/'] + rest);
        assert(home =~= h1 + seq![b'/'] + e);
        lemma_segs_concat(h1, rest);
        lemma_segs_concat(h1, e);
        assert(segs(h1) + Seq::<Seq<u8>>::empty() =~= segs(h1));
    }
}

/// Builds `spec_tilde_join` of `home` and `rest`.
// Only the Unix glue calls this byte-level entry point.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn tilde_join(home: &[u8], rest: &[u8]) -> (out: Vec<u8>)
    ensures
        out@ == spec_tilde_join(home@, rest@),
{
    let mut out = copy_range(home, 0, home.len());
    proof {
        assert(home@.subrange(0, home.len() as int) =~= home@);
    }
    let ghost h = out@;
    if !home.is_empty() && home[home.len() - 1] != b'/' {
        out.push(b'/');
    }
    let ghost mid = out@;
    let mut k: usize = 0;
    proof {
        assert(rest@.subrange(0, 0) =~= Seq::<u8>::empty());
        assert(out@ =~= mid + rest@.subrange(0, 0));
    }
    while k < rest.len()
        invariant
            k <= rest.len(),
            out@ == mid + rest@.subrange(0, k as int),
        decreases rest.len() - k,
    {
        out.push(rest[k]);
        proof {
            assert(out@ =~= mid + rest@.subrange(0, k + 1));
        }
        k += 1;
    }
    proof {
        assert(rest@.subrange(0, rest.len() as int) =~= rest@);
        if home.len() > 0 && home@.last() != b'/' {
            assert(mid =~= home@ + seq![b'/']);
        } else {
            assert(mid =~= home@);
        }
    }
    out
}

// ===========================================================================
// Deepest existing prefix (for `canonicalize_existing`)
// ===========================================================================

/// The oracle reported every prefix longer than `k`, up to the whole path of
/// `n` components, as absent.
pub open spec fn absent_above<F: Fn(usize) -> bool>(present: F, k: int, n: int) -> bool {
    forall|j: usize| k < j <= n ==> #[trigger] present.ensures((j,), false)
}

/// Every component from index `k` on is a named component.
pub open spec fn named_from(kinds: Seq<CompKind>, k: int) -> bool {
    forall|j: int| k <= j < kinds.len() ==> #[trigger] kinds[j] is Normal
}

/// Walks up a path's components looking for the deepest prefix that exists.
///
/// `present(k)` answers whether the prefix of the first `k` components
/// exists. Starting at the whole path, the search steps up one component at
/// a time, but only over named components. `Ok(k)`: the prefix of length `k`
/// was observed to exist, every longer prefix was observed not to exist, and
/// the remainder `kinds[k..]` consists of named components only. `Err(m)`:
/// the prefix of length `m` was observed not to exist (as was every longer
/// one), and the search cannot step further up (`m == 0`, or the component
/// before it is not a named one). Each prefix length is asked at most once,
/// from longest to shortest.
///
/// The oracle's precondition is only granted for prefix lengths `k` whose
/// remainder `kinds[k..]` is all named components, so the search provably
/// never probes a prefix above a root, prefix, `.` or `..` component.
pub fn deepest_existing_prefix<F: Fn(usize) -> bool>(kinds: &[CompKind], present: &F) -> (r: Result<
    usize,
    usize,
>)
    requires
        forall|k: usize|
            k <= kinds.len() && named_from(kinds@, k as int) ==> #[trigger] present.requires((k,)),
    ensures
        r matches Ok(k) ==> k <= kinds.len() && present.ensures((k,), true) && absent_above(
            *present,
            k as int,
            kinds.len() as int,
        ) && named_from(kinds@, k as int),
        r matches Err(m) ==> m <= kinds.len() && present.ensures((m,), false) && absent_above(
            *present,
            m as int,
            kinds.len() as int,
        ) && named_from(kinds@, m as int) && (m == 0 || !(kinds[m - 1] is Normal)),
{
    let mut k = kinds.len();
    loop
        invariant
            k <= kinds.len(),
            forall|k: usize|
                k <= kinds.len() && named_from(kinds@, k as int) ==> #[trigger] present.requires(
                    (k,),
                ),
            absent_above(*present, k as int, kinds.len() as int),
            named_from(kinds@, k as int),
        decreases k,
    {
        let found = present(k);
        if found {
            return Ok(k);
        }
        if k == 0 || !matches!(kinds[k - 1], CompKind::Normal) {
            return Err(k);
        }
        k -= 1;
    }
}

// ===========================================================================
// Provenance policy (for `PathResolver::resolve`)
// ===========================================================================

/// The outcome of resolving a normalized path according to its provenance.
pub enum Resolution<T, E> {
    /// Use the normalized path as is (explicit provenance; the filesystem was
    /// not consulted).
    Normalized,
    /// Use this canonical path (implicit provenance, canonicalization
    /// succeeded).
    Canonical(T),
    /// Fail (implicit provenance, canonicalization failed); `warn` says
    /// whether to print the nonexistent-path warning first.
    Failed {
        /// The oracle's failure: the error, and whether it means the path
        /// does not exist.
        failure: (E, bool),
        /// Whether to warn before failing.
        warn: bool,
    },
}

/// The provenance policy. Explicit paths are only normalized, and the
/// canonicalization oracle is not called. Implicit paths are canonicalized
/// and never fall back to the normalized path: a failure is an error, with a
/// warning exactly when the path does not exist and warnings are enabled.
///
/// The oracle's precondition is only granted for implicit provenance
/// (`implicit ==> canonicalize.requires(())`), so a body that consulted the
/// oracle for an explicit path would fail verification: the explicit branch
/// provably never calls it.
///
/// `canonicalize` returns the canonical path, or the error paired with
/// whether it means "not found".
pub fn resolve_by_provenance<T, E, F: Fn() -> Result<T, (E, bool)>>(
    implicit: bool,
    warn_enabled: bool,
    canonicalize: &F,
) -> (r: Resolution<T, E>)
    requires
        implicit ==> canonicalize.requires(()),
    ensures
        !implicit <==> r is Normalized,
        r matches Resolution::Canonical(c) ==> canonicalize.ensures((), Ok::<T, (E, bool)>(c)),
        r matches Resolution::Failed { failure, warn } ==> canonicalize.ensures(
            (),
            Err::<T, (E, bool)>(failure),
        ) && warn == (failure.1 && warn_enabled),
{
    if !implicit {
        return Resolution::Normalized;
    }
    match canonicalize() {
        Ok(c) => Resolution::Canonical(c),
        Err(failure) => {
            let warn = failure.1 && warn_enabled;
            Resolution::Failed { failure, warn }
        },
    }
}

} // verus!
