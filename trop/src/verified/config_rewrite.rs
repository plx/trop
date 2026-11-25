//! Verified kernel: key-preserving configuration-file rewrites, and the
//! choice of the file an exclusion edit writes to.
//!
//! `trop exclude`, `trop compact-exclusions`, and `trop scan --autoexclude` /
//! `--autocompact` edit one top-level key (`excluded_ports`) of a YAML
//! configuration file. The file is read as a YAML mapping (a sequence of
//! `(key, value)` entries with distinct keys), and the kernel plans the
//! rewritten mapping: every other entry is copied unchanged and in order,
//! and the edited key is replaced in place, or appended when absent
//! (`lemma_rewrite_replaces_in_place`, `lemma_rewrite_appends`). In
//! particular a key that was absent stays absent and an explicit `null`
//! stays `null`, so the rewrite never changes how a layer overlays the
//! layers below it (`lemma_rewrite_preserves_other_keys`): an omitted
//! `reservations` key keeps inheriting the group of `trop.yaml` instead of
//! becoming an explicit `null` that clears it.
//!
//! Production glue: `crate::port::exclusions` (`rewrite_excluded_ports`, and
//! the write-target selection used by `trop-cli`'s `config_file_for_write`).
//! Parsing and emitting YAML (`serde_yaml`) stay outside the kernel; the
//! glue checks that the emitted text parses back to the planned mapping, and
//! that every other key of the original reads back with its exact value.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Configuration documents and their overlay meaning
// ---------------------------------------------------------------------------
/// The state of one top-level key in a configuration document.
// A ghost type: only specifications use it. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint.
#[allow(dead_code)]
pub ghost enum KeyState {
    /// The document does not mention the key.
    Absent,
    /// The document sets the key to an explicit `null`.
    Null,
    /// The document sets the key to a value.
    Present,
}

/// What a configuration layer does with one key when it is laid over the
/// layers below it.
// A ghost type, like `KeyState`.
#[allow(dead_code)]
pub ghost enum Overlay {
    /// The layer keeps the value of the layers below.
    Inherit,
    /// The layer removes the value of the layers below.
    Clear,
    /// The layer supplies its own value.
    Replace,
}

/// The loader's reading of a key state. For most keys an explicit `null` is
/// read like an absent key (`null_clears == false`); for `reservations` it
/// clears the inherited group (`null_clears == true`).
pub open spec fn overlay_of(state: KeyState, null_clears: bool) -> Overlay {
    match state {
        KeyState::Absent => Overlay::Inherit,
        KeyState::Null => if null_clears {
            Overlay::Clear
        } else {
            Overlay::Inherit
        },
        KeyState::Present => Overlay::Replace,
    }
}

