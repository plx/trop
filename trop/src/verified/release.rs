//! Verified kernel: release selection and the unrelated-path guard over the
//! affected set.
//!
//! - `tag_selector`: the specification's tag table for `trop release`
//!   (`--tag` and `--untagged-only` together are an error; neither selects
//!   every tag; `--untagged-only` selects the untagged reservation; `--tag t`
//!   selects tag `t`). `plan_flags` / `selector_from_plan` translate it to and
//!   from the planner's `(key tag, all tags)` pair without loss
//!   (`lemma_plan_flags_round_trip`).
//! - `select_and_guard`: exactly the candidate rows a release deletes, in
//!   order: rows at the target path (or, recursively, at a component
//!   descendant of it) whose tag the selector accepts (`release_selects`).
//!   Paths are component-token sequences, so `/work/ab` is never under
//!   `/work/a` (`lemma_component_aware`). The non-recursive selection is
//!   contained in the recursive one and every selector's in `AllTags`'.
//!   The unrelated-path guard is applied in the same step to exactly the
//!   selected rows.
//! - The unrelated-path guard (`guard_paths`, `select_and_guard`, and
//!   `super::migrate::guard_moves`): with neither `--force` nor
//!   `--allow-unrelated-path`, an operation may proceed only if every key it
//!   affects is lexically related to the working directory
//!   (`lexically_related`: `spec_between` is not `Unrelated`) or the
//!   physical fallback permitted it; otherwise it names the first refused
//!   key. With either flag, the fallback is never consulted.
//!
//! Production glue: `crate::operations::release`, `crate::operations::migrate`
//! (the guard), and `trop-cli`'s `release` command (the tag table).

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// Tag selection
// ===========================================================================

/// Which tags a release selects at every selected path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagSelector<T> {
    /// Every reservation, tagged or not (no `--tag`, no `--untagged-only`).
    AllTags,
    /// Only the untagged reservation (`--untagged-only`).
    UntaggedOnly,
    /// Only the reservation with this tag (`--tag`).
    Exact(T),
}

/// `--tag` and `--untagged-only` were both given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictingTagFlags;

/// The specification's tag table for `trop release`.
pub open spec fn spec_tag_selector<T>(tag: Option<T>, untagged_only: bool) -> Result<
    TagSelector<T>,
    ConflictingTagFlags,
> {
    match tag {
        Some(t) => if untagged_only {
            Err(ConflictingTagFlags)
        } else {
            Ok(TagSelector::Exact(t))
        },
        None => if untagged_only {
            Ok(TagSelector::UntaggedOnly)
        } else {
            Ok(TagSelector::AllTags)
        },
    }
}

/// The selector does not depend on the tag's type: it is an error exactly
/// when both flags are given.
pub proof fn lemma_tag_selector_cases<T>(tag: Option<T>, untagged_only: bool)
    ensures
        (spec_tag_selector(tag, untagged_only) is Err) <==> (tag is Some && untagged_only),
        tag is None && !untagged_only ==> spec_tag_selector(tag, untagged_only) == Ok::<
            TagSelector<T>,
            ConflictingTagFlags,
        >(TagSelector::AllTags),
        tag is None && untagged_only ==> spec_tag_selector(tag, untagged_only) == Ok::<
            TagSelector<T>,
            ConflictingTagFlags,
        >(TagSelector::UntaggedOnly),
        tag is Some && !untagged_only ==> spec_tag_selector(tag, untagged_only) == Ok::<
            TagSelector<T>,
            ConflictingTagFlags,
        >(TagSelector::Exact(tag->Some_0)),
{
}

/// Resolves `--tag` / `--untagged-only` (`spec_tag_selector`).
pub fn tag_selector<T>(tag: Option<T>, untagged_only: bool) -> (r: Result<
    TagSelector<T>,
    ConflictingTagFlags,
>)
    ensures
        r == spec_tag_selector(tag, untagged_only),
{
    match tag {
        Some(t) => {
            if untagged_only {
                Err(ConflictingTagFlags)
            } else {
                Ok(TagSelector::Exact(t))
            }
        },
        None => {
            if untagged_only {
                Ok(TagSelector::UntaggedOnly)
            } else {
                Ok(TagSelector::AllTags)
            }
        },
    }
}

