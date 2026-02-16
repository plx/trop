# P3-05: CLI Module Duplication Between `lib.rs` and `main.rs`

## Problem

The CLI crate compiles similar module graphs twice:

- as library modules (exported in `src/lib.rs`),
- and again as binary-local modules (declared in `src/main.rs`).

This creates duplicated compilation/test surface and maintenance overhead.

## Evidence

- Library module exports:
  - `trop-cli/src/lib.rs:6`
  - `trop-cli/src/lib.rs:9`
- Binary redeclares same modules:
  - `trop-cli/src/main.rs:11`
  - `trop-cli/src/main.rs:14`

## Why This Matters

- Redundant compile/test work (observed duplicate unit test execution for CLI internals).
- Increases chance of divergence between lib-facing and bin-facing module wiring.
- Makes ownership boundaries less clear.

## Suggested Remediation

1. Make binary depend on library exports:
   - in `main.rs`, import from `trop_cli::{cli::Cli, ...}` (or equivalent crate path),
   - remove duplicate `mod ...;` declarations from `main.rs`.
2. Keep command execution code in shared library module(s), leaving `main.rs` as thin entrypoint.
3. Re-run test suite to confirm behavior is unchanged and duplicate unit runs are eliminated.

## Acceptance Criteria

- `main.rs` no longer redeclares library modules.
- CLI functionality and tests remain green.
- Duplicate module-level unit test execution is reduced.

