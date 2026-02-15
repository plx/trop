# P1-01: Sticky Field Overrides Do Not Persist Metadata Changes

## Problem

`trop reserve` validates sticky-field changes (`project`, `task`) but does not persist allowed changes for existing reservations.

Today, when a reservation already exists:

1. Sticky-field checks run.
2. If allowed (via `--allow-project-change`, `--allow-task-change`, `--allow-change`, or `--force`), planning still returns an idempotent `UpdateLastUsed` action.
3. No action updates `project`/`task` values in the database row.

This means override flags can suppress errors but still leave metadata unchanged.

## Evidence

- Existing-reservation path short-circuits to timestamp update:
  - `trop/src/operations/reserve.rs:307`
  - `trop/src/operations/reserve.rs:314`
  - `trop/src/operations/reserve.rs:315`
- Sticky validation only gates errors; it does not emit metadata update actions:
  - `trop/src/operations/reserve.rs:371`
  - `trop/src/operations/reserve.rs:397`

## Why This Matters

- Violates expected semantics of change-override flags.
- Creates operator confusion: command succeeds but state does not change.
- Makes metadata correction/migration impossible without delete/recreate workflows.

## Suggested Remediation

1. In `ReservePlan::build_plan`, detect field deltas when reservation exists:
   - `project_changed = requested_project != existing.project()`
   - `task_changed = requested_task != existing.task()`
2. Keep current sticky-field authorization checks.
3. If no deltas, keep current idempotent `UpdateLastUsed`.
4. If deltas are present and authorized, emit a row-rewrite action:
   - Preferred: add/emit `PlanAction::UpdateReservation(updated_reservation)`
   - Alternative: `DeleteReservation + CreateReservation` in same transaction.
5. Ensure transaction behavior remains all-or-nothing (already true in CLI reserve path).

## Acceptance Criteria

- Reserving with allowed sticky changes updates stored metadata.
- Reserving without allowed changes still returns sticky-field errors.
- Idempotent no-change reserve path still only updates `last_used_at`.
- New tests cover:
  - allow-project-change updates project
  - allow-task-change updates task
  - allow-change updates either field
  - force updates both fields

