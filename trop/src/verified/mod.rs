//! Formally verified kernels.
//!
//! Every module in this tree is written inside Verus `verus! { ... }` blocks
//! and is machine-checked by the [Verus](https://github.com/verus-lang/verus)
//! verifier: each executable function carries a specification (`requires` /
//! `ensures`), and Verus proves — for every possible input — that the body
//! satisfies it, that no arithmetic overflows, that no index is out of bounds,
//! and that every loop terminates.
//!
//! The rest of `trop` delegates its decision logic to these kernels through
//! thin glue that converts domain types (`Port`, `Path`, configuration
//! structs, ...) into the primitive values the kernels operate on.
//!
//! # Rules for code in this tree
//!
//! - **Self-contained.** Nothing here may refer to the rest of `trop` (no
//!   `crate::port::Port`, `crate::Error`, `std::path`, database or OS APIs).
//!   Refer to sibling kernels through `super::` so the tree verifies both as
//!   `trop::verified` and inside the `trop-verification` harness crate.
//! - **Total entry points.** Functions called from unverified production code
//!   must not carry `requires` clauses the caller could violate. Either accept
//!   every input (and specify the result for invalid ones too), or take a
//!   kernel type whose private fields and `#[verifier::type_invariant]`
//!   guarantee the precondition — Rust privacy then enforces it even for
//!   unverified callers.
//! - **No trusted escapes without a record.** `assume`, `admit`,
//!   `#[verifier::external_body]`, and `assume_specification` extend the
//!   trusted computing base; each use must be justified in a comment and
//!   listed in `docs/verification.md`.
//! - **Erasure-safe.** In ordinary builds the `verus!` macro removes specs,
//!   proofs, and ghost code, so only the executable Rust remains. Do not call
//!   `vstd` executable helpers; use `core`/`std` operations whose behavior
//!   `vstd` specifies.
//!
//! Run `just verify` to check the proofs (requires `just install-verus`).

pub mod allocation;
pub mod env_name;
pub mod exclusions;
pub mod group;
pub mod identifier;
pub mod occupancy;
pub mod path;
pub mod path_relation;
pub mod port;
pub mod port_bounds;
pub mod sticky;
pub mod text;
