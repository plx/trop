//! Verified kernel: bounded symlink following for safe canonicalization.
//!
//! `follow_links` follows a chain of symbolic links from a start state,
//! generic over the state type and over two oracles: `step` inspects one
//! state (a link to another state, not a link, missing, or an error) and
//! `same` decides whether two states name the same path. It returns how the
//! chain ended together with a ghost record of the walk (`Walk`): every
//! state the oracle was asked about, in order, and (after a link that was
//! not followed) that link's target. Proved (`followed`):
//!
//! - **The chain is real.** It starts at the start state, and every state
//!   but the last is a link to the next one (`follows`).
//! - **At most `max_depth` links are followed.** `TooDeep` is reported only
//!   when exactly `max_depth` links were followed and the state reached is a
//!   link again, so a chain of exactly `max_depth` links resolves, and
//!   `max_depth == 0` accepts a state that is not a link (the historical loop
//!   checked the depth before reading the link and rejected both).
//! - **Loop detection is sound and complete.** `Loop(s)` is reported only
//!   when `s` is the same (per `same`) as an earlier state of the chain
//!   (`walk.earlier`), and `same` said "different" for `s` and every state
//!   before that one, so it is the first match. (That the states after it
//!   were never compared with `s` holds by construction; it is not proved.)
//!   On every outcome, `same` said "different" for every pair of states the
//!   `step` oracle inspected (`distinct`; for `Loop` and `TooDeep`, every
//!   state but the last).
//! - **Every other outcome is the last state's own answer:** `Resolved` (not
//!   a link), `Missing`, or `Failed` with the oracle's error.
//! - It terminates for every oracle: the measure is `max_depth - depth`.
//!
//! Production glue: `crate::path::canonicalize` (`canonicalize_safe`, and
//! `canonicalize_bounded`, which `PathResolver` uses with its
//! `max_symlink_depth`). The glue's `step` classifies `fs::symlink_metadata`
//! and joins a relative link target to the link's parent with `Path::join`;
//! `same` is `Path` equality. Both are trusted, as are the final
//! `fs::canonicalize` of a resolved state and the error mapping.

use vstd::prelude::*;

verus! {

/// What inspecting one state found.
pub enum LinkStep<S, E> {
    /// The state is a symbolic link to this state.
    Link(S),
    /// The state exists and is not a symbolic link.
    NotLink,
    /// The state does not exist.
    Missing,
    /// Inspecting the state failed.
    Failed(E),
}

/// How following the chain ended.
pub enum Followed<S, E> {
    /// The chain ended at this state, which is not a link.
    Resolved(S),
    /// The chain ended at this state, which does not exist.
    Missing(S),
    /// This state is the same as an earlier state of the chain.
    Loop(S),
    /// `max_depth` links were followed and the state reached is a link
    /// again.
    TooDeep,
    /// Inspecting this state failed with this error.
    Failed(S, E),
}

/// The ghost record of a walk: the chain of states (for `TooDeep`, followed
/// by the target of the link that was not followed), and for `Loop` the
/// index of the earlier state that the last one repeats.
// A ghost type: only specifications use it. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint.
#[allow(dead_code)]
pub ghost struct Walk<S> {
    /// The states, in order.
    pub chain: Seq<S>,
    /// For `Loop`: the index of the earlier, same state.
    pub earlier: int,
}

/// Every state of `chain` but the last is a link to the next one.
pub open spec fn follows<S, E, St: Fn(&S) -> LinkStep<S, E>>(step: St, chain: Seq<S>) -> bool {
    forall|i: int|
        0 <= i < chain.len() - 1 ==> #[trigger] step.ensures(
            (&chain[i],),
            LinkStep::<S, E>::Link(chain[i + 1]),
        )
}

/// `same` said "different" for every pair of states of `chain`.
pub open spec fn distinct<S, Eq: Fn(&S, &S) -> bool>(same: Eq, chain: Seq<S>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < chain.len() ==> #[trigger] same.ensures((&chain[i], &chain[j]), false)
}

