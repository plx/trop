//! Verified kernel: sticky project/task metadata reconciliation.
//!
//! Project and task are "sticky": once stored on a reservation, a request may
//! only change them with permission (`--force` or the field's own
//! `--allow-*-change` flag). Omitting a value preserves the stored one, and
//! clearing is never implicit.
//!
//! Values are compared as the UTF-8 bytes of the (already trimmed) strings;
//! byte equality of UTF-8 encodings coincides with `String` equality.
//!
//! Production glue: `crate::port::group` (group refresh and forced
//! replacement). `can_change_field` states the single-reservation rule of
//! `crate::operations::reserve` so both paths can share one definition.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Byte strings
// ---------------------------------------------------------------------------

/// The contents of an optional byte string.
pub open spec fn opt_bytes(value: Option<&[u8]>) -> Option<Seq<u8>> {
    match value {
        Some(bytes) => Some(bytes@),
        None => None,
    }
}

/// The contents of an optional owned byte string.
pub open spec fn opt_vec_bytes(value: Option<Vec<u8>>) -> Option<Seq<u8>> {
    match value {
        Some(bytes) => Some(bytes@),
        None => None,
    }
}

/// Byte-wise equality.
pub fn bytes_eq(a: &[u8], b: &[u8]) -> (equal: bool)
    ensures
        equal == (a@ == b@),
{
    if a.len() != b.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < a.len()
        invariant
            a@.len() == b@.len(),
            0 <= i <= a@.len(),
            forall|j: int| 0 <= j < i ==> a@[j] == b@[j],
        decreases a@.len() - i,
    {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    assert(a@ =~= b@);
    true
}

/// Equality of optional byte strings (`None` equals only `None`).
pub fn opt_bytes_eq(a: Option<&[u8]>, b: Option<&[u8]>) -> (equal: bool)
    ensures
        equal == (opt_bytes(a) == opt_bytes(b)),
{
    match (a, b) {
        (Some(x), Some(y)) => bytes_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// Borrows an optional owned byte string.
pub fn as_opt_slice(value: Option<&Vec<u8>>) -> (r: Option<&[u8]>)
    ensures
        opt_bytes(r) == match value {
            Some(bytes) => Some(bytes@),
            None => None::<Seq<u8>>,
        },
{
    match value {
        Some(bytes) => Some(bytes.as_slice()),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// The sticky-field rule
// ---------------------------------------------------------------------------

/// A sticky field may move from `existing` to `new` exactly when `force` or
/// the field's own permission is given, or when the value does not change.
pub open spec fn change_permitted(
    new: Option<Seq<u8>>,
    existing: Option<Seq<u8>>,
    force: bool,
    allow: bool,
) -> bool {
    force || allow || new == existing
}

/// The sticky-field rule of a single reservation: whether the stored value
/// `existing` may become `new`.
pub fn can_change_field(new: Option<&[u8]>, existing: Option<&[u8]>, force: bool, allow: bool) -> (r:
    bool)
    ensures
        r == change_permitted(opt_bytes(new), opt_bytes(existing), force, allow),
        !force && !allow ==> (r <==> opt_bytes(new) == opt_bytes(existing)),
{
    force || allow || opt_bytes_eq(new, existing)
}

/// How a refreshed reservation's sticky field is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyResolution {
    /// Keep the stored value (the request omitted the field).
    KeepExisting,
    /// Store the requested value.
    UseRequested,
}

/// The value a resolution produces.
pub open spec fn resolved_value(
    resolution: StickyResolution,
    requested: Option<Seq<u8>>,
    existing: Option<Seq<u8>>,
) -> Option<Seq<u8>> {
    match resolution {
        StickyResolution::KeepExisting => existing,
        StickyResolution::UseRequested => requested,
    }
}

/// The group refresh rule for one sticky field (`None` = change denied).
///
/// An omitted request keeps the stored value. A requested value is used when
/// the change is permitted by `change_permitted` (the same rule as a single
/// reservation's `can_change_field`).
pub open spec fn sticky_decision(
    requested: Option<Seq<u8>>,
    existing: Option<Seq<u8>>,
    force: bool,
    allow: bool,
) -> Option<StickyResolution> {
    if requested is None {
        Some(StickyResolution::KeepExisting)
    } else if change_permitted(requested, existing, force, allow) {
        Some(StickyResolution::UseRequested)
    } else {
        None
    }
}

/// Reconciles one sticky field of a stored reservation with a request.
///
/// Returns `None` when the request would change the stored value without
/// permission.
pub fn reconcile_sticky_field(
    requested: Option<&[u8]>,
    existing: Option<&[u8]>,
    force: bool,
    allow: bool,
) -> (r: Option<StickyResolution>)
    ensures
        r == sticky_decision(opt_bytes(requested), opt_bytes(existing), force, allow),
        // Omission preserves the stored value.
        requested is None ==> r == Some(StickyResolution::KeepExisting),
        // A requested value is accepted exactly when permitted, and then stored.
        requested is Some ==> (r is Some <==> (force || allow || opt_bytes(requested) == opt_bytes(
            existing,
        ))),
        requested is Some && r is Some ==> resolved_value(
            r->Some_0,
            opt_bytes(requested),
            opt_bytes(existing),
        ) == opt_bytes(requested),
        // Without permission the stored value never changes.
        r matches Some(res) ==> (!force && !allow ==> resolved_value(
            res,
            opt_bytes(requested),
            opt_bytes(existing),
        ) == opt_bytes(existing)),
        // Clearing is never implicit.
        r matches Some(res) ==> (resolved_value(res, opt_bytes(requested), opt_bytes(existing))
            is None ==> existing is None && requested is None),
        // Same rule as a single reservation: an explicit value passes exactly
        // when `can_change_field` would allow it, and an omitted value
        // resolves to the stored one, which `can_change_field` always allows.
        requested is Some ==> (r is Some <==> change_permitted(
            opt_bytes(requested),
            opt_bytes(existing),
            force,
            allow,
        )),
        r matches Some(res) ==> change_permitted(
            resolved_value(res, opt_bytes(requested), opt_bytes(existing)),
            opt_bytes(existing),
            force,
            allow,
        ),
{
    if requested.is_none() {
        Some(StickyResolution::KeepExisting)
    } else if can_change_field(requested, existing, force, allow) {
        Some(StickyResolution::UseRequested)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Group refresh: every stored row, project before task
// ---------------------------------------------------------------------------

/// The sticky metadata stored on one reservation of a group.
pub struct StoredMetadata {
    /// Stored project, if any.
    pub project: Option<Vec<u8>>,
    /// Stored task, if any.
    pub task: Option<Vec<u8>>,
}

/// Which sticky field a refresh would change without permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyField {
    /// The project field.
    Project,
    /// The task field.
    Task,
}

/// The first unauthorized sticky change of a group refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StickyViolation {
    /// Index of the offending stored row.
    pub row: usize,
    /// The field that would change.
    pub field: StickyField,
}

/// The project decision for stored row `row`.
pub open spec fn project_decision(
    requested: Option<Seq<u8>>,
    row: StoredMetadata,
    force: bool,
    allow_project: bool,
) -> Option<StickyResolution> {
    sticky_decision(requested, opt_vec_bytes(row.project), force, allow_project)
}

/// The task decision for stored row `row`.
pub open spec fn task_decision(
    requested: Option<Seq<u8>>,
    row: StoredMetadata,
    force: bool,
    allow_task: bool,
) -> Option<StickyResolution> {
    sticky_decision(requested, opt_vec_bytes(row.task), force, allow_task)
}

/// The request may refresh stored row `row`: neither field changes without
/// permission.
pub open spec fn row_permitted(
    requested_project: Option<Seq<u8>>,
    requested_task: Option<Seq<u8>>,
    row: StoredMetadata,
    force: bool,
    allow_project: bool,
    allow_task: bool,
) -> bool {
    &&& project_decision(requested_project, row, force, allow_project) is Some
    &&& task_decision(requested_task, row, force, allow_task) is Some
}

/// `resolution` is the (project, task) resolution of stored row `row`.
pub open spec fn row_resolution(
    resolution: (StickyResolution, StickyResolution),
    requested_project: Option<Seq<u8>>,
    requested_task: Option<Seq<u8>>,
    row: StoredMetadata,
    force: bool,
    allow_project: bool,
    allow_task: bool,
) -> bool {
    &&& Some(resolution.0) == project_decision(requested_project, row, force, allow_project)
    &&& Some(resolution.1) == task_decision(requested_task, row, force, allow_task)
}

/// Plans the sticky metadata of a group refresh before any write.
///
/// Every stored row is reconciled, project before task. Either every row
/// passes and the per-row resolutions are returned, or the first violation
/// (in row order, project before task) is reported and nothing is planned.
pub fn plan_group_metadata(
    requested_project: Option<&[u8]>,
    requested_task: Option<&[u8]>,
    rows: &[StoredMetadata],
    allow_project: bool,
    allow_task: bool,
    force: bool,
) -> (r: Result<Vec<(StickyResolution, StickyResolution)>, StickyViolation>)
    ensures
        r is Ok <==> forall|i: int|
            0 <= i < rows@.len() ==> #[trigger] row_permitted(
                opt_bytes(requested_project),
                opt_bytes(requested_task),
                rows@[i],
                force,
                allow_project,
                allow_task,
            ),
        r matches Ok(out) ==> out@.len() == rows@.len() && forall|i: int|
            0 <= i < rows@.len() ==> #[trigger] row_resolution(
                out@[i],
                opt_bytes(requested_project),
                opt_bytes(requested_task),
                rows@[i],
                force,
                allow_project,
                allow_task,
            ),
        r matches Err(v) ==> {
            &&& v.row < rows@.len()
            &&& forall|k: int|
                0 <= k < v.row ==> #[trigger] row_permitted(
                    opt_bytes(requested_project),
                    opt_bytes(requested_task),
                    rows@[k],
                    force,
                    allow_project,
                    allow_task,
                )
            &&& v.field == StickyField::Project ==> project_decision(
                opt_bytes(requested_project),
                rows@[v.row as int],
                force,
                allow_project,
            ) is None
            &&& v.field == StickyField::Task ==> project_decision(
                opt_bytes(requested_project),
                rows@[v.row as int],
                force,
                allow_project,
            ) is Some && task_decision(
                opt_bytes(requested_task),
                rows@[v.row as int],
                force,
                allow_task,
            ) is None
        },
{
    let ghost rp = opt_bytes(requested_project);
    let ghost rt = opt_bytes(requested_task);
    let mut out: Vec<(StickyResolution, StickyResolution)> = Vec::new();
    let mut i: usize = 0;
    while i < rows.len()
        invariant
            rp == opt_bytes(requested_project),
            rt == opt_bytes(requested_task),
            0 <= i <= rows@.len(),
            out@.len() == i,
            forall|k: int|
                0 <= k < i ==> #[trigger] row_resolution(
                    out@[k],
                    rp,
                    rt,
                    rows@[k],
                    force,
                    allow_project,
                    allow_task,
                ),
            forall|k: int|
                0 <= k < i ==> #[trigger] row_permitted(
                    rp,
                    rt,
                    rows@[k],
                    force,
                    allow_project,
                    allow_task,
                ),
        decreases rows@.len() - i,
    {
        let row = &rows[i];
        let Some(project) = reconcile_sticky_field(
            requested_project,
            as_opt_slice(row.project.as_ref()),
            force,
            allow_project,
        ) else {
            assert(!row_permitted(rp, rt, rows@[i as int], force, allow_project, allow_task));
            return Err(StickyViolation { row: i, field: StickyField::Project });
        };
        let Some(task) = reconcile_sticky_field(
            requested_task,
            as_opt_slice(row.task.as_ref()),
            force,
            allow_task,
        ) else {
            assert(!row_permitted(rp, rt, rows@[i as int], force, allow_project, allow_task));
            return Err(StickyViolation { row: i, field: StickyField::Task });
        };
        out.push((project, task));
        assert(row_resolution(
            out@[i as int],
            rp,
            rt,
            rows@[i as int],
            force,
            allow_project,
            allow_task,
        ));
        assert(row_permitted(rp, rt, rows@[i as int], force, allow_project, allow_task));
        i += 1;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Forced shape replacement: explicit or unanimous metadata
// ---------------------------------------------------------------------------

/// Where a forced replacement takes one sticky field from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplacementSource {
    /// The request's explicit value.
    Requested,
    /// The value every stored row agrees on (the first row's; none when
    /// there are no rows).
    InheritStored,
}

