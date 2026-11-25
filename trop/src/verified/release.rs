//! Verified kernel: release selection and the unrelated-path guard over the
//! affected set.
//!
//! - `tag_selector`: the specification's tag table for `trop release`
//!   (`--tag` and `--untagged-only` together are an error; neither selects
//!   every tag; `--untagged-only` selects the untagged reservation; `--tag t`
//!   selects tag `t`). `plan_flags` / `selector_from_plan` translate it to and
//!   from the planner's `(key tag, all tags)` pair without loss
//!   (`lemma_plan_flags_round_trip`).
//! - `select_rows`: exactly the candidate rows a release deletes, in order:
//!   rows at the target path (or, recursively, at a component descendant of
//!   it) whose tag the selector accepts (`release_selects`). Paths are
//!   component sequences, so `/work/ab` is never under `/work/a`
//!   (`lemma_component_aware`). The non-recursive selection is contained in
//!   the recursive one and every selector's in `AllTags`'.
//! - `check_affected`: with neither `--force` nor `--allow-unrelated-path`,
//!   an operation may proceed only if the per-key gate permitted every key it
//!   affects; otherwise it names the first key the gate refused. With either
//!   flag, the gate is never consulted.
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
/// descendant of it.
pub open spec fn selects_path(recursive: bool, target: Seq<Seq<u8>>, path: Seq<Seq<u8>>) -> bool {
    if recursive {
        super::path_relation::is_prefix_of(target, path)
    } else {
        path == target
    }
}

/// A release deletes the reservation at `path` with `tag`.
pub open spec fn release_selects(
    sel: TagSelector<Seq<u8>>,
    recursive: bool,
    target: Seq<Seq<u8>>,
    path: Seq<Seq<u8>>,
    tag: Option<Seq<u8>>,
) -> bool {
    selects_path(recursive, target, path) && selects_tag(sel, tag)
}

/// One candidate reservation: its path as component keys and its tag.
pub struct ReleaseRow {
    /// The path's components, one key per component.
    pub path: Vec<Vec<u8>>,
    /// The tag's bytes, if tagged.
    pub tag: Option<Vec<u8>>,
}