/// `i` is the first entry of `doc` whose key is `k`.
pub open spec fn first_key_index<K, V>(doc: Seq<(K, V)>, k: K, i: int) -> bool {
    &&& 0 <= i < doc.len()
    &&& doc[i].0 == k
    &&& forall|j: int| 0 <= j < i ==> (#[trigger] doc[j]).0 != k
}

/// The value `doc` gives the key `k` (its first entry for `k`), if any.
pub open spec fn lookup<K, V>(doc: Seq<(K, V)>, k: K) -> Option<V> {
    if exists|i: int| first_key_index(doc, k, i) {
        Some(doc[choose|i: int| first_key_index(doc, k, i)].1)
    } else {
        None
    }
}

/// The state of the key `k` in `doc`, where `is_null` recognizes the
/// explicit `null` value.
pub open spec fn key_state<K, V>(doc: Seq<(K, V)>, k: K, is_null: spec_fn(V) -> bool) -> KeyState {
    match lookup(doc, k) {
        None => KeyState::Absent,
        Some(v) => if is_null(v) {
            KeyState::Null
        } else {
            KeyState::Present
        },
    }
}

/// At most one entry of `doc` has the key `k` (YAML mappings have distinct
/// keys).
pub open spec fn key_at_most_once<K, V>(doc: Seq<(K, V)>, k: K) -> bool {
    forall|i: int, j: int|
        0 <= i < doc.len() && 0 <= j < doc.len() && i != j && (#[trigger] doc[i]).0 == k ==> (
        #[trigger] doc[j]).0 != k
}

// ---------------------------------------------------------------------------
// Rewrite plans
// ---------------------------------------------------------------------------
/// One entry of a rewritten mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteStep {
    /// Copy entry `i` of the original mapping (key and value) unchanged.
    Keep(usize),
    /// Write the edited key with its new value.
    Set,
}

/// `Keep(i)` for every `i` in `lo..hi` whose entry is not the edited key's,
/// in order.
pub open spec fn kept(t: Seq<bool>, lo: int, hi: int) -> Seq<RewriteStep>
    decreases hi - lo,
{
    if hi <= lo {
        Seq::empty()
    } else {
        let rest = kept(t, lo, hi - 1);
        if t[hi - 1] {
            rest
        } else {
            rest.push(RewriteStep::Keep((hi - 1) as usize))
        }
    }
}

/// `f` is the first entry marked in `t`.
pub open spec fn first_target(t: Seq<bool>, f: int) -> bool {
    &&& 0 <= f < t.len()
    &&& t[f]
    &&& forall|j: int| 0 <= j < f ==> !#[trigger] t[j]
}

/// The plan for a mapping whose entries for the edited key are marked in
/// `t`: every other entry is kept, in order; the first entry for the edited
/// key is replaced in place by the new value (later ones, which a YAML
/// mapping cannot have, are dropped); without such an entry the new value
/// is appended.
pub open spec fn planned(t: Seq<bool>) -> Seq<RewriteStep> {
    if exists|f: int| first_target(t, f) {
        let f = choose|f: int| first_target(t, f);
        kept(t, 0, f) + seq![RewriteStep::Set] + kept(t, f + 1, t.len() as int)
    } else {
        kept(t, 0, t.len() as int).push(RewriteStep::Set)
    }
}

/// The marks of a document: entry `i` holds the key `key`.
pub open spec fn targets<K, V>(doc: Seq<(K, V)>, key: K) -> Seq<bool> {
    Seq::new(doc.len(), |i: int| doc[i].0 == key)
}

/// The mapping a plan produces from `doc` when `key` is set to `value`.
pub open spec fn apply_plan<K, V>(doc: Seq<(K, V)>, plan: Seq<RewriteStep>, key: K, value: V) -> Seq<
    (K, V),
> {
    plan.map_values(
        |s: RewriteStep|
            match s {
                RewriteStep::Keep(i) => doc[i as int],
                RewriteStep::Set => (key, value),
            },
    )
}

/// The mapping `doc` with `key` set to `value` by the planned rewrite.
pub open spec fn rewritten<K, V>(doc: Seq<(K, V)>, key: K, value: V) -> Seq<(K, V)> {
    apply_plan(doc, planned(targets(doc, key)), key, value)
}

/// The first target is unique.
proof fn lemma_first_target_unique(t: Seq<bool>, f: int, g: int)
    requires
        first_target(t, f),
        first_target(t, g),
    ensures
        f == g,
{
    if f < g {
        assert(!t[f]);
    }
    if g < f {
        assert(!t[g]);
    }
}

/// One more step of `kept`.
proof fn lemma_kept_step(t: Seq<bool>, lo: int, hi: int)
    requires
        lo <= hi < t.len(),
    ensures
        kept(t, lo, hi + 1) == if t[hi] {
            kept(t, lo, hi)
        } else {
            kept(t, lo, hi).push(RewriteStep::Keep(hi as usize))
        },
{
}

/// Every step of `kept(t, lo, hi)` keeps an unmarked entry of `lo..hi`.
proof fn lemma_kept_indices(t: Seq<bool>, lo: int, hi: int)
    requires
        0 <= lo,
        hi <= t.len() <= usize::MAX,
    ensures
        forall|j: int|
            0 <= j < kept(t, lo, hi).len() ==> (#[trigger] kept(t, lo, hi)[j] matches RewriteStep::Keep(
                i,
            ) && lo <= i < hi && !t[i as int]),
    decreases hi - lo,
{
    if hi > lo {
        lemma_kept_indices(t, lo, hi - 1);
        let rest = kept(t, lo, hi - 1);
        assert forall|j: int| 0 <= j < kept(t, lo, hi).len() implies (#[trigger] kept(
            t,
            lo,
            hi,
        )[j] matches RewriteStep::Keep(i) && lo <= i < hi && !t[i as int]) by {
            if j < rest.len() {
                assert(kept(t, lo, hi)[j] == rest[j]);
            }
        }
    }
}