/// The planner's representation of a selector: the key's tag and whether
/// every tag at a path is selected.
pub open spec fn spec_plan_flags<T>(sel: TagSelector<T>) -> (Option<T>, bool) {
    match sel {
        TagSelector::AllTags => (None, true),
        TagSelector::UntaggedOnly => (None, false),
        TagSelector::Exact(t) => (Some(t), false),
    }
}

/// The selector a planner `(key tag, all tags)` pair denotes: all tags when
/// the flag is set (the key's tag is then ignored), otherwise exactly the
/// key's optional tag.
pub open spec fn spec_selector_from_plan<T>(tag: Option<T>, all_tags: bool) -> TagSelector<T> {
    if all_tags {
        TagSelector::AllTags
    } else {
        match tag {
            Some(t) => TagSelector::Exact(t),
            None => TagSelector::UntaggedOnly,
        }
    }
}

/// Translating a selector to planner flags and back is the identity.
pub proof fn lemma_plan_flags_round_trip<T>(sel: TagSelector<T>)
    ensures
        spec_selector_from_plan(spec_plan_flags(sel).0, spec_plan_flags(sel).1) == sel,
{
}

/// The planner flags for a selector (`spec_plan_flags`).
pub fn plan_flags<T>(sel: TagSelector<T>) -> (r: (Option<T>, bool))
    ensures
        r == spec_plan_flags(sel),
{
    match sel {
        TagSelector::AllTags => (None, true),
        TagSelector::UntaggedOnly => (None, false),
        TagSelector::Exact(t) => (Some(t), false),
    }
}

/// The selector for planner flags (`spec_selector_from_plan`).
pub fn selector_from_plan<T>(tag: Option<T>, all_tags: bool) -> (r: TagSelector<T>)
    ensures
        r == spec_selector_from_plan(tag, all_tags),
{
    if all_tags {
        TagSelector::AllTags
    } else {
        match tag {
            Some(t) => TagSelector::Exact(t),
            None => TagSelector::UntaggedOnly,
        }
    }
}


// ===========================================================================
// Row selection
// ===========================================================================

/// The bytes a selector holds.
pub open spec fn sel_view(sel: TagSelector<Vec<u8>>) -> TagSelector<Seq<u8>> {
    match sel {
        TagSelector::AllTags => TagSelector::AllTags,
        TagSelector::UntaggedOnly => TagSelector::UntaggedOnly,
        TagSelector::Exact(t) => TagSelector::Exact(t@),
    }
}

/// The selector accepts a reservation with this tag.
pub open spec fn selects_tag(sel: TagSelector<Seq<u8>>, tag: Option<Seq<u8>>) -> bool {
    match sel {
        TagSelector::AllTags => true,
        TagSelector::UntaggedOnly => tag is None,
        TagSelector::Exact(t) => tag == Some(t),
    }
}

/// A path is selected: the target itself, or (recursively) any component
/// descendant of it. Paths are sequences of components, of any element type.
pub open spec fn selects_path<A>(recursive: bool, target: Seq<A>, path: Seq<A>) -> bool {
    if recursive {
        super::path_relation::is_prefix_of(target, path)
    } else {
        path == target
    }
}

/// A release deletes the reservation at `path` with `tag`.
pub open spec fn release_selects<A>(
    sel: TagSelector<Seq<u8>>,
    recursive: bool,
    target: Seq<A>,
    path: Seq<A>,
    tag: Option<Seq<u8>>,
) -> bool {
    selects_path(recursive, target, path) && selects_tag(sel, tag)
}

/// One candidate reservation: its path as component tokens (the
/// `super::path_relation` model of `std::path::Component`s) and its tag.
pub struct ReleaseRow {
    /// The path's components.
    pub path: Vec<super::path_relation::PathToken>,
    /// The tag's bytes, if tagged.
    pub tag: Option<Vec<u8>>,
}

/// The components of a row's path.
pub open spec fn row_path(row: ReleaseRow) -> Seq<(super::path::CompKind, Seq<u8>)> {
    super::path_relation::tok_views(row.path@)
}

/// The bytes of an optional tag.
pub open spec fn tag_view(t: Option<Vec<u8>>) -> Option<Seq<u8>> {
    match t {
        Some(v) => Some(v@),
        None => None,
    }
}

