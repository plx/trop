# P3-04: Backup Source Artifacts (`*.bak`) Are Checked Into `src/`

## Problem

Backup files are committed alongside production Rust source in `trop/src/config`.

## Evidence

- `trop/src/config/builder.rs.bak`
- `trop/src/config/merger.rs.bak`
- `trop/src/config/validator.rs.bak`

## Why This Matters

- Adds noise to code search and review.
- Increases risk of stale-copy confusion during edits.
- Makes repository hygiene and intent less clear.

## Suggested Remediation

1. Remove `.bak` files from source control.
2. Add ignore pattern(s) to prevent reintroduction:
   - either targeted entries for these files or a scoped `*.bak` policy.
3. If historical snapshots are needed, move to dedicated archival docs/notes area instead of `src/`.

## Acceptance Criteria

- No `.bak` artifacts remain under `trop/src/`.
- Git ignore policy prevents accidental recommit of backup files.
- Build/test behavior unchanged after cleanup.