/// `r` and `walk` are a correct end of following links from `start`, with
/// at most `max_depth` links followed.
pub open spec fn followed<S, E, St: Fn(&S) -> LinkStep<S, E>, Eq: Fn(&S, &S) -> bool>(
    start: S,
    max_depth: int,
    step: St,
    same: Eq,
    walk: Walk<S>,
    r: Followed<S, E>,
) -> bool {
    let chain = walk.chain;
    &&& 1 <= chain.len()
    &&& chain[0] == start
    &&& follows(step, chain)
    &&& match r {
        Followed::Resolved(s) => {
            &&& chain.len() <= max_depth + 1
            &&& s == chain.last()
            &&& step.ensures((&s,), LinkStep::<S, E>::NotLink)
            &&& distinct(same, chain)
        },
        Followed::Missing(s) => {
            &&& chain.len() <= max_depth + 1
            &&& s == chain.last()
            &&& step.ensures((&s,), LinkStep::<S, E>::Missing)
            &&& distinct(same, chain)
        },
        Followed::Failed(s, e) => {
            &&& chain.len() <= max_depth + 1
            &&& s == chain.last()
            &&& step.ensures((&s,), LinkStep::<S, E>::Failed(e))
            &&& distinct(same, chain)
        },
        // The states `chain[0..=max_depth]` were visited (`max_depth` links
        // followed), and the last of them is a link to `chain.last()`, which
        // was not followed.
        Followed::TooDeep => {
            &&& chain.len() == max_depth + 2
            &&& distinct(same, chain.drop_last())
        },
        // `s` repeats `chain[walk.earlier]`, the first earlier state `same`
        // matched it with. (That the states after that one were never
        // compared with `s` holds by construction and is not stated here.)
        Followed::Loop(s) => {
            &&& 2 <= chain.len() <= max_depth + 1
            &&& s == chain.last()
            &&& distinct(same, chain.drop_last())
            &&& 0 <= walk.earlier < chain.len() - 1
            &&& same.ensures((&chain[walk.earlier], &s), true)
            &&& forall|q: int|
                0 <= q < walk.earlier ==> #[trigger] same.ensures((&chain[q], &s), false)
        },
    }
}

/// Follows links from `start`, at most `max_depth` of them (see `followed`).
pub fn follow_links<S, E, St, Eq>(start: S, max_depth: usize, step: &St, same: &Eq) -> (r: (
    Followed<S, E>,
    Ghost<Walk<S>>,
)) where St: Fn(&S) -> LinkStep<S, E>, Eq: Fn(&S, &S) -> bool
    requires
        forall|s: &S| step.requires((s,)),
        forall|a: &S, b: &S| same.requires((a, b)),
    ensures
        followed(start, max_depth as int, *step, *same, r.1@, r.0),
{
    let mut visited: Vec<S> = Vec::new();
    let mut current = start;
    let mut depth: usize = 0;
    loop
        invariant
            forall|s: &S| step.requires((s,)),
            forall|a: &S, b: &S| same.requires((a, b)),
            visited@.len() == depth,
            depth <= max_depth,
            visited@.push(current)[0] == start,
            follows(*step, visited@.push(current)),
            distinct(*same, visited@),
        decreases max_depth - depth,
    {
        let ghost chain = visited@.push(current);
        // Is the current state one already visited?
        let mut k: usize = 0;
        while k < visited.len()
            invariant
                forall|a: &S, b: &S| same.requires((a, b)),
                k <= visited@.len(),
                chain == visited@.push(current),
                visited@.len() == depth,
                depth <= max_depth,
                chain[0] == start,
                follows(*step, chain),
                distinct(*same, visited@),
                forall|q: int| 0 <= q < k ==> #[trigger] same.ensures((&visited@[q], &current), false),
            decreases visited.len() - k,
        {
            if same(&visited[k], &current) {
                proof {
                    assert(chain.drop_last() =~= visited@);
                    assert(chain[k as int] == visited@[k as int]);
                    assert(same.ensures((&chain[k as int], &current), true));
                    assert forall|q: int| 0 <= q < k implies #[trigger] same.ensures(
                        (&chain[q], &current),
                        false,
                    ) by {
                        assert(chain[q] == visited@[q]);
                    }
                }
                return (Followed::Loop(current), Ghost(Walk { chain, earlier: k as int }));
            }
            k += 1;
        }
        proof {
            assert forall|i: int, j: int| 0 <= i < j < chain.len() implies #[trigger] same.ensures(
                (&chain[i], &chain[j]),
                false,
            ) by {
                if j < visited@.len() {
                    assert(chain[i] == visited@[i] && chain[j] == visited@[j]);
                } else {
                    assert(chain[i] == visited@[i] && chain[j] == current);
                }
            }
        }
        match step(&current) {
            LinkStep::Link(next) => {
                let ghost next_chain = chain.push(next);
                proof {
                    assert forall|i: int| 0 <= i < next_chain.len() - 1 implies #[trigger] step.ensures(
                        (&next_chain[i],),
                        LinkStep::<S, E>::Link(next_chain[i + 1]),
                    ) by {
                        if i < chain.len() - 1 {
                            assert(next_chain[i] == chain[i] && next_chain[i + 1] == chain[i + 1]);
                        }
                    }
                    assert(next_chain.drop_last() =~= chain);
                }
                if depth == max_depth {
                    return (Followed::TooDeep, Ghost(Walk { chain: next_chain, earlier: 0 }));
                }
                visited.push(current);
                current = next;
                depth += 1;
                proof {
                    assert(visited@.push(current) =~= next_chain);
                    assert(visited@ =~= chain);
                };
            },
            LinkStep::NotLink => {
                return (Followed::Resolved(current), Ghost(Walk { chain, earlier: 0 }));
            },
            LinkStep::Missing => {
                return (Followed::Missing(current), Ghost(Walk { chain, earlier: 0 }));
            },
            LinkStep::Failed(e) => {
                return (Followed::Failed(current, e), Ghost(Walk { chain, earlier: 0 }));
            },
        }
    }
}

} // verus!
