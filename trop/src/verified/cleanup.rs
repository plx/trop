//! Verified kernel: cleanup selection, per-reservation reasons, the prune
//! probe classification, probe-once deduplication, the removal plan, the
//! revalidate-then-delete accounting, and the autoclean partition.
//!
//! - **Selection.** `days_to_secs` converts a day count to seconds exactly
//!   (no `u32` day count overflows `u64`). `command_selection` is the
//!   selection of `trop prune` / `expire` / `autoclean` (the threshold as
//!   given), and `automatic_selection` that of reserve's automatic cleanup,
//!   which falls back to the 30-day default whenever expiry is enabled and no
//!   threshold is configured (`lemma_automatic_expiry_always_has_threshold`).
//! - **Reasons.** `cleanup_reason` decides `Prune`, `Expire` or `Keep` for
//!   one reservation: prune exactly when the prune phase runs and the path is
//!   prunable; otherwise expire exactly when the expire phase runs and the
//!   time since the last use exceeds the maximum age (`Duration` order,
//!   `super::expiry::duration_gt`; with whole seconds this is
//!   `super::expiry::expired`, `lemma_whole_second_reason`).
//! - **Probe classification.** `classify_probe` turns one filesystem probe
//!   into a path status. Only definitive evidence (a non-directory target, or
//!   an error proving absence, a non-directory ancestor, or an invalid path)
//!   makes a path prunable; every other error preserves its reservations
//!   (fail closed), with the error kind decided in a fixed precedence.
//! - **Probe once.** `representatives` maps each reservation to the first
//!   reservation with the same path id, so each distinct path is probed at
//!   its first reservation only and every reservation on it shares that one
//!   observation.
//! - **Plan.** `plan_removals` splits the scan into the kept rows (ascending)
//!   and the removal candidates in removal order (`super::sort`); the two
//!   partition the scan.
//! - **Revalidation.** `revalidate_all` re-reads each candidate before its
//!   guarded delete. Each candidate gets exactly one `Fate`, justified by the
//!   oracles' answers; a delete is only ever requested for a candidate just
//!   observed unchanged and still eligible, and a dry run consults no oracle.
//! - **Autoclean partition.** `split_by_reason` splits removals into pruned
//!   and expired lists that partition them, so the counts add up.
//!
//! Production glue: `trop/src/operations/cleanup.rs`. The `io::Error`
//! classification tables, the database oracles, the clock, and the
//! `SystemTime` arithmetic stay outside the kernel.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ===========================================================================
// Selection: which phases run, and the maximum age
// ===========================================================================

/// Seconds in a day.
pub const SECONDS_PER_DAY: u64 = 86400;

/// `days` days in seconds.
pub open spec fn days_secs(days: int) -> int {
    days * 86400
}

/// `days` days in seconds. Exact: `u32::MAX * 86400 < 2^64`, so no day
/// count overflows.
pub fn days_to_secs(days: u32) -> (secs: u64)
    ensures
        secs as int == days_secs(days as int),
{
    let days = u64::from(days);
    assert(days * 86400 <= 4294967295 * 86400) by (nonlinear_arith)
        requires
            days <= 4294967295,
    ;
    days * SECONDS_PER_DAY
}

/// Which cleanup phases one invocation runs: the prune phase, and the expire
/// phase with its maximum age in seconds (`None` when it does not run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The prune phase runs.
    pub prune: bool,
    /// The expire phase runs with this maximum age, in seconds.
    pub max_age_secs: Option<u64>,
}

/// The maximum age of an expiry threshold of `days` days.
pub open spec fn threshold_secs(days: Option<u32>) -> Option<int> {
    match days {
        Some(d) => Some(days_secs(d as int)),
        None => None,
    }
}

/// The selection's maximum age, as an integer.
pub open spec fn selected_max_age(selection: Selection) -> Option<int> {
    match selection.max_age_secs {
        Some(m) => Some(m as int),
        None => None,
    }
}

/// The selection of an explicit cleanup command: `trop prune` (prune, no
/// threshold), `trop expire` (a threshold, no prune) and `trop autoclean`
/// (prune, and expire exactly when a threshold is configured).
pub fn command_selection(prune: bool, expire_after_days: Option<u32>) -> (selection: Selection)
    ensures
        selection.prune == prune,
        selected_max_age(selection) == threshold_secs(expire_after_days),
{
    Selection {
        prune,
        max_age_secs: expire_after_days.map(days_to_secs),
    }
}

/// The expiry threshold automatic cleanup applies: the configured one, or the
/// built-in default when none is configured.
pub open spec fn automatic_days(expire_after_days: Option<u32>) -> u32 {
    match expire_after_days {
        Some(d) => d,
        None => super::config::DEFAULT_EXPIRE_AFTER_DAYS,
    }
}

/// The selection of reserve's automatic cleanup after port exhaustion: the
/// enabled phases, and for the expire phase the configured threshold or the
/// 30-day default (`super::config::DEFAULT_EXPIRE_AFTER_DAYS`). An unset
/// threshold is the default whether the `cleanup` section is missing or
/// present without `expire_after_days`.
pub fn automatic_selection(prune: bool, expire: bool, expire_after_days: Option<u32>) -> (selection:
    Selection)
    ensures
        selection.prune == prune,
        selected_max_age(selection) == if expire {
            Some(days_secs(automatic_days(expire_after_days) as int))
        } else {
            None::<int>
        },
{
    let days = match expire_after_days {
        Some(days) => days,
        None => super::config::DEFAULT_EXPIRE_AFTER_DAYS,
    };
    let max_age_secs = if expire {
        Some(days_to_secs(days))
    } else {
        None
    };
    Selection { prune, max_age_secs }
}

