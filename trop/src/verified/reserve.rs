//! Verified kernel: single-reservation decisions.
//!
//! - **Metadata intents** (`Intent`, `resolve`, `uses_fallback`): how a
//!   requested project/task operation (keep, set, clear) resolves against the
//!   stored value of an existing reservation, or against the inferred value
//!   of a new one, and when inference is needed at all.
//! - **The existing-reservation decision** (`decide_existing`): the sticky
//!   project/task rule (shared with the group path in `super::sticky`), then
//!   reallocation, metadata update, or a last-used refresh. Lemmas prove force
//!   dominance, that omitted metadata never changes stored values, that
//!   repeating a request is a no-op refresh (idempotence), that adding a
//!   permission never turns an accepted request into a rejection, and that
//!   the single and group paths accept, reject, and store exactly alike.
//! - **The reserve policy** (`reserve_policy`): the effective checks derived
//!   from the request flags; `--force` dominates every protection and the
//!   policy is monotone in the permissive flags.
//! - **Cleanup-then-retry** (`allocate_with_cleanup_retry`): generic over the
//!   allocation and cleanup oracles; at most one cleanup, only after an
//!   exhausted first allocation, and only when cleanup is possible.
//! - **Deferred reconciliation** (`deferred_step`): the execution-time replay
//!   of the existing-reservation decision for a plan that deferred allocation.
//! - **The reported port** (`reported_port`): the port a reserve plan reports,
//!   shared by live execution and dry runs.
//!
//! Strings are compared as the UTF-8 bytes of the (already trimmed) values;
//! byte equality of UTF-8 encodings coincides with `str` equality. Trimming
//! (`str::trim`) stays in the production glue.
//!
//! Production glue: `crate::operations::reserve` (`ReservePlan::build_plan`,
//! `reconcile_deferred_key`, `execute_reserve_after_cleanup_with_barrier`,
//! `MetadataIntent`), `crate::operations::executor` (reported port).

use vstd::prelude::*;

verus! {

use super::allocation::Fallback;
use super::sticky::{can_change_field, opt_bytes_eq};

// Spec-only items of sibling kernels are referred to by path
// (`super::sticky::change_permitted`): ordinary builds erase them. Every
// entry point is total (no `requires`), or states only its oracle closures'
// own preconditions.

// ---------------------------------------------------------------------------
// Metadata intents
// ---------------------------------------------------------------------------

/// The requested operation for one optional metadata field (project or task).
///
/// Production glue maps `MetadataIntent` one to one, trimming the value of
/// `Set` (`str::trim`) before it reaches the kernel.
pub enum Intent<T> {
    /// Keep the stored value of an existing reservation; use the inferred
    /// value for a new one.
    Preserve,
    /// Store this value.
    Set(T),
    /// Store no value.
    Clear,
}

/// The value an intent resolves to, given the fallback: the stored value of
/// an existing reservation, or the inferred value of a new one.
pub open spec fn resolved<T>(intent: Intent<T>, fallback: Option<T>) -> Option<T> {
    match intent {
        Intent::Preserve => fallback,
        Intent::Set(value) => Some(value),
        Intent::Clear => None,
    }
}

/// Resolves an intent against its fallback (see `resolved`).
pub fn resolve<T: Copy>(intent: &Intent<T>, fallback: Option<T>) -> (r: Option<T>)
    ensures
        r == resolved(*intent, fallback),
        *intent is Preserve ==> r == fallback,
        *intent matches Intent::Set(value) ==> r == Some(value),
        *intent is Clear ==> r is None,
{
    match intent {
        Intent::Preserve => fallback,
        Intent::Set(value) => Some(*value),
        Intent::Clear => None,
    }
}

/// Whether the fallback can affect the resolution: exactly for `Preserve`.
///
/// When it returns `false`, `lemma_explicit_intent_ignores_fallback` shows
/// that every fallback resolves alike, so the glue skips git inference.
pub fn uses_fallback<T>(intent: &Intent<T>) -> (b: bool)
    ensures
        b == (*intent is Preserve),
{
    matches!(intent, Intent::Preserve)
}

/// An explicit intent (`Set` or `Clear`) resolves the same whatever the
/// fallback: inference cannot influence it.
pub proof fn lemma_explicit_intent_ignores_fallback<T>(
    intent: Intent<T>,
    a: Option<T>,
    b: Option<T>,
)
    requires
        !(intent is Preserve),
    ensures
        resolved(intent, a) == resolved(intent, b),
{
}

/// Resolution is idempotent: resolving again against the result changes
/// nothing.
pub proof fn lemma_resolution_idempotent<T>(intent: Intent<T>, fallback: Option<T>)
    ensures
        resolved(intent, resolved(intent, fallback)) == resolved(intent, fallback),
{
}

/// The contents of an optional byte string (as in `super::sticky`).
pub open spec fn opt_bytes(value: Option<&[u8]>) -> Option<Seq<u8>> {
    super::sticky::opt_bytes(value)
}

/// The byte contents of an intent over borrowed bytes.
pub open spec fn intent_bytes(intent: Intent<&[u8]>) -> Intent<Seq<u8>> {
    match intent {
        Intent::Preserve => Intent::Preserve,
        Intent::Set(value) => Intent::Set(value@),
        Intent::Clear => Intent::Clear,
    }
}

/// Resolution commutes with taking byte contents.
pub proof fn lemma_resolved_bytes(intent: Intent<&[u8]>, fallback: Option<&[u8]>)
    ensures
        opt_bytes(resolved(intent, fallback)) == resolved(intent_bytes(intent), opt_bytes(fallback)),
{
}

// ---------------------------------------------------------------------------
// Request flags and the reserve policy
// ---------------------------------------------------------------------------

/// Per-field permissions to change sticky metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataPermissions {
    /// `--allow-project-change`.
    pub allow_project_change: bool,
    /// `--allow-task-change`.
    pub allow_task_change: bool,
}

