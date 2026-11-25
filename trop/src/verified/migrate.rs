//! Verified kernel: migration conflict classification, the migration action
//! schedule, and canonical path rebasing.
//!
//! A migration moves the reservations stored under the *source* keys to the
//! *destination* keys. The glue interns every reservation key it touches into
//! an id in `0..key_count` (equal keys get equal ids), so the kernel works on
//! plain `usize` ids:
//!
//! - `classify_conflicts`: item `i` conflicts exactly when its destination
//!   already holds a reservation that is *not itself being moved away*
//!   (`is_conflict`). Under a consistent snapshot this is exactly "the
//!   migration would overwrite an existing reservation" (`lemma_conflicts_are_overwrites`);
//!   a migration of a key onto itself, or onto another source, is not a
//!   conflict.
//! - `schedule_migration`: the plan deletes every source key, then every
//!   destination key that is not a source, then creates every destination
//!   (`spec_schedule`). Over an abstract store (`Map<usize, R>`), running it
//!   yields exactly `moved`: each payload lands at its destination, every
//!   other source row is gone, and every other row is untouched
//!   (`lemma_schedule_correct`). If ports are unique beforehand, no step can
//!   violate `UNIQUE(port)` and every create targets an absent key
//!   (`lemma_schedule_port_safe`). The previous interleaved schedule loses a
//!   row when a destination is another item's source
//!   (`lemma_legacy_schedule_loses_data`).
//! - `guard_moves`: the unrelated-path guard over every source and every
//!   destination key of the same id lists (see `super::release` for the
//!   per-key gate: lexical relation decided here, physical fallback as an
//!   oracle).
//! - `rebase_canonical`: moves a canonical absolute path from under `from`
//!   to under `to` (`super::path_relation::spec_rebase` on segments), and the
//!   result is canonical.
//!
//! Production glue: `crate::operations::migrate`.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// The abstract store and the action language
// ===========================================================================

/// One step of a migration plan over interned key ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigAction {
    /// Delete whatever is stored under this key (a no-op when absent).
    DeleteKey(usize),
    /// Store item `i`'s preserved reservation under its destination key.
    Create(usize),
}

/// One action applied to a store of rows keyed by id. `Create(i)` stores
/// `payload[i]` under `dsts[i]` (an upsert, like the executor's create).
pub open spec fn apply_action<R>(
    db: Map<usize, R>,
    a: MigAction,
    dsts: Seq<usize>,
    payload: Seq<R>,
) -> Map<usize, R> {
    match a {
        MigAction::DeleteKey(k) => db.remove(k),
        MigAction::Create(i) => db.insert(dsts[i as int], payload[i as int]),
    }
}

/// The store after running `acts` in order.
pub open spec fn apply<R>(
    db: Map<usize, R>,
    acts: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
) -> Map<usize, R>
    decreases acts.len(),
{
    if acts.len() == 0 {
        db
    } else {
        apply_action(apply(db, acts.drop_last(), dsts, payload), acts.last(), dsts, payload)
    }
}

/// The intended result of a migration: every source row is gone, payload
/// `i` is stored at `dsts[i]` (overwriting whatever was there), and every
/// other row is untouched.
pub open spec fn moved<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    payload: Seq<R>,
) -> Map<usize, R> {
    Map::new(
        db.dom().filter(|k: usize| !srcs.contains(k)) + dsts.to_set(),
        |k: usize|
            if dsts.contains(k) {
                payload[dsts.index_of(k)]
            } else {
                db[k]
            },
    )
}

/// No id occurs twice.
pub open spec fn distinct(s: Seq<usize>) -> bool {
    forall|i: int, j: int| 0 <= i < j < s.len() ==> s[i] != s[j]
}

/// Every id is below `key_count`.
pub open spec fn in_range(s: Seq<usize>, key_count: usize) -> bool {
    forall|i: int| 0 <= i < s.len() ==> #[trigger] s[i] < key_count
}

/// The key lists describe a migration: one source and one destination per
/// item, ids in range, and no key used twice on either side.
pub open spec fn well_formed(key_count: usize, srcs: Seq<usize>, dsts: Seq<usize>) -> bool {
    &&& srcs.len() == dsts.len()
    &&& in_range(srcs, key_count)
    &&& in_range(dsts, key_count)
    &&& distinct(srcs)
    &&& distinct(dsts)
}

/// The payloads are the rows currently stored at the sources (the plan was
/// read from the store it runs against).
pub open spec fn snapshot<R>(db: Map<usize, R>, srcs: Seq<usize>, payload: Seq<R>) -> bool {
    &&& payload.len() == srcs.len()
    &&& forall|i: int|
        0 <= i < srcs.len() ==> db.dom().contains(#[trigger] srcs[i]) && payload[i] == db[srcs[i]]
}

// ===========================================================================
// Conflicts
// ===========================================================================

/// Item `i` would overwrite a reservation: its destination exists and is not
/// itself one of the keys being moved away.
pub open spec fn is_conflict(
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    dest_exists: Seq<bool>,
    i: int,
) -> bool {
    dest_exists[i] && !srcs.contains(dsts[i])
}

/// `c` lists exactly the conflicting items, in increasing order.
pub open spec fn conflict_list(
    c: Seq<usize>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    dest_exists: Seq<bool>,
) -> bool {
    &&& forall|j: int, l: int| 0 <= j < l < c.len() ==> c[j] < c[l]
    &&& forall|j: int| 0 <= j < c.len() ==> #[trigger] c[j] < dsts.len()
    &&& forall|i: int|
        0 <= i < dsts.len() ==> (c.contains(i as usize) <==> is_conflict(srcs, dsts, dest_exists, i))
}

/// A key whose current row the migration replaces: it exists, it is not a
/// source, and something is moved onto it.
pub open spec fn overwritten<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    k: usize,
) -> bool {
    db.dom().contains(k) && !srcs.contains(k) && dsts.contains(k)
}

/// `dest_exists` was read from `db`.
pub open spec fn exists_observed<R>(db: Map<usize, R>, dsts: Seq<usize>, dest_exists: Seq<bool>) -> bool {
    &&& dest_exists.len() == dsts.len()
    &&& forall|i: int| 0 <= i < dsts.len() ==> dest_exists[i] == db.dom().contains(#[trigger] dsts[i])
}

// ===========================================================================
// The schedule
// ===========================================================================

/// Deletes of every key in `ks`, in order.
pub open spec fn delete_all(ks: Seq<usize>) -> Seq<MigAction> {
    ks.map_values(|k: usize| MigAction::DeleteKey(k))
}

/// Deletes of the destination keys that are not sources, in item order.
pub open spec fn dest_deletes(srcs: Seq<usize>, dsts: Seq<usize>) -> Seq<MigAction>
    decreases dsts.len(),
{
    if dsts.len() == 0 {
        Seq::empty()
    } else {
        let rest = dest_deletes(srcs, dsts.drop_last());
        if srcs.contains(dsts.last()) {
            rest
        } else {
            rest.push(MigAction::DeleteKey(dsts.last()))
        }
    }
}

/// `Create(0), ..., Create(n - 1)`.
pub open spec fn create_all(n: nat) -> Seq<MigAction> {
    Seq::new(n, |i: int| MigAction::Create(i as usize))
}

/// The migration plan: delete every source, then every destination that is
/// not a source, then create every destination.
pub open spec fn spec_schedule(srcs: Seq<usize>, dsts: Seq<usize>) -> Seq<MigAction> {
    delete_all(srcs) + dest_deletes(srcs, dsts) + create_all(dsts.len())
}

/// The schedule the previous implementation ran: delete the destinations it
/// classified as conflicts, then `Delete(srcs[i]); Create(i)` per item.
pub open spec fn legacy_schedule(srcs: Seq<usize>, conflict_keys: Seq<usize>) -> Seq<MigAction> {
    delete_all(conflict_keys) + interleaved(srcs)
}

/// `Delete(srcs[0]), Create(0), Delete(srcs[1]), Create(1), ...`.
pub open spec fn interleaved(srcs: Seq<usize>) -> Seq<MigAction>
    decreases srcs.len(),
{
    if srcs.len() == 0 {
        Seq::empty()
    } else {
        interleaved(srcs.drop_last()) + seq![
            MigAction::DeleteKey(srcs.last()),
            MigAction::Create((srcs.len() - 1) as usize),
        ]
    }
}

// ===========================================================================
// Port safety
// ===========================================================================

/// No two stored rows share a port (the `UNIQUE(port)` constraint).
pub open spec fn ports_unique<R>(m: Map<usize, R>, port: spec_fn(R) -> int) -> bool {
    forall|a: usize, b: usize|
        #![trigger m[a], m[b]]
        m.dom().contains(a) && m.dom().contains(b) && a != b ==> port(m[a]) != port(m[b])
}

/// Storing `v` under `k` in `s` is a plain insert that keeps ports unique:
/// `k` is absent and no stored row has `v`'s port.
pub open spec fn insert_safe<R>(s: Map<usize, R>, k: usize, v: R, port: spec_fn(R) -> int) -> bool {
    &&& !s.dom().contains(k)
    &&& forall|j: usize| #[trigger] s.dom().contains(j) ==> port(s[j]) != port(v)
}

/// The step is safe in state `s` (deletes always are).
pub open spec fn step_safe<R>(
    s: Map<usize, R>,
    a: MigAction,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
) -> bool {
    match a {
        MigAction::DeleteKey(_) => true,
        MigAction::Create(i) => insert_safe(s, dsts[i as int], payload[i as int], port),
    }
}

/// Every step of `acts`, run from `db`, is safe.
pub open spec fn run_safe<R>(
    db: Map<usize, R>,
    acts: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
) -> bool
    decreases acts.len(),
{
    acts.len() == 0 || (run_safe(db, acts.drop_last(), dsts, payload, port) && step_safe(
        apply(db, acts.drop_last(), dsts, payload),
        acts.last(),
        dsts,
        payload,
        port,
    ))
}

// ===========================================================================
// Lemmas: running action sequences
// ===========================================================================

/// Running `a + b` runs `a`, then `b`.
pub proof fn lemma_apply_concat<R>(
    db: Map<usize, R>,
    a: Seq<MigAction>,
    b: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
)
    ensures
        apply(db, a + b, dsts, payload) == apply(apply(db, a, dsts, payload), b, dsts, payload),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        lemma_apply_concat(db, a, b.drop_last(), dsts, payload);
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
    }
}