/// An unset threshold behaves exactly as the 30-day default, and enabled
/// automatic expiry runs with a threshold of at least one day whenever the
/// configured threshold is valid (`super::config::expire_after_days_ok`) or
/// unset. (`automatic_selection` gives a maximum age exactly when expiry is
/// enabled.)
pub proof fn lemma_automatic_expiry_always_has_threshold(expire_after_days: Option<u32>)
    ensures
        automatic_days(None) == super::config::DEFAULT_EXPIRE_AFTER_DAYS,
        automatic_days(None) == automatic_days(Some(super::config::DEFAULT_EXPIRE_AFTER_DAYS)),
        days_secs(automatic_days(None) as int) == 30 * 86400,
        (expire_after_days matches Some(d) ==> d >= 1) ==> days_secs(
            automatic_days(expire_after_days) as int,
        ) >= 86400,
{
}

// ===========================================================================
// The reason for one reservation
// ===========================================================================

/// What cleanup does with one reservation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupReason {
    /// Preserved.
    Keep,
    /// Removed by the prune phase: its path is prunable.
    Prune,
    /// Removed by the expire phase: unused for longer than the maximum age.
    Expire,
}

/// Time elapsed since a reservation's last use (`Duration`'s seconds and
/// nanoseconds); absent when the last use lies in the future.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Elapsed {
    /// Whole seconds.
    pub secs: u64,
    /// Nanoseconds past the whole seconds.
    pub nanos: u32,
}

/// The elapsed time is longer than `max_age_secs` seconds (`Duration`'s
/// order); a last use in the future is never older than anything.
pub open spec fn older_than(age: Option<Elapsed>, max_age_secs: int) -> bool {
    match age {
        Some(a) => super::expiry::duration_gt(a.secs as int, a.nanos as int, max_age_secs, 0),
        None => false,
    }
}

/// The selection expires a reservation of this age: the expire phase runs
/// and the age exceeds its maximum age.
pub open spec fn expires(max_age_secs: Option<u64>, age: Option<Elapsed>) -> bool {
    match max_age_secs {
        Some(m) => older_than(age, m as int),
        None => false,
    }
}

/// The reason for one reservation: prune when the prune phase runs and the
/// path is prunable; otherwise expire when the expire phase expires it;
/// otherwise keep.
pub open spec fn spec_reason(
    prune_enabled: bool,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    age: Option<Elapsed>,
) -> CleanupReason {
    if prune_enabled && path_prunable {
        CleanupReason::Prune
    } else if expires(max_age_secs, age) {
        CleanupReason::Expire
    } else {
        CleanupReason::Keep
    }
}

/// Decides one reservation's reason (`spec_reason`). Prune takes precedence,
/// so the prune and expire sets are disjoint; a reason names a phase that
/// runs (`Prune` only with the prune phase, `Expire` only with a maximum
/// age).
pub fn cleanup_reason(
    prune_enabled: bool,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    age: Option<Elapsed>,
) -> (reason: CleanupReason)
    ensures
        reason == spec_reason(prune_enabled, path_prunable, max_age_secs, age),
        (reason == CleanupReason::Prune) <==> prune_enabled && path_prunable,
        (reason == CleanupReason::Expire) <==> !(prune_enabled && path_prunable) && expires(
            max_age_secs,
            age,
        ),
        reason == CleanupReason::Expire ==> max_age_secs is Some,
{
    if prune_enabled && path_prunable {
        return CleanupReason::Prune;
    }
    let expired = match (max_age_secs, age) {
        (Some(max), Some(age)) => super::expiry::duration_exceeds(age.secs, age.nanos, max, 0),
        _ => false,
    };
    if expired {
        CleanupReason::Expire
    } else {
        CleanupReason::Keep
    }
}

/// With whole-second times (a stored `last_used_at` and an evaluation time
/// captured in whole seconds), the age is `now - last_used` when the last use
/// is not in the future, and `Expire` is exactly `super::expiry::expired`:
/// strictly more than the maximum age has passed. So cleanup and the SQL
/// query `last_used_at < super::expiry::expiry_cutoff(now, max)` select the
/// same reservations, and an age of exactly the maximum is kept.
pub proof fn lemma_whole_second_reason(
    prune_enabled: bool,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    now: int,
    last_used: int,
    age: Option<Elapsed>,
)
    requires
        now >= last_used ==> (age matches Some(a) && a.secs == now - last_used && a.nanos == 0),
        now < last_used ==> age is None,
    ensures
        (spec_reason(prune_enabled, path_prunable, max_age_secs, age) == CleanupReason::Expire)
            <==> (max_age_secs matches Some(m) && !(prune_enabled && path_prunable)
            && super::expiry::expired(now, last_used, m as int)),
{
    if let Some(m) = max_age_secs {
        if now >= last_used {
            super::expiry::lemma_whole_second_age_agrees(now, last_used, m as int, 0);
        } else {
            super::expiry::lemma_future_never_expired(now, last_used, m as int);
        }
    }
}

