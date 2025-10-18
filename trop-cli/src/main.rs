//! Main entry point for the trop CLI.
//!
//! This is the command-line interface for the trop port reservation system.
//! It provides commands for managing port reservations:
//! - `reserve`: Reserve a port for a directory
//! - `release`: Release a port reservation
//! - `list`: List active reservations
//! - `reserve-group`: Reserve ports for a group of services
//! - `autoreserve`: Automatically discover and reserve ports

mod cli;
mod commands;
mod error;
mod utils;

use clap::Parser;
use cli::Cli;
use utils::GlobalOptions;

fn main() {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging based on verbosity
    let _logger = trop::init_logger(cli.verbose, cli.quiet);

    // Convert CLI args to GlobalOptions
    let global = GlobalOptions {
        verbose: cli.verbose,
        quiet: cli.quiet,
        data_dir: cli.data_dir,
        busy_timeout: cli.busy_timeout,
        disable_autoinit: cli.disable_autoinit,
    };

    // Execute the command
    let result = match cli.command {
        cli::Command::Reserve(cmd) => cmd.execute(&global),
        cli::Command::Release(cmd) => cmd.execute(&global),
        cli::Command::List(cmd) => cmd.execute(&global),
        cli::Command::ReserveGroup(cmd) => cmd.execute(&global),
        cli::Command::Autoreserve(cmd) => cmd.execute(&global),
        cli::Command::Prune(cmd) => cmd.execute(&global),
        cli::Command::Expire(cmd) => cmd.execute(&global),
        cli::Command::Autoclean(cmd) => cmd.execute(&global),
        cli::Command::AssertReservation(cmd) => cmd.execute(&global),
        cli::Command::AssertPort(cmd) => cmd.execute(&global),
        cli::Command::AssertDataDir(cmd) => cmd.execute(&global),
        cli::Command::PortInfo(cmd) => cmd.execute(&global),
        cli::Command::ShowDataDir(cmd) => cmd.execute(&global),
        cli::Command::ShowPath(cmd) => cmd.execute(&global),
        cli::Command::Scan(cmd) => cmd.execute(&global),
        cli::Command::Validate(cmd) => cmd.execute(&global),
        cli::Command::Exclude(cmd) => cmd.execute(&global),
        cli::Command::CompactExclusions(cmd) => cmd.execute(&global),
    };

    // Handle errors and set exit code
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(e.exit_code());
        }
    }
}
