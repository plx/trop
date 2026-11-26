//! Verified kernel: a stable merge sort of positions by `u64` keys, and the
//! cleanup removal order built on it.
//!
//! - `stable_order(keys)` returns the positions `0..keys.len()` sorted by
//!   key, ties in position order (`is_stable_order`): every position exactly
//!   once, in strictly increasing `key_before` order. It is a merge sort
//!   (`O(n log n)` comparisons; the insertion sort of `super::render` is
//!   quadratic). A stable order is unique (`lemma_stable_order_unique`), so
//!   the result is a function of the keys alone.
//! - `order_removals(cands)` is the order in which cleanup removes and
//!   reports its candidates (`is_removal_order`): every prune candidate
//!   before every expire candidate, prune candidates in scan order, expire
//!   candidates oldest first (by `last_used_secs`) with ties in scan order.
//!   `Keep` positions are left out and every other position appears exactly
//!   once. That order is unique too (`lemma_removal_order_unique`).
//!
//! Membership is stated with `super::render::occurs`, the vocabulary of
//! `super::render::sort_order` (whose byte-string keys and insertion sort do
//! not fit these integer keys and up to 65535 candidates).
//!
//! Production glue: `trop/src/operations/cleanup.rs` orders its planned
//! removals through `super::cleanup::plan_removals`, which calls
//! `order_removals`; the glue converts each `last_used_at` (stored in whole
//! seconds) to `last_used_secs`.

use super::cleanup::{Candidate, CleanupReason};
use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// Stable order by key
// ===========================================================================

/// Position `i` sorts before position `j`: a smaller key, or the same key and
/// an earlier position.
pub open spec fn key_before(keys: Seq<u64>, i: int, j: int) -> bool {
    keys[i] < keys[j] || (keys[i] == keys[j] && i < j)
}

/// `order` lists exactly the positions `lo..hi`, each once, in strictly
/// increasing `key_before` order.
pub open spec fn sorts_range(keys: Seq<u64>, lo: int, hi: int, order: Seq<usize>) -> bool {
    &&& order.len() == hi - lo
    &&& forall|a: int| 0 <= a < order.len() ==> lo <= #[trigger] order[a] < hi
    &&& forall|a: int, b: int|
        0 <= a < b < order.len() ==> key_before(
            keys,
            #[trigger] order[a] as int,
            #[trigger] order[b] as int,
        )
    &&& forall|k: int| lo <= k < hi ==> #[trigger] super::render::occurs(order, k)
}

/// `order` is the stable sorted order of all positions of `keys`.
pub open spec fn is_stable_order(keys: Seq<u64>, order: Seq<usize>) -> bool {
    sorts_range(keys, 0, keys.len() as int, order)
}