/// Row `i` is selected.
pub open spec fn row_selected(
    sel: TagSelector<Seq<u8>>,
    recursive: bool,
    target: Seq<(super::path::CompKind, Seq<u8>)>,
    rows: Seq<ReleaseRow>,
    i: int,
) -> bool {
    release_selects(sel, recursive, target, row_path(rows[i]), tag_view(rows[i].tag))
}

/// `v` lists exactly the selected rows, strictly increasing (so no row is
/// deleted twice and the database order is kept).
pub open spec fn is_selection(
    v: Seq<usize>,
    sel: TagSelector<Seq<u8>>,
    recursive: bool,
    target: Seq<(super::path::CompKind, Seq<u8>)>,
    rows: Seq<ReleaseRow>,
) -> bool {
    &&& forall|j: int, l: int| 0 <= j < l < v.len() ==> v[j] < v[l]
    &&& forall|j: int| 0 <= j < v.len() ==> #[trigger] v[j] < rows.len()
    &&& forall|i: int|
        0 <= i < rows.len() ==> (v.contains(i as usize) <==> row_selected(
            sel,
            recursive,
            target,
            rows,
            i,
        ))
}

/// The tag table selects as the specification describes it.
pub proof fn lemma_selects_tag_meaning(t: Seq<u8>, x: Option<Seq<u8>>)
    ensures
        selects_tag(TagSelector::AllTags, x),
        selects_tag(TagSelector::UntaggedOnly, x) <==> x is None,
        selects_tag(TagSelector::Exact(t), x) <==> x == Some(t),
{
}

/// Inclusions: a non-recursive release selects a subset of the recursive
/// one, and `--tag` / `--untagged-only` select subsets of the default (all
/// tags).
pub proof fn lemma_selection_inclusions<A>(
    sel: TagSelector<Seq<u8>>,
    target: Seq<A>,
    path: Seq<A>,
    tag: Option<Seq<u8>>,
)
    ensures
        release_selects(sel, false, target, path, tag) ==> release_selects(
            sel,
            true,
            target,
            path,
            tag,
        ),
        release_selects(sel, true, target, path, tag) ==> release_selects(
            TagSelector::AllTags,
            true,
            target,
            path,
            tag,
        ),
        release_selects(sel, false, target, path, tag) ==> release_selects(
            TagSelector::AllTags,
            false,
            target,
            path,
            tag,
        ),
{
    if path == target {
        assert(path.subrange(0, target.len() as int) =~= target);
    }
}

/// Selection is component-aware: a path that differs from the target in any
/// of the target's components is never selected, even recursively. Bytes of
/// a component are never compared across a component boundary, so (see
/// `lemma_component_aware_example`) `/work/ab` is not under `/work/a`.
pub proof fn lemma_component_aware<A>(
    sel: TagSelector<Seq<u8>>,
    target: Seq<A>,
    path: Seq<A>,
    tag: Option<Seq<u8>>,
    j: int,
)
    requires
        0 <= j < target.len(),
        j < path.len(),
        path[j] != target[j],
    ensures
        !release_selects(sel, true, target, path, tag),
        !release_selects(sel, false, target, path, tag),
{
    if target.len() <= path.len() {
        assert(path.subrange(0, target.len() as int)[j] == path[j]);
    }
}

/// `lemma_component_aware` on the tokens the glue produces for the Unix
/// paths `/work/a` (target) and `/work/ab`.
pub proof fn lemma_component_aware_example(sel: TagSelector<Seq<u8>>, tag: Option<Seq<u8>>)
    ensures
        !release_selects(
            sel,
            true,
            seq![
                (super::path::CompKind::Root, seq![b'/']),
                (super::path::CompKind::Normal, seq![b'w', b'o', b'r', b'k']),
                (super::path::CompKind::Normal, seq![b'a']),
            ],
            seq![
                (super::path::CompKind::Root, seq![b'/']),
                (super::path::CompKind::Normal, seq![b'w', b'o', b'r', b'k']),
                (super::path::CompKind::Normal, seq![b'a', b'b']),
            ],
            tag,
        ),
{
    let target = seq![
        (super::path::CompKind::Root, seq![b'/']),
        (super::path::CompKind::Normal, seq![b'w', b'o', b'r', b'k']),
        (super::path::CompKind::Normal, seq![b'a']),
    ];
    let path = seq![
        (super::path::CompKind::Root, seq![b'/']),
        (super::path::CompKind::Normal, seq![b'w', b'o', b'r', b'k']),
        (super::path::CompKind::Normal, seq![b'a', b'b']),
    ];
    assert(path[2].1.len() != target[2].1.len());
    lemma_component_aware(sel, target, path, tag, 2);
}

