# P3-01: Test Coverage Gaps for `completions` and Reserve Occupancy Controls

## Problem

Two coverage gaps remain in CLI integration tests:

1. `trop completions` behavior is not covered by integration tests.
2. `reserve` occupancy-control flags are not exercised in CLI tests (`--skip-occupancy-check`, `--skip-tcp`, `--skip-udp`, `--skip-ipv4`, `--skip-ipv6`, `--check-all-interfaces`).

## Evidence

- `completions` is a shipped command:
  - `trop-cli/src/cli.rs:112`
  - `trop-cli/src/cli.rs:113`
- `reserve` exposes occupancy flags:
  - `trop-cli/src/commands/reserve.rs:95`
  - `trop-cli/src/commands/reserve.rs:99`
  - `trop-cli/src/commands/reserve.rs:103`
  - `trop-cli/src/commands/reserve.rs:107`
  - `trop-cli/src/commands/reserve.rs:111`
  - `trop-cli/src/commands/reserve.rs:115`
- Test search showed no `completions` command coverage and no usage of reserve occupancy flags in CLI tests.

## Why This Matters

- Regressions in shell completion output/arguments may ship unnoticed.
- Flag parsing and behavior drift is more likely when never exercised end-to-end.

## Suggested Remediation

1. Add `trop-cli/tests/completions_command.rs`:
   - test each supported shell output is generated,
   - assert non-empty output and expected header fragments.
2. Add targeted reserve-flag tests:
   - parse acceptance for each occupancy flag,
   - behavioral assertions where practical (or explicit pending behavior if not yet wired).
3. Include at least one matrix-style test for combined flags.

## Acceptance Criteria

- `completions` has integration coverage for all supported shells.
- Reserve occupancy flags have explicit CLI tests.
- New tests run in default CI suite (non-ignored).

