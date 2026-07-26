# Post-remediation production-readiness audit

## Purpose

This runbook is the final independent audit for `trop` after the remediation
issues from the initial due-diligence review have been closed. Its purpose is
not to confirm that tickets have a `closed` status. Its purpose is to determine,
from source, executable behavior, persisted state, release artifacts, and
adversarial testing, whether the audited revision is suitable for dependable
use outside toy, hobby, or experimental prototypes.

The auditor must assume that a remediation can be incomplete even when its
unit tests pass. In particular, the audit must verify behavior through the
public CLI and the resulting SQLite/configuration state, not just by testing
helpers in isolation.

The remediation program is indexed in the
[July 2026 remediation roadmap](2026-07-25-remediation-roadmap.md), and its
independent-audit gate is
[#137](https://github.com/plx/trop/issues/137). Closed remediation tickets and
a green milestone are prerequisites for this runbook, not substitutes for
executing it. A `GO` at #137 is followed by #150 publication and #151
distribution; it is neither the final program gate nor publication authority.

The authoritative product contract is
`reference/ImplementationSpecification.md`. Do not modify that file as part of
this audit. If implementation and specification disagree, record a finding.
An intentional deviation is acceptable only when an independently reviewed,
approved specification change already exists.

The static documentation site and landing page are outside this runbook unless
they have become part of the release's supported installation or security
surface. User-facing CLI and crate documentation, man pages, completions,
licenses, release notes, and packaging metadata are in scope.

## Audit principles

1. **Audit an immutable revision.** Record and use one full Git commit SHA.
   Changes made in response to findings require a new revision and a complete
   rerun of every affected gate.
2. **Use isolated state.** Never point destructive tests at a developer's real
   data directory, global configuration, reservations, or shell startup files.
3. **Capture evidence.** A claim without a command transcript, test result,
   state inspection, or source reference is not a passed gate.
4. **Test externally observable semantics.** A zero exit status is not enough.
   Inspect stdout, stderr, exit code, database rows, timestamps, files, and
   subsequent command behavior.
5. **Challenge the tests.** For critical regressions, demonstrate in a
   disposable worktree that a controlled mutation or reintroduction of the old
   defect makes the test fail.
6. **Fail conservatively.** Unsupported platforms, skipped scenarios,
   nondeterministic failures, missing provenance, or an unreviewed spec
   discrepancy are not silent passes.
7. **Do not repair while auditing.** Record findings against the immutable
   revision. Fixes belong in follow-up changes and trigger re-audit.

## Approved audit subject and publication boundary

The maintainer-approved lifecycle is recorded in the
[production-readiness remediation goal](production-readiness-remediation-goal.md).
For #137, the audit subject is the single public GitHub prerelease closed by
candidate gate [#149](https://github.com/plx/trop/issues/149): one exact final
commit, comprehensive version, never-moved tag, identity manifest, saved Cargo
package set, and complete target-artifact set. The auditor must download that
public subject as an unauthenticated user. Do not create another tag or release,
rebuild or repack a substitute, replace an asset, or audit a merely equivalent
candidate.

Comprehensive production publication is deliberately absent during #137.
Verify through unauthenticated production crates.io and index queries that the
exact comprehensive version is unpublished for both `trop` and `trop-cli`.
Exercise library-first, index-wait, then CLI publication only through the
disposable local registry or staging mechanism rehearsed by #135. Production
credentials and production publication are outside this audit.

Every target artifact in #149 must have a SHA-256 checksum, a verifiable
signature, an SPDX or CycloneDX SBOM, and provenance tied to its exact source
and workflow. Missing or unverifiable evidence is a failed mandatory gate.
Download and verify each artifact and all four evidence objects without
authentication. The custom-tap formula closed by
[#136](https://github.com/plx/trop/issues/136) must consume the exact #149 asset
URL and checksum; a rebuilt archive or alternate formula is not the audit
subject.

Issue #135 has already proved candidate construction and disposable-remote
mechanics. This audit instead rehearses publication and promotion from #149's
saved artifacts without production credentials and proves that no source,
version, tag, lockfile, package, artifact, checksum, signature, SBOM,
provenance, README, changelog, Cargo metadata, or candidate-contained status
text change is required. After `GO`, any such candidate-affecting change
invalidates the verdict.

An exact `GO` closes #137 and only unlocks publication gate
[#150](https://github.com/plx/trop/issues/150); it does not authorize
publication. Issue #150 requires a fresh irreversible-action approval, publishes
the saved `trop` package, waits for registry availability, publishes the saved
`trop-cli` package, and promotes the existing prerelease without changing its
tag or assets. Distribution gate
[#151](https://github.com/plx/trop/issues/151) then performs post-release
custom-tap verification. Neither gate is part of the #137 audit verdict or may
be skipped.

## Required outputs

The audit must produce an evidence directory and a signed-off report. The
report must include:

- audited commit, version, date, auditor, and host/container details;
- exact Rust/Cargo/SQLite/tool versions;
- the specification traceability matrix;
- every command run and its exit code;
- logs for all test, lint, audit, package, install, and load runs;
- hashes of source and release artifacts;
- the #149 prerelease URL, identity manifest, saved-package identities, and
  unauthenticated download evidence;
- proof that the exact comprehensive `trop` and `trop-cli` versions are absent
  from production crates.io throughout the audit;
- the #136 formula revision, exact #149 asset URL and checksum, and platform
  results;
- the saved-artifact publication/promotion rehearsal and its no-change
  comparison;
- before/after SQLite dumps for mutation scenarios;
- platform-specific results for Linux, macOS, and Windows;
- skipped tests and why they were skipped;
- every finding with severity, reproducible steps, expected/actual behavior,
  source locations, and attached evidence;
- the final go/no-go decision using the rubric at the end of this document.

Preserve the evidence bundle with the release record. Do not commit secrets,
private paths, usernames, signing keys, or unredacted environment dumps.

## 1. Prepare a clean, reproducible audit environment

### 1.1 Freeze the subject

Start from a fresh clone or a new worktree with no untracked files:

```bash
git status --short
git rev-parse HEAD
git show --no-patch --format=fuller HEAD
git describe --always --dirty --tags
git submodule status --recursive
```

The worktree must be clean, `git describe` must not say `dirty`, and `HEAD` must
be the exact commit named by #149's public prerelease, never-moved tag, and
identity manifest. Verify the tag resolves directly to that commit and has not
been recreated or moved. Record the output before doing anything else.

Confirm that the version in both crate manifests, `trop --version`, the
changelog, tag, saved Cargo packages, and target artifact names agree. Confirm
that the lockfile is tracked and unchanged, every #149 release-asset hash
matches the identity manifest, and normalized package file-manifest/content
digests match wherever a registry regenerates transport archives.

Before executing the rest of the runbook, verify #84-#89, #149, and #136
closed through their dedicated evidence PRs and that the component evidence is
fresh for this exact commit. Record unauthenticated proof that the comprehensive
version is absent for both production crates.

### 1.2 Create isolated state

On Unix-like systems, create an audit root and retain it as evidence:

```bash
audit_root="$(mktemp -d)"
audit_data_dir="$audit_root/data"
audit_projects="$audit_root/projects"
audit_evidence="$audit_root/evidence"
mkdir -p "$audit_data_dir" "$audit_projects" "$audit_evidence"
export TROP_DATA_DIR="$audit_data_dir"
```

Use a fresh data directory for each scenario that changes database state. A
test harness should create those directories automatically and pass them using
`--data-dir` or `TROP_DATA_DIR`. Do not override `HOME` to simulate global
configuration; use the product's explicit data-directory controls.

On Windows, use an equivalent directory created beneath
`$env:TEMP`, set `$env:TROP_DATA_DIR`, and record the fully resolved paths.

Before every destructive CLI scenario:

1. print the resolved data directory with `trop show-data-dir`;
2. assert that it is beneath the audit root;
3. record the command and resolved directory;
4. seed only disposable fixtures.

### 1.3 Record the build environment

Capture:

```bash
rustc --version --verbose
cargo --version --verbose
rustup show
git --version
sqlite3 --version
uname -a
cargo metadata --locked --format-version 1
cargo tree --workspace --all-features
cargo tree --workspace --all-features --duplicates
```

On Windows, also capture the OS build, architecture, PowerShell version, and
MSVC toolchain details. On macOS, capture the macOS and Xcode/Command Line Tools
versions. On Linux, capture distribution, libc, and kernel versions.

List only the names of relevant environment variables; redact values that may
contain paths, tokens, credentials, or private data. Explicitly establish that
no `TROP_*` variable other than those intended by the scenario leaked into the
test environment.

### 1.4 Required platform matrix

At minimum, run the release gates on:

- current supported Ubuntu or equivalent glibc Linux, x86-64;
- current supported macOS on Apple Silicon;
- current supported Windows on x86-64;
- every additional target for which a prebuilt artifact will be published.

If Intel macOS or musl Linux artifacts are published, test those artifacts on
their native or faithful virtualized runtime. Cross-compilation alone does not
verify runtime behavior.

Run network tests as an ordinary unprivileged user. If a privileged run is
also performed, report it separately; privileged success must not substitute
for the ordinary-user result.

## 2. Establish specification traceability

Create a matrix with one row for every normative requirement in
`reference/ImplementationSpecification.md`. At minimum, assign stable
requirement IDs to:

- scope and single-user behavior;
- path normalization, canonicalization, and hierarchy protections;
- reservation keys and sticky metadata;
- configuration fields, sources, merge precedence, and provenance;
- every global option and subcommand;
- allocation and group-allocation algorithms;
- occupancy protocols, address families, and interfaces;
- errors, stdout/stderr discipline, and exit codes;
- SQLite mode, timeouts, transaction boundaries, schema, and migrations;
- plan/execute behavior and dry-run;
- test, logging, packaging, and implementation requirements.

For each row, record:

| Field | Required content |
| --- | --- |
| Requirement ID | Stable audit-local identifier |
| Specification reference | Heading and line or paragraph |
| Implementation | Source file and symbol |
| Positive test | Test name proving expected behavior |
| Negative/adversarial test | Test name proving rejection or safety behavior |
| CLI evidence | Black-box scenario, when user-observable |
| Result | Pass, fail, or blocked |
| Notes | Approved deviation or finding reference |

Do not use “covered by unit tests” as evidence. Name the test and explain which
observable invariant it asserts. A requirement with no direct evidence is
unverified.

Review every remediation issue and pull request only after building this matrix.
For each issue:

- reproduce the original failing scenario against the fixed revision;
- confirm its regression test executes in normal CI;
- confirm the test asserts state and output, not merely success;
- inspect adjacent paths for variants the issue did not mention;
- record the issue and fixing commit in the traceability row.

## 3. Baseline source and build quality gates

Run from a clean checkout with the locked dependency graph:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo build --release --workspace --all-targets --all-features --locked
```

Also run each feature combination that users can select. `--all-features`
does not detect incompatible or accidentally coupled feature combinations.
Use `cargo hack` or an equivalent enumerator if the crates have optional
features:

```bash
cargo hack check --workspace --feature-powerset --locked
```

Audit the code structure:

- core behavior belongs in the library and CLI modules remain translation/glue;
- public mutation APIs make transaction ownership explicit;
- no public API permits bypassing invariants without an unmistakably unsafe or
  low-level contract;
- `#![forbid(unsafe_code)]` or equivalent policy covers production crates;
- all public API has accurate rustdoc, examples compile, and errors are typed;
- no panic, `unwrap`, `expect`, unchecked cast, or indexing operation is
  reachable from config, CLI, filesystem, database, clock, socket, or other
  untrusted runtime input;
- logs do not leak paths or environment values at normal verbosity;
- direct dependencies are used and justified.

Run a panic-focused search and disposition every result:

```bash
rg -n '\b(unwrap|expect|panic!|unreachable!|todo!|unimplemented!)\b' \
  trop trop-cli --glob '*.rs'
rg -n '\bunsafe\b|allow\(unsafe_code\)|expect\(clippy' \
  trop trop-cli --glob '*.rs'
```

Test malformed and extreme input under both debug and release builds. Neither
build may panic or abort. A caught panic is still a finding if ordinary input
can trigger it.

## 4. Security audit

The specification does not promise multi-user isolation or enforcement of
port ownership. That limited security model does **not** permit arbitrary code
execution, unsafe generated shell syntax, SQL injection, path confusion,
unintended writes, secret disclosure, or memory unsafety.

### 4.1 Generated shell and dotenv output

Construct group configurations containing service tags and explicit `env`
values drawn from these classes:

- valid portable identifiers: `PORT`, `WEB_PORT`, `_PRIVATE_PORT`;
- whitespace-only and leading/trailing whitespace;
- spaces, tabs, and embedded newlines;
- semicolon, ampersand, pipe, redirection, parentheses, and braces;
- dollar expansion, command-substitution syntax, and backticks;
- single quote, double quote, backslash, percent, colon, and equals;
- Unicode letters, combining marks, control characters, and NUL attempts;
- shell-specific forms for Bash/Zsh, Fish, and PowerShell.

Expected behavior:

- invalid explicit environment names are rejected during validation with a
  nonzero error exit and no reservation mutation;
- an unmapped service tag is either transformed by one documented,
  injective, validated mapping or rejected;
- validation errors are never discarded in favor of raw configuration text;
- `export`, Fish, PowerShell, and dotenv renderers each validate the final
  identifier;
- values are valid numeric ports and cannot introduce additional statements;
- no untrusted text reaches the variable-name position unquoted;
- JSON remains valid and does not contain terminal-control injection;
- invalid configuration never emits partially usable shell text.

Parse valid generated scripts with the actual supported shell:

```bash
bash -n generated.bash
zsh -n generated.zsh
fish --no-execute generated.fish
pwsh -NoProfile -Command \
  '$errors=$null; [System.Management.Automation.Language.Parser]::ParseFile(
    "generated.ps1",[ref]$null,[ref]$errors) > $null; if ($errors) { exit 1 }'
```

Do not evaluate adversarial output on an ordinary host. If execution resistance
is tested dynamically, do so in a disposable container/VM with no credentials,
no host mounts, no network, a read-only root filesystem, and a single writable
sentinel directory. Verify both that the command is rejected and that no
sentinel file/process/network activity occurs.

The regression suite must fail if the final validation is replaced with the
historic “uppercase raw tag” fallback. Prove that in a disposable worktree.

### 4.2 Input and persistence boundaries

Review and test:

- every SQL statement uses parameters for data;
- identifiers and schema statements are fixed by the program;
- malformed YAML, duplicate keys, aliases, deeply nested documents, very long
  strings, invalid UTF-8 where representable, and oversized files fail with
  bounded resource use and no partial mutation;
- path input cannot redirect writes outside the chosen data/config target;
- symlinked data directories, database files, config files, and replacement
  targets have deliberate, documented handling;
- temp files use collision-resistant creation and restrictive permissions;
- config replacement is lock-aware and uses same-filesystem atomic rename;
- errors and verbose logs do not reveal unrelated environment variables,
  credentials, database contents, or shell startup data;
- `--data-dir`, config discovery, and auto-init do not traverse into an
  attacker-selected location through unresolved symlinks unexpectedly;
- no command invokes a shell to perform ordinary operations;
- subprocess arguments, if any, are passed without shell interpolation.

Inspect resulting permissions under ordinary umasks. Databases, WAL/SHM files,
temporary files, and private config must not become group/world writable. If
the project promises a stricter read policy, verify it explicitly.

### 4.3 Dependency and repository security

Perform the supply-chain checks in section 13. In addition, review security
advisories for the exact locked versions and determine reachability. The
release gate remains failed for an unmitigated advisory even when the auditor
believes it is “probably unreachable”; reachability must be documented and an
exception explicitly approved.

Scan the repository and built artifacts for accidentally committed secrets,
private keys, tokens, credentials, machine-specific data, and debug symbols or
paths that violate the distribution policy.

## 5. Reservation correctness and idempotency

Use black-box CLI integration tests backed by an isolated real SQLite database.
For every case, assert stdout, stderr, exit code, row count, full row values,
and relevant timestamps.

### 5.1 Single reservations

Test all of the following:

1. First reserve for `(path, no tag)` creates exactly one row and returns its
   port as the only normal stdout value.
2. Repeating the exact request returns the same port, preserves `created_at`,
   advances `last_used_at`, and creates no extra row.
3. Repeating using equivalent normalized path spellings has the specified
   identity behavior.
4. A different tag at the same path creates a distinct key and port.
5. The same tag at a different path creates a distinct key and port.
6. Allocation chooses the lowest usable port after reservations, exclusions,
   and occupancy are considered.
7. A valid free preferred port is selected.
8. An unavailable preferred port follows the documented fallback/error rules.
9. Exhaustion invokes prune and expire once, retries the full range, and then
   either succeeds with newly freed capacity or returns an actionable error.
10. Each cleanup-disable flag suppresses only its intended cleanup phase;
    disabling both performs no cleanup.
11. Automatic cleanup plus allocation is atomic: no observer sees cleanup
    committed if the eventual allocation fails.

### 5.2 Sticky metadata and overrides

Seed a reservation with project and task metadata. Exercise unchanged values,
omitted values, changes to each field, clearing if supported, and changes to
both fields with:

- no override;
- `--allow-project-change`;
- `--allow-task-change`;
- `--allow-change`;
- `--overwrite`;
- `--force`;
- equivalent environment/config controls.

Unauthorized changes must fail without changing any row or timestamp.
Authorized changes must persist the requested metadata, preserve or replace
creation time according to the documented overwrite contract, refresh
`last_used_at`, and keep the reservation key/port consistent. `--force` must
have exactly the documented composite semantics, including preferred-port
occupancy and exclusion handling.

Verify project/task inference in an actual Git repository, in a worktree, in a
detached HEAD, outside Git, and when Git metadata is malformed or inaccessible.
Explicit CLI and environment values must override inference.

### 5.3 Group reservations

Cover offset-only, preferred-only, and mixed groups:

- first reservation creates the complete expected key set;
- repeated `reserve-group` returns exactly the same mapping;
- repeated `autoreserve` returns exactly the same mapping;
- alternating the two commands remains stable;
- `created_at` is stable and every member's `last_used_at` refreshes;
- a competing reservation forces selection of the next complete valid pattern;
- preferred ports outside the ordinary scan range work as specified;
- an unavailable preferred value follows its service offset fallback;
- preferred values are included when checking same-request collisions;
- exclusions and every occupancy dimension are applied to every member;
- no partial row set persists after any service fails;
- metadata changes obey the same sticky/override contract as single reserve;
- group definition changes have one documented, atomic reconciliation outcome;
- output order is deterministic across runs and platforms.

Use a barrier-based multiprocess test in which two processes request the same
new group concurrently. Both must return the same complete mapping, and the
database must contain one row per group service with unique ports.

### 5.4 Regression-test strength

For the single and group idempotency tests, temporarily mutate the allocator or
planner in a disposable worktree to force reallocation on the second request.
The black-box tests must fail on port equality and database-state assertions.
Restore/discard the disposable worktree after capturing the failing result.

## 6. Paths and configuration precedence

### 6.1 Path provenance and identity

Create real directories, symlinks, nested paths, nonexistent explicit paths,
and path spellings containing `.`, `..`, trailing separators, and platform
specific separators. Verify:

- every stored reservation path is absolute and normalized;
- an implicitly inferred CWD is canonicalized through symlinks;
- an explicitly supplied CLI/environment path is normalized but is not
  canonicalized;
- group config parent paths follow the specified provenance rule and are never
  relative or empty;
- a bare config filename, a relative config path, and its absolute equivalent
  produce one identity;
- `show-path` uses the exact same resolver as mutating commands;
- Unicode and non-UTF-8 path behavior is deliberate, portable where claimed,
  non-lossy, and tested;
- nonexistent explicit paths produce the documented warning/error without
  storing an accidental alternate identity;
- path relationship checks accept ancestors and descendants but reject
  sideways paths unless the exact override is present.

Delete a worktree path only after seeding a reservation, then verify pruning
removes the correct normalized identity. A reservation for an existing path
must survive.

### 6.2 Complete precedence matrix

For every configurable field, test the complete precedence order:

1. built-in default;
2. user config beneath the selected data directory;
3. project `trop.yaml`;
4. private `trop.local.yaml`;
5. environment;
6. CLI argument.

At each layer, set a distinguishable value and verify the effective result.
Then remove one layer at a time and verify fallback to the next. Include at
least:

- port minimum, maximum, and maximum offset;
- project and sticky-change policies;
- exclusions;
- cleanup age and auto-cleanup controls;
- occupancy protocol/address/interface controls;
- busy timeout and auto-init;
- output format;
- reservation-group fields.

Run the matrix for `reserve`, `reserve-group`, `autoreserve`, `scan`,
`port-info`, `list`, cleanup, assertions, and every other consuming command.
A merger unit test does not prove command plumbing.

Verify upward discovery from a nested directory, with both project files at
the same level and files at different ancestor levels. Confirm that group
definitions and ordinary settings merge according to the documented model;
the implementation must not silently pick one file and discard required
fields from another.

Audit all `TROP_*` names against CLI help and schema names. Every documented
name must work, stale aliases must have an explicit compatibility policy, and
unknown names must not silently masquerade as valid configuration.

### 6.3 Source-aware configuration mutation

Test `exclude`, `compact-exclusions`, `scan --autoexclude`, and every other
config writer using different values at all precedence layers.

Required assertions:

- project mutation fails if no project config exists unless `--global` was
  explicitly supplied;
- global mutation changes only the selected global file;
- effective defaults, environment values, private values, and values from
  other source files are not materialized into the target file;
- port/range input is validated before writing;
- overlapping and adjacent exclusions are merged without changing the set;
- a reserved port is rejected unless the documented override is supplied;
- reservation lookup errors fail closed rather than being treated as “not
  reserved”;
- comments and formatting follow the documented preservation/warning policy;
- writes use a validated temporary file, flush as required, and atomically
  replace the intended file;
- concurrent writers do not lose updates;
- injected failure before rename leaves the original valid file untouched;
- injected failure after rename yields one complete valid old/new version,
  never a truncated file.

After every mutation, run `trop validate` and independently parse the YAML.
Then invoke a consuming command to prove that the stored configuration remains
usable.

## 7. Occupancy and scanning

Implement the audit fixture with real sockets, not a mocked checker. For each
supported platform, hold listeners/sockets for this matrix:

| Protocol | Address family | Bind address |
| --- | --- | --- |
| TCP | IPv4 | `127.0.0.1` |
| UDP | IPv4 | `127.0.0.1` |
| TCP | IPv6 | `::1` |
| UDP | IPv6 | `::1` |
| TCP | IPv4 | `0.0.0.0` |
| UDP | IPv4 | `0.0.0.0` |
| TCP | IPv6 | `::` |
| UDP | IPv6 | `::` |

Use OS-assigned fixture ports where possible and keep socket handles open until
assertions complete. Account explicitly for platform-specific dual-stack
behavior such as an IPv6 wildcard socket also occupying IPv4.

For each row:

- the conservative default must classify the port as occupied when the bind
  address is within default scope;
- skipping that protocol must permit allocation if no checked combination is
  occupied;
- skipping the other protocol must continue to block it;
- skipping that address family must permit it if no checked family is occupied;
- `--check-all-interfaces` must add wildcard/non-loopback checks as specified;
- skipping all protocols or all families must have a documented, validated
  result rather than accidental behavior;
- config, environment, and CLI forms must be equivalent;
- `reserve`, `reserve-group`, `scan`, and `port-info` must agree.

Also test:

- a port reserved in SQLite but not bound;
- a bound port not reserved in SQLite;
- both reserved and bound;
- a socket closing between scan and commit;
- another process binding between check and consumer use;
- permission-denied or unsupported-family errors;
- IPv6 disabled at the OS/container level;
- ephemeral bind failures unrelated to `AddrInUse`.

Probe errors must fail closed for allocation and remain distinguishable in
diagnostic output. Unsupported IPv6 must not cause every port to appear
occupied when IPv4 remains usable.

For `scan`, independently establish listeners and verify the exact reported
port, protocol, address family, interface, process, and user where the platform
can supply them. Degraded fields must be explicit, not fabricated. Test table,
JSON, CSV, and TSV output and the configured maximum-offset range.

## 8. SQLite invariants, migrations, and corruption

### 8.1 Schema invariants

Inspect the actual schema:

```bash
sqlite3 "$TROP_DATA_DIR/trop.db" '.schema'
sqlite3 "$TROP_DATA_DIR/trop.db" 'PRAGMA journal_mode;'
sqlite3 "$TROP_DATA_DIR/trop.db" 'PRAGMA synchronous;'
sqlite3 "$TROP_DATA_DIR/trop.db" 'PRAGMA integrity_check;'
sqlite3 "$TROP_DATA_DIR/trop.db" \
  "SELECT key, value FROM metadata ORDER BY key;"
```

Prove at the database layer, not only through Rust constructors, that:

- reservation identity is unique for tagged and untagged keys;
- port numbers are globally unique and in `1..=65535`;
- timestamps are representable, nonnegative, and internally consistent;
- required paths are nonempty and follow the chosen storage invariant;
- schema version has exactly one valid value;
- foreign or duplicate metadata cannot create ambiguous startup behavior.

Attempt direct invalid inserts for each constraint and require SQLite to reject
them. Then invoke `assert-data-dir --validate` and require logical validation
to detect any corrupt state that the physical schema cannot forbid.

### 8.2 Logical and physical corruption

In disposable database copies, inject:

- negative and overflowing timestamps;
- `last_used_at` earlier than `created_at`, if disallowed;
- port zero and values over 65535;
- duplicate untagged identities;
- duplicate ports;
- empty, relative, and malformed path values;
- missing, nonnumeric, old, and future schema versions;
- missing tables, columns, and indices;
- truncated pages and random byte damage in a copied database.

Every CLI command must return a typed, nonpanic error. Validation must
distinguish invalid/corrupt data from “directory does not exist,” use stderr,
and must never claim the database is valid. `--not` must invert only the
semantic predicate; it must not turn an internal validation error into success.

The recovery message must be actionable and must not mutate or silently repair
data unless an explicitly documented migration is being performed.

### 8.3 Migration chain

For every previously released schema version:

1. Create or obtain a golden database using the old released binary.
2. Populate empty, typical, maximum-size, tagged/untagged, Unicode-path, and
   boundary-port data.
3. Preserve an untouched copy and its hash.
4. Open a copy with the candidate binary.
5. Verify each migration runs once, atomically, and advances one documented
   version at a time.
6. Compare every semantic row before and after.
7. Verify all new constraints and indices.
8. Reopen with the candidate; it must not rerun migration.
9. Open with the old binary; it must reject the newer schema without mutation.
10. Inject failure at each migration statement/phase and prove rollback leaves
    either the complete old schema or complete new schema, never a hybrid.

Test migration with active WAL/SHM files, read-only directories, disk-full
simulation, interrupted processes, and a concurrently held connection.
Document whether a backup is created, how it is named and protected, and how a
user restores it.

## 9. Transactions, races, crash safety, and fault injection

Source inspection must identify the transaction boundary for every mutating
command. Each invocation may have at most one atomic `BEGIN IMMEDIATE`
transaction, as required by the specification. Planning that depends on
mutable database state—candidate selection, cleanup predicates, conflict
checks, source enumeration—must occur within the protected transaction or be
revalidated before commit.

### 9.1 Barrier-controlled races

Use deterministic barriers or failpoints rather than relying only on timing.
At minimum test:

- two new single reserves for the same key;
- many new single reserves for different keys in a tiny range;
- two identical group reserves;
- overlapping groups;
- reserve racing with prune on the same path;
- reserve refresh racing with expire on the same row;
- release racing with reserve/re-reserve;
- recursive release racing with child creation;
- migration racing with destination creation;
- migration racing with source refresh/removal;
- config writer racing with config writer;
- auto-init racing with auto-init;
- schema migration racing with ordinary startup.

For every race, assert allowed results for both processes and the final state.
There must be no lost refresh, overwritten destination without authorization,
duplicate identity/port, partial group, partial recursive operation, stale
predicate deletion, malformed config, or panic.

Run high-contention tests repeatedly:

```bash
for run in $(seq 1 50); do
  cargo test --release -p trop --test concurrent_operations --locked ||
    exit 1
  cargo test --release -p trop --test race_conditions --locked ||
    exit 1
done
```

Adapt test target names if the suite is reorganized, but preserve the 50 clean
runs as evidence. Any flake is a failed gate until explained and fixed.

### 9.2 Lock timeout

Hold an external `BEGIN IMMEDIATE` transaction, invoke a mutating command with
a one-second busy timeout, and measure elapsed time.

Expected behavior:

- it waits approximately the configured interval, within a documented
  scheduler tolerance;
- exits with the specified timeout code (`2` in the current specification);
- prints one actionable error to stderr and no normal stdout;
- performs no mutation;
- does not perform hidden retry loops after SQLite's timeout;
- succeeds normally after the lock is released.

Repeat through CLI, environment, and config timeout sources.

### 9.3 Crash and I/O fault injection

Provide test-only failpoints immediately before and after significant write
steps and commit boundaries. For each mutating operation, terminate the process
at each failpoint and reopen the database/config with a fresh process.

Test:

- process exit and forced termination before commit;
- termination during commit/WAL activity;
- termination immediately after successful commit but before output;
- short write, fsync failure, rename failure, permission loss, read-only
  filesystem, and disk-full behavior where the test environment supports them;
- abrupt machine/container stop with persisted volume for representative
  reserve, cleanup, group, migration, init, and config-write cases.

The resulting state must be one of the documented atomic outcomes, pass
physical and logical integrity checks, and remain usable. If a commit succeeds
but output is lost, rerunning the idempotent command must safely reveal the
committed result.

## 10. Destructive-operation safety

For every scenario, snapshot the complete database/config before the command,
run it, and compare the full state afterward.

### 10.1 Release

Seed exact-path untagged and multiple tagged reservations plus parent, child,
and unrelated paths. Verify:

- default exact release removes every tag at exactly that path and nothing
  else;
- `--tag` removes only the selected tag;
- `--untagged-only` removes only the untagged row;
- recursive variants apply the same selector to descendants;
- missing targets are successful idempotent no-ops;
- unrelated sideways paths are rejected without an explicit override;
- `--force` and `--allow-unrelated-path` have documented behavior;
- recursive deletion is all-or-nothing under injected failure;
- dry-run reports the exact rows that a real run would remove and changes
  neither rows nor timestamps.

### 10.2 Prune, expire, and autoclean

Seed existing, nonexistent, expired, fresh, and overlapping
nonexistent-and-expired reservations. Verify:

- prune deletes only paths definitively reported as absent;
- permission denied, symlink loop, transient I/O error, and undecodable path
  preserve the reservation and report a warning/error;
- expire uses one captured notion of “now” and correct boundary semantics;
- zero, negative, overflowing, and malformed day values are rejected before
  mutation;
- autoclean deduplicates candidates and reports each row once;
- dry-run count and row set exactly match a subsequent real run from the same
  snapshot;
- cleanup selection and deletion are one atomic operation;
- reserve refresh racing with expire cannot delete the refreshed row;
- auto-cleanup on range exhaustion rolls back if allocation later fails.

### 10.3 Migrate

Seed single, tagged, recursive, overlapping, and conflicting path trees.
Verify:

- destination existence is required;
- missing nonrecursive source fails, while an empty recursive source follows
  the specified no-op behavior;
- all tags migrate;
- paths are transformed by filesystem components, not string prefixes;
- destination conflicts fail without mutation;
- `--force` overwrites only the documented conflicts;
- ancestor-to-descendant, descendant-to-ancestor, and overlapping mappings do
  not collide with intermediate rows;
- source enumeration and destination conflict checks are protected against
  concurrent changes;
- injected failure leaves the entire original tree or entire migrated tree;
- dry-run describes exactly the eventual transformation without mutation.

### 10.4 Init and configuration writers

Verify that `init`, overwrite, exclusions, compaction, and autoexclude:

- cannot target an unintended data/config location;
- preserve the original on validation or I/O failure;
- handle existing database plus WAL/SHM files coherently;
- do not lose unrelated user configuration;
- never leave a zero-length or partially serialized file;
- have an accurate dry-run;
- are safe under concurrent invocation.

## 11. CLI contract, output, and exit codes

Build a black-box table for every subcommand and global option. For success,
semantic negative result, invalid input, missing data directory, lock timeout,
configuration error, database corruption, permission failure, range
exhaustion, and unexpected I/O failure, assert:

- exact exit-code class;
- stdout content;
- stderr content;
- whether state changed;
- quiet and verbose behavior.

At minimum verify the current specified codes:

- `0`: success;
- `1`: semantic negative result for assertion/validation-style commands;
- `2`: busy timeout;
- `3`: missing data directory with auto-init disabled;
- `4+`: distinct documented internal/error classes.

No internal error may be converted into semantic success by `--not`.
Errors go exclusively to stderr. Machine-readable stdout must contain no
warnings, logs, progress text, or human commentary.

### 11.1 Output formats

For `list`, group output, scan, and other structured commands:

- parse JSON with an independent parser and validate its documented schema;
- parse CSV/TSV with a standards-compliant parser, including fields with
  delimiters, quotes, Unicode, and newlines;
- verify deterministic record ordering or explicitly document that order is
  not stable;
- snapshot all shell formats and parse them as described in the security
  section;
- verify table output at narrow/wide terminals and with long paths;
- test empty, one-row, and many-row results;
- verify timestamps, nulls, paths, tags, project/task, and occupancy fields;
- confirm `--quiet` never suppresses required machine output and `--verbose`
  never contaminates stdout.

### 11.2 Help, logging, and dry-run

Check:

- running `trop` without a command produces the documented help behavior;
- every option appears on the appropriate command and nowhere misleading;
- environment variable names in help match actual parsing;
- `TROP_LOG_MODE`, `--quiet`, and `--verbose` have defined precedence;
- logging is initialized once and all logs use stderr;
- no debug traces appear in release output by default;
- every mutating command's dry-run builds and validates the real plan,
  discovers exhaustion/conflicts, and reports exact changes without opening a
  write transaction or changing state.

For each dry-run, compare its normalized action list with an immediately
following real run from the same snapshot.

## 12. Cross-platform support and MSRV

### 12.1 Native platform tests

Run all baseline and black-box suites natively on Linux, macOS, and Windows.
Pay special attention to:

- drive letters, UNC paths, separators, case behavior, and reserved Windows
  path forms;
- symlink/junction behavior and permissions;
- non-UTF-8 Unix paths and Unicode Windows paths;
- IPv4/IPv6 dual-stack differences;
- file replacement and SQLite WAL locking;
- process termination and busy-timeout timing;
- Bash/Zsh/Fish availability on Unix and PowerShell syntax on Windows;
- newline conventions in dotenv, CSV/TSV, man pages, and completions.

Do not hide platform-specific test failures behind unconditional `#[ignore]`.
Conditional skips must identify a capability check and be reported.

### 12.2 Minimum supported Rust version

The manifests, README, CI, and release policy must state one MSRV. Determine it
empirically from a clean dependency resolution compatible with that policy:

```bash
cargo +<msrv> check --workspace --all-targets --all-features --locked
cargo +<msrv> test --workspace --all-targets --all-features --locked
```

Also test the pinned development toolchain and latest stable. A release fails
if the declared MSRV cannot build the locked graph, if dependencies require a
newer compiler, or if CI does not continuously enforce the declaration.

Confirm the Rust edition in both crates matches the specification or an
approved update.

## 13. Dependencies and supply-chain integrity

Run:

```bash
cargo audit
cargo deny check
cargo tree --workspace --all-features --duplicates
cargo outdated --workspace
```

Record the advisory database revision and tool versions. Requirements:

- no unmitigated vulnerability, unsoundness advisory, or denied license;
- every warning has a documented disposition and owner;
- deprecated/unmaintained dependencies have been removed, replaced, or
  explicitly risk-accepted;
- direct dependencies are used and minimal;
- duplicate major versions are understood;
- runtime and development-only reachability are distinguished without
  dismissing development supply-chain risk;
- license attribution is generated from the exact lockfile and checked for
  drift;
- dependency updates and notices can land without circular CI failures.

Review CI and release workflows:

- third-party actions are pinned to immutable commit SHAs;
- installed tools are version-pinned and checksum-verified where practical;
- workflow permissions default to read-only and are elevated per job;
- pull-request workflows cannot access publishing/signing secrets;
- crates.io uses the protected Trusted Publishing workflow approved under #135;
- GitHub environments require appropriate release approval;
- release artifacts are produced from the tagged commit, not rebuilt by hand;
- the build records compiler, lockfile, source, and workflow provenance;
- Dependabot/Renovate and RustSec checks run on ordinary CI or a reliable
  schedule, not only manually.

If reproducible builds are claimed, build the same artifact in two independent
clean environments and compare hashes. If exact reproducibility is not
achieved, document and compare the expected nondeterministic sections rather
than making the claim.

## 14. Performance, load, and soak testing

Correctness under load is mandatory; absolute speed targets must be agreed and
documented before the audit. Record median, p95, p99, maximum, failures, busy
timeouts, CPU, peak memory, database size, and write amplification.

### 14.1 Dataset and concurrency matrix

Measure representative commands with:

- 0, 1, 100, 1,000, 10,000, and 100,000 reservations where practical;
- short and long paths, many tags, and many project/task values;
- sparse and near-exhausted port ranges;
- 1, 8, 32, and 128 concurrent processes;
- scan ranges of 100, the normal default range, and the maximum supported
  practical range;
- fresh database, WAL-populated database, and post-cleanup database.

Include:

- repeated idempotent reserve;
- distinct-key reserve;
- group reserve with small and large offset spans;
- list/filter/output formats;
- prune, expire, and autoclean;
- recursive release and migration;
- config discovery and validation;
- occupancy scan with a controlled fraction of bound ports.

Use release binaries and an external harness for end-to-end latency. Criterion
benchmarks may supplement but not replace process-level measurements:

```bash
cargo bench --workspace --all-features --locked
```

Ensure benchmark inputs cannot be optimized away and verify resulting state
after load. A faster operation that loses updates is a failure.

### 14.2 Soak and contention

Run at least a 24-hour soak on the primary supported platform with concurrent
workers repeatedly reserving, refreshing, grouping, listing, releasing,
cleaning, and migrating disposable paths. Include periodic process termination
and lock contention. Continuously check:

- `PRAGMA quick_check` and periodic `integrity_check`;
- logical database invariants;
- duplicate keys/ports;
- process memory and file-descriptor growth;
- database/WAL growth and checkpoint behavior;
- latency and timeout trends;
- orphan temporary/config files;
- panic, deadlock, starvation, or data-loss events.

At completion, stop writers cleanly, checkpoint, reopen with a new process, run
logical validation, and verify every model invariant. Any unexplained failure
or integrity anomaly is a no-go.

## 15. Packaging and installation

### 15.1 Crate packages

Inspect and verify both packages:

```bash
cargo package -p trop --locked
cargo package -p trop-cli --locked
cargo publish -p trop --dry-run --locked
cargo publish -p trop-cli --dry-run --locked
```

List each `.crate` archive and confirm it contains only intended source and
required metadata. It must include:

- correct Cargo metadata and repository/homepage links;
- both license texts matching the SPDX expression;
- the intended README;
- required generated or source assets;
- no build output, test corpus bloat without justification, credentials,
  machine paths, audit evidence, or private files.

Build and test the packaged source in an offline clean environment after all
dependencies have been fetched. Do not assume a workspace path dependency
makes the independently published CLI installable.

Use the saved Cargo package outputs identified by #149 for the
registry-equivalent test; do not replace them with a newly packed audit
substitute. A fresh `cargo package` run may test reproducibility only when its
normalized file-manifest/content digest is compared with the precommitted
identity rule.

In the disposable local registry or staging environment, publish/install the
saved library version, wait for index availability, then publish/install the
saved CLI version that depends on it. Verify exact version constraints,
publication order, and clean independent installation without production
credentials.

Before and after that rehearsal, query production crates.io and its index as an
unauthenticated user. Retain proof that the exact comprehensive version remains
absent for both packages. If either production package already exists, the
approved pre-publication boundary has been violated and the result is
`NO-GO`; do not continue with a substituted version.

### 15.2 Install, invoke, upgrade, and uninstall

Install beneath an isolated root:

```bash
install_root="$(mktemp -d)"
cargo install --path trop-cli --locked --root "$install_root"
"$install_root/bin/trop" --version
"$install_root/bin/trop" --help
```

Then verify:

- the executable is named `trop`, not the package name;
- first-run auto-init and disabled auto-init work from outside the checkout;
- no runtime dependency on repository files or current working directory;
- an upgrade preserves and migrates an old database safely;
- a downgrade rejects a newer schema without mutation;
- uninstall removes installed program files but never user data;
- reinstall is idempotent.

Repeat with the actual packaged/release artifact, not only `--path`.

### 15.3 Man pages and completions

Generate man pages and completions from the exact Clap command model used by
the binary. Verify:

- every subcommand and option is present with matching spelling/defaults;
- pages are installed into the documented location by each distribution
  channel;
- `man trop` works after installation from outside the checkout;
- generated completion filenames and root commands use `trop`;
- Bash, Zsh, Fish, and PowerShell load the completion without syntax errors;
- representative root, subcommand, option, enum, and path completions work;
- generated files are reproducible or versioned through a documented process.

Do not accept a build-script artifact left only in Cargo `OUT_DIR` as installed
documentation.

### 15.4 Release artifacts, SBOM, and provenance

Use only the target artifacts attached to #149's public prerelease. For every
target artifact:

- unpack it and inventory every file;
- verify executable architecture and minimum OS compatibility;
- run install/smoke/uninstall on the target platform;
- include README/release notes and both licenses;
- require and verify its SHA-256 checksum;
- require and verify its signature with a public documented command;
- require and verify an SPDX or CycloneDX SBOM for the exact lockfile/artifact;
- require build provenance tying artifact hash to commit, workflow, compiler,
  target, and dependency graph;
- verify the checksum, signature, SBOM, and provenance after downloading from
  the release page as an unauthenticated user.

Use a clean unauthenticated session with no repository checkout, GitHub token,
or cached artifact. Verify the identity manifest before executing any
downloaded binary. A missing, inaccessible, or unverifiable checksum,
signature, SBOM, or provenance object for any target is an automatic `NO-GO`,
not a conditional gap.

Ensure the #149 tag, prerelease, saved crate packages, target artifacts,
checksum files, #136 Homebrew formula, changelog, and `trop --version` identify
the same release. Verify separately that this exact version is still absent for
both production crates.

## 16. Release rehearsal, rollback, and operational response

Issue #135 already rehearsed candidate construction and public-prerelease
mechanics in a disposable remote. Do not create a new candidate tag, prerelease,
package, target artifact, or formula during #137. Perform this full rehearsal
without production credentials from #149's retained subject:

1. download the existing public-prerelease assets as an unauthenticated user;
2. verify the identity manifest, hashes, signatures, SBOMs, and provenance;
3. publish the saved `trop` package to the disposable registry, wait for index
   availability, then publish the saved `trop-cli` package;
4. install the saved packages and artifacts on every target platform;
5. upgrade a copy of each prior supported database;
6. execute the smoke and destructive-operation suites;
7. install and test the exact custom-tap formula retained by #136;
8. simulate #150's production publication and existing-prerelease promotion
   from saved outputs without contacting a production write endpoint;
9. compare every candidate-affecting input and output with the #149 identity
   manifest and require no change;
10. simulate announcement and security-advisory metadata; and
11. practice rollback and failed-candidate withdrawal.

The simulation in step 8 must prove that #150 can publish in `trop`,
index-wait, `trop-cli` order and promote the existing public prerelease without
rebuilding, repacking, retagging, replacing assets, or editing source, version,
lockfile, package contents, integrity evidence, README, changelog, Cargo
metadata, or status text. A new disposable candidate or rebuilt substitute does
not satisfy this rehearsal.

The rollback plan must distinguish:

- **binary rollback:** restoring a previous executable;
- **schema rollback:** old binaries must reject newer schemas safely; restore
  from an explicit backup if reverse migration is not supported;
- **crate response:** yanking prevents new broad resolution but does not remove
  exact-version or lockfile installations;
- **GitHub artifacts:** never replace assets under an existing version; publish
  a new version;
- **Homebrew rollback:** update the tap to a known-good immutable artifact and
  checksum;
- **security response:** publish an advisory, affected versions, mitigation,
  and fixed version without relying on repository history edits.

Confirm maintainers can perform each action, have access to necessary accounts,
and know where backups/evidence are located. Document recovery instructions for
users whose database was migrated by a withdrawn release.

## 17. Homebrew custom-tap verification

Audit the exact custom-tap formula whose evidence closed #136. It must consume
the exact #149 public-prerelease asset URL and SHA-256 recorded in the identity
manifest, not a branch archive, rebuilt archive, or substitute version. Do not
generate or switch formulas during #137.

In a clean Homebrew environment run:

```bash
brew tap <owner>/<tap>
brew audit --strict --online <formula>
brew install <formula>
trop --version
brew test <formula>
brew uninstall <formula>
```

Also run `brew test-bot` in the tap's CI and test upgrade from the prior
formula. Verify:

- Apple Silicon macOS and every claimed Homebrew platform;
- formula metadata, license, homepage, and version;
- installed binary, man pages, and completions;
- formula test uses an isolated data directory and proves a real
  reserve/idempotency/list operation;
- uninstall does not delete user reservations;
- formula URL and checksum correspond exactly to #149's public prerelease asset;
- the artifact's signature is independently verifiable;
- no source checkout or network access is needed at runtime.

Record the formula revision, tap PR, asset URL, checksum, and results. Its
pre-`GO` status must be honest: #136 proves candidate consumption, not stable
publication. After #137 records `GO`, #150 must promote that existing candidate
without changing the formula's artifact identity; #151 later owns
post-publication tap verification.

Do not treat inclusion in a custom tap as evidence that Homebrew/core's
notability or policy requirements are met. Consider a core submission only
after production readiness, stable release history, real external usage, and a
fresh review of Homebrew's then-current policy.

## 18. Test-suite adversarial validation

Passing tests can still encode the wrong contract. Before sign-off:

1. Compare each critical test expectation directly with the specification.
2. In disposable worktrees, introduce one controlled mutation for each class:
   - raw shell identifier fallback;
   - reallocate an existing group;
   - skip path normalization;
   - ignore one config precedence layer;
   - ignore a protocol/address-family skip flag;
   - release only untagged rows by default;
   - treat every metadata error as nonexistent;
   - perform cleanup selection outside its transaction;
   - allow duplicate untagged database keys;
   - cast a negative timestamp without checking;
   - map SQLite Busy to a generic exit code.
3. Run the narrow regression test and the relevant black-box suite.
4. Require a clear failure tied to the violated invariant.
5. Discard the worktree and record the mutation, command, and failure output.

Also inspect CI to ensure these tests run on pull requests and release tags
without opt-in flags. An ignored, quarantined, or manual-only regression test
does not satisfy the gate.

## 19. Final report and go/no-go rubric

### Verdict ownership and closing references

Run #137 in a fresh independent session or context. The active remediation
context may provide a factual handoff, but it must not author, review, approve,
sign, or otherwise determine its own verdict. Use an auditor and final reviewer
independent of the remediation sequence wherever practical.

Every audit-preparation, audit-in-progress, `CONDITIONAL NO-GO`, or `NO-GO` PR
must use only `Refs #137`, contain no workflow closing keyword, and expose
`closingIssuesReferences: []` after GitHub indexing. It leaves #137 open. Only
one final, dedicated, independently authored and reviewed report PR may use the
sole `Closes #137`, and only after its committed signed-off report states
exactly `GO` for the exact #149 candidate and contains or links every mandatory
evidence item. That PR must target `main`, close no other workflow issue, and
expose exactly #137 in `closingIssuesReferences` after indexing.

### Automatic no-go conditions

The release is **NO-GO** if any of the following is true:

- the tested commit, version, tag, prerelease, package, artifact, identity
  manifest, or formula is not the exact subject closed by #149 and #136;
- the exact comprehensive version of `trop` or `trop-cli` is already present on
  production crates.io before #150;
- any known critical or high-severity correctness/security finding remains;
- generated shell output can contain an unvalidated identifier or statement;
- single or group reservations are not idempotent;
- a mutating command can partially commit its specified atomic operation;
- a demonstrated race can lose, overwrite, duplicate, or incorrectly delete
  valid reservation/config state;
- a reachable panic/abort exists for runtime-controlled input or stored data;
- logical corruption is accepted as valid;
- cleanup can delete on an ambiguous filesystem error;
- configuration precedence or path identity differs across commands;
- occupancy flags do not control the promised protocol/family/interface;
- a destructive command bypasses its path, force, tag, or dry-run contract;
- an unmitigated dependency vulnerability or unsoundness advisory remains;
- a supported platform, declared MSRV, packaged crate, or release artifact
  fails its required tests;
- release artifacts lack required licenses, checksums, signatures, SBOM, or
  provenance;
- an unauthenticated user cannot download and verify every #149 target artifact
  and all four required integrity objects;
- the #136 formula does not consume the exact #149 asset URL and checksum;
- saved-artifact rehearsal requires a candidate-affecting change or cannot
  prove the exact #150 publication and promotion path;
- migration/rollback from every previously published schema is unverified;
- any required test was skipped without an approved release-scope reduction;
- any traceability row is failed, blocked, or lacks evidence;
- the 24-hour soak produces an unexplained failure, invariant violation,
  integrity error, deadlock, leak, or material performance degradation.

### Conditional no-go

The result is **CONDITIONAL NO-GO** only when no automatic blocker is known and
the remaining items are bounded, non-safety evidence gaps: for example, a
platform is temporarily unavailable, a non-safety risk exception awaits
approval, performance budgets are undefined, or a nondeterministic non-safety
result remains unexplained. Security, data-integrity, destructive-operation,
candidate-identity, packaging-integrity, or other automatic no-go conditions
cannot be conditional. Every condition must name its accountable owner, linked
remediation or evidence action, and a dated near-term re-audit plan.

`CONDITIONAL NO-GO` is not authorization to publish and does not preserve the
candidate for a later resumed audit. Apply the abandonment procedure below and
rerun against a successor candidate.

### Go criteria

The release is **GO** only when:

- every mandatory evidence item is present, and no open P0, P1, or unwaived
  release blocker remains;
- every specification requirement has passing independent evidence;
- every original due-diligence defect has been reproduced as fixed and guarded
  by a regression test that fails under controlled mutation;
- baseline, cross-platform, MSRV, security, supply-chain, migration, packaging,
  installation, Homebrew-tap, load, race, crash, and soak gates all pass;
- the exact #149 assets and mandatory integrity evidence pass unauthenticated
  verification, while both comprehensive production crates remain absent;
- the exact #136 formula consumes #149's asset URL and checksum on every
  claimed platform;
- test runs are repeatable and free of unexplained flakes;
- release and rollback rehearsals succeed from the exact saved candidate
  artifacts and prove #150 requires no candidate-affecting change;
- all findings are closed or are low-severity, explicitly risk-accepted,
  documented, and noncontractual;
- at least one reviewer independent of the remediation work signs the evidence
  bundle and decision.

### Verdict consequences

Every `CONDITIONAL NO-GO` or `NO-GO` permanently abandons the audited version
and tag, even when caused only by missing evidence or platform availability.
Mark the public prerelease honestly as failed or withdrawn. Never move or
delete its tag, replace or delete its assets, or reuse that version. Leave #137
open; reopen #149, #136, #89, and every affected component gate; file and land
the linked remediation or evidence work; and obtain a successor #130 version
decision when needed. The next attempt requires a successor candidate, fresh
tap evidence, reclosed affected gates, and a fresh independent audit.

An exact `GO` closes only #137. It unlocks #150 but does not authorize
production publication. Fresh explicit maintainer approval at #150 remains
mandatory. After that gate publishes the saved packages and promotes the
existing prerelease without a candidate-affecting change, #151 must close with
post-release custom-tap evidence before the program may advance to #83.

No candidate-affecting change may land between `GO` and #150 publication. If
one becomes necessary, stop, invalidate the old `GO`, reopen issue #137,
candidate gate #149, issues #136 and #89, and every affected component gate,
permanently abandon the old version and tag, and repeat the
successor-candidate lifecycle. A prior `GO` cannot authorize changed content.

### Required sign-off block

Include this completed block in the final report:

```text
Audited commit:
#149 candidate URL:
Comprehensive version/tag:
#149 identity manifest and SHA-256:
#136 formula revision, asset URL, and checksum:
Production crate absence evidence:
Audit dates:
Auditor(s):
Independent reviewer:
Platforms:
Specification matrix: PASS / FAIL / INCOMPLETE
Security and dependencies: PASS / FAIL / INCOMPLETE
Correctness and destructive operations: PASS / FAIL / INCOMPLETE
Concurrency, crash, and corruption: PASS / FAIL / INCOMPLETE
Cross-platform and MSRV: PASS / FAIL / INCOMPLETE
Performance and 24-hour soak: PASS / FAIL / INCOMPLETE
Packaging, artifacts, and custom tap: PASS / FAIL / INCOMPLETE
Release and rollback rehearsal: PASS / FAIL / INCOMPLETE
Saved-artifact no-change comparison: PASS / FAIL / INCOMPLETE
Open findings by severity:
Skipped/blocked tests:
Risk acceptances:
Evidence bundle location and SHA-256:
Decision: GO / CONDITIONAL NO-GO / NO-GO
Rationale:
```

Issue closure, code-review approval, CI green status, and a successful
`cargo test` are inputs to this decision. None is a substitute for the complete
evidence-backed audit above.
