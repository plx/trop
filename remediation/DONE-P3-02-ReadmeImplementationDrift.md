# P3-02: README Files Drift from Current Implementation

## Problem

Repository README documents no longer match real implementation status and command behavior.

Current drift includes:

1. CLI README still claims only phase-1/help/version functionality.
2. Library README still claims only phase-1 core types.
3. Root README includes a broken command example (`--api` instead of `--tag`).

## Evidence

- CLI README states early phase and limited commands:
  - `trop-cli/README.md:11`
  - `trop-cli/README.md:13`
  - `trop-cli/README.md:59`
- Library README phase status outdated:
  - `trop/README.md:108`
  - `trop/README.md:110`
  - `trop/README.md:112`
- Root README broken example:
  - `README.md:50`
- Actual CLI includes many implemented commands:
  - `trop-cli/src/cli.rs:47`
  - `trop-cli/src/cli.rs:113`

## Why This Matters

- New users get incorrect setup/usage guidance.
- Increases support and onboarding friction.
- Undermines confidence in project maturity and docs quality.

## Suggested Remediation

1. Refresh `trop-cli/README.md` from actual `trop --help` output.
2. Rewrite `trop/README.md` status section to reflect implemented modules and API surface.
3. Fix invalid examples in root README (`--tag`).
4. Add a lightweight docs-sync practice:
   - release checklist item or CI doc consistency check against command list.

## Acceptance Criteria

- README command examples run successfully against current binary.
- Status sections accurately describe implemented phases/features.
- No references remain that claim the CLI/library is phase-1 only.

