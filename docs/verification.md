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
unverified. Every kernel entry point therefore follows one of three rules:

1. **Total functions.** No `requires` clause. The specification covers every
   possible input, including values production code should never pass (for
   example, port `0`).
2. **Invariant-carrying types.** A kernel type with private fields and a
   `#[verifier::type_invariant]`, constructible only through verified
   constructors (for example `PortSpan`, which guarantees
   `1 <= min <= max`). Rust's privacy rules enforce the invariant for
   unverified code too, so methods on the type may rely on it.
3. **Oracle closures.** Kernels that interleave decisions with effects
   (database lookups, OS probes, filesystem checks) are generic over `Fn` or
   `FnOnce` closures supplied by production code. Their `requires` clauses
   state only the closures' own preconditions (`f.requires(..)`), and every
   result is specified relative to the answers the closures gave
   (`f.ensures`). Closures written in ordinary Rust have no Verus
   precondition, so an unverified caller always meets these clauses.
   - The preconditions are rarely `forall|x| f.requires((x,))`. Most are
     restricted to a domain or made conditional: valid ports only
     (`allocation::oracles_callable`), the valid ports an exclusion entry
     covers (`exclusions::callable_on_entry`), prefix lengths followed only
     by named components (`path::deepest_existing_prefix`), levels or
     indices below a length (`config::first_config_level`), only for an
     implicit path (`path::resolve_by_provenance`), only when the
     unrelated-path guard applies (`release::guard_paths`,
     `release::select_and_guard`, `migrate::guard_moves`), only when there
     are stored rows (`group::reconcile_group`), or only after an earlier
     answer (`allocation::fallback_discipline`,
     `reserve::retry_discipline`).
   - The restriction is load-bearing. A kernel may call a closure only where
     its precondition is granted, so the proof shows that the kernel never
     calls it outside that domain. The glue relies on this when an oracle
     rebuilds a `Port` from the `u16` it receives (`port/allocator.rs`,
     `port/exclusions.rs`) or indexes with the value it receives
     (`levels[level]` in `config/loader.rs`, `keys.keys[id]` in
     `operations/migrate.rs`, `candidates[index]` in
     `operations/release.rs`, `components[..k]` in
     `path/canonicalize.rs`).
   - That the closures do not panic on the inputs they are given is part of
     the trusted glue.

## Trusted computing base

A verified property holds for the shipped binary only if the following are
correct; everything else in a kernel is checked. This is a summary by
category: the "Trusted" list in each kernel section below is part of the
trusted base too, and names the specific facts.

- the Verus verifier and the Z3 SMT solver (release pinned in
  `scripts/install-verus.sh`; `vstd` pinned in the workspace `Cargo.toml`);
- `vstd`'s specifications of the `core`/`std` operations a kernel calls
  (for example `u16::checked_add`);
- the toolchain skew: those specifications are written for the Rust
  toolchain the verifier is built with (`VERUS_RUST_TOOLCHAIN` in
  `scripts/install-verus.sh`, 1.100.0), and Verus resolves them against that
  toolchain's names and signatures only; their content is trusted
  (`assume_specification` inside `vstd`). `trop` is built and tested with
  the toolchain in `rust-toolchain.toml` (1.99.1). The specified operations
  are assumed to behave as specified in both. A few are checked under the
  build toolchain by exhaustive tests (`char::is_whitespace`,
  `char::len_utf8`, `char::escape_default`); most (`str::split_at`,
  `str::chars`, `Vec::insert`, `String::push`, ...) are not;
- the `verus!` macro's erasure (it removes ghost code only);
- the production glue that converts domain types to kernel inputs and back,
  and the few checks glue makes on its own. For example, a group dry run
  without a database (with auto-init enabled) checks that the database's
  directory could be created (`check_database_directory_creatable`,
  unverified), and every group dry run checks the busy timeout before it
  opens its snapshot through `DatabaseConfig::check_busy_timeout`, the check
  `Database::open` runs (the verified `config::busy_timeout_ok`, with the
  same validation error); that it runs there is glue;
- the oracle closures production code passes to oracle-generic kernels. They
  must not panic on the inputs they are given. The kernels' specifications
  are relative to the closures' answers, so they do not assume the closures
  are correct; an end-to-end property does. Some closures also observe or
  decide facts of their own, which no kernel specification constrains:
  - the allocator's reservation oracle compares the stored holder's key with
    the key being replaced (`PortAllocator::is_reserved_by_other`); the rule
    built on that comparison is the verified `allocation::reserved_by_other`.
    The README's "No flag can take a port owned by another reservation key"
    rests on both;
  - the unrelated-path guard's physical fallback
    (`Database::validate_path_relationship`, called by `reserve`,
    `reserve-group`, `migrate` and `release`) decides a lexically unrelated
    path by canonical comparison;
- the `std`/`core` behavior that glue relies on and kernels do not model,
  pinned by tests where noted per kernel (for example `std::path` component
  splitting, and the agreement of the kernels' decimal parsers with
  `from_str`);
- third-party libraries and SQLite, as each kernel section lists them:
  `serde_yaml` parsing, emitting and key identification (`config_rewrite`,
  checked at run time); SQLite's TEXT `=` and `instr`, `typeof` storage
  classes, `CHECK`/`UNIQUE` constraints, one-snapshot reads within a
  transaction, and the atomicity of `IMMEDIATE` transactions (`storage`,
  `schema_version`, `migrate`, the dry runs); rusqlite's value mapping; and
  chrono's `MAX_UTC` (`storage`);
- the faithfulness of models that stand in for external systems: the
  bash/zsh, fish, PowerShell and dotenv lexer models behind the export-line
  theorems (`shell`, `render`); the Unicode tables (`White_Space` in
  `identifier`; `Bidi_Control`, `Zl` and `Zp` in `render`), each checked
  over every scalar value against `std` or an independent table; and
  `lex_lt` as Rust's `str` order (tested);
- the platform scope. The byte-level path properties hold on Unix only:
  lexical normalization and the tilde join (`path`), stored-path canonical
  form and the prefix query (`storage`), and `rebase_canonical`
  (`migrate`). Other targets use `std` there instead (`Path::join`,
  `Path::is_absolute`, `Path::starts_with`, `strip_prefix`), or feed the
  storage kernel platform facts that `std` decides. Those properties are not
  claimed for them; the component-kind kernels (`resolve_component_kinds`,
  `path_relationship`) still decide on every platform;
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
`compact_exclusions`, `is_compact`), `trop/src/config/validator.rs`
(`validate_excluded_ports`), `trop/src/config/schema.rs` (`PortExclusion`'s
`FromStr`, used by the YAML string form), `trop/src/config/environment.rs`
(`TROP_EXCLUDED_PORTS`, through the same `FromStr`; `parse_u16` also decides
`TROP_PORT_MIN`, `TROP_PORT_MAX` and `TROP_PORT_MAX_OFFSET`), and
`trop/src/cli_support.rs` (`unreserved_ports`). The CLI reaches the kernel
through `trop::port::exclusions` and through that `FromStr`: `trop exclude`
parses its argument with `PortExclusion`'s `FromStr`
(`trop-cli/src/commands/exclude.rs`, `parse_exclusion`);
`trop-cli/src/commands/compact_exclusions.rs` and `scan --autocompact`
(`trop-cli/src/commands/scan.rs`) call
`trop::port::exclusions::{compact_exclusions, is_compact}` directly; the
exclusion-list edits below list the rest.

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
- Construction (`canonical_runs`, behind `compact_exclusions` and
  `ExclusionSet::from_entries`): the entries are validated, ordered by their
  lowest port with the verified stable merge sort
  (`verified::sort::stable_order`), and merged in one pass (`extend_runs`:
  each entry extends the last run or starts a new one). That is
  `O(n log n)` for `n` entries; the previous construction copied the run
  list once per entry. The proof carries the coverage of the sorted prefix
  (`in_order`) and shows that a stable order excludes the same ports as the
  list (`lemma_in_order_coverage`). Incremental inserts (`insert_port`,
  `insert_span`, `insert_entry`) still rebuild the run list once per insert
  (`insert_run`), which is linear in the number of runs.
- `ExclusionSet` (inside `ExclusionManager`): type invariant "the run list is
  canonical"; its view is the set of valid ports covered, so port 0 is never
  a member. `from_entries` builds exactly the covered set or reports the
  first invalid entry; `contains` (binary search) is exact;
  `insert_port` / `insert_span` / `insert_entry` add exactly the entry's
  ports, and an invalid entry leaves the set unchanged; `members_between`
  returns exactly the members in `lo..=hi`, strictly ascending; `len` is the
  set's cardinality (at most 65535, no overflow); `runs` is the set's unique
  canonical list; `clone` returns the same run list (`res.spec_runs() ==
  self.spec_runs()`) and so the same set (`res@ == self@`).
- `parse_u16` accepts exactly `is_u16_text` (an optional `+`, then one or
  more ASCII digits, value at most 65535) for any bytes, without overflow.
  That this is also the syntax of `u16::from_str` is not proved (`vstd`
  does not specify `from_str`); it is trusted and pinned by differential
  tests. `find_range_separator` finds the first `..` (where
  `split_once("..")` splits). `lemma_decimal_text` and
  `lemma_range_text_roundtrip`: a port's decimal text, and `"start..end"`,
  split and parse back to the same values. Trimming (Unicode `White_Space`)
  is the verified `identifier::trim` (through
  `crate::identifier::trim_identifier`), in `PortExclusion::from_str` and
  for each `TROP_EXCLUDED_PORTS` item.

Glue: entries are copied field for field between `PortExclusion` and
`ExclusionEntry`; kernel errors map to the validator's `excluded_ports[i]`
messages, or to the `InvalidPort` / `InvalidPortRange` errors
`ExclusionManager` always reported; `excluded_in_range` and `iter` wrap the
kernel's (valid) members as `Port`; `add_range` passes the range's
`PortSpan`; `PortExclusion::from_str` splits at the kernel's separator index,
trims each side (the whole text when there is no separator) with the
verified trim, and parses each side with `parse_u16`.

Trusted (in addition to the base): the agreement of `parse_u16` with
`u16::from_str`, pinned by differential tests
(`test_parse_u16_kernel_matches_from_str` in `trop/src/config/schema.rs`,
`prop_decimal_parsers_match_std` in `trop/src/config/environment.rs`). The
exclusion-text trim is no longer `str::trim`; that the switch kept behavior
identical is pinned over every Unicode scalar value
(`test_exclusion_text_trim_matches_str_trim`,
`test_parse_excluded_ports_trim_matches_str_trim`).

#### Exclusion-list edits: `trop exclude`, `scan --autoexclude`, canonical form, reserved-port guard

Glue:
- `trop/src/port/exclusions.rs`: `add_exclusion_to_list`, `add_ports_to_list`,
  `is_compact`, `first_reserved_port`.
- Called by `trop-cli/src/commands/exclude.rs` (`execute`, `check_reserved`),
  `trop-cli/src/commands/compact_exclusions.rs`, and
  `trop-cli/src/commands/scan.rs` (`add_exclusions`).

- `add_exclusion` (`trop exclude`):
  - Validation: invalid entries in the list are reported first, then the new
    entry (at index `es.len()`).
  - Coverage: the result excludes exactly the old ports plus the new entry's
    ports.
  - No-op: when one entry already excludes every port of the new one
    (`already_excluded`, which includes an identical entry), the list is
    returned unchanged.
  - Merge (`added_through`): otherwise the new entry is merged through its
    merge block. That is the least block that contains it and holds every entry
    overlapping the block, so merging is transitive.
    - If no entry overlaps the block, the new entry is appended unchanged.
    - Otherwise the overlapping entries are replaced by one range for the
      block, placed at the first one's position. Every other entry keeps its
      content and relative order, and no other entry overlaps the block
      afterwards.
  - Adjacency is not overlap (`lemma_overlaps_block_meaning`): `5010` next to
    `5000..5009` is appended.
  - The `changed` flag is exactly "the list differs", which is also exactly
    `!already_excluded`.
  - `trop exclude` asks the kernel first and runs the reserved-port guard only
    when `changed` holds: an identical or covered exclusion is a no-op even
    when one of its ports is reserved. The specification says "If the
    excluded port or range is *identical* to a pre-existing exclusion, the
    command should be a no-op." Treating a covered exclusion as a no-op too
    is the kernel's `already_excluded` choice. Pinned by
    `exclude_of_an_already_excluded_reserved_port_is_a_no_op` in
    `trop-cli/tests/exclusion_edits.rs`.
- `add_ports` (`trop scan --autoexclude`):
  - The result is the old list followed by one `Single` for each newly excluded
    valid port, each port once, in the order of its first occurrence in the
    input (`fresh_in_order`).
  - The count equals the cardinality of the set of newly excluded ports
    (`fresh_ports`, `newly_excluded`).
  - Coverage is the union of the old ports and the ports passed in. Port 0 is
    ignored.
- `is_compact`: holds exactly when the list is canonical, that is, when it is
  its own (unique) compaction. `compact-exclusions` and `scan --autocompact`
  rewrite exactly when it is false; entry counts are no longer compared.
- `first_reserved_port` (oracle-generic):
  - Its precondition grants the oracle's own precondition only at the valid
    ports the entry names (`callable_on_entry`). The kernel is therefore proved
    to consult the reservation oracle nowhere else: a call at any other port
    fails verification.
  - It returns the lowest port answered "reserved", or the failure at a covered
    port. Every lower port of the entry answered "not reserved".
  - Errors propagate (no fail-open). The cursor does not overflow at 65535.
  - Not specified: the ascending order and one call per port. Closure calls
    leave no trace in the specification; the glue's unit tests pin both.
- Glue details:
  - Entries are copied field for field.
  - Error index `es.len()` maps to field `exclusion`; other indices map to
    `excluded_ports[i]`.
  - The closure wraps `Database::is_port_reserved` and builds `Port` directly,
    which is sound because the kernel can consult and return only covered ports
    `>= 1`.
  - The order "decide `changed`, then guard, then write" in `exclude.rs` is
    glue (not proved).
  - Pinned by `trop/tests/exclusion_edits.rs`, including differential proptests
    against set oracles, one of which checks the exact appended order.

#### Additions to `verified::exclusions`

- `unreserved_ports(occupied, reserved)`: exactly `occupied` filtered to the
  ports not in `reserved`, in order (duplicates kept). No reserved port is
  returned and every occupied unreserved port is, so `scan --autoexclude` never
  excludes a reserved port (`trop::cli_support::unreserved_ports`).

### `verified::port_bounds` — configured port bounds: validation, resolution, layering, defaults, and `reservations.base`

Glue (where production code calls the kernel):
- `trop/src/port/allocator.rs`: `port_range_from_config` and
  `port_span_from_config`. These are used by `allocator_from_config`, `trop
  scan`'s `determine_range`, and `reserve-group`.
- `trop/src/config/validator.rs`:
  - `validate_port_config`.
  - `validate_group_base_in_range`.
  - `validate` / `validate_effective`: they differ only in the base rule.
- `trop/src/config/merger.rs`: `merge_port_config`.
- `trop/src/config/effective.rs`:
  - `apply_port_layer`, called by `apply_precise`.
  - `validate_configured_group_base`.
  - `BUILT_IN_PORTS`.
  - the transactional `apply_cli_config`.
- `trop/src/config/environment.rs`: `apply_port_overrides`.
- `trop/src/config/schema.rs`:
  - `DEFAULT_MIN_PORT`/`DEFAULT_MAX_PORT`.
  - `PortConfig::default`.
  - `PortConfig::bounds`/`from_bounds`.
  - `PortConfig::check_command_line_bounds`.
- `trop/src/config/builder.rs`:
  - `default_config`.
  - the configured-range base check after the file layers.
  - per-document validation of the programmatic layer.
- `trop/src/operations/reserve_group.rs`:
  `config_with_group_base_as_scan_start`.
- `trop-cli/src/cli.rs`: `set_port_overrides` and `parse_port_flag`.

What the glue does: it copies `PortConfig` field for field into
`PortBounds`/`PortLayer`. It builds the command-line layer from the selected
`ConfigField`s; trop-cli's layer never sets `max_offset`. It records provenance
from the returned `LayerLeaves`, and maps each kernel error to its historical
`(field, message)`.

- `resolve_port_bounds` (resolution): unchanged. It is the one resolution
  behind the allocator, `trop scan`, and `reserve-group`'s base narrowing.
  `verified::group::group_scan_bounds`, a second copy, was removed.
- `check_port_bounds` (document rules): `ConfigValidator::validate_port_config`
  delegates to it. A differential test shows every `(field, message)` is
  byte-identical to the previous validator. The unreachable "Offset would
  create invalid max port" branch was deleted
  (`lemma_validated_offset_max_is_valid_port`).
- `lemma_resolution_leniency` is an iff. A resolvable configuration is strictly
  valid exactly when it is neither "both `max` and `max_offset`" nor
  "`max_offset == 0`".
- `overlay_port_bounds` lays one configuration layer over the section below. A
  layer is a YAML document, the `TROP_PORT_*` environment, a programmatic
  override, or the command line. The order is defaults < user < project < local
  < environment < programmatic < command line.
  - The result's `min` is the layer's own, otherwise the one below.
  - A layer that sets an upper bound determines it, anchored at the `min` in
    effect at that layer.
  - A layer that sets no upper bound keeps a resolved upper bound below
    unchanged.
  - Results are normalized: a resolvable `max_offset` is stored as the absolute
    `max` it names.
  - Shapes validation rejects (both forms, or a zero or overflowing offset) are
    kept, and validation of the next state rejects them.
  - Such an unresolvable offset is the one value a later `min`-only layer
    re-evaluates. `ConfigBuilder` validates every file and the programmatic
    layer on their own and checks the environment's declaration, and
    `apply_cli_config` is transactional, so only a bare `ConfigMerger` merge
    can produce it.
- Theorems:
  - `{min: m, max_offset: o}` and `{min: m, max: m + o}` are interchangeable in
    every layer stack (`lemma_offset_form_interchangeable`).
  - Every effective section that passes validation holds exactly one absolute
    `max` (`lemma_validated_overlay_is_canonical`;
    `lemma_overlay_all_canonical` gives the same before validation). So "Either
    max or max_offset must be specified" and the offset errors are unreachable
    for effective configurations (`lemma_canonical_bounds_checks`).
  - A complete valid document yields exactly its own range over anything below
    (`lemma_valid_document_layer`).
  - Layering is associative for layers that anchor their offsets at their own
    `min`, over sections without an unresolvable offset
    (`lemma_overlay_compose`).