/// Relaxations of the preferred-port checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocationRelaxations {
    /// Accept a preferred port the occupancy probe reports in use.
    pub ignore_occupied: bool,
    /// Accept a preferred port on the exclusion list.
    pub ignore_exclusions: bool,
}

/// Automatic-cleanup switches of a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupSwitches {
    /// `--disable-autoprune`.
    pub disable_autoprune: bool,
    /// `--disable-autoexpire`.
    pub disable_autoexpire: bool,
}

/// The flags of a single reserve request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveFlags {
    /// `--force`: overrides every protection.
    pub force: bool,
    /// `--overwrite`: reallocate an existing reservation.
    pub overwrite: bool,
    /// `--allow-unrelated-path`.
    pub allow_unrelated_path: bool,
    /// Sticky-metadata permissions.
    pub permissions: MetadataPermissions,
    /// Preferred-port relaxations.
    pub relaxations: AllocationRelaxations,
    /// Automatic-cleanup switches.
    pub cleanup: CleanupSwitches,
}

/// Which automatic cleanups may run when the range is exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupPolicy {
    /// Prune reservations whose paths no longer exist.
    pub prune: bool,
    /// Expire reservations unused for too long.
    pub expire: bool,
}

/// The effective checks of a reserve request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservePolicy {
    /// Reject paths unrelated to the working directory.
    pub check_path_relationship: bool,
    /// An existing reservation is reallocated rather than kept.
    pub reallocate_existing: bool,
    /// The effective preferred-port relaxations.
    pub allocation: AllocationRelaxations,
    /// The automatic cleanups available after exhaustion.
    pub cleanup: CleanupPolicy,
}

/// An existing reservation is reallocated: `--overwrite` or `--force`.
pub open spec fn reallocates(flags: ReserveFlags) -> bool {
    flags.overwrite || flags.force
}

/// A cleanup-and-retry is possible: some automatic cleanup is enabled.
pub open spec fn some_cleanup_enabled(policy: CleanupPolicy) -> bool {
    policy.prune || policy.expire
}

impl CleanupPolicy {
    /// Whether an exhausted allocation may be retried after automatic
    /// cleanup. Planning (defer or fail) and execution (clean up or fail)
    /// both ask this one question.
    pub const fn retry_possible(self) -> (b: bool)
        ensures
            b == some_cleanup_enabled(self),
    {
        self.prune || self.expire
    }
}

/// Derives the effective checks of a reserve request.
///
/// The specification's flag semantics: the path check applies unless
/// `--force` or `--allow-unrelated-path`; `--force` acts as `--overwrite`,
/// `--ignore-occupied`, and `--ignore-exclusions` together; each automatic
/// cleanup is on unless disabled. (`lemma_force_dominates` and
/// `lemma_policy_monotone` state the consequences.)
pub const fn reserve_policy(flags: ReserveFlags) -> (policy: ReservePolicy)
    ensures
        policy.check_path_relationship == (!flags.force && !flags.allow_unrelated_path),
        policy.reallocate_existing == reallocates(flags),
        policy.allocation.ignore_occupied == (flags.relaxations.ignore_occupied || flags.force),
        policy.allocation.ignore_exclusions == (flags.relaxations.ignore_exclusions || flags.force),
        policy.cleanup.prune == !flags.cleanup.disable_autoprune,
        policy.cleanup.expire == !flags.cleanup.disable_autoexpire,
{
    ReservePolicy {
        check_path_relationship: !flags.force && !flags.allow_unrelated_path,
        reallocate_existing: flags.overwrite || flags.force,
        allocation: AllocationRelaxations {
            ignore_occupied: flags.relaxations.ignore_occupied || flags.force,
            ignore_exclusions: flags.relaxations.ignore_exclusions || flags.force,
        },
        cleanup: CleanupPolicy {
            prune: !flags.cleanup.disable_autoprune,
            expire: !flags.cleanup.disable_autoexpire,
        },
    }
}

/// The policy `reserve_policy` returns (its postcondition as a value).
pub open spec fn policy_of(flags: ReserveFlags) -> ReservePolicy {
    ReservePolicy {
        check_path_relationship: !flags.force && !flags.allow_unrelated_path,
        reallocate_existing: reallocates(flags),
        allocation: AllocationRelaxations {
            ignore_occupied: flags.relaxations.ignore_occupied || flags.force,
            ignore_exclusions: flags.relaxations.ignore_exclusions || flags.force,
        },
        cleanup: CleanupPolicy {
            prune: !flags.cleanup.disable_autoprune,
            expire: !flags.cleanup.disable_autoexpire,
        },
    }
}

/// The requested preferred port: an explicit `--port` wins over a
/// configured preference.
pub fn choose_preferred<T>(explicit: Option<T>, hint: Option<T>) -> (r: Option<T>)
    ensures
        r == (if explicit is Some {
            explicit
        } else {
            hint
        }),
{
    match explicit {
        Some(value) => Some(value),
        None => hint,
    }
}