/// Token-sequence prefix (and, at equal lengths, equality).
fn tokens_prefix(
    p: &[super::path_relation::PathToken],
    s: &[super::path_relation::PathToken],
) -> (r: bool)
    ensures
        r == super::path_relation::is_prefix_of(
            super::path_relation::tok_views(p@),
            super::path_relation::tok_views(s@),
        ),
        p.len() == s.len() ==> (r == (super::path_relation::tok_views(p@)
            == super::path_relation::tok_views(s@))),
{
    let ghost cp = super::path_relation::tok_views(p@);
    let ghost cs = super::path_relation::tok_views(s@);
    if p.len() > s.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < p.len()
        invariant
            p.len() <= s.len(),
            i <= p.len(),
            cp == super::path_relation::tok_views(p@),
            cs == super::path_relation::tok_views(s@),
            forall|j: int| 0 <= j < i ==> cs[j] == cp[j],
        decreases p.len() - i,
    {
        if !super::path_relation::token_eq(&p[i], &s[i]) {
            proof {
                assert(cs.subrange(0, p.len() as int)[i as int] == cs[i as int]);
                assert(cp[i as int] == super::path_relation::tok_view(p@[i as int]));
                assert(cs[i as int] == super::path_relation::tok_view(s@[i as int]));
            }
            return false;
        }
        proof {
            assert(cp[i as int] == super::path_relation::tok_view(p@[i as int]));
            assert(cs[i as int] == super::path_relation::tok_view(s@[i as int]));
        }
        i += 1;
    }
    proof {
        assert(cs.subrange(0, p.len() as int) =~= cp);
        if p.len() == s.len() {
            assert(cs =~= cp);
        }
    }
    true
}

/// Token-sequence equality.
fn tokens_eq(
    a: &[super::path_relation::PathToken],
    b: &[super::path_relation::PathToken],
) -> (r: bool)
    ensures
        r == (super::path_relation::tok_views(a@) == super::path_relation::tok_views(b@)),
{
    if a.len() != b.len() {
        assert(super::path_relation::tok_views(a@).len() != super::path_relation::tok_views(
            b@,
        ).len());
        return false;
    }
    tokens_prefix(a, b)
}

/// The selector accepts `tag` (`selects_tag`).
pub fn tag_matches(sel: &TagSelector<Vec<u8>>, tag: Option<&[u8]>) -> (r: bool)
    ensures
        r == selects_tag(sel_view(*sel), super::sticky::opt_bytes(tag)),
{
    match sel {
        TagSelector::AllTags => true,
        TagSelector::UntaggedOnly => tag.is_none(),
        TagSelector::Exact(t) => match tag {
            Some(x) => super::sticky::bytes_eq(t.as_slice(), x),
            None => false,
        },
    }
}