- `check_upper_declaration` is the environment layer's self-check. It requires
  that the layer does not set both forms, and that an offset `>= 1` lands on a
  port at the `min` in effect. Validation rejects every document this check
  rejects. When a document's first violation is an upper-bound rule, both
  report the same error (`lemma_upper_declaration_agrees_with_validation`).
- `check_override_bounds` checks `trop reserve`/`trop scan` `--min`/`--max`.
  Each value must be a valid port and `min <= max`, with the document rules'
  order (`lemma_override_matches_document_rules`). An accepted pair becomes
  exactly the effective range (`lemma_override_pair_is_effective_range`). Both
  commands exit 4 on a bad pair.
- `group_scan_span` gives reserve-group's scan range. Without a base it is the
  configured span. With base `b` it exists exactly when `b` lies in the span,
  and is then `b..=span.max`: the span's ports `>= b`, in order, never `b +
  max_offset`.
- `group_base_outside_range` is the `reservations.base` rule for one section. A
  strictly valid section admits `min..=requested_max`, or `min..` when it names
  no upper bound (`base_within_section`), and the rule reports exactly a base
  outside that. `lemma_base_rule_predicts_document_layer`: a rejected base is
  outside the range of that document laid over any section. The rule is applied
  in three places:
  - by `ConfigValidator::validate` to each document (`trop validate`, every
    loaded file, the programmatic layer);
  - by `ConfigBuilder` to the section the configuration files name
    (`EffectiveConfig::validate_configured_group_base`, attributed to the
    newest of the group's and the port bounds' sources);
  - by `reserve-group` to the effective range it scans.

  Environment and command-line port overrides do not invalidate a group's base
  for other commands.
- `DEFAULT_MIN_PORT`/`DEFAULT_MAX_PORT` (5000/7000) are kernel constants.
  `default_port_bounds` (proved canonical and valid by `lemma_defaults_valid`)
  is the source of `PortConfig::default` and `EffectiveConfig`'s built-in
  fallback.
- Not proved:
  - unconditional associativity;
  - re-anchoring of an unresolvable `max_offset` by a bare `ConfigMerger` merge
    (historical);
  - per-document prediction of the base for documents without a complete
    `ports` section;
  - the glue mappings, provenance and attribution, pinned by
    `trop/tests/port_bounds_layering.rs` and
    `trop-cli/tests/port_bounds_cli.rs`.


### `verified::allocation` — availability classification and allocation scans

Glue: `trop/src/port/allocator.rs`:
- `PortAllocator::{port_availability, allocate_single_ignoring,
  find_next_available_ignoring, find_next_allocatable, exhaustion_blockers}`;
- `PortAvailability`, an alias of the kernel's `Availability`;
- `unavailable_reason`;
- the oracle closures `reservation_oracle` (database: reserved by a key
  other than the one being replaced), `exclusion_oracle` and
  `occupancy_oracle`. `reservation_oracle` calls `is_reserved_by_other`,
  which looks up the port's holder, compares its key with the replaced key
  (`Holder::{Nobody, ReplacedKey, OtherKey}`), and returns the verified
  `reserved_by_other`.

Every result is specified relative to the oracles' answers
(`oracle.ensures`). The database and the OS are not modeled.

- `classify_port` (`classified`):
  - A port outside the scope (port 0, or outside the range when the range is
    required) is `Excluded`. That no oracle is consulted for it is proved
    for port 0 only, where the oracles' preconditions are not granted; for
    a valid port outside the range it holds by construction.
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
- `reserved_by_other` (`held_by_other`): the reservation check's answer for
  the holder class the glue reports. An `OtherKey` holder blocks; a
  `ReplacedKey` holder and `Nobody` never block. Which stored reservation
  counts as the replaced key (the same path *and* the same tag) is the
  trusted glue comparison in `is_reserved_by_other` (see "Trusted for these
  kernels"), not this kernel. `lemma_other_holder_always_blocks`: under a
  consistent snapshot, a port for which the reservation oracle answered
  "reserved" (the answer `reserved_by_other` gives for an `OtherKey` holder)
  is classified `Reserved` under every combination of ignore flags
  (`--force` included), or `Excluded` outside the scope; it is never
  `Available`, and no probe error is reported for it. The lemma does not tie
  `Holder` to the oracle closure; that the oracle answers "reserved" for
  every port another key holds is that glue comparison. This is the kernel
  side of the README's "No flag can take a port owned by another reservation
  key".
- The kernels consult an oracle only at a valid port (`1..=65535`): their
  precondition grants the oracles' preconditions nowhere else. This makes
  the glue's `Port` reconstruction sound.
- Lemmas, assuming a consistent snapshot (`deterministic_oracles`):
  - Classification is unique: the specification determines the result,
    including which error.
  - The scan returns the least available port.
  - When the scan reports the range exhausted, the diagnostics set at least
    one blocker flag.
  - `lemma_blockers_exact` (with `lemma_witnessed_kind_is_flagged`): given
    `exhaustion_blockers`' success postcondition, each flag is set *exactly*
    when some port of the range is classified that way. The "if" direction
    is the lemma's; the "only if" is in the postcondition.
- Further lemmas:
  - Turning an ignore flag on never makes an allocatable preferred port
    unavailable (monotonicity).
  - The flags are independent: with both set (`--force`), an in-range port
    can be blocked only by a reservation.
  - Without flags, the preferred check equals the scan's classification.

Not proved:
- Each outcome is justified by *some* answer of each oracle
  (`answered(o, p, a)` is `o.ensures((p,), Ok(a))`), not by the only one. A
  body that asked an oracle again after an unfavourable answer and acted on
  a later "free" would also verify, so "fail closed" holds relative to some
  answer the oracle gave. Under `deterministic_oracles` the two coincide.
- Calling each oracle at most once per port, laziness for valid ports
  outside the range, and stopping a scan at the first failure. These hold
  by construction and are pinned for the probe by the `RecordingChecker`
  tests in `trop/src/port/allocator.rs`
  (`scan_probes_each_candidate_once_in_order_and_skips_blocked_ports`,
  `scan_errors_stop_at_the_first_failing_candidate`,
  `probe_failures_on_blocked_ports_do_not_abort_allocation`,
  `ignored_probe_is_never_run_and_ignored_exclusion_still_probes`). No test
  counts the reservation and exclusion oracles' calls.

Trusted for these kernels:
- The oracle closures are total (no panics) on valid ports. The kernels'
  `requires` states only their preconditions.
- The holder lookup and key comparison in `is_reserved_by_other`
  (`ignored_key == Some(reservation.key())`, that is, `ReservationKey`
  equality on path and tag); the rule itself is `reserved_by_other`. Pinned
  by `test_reserved_by_other_glue_exempts_only_the_replaced_key` (ports 1,
  5000 and 65535; sibling tags of the replaced key's path block) and
  `test_replacement_ignores_only_the_current_keys_port`.
- `#[allow(dead_code)]` appears on `ScanFailure::port`, a witness that only
  the specs read. It is not a verification escape. This Verus version cannot
  prove
  existential postconditions over a generic error type, so the scans return
  the failing port as a witness.

#### The preferred-then-scan fallback (`allocate_with_fallback`)

Glue: `trop/src/operations/reserve.rs`. `single_attempt` is the oracle; it
wraps `PortAllocator::allocate_single`/`_replacing` and is passed as both the
request and the fallback request.

- `allocate_with_fallback` (`fallen_back`): asks for the preferred port.
  Only after a given preference is reported unavailable (`fell_back`) does it
  make the fallback request, with no preference. This call discipline is
  verified through the oracles' preconditions (`fallback_discipline`), and
  each oracle is `FnOnce`. `Fallback` cannot represent "preferred
  unavailable", which removed three `unreachable!()` arms. An oracle that
  violates its contract is reported as exhaustion, not a panic.
- `lemma_allocator_keeps_fallback_contract`: the verified `allocate_single`
  never reports a preferred port unavailable without a preference, and never
  reports the range exhausted with one. So, with the production allocator,
  `Exhausted` means the scan found every port of the range blocked.

### `verified::occupancy` — occupancy-check policy

Glue: `trop/src/port/occupancy.rs`:
`OccupancyCheckConfig::from(&OccupancyConfig)` and the `flags`/`from_flags`
field copies; `OccupancyProbe::new` (kernel `Probe` to `127.0.0.1` / `0.0.0.0`
/ `::1` / `::` plus the port); `SystemOccupancyChecker::report_with`
(classifies each bind `io::Result`: `Ok` is `Free`, `AddrInUse` is `InUse`, any
other error is `Failed(error)`, and the `FnMut` runner is wrapped in a
`RefCell`); `scan_range` (`Port` <-> `u16`); `MockOccupancyChecker`;
`OccupancyOwner::refresh_status`. Socket binds, error-kind classification, and
owner lookups stay outside the kernel.

- `effective_occupancy_flags`: each skip flag is on exactly when the global
  `skip` or its own setting is `Some(true)`. `None` and `Some(false)` both mean
  off, so an explicit `false` never re-enables a probe another switch disabled.
  `check_all_interfaces` is on exactly when set to `Some(true)`. End to end, a
  probe is enabled iff no covering skip switch (global, its protocol, its
  family) is on and it is a localhost probe or `check_all_interfaces` is on,
  and `skip: true` disables every probe.
- `probe_matrix`: exactly the enabled protocol x family x scope probes, each
  once, strictly increasing in the documented order (IPv4 before IPv6, then
  localhost before wildcard, then TCP before UDP). Its length is |protocols| x
  |families| x |scopes| <= 8. Lemmas: this characterization determines the
  sequence uniquely; unset settings give exactly TCP/UDP x IPv4/IPv6 on
  localhost; `check_all_interfaces` keeps every probe and only adds wildcard
  probes.
- `occupancy_disabled` (the mock checker's guard): true exactly when no probe
  is enabled, i.e. when the system checker's matrix is empty and its report
  `was_skipped()`.
- `run_probe_matrix` (generic over the probe oracle): on success, every enabled
  probe ran and completed, `enabled` equals the matrix length, and the evidence
  is exactly the enabled probes observed in use, in matrix order. So a report
  is occupied iff some probe was in use, and an empty matrix means skipped and
  not occupied. On failure, the returned probe is enabled, one of its observed
  outcomes was `Failed(error)`, and every enabled probe before it in matrix
  order has a non-failed observed outcome. Only probe failures produce
  errors.
- `scan_ports` (the default `find_occupied_ports`, the mock's override, and
  `find_occupied_reports`): inspects `lo..=hi` in ascending order. On success
  it returns exactly the kept values in ascending port order. On error it
  returns only the error: the inspection of some port in the range returned
  it, and every port before that one has an inspection observed without
  error. An empty range inspects nothing.
- `owner_status`: `Available` iff PID, process name, and user are all known;
  `Unavailable` iff none is; `Partial` otherwise.

Not proved (Verus specifies `Fn` closures by their results, not by the
number of calls):
- That each probe or port runs exactly once. A kernel that retried a failed
  probe or port and went on after a later success would also verify: the
  failure would vanish from the report, or a later port's error would be
  reported instead of the first one's. So "the error is the first failure's"
  and "collected evidence never hides a failure" hold by construction, not
  by proof.
- That nothing runs after the first failure.
- The glue mappings above (covered by exhaustive tests over all 729 settings
  and all 32 flag combinations).

The first two are pinned by call-counting tests in
`trop/src/port/occupancy.rs`
(`test_failure_on_last_probe_runs_every_earlier_probe_and_fails_closed`,
`test_default_find_occupied_ports_stops_at_first_failure`).
`find_occupied_reports` delegates to
`SystemOccupancyChecker::find_occupied_reports_with(range, config,
run_probe)`, the `scan_ports` glue with an injectable probe runner. A test
over `65532..=65535` with one enabled probe pins it:
`test_find_occupied_reports_keeps_occupied_ports_in_order_and_stops_at_first_failure`
checks that only occupied ports are reported, in ascending order, each with
its in-use probe as evidence, and that the scan stops at (and reports) the
first failing port without probing later ports.

### `verified::group` — group requests, offset-pattern search, fresh plans, stored groups, output selection

Glue: `trop/src/port/group.rs` (`GroupAllocationRequest::{normalized,
sorted_shape}`, `PortAllocator::{find_pattern_match,
allocate_group_with_cleanup, allocate_fresh_group, compatible_existing_group,
plan_reconciliation}`), `trop/src/operations/reserve_group.rs`
(`build_group_request`, `config_with_group_base_as_scan_start`,
`execute_group_allocation`, `select_group_output`, `ReserveGroupPlan::dry_run`,
`open_dry_run_snapshot`, `check_database_directory_creatable`),
`trop/src/operations/autoreserve.rs` (config selection,
`AutoreservePlan::dry_run`), `trop/src/operations/executor.rs` (the
`AllocateGroup` arm), and
`trop-cli/src/commands/{reserve_group,autoreserve}.rs` (`--format`/`--shell`,
`--dry-run`, `dry_run_database`). The glue sorts services by tag, supplies the
probe closures (database reservations, exclusions, OS occupancy) and the
cleanup closure, maps errors to messages, and performs the database writes. It
no longer trims anything: the kernel does.

- `validate_group_request` / `ValidatedGroupShape` / `ValidatedGroupRequest`:
  the one identifier rule is `super::identifier`'s.
  - A request is accepted exactly when it is non-empty, `identifier_verdict`
    accepts the project and the task (when given), and the services form a
    valid request (`group_request_valid`): every tag accepted by
    `identifier_verdict`, some allocation strategy and no preferred port 0 per
    service, and trimmed tags, offsets, and preferred ports pairwise distinct.
  - The result keeps the services in order, with each tag replaced by its
    trimmed value (`shape_normalized`), plus the trimmed project and task
    (`opt_normalized`). The verified `identifier::check_identifier` does the
    trimming, so request validation no longer trusts `str::trim`.
  - Otherwise the first error is reported in production order: no services,
    project, task, then the first service with an issue. A service's issues are
    checked in this order: the identifier rule (`Blank`, `ContainsNul`,
    `TooLong`), duplicate trimmed tag, missing strategy, preferred 0, duplicate
    offset, duplicate preferred.
  - `ValidatedGroupShape`'s type invariant (`group_shape_valid`) states that
    every tag is a stored identifier value (`identifier_verdict(t) == Ok(t)`:
    valid and already trimmed).
  - `lemma_normalized_shape_valid` connects the two predicates.
    `lemma_validation_idempotent` proves that a validated shape is a valid
    request that normalizes to itself, so normalizing twice changes nothing.
  - The group kernel's former byte-level copy of the identifier rule
    (`IdentifierIssue`) was deleted.
- `pattern_scan_end` / `find_pattern_base`: the lowest base `b >= min` at which
  every `b + offset` is in range, not a pinned port, and reported available by
  the probe. Every lower base was rejected. `None` means every base at all (not
  just the scanned ones) is rejected, because the early stop at `max -
  max(pattern)` is sound. The probe is only ever called on in-range,
  non-avoided ports, and no `u16` arithmetic overflows. Error precedence is
  specified (`search_failed_at`): an error comes from a base `c >= min` such
  that every lower base was rejected, from the probe of a pattern port of
  `c` (in range, not avoided) whose earlier pattern ports were usable. So
  the result, given the probe's answers, is one an ascending scan gives: an
  error never pre-empts a lower base that fits. The order of probe calls is
  not specified (Verus specifies closures by their answers), so a body that
  probed `range_max` first and discarded the answer would still verify. An
  empty pattern is `Ok(None)`. (A body that returned a higher base, or a
  higher base's probe error, before every lower base was rejected, or a
  probe error for an empty pattern, would not verify.) The search is
  generic over the probe closure, and its guarantees are stated in terms of
  the answers the probe gave, so no stable oracle has to be trusted.
- `plan_fresh_group`: a service is pinned to its preferred port only if the
  preferred probe reported it available (whole port domain, no range check).
  Every other service legitimately falls back to its offset and gets `base +
  offset` inside the scan range, reported available by the scan probe. All
  ports are valid (`>= 1`) and pairwise distinct, the base is the lowest that
  works, and there is no base exactly when every service is pinned. The errors
  are precise: a preferred-only service whose preferred port is unavailable,
  complete pattern exhaustion (every base rejected), or the first probe error
  in classification-then-search order. That is either the preferred probe of
  a service after every earlier service was classified
  (`preferred_error_first` / `preferred_failed_at`), or the scan probe at a
  base every lower base of which was rejected, on a fallback port whose
  earlier fallback ports (in service order) at that base were usable
  (`scan_error_first` / `scan_failed_at`, via `lemma_scan_failure_maps`).
  The weaker `probe_error_source` is still stated. (A body that returned a
  scan-probe error before classifying every service would not verify.) As
  for `find_pattern_base`, the order of probe calls, and the absence of
  extra probes whose answers are discarded, are implemented but not
  specified.
- Exhaustion retry: an exhausted fresh plan (a new group, or one allocated
  after a forced replacement deleted the stored group) goes through
  `verified::reserve::allocate_with_cleanup_retry`, the control single
  reservations use.
  - The policy is `verified::reserve::cleanup_policy` of the configuration's
    `disable_autoprune`/`disable_autoexpire` (configuration files and
    environment only: `--disable-autoprune`, `--disable-autoexpire` and
    `--disable-autoclean` exist only for `trop reserve`). See "Shared cleanup
    policy" under `verified::reserve` for how far it is the same rule as
    `reserve_policy`.
  - With prune or expire enabled, exactly one cleanup runs after an exhausted
    first plan, and one re-plan only when that cleanup freed something;
    otherwise `GroupAllocationFailed` reports the cleanup counts and that the
    scan was not retried. The cleanup never runs when both are disabled.
    Pinned by `cleanup_that_freed_nothing_is_not_retried`
    (`trop/src/port/group.rs`) and
    `disabled_expiry_keeps_stale_reservations_and_the_group_fails`
    (`trop/tests/group_cleanup_retry.rs`).
  - Everything runs in the group's savepoint, so a failure after the cleanup
    also rolls the cleanup back. A success after cleanup reports a warning
    with the counts.
- `match_stored_tags`: stored rows must be tagged with distinct tags and carry
  exactly the requested tag set (tags compared as `String`s). The returned
  matching is a bijection between requested services and stored rows. The first
  untagged or duplicate row is reported first.
- `derive_compatible_base`: sound (every service is on its preferred port or at
  `base + offset` with base and port in the scan range), complete (if any base
  fits, the group is accepted), and unique (the reported base is the only one
  that fits). Errors name the first failing service in tag order. A
  preferred-only service off its preferred port is a conflict, so `--force` can
  replace the group.
- `lemma_fresh_plan_is_compatible`: every fresh plan passes the
  shape-compatibility check `derive_compatible_base` under the same range,
  with the same ports and base. That is one condition of the refresh path,
  not all of it: no theorem states that `match_stored_tags` accepts the
  stored rows of a fresh plan, that `plan_group_metadata` passes for
  unchanged metadata, or that the glue's mapping to `CompatClass` is right.
  That a repeated `reserve-group`/`autoreserve` refreshes end to end is
  pinned by `repeating_a_fresh_allocation_refreshes_it_unchanged`
  (`trop/src/port/group.rs`).
- `decide_group_reconciliation`: no stored rows means allocate fresh. A
  compatible group is refreshed. Only `--force` replaces a group, and only one
  that conflicts. Other errors are never overridden. `reconcile_group`
  applies it: it classifies the stored group through its oracle only when
  there are stored rows (the oracle's precondition is granted only then),
  and returns the classifier's result in the variant (`Refresh` only with a
  `Compatible` result, `Replace` only with a forced `Conflict`, `Fail` only
  with `OtherError` or an unforced `Conflict`).
- `select_group_output` (`--format`/`--shell` of `reserve-group` and
  `autoreserve`):
  - JSON, dotenv, and human output are selected whatever `--shell` says.
  - Export output uses the explicit shell's parse result. An unknown shell is
    an error and is never replaced by detection. Without an explicit shell,
    export uses the detection result.
  - The call discipline is verified through the oracles' preconditions
    (`output_discipline`): the parser runs only for export, and the detector
    only for export without an explicit shell.
- The `reservations.base` scan range is
  `verified::port_bounds::group_scan_span` over the one verified resolution of
  the configured bounds.
- `select_highest_precedence` (last maximum, like `Iterator::max_by_key`),
  `first_present`, and `config_service_offset` (omitted offsets default to 0).
  Together with `lemma_offsets_exclude_missing_fallbacks`, this makes the
  preferred-only failure paths unreachable from configuration.

Bugs fixed (group-rest):
- Group allocation never ran the automatic prune/expire-then-retry step the
  specification requires for groups ("When allocating a port group, we do the
  same thing"). A stale row blocked `reserve-group` with
  `GroupAllocationFailed`.
- `reserve-group --dry-run` / `autoreserve --dry-run` returned success before
  validating `--shell` or planning. They exited 0 for requests a live run
  rejects (unknown shell, unrelated path, sticky task change, incompatible
  group, disabled auto-init without a database) and printed no ports.
  - A group dry run now validates the output first. It then builds and executes
    the plan exactly as a live run does, against a private in-memory copy of
    the database.
  - `open_dry_run_snapshot` builds that copy: the file is opened read-only and
    copied in one read transaction, then initialized or migrated in memory by
    the same schema check `Database::open` runs.
  - The dry run fails with the live run's error and exit code (for an
    uncreatable data directory, see fix-reserve-group below). It prints the
    live run's ports in the requested format, simulates the automatic cleanup
    (and the retry, when the cleanup freed something), and never creates,
    writes, initializes, or migrates the database. Opening a WAL database
    read-only may leave `-wal`/`-shm` files; the contents never change.

Bugs fixed (fix-reserve-group):
- A cleanup that freed nothing was followed by a re-plan, and
  `GroupAllocationFailed` said the scan was retried once (see
  `verified::reserve`).
- `ReserveGroupPlan::dry_run` / `AutoreservePlan::dry_run` with a busy
  timeout above `i32::MAX` ms panicked inside rusqlite (existing database) or
  ignored it (no database). Now `open_dry_run_snapshot` first rejects such a
  timeout by calling `DatabaseConfig::check_busy_timeout`, the check
  `Database::open` runs (the verified `config::busy_timeout_ok`, with the
  same validation error), with or without a database file. Pinned by
  `dry_run_rejects_a_busy_timeout_sqlite_cannot_store_like_a_live_run`
  (`trop/tests/group_dry_run.rs`).
- `reserve-group --dry-run` / `autoreserve --dry-run` with an uncreatable
  `--data-dir` succeeded and printed ports while the live run failed. Now,
  without a database and with auto-init disabled, the CLI's
  `dry_run_database` fails with `NoDataDirectory`, as a live run does.
  Without a database and with auto-init enabled, it calls
  `check_database_directory_creatable`: when the deepest existing ancestor
  of the database's directory is not a directory (or an ancestor cannot be
  inspected), the dry run fails with an I/O error, the live run's error
  class and exit code, and creates nothing. Only the message differs: it
  describes the obstruction instead of repeating the OS text. Permissions
  are not checked. Pinned by
  `dry_run_fails_like_a_live_run_when_the_data_directory_cannot_be_created`
  (`trop-cli/tests/group_dry_run.rs`).

