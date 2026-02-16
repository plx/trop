# P2-02: Critical Scenarios Are Covered by Ignored Tests Only

## Problem

Several high-value tests are marked `#[ignore]`, so they do not run in normal `cargo test` or main CI test jobs. This leaves important behavior effectively unguarded.

## Evidence

- Ignored stress/performance tests:
  - `trop/tests/stress_testing.rs:63`
  - `trop/tests/stress_testing.rs:172`
  - `trop/tests/stress_testing.rs:285`
- Ignored CLI behavior tests:
  - `trop-cli/tests/release_command.rs:88`
  - `trop-cli/tests/phase_10_commands.rs:1061`
- CI runs default tests without `--ignored`:
  - `justfile:128`
  - `justfile:132`
  - `.github/workflows/ci.yml:167`

## Why This Matters

- Regressions in stress behavior, default-path release behavior, and global exclusion flow can land undetected.
- Test confidence appears higher than actual enforced coverage.

## Suggested Remediation

1. Reclassify tests by cost and criticality:
   - keep very heavy tests ignored but move a reduced, representative subset into regular CI.
2. For ignored CLI tests, fix fixture assumptions so they can run normally.
3. Add dedicated scheduled/manual workflow for full stress suite:
   - nightly or on-demand with explicit reporting.
4. Track ignored tests in CI summary so skipped critical tests are visible.

## Acceptance Criteria

- Critical command semantics are covered by non-ignored tests.
- Performance stress checks run at least on a scheduled or explicit workflow.
- CI documentation states which suites run per trigger.

