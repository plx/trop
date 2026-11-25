//! Verified kernel: single-port availability classification and allocation
//! scans.
//!
//! The allocator's decisions interleave pure checks with effects: a database
//! lookup (is the port reserved by another key?), the exclusion list, and an
//! operating-system occupancy probe. The kernel is generic over three
//! *oracles* — closures the production glue builds over the database
//! connection, the exclusion manager, and the occupancy checker:
//!
//! - `reserved: Fn(u16) -> Result<bool, E>` — reserved by a key other than the
//!   one being replaced (the glue decides that ownership rule; no
//!   specification here mentions the replaced key);
//! - `excluded: Fn(u16) -> bool` — on the exclusion list;
//! - `occupied: Fn(u16) -> Result<bool, E>` — the occupancy probe.
//!
//! Every result is specified *relative to what the oracles answered*
//! (`oracle.ensures((p,), answer)`): the database and the OS are not modeled,
//! and nothing here rules out the world changing between two queries. Each
//! outcome is justified by *some* answer of each oracle, so the
//! specifications do not rule out asking an oracle twice and acting on the
//! later answer; calling each oracle at most once per port holds by
//! construction (and is pinned for the probe by tests). Under a consistent
//! snapshot (`deterministic_oracles`), the lemmas show that the
//! classification is a function of the snapshot and that the forward scan
//! returns the least available port.
//!
//! The kernel only ever consults an oracle at a valid port (`1..=65535`): its
//! precondition grants the oracles' preconditions only there. Production glue
//! relies on that to rebuild a `Port` from the `u16` it is handed.
//!
//! Production glue: `crate::port::allocator::PortAllocator`
//! (`port_availability`, `allocate_single_ignoring`,
//! `find_next_available_ignoring`, `find_next_allocatable`,
//! `exhaustion_blockers`) and `PortAvailability` (an alias of
//! [`Availability`]). The caller-side preferred-then-scan fallback
//! ([`allocate_with_fallback`], generic over two single-use allocation
//! oracles, with a verified call discipline) is glued in
//! `crate::operations::reserve`, whose oracle wraps
//! `PortAllocator::allocate_single`.

use vstd::prelude::*;

verus! {

use super::port::PortSpan;

// Spec-only items are erased from ordinary builds, so they are referred to by
// path (`super::port::is_valid_port`) rather than imported.

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Why a port cannot be handed out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// Reserved (in the database) by another key.
    Reserved,
    /// On the exclusion list, or outside the range that applies.
    Excluded,
    /// In use on this machine, according to the occupancy probe.
    Occupied,
}

/// The availability classification of one port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// Every applicable check found the port free.
    Available,
    /// Reserved (in the database) by another key.
    Reserved,
    /// On the exclusion list, or outside the range that applies.
    Excluded,
    /// In use on this machine, according to the occupancy probe.
    Occupied,
}

/// Which checks apply when classifying a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checks {
    /// Ports outside the allocator's range are unavailable (`Excluded`).
    pub require_in_range: bool,
    /// Skip the exclusion list.
    pub ignore_exclusions: bool,
    /// Skip the occupancy probe.
    pub ignore_occupied: bool,
}

/// The outcome of allocating a single port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleOutcome {
    /// This port may be handed out.
    Allocated(u16),
    /// The preferred port (the first field) is unavailable for this reason.
    PreferredUnavailable(u16, Blocker),
    /// No port of the range is available.
    Exhausted,
}

/// An oracle failure during a scan: the port whose classification failed,
/// and the oracle's error.
///
/// The port is the witness the specifications name. (This Verus version
/// cannot prove a postcondition of the form "some port failed with `e`" for
/// a generic error type `E`, so the kernels return the witness instead.)
#[derive(Debug)]
pub struct ScanFailure<E> {
    /// The port being classified when the oracle failed.
    // Production glue only needs the error (which already names the port);
    // the field is read by the specifications, which ordinary builds erase.
    #[allow(dead_code)]
    pub port: u16,
    /// The oracle's error, unchanged.
    pub error: E,
}

/// Which kinds of blockers were seen across a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blockers {
    /// Some port was reserved by another key.
    pub reserved: bool,
    /// Some port was excluded.
    pub excluded: bool,
    /// Some port was occupied.
    pub occupied: bool,
}

// ---------------------------------------------------------------------------
// Specifications
// ---------------------------------------------------------------------------

/// The classification reported for a port that `b` blocks.
pub open spec fn blocked_as(b: Blocker) -> Availability {
    match b {
        Blocker::Reserved => Availability::Reserved,
        Blocker::Excluded => Availability::Excluded,
        Blocker::Occupied => Availability::Occupied,
    }
}

/// The checks every allocation scan applies: all of them.
pub open spec fn scan_checks() -> Checks {
    Checks { require_in_range: true, ignore_exclusions: false, ignore_occupied: false }
}