Tests added for trusted glue:
`forced_replacement_reports_the_field_stored_rows_disagree_on`,
`stored_tag_set_mismatch_is_reported` and
`untagged_and_duplicate_stored_rows_are_reported` (`trop/src/port/group.rs`);
`dry_run_migrates_a_v1_database_only_in_memory`
(`trop/tests/group_dry_run.rs`).

Trusted for this kernel, in addition to the base:
- `vstd`'s specs of `String::eq`, `String::as_str`, and `str::to_owned` (with
  those `verified::identifier` already lists);
- `std` sorting by tag, and the `HashMap` <-> `Vec` conversions;
- the probe, cleanup, shell-parser, and shell-detector closures (total);
- the glue's mapping of a `PatternExhausted` plan to `Fallback::Exhausted`,
  with its fallback count kept in a `Cell`;
- the dry-run copy's faithfulness (schema replay plus row copy), pinned by
  `dry_run_snapshot_copies_rows_constraints_and_indexes` and the dry-run
  integration tests.

Dry-run fidelity is by construction (the same planner and executor on a copy of
the same snapshot), not a kernel theorem.

Glue:
- `GroupAllocationRequest::normalized` passes the raw tags, project, and task
  to `validate_group_request` and copies back its trimmed values.
- `sorted_shape` sorts by tag and re-validates the services.
- `allocate_fresh_group` maps `PortAvailability` to `PreferredVerdict`, wraps
  `plan_fresh_group` as the `allocate`/`retry` oracle of
  `allocate_with_cleanup_retry`, and inserts the planned rows.
- `execute_group_allocation` builds `CleanupSwitches` from the
  configuration, calls `cleanup_policy`, and passes the configured `cleanup`
  section (or `CleanupConfig::default()`, threshold unset) to
  `CleanupOperations::automatic_cleanup_in_connection`, mapping its result
  to `CleanupCounts`; the 30-day default comes from
  `cleanup::automatic_selection`. `GroupCleanupCounts` is an alias of the
  kernel's `CleanupCounts`.
- `plan_reconciliation` passes the compatibility check to `reconcile_group`
  as its classifier and has one arm per `Reconciliation` variant. Its two
  `unreachable!`s rest on its own mapping of `Ok` to `Compatible` and of
  `Err` to `Conflict` or `OtherError`.
- `select_group_output` maps `GroupOutputFormat` to `FormatArg` and passes
  `ShellType::from_string`/`ShellType::detect` as the oracles.

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

#### Shared with single reservations

`can_change_field` is now also the single-reservation rule:
`reserve::decide_existing` applies it to each field. The
`trop/src/operations/reserve.rs` `can_change_field` helper is a test-only
delegate.

### `verified::identifier` — whitespace trimming, the identifier rule, stored tags

Glue: `trop/src/identifier.rs` (`trim_identifier`, `check_identifier`,
`is_canonical_identifier`, `identifier_error_message`). Callers:
- `ReservationKey::new` trims and rejects blank tags. It is lenient: it is the
  constructor for stored rows, migration and lookups.
- `ReservationKey::new_validated` applies the identifier rule. `trop reserve`
  uses it.
- `ReservationBuilder::project` and `task` trim; `build` requires the canonical
  form.
- `ConfigValidator::validate_identifier` and `validate_runtime_identifier`
  apply the rule. `validate_reservation_group` delegates to
  `config::check_config_group`, which applies `check_identifier` to each tag
  once and uses the trimmed tag for uniqueness and env-name derivation.
- `trop/src/config/schema.rs` (`PortExclusion::from_str`) and
  `trop/src/config/environment.rs` (`TROP_EXCLUDED_PORTS` items) trim with
  `trim_identifier`, so exclusion text has the same verified trim as
  identifiers; the crate has one whitespace trim on these paths.
- `trop/src/operations/inference.rs` (`normalize_inferred_identifier`) keeps
  the value `check_identifier` returns (the kernel's trimmed sub-slice), so
  an inferred project or task is trimmed and validated by the kernel and is
  canonical. It no longer trims with `str::trim` and discards the kernel's
  result.

Properties:
- `trimmed(s)` specifies `str::trim`: drop leading, then trailing, Unicode
  `White_Space` chars. `is_ws` lists the 25 code points and is proved equal to
  `vstd`'s model of `char::is_whitespace`.
- `trim(&str) -> &str` returns exactly `trimmed(s)` as a sub-slice of its
  input. Its `split_at` cuts are proved to fall on char boundaries, so it
  cannot panic.
- Trim bounds characterize the result: everything outside `s[a..b]` is
  whitespace, `s[a]` and `s[b-1]` are not, and the bounds are unique when the
  result is non-empty. Trimming is idempotent and ignores whitespace padding,
  and the result is empty exactly when `s` is all whitespace.
- `check_identifier` is exactly `identifier_verdict`: `Blank`, then
  `ContainsNul`, then `TooLong` (more than 255 UTF-8 bytes of the trimmed
  value), otherwise `Ok(trimmed)`. An accepted value is canonical, re-validates
  to the same result, and contains no NUL anywhere. NUL is never whitespace, so
  the trimmed value contains NUL exactly when the raw value does.
- `is_canonical_identifier` decides `is_canonical(s)`: `s` is non-empty and
  `trimmed(s) == s`, equivalently neither its first nor its last char is
  whitespace. `ReservationBuilder::build` decides with it, so every built
  reservation has the form the stored-row readers require.
- Stored-tag sentinel (the specification of the codec). Schema v2 stores
  `None` as `""`. Decoding inverts encoding for every valid tag, the encoding
  is injective (a tagged key never aliases the untagged key), and every tag
  accepted by `check_identifier` round-trips. The executable codec is in
  `verified::storage` (`encode_storage_key`, `lookup_key`,
  `decode_stored_tag`), specified with these definitions. Every reservation
  write path enforces `is_valid_tag` even though `ReservationKey` has public
  fields and derives `Deserialize`: it binds only keys that
  `encode_storage_key` produced (`StoredKey` and `StoredRow` have private
  fields). Inserts and `delete_reservation_if_unchanged` get theirs from
  `encode_record`, which calls `encode_storage_key`; the other writes get
  theirs from `ReservationKey::storage_key`. (The schema v1-to-v2 migration
  copies rows in SQL instead; its preflight,
  `schema_version::classify_legacy_row`, blocks every row whose copy the v2
  decoder would reject.)

### `verified::env_name` — portable environment-variable names

Glue: `trop/src/identifier.rs` (`EnvironmentVariableName::{parse,
resolve}`), called by `trop/src/output/shell.rs` (`parse`, for the name of
an export line) and `trop/src/output/formatters.rs` (`resolve`, for each
group entry). `collision_key` and `derive_from_tag` are test-only, as is the
validator's call to `resolve` (in its reference model). Production
configuration and rendering call the kernel from other kernels:
`config::check_config_group` uses `check_env_name`, `resolve_env_name` and
`env_collision_key`, and `render::render_group` re-checks each name with
`check_env_name_bytes` and compares names ignoring ASCII case itself.
`environment_variable_name_message` (which renders an `EnvNameError`) and
`NON_CONVERTIBLE_SERVICE_TAG_MESSAGE`, both crate-visible in
`trop/src/identifier.rs`, are the one message table for the env-name
diagnostics, reported by `EnvironmentVariableName` and by the validator's
group checks.

- `check_env_name` is exactly `env_name_verdict` over the name's UTF-8 bytes:
  `Empty`, then `TooLong` (more than 255 bytes), then `BadFirst`, then
  `BadRest`. It accepts exactly `[A-Za-z_][A-Za-z0-9_]*` of 1 to 255 bytes.
- Shell-inertness: a portable name is pure ASCII `[0-9A-Za-z_]`. It contains no
  control byte, space, quote, backtick, `$ = ; & | < > ( ) { } [ ] * ? # \ ! ~
  % ^ , . : @ + -`, and no non-ASCII byte.
- `derive_env_name` returns a name exactly when the byte-wise derivation (`-`
  to `_`, `a-z` to `A-Z`) is portable, and then returns those bytes. Tags with
  non-ASCII text never derive, so the old `is_ascii` pre-check was redundant.
- Derivation preserves length, removes lowercase and `-`, and is idempotent.
  Two tags derive the same name iff they agree byte-wise up to ASCII case and
  `-`/`_`.
- `resolve_env_name`: an explicit mapping is validated on its own and returned
  verbatim, never falling back to the tag; otherwise the name is derived. Every
  resolved name is portable.
- `env_collision_key` is the ASCII uppercase of the name. For portable names
  the key is portable, and two keys are equal exactly when the names are equal
  ignoring ASCII case.

### `verified::text` — ASCII and UTF-8 helpers

No direct production glue. Other kernels use it: `identifier` (the
char-offset slicing behind `trim`), `env_name` (`string_from_ascii`,
`ascii_uppercase`), `render`, `shell`, `verbosity` and `schema_version`
(`upper_byte`, `bytes_equal`, `bytes_eq_ignore_ascii_case`,
`string_from_ascii`), and `config` (the folding specifications and lemmas).

- ASCII case folding over bytes and over chars is idempotent, and folded keys
  are equal exactly under ASCII-case-insensitive equality.
- ASCII bytes and chars coincide: ASCII bytes round-trip through UTF-8, and an
  all-ASCII encoding means ASCII chars, one byte per char.
- Every char prefix ends on a UTF-8 char boundary, and prefix byte offsets are
  injective. `char_slice` (kernel-internal) slices a `&str` at proved char
  offsets.
- Exec helpers, each total and exactly its kernel specification:
  `ascii_uppercase` (`ascii_upper_chars`: each char `a`-`z` mapped to
  `A`-`Z`, every other char kept), `upper_byte` (`ascii_upper`, the same on
  bytes), `bytes_equal` (equal byte sequences), `bytes_eq_ignore_ascii_case`
  (`eq_ignore_ascii_case`: equal lengths, and equal bytes after
  `ascii_upper`), and `string_from_ascii` (the string of the given code
  points; the inverse of UTF-8 encoding on ASCII). The first four are
  written to compute what `str::to_ascii_uppercase`,
  `u8::to_ascii_uppercase`, `<[u8]>::eq` and `<[u8]>::eq_ignore_ascii_case`
  compute, but no lemma relates them to those functions (`vstd` specifies
  none of the three ASCII ones), so that correspondence holds by
  construction and is not proved.

Trusted-base additions (no new escape hatches): `vstd`'s specs of
`char::is_whitespace`, `char::len_utf8`, `str::chars` / `Iterator::next`,
`str::as_bytes`, `str::split_at`, `String::push` and
`char::to_ascii_uppercase`.
- `char::is_whitespace` and `char::len_utf8` are checked against the kernel
  model for all 1,112,064 scalar values by
  `identifier::tests::kernel_whitespace_model_matches_std_for_every_scalar_value`.
- The trim and the identifier rule are differential-tested against `str::trim`
  and the pre-kernel implementation, and the env-name grammar and derivation
  against independent pre-kernel oracles, by the property tests in
  `trop/src/identifier.rs`.
- Glue-level trust: the choice of `ReservationKey::new` (lenient) versus
  `new_validated` (rule) at each call site.

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
  normalized. The oracle precondition is `implicit ==>
  canonicalize.requires(())`,
  so the explicit branch provably never calls the canonicalization oracle.
  Implicit paths yield the oracle's canonical path or its error, never a
  silent fallback. They warn exactly when the error is "not found" and
  warnings are enabled.
- `has_parent_segment` (Unix; `storage::prefix_query`): total, and true
  exactly when some segment is `..` (`has_parent_seg_spec(segs(s))`).
  `lemma_parent_free_segs`: without a `..` segment every segment is named,
  and resolving them changes nothing.

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
  canonical path. `canonical_is_component_prefix` (Unix; the post-filter of
  `storage`'s prefix query, and the prefix test in
  `migrate::rebase_canonical`) decides `spec_component_prefix` and is
  total.
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

#### Additions to `verified::path_relation`

- `home_display(path, home)` over `PathToken`s (`std::path` components): `Home`
  exactly when the non-empty `home` equals `path`, `Below { rest_start =
  home.len() }` exactly when it is a strict component prefix, and `Elsewhere`
  otherwise, so `/home/al` never shortens `/home/alice`.
  `lemma_home_display_round_trip`: home's components followed by the rest give
  back the path, and two paths with the same rest are the same path. `trop
  list` shows `~/` for the home directory itself and `~/rest` below it, as
  before; a test checks agreement with the previous `Path::strip_prefix`
  formatting.

### `verified::reserve` — single-reservation decisions

Glue: `trop/src/operations/reserve.rs` (`MetadataIntent::{kernel_intent,
kernel_bytes, resolve_existing, resolve_new}`, `ReservePlan::{flags, policy,
preferred_port, decide_existing, build_plan, plan_existing, allocate_port,
plan_cleanup_retry}`, `single_attempt`, `reconcile_deferred_key`,
`execute_reserve_after_cleanup_with_barrier`, `cleanup_status`),
`trop/src/operations/executor.rs` (`PlanExecutor::{reported_port,
stored_port}` and the `UpdateLastUsed` arm), and
`trop-cli/src/commands/reserve.rs` (`--dry-run`). The glue copies flags
field for field, trims `Set` values with the verified
`verified::identifier::trim` (through `trim_identifier`, the trim the
reservation builder also applies), and passes stored values as
`as_bytes()`. It maps each decision to the historical error, action, or
message. It also performs the database, allocator, cleanup, and git effects.
Keeping `created_at` and excluding the row's own key when reallocating are
glue, not kernel outputs. The cleanup closure of
`execute_reserve_after_cleanup_with_barrier` maps `AutocleanResult` to the
kernel's `CleanupCounts { pruned: pruned_count, expired: expired_count }`.
`cleanup_status(policy, cleanup, retried)` maps the kernel's
`Retry::Exhausted { cleanup, retried }` to `AutomaticCleanupStatus`
(`CleanupRun::{Skipped, NotRetried, Retried}` in `trop/src/port.rs`, read
through `attempted()` and `retried()`), which selects the exhaustion
message.

- **Metadata intents.** `resolve` resolves `Preserve` to the fallback (the
  stored value, or the inferred value of a new reservation), `Set(v)` to `v`,
  and `Clear` to nothing. `uses_fallback` is true exactly for `Preserve`.
  Explicit intents resolve the same way for every fallback
  (`lemma_explicit_intent_ignores_fallback`), so git inference runs only for
  fields the request leaves unset. Resolution is idempotent.
- **`decide_existing`** (`existing_decided`): the project is checked first.
  The result is `StickyProject` exactly when the resolved project differs from
  the stored one without `--force` or `--allow-project-change`, then
  `StickyTask` by the same rule; both use `sticky::change_permitted`.
  Otherwise `--overwrite`/`--force` reallocate. Otherwise the metadata is
  updated exactly when it changes, and the reservation is only refreshed
  (`Touch`) when it does not. The clauses determine the decision
  (`lemma_existing_decision_unique`). Lemmas:
  - `--force` always reallocates.
  - Omitted metadata is never rejected and never changes stored values.
  - Clearing a stored value is a protected change.
  - Without permission, a stored value never changes.
  - Adding permissions never turns an accepted request into a rejection.
  - Idempotence (`lemma_repeat_request_touches`): once a request has been
    applied, by any writer whose normalization fixes the explicit values (the
    reservation builder trims), repeating it without `--overwrite`/`--force`
    is a `Touch`. This holds for creates too. For the production writer the
    premise is trim idempotence, `identifier::lemma_trimmed_idempotent`; the
    lemma's `norm` works on bytes and that one on chars, and the step from
    one to the other is stated, not proved.
  - Agreement with the group path (`lemma_decision_matches_group_row`):
    `trop reserve` and `reserve-group`'s `plan_group_metadata` reject the same
    requests, name the same field first, and store the same metadata.
  Planning (`build_plan`) and deferred execution (`deferred_step`) both call
  it.
- **`reserve_policy`**: its postcondition is `policy == policy_of(flags)`,
  plus the per-field formulas: the path check applies unless `--force` or
  `--allow-unrelated-path` is given, `--force` implies reallocation and both
  preferred-port relaxations, and each automatic cleanup is on unless its
  switch disables it. Its cleanup part is computed by calling
  `cleanup_policy`, so the cleanup rule exists once. Because of the
  `policy_of` postcondition, `lemma_force_dominates` (`--force` dominates)
  and `lemma_policy_monotone` (monotone in the permissive flags) hold for
  the value production uses, and `release::lemma_reserve_guard_is_policy`
  ties reserve's unrelated-path guard to it. (A body in which `--force` no
  longer bypassed the path check would not verify, even with the per-field
  formulas changed to match.) `CleanupPolicy::retry_possible` (prune or
  expire) is the one predicate that both plan time (defer or fail) and
  execution (clean up or fail) use. `choose_preferred`: an explicit
  `--port` wins over the configured hint.