/// Safety of `a + b` is safety of `a`, then of `b` from where `a` ends.
pub proof fn lemma_run_safe_concat<R>(
    db: Map<usize, R>,
    a: Seq<MigAction>,
    b: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
)
    ensures
        run_safe(db, a + b, dsts, payload, port) == (run_safe(db, a, dsts, payload, port)
            && run_safe(apply(db, a, dsts, payload), b, dsts, payload, port)),
    decreases b.len(),
{
    if b.len() == 0 {
        assert(a + b =~= a);
    } else {
        lemma_run_safe_concat(db, a, b.drop_last(), dsts, payload, port);
        assert((a + b).drop_last() =~= a + b.drop_last());
        assert((a + b).last() == b.last());
        lemma_apply_concat(db, a, b.drop_last(), dsts, payload);
    }
}

/// Deleting the keys `ks` removes exactly those keys.
pub proof fn lemma_apply_delete_all<R>(
    db: Map<usize, R>,
    ks: Seq<usize>,
    dsts: Seq<usize>,
    payload: Seq<R>,
)
    ensures
        forall|k: usize| #[trigger]
            apply(db, delete_all(ks), dsts, payload).dom().contains(k) <==> (db.dom().contains(k)
                && !ks.contains(k)),
        forall|k: usize| #[trigger]
            apply(db, delete_all(ks), dsts, payload).dom().contains(k) ==> apply(
                db,
                delete_all(ks),
                dsts,
                payload,
            )[k] == db[k],
    decreases ks.len(),
{
    if ks.len() > 0 {
        let prev = ks.drop_last();
        lemma_apply_delete_all(db, prev, dsts, payload);
        assert(delete_all(ks).drop_last() =~= delete_all(prev));
        assert(delete_all(ks).last() == MigAction::DeleteKey(ks.last()));
        let s = apply(db, delete_all(prev), dsts, payload);
        let t = apply(db, delete_all(ks), dsts, payload);
        assert(t == s.remove(ks.last()));
        lemma_remove_step(s, ks.last());
        assert forall|k: usize| prev.contains(k) || k == ks.last() <==> ks.contains(k) by {
            if ks.contains(k) {
                let i = choose|i: int| 0 <= i < ks.len() && ks[i] == k;
                if i < prev.len() {
                    assert(prev[i] == k);
                }
            }
            if prev.contains(k) {
                let i = choose|i: int| 0 <= i < prev.len() && prev[i] == k;
                assert(ks[i] == k);
            }
            if k == ks.last() {
                assert(ks[ks.len() - 1] == k);
            }
        }
        assert forall|k: usize| #[trigger] t.dom().contains(k) <==> (db.dom().contains(k)
            && !ks.contains(k)) by {}
        assert forall|k: usize| #[trigger] t.dom().contains(k) implies t[k] == db[k] by {}
    } else {
        assert(delete_all(ks) =~= Seq::<MigAction>::empty());
    }
}

/// Removing one key removes exactly that key.
pub proof fn lemma_remove_step<R>(s: Map<usize, R>, k0: usize)
    ensures
        forall|k: usize| #[trigger] s.remove(k0).dom().contains(k) <==> (s.dom().contains(k) && k != k0),
        forall|k: usize| #[trigger] s.remove(k0).dom().contains(k) ==> s.remove(k0)[k] == s[k],
{
}

/// Inserting one key adds exactly that key.
pub proof fn lemma_insert_step<R>(s: Map<usize, R>, k0: usize, v: R)
    ensures
        forall|k: usize| #[trigger] s.insert(k0, v).dom().contains(k) <==> (s.dom().contains(k) || k == k0),
        s.insert(k0, v)[k0] == v,
        forall|k: usize| k != k0 ==> #[trigger] s.insert(k0, v)[k] == s[k],
{
}

/// Every delete-only sequence is safe, whatever the port function.
pub proof fn lemma_deletes_safe<R>(
    db: Map<usize, R>,
    acts: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
)
    requires
        forall|j: int| 0 <= j < acts.len() ==> #[trigger] acts[j] is DeleteKey,
    ensures
        run_safe(db, acts, dsts, payload, port),
    decreases acts.len(),
{
    if acts.len() > 0 {
        lemma_deletes_safe(db, acts.drop_last(), dsts, payload, port);
        assert(acts[acts.len() - 1] is DeleteKey);
    }
}

/// Deleting the destinations that are not sources removes exactly those.
pub proof fn lemma_apply_dest_deletes<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    cdsts: Seq<usize>,
    payload: Seq<R>,
)
    ensures
        forall|k: usize| #[trigger]
            apply(db, dest_deletes(srcs, dsts), cdsts, payload).dom().contains(k) <==> (
            db.dom().contains(k) && !(dsts.contains(k) && !srcs.contains(k))),
        forall|k: usize| #[trigger]
            apply(db, dest_deletes(srcs, dsts), cdsts, payload).dom().contains(k) ==> apply(
                db,
                dest_deletes(srcs, dsts),
                cdsts,
                payload,
            )[k] == db[k],
        forall|j: int|
            0 <= j < dest_deletes(srcs, dsts).len() ==> #[trigger] dest_deletes(srcs, dsts)[j] is DeleteKey,
    decreases dsts.len(),
{
    if dsts.len() > 0 {
        let prev = dsts.drop_last();
        lemma_apply_dest_deletes(db, srcs, prev, cdsts, payload);
        let rest = dest_deletes(srcs, prev);
        let s = apply(db, rest, cdsts, payload);
        let t = apply(db, dest_deletes(srcs, dsts), cdsts, payload);
        assert forall|k: usize| prev.contains(k) || k == dsts.last() <==> dsts.contains(k) by {
            if dsts.contains(k) {
                let i = choose|i: int| 0 <= i < dsts.len() && dsts[i] == k;
                if i < prev.len() {
                    assert(prev[i] == k);
                }
            }
            if prev.contains(k) {
                let i = choose|i: int| 0 <= i < prev.len() && prev[i] == k;
                assert(dsts[i] == k);
            }
            if k == dsts.last() {
                assert(dsts[dsts.len() - 1] == k);
            }
        }
        if !srcs.contains(dsts.last()) {
            let d = rest.push(MigAction::DeleteKey(dsts.last()));
            assert(d.drop_last() =~= rest);
            assert(t == s.remove(dsts.last()));
            lemma_remove_step(s, dsts.last());
        } else {
            assert(t == s);
        }
        assert forall|k: usize| #[trigger] t.dom().contains(k) <==> (db.dom().contains(k) && !(
        dsts.contains(k) && !srcs.contains(k))) by {}
        assert forall|k: usize| #[trigger] t.dom().contains(k) implies t[k] == db[k] by {}
        assert forall|j: int| 0 <= j < dest_deletes(srcs, dsts).len() implies #[trigger] dest_deletes(
            srcs,
            dsts,
        )[j] is DeleteKey by {
            if j < rest.len() {
                assert(dest_deletes(srcs, dsts)[j] == rest[j]);
            }
        }
    }
}

/// Creating items `0..m` (with distinct destinations) on a store that holds
/// none of the destinations stores exactly payload `j` at `dsts[j]`, and
/// every step is a safe insert when ports are unique, distinct from the
/// payload ports, and the payload ports are pairwise distinct.
pub proof fn lemma_apply_creates<R>(
    d: Map<usize, R>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    m: nat,
    port: spec_fn(R) -> int,
)
    requires
        m <= dsts.len() <= usize::MAX,
        payload.len() == dsts.len(),
        distinct(dsts),
        forall|j: int| 0 <= j < dsts.len() ==> !d.dom().contains(#[trigger] dsts[j]),
    ensures
        forall|k: usize| #[trigger]
            apply(d, create_all(m), dsts, payload).dom().contains(k) <==> (d.dom().contains(k)
                || exists|j: int| 0 <= j < m && dsts[j] == k),
        forall|j: int|
            0 <= j < m ==> apply(d, create_all(m), dsts, payload)[#[trigger] dsts[j]] == payload[j],
        forall|k: usize|
            #![trigger apply(d, create_all(m), dsts, payload)[k]]
            d.dom().contains(k) ==> apply(d, create_all(m), dsts, payload)[k] == d[k],
        (forall|a: usize, b: int|
            #![trigger d[a], payload[b]]
            d.dom().contains(a) && 0 <= b < dsts.len() ==> port(d[a]) != port(payload[b]))
            && (forall|a: int, b: int|
            #![trigger payload[a], payload[b]]
            0 <= a < b < dsts.len() ==> port(payload[a]) != port(payload[b])) ==> run_safe(
            d,
            create_all(m),
            dsts,
            payload,
            port,
        ),
    decreases m,
{
    if m > 0 {
        let p = (m - 1) as nat;
        lemma_apply_creates(d, dsts, payload, p, port);
        assert(create_all(m).drop_last() =~= create_all(p));
        assert(create_all(m).last() == MigAction::Create(p as usize));
        let s = apply(d, create_all(p), dsts, payload);
        let k0 = dsts[p as int];
        let t = apply(d, create_all(m), dsts, payload);
        assert((p as usize) as int == p);
        assert(t == s.insert(k0, payload[p as int]));
        lemma_insert_step(s, k0, payload[p as int]);
        assert forall|k: usize| #[trigger]
            apply(d, create_all(m), dsts, payload).dom().contains(k) <==> (d.dom().contains(k)
                || exists|j: int| 0 <= j < m && dsts[j] == k) by {
            if exists|j: int| 0 <= j < m && dsts[j] == k {
                let j = choose|j: int| 0 <= j < m && dsts[j] == k;
                if j < p {
                    assert(exists|j2: int| 0 <= j2 < p && dsts[j2] == k);
                }
            }
            if exists|j: int| 0 <= j < p && dsts[j] == k {
                let j = choose|j: int| 0 <= j < p && dsts[j] == k;
                assert(0 <= j < m && dsts[j] == k);
            }
            if k == k0 {
                assert(0 <= p < m && dsts[p as int] == k);
            }
        }
        assert forall|j: int| 0 <= j < m implies apply(d, create_all(m), dsts, payload)[
            #[trigger] dsts[j]] == payload[j] by {
            if j < p {
                assert(dsts[j] != k0);
                assert(s.dom().contains(dsts[j]));
            }
        }
        assert forall|k: usize| d.dom().contains(k) implies #[trigger] apply(
            d,
            create_all(m),
            dsts,
            payload,
        )[k] == d[k] by {
            assert(k != k0);
        }
        if (forall|a: usize, b: int|
            #![trigger d[a], payload[b]]
            d.dom().contains(a) && 0 <= b < dsts.len() ==> port(d[a]) != port(payload[b]))
            && (forall|a: int, b: int|
            #![trigger payload[a], payload[b]]
            0 <= a < b < dsts.len() ==> port(payload[a]) != port(payload[b])) {
            assert(!s.dom().contains(k0)) by {
                if s.dom().contains(k0) {
                    let j = choose|j: int| 0 <= j < p && dsts[j] == k0;
                    assert(dsts[j] == dsts[p as int]);
                }
            }
            assert forall|k: usize| #[trigger] s.dom().contains(k) implies port(s[k]) != port(
                payload[p as int],
            ) by {
                if d.dom().contains(k) {
                    assert(s[k] == d[k]);
                    assert(port(d[k]) != port(payload[p as int]));
                } else {
                    let j = choose|j: int| 0 <= j < p && dsts[j] == k;
                    assert(s[dsts[j]] == payload[j]);
                    assert(port(payload[j]) != port(payload[p as int]));
                }
            }
            assert(step_safe(s, MigAction::Create(p as usize), dsts, payload, port));
        }
    } else {
        assert(create_all(m) =~= Seq::<MigAction>::empty());
    }
}

