# P3-03: AGENTIC Navigation Guide Is Stale

## Problem

`AGENTIC_NAVIGATION_GUIDE.md` describes an old, reduced project layout and omits many current modules/tests. It no longer reflects the actual repository structure.

## Evidence

- Guide lists only early-phase files/modules:
  - `AGENTIC_NAVIGATION_GUIDE.md:9`
  - `AGENTIC_NAVIGATION_GUIDE.md:13`
  - `AGENTIC_NAVIGATION_GUIDE.md:31`
- Current repository includes much larger structure (`trop/src/database`, `trop/src/operations`, many CLI commands/tests), not represented in the guide.

## Why This Matters

- Agent delegation and contributor orientation rely on this file.
- Stale guide causes wasted exploration time and mistaken assumptions.

## Suggested Remediation

1. Regenerate the guide from current tree with meaningful annotations.
2. Keep high-level navigation concise but include all major subsystems:
   - config, database, operations, path, port, output, CLI commands, key test suites.
3. Add maintenance rule:
   - update guide when introducing new top-level modules or command families.

## Acceptance Criteria

- Guide accurately reflects current major directories and key files.
- Newly onboarded contributors can locate all major subsystems from guide alone.
- CI `navigation-guide` check passes with updated content.

