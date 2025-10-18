//! CLI structure and command definitions.
//!
//! This module defines the main CLI structure using clap's derive macros,
//! including global options and subcommands.

use crate::commands::{
    AssertDataDirCommand, AssertPortCommand, AssertReservationCommand, AutocleanCommand,
    AutoreserveCommand, CompactExclusionsCommand, ExcludeCommand, ExpireCommand, ListCommand,
    PortInfoCommand, PruneCommand, ReleaseCommand, ReserveCommand, ReserveGroupCommand,
    ScanCommand, ShowDataDirCommand, ShowPathCommand, ValidateCommand,
};
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

    /// Reserve ports for a group of services defined in a config file
    ReserveGroup(ReserveGroupCommand),

    /// Automatically discover and reserve ports from project config
    Autoreserve(AutoreserveCommand),

    /// Remove reservations for non-existent directories
    Prune(PruneCommand),

    /// Remove reservations based on age
    Expire(ExpireCommand),

    /// Combined cleanup (prune + expire)
    Autoclean(AutocleanCommand),

    /// Assert that a reservation exists for a path/tag
    AssertReservation(AssertReservationCommand),

    /// Assert that a specific port is reserved
    AssertPort(AssertPortCommand),

    /// Assert that the data directory exists and is valid
    AssertDataDir(AssertDataDirCommand),

    /// Display information about a specific port
    #[command(name = "port-info")]
    PortInfo(PortInfoCommand),

    /// Show the resolved data directory path
    ShowDataDir(ShowDataDirCommand),

    /// Show the resolved path for a reservation
    ShowPath(ShowPathCommand),

    /// Scan port range for occupied ports
    Scan(ScanCommand),

    /// Validate a configuration file
    Validate(ValidateCommand),

    /// Add port or range to exclusion list
    Exclude(ExcludeCommand),

    /// Compact exclusion list to minimal representation
    CompactExclusions(CompactExclusionsCommand),
}