// ===========================================================================
// Theorems about the schedule
// ===========================================================================

/// The state after the deletes of the schedule: everything except the
/// sources and the destinations.
pub proof fn lemma_schedule_deletes<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    payload: Seq<R>,
)
    ensures
        ({
            let d = apply(db, delete_all(srcs) + dest_deletes(srcs, dsts), dsts, payload);
            &&& forall|k: usize| #[trigger]
                d.dom().contains(k) <==> (db.dom().contains(k) && !srcs.contains(k)
                    && !dsts.contains(k))
            &&& forall|k: usize| #[trigger] d.dom().contains(k) ==> d[k] == db[k]
        }),
        forall|j: int|
            0 <= j < (delete_all(srcs) + dest_deletes(srcs, dsts)).len() ==> #[trigger] (delete_all(
                srcs,
            ) + dest_deletes(srcs, dsts))[j] is DeleteKey,
{
    let d1 = apply(db, delete_all(srcs), dsts, payload);
    lemma_apply_delete_all(db, srcs, dsts, payload);
    lemma_apply_dest_deletes(d1, srcs, dsts, dsts, payload);
    lemma_apply_concat(db, delete_all(srcs), dest_deletes(srcs, dsts), dsts, payload);
    let ds = delete_all(srcs) + dest_deletes(srcs, dsts);
    assert forall|j: int| 0 <= j < ds.len() implies #[trigger] ds[j] is DeleteKey by {
        if j < srcs.len() {
            assert(ds[j] == MigAction::DeleteKey(srcs[j]));
        } else {
            assert(ds[j] == dest_deletes(srcs, dsts)[j - srcs.len()]);
        }
    }
}

/// Correctness: for every store, running the schedule produces exactly the
/// intended result `moved` — the payloads at their destinations (overwriting
/// whatever was there), the other sources gone, every other row untouched.
pub proof fn lemma_schedule_correct<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    payload: Seq<R>,
)
    requires
        srcs.len() == dsts.len() <= usize::MAX,
        payload.len() == dsts.len(),
        distinct(dsts),
    ensures
        apply(db, spec_schedule(srcs, dsts), dsts, payload) == moved(db, srcs, dsts, payload),
{
    let ds = delete_all(srcs) + dest_deletes(srcs, dsts);
    lemma_schedule_deletes(db, srcs, dsts, payload);
    let d = apply(db, ds, dsts, payload);
    assert forall|j: int| 0 <= j < dsts.len() implies !d.dom().contains(#[trigger] dsts[j]) by {
        assert(dsts.contains(dsts[j]));
    }
    lemma_apply_creates(d, dsts, payload, dsts.len(), |r: R| 0int);
    lemma_apply_concat(db, ds, create_all(dsts.len()), dsts, payload);
    let out = apply(db, spec_schedule(srcs, dsts), dsts, payload);
    let want = moved(db, srcs, dsts, payload);
    assert forall|k: usize| out.dom().contains(k) <==> want.dom().contains(k) by {
        if dsts.contains(k) {
            let j = choose|j: int| 0 <= j < dsts.len() && dsts[j] == k;
            assert(dsts.to_set().contains(k));
        }
        if dsts.to_set().contains(k) {
            assert(dsts.contains(k));
        }
    }
    assert(out.dom() =~= want.dom());
    assert forall|k: usize| out.dom().contains(k) implies #[trigger] out[k] == want[k] by {
        if dsts.contains(k) {
            let j = dsts.index_of(k);
            assert(0 <= j < dsts.len() && dsts[j] == k);
            assert(out[dsts[j]] == payload[j]);
        } else {
            assert(d.dom().contains(k));
        }
    }
    assert(out =~= want);
}

/// Port safety: if the store's ports are unique and the payloads are the
/// current source rows, every step of the schedule is safe — no create meets
/// an existing row with its port or an occupied key — so `UNIQUE(port)` and
/// the primary key never abort the transaction, and the result again has
/// unique ports.
pub proof fn lemma_schedule_port_safe<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
)
    requires
        srcs.len() == dsts.len() <= usize::MAX,
        distinct(srcs),
        distinct(dsts),
        snapshot(db, srcs, payload),
        ports_unique(db, port),
    ensures
        run_safe(db, spec_schedule(srcs, dsts), dsts, payload, port),
        ports_unique(apply(db, spec_schedule(srcs, dsts), dsts, payload), port),
{
    let ds = delete_all(srcs) + dest_deletes(srcs, dsts);
    lemma_schedule_deletes(db, srcs, dsts, payload);
    let d = apply(db, ds, dsts, payload);
    lemma_deletes_safe(db, ds, dsts, payload, port);
    assert forall|j: int| 0 <= j < dsts.len() implies !d.dom().contains(#[trigger] dsts[j]) by {
        assert(dsts.contains(dsts[j]));
    }
    assert forall|a: usize, b: int|
        #![trigger d[a], payload[b]]
        d.dom().contains(a) && 0 <= b < dsts.len() implies port(d[a]) != port(payload[b]) by {
        assert(srcs.contains(srcs[b]));
        assert(a != srcs[b]);
        assert(db.dom().contains(srcs[b]));
        assert(db[a] == d[a]);
        assert(payload[b] == db[srcs[b]]);
    }
    assert forall|a: int, b: int|
        #![trigger payload[a], payload[b]]
        0 <= a < b < dsts.len() implies port(payload[a]) != port(payload[b]) by {
        assert(srcs[a] != srcs[b]);
        assert(db.dom().contains(srcs[a]));
        assert(db.dom().contains(srcs[b]));
        assert(payload[a] == db[srcs[a]]);
        assert(payload[b] == db[srcs[b]]);
    }
    lemma_apply_creates(d, dsts, payload, dsts.len(), port);
    lemma_run_safe_concat(db, ds, create_all(dsts.len()), dsts, payload, port);
    lemma_safe_run_keeps_ports_unique(db, spec_schedule(srcs, dsts), dsts, payload, port);
}

/// A safe run keeps ports unique at every step.
pub proof fn lemma_safe_run_keeps_ports_unique<R>(
    db: Map<usize, R>,
    acts: Seq<MigAction>,
    dsts: Seq<usize>,
    payload: Seq<R>,
    port: spec_fn(R) -> int,
)
    requires
        ports_unique(db, port),
        run_safe(db, acts, dsts, payload, port),
    ensures
        ports_unique(apply(db, acts, dsts, payload), port),
    decreases acts.len(),
{
    if acts.len() > 0 {
        lemma_safe_run_keeps_ports_unique(db, acts.drop_last(), dsts, payload, port);
        let s = apply(db, acts.drop_last(), dsts, payload);
        let t = apply(db, acts, dsts, payload);
        match acts.last() {
            MigAction::DeleteKey(k) => {
                assert forall|a: usize, b: usize|
                    #![trigger t[a], t[b]]
                    t.dom().contains(a) && t.dom().contains(b) && a != b implies port(t[a]) != port(
                        t[b],
                    ) by {
                    assert(s.dom().contains(a) && s.dom().contains(b));
                    assert(t[a] == s[a] && t[b] == s[b]);
                }
            },
            MigAction::Create(i) => {
                let k = dsts[i as int];
                assert forall|a: usize, b: usize|
                    #![trigger t[a], t[b]]
                    t.dom().contains(a) && t.dom().contains(b) && a != b implies port(t[a]) != port(
                        t[b],
                    ) by {
                    if a == k {
                        assert(s.dom().contains(b));
                        assert(port(s[b]) != port(payload[i as int]));
                    } else if b == k {
                        assert(s.dom().contains(a));
                        assert(port(s[a]) != port(payload[i as int]));
                    } else {
                        assert(t[a] == s[a] && t[b] == s[b]);
                    }
                }
            },
        }
    }
}