/// A larger maximum age expires a subset: whatever a larger maximum age
/// expires, a smaller one expires too (and the prune decision does not
/// depend on the maximum age).
pub proof fn lemma_larger_max_age_expires_subset(
    prune_enabled: bool,
    path_prunable: bool,
    small: u64,
    large: u64,
    age: Option<Elapsed>,
)
    requires
        small <= large,
    ensures
        spec_reason(prune_enabled, path_prunable, Some(large), age) == CleanupReason::Expire
            ==> spec_reason(prune_enabled, path_prunable, Some(small), age)
            == CleanupReason::Expire,
        (spec_reason(prune_enabled, path_prunable, Some(large), age) == CleanupReason::Prune)
            == (spec_reason(prune_enabled, path_prunable, Some(small), age)
            == CleanupReason::Prune),
{
}

/// A removal candidate is still eligible: its reason's predicate still
/// holds (prune: the path is prunable; expire: the expire phase expires it).
pub open spec fn spec_still_eligible(
    reason: CleanupReason,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    age: Option<Elapsed>,
) -> bool {
    match reason {
        CleanupReason::Prune => path_prunable,
        CleanupReason::Expire => expires(max_age_secs, age),
        CleanupReason::Keep => false,
    }
}

/// Re-checks a candidate's reason against fresh evidence
/// (`spec_still_eligible`).
pub fn still_eligible(
    reason: CleanupReason,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    age: Option<Elapsed>,
) -> (eligible: bool)
    ensures
        eligible == spec_still_eligible(reason, path_prunable, max_age_secs, age),
{
    match reason {
        CleanupReason::Prune => path_prunable,
        CleanupReason::Expire => match (max_age_secs, age) {
            (Some(max), Some(age)) => super::expiry::duration_exceeds(
                age.secs,
                age.nanos,
                max,
                0,
            ),
            _ => false,
        },
        CleanupReason::Keep => false,
    }
}

/// Re-evaluating the same evidence keeps every candidate eligible: a row
/// that is unchanged since planning (same path status, same last use) is
/// still eligible for the reason it was planned with.
pub proof fn lemma_unchanged_candidate_still_eligible(
    prune_enabled: bool,
    path_prunable: bool,
    max_age_secs: Option<u64>,
    age: Option<Elapsed>,
)
    ensures
        spec_reason(prune_enabled, path_prunable, max_age_secs, age) != CleanupReason::Keep
            ==> spec_still_eligible(
            spec_reason(prune_enabled, path_prunable, max_age_secs, age),
            path_prunable,
            max_age_secs,
            age,
        ),
{
}

// ===========================================================================
// Probe classification (prune safety)
// ===========================================================================

/// The `io::ErrorKind` of a failed probe, in the classes prune reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoKindClass {
    /// `PermissionDenied`.
    PermissionDenied,
    /// `Unsupported`.
    Unsupported,
    /// A retryable kind (`Interrupted`, `WouldBlock`, `TimedOut`, connection
    /// errors).
    Transient,
    /// Any other kind.
    Other,
}

/// Evidence that a failed probe proves the path is not a directory. Each
/// flag is computed independently by the glue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsenceEvidence {
    /// The error kind is `NotFound`.
    pub not_found: bool,
    /// The OS reported a non-directory ancestor (`ENOTDIR`, `ERROR_DIRECTORY`).
    pub not_a_directory: bool,
    /// The path is invalid on this host (`InvalidInput`, `InvalidFilename`,
    /// `ENAMETOOLONG`, invalid-name codes).
    pub invalid_path: bool,
}

/// The facts that classify a failed probe that proves nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorFacts {
    /// The OS reported a symlink loop (`ELOOP`, `ERROR_CANT_RESOLVE_FILENAME`).
    pub symlink_loop: bool,
    /// The OS code is a retryable availability error (`ESTALE`, `EBUSY`, ...).
    pub transient_os: bool,
    /// The error kind's class.
    pub kind: IoKindClass,
}

/// The facts the glue extracts from one filesystem probe of a reserved path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The path resolves to a directory.
    Directory,
    /// The path resolves to something other than a directory.
    NonDirectory,
    /// The probe failed.
    Failed(AbsenceEvidence, ErrorFacts),
}

/// Why an uninspectable path's reservations were preserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PruneErrorKind {
    /// Access was denied.
    PermissionDenied,
    /// Symlink traversal looped.
    SymlinkLoop,
    /// A retryable or availability-related failure.
    Transient,
    /// The inspection is not supported.
    Unsupported,
    /// Anything else.
    Other,
}

/// The status of one reserved path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatusKind {
    /// The path is a directory: keep.
    ExistingDirectory,
    /// The path definitively does not exist: prunable.
    Missing,
    /// The path does not resolve to a directory: prunable.
    NotDirectory,
    /// The probe failed without proving absence: keep.
    Uninspectable(PruneErrorKind),
}

/// The status makes a path's reservations prunable.
pub open spec fn prunable(status: PathStatusKind) -> bool {
    status is Missing || status is NotDirectory
}

/// Whether a path status makes its reservations prunable (`prunable`).
pub const fn status_prunable(status: PathStatusKind) -> (r: bool)
    ensures
        r == prunable(status),
{
    matches!(status, PathStatusKind::Missing | PathStatusKind::NotDirectory)
}

