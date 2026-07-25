# Pre-release due-diligence audit

**Project:** `trop`

**Audit date:** 2026-07-25

**Audit scope:** The Rust library, CLI, persistence layer, command behavior,
configuration, tests, automation, packaging, and release posture. The static
documentation site, landing page, and design-system work were explicitly out of
scope.

**Status:** Historical audit baseline. Findings in this document describe the
repository at the audited revision and should not be assumed to describe later
revisions without a follow-up audit.

## Executive verdict

**No-go for a formal or stable release at the audited revision.**

The implementation should not be submitted to Homebrew, described as reliable,
or recommended for workplace use until the release-blocking findings in this
report are fixed and independently re-audited.

This conclusion is not a recommendation to rewrite the project. The foundation
is considerably better than its experimental origin might suggest: the Rust is
generally clean, typed, documented, modular, and extensively tested. The
dominant failure mode is incomplete end-to-end wiring. Individual components and
tests often look sound while several commands do not uphold the product-level
contracts in the authoritative
[implementation specification](../../reference/ImplementationSpecification.md).

Two findings are immediate blockers:

1. Generated shell output can incorporate unvalidated configuration data into
   executable syntax, while the README recommends evaluating that output.
2. Group reservation operations violate `trop`'s central idempotency guarantee.

Several additional high-severity failures affect canonical reservation identity,
cleanup safety, transactional behavior, release and migration safeguards,
configuration precedence, occupancy checks, and corrupted-database handling.

## Scope and methodology

The audit treated
[`reference/ImplementationSpecification.md`](../../reference/ImplementationSpecification.md)
as the source of truth and did not modify it. The review covered:

- Command behavior and CLI-to-library option plumbing.
- Path identity, configuration precedence, and environment handling.
- Port allocation, group relationships, idempotency, and occupancy detection.
- SQLite schema, transactions, locking, migrations, cleanup, and corruption
  handling.
- Shell, dotenv, human-readable, and machine-readable output.
- Destructive-operation safeguards and dry-run behavior.
- Unit, integration, property, concurrency, and benchmark coverage.
- Rust quality, dependency security, packaging, metadata, CI, and publication
  state.

The audit combined source inspection, specification comparison, test-suite
inspection, targeted black-box reproductions, database inspection, package
dry-runs, and release-build checks. Among the targeted reproductions were:

- Repeating offset and preferred group reservations.
- Invoking group commands with relative and basename-only configuration paths.
- Exhausting a range containing a stale reservation.
- Requesting allowed project metadata changes.
- Releasing tagged reservations without specifying a tag.
- Releasing unrelated absolute paths without force.
- Selectively disabling UDP occupancy checks.
- Pruning through a permission-denied parent.
- Running dry-run cleanup on overlapping prune/expire candidates.
- Migrating to a nonexistent destination.
- Injecting a negative timestamp into SQLite and comparing validation with list
  behavior.
- Holding an immediate database lock past the configured timeout.
- Creating invalid exclusions through the CLI.

No production or repository state was deliberately mutated as part of the final
report; reproductions used isolated temporary data directories and fixtures.

The findings were subsequently decomposed into the native GitHub epic and
dependency structure recorded in the
[July 2026 remediation roadmap](2026-07-25-remediation-roadmap.md). That
roadmap, rather than this historical narrative, is the live index for
implementation sequencing.

## Verification results

