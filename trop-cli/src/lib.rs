//! Library exports for trop-cli.
//!
//! This module exports the CLI structure for use by the build script
//! to generate man pages and other documentation.

pub mod cli;
pub mod commands;
pub mod error;
pub mod invocation;
pub mod utils;

// Re-export CLI for build script
pub use cli::Cli;

use invocation::InvocationContext;
use utils::GlobalOptions;

/// Resolve one CLI invocation and execute its selected command.
pub fn run(cli: Cli) -> Result<(), error::CliError> {
    let global = GlobalOptions {
        verbose: cli.verbose,
        quiet: cli.quiet,
        data_dir: resolve_data_dir(cli.data_dir),
        busy_timeout: cli.busy_timeout,
        disable_autoinit: cli.disable_autoinit,
    };

    let request = cli.command.config_request(&global)?;
    let context = InvocationContext::resolve(global, request)?;
    cli.command.execute(&context)
}

fn resolve_data_dir(cli_value: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    cli_value
        .or_else(|| std::env::var_os("TROP_DATA_DIR").map(std::path::PathBuf::from))
        .or_else(|| trop::database::default_data_dir().ok())
}
