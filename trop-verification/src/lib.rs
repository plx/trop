//! Verus verification harness for `trop`.
//!
//! This crate is never published and has no code of its own. It mounts the
//! `trop/src/verified` module tree at the same path (`crate::verified`) that
//! the `trop` library uses, so `cargo verus verify` checks exactly the source
//! files that `trop` compiles. In ordinary `cargo build` runs the `verus!`
//! macro erases every specification and proof, and this crate is just a
//! second, redundant compilation of the same executable code.
//!
//! See `docs/verification.md` for the architecture and trusted base.

#[path = "../../trop/src/verified/mod.rs"]
pub mod verified;