/// `b` permits everything `a` permits: every permissive flag of `a` is set
/// in `b`, and every cleanup `a` leaves enabled is enabled in `b`.
pub open spec fn at_least_as_permissive(b: ReserveFlags, a: ReserveFlags) -> bool {
    &&& a.force ==> b.force
    &&& a.overwrite ==> b.overwrite
    &&& a.allow_unrelated_path ==> b.allow_unrelated_path
    &&& a.permissions.allow_project_change ==> b.permissions.allow_project_change
    &&& a.permissions.allow_task_change ==> b.permissions.allow_task_change
    &&& a.relaxations.ignore_occupied ==> b.relaxations.ignore_occupied
    &&& a.relaxations.ignore_exclusions ==> b.relaxations.ignore_exclusions
    &&& !a.cleanup.disable_autoprune ==> !b.cleanup.disable_autoprune
    &&& !a.cleanup.disable_autoexpire ==> !b.cleanup.disable_autoexpire
}

/// Policy `q` is no stricter than policy `p`.
pub open spec fn no_stricter(q: ReservePolicy, p: ReservePolicy) -> bool {
    &&& q.check_path_relationship ==> p.check_path_relationship
    &&& p.reallocate_existing ==> q.reallocate_existing
    &&& p.allocation.ignore_occupied ==> q.allocation.ignore_occupied
    &&& p.allocation.ignore_exclusions ==> q.allocation.ignore_exclusions
    &&& p.cleanup.prune ==> q.cleanup.prune
    &&& p.cleanup.expire ==> q.cleanup.expire
}

/// `--force` bypasses the path check, reallocates, and relaxes both
/// preferred-port checks, whatever the other flags say.
pub proof fn lemma_force_dominates(flags: ReserveFlags)
    requires
        flags.force,
    ensures
        !policy_of(flags).check_path_relationship,
        policy_of(flags).reallocate_existing,
        policy_of(flags).allocation.ignore_occupied,
        policy_of(flags).allocation.ignore_exclusions,
{
}

/// Adding a permission never makes the policy stricter.
pub proof fn lemma_policy_monotone(a: ReserveFlags, b: ReserveFlags)
    requires
        at_least_as_permissive(b, a),
    ensures
        no_stricter(policy_of(b), policy_of(a)),
{
}

// ---------------------------------------------------------------------------
// The existing-reservation decision
// ---------------------------------------------------------------------------

/// What a reserve request does to the reservation already stored under its
/// key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistingDecision {
    /// Rejected: the project would change without permission.
    StickyProject,
    /// Rejected: the task would change without permission (the project
    /// check passed).
    StickyTask,
    /// Allocate a new port; store the resolved metadata, keep `created_at`.
    Reallocate,
    /// Keep the port; store the resolved metadata, keep `created_at`.
    UpdateMetadata,
    /// Nothing changes: refresh `last_used_at` and report the stored port.
    Touch,
}

/// The project may take its resolved value (the sticky rule of
/// `super::sticky`).
pub open spec fn project_permitted(
    stored_project: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    flags: ReserveFlags,
) -> bool {
    super::sticky::change_permitted(
        resolved(project, stored_project),
        stored_project,
        flags.force,
        flags.permissions.allow_project_change,
    )
}

/// The task may take its resolved value (the sticky rule of
/// `super::sticky`).
pub open spec fn task_permitted(
    stored_task: Option<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
) -> bool {
    super::sticky::change_permitted(
        resolved(task, stored_task),
        stored_task,
        flags.force,
        flags.permissions.allow_task_change,
    )
}

/// The resolved metadata differs from the stored metadata.
pub open spec fn metadata_changes(
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
) -> bool {
    resolved(project, stored_project) != stored_project || resolved(task, stored_task)
        != stored_task
}

/// `r` is the correct decision for a request (`project`, `task`, `flags`)
/// against a stored reservation (`stored_project`, `stored_task`).
///
/// - The project is checked first: `StickyProject` exactly when the
///   resolved project differs from the stored one without `--force` or
///   `--allow-project-change`.
/// - `StickyTask` exactly when the project passes and the task fails the
///   same rule with `--allow-task-change`.
/// - Otherwise `--overwrite` or `--force` reallocates.
/// - Otherwise the metadata is updated exactly when it changes, and the
///   reservation is merely refreshed (`Touch`) exactly when it does not.
///
/// These clauses determine `r` (`lemma_existing_decision_unique`).
pub open spec fn existing_decided(
    r: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
) -> bool {
    let project_ok = project_permitted(stored_project, project, flags);
    let task_ok = task_permitted(stored_task, task, flags);
    let changes = metadata_changes(stored_project, stored_task, project, task);
    &&& (r == ExistingDecision::StickyProject <==> !project_ok)
    &&& (r == ExistingDecision::StickyTask <==> project_ok && !task_ok)
    &&& (project_ok && task_ok && reallocates(flags) ==> r == ExistingDecision::Reallocate)
    &&& (project_ok && task_ok && !reallocates(flags) ==> (r == ExistingDecision::UpdateMetadata
        <==> changes) && (r == ExistingDecision::Touch <==> !changes))
}

/// The decision is a sticky-field rejection.
pub open spec fn is_sticky(r: ExistingDecision) -> bool {
    r == ExistingDecision::StickyProject || r == ExistingDecision::StickyTask
}