/// Plans the rewrite of a mapping whose entries for the edited key are
/// marked in `is_target` (see `planned`). Total: any marking is accepted.
///
/// Every `Keep(i)` names an unmarked entry (`i < is_target.len()`), so
/// executing the plan never indexes out of bounds, and exactly one step is
/// `Set`.
pub fn plan_key_rewrite(is_target: &[bool]) -> (plan: Vec<RewriteStep>)
    ensures
        plan@ == planned(is_target@),
        forall|j: int|
            0 <= j < plan@.len() ==> (#[trigger] plan@[j] matches RewriteStep::Keep(i) ==> i
                < is_target@.len() && !is_target@[i as int]),
        exists|s: int|
            0 <= s < plan@.len() && plan@[s] == RewriteStep::Set && forall|j: int|
                0 <= j < plan@.len() && j != s ==> #[trigger] plan@[j] != RewriteStep::Set,
{
    let n = is_target.len();
    let mut plan: Vec<RewriteStep> = Vec::new();
    let mut set_done = false;
    let ghost mut f: int = 0;
    let mut i: usize = 0;
    while i < n
        invariant
            i <= n == is_target@.len(),
            set_done ==> first_target(is_target@, f) && f < i && plan@ == kept(is_target@, 0, f)
                + seq![RewriteStep::Set] + kept(is_target@, f + 1, i as int),
            !set_done ==> (forall|j: int| 0 <= j < i ==> !#[trigger] is_target@[j]) && plan@
                == kept(is_target@, 0, i as int),
        decreases n - i,
    {
        let target = is_target[i];
        let first = target && !set_done;
        proof {
            lemma_kept_step(is_target@, 0, i as int);
            if set_done {
                lemma_kept_step(is_target@, f + 1, i as int);
            }
            if first {
                f = i as int;
            }
        }
        if first {
            plan.push(RewriteStep::Set);
            set_done = true;
        } else if !target {
            plan.push(RewriteStep::Keep(i));
        }
        proof {
            assert(kept(is_target@, i + 1, i + 1) =~= Seq::<RewriteStep>::empty());
            if set_done {
                assert(plan@ =~= kept(is_target@, 0, f) + seq![RewriteStep::Set] + kept(
                    is_target@,
                    f + 1,
                    i + 1,
                ));
            }
        }
        i += 1;
    }
    proof {
        let t = is_target@;
        if set_done {
            let g = choose|g: int| first_target(t, g);
            lemma_first_target_unique(t, f, g);
            lemma_kept_indices(t, 0, f);
            lemma_kept_indices(t, f + 1, n as int);
            let a = kept(t, 0, f);
            let b = kept(t, f + 1, n as int);
            assert forall|j: int| 0 <= j < plan@.len() && j != a.len() implies #[trigger] plan@[j]
                != RewriteStep::Set && (plan@[j] matches RewriteStep::Keep(i) ==> i < t.len()
                && !t[i as int]) by {
                if j < a.len() {
                    assert(plan@[j] == a[j]);
                } else {
                    assert(plan@[j] == b[j - a.len() - 1]);
                }
            }
            assert(plan@[a.len() as int] == RewriteStep::Set);
        } else {
            assert(!exists|g: int| first_target(t, g));
            lemma_kept_indices(t, 0, n as int);
        }
    }
    if !set_done {
        plan.push(RewriteStep::Set);
    }
    proof {
        if !set_done {
            let a = kept(is_target@, 0, n as int);
            assert forall|j: int| 0 <= j < plan@.len() && j != a.len() implies #[trigger] plan@[j]
                != RewriteStep::Set by {
                assert(plan@[j] == a[j]);
            }
        }
        let s = if set_done {
            kept(is_target@, 0, f).len() as int
        } else {
            kept(is_target@, 0, n as int).len() as int
        };
        assert(plan@[s] == RewriteStep::Set);
        assert(exists|s2: int|
            0 <= s2 < plan@.len() && plan@[s2] == RewriteStep::Set && forall|j: int|
                0 <= j < plan@.len() && j != s2 ==> #[trigger] plan@[j] != RewriteStep::Set);
    }
    plan
}

// ---------------------------------------------------------------------------
// What a planned rewrite does to a document
// ---------------------------------------------------------------------------
/// Kept entries copy the original entries of `lo..hi` when none of them is
/// the edited key's.
proof fn lemma_apply_kept<K, V>(doc: Seq<(K, V)>, key: K, value: V, lo: int, hi: int)
    requires
        0 <= lo <= hi <= doc.len() <= usize::MAX,
        forall|j: int| lo <= j < hi ==> (#[trigger] doc[j]).0 != key,
    ensures
        apply_plan(doc, kept(targets(doc, key), lo, hi), key, value) == doc.subrange(lo, hi),
    decreases hi - lo,
{
    let t = targets(doc, key);
    if hi > lo {
        lemma_apply_kept(doc, key, value, lo, hi - 1);
        lemma_kept_step(t, lo, hi - 1);
        assert(!t[hi - 1]);
        let prefix = kept(t, lo, hi - 1);
        assert(apply_plan(doc, prefix.push(RewriteStep::Keep((hi - 1) as usize)), key, value)
            =~= apply_plan(doc, prefix, key, value).push(doc[hi - 1]));
        assert(doc.subrange(lo, hi) =~= doc.subrange(lo, hi - 1).push(doc[hi - 1]));
    } else {
        assert(apply_plan(doc, kept(t, lo, hi), key, value) =~= doc.subrange(lo, hi));
    }
}

/// With an entry for the edited key (at `f`), the rewrite replaces that
/// entry's value in place and keeps every other entry, in order.
pub proof fn lemma_rewrite_replaces_in_place<K, V>(doc: Seq<(K, V)>, key: K, value: V, f: int)
    requires
        key_at_most_once(doc, key),
        doc.len() <= usize::MAX,
        0 <= f < doc.len(),
        doc[f].0 == key,
    ensures
        rewritten(doc, key, value) == doc.update(f, (key, value)),
{
    let t = targets(doc, key);
    let n = doc.len() as int;
    assert(first_target(t, f)) by {
        assert forall|j: int| 0 <= j < f implies !#[trigger] t[j] by {
            assert(doc[j].0 != key);
        }
    }
    let g = choose|g: int| first_target(t, g);
    lemma_first_target_unique(t, f, g);
    assert forall|j: int| f + 1 <= j < n implies (#[trigger] doc[j]).0 != key by {
        assert(doc[f].0 == key);
    }
    lemma_apply_kept(doc, key, value, 0, f);
    lemma_apply_kept(doc, key, value, f + 1, n);
    let a = kept(t, 0, f);
    let b = kept(t, f + 1, n);
    assert(apply_plan(doc, a + seq![RewriteStep::Set] + b, key, value) =~= apply_plan(
        doc,
        a,
        key,
        value,
    ) + seq![(key, value)] + apply_plan(doc, b, key, value));
    assert(doc.update(f, (key, value)) =~= doc.subrange(0, f) + seq![(key, value)]
        + doc.subrange(f + 1, n));
}

/// Without an entry for the edited key, the rewrite appends it and keeps
/// every other entry, in order.
pub proof fn lemma_rewrite_appends<K, V>(doc: Seq<(K, V)>, key: K, value: V)
    requires
        doc.len() <= usize::MAX,
        forall|j: int| 0 <= j < doc.len() ==> (#[trigger] doc[j]).0 != key,
    ensures
        rewritten(doc, key, value) == doc.push((key, value)),
{
    let t = targets(doc, key);
    let n = doc.len() as int;
    assert(!exists|f: int| first_target(t, f)) by {
        if exists|f: int| first_target(t, f) {
            let f = choose|f: int| first_target(t, f);
            assert(doc[f].0 != key);
        }
    }
    lemma_apply_kept(doc, key, value, 0, n);
    assert(apply_plan(doc, kept(t, 0, n).push(RewriteStep::Set), key, value) =~= apply_plan(
        doc,
        kept(t, 0, n),
        key,
        value,
    ).push((key, value)));
    assert(doc.subrange(0, n) =~= doc);
}

/// Rewriting a key with the value it already has returns the document
/// unchanged (so an unchanged list needs no write).
pub proof fn lemma_rewrite_unchanged_is_identity<K, V>(doc: Seq<(K, V)>, key: K, value: V, f: int)
    requires
        key_at_most_once(doc, key),
        doc.len() <= usize::MAX,
        0 <= f < doc.len(),
        doc[f] == (key, value),
    ensures
        rewritten(doc, key, value) == doc,
{
    lemma_rewrite_replaces_in_place(doc, key, value, f);
    assert(doc.update(f, (key, value)) =~= doc);
}

/// The rewrite keeps the edited key at most once.
pub proof fn lemma_rewrite_keeps_key_unique<K, V>(doc: Seq<(K, V)>, key: K, value: V)
    requires
        key_at_most_once(doc, key),
        doc.len() <= usize::MAX,
    ensures
        key_at_most_once(rewritten(doc, key, value), key),
{
    let out = rewritten(doc, key, value);
    if exists|f: int| 0 <= f < doc.len() && (#[trigger] doc[f]).0 == key {
        let f = choose|f: int| 0 <= f < doc.len() && (#[trigger] doc[f]).0 == key;
        lemma_rewrite_replaces_in_place(doc, key, value, f);
        assert forall|i: int, j: int|
            0 <= i < out.len() && 0 <= j < out.len() && i != j && (#[trigger] out[i]).0
                == key implies (#[trigger] out[j]).0 != key by {
            if i != f {
                assert(out[i] == doc[i]);
            }
            if j != f {
                assert(out[j] == doc[j]);
            }
        }
    } else {
        lemma_rewrite_appends(doc, key, value);
        assert forall|i: int, j: int|
            0 <= i < out.len() && 0 <= j < out.len() && i != j && (#[trigger] out[i]).0
                == key implies (#[trigger] out[j]).0 != key by {
            if i < doc.len() {
                assert(out[i] == doc[i]);
            }
            if j < doc.len() {
                assert(out[j] == doc[j]);
            }
        }
    }
}

/// Overlay preservation: the rewrite gives every other key exactly the value
/// (and so the state, and the overlay meaning under either reading of
/// `null`) it had, and gives the edited key the new value.
///
/// A key that was absent stays absent and an explicit `null` stays `null`:
/// an omitted `reservations` key never becomes an explicit `null` that
/// clears the group of the layers below.
pub proof fn lemma_rewrite_preserves_other_keys<K, V>(
    doc: Seq<(K, V)>,
    key: K,
    value: V,
    is_null: spec_fn(V) -> bool,
)
    requires
        key_at_most_once(doc, key),
        doc.len() <= usize::MAX,
    ensures
        forall|k: K|
            k != key ==> #[trigger] lookup(rewritten(doc, key, value), k) == lookup(doc, k),
        forall|k: K|
            k != key ==> #[trigger] key_state(rewritten(doc, key, value), k, is_null)
                == key_state(doc, k, is_null),
        forall|k: K, null_clears: bool|
            k != key ==> #[trigger] overlay_of(
                key_state(rewritten(doc, key, value), k, is_null),
                null_clears,
            ) == overlay_of(key_state(doc, k, is_null), null_clears),
        lookup(rewritten(doc, key, value), key) == Some(value),
        !is_null(value) ==> key_state(rewritten(doc, key, value), key, is_null)
            == KeyState::Present,
{
    let out = rewritten(doc, key, value);
    // `out` and `doc` agree at every position except `f`, where both hold
    // the edited key (or `out` has one more entry, holding the edited key).
    let has = exists|f: int| 0 <= f < doc.len() && (#[trigger] doc[f]).0 == key;
    let f = if has {
        choose|f: int| 0 <= f < doc.len() && (#[trigger] doc[f]).0 == key
    } else {
        doc.len() as int
    };
    if has {
        lemma_rewrite_replaces_in_place(doc, key, value, f);
    } else {
        lemma_rewrite_appends(doc, key, value);
    }
    assert(0 <= f < out.len());
    assert(out[f] == (key, value));
    assert forall|j: int| 0 <= j < out.len() && j != f implies j < doc.len() && #[trigger] out[j]
        == doc[j] by {}
    assert forall|j: int| 0 <= j < f implies (#[trigger] doc[j]).0 != key by {
        if has {
            assert(doc[f].0 == key);
        }
    }
    assert forall|k: K| k != key implies #[trigger] lookup(out, k) == lookup(doc, k) by {
        assert forall|i: int| first_key_index(out, k, i) <==> first_key_index(doc, k, i) by {
            if first_key_index(out, k, i) {
                assert(i != f);
                assert forall|j: int| 0 <= j < i implies (#[trigger] doc[j]).0 != k by {
                    if j != f {
                        assert(out[j] == doc[j]);
                        assert(out[j].0 != k);
                    }
                }
            }
            if first_key_index(doc, k, i) {
                assert(i != f);
                assert(out[i] == doc[i]);
                assert forall|j: int| 0 <= j < i implies (#[trigger] out[j]).0 != k by {
                    if j != f {
                        assert(out[j] == doc[j]);
                        assert(doc[j].0 != k);
                    }
                }
            }
        }
        if exists|i: int| first_key_index(doc, k, i) {
            let i1 = choose|i: int| first_key_index(doc, k, i);
            let i2 = choose|i: int| first_key_index(out, k, i);
            assert(first_key_index(out, k, i1));
            assert(first_key_index(doc, k, i2));
            // The first index is unique.
            if i1 < i2 {
                assert(doc[i1].0 == k);
            }
            if i2 < i1 {
                assert(doc[i2].0 == k);
            }
            assert(out[i1] == doc[i1]);
        } else {
            assert(!exists|i: int| first_key_index(out, k, i)) by {
                if exists|i: int| first_key_index(out, k, i) {
                    let i = choose|i: int| first_key_index(out, k, i);
                    assert(first_key_index(doc, k, i));
                }
            }
        }
    }
    assert forall|k: K| k != key implies #[trigger] key_state(out, k, is_null) == key_state(
        doc,
        k,
        is_null,
    ) by {
        assert(lookup(out, k) == lookup(doc, k));
    }
    assert forall|k: K, null_clears: bool| k != key implies #[trigger] overlay_of(
        key_state(out, k, is_null),
        null_clears,
    ) == overlay_of(key_state(doc, k, is_null), null_clears) by {
        assert(key_state(out, k, is_null) == key_state(doc, k, is_null));
    }
    // The edited key's first entry is the new one.
    assert forall|j: int| 0 <= j < f implies (#[trigger] out[j]).0 != key by {
        assert(out[j] == doc[j]);
    }
    assert(first_key_index(out, key, f));
    let i = choose|i: int| first_key_index(out, key, i);
    if i < f {
        assert(out[i].0 != key);
    }
    if f < i {
        assert(out[f].0 == key);
    }
}

// ---------------------------------------------------------------------------
// Which file an exclusion edit writes to
// ---------------------------------------------------------------------------
/// A configuration file an exclusion edit can write to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteTarget {
    /// The user configuration (`config.yaml` in the data directory).
    Global,
    /// The project's `trop.local.yaml`.
    Local,
    /// The project's `trop.yaml`.
    Project,
}

/// Which files an edit may write to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteScope {
    /// Only the global file (`trop exclude --global`).
    Global,
    /// Only a project-level file, which must exist (`trop exclude`).
    Project,
    /// A project-level file if one was loaded, otherwise the global file
    /// (`trop scan --autoexclude`).
    ProjectOrGlobal,
}

/// The file an edit writes to: the global file for the global scope;
/// otherwise the highest-precedence project-level file that was loaded
/// (`trop.local.yaml` over `trop.yaml`); with neither, the global file when
/// the scope allows it, and no file for the project scope.
pub open spec fn write_target(scope: WriteScope, has_local: bool, has_project: bool) -> Option<
    WriteTarget,
> {
    match scope {
        WriteScope::Global => Some(WriteTarget::Global),
        _ => if has_local {
            Some(WriteTarget::Local)
        } else if has_project {
            Some(WriteTarget::Project)
        } else if scope == WriteScope::Project {
            None
        } else {
            Some(WriteTarget::Global)
        },
    }
}

/// Chooses the file an exclusion edit writes to (see `write_target`), given
/// which project-level files were loaded.
///
/// `trop exclude` uses the project scope, so it fails without a project
/// file unless `--global` is given; `trop scan --autoexclude` falls back to
/// the global file.
#[must_use]
pub fn select_write_target(scope: WriteScope, has_local: bool, has_project: bool) -> (r: Option<
    WriteTarget,
>)
    ensures
        r == write_target(scope, has_local, has_project),
        // The global file is written only in a scope that allows it, and
        // never in the project scope.
        r == Some(WriteTarget::Global) ==> scope != WriteScope::Project,
        // A project-level file is written only if it was loaded, and never
        // in the global scope.
        r == Some(WriteTarget::Local) ==> has_local && scope != WriteScope::Global,
        r == Some(WriteTarget::Project) ==> has_project && !has_local && scope
            != WriteScope::Global,
        // No file only when the project scope finds no project file.
        r is None <==> (scope == WriteScope::Project && !has_local && !has_project),
{
    match scope {
        WriteScope::Global => Some(WriteTarget::Global),
        WriteScope::Project | WriteScope::ProjectOrGlobal => {
            if has_local {
                Some(WriteTarget::Local)
            } else if has_project {
                Some(WriteTarget::Project)
            } else if matches_project(scope) {
                None
            } else {
                Some(WriteTarget::Global)
            }
        },
    }
}

/// Whether `scope` is the project scope.
const fn matches_project(scope: WriteScope) -> (b: bool)
    ensures
        b == (scope == WriteScope::Project),
{
    match scope {
        WriteScope::Project => true,
        WriteScope::Global | WriteScope::ProjectOrGlobal => false,
    }
}

} // verus!