/// The checks applied to a single-port allocation's preferred port.
pub open spec fn preferred_checks(ignore_exclusions: bool, ignore_occupied: bool) -> Checks {
    Checks { require_in_range: true, ignore_exclusions, ignore_occupied }
}

/// The oracle answered `answer` for port `p`.
pub open spec fn answered<E, F: Fn(u16) -> Result<bool, E>>(oracle: F, p: u16, answer: bool) -> bool {
    oracle.ensures((p,), Ok::<bool, E>(answer))
}

/// The oracle failed with `e` for port `p`.
pub open spec fn failed<E, F: Fn(u16) -> Result<bool, E>>(oracle: F, p: u16, e: E) -> bool {
    oracle.ensures((p,), Err::<bool, E>(e))
}

/// The oracles may be consulted at every valid port (and only there).
pub open spec fn oracles_callable<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(reserved: R, excluded: X, occupied: O) -> bool {
    &&& forall|q: u16| super::port::is_valid_port(q as int) ==> #[trigger] reserved.requires((q,))
    &&& forall|q: u16| super::port::is_valid_port(q as int) ==> #[trigger] excluded.requires((q,))
    &&& forall|q: u16| super::port::is_valid_port(q as int) ==> #[trigger] occupied.requires((q,))
}

/// The oracles answer consistently: one answer per port (a snapshot).
pub open spec fn deterministic_oracles<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(reserved: R, excluded: X, occupied: O) -> bool {
    &&& forall|p: u16, a: Result<bool, E>, b: Result<bool, E>|
        #![trigger reserved.ensures((p,), a), reserved.ensures((p,), b)]
        reserved.ensures((p,), a) && reserved.ensures((p,), b) ==> a == b
    &&& forall|p: u16, a: bool, b: bool|
        #![trigger excluded.ensures((p,), a), excluded.ensures((p,), b)]
        excluded.ensures((p,), a) && excluded.ensures((p,), b) ==> a == b
    &&& forall|p: u16, a: Result<bool, E>, b: Result<bool, E>|
        #![trigger occupied.ensures((p,), a), occupied.ensures((p,), b)]
        occupied.ensures((p,), a) && occupied.ensures((p,), b) ==> a == b
}

/// Port `p` is considered at all: it is a valid port and, when the range is
/// required, inside `range`. Ports outside the scope are `Excluded` without
/// consulting any oracle.
pub open spec fn in_scope(p: u16, checks: Checks, range: PortSpan) -> bool {
    &&& super::port::is_valid_port(p as int)
    &&& checks.require_in_range ==> range.spec_min() <= p <= range.spec_max()
}

/// The exclusion check passes: it is skipped, or the list does not hold `p`.
pub open spec fn exclusion_clear<X: Fn(u16) -> bool>(p: u16, checks: Checks, excluded: X) -> bool {
    checks.ignore_exclusions || excluded.ensures((p,), false)
}

/// `r` is a correct classification of port `p` under `checks`, given the
/// oracles' answers.
///
/// The checks run in a fixed order — scope, reservation, exclusion,
/// occupancy — and each outcome is justified by exactly the answers that
/// order requires:
///
/// - a port outside the scope is `Excluded` (that no oracle is consulted for
///   it is proved only for port 0, where `oracles_callable` grants no
///   precondition);
/// - `Reserved` needs the reservation oracle to have said "reserved"; no flag
///   bypasses it;
/// - `Excluded` needs "not reserved" and the (non-ignored) exclusion list to
///   hold `p`;
/// - `Occupied` needs "not reserved", the exclusion check passing, and the
///   (non-ignored) probe to have said "occupied";
/// - `Available` needs *every* applicable check to have said "free" (fail
///   closed, relative to some answer each oracle gave: `answered` is
///   `ensures`, not "the only answer");
/// - an error is exactly an error the reservation oracle returned, or one the
///   probe returned after the reservation and exclusion checks passed (so a
///   probe failure never surfaces for a reserved or excluded port, nor when
///   the probe is ignored).
pub open spec fn classified<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    p: u16,
    checks: Checks,
    range: PortSpan,
    reserved: R,
    excluded: X,
    occupied: O,
    r: Result<Availability, E>,
) -> bool {
    if !in_scope(p, checks, range) {
        r == Ok::<Availability, E>(Availability::Excluded)
    } else {
        match r {
            Ok(Availability::Reserved) => answered(reserved, p, true),
            Ok(Availability::Excluded) => answered(reserved, p, false) && !checks.ignore_exclusions
                && excluded.ensures((p,), true),
            Ok(Availability::Occupied) => answered(reserved, p, false) && exclusion_clear(
                p,
                checks,
                excluded,
            ) && !checks.ignore_occupied && answered(occupied, p, true),
            Ok(Availability::Available) => answered(reserved, p, false) && exclusion_clear(
                p,
                checks,
                excluded,
            ) && (checks.ignore_occupied || answered(occupied, p, false)),
            Err(e) => failed(reserved, p, e) || (answered(reserved, p, false) && exclusion_clear(
                p,
                checks,
                excluded,
            ) && !checks.ignore_occupied && failed(occupied, p, e)),
        }
    }
}