/// Decides what a reserve request does to the reservation stored under its
/// key (see `existing_decided`).
///
/// `stored_project`/`stored_task` are the stored values; `project`/`task`
/// are the request's intents (with `Set` values already trimmed). On a
/// non-sticky decision the reservation's new metadata is
/// `resolved(project, stored_project)` and `resolved(task, stored_task)`.
pub fn decide_existing(
    stored_project: Option<&[u8]>,
    stored_task: Option<&[u8]>,
    project: &Intent<&[u8]>,
    task: &Intent<&[u8]>,
    flags: ReserveFlags,
) -> (r: ExistingDecision)
    ensures
        existing_decided(
            r,
            opt_bytes(stored_project),
            opt_bytes(stored_task),
            intent_bytes(*project),
            intent_bytes(*task),
            flags,
        ),
{
    let resolved_project = resolve(project, stored_project);
    let resolved_task = resolve(task, stored_task);
    proof {
        lemma_resolved_bytes(*project, stored_project);
        lemma_resolved_bytes(*task, stored_task);
    }
    if !can_change_field(
        resolved_project,
        stored_project,
        flags.force,
        flags.permissions.allow_project_change,
    ) {
        return ExistingDecision::StickyProject;
    }
    if !can_change_field(
        resolved_task,
        stored_task,
        flags.force,
        flags.permissions.allow_task_change,
    ) {
        return ExistingDecision::StickyTask;
    }
    if flags.overwrite || flags.force {
        return ExistingDecision::Reallocate;
    }
    if opt_bytes_eq(resolved_project, stored_project) && opt_bytes_eq(resolved_task, stored_task) {
        ExistingDecision::Touch
    } else {
        ExistingDecision::UpdateMetadata
    }
}

