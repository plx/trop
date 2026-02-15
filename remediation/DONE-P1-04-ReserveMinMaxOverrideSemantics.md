# P1-04: `reserve --min/--max` Override Logic Can Clobber Config Range

## Problem

When either `--min` or `--max` is provided to `reserve`, CLI code rebuilds `config.ports` with:

- `min = provided min OR DEFAULT_MIN_PORT`
- `max = provided max`

This can silently ignore existing configured `min`/`max` values and widen/shift the effective range unexpectedly when only one side is overridden.

## Evidence

- Override branch and defaulting behavior:
  - `trop-cli/src/commands/reserve.rs:156`
  - `trop-cli/src/commands/reserve.rs:160`
  - `trop-cli/src/commands/reserve.rs:164`

## Why This Matters

- Surprising runtime behavior vs user intent.
- Can allocate outside expected project range.
- Hard to diagnose because command succeeds and looks valid.

## Suggested Remediation

1. Preserve current config values for unspecified bounds:
   - if only `--max` is set, keep existing `min`;
   - if only `--min` is set, keep existing `max`/`max_offset` semantics.
2. Define and enforce clear precedence/compatibility rules:
   - CLI min/max vs existing `max_offset`.
   - Behavior when no existing `ports` config exists.
3. Consider stricter UX:
   - either require both `--min` and `--max`, or
   - allow partial overrides but print effective range when verbose.
4. Add tests for partial override cases with non-default config.

## Acceptance Criteria

- Partial bound overrides do not discard unrelated configured bound values.
- Effective range is deterministic and documented.
- Tests cover:
  - only `--min`
  - only `--max`
  - both provided
  - coexistence with configured `max_offset`.