/// The components of a path.
pub open spec fn comps(p: Seq<Vec<u8>>) -> Seq<Seq<u8>> {
    p.map_values(|c: Vec<u8>| c@)
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
    target: Seq<Seq<u8>>,
    rows: Seq<ReleaseRow>,
    i: int,
) -> bool {
    release_selects(sel, recursive, target, comps(rows[i].path@), tag_view(rows[i].tag))
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
pub proof fn lemma_selection_inclusions(
    sel: TagSelector<Seq<u8>>,
    target: Seq<Seq<u8>>,
    path: Seq<Seq<u8>>,
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

/// Selection is component-aware: `/work/ab` is not under `/work/a`, so a
/// recursive release of `/work/a` keeps it.
pub proof fn lemma_component_aware(sel: TagSelector<Seq<u8>>, tag: Option<Seq<u8>>)
    ensures
        !release_selects(
            sel,
            true,
            seq![seq![b'/'], seq![b'w', b'o', b'r', b'k'], seq![b'a']],
            seq![seq![b'/'], seq![b'w', b'o', b'r', b'k'], seq![b'a', b'b']],
            tag,
        ),
{
    let target = seq![seq![b'/'], seq![b'w', b'o', b'r', b'k'], seq![b'a']];
    let path = seq![seq![b'/'], seq![b'w', b'o', b'r', b'k'], seq![b'a', b'b']];
    assert(path.subrange(0, 3)[2] == path[2]);
    assert(path[2] != target[2]) by {
        assert(path[2].len() != target[2].len());
    }
}

/// Component-sequence equality.
fn components_eq(a: &[Vec<u8>], b: &[Vec<u8>]) -> (r: bool)
    ensures
        r == (comps(a@) == comps(b@)),
{
    if a.len() != b.len() {
        assert(comps(a@).len() != comps(b@).len());
        return false;
    }
    components_prefix(a, b)
}

/// `p`'s components are a prefix of `s`'s.
fn components_prefix(p: &[Vec<u8>], s: &[Vec<u8>]) -> (r: bool)
    ensures
        r == super::path_relation::is_prefix_of(comps(p@), comps(s@)),
        p.len() == s.len() ==> (r == (comps(p@) == comps(s@))),
{
    let ghost cp = comps(p@);
    let ghost cs = comps(s@);
    if p.len() > s.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < p.len()
        invariant
            p.len() <= s.len(),
            i <= p.len(),
            cp == comps(p@),
            cs == comps(s@),
            forall|j: int| 0 <= j < i ==> cs[j] == cp[j],
        decreases p.len() - i,
    {
        if !super::sticky::bytes_eq(p[i].as_slice(), s[i].as_slice()) {
            proof {
                assert(cs.subrange(0, p.len() as int)[i as int] == cs[i as int]);
                assert(cp[i as int] == p@[i as int]@);
                assert(cs[i as int] == s@[i as int]@);
            }
            return false;
        }
        proof {
            assert(cp[i as int] == p@[i as int]@);
            assert(cs[i as int] == s@[i as int]@);
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

/// The indices of exactly the selected rows, strictly increasing (so no row
/// is deleted twice and the database order is kept).
pub fn select_rows(
    sel: &TagSelector<Vec<u8>>,
    recursive: bool,
    target: &[Vec<u8>],
    rows: &[ReleaseRow],
) -> (r: Vec<usize>)
    ensures
        forall|j: int, l: int| 0 <= j < l < r@.len() ==> r@[j] < r@[l],
        forall|j: int| 0 <= j < r@.len() ==> #[trigger] r@[j] < rows.len(),
        forall|i: int|
            0 <= i < rows.len() ==> (r@.contains(i as usize) <==> row_selected(
                sel_view(*sel),
                recursive,
                comps(target@),
                rows@,
                i,
            )),
{
    let mut out: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < rows.len()
        invariant
            i <= rows.len(),
            forall|j: int, l: int| 0 <= j < l < out@.len() ==> out@[j] < out@[l],
            forall|j: int| 0 <= j < out@.len() ==> #[trigger] out@[j] < i,
            forall|l: int|
                0 <= l < i ==> (out@.contains(l as usize) <==> row_selected(
                    sel_view(*sel),
                    recursive,
                    comps(target@),
                    rows@,
                    l,
                )),
        decreases rows.len() - i,
    {
        let row = &rows[i];
        let path_ok = if recursive {
            components_prefix(target, row.path.as_slice())
        } else {
            components_eq(row.path.as_slice(), target)
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
                comps(target@),
                rows@,
                i as int,
            ));
            assert forall|l: int| 0 <= l < i + 1 implies (out@.contains(l as usize) <==> row_selected(
                sel_view(*sel),
                recursive,
                comps(target@),
                rows@,
                l,
            )) by {
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

/// The gate permitted every key in `0..n` (answered `None`, no refusal).
pub open spec fn all_permitted<E, F: Fn(usize) -> Option<E>>(gate: F, n: int) -> bool {
    forall|i: usize| 0 <= i < n ==> #[trigger] gate.ensures((i,), None::<E>)
}

/// The first affected key the gate refused, with its error.
pub struct GuardFailure<E> {
    /// The index of the refused key.
    pub index: usize,
    /// The gate's error.
    pub error: E,
}

/// The unrelated-path guard over the keys `0..n` an operation affects. The
/// gate answers `None` when it permits key `i` and `Some(error)` when it
/// refuses it.
///
/// - Soundness: when the guard applies, success means the gate permitted
///   every affected key, so no key sideways of the working directory is
///   modified without an override.
/// - A failure names the first refused key: every earlier key was permitted.
/// - With `--force` or `--allow-unrelated-path` the operation proceeds and
///   the gate is never consulted (its precondition is only granted when the
///   guard applies).
pub fn check_affected<E, F: Fn(usize) -> Option<E>>(
    force: bool,
    allow_unrelated: bool,
    n: usize,
    gate: &F,
) -> (r: Result<(), GuardFailure<E>>)
    requires
        path_check_required(force, allow_unrelated) ==> forall|i: usize| i < n ==> gate.requires((i,)),
    ensures
        !path_check_required(force, allow_unrelated) ==> r is Ok,
        r is Ok && path_check_required(force, allow_unrelated) ==> all_permitted(*gate, n as int),
        r matches Err(f) ==> path_check_required(force, allow_unrelated) && f.index < n
            && gate.ensures((f.index,), Some(f.error)) && all_permitted(*gate, f.index as int),
{
    if !requires_path_check(force, allow_unrelated) {
        return Ok(());
    }
    let mut i: usize = 0;
    while i < n
        invariant
            i <= n,
            path_check_required(force, allow_unrelated),
            forall|j: usize| j < n ==> gate.requires((j,)),
            all_permitted(*gate, i as int),
        decreases n - i,
    {
        let answer = gate(i);
        if let Some(error) = answer {
            return Err(GuardFailure { index: i, error });
        }
        proof {
            assert forall|j: usize| 0 <= j < i + 1 implies #[trigger] gate.ensures((j,), None::<E>) by {
                if j < i {
                    assert(all_permitted(*gate, i as int));
                }
            }
        }
        i += 1;
    }
    Ok(())
}

} // verus!