/// A scan found port `p` unavailable (classified, with all checks, as
/// something other than `Available`).
pub open spec fn blocked<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(p: u16, range: PortSpan, reserved: R, excluded: X, occupied: O) -> bool {
    ||| classified(p, scan_checks(), range, reserved, excluded, occupied, Ok(Availability::Reserved))
    ||| classified(p, scan_checks(), range, reserved, excluded, occupied, Ok(Availability::Excluded))
    ||| classified(p, scan_checks(), range, reserved, excluded, occupied, Ok(Availability::Occupied))
}

/// A scan classified port `p` successfully (as anything).
pub open spec fn settled<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(p: u16, range: PortSpan, reserved: R, excluded: X, occupied: O) -> bool {
    ||| classified(p, scan_checks(), range, reserved, excluded, occupied, Ok(Availability::Available))
    ||| blocked(p, range, reserved, excluded, occupied)
}

/// `r` is a correct result of the forward scan from `start` to the top of
/// `range`:
///
/// - `Some(p)`: `p` (at or after `start`) was classified `Available`, and
///   every earlier candidate was found unavailable (the lowest free port);
/// - `None`: every candidate in `start..=max` was found unavailable;
/// - a failure: the classification of candidate `port` failed with exactly
///   that error, after every earlier candidate was found unavailable.
pub open spec fn scanned<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    start: u16,
    range: PortSpan,
    reserved: R,
    excluded: X,
    occupied: O,
    r: Result<Option<u16>, ScanFailure<E>>,
) -> bool {
    match r {
        Ok(Some(p)) => {
            &&& start <= p
            &&& classified(
                p,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Available),
            )
            &&& forall|q: u16|
                start <= q < p ==> #[trigger] blocked(q, range, reserved, excluded, occupied)
        },
        Ok(None) => forall|q: u16|
            start <= q <= range.spec_max() ==> #[trigger] blocked(
                q,
                range,
                reserved,
                excluded,
                occupied,
            ),
        Err(failure) => {
            &&& start <= failure.port <= range.spec_max()
            &&& classified(
                failure.port,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Err(failure.error),
            )
            &&& forall|q: u16|
                start <= q < failure.port ==> #[trigger] blocked(
                    q,
                    range,
                    reserved,
                    excluded,
                    occupied,
                )
        },
    }
}

/// `r` is a correct single-port allocation.
///
/// With a preferred port `p`: `Allocated(p)` exactly when `p` is classified
/// `Available` under the preferred checks (in range — always required — not
/// reserved by another key, and not excluded / not occupied unless the
/// corresponding flag says to ignore that check); otherwise
/// `PreferredUnavailable(p, why)` or the classification's error. Never
/// `Exhausted`, and no other port.
///
/// Without one: the forward scan from the bottom of the range with all checks
/// (the ignore flags do not apply), mapped as `Some(q)` to `Allocated(q)` and
/// `None` to `Exhausted`. Never `PreferredUnavailable`.
pub open spec fn allocated<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    range: PortSpan,
    preferred: Option<u16>,
    ignore_exclusions: bool,
    ignore_occupied: bool,
    reserved: R,
    excluded: X,
    occupied: O,
    r: Result<SingleOutcome, ScanFailure<E>>,
) -> bool {
    match preferred {
        Some(p) => {
            let checks = preferred_checks(ignore_exclusions, ignore_occupied);
            match r {
                Ok(SingleOutcome::Allocated(q)) => q == p && classified(
                    p,
                    checks,
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Ok(Availability::Available),
                ),
                Ok(SingleOutcome::PreferredUnavailable(q, b)) => q == p && classified(
                    p,
                    checks,
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Ok(blocked_as(b)),
                ),
                Ok(SingleOutcome::Exhausted) => false,
                Err(failure) => failure.port == p && classified(
                    p,
                    checks,
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Err(failure.error),
                ),
            }
        },
        None => match r {
            Ok(SingleOutcome::Allocated(q)) => scanned(
                range.spec_min(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Some(q)),
            ),
            Ok(SingleOutcome::PreferredUnavailable(..)) => false,
            Ok(SingleOutcome::Exhausted) => scanned(
                range.spec_min(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(None),
            ),
            Err(failure) => scanned(
                range.spec_min(),
                range,
                reserved,
                excluded,
                occupied,
                Err(failure),
            ),
        },
    }
}

/// Port `p`'s classification is reflected in `b`: it was found available, or
/// the flag for its blocker is set.
pub open spec fn accounted<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(b: Blockers, p: u16, range: PortSpan, reserved: R, excluded: X, occupied: O) -> bool {
    ||| classified(p, scan_checks(), range, reserved, excluded, occupied, Ok(Availability::Available))
    ||| b.reserved && classified(
        p,
        scan_checks(),
        range,
        reserved,
        excluded,
        occupied,
        Ok(Availability::Reserved),
    )
    ||| b.excluded && classified(
        p,
        scan_checks(),
        range,
        reserved,
        excluded,
        occupied,
        Ok(Availability::Excluded),
    )
    ||| b.occupied && classified(
        p,
        scan_checks(),
        range,
        reserved,
        excluded,
        occupied,
        Ok(Availability::Occupied),
    )
}

/// Some port of `range` was classified `a` (with all checks).
pub open spec fn witnessed<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(range: PortSpan, reserved: R, excluded: X, occupied: O, a: Availability) -> bool {
    exists|p: u16|
        range.spec_min() <= p <= range.spec_max() && #[trigger] classified(
            p,
            scan_checks(),
            range,
            reserved,
            excluded,
            occupied,
            Ok::<Availability, E>(a),
        )
}