/// The probe proves the path is not a directory.
pub open spec fn definitive(outcome: ProbeOutcome) -> bool {
    match outcome {
        ProbeOutcome::Directory => false,
        ProbeOutcome::NonDirectory => true,
        ProbeOutcome::Failed(absence, _) => absence.not_found || absence.not_a_directory
            || absence.invalid_path,
    }
}

/// The reported kind of an error that proves nothing: a symlink loop first,
/// then a transient OS code, then the error kind's class.
pub open spec fn spec_error_kind(facts: ErrorFacts) -> PruneErrorKind {
    if facts.symlink_loop {
        PruneErrorKind::SymlinkLoop
    } else if facts.transient_os {
        PruneErrorKind::Transient
    } else {
        match facts.kind {
            IoKindClass::PermissionDenied => PruneErrorKind::PermissionDenied,
            IoKindClass::Unsupported => PruneErrorKind::Unsupported,
            IoKindClass::Transient => PruneErrorKind::Transient,
            IoKindClass::Other => PruneErrorKind::Other,
        }
    }
}

/// The status of a probed path: a directory is kept; a non-directory target
/// is `NotDirectory`; a `NotFound` error is `Missing`; otherwise a
/// non-directory ancestor or an invalid path is `NotDirectory`; any other
/// failure is `Uninspectable` with `spec_error_kind`.
pub open spec fn spec_classify(outcome: ProbeOutcome) -> PathStatusKind {
    match outcome {
        ProbeOutcome::Directory => PathStatusKind::ExistingDirectory,
        ProbeOutcome::NonDirectory => PathStatusKind::NotDirectory,
        ProbeOutcome::Failed(absence, facts) => if absence.not_found {
            PathStatusKind::Missing
        } else if absence.not_a_directory || absence.invalid_path {
            PathStatusKind::NotDirectory
        } else {
            PathStatusKind::Uninspectable(spec_error_kind(facts))
        },
    }
}

/// The reported kind of an error that proves nothing (`spec_error_kind`).
pub fn error_kind(facts: ErrorFacts) -> (kind: PruneErrorKind)
    ensures
        kind == spec_error_kind(facts),
{
    if facts.symlink_loop {
        PruneErrorKind::SymlinkLoop
    } else if facts.transient_os {
        PruneErrorKind::Transient
    } else {
        match facts.kind {
            IoKindClass::PermissionDenied => PruneErrorKind::PermissionDenied,
            IoKindClass::Unsupported => PruneErrorKind::Unsupported,
            IoKindClass::Transient => PruneErrorKind::Transient,
            IoKindClass::Other => PruneErrorKind::Other,
        }
    }
}

/// Classifies one probe (`spec_classify`).
///
/// - Prune safety: the path is prunable exactly when the probe is definitive
///   evidence (`definitive`). A directory, and every failure without such
///   evidence, is preserved whatever its other facts (fail closed).
/// - A failure without definitive evidence is `Uninspectable`, with the kind
///   `spec_error_kind` (a symlink loop wins over everything, a transient OS
///   code over the error kind).
pub fn classify_probe(outcome: ProbeOutcome) -> (status: PathStatusKind)
    ensures
        status == spec_classify(outcome),
        prunable(status) <==> definitive(outcome),
        match outcome {
            ProbeOutcome::Failed(_, facts) => !definitive(outcome) ==> status
                == PathStatusKind::Uninspectable(spec_error_kind(facts)),
            _ => true,
        },
        outcome is Directory ==> status is ExistingDirectory,
        status is Uninspectable ==> outcome is Failed,
{
    match outcome {
        ProbeOutcome::Directory => PathStatusKind::ExistingDirectory,
        ProbeOutcome::NonDirectory => PathStatusKind::NotDirectory,
        ProbeOutcome::Failed(absence, facts) => {
            if absence.not_found {
                PathStatusKind::Missing
            } else if absence.not_a_directory || absence.invalid_path {
                PathStatusKind::NotDirectory
            } else {
                PathStatusKind::Uninspectable(error_kind(facts))
            }
        },
    }
}

// ===========================================================================
// Probe once per distinct path
// ===========================================================================

/// `rep` is the first position whose id equals position `i`'s.
pub open spec fn is_first_with_same_id(ids: Seq<usize>, i: int, rep: int) -> bool {
    &&& 0 <= rep <= i
    &&& ids[rep] == ids[i]
    &&& forall|j: int| 0 <= j < rep ==> ids[j] != ids[i]
}

/// Position `i` is the first occurrence of its id.
pub open spec fn first_occurrence(ids: Seq<usize>, i: int) -> bool {
    forall|j: int| 0 <= j < i ==> ids[j] != ids[i]
}