- **`allocate_with_cleanup_retry`** (generic over the port type and the
  error; the cleanup oracle returns `CleanupCounts { pruned, expired }`): the
  specification's allocation steps 4-6 ("If anything got freed up, retry the
  forward scan ... If that still fails, report an error"). `freed_any(c)` is
  `c.pruned > 0 || c.expired > 0`; the two counts partition the removals
  (`cleanup::split_by_reason`). The call discipline is verified through the
  oracles' preconditions (`retry_discipline`), which ordinary closures
  always meet: `cleanup` (prune/expire, which is destructive) may run only
  when cleanup is possible and `allocate` returned `Exhausted`, and `retry`
  may run only after `cleanup` returned counts that freed something. Each
  oracle is `FnOnce`, so each runs at most once. Production passes the same
  allocation closure as `allocate` and `retry`. Results (`retried`):
  - `Allocated { cleanup: None }`: the first allocation succeeded.
  - `Allocated { cleanup: Some(c) }`: the first allocation was exhausted,
    cleanup was possible and returned `c`, `freed_any(c)`, and the retry
    succeeded.
  - `Exhausted { cleanup: None, retried: false }`: the first allocation was
    exhausted and cleanup was not possible, so it never ran.
  - `Exhausted { cleanup: Some(c), retried }`: exhausted and cleaned up;
    `retried == freed_any(c)`, and when it retried, the retry was exhausted
    too. A cleanup that freed nothing is never followed by a retry.
  - Errors are an oracle's own, unchanged; a retry's error only after a
    cleanup that freed something (`exists c. cleanup.ensures(Ok(c)) &&
    freed_any(c)`).

  The glue reports `tried_cleanup` and the counts exactly when a cleanup ran
  (zero counts otherwise), and `AutomaticCleanupStatus::retried()` as the
  kernel's `retried`. The exhaustion message says "remaining blockers after
  the single retry" only after a retry; after a cleanup that freed nothing it
  says "automatic cleanup pruned 0 and expired 0 reservation(s), so the scan
  was not retried". (A body that retried unconditionally would not verify.)
- **`deferred_step`**: a key that vanished after planning is allocated fresh.
  Its metadata is re-inferred when a reservation existed at planning, and
  otherwise comes from the placeholder. A key that still exists gets exactly
  the planning decision (`lemma_deferred_existing_agrees_with_planning`).
- **`reported_port`** (`reported`): the last port that a deferred allocation
  produced at execution, otherwise the own port of the first create, update,
  or refresh in plan order. A refresh carries the port its reservation holds:
  in live execution it is read right after the refresh, and in a dry run it
  is read as stored. Read errors propagate. Live runs and dry runs both call
  it. Lemmas:
  - Every plan whose first action carries its own port, with no deferred
    port, reports that port. In particular, every successful reserve plan
    without a deferred allocation reports one.
  - A dry run reports the same port as live execution whenever no deferred
    action produced one and both run in the same snapshot.

Bugs fixed with this kernel:
- A cleanup that freed nothing was followed by a second full scan (with
  occupancy probes), so an exhaustion error cost two scans, a port that
  became free meanwhile (for example, a process released it) could be
  granted where the specification reports exhaustion, and the error claimed
  a retry. Pinned by
  `test_cleanup_that_freed_nothing_is_not_retried` and
  `cleanup_retry_runs_cleanup_at_most_once`
  (`trop/src/operations/reserve.rs`).
- The executor used to swallow errors when it re-read a refreshed
  reservation's port, so `trop reserve` could exit 0 with empty stdout. A
  refresh of a vanished reservation now fails with `NotFound`, read errors
  propagate, and a plan run after a concurrent reallocation reports the
  current port.
- An explicit `--project`/`--task` no longer triggers git discovery.
- `trop reserve --dry-run` now plans on a read-only connection inside a
  rolled-back transaction. It fails exactly like a live run, prints the plan
  and the port, and never creates, initializes, or migrates a database. With
  auto-init disabled, a missing database fails as in a live run. A dry run
  says when it cannot predict the outcome: a database that is uninitialized
  or needs migration, or an allocation deferred to automatic cleanup (a live
  run cleans up and, only if that freed anything, retries once). Opening a
  WAL database read-only may leave `trop.db-wal`/`trop.db-shm` behind; the
  contents never change.
- The reserve dry-run message and the plan descriptions said a live run
  would retry after cleanup unconditionally; they now say it retries only
  if the cleanup freed anything. Pinned by
  `dry_run_of_a_deferred_allocation_says_it_cannot_predict_the_outcome`
  (`trop-cli/tests/reserve_dry_run.rs`).

Trusted: the char-to-byte step from `identifier::lemma_trimmed_idempotent`
to the byte-level `norm` of `lemma_repeat_request_touches`, exercised by
`trimmed_intent_values_are_fixed_points`. `str`/byte equality is checked by
the exhaustive differential test
`existing_decision_matches_the_previous_rules`.
The oracle closures are total. The dry run's read-only connection cannot
write.

#### Shared cleanup policy (single and group reservations)

Glue: `trop/src/operations/reserve_group.rs` (`execute_group_allocation` calls
`cleanup_policy`).

- `cleanup_policy` (`cleanup_of`): each automatic cleanup runs unless its
  switch disables it. `reserve_policy` calls it for its cleanup part, and
  `execute_group_allocation` calls it for group allocations, so single and
  group reservations clean up alike *for the same switches*.
  `lemma_cleanup_policy_shared` is a definitional identity (`policy_of`'s
  cleanup part is `cleanup_of`, and a retry is possible unless both
  cleanups are disabled); with `reserve_policy`'s `policy ==
  policy_of(flags)` it applies to the policy production uses. The two paths
  do not read the same switches: `trop reserve` takes them from the
  configuration, the environment and its
  `--disable-autoprune`/`--disable-autoexpire`/`--disable-autoclean` flags,
  while `reserve-group` and `autoreserve` have no such flags (the
  specification lists them only for `reserve`) and read
  `disable_autoprune`/`disable_autoexpire` from the configuration and the
  environment only.
- Both paths share `allocate_with_cleanup_retry`, so both retry only after a
  cleanup that freed something. Both pass an unset `expire_after_days`
  through to `cleanup::automatic_selection`, which applies the 30-day
  default (the group path no longer has its own `Some(30)` literal).

### `verified::inference` — project and task names from repository facts

Glue: `trop/src/operations/inference.rs` (`repository_name`, `branch_name`,
used by `infer_project` and `infer_task`). Discovery, canonicalization, and
the HEAD lookup stay in the glue. The name is then trimmed and validated by
`identifier::check_identifier` (through `crate::identifier`), whose trimmed
result is kept, so an inferred name is canonical.

- `repository_name_prefix`: the parent directory's name when the canonical
  common directory is `.git`, and otherwise the common directory's own name
  (a bare repository), with exactly one trailing `.git` removed
  (`repo.git.git` becomes `repo.git`). The result is `None` exactly when the
  common directory is `.git` and its parent has no usable name.
- `branch_name_start`: `Some(11)` exactly for references under
  `refs/heads/`. The branch is the rest of the reference, slashes kept; other
  references name no branch.
- Every returned length or offset sits next to an ASCII byte, so it is a
  UTF-8 character boundary of the glue's `&str`, which slices with `str::get`.

### `verified::config` — configuration text, the `reservations` rules, lock-wait bounds, defaults, layering, placement, and discovery

Glue:
- `trop/src/config/environment.rs`:
  - Every numeric `TROP_*` variable: `TROP_PORT_MIN`/`MAX`/`MAX_OFFSET` through
    `exclusions::parse_u16`; `TROP_EXPIRE_AFTER_DAYS` through
    `parse_u32_decimal`; `TROP_BUSY_TIMEOUT` and its alias through
    `parse_u64_decimal`.
  - `parse_bool` through `parse_bool_word`, and `TROP_OUTPUT_FORMAT` through
    `parse_output_format`.
- `trop/src/config/schema.rs`: `OutputFormat::{code, from_code,
  parse_keyword}`; `Display` writes `output_format_keyword`.
- `trop/src/config/validator.rs`:
  - `validate_reservation_group` sorts the services by tag, copies them into
    `ServiceEntry`s, calls `check_config_group`, and maps each `GroupIssue` to
    its historical `(field, message)`.
  - `validate_source_document` calls `check_placement` and returns the
    effective `ReservationOverlay`.
  - The group checks report the env-name and non-convertible-tag
    diagnostics from the one message table in `trop/src/identifier.rs`
    (`environment_variable_name_message`,
    `NON_CONVERTIBLE_SERVICE_TAG_MESSAGE`).
  - The `maximum_lock_wait_seconds` and `cleanup.expire_after_days` checks use
    `lock_wait_seconds_ok` and `expire_after_days_ok`.
- `trop/src/config/merger.rs` (`merge_into_observed`) and
  `trop/src/config/effective.rs` (`apply_precise`): every scalar, occupancy and
  cleanup leaf goes through `overlay_leaf`; its `set` flag drives provenance
  recording.
- `trop/src/config/effective.rs`: flag and permission accessors
  (`flag_enabled`, `derive_permissions`) and the built-in defaults.
- `trop/src/config/builder.rs`: `kind_for_precedence`, the placement overlay,
  and `default_config` (kernel constants).
- `trop/src/config/loader.rs`: `discover_project_configs_effective` calls
  `first_config_level` with an existence-oracle closure over
  `Path::ancestors`, and loads the chosen level through `load_level`, which
  loads exactly the files `level_load_order` lists, in its order, at
  `level_file_precedence`; `load_with_project_file` loads the directory of a
  nominated `trop.yaml`/`trop.local.yaml` through the same `load_level`;
  precedence constants; `ReservationOverlay::{code, from_code}`.
- `trop/src/database/config.rs`: `MAX_BUSY_TIMEOUT_MILLISECONDS`/`SECONDS` come
  from the kernel, and `DatabaseConfig::check_busy_timeout` calls
  `busy_timeout_ok(as_secs, subsec_nanos)`. `Database::open` and
  `Database::validate` (through `validate_busy_timeout`) and the group dry
  run's `open_dry_run_snapshot` all call it.

Numbers:
- `exclusions::parse_unsigned(s, max)` generalizes the exclusion parser and is
  exact for every byte input: an optional `+`, then one or more ASCII digits
  (leading zeros allowed), value `<= max`. It cannot overflow.
- `parse_u16`, `parse_u32_decimal` and `parse_u64_decimal` are its
  `u16`/`u32`/`u64` instances. They decide every numeric `TROP_*` variable,
  so those share this syntax (untrimmed: `TROP_PORT_MIN=" 1"` is rejected).
  Exclusion text uses the same digit syntax after trimming each side.
- Not every number goes through it. YAML integer scalars (`ports`,
  `maximum_lock_wait_seconds`, `cleanup.expire_after_days`, `reservations`
  base/offset/preferred) are parsed by `serde_yaml`, which also accepts
  `0x`/`0o`/`0b` and rejects leading zeros (`min: 05000` is a string). On
  the command line only `trop exclude`'s argument reaches `parse_u16`
  (through `PortExclusion`'s `FromStr`). `reserve`'s `--port`, `--min` and
  `--max` are strings parsed by `str::parse::<u32>` and then range-checked
  (`parse_port_string` in `trop-cli/src/commands/reserve.rs`). The typed
  arguments (`scan --min`/`--max`, the `port` of `assert-port` and
  `port-info`, `expire`/`autoclean --days`, `--busy-timeout`) are parsed by
  `clap` (as an `i64`, then range-checked).
- `lemma_decimal_text_accepted` (completeness): every value up to `max`, with
  any leading zeros and an optional `+`, parses back to itself.

Words:
- `parse_bool_word` accepts exactly `true`/`1`/`yes`/`on` and
  `false`/`0`/`no`/`off`, ignoring ASCII case. The two lists are disjoint, and
  the result depends only on the text up to ASCII case.
- `parse_output_format` accepts exactly the four keywords, ignoring ASCII case.
  Keywords are injective, and `Display` text parses back to the same format.
- The previous Unicode `to_lowercase` is equivalent for these words. An
  exhaustive test over all scalar values shows that the Kelvin sign is the only
  non-ASCII char whose lowercase is ASCII.

`reservations`:
- `check_config_group` succeeds exactly for `config_group_valid`:
  - the base is not 0;
  - every tag is a valid identifier;
  - no preferred port is 0;
  - every service resolves to a portable name: explicit, or derived from the
    trimmed tag;
  - trimmed tags, defaulted offsets, present preferred ports and ASCII-case
    collision keys are pairwise distinct.
- Otherwise it reports the first issue in the order given, in the validator's
  historical check order. The glue sorts by tag, so the error no longer depends
  on `HashMap` order.
- `lemma_config_group_permutation`: acceptance is invariant under any
  reordering.
- `lemma_config_group_consequences`: at most one service omits its offset, and
  resolved names are portable.
- `lemma_valid_config_group_is_valid_request`: every valid non-empty group,
  requested with its tags as written, its defaulted offsets and its preferred
  ports (`requested_as`), passes reserve-group's request validation
  (`group::group_request_valid`, which `validate_group_request` accepts
  exactly).

Lock wait:
- `lock_wait_seconds_ok(s)` is `s * 1000 <= i32::MAX`.
- `busy_timeout_ok(secs, nanos)` is `secs * 1000 + nanos / 10^6 <= i32::MAX`,
  total and overflow-free.
- `MAX_LOCK_WAIT_SECONDS = MAX_BUSY_TIMEOUT_MILLIS / 1000 = 2147483` is derived
  and tight.
- Every accepted lock wait, and every shorter remaining duration, passes the
  database check.

Defaults and flags:
- `DEFAULT_EXPIRE_AFTER_DAYS = 30`, `DEFAULT_LOCK_WAIT_SECONDS = 5`,
  `DEFAULT_OUTPUT_FORMAT = Table`, all proved valid.
- Unset flags are off.
- `derive_permissions`: `allow_change` grants both permissions. It is monotone
  and denies by default.

Layering:
- `overlay_leaf` (exec, called by the merger) takes the layer's value if it
  sets one, otherwise the value below, and reports whether the layer set it.
- The rest are lemmas about spec-only models (`fold_overlay`,
  `accumulate_exclusions`, `group_winner`). No exec code is checked against
  them; the builder and merger implement them unverified (see "Not proved").
  - The overlay is a monoid with `None` as identity.
  - In the model, the fold over the layers equals the highest-precedence
    layer that sets the leaf (the last layer whose overlay reported "set"),
    or the default when no layer sets it.
  - Folding is compositional.
  - In the model, accumulated `excluded_ports` exclude exactly the union of
    the layers' ports.

Files:
- `check_placement`: a user file may name neither a project (checked first) nor
  a group, and its `reservations: null` is inert. Other files keep their
  overlay.
- `kind_for_precedence` (`spec_kind_for_precedence`): 1 is User, 3 is Local,
  anything else is Project. User < Project < Local.
- `lemma_group_winner` (model): `group_winner` is the last `Replace` with
  only `Inherit` after it.
- `first_config_level` returns the nearest level at which the oracle reported
  `trop.yaml` or `trop.local.yaml`, with what it reported there, after it
  reported neither at every nearer level. It loads nothing.
- `level_load_order(has_project, has_local)` is the load plan of a level:
  exactly the files reported present (`trop.yaml` iff `has_project`,
  `trop.local.yaml` iff `has_local`), each once (the length is the number
  present), in strictly ascending `level_file_precedence`. So a level's
  `trop.yaml` (precedence 2) is loaded before its `trop.local.yaml`
  (precedence 3), the order `ConfigLoader::discover_project_configs`
  returns. The overlay order is not decided here: the loader functions the
  builder calls (`load_all_effective`, `load_with_project_file`) re-sort the
  loaded sources by precedence in glue (`sort_by_key`) before the builder
  overlays them in that order, which puts `trop.local.yaml` over
  `trop.yaml`. `lemma_level_file_kinds` relates the two kernels production
  composes: `kind_for_precedence` maps the precedence `level_file_precedence`
  gives `trop.yaml` to Project and the one it gives `trop.local.yaml` to
  Local, and no level file's to User. Passing the precedence from the
  loader's source to the builder's `kind_for_precedence` call is glue.
  `lemma_precedence_order` only checks the constants: user < project <
  local.

Not proved:
- `.trop.yaml` discovery (out of scope).
- The explicit project-file plan (beyond the shared `load_level`) and the
  `is_tropfile` computation.
- The `project`/`reservations` leaves, which still clone when present.
- Provenance bookkeeping.
- That production layering implements the models above. The effective
  reservation group is chosen by the builder's loop (replace in
  `merge_config`, clear on `Clear`), not by `group_winner`; exclusions
  accumulate through `Vec::extend` at three sites, not through
  `accumulate_exclusions`; the environment layer assigns its scalar leaves
  directly, not through `overlay_leaf`; and the cleanup leaf still
  materializes `Some(CleanupConfig { .. })` for an empty source section.
- That `apply_precise` and `merge_into_observed` agree.
- That the glue maps `LevelFile::Project`/`Local` to the names `trop.yaml`
  and `trop.local.yaml` (`level_file_name`, one `match`, pinned by
  `test_level_files_load_in_kernel_order`), and that `load_all_effective`'s
  stable sort by precedence keeps the plan's order (it is already
  ascending).
- The connection's retry timeout and the PRAGMA readback; that glue is outside
  this package.

Trusted:
- std `from_str` equivalence of `parse_u16`/`parse_u32_decimal`/
  `parse_u64_decimal`: differential tests and a proptest
  (`prop_decimal_parsers_match_std`).
- `serde_yaml`'s parsing of the YAML numeric fields.
- `Duration::as_millis` semantics: boundary test.
- `Path::ancestors`/`exists`.
- The group glue's sort and message mapping: differential tests against the
  pre-kernel validator, and
  `group_env_diagnostics_have_the_shared_field_and_text` for the shared
  env-name diagnostics.
- vstd's string-literal axioms, used for the keyword text.
- No new escape hatches.

### `verified::config_rewrite` — key-preserving configuration rewrites and the exclusion write target

Glue:
- `trop/src/port/exclusions.rs`: `rewrite_excluded_ports`, `excluded_ports_in`,
  the re-exported `select_write_target`, `WriteScope` and `WriteTarget`, and
  the private loader-style reader `read_configuration`.
- `trop-cli/src/invocation.rs`: `config_file_for_write`.
- The writers in `exclude.rs`, `compact_exclusions.rs` and `scan.rs`, which
  share `read_config_document`, `current_exclusions`, `write_exclusions` and
  `error_detail`.

- The plan (`plan_key_rewrite`): a configuration file is read as a YAML
  mapping, and the kernel plans the rewritten mapping over the entries' "is
  `excluded_ports`" marks (`planned`).
  - Keys are compared as the configuration reader identifies them: by text,
    with YAML tags ignored, so `!tag excluded_ports` is the field.
  - Every other entry is kept, in order.
  - The first `excluded_ports` entry is replaced in place, or the key is
    appended when absent.
  - Every `Keep(i)` names an existing, unmarked entry, and exactly one step
    writes the new value.
  - A tag on the whole document is kept.
  - A document whose only content is an empty plain scalar with a core tag
    (`!!null`, `!!str`, `!!int`, ...) is not a mapping, but the loader reads
    it as a mapping with no entries. The glue rewrites it as an untagged
    empty mapping (a core tag names the scalar's type; any other tag, as in
    `--- !foo`, is kept). `trop exclude` used to fail on such a file with
    "Invalid YAML" or "not a YAML mapping". This applies only when the root
    has no entries, counted through `serde_yaml`'s `deserialize_map`, the
    call behind the loader's `deserialize_struct` (`root_has_no_entries`). A
    mapping holding a key or value `serde_yaml` cannot read as a `Value` is
    refused as invalid YAML although the loader reads it, because its
    entries could not be kept. Examples: `cleanup: !!bool` (which the loader
    reads as an empty `cleanup` section), `!!int project:`, `project:`
    followed by `!!null allow_change:`. Pinned by
    `rewrite_refuses_mappings_whose_entries_it_cannot_read` and
    `excluded_ports_in_reads_documents_exactly_as_the_loader_does`
    (`trop/tests/exclusion_edits.rs`).
- What the plan does to a document:
  - With distinct keys, the rewritten document is `doc.update(f, (key, value))`
    (`lemma_rewrite_replaces_in_place`), or `doc.push((key, value))` when the
    key was absent (`lemma_rewrite_appends`).
  - Rewriting a key with its current value is the identity
    (`lemma_rewrite_unchanged_is_identity`), and the key stays unique
    (`lemma_rewrite_keeps_key_unique`).
  - The distinct-keys precondition holds because the original must read as a
    `Config`, and the reader rejects repeated fields.
- Overlay preservation (`lemma_rewrite_preserves_other_keys`):
  - Every other key keeps its exact value, and so its state (`KeyState`:
    absent, null or present) and its overlay meaning (`overlay_of`) under
    either reading of `null`.
  - An omitted `reservations` key in `trop.local.yaml` keeps inheriting the
    group of `trop.yaml`; it does not become an explicit `null` that clears it.
  - The edited key holds the new value.
- `select_write_target(scope, has_local, has_project)`:
  - The `Global` scope writes the user `config.yaml`.
  - Otherwise the loaded `trop.local.yaml` is used, then the loaded
    `trop.yaml`.
  - With neither, `Project` (`trop exclude` without `--global`) yields no file
    and the command fails. `ProjectOrGlobal` (`scan --autoexclude`) falls back
    to the global file.
  - The global file is never chosen for the `Project` scope.
- Trusted, and checked at run time:
  - `serde_yaml` parsing and emitting, `Mapping` insertion order, and the
    identification of keys (text, tags ignored).
  - The loader's reading of a file, reproduced by `read_configuration`,
    which reads exactly as `ConfigLoader::load_document` does
    (`serde_yaml::from_str` into `Config`, plus the absent/null/set state of
    `reservations`, as in `ConfigLoader`'s private `ReservationPresence`),
    with no special cases. A document with no content (empty, comments
    only, `---`), or whose only content is an empty plain scalar with a tag
    (`!!null`, `!!str`, `--- !foo`), is the default configuration in both;
    an explicit null scalar (`null`, `~`, tagged or not: `!!null null`,
    `!!null ~`, `!foo null`) is rejected by both. Previously
    `read_configuration` mapped every null document to the default
    configuration, so `compact-exclusions` reported "No exclusions to
    compact" for a file the loader rejects. Pinned by
    `excluded_ports_in_reads_documents_exactly_as_the_loader_does`
    (`trop/tests/exclusion_edits.rs`) and
    `compact_exclusions_reads_null_documents_as_the_loader_does`
    (`trop-cli/tests/exclusion_edits.rs`).
  - `rewrite_excluded_ports` refuses to write unless all three checks hold:
    1. The emitted text reads back as exactly the planned document, with
       top-level keys in order.
    2. Every unmarked key reads back with its exact value, exactly one key
       reads as `excluded_ports` and holds the new list, and no other key was
       added.
    3. The loader-style reading of the text equals that of the original with
       only `excluded_ports` replaced.
- Not preserved:
  - YAML comments and formatting (a `serde_yaml` limitation).
  - Non-canonical scalar spellings. A string-typed value written as an unquoted
    non-string scalar (`project: 0x10`) would change meaning when re-emitted,
    so check 3 refuses the write and the file is left alone.
  - Not rewritable: a mapping the loader reads although `serde_yaml` cannot
    read one of its keys or values as a `Value` (see the plan).
    `trop exclude` fails with "Invalid YAML" and leaves the file alone.
- Not proved:
  - The composition with the loader (`load(rewrite(doc))`). Check 3 covers it
    at run time.
  - That `scan` passes the right occupied and reserved lists to the
    verified unreserved filter (`exclusions::unreserved_ports`, through
    `trop::cli_support::unreserved_ports`); see "Additions to
    `verified::exclusions`".
- Escape hatches: none. The ghost enums `KeyState` and `Overlay` carry
  `#[allow(dead_code)]` because ordinary builds keep their declarations.