// ---------------------------------------------------------------------------
// Executable kernels
// ---------------------------------------------------------------------------

impl Availability {
    /// The blocker behind an unavailable classification: `None` exactly for
    /// `Available`, and otherwise the blocker this classification reports.
    pub const fn blocker(self) -> (b: Option<Blocker>)
        ensures
            b is None <==> self == Availability::Available,
            b matches Some(x) ==> blocked_as(x) == self,
    {
        match self {
            Availability::Available => None,
            Availability::Reserved => Some(Blocker::Reserved),
            Availability::Excluded => Some(Blocker::Excluded),
            Availability::Occupied => Some(Blocker::Occupied),
        }
    }
}

/// Classifies port `p` under `checks` (see `classified`).
///
/// Laziness is by construction: the reservation oracle runs only for a port
/// in scope, the exclusion list only after "not reserved" (and only when not
/// ignored), and the probe only after both passed (and only when not ignored).
pub fn classify_port<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(p: u16, checks: Checks, range: PortSpan, reserved: &R, excluded: &X, occupied: &O) -> (r: Result<
    Availability,
    E,
>)
    requires
        oracles_callable(*reserved, *excluded, *occupied),
    ensures
        classified(p, checks, range, *reserved, *excluded, *occupied, r),
{
    if p == 0 || (checks.require_in_range && !range.contains(p)) {
        return Ok(Availability::Excluded);
    }
    match reserved(p) {
        Err(e) => return Err(e),
        Ok(true) => return Ok(Availability::Reserved),
        Ok(false) => {},
    }
    if !checks.ignore_exclusions && excluded(p) {
        return Ok(Availability::Excluded);
    }
    if checks.ignore_occupied {
        return Ok(Availability::Available);
    }
    match occupied(p) {
        Err(e) => Err(e),
        Ok(true) => Ok(Availability::Occupied),
        Ok(false) => Ok(Availability::Available),
    }
}

/// The forward scan: the lowest port in `start..=range.max` that is
/// `Available` under all checks (see `scanned`).
///
/// Candidates are classified one at a time in ascending order; the scan stops
/// at the first available port or the first error. The cursor never
/// overflows, including when the range ends at 65535.
pub fn first_available<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(start: u16, range: PortSpan, reserved: &R, excluded: &X, occupied: &O) -> (r: Result<
    Option<u16>,
    ScanFailure<E>,
>)
    requires
        oracles_callable(*reserved, *excluded, *occupied),
    ensures
        scanned(start, range, *reserved, *excluded, *occupied, r),
        r matches Ok(Some(p)) ==> range.spec_min() <= p <= range.spec_max()
            && super::port::is_valid_port(p as int),
{
    let end = range.max();
    if start > end {
        return Ok(None);
    }
    let checks = Checks { require_in_range: true, ignore_exclusions: false, ignore_occupied: false };
    let mut p = start;
    loop
        invariant
            start <= p <= end,
            end == range.spec_max(),
            checks == scan_checks(),
            oracles_callable(*reserved, *excluded, *occupied),
            forall|q: u16|
                start <= q < p ==> #[trigger] blocked(q, range, *reserved, *excluded, *occupied),
        decreases end - p,
    {
        let answer = classify_port(p, checks, range, reserved, excluded, occupied);
        match answer {
            Err(error) => {
                return Err(ScanFailure { port: p, error });
            },
            Ok(Availability::Available) => {
                return Ok(Some(p));
            },
            Ok(_) => {
                assert(blocked(p, range, *reserved, *excluded, *occupied));
            },
        }
        if p == end {
            return Ok(None);
        }
        p += 1;
    }
}

