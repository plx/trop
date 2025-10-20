//! Library exports for trop-cli.
//!
//! This module exports the CLI structure for use by the build script
//! to generate man pages and other documentation.

pub mod cli;
pub mod commands;
pub mod error;
pub mod utils;

// Re-export CLI for build script
pub use cli::Cli;