/// After appending `x` to `old`, where `x` sorts after every element of `old`
/// and before `next`, every element sorts before `next` (`key_before` is
/// transitive).
proof fn lemma_append_before(keys: Seq<u64>, old: Seq<usize>, x: usize, next: int)
    requires
        forall|a: int| 0 <= a < old.len() ==> key_before(keys, #[trigger] old[a] as int, x as int),
        key_before(keys, x as int, next),
    ensures
        forall|a: int|
            0 <= a < old.len() + 1 ==> key_before(keys, #[trigger] old.push(x)[a] as int, next),
{
    assert forall|a: int| 0 <= a < old.len() + 1 implies key_before(
        keys,
        #[trigger] old.push(x)[a] as int,
        next,
    ) by {
        if a < old.len() {
            assert(old.push(x)[a] == old[a]);
            assert(key_before(keys, old[a] as int, x as int));
        }
    }
}

/// Merges the sorted orders of `lo..mid` and `mid..hi` into the sorted order
/// of `lo..hi`.
fn merge(
    keys: &[u64],
    left: &[usize],
    right: &[usize],
    Ghost(lo): Ghost<usize>,
    Ghost(mid): Ghost<usize>,
    Ghost(hi): Ghost<usize>,
) -> (out: Vec<usize>)
    requires
        lo <= mid <= hi <= keys@.len(),
        sorts_range(keys@, lo as int, mid as int, left@),
        sorts_range(keys@, mid as int, hi as int, right@),
    ensures
        sorts_range(keys@, lo as int, hi as int, out@),
{
    let mut out: Vec<usize> = Vec::new();
    let (mut i, mut j): (usize, usize) = (0, 0);
    // Where each element of `left` / `right` landed in `out`.
    let ghost mut lpos: Seq<int> = Seq::empty();
    let ghost mut rpos: Seq<int> = Seq::empty();
    while i < left.len() || j < right.len()
        invariant
            lo <= mid <= hi <= keys@.len(),
            sorts_range(keys@, lo as int, mid as int, left@),
            sorts_range(keys@, mid as int, hi as int, right@),
            i <= left@.len(),
            j <= right@.len(),
            out@.len() == i + j,
            forall|a: int| 0 <= a < out@.len() ==> lo <= #[trigger] out@[a] < hi,
            forall|a: int, b: int|
                0 <= a < b < out@.len() ==> key_before(
                    keys@,
                    #[trigger] out@[a] as int,
                    #[trigger] out@[b] as int,
                ),
            i < left@.len() ==> forall|a: int|
                0 <= a < out@.len() ==> key_before(keys@, #[trigger] out@[a] as int, left@[i as int] as int),
            j < right@.len() ==> forall|a: int|
                0 <= a < out@.len() ==> key_before(keys@, #[trigger] out@[a] as int, right@[j as int] as int),
            lpos.len() == i,
            rpos.len() == j,
            forall|a: int|
                0 <= a < i ==> 0 <= #[trigger] lpos[a] < out@.len() && out@[lpos[a]] == left@[a],
            forall|b: int|
                0 <= b < j ==> 0 <= #[trigger] rpos[b] < out@.len() && out@[rpos[b]] == right@[b],
        decreases left@.len() + right@.len() - i - j,
    {
        // Ties go left: every position of `left` is below every one of `right`.
        let take_left = if i < left.len() && j < right.len() {
            keys[left[i]] <= keys[right[j]]
        } else {
            i < left.len()
        };
        let x = if take_left {
            left[i]
        } else {
            right[j]
        };
        let ghost old_out = out@;
        proof {
            if i < left@.len() && j < right@.len() {
                assert(lo <= left@[i as int] < mid <= right@[j as int] < hi);
            }
            if take_left && i + 1 < left@.len() {
                assert(key_before(keys@, x as int, left@[i + 1] as int));
                lemma_append_before(keys@, old_out, x, left@[i + 1] as int);
            }
            if take_left && j < right@.len() {
                lemma_append_before(keys@, old_out, x, right@[j as int] as int);
            }
            if !take_left && j + 1 < right@.len() {
                assert(key_before(keys@, x as int, right@[j + 1] as int));
                lemma_append_before(keys@, old_out, x, right@[j + 1] as int);
            }
            if !take_left && i < left@.len() {
                lemma_append_before(keys@, old_out, x, left@[i as int] as int);
            }
        }
        out.push(x);
        proof {
            assert forall|a: int| 0 <= a < old_out.len() implies #[trigger] out@[a] == old_out[a] by {}
            if take_left {
                lpos = lpos.push(old_out.len() as int);
            } else {
                rpos = rpos.push(old_out.len() as int);
            }
        }
        if take_left {
            i += 1;
        } else {
            j += 1;
        }
    }
    proof {
        assert forall|k: int| lo <= k < hi implies #[trigger] super::render::occurs(out@, k) by {
            if k < mid {
                assert(super::render::occurs(left@, k));
                let a = choose|a: int| 0 <= a < left@.len() && left@[a] as int == k;
                assert(out@[lpos[a]] as int == k);
            } else {
                assert(super::render::occurs(right@, k));
                let b = choose|b: int| 0 <= b < right@.len() && right@[b] as int == k;
                assert(out@[rpos[b]] as int == k);
            }
        }
    }
    out
}

/// The stable sorted order of the positions `lo..hi` (a top-down merge
/// sort).
fn sort_range(keys: &[u64], lo: usize, hi: usize) -> (order: Vec<usize>)
    requires
        lo <= hi <= keys@.len(),
    ensures
        sorts_range(keys@, lo as int, hi as int, order@),
    decreases hi - lo,
{
    if hi - lo <= 1 {
        let mut order: Vec<usize> = Vec::new();
        if lo < hi {
            order.push(lo);
            assert(order@[0] as int == lo as int);
        }
        return order;
    }
    let mid = lo + (hi - lo) / 2;
    let left = sort_range(keys, lo, mid);
    let right = sort_range(keys, mid, hi);
    merge(keys, left.as_slice(), right.as_slice(), Ghost(lo), Ghost(mid), Ghost(hi))
}

/// The positions of `keys`, sorted by key with ties in position order: the
/// unique stable order (`lemma_stable_order_unique`).
pub fn stable_order(keys: &[u64]) -> (order: Vec<usize>)
    ensures
        is_stable_order(keys@, order@),
{
    sort_range(keys, 0, keys.len())
}

/// In a stable order, positions follow `key_before`, and no position repeats.
pub proof fn lemma_stable_positions(keys: Seq<u64>, order: Seq<usize>, a: int, b: int)
    requires
        is_stable_order(keys, order),
        0 <= a < order.len(),
        0 <= b < order.len(),
    ensures
        key_before(keys, order[a] as int, order[b] as int) <==> a < b,
        order[a] == order[b] <==> a == b,
{
    if a < b {
        assert(key_before(keys, order[a] as int, order[b] as int));
    } else if b < a {
        assert(key_before(keys, order[b] as int, order[a] as int));
    }
}

/// Two stable orders agree at `a` when they agree before it.
proof fn lemma_stable_agree_at(keys: Seq<u64>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_stable_order(keys, o1),
        is_stable_order(keys, o2),
        0 <= a < o1.len(),
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    ensures
        o1[a] == o2[a],
{
    let x = o1[a] as int;
    let y = o2[a] as int;
    if x != y {
        assert(super::render::occurs(o1, y));
        let b = choose|b: int| 0 <= b < o1.len() && o1[b] as int == y;
        assert(super::render::occurs(o2, x));
        let c = choose|c: int| 0 <= c < o2.len() && o2[c] as int == x;
        if b < a {
            assert(o2[b] == o1[b]);
            lemma_stable_positions(keys, o2, b, a);
        }
        if c < a {
            assert(o1[c] == o2[c]);
            lemma_stable_positions(keys, o1, c, a);
        }
        lemma_stable_positions(keys, o1, a, b);
        lemma_stable_positions(keys, o2, a, c);
    }
}

/// Two stable orders agree below `a`.
proof fn lemma_stable_agree_upto(keys: Seq<u64>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_stable_order(keys, o1),
        is_stable_order(keys, o2),
        0 <= a <= o1.len(),
    ensures
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    decreases a,
{
    if a > 0 {
        lemma_stable_agree_upto(keys, o1, o2, a - 1);
        lemma_stable_agree_at(keys, o1, o2, a - 1);
    }
}

/// The stable order of a key sequence is unique.
pub proof fn lemma_stable_order_unique(keys: Seq<u64>, o1: Seq<usize>, o2: Seq<usize>)
    requires
        is_stable_order(keys, o1),
        is_stable_order(keys, o2),
    ensures
        o1 == o2,
{
    lemma_stable_agree_upto(keys, o1, o2, o1.len() as int);
    assert(o1 =~= o2);
}

// ===========================================================================
// The cleanup removal order
// ===========================================================================

/// A removal candidate's rank: prune candidates first, then expire
/// candidates by last use (`Keep` positions are never ordered).
pub open spec fn removal_rank(c: Candidate) -> int {
    match c.reason {
        CleanupReason::Prune => 0,
        CleanupReason::Expire => 1 + c.last_used_secs,
        CleanupReason::Keep => -1,
    }
}

/// Candidate `i` is removed before candidate `j`: a lower rank, or the same
/// rank and an earlier scan position.
pub open spec fn removal_before(cands: Seq<Candidate>, i: int, j: int) -> bool {
    removal_rank(cands[i]) < removal_rank(cands[j]) || (removal_rank(cands[i]) == removal_rank(
        cands[j],
    ) && i < j)
}

/// `order` is the removal order of `cands`: exactly the non-`Keep`
/// positions, each once, in strictly increasing `removal_before` order.
pub open spec fn is_removal_order(cands: Seq<Candidate>, order: Seq<usize>) -> bool {
    &&& forall|a: int|
        0 <= a < order.len() ==> (#[trigger] order[a] as int) < cands.len() && cands[order[a] as int].reason
            != CleanupReason::Keep
    &&& forall|a: int, b: int|
        0 <= a < b < order.len() ==> removal_before(
            cands,
            #[trigger] order[a] as int,
            #[trigger] order[b] as int,
        )
    &&& forall|k: int|
        0 <= k < cands.len() && #[trigger] cands[k].reason != CleanupReason::Keep ==> super::render::occurs(
            order,
            k,
        )
    &&& order.len() == super::cleanup::removal_count(cands)
}

/// What the removal order means: no position repeats; every prune candidate
/// comes before every expire candidate; prune candidates keep scan order;
/// expire candidates are oldest first, ties in scan order.
pub proof fn lemma_removal_order_meaning(cands: Seq<Candidate>, order: Seq<usize>, a: int, b: int)
    requires
        is_removal_order(cands, order),
        0 <= a < b < order.len(),
    ensures
        order[a] != order[b],
        cands[order[a] as int].reason == CleanupReason::Expire ==> cands[order[b] as int].reason
            == CleanupReason::Expire,
        cands[order[a] as int].reason == CleanupReason::Prune && cands[order[b] as int].reason
            == CleanupReason::Prune ==> order[a] < order[b],
        cands[order[a] as int].reason == CleanupReason::Expire && cands[order[b] as int].reason
            == CleanupReason::Expire ==> cands[order[a] as int].last_used_secs < cands[order[b] as int].last_used_secs
            || (cands[order[a] as int].last_used_secs == cands[order[b] as int].last_used_secs
            && order[a] < order[b]),
{
    assert(removal_before(cands, order[a] as int, order[b] as int));
}

/// Two removal orders positions agree at `a` when they agree before it.
proof fn lemma_removal_agree_at(cands: Seq<Candidate>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_removal_order(cands, o1),
        is_removal_order(cands, o2),
        o1.len() == o2.len(),
        0 <= a < o1.len(),
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    ensures
        o1[a] == o2[a],
{
    let x = o1[a] as int;
    let y = o2[a] as int;
    if x != y {
        assert(cands[y].reason != CleanupReason::Keep);
        assert(super::render::occurs(o1, y));
        let b = choose|b: int| 0 <= b < o1.len() && o1[b] as int == y;
        assert(cands[x].reason != CleanupReason::Keep);
        assert(super::render::occurs(o2, x));
        let c = choose|c: int| 0 <= c < o2.len() && o2[c] as int == x;
        if b < a {
            assert(o2[b] == o1[b]);
            assert(removal_before(cands, o2[b] as int, o2[a] as int));
        }
        if b > a {
            assert(removal_before(cands, x, y));
        }
        if c < a {
            assert(o1[c] == o2[c]);
            assert(removal_before(cands, o1[c] as int, o1[a] as int));
        }
        if c > a {
            assert(removal_before(cands, y, x));
        }
    }
}

/// Two removal orders agree below `a`.
proof fn lemma_removal_agree_upto(cands: Seq<Candidate>, o1: Seq<usize>, o2: Seq<usize>, a: int)
    requires
        is_removal_order(cands, o1),
        is_removal_order(cands, o2),
        0 <= a <= o1.len(),
    ensures
        forall|b: int| 0 <= b < a ==> o1[b] == o2[b],
    decreases a,
{
    if a > 0 {
        lemma_removal_agree_upto(cands, o1, o2, a - 1);
        lemma_removal_agree_at(cands, o1, o2, a - 1);
    }
}

/// The removal order of a candidate list is unique.
pub proof fn lemma_removal_order_unique(cands: Seq<Candidate>, o1: Seq<usize>, o2: Seq<usize>)
    requires
        is_removal_order(cands, o1),
        is_removal_order(cands, o2),
    ensures
        o1 == o2,
{
    lemma_removal_agree_upto(cands, o1, o2, o1.len() as int);
    assert(o1 =~= o2);
}

/// The prune positions and the expire positions of a scan, each ascending,
/// with the expire positions' last-use times.
pub struct SplitCandidates {
    /// The prune positions, ascending.
    pub prunes: Vec<usize>,
    /// The expire positions, ascending.
    pub expires: Vec<usize>,
    /// `expire_keys[a]` is the last use of `expires[a]`.
    pub expire_keys: Vec<u64>,
}

/// `list` holds exactly the positions of `cands` with reason `reason`,
/// ascending.
pub open spec fn lists_positions(cands: Seq<Candidate>, reason: CleanupReason, list: Seq<usize>) -> bool {
    &&& super::cleanup::increasing(list)
    &&& forall|a: int|
        0 <= a < list.len() ==> (#[trigger] list[a] as int) < cands.len() && cands[list[a] as int].reason
            == reason
    &&& forall|k: int|
        0 <= k < cands.len() && #[trigger] cands[k].reason == reason ==> super::render::occurs(
            list,
            k,
        )
}

/// Splits a scan into its prune and expire positions.
fn split_candidates(cands: &[Candidate]) -> (r: SplitCandidates)
    ensures
        lists_positions(cands@, CleanupReason::Prune, r.prunes@),
        lists_positions(cands@, CleanupReason::Expire, r.expires@),
        r.expire_keys@.len() == r.expires@.len(),
        forall|a: int|
            0 <= a < r.expires@.len() ==> #[trigger] r.expire_keys@[a] == cands@[r.expires@[a] as int].last_used_secs,
        r.prunes@.len() + r.expires@.len() == super::cleanup::removal_count(cands@),
{
    let n = cands.len();
    let mut prunes: Vec<usize> = Vec::new();
    let mut expires: Vec<usize> = Vec::new();
    let mut expire_keys: Vec<u64> = Vec::new();
    // Where each prune or expire position landed in its list.
    let ghost mut inv: Seq<int> = Seq::empty();
    let mut i: usize = 0;
    while i < n
        invariant
            n == cands@.len(),
            i <= n,
            super::cleanup::increasing(prunes@),
            super::cleanup::increasing(expires@),
            expire_keys@.len() == expires@.len(),
            forall|a: int|
                0 <= a < prunes@.len() ==> (#[trigger] prunes@[a] as int) < i && cands@[prunes@[a] as int].reason
                    == CleanupReason::Prune,
            forall|a: int|
                0 <= a < expires@.len() ==> (#[trigger] expires@[a] as int) < i
                    && cands@[expires@[a] as int].reason == CleanupReason::Expire
                    && expire_keys@[a] == cands@[expires@[a] as int].last_used_secs,
            inv.len() == i,
            forall|k: int|
                0 <= k < i && cands@[k].reason == CleanupReason::Prune ==> 0 <= #[trigger] inv[k]
                    < prunes@.len() && prunes@[inv[k]] as int == k,
            forall|k: int|
                0 <= k < i && cands@[k].reason == CleanupReason::Expire ==> 0 <= #[trigger] inv[k]
                    < expires@.len() && expires@[inv[k]] as int == k,
            prunes@.len() + expires@.len() == super::cleanup::removal_count(cands@.take(i as int)),
        decreases n - i,
    {
        proof {
            super::cleanup::lemma_removal_count_step(cands@, i as int);
            let slot = match cands@[i as int].reason {
                CleanupReason::Prune => prunes@.len() as int,
                CleanupReason::Expire => expires@.len() as int,
                CleanupReason::Keep => 0,
            };
            inv = inv.push(slot);
        }
        match cands[i].reason {
            CleanupReason::Prune => prunes.push(i),
            CleanupReason::Expire => {
                expires.push(i);
                expire_keys.push(cands[i].last_used_secs);
            },
            CleanupReason::Keep => {},
        }
        i += 1;
    }
    proof {
        assert(cands@.take(n as int) =~= cands@);
        assert forall|k: int|
            0 <= k < n && #[trigger] cands@[k].reason == CleanupReason::Prune implies super::render::occurs(
            prunes@,
            k,
        ) by {
            assert(prunes@[inv[k]] as int == k);
        }
        assert forall|k: int|
            0 <= k < n && #[trigger] cands@[k].reason == CleanupReason::Expire implies super::render::occurs(
            expires@,
            k,
        ) by {
            assert(expires@[inv[k]] as int == k);
        }
    }
    SplitCandidates { prunes, expires, expire_keys }
}

/// Orders the removal candidates of a scan (`is_removal_order`): prune
/// candidates in scan order, then expire candidates by the stable order of
/// their last-use times.
pub fn order_removals(cands: &[Candidate]) -> (order: Vec<usize>)
    ensures
        is_removal_order(cands@, order@),
{
    let split = split_candidates(cands);
    let expires = split.expires;
    let expire_keys = split.expire_keys;
    let local = stable_order(expire_keys.as_slice());
    let mut order = split.prunes;
    let ghost prune_part = order@;
    let ghost prunes: int = order@.len() as int;
    let mut a: usize = 0;
    while a < local.len()
        invariant
            prunes == prune_part.len(),
            lists_positions(cands@, CleanupReason::Prune, prune_part),
            lists_positions(cands@, CleanupReason::Expire, expires@),
            expire_keys@.len() == expires@.len(),
            forall|x: int|
                0 <= x < expires@.len() ==> #[trigger] expire_keys@[x] == cands@[expires@[x] as int].last_used_secs,
            prune_part.len() + expires@.len() == super::cleanup::removal_count(cands@),
            is_stable_order(expire_keys@, local@),
            a <= local@.len(),
            order@.len() == prunes + a,
            forall|x: int| 0 <= x < prunes ==> #[trigger] order@[x] == prune_part[x],
            forall|x: int|
                0 <= x < a ==> #[trigger] order@[prunes + x] == expires@[local@[x] as int],
        decreases local@.len() - a,
    {
        order.push(expires[local[a]]);
        a += 1;
    }
    proof {
        lemma_concatenated_order(cands@, prune_part, expires@, expire_keys@, local@, order@);
    }
    order
}

/// The prune list followed by the expire positions in the stable order of
/// their last-use times is the removal order.
proof fn lemma_concatenated_order(
    cands: Seq<Candidate>,
    prune_part: Seq<usize>,
    expires: Seq<usize>,
    expire_keys: Seq<u64>,
    local: Seq<usize>,
    o: Seq<usize>,
)
    requires
        lists_positions(cands, CleanupReason::Prune, prune_part),
        lists_positions(cands, CleanupReason::Expire, expires),
        expire_keys.len() == expires.len(),
        forall|x: int|
            0 <= x < expires.len() ==> #[trigger] expire_keys[x] == cands[expires[x] as int].last_used_secs,
        prune_part.len() + expires.len() == super::cleanup::removal_count(cands),
        is_stable_order(expire_keys, local),
        o.len() == prune_part.len() + local.len(),
        forall|x: int| 0 <= x < prune_part.len() ==> #[trigger] o[x] == prune_part[x],
        forall|x: int|
            0 <= x < local.len() ==> #[trigger] o[prune_part.len() + x] == expires[local[x] as int],
    ensures
        is_removal_order(cands, o),
{
    let p = prune_part.len() as int;
    assert forall|x: int| 0 <= x < o.len() implies (#[trigger] o[x] as int) < cands.len()
        && cands[o[x] as int].reason != CleanupReason::Keep by {
        if x >= p {
            assert(o[p + (x - p)] == expires[local[x - p] as int]);
        }
    }
    assert forall|x: int, y: int| 0 <= x < y < o.len() implies removal_before(
        cands,
        #[trigger] o[x] as int,
        #[trigger] o[y] as int,
    ) by {
        if y < p {
            assert(o[x] == prune_part[x] && o[y] == prune_part[y]);
        } else if x < p {
            assert(o[x] == prune_part[x]);
            assert(o[p + (y - p)] == expires[local[y - p] as int]);
        } else {
            let lx = local[x - p] as int;
            let ly = local[y - p] as int;
            assert(o[p + (x - p)] == expires[lx]);
            assert(o[p + (y - p)] == expires[ly]);
            assert(key_before(expire_keys, lx, ly));
            if lx < ly {
                assert(expires[lx] < expires[ly]);
            }
        }
    }
    assert forall|k: int|
        0 <= k < cands.len() && #[trigger] cands[k].reason != CleanupReason::Keep implies super::render::occurs(
        o,
        k,
    ) by {
        if cands[k].reason == CleanupReason::Prune {
            assert(super::render::occurs(prune_part, k));
            let x = choose|x: int| 0 <= x < prune_part.len() && prune_part[x] as int == k;
            assert(o[x] == prune_part[x]);
        } else {
            assert(super::render::occurs(expires, k));
            let e = choose|e: int| 0 <= e < expires.len() && expires[e] as int == k;
            assert(super::render::occurs(local, e));
            let x = choose|x: int| 0 <= x < local.len() && local[x] as int == e;
            assert(o[p + x] == expires[local[x] as int]);
        }
    }
}

} // verus!
