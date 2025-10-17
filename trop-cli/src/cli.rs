//! CLI structure and command definitions.
//!
//! This module defines the main CLI structure using clap's derive macros,
//! including global options and subcommands.

use crate::commands::{ListCommand, ReleaseCommand, ReserveCommand};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Command-line tool for managing ephemeral port reservations.
#[derive(Parser)]
#[command(name = "trop")]
#[command(version, about = "Manage ephemeral port reservations", long_about = None)]
pub struct Cli {
    /// Enable verbose output
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Override the data directory location
    #[arg(long, value_name = "PATH", global = true, env = "TROP_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Override the default busy timeout (in seconds)
    #[arg(long, value_name = "SECONDS", global = true, env = "TROP_BUSY_TIMEOUT")]
    pub busy_timeout: Option<u32>,

    /// Disable automatic database initialization
    #[arg(long, global = true, env = "TROP_DISABLE_AUTOINIT")]
    pub disable_autoinit: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Available CLI commands.
#[derive(Subcommand)]
pub enum Command {
    /// Reserve a port for a directory
    Reserve(ReserveCommand),

    /// Release a port reservation
    Release(ReleaseCommand),

    /// List active reservations
    List(ListCommand),
}