/// Allocates one port (see `allocated`): the preferred port if it passes
/// the preferred checks, otherwise — only when no port was preferred — the
/// lowest available port of the range.
pub fn allocate_single<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    range: PortSpan,
    preferred: Option<u16>,
    ignore_exclusions: bool,
    ignore_occupied: bool,
    reserved: &R,
    excluded: &X,
    occupied: &O,
) -> (r: Result<SingleOutcome, ScanFailure<E>>)
    requires
        oracles_callable(*reserved, *excluded, *occupied),
    ensures
        allocated(
            range,
            preferred,
            ignore_exclusions,
            ignore_occupied,
            *reserved,
            *excluded,
            *occupied,
            r,
        ),
        r matches Ok(SingleOutcome::Allocated(p)) ==> range.spec_min() <= p <= range.spec_max()
            && super::port::is_valid_port(p as int),
{
    match preferred {
        Some(p) => {
            let checks = Checks { require_in_range: true, ignore_exclusions, ignore_occupied };
            match classify_port(p, checks, range, reserved, excluded, occupied) {
                Err(error) => Err(ScanFailure { port: p, error }),
                Ok(availability) => match availability.blocker() {
                    None => Ok(SingleOutcome::Allocated(p)),
                    Some(blocker) => Ok(SingleOutcome::PreferredUnavailable(p, blocker)),
                },
            }
        },
        None => match first_available(range.min(), range, reserved, excluded, occupied) {
            Err(failure) => Err(failure),
            Ok(Some(p)) => Ok(SingleOutcome::Allocated(p)),
            Ok(None) => Ok(SingleOutcome::Exhausted),
        },
    }
}

/// Classifies every port of `range` (all checks, ascending) and records which
/// kinds of blockers occurred.
///
/// Each flag is set only if some port of the range was classified that way,
/// and every port's classification is reflected in the flags. If no flag is
/// set, some port was found available — which, after a scan reported the
/// range exhausted, can only happen if the world changed in between (see
/// `lemma_exhausted_range_has_blocker`). Errors propagate (fail closed).
pub fn exhaustion_blockers<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(range: PortSpan, reserved: &R, excluded: &X, occupied: &O) -> (r: Result<
    Blockers,
    ScanFailure<E>,
>)
    requires
        oracles_callable(*reserved, *excluded, *occupied),
    ensures
        r matches Ok(b) ==> {
            &&& forall|p: u16|
                range.spec_min() <= p <= range.spec_max() ==> #[trigger] accounted(
                    b,
                    p,
                    range,
                    *reserved,
                    *excluded,
                    *occupied,
                )
            &&& b.reserved ==> witnessed(
                range,
                *reserved,
                *excluded,
                *occupied,
                Availability::Reserved,
            )
            &&& b.excluded ==> witnessed(
                range,
                *reserved,
                *excluded,
                *occupied,
                Availability::Excluded,
            )
            &&& b.occupied ==> witnessed(
                range,
                *reserved,
                *excluded,
                *occupied,
                Availability::Occupied,
            )
            &&& (b.reserved || b.excluded || b.occupied || witnessed(
                range,
                *reserved,
                *excluded,
                *occupied,
                Availability::Available,
            ))
        },
        r matches Err(failure) ==> {
            &&& range.spec_min() <= failure.port <= range.spec_max()
            &&& classified(
                failure.port,
                scan_checks(),
                range,
                *reserved,
                *excluded,
                *occupied,
                Err(failure.error),
            )
            &&& forall|q: u16|
                range.spec_min() <= q < failure.port ==> #[trigger] settled(
                    q,
                    range,
                    *reserved,
                    *excluded,
                    *occupied,
                )
        },
{
    let lo = range.min();
    let hi = range.max();
    let checks = Checks { require_in_range: true, ignore_exclusions: false, ignore_occupied: false };
    let mut b = Blockers { reserved: false, excluded: false, occupied: false };
    let mut p = lo;
    loop
        invariant
            lo <= p <= hi,
            lo == range.spec_min(),
            hi == range.spec_max(),
            checks == scan_checks(),
            oracles_callable(*reserved, *excluded, *occupied),
            forall|q: u16|
                lo <= q < p ==> #[trigger] accounted(b, q, range, *reserved, *excluded, *occupied),
            forall|q: u16|
                lo <= q < p ==> #[trigger] settled(q, range, *reserved, *excluded, *occupied),
            b.reserved ==> witnessed(range, *reserved, *excluded, *occupied, Availability::Reserved),
            b.excluded ==> witnessed(range, *reserved, *excluded, *occupied, Availability::Excluded),
            b.occupied ==> witnessed(range, *reserved, *excluded, *occupied, Availability::Occupied),
        decreases hi - p,
    {
        let answer = classify_port(p, checks, range, reserved, excluded, occupied);
        let ghost old_b = b;
        match answer {
            Err(error) => {
                return Err(ScanFailure { port: p, error });
            },
            Ok(Availability::Available) => {},
            Ok(Availability::Reserved) => {
                b.reserved = true;
            },
            Ok(Availability::Excluded) => {
                b.excluded = true;
            },
            Ok(Availability::Occupied) => {
                b.occupied = true;
            },
        }
        assert forall|q: u16| lo <= q <= p implies #[trigger] accounted(
            b,
            q,
            range,
            *reserved,
            *excluded,
            *occupied,
        ) by {
            if q < p {
                assert(accounted(old_b, q, range, *reserved, *excluded, *occupied));
            }
        }
        assert(settled(p, range, *reserved, *excluded, *occupied));
        if p == hi {
            proof {
                if !(b.reserved || b.excluded || b.occupied) {
                    assert(accounted(b, lo, range, *reserved, *excluded, *occupied));
                }
            }
            return Ok(b);
        }
        p += 1;
    }
}

