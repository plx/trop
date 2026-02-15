# P1-03: Reserve Occupancy Flags Are Exposed but Largely Unwired

## Problem

`reserve` exposes fine-grained occupancy flags (`--skip-tcp`, `--skip-udp`, `--skip-ipv4`, `--skip-ipv6`, `--check-all-interfaces`), but command execution only maps `--skip-occupancy-check` into a broad `ignore_occupied` option. The finer flags are parsed but not propagated into occupancy-check behavior.

## Evidence

- Flags defined on command:
  - `trop-cli/src/commands/reserve.rs:95`
  - `trop-cli/src/commands/reserve.rs:99`
  - `trop-cli/src/commands/reserve.rs:103`
  - `trop-cli/src/commands/reserve.rs:107`
  - `trop-cli/src/commands/reserve.rs:111`
  - `trop-cli/src/commands/reserve.rs:115`
- Options wiring only uses global skip as `ignore_occupied`:
  - `trop-cli/src/commands/reserve.rs:171`
- Reserve planner consumes occupancy config from merged config, not CLI flag overrides:
  - `trop/src/operations/reserve.rs:260`
  - `trop/src/operations/reserve.rs:326`

## Why This Matters

- CLI advertises behavior that does not occur.
- Users cannot reliably tune occupancy checks at invocation time.
- Increases risk of false assumptions during automation and troubleshooting.

## Suggested Remediation

1. Decide desired semantics:
   - either per-invocation occupancy override is supported for `reserve`,
   - or only config-file occupancy behavior is supported.
2. If supported:
   - add occupancy override fields to reserve options/planner path,
   - merge CLI flags into `OccupancyCheckConfig` before allocation call,
   - define precedence with config/env values.
3. If not supported:
   - remove these flags from `reserve` and document where occupancy controls are configured.
4. Add explicit integration tests proving each flag changes effective behavior.

## Acceptance Criteria

- Every exposed occupancy flag has an observable effect on reserve behavior, or is removed.
- Help text and README/spec alignment is explicit about supported controls.
- Tests assert both parsing and behavioral outcomes for occupancy flag combinations.