/// Shape of the schedule: every delete comes before every create, the
/// creates are `Create(0), ..., Create(n - 1)` (each item exactly once), the
/// deleted keys are exactly the sources and the destinations, and no key is
/// deleted twice when the source and destination lists are distinct.
pub proof fn lemma_schedule_shape(srcs: Seq<usize>, dsts: Seq<usize>)
    requires
        dsts.len() <= usize::MAX,
        distinct(srcs),
        distinct(dsts),
    ensures
        ({
            let acts = spec_schedule(srcs, dsts);
            let p = acts.len() - dsts.len();
            &&& p >= 0
            &&& forall|j: int| 0 <= j < p ==> #[trigger] acts[j] is DeleteKey
            &&& forall|j: int| p <= j < acts.len() ==> #[trigger] acts[j] == MigAction::Create(
                (j - p) as usize,
            )
            &&& forall|k: usize|
                (exists|j: int| 0 <= j < p && #[trigger] acts[j] == MigAction::DeleteKey(k)) <==> (
                srcs.contains(k) || dsts.contains(k))
            &&& forall|j: int, l: int|
                0 <= j < l < p ==> #[trigger] acts[j] != #[trigger] acts[l]
        }),
{
    let ds = delete_all(srcs) + dest_deletes(srcs, dsts);
    let acts = spec_schedule(srcs, dsts);
    lemma_schedule_deletes(Map::<usize, int>::empty(), srcs, dsts, Seq::<int>::empty());
    lemma_dest_deletes_members(srcs, dsts);
    let n = srcs.len() as int;
    let dd = dest_deletes(srcs, dsts);
    assert(acts.len() == ds.len() + dsts.len());
    assert forall|j: int| ds.len() <= j < acts.len() implies #[trigger] acts[j] == MigAction::Create(
        (j - ds.len()) as usize,
    ) by {
        assert(acts[j] == create_all(dsts.len())[j - ds.len()]);
    }
    assert forall|j: int| 0 <= j < ds.len() implies #[trigger] acts[j] == ds[j] by {}
    assert forall|k: usize|
        (exists|j: int| 0 <= j < ds.len() && #[trigger] acts[j] == MigAction::DeleteKey(k)) <==> (
        srcs.contains(k) || dsts.contains(k)) by {
        if exists|j: int| 0 <= j < ds.len() && #[trigger] acts[j] == MigAction::DeleteKey(k) {
            let j = choose|j: int| 0 <= j < ds.len() && #[trigger] acts[j] == MigAction::DeleteKey(k);
            if j < n {
                assert(ds[j] == MigAction::DeleteKey(srcs[j]));
                assert(srcs[j] == k);
            } else {
                assert(ds[j] == dd[j - n]);
                assert(dd[j - n] == MigAction::DeleteKey(k));
            }
        }
        if srcs.contains(k) {
            let j = choose|j: int| 0 <= j < n && srcs[j] == k;
            assert(acts[j] == ds[j]);
            assert(ds[j] == MigAction::DeleteKey(k));
        } else if dsts.contains(k) {
            let l = choose|l: int| 0 <= l < dd.len() && dd[l] == MigAction::DeleteKey(k);
            assert(acts[n + l] == ds[n + l]);
            assert(ds[n + l] == dd[l]);
        }
    }
    assert forall|j: int, l: int| 0 <= j < l < ds.len() implies #[trigger] acts[j]
        != #[trigger] acts[l] by {
        assert(acts[j] == ds[j]);
        assert(acts[l] == ds[l]);
        if l < n {
            assert(ds[j] == MigAction::DeleteKey(srcs[j]));
            assert(ds[l] == MigAction::DeleteKey(srcs[l]));
        } else if j < n {
            assert(ds[j] == MigAction::DeleteKey(srcs[j]));
            assert(ds[l] == dd[l - n]);
            assert(srcs.contains(srcs[j]));
        } else {
            assert(ds[j] == dd[j - n]);
            assert(ds[l] == dd[l - n]);
        }
    }
}

/// The destination deletes are distinct deletes of exactly the destinations
/// that are not sources.
pub proof fn lemma_dest_deletes_members(srcs: Seq<usize>, dsts: Seq<usize>)
    requires
        distinct(dsts),
    ensures
        forall|l: int|
            #![trigger dest_deletes(srcs, dsts)[l]]
            0 <= l < dest_deletes(srcs, dsts).len() ==> exists|i: int|
                #![trigger dsts[i]]
                0 <= i < dsts.len() && !srcs.contains(dsts[i]) && dest_deletes(srcs, dsts)[l]
                    == MigAction::DeleteKey(dsts[i]),
        forall|k: usize|
            (dsts.contains(k) && !srcs.contains(k)) ==> exists|l: int|
                0 <= l < dest_deletes(srcs, dsts).len() && #[trigger] dest_deletes(srcs, dsts)[l]
                    == MigAction::DeleteKey(k),
        forall|j: int, l: int|
            0 <= j < l < dest_deletes(srcs, dsts).len() ==> #[trigger] dest_deletes(srcs, dsts)[j]
                != #[trigger] dest_deletes(srcs, dsts)[l],
    decreases dsts.len(),
{
    if dsts.len() > 0 {
        let prev = dsts.drop_last();
        assert(distinct(prev));
        lemma_dest_deletes_members(srcs, prev);
        let rest = dest_deletes(srcs, prev);
        let last = dsts.last();
        let cur = dest_deletes(srcs, dsts);
        assert forall|l: int| #![trigger cur[l]] 0 <= l < cur.len() implies exists|i: int|
            #![trigger dsts[i]]
            0 <= i < dsts.len() && !srcs.contains(dsts[i]) && cur[l] == MigAction::DeleteKey(dsts[i]) by {
            if l < rest.len() {
                assert(cur[l] == rest[l]);
                let i = choose|i: int|
                    #![trigger prev[i]]
                    0 <= i < prev.len() && !srcs.contains(prev[i]) && rest[l] == MigAction::DeleteKey(
                        prev[i],
                    );
                assert(dsts[i] == prev[i]);
            } else {
                assert(cur[l] == MigAction::DeleteKey(last));
                assert(dsts[dsts.len() - 1] == last);
            }
        }
        assert forall|k: usize| (dsts.contains(k) && !srcs.contains(k)) implies exists|l: int|
            0 <= l < cur.len() && #[trigger] cur[l] == MigAction::DeleteKey(k) by {
            let i = choose|i: int| 0 <= i < dsts.len() && dsts[i] == k;
            if i < prev.len() {
                assert(prev[i] == k);
                assert(prev.contains(k));
                let l = choose|l: int| 0 <= l < rest.len() && #[trigger] rest[l] == MigAction::DeleteKey(k);
                assert(cur[l] == rest[l]);
            } else {
                assert(cur[cur.len() - 1] == MigAction::DeleteKey(k));
            }
        }
        assert forall|j: int, l: int| 0 <= j < l < cur.len() implies #[trigger] cur[j]
            != #[trigger] cur[l] by {
            if l < rest.len() {
                assert(cur[j] == rest[j] && cur[l] == rest[l]);
            } else {
                assert(cur[l] == MigAction::DeleteKey(last));
                assert(cur[j] == rest[j]);
                let i = choose|i: int|
                    #![trigger prev[i]]
                    0 <= i < prev.len() && !srcs.contains(prev[i]) && rest[j] == MigAction::DeleteKey(
                        prev[i],
                    );
                assert(prev[i] == dsts[i]);
                assert(dsts[i] != dsts[dsts.len() - 1]);
            }
        }
    }
}

// ===========================================================================
// Theorems about conflicts
// ===========================================================================

/// Under a consistent snapshot, item `i` conflicts exactly when its
/// destination's current row is overwritten, the overwritten keys are
/// exactly the conflicting destinations, and every row that is neither a
/// source nor overwritten survives unchanged. So a migration without
/// conflicts replaces no existing reservation.
pub proof fn lemma_conflicts_are_overwrites<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dsts: Seq<usize>,
    dest_exists: Seq<bool>,
    payload: Seq<R>,
)
    requires
        exists_observed(db, dsts, dest_exists),
    ensures
        forall|i: int|
            0 <= i < dsts.len() ==> (is_conflict(srcs, dsts, dest_exists, i) <==> overwritten(
                db,
                srcs,
                dsts,
                #[trigger] dsts[i],
            )),
        forall|k: usize|
            overwritten(db, srcs, dsts, k) ==> exists|i: int|
                0 <= i < dsts.len() && #[trigger] dsts[i] == k && is_conflict(srcs, dsts, dest_exists, i),
        forall|k: usize|
            #![trigger moved(db, srcs, dsts, payload)[k]]
            db.dom().contains(k) && !srcs.contains(k) && !overwritten(db, srcs, dsts, k) ==> moved(
                db,
                srcs,
                dsts,
                payload,
            ).dom().contains(k) && moved(db, srcs, dsts, payload)[k] == db[k],
        (forall|i: int| 0 <= i < dsts.len() ==> !is_conflict(srcs, dsts, dest_exists, i)) ==> (forall|
            k: usize,
        |
            #![trigger moved(db, srcs, dsts, payload)[k]]
            db.dom().contains(k) && !srcs.contains(k) ==> moved(db, srcs, dsts, payload).dom().contains(
                k,
            ) && moved(db, srcs, dsts, payload)[k] == db[k]),
{
    let want = moved(db, srcs, dsts, payload);
    assert forall|i: int| 0 <= i < dsts.len() implies (is_conflict(srcs, dsts, dest_exists, i)
        <==> overwritten(db, srcs, dsts, #[trigger] dsts[i])) by {
        assert(dsts.contains(dsts[i]));
    }
    assert forall|k: usize| overwritten(db, srcs, dsts, k) implies exists|i: int|
        0 <= i < dsts.len() && #[trigger] dsts[i] == k && is_conflict(srcs, dsts, dest_exists, i) by {
        let i = choose|i: int| 0 <= i < dsts.len() && dsts[i] == k;
        assert(dest_exists[i] == db.dom().contains(dsts[i]));
    }
    assert forall|k: usize|
        db.dom().contains(k) && !srcs.contains(k) && !overwritten(db, srcs, dsts, k) implies want.dom().contains(k)
        && #[trigger] want[k] == db[k] by {
        assert(!dsts.contains(k));
        assert(db.dom().filter(|k: usize| !srcs.contains(k)).contains(k));
    }
    if forall|i: int| 0 <= i < dsts.len() ==> !is_conflict(srcs, dsts, dest_exists, i) {
        assert forall|k: usize| db.dom().contains(k) && !srcs.contains(k) implies want.dom().contains(k)
            && #[trigger] want[k] == db[k] by {
            if dsts.contains(k) {
                let i = choose|i: int| 0 <= i < dsts.len() && dsts[i] == k;
                assert(dest_exists[i] == db.dom().contains(dsts[i]));
                assert(!is_conflict(srcs, dsts, dest_exists, i));
            }
            assert(db.dom().filter(|k: usize| !srcs.contains(k)).contains(k));
        }
    }
}

