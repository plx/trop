# P2-01: Data Directory Resolution Can Panic Instead of Returning CLI Error

## Problem

CLI utility code resolves default data directory with an unconditional `expect(...)`. If home directory resolution fails and `TROP_DATA_DIR` is unset, the process panics instead of returning a structured CLI error with controlled exit code.

## Evidence

- Panic path:
  - `trop-cli/src/utils.rs:186`
  - `trop-cli/src/utils.rs:187`
  - `trop-cli/src/utils.rs:188`

## Why This Matters

- Crashes violate CLI reliability expectations.
- Harder for automation to handle than typed exit codes.
- Affects headless/containerized environments where HOME may be absent.

## Suggested Remediation

1. Change `resolve_data_dir()` to return `Result<PathBuf, CliError>`.
2. Map home-resolution failure into existing error taxonomy:
   - likely `CliError::NoDataDirectory` or a new explicit variant.
3. Thread `?` propagation through callers (`resolve_config_file`, commands that call it).
4. Add integration test simulating missing HOME and no `TROP_DATA_DIR`:
   - assert non-zero, stable exit code,
   - assert user-facing error message, no panic.

## Acceptance Criteria

- No panic path remains for data-dir resolution.
- Missing-home scenario produces stable CLI error and exit code.
- Existing success behavior with `--data-dir`/`TROP_DATA_DIR` remains unchanged.

