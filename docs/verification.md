# Formal verification with Verus

`trop` proves the correctness of its core decision logic with
[Verus](https://github.com/verus-lang/verus), an SMT-based verifier for Rust.
This document explains how the verified code is organized, what is (and is
not) proved, what is trusted, and how to work on it.

## Quick start

```sh
just install-verus   # once: pinned Verus release + the Rust toolchain it needs
just verify          # check every proof
```

`just preflight-pr` runs the proofs when Verus is installed locally, and the
`Verus verification` CI job runs them on every push and pull request. The
`Test` jobs depend on it.

## Architecture

```
trop/src/verified/        verified kernels (Verus `verus! { ... }` blocks)
  mod.rs                  module list + rules for code in this tree
  port.rs, ...            one kernel per concern
trop/src/**               production code; delegates decisions to the kernels
trop-verification/        publish = false harness crate; mounts trop/src/verified
                          with #[path] so `cargo verus verify` checks the
                          exact files that `trop` compiles
```

- **One copy of the code.** The kernels are ordinary modules of the `trop`
  library. In a normal `cargo build`, the `verus!` macro (from the `vstd`
  crate) erases every specification, proof, and ghost value, leaving plain
  Rust. `cargo verus verify` in `trop-verification/` compiles the *same
  files* with the verifier enabled. There is no model-vs-implementation gap
  inside the kernels.
- **Thin glue.** Production code converts its domain types (`Port`,
  `PortRange`, `Path`, configuration structs, database rows, ...) into the
  primitive values the kernels take and back. The glue is deliberately
  trivial so that it can be audited by eye; it is listed per kernel below.
- **Effects stay outside.** SQLite queries, OS occupancy probes, the clock,
  the filesystem, and git discovery are not modeled. Where an algorithm
  interleaves decisions with effects (for example, scanning ports and probing
  each one), the kernel is either generic over an oracle closure or proves
  the pure parts (bounds, candidate order, minimality given the oracle's
  answers).

## Soundness at the verified/unverified boundary

Verus checks callers *inside* `verus!`, but production callers are
unverified. Every kernel entry point therefore follows one of two rules:

1. **Total functions.** No `requires` clause. The specification covers every
   possible input, including values production code should never pass (for
   example, port `0`).
2. **Invariant-carrying types.** A kernel type with private fields and a
   `#[verifier::type_invariant]`, constructible only through verified
   constructors (for example `PortSpan`, which guarantees
   `1 <= min <= max`). Rust's privacy rules enforce the invariant for
   unverified code too, so methods on the type may rely on it.
3. **Oracle closures.** Kernels that interleave decisions with effects
   (database lookups, OS probes, filesystem checks) are generic over `Fn`
   closures supplied by production code. Their only `requires` clause states
   the closure's own precondition (`forall|x| f.requires((x,))`), and every
   result is specified relative to the answers the closure gave
   (`f.ensures`). Closures written in ordinary Rust have no Verus
   precondition, so this holds trivially; that the closures do not panic is
   part of the trusted glue.

## Trusted computing base

A verified property holds for the shipped binary if the following are
correct; everything else in a kernel is checked:

- the Verus verifier and the Z3 SMT solver (release pinned in
  `scripts/install-verus.sh`; `vstd` pinned in the workspace `Cargo.toml`);
- `vstd`'s specifications of the `core`/`std` operations a kernel calls
  (for example `u16::checked_add`);
- the `verus!` macro's erasure (it removes ghost code only);
- the production glue that converts domain types to kernel inputs and back;
- the oracle closures production code passes to oracle-generic kernels (they
  must be total; the kernels specify their results relative to the answers
  the closures give, so the closures' correctness is not assumed);
- the `std`/`core` behavior that glue relies on and kernels do not model,
  pinned by tests where noted per kernel (for example `str::trim` and
  `std::path` component splitting);
- every `assume_specification`, `#[verifier::external_body]`, `assume`, or
  `admit` listed in the table below (currently: none).

| Escape hatch | Location | Justification |
| --- | --- | --- |
| *(none)* | | |

## What is verified

### `verified::port` — port numbers and ranges

Glue: `trop/src/port.rs` (`Port`, `PortRange`, `PortRangeIter`).

- `port_is_valid`: exactly `1..=65535` is valid. `Port::try_from(u16)` and
  `Port`'s `Deserialize` impl (via `#[serde(try_from = "u16")]`) both use it,
  so no `Port` holds `0`.
- `port_checked_add` / `port_checked_sub`: return `port ± offset` exactly when
  the result is a valid port, `None` otherwise (no wrap-around, no port 0).
- `port_is_privileged`: `port < 1024`.
- `PortSpan` (stored inside every `PortRange`): type invariant
  `1 <= min <= max`; `new` succeeds exactly for valid ranges; `contains` is
  inclusive on both ends and agrees with membership in the span's port
  sequence; `len` equals `max - min + 1`, never overflows `u16`, and is at
  least 1; `is_empty` is always `false`.
- `PortCursor` (inside `PortRangeIter`): yields exactly the ascending sequence
  `min, min + 1, ..., max` — every port once, in order — then `None` forever;
  `remaining_len` (the iterator's exact `size_hint`) equals the number of
  ports left.

### `verified::exclusions` — exclusion entries, the excluded-port set, compaction, and exclusion text

Glue: `trop/src/port/exclusions.rs` (`ExclusionManager`, `validate_exclusions`,
`compact_exclusions`), `trop/src/config/validator.rs`
(`validate_excluded_ports`), `trop/src/config/schema.rs` (`PortExclusion`'s
`FromStr`, used by the YAML string form), `trop/src/config/environment.rs`
(`TROP_EXCLUDED_PORTS`), `trop-cli/src/commands/exclude.rs`, and
`trop-cli/src/commands/compact_exclusions.rs` (also reached by
`scan --autocompact`).

- `validate_entry` / `validate_entries`: an entry is valid exactly when it
  names a non-empty set of valid ports (`lemma_entry_valid_meaning`): a port
  `>= 1`, or a range with `1 <= start <= end` (`5000..5000` is valid and
  compacts to `5000`, `lemma_one_port_range`). Errors follow the validator's
  precedence (zero start, then zero end, then reversed); a list reports its
  first invalid index.
- `compact_exclusions`: the result excludes exactly the same ports and is
  canonical (sorted, every two entries separated by at least one
  non-excluded port, `Single` exactly for one-port runs). It is no longer
  than any list — valid or not — with the same coverage
  (`lemma_canonical_minimal`), and it is the only canonical list with that
  coverage (`lemma_canonical_unique`), so it does not depend on entry order,
  duplicates, or overlaps, and compacting it again returns it unchanged.
  Invalid input is rejected, never dropped.
- `ExclusionSet` (inside `ExclusionManager`): type invariant "the run list is
  canonical"; its view is the set of valid ports covered, so port 0 is never
  a member. `from_entries` builds exactly the covered set or reports the
  first invalid entry; `contains` (binary search) is exact;
  `insert_port` / `insert_span` / `insert_entry` add exactly the entry's
  ports, and an invalid entry leaves the set unchanged; `members_between`
  returns exactly the members in `lo..=hi`, strictly ascending; `len` is the
  set's cardinality (at most 65535, no overflow); `runs` is the set's unique
  canonical list.
- `parse_u16` accepts exactly the syntax of `u16::from_str` (optional `+`,
  one or more ASCII digits, value at most 65535) for any bytes, without
  overflow; `find_range_separator` finds the first `..` (where
  `split_once("..")` splits). `lemma_decimal_text` and
  `lemma_range_text_roundtrip`: a port's decimal text, and `"start..end"`,
  split and parse back to the same values. Trimming (Unicode whitespace) is
  `str::trim` in the glue.

Glue: entries are copied field for field between `PortExclusion` and
`ExclusionEntry`; kernel errors map to the validator's `excluded_ports[i]`
messages, or to the `InvalidPort` / `InvalidPortRange` errors
`ExclusionManager` always reported; `excluded_in_range` and `iter` wrap the
kernel's (valid) members as `Port`; `add_range` passes the range's
`PortSpan`; `PortExclusion::from_str` splits at the kernel's separator index,
trims each side with `str::trim`, and parses each side with `parse_u16`.

### `verified::port_bounds` — configured port-range bounds

Glue: `trop/src/port/allocator.rs` (`port_range_from_config`, used by
`allocator_from_config`), `trop-cli/src/commands/scan.rs`
(`ScanCommand::determine_range`).

- `resolve_port_bounds` (resolution, used by the allocator and `trop scan`):
  succeeds exactly when the requested upper bound forms a valid range with
  `min`. The requested bound is the explicit `max`, otherwise
  `min + max_offset` in unbounded arithmetic. On success it returns exactly
  `min..=that bound`, with no `u16` overflow. Errors follow
  `allocator_from_config`'s historical order: `InvalidMin`; then, for an
  explicit `max`, `InvalidMax` before `MaxBelowMin`; otherwise
  `OffsetOverflow` or `MissingMax`. The glue maps each error to the
  historical `Error` value, and a differential test against the previous
  code checks this. Resolution is lenient: an explicit `max` wins over
  `max_offset`, and `max_offset == 0` gives `min..=min`. `trop scan` now
  resolves through it; it used to ignore `max_offset`.
- `check_port_bounds` (configuration-file rules; written for
  `ConfigValidator::validate_port_config` to adopt): succeeds exactly for
  strictly valid bounds. That means a valid `min`; an explicit `max` that is
  valid and `>= min`; a `max_offset >= 1` that lands on a valid port; and not
  both `max` and `max_offset`. It returns the named range, or `None` when
  neither bound is given, which validation allows. Violations follow the
  validator's order: `InvalidMin`, `InvalidMax`, `MaxBelowMin`, `ZeroOffset`,
  `OffsetOverflow`, `BothMaxAndOffset`. A differential test checks the kernel
  against the current validator.
- Lemmas:
  - Each error table is consistent with its semantics.
  - The validator's "offset would create invalid max port" branch is
    unreachable.
  - Every validated configuration that names a bound resolves to the same
    range. One that names neither fails resolution only with `MissingMax`.
  - Resolution accepts exactly two shapes that validation rejects: both
    `max` and `max_offset` are given, or `max_offset == 0`.

### `verified::allocation` — availability classification and allocation scans

Glue: `trop/src/port/allocator.rs`:
- `PortAllocator::{port_availability, allocate_single_ignoring,
  find_next_available_ignoring, find_next_allocatable, exhaustion_blockers}`;
- `PortAvailability`, an alias of the kernel's `Availability`;
- `unavailable_reason`;
- the oracle closures `reservation_oracle` (database: reserved by a key
  other than the one being replaced), `exclusion_oracle` and
  `occupancy_oracle`.

Every result is specified relative to the oracles' answers
(`oracle.ensures`). The database and the OS are not modeled.

- `classify_port` (`classified`):
  - A port outside the scope (port 0, or outside the range when the range is
    required) is `Excluded`, and no oracle is consulted.
  - `Reserved` requires the reservation oracle to answer "reserved". No
    flag bypasses this.
  - `Excluded` requires "not reserved" and a hit on the exclusion list,
    unless exclusions are ignored.
  - `Occupied` requires both earlier checks to pass and the probe to answer
    "occupied", unless occupancy is ignored.
  - `Available` requires every applicable check to answer "free" (fail
    closed).
  - An error is exactly a reservation-oracle error, or a probe error after
    the earlier checks passed. A probe failure therefore never surfaces for
    a reserved or excluded port, or when the probe is ignored.
- `first_available` (forward scan, `scanned`): a returned port lies in
  `start..=max` and in the range, was classified `Available`, and every
  earlier candidate was classified unavailable. `None` means every candidate
  was. A failure names the first port whose classification failed, after
  every earlier candidate was classified unavailable. The cursor never
  overflows, including for ranges that end at 65535.
- `allocate_single` (`allocated`):
  - With a preferred port, the result is `Allocated(p)`,
    `PreferredUnavailable(p, why)` or a failure at `p`, exactly as
    classified under the preferred checks. The range is always required, and
    each ignore flag bypasses only its own check. The result is never
    `Exhausted` and never another port.
  - Without a preferred port, the result is exactly the forward scan from
    the range minimum with every check. It is never `PreferredUnavailable`.
  - An allocated port always lies in the range.
- `exhaustion_blockers`: classifies every port of the range in ascending
  order. A flag is set only if some port was classified that way, and every
  port's classification is reflected in the flags. If no flag is set, some
  port was found available. Failures propagate.
- `Availability::blocker` (behind `unavailable_reason`) is `None` exactly
  for `Available`, so the `expect` in `group.rs` is unreachable.
- The kernels consult an oracle only at a valid port (`1..=65535`): their
  precondition grants the oracles' preconditions nowhere else. This makes
  the glue's `Port` reconstruction sound.
- Lemmas, assuming a consistent snapshot (`deterministic_oracles`):
  - Classification is unique: the specification determines the result,
    including which error.
  - The scan returns the least available port.
  - When the scan reports the range exhausted, the diagnostics set at least
    one blocker flag.
- Further lemmas:
  - Turning an ignore flag on never makes an allocatable preferred port
    unavailable (monotonicity).
  - The flags are independent: with both set (`--force`), an in-range port
    can be blocked only by a reservation.
  - Without flags, the preferred check equals the scan's classification.

Trusted for these kernels:
- The oracle closures are total (no panics) on valid ports. The kernels'
  `requires` states only their preconditions.
- `#[allow(dead_code)]` appears on `check_port_bounds`, which is not yet
  called, and on `ScanFailure::port`, a witness that only the specs read.
  Neither is a verification escape. This Verus version cannot prove
  existential postconditions over a generic error type, so the scans return
  the failing port as a witness.

### `verified::occupancy` — occupancy-check policy

Glue: `trop/src/port/occupancy.rs`: `OccupancyCheckConfig::from(&OccupancyConfig)` and the `flags`/`from_flags` field copies; `OccupancyProbe::new` (kernel `Probe` to `127.0.0.1` / `0.0.0.0` / `::1` / `::` plus the port); `SystemOccupancyChecker::report_with` (classifies each bind `io::Result`: `Ok` is `Free`, `AddrInUse` is `InUse`, any other error is `Failed(error)`, and the `FnMut` runner is wrapped in a `RefCell`); `scan_range` (`Port` <-> `u16`); `MockOccupancyChecker`; `OccupancyOwner::refresh_status`. Socket binds, error-kind classification, and owner lookups stay outside the kernel.

- `effective_occupancy_flags`: each skip flag is on exactly when the global `skip` or its own setting is `Some(true)`. `None` and `Some(false)` both mean off, so an explicit `false` never re-enables a probe another switch disabled. `check_all_interfaces` is on exactly when set to `Some(true)`. End to end, a probe is enabled iff no covering skip switch (global, its protocol, its family) is on and it is a localhost probe or `check_all_interfaces` is on, and `skip: true` disables every probe.
- `probe_matrix`: exactly the enabled protocol x family x scope probes, each once, strictly increasing in the documented order (IPv4 before IPv6, then localhost before wildcard, then TCP before UDP). Its length is |protocols| x |families| x |scopes| <= 8. Lemmas: this characterization determines the sequence uniquely; unset settings give exactly TCP/UDP x IPv4/IPv6 on localhost; `check_all_interfaces` keeps every probe and only adds wildcard probes.
- `occupancy_disabled` (the mock checker's guard): true exactly when no probe is enabled, i.e. when the system checker's matrix is empty and its report `was_skipped()`.
- `run_probe_matrix` (generic over the probe oracle): on success, every enabled probe ran and completed, `enabled` equals the matrix length, and the evidence is exactly the enabled probes observed in use, in matrix order. So a report is occupied iff some probe was in use, and an empty matrix means skipped and not occupied. On failure, the returned probe is enabled, its observed outcome was `Failed(error)`, and every enabled probe before it in matrix order completed. Only probe failures produce errors, and collected evidence never hides the first one.
- `scan_ports` (the default `find_occupied_ports`, the mock's override, and `find_occupied_reports`): inspects `lo..=hi` in ascending order. On success it returns exactly the kept values in ascending port order. On error, the error is the first failing port's, and every earlier port was inspected without error. An empty range inspects nothing.
- `owner_status`: `Available` iff PID, process name, and user are all known; `Unavailable` iff none is; `Partial` otherwise.

Not proved: that no probe or port is inspected after the first failure (Verus specifies `Fn` closures by their results, not by the absence of calls; tests cover it), and the glue mappings above (covered by exhaustive tests over all 729 settings and all 32 flag combinations).

### `verified::group` — group requests, offset-pattern search, fresh plans, stored groups

Glue: `trop/src/port/group.rs` (`GroupAllocationRequest::normalized`,
`PortAllocator::{find_pattern_match, allocate_fresh_group,
compatible_existing_group, plan_reconciliation}`),
`trop/src/operations/reserve_group.rs` (`build_group_request`,
`config_with_group_base_as_scan_start`), `trop/src/operations/autoreserve.rs`
(config selection). The glue trims strings, sorts services by tag, supplies the
probe closures (database reservations, exclusions, OS occupancy), maps errors
to messages, and performs the database writes.

- `validate_group_request` / `ValidatedGroupShape`: a request is accepted
  exactly when it is non-empty, project and task (when given) are valid
  identifiers (1 to 255 bytes, no NUL), every tag is a valid identifier, every
  service has an offset or a preferred port (never port 0), and tags, offsets,
  and preferred ports are pairwise distinct. Services are kept unchanged and in
  order. Otherwise the first error is reported in production order: no
  services, project, task, then the first service with an issue (tag rule,
  duplicate tag, missing strategy, preferred 0, duplicate offset, duplicate
  preferred). `ValidatedGroupShape`'s type invariant is that validity
  predicate, and validation is idempotent.
- `pattern_scan_end` / `find_pattern_base`: the lowest base `b >= min` at which
  every `b + offset` is in range, not a pinned port, and reported available by
  the probe. Every lower base was rejected. `None` means every base at all
  (not just the scanned ones) is rejected, because the early stop at
  `max - max(pattern)` is sound. The probe is only ever called on in-range,
  non-avoided ports, a probe error is returned unchanged, and no `u16`
  arithmetic overflows. The search is generic over the probe closure, and its
  guarantees are stated in terms of the answers the probe gave, so no stable
  oracle has to be trusted.
- `plan_fresh_group`: a service is pinned to its preferred port only if the
  preferred probe reported it available (whole port domain, no range check).
  Every other service legitimately falls back to its offset and gets
  `base + offset` inside the scan range, reported available by the scan
  probe. All ports are valid (`>= 1`) and pairwise distinct, the base is the
  lowest that works, and there is no base exactly when every service is
  pinned. The errors are precise: a preferred-only service whose preferred
  port is unavailable, complete pattern exhaustion (every base rejected), or a
  probe error from a preferred port or an in-range scan port.
- `match_stored_tags`: stored rows must be tagged with distinct tags and carry
  exactly the requested tag set. The returned matching is a bijection between
  requested services and stored rows. The first untagged or duplicate row is
  reported first.
- `derive_compatible_base`: sound (every service is on its preferred port or
  at `base + offset` with base and port in the scan range), complete (if any
  base fits, the group is accepted), and unique (the reported base is the only
  one that fits). Errors name the first failing service in tag order. A
  preferred-only service off its preferred port is a conflict, so `--force`
  can replace the group.
- `lemma_fresh_plan_is_compatible`: every fresh plan passes
  `derive_compatible_base` under the same range, with the same ports and base.
  Repeating `reserve-group`/`autoreserve` therefore refreshes and never
  conflicts.
- `decide_group_reconciliation`: no stored rows means allocate fresh. A
  compatible group is refreshed. Only `--force` replaces a group, and only one
  that conflicts. Other errors are never overridden.
- `group_scan_bounds`: with `reservations.base`, the scan runs from the base to
  the configured maximum (`max`, else `min + max_offset` without overflow,
  always computed from the configured `min`). The base must lie in the
  configured range, and rebasing only narrows the range.
- `select_highest_precedence` (last maximum, like `Iterator::max_by_key`),
  `first_present`, and `config_service_offset` (omitted offsets default to 0).
  Together with `lemma_offsets_exclude_missing_fallbacks`, this makes the
  preferred-only failure paths unreachable from configuration.

### `verified::sticky` — sticky project/task metadata

Glue: `trop/src/port/group.rs` (`plan_metadata_refresh`,
`replacement_metadata`). `can_change_field` states the single-reservation rule
of `trop/src/operations/reserve.rs`, so that path can delegate to it.

- `can_change_field`: a change is allowed exactly with `force`, with the
  field's own permission, or when the value does not change.
- `reconcile_sticky_field`: omitting a value keeps the stored one. An explicit
  value is accepted exactly when `can_change_field` allows it. Without
  permission the stored value never changes, and clearing is never implicit.
- `plan_group_metadata`: every stored row is checked, project before task,
  before any write. Either every row's resolution is returned, or the first
  violation is reported and nothing is planned.
- `replacement_metadata`: a forced shape replacement uses the explicit value,
  or the value every stored row agrees on. Otherwise it reports the first
  disagreeing row. It never invents or mixes metadata.

### `verified::identifier` — whitespace trimming, the identifier rule, stored tags

Glue: `trop/src/identifier.rs` (`trim_identifier`, `check_identifier`, `is_canonical_identifier`, `identifier_error_message`). Callers:
- `ReservationKey::new` trims and rejects blank tags. It is lenient: it is the constructor for stored rows, migration and lookups.
- `ReservationKey::new_validated` applies the identifier rule. `trop reserve` uses it.
- `ReservationBuilder::project` and `task` trim; `build` requires the canonical form.
- `ConfigValidator::validate_identifier` and `validate_runtime_identifier` apply the rule, and `validate_reservation_group` trims each tag once, for uniqueness and for env-name derivation.

Properties:
- `trimmed(s)` specifies `str::trim`: drop leading, then trailing, Unicode `White_Space` chars. `is_ws` lists the 25 code points and is proved equal to `vstd`'s model of `char::is_whitespace`.
- `trim(&str) -> &str` returns exactly `trimmed(s)` as a sub-slice of its input. Its `split_at` cuts are proved to fall on char boundaries, so it cannot panic.
- Trim bounds characterize the result: everything outside `s[a..b]` is whitespace, `s[a]` and `s[b-1]` are not, and the bounds are unique when the result is non-empty. Trimming is idempotent and ignores whitespace padding, and the result is empty exactly when `s` is all whitespace.
- `check_identifier` is exactly `identifier_verdict`: `Blank`, then `ContainsNul`, then `TooLong` (more than 255 UTF-8 bytes of the trimmed value), otherwise `Ok(trimmed)`. An accepted value is canonical, re-validates to the same result, and contains no NUL anywhere. NUL is never whitespace, so the trimmed value contains NUL exactly when the raw value does.
- `is_canonical_identifier` decides `is_canonical(s)`: `s` is non-empty and `trimmed(s) == s`, equivalently neither its first nor its last char is whitespace. `ReservationBuilder::build` decides with it, so every built reservation has the form the stored-row readers require.
- Stored-tag sentinel (specification only). Schema v2 stores `None` as `""`. Decoding inverts encoding for every valid tag, the encoding is injective (a tagged key never aliases the untagged key), and every tag accepted by `check_identifier` round-trips. No exec kernel code encodes or decodes stored tags yet: `database/` does it unverified. Because `ReservationKey` has public fields and derives `Deserialize`, the shipped write path does not enforce `is_valid_tag`.

### `verified::env_name` — portable environment-variable names

Glue: `trop/src/identifier.rs` (`EnvironmentVariableName::{parse, resolve, collision_key}`, plus `derive_from_tag` in tests). The validator calls `resolve` with the trimmed tag.

- `check_env_name` is exactly `env_name_verdict` over the name's UTF-8 bytes: `Empty`, then `TooLong` (more than 255 bytes), then `BadFirst`, then `BadRest`. It accepts exactly `[A-Za-z_][A-Za-z0-9_]*` of 1 to 255 bytes.
- Shell-inertness: a portable name is pure ASCII `[0-9A-Za-z_]`. It contains no control byte, space, quote, backtick, `$ = ; & | < > ( ) { } [ ] * ? # \ ! ~ % ^ , . : @ + -`, and no non-ASCII byte.
- `derive_env_name` returns a name exactly when the byte-wise derivation (`-` to `_`, `a-z` to `A-Z`) is portable, and then returns those bytes. Tags with non-ASCII text never derive, so the old `is_ascii` pre-check was redundant.
- Derivation preserves length, removes lowercase and `-`, and is idempotent. Two tags derive the same name iff they agree byte-wise up to ASCII case and `-`/`_`.
- `resolve_env_name`: an explicit mapping is validated on its own and returned verbatim, never falling back to the tag; otherwise the name is derived. Every resolved name is portable.
- `env_collision_key` is the ASCII uppercase of the name. For portable names the key is portable, and two keys are equal exactly when the names are equal ignoring ASCII case.

### `verified::text` — ASCII and UTF-8 helpers

Used by `identifier` and `env_name`; no direct production glue.

- ASCII case folding over bytes and over chars is idempotent, and folded keys are equal exactly under ASCII-case-insensitive equality.
- ASCII bytes and chars coincide: ASCII bytes round-trip through UTF-8, and an all-ASCII encoding means ASCII chars, one byte per char.
- Every char prefix ends on a UTF-8 char boundary, and prefix byte offsets are injective. `char_slice` (kernel-internal) slices a `&str` at proved char offsets.

Trusted-base additions (no new escape hatches): `vstd`'s specs of `char::is_whitespace`, `char::len_utf8`, `str::chars` / `Iterator::next`, `str::as_bytes`, `str::split_at`, `String::push` and `char::to_ascii_uppercase`.
- `char::is_whitespace` and `char::len_utf8` are checked against the kernel model for all 1,112,064 scalar values by `identifier::tests::kernel_whitespace_model_matches_std_for_every_scalar_value`.
- The trim and the identifier rule are differential-tested against `str::trim` and the pre-kernel implementation, and the env-name grammar and derivation against independent pre-kernel oracles, by the property tests in `trop/src/identifier.rs`.
- Glue-level trust: the choice of `ReservationKey::new` (lenient) versus `new_validated` (rule) at each call site.

### `verified::path` — lexical path normalization

Glue: `trop/src/path/normalize.rs` (`expand_tilde`, `resolve_components`,
`normalize`), `trop/src/path/types.rs` (`NormalizedPath::new`),
`trop/src/path/canonicalize.rs` (`canonicalize_existing`),
`trop/src/path/resolver.rs` (`PathResolver::resolve`), and the helpers
`component_kind` / `is_canonical_absolute` in `trop/src/path/mod.rs`.

- `resolve_component_kinds` (all platforms; `resolve_components`): computes
  `spec_resolve`, a stack over component kinds. `.` is dropped, roots,
  prefixes and names are pushed, and `..` pops only a named component;
  anything else is an error ("escapes root", unchanged). The kept indices
  are strictly increasing, select only anchors and names, and include every
  root/prefix (absolute stays absolute; the old "re-push the root" fallback
  was dead). Resolution is idempotent.
- `normalize_unix` (Unix `normalize`): computes `spec_normalize(p, cwd)`:
  take the segments of `p` (or of `cwd/p` for relative `p`), resolve `..`,
  and render the result as `/` or `/a/b`. It is total. Proved:
  - every result is canonical (`spec_is_canonical_abs`), and its segments
    are the resolved segments of the input;
  - normalization is idempotent, even across working directories;
  - absolute input ignores the working directory;
  - no byte other than `/` is introduced, so a NUL appears only if the input
    had one, and backslash-free input yields the backslash-free portable form.
- `is_canonical_abs_path` (Unix `NormalizedPath::new`): exactly
  `spec_is_canonical_abs`: `/`, or `/` followed by segments that are
  non-empty and not `.` or `..` (optionally also free of `\`). With `\`
  allowed it accepts exactly the fixed points of normalization
  (`lemma_canonical_iff_fixpoint`). On canonical strings, byte equality is
  segment equality (`lemma_canonical_injective`).
- `classify_tilde` / `tilde_join` (`expand_tilde`): an exact classification
  of a leading `~`.
  - A home-relative path skips every separator after `~`, so `~//etc` stays
    under the home directory.
  - `\` is a separator only on Windows; on Unix `~\x` is a literal name.
  - The expansion's segments are the home directory's segments followed by
    the rest's (`lemma_tilde_join_segments`).
  - On Windows the glue also rejects a rest that `std` parses as starting
    with a drive prefix (`~/C:x`), which `join` would otherwise use to
    replace the home directory.
- `deepest_existing_prefix` (`canonicalize_existing`): works over an
  `exists` oracle and walks up one named component at a time. It returns the
  deepest prefix observed to exist; every longer prefix was observed absent,
  and the remainder consists of named components only. Its oracle
  precondition covers only prefixes followed by named components, so it
  provably never probes above a root, prefix, `.` or `..`.
- `resolve_by_provenance` (`PathResolver::resolve`): explicit paths are only
  normalized. The oracle precondition is `implicit ==> canonicalize.requires(())`,
  so the explicit branch provably never calls the canonicalization oracle.
  Implicit paths yield the oracle's canonical path or its error, never a
  silent fallback. They warn exactly when the error is "not found" and
  warnings are enabled.

### `verified::path_relation` — path relationships, component prefixes, rebasing

Glue: `trop/src/path/relationship.rs` (`PathRelationship::between`,
`is_within`, `contains`, and the component tokenization).

- `path_relationship` computes `spec_between`; `path_is_within` and
  `path_contains` compute `between_within` / `between_contains`.
  - Both paths are first resolved lexically (`spec_lex_resolve`): `.` is
    dropped and `..` cancels a preceding name. A `..` with nothing to cancel
    (at the root, at the start of a relative path, after a drive prefix) is
    kept as part of the path's *head*, with its root or prefix. A `..` is
    kept exactly when `resolve_components` would fail
    (`lemma_lex_resolve_keeps_parent_iff_strict_fails`), and the two
    resolutions agree whenever it succeeds.
  - A resolved path is its head followed by names
    (`lemma_lex_resolved_shape`). Paths with different heads are
    `Unrelated`: relative vs rooted (including the empty path), different
    prefixes, or a different number of unresolvable `..` (so `../..` vs `..`
    and `/..` vs `/` fail closed). Otherwise `spec_rel` decides: `Same` when
    equal, `Ancestor`/`Descendant` for a strict prefix, else `Unrelated`.
  - Proved:
    - the relation is reflexive and flip-symmetric, and `Ancestor` is
      transitive;
    - `Same` holds exactly when the resolved paths are equal;
    - a relative path is never related to an anchored one;
    - `Ancestor`/`Descendant` mean that the deeper path extends the
      shallower by named components only;
    - within/contains are converses and form a partial order up to
      resolution.
- The `spec_rel` algebra is proved for any element type: exactly one case
  holds, flip, reflexivity, transitivity, within/contains as the prefix
  order, and appending segments gives an ancestor.
- Canonical strings: `spec_component_prefix` (a byte prefix that ends at a
  `/` boundary, so `/a/b` is not a component prefix of `/a/bc`) is
  equivalent to segment prefix. Segment prefix implies byte prefix, so a
  byte-prefix prefilter is complete. `/` is a component prefix of every
  canonical path.
- `spec_rebase` (moving a subtree between ancestors): it round-trips, is
  injective, preserves `spec_rel`, and preserves canonical form.

Trusted (in addition to the base): the `std::path` model.
- On Unix, for an absolute byte string `s`, `Path::components()` is the root
  followed by `segs(s)`. (A relative `./a` also starts with a `CurDir`,
  which `segs` drops; the glue relies on the model for absolute strings
  only.) Pushing components rebuilds the path, and `PathBuf::push` of a
  relative remainder is `spec_tilde_join`.
- Roots and prefixes come only as a leading run, and `PathBuf::pop` removes
  a trailing name. Unmodeled: on Windows, a mid-path name carrying a drive
  prefix followed by `..`, where legacy push/pop behavior may differ.
- Components are keyed by an injective `OsStr` encoding: bytes on Unix,
  UTF-16 units on Windows, `Debug` elsewhere. A Windows prefix is keyed by
  its parsed `Prefix` value.

Differential tests against the replaced implementations pin this model.
Filesystem and environment answers (`exists`, `canonicalize`, `current_dir`,
`home_dir`) are trusted as observed. The oracle closures are total; their
preconditions are conditional, so they also restrict which oracle calls the
kernels may make.

## Working on verified code

- Read the rules at the top of `trop/src/verified/mod.rs`.
- Kernel code is linted by the same `clippy::pedantic` settings as the rest of
  `trop`. Write lint-clean Rust (by-value `self` for small `Copy` types,
  `u32::from` instead of widening `as`, `T::try_from` instead of narrowing
  `as`); Verus supports all of these.
- `rustfmt` does not format the inside of `verus!` blocks. Follow the
  surrounding style (Verus's own `verusfmt` layout).
- In this Verus version, postconditions of `&mut` parameters refer to the
  final value as `final(x)` and the initial value as `old(x)`.
- Iterate on one module with `just verify-module <name>` (it runs
  `cargo verus focus`; `cargo verus verify` rejects partial runs), then run
  the full `just verify`. A run proves something only if it prints a
  `verification results`
  line; cargo-verus uses content-checksum freshness, so give each checkout
  (and each git worktree) its own `CARGO_TARGET_DIR`, or one checkout's cached
  result can be replayed for another.
- This Verus version cannot prove existential postconditions over a generic
  error type (`r matches Err(e) ==> exists|p| ...`). Return the witness
  instead (for example `ScanFailure { port, error }` in
  `verified::allocation`).
- `has` is a keyword inside `verus!`; spec functions from sibling kernels are
  referenced by full path (`super::port::is_valid_port`) rather than `use`d;
  ghost-only parameters take the form `Ghost(x): Ghost<T>`.
- `clippy` sees proof blocks before erasure (they count toward
  `too_many_lines`, and patterns such as `single_match_else` fire on them):
  move long proofs into lemmas and prefer `let ... else`.
- To bump Verus: pick a weekly release tag `release/0.YYYY.MM.DD.<sha>` from
  <https://github.com/verus-lang/verus/releases>, set `VERUS_VERSION`, the
  checksums, and the Rust toolchain in `scripts/install-verus.sh`, and set
  `vstd` in `Cargo.toml` to the crates.io version published from the same
  commit (`0.0.0-YYYY-MM-DD-HHMM`). Re-run `just verify`.