/// When no destination is also a source (the usual migration between
/// disjoint trees), a conflict is exactly an existing destination.
pub proof fn lemma_disjoint_conflicts(srcs: Seq<usize>, dsts: Seq<usize>, dest_exists: Seq<bool>)
    requires
        forall|i: int| 0 <= i < dsts.len() ==> !srcs.contains(#[trigger] dsts[i]),
    ensures
        forall|i: int|
            0 <= i < dsts.len() ==> #[trigger] is_conflict(srcs, dsts, dest_exists, i) == dest_exists[i],
{
}

/// Migrating every key onto itself (`migrate --from P --to P`) has no
/// conflicts, whatever exists, and leaves the store unchanged.
pub proof fn lemma_self_migration_is_identity<R>(
    db: Map<usize, R>,
    srcs: Seq<usize>,
    dest_exists: Seq<bool>,
    payload: Seq<R>,
)
    requires
        snapshot(db, srcs, payload),
    ensures
        forall|i: int| 0 <= i < srcs.len() ==> !#[trigger] is_conflict(srcs, srcs, dest_exists, i),
        moved(db, srcs, srcs, payload) == db,
{
    let want = moved(db, srcs, srcs, payload);
    assert forall|i: int| 0 <= i < srcs.len() implies !#[trigger] is_conflict(
        srcs,
        srcs,
        dest_exists,
        i,
    ) by {
        assert(srcs.contains(srcs[i]));
    }
    assert forall|k: usize| want.dom().contains(k) <==> db.dom().contains(k) by {
        if srcs.contains(k) {
            let i = choose|i: int| 0 <= i < srcs.len() && srcs[i] == k;
            assert(srcs.to_set().contains(srcs[i]));
        }
        if srcs.to_set().contains(k) {
            assert(srcs.contains(k));
        }
    }
    assert(want.dom() =~= db.dom());
    assert forall|k: usize| want.dom().contains(k) implies #[trigger] want[k] == db[k] by {
        if srcs.contains(k) {
            let j = srcs.index_of(k);
            assert(payload[j] == db[srcs[j]]);
        }
    }
    assert(want =~= db);
}

/// The previous schedule loses data. With rows `0 -> 10` and `1 -> 11`,
/// moving `0 -> 1` and `1 -> 2` (destination inside the source tree), it
/// first deletes the "conflict" `1`, then deletes `0` and creates `1`, then
/// deletes `1` again — the row it just created — and creates `2`. Row `10`
/// is lost, although the intended result keeps it at `1`. The schedule of
/// this kernel produces the intended result for the same input.
pub proof fn lemma_legacy_schedule_loses_data()
    ensures
        ({
            let db = Map::<usize, int>::empty().insert(0, 10).insert(1, 11);
            let srcs = seq![0usize, 1usize];
            let dsts = seq![1usize, 2usize];
            let payload = seq![10int, 11int];
            &&& snapshot(db, srcs, payload)
            &&& distinct(srcs) && distinct(dsts)
            &&& !apply(db, legacy_schedule(srcs, seq![1usize]), dsts, payload).dom().contains(1)
            &&& moved(db, srcs, dsts, payload).dom().contains(1)
            &&& moved(db, srcs, dsts, payload)[1] == 10
            &&& apply(db, spec_schedule(srcs, dsts), dsts, payload) == moved(db, srcs, dsts, payload)
        }),
{
    let db = Map::<usize, int>::empty().insert(0, 10).insert(1, 11);
    let srcs = seq![0usize, 1usize];
    let dsts = seq![1usize, 2usize];
    let payload = seq![10int, 11int];
    let d0 = MigAction::DeleteKey(0);
    let d1 = MigAction::DeleteKey(1);
    let c0 = MigAction::Create(0);
    let c1 = MigAction::Create(1);
    assert(srcs.drop_last() =~= seq![0usize]);
    assert(srcs.last() == 1usize);
    assert(seq![0usize].drop_last() =~= Seq::<usize>::empty());
    assert(seq![0usize].last() == 0usize);
    assert(interleaved(Seq::<usize>::empty()) =~= Seq::<MigAction>::empty());
    assert(Seq::<MigAction>::empty() + seq![d0, c0] =~= seq![d0, c0]);
    assert(interleaved(seq![0usize]) =~= seq![d0, c0]);
    assert(interleaved(srcs) =~= seq![d0, c0, d1, c1]);
    assert(delete_all(seq![1usize]) =~= seq![d1]);
    let legacy = legacy_schedule(srcs, seq![1usize]);
    assert(legacy =~= seq![d1, d0, c0, d1, c1]);
    assert(legacy.drop_last() =~= seq![d1, d0, c0, d1]);
    assert(seq![d1, d0, c0, d1].drop_last() =~= seq![d1, d0, c0]);
    assert(seq![d1, d0, c0].drop_last() =~= seq![d1, d0]);
    assert(seq![d1, d0].drop_last() =~= seq![d1]);
    assert(seq![d1].drop_last() =~= Seq::<MigAction>::empty());
    assert(seq![d1].last() == d1);
    assert(seq![d1, d0].last() == d0);
    assert(seq![d1, d0, c0].last() == c0);
    assert(seq![d1, d0, c0, d1].last() == d1);
    assert(legacy.last() == c1);
    assert(apply(db, Seq::<MigAction>::empty(), dsts, payload) == db);
    let s1 = apply(db, seq![d1], dsts, payload);
    let s2 = apply(db, seq![d1, d0], dsts, payload);
    let s3 = apply(db, seq![d1, d0, c0], dsts, payload);
    let s4 = apply(db, seq![d1, d0, c0, d1], dsts, payload);
    let s5 = apply(db, legacy, dsts, payload);
    assert(s1 == db.remove(1));
    assert(s2 == s1.remove(0));
    assert(s3 == s2.insert(1, 10));
    assert(s4 == s3.remove(1));
    assert(s5 == s4.insert(2, 11));
    assert(!s4.dom().contains(1));
    assert(!s5.dom().contains(1));
    assert(srcs[0] == 0 && srcs[1] == 1);
    assert(dsts.contains(1usize)) by {
        assert(dsts[0] == 1);
    }
    let want = moved(db, srcs, dsts, payload);
    assert(dsts.to_set().contains(1usize));
    assert(want.dom().contains(1));
    assert(dsts.index_of(1usize) == 0);
    assert(want[1] == 10);
    assert(snapshot(db, srcs, payload)) by {
        assert(db.dom().contains(0usize) && db.dom().contains(1usize));
        assert(db[0usize] == 10 && db[1usize] == 11);
    }
    lemma_schedule_correct(db, srcs, dsts, payload);
}

// ===========================================================================
// Executable kernels
// ===========================================================================

/// Why key lists do not describe a migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationKeyError {
    /// The lists have different lengths.
    LengthMismatch,
    /// A key id is not below `key_count`.
    KeyOutOfRange,
    /// Items `first < second` have the same source key.
    DuplicateSource {
        /// The earlier item.
        first: usize,
        /// The later item.
        second: usize,
    },
    /// Items `first < second` have the same destination key.
    DuplicateDest {
        /// The earlier item.
        first: usize,
        /// The later item.
        second: usize,
    },
}

/// Why `index_keys` failed.
enum IndexIssue {
    OutOfRange,
    Duplicate(usize, usize),
}

/// `pos` is the inverse of `ids` on `0..key_count`.
pub open spec fn indexes(pos: Seq<Option<usize>>, ids: Seq<usize>, key_count: usize) -> bool {
    &&& pos.len() == key_count
    &&& forall|k: int|
        0 <= k < key_count ==> match #[trigger] pos[k] {
            Some(i) => i < ids.len() && ids[i as int] == k,
            None => !ids.contains(k as usize),
        }
}

/// Builds the inverse of `ids` on `0..key_count`, checking that every id is
/// in range and none repeats.
fn index_keys(key_count: usize, ids: &[usize]) -> (r: Result<Vec<Option<usize>>, IndexIssue>)
    ensures
        r matches Ok(pos) ==> in_range(ids@, key_count) && distinct(ids@) && indexes(
            pos@,
            ids@,
            key_count,
        ),
        r matches Err(IndexIssue::OutOfRange) ==> !in_range(ids@, key_count),
        r matches Err(IndexIssue::Duplicate(first, second)) ==> first < second < ids.len()
            && ids@[first as int] == ids@[second as int],
{
    let mut pos: Vec<Option<usize>> = Vec::new();
    while pos.len() < key_count
        invariant
            pos.len() <= key_count,
            forall|k: int| 0 <= k < pos.len() ==> #[trigger] pos@[k] is None,
        decreases key_count - pos.len(),
    {
        pos.push(None);
    }
    let mut i: usize = 0;
    while i < ids.len()
        invariant
            i <= ids.len(),
            pos.len() == key_count,
            in_range(ids@.take(i as int), key_count),
            distinct(ids@.take(i as int)),
            forall|k: int|
                0 <= k < key_count ==> match #[trigger] pos@[k] {
                    Some(j) => j < i && ids@[j as int] == k,
                    None => forall|j: int| 0 <= j < i ==> ids@[j] != k,
                },
        decreases ids.len() - i,
    {
        let k = ids[i];
        if k >= key_count {
            assert(!in_range(ids@, key_count)) by {
                assert(ids@[i as int] >= key_count);
            }
            return Err(IndexIssue::OutOfRange);
        }
        if let Some(first) = pos[k] {
            return Err(IndexIssue::Duplicate(first, i));
        }
        proof {
            let t = ids@.take(i + 1);
            assert forall|a: int, b: int|
                #![trigger t[a], t[b]]
                0 <= a < b < i + 1 implies t[a] != t[b] by {
                if b < i {
                    assert(ids@.take(i as int)[a] == ids@[a]);
                    assert(ids@.take(i as int)[b] == ids@[b]);
                } else {
                    assert(pos@[k as int] is None);
                }
            }
            assert forall|a: int| 0 <= a < i + 1 implies #[trigger] t[a] < key_count by {
                if a < i {
                    assert(ids@.take(i as int)[a] == ids@[a]);
                }
            }
        }
        pos[k] = Some(i);
        i += 1;
    }
    proof {
        assert(ids@.take(ids.len() as int) =~= ids@);
        assert forall|k: int| 0 <= k < key_count implies match #[trigger] pos@[k] {
            Some(j) => j < ids.len() && ids@[j as int] == k,
            None => !ids@.contains(k as usize),
        } by {
            if pos@[k] is None {
                if ids@.contains(k as usize) {
                    let j = choose|j: int| 0 <= j < ids.len() && ids@[j] == k as usize;
                    assert(ids@[j] != k);
                }
            }
        }
    }
    Ok(pos)
}

