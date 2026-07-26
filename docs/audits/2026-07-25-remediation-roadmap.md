# July 2026 production-readiness remediation roadmap

This document is the durable index for the GitHub work created from the
[July 2026 pre-release due-diligence audit](2026-07-25-pre-release-due-diligence.md).
It records the issue taxonomy, epic hierarchy, stable ticket keys, and intended
dependency order as they existed when the audit-remediation program was
created.

GitHub's native sub-issue and blocked-by relationships are authoritative for
live planning. This file exists so that the intended structure remains
understandable from a checkout, in review, and across agent context compaction.

## Program container

- Milestone:
  [Production readiness / v0.2.0](https://github.com/plx/trop/milestone/2)
- Top-level epic:
  [#83 — Production readiness stabilization for trop v0.2.0](https://github.com/plx/trop/issues/83)
- Final gate:
  [#137 — Execute the independent post-remediation production-readiness audit](https://github.com/plx/trop/issues/137)
- Audit runbook:
  [Post-remediation production-readiness audit](post-remediation-production-readiness-audit.md)
- Remediation execution goal:
  [Production-readiness remediation goal](production-readiness-remediation-goal.md)

At creation time the program contained 55 issues, 54 native parent
relationships, and 74 native blocked-by relationships. An API-level graph check
found no missing parent, missing label axis, missing milestone, or dependency
cycle.

The automatic work-selection setup later added 48 landed-only gate
relationships: each component epic is blocked by all of its actionable
sub-issues, and the top-level epic is blocked by the final audit. The live
graph therefore has 122 native blocked-by relationships: 74 implementation
prerequisites plus 48 gate prerequisites. See the
[production-readiness work-selection guide](production-readiness-work-selection.md)
for the operational contract.

## Label taxonomy

Every issue created by this audit carries one label from each primary axis:

1. **Priority:** `P0`, `P1`, `P2`, or `P3`.
2. **Work type:** `type:epic`, `type:security`, `type:bug`,
   `type:hardening`, `type:release`, or `type:testing`.
3. **Domain:** the primary quality concern, such as `domain:security`,
   `domain:correctness`, `domain:reliability`, `domain:data-integrity`,
   `domain:release`, `domain:maintainability`, `domain:performance`, or
   `domain:testing`.
4. **Component:** the implementation owner, such as `component:groups`,
   `component:database`, `component:cleanup`, `component:cli`, or
   `component:packaging`.

Cross-cutting labels have separate meanings:

- `release-blocker`: must be resolved before a supported
  production-oriented release.
- `breaking-change`: requires an explicit compatibility decision.
- `data-migration`: changes or migrates persistent representation.
- `audit:2026-07`: belongs to this due-diligence program.

Three additional labels support fail-closed automatic scheduling:

- `workflow:production-readiness`: all 55 issues in the canonical audit
  universe;
- `workflow:production-readiness-leaf`: the 47 independently actionable
  issues, #90-#136; and
- `workflow:production-readiness-gate`: the eight aggregate gates, #83-#89
  and #137.

The selector requires the `audit:2026-07` and
`workflow:production-readiness` cohorts to match exactly. Every member must
carry exactly one leaf/gate label and exactly one recognized `P0`-`P3` label.

The pre-existing `P0`-`P3` labels were retained so historical issues and the
new program use one priority vocabulary.

## Epic hierarchy

- [#84 — Security and public-safety response](https://github.com/plx/trop/issues/84)
- [#85 — Reservation semantics, configuration, paths, and occupancy](https://github.com/plx/trop/issues/85)
- [#86 — Persistence, transactions, cleanup, release, migration, and integrity](https://github.com/plx/trop/issues/86)
- [#87 — CLI contracts and safe configuration mutation](https://github.com/plx/trop/issues/87)
- [#88 — Dependencies, platform policy, test rigor, and performance](https://github.com/plx/trop/issues/88)
- [#89 — Packaging, publishing, and distribution](https://github.com/plx/trop/issues/89)

Each actionable ticket is a native sub-issue of exactly one component epic.
The component epics and final audit are native sub-issues of #83.

## Ticket catalog

### Security and public safety

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `SEC-1` | [#90 — Reject unsafe shell and dotenv identifiers at every output boundary](https://github.com/plx/trop/issues/90) | P0 | None |
| `SEC-2` | [#91 — Publish a fixed security release and respond to affected 0.1.0 crates](https://github.com/plx/trop/issues/91) | P0 | #90 |

### Reservation semantics, configuration, paths, and occupancy

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `CFG-1` | [#92 — Introduce one source-aware effective configuration pipeline for every command](https://github.com/plx/trop/issues/92) | P1 | None |
| `PATH-1` | [#93 — Fix upward config discovery and canonicalize inferred paths](https://github.com/plx/trop/issues/93) | P1 | #92 |
| `GRP-1` | [#94 — Store an absolute canonical reservation identity for group config parents](https://github.com/plx/trop/issues/94) | P0 | #93 |
| `GRP-2` | [#95 — Make reserve-group and autoreserve idempotent and transactionally reconcile existing groups](https://github.com/plx/trop/issues/95) | P0 | #94 |
| `GRP-3` | [#96 — Implement group sticky-field, path-safety, force, and shape-change semantics](https://github.com/plx/trop/issues/96) | P1 | #92, #95 |
| `GRP-4` | [#97 — Correct preferred-port and offset-pattern semantics for groups](https://github.com/plx/trop/issues/97) | P1 | #94, #95 |
| `CFG-2` | [#98 — Merge full configuration for reserve-group and autoreserve](https://github.com/plx/trop/issues/98) | P1 | #92, #93 |
| `RES-1` | [#99 — Implement reserve overwrite, force, and authorized metadata-change semantics](https://github.com/plx/trop/issues/99) | P1 | #92 |
| `RES-2` | [#100 — Distinguish omitted metadata and wire project/task inference and config values](https://github.com/plx/trop/issues/100) | P1 | #92, #99 |
| `RES-3` | [#101 — Implement configured cleanup and one retry on reserve exhaustion](https://github.com/plx/trop/issues/101) | P1 | #92, #108, #109 |
| `OCC-1` | [#102 — Implement the documented protocol/address/interface occupancy matrix](https://github.com/plx/trop/issues/102) | P1 | None |
| `OCC-2` | [#103 — Use effective occupancy policy consistently in reserve, scan, and port-info](https://github.com/plx/trop/issues/103) | P1 | #92, #102 |
| `PATH-2` | [#104 — Define and enforce a cross-platform path storage and key-invariant policy](https://github.com/plx/trop/issues/104) | P2 | #93, #105 |

### Persistence, transactions, and data integrity

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `DB-1` | [#105 — Introduce schema v2 with enforced key, port, and timestamp invariants](https://github.com/plx/trop/issues/105) | P1 | None |
| `DB-2` | [#106 — Make row decoding non-panicking and validate logical database integrity](https://github.com/plx/trop/issues/106) | P1 | #105 |
| `DB-3` | [#107 — Map SQLite Busy and Locked failures to the lock-timeout contract](https://github.com/plx/trop/issues/107) | P1 | None |
| `CLN-1` | [#108 — Make prune conservative on filesystem errors](https://github.com/plx/trop/issues/108) | P1 | None |
| `CLN-2` | [#109 — Make prune, expire, and autoclean one race-safe atomic transaction](https://github.com/plx/trop/issues/109) | P1 | #108 |
| `CLN-3` | [#110 — Validate cleanup thresholds and align dry-run accounting](https://github.com/plx/trop/issues/110) | P2 | #109 |
| `REL-1` | [#111 — Release all exact-path tags by default and restore path safeguards](https://github.com/plx/trop/issues/111) | P1 | #93 |
| `REL-2` | [#112 — Make recursive release component-aware and atomic](https://github.com/plx/trop/issues/112) | P1 | #111 |
| `MIG-1` | [#113 — Move migration planning into one transaction and require a destination](https://github.com/plx/trop/issues/113) | P1 | #93 |
| `MIG-2` | [#114 — Handle recursive migration overlap without data loss](https://github.com/plx/trop/issues/114) | P1 | #113 |
| `INIT-1` | [#115 — Make init and force-reinitialization recoverable and permission-safe](https://github.com/plx/trop/issues/115) | P1 | #105 |
| `PLAN-1` | [#116 — Make transaction ownership explicit in the public plan-execute API](https://github.com/plx/trop/issues/116) | P2 | None |

### CLI contracts and safe configuration mutation

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `YAML-1` | [#117 — Build a source-aware, validated, locked, atomic YAML editing primitive](https://github.com/plx/trop/issues/117) | P1 | #92 |
| `EXC-1` | [#118 — Fix exclude target selection, validation, reservation errors, and range merging](https://github.com/plx/trop/issues/118) | P1 | #92, #117 |
| `SCAN-1` | [#119 — Make scan and autoexclude respect provenance, max_offset, and scale](https://github.com/plx/trop/issues/119) | P1 | #92, #103, #117 |
| `DRY-1` | [#120 — Make every dry-run execute the real plan and differ only at commit](https://github.com/plx/trop/issues/120) | P1 | #95, #99, #109, #112, #113, #117 |
| `CLI-1` | [#121 — Wire logging, global output behavior, and explicit shell detection](https://github.com/plx/trop/issues/121) | P2 | #90 |
| `CLI-2` | [#122 — Harden deterministic output, escaping, errors, and broken-pipe behavior](https://github.com/plx/trop/issues/122) | P2 | #90 |
| `CLI-3` | [#123 — Snapshot the supported CLI, output schemas, environment, and exits](https://github.com/plx/trop/issues/123) | P2 | #90, #107, #121, #122 |

### Dependencies, platform policy, testing, and performance

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `DEP-1` | [#124 — Remediate RustSec advisories and deprecated/unused dependencies](https://github.com/plx/trop/issues/124) | P1 | None |
| `DEP-2` | [#125 — Enforce dependency, license, and update-health policy in CI](https://github.com/plx/trop/issues/125) | P1 | #124 |
| `PLAT-1` | [#126 — Adopt Rust 2024 and establish MSRV and target support tiers](https://github.com/plx/trop/issues/126) | P1 | #124 |
| `TEST-1` | [#127 — Repair property CI and add race/fault/invariant infrastructure](https://github.com/plx/trop/issues/127) | P1 | None |
| `PERF-1` | [#128 — Establish performance, contention, load, and soak budgets](https://github.com/plx/trop/issues/128) | P2 | #95, #103, #109, #127 |
| `API-1` | [#129 — Review and deliberately define the public API surface](https://github.com/plx/trop/issues/129) | P2 | #116, #126 |

### Packaging, publishing, and distribution

| Key | Issue | Priority | Native blockers |
| --- | --- | --- | --- |
| `PKG-1` | [#130 — Choose the next version and correct changelog/repository/Cargo metadata](https://github.com/plx/trop/issues/130) | P1 | #126 |
| `PKG-2` | [#131 — Package both license texts and synchronized third-party notices](https://github.com/plx/trop/issues/131) | P1 | #124 |
| `PKG-3` | [#132 — Generate and test completions for the installed trop executable](https://github.com/plx/trop/issues/132) | P1 | #123 |
| `PKG-4` | [#133 — Generate complete manpages from the real CLI and install them](https://github.com/plx/trop/issues/133) | P1 | #123, #129 |
| `PKG-5` | [#134 — Add package, installed-binary, docs.rs, and reproducibility gates](https://github.com/plx/trop/issues/134) | P1 | #126, #130, #131, #132, #133 |
| `RELENG-1` | [#135 — Create a trusted, immutable, verifiable release pipeline](https://github.com/plx/trop/issues/135) | P1 | #125, #126, #134 |
| `BREW-1` | [#136 — Publish and test a custom Homebrew tap formula](https://github.com/plx/trop/issues/136) | P2 | #135 |

## Dependency-ordered burn-down

The following waves are a topological layering of the native dependency graph.
Issues within a wave can be worked in parallel. Priority still applies within a
wave, especially the immediate `SEC-1` public-safety work.

### Wave 0: independent foundations

- #90 `SEC-1`
- #92 `CFG-1`
- #102 `OCC-1`
- #105 `DB-1`
- #107 `DB-3`
- #108 `CLN-1`
- #116 `PLAN-1`
- #124 `DEP-1`
- #127 `TEST-1`

### Wave 1: first consumers of the foundations

- #91 `SEC-2`
- #93 `PATH-1`
- #99 `RES-1`
- #106 `DB-2`
- #109 `CLN-2`
- #115 `INIT-1`
- #117 `YAML-1`
- #121 `CLI-1`
- #122 `CLI-2`
- #125 `DEP-2`
- #126 `PLAT-1`
- #131 `PKG-2`

### Wave 2: identity, command, and contract integration

- #94 `GRP-1`
- #98 `CFG-2`
- #100 `RES-2`
- #101 `RES-3`
- #103 `OCC-2`
- #104 `PATH-2`
- #110 `CLN-3`
- #111 `REL-1`
- #113 `MIG-1`
- #118 `EXC-1`
- #123 `CLI-3`
- #129 `API-1`
- #130 `PKG-1`

### Wave 3: dependent semantic and packaging work

- #95 `GRP-2`
- #112 `REL-2`
- #114 `MIG-2`
- #119 `SCAN-1`
- #132 `PKG-3`
- #133 `PKG-4`

### Wave 4: full behavior and candidate validation

- #96 `GRP-3`
- #97 `GRP-4`
- #120 `DRY-1`
- #128 `PERF-1`
- #134 `PKG-5`

### Wave 5: release automation

- #135 `RELENG-1`

### Wave 6: distribution rehearsal

- #136 `BREW-1`

### Final gate

The selector offers each component epic #84-#89 only after all of its native
leaf blockers are actually closed. Execute the epic's aggregate acceptance
criteria before closing it. Then execute #137 exactly as written in the
[post-remediation audit runbook](post-remediation-production-readiness-audit.md).
After #137 closes, #83 becomes the last selectable gate. A `GO` decision is not
implied by reaching any of these points.

## Ticket execution standard

An implementation agent should be able to work from one ticket without relying
on conversational context. Every actionable issue therefore contains:

- the violated behavior and why it matters;
- required implementation properties without prescribing a brittle patch;
- concrete regression scenarios that should fail before the fix;
- validation commands and failure-path expectations;
- acceptance criteria;
- one stable key, one native parent, and native blocked-by relationships.

When a ticket discovers a specification ambiguity, stop and record the decision
explicitly rather than silently choosing behavior. When implementation reveals
another independently actionable defect, file and link a new issue instead of
expanding the current ticket beyond reviewable scope.