/// The value a forced replacement inherits from the stored rows.
pub open spec fn inherited_value(stored: Seq<Option<Vec<u8>>>) -> Option<Seq<u8>> {
    if stored.len() == 0 {
        None
    } else {
        opt_vec_bytes(stored[0])
    }
}

/// Chooses the sticky value of a forced group replacement.
///
/// An explicit request wins (force authorizes it). Otherwise the stored rows
/// must agree; `Err(k)` names the first row that disagrees with row 0. The
/// inherited value is never invented or mixed from several rows.
pub fn replacement_metadata(explicit: bool, stored: &[Option<Vec<u8>>]) -> (r: Result<
    ReplacementSource,
    usize,
>)
    ensures
        explicit ==> r == Ok::<ReplacementSource, usize>(ReplacementSource::Requested),
        !explicit ==> (r is Ok <==> forall|k: int|
            0 <= k < stored@.len() ==> opt_vec_bytes(#[trigger] stored@[k]) == inherited_value(
                stored@,
            )),
        !explicit && r is Ok ==> r == Ok::<ReplacementSource, usize>(
            ReplacementSource::InheritStored,
        ),
        r matches Err(k) ==> {
            &&& !explicit
            &&& 1 <= k < stored@.len()
            &&& opt_vec_bytes(stored@[k as int]) != opt_vec_bytes(stored@[0])
            &&& forall|j: int|
                1 <= j < k ==> opt_vec_bytes(#[trigger] stored@[j]) == opt_vec_bytes(stored@[0])
        },
{
    if explicit {
        return Ok(ReplacementSource::Requested);
    }
    if stored.is_empty() {
        return Ok(ReplacementSource::InheritStored);
    }
    let first = as_opt_slice(stored[0].as_ref());
    let mut k: usize = 1;
    while k < stored.len()
        invariant
            !explicit,
            1 <= k <= stored@.len(),
            opt_bytes(first) == opt_vec_bytes(stored@[0]),
            forall|j: int|
                0 <= j < k ==> opt_vec_bytes(#[trigger] stored@[j]) == opt_vec_bytes(stored@[0]),
        decreases stored@.len() - k,
    {
        if !opt_bytes_eq(as_opt_slice(stored[k].as_ref()), first) {
            return Err(k);
        }
        k += 1;
    }
    Ok(ReplacementSource::InheritStored)
}

} // verus!