/// Checks that the key lists describe a migration and returns the inverse
/// of the source list.
fn check_keys(key_count: usize, sources: &[usize], dests: &[usize]) -> (r: Result<
    Vec<Option<usize>>,
    MigrationKeyError,
>)
    ensures
        r is Ok <==> well_formed(key_count, sources@, dests@),
        r matches Ok(pos) ==> indexes(pos@, sources@, key_count),
        sources.len() != dests.len() ==> r == Err::<Vec<Option<usize>>, MigrationKeyError>(
            MigrationKeyError::LengthMismatch,
        ),
        r matches Err(MigrationKeyError::LengthMismatch) ==> sources.len() != dests.len(),
        r matches Err(MigrationKeyError::KeyOutOfRange) ==> sources.len() == dests.len() && !(
        in_range(sources@, key_count) && in_range(dests@, key_count)),
        r matches Err(MigrationKeyError::DuplicateSource { first, second }) ==> sources.len()
            == dests.len() && first < second < sources.len() && sources@[first as int]
            == sources@[second as int],
        r matches Err(MigrationKeyError::DuplicateDest { first, second }) ==> sources.len()
            == dests.len() && in_range(sources@, key_count) && distinct(sources@) && first
            < second < dests.len() && dests@[first as int] == dests@[second as int],
{
    if sources.len() != dests.len() {
        return Err(MigrationKeyError::LengthMismatch);
    }
    let src_pos = match index_keys(key_count, sources) {
        Ok(pos) => pos,
        Err(IndexIssue::OutOfRange) => return Err(MigrationKeyError::KeyOutOfRange),
        Err(IndexIssue::Duplicate(first, second)) => {
            return Err(MigrationKeyError::DuplicateSource { first, second });
        },
    };
    match index_keys(key_count, dests) {
        Ok(_) => Ok(src_pos),
        Err(IndexIssue::OutOfRange) => Err(MigrationKeyError::KeyOutOfRange),
        Err(IndexIssue::Duplicate(first, second)) => Err(
            MigrationKeyError::DuplicateDest { first, second },
        ),
    }
}

/// Classifies the items of a migration: returns, in increasing order, the
/// items whose destination already holds a reservation that is not itself
/// being moved (`is_conflict`). Fails exactly when the key lists do not
/// describe a migration or `dest_exists` has the wrong length.
///
/// The code checks the lengths, then scans the sources left to right and
/// reports the first out-of-range id or repeat it meets, then the
/// destinations the same way. The postconditions pin `LengthMismatch` and
/// tie each other error to its cause; `DuplicateDest` requires in-range,
/// distinct sources. They do not fix the choice between `KeyOutOfRange`
/// and `DuplicateSource` when a source repeats and some id is out of range,
/// or between `KeyOutOfRange` and `DuplicateDest` when, with valid sources,
/// the destinations hold both an out-of-range id and a repeat.
pub fn classify_conflicts(
    key_count: usize,
    sources: &[usize],
    dests: &[usize],
    dest_exists: &[bool],
) -> (r: Result<Vec<usize>, MigrationKeyError>)
    ensures
        r is Ok <==> (well_formed(key_count, sources@, dests@) && dest_exists.len() == dests.len()),
        r matches Ok(c) ==> conflict_list(c@, sources@, dests@, dest_exists@),
        // Every error names its cause, checked in this order: lengths, then
        // the sources (range, repeats), then the destinations.
        (sources.len() != dests.len() || dest_exists.len() != dests.len()) ==> r == Err::<
            Vec<usize>,
            MigrationKeyError,
        >(MigrationKeyError::LengthMismatch),
        r matches Err(MigrationKeyError::LengthMismatch) ==> sources.len() != dests.len()
            || dest_exists.len() != dests.len(),
        r matches Err(MigrationKeyError::KeyOutOfRange) ==> sources.len() == dests.len() && !(
        in_range(sources@, key_count) && in_range(dests@, key_count)),
        r matches Err(MigrationKeyError::DuplicateSource { first, second }) ==> sources.len()
            == dests.len() && first < second < sources.len() && sources@[first as int]
            == sources@[second as int],
        r matches Err(MigrationKeyError::DuplicateDest { first, second }) ==> sources.len()
            == dests.len() && in_range(sources@, key_count) && distinct(sources@) && first
            < second < dests.len() && dests@[first as int] == dests@[second as int],
{
    if dest_exists.len() != dests.len() {
        return Err(MigrationKeyError::LengthMismatch);
    }
    let src_pos = check_keys(key_count, sources, dests)?;
    let mut c: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < dests.len()
        invariant
            i <= dests.len(),
            dest_exists.len() == dests.len(),
            well_formed(key_count, sources@, dests@),
            indexes(src_pos@, sources@, key_count),
            forall|j: int, l: int| 0 <= j < l < c.len() ==> c@[j] < c@[l],
            forall|j: int| 0 <= j < c.len() ==> #[trigger] c@[j] < i,
            forall|l: int|
                0 <= l < i ==> (c@.contains(l as usize) <==> is_conflict(
                    sources@,
                    dests@,
                    dest_exists@,
                    l,
                )),
        decreases dests.len() - i,
    {
        let k = dests[i];
        proof {
            assert(k < key_count);
        }
        let is_source = src_pos[k].is_some();
        proof {
            if is_source {
                let j = src_pos@[k as int]->Some_0;
                assert(sources@[j as int] == k);
                assert(sources@.contains(k));
            }
        }
        let ghost before = c@;
        if dest_exists[i] && !is_source {
            c.push(i);
        }
        proof {
            assert forall|l: int| 0 <= l < i + 1 implies (c@.contains(l as usize) <==> is_conflict(
                sources@,
                dests@,
                dest_exists@,
                l,
            )) by {
                if l < i {
                    if c@.contains(l as usize) {
                        let j = choose|j: int| 0 <= j < c@.len() && c@[j] == l as usize;
                        if j < before.len() {
                            assert(before[j] == l);
                        }
                    }
                    if before.contains(l as usize) {
                        let j = choose|j: int| 0 <= j < before.len() && before[j] == l as usize;
                        assert(c@[j] == l);
                    }
                } else {
                    if c@.contains(l as usize) && c@.len() == before.len() {
                        let j = choose|j: int| 0 <= j < c@.len() && c@[j] == l as usize;
                        assert(before[j] < i);
                    }
                    if c@.len() > before.len() {
                        assert(c@[before.len() as int] == i);
                    }
                }
            }
        }
        i += 1;
    }
    Ok(c)
}

/// The migration plan for well-formed key lists: exactly `spec_schedule`
/// (all deletes, then every create once; see `lemma_schedule_correct`,
/// `lemma_schedule_port_safe`, and `lemma_schedule_shape`). Fails exactly
/// when the key lists do not describe a migration, with errors checked and
/// specified as for `classify_conflicts`.
pub fn schedule_migration(key_count: usize, sources: &[usize], dests: &[usize]) -> (r: Result<
    Vec<MigAction>,
    MigrationKeyError,
>)
    ensures
        r is Ok <==> well_formed(key_count, sources@, dests@),
        r matches Ok(acts) ==> acts@ == spec_schedule(sources@, dests@),
        // Correctness over every store (rows abstracted as ints): running the
        // plan yields exactly the intended result.
        r matches Ok(acts) ==> forall|db: Map<usize, int>, payload: Seq<int>|
            payload.len() == dests.len() ==> #[trigger] apply(db, acts@, dests@, payload) == moved(
                db,
                sources@,
                dests@,
                payload,
            ),
        // Port safety: from a store with unique ports, with the payloads read
        // from it, every step is a safe insert or a delete.
        r matches Ok(acts) ==> forall|db: Map<usize, int>, payload: Seq<int>, port: spec_fn(int) -> int|
            snapshot(db, sources@, payload) && ports_unique(db, port) ==> #[trigger] run_safe(
                db,
                acts@,
                dests@,
                payload,
                port,
            ),
        // Every error names its cause, checked in this order: lengths, then
        // the sources (range, repeats), then the destinations.
        sources.len() != dests.len() ==> r == Err::<Vec<MigAction>, MigrationKeyError>(
            MigrationKeyError::LengthMismatch,
        ),
        r matches Err(MigrationKeyError::LengthMismatch) ==> sources.len() != dests.len(),
        r matches Err(MigrationKeyError::KeyOutOfRange) ==> sources.len() == dests.len() && !(
        in_range(sources@, key_count) && in_range(dests@, key_count)),
        r matches Err(MigrationKeyError::DuplicateSource { first, second }) ==> sources.len()
            == dests.len() && first < second < sources.len() && sources@[first as int]
            == sources@[second as int],
        r matches Err(MigrationKeyError::DuplicateDest { first, second }) ==> sources.len()
            == dests.len() && in_range(sources@, key_count) && distinct(sources@) && first
            < second < dests.len() && dests@[first as int] == dests@[second as int],
{
    let src_pos = check_keys(key_count, sources, dests)?;
    let mut acts: Vec<MigAction> = Vec::new();
    let mut i: usize = 0;
    while i < sources.len()
        invariant
            i <= sources.len(),
            acts@ == delete_all(sources@.take(i as int)),
        decreases sources.len() - i,
    {
        proof {
            assert(sources@.take(i + 1) =~= sources@.take(i as int).push(sources@[i as int]));
            assert(delete_all(sources@.take(i + 1)) =~= delete_all(sources@.take(i as int)).push(
                MigAction::DeleteKey(sources@[i as int]),
            ));
        }
        acts.push(MigAction::DeleteKey(sources[i]));
        i += 1;
    }
    proof {
        assert(sources@.take(sources.len() as int) =~= sources@);
    }
    let ghost deletes = acts@;
    let mut i: usize = 0;
    while i < dests.len()
        invariant
            i <= dests.len(),
            well_formed(key_count, sources@, dests@),
            indexes(src_pos@, sources@, key_count),
            deletes == delete_all(sources@),
            acts@ == deletes + dest_deletes(sources@, dests@.take(i as int)),
        decreases dests.len() - i,
    {
        let k = dests[i];
        proof {
            assert(k < key_count);
            assert(dests@.take(i + 1).drop_last() =~= dests@.take(i as int));
            assert(dests@.take(i + 1).last() == k);
        }
        let is_source = src_pos[k].is_some();
        proof {
            if is_source {
                let j = src_pos@[k as int]->Some_0;
                assert(sources@[j as int] == k);
                assert(sources@.contains(k));
            }
        }
        if !is_source {
            acts.push(MigAction::DeleteKey(k));
        }
        proof {
            assert(acts@ =~= deletes + dest_deletes(sources@, dests@.take(i + 1)));
        }
        i += 1;
    }
    proof {
        assert(dests@.take(dests.len() as int) =~= dests@);
    }
    let ghost prefix = acts@;
    let mut i: usize = 0;
    while i < dests.len()
        invariant
            i <= dests.len(),
            acts@ == prefix + create_all(i as nat),
        decreases dests.len() - i,
    {
        proof {
            assert(create_all((i + 1) as nat) =~= create_all(i as nat).push(MigAction::Create(i)));
        }
        acts.push(MigAction::Create(i));
        i += 1;
    }
    proof {
        assert(acts@ == spec_schedule(sources@, dests@));
        assert forall|db: Map<usize, int>, payload: Seq<int>|
            payload.len() == dests.len() implies #[trigger] apply(db, acts@, dests@, payload)
            == moved(db, sources@, dests@, payload) by {
            lemma_schedule_correct(db, sources@, dests@, payload);
        }
        assert forall|db: Map<usize, int>, payload: Seq<int>, port: spec_fn(int) -> int|
            snapshot(db, sources@, payload) && ports_unique(db, port) implies #[trigger] run_safe(
            db,
            acts@,
            dests@,
            payload,
            port,
        ) by {
            lemma_schedule_port_safe(db, sources@, dests@, payload, port);
        }
    }
    Ok(acts)
}