/// For each position, the first position with the same id (the path's one
/// probe). The glue gives each distinct path one id, so a path is probed at
/// its first reservation only (`lemma_representatives`), and every
/// reservation on it shares that one observation. Total: ids need not be
/// dense (an id of at least `ids.len()` is found by a linear scan; the glue's
/// ids are below `ids.len()`, found in constant time).
pub fn representatives(ids: &[usize]) -> (reps: Vec<usize>)
    ensures
        reps@.len() == ids@.len(),
        forall|i: int|
            0 <= i < ids@.len() ==> is_first_with_same_id(ids@, i, #[trigger] reps@[i] as int),
{
    let n = ids.len();
    let mut first: Vec<Option<usize>> = Vec::new();
    while first.len() < n
        invariant
            first@.len() <= n,
            forall|k: int| 0 <= k < first@.len() ==> #[trigger] first@[k] is None,
        decreases n - first@.len(),
    {
        first.push(None);
    }
    let mut reps: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < n
        invariant
            n == ids@.len(),
            first@.len() == n,
            i <= n,
            reps@.len() == i,
            forall|x: int| 0 <= x < i ==> is_first_with_same_id(ids@, x, #[trigger] reps@[x] as int),
            forall|k: int|
                0 <= k < n ==> match #[trigger] first@[k] {
                    Some(j) => j < i && ids@[j as int] == k && forall|l: int|
                        0 <= l < j ==> ids@[l] != k,
                    None => forall|l: int| 0 <= l < i ==> ids@[l] != k,
                },
        decreases n - i,
    {
        let k = ids[i];
        if k < n {
            if let Some(j) = first[k] {
                reps.push(j);
            } else {
                first[k] = Some(i);
                reps.push(i);
            }
        } else {
            let mut j: usize = 0;
            while ids[j] != k
                invariant
                    j <= i < n == ids@.len(),
                    ids@[i as int] == k,
                    forall|l: int| 0 <= l < j ==> ids@[l] != k,
                decreases i - j,
            {
                j += 1;
            }
            reps.push(j);
        }
        i += 1;
    }
    reps
}

/// The meaning of `representatives`: position `i` is probed (it is its own
/// representative) exactly when it is the first occurrence of its id, and
/// two positions share a representative exactly when they share an id. So
/// each distinct id is probed exactly once.
pub proof fn lemma_representatives(ids: Seq<usize>, reps: Seq<usize>, i: int, j: int)
    requires
        reps.len() == ids.len(),
        forall|x: int| 0 <= x < ids.len() ==> is_first_with_same_id(ids, x, #[trigger] reps[x] as int),
        0 <= i < ids.len(),
        0 <= j < ids.len(),
    ensures
        reps[i] == i <==> first_occurrence(ids, i),
        reps[i] == reps[j] <==> ids[i] == ids[j],
        reps[reps[i] as int] == reps[i],
{
    let ri = reps[i] as int;
    let rj = reps[j] as int;
    assert(is_first_with_same_id(ids, i, ri));
    assert(is_first_with_same_id(ids, j, rj));
    assert(is_first_with_same_id(ids, ri, reps[ri] as int));
    if ids[i] == ids[j] {
        if ri < rj {
            assert(ids[ri] != ids[j]);
        } else if rj < ri {
            assert(ids[rj] != ids[i]);
        }
    }
    if reps[ri] as int != ri {
        assert(ids[reps[ri] as int] == ids[ri]);
    }
}

// ===========================================================================
// The removal plan
// ===========================================================================

/// One scanned reservation, as the removal order sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Its reason.
    pub reason: CleanupReason,
    /// Its last use, in whole seconds since the Unix epoch.
    pub last_used_secs: u64,
}

/// The number of removal candidates (reasons other than `Keep`).
pub open spec fn removal_count(cands: Seq<Candidate>) -> nat
    decreases cands.len(),
{
    if cands.len() == 0 {
        0
    } else {
        removal_count(cands.drop_last()) + if cands.last().reason != CleanupReason::Keep {
            1nat
        } else {
            0nat
        }
    }
}

/// The removal count of a one-longer prefix.
pub proof fn lemma_removal_count_step(cands: Seq<Candidate>, i: int)
    requires
        0 <= i < cands.len(),
    ensures
        removal_count(cands.take(i + 1)) == removal_count(cands.take(i)) + if cands[i].reason
            != CleanupReason::Keep {
            1nat
        } else {
            0nat
        },
{
    assert(cands.take(i + 1).drop_last() =~= cands.take(i));
}

/// `s` is strictly increasing.
pub open spec fn increasing(s: Seq<usize>) -> bool {
    forall|a: int, b: int| 0 <= a < b < s.len() ==> #[trigger] s[a] < #[trigger] s[b]
}

/// `keep` lists exactly the `Keep` positions of `cands`, ascending.
pub open spec fn is_keep_list(cands: Seq<Candidate>, keep: Seq<usize>) -> bool {
    &&& increasing(keep)
    &&& forall|a: int|
        0 <= a < keep.len() ==> (#[trigger] keep[a] as int) < cands.len() && cands[keep[a] as int].reason
            == CleanupReason::Keep
    &&& forall|k: int|
        0 <= k < cands.len() && #[trigger] cands[k].reason == CleanupReason::Keep
            ==> super::render::occurs(keep, k)
}

/// How a cleanup scan splits.
pub struct CleanupPlan {
    /// The preserved positions, in scan order.
    pub keep: Vec<usize>,
    /// The removal candidates, in removal order (`super::sort`).
    pub removals: Vec<usize>,
}

/// Splits a scan into the kept positions (ascending) and the removal
/// candidates in removal order. Every position is in exactly one of the two
/// lists, so `keep.len() + removals.len() == cands.len()`: the considered
/// rows are exactly the preserved ones plus the candidates.
pub fn plan_removals(cands: &[Candidate]) -> (plan: CleanupPlan)
    ensures
        is_keep_list(cands@, plan.keep@),
        super::sort::is_removal_order(cands@, plan.removals@),
        plan.keep@.len() + plan.removals@.len() == cands@.len(),
{
    let n = cands.len();
    let mut keep: Vec<usize> = Vec::new();
    let ghost mut inv: Seq<int> = Seq::empty();
    let mut i: usize = 0;
    while i < n
        invariant
            n == cands@.len(),
            i <= n,
            increasing(keep@),
            forall|a: int|
                0 <= a < keep@.len() ==> (#[trigger] keep@[a] as int) < i && cands@[keep@[a] as int].reason
                    == CleanupReason::Keep,
            inv.len() == i,
            forall|k: int|
                0 <= k < i && cands@[k].reason == CleanupReason::Keep ==> 0 <= #[trigger] inv[k]
                    < keep@.len() && keep@[inv[k]] as int == k,
            keep@.len() + removal_count(cands@.take(i as int)) == i,
        decreases n - i,
    {
        proof {
            lemma_removal_count_step(cands@, i as int);
        }
        proof {
            inv = inv.push(keep@.len() as int);
        }
        if matches!(cands[i].reason, CleanupReason::Keep) {
            keep.push(i);
        }
        i += 1;
    }
    proof {
        assert(cands@.take(n as int) =~= cands@);
        assert forall|k: int|
            0 <= k < n && #[trigger] cands@[k].reason == CleanupReason::Keep implies super::render::occurs(
            keep@,
            k,
        ) by {
            assert(keep@[inv[k]] as int == k);
        }
    }
    let removals = super::sort::order_removals(cands);
    CleanupPlan { keep, removals }
}

// ===========================================================================
// Revalidate, then delete
// ===========================================================================

/// A fresh read of a planned candidate's row.
pub enum Observation<R> {
    /// The row is gone (a database trigger of an earlier delete removed it).
    Vanished,
    /// The row is exactly the planned snapshot.
    Unchanged,
    /// The row changed since planning; the current row.
    Changed(R),
}

/// The answer of a guarded delete (delete only if unchanged), with a re-read
/// when it deleted nothing.
pub enum Deletion<R> {
    /// The guarded delete removed the row.
    Deleted,
    /// It deleted nothing, and the row is gone.
    Vanished,
    /// It deleted nothing, and the row is present: the current row.
    StillPresent(R),
}

/// What happened to one planned candidate, with its evidence.
pub enum Fate<R> {
    /// Dry run: selected, nothing read or deleted.
    Selected,
    /// Gone when re-read: reported removed.
    Vanished,
    /// Changed since planning: preserved, with its current row.
    Changed(R),
    /// Unchanged but no longer eligible: preserved as planned.
    Ineligible,
    /// Removed by the guarded delete.
    Deleted,
    /// The guarded delete matched nothing and the row is gone: reported
    /// removed.
    VanishedAtDelete,
    /// The guarded delete matched nothing and a row is present: preserved,
    /// with that row.
    Replaced(R),
}

/// The fate counts as a removal (otherwise the candidate is preserved).
pub open spec fn removes<R>(fate: Fate<R>) -> bool {
    fate is Selected || fate is Vanished || fate is Deleted || fate is VanishedAtDelete
}

/// The oracle answered `Unchanged` for candidate `i`.
pub open spec fn observed_unchanged<R, E, O: Fn(usize) -> Result<Observation<R>, E>>(
    observe: O,
    i: usize,
) -> bool {
    observe.ensures((i,), Ok::<Observation<R>, E>(Observation::Unchanged))
}

/// A delete may be requested for candidate `i`: a live run, the candidate is
/// still eligible, and it was just observed unchanged.
pub open spec fn delete_permitted<R, E, O: Fn(usize) -> Result<Observation<R>, E>>(
    eligible: Seq<bool>,
    observe: O,
    dry_run: bool,
    i: usize,
) -> bool {
    !dry_run && eligible[i as int] && observed_unchanged::<R, E, O>(observe, i)
}

/// Candidate `i`'s fate is the one the oracles' answers determine:
///
/// - dry run: `Selected`;
/// - observed gone: `Vanished`; observed changed to `r`: `Changed(r)`;
/// - observed unchanged but ineligible: `Ineligible`;
/// - observed unchanged and eligible: the guarded delete's answer
///   (`Deleted`, `VanishedAtDelete`, or `Replaced(r)`).
pub open spec fn justified<
    R,
    E,
    O: Fn(usize) -> Result<Observation<R>, E>,
    D: Fn(usize) -> Result<Deletion<R>, E>,
>(eligible: Seq<bool>, observe: O, delete: D, dry_run: bool, i: usize, fate: Fate<R>) -> bool {
    match fate {
        Fate::Selected => dry_run,
        Fate::Vanished => !dry_run && observe.ensures(
            (i,),
            Ok::<Observation<R>, E>(Observation::Vanished),
        ),
        Fate::Changed(r) => !dry_run && observe.ensures(
            (i,),
            Ok::<Observation<R>, E>(Observation::Changed(r)),
        ),
        Fate::Ineligible => !dry_run && !eligible[i as int] && observed_unchanged::<R, E, O>(
            observe,
            i,
        ),
        Fate::Deleted => delete_permitted::<R, E, O>(eligible, observe, dry_run, i)
            && delete.ensures((i,), Ok::<Deletion<R>, E>(Deletion::Deleted)),
        Fate::VanishedAtDelete => delete_permitted::<R, E, O>(eligible, observe, dry_run, i)
            && delete.ensures((i,), Ok::<Deletion<R>, E>(Deletion::Vanished)),
        Fate::Replaced(r) => delete_permitted::<R, E, O>(eligible, observe, dry_run, i)
            && delete.ensures((i,), Ok::<Deletion<R>, E>(Deletion::StillPresent(r))),
    }
}

/// What a justified fate means for the accounting: a candidate counts as
/// removed only in a dry run, when it was observed gone, or when its guarded
/// delete (requested only after an unchanged, eligible read) removed it or
/// found it gone; a preserved candidate was observed changed or ineligible,
/// or survived its guarded delete, and is never preserved in a dry run.
pub proof fn lemma_fate_meaning<
    R,
    E,
    O: Fn(usize) -> Result<Observation<R>, E>,
    D: Fn(usize) -> Result<Deletion<R>, E>,
>(eligible: Seq<bool>, observe: O, delete: D, dry_run: bool, i: usize, fate: Fate<R>)
    requires
        justified(eligible, observe, delete, dry_run, i, fate),
    ensures
        removes(fate) ==> dry_run || observe.ensures(
            (i,),
            Ok::<Observation<R>, E>(Observation::Vanished),
        ) || (delete_permitted::<R, E, O>(eligible, observe, dry_run, i) && (delete.ensures(
            (i,),
            Ok::<Deletion<R>, E>(Deletion::Deleted),
        ) || delete.ensures((i,), Ok::<Deletion<R>, E>(Deletion::Vanished)))),
        !removes(fate) ==> !dry_run,
        fate is Deleted || fate is VanishedAtDelete || fate is Replaced ==> delete_permitted::<
            R,
            E,
            O,
        >(eligible, observe, dry_run, i),
{
}

/// The oracle error that stopped a revalidation, at candidate `index`.
pub struct RevalFailure<E> {
    /// The candidate being revalidated.
    pub index: usize,
    /// The oracle's error.
    pub error: E,
}

/// Revalidates and deletes the planned candidates `0..eligible.len()` in
/// order (`eligible[i]`: candidate `i`'s reason still holds, see
/// `still_eligible`).
///
/// - Call discipline (the `requires`, which ordinary closures always meet):
///   a dry run consults neither oracle, and `delete(i)` is only requested
///   for a live run's candidate `i` that is still eligible and was just
///   observed `Unchanged` — deletion acts only on fresh, still-eligible
///   evidence.
/// - On success every candidate has exactly one fate, the one the oracles'
///   answers determine (`justified`); so the removed and preserved
///   candidates partition the plan, and a candidate counts as removed only
///   when it was observed gone or the guarded delete removed it or found it
///   gone (or in a dry run).
/// - A failure is the first oracle error, from candidate `index`'s read or
///   (after an unchanged, eligible read) its delete.
pub fn revalidate_all<
    R,
    E,
    O: Fn(usize) -> Result<Observation<R>, E>,
    D: Fn(usize) -> Result<Deletion<R>, E>,
>(eligible: &[bool], observe: &O, delete: &D, dry_run: bool) -> (r: Result<
    Vec<Fate<R>>,
    RevalFailure<E>,
>)
    requires
        !dry_run ==> forall|i: usize| i < eligible@.len() ==> #[trigger] observe.requires((i,)),
        forall|i: usize|
            i < eligible@.len() && delete_permitted::<R, E, O>(eligible@, *observe, dry_run, i)
                ==> #[trigger] delete.requires((i,)),
    ensures
        r matches Ok(fates) ==> fates@.len() == eligible@.len() && forall|i: int|
            0 <= i < eligible@.len() ==> justified(
                eligible@,
                *observe,
                *delete,
                dry_run,
                i as usize,
                #[trigger] fates@[i],
            ),
        dry_run ==> (r matches Ok(fates) && forall|i: int|
            0 <= i < fates@.len() ==> #[trigger] fates@[i] is Selected),
        r matches Err(f) ==> !dry_run && f.index < eligible@.len() && (observe.ensures(
            (f.index,),
            Err::<Observation<R>, E>(f.error),
        ) || (delete_permitted::<R, E, O>(eligible@, *observe, dry_run, f.index)
            && delete.ensures((f.index,), Err::<Deletion<R>, E>(f.error)))),
{
    let n = eligible.len();
    let mut fates: Vec<Fate<R>> = Vec::new();
    if dry_run {
        let mut i: usize = 0;
        while i < n
            invariant
                n == eligible@.len(),
                i <= n,
                dry_run,
                fates@.len() == i,
                forall|x: int| 0 <= x < i ==> #[trigger] fates@[x] is Selected,
            decreases n - i,
        {
            fates.push(Fate::Selected);
            i += 1;
        }
        return Ok(fates);
    }
    let mut i: usize = 0;
    while i < n
        invariant
            n == eligible@.len(),
            i <= n,
            !dry_run,
            forall|x: usize| x < n ==> #[trigger] observe.requires((x,)),
            forall|x: usize|
                x < n && delete_permitted::<R, E, O>(eligible@, *observe, dry_run, x)
                    ==> #[trigger] delete.requires((x,)),
            fates@.len() == i,
            forall|x: int|
                0 <= x < i ==> justified(
                    eligible@,
                    *observe,
                    *delete,
                    dry_run,
                    x as usize,
                    #[trigger] fates@[x],
                ),
        decreases n - i,
    {
        let fate = match observe(i) {
            Err(error) => {
                return Err(RevalFailure { index: i, error });
            },
            Ok(Observation::Vanished) => Fate::Vanished,
            Ok(Observation::Changed(current)) => Fate::Changed(current),
            Ok(Observation::Unchanged) => {
                if eligible[i] {
                    proof {
                        assert(delete_permitted::<R, E, O>(eligible@, *observe, dry_run, i));
                    }
                    match delete(i) {
                        Err(error) => {
                            return Err(RevalFailure { index: i, error });
                        },
                        Ok(Deletion::Deleted) => Fate::Deleted,
                        Ok(Deletion::Vanished) => Fate::VanishedAtDelete,
                        Ok(Deletion::StillPresent(current)) => Fate::Replaced(current),
                    }
                } else {
                    Fate::Ineligible
                }
            },
        };
        fates.push(fate);
        i += 1;
    }
    Ok(fates)
}

// ===========================================================================
// The autoclean partition
// ===========================================================================

/// `list` holds exactly the positions of `reasons` with reason `reason`, in
/// ascending order.
pub open spec fn lists_reason(reasons: Seq<CleanupReason>, reason: CleanupReason, list: Seq<usize>) -> bool {
    &&& increasing(list)
    &&& forall|a: int|
        0 <= a < list.len() ==> (#[trigger] list[a] as int) < reasons.len() && reasons[list[a] as int]
            == reason
    &&& forall|k: int|
        0 <= k < reasons.len() && #[trigger] reasons[k] == reason ==> super::render::occurs(list, k)
}

/// The number of positions of `reasons` that are not `Keep`.
pub open spec fn non_keep_count(reasons: Seq<CleanupReason>) -> nat
    decreases reasons.len(),
{
    if reasons.len() == 0 {
        0
    } else {
        non_keep_count(reasons.drop_last()) + if reasons.last() != CleanupReason::Keep {
            1nat
        } else {
            0nat
        }
    }
}

/// Splits removals by reason into the pruned and the expired positions, each
/// ascending (so each keeps the removal order). The two lists are disjoint
/// (different reasons) and together hold every non-`Keep` position once, so
/// `pruned.len() + expired.len()` is the number of removals.
pub fn split_by_reason(reasons: &[CleanupReason]) -> (r: (Vec<usize>, Vec<usize>))
    ensures
        lists_reason(reasons@, CleanupReason::Prune, r.0@),
        lists_reason(reasons@, CleanupReason::Expire, r.1@),
        r.0@.len() + r.1@.len() == non_keep_count(reasons@),
{
    let n = reasons.len();
    let mut pruned: Vec<usize> = Vec::new();
    let mut expired: Vec<usize> = Vec::new();
    let ghost mut pinv: Seq<int> = Seq::empty();
    let ghost mut einv: Seq<int> = Seq::empty();
    let mut i: usize = 0;
    while i < n
        invariant
            n == reasons@.len(),
            i <= n,
            increasing(pruned@),
            increasing(expired@),
            forall|a: int|
                0 <= a < pruned@.len() ==> (#[trigger] pruned@[a] as int) < i && reasons@[pruned@[a] as int]
                    == CleanupReason::Prune,
            forall|a: int|
                0 <= a < expired@.len() ==> (#[trigger] expired@[a] as int) < i
                    && reasons@[expired@[a] as int] == CleanupReason::Expire,
            pinv.len() == i,
            einv.len() == i,
            forall|k: int|
                0 <= k < i && reasons@[k] == CleanupReason::Prune ==> 0 <= #[trigger] pinv[k]
                    < pruned@.len() && pruned@[pinv[k]] as int == k,
            forall|k: int|
                0 <= k < i && reasons@[k] == CleanupReason::Expire ==> 0 <= #[trigger] einv[k]
                    < expired@.len() && expired@[einv[k]] as int == k,
            pruned@.len() + expired@.len() == non_keep_count(reasons@.take(i as int)),
        decreases n - i,
    {
        proof {
            assert(reasons@.take(i + 1).drop_last() =~= reasons@.take(i as int));
        }
        proof {
            pinv = pinv.push(pruned@.len() as int);
            einv = einv.push(expired@.len() as int);
        }
        match reasons[i] {
            CleanupReason::Prune => pruned.push(i),
            CleanupReason::Expire => expired.push(i),
            CleanupReason::Keep => {},
        }
        i += 1;
    }
    proof {
        assert(reasons@.take(n as int) =~= reasons@);
        assert forall|k: int|
            0 <= k < n && #[trigger] reasons@[k] == CleanupReason::Prune implies super::render::occurs(
            pruned@,
            k,
        ) by {
            assert(pruned@[pinv[k]] as int == k);
        }
        assert forall|k: int|
            0 <= k < n && #[trigger] reasons@[k] == CleanupReason::Expire implies super::render::occurs(
            expired@,
            k,
        ) by {
            assert(expired@[einv[k]] as int == k);
        }
    }
    (pruned, expired)
}

} // verus!