// ---------------------------------------------------------------------------
// Lemmas
// ---------------------------------------------------------------------------

/// Under a consistent snapshot, a port has exactly one classification: the
/// specification determines the result completely (including which error).
pub proof fn lemma_classification_unique<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    p: u16,
    checks: Checks,
    range: PortSpan,
    reserved: R,
    excluded: X,
    occupied: O,
    r1: Result<Availability, E>,
    r2: Result<Availability, E>,
)
    requires
        deterministic_oracles(reserved, excluded, occupied),
        classified(p, checks, range, reserved, excluded, occupied, r1),
        classified(p, checks, range, reserved, excluded, occupied, r2),
    ensures
        r1 == r2,
{
}

/// Under a consistent snapshot, the port the forward scan returns is the
/// *least* available port at or after `start`: no earlier candidate is
/// classified `Available`.
pub proof fn lemma_first_available_is_least<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(start: u16, range: PortSpan, reserved: R, excluded: X, occupied: O, p: u16)
    requires
        deterministic_oracles(reserved, excluded, occupied),
        scanned(start, range, reserved, excluded, occupied, Ok(Some(p))),
    ensures
        forall|q: u16|
            start <= q < p ==> !#[trigger] classified(
                q,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Available),
            ),
{
    assert forall|q: u16| start <= q < p implies !#[trigger] classified(
        q,
        scan_checks(),
        range,
        reserved,
        excluded,
        occupied,
        Ok(Availability::Available),
    ) by {
        assert(blocked(q, range, reserved, excluded, occupied));
        if classified(
            q,
            scan_checks(),
            range,
            reserved,
            excluded,
            occupied,
            Ok(Availability::Available),
        ) {
            if classified(
                q,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Reserved),
            ) {
                lemma_classification_unique(
                    q,
                    scan_checks(),
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Ok(Availability::Available),
                    Ok(Availability::Reserved),
                );
            } else if classified(
                q,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Excluded),
            ) {
                lemma_classification_unique(
                    q,
                    scan_checks(),
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Ok(Availability::Available),
                    Ok(Availability::Excluded),
                );
            } else {
                lemma_classification_unique(
                    q,
                    scan_checks(),
                    range,
                    reserved,
                    excluded,
                    occupied,
                    Ok(Availability::Available),
                    Ok(Availability::Occupied),
                );
            }
        }
    }
}

/// Under a consistent snapshot, if the forward scan over a range came back
/// empty, the exhaustion diagnostics name at least one blocker (so the
/// "unavailable" fallback message is reachable only after a race).
pub proof fn lemma_exhausted_range_has_blocker<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(range: PortSpan, reserved: R, excluded: X, occupied: O, b: Blockers)
    requires
        deterministic_oracles(reserved, excluded, occupied),
        scanned(range.spec_min(), range, reserved, excluded, occupied, Ok(None)),
        b.reserved || b.excluded || b.occupied || witnessed(
            range,
            reserved,
            excluded,
            occupied,
            Availability::Available,
        ),
    ensures
        b.reserved || b.excluded || b.occupied,
{
    if !(b.reserved || b.excluded || b.occupied) {
        let p = choose|p: u16|
            range.spec_min() <= p <= range.spec_max() && #[trigger] classified(
                p,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok::<Availability, E>(Availability::Available),
            );
        lemma_first_available_is_least(range.spec_min(), range, reserved, excluded, occupied, p);
        // `p` is blocked (the scan found nothing), yet classified Available.
        assert(blocked(p, range, reserved, excluded, occupied));
        if classified(
            p,
            scan_checks(),
            range,
            reserved,
            excluded,
            occupied,
            Ok(Availability::Reserved),
        ) {
            lemma_classification_unique(
                p,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Available),
                Ok(Availability::Reserved),
            );
        } else if classified(
            p,
            scan_checks(),
            range,
            reserved,
            excluded,
            occupied,
            Ok(Availability::Excluded),
        ) {
            lemma_classification_unique(
                p,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Available),
                Ok(Availability::Excluded),
            );
        } else {
            lemma_classification_unique(
                p,
                scan_checks(),
                range,
                reserved,
                excluded,
                occupied,
                Ok(Availability::Available),
                Ok(Availability::Occupied),
            );
        }
    }
}