The following commands passed against the audited revision:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --release --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo package -p trop --allow-dirty
cargo package -p trop-cli --allow-dirty
```

The main library suite alone contained 654 passing tests, with additional
integration, property, concurrency, CLI, and benchmark tests. Both crate
packages were successfully assembled and verified.

The dependency security audit did **not** pass:

```text
cargo audit
```

It reported three RustSec vulnerabilities and five warnings. Details appear in
[AUDIT-011](#audit-011-dependency-security-is-not-release-clean).

The green test and lint results are meaningful evidence of engineering effort,
but they are not sufficient release evidence. Some tests explicitly require
behavior that contradicts the authoritative specification, including changing
group ports on repeated requests.

## Publication state at audit time

As of 2026-07-25:

- [`trop` 0.1.0](https://crates.io/crates/trop) and
  [`trop-cli` 0.1.0](https://crates.io/crates/trop-cli) had already been
  published to crates.io on 2025-10-21.
- The audited source still declared version `0.1.0` in
  [`trop/Cargo.toml`](../../trop/Cargo.toml#L3) and
  [`trop-cli/Cargo.toml`](../../trop-cli/Cargo.toml#L3), but differed materially
  from the published source. It therefore could not replace crates.io's
  immutable 0.1.0 packages.
- The repository had no
  [GitHub release](https://github.com/plx/trop/releases) and no
  [Git tag](https://github.com/plx/trop/tags).
- No `trop` formula or cask was present in the official Homebrew index.
- The actual repository was `plx/trop`, but manifest metadata, badges, and
  several installation links still referred to `prb/trop`.

The next crates.io publication must be at least `0.1.1`. Because the complete
repair is expected to include material behavioral and likely schema changes, a
small `0.1.1` security hotfix followed by a comprehensively stabilized `0.2.0`
would be a reasonable versioning strategy.

## Severity model

- **Critical:** A security flaw or failure of the tool's defining contract that
  makes normal documented use unsafe or fundamentally unreliable.
- **High:** A correctness, data-safety, transactional, or configuration failure
  that can silently produce the wrong reservation state, delete valid state,
  bypass a safety boundary, or crash on plausible stored input.
- **Medium:** A release-quality or robustness defect that materially weakens
  portability, maintainability, automation, diagnosability, packaging, or
  less-common workflows.

## Critical and high-severity findings

### AUDIT-001: Generated shell output can execute configuration-supplied syntax

**Severity:** Critical

**Areas:** Security, output formatting, configuration, documentation

Service tags permit almost arbitrary nonempty strings in
[`validator.rs`](../../trop/src/config/validator.rs#L85-L110). When a service
does not have an explicit `env:` mapping,
[`resolve_env_var_name`](../../trop/src/output/formatters.rs#L51-L63) suppresses
its own identifier-validation failure and falls back to the raw uppercased tag.
That value is interpolated into shell assignment syntax by
[`shell.rs`](../../trop/src/output/shell.rs#L103-L108).

This path reaches the default output of `reserve-group` and `autoreserve`, while
the [README](../../README.md#L73-L77) recommends evaluating that output:

```bash
eval "$(trop autoreserve)"
```

A repository-controlled configuration can therefore cause unintended shell
syntax to execute under the user's account when the documented workflow is
followed. An accidentally malformed tag can at minimum corrupt the generated
script or dotenv output. The affected formatter and CLI pathways were also
present in the already-published 0.1.0 crates.

Required remediation:

- Remove the unsafe fallback. An invalid identifier must produce a typed error,
  never raw output.
- Validate every generated name during configuration validation and again at
  the rendering boundary.
- Define and document the portable identifier grammar, including the policy for
  deriving a name from a tag.
- Ensure explicit mappings and derived names cannot collide.
- Escape values according to the selected shell; do not attempt to make invalid
  variable names safe by quoting them.
- Remove or prominently suspend the `eval` recommendation until a fixed version
  is available.
- Add adversarial, pre-fix-failing tests for Bash, Zsh, Fish, PowerShell, and
  dotenv, covering punctuation, whitespace, quotes, substitutions, newlines,
  Unicode, duplicate derived names, and explicitly mapped names.

Release response:

- Publish a fixed version.
- Issue a security notice describing affected versions and the documented
  trigger without providing unnecessary exploit material.
- Yank both 0.1.0 crates after the replacement is available. Yanking will not
  revoke existing installations or lockfiles, so disclosure remains necessary.

### AUDIT-002: Group reservations are not idempotent

**Severity:** Critical

**Areas:** Allocation, group reservations, autoreserve, transactions

The specification's defining invariant is that repeated requests for the same
directory/tag identity return the same reservation. Group planning never
reconciles an existing compatible group. It always requests a fresh allocation
in
[`operations/reserve_group.rs`](../../trop/src/operations/reserve_group.rs#L198-L236).
The allocator then treats the group's currently reserved ports as unavailable
and replaces the same keys in
[`port/group.rs`](../../trop/src/port/group.rs#L219-L327).

Observed consequences:

- Repeating an offset group moves its ports.
- Repeating a preferred-only group can fail because its own reservation occupies
  the preferred port.
- Alternating `reserve-group` and `autoreserve` is unstable.
- A script cannot treat group output as a stable, idempotent interface.

Tests in
[`group_commands.rs`](../../trop-cli/tests/group_commands.rs#L1531-L1638)
explicitly assert changed ports on repetition, preserving the defect rather than
the specification.

Required remediation:

- Load existing reservations for every requested service under the same
  immediate transaction used to allocate.
- Return and refresh an existing compatible complete group.
- Distinguish complete compatible groups, partial groups, relationship changes,
  preferred-port changes, and sticky metadata changes.
- Apply explicitly permitted changes to the complete group atomically; otherwise
  return a clear conflict.
- Exclude the group's own current ports correctly when evaluating a permitted
  reallocation.
- Ensure either the entire group replacement commits or no member changes.
- Replace the existing defect-locking tests with tests that fail before the fix:
  repeated offset groups, repeated preferred groups, alternating commands,
  partial database state, metadata-change policy, and multiprocess concurrent
  requests must all preserve one stable complete group.

### AUDIT-003: Group reservation identity can be relative or empty

**Severity:** High

**Areas:** Paths, group reservations, cleanup

`ReserveGroupPlan` derives the reservation path from raw
`config_path.parent()` in
[`operations/reserve_group.rs`](../../trop/src/operations/reserve_group.rs#L136-L155).
`ReservationKey::new` then accepts that path without normalization in
[`reservation.rs`](../../trop/src/reservation.rs#L75-L90).

The audit reproduced:

- A relative config argument storing a relative reservation path.
- A basename-only argument such as `trop.yaml` storing an empty path.
- A later prune removing that reservation because the empty path appears not to
  exist.

Reservation identity consequently depends on command spelling and current
working directory.

Required remediation:

- Resolve the config path and its parent to one absolute normalized identity
  before constructing any key.
- Apply the specification's canonicalization policy for inferred paths.
- Reject empty or nonabsolute persisted reservation paths.
- Add database-level logical validation for path invariants.
- Add pre-fix-failing tests for absolute, relative, `./`, basename-only,
  symlinked, and alternate-spelling config paths, followed by repeated reserve
  and prune operations.

### AUDIT-004: `reserve` exposes guarantees and controls it does not implement

**Severity:** High

**Areas:** Single reservation, CLI plumbing, metadata, cleanup, dry-run

Several user-facing options are parsed but ineffective:

- `--overwrite` is declared but unused in
  [`commands/reserve.rs`](../../trop-cli/src/commands/reserve.rs#L45-L47).
- `--force` does not imply overwrite plus occupancy and exclusion bypass as
  required by the specification.
- An existing reservation only receives `UpdateLastUsed` in
  [`operations/reserve.rs`](../../trop/src/operations/reserve.rs#L307-L315);
  permitted project, task, and related metadata changes are never persisted.
- Omitting project/task on a repeated request is treated as trying to clear
  existing metadata rather than leaving it unchanged.
- Git project/task inference exists in the library but has no production CLI
  caller.
- Configured project, cleanup, and change-policy values are mostly discarded.
- Auto-prune and auto-expire settings are stored but never used. Exhaustion
  immediately returns `tried_cleanup: false` in
  [`operations/reserve.rs`](../../trop/src/operations/reserve.rs#L318-L356).
- Dry-run returns before opening the database or building a real plan in
  [`commands/reserve.rs`](../../trop-cli/src/commands/reserve.rs#L215-L223), so
  impossible operations can report a successful preview.

The audit reproduced a one-port range with a stale reservation failing instead
of pruning and retrying. It also reproduced an allowed project change reporting
success while retaining the original project in SQLite.

Required remediation:

- Define one effective `ReserveOptions` construction path with explicit
  precedence and complete CLI/config/environment mapping.
- Implement overwrite, force, allow-change, and omitted-metadata semantics
  exactly as specified.
- Either wire Git inference into normal execution or remove the unsupported
  claim and unused pathway.
- On exhaustion, run only the permitted cleanup stages, once, in the required
  order, then retry allocation once.
- Make dry-run construct and validate the same plan as execution without
  committing it.
- Add pre-fix-failing black-box tests for every flag independently and in
  combinations, configuration-derived equivalents, metadata preservation and
  changes, stale-range retry, exhaustion after retry, and dry-run parity.

### AUDIT-005: Configuration precedence is not honored end to end

**Severity:** High

**Areas:** Configuration, command integration, paths, environment

The typed merge implementation is substantial, but commands routinely bypass or
discard its result:

- `reserve-group` loads only the supplied raw file, not the built-in, global,
  project, local, and environment layers.
- `autoreserve` discovers multiple files but selects one highest-precedence file
  rather than merging fields.
- The README's reservations-only example omits `ports`; direct group loading
  consequently fails instead of inheriting the documented default range.
- Default discovery begins from relative `"."` in
  [`config/builder.rs`](../../trop/src/config/builder.rs#L114), while the upward
  walker in [`config/loader.rs`](../../trop/src/config/loader.rs#L117) expects a
  meaningful parent chain.
- File-level `disable_autoinit` is ignored because
  [`open_database`](../../trop-cli/src/utils.rs#L102-L119) checks only the
  global Clap flag.
- Implicit current-directory paths are normalized but not canonicalized in
  [`utils.rs`](../../trop-cli/src/utils.rs#L43-L50).
- Some environment variable names disagree between help/documentation and the
  config loader.
- Output format and logging configuration do not consistently reach commands.

Required remediation:

- Create one source-aware effective-configuration pipeline shared by every
  command.
- Preserve provenance for values and for commands that mutate a particular YAML
  source.
- Resolve the discovery root to an absolute path before upward traversal.
- Define a single generated or centrally declared mapping between fields, CLI
  arguments, and environment names.
- Add table-driven precedence tests across defaults, user-global, project,
  private-local, environment, and CLI sources for every configurable field.
- Add end-to-end tests from nested directories and alternate path spellings,
  including `disable_autoinit`, output format, reservations-only files, and
  exclusions-only local overlays.

### AUDIT-006: Selective occupancy behavior is not implemented

**Severity:** High

**Areas:** Port occupancy, allocation, scan, portability

[`SystemOccupancyChecker`](../../trop/src/port/occupancy.rs#L124-L145) handles
only the cases where both protocols or both address families are skipped.
Otherwise it delegates to `port-selector::is_free`, ignoring the individual
TCP, UDP, IPv4, IPv6, loopback/wildcard, and interface selections.

The audit reproduced a UDP-only listener blocking allocation even when
`--skip-udp` was supplied. If an environment cannot bind IPv6, the delegated
check can also classify all ports as occupied while `--skip-ipv6` cannot recover.
`scan` and `port-info` do not consistently use the effective occupancy policy.

Required remediation:

- Implement an explicit protocol × address-family × interface bind matrix.
- Distinguish address-in-use, permission, unsupported-family, and other socket
  failures; preserve the required fail-closed policy for unknown errors.
- Apply one occupancy policy consistently to allocation, scan, and port-info.
- Decide and document loopback versus wildcard behavior.
- Add pre-fix-failing tests for TCP and UDP over IPv4 and IPv6, every selective
  skip flag, unsupported IPv6, wildcard versus loopback listeners, and injected
  probe errors.

### AUDIT-007: Release semantics and path safeguards are incorrect

**Severity:** High

**Areas:** Destructive operations, release, transactions, paths

A nonrecursive `release` without a tag constructs only an untagged key in
[`commands/release.rs`](../../trop-cli/src/commands/release.rs#L126-L135).
The specification requires releasing every reservation at the exact path,
tagged and untagged. The audit reproduced a tagged reservation surviving a
successful default release.

The CLI also hardcodes `allow_unrelated_path(true)` for normal and recursive
branches in
[`commands/release.rs`](../../trop-cli/src/commands/release.rs#L86-L89),
bypassing the unrelated-path safety policy. The audit reproduced deleting a
reservation at a sideways absolute path without `--force`.

Recursive release reads and plans rows outside a transaction and commits each
deletion separately, permitting races and partial completion.

Required remediation:

- Make tag omission select all exact-path reservations.
- Restore the relationship guard and require the specified explicit override
  for unrelated paths.
- Resolve, enumerate, plan, and delete under one immediate transaction.
- Ensure recursive selection is defined for normalized path-component
  descendants, not string prefixes.
- Make dry-run report the exact rows the real transaction would delete.
- Add pre-fix-failing tests for mixed tagged/untagged rows, no-match behavior,
  unrelated and ancestor/descendant paths, prefix lookalikes, recursive
  all-or-nothing failure, and concurrent reserve/release races.

### AUDIT-008: Cleanup can delete valid state and is not atomic

**Severity:** High

**Areas:** Cleanup, filesystem errors, transactions, destructive operations

The cleanup implementation says it fails open on filesystem errors, but
[`check_path_exists`](../../trop/src/operations/cleanup.rs#L265-L271) reduces
existence to `fs::metadata(path).is_ok()`. Permission denial, symlink loops, and
transient I/O errors are treated as nonexistence. The audit reproduced deletion
of a reservation after making the existing path's parent unreadable.

Prune and expire select candidates and then delete each in a separate
transaction:

- [`prune`](../../trop/src/operations/cleanup.rs#L104-L128)
- [`expire`](../../trop/src/operations/cleanup.rs#L173-L199)

This creates a destructive race:

1. Expire observes an old `last_used_at`.
2. Another reserve refreshes the reservation.
3. Expire deletes it by key using the stale observation.

Further issues:

- Dry-run autoclean double-counts a row that is both nonexistent and expired.
- CLI `--days 0` bypasses the configuration validator and can immediately
  expire every reservation.
- Partial errors leave partial cleanup results.

Required remediation:

- Treat only definitive `NotFound` and `NotADirectory` results as absent;
  preserve reservations and report diagnostics for unknown filesystem errors.
- Validate all CLI durations through the same rules as file configuration.
- Select, deduplicate, revalidate, and delete candidates in one immediate
  transaction.
- Recheck mutable predicates before deletion or express them in guarded SQL.
- Ensure dry-run and execution share candidate-selection logic.
- Add pre-fix-failing tests for permission denial, symlink loops, transient
  errors, overlap/deduplication, zero days, concurrent refresh-versus-expire,
  process interruption, and all-or-nothing failures.

### AUDIT-009: Migration planning is vulnerable to time-of-check/time-of-use races

**Severity:** High

**Areas:** Migration, transactions, paths

Migration conflict detection and source enumeration occur before the write
transaction in
[`commands/migrate.rs`](../../trop-cli/src/commands/migrate.rs#L43-L53).
Execution begins its transaction later in
[`operations/migrate.rs`](../../trop/src/operations/migrate.rs#L398-L412).

A destination reservation created between planning and execution can therefore
be overwritten without force. Source changes can likewise be applied from stale
data. The implementation also accepts a nonexistent destination despite the
specification requiring it to exist, and overlapping recursive transformations
can conflict with rows created earlier in their own plan.

Required remediation:

- Validate source/destination existence and path relationships before mutation.
- Perform normalization, enumeration, mapping, conflict checks, and writes
  under one immediate transaction, or use an equivalently verified snapshot and
  commit protocol.
- Define safe ordering for overlapping source/destination trees.
- Never implement conflict replacement as an unconditional delete followed by
  insert unless the current row was revalidated and replacement was authorized.
- Add pre-fix-failing barrier-based concurrency tests for a destination created
  after initial inspection, source refresh/removal, recursive overlap,
  nonexistent destinations, forced replacement, and rollback after mid-plan
  failure.

### AUDIT-010: Logical database corruption can panic while validation succeeds

**Severity:** High

**Areas:** Database, schema, corruption, error handling

Timestamp conversion casts a signed SQLite value directly to `u64` in
[`database/operations.rs`](../../trop/src/database/operations.rs#L34-L38).
After setting `last_used_at = -1`, the audit observed:

- `trop assert-data-dir --validate` exiting successfully.
- `trop list` panicking with exit status 101 due to `SystemTime` overflow.

Validation currently relies on SQLite's physical integrity check in
[`database/operations.rs`](../../trop/src/database/operations.rs#L686), not
logical application invariants. `assert-data-dir` also converts validation errors
to a boolean, allowing some corruption/error combinations to be misreported.

The schema declares nullable `tag` as part of an ordinary composite primary key
in [`database/schema.rs`](../../trop/src/database/schema.rs#L28-L38). SQLite can
therefore hold multiple `(path, NULL)` rows despite intended uniqueness. Writers
work around this by deleting before inserting, but the database does not enforce
the invariant.

Required remediation:

- Introduce a schema migration with enforced untagged uniqueness, for example a
  non-null normalized tag representation or an expression-based unique index.
- Add `CHECK` constraints for valid ports, nonnegative/representable timestamps,
  and other enforceable invariants.
- Validate absolute normalized paths and key uniqueness logically.
- Replace all unchecked conversions with typed corruption errors; stored data
  must never cause a panic.
- Make validation distinguish invalid layout, inaccessible database, lock
  timeout, physical corruption, and logical corruption.
- Add pre-fix-failing tests using malformed timestamps, ports, paths, duplicate
  untagged rows, unknown schema versions, and corrupted table contents.
- Add migration backup, rollback, and recovery tests.

Database lock errors are also mapped to generic database failures, leaving the
advertised `LockTimeout` exit path effectively unreachable. Holding an immediate
lock past the requested timeout produced exit code 6 instead of the specified
code 2.

### AUDIT-011: Dependency security is not release-clean

**Severity:** High

**Areas:** Dependencies, supply chain, CI

`cargo audit` reported:

- Runtime:
  [`RUSTSEC-2025-0140`](https://rustsec.org/advisories/RUSTSEC-2025-0140.html)
  through `gix-date 0.9.4`.
- Runtime:
  [`RUSTSEC-2025-0021`](https://rustsec.org/advisories/RUSTSEC-2025-0021.html)
  through `gix-features 0.39.1`.
- Development-only:
  [`RUSTSEC-2026-0204`](https://rustsec.org/advisories/RUSTSEC-2026-0204.html)
  through `crossbeam-epoch 0.9.18`.

The affected `gix` path was not wired into the normal CLI reserve flow at audit
time, which reduces immediate CLI reachability, but it remained public library
functionality. The audit also warned about locked `anyhow`, `memmap2`, and
development dependency versions. `serde_yaml 0.9` is deprecated/unmaintained,
and some direct core dependencies appeared unused.

Required remediation:

- Upgrade, replace, remove, or narrowly justify every advisory before release.
- Replace deprecated YAML tooling with a maintained implementation and include
  compatibility/round-trip tests.
- Remove unused direct dependencies.
- Run RustSec and dependency-policy checks in normal CI and release CI.
- Establish a documented policy for advisory exceptions, including owner,
  reachability analysis, compensating controls, and expiration date.
- Add lockfile-aware dependency update automation that is not blocked by stale
  generated notices.

### AUDIT-012: Preferred group allocation semantics contradict the specification

**Severity:** High

**Areas:** Group allocation, preferred ports, fallback

Group preferred-port behavior has several independent faults:

- Preferred ports outside the ordinary scan range are rejected as excluded by
  [`port/allocator.rs`](../../trop/src/port/allocator.rs#L214-L221), although
  group preferred ports are specified as absolute requests that may sit outside
  the scan range.
- An unavailable preferred port errors rather than falling back to that
  service's offset pattern in
  [`port/group.rs`](../../trop/src/port/group.rs#L256-L280).
- Pattern selection does not fully account for preferred ports selected in the
  same request, so it can choose a collision and abort instead of scanning the
  next valid base.

Required remediation:

- Separate absolute preferred-port validation from ordinary candidate-range
  scanning.
- Encode the documented preferred-then-offset fallback order explicitly.
- Reserve all fixed choices in the in-memory candidate set before selecting a
  pattern base.
- Continue scanning after an internal collision rather than failing the whole
  operation prematurely.
- Add pre-fix-failing tests for preferred ports below/above the range,
  occupied/reserved/excluded preferred ports, fallback, multiple preferred
  services, collision with an offset, and finding a later valid base.

## Medium-severity findings

### AUDIT-013: The database does not enforce untagged identity uniqueness

**Severity:** Medium independently; incorporated into the high-severity
corruption/schema remediation above.

**Areas:** Schema, invariants

Because `tag` is nullable in the composite primary key, SQLite permits multiple
rows representing the same untagged identity. The public raw connection surface
and external tooling can create this state, and physical integrity validation
will accept it. Resolve this in the schema migration described in
[AUDIT-010](#audit-010-logical-database-corruption-can-panic-while-validation-succeeds).

### AUDIT-014: Lock timeouts use the wrong error and exit contract

**Severity:** Medium

**Areas:** Database locking, scripting interface, errors

The dedicated lock-timeout variant is never constructed. `SQLITE_BUSY` and
`SQLITE_LOCKED` propagate as generic database errors. A reproduced timeout
exited 6 rather than the specification's reserved exit code 2.

Normalize SQLite busy/locked results at every transaction boundary, preserve
useful context, and add subprocess tests that hold read/write locks for shorter
and longer than the requested timeout.

### AUDIT-015: Path handling is incomplete and non-UTF-8 behavior is inconsistent

**Severity:** Medium

**Areas:** Paths, portability

[`utils.rs`](../../trop-cli/src/utils.rs#L43-L50) uses explicit-path resolution
even when the path is the implicit current working directory, contrary to the
canonicalization rule for inferred paths. Normalization also rejects non-UTF-8
paths in
[`path/normalize.rs`](../../trop/src/path/normalize.rs#L44-L48), while
reservation persistence exposes lossy conversion in
[`reservation.rs`](../../trop/src/reservation.rs#L160-L166).

Define one platform-aware path identity contract, avoid silent lossy conversion,
and test symlinks, missing leaves, dot segments, Unicode normalization where
relevant, non-UTF-8 Unix paths, and Windows prefixes/case behavior.

### AUDIT-016: Exclusion and autoexclude mutations can corrupt or leak configuration

**Severity:** Medium

**Areas:** Configuration mutation, exclusions, scan, concurrency

`exclude` has multiple unsafe behaviors:

- If no project configuration exists, it can fall back to the global file
  without an explicit global request through
  [`utils.rs`](../../trop-cli/src/utils.rs#L229-L247).
- It accepts invalid ports/ranges without shared configuration validation.
- It swallows database lookup errors using `unwrap_or(false)`.
- It detects only exact duplicates and does not merge overlapping ranges as
  specified.
- It rewrites the complete YAML file directly, losing comments and allowing
  concurrent writers or crashes to corrupt or lose changes.

The audit reproduced `trop exclude 0 --global` writing a configuration that a
subsequent `trop list` rejected.

`scan --autoexclude` is more dangerous: it starts from the fully merged effective
configuration and serializes that into one selected source file in
[`commands/scan.rs`](../../trop-cli/src/commands/scan.rs#L127-L155). This can
persist built-in defaults, environment values, global configuration, or private
local values into the wrong file.

Required remediation:

- Mutate only the explicitly selected source document.
- Refuse an ambiguous or missing target rather than silently choosing global.
- Validate the result through the same typed validator before replacement.
- Merge and normalize overlapping/adjacent exclusions deterministically.
- Propagate database errors.
- Preserve comments where feasible or document a structured rewrite policy.
- Use a same-directory temporary file, flush/fsync as appropriate, and atomic
  rename under an interprocess lock.
- Add pre-fix-failing tests for invalid endpoints, target selection, overlapping
  ranges, comments, environment-secret non-persistence, concurrent writers, and
  injected write/rename failures.

The same safe-write primitive should be used by `init`, `compact-exclusions`,
and other YAML mutation commands.

### AUDIT-017: Dry-run behavior is superficial

**Severity:** Medium

**Areas:** Planning, CLI, safety

Single reserve, group reserve, and autoreserve dry-runs return before meaningful
database inspection, validation, or allocation. They do not reveal exhaustion,
sticky metadata conflicts, invalid groups, or planned ports. A dry-run that
cannot predict whether the real operation can succeed is not a useful safety
interface.

Execution and dry-run should share plan construction and validation; only the
commit should differ. Add parity tests asserting that dry-run and execution
either fail with the same semantic error or describe the same actions.

### AUDIT-018: `PlanExecutor` makes atomicity an undocumented caller obligation

**Severity:** Medium

**Areas:** Public library API, transactions

[`operations/executor.rs`](../../trop/src/operations/executor.rs#L173-L229)
executes actions over an arbitrary `&Connection` without opening an outer
transaction. The CLI correctly wraps some operations, but examples and public
callers can accidentally execute a multi-action plan nonatomically.

Make transaction ownership explicit in the type/API, accept a transaction
rather than a raw connection for mutating execution, or provide one canonical
atomic execute method and make lower-level behavior clearly internal.

### AUDIT-019: Rust edition and minimum-supported-version policy are inconsistent

**Severity:** Medium

**Areas:** Toolchain, manifests, release policy

The specification requires Rust 2024, while both
[`trop/Cargo.toml`](../../trop/Cargo.toml#L1-L8) and
[`trop-cli/Cargo.toml`](../../trop-cli/Cargo.toml#L1-L8) used edition 2021.
`.clippy.toml` named Rust 1.70, the toolchain file pinned 1.95, and dependencies
rejected 1.78. Neither manifest declared `rust-version`. Rust 1.85 successfully
checked during the audit, but the exact minimum was unestablished.

Choose and document the MSRV, declare it in each manifest, test it in CI, and
align the edition and toolchain files with the supported policy.

### AUDIT-020: Release metadata and distribution assets are incomplete

**Severity:** Medium

**Areas:** Packaging, release engineering, user installation

Packaging defects included:

- Stale `prb/trop` repository metadata and README links.
- Missing license texts from both `.crate` archives despite the manifests'
  `Apache-2.0 OR MIT` declaration.
- Member READMEs describing the license as MIT-only.
- No changelog, security policy, or clearly established contribution/release
  process.
- No tag or GitHub release.
- No release workflow for version verification, crates publication, immutable
  artifacts, checksums, signatures, SBOM/provenance, or release creation.
- Completion generation uses the package name in
  [`commands/completions.rs`](../../trop-cli/src/commands/completions.rs#L13),
  producing completions for `trop-cli` rather than the installed `trop`
  executable.
- [`build.rs`](../../trop-cli/build.rs#L135-L149) emits an incomplete root
  manpage only under `OUT_DIR`; `cargo install` does not install it despite the
  README promising `man trop`.
- Crate archives include the full test, benchmark, fixture, and property
  regression trees, increasing package size without a deliberate policy.

Create a reproducible, immutable release pipeline and test the final installed
artifact rather than only the workspace binary. Generate completions and
manpages from the actual Clap command model, verify their command names and
subcommands, and decide how non-Cargo assets will be distributed.

### AUDIT-021: CI and supply-chain policy are not sufficient for releases

**Severity:** Medium

**Areas:** CI, automation, provenance

The normal OS matrix, strict Clippy run, docs, and license checks are strengths,
but release coverage has important holes:

- Property testing has a manually dispatched workflow whose `just` dependency
  is not installed.
- There is no normal security audit, dependency policy, semver/API check,
  package-install smoke test, release artifact test, or migration compatibility
  matrix.
- Workflow actions and installed CLI tools are not pinned tightly enough for a
  trusted release path.
- `cargo install agentic-navigation-guide` is unpinned.
- No scheduled contention, soak, or crash-recovery jobs exist.

Add required release gates, pin third-party automation by immutable versions or
commit SHAs, and separate fast pull-request validation from scheduled/release
stress suites.

### AUDIT-022: Scan and diagnostic output fall short of the specified behavior

**Severity:** Medium

**Areas:** Scan, diagnostics, performance, output stability

[`commands/scan.rs`](../../trop-cli/src/commands/scan.rs#L196-L253) reports only
port, status, and reservation state rather than the required process, user, and
protocol details. It performs sequential socket probes, repeats linear
membership checks, and does not consistently honor configured scan bounds such
as `max_offset`.

Some machine-readable output derives ordering from unordered collections and
lacks compatibility snapshots. Unknown shell detection silently guesses Bash,
and logging is largely unwired: a logger is constructed and dropped while
`log::debug!` has no installed backend.

Define the diagnostic data contract, make ordering deterministic, implement
configured bounds and occupancy policy consistently, and add golden/snapshot
tests for JSON, shell, dotenv, human formats, quiet/verbose behavior, and stable
error output.

### AUDIT-023: Initialization and configuration replacement need crash safety

**Severity:** Medium

**Areas:** Initialization, filesystem safety, recovery

`init --force` and related writers rely on direct replacement patterns without a
complete recovery story for the SQLite database plus WAL/SHM files or for YAML
files. Data-directory and database permissions depend on ambient umask.
Initialization/schema repair also contains ad hoc version-1 repairs under an
unchanged schema version and can misclassify locking failures as read-only state.

Required remediation includes explicit ownership/permission policy, atomic
same-filesystem replacement, directory fsync where supported, backup and
rollback, WAL/SHM handling, typed lock/read-only errors, and fault-injection
tests at each replacement boundary.

## Transactional architecture assessment

The specification requires at most one atomic `BEGIN IMMEDIATE` transaction per
mutating invocation. The audited state was uneven:

<!-- markdownlint-disable MD013 -->

| Operation | Audited behavior | Assessment |
| --- | --- | --- |
| Single reserve | Planning and execution occur inside one immediate transaction | Good foundation |
| Group reserve | Atomic insertion/savepoint, but incorrect reconciliation semantics | Structurally promising; behavior wrong |
| Prune/expire | Candidate read followed by per-row transactions | Unsafe |
| Recursive release | Plans outside transaction and commits per row | Unsafe |
| Migration | Writes are grouped, but planning/conflict checks occur outside | TOCTOU-prone |
| YAML mutation | Direct truncate-and-write patterns | Crash- and concurrency-unsafe |

<!-- markdownlint-enable MD013 -->

The cleanest architectural repair is to make transaction ownership part of each
mutating operation's API rather than something every CLI command must remember
to arrange.

## Strengths worth preserving

The audit also identified substantial positive engineering:

- Clean library/CLI separation and generally coherent module boundaries.
- Strong `Port`, `PortRange`, reservation, configuration, and error types.
- The core crate denies missing documentation and unsafe code and enables
  pedantic Clippy policy in [`lib.rs`](../../trop/src/lib.rs#L1).
- No production project `unsafe` code was found.
- SQL is parameterized rather than constructed from user strings.
- SQLite WAL mode, busy timeouts, deterministic allocation, and a unique-port
  constraint are appropriate choices.
- The normal single-reserve path wraps planning and execution in one immediate
  transaction.
- Group insertion uses a savepoint, preventing partial insertion on a detected
  collision.
- Occupancy probe errors fail closed.
- Unit, integration, property, multiprocess, and benchmark coverage is
  unusually substantial for a project at this maturity.
- CI already covers Ubuntu, macOS, and Windows in debug and release modes with
  strict linting.
- Both crates package successfully, isolated Cargo installation works, and
  repeated same-host release builds were byte-identical during the audit.
- The release binary was modest in size, approximately 4 MiB.

These strengths support an incremental hardening plan rather than a rewrite.

## Performance and operational-readiness assessment

No raw performance blocker was demonstrated for the intended small local
workload. SQLite and the existing indexes are a reasonable foundation.
Production-readiness under heavier use is nevertheless unproven:

- Existing benchmark scale reaches only about 1,000 reservations.
- `scan` performs sequential socket checks and avoidable linear membership
  searches.
- All writers intentionally serialize through one user-global database.
- There are no regression thresholds, long-running soak tests, crash/fault
  injection, or high-contention cleanup and migration workloads.

After correctness repairs, add multiprocess suites with dozens of concurrent
allocators, forced busy-lock contention, process interruption around commit
boundaries, large stale tables, and concurrent reserve-versus-cleanup/migration
operations. Performance gates should measure both latency and correctness; a
fast but non-idempotent or partially committed result is a failure.

## Remediation roadmap

### Phase 0: Immediate public-safety response

1. Remove the unsafe `eval` guidance.
2. Fix identifier validation and output rendering with defense in depth.
3. Add adversarial shell and dotenv tests.
4. Publish a fixed release, disclose the issue, and yank 0.1.0 after a
   replacement is available.

### Phase 1: Restore defining semantics

1. Convert the specification's promises into black-box contract tests.
2. Implement group idempotency and canonical group identity.
3. Correct preferred-group and fallback semantics.
4. Unify effective configuration resolution for all commands.
5. Complete reserve overwrite, force, allow-change, inference, and cleanup
   behavior.
6. Implement exact release-all-tags behavior and restore path safeguards.
7. Implement the complete occupancy matrix.

### Phase 2: Harden transactions and stored data

1. Make every mutating invocation one immediate transaction.
2. Move migration planning and conflict detection inside its transaction.
3. Make cleanup error-conservative, deduplicated, and race-safe.
4. Introduce schema version 2 with logical constraints and a tested migration.
5. Add corruption, crash-recovery, lock-timeout, and filesystem-fault tests.
6. Make YAML mutation source-aware, validated, locked, and atomically replaced.
7. Make dry-run use the real planner without committing.

### Phase 3: Release engineering

1. Resolve RustSec findings and migrate away from deprecated YAML tooling.
2. Establish edition, MSRV, supported targets, and public API policy.
3. Bump versions and add a changelog and security policy.
4. Correct repository metadata and package both license texts.
5. Generate completions and all manpages from the real Clap model.
6. Add package/install/audit/semver/migration/release-artifact CI gates.
7. Produce an immutable tag and GitHub release with checksums, notices, and
   preferably SBOM and provenance.
8. Publish via
   [crates.io Trusted Publishing](https://crates.io/docs/trusted-publishing).
9. Ship a custom Homebrew tap and dogfood it before considering Homebrew/core.

## Release gate

A follow-up audit should not declare release readiness until all of the
following are true:

- Every critical and high finding is closed with a regression test that fails on
  the audited revision and passes on the candidate.
- The specification and tests agree on repeated single and group behavior.
- All mutating commands demonstrate atomicity under forced concurrency and
  injected failure.
- Malformed configuration and corrupted databases produce typed errors, never
  panics, unsafe output, or partial mutation.
- `cargo fmt`, strict Clippy, all tests, Rustdoc, package verification, install
  smoke tests, MSRV checks, and `cargo audit` pass in CI.
- The exact packaged artifacts—not only workspace builds—pass command, manpage,
  completion, migration, and platform smoke tests.
- Publication metadata, licenses, versions, changelog, security notice, tag,
  checksums, and provenance describe the exact same immutable source.

## Bottom line

`trop` has a solid core and is worth finishing. It is neither an embarrassing
codebase nor architecturally doomed. At the audited revision, however, several
central product invariants—especially secure shell integration, group
idempotency, cleanup safety, and transactional mutation—were absent or
explicitly contradicted.

Until stabilization, personal use should be limited to simple direct
`trop reserve` calls. Avoid evaluating group output and avoid destructive
cleanup, recursive release, migration, and autoexclude operations. A formal
release should follow only after the remediation sequence above and the rigorous
follow-up audit maintained separately in this repository.
