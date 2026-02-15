# P1-02: Group Reservation Creation Is Not Atomic

## Problem

Group reservation creation inserts rows one-by-one without wrapping inserts in a transaction at the allocation point. If any insert fails mid-loop, earlier inserts may persist, resulting in partial group allocation.

The inline comment says a caller-managed transaction exists, but this function currently only receives a `&Connection` and does not itself enforce transactional boundaries.

## Evidence

- Per-reservation insert loop:
  - `trop/src/port/group.rs:327`
  - `trop/src/port/group.rs:328`
- Comment indicating transaction assumption:
  - `trop/src/port/group.rs:325`
  - `trop/src/port/group.rs:326`

## Why This Matters

- Violates expected all-or-nothing semantics for `reserve-group`.
- Can leave orphaned partial allocations that break downstream automation.
- Makes recovery harder and increases user-facing inconsistency.

## Suggested Remediation

1. Move group row creation into an explicit transaction boundary:
   - Begin transaction before first insert.
   - Insert all reservations.
   - Commit only after all succeed.
2. Prefer existing batch primitive if available (`batch_create_reservations`) to reduce repeated insert logic.
3. If transaction management belongs at caller level, enforce it by API shape:
   - accept `&Transaction` instead of `&Connection`, or
   - provide a transactional wrapper and keep this function internal.
4. Add rollback-path tests (simulate conflict/constraint failure on Nth insert).

## Acceptance Criteria

- Any failure during group create leaves zero new group reservations.
- Success path still creates all service reservations.
- Tests verify both success and forced-midstream-failure behavior.