/// Turning an ignore flag on never turns an allocatable preferred port into an
/// unavailable one: with the same oracle answers, `Available` under `checks`
/// stays `Available` under any `wider` checks.
pub proof fn lemma_ignore_flags_monotone<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    p: u16,
    checks: Checks,
    wider: Checks,
    range: PortSpan,
    reserved: R,
    excluded: X,
    occupied: O,
)
    requires
        wider.require_in_range == checks.require_in_range,
        checks.ignore_exclusions ==> wider.ignore_exclusions,
        checks.ignore_occupied ==> wider.ignore_occupied,
        classified(p, checks, range, reserved, excluded, occupied, Ok(Availability::Available)),
    ensures
        classified(p, wider, range, reserved, excluded, occupied, Ok(Availability::Available)),
{
}

/// The ignore flags only ever bypass their own check. Under any flags, an
/// available port was answered "not reserved"; with exclusions honored it was
/// answered "not excluded"; with occupancy honored the probe answered "free".
/// With both flags set (`--force`), an in-range port is unavailable only
/// because it is reserved by another key, and only the reservation lookup can
/// fail.
pub proof fn lemma_ignore_flags_are_independent<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    p: u16,
    checks: Checks,
    range: PortSpan,
    reserved: R,
    excluded: X,
    occupied: O,
    r: Result<Availability, E>,
)
    requires
        classified(p, checks, range, reserved, excluded, occupied, r),
    ensures
        r == Ok::<Availability, E>(Availability::Available) ==> {
            &&& answered(reserved, p, false)
            &&& !checks.ignore_exclusions ==> excluded.ensures((p,), false)
            &&& !checks.ignore_occupied ==> answered(occupied, p, false)
        },
        checks.ignore_exclusions && checks.ignore_occupied && in_scope(p, checks, range) ==> {
            &&& r matches Ok(a) ==> a == Availability::Available || a == Availability::Reserved
            &&& r matches Err(e) ==> failed(reserved, p, e)
        },
{
}

/// Without ignore flags, the preferred-port check is exactly the scan's
/// availability classification: a free preferred port is what the scan would
/// have accepted.
pub proof fn lemma_preferred_without_flags_is_scan_check<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(p: u16, range: PortSpan, reserved: R, excluded: X, occupied: O, r: Result<Availability, E>)
    ensures
        classified(p, preferred_checks(false, false), range, reserved, excluded, occupied, r)
            == classified(p, scan_checks(), range, reserved, excluded, occupied, r),
{
}


// ---------------------------------------------------------------------------
// Preferred port, then the scan: the caller-side fallback
// ---------------------------------------------------------------------------

/// One single-port allocation attempt, as a caller observes it (the port
/// type is generic so production glue keeps its own `Port`).
pub enum Attempt<P> {
    /// This port may be handed out.
    Allocated(P),
    /// The preferred port is unavailable.
    PreferredUnavailable,
    /// No port of the range is available.
    Exhausted,
}

/// The outcome of a single allocation with fallback: it cannot represent
/// "preferred port unavailable", so callers need no unreachable arm.
pub enum Fallback<P> {
    /// This port may be handed out.
    Allocated(P),
    /// No port is available.
    Exhausted,
}

/// The allocator, asked for `preferred`, reported it unavailable: the
/// condition under which `allocate_with_fallback` makes the fallback
/// request.
pub open spec fn fell_back<P, E, A: FnOnce(Option<P>) -> Result<Attempt<P>, E>>(
    preferred: Option<P>,
    request: A,
) -> bool {
    preferred is Some && request.ensures((preferred,), Ok::<Attempt<P>, E>(Attempt::PreferredUnavailable))
}

/// The call discipline of `allocate_with_fallback`, stated as the oracles'
/// preconditions (a body that calls an oracle out of turn does not verify
/// against oracles whose preconditions are exactly these):
///
/// - the request asks for `preferred`;
/// - the fallback request may run only after the request reported a given
///   preference unavailable (`fell_back`), and it asks for no preference.
///
/// Each oracle is an `FnOnce`, so each runs at most once. Ordinary Rust
/// closures have no preconditions, so production glue meets these.
pub open spec fn fallback_discipline<
    P,
    E,
    A: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
    S: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
>(preferred: Option<P>, request: A, fallback: S) -> bool {
    &&& request.requires((preferred,))
    &&& fell_back(preferred, request) ==> fallback.requires((None::<P>,))
}