### `verified::storage` — stored keys and rows, the read/write predicates, lookups, prefix queries

Glue: `trop/src/database/operations.rs` (`row_to_reservation`,
`encode_reservation`, every write path, among them
`update_metadata_and_last_used_simple` through `encode_metadata_update`,
`get_reservation` and the exact-path lookups,
`get_reservations_by_path_prefix`, the busy-timeout readback),
`trop/src/database/transaction.rs` (batch create/delete),
`trop/src/reservation.rs` (`ReservationKey::storage_key`),
`trop/src/path/mod.rs` (`storage_platform`). SQLite queries, value storage
classes and `Path::to_str` stay in the glue.

- Ports and timestamps: `decode_stored_port` accepts exactly `1..=65535` (the
  schema `CHECK`). `encode_unix_secs` and `decode_unix_secs` are each `Ok`
  exactly on `0..=MAX_STORABLE_UNIX_SECS` and preserve the value. The codec
  therefore round-trips, and the write domain equals the read domain.
- Paths: `classify_stored_path` decides `path_verdict`. On Unix a path is
  storable exactly when it is canonical (`spec_is_canonical_abs`), i.e. a fixed
  point of normalization, and every normalization result is storable.
  Elsewhere, `std`'s `is_absolute` and component fixed point decide native
  paths, and `/`-rooted spellings are judged by the portable form.
- Keys: `encode_storage_key` is the write-side check. It succeeds exactly for
  storable keys. The stored path is the key's own text (non-UTF-8 paths are
  rejected, never aliased) and the stored tag is `encode_stored_tag`. The write
  predicate is the read predicate (`lemma_storable_key_iff_decodes`), stored
  keys are injective (`lemma_stored_key_injective`), and on Unix equal stored
  columns mean equal components and tags.
- Rows: `encode_record` is the whole write path for one reservation. An error
  is the first failing check in field order (`first_record_error`: key, port,
  project, task, creation time, last use; the glue rejects a pre-epoch time
  first). Every encoded row decodes to exactly the record's fields
  (`lemma_encoded_record_round_trips`; `sticky` is not persisted). In the
  other direction, `lemma_decoded_row_re_encodes` proves that a decodable
  row's key is storable and its tag re-encodes to the stored tag text (the
  tag codec bijection); its port and timestamp facts restate `row_decodes`,
  and it does not state `encodes(f, row)` for a decoded record.
- Metadata updates: `encode_metadata_update(key, project, task,
  last_used_secs)` builds the values bound by the `UPDATE` of a stored row's
  project, task and last use (`StoredMetadataUpdate`, private fields).
  - It succeeds exactly when `first_update_error` is `None` (checked in the
    order project, task, last use), and otherwise returns that first error.
  - The key columns are the `StoredKey`'s, the new values are the inputs,
    and the last use is storable (`update_values_valid`).
  - `lemma_update_checks_are_record_checks`: for a record whose key, port
    and creation time are storable, `first_record_error` equals
    `first_update_error` of its project, task and last use. Inserts and
    updates therefore apply one rule.
  - `lemma_update_preserves_decoding`: updating a row whose values decode
    leaves a row that decodes, to the new values.
- Write paths: `StoredKey`, `StoredRow` and `StoredMetadataUpdate` have
  private fields. So every bound key comes from `encode_storage_key`, every
  inserted row from `encode_record`, and every metadata update from
  `encode_metadata_update`.
  - The last-use-only `UPDATE` binds a `StoredKey` and an
    `encode_unix_secs` result.
  - Every reservation write path returns `Error::Validation` before it
    writes a value the reader would reject.
  - Keys and supplied values are checked before any SQL runs.
    `update_last_used` converts the current time after its
    `BEGIN IMMEDIATE`, before the `UPDATE`.
  - As for `encode_record`, the glue rejects a time before the Unix epoch
    (no `u64` seconds) before calling the kernel.
- Lookups: `lookup_key` is the key's exact text. Among rows whose tag decodes,
  it matches exactly the rows holding the requested key, so a lookup never
  returns another key's row (a `Some("")` tag never aliases the untagged one)
  and never misses the key's own row. There is no lookup only when no row can
  hold the key (no UTF-8 text, or a `Some("")` tag). A corrupt row stored under
  a non-storable key's text is still found and reported ("Corruption Detection,
  But No Recovery").
- Prefix queries (Unix): `prefix_query` maps a `Path::starts_with` prefix to
  `Everything`, `Nothing` (relative, or containing `..`) or the canonical key
  of its segments. On canonical stored paths, `canonical_is_component_prefix`
  with the key is exactly the `starts_with` model (`prefix_query`'s
  postcondition). Every match starts with the key's bytes
  (`lemma_prefix_query_exact`, which holds by the definition of
  `spec_component_prefix`), so the byte-exact prefilter
  `instr(path, key) = 1` never drops a match. Non-Unix targets test each stored
  text with `Path::starts_with` and decode only the matches (unverified).
- `busy_timeout_millis` is `max(v, 0)`.

Trusted: `MAX_STORABLE_UNIX_SECS` is chrono's `MAX_UTC` (pinned by a test).
`instr(path, key) = 1` is true exactly when the stored text's bytes start with
the key's: no wildcards, case folding, NUL truncation or pattern-length limit
(tested, including a 60,000-byte key). TEXT `=` is text equality. The
`std::path` model of `starts_with` and `is_absolute` on Unix. `Path::to_str`.
On Windows, the `std` facts in `PathPlatform::Native`. rusqlite's value
mapping. The platform `SystemTime` range on read.

### `verified::expiry` — expiry rule and SQL cutoff

Glue: `Database::find_expired_reservations` (`expiry_cutoff(now,
max_age.as_secs())`), `Reservation::is_expired` (`duration_exceeds`).

- `expired(now, last, max) := now - last > max`. A future last use is never
  expired, an age of exactly `max` is not expired, and a larger `max` expires a
  subset.
- `expiry_cutoff`: `last_used_at < cutoff` selects exactly the expired `i64`
  timestamps, computed without overflow. Huge maximum ages select nothing. (The
  old `as i64` cast wrapped ages of 2^63 s or more and expired everything.)
- `duration_exceeds` is `Duration`'s order. For a whole-second age it agrees
  with `expired` (`lemma_whole_second_age_agrees`), so a caller that captures
  `now` in whole seconds agrees with the SQL query. `Reservation::is_expired`
  measures a sub-second age, so it can differ from `find_expired_reservations`
  by up to one second at the boundary; this is not claimed.

### `verified::schema_version` — schema versions, the v1 preflight, table layouts

Glue: `trop/src/database/migrations.rs` (`get_schema_version`,
`check_schema_compatibility`, `initialize_schema`, the post-lock recheck,
`collect_legacy_violations`, the published-v1 layout check),
`trop/src/database/validation.rs` (`validate_schema_version`,
`validate_metadata_rows`, `column_shapes_match` / `columns_match`). The glue
observes the metadata in one snapshot: a read transaction, or the caller's
transaction. It records whether the `reservations` table exists, the first
`schema_version` value, and whether a second row exists, then maps dispositions
to the existing errors. Opening, `get_schema_version` and the post-lock
recheck render the kernel's verdicts through one message table in
`trop/src/database/migrations.rs`: `version_corruption` (a `Corruption`
verdict) and `unsupported_version` (a newer version). Validation
(`schema_version_verdict` in `trop/src/database/validation.rs`) calls the
same two functions, so opening, the post-lock recheck and validation report
the same text for the same metadata by construction.
`trop/tests/schema_version_messages.rs` pins the texts for type and
row-count corruption and for a newer version, the text opening reports for
a missing metadata table ("the metadata table is missing although a
reservations table exists"; validation first reports the table's missing
columns), and that `validate_metadata_rows`' diagnostics name the key once
(`key="other"`, no longer `key=key="other"`;
`metadata_row_diagnostics_name_the_key_once`).

- `parse_i32_decimal` parses exactly `i32::from_str`'s grammar and range.
- `classify_for_open`: a database is initialized only when neither table
  exists. A `metadata` table without exactly one integer `schema_version` row,
  or reservations without metadata, is corruption and is never restamped.
  Initialization (including the public `initialize_schema`) is decided on an
  observation made inside the same `IMMEDIATE` transaction that writes, so
  concurrent first opens neither misreport corruption nor restamp. A newer
  version is refused by opening, the post-lock recheck and validation. Only
  version 1 migrates. Opening finds the schema current exactly when validation
  finds it valid. `get_schema_version` returns 0 only for an uninitialized
  database; a stored zero is `MigrationRequired { found: 0 }`.
- `classify_legacy_row`: each preflight category has an exact meaning. A v1 row
  violates none exactly when the migration copies it faithfully: every value
  already has its v2 storage class, and the copied row satisfies the v2 decoder
  and keeps its key. The preflight therefore blocks exactly the rows the
  migration would not copy faithfully. That includes every row that would make
  the migrated database fail validation, and also a lone `''` tag or a `TEXT`
  port the copy would change.
- `columns_match`: equal lengths and, at each position, byte-equal names,
  declared types equal ignoring ASCII case, equal `NOT NULL`, no hidden column,
  and equal primary-key position (when checked). `validate_metadata_rows`
  re-classifies the rows it scanned with `classify_for_validation` instead of
  re-checking them by hand.

Trusted: `i32::from_str` equivalence (differential test); SQLite metadata
observations, `typeof` storage classes, and one-snapshot reads within a
transaction (tested deterministically and under concurrent first opens).

### `verified::escape` — escaping stored text in diagnostics

Glue: `escape_text` in `trop/src/database/operations.rs` and
`trop/src/database/validation.rs`.

- `escape_default` computes `escape_text`, the specification of
  `chars().flat_map(char::escape_default)`.
- Every output char is printable ASCII. Quote-safe: stored text inside
  `key="..."` cannot close the quotes or forge fields. Injective.

Trusted: equality with `char::escape_default`, checked exhaustively.

### `verified::migrate` — migration conflicts, the action schedule, path rebasing, and the migration guard