/// The specification determines the decision.
pub proof fn lemma_existing_decision_unique(
    r1: ExistingDecision,
    r2: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        existing_decided(r1, stored_project, stored_task, project, task, flags),
        existing_decided(r2, stored_project, stored_task, project, task, flags),
    ensures
        r1 == r2,
{
}

/// `--force` reallocates, whatever the metadata and the other flags.
pub proof fn lemma_force_always_reallocates(
    r: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        flags.force,
        existing_decided(r, stored_project, stored_task, project, task, flags),
    ensures
        r == ExistingDecision::Reallocate,
{
}

/// Omitted metadata is harmless: a request that names neither project nor
/// task is never rejected, never changes the stored metadata, and is a mere
/// refresh unless it reallocates.
pub proof fn lemma_omitted_metadata_preserves(
    r: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        existing_decided(
            r,
            stored_project,
            stored_task,
            Intent::Preserve,
            Intent::Preserve,
            flags,
        ),
    ensures
        !is_sticky(r),
        resolved(Intent::Preserve, stored_project) == stored_project,
        resolved(Intent::Preserve, stored_task) == stored_task,
        r == (if reallocates(flags) {
            ExistingDecision::Reallocate
        } else {
            ExistingDecision::Touch
        }),
{
}

/// Clearing is a change like any other: an explicit `Clear` of a stored
/// project (or, once the project passes, task) without permission is
/// rejected, naming that field.
pub proof fn lemma_clear_is_protected(
    r: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        existing_decided(r, stored_project, stored_task, project, task, flags),
        !flags.force,
    ensures
        project is Clear && stored_project is Some && !flags.permissions.allow_project_change
            ==> r == ExistingDecision::StickyProject,
        project is Preserve && task is Clear && stored_task is Some
            && !flags.permissions.allow_task_change ==> r == ExistingDecision::StickyTask,
{
}

/// Without permission (`--force` or the field's `--allow-*-change`), a
/// stored value never changes: every accepted request resolves each field
/// to its stored value.
pub proof fn lemma_sticky_without_permission(
    r: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        existing_decided(r, stored_project, stored_task, project, task, flags),
        !is_sticky(r),
        !flags.force,
    ensures
        !flags.permissions.allow_project_change ==> resolved(project, stored_project)
            == stored_project,
        !flags.permissions.allow_task_change ==> resolved(task, stored_task) == stored_task,
        !flags.permissions.allow_project_change && !flags.permissions.allow_task_change
            && !flags.overwrite ==> r == ExistingDecision::Touch,
{
}

/// The value written back for one field: the resolved value passed through
/// the writer's normalization (`norm`; the reservation builder trims).
pub open spec fn written(norm: spec_fn(Seq<u8>) -> Seq<u8>, value: Option<Seq<u8>>) -> Option<
    Seq<u8>,
> {
    match value {
        Some(bytes) => Some(norm(bytes)),
        None => None,
    }
}

/// `norm` leaves an intent's explicit value unchanged (the glue trims `Set`
/// values, and trimming a trimmed string changes nothing).
pub open spec fn normalized_intent(norm: spec_fn(Seq<u8>) -> Seq<u8>, intent: Intent<Seq<u8>>) -> bool {
    intent matches Intent::Set(value) ==> norm(value) == value
}

/// Idempotence: once a request has been applied — the stored reservation
/// now holds its resolved metadata, as written by a normalizing writer —
/// repeating the request without `--overwrite`/`--force` is a mere refresh
/// (`Touch`), under any flags.
///
/// This covers both an accepted decision on an existing reservation
/// (`stored_*` are the values it held) and a fresh reservation (`stored_*`
/// are the inferred values `resolve` used). The only assumption is that the
/// explicit values are fixed points of the writer's normalization; for the
/// production writer (`str::trim`) that is trim idempotence, a trusted fact
/// about `std` pinned by tests.
pub proof fn lemma_repeat_request_touches(
    norm: spec_fn(Seq<u8>) -> Seq<u8>,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    again: ReserveFlags,
)
    requires
        normalized_intent(norm, project),
        normalized_intent(norm, task),
        !again.overwrite,
        !again.force,
    ensures
        existing_decided(
            ExistingDecision::Touch,
            written(norm, resolved(project, stored_project)),
            written(norm, resolved(task, stored_task)),
            project,
            task,
            again,
        ),
{
}

/// Idempotence with an exact writer (no normalization).
pub proof fn lemma_repeat_request_touches_exact(
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    again: ReserveFlags,
)
    requires
        !again.overwrite,
        !again.force,
    ensures
        existing_decided(
            ExistingDecision::Touch,
            resolved(project, stored_project),
            resolved(task, stored_task),
            project,
            task,
            again,
        ),
{
    let id = |s: Seq<u8>| s;
    lemma_repeat_request_touches(id, stored_project, stored_task, project, task, again);
    assert(written(id, resolved(project, stored_project)) == resolved(project, stored_project));
    assert(written(id, resolved(task, stored_task)) == resolved(task, stored_task));
}

/// Adding permissions never turns an accepted request into a rejection.
pub proof fn lemma_permissions_monotone(
    r1: ExistingDecision,
    r2: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    a: ReserveFlags,
    b: ReserveFlags,
)
    requires
        at_least_as_permissive(b, a),
        existing_decided(r1, stored_project, stored_task, project, task, a),
        existing_decided(r2, stored_project, stored_task, project, task, b),
        !is_sticky(r1),
    ensures
        !is_sticky(r2),
{
}

// ---------------------------------------------------------------------------
// Agreement with the group path (`super::sticky`)
// ---------------------------------------------------------------------------

/// The request a group refresh sees for an intent: an omitted field is
/// `None`, an explicit value is `Some`. (Group requests cannot clear.)
pub open spec fn group_request(intent: Intent<Seq<u8>>) -> Option<Seq<u8>> {
    match intent {
        Intent::Set(value) => Some(value),
        _ => None,
    }
}

/// One field: the group rule (`sticky_decision`) accepts exactly when the
/// single-reservation rule does, and both store the same value.
pub proof fn lemma_field_rule_matches_group(
    intent: Intent<Seq<u8>>,
    stored: Option<Seq<u8>>,
    force: bool,
    allow: bool,
)
    requires
        !(intent is Clear),
    ensures
        super::sticky::sticky_decision(group_request(intent), stored, force, allow) is Some
            <==> super::sticky::change_permitted(resolved(intent, stored), stored, force, allow),
        super::sticky::sticky_decision(group_request(intent), stored, force, allow) matches Some(
            res,
        ) ==> super::sticky::resolved_value(res, group_request(intent), stored) == resolved(
            intent,
            stored,
        ),
{
}

/// A whole stored row: `trop reserve` and `trop reserve-group` (via
/// `super::sticky::plan_group_metadata`) reject the same requests, report
/// the same field first (project before task), and store the same metadata.
pub proof fn lemma_decision_matches_group_row(
    r: ExistingDecision,
    row: super::sticky::StoredMetadata,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        !(project is Clear),
        !(task is Clear),
        existing_decided(
            r,
            super::sticky::opt_vec_bytes(row.project),
            super::sticky::opt_vec_bytes(row.task),
            project,
            task,
            flags,
        ),
    ensures
        (r == ExistingDecision::StickyProject) <==> super::sticky::project_decision(
            group_request(project),
            row,
            flags.force,
            flags.permissions.allow_project_change,
        ) is None,
        (r == ExistingDecision::StickyTask) <==> (super::sticky::project_decision(
            group_request(project),
            row,
            flags.force,
            flags.permissions.allow_project_change,
        ) is Some && super::sticky::task_decision(
            group_request(task),
            row,
            flags.force,
            flags.permissions.allow_task_change,
        ) is None),
        !is_sticky(r) <==> super::sticky::row_permitted(
            group_request(project),
            group_request(task),
            row,
            flags.force,
            flags.permissions.allow_project_change,
            flags.permissions.allow_task_change,
        ),
{
    lemma_field_rule_matches_group(
        project,
        super::sticky::opt_vec_bytes(row.project),
        flags.force,
        flags.permissions.allow_project_change,
    );
    lemma_field_rule_matches_group(
        task,
        super::sticky::opt_vec_bytes(row.task),
        flags.force,
        flags.permissions.allow_task_change,
    );
}

// ---------------------------------------------------------------------------
// Cleanup, then one retry
// ---------------------------------------------------------------------------

/// The result of an allocation with one cleanup-and-retry.
pub enum Retry<P, C> {
    /// A port was allocated. `cleanup` holds the cleanup's result when the
    /// first allocation was exhausted and the retry after cleanup succeeded.
    Allocated { port: P, cleanup: Option<C> },
    /// The range stayed exhausted. `cleanup` holds the cleanup's result when
    /// a cleanup ran before the (also exhausted) retry.
    Exhausted { cleanup: Option<C> },
}

/// `allocate(after_cleanup)` was observed to return `answer`.
pub open spec fn allocation_returned<P, E, A: Fn(bool) -> Result<Fallback<P>, E>>(
    allocate: A,
    after_cleanup: bool,
    answer: Result<Fallback<P>, E>,
) -> bool {
    allocate.ensures((after_cleanup,), answer)
}

/// The first allocation was exhausted and cleanup was possible, so the
/// cleanup ran and returned `counts`.
pub open spec fn cleaned_up<P, C, E, A: Fn(bool) -> Result<Fallback<P>, E>, K: FnOnce() -> Result<
    C,
    E,
>>(cleanup_possible: bool, allocate: A, cleanup: K, counts: C) -> bool {
    &&& cleanup_possible
    &&& allocation_returned(allocate, false, Ok(Fallback::Exhausted))
    &&& cleanup.ensures((), Ok::<C, E>(counts))
}

/// `r` is a correct result of one allocation with at most one
/// cleanup-and-retry:
///
/// - `Allocated { cleanup: None }`: the first allocation succeeded with that
///   port; no cleanup ran.
/// - `Allocated { cleanup: Some(c) }`: the first allocation was exhausted,
///   cleanup was possible and returned `c`, and the retry succeeded.
/// - `Exhausted { cleanup: None }`: the first allocation was exhausted and
///   cleanup was not possible (so it never ran).
/// - `Exhausted { cleanup: Some(c) }`: exhausted, cleaned up (`c`), and the
///   retry was exhausted too.
/// - an error is an oracle's error, unchanged: from the first allocation, or
///   — only after an exhausted first allocation with cleanup possible — from
///   the cleanup or the retry.
pub open spec fn retried<P, C, E, A: Fn(bool) -> Result<Fallback<P>, E>, K: FnOnce() -> Result<
    C,
    E,
>>(cleanup_possible: bool, allocate: A, cleanup: K, r: Result<Retry<P, C>, E>) -> bool {
    match r {
        Ok(Retry::Allocated { port, cleanup: None }) => allocation_returned(
            allocate,
            false,
            Ok(Fallback::Allocated(port)),
        ),
        Ok(Retry::Allocated { port, cleanup: Some(counts) }) => cleaned_up(
            cleanup_possible,
            allocate,
            cleanup,
            counts,
        ) && allocation_returned(allocate, true, Ok(Fallback::Allocated(port))),
        Ok(Retry::Exhausted { cleanup: None }) => !cleanup_possible && allocation_returned(
            allocate,
            false,
            Ok(Fallback::Exhausted),
        ),
        Ok(Retry::Exhausted { cleanup: Some(counts) }) => cleaned_up(
            cleanup_possible,
            allocate,
            cleanup,
            counts,
        ) && allocation_returned(allocate, true, Ok(Fallback::Exhausted)),
        Err(e) => allocation_returned(allocate, false, Err(e)) || (cleanup_possible
            && allocation_returned(allocate, false, Ok(Fallback::Exhausted)) && (cleanup.ensures(
            (),
            Err::<C, E>(e),
        ) || allocation_returned(allocate, true, Err(e)))),
    }
}

/// Allocates; when the range is exhausted and cleanup is possible, runs the
/// cleanup once and retries once (see `retried`).
///
/// The body is straight-line: `allocate(false)` runs first; `cleanup` (an
/// `FnOnce`, so at most once) runs only after it returned `Exhausted` with
/// `cleanup_possible`; `allocate(true)` runs only after the cleanup
/// succeeded. Generic over the port type, the cleanup's result, and the
/// error, so the group path can reuse it.
pub fn allocate_with_cleanup_retry<
    P,
    C,
    E,
    A: Fn(bool) -> Result<Fallback<P>, E>,
    K: FnOnce() -> Result<C, E>,
>(cleanup_possible: bool, allocate: &A, cleanup: K) -> (r: Result<Retry<P, C>, E>)
    requires
        allocate.requires((false,)),
        allocate.requires((true,)),
        cleanup.requires(()),
    ensures
        retried(cleanup_possible, *allocate, cleanup, r),
        !cleanup_possible ==> !(r matches Ok(Retry::Allocated { cleanup: Some(_), .. })),
        !cleanup_possible ==> !(r matches Ok(Retry::Exhausted { cleanup: Some(_) })),
{
    match allocate(false)? {
        Fallback::Allocated(port) => {
            return Ok(Retry::Allocated { port, cleanup: None });
        },
        Fallback::Exhausted => {},
    }
    if !cleanup_possible {
        return Ok(Retry::Exhausted { cleanup: None });
    }
    let counts = cleanup()?;
    match allocate(true)? {
        Fallback::Allocated(port) => Ok(Retry::Allocated { port, cleanup: Some(counts) }),
        Fallback::Exhausted => Ok(Retry::Exhausted { cleanup: Some(counts) }),
    }
}

// ---------------------------------------------------------------------------
// Deferred reconciliation
// ---------------------------------------------------------------------------

/// What a deferred reserve does at execution time, once the key has been
/// looked up again. `R` is the caller's handle on the stored reservation.
pub enum DeferredStep<R> {
    /// The key holds no reservation now: allocate a fresh one. Its metadata
    /// is re-inferred when a reservation existed at planning (the plan's
    /// placeholder carries that reservation's metadata), and otherwise taken
    /// from the placeholder (resolved at planning for a new key). Nothing is
    /// ignored and no `created_at` is kept.
    AllocateFresh {
        /// Re-infer the metadata instead of using the placeholder's.
        reinfer: bool,
    },
    /// The key holds the reservation `R` now: apply the existing-reservation
    /// decision, exactly as planning would.
    Existing(R, ExistingDecision),
}

/// A stored reservation, with the bytes of its project and task.
pub struct StoredRow<'a, R> {
    /// The caller's handle on the reservation.
    pub reservation: R,
    /// The stored project.
    pub project: Option<&'a [u8]>,
    /// The stored task.
    pub task: Option<&'a [u8]>,
}

/// Reconciles a deferred reserve with the key's current state (see
/// `DeferredStep`). `stored` is the key's current reservation, if any.
pub fn deferred_step<R>(
    stored: Option<StoredRow<'_, R>>,
    project: &Intent<&[u8]>,
    task: &Intent<&[u8]>,
    flags: ReserveFlags,
    existed_at_planning: bool,
) -> (step: DeferredStep<R>)
    ensures
        stored is None ==> step == (DeferredStep::<R>::AllocateFresh {
            reinfer: existed_at_planning,
        }),
        stored matches Some(row) ==> (step matches DeferredStep::Existing(handle, d) && handle
            == row.reservation && existing_decided(
            d,
            opt_bytes(row.project),
            opt_bytes(row.task),
            intent_bytes(*project),
            intent_bytes(*task),
            flags,
        )),
{
    match stored {
        None => DeferredStep::AllocateFresh { reinfer: existed_at_planning },
        Some(row) => {
            let decision = decide_existing(row.project, row.task, project, task, flags);
            DeferredStep::Existing(row.reservation, decision)
        },
    }
}

/// Planning and deferred execution agree on an existing key: a request that
/// reallocates replaces the row (ignoring its own key, keeping
/// `created_at`), and one that does not completes without allocating.
pub proof fn lemma_deferred_existing_agrees_with_planning(
    d: ExistingDecision,
    stored_project: Option<Seq<u8>>,
    stored_task: Option<Seq<u8>>,
    project: Intent<Seq<u8>>,
    task: Intent<Seq<u8>>,
    flags: ReserveFlags,
)
    requires
        existing_decided(d, stored_project, stored_task, project, task, flags),
        !is_sticky(d),
    ensures
        d == ExistingDecision::Reallocate <==> reallocates(flags),
        !reallocates(flags) ==> (d == ExistingDecision::UpdateMetadata || d
            == ExistingDecision::Touch),
{
}

// ---------------------------------------------------------------------------
// The reported port
// ---------------------------------------------------------------------------

/// How one plan action contributes to the port a plan reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportedPort<P> {
    /// A reservation written with this port at planning time.
    Static(P),
    /// A refresh of a reservation whose port, recorded at planning time, is
    /// this one.
    Touch(P),
    /// An allocation deferred to execution: the port it produced (live
    /// execution), or `None` (a dry run, or no port).
    Deferred(Option<P>),
    /// An action that reports no port (release, group allocation).
    NoPort,
}

/// The action carries a planning-time port.
pub open spec fn is_planned<P>(a: ReportedPort<P>) -> bool {
    a is Static || a is Touch
}

/// The planning-time port of an action that carries one.
pub open spec fn planned_port<P>(a: ReportedPort<P>) -> P
    recommends
        is_planned(a),
{
    match a {
        ReportedPort::Static(port) => port,
        ReportedPort::Touch(port) => port,
        _ => arbitrary(),
    }
}

/// The port of the last deferred action that produced one.
pub open spec fn last_deferred<P>(actions: Seq<ReportedPort<P>>) -> Option<P>
    decreases actions.len(),
{
    if actions.len() == 0 {
        None
    } else {
        match actions.last() {
            ReportedPort::Deferred(Some(port)) => Some(port),
            _ => last_deferred(actions.drop_last()),
        }
    }
}

/// The planning-time port of the first action that carries one.
pub open spec fn first_planned<P>(actions: Seq<ReportedPort<P>>) -> Option<P>
    decreases actions.len(),
{
    if actions.len() == 0 {
        None
    } else {
        match first_planned(actions.drop_last()) {
            Some(port) => Some(port),
            None => if is_planned(actions.last()) {
                Some(planned_port(actions.last()))
            } else {
                None
            },
        }
    }
}

/// The port a plan reports: a port produced at execution by a deferred
/// action wins (the last one); otherwise the planning-time port of the first
/// action that carries one, in plan order.
pub open spec fn reported<P>(actions: Seq<ReportedPort<P>>) -> Option<P> {
    match last_deferred(actions) {
        Some(port) => Some(port),
        None => first_planned(actions),
    }
}

/// Computes the port a plan reports (see `reported`). Live execution and
/// dry runs call this one function on the same plan.
pub fn reported_port<P: Copy>(actions: &[ReportedPort<P>]) -> (r: Option<P>)
    ensures
        r == reported(actions@),
{
    let mut deferred: Option<P> = None;
    let mut planned: Option<P> = None;
    let mut i: usize = 0;
    while i < actions.len()
        invariant
            0 <= i <= actions@.len(),
            deferred == last_deferred(actions@.take(i as int)),
            planned == first_planned(actions@.take(i as int)),
        decreases actions@.len() - i,
    {
        let ghost prefix = actions@.take(i as int);
        let ghost next = actions@.take(i as int + 1);
        assert(next.drop_last() =~= prefix);
        assert(next.last() == actions@[i as int]);
        let action = &actions[i];
        if let ReportedPort::Deferred(Some(port)) = action {
            deferred = Some(*port);
        }
        if planned.is_none() {
            planned = match action {
                ReportedPort::Static(port) | ReportedPort::Touch(port) => Some(*port),
                _ => None,
            };
        }
        i += 1;
    }
    assert(actions@.take(actions@.len() as int) =~= actions@);
    match deferred {
        Some(port) => Some(port),
        None => planned,
    }
}

/// `first_planned` is the first action that carries a planning-time port.
pub proof fn lemma_first_planned_is_first<P>(actions: Seq<ReportedPort<P>>, i: int)
    requires
        0 <= i < actions.len(),
        is_planned(actions[i]),
        forall|j: int| 0 <= j < i ==> !is_planned(#[trigger] actions[j]),
    ensures
        first_planned(actions) == Some(planned_port(actions[i])),
    decreases actions.len(),
{
    if i < actions.len() - 1 {
        lemma_first_planned_is_first(actions.drop_last(), i);
    } else {
        lemma_no_planned_prefix(actions.drop_last());
    }
}

/// No action carries a planning-time port: `first_planned` is `None`.
pub proof fn lemma_no_planned_prefix<P>(actions: Seq<ReportedPort<P>>)
    requires
        forall|j: int| 0 <= j < actions.len() ==> !is_planned(#[trigger] actions[j]),
    ensures
        first_planned(actions) is None,
    decreases actions.len(),
{
    if actions.len() > 0 {
        lemma_no_planned_prefix(actions.drop_last());
    }
}

/// `last_deferred` is the last deferred action that produced a port.
pub proof fn lemma_last_deferred_is_last<P>(actions: Seq<ReportedPort<P>>, i: int, port: P)
    requires
        0 <= i < actions.len(),
        actions[i] == ReportedPort::Deferred(Some(port)),
        forall|j: int|
            i < j < actions.len() ==> !(#[trigger] actions[j] matches ReportedPort::Deferred(
                Some(_),
            )),
    ensures
        last_deferred(actions) == Some(port),
    decreases actions.len(),
{
    if i < actions.len() - 1 {
        let shorter = actions.drop_last();
        assert forall|j: int| i < j < shorter.len() implies !(
        #[trigger] shorter[j] matches ReportedPort::Deferred(Some(_))) by {
            assert(shorter[j] == actions[j]);
        }
        lemma_last_deferred_is_last(shorter, i, port);
        assert(!(actions.last() matches ReportedPort::Deferred(Some(_))));
    }
}

/// No action is a deferred action that produced a port.
pub open spec fn no_deferred_port<P>(actions: Seq<ReportedPort<P>>) -> bool {
    forall|j: int| 0 <= j < actions.len() ==> !(#[trigger] actions[j] matches ReportedPort::Deferred(
        Some(_),
    ))
}

/// Without a port from a deferred action, `last_deferred` is `None`.
pub proof fn lemma_no_deferred_port<P>(actions: Seq<ReportedPort<P>>)
    requires
        no_deferred_port(actions),
    ensures
        last_deferred(actions) is None,
    decreases actions.len(),
{
    if actions.len() > 0 {
        lemma_no_deferred_port(actions.drop_last());
    }
}

/// A plan whose first action carries a planning-time port, and in which no
/// deferred action produced a port, reports that port. In particular every
/// reserve plan without a deferred allocation (one `Create`/`Update` or one
/// `Touch`) reports a port, in live execution and dry runs alike.
pub proof fn lemma_planned_first_action_is_reported<P>(actions: Seq<ReportedPort<P>>)
    requires
        actions.len() > 0,
        is_planned(actions[0]),
        no_deferred_port(actions),
    ensures
        reported(actions) == Some(planned_port(actions[0])),
{
    lemma_no_deferred_port(actions);
    lemma_first_planned_is_first(actions, 0);
}

/// The dry-run view of an action: a deferred action has not run, so it has
/// produced no port.
pub open spec fn dry_run_view<P>(a: ReportedPort<P>) -> ReportedPort<P> {
    match a {
        ReportedPort::Deferred(_) => ReportedPort::Deferred(None),
        other => other,
    }
}

/// Dry runs and live execution report the same port whenever no deferred
/// action produced a port at execution.
pub proof fn lemma_dry_run_agrees<P>(live: Seq<ReportedPort<P>>, dry: Seq<ReportedPort<P>>)
    requires
        live.len() == dry.len(),
        forall|j: int| 0 <= j < live.len() ==> dry[j] == dry_run_view(#[trigger] live[j]),
        no_deferred_port(live),
    ensures
        reported(dry) == reported(live),
    decreases live.len(),
{
    lemma_no_deferred_port(live);
    assert(no_deferred_port(dry)) by {
        assert forall|j: int| 0 <= j < dry.len() implies !(#[trigger] dry[j] matches ReportedPort::Deferred(
            Some(_),
        )) by {
            assert(dry[j] == dry_run_view(live[j]));
        }
    }
    lemma_no_deferred_port(dry);
    lemma_first_planned_agrees(live, dry);
}

/// `first_planned` sees through the dry-run view.
proof fn lemma_first_planned_agrees<P>(live: Seq<ReportedPort<P>>, dry: Seq<ReportedPort<P>>)
    requires
        live.len() == dry.len(),
        forall|j: int| 0 <= j < live.len() ==> dry[j] == dry_run_view(#[trigger] live[j]),
    ensures
        first_planned(dry) == first_planned(live),
    decreases live.len(),
{
    if live.len() > 0 {
        let l = live.drop_last();
        let d = dry.drop_last();
        assert forall|j: int| 0 <= j < l.len() implies d[j] == dry_run_view(#[trigger] l[j]) by {
            assert(d[j] == dry[j]);
            assert(l[j] == live[j]);
        }
        lemma_first_planned_agrees(l, d);
        assert(dry.last() == dry_run_view(live.last()));
    }
}

} // verus!
