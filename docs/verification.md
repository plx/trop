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

## Trusted computing base

A verified property holds for the shipped binary if the following are
correct; everything else in a kernel is checked:

- the Verus verifier and the Z3 SMT solver (release pinned in
  `scripts/install-verus.sh`; `vstd` pinned in the workspace `Cargo.toml`);
- `vstd`'s specifications of the `core`/`std` operations a kernel calls
  (for example `u16::checked_add`);
- the `verus!` macro's erasure (it removes ghost code only);
- the production glue that converts domain types to kernel inputs and back;
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
- To bump Verus: pick a weekly release tag `release/0.YYYY.MM.DD.<sha>` from
  <https://github.com/verus-lang/verus/releases>, set `VERUS_VERSION`, the
  checksums, and the Rust toolchain in `scripts/install-verus.sh`, and set
  `vstd` in `Cargo.toml` to the crates.io version published from the same
  commit (`0.0.0-YYYY-MM-DD-HHMM`). Re-run `just verify`.