Glue: `trop/src/operations/migrate.rs` (`MigratePlan::build` /
`build_with_connection`, `to_operation_plan`, `execute_migrate`,
`calculate_new_path` on Unix, `KeyTable` interning) and
`trop-cli/src/commands/migrate.rs`. The glue interns every source and
destination key to a dense id by its database identity: stored path text, and
stored tag text. The same id lists feed `guard_moves`, `classify_conflicts` and
`schedule_migration`. The database is not modeled.

- `classify_conflicts` returns, strictly increasing, exactly the items whose
  destination exists and is not itself a source (`is_conflict`). It fails
  exactly on malformed key lists, and `key_error_found` specifies the error
  exactly:
  - a length mismatch first (`LengthMismatch`, also for a `dest_exists`
    mismatch), pinned in both directions;
  - then the first problem a left-to-right scan of the sources meets:
    - `KeyOutOfRange` when it is an out-of-range id
      (`first_problem_out_of_range`);
    - `DuplicateSource { first, second }` when it is a repeat
      (`first_problem_repeat`: the ids before `second` are in range and
      distinct, and `second` repeats `first`);
  - then, with in-range distinct sources, the first problem in the
    destinations, found the same way (`KeyOutOfRange` or `DuplicateDest`).

  `lemma_first_problem_unique`:
  - the first problem is unique: two out-of-range positions coincide, two
    repeats name the same items, and an out-of-range first problem excludes
    a repeat;
  - a list that is not in range and distinct has one.

  So the key lists determine the error variant and the items it names. With
  three keys, sources `[0, 0]` and destinations `[9, 1]` give
  `DuplicateSource { first: 0, second: 1 }`, never `KeyOutOfRange`
  (`test_key_errors_follow_scan_order`).

  Lemmas:
  - `lemma_conflicts_are_overwrites`: under a consistent snapshot, the
    conflicts are exactly the existing non-source rows the migration replaces.
    Without conflicts, every other existing row survives unchanged; this is the
    specification's "would overwrite an existing reservation".
  - `lemma_disjoint_conflicts`: between disjoint trees, a conflict is exactly
    an existing destination.
  - `lemma_self_migration_is_identity`: `migrate --from P --to P` has no
    conflict and changes nothing.
- `schedule_migration` returns exactly `spec_schedule`: delete every source,
  then every destination that is not a source, then create `0..n`. Its errors
  are specified as above (without `dest_exists`). Its postcondition, for
  every store:
  - running the plan yields `moved`: each payload at its destination
    (overwriting what was there), the other sources gone, every other row
    untouched;
  - from a store with unique ports, with payloads read from it, every step is
    safe. Each create targets an absent key and a port no live row holds, so
    neither the primary key nor `UNIQUE(port)` can abort the transaction.

  Generic lemmas:
  - `lemma_schedule_correct<R>`;
  - `lemma_schedule_port_safe<R>`;
  - `lemma_schedule_shape`: deletes precede creates, each item is created once,
    the deleted keys are exactly sources ∪ destinations, and no key is deleted
    twice.

  `lemma_legacy_schedule_loses_data`: the previous interleaved schedule loses a
  row when a destination is another item's source. That was the `W/a` → `W/a/b`
  bug.
- `guard_moves`: the unrelated-path guard over every source and every
  destination id. When the guard applies, success means each key is lexically
  related to the working directory (`spec_between` is not `Unrelated`, decided
  in the kernel) or permitted by the physical fallback. A refusal names the
  first refused key, sources first.
- `rebase_canonical` (Unix `calculate_new_path`):
  - It returns `Some` exactly when all three paths are canonical and `from`
    is a component prefix of `orig`.
  - The prefix is decided by `path_relation::canonical_is_component_prefix`,
    the executable component-prefix test on canonical paths that the
    storage prefix query also uses as its post-filter.
  - The result is canonical, and its segments are `spec_rebase` of the
    inputs' segments, so the second `normalize` is a `debug_assert`.
  - `lemma_rebase_canonical_injective`: distinct sources never share a
    destination.

Glue guarantees: `execute_migrate` requires a plan from `MigratePlan::build`.
It rebuilds the plan inside its `IMMEDIATE` transaction and refuses to run if
the moves, the moved reservations' data (port, project, task, timestamps) or
the conflicts changed since planning. So the snapshot hypotheses (`snapshot`,
`exists_observed`) hold for the store being modified, and the written data is
what the plan showed.

Trusted: delete and create act as `Map::remove` and insert-into-absent-key, and
the transaction is atomic. A key with two spellings cannot reach the planner:
stored paths must be canonical, and others are reported as corruption on read.
`DuplicateDest` handling is a defensive check, exercised with hand-built plans.
Outside Unix, `calculate_new_path` keeps `strip_prefix` + `join` + `normalize`,
which `test_calculate_new_path_matches_std` compares against the kernel.

### `verified::release` — the release tag table, row selection, and the unrelated-path guard

Glue: `trop/src/operations/release.rs` (`ReleasePlan::build_plan`,
`ReleaseTagSelection`, `path_guard`, `stored_tag`, `stored_target`) and
`trop-cli/src/commands/release.rs`. Paths are `super::path_relation` tokens
from `crate::path::relationship::path_tokens`, the same tokens
`PathRelationship::between` uses. The target is passed as its stored
identity: `stored_target` gives its tokens when
`storage::classify_stored_path(text, storage_platform(path))` is
`Canonical`, and `None` otherwise. This gate is unverified glue (see
"Trusted").

- `tag_selector` implements the specification's table:
  - `--tag` with `--untagged-only` is an error;
  - neither flag selects all tags;
  - `--untagged-only` selects the untagged reservation;
  - `--tag t` selects `t`.

  `plan_flags` / `selector_from_plan` convert to the planner's `(key tag, all
  tags)` pair and back without loss (`lemma_plan_flags_round_trip`).
- `select_and_guard` decides, in one step, exactly which candidate rows a
  release deletes: strictly increasing, at the target path (recursively, at a
  component descendant) with a tag the selector accepts (`release_selects`).
  When the guard applies, it also decides that every one of those rows is
  lexically related to the working directory or permitted by the physical
  fallback; a failure names the first refused selected row. So a recursive
  release can no longer delete a reservation sideways of the working directory
  without an override.
- The target reaches the kernel as an optional token sequence
  (`target_view`); the kernel never sees its text. The glue `stored_target`
  passes:
  - its tokens when the target is spelled as a stored key's path, that is,
    when the storage kernel's write-side path check accepts it (on Unix,
    exactly the canonical absolute spellings, decided by the verified
    recognizer);
  - `None` otherwise, including a path that is not UTF-8.

  `row_selected` requires `Some`, so a `None` target selects nothing in
  every mode (exact key, all tags, recursive) and with every selector
  (`lemma_unstored_target_selects_nothing`, which follows from
  `row_selected`'s definition). With the glue's gate, a target spelled any
  other way (`/w/a/`, `/w/./a`, `/w//a`, `/w/a/.`) therefore selects
  nothing. The release then reports it as already released. This is the
  exact-text identity that `storage::lookup_key` uses. Before this change,
  the exact-key and all-tags queries found nothing for `/w/a/`, while
  recursive mode deleted the `/w/a` rows.
- Selection lemmas:
  - non-recursive ⊆ recursive;
  - `--tag` / `--untagged-only` ⊆ all tags;
  - a path differing from the target in any target component is never selected
    (`lemma_component_aware`, with the `/work/a` vs `/work/ab` token instance).
- `guard_paths` checks the requested path first; this is the unchanged
  single-key decision. `gate_path` is the per-key gate. The lexical relation is
  decided in the kernel by the verified `path_relationship`. Only a lexically
  unrelated path consults the fallback, `Database::validate_path_relationship`:
  its canonical, physical comparison stays an unverified oracle. `PathGuard`
  carries `--force`, `--allow-unrelated-path` and the working directory's
  tokens. With either override the fallback is never consulted.

The SQL queries only produce candidates; the kernel selects among them.
Recursive mode uses the prefix query, all-tags mode the exact-path query, and
exact-key mode the key's own row. For a canonical target, stored-text
equality and token equality coincide, so every candidate query returns a
superset of the rows the kernel selects. For any other target the glue
passes `None`, and the kernel selects nothing. The queries still run for a
non-canonical target, so a corrupt row stored under that exact text is
reported as before. The tag is compared as stored. A struct-literal
`Some("")` tag selects nothing, since decoded rows never carry it (and
`storage::lookup_key` finds nothing for it).

Trusted:
- token equality and prefix coincide with `Path` component equality and
  `Path::starts_with` (`test_path_tokens_agree_with_std`);
- `stored_target`'s gate: the kernel receives `None` exactly for a target
  not spelled as a stored key's path (pinned by
  `test_noncanonical_target_selects_nothing_in_every_mode`);
- for a canonical target, the candidate queries return a superset of the
  selected rows (pinned by
  `test_noncanonical_target_selects_nothing_in_every_mode` and the
  component-aware tests);
- on non-Unix targets, the `std` facts that `storage_platform` feeds the
  storage kernel (`is_absolute`, the component fixed point), as for every
  stored key;
- the working directory read for the tokens is the one the fallback reads.

#### Reserve adopts the unrelated-path guard

Glue: `ReservePlan::check_path_relationship` in
`trop/src/operations/reserve.rs`:
- `path_guard(false, !policy.check_path_relationship, true)`, where `policy` is
  the verified `reserve_policy`;
- `guard_paths` over `path_tokens(path)`;
- `Database::validate_path_relationship(path, false)` as the physical fallback.

- `lemma_reserve_guard_is_policy`: the guard's flag rule (`!force &&
  !allow_unrelated_path`) is reserve's verified policy field
  `check_path_relationship`. The guard is built from that field, so it applies
  exactly when the policy says so. Either override bypasses it, and then the
  fallback is never consulted.
- The lexical relation is decided by `path_relation`: same, ancestor or
  descendant is allowed.
- `test_path_guard_matches_previous_check` is a differential test. It shows
  identical accept/refuse decisions and errors to the previous check for each
  tested path under all four flag combinations. The paths include `.`/`..`
  components, a trailing separator, and a string prefix that is not a component
  prefix.

### `verified::shell` — `--shell` names, shell detection, export lines

Glue: `trop/src/output/shell.rs` (`ShellType::{from_string, detect,
format_export}`, `ShellType::kind`). `detect` reads the environment:
`ZSH_VERSION`, `FISH_VERSION` and `PSModulePath` count when set to valid
Unicode, and `SHELL` is read with `var_os` plus a lossy conversion. It maps
"undetermined" to a validation error asking for `--shell`.

- `parse_shell_name` (`ShellType::from_string`) is exactly the alias table
  `shell_alias`: `bash`/`sh`, `zsh`, `fish`, `powershell`/`pwsh`/`ps`, compared
  ignoring ASCII case.
  - `lemma_shell_alias_exact`: the alias sets are disjoint even ignoring case.
  - Accepted input is 2 to 10 ASCII letters, so it is never trimmed or
    prefix-matched.
  - Parsing ignores ASCII case (`lemma_shell_alias_case_insensitive`).
  - The unknown-shell error shows the input through `display_field`.
- `classify_shell_path` / `detect_shell` (`ShellType::detect`):
  - Precedence is `ZSH_VERSION`, then `FISH_VERSION`, then `PSModulePath`,
    then the basename of `SHELL` (the bytes after the last `/` or `\`, with a
    trailing `.exe` removed, ignoring ASCII case).
    - The order is inherited from the original detection code. It is a
      maintainer decision, not a verified property of correct detection.
    - Any of the three variables outranks `SHELL`.
    - Windows defines `PSModulePath` system-wide, so processes normally
      inherit it. So on Windows, detection normally answers PowerShell, even
      in Git Bash, MSYS2 or Cygwin with `SHELL=/usr/bin/bash`. The same holds
      for any child of `pwsh` on Unix. There a POSIX shell needs
      `--shell bash`.
  - `lemma_basename_ignores_directory`: directory components never change the
    result, so `/home/selfish/bin/bash` is bash (`lemma_selfish_bash_is_bash`).
  - The result table (`lemma_program_table_posix`,
    `lemma_program_table_others`): `zsh`; `fish`; `pwsh`/`powershell`; and
    `bash`, `sh`, `dash`, `ksh`, `mksh`, `ash` or `yash`, which take bash
    syntax.
  - `lemma_detection_undetermined`: detection fails exactly when no version
    variable is set and `SHELL` is unset or unknown. (The CLI test harness pins
    `SHELL=/bin/bash` so tests do not depend on the host.)
- `export_line` (`ShellType::format_export`) and `push_export_line` (group
  blocks) render `export N=V`, `set -x N V` or `$env:N="V"`.
  - `format_export` succeeds exactly for a portable name and an inert value: a
    non-empty word of `[A-Za-z0-9_./:@+,-]` not starting with `-`. Every port's
    decimal text is inert.
  - Model-lexer theorems:
    - bash/zsh: the words are exactly `[export, N=V]`, and the assignment
      splits at its first `=` into `(N, V)`.
    - fish: the words are exactly `[set, -x, N, V]`.
    - PowerShell: `ps_assignment` reads `(N, V)`, and the only non-word bytes
      are `$` at 0 and the two quotes.
  - Word bytes are never metacharacters (`lemma_word_bytes_are_inert`).
  - Every shell's model reads the line as exactly its assignment
    (`lemma_export_round_trip`), so distinct assignments give distinct lines
    (`lemma_export_line_injective`). Lines are ASCII, without `\n` or `\r`.

### `verified::render` — group blocks, deterministic order, display escaping

Glue: `trop/src/output/formatters.rs` (`render_assignments` for
`ExportFormatter`/`DotenvFormatter`; `HumanFormatter`) and
`trop/src/output/mod.rs` (`display_field`, `tab_row`).

- `render_assignments` builds one `GroupEntry` per allocation: the tag's bytes,
  the name resolved by `EnvironmentVariableName::resolve` (`None` on failure),
  and the port.
- It maps `Unresolved{source}` to that entry's own resolution error and
  `Collision` to the constant uniqueness message.

- `split_on` is `str::split` for one separator. `lemma_split_join`: splitting a
  join of separator-free parts returns the parts.
- `u16_decimal` / `u16_decimal_string` (export, dotenv and human port text) are
  `decimal_text` (`verified::exclusions`). `lemma_decimal_canonical`: the text
  is digits only, 1 to 5 bytes, has no leading zero, and parses back to the
  value.
- `sort_order` is a stable sort by byte-lexicographic key (`lex_lt`, a strict
  total order).
  - The result is a permutation that is strictly increasing by key, ties by
    position.
  - `lemma_sorted_order_unique`: the sorted order is unique
    (`canonical_order`). `lemma_canonical_order_sorted`: every key sequence has
    one (a ghost insertion sort), so `canonical_order` is always a sorted
    order.
- `render_group`:
  - It succeeds exactly when every entry has a portable name and no two names
    are equal ignoring ASCII case (PowerShell and Windows read names
    case-insensitively).
  - On success the text is `block_spec`: one statement per entry, in tag order,
    joined by `\n` with no trailing newline. The empty group renders as `""`.
  - Splitting the text on `\n` gives exactly the lines, and each line is read
    as exactly its name and port's decimal text (`lemma_block`). The dotenv
    model (`dotenv_assignment`) reads the key before the first `=`; the lines
    have no `#`, quote, `$` or space.
  - Otherwise the error names the group's first failure in tag order
    (`first_failure_at`): everything before it is fine (`fine_before`), and an
    entry's own resolution failure is reported before its collision. A group
    has at most one first failure (`lemma_first_failure_unique`).
  - Input order cannot leak: for groups with distinct tags and the same entries
    in any order, `lemma_group_order_independent` gives the same text, and
    `lemma_group_failure_order_independent` gives the same Ok/Err outcome and a
    first failure with the same tag, name and port.
- `escape_display_field` (`display_field`; the human formatter's tags;
  unknown-shell and log-level diagnostics):
  - It escapes `is_unsafe_char`: control characters (C0, DEL, C1) and layout
    controls (the Unicode `Bidi_Control` characters `U+061C`, `U+200E`,
    `U+200F`, `U+202A..=U+202E`, `U+2066..=U+2069`, and the separators
    `U+2028`/`U+2029`).
  - It is the identity on text without such characters that does not start with
    `"`.
  - Otherwise the text is quoted, with `\"` `\\` `\n` `\r` `\t`, `\xHH` for any
    other control character, and `\u{HHHH}` for a layout control.
  - The output never contains an unsafe character, so no line break (including
    for `str.splitlines`/JavaScript readers), tab, terminal control sequence or
    bidi control survives. `unescape_field` inverts it, so escaping is
    injective. Right-to-left letters and other invisible or look-alike
    characters are shown as they are.
- `tab_row`: for one or more fields, splitting a row on tabs gives exactly the
  escaped fields (one column per field), and the row has no line break.

#### Additions to `verified::render`

- `delimited_record(fields, d)`: one CSV (`,`) or TSV (`\t`) record. Each field
  is written verbatim unless it contains the delimiter, `"`, `\r` or `\n`, in
  which case it is quoted with inner quotes doubled. Fields are joined by the
  delimiter, with one `\n` terminator. A record of no fields, or of a single
  empty field, is written `""`. This is the `csv` crate's default writer byte
  for byte: an exhaustive differential test covers the zero-field record and
  all strings of up to 3 characters from `a , \t " \r \n é space`, alone and in
  pairs, for both delimiters. `lemma_delimited_round_trip`: for a delimiter
  other than `"`, `\r`, `\n`, a strict RFC 4180 reader model (`read_record`:
  quotes only open a field, a doubled quote is one quote, a line break outside
  quotes is malformed) reads every written non-empty record back as exactly its
  fields. So the column count and every field survive any text, and the
  terminator is the record's only line break outside quotes. Fields that need
  no quoting are written verbatim (`lemma_delimited_field_shape`), so ordinary
  output is unchanged. `trop list --format csv|tsv` and `trop scan --format
  csv|tsv` write every record with it; the `csv` crate is now only a
  dev-dependency used by the differential tests.
- Table output: `trop list` rows and header go through `tab_row`.
  `list-projects`, `port-info` (path, tag, project, task, owner process and
  user) and `scan`'s table (owner process and user) show user and OS text with
  `display_field`, so each reservation or probe stays on one line with a fixed
  column count.

### `verified::verbosity` — log levels, precedence, severity filter

Glue: `trop/src/logging.rs` (`LogLevel::parse`, `Logger::{error, warn, info,
debug}` via `Logger::emits`, `resolve_log_level`, `init_logger`).