/// The indices of exactly the selected rows, strictly increasing.
fn select_rows(
    sel: &TagSelector<Vec<u8>>,
    recursive: bool,
    target: &[super::path_relation::PathToken],
    rows: &[ReleaseRow],
) -> (r: Vec<usize>)
    ensures
        is_selection(
            r@,
            sel_view(*sel),
            recursive,
            super::path_relation::tok_views(target@),
            rows@,
        ),
{
    let ghost t = super::path_relation::tok_views(target@);
    let mut out: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < rows.len()
        invariant
            i <= rows.len(),
            t == super::path_relation::tok_views(target@),
            forall|j: int, l: int| 0 <= j < l < out@.len() ==> out@[j] < out@[l],
            forall|j: int| 0 <= j < out@.len() ==> #[trigger] out@[j] < i,
            forall|l: int|
                0 <= l < i ==> (out@.contains(l as usize) <==> row_selected(
                    sel_view(*sel),
                    recursive,
                    t,
                    rows@,
                    l,
                )),
        decreases rows.len() - i,
    {
        let row = &rows[i];
        let path_ok = if recursive {
            tokens_prefix(target, row.path.as_slice())
        } else {
            tokens_eq(row.path.as_slice(), target)
        };
        let tag_arg: Option<&[u8]> = match &row.tag {
            Some(t) => Some(t.as_slice()),
            None => None,
        };
        proof {
            assert(super::sticky::opt_bytes(tag_arg) == tag_view(rows@[i as int].tag));
        }
        let tag_ok = tag_matches(sel, tag_arg);
        let ghost before = out@;
        if path_ok && tag_ok {
            out.push(i);
        }
        proof {
            assert(path_ok && tag_ok <==> row_selected(
                sel_view(*sel),
                recursive,
                t,
                rows@,
                i as int,
            ));
            assert forall|l: int| 0 <= l < i + 1 implies (out@.contains(l as usize)
                <==> row_selected(sel_view(*sel), recursive, t, rows@, l)) by {
                if l < i {
                    if out@.contains(l as usize) {
                        let j = choose|j: int| 0 <= j < out@.len() && out@[j] == l as usize;
                        if j < before.len() {
                            assert(before[j] == l);
                        }
                    }
                    if before.contains(l as usize) {
                        let j = choose|j: int| 0 <= j < before.len() && before[j] == l as usize;
                        assert(out@[j] == l);
                    }
                } else {
                    if out@.contains(l as usize) && out@.len() == before.len() {
                        let j = choose|j: int| 0 <= j < out@.len() && out@[j] == l as usize;
                        assert(before[j] < i);
                    }
                    if out@.len() > before.len() {
                        assert(out@[before.len() as int] == i);
                    }
                }
            }
        }
        i += 1;
    }
    out
}

// ===========================================================================
// The unrelated-path guard over the affected set
// ===========================================================================

/// The guard applies: neither `--force` nor `--allow-unrelated-path`.
pub open spec fn path_check_required(force: bool, allow_unrelated: bool) -> bool {
    !force && !allow_unrelated
}

/// Whether the unrelated-path guard applies (`path_check_required`).
pub fn requires_path_check(force: bool, allow_unrelated: bool) -> (b: bool)
    ensures
        b == path_check_required(force, allow_unrelated),
{
    !force && !allow_unrelated
}

/// The unrelated-path guard's inputs: the two overrides and the working
/// directory's component tokens (the glue reads the directory only when the
/// guard applies; otherwise it is never consulted).
pub struct PathGuard {
    /// `--force`.
    pub force: bool,
    /// `--allow-unrelated-path` (the flag or the configuration).
    pub allow_unrelated: bool,
    /// The working directory's components.
    pub cwd: Vec<super::path_relation::PathToken>,
}

/// The guard applies (`path_check_required` of its overrides).
pub open spec fn guard_applies(guard: PathGuard) -> bool {
    path_check_required(guard.force, guard.allow_unrelated)
}

/// The working directory's components.
pub open spec fn guard_cwd(guard: PathGuard) -> Seq<(super::path::CompKind, Seq<u8>)> {
    super::path_relation::tok_views(guard.cwd@)
}

/// The lexical half of the per-key gate: `PathRelationship::between(path,
/// cwd)` (`spec_between` over component tokens) is not `Unrelated`, so the
/// path is the working directory, an ancestor of it, or a descendant of it.
pub open spec fn lexically_related(
    path: Seq<(super::path::CompKind, Seq<u8>)>,
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
) -> bool {
    !(super::path_relation::spec_between(path, cwd) is Unrelated)
}

/// The gate permits `path` (which the physical fallback knows as `at`): it
/// is lexically related to the working directory, or else the fallback (the
/// canonical, physical comparison) answered `None`.
pub open spec fn permitted<E, F: Fn(usize) -> Option<E>>(
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
    path: Seq<(super::path::CompKind, Seq<u8>)>,
    fallback: F,
    at: usize,
) -> bool {
    lexically_related(path, cwd) || fallback.ensures((at,), None::<E>)
}

/// The gate refused `path` with `e`: it is lexically unrelated to the
/// working directory and the fallback refused it too.
pub open spec fn refused<E, F: Fn(usize) -> Option<E>>(
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
    path: Seq<(super::path::CompKind, Seq<u8>)>,
    fallback: F,
    at: usize,
    e: E,
) -> bool {
    !lexically_related(path, cwd) && fallback.ensures((at,), Some(e))
}