// ===========================================================================
// The unrelated-path guard over every key a migration touches
// ===========================================================================

/// Why the unrelated-path guard stopped a migration.
pub enum MoveGuardError<E> {
    /// A key id is not below the number of key paths.
    KeyOutOfRange,
    /// The guard refused a key. `index` is its position in
    /// `affected_ids(sources, dests)` (sources first, then destinations).
    Refused(super::release::GuardFailure<E>),
}

/// The key ids a migration affects, in the order the guard checks them:
/// every source (deleted), then every destination (written).
pub open spec fn affected_ids(srcs: Seq<usize>, dsts: Seq<usize>) -> Seq<usize> {
    srcs + dsts
}

/// The guard permitted key `k`: its path is lexically related to the
/// working directory or the physical fallback permitted it.
pub open spec fn key_permitted<E, F: Fn(usize) -> Option<E>>(
    cwd: Seq<(super::path::CompKind, Seq<u8>)>,
    key_paths: Seq<Vec<super::path_relation::PathToken>>,
    fallback: F,
    k: usize,
) -> bool {
    k < key_paths.len() && super::release::permitted(
        cwd,
        super::path_relation::tok_views(key_paths[k as int]@),
        fallback,
        k,
    )
}

/// The unrelated-path guard over every key a migration touches: each source
/// (whose reservation is deleted) and each destination (which is written).
/// `key_paths[k]` is the path of key id `k`, and `sources` / `dests` are the
/// same id lists `classify_conflicts` and `schedule_migration` take.
///
/// - Soundness: when the guard applies, success means every source and every
///   destination key is lexically related to the working directory or
///   permitted by the physical fallback.
/// - A refusal names the first refused key in `affected_ids` order; every
///   earlier key was permitted.
/// - With `--force` or `--allow-unrelated-path` the guard passes and the
///   fallback is never consulted.
pub fn guard_moves<E, F: Fn(usize) -> Option<E>>(
    guard: &super::release::PathGuard,
    key_paths: &[Vec<super::path_relation::PathToken>],
    sources: &[usize],
    dests: &[usize],
    fallback: &F,
) -> (r: Result<(), MoveGuardError<E>>)
    requires
        super::release::guard_applies(*guard) ==> forall|k: usize|
            k < key_paths.len() ==> fallback.requires((k,)),
    ensures
        !super::release::guard_applies(*guard) ==> r is Ok,
        r is Ok && super::release::guard_applies(*guard) ==> (forall|i: int|
            0 <= i < sources.len() ==> key_permitted(
                super::release::guard_cwd(*guard),
                key_paths@,
                *fallback,
                #[trigger] sources@[i],
            )) && (forall|i: int|
            0 <= i < dests.len() ==> key_permitted(
                super::release::guard_cwd(*guard),
                key_paths@,
                *fallback,
                #[trigger] dests@[i],
            )),
        r matches Err(MoveGuardError::KeyOutOfRange) ==> super::release::guard_applies(*guard)
            && !(in_range(sources@, key_paths.len()) && in_range(dests@, key_paths.len())),
        r matches Err(MoveGuardError::Refused(f)) ==> super::release::guard_applies(*guard)
            && f.index < sources.len() + dests.len() && affected_ids(sources@, dests@)[f.index as int]
            < key_paths.len() && super::release::refused(
            super::release::guard_cwd(*guard),
            super::path_relation::tok_views(
                key_paths@[affected_ids(sources@, dests@)[f.index as int] as int]@,
            ),
            *fallback,
            affected_ids(sources@, dests@)[f.index as int],
            f.error,
        ) && forall|p: int|
            0 <= p < f.index ==> key_permitted(
                super::release::guard_cwd(*guard),
                key_paths@,
                *fallback,
                #[trigger] affected_ids(sources@, dests@)[p],
            ),
{
    if !super::release::requires_path_check(guard.force, guard.allow_unrelated) {
        return Ok(());
    }
    let ghost c = super::release::guard_cwd(*guard);
    let ghost ids = affected_ids(sources@, dests@);
    let mut chain: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < sources.len()
        invariant
            i <= sources.len(),
            chain@ == sources@.take(i as int),
        decreases sources.len() - i,
    {
        proof {
            assert(sources@.take(i + 1) =~= sources@.take(i as int).push(sources@[i as int]));
        }
        chain.push(sources[i]);
        i += 1;
    }
    let mut i: usize = 0;
    while i < dests.len()
        invariant
            i <= dests.len(),
            chain@ == sources@ + dests@.take(i as int),
        decreases dests.len() - i,
    {
        proof {
            assert(sources@.take(sources.len() as int) =~= sources@);
            assert(sources@ + dests@.take(i + 1) =~= (sources@ + dests@.take(i as int)).push(
                dests@[i as int],
            ));
        }
        chain.push(dests[i]);
        i += 1;
    }
    proof {
        assert(sources@.take(sources.len() as int) =~= sources@);
        assert(dests@.take(dests.len() as int) =~= dests@);
        assert(chain@ =~= ids);
    }
    let mut p: usize = 0;
    while p < chain.len()
        invariant
            p <= chain.len(),
            chain@ == ids,
            ids == affected_ids(sources@, dests@),
            c == super::release::guard_cwd(*guard),
            super::release::guard_applies(*guard),
            forall|k: usize| k < key_paths.len() ==> fallback.requires((k,)),
            forall|q: int| 0 <= q < p ==> key_permitted(c, key_paths@, *fallback, #[trigger] ids[q]),
        decreases chain.len() - p,
    {
        let k = chain[p];
        if k >= key_paths.len() {
            proof {
                if p < sources.len() {
                    assert(ids[p as int] == sources@[p as int]);
                    assert(!in_range(sources@, key_paths.len()));
                } else {
                    assert(ids[p as int] == dests@[p - sources.len()]);
                    assert(!in_range(dests@, key_paths.len()));
                }
            }
            return Err(MoveGuardError::KeyOutOfRange);
        }
        if let Some(error) = super::release::gate_path(
            guard.cwd.as_slice(),
            key_paths[k].as_slice(),
            fallback,
            k,
        ) {
            return Err(MoveGuardError::Refused(super::release::GuardFailure { index: p, error }));
        }
        p += 1;
    }
    proof {
        assert forall|i: int| 0 <= i < sources.len() implies key_permitted(
            c,
            key_paths@,
            *fallback,
            #[trigger] sources@[i],
        ) by {
            assert(ids[i] == sources@[i]);
        }
        assert forall|i: int| 0 <= i < dests.len() implies key_permitted(
            c,
            key_paths@,
            *fallback,
            #[trigger] dests@[i],
        ) by {
            assert(ids[sources.len() + i] == dests@[i]);
        }
    }
    Ok(())
}

// ===========================================================================
// Rebasing canonical paths
// ===========================================================================

/// `p` is a byte prefix of `s`.
fn starts_with(s: &[u8], p: &[u8]) -> (r: bool)
    ensures
        r == super::path_relation::is_prefix_of(p@, s@),
{
    if p.len() > s.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < p.len()
        invariant
            p.len() <= s.len(),
            i <= p.len(),
            forall|j: int| 0 <= j < i ==> s@[j] == p@[j],
        decreases p.len() - i,
    {
        if s[i] != p[i] {
            assert(s@.subrange(0, p.len() as int)[i as int] != p@[i as int]);
            return false;
        }
        i += 1;
    }
    assert(s@.subrange(0, p.len() as int) =~= p@);
    true
}

/// `s[start..]` followed by nothing, appended to `out`.
fn push_from(out: &mut Vec<u8>, s: &[u8], start: usize)
    requires
        start <= s.len(),
    ensures
        final(out)@ == old(out)@ + s@.subrange(start as int, s.len() as int),
{
    let mut i = start;
    while i < s.len()
        invariant
            start <= i <= s.len(),
            out@ == old(out)@ + s@.subrange(start as int, i as int),
        decreases s.len() - i,
    {
        out.push(s[i]);
        proof {
            assert(s@.subrange(start as int, i + 1) =~= s@.subrange(start as int, i as int).push(
                s@[i as int],
            ));
        }
        i += 1;
    }
}

/// The canonical path `orig` moved from under `from` to under `to`: the
/// result is canonical and its segments are `spec_rebase` of the inputs'
/// segments (`to`'s segments followed by `orig`'s segments below `from`).
/// Returns `None` exactly when one input is not canonical or `from` is not a
/// component prefix of `orig` (`/a/b` is not a prefix of `/a/bc`).
pub fn rebase_canonical(orig: &[u8], from: &[u8], to: &[u8]) -> (r: Option<Vec<u8>>)
    ensures
        r is Some <==> (super::path::spec_is_canonical_abs(orig@, false)
            && super::path::spec_is_canonical_abs(from@, false)
            && super::path::spec_is_canonical_abs(to@, false)
            && super::path_relation::spec_component_prefix(from@, orig@)),
        r is Some ==> super::path_relation::spec_rebase(
            super::path::segs(orig@),
            super::path::segs(from@),
            super::path::segs(to@),
        ) is Some,
        r matches Some(out) ==> super::path::spec_is_canonical_abs(out@, false) && super::path::segs(
            out@,
        ) == super::path_relation::spec_rebase(
            super::path::segs(orig@),
            super::path::segs(from@),
            super::path::segs(to@),
        )->Some_0,
{
    if !super::path::is_canonical_abs_path(orig, false) || !super::path::is_canonical_abs_path(
        from,
        false,
    ) || !super::path::is_canonical_abs_path(to, false) {
        return None;
    }
    proof {
        super::path::lemma_canonical_render(orig@, false);
        super::path::lemma_canonical_render(from@, false);
        super::path::lemma_canonical_render(to@, false);
        super::path_relation::lemma_component_prefix_iff_segs(from@, orig@);
    }
    if !starts_with(orig, from) {
        return None;
    }
    let root = from.len() == 1;
    if !(from.len() == orig.len() || root || orig[from.len()] == b'/') {
        proof {
            if from@ == seq![b'/'] {
                assert(from.len() == 1);
            }
        }
        return None;
    }
    proof {
        if root {
            assert(from@ =~= seq![b'/']) by {
                assert(from@[0] == b'/');
            }
        }
    }
    // Below any `from` but the root, the rest follows `from`'s bytes. Below
    // the root `/` it is all of `orig`, or nothing when `orig` is `/` itself.
    let rest_start: usize = if !root {
        from.len()
    } else if orig.len() == 1 {
        orig.len()
    } else {
        0
    };
    let ghost o = super::path::segs(orig@);
    let ghost f = super::path::segs(from@);
    let ghost t = super::path::segs(to@);
    let ghost tail = o.subrange(f.len() as int, o.len() as int);
    proof {
        lemma_rest_is_tail(orig@, from@, rest_start as int);
    }
    let mut out: Vec<u8> = Vec::new();
    if to.len() == 1 && rest_start == orig.len() {
        out.push(b'/');
    } else if to.len() == 1 {
        push_from(&mut out, orig, rest_start);
    } else {
        push_from(&mut out, to, 0);
        push_from(&mut out, orig, rest_start);
    }
    proof {
        lemma_rebased_bytes(orig@, from@, to@, rest_start as int, out@);
    }
    Some(out)
}

/// Rebasing is injective on canonical paths: two different canonical paths
/// under `from` never land on the same destination, so distinct source
/// paths never collide at the destination (and, with byte equality being
/// segment equality on canonical strings, neither do their keys).
pub proof fn lemma_rebase_canonical_injective(a: Seq<u8>, b: Seq<u8>, from: Seq<u8>, to: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(a, false),
        super::path::spec_is_canonical_abs(b, false),
        super::path::spec_is_canonical_abs(from, false),
        super::path_relation::spec_component_prefix(from, a),
        super::path_relation::spec_component_prefix(from, b),
        super::path_relation::spec_rebase(super::path::segs(a), super::path::segs(from), super::path::segs(to))
            == super::path_relation::spec_rebase(
            super::path::segs(b),
            super::path::segs(from),
            super::path::segs(to),
        ),
    ensures
        a == b,
{
    super::path_relation::lemma_component_prefix_iff_segs(from, a);
    super::path_relation::lemma_component_prefix_iff_segs(from, b);
    super::path_relation::lemma_rebase_injective(
        super::path::segs(a),
        super::path::segs(b),
        super::path::segs(from),
        super::path::segs(to),
    );
    super::path::lemma_canonical_injective(a, b, false);
}

/// The bytes of `orig` from `rest_start` on spell the segments of `orig`
/// below `from`.
proof fn lemma_rest_is_tail(orig: Seq<u8>, from: Seq<u8>, rest_start: int)
    requires
        super::path::spec_is_canonical_abs(orig, false),
        super::path::spec_is_canonical_abs(from, false),
        super::path_relation::spec_component_prefix(from, orig),
        rest_start == if from.len() == 1 {
            if orig.len() == 1 {
                1int
            } else {
                0int
            }
        } else {
            from.len() as int
        },
    ensures
        0 <= rest_start <= orig.len(),
        super::path_relation::is_prefix_of(super::path::segs(from), super::path::segs(orig)),
        orig.subrange(rest_start, orig.len() as int) == super::path::render_tail(
            super::path::segs(orig).subrange(
                super::path::segs(from).len() as int,
                super::path::segs(orig).len() as int,
            ),
        ),
{
    let o = super::path::segs(orig);
    let f = super::path::segs(from);
    super::path::lemma_canonical_render(orig, false);
    super::path::lemma_canonical_render(from, false);
    super::path_relation::lemma_component_prefix_iff_segs(from, orig);
    let tail = o.subrange(f.len() as int, o.len() as int);
    assert(o =~= f + tail);
    super::path_relation::lemma_render_tail_concat(f, tail);
    if f.len() == 0 {
        assert(from == seq![b'/']);
        assert(f + tail =~= tail);
        if tail.len() == 0 {
            assert(orig == seq![b'/']);
            assert(orig.subrange(1, 1) =~= Seq::<u8>::empty());
            assert(super::path::render_tail(tail) =~= Seq::<u8>::empty());
        } else {
            assert(orig == super::path::render_tail(tail));
            super::path::lemma_render_tail_first(tail);
            assert(orig.len() != 1) by {
                if orig.len() == 1 {
                    assert(orig == seq![b'/']) by {
                        assert(orig[0] == b'/');
                        assert(orig =~= seq![b'/']);
                    }
                    super::path::lemma_segs_trivial();
                }
            }
            assert(orig.subrange(0, orig.len() as int) =~= orig);
        }
    } else {
        assert(from == super::path::render_tail(f));
        assert(from.len() != 1) by {
            if from.len() == 1 {
                assert(from[0] == b'/');
                assert(from =~= seq![b'/']);
                super::path::lemma_segs_trivial();
            }
        }
        if tail.len() == 0 {
            assert(f + tail =~= f);
            assert(orig == from);
            assert(super::path::render_tail(tail) =~= Seq::<u8>::empty());
            assert(orig.subrange(from.len() as int, orig.len() as int) =~= Seq::<u8>::empty());
        } else {
            assert(orig == super::path::render_tail(f + tail));
            assert(orig == from + super::path::render_tail(tail));
            assert(orig.subrange(from.len() as int, orig.len() as int) =~= super::path::render_tail(
                tail,
            ));
        }
    }
}

/// The bytes `rebase_canonical` writes are the canonical rendering of the
/// rebased segments.
proof fn lemma_rebased_bytes(orig: Seq<u8>, from: Seq<u8>, to: Seq<u8>, rest_start: int, out: Seq<u8>)
    requires
        super::path::spec_is_canonical_abs(orig, false),
        super::path::spec_is_canonical_abs(from, false),
        super::path::spec_is_canonical_abs(to, false),
        0 <= rest_start <= orig.len(),
        super::path_relation::is_prefix_of(super::path::segs(from), super::path::segs(orig)),
        orig.subrange(rest_start, orig.len() as int) == super::path::render_tail(
            super::path::segs(orig).subrange(
                super::path::segs(from).len() as int,
                super::path::segs(orig).len() as int,
            ),
        ),
        out == if to.len() == 1 && rest_start == orig.len() {
            seq![b'/']
        } else if to.len() == 1 {
            orig.subrange(rest_start, orig.len() as int)
        } else {
            to + orig.subrange(rest_start, orig.len() as int)
        },
    ensures
        super::path::spec_is_canonical_abs(out, false),
        super::path_relation::spec_rebase(
            super::path::segs(orig),
            super::path::segs(from),
            super::path::segs(to),
        ) is Some,
        super::path::segs(out) == super::path_relation::spec_rebase(
            super::path::segs(orig),
            super::path::segs(from),
            super::path::segs(to),
        )->Some_0,
{
    let o = super::path::segs(orig);
    let f = super::path::segs(from);
    let t = super::path::segs(to);
    let tail = o.subrange(f.len() as int, o.len() as int);
    let r = t + tail;
    super::path::lemma_canonical_render(orig, false);
    super::path::lemma_canonical_render(from, false);
    super::path::lemma_canonical_render(to, false);
    super::path_relation::lemma_rebase_canonical(o, f, t);
    assert(super::path_relation::spec_rebase(o, f, t) == Some(r));
    let rest = orig.subrange(rest_start, orig.len() as int);
    super::path_relation::lemma_render_tail_concat(t, tail);
    if t.len() == 0 {
        assert(to.len() == 1);
        assert(r =~= tail);
        if tail.len() == 0 {
            assert(rest.len() == 0) by {
                assert(super::path::render_tail(tail) =~= Seq::<u8>::empty());
            }
            assert(out == seq![b'/']);
            assert(super::path::render(r) == seq![b'/']);
        } else {
            super::path::lemma_render_tail_first(tail);
            assert(out == rest);
            assert(super::path::render(r) == super::path::render_tail(tail));
        }
    } else {
        assert(to == super::path::render_tail(t));
        assert(to.len() != 1) by {
            super::path::lemma_render_tail_first(t);
            let st1 = t.drop_last();
            assert(to == super::path::render_tail(st1) + seq![b'/'] + t.last());
            assert(super::path::is_normal_seg(t[t.len() - 1]));
        }
        assert(out == to + rest);
        assert(super::path::render(r) == super::path::render_tail(r));
        assert(super::path::render_tail(r) == to + rest);
    }
    assert(out == super::path::render(r));
}

} // verus!