- `parse_verbosity` accepts exactly `quiet`, `normal` and `verbose`, ignoring
  ASCII case.
  - Names round-trip in either case (`lemma_parse_name_round_trip`), and the
    order is total (`lemma_rank_injective`).
  - The invalid-level error shows the input through `display_field`.
- `resolve_verbosity` is the single precedence rule (`resolved_level`). It
  drives both the logger and the commands' quiet/verbose behavior. The
  order:
  - `--quiet` wins, also over `--verbose` (a request to suppress output is
    never overridden);
  - then `--verbose`;
  - then a valid `TROP_LOG_MODE` (`env_level`);
  - then `Normal`.

  There is no second, verbose-first rule. `lemma_precedence`:
  - `--quiet` always gives `Quiet`;
  - `--verbose` without `--quiet` always gives `Verbose`;
  - either flag makes the environment irrelevant;
  - without flags, a level name in either case is used as is, and any other
    value (or none) gives `Normal`;
  - adding `--verbose` never lowers the level, and adding `--quiet` never
    raises it.

  An empty `TROP_LOG_MODE` is the same as an unset one and is not diagnosed
  (`lemma_empty_log_mode_is_unset`).
- `should_emit`: `Quiet` prints nothing, `Normal` prints errors and warnings,
  and `Verbose` prints everything. The filter is monotone in level and in
  severity (`lemma_filter_table`).
  - Open policy question: `Quiet` also suppresses the logger's error messages.
    This keeps the previous behavior and treats logged errors as non-essential
    (spec: "`--quiet`: Suppress non-essential output"). A command's fatal error
    is still printed, because `main` reports it with `eprintln!`, not through
    the logger. If maintainers decide logged errors are essential, set
    `threshold(Severity::Error)` to 0 and update `lemma_filter_table` and
    `test_logger_severity_filter_table`.

Trusted for these kernels (no new escape hatches):
- The model lexers describe bash/zsh, fish, PowerShell and dotenv for these
  restricted lines.
- `lex_lt` is Rust's `str` order (tested).
- ASCII folding equals Unicode `to_lowercase` on the accepted names (exhaustive
  test over all scalar values; only U+212A lowers to an ASCII letter, and no
  name contains `k`).
- `is_layout_control` is `Bidi_Control` plus `Zl`/`Zp` (Unicode data;
  independent table checked over all scalar values).
- `vstd`'s specs of `String::push`/`push_str`/`as_str`, `str::to_owned`,
  `str::chars`, `Vec::insert`, `u8::try_from`, `Result::unwrap_or_default`,
  array `as_slice`, and char casts.
- The glue mappings, covered by a differential proptest against the pre-kernel
  group rendering.

`#[allow(clippy::byte_char_slices)]` on four literal-comparing functions is a
lint exception, not a verification escape. Each one carries a comment
explaining that Verus cannot see inside `b"..."` literals.

#### Additions to `verified::verbosity`

- `trop::logging::resolve_log_level` (and so `init_logger`) calls
  `resolve_verbosity`. `trop-cli` (`run` in `trop-cli/src/lib.rs`) resolves
  the level once and derives both its `Logger` and `GlobalOptions { quiet,
  verbose }` from it, so `TROP_LOG_MODE=quiet` silences commands (before, it
  had no effect) and the result is never both quiet and verbose. The
  separate command-line rule (`cli_level`, `resolve_cli_verbosity`) is gone.
- `invalid_log_mode` is exactly "set, non-empty and not a level name"
  (`is_invalid_log_mode`). The CLI then warns through the logger, showing
  the value escaped by `display_field` between backticks, and ignores it.

### `verified::cli` — exit codes, lock contention, data directory, `init`, assertions, flags, occupancy layers, list filters, port arguments

Glue: `trop/src/cli_support.rs` (`#[doc(hidden)] pub mod cli_support`: thin
wrappers and re-exports for `trop-cli`, such as `process_exit_code`,
`Termination`, `assertion_outcome`, `prints_reservation_port`,
`identifier_filter_matches` and `parse_port_arg`), `trop/src/error.rs`
(`sqlite_error_is_lock_contention`, which calls `is_lock_contention_code`),
`trop/src/database/config.rs` (`default_data_dir` via the private
`data_dir_from`, which calls `select_data_dir` over `HomeFacts`;
`resolve_database_path`), `trop/src/operations/init.rs` (`plan_init`, with
`InitFacts` from the private `blocking_entry`, `Path::is_dir` and
`Path::exists`; `init_database`). In `trop-cli`: `main.rs`
(`Cli::try_parse`), `lib.rs` (`parse_failure_exit_code`, `run`,
`log_mode_warning`), `error.rs` (`CliError::class`/`exit_code`), `cli.rs`
(`cleanup_switches`, `set_occupancy_overrides`, `scan`'s `--min/--max`,
`validate_task_like_reserve`), `invocation.rs` (`resolve_reservation_metadata`,
data-dir errors),
`commands/{assert_port,assert_reservation,assert_data_dir,autoreserve,init,list,reserve_group,scan,port_info,reserve,show_data_dir}.rs`,
and `utils.rs` (`shorten_path`, `resolve_data_dir`). The glue reads the
environment (`var_os`), computes `OsStr::is_empty` and `Path::is_absolute`,
classifies clap's `ErrorKind` and `CliError`/`trop::Error` into kernel enums,
gathers `symlink_metadata`/`is_dir`/`exists` facts for `init`, and joins
`.trop`.

- `exit_code_for(ErrorClass)`: the specification's table. The result lies in
  `1..=7`. It is 1 exactly for the semantic classes (assertion failure, refused
  sticky-field change, refused unrelated path), 2 exactly for `LockTimeout`, 3
  exactly for `NoDataDir`, and at least 4 exactly for every other class. Codes
  from 2 up identify the class (`lemma_exit_codes_identify_class`).
  `process_exit_code(Termination)` covers the whole process: success and
  `--help`/`--version` exit 0, every other clap parse failure is `Usage` and
  exits 4 (invalid arguments), and a failed command exits with its class's
  code. So 0 means success and 2 means a lock timeout and nothing else. Before,
  clap's usage errors also exited 2: `trop scan --min 70000`.
- `is_lock_contention_code(extended)`: true exactly when the low byte of the
  SQLite (extended) result code is 5 (`SQLITE_BUSY`) or 6 (`SQLITE_LOCKED`).
  For non-negative codes this is "the code is 5 or 6 modulo 256". So every
  extended code of the two families classifies (`BUSY_RECOVERY` 261,
  `BUSY_SNAPSHOT` 517, `BUSY_TIMEOUT` 773, `LOCKED_SHAREDCACHE` 262,
  `LOCKED_VTAB` 518), and `OK`, `ERROR`, `CORRUPT`, `CANTOPEN`, `NOTADB` and
  `IOERR_LOCK` never do (`lemma_lock_contention_families`,
  `lemma_lock_contention_examples`, by `bit_vector`). The glue passes
  `ffi::Error::extended_code`; a test checks the kernel against rusqlite's own
  `DatabaseBusy`/`DatabaseLocked` classification for every code in `0..=0xFFFF`
  and the `i32` extremes.
- `select_data_dir(trop_data_dir, home, userprofile)`: returns
  `selected_source`, the first usable source in the order `TROP_DATA_DIR`,
  `HOME`, `USERPROFILE`, or `None` exactly when no source is usable.
  `TROP_DATA_DIR` is explicit and used as given, so a relative value stays
  relative, as before. A home variable is usable only when it is set, non-empty
  and absolute. `lemma_data_dir_selection` is stated over the selection result:
  `TROP_DATA_DIR` wins whatever the home variables are; `HOME` or `USERPROFILE`
  is selected only when it is non-empty and absolute, so a home-derived default
  never depends on the working directory; a usable `HOME` is preferred; an
  unusable `HOME` falls through to `USERPROFILE`; and there is no default
  exactly when no source is usable. `None` is the "no data directory" error:
  `trop show-data-dir` and every command that needs a data directory exit 3.
  Before, they panicked (exit 101) or used a cwd-relative
  `.trop`/`relhome/.trop`. `--data-dir` (which clap also fills from
  `TROP_DATA_DIR`) takes precedence in the CLI glue. The database path and the
  user `config.yaml` both come from this one selection.
- `plan_init(InitFacts)`: `RefuseNotADirectory` exactly when the data
  directory's path is blocked (checked first: the deepest existing entry on the
  path, the path itself or its nearest existing ancestor, is not a directory or
  a link to one). Otherwise `RefuseExistingDatabase` exactly when the database
  exists and `--overwrite` is not given. Otherwise the steps achieve the goal,
  are safe and are minimal. The goal: the data directory exists, the database
  is fresh, and a configuration file exists exactly when one existed or
  `--with-config` asked for one, never replacing an existing one. Safe: the
  database is removed only with `--overwrite`. Minimal: no step whose effect
  already holds. `lemma_init_steps_unique` shows these determine the steps, and
  without a database `--overwrite` changes nothing
  (`lemma_overwrite_irrelevant_without_database`). The reference's
  "`--overwrite`: Overwrite existing files" is read as the database only: an
  existing `config.yaml` may hold the user's edits and is kept even with
  `--overwrite`. This was also the behavior before the kernel, and the lead may
  revisit it. The live run and `--dry-run` both use
  `trop::operations::init::plan_init`, so a dry run fails exactly when the live
  run would (exit 6, same message), for an existing database and for a path
  that is a file alike. "Recreated database" is printed only when an existing
  database was replaced (with its `-wal`, `-shm` and `-journal` files).
- `assertion_outcome(observed, negate)`: `Holds` exactly when `observed !=
  negate`. A failure is `FailsPresent` exactly when the thing exists (with
  `--not`) and `FailsAbsent` exactly when it does not, so the failure message
  always states the observed fact. `lemma_negation`: `--not` inverts the
  verdict. `prints_reservation_port` is `exists && !negate && !quiet`, and
  printing implies the assertion holds. Used by `assert-port`,
  `assert-reservation` and `assert-data-dir`. A failed `--validate` stays an
  internal error that `--not` cannot invert, and the unreachable "database
  validation failed" branch is gone.
- `cleanup_switches(prune, expire, clean)`: `(prune || clean, expire ||
  clean)`; with `--disable-autoclean` the reserve kernel's `cleanup_of` runs
  neither cleanup. `metadata_intent(clear, value)`: `Clear` exactly with
  `--clear-*` and `Preserve` exactly without a value. Resolved against the
  stored value, the three intents give `None`, the value, or the stored value
  respectively.
- `occupancy_layer(OccupancyFlags)`: no layer exactly when no switch is given,
  otherwise `Some(true)` for exactly the given switches. Over any lower
  settings the layer only disables probes, except for the wildcard probes
  `--check-all-interfaces` adds, and every given skip switch disables its
  probes whatever the lower layers say (`lemma_layer_only_disables`, stated
  with `occupancy::settings_request_probe` and the configuration overlay).
- `scan_probes(OccSettings)`: `trop scan` probes with the effective settings
  minus the global `skip`. It has no `--skip-occupancy-check`, because scanning
  is its purpose. The result does not depend on `skip`, equals what an
  allocation would probe when `skip` is not set, and always includes the
  localhost probes when no protocol or family is skipped
  (`lemma_scan_ignores_global_skip`). `NotChecked` exactly when no probe
  remains; `scan` then fails (exit 7, "ports were not checked") instead of
  reporting that nothing is occupied.
- `identifier_filter_matches(filter, stored)`: true exactly when the filter's
  trimmed form (`identifier::trimmed`, `str::trim`) is non-empty and equals the
  stored value. `lemma_filter_canonical`: whitespace padding of any amount,
  independently on each side, does not matter; a blank filter selects nothing;
  and a stored canonical identifier is selected by its own text and by any
  padded form of it. `list --filter-project/--filter-tag` use it. Before,
  `--filter-tag ' web'` found nothing.