/// The per-key gate for one path. The lexical relation to the working
/// directory is decided here; the fallback is consulted only for a path
/// that is lexically unrelated.
pub fn gate_path<E, F: Fn(usize) -> Option<E>>(
    cwd: &[super::path_relation::PathToken],
    path: &[super::path_relation::PathToken],
    fallback: &F,
    at: usize,
) -> (r: Option<E>)
    requires
        fallback.requires((at,)),
    ensures
        r is None ==> permitted(
            super::path_relation::tok_views(cwd@),
            super::path_relation::tok_views(path@),
            *fallback,
            at,
        ),
        r matches Some(e) ==> refused(
            super::path_relation::tok_views(cwd@),
            super::path_relation::tok_views(path@),
            *fallback,
            at,
            e,
        ),
{
    match super::path_relation::path_relationship(path, cwd) {
        super::path_relation::Rel::Unrelated => fallback(at),
        _ => None,
    }
}

/// The first affected key the gate refused, with its error.
pub struct GuardFailure<E> {
    /// The index of the refused key.
    pub index: usize,
    /// The gate's error.
    pub error: E,
}

/// The gate permitted every path in `paths[..n]`.
pub open spec fn paths_permitted<E, F: Fn(usize) -> Option<E>>(
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
    paths: Seq<Vec<super::path_relation::PathToken>>,
    fallback: F,
    n: int,
) -> bool {
    forall|i: int|
        0 <= i < n ==> permitted(
            cwd,
            super::path_relation::tok_views(#[trigger] paths[i]@),
            fallback,
            i as usize,
        )
}

/// The unrelated-path guard over explicit paths (the requested path of a
/// release). With neither `--force` nor `--allow-unrelated-path`, success
/// means every path is lexically related to the working directory or the
/// fallback permitted it; a failure names the first refused path. With
/// either flag the guard passes and the fallback is never consulted (its
/// precondition is only granted when the guard applies).
pub fn guard_paths<E, F: Fn(usize) -> Option<E>>(
    guard: &PathGuard,
    paths: &[Vec<super::path_relation::PathToken>],
    fallback: &F,
) -> (r: Result<(), GuardFailure<E>>)
    requires
        guard_applies(*guard) ==> forall|i: usize|
            i < paths.len() ==> fallback.requires((i,)),
    ensures
        !guard_applies(*guard) ==> r is Ok,
        r is Ok && guard_applies(*guard) ==> paths_permitted(
            guard_cwd(*guard),
            paths@,
            *fallback,
            paths.len() as int,
        ),
        r matches Err(f) ==> guard_applies(*guard) && f.index < paths.len()
            && refused(
            guard_cwd(*guard),
            super::path_relation::tok_views(paths@[f.index as int]@),
            *fallback,
            f.index,
            f.error,
        ) && paths_permitted(
            guard_cwd(*guard),
            paths@,
            *fallback,
            f.index as int,
        ),
{
    if !requires_path_check(guard.force, guard.allow_unrelated) {
        return Ok(());
    }
    let mut i: usize = 0;
    while i < paths.len()
        invariant
            i <= paths.len(),
            guard_applies(*guard),
            forall|j: usize| j < paths.len() ==> fallback.requires((j,)),
            paths_permitted(guard_cwd(*guard), paths@, *fallback, i as int),
        decreases paths.len() - i,
    {
        if let Some(error) = gate_path(guard.cwd.as_slice(), paths[i].as_slice(), fallback, i) {
            return Err(GuardFailure { index: i, error });
        }
        i += 1;
    }
    Ok(())
}

/// The gate permitted row `i`'s path.
pub open spec fn row_permitted<E, F: Fn(usize) -> Option<E>>(
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
    rows: Seq<ReleaseRow>,
    fallback: F,
    i: usize,
) -> bool {
    permitted(cwd, row_path(rows[i as int]), fallback, i)
}

/// A release's selection and its unrelated-path guard in one step, so the
/// guard covers exactly the rows the release deletes.
///
/// - Selection: on success, exactly the selected rows (`is_selection`:
///   strictly increasing, each candidate listed iff `row_selected`).
/// - Soundness: when the guard applies, every selected row is lexically
///   related to the working directory or permitted by the physical fallback
///   (`row_permitted`), so a release never deletes a reservation sideways
///   of the working directory without an override.
/// - A failure names the first refused selected row; every selected row
///   before it was permitted.
/// - With `--force` or `--allow-unrelated-path` the release proceeds and the
///   fallback is never consulted.
pub fn select_and_guard<E, F: Fn(usize) -> Option<E>>(
    sel: &TagSelector<Vec<u8>>,
    recursive: bool,
    target: &[super::path_relation::PathToken],
    rows: &[ReleaseRow],
    guard: &PathGuard,
    fallback: &F,
) -> (r: Result<Vec<usize>, GuardFailure<E>>)
    requires
        guard_applies(*guard) ==> forall|i: usize|
            i < rows.len() ==> fallback.requires((i,)),
    ensures
        !guard_applies(*guard) ==> r is Ok,
        r matches Ok(v) ==> is_selection(
            v@,
            sel_view(*sel),
            recursive,
            super::path_relation::tok_views(target@),
            rows@,
        ),
        r is Ok && guard_applies(*guard) ==> forall|i: int|
            0 <= i < rows.len() && #[trigger] row_selected(
                sel_view(*sel),
                recursive,
                super::path_relation::tok_views(target@),
                rows@,
                i,
            ) ==> row_permitted(guard_cwd(*guard), rows@, *fallback, i as usize),
        r matches Err(f) ==> guard_applies(*guard) && f.index < rows.len()
            && row_selected(
            sel_view(*sel),
            recursive,
            super::path_relation::tok_views(target@),
            rows@,
            f.index as int,
        ) && refused(
            guard_cwd(*guard),
            row_path(rows@[f.index as int]),
            *fallback,
            f.index,
            f.error,
        ) && forall|i: int|
            0 <= i < f.index && #[trigger] row_selected(
                sel_view(*sel),
                recursive,
                super::path_relation::tok_views(target@),
                rows@,
                i,
            ) ==> row_permitted(guard_cwd(*guard), rows@, *fallback, i as usize),
{
    let ghost t = super::path_relation::tok_views(target@);
    let ghost c = guard_cwd(*guard);
    let selected = select_rows(sel, recursive, target, rows);
    if !requires_path_check(guard.force, guard.allow_unrelated) {
        return Ok(selected);
    }
    let mut j: usize = 0;
    while j < selected.len()
        invariant
            j <= selected.len(),
            guard_applies(*guard),
            forall|i: usize| i < rows.len() ==> fallback.requires((i,)),
            is_selection(selected@, sel_view(*sel), recursive, t, rows@),
            t == super::path_relation::tok_views(target@),
            c == guard_cwd(*guard),
            forall|p: int|
                0 <= p < j ==> row_permitted(c, rows@, *fallback, #[trigger] selected@[p]),
        decreases selected.len() - j,
    {
        let i = selected[j];
        proof {
            assert(selected@[j as int] < rows.len());
        }
        if let Some(error) = gate_path(guard.cwd.as_slice(), rows[i].path.as_slice(), fallback, i) {
            proof {
                assert(selected@.contains(i));
                assert forall|l: int|
                    0 <= l < i && #[trigger] row_selected(
                        sel_view(*sel),
                        recursive,
                        t,
                        rows@,
                        l,
                    ) implies row_permitted(c, rows@, *fallback, l as usize) by {
                    assert(selected@.contains(l as usize));
                    let p = choose|p: int| 0 <= p < selected@.len() && selected@[p] == l as usize;
                    if p >= j {
                        if p > j {
                            assert(selected@[j as int] < selected@[p]);
                        }
                    }
                    assert(p < j);
                }
            }
            return Err(GuardFailure { index: i, error });
        }
        j += 1;
    }
    proof {
        assert forall|i: int|
            0 <= i < rows.len() && #[trigger] row_selected(
                sel_view(*sel),
                recursive,
                t,
                rows@,
                i,
            ) implies row_permitted(c, rows@, *fallback, i as usize) by {
            assert(selected@.contains(i as usize));
            let p = choose|p: int| 0 <= p < selected@.len() && selected@[p] == i as usize;
            assert(row_permitted(c, rows@, *fallback, selected@[p]));
        }
    }
    Ok(selected)
}

} // verus!