/// `r` is a correct result of the preferred-then-scan fallback over the
/// allocation oracles `request` (asked for `preferred`) and `fallback`
/// (asked for no preference):
///
/// - `Allocated(p)`: the request allocated `p`, or it reported the
///   preference unavailable and the fallback request allocated `p`;
/// - `Exhausted`: the request reported the range exhausted, or the fallback
///   request did, or an oracle violated its contract by reporting a
///   preference unavailable when none was given (which
///   `lemma_allocator_keeps_fallback_contract` rules out for
///   `allocate_single`);
/// - an error is an oracle's error, unchanged, from the request or from the
///   fallback request.
pub open spec fn fallen_back<
    P,
    E,
    A: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
    S: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
>(preferred: Option<P>, request: A, fallback: S, r: Result<Fallback<P>, E>) -> bool {
    match r {
        Ok(Fallback::Allocated(p)) => request.ensures(
            (preferred,),
            Ok::<Attempt<P>, E>(Attempt::Allocated(p)),
        ) || (fell_back(preferred, request) && fallback.ensures(
            (None::<P>,),
            Ok::<Attempt<P>, E>(Attempt::Allocated(p)),
        )),
        Ok(Fallback::Exhausted) => request.ensures(
            (preferred,),
            Ok::<Attempt<P>, E>(Attempt::Exhausted),
        ) || (preferred is None && request.ensures(
            (preferred,),
            Ok::<Attempt<P>, E>(Attempt::PreferredUnavailable),
        )) || (fell_back(preferred, request) && (fallback.ensures(
            (None::<P>,),
            Ok::<Attempt<P>, E>(Attempt::Exhausted),
        ) || fallback.ensures((None::<P>,), Ok::<Attempt<P>, E>(Attempt::PreferredUnavailable)))),
        Err(e) => request.ensures((preferred,), Err::<Attempt<P>, E>(e)) || (fell_back(
            preferred,
            request,
        ) && fallback.ensures((None::<P>,), Err::<Attempt<P>, E>(e))),
    }
}

/// Allocates the preferred port, or — when the allocator reports it
/// unavailable — whatever the allocator finds with no preference (see
/// `fallen_back`). The call discipline is verified (`fallback_discipline`):
/// at most one request for `preferred` and at most one fallback request,
/// which asks for no preference and runs only after a given preference was
/// reported unavailable. The ignore flags are the caller's business (the
/// oracles close over them). Production passes the same allocation closure
/// as both oracles.
pub fn allocate_with_fallback<
    P,
    E,
    A: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
    S: FnOnce(Option<P>) -> Result<Attempt<P>, E>,
>(preferred: Option<P>, request: A, fallback: S) -> (r: Result<Fallback<P>, E>)
    requires
        // Only the oracles' own (conditional) preconditions: see
        // `fallback_discipline`.
        fallback_discipline(preferred, request, fallback),
    ensures
        fallen_back(preferred, request, fallback, r),
{
    let had_preference = preferred.is_some();
    match request(preferred)? {
        Attempt::Allocated(p) => Ok(Fallback::Allocated(p)),
        Attempt::Exhausted => Ok(Fallback::Exhausted),
        Attempt::PreferredUnavailable => {
            if !had_preference {
                // A contract violation of the oracle: reported as exhaustion
                // rather than a panic.
                return Ok(Fallback::Exhausted);
            }
            match fallback(None)? {
                Attempt::Allocated(p) => Ok(Fallback::Allocated(p)),
                Attempt::Exhausted | Attempt::PreferredUnavailable => Ok(Fallback::Exhausted),
            }
        },
    }
}

/// `allocate_single` keeps the fallback oracle's contract: without a
/// preference it never reports a preferred port unavailable, and with one it
/// never reports the range exhausted. So, over the production allocator,
/// `allocate_with_fallback` answers `Exhausted` only when the scan without
/// preference found every port of the range unavailable.
pub proof fn lemma_allocator_keeps_fallback_contract<
    E,
    R: Fn(u16) -> Result<bool, E>,
    X: Fn(u16) -> bool,
    O: Fn(u16) -> Result<bool, E>,
>(
    range: PortSpan,
    preferred: Option<u16>,
    ignore_exclusions: bool,
    ignore_occupied: bool,
    reserved: R,
    excluded: X,
    occupied: O,
    r: Result<SingleOutcome, ScanFailure<E>>,
)
    requires
        allocated(range, preferred, ignore_exclusions, ignore_occupied, reserved, excluded, occupied, r),
    ensures
        preferred is None ==> !(r matches Ok(SingleOutcome::PreferredUnavailable(..))),
        preferred is Some ==> !(r matches Ok(SingleOutcome::Exhausted)),
        (preferred is None && r matches Ok(SingleOutcome::Exhausted)) ==> forall|q: u16|
            range.spec_min() <= q <= range.spec_max() ==> #[trigger] blocked(
                q,
                range,
                reserved,
                excluded,
                occupied,
            ),
{
}

} // verus!