- `parse_port_arg(bytes)`: `Ok(p)` exactly for the text of a port from 1 to
  65535 in `u16::from_str` syntax (`is_port_text`). Otherwise `NotANumber`
  exactly when the text is not `u32` text, `Zero` exactly for zero, and
  `TooLarge(v)` with the text's value. Every port argument (`reserve
  --port/--min/--max`, `scan --min/--max`, `assert-port PORT`, `port-info
  PORT`) goes through it, with the same messages and exit 4.

Not proved (glue, pinned by tests): the clap `ErrorKind` and
`CliError`/`trop::Error` classifications (exhaustive matches, unit-tested per
class); that `Path::is_absolute` and `var_os` report the environment correctly
(data-dir tests over empty, relative, `.` and absolute values, without touching
the process environment); and the filesystem facts `init` observes
(time-of-check to time-of-use; a permission-denied ancestor is not modeled).
Trusted with them: clap's error kinds and rusqlite's extended result codes
(the latter checked against rusqlite's own classification, above).
Also not proved: the configuration merger applying the occupancy layer leaf by
leaf (`overlay_settings` models it), and `--filter-path` (still
`Path::starts_with` plus canonicalization). `reserve-group`/`autoreserve` take
their task from the effective configuration (`--task`, else `TROP_TASK`), as
`reserve` does, so an invalid value is reported with its real source (`<command
line>` or `TROP_TASK`) and exit 7 by all three commands.

### `verified::cleanup` — cleanup selection, reasons, prune probe classification, probe-once, removal plan, revalidation, settlement

Glue in `trop/src/operations/cleanup.rs`:
- `CleanupOperations::{prune, expire, autoclean,
  automatic_cleanup_in_connection}` (selections);
- `execute_cleanup_in_connection` (reasons, plan, revalidation oracles,
  settlement);
- `probe_distinct_paths` (path ids, probe once);
- `classify_path`, with `absence_evidence`, `error_facts` and `io_kind_class`
  (probe facts);
- `autoclean_result` (pruned/expired split);
- `PrunePathStatus::is_prunable`.

Reserve's automatic-cleanup call site is in `trop/src/operations/reserve.rs`.
Reserve-group and autoreserve share `automatic_cleanup_in_connection`.

What the glue does:
- reads the clock (whole seconds) and computes `Elapsed` with
  `SystemTime::duration_since`;
- converts each stored `last_used_at` to whole seconds;
- numbers distinct paths with a `BTreeMap<&Path, usize>`;
- extracts independent facts from each probe's `io::Error`;
- runs the database oracles;
- pushes each candidate into the list `settle` decides.

Selection:
- `days_to_secs` is exact; no `u32` day count overflows.
- `command_selection` is the selection of `trop prune`, `expire` and
  `autoclean`: the threshold as given.
- `automatic_selection` gives a maximum age exactly when automatic expiry is
  enabled: the configured threshold, or the 30-day default when it is unset
  (`lemma_automatic_expiry_always_has_threshold`).
- So a `cleanup` section without `expire_after_days` expires exactly as a
  missing section does. Before this, such a section reported automatic
  expiration as enabled but expired nothing. That covered single reservations,
  and groups loaded from a `trop.yaml` with `cleanup: {}`.

Reasons:
- `cleanup_reason` decides `Prune` exactly when the prune phase runs and the
  path is prunable.
- Otherwise it decides `Expire` exactly when a maximum age is set and the time
  since the last use exceeds it, in `Duration` order (`expiry::duration_gt`). A
  last use in the future never expires.
- The two reasons are disjoint, and each names a phase that runs.
- `lemma_whole_second_reason`: with whole-second times, `Expire` is
  `expiry::expired(now, last_used, max)`. An age of exactly the maximum is
  kept, and cleanup agrees with the SQL cutoff.
- `lemma_larger_max_age_expires_subset`: a larger maximum age expires a subset.
- `lemma_unchanged_candidate_still_eligible`: re-checking the same evidence
  (`still_eligible`) never drops a candidate. Production re-checks with the
  planning evidence, so its eligibility re-check is provably redundant and
  `Fate::Ineligible` cannot occur. Revalidation is, in effect, the row-snapshot
  equality check.

Prune safety:
- `classify_probe` makes a path prunable exactly on definitive evidence: a
  non-directory target, or a failure with `NotFound`, a non-directory ancestor
  or an invalid path.
- A directory, and every other failure, is preserved (fail closed). Such a
  failure is `Uninspectable`, with kind precedence symlink loop > transient OS
  code > `io::ErrorKind` class.

Probe once:
- `representatives` maps each reservation to the first reservation with the
  same path id.
- So a path is probed exactly at its first reservation, and every reservation
  on it shares that observation (`lemma_representatives`).

Plan:
- `plan_removals` returns the kept positions (ascending) and the removal
  candidates in removal order (`verified::sort`).
- Every scanned position is in exactly one list.

Revalidation (`revalidate_all`, generic over `observe` and `delete` oracle
closures). The call discipline is in `requires`, and ordinary closures meet it:
- A dry run consults no oracle.
- `delete(i)` is only requested in a live run, for an eligible candidate, after
  some read of it answered `Unchanged` (`delete_permitted`).
- On success, every candidate has exactly one `Fate`, justified by the oracles'
  answers (`justified`).
- A candidate counts as removed only in a dry run, when it was observed gone,
  or when its permitted guarded delete removed it or found it gone
  (`lemma_fate_meaning`).
- A failure is an oracle error, with its candidate index.

What the revalidation spec does not state:
- the call order;
- at most one call per index;
- that no re-read happens between the `Unchanged` answer and the delete.

`Fn` oracles are specified by their answers, not by call history. That the
delete acts on current evidence rests on the loop's structure (one read, then
the delete) and on the trusted guarded DELETE, which compares the whole row
with the planned snapshot in one statement.

Settlement:
- `settle` places each fate in exactly one of the removed and preserved lists
  (`Removed <==> removes(fate)`). A changed or replaced row is preserved with
  its current value; an ineligible one as planned.
- With one fate per candidate, the considered rows are the preserved ones plus
  the removed ones.

Autoclean:
- `split_by_reason` splits the removals into pruned and expired positions, each
  ascending, so each keeps the removal order.
- The two lists partition the removals, so `total_removed == pruned_count +
  expired_count`.

Not proved:
- revalidation freshness and call order (above);
- the three-arm glue match that pushes into the decided list;
- the `io::Error` fact tables;
- the `Path`-ordered `path_decisions` sort.

These are pinned by `trop/tests/cleanup_regressions.rs` and the `test_glue_*`
tests in `operations/cleanup.rs`, which include differential tests against the
previous expiry predicate and the previous stable `sort_by` order.

### `verified::sort` — stable merge sort and the cleanup removal order

Glue: none directly; `verified::cleanup::plan_removals` calls `order_removals`,
and `verified::exclusions` (`canonical_runs`) calls `stable_order`.

- `stable_order(keys)` is a merge sort. It returns every position of `0..n`
  exactly once, in strictly increasing `(key, position)` order
  (`is_stable_order`). That order is unique (`lemma_stable_order_unique`).
- It runs in `O(n log n)`; `verified::render::sort_order` is a quadratic
  insertion sort over byte keys. Membership uses `verified::render::occurs`.
- `order_removals(cands)` returns exactly the non-`Keep` positions, each once
  (`is_removal_order`). By `lemma_removal_order_meaning`:
  - every prune candidate comes before every expire candidate;
  - prune candidates keep scan (path, tag) order;
  - expire candidates come oldest first by `last_used_secs`, ties in scan
    order.
- The removal order is unique (`lemma_removal_order_unique`).
- A differential test shows it equals the previous stable `sort_by` comparator,
  and 65,535 candidates sort quickly.

### `verified::sql_schema` — schema-SQL tokenizer, normalized form, token-run containment

Glue: `trop/src/database/validation.rs` (`validate_required_constraints`,
`validate_exact_table_definitions`).

- `tokenize_schema_sql` computes `tokenize(sql, allowed)`, a recursive
  byte-level specification. At each position it skips ASCII whitespace
  (`u8::is_ascii_whitespace`) and reads one token:
  - a bare word: a maximal run of ASCII alphanumerics, `_`, `$` and non-ASCII
    bytes, ASCII-lowercased (`w`);
  - a `"..."` identifier with `""` escapes. It reads as the bare word when it
    is closed and its decoded text is exactly the allowed table name; otherwise
    it is kept raw (`q`);
  - `'...'` / `` `...` `` literals (kind = the delimiter), `[...]` (`[`), and
    `--` and `/* */` comments (`c`). Each is kept raw and runs to the end when
    unterminated;
  - an operator of 1, 2 (`<= >= != == <> -> ||`) or 3 (`->>`) bytes (`s`).

  It is total and panic-free, and it terminates on every byte string: every
  token is non-empty, which is the `#[via_fn]` termination proof.
- `normalize_schema_sql` is `encode_tokens(tokenize(..))`. Each token is
  written as kind, decimal byte length, `:`, value, `;`. The encoding is
  injective (`lemma_encode_tokens_injective`, via `lemma_decimal` and
  `lemma_decimal_colon_split`), so normalized forms are equal exactly when the
  token streams are (`lemma_normalized_eq_iff`).
- `token_run_contains` is exactly `contains_run`: the needle's tokens occur as
  a contiguous run of the haystack's tokens.
  - The required-constraint check now uses it.
  - Before, `String::contains` on the encoded stream could be satisfied by a
    comment or a quoted literal whose raw text contained the encoded
    constraint. Validation then failed later for the wrong reason.
- What equal tokens mean:
  - **Strictness** (`lemma_tokens_cover_input`): the token spans tile the
    input, and every byte outside them is whitespace. Each token's value is
    fixed by its raw text (`token_text`):
    - a bare word is its word bytes, lowercased;
    - the allowed identifier is `"` + the name with `"` doubled + `"`;
    - everything else is exactly its raw text.

    So equal token streams mean the texts differ only in inter-token
    whitespace, the ASCII case of bare words, and the quoting of the allowed
    identifier.
  - **Robustness, whitespace** (`lemma_space_between_tokens`): inserting an
    ASCII whitespace byte at any position before the end of the input that the
    lexer reaches leaves the tokens unchanged. At the very end it need not,
    because an unterminated quote, bracket or comment absorbs the byte.
  - **Robustness, case** (`lemma_word_case_irrelevant`): replacing a byte
    inside a bare word by a byte with the same ASCII lowercase leaves the
    tokens unchanged. It also leaves unchanged the positions the lexer reaches
    and every word's extent, so any number of such changes leaves the tokens
    unchanged too. No scanner end moves, because every scanner compares bytes
    only with non-word bytes (`lemma_recase_*`).
  - **Quoting** (`lemma_closed_quote_round_trip`): a closed quoted token is its
    decoded text with delimiters doubled.
- Byte-level tokenizing equals the historical `char`-level tokenizer on every
  `&str`. All delimiters are ASCII, and every byte of a non-ASCII UTF-8
  sequence is a word byte. This is a meta-argument, pinned by a differential
  test against the historical implementation, which is kept verbatim in the
  glue's tests: 200,000 random strings with multi-byte and control chars, times
  5 allowed values, plus edge cases.

Glue: `sql.as_bytes()` of the `sqlite_schema` text and of the expected CREATE
text (after `replacen("IF NOT EXISTS", "", 1)`), `Some(table.as_bytes())` as
the allowed identifier, and `!=` on the normalized byte vectors.

### `verified::wal` — enabling WAL journal mode with a bounded retry

Glue: `trop/src/database/connection.rs` (`enable_wal_with_retry`,
`is_in_memory`).

- `journal_mode_accepted` is `mode_accepted`: the reported mode is `wal` (ASCII
  case-insensitive), or `memory` only for an in-memory database.
  - The reported mode was previously ignored. A database SQLite keeps in
    another mode now fails with a clear `SQLITE_CANTOPEN` error; for example, a
    temporary database reports `delete`.
  - Read-only opens and `Database::validate` never run the pragma.
  - Only an empty file name counts as in memory (`is_in_memory`). A name
    rusqlite cannot report (not UTF-8) counts as a file.
- `enable_wal` drives four oracles: the clock (ns since the start), `PRAGMA
  busy_timeout`, `PRAGMA journal_mode = WAL`, and a pause.
  - Every call takes a ticket and returns the next one.
  - It returns the outcome, the last ticket, and a ghost log `WalLog { events,
    tickets }`.
  - It proves `wal_run`, made up of the parts below.
- **The log is the calls** (`calls_chained`): call `i` took `tickets[i]`,
  returned `tickets[i + 1]`, and gave the answer recorded in `events[i]`. For a
  pause, the record is the length actually requested.
  - The ticket type is generic and unbounded, so the kernel can neither copy
    nor create a ticket.
  - The log is therefore the single chain of calls from the caller's ticket to
    the returned one: no replayed answer, no unrecorded call, no reordering.
  - The glue numbers its calls (ticket `n` in, `n + 1` out).
- **Exact shape**: `k` retried rounds, then one last round (`settled_run`),
  then the restore of `busy_timeout` to the whole timeout as the last effect on
  every exit.
  - `retried_round`, five effects:
    1. a clock reading `t`; the elapsed time is the reading, raised to the
       previous round's elapsed time plus its pause (`clock_at`,
       `round_floor`);
    2. a successful `busy_timeout` of `floor((timeout - t)+ / 1 ms)`;
    3. an attempt that reported contention;
    4. a clock reading `u < timeout`;
    5. a pause of `min(timeout - u, 10 ms)`, which is between 1 ns and 10 ms
       (`lemma_pause_bounds`).
  - `last_round`: the same opening, then exactly one of:
    - `BusyTimeoutFailed(e)`;
    - an accepted mode: `Enabled`, or `RestoreFailed(e)` when the restore
      fails;
    - `NotWal(m)`;
    - `Failed(e)`: any other error, returned immediately;
    - `TimedOut(e)`: contention, then a clock reading at or past the timeout.
  - So an attempt is retried only after contention, only while time remains,
    and only after a pause. An earlier error wins over a failed restore.
- `total_pause <= timeout`, and the loop terminates for every clock oracle
  (measure `timeout - floor`).

Glue: `Instant::elapsed().as_nanos()`, `execute_batch("PRAGMA busy_timeout =
{ms}")`, `query_row("PRAGMA journal_mode = WAL")` classified by
`sqlite_error_is_lock_contention`, and `thread::sleep(Duration::from_nanos)`.
Each closure returns `call + 1` as its ticket.
- With a monotonic clock and sleeps at least as long as requested, the
  decisions equal the historical loop's.
- The chain guarantee carries over from an arbitrary ticket type to the glue's
  `u64` by parametricity. This is checked by reading: the kernel uses no
  type-inspecting operation (no `size_of`, `Any` or trait bound on the ticket).

### `verified::symlink` — bounded symlink following

Glue: `trop/src/path/canonicalize.rs` (`canonicalize_safe`,
`canonicalize_bounded`) and `trop/src/path/resolver.rs`
(`PathResolver::resolve` for implicit paths, `resolve_canonical`).

- `follow_links` is generic over the state, a `step` oracle (link / not a link
  / missing / error) and a `same` oracle. It returns the outcome and a ghost
  `Walk`, and proves `followed`:
  - The chain is real: it starts at the start, and each state links to the next
    (`follows`).
  - At most `max_depth` links are followed. `TooDeep` is reported only when
    exactly `max_depth` were followed and the state reached is a link again. So
    a chain of exactly `max_depth` links resolves, and `max_depth == 0` accepts
    a non-link. The historical loop checked the depth before reading the link
    and rejected both.
  - Loop detection is sound and complete for `same`. `Loop(s)` is reported only
    when `s` is the same as an earlier state (at `walk.earlier`). On every
    other outcome, `same` said "different" for every pair of visited states
    (`distinct`).
  - Resolved, Missing and Failed are the last state's own answer.
  - It terminates, with measure `max_depth - depth`.
- `PathResolver`'s `max_symlink_depth` (default 40) now bounds the links
  followed at the path itself. Before, it had no effect. Links in parent
  directories are resolved by the OS.
- A loop at the path that repeats a path exactly is reported as `SymlinkLoop`,
  naming the repeated path; it was previously `Io(ELOOP)`. Because `same` is
  `PathBuf` equality, a loop spelled differently (for example through `..`)
  never repeats exactly and hits the depth bound instead.

Glue:
- `step` classifies `fs::symlink_metadata` (`read_link` for links) and joins a
  relative target to the link's parent with `Path::join`. `..` is deliberately
  not normalized lexically: the OS must resolve it physically.
- `same` is `PathBuf` equality, as the old `HashSet` used.
- A resolved state goes through `fs::canonicalize`.
- `canonicalize_safe` resolves a dangling chain end physically. Its deepest
  existing ancestor is canonicalized through `canonicalize_existing` (the
  verified `deepest_existing_prefix` search), and the missing named components
  are appended. Before, the lexically joined path was returned, such as
  `<T>/symdir/../missing` for what is physically `<T>/x/missing`.
- `canonicalize_bounded` names the input path in not-found, permission and
  depth errors, like `canonicalize`.

Trusted-base additions: none (no
`assume`/`admit`/`external_body`/`assume_specification`).

### `verified::civil_time`: Unix time to UTC civil date and time, and its `%Y-%m-%d %H:%M:%S` text

Glue: `trop/src/output/mod.rs` (`format_utc_timestamp`). It splits a
`SystemTime` with `duration_since(UNIX_EPOCH)` into a direction, whole seconds
and nanoseconds. trop-cli's `format_timestamp` calls it for `trop list` and
`trop port-info`.

- Calendar vocabulary:
  - `is_leap_year`: the Gregorian rule. `%` is Euclidean and years are
    astronomical (year 0 is 1 BC).
  - `days_in_month`, `days_in_year`.
  - `days_from_civil(y, m, d) = days_before_year(y) + days_before_month(y, m) +
    d - 1`. Both terms are plain sums of year and month lengths counted from
    1970-01-01, so the definition is the calendar itself, not a closed formula.
- Calendar lemmas:
  - `lemma_year_period`: 400 years hold 146097 days.
  - `lemma_cycle_start`.
  - `lemma_years_increase`, `lemma_months_increase`, `lemma_date_bounds`.
  - `lemma_dates_increase`: day numbers follow the calendar order of valid
    dates.
- `civil_from_instant(before_epoch, secs, nanos)`:
  - It is total for every `u64` second count and `u32` nanosecond count, in
    both directions, with no overflow. The Unix times the kernel converts
    (`supported_seconds`) are `|s| <= 2^65`, which over-approximates the
    floors of `Duration`-sized instants (within about `2^64` s of the
    epoch).
  - The result is valid: month in `1..=12`, day in `1..=days_in_month`, hour
    below 24, minute and second below 60.
  - It is an exact inverse: `days_from_civil * 86400 + h*3600 + m*60 + s` is
    `floor_unix_seconds`, the floor of the instant. Sub-second instants before
    the epoch round down, as chrono does.
  - `lemma_civil_unique`: no other valid civil time has that second. The
    conversion is a bijection, and the result is `utc_civil(..)`.
- `civil_text`: chrono's `%Y-%m-%d %H:%M:%S`.
  - The year is zero-padded to four digits, with `+` above 9999 and `-` below
    0. The other fields are two digits.
  - `format_unix_instant` returns a `String` whose UTF-8 bytes are
    `civil_text(utc_civil(floor_unix_seconds(..)))`. Its ensures also state
    that `utc_civil(floor_unix_seconds(..))` is valid and has that Unix
    second, so the `choose` in `utc_civil` has a witness and callers can
    unfold the text with the `civil_text` lemmas.
- Text lemmas:
  - `lemma_civil_text_ascii`.
  - `lemma_civil_text_shape`: for years `0..=9999` the text is 19 bytes.
    Separators sit at fixed positions, and each field's digits parse to the
    field.
  - `lemma_text_order`: for years `0..=9999`, Unix-time order is exactly byte
    order of the text.
  - `lemma_civil_text_injective`: for every year, distinct civil times give
    distinct text.

Behavior change: the old chrono path panicked for a `SystemTime` outside
chrono's range (before `-262143-01-01`, after `+262142-12-31 23:59:59`). Such
times are unreachable from stored reservations. The kernel continues the
calendar instead, for example `+262143-01-01 00:00:00`.

Trusted: byte equality with chrono over its whole range is pinned by
`trop/tests/timestamp_format.rs`:
- every day of 800 years at both ends of the day;
- random instants with random nanoseconds;
- named boundaries.

Beyond chrono's range, the same file compares against an independent
closed-form (Hinnant) reference.

### `verified::json`: the JSON port map (`reserve-group --format=json`)

Glue: `trop/src/output/formatters.rs` (`JsonFormatter`). It passes the map's
`(tag, port)` pairs to `render_port_map_pretty`. Formatting has no error path.

- `json_escape_char` / `json_escape_body`: `serde_json`'s escaping.
  - Two-byte escapes: `\"`, `\\`, `\b`, `\t`, `\n`, `\f`, `\r`.
  - Other C0 controls: `\u00xx` with lowercase hex.
  - Everything else is copied, including DEL, non-ASCII text and
    `U+2028`/`U+2029`.
  - `lemma_escape_matches_serde_bytes`: the UTF-8 text of the escape equals
    `json_escape_bytes` of the UTF-8 text. That function is serde_json's
    per-byte `ESCAPE` table, transcribed.
- `json_scan_string`: the RFC 8259 §7 grammar after an opening quote. It stops
  at the first unescaped quote, rejects raw controls, and decodes every escape
  in either hex case. Surrogate escapes are outside it.
  - `lemma_json_string_round_trip`: every escaped string scans back to itself
    and ends exactly at its closing quote, whatever follows. The escaped text
    contains no raw control character.
  - `lemma_json_string_injective`.
- `json_object_pretty`: `{}` when empty, otherwise `{\n  "k": v,\n  ...\n}`.
  This is serde_json's `PrettyFormatter` for a flat object. Values are the
  canonical decimal text of the port (`verified::render`).
- `read_json_object`: RFC 8259 for a single object whose values are
  non-negative `int`s, with optional whitespace around every structural
  character.
  - `lemma_object_reads_back`: the rendered object reads back as exactly its
    entries, in order. The output is valid JSON and loses nothing.
  - `lemma_port_map_reads_back`.
- `render_port_map_pretty`: renders the entries in `render::canonical_order` of
  their UTF-8 key bytes (`BTreeMap<String, _>` order). The order comes from
  `render::sort_order`, an `O(n^2)` insertion sort, negligible at group
  sizes (the former `BTreeMap` path was `O(n log n)`).
  - `lemma_sorted_entries_members`: the sorted entries are exactly the input
    entries.
  - `lemma_sorted_keys_increase`: for distinct keys, the keys strictly
    increase.
  - `lemma_render_order_independent`: the text depends only on the map, not on
    the input order.

Glue: `(tag.as_str(), port.value())` for each `HashMap` entry. Keys are
distinct.

Trusted: the layout and the `ESCAPE` table transcription match
`serde_json::to_string_pretty`. This is pinned by
`trop/tests/json_port_map.rs`: a snapshot, a character sweep, adversarial and
arbitrary property tests, and a serde_json parse-back.

Proof engineering: `#[verifier::spinoff_prover]` isolates four heavy `json`
functions from shared solver state. The reader functions are
`#[verifier::opaque]`, and `read_members` terminates by a `#[via_fn]` proof.
None of these extend the trusted base.

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
  `verification results:: N verified, 0 errors` line. When the harness
  crate is fresh, cargo replays its cached output instead of running the
  verifier; the replay has no results line and ignores which module was
  asked for. Both recipes therefore drop the harness crate's cached build
  before running (dependencies stay cached) and fail, through
  `scripts/check-verus-results.sh`, unless every results line reports 0
  errors and at least one is printed. Calling `cargo verus`
  directly does neither, and one checkout's cached result can then be
  replayed for another, so give each checkout (and each git worktree) its
  own `CARGO_TARGET_DIR`.
- Proofs near Z3's resource limit are flaky: Verus spreads a module's
  functions over several solver processes, so the incremental context a
  query meets varies between runs, and adding unrelated kernels can tip a
  proof over the limit. After a large change, run the whole suite once at
  half the default limit (`cd trop-verification && cargo verus verify --
  --rlimit 5`). Give any proof that fails there its own solver
  (`#[verifier::spinoff_prover]`), simplify it (an explicit equality chain
  often removes a sequence-index matching loop), or, if it still needs more
  than half the budget in isolation, explicit headroom
  (`#[verifier::rlimit(20)]`, which changes how long Z3 may search, not what
  is proved).
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
- Keep the build toolchain and the verifier's toolchain in step. `vstd`'s
  `std` specifications are written for, and resolved against,
  `VERUS_RUST_TOOLCHAIN` (now 1.100.0), while `rust-toolchain.toml` and the
  CI jobs build `trop` with 1.99.1 (see "Trusted computing base"). When
  bumping either, move the other with it where possible, or update the skew
  recorded in the trusted base. The build toolchain is `rust-toolchain.toml`
  plus every `dtolnay/rust-toolchain@1.99.1` pin in
  `.github/workflows/ci.yml` and `.github/workflows/property-tests.yml`;
  move them together. The licenses job's `@stable` is not a pin and stays.
