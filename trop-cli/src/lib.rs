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
use trop::cli_support::{process_exit_code, Termination};
use trop::{LogLevel, Logger};
use utils::GlobalOptions;

/// The exit code for an argument-parsing failure of the given kind:
/// `--help` and `--version` exit 0, and every other parse failure is a
/// usage error with the invalid-arguments exit code (4). The verified
/// kernel `verified::cli::process_exit_code` decides, so a usage error never
/// exits with 2, the code reserved for lock timeouts.
#[must_use]
pub fn parse_failure_exit_code(kind: clap::error::ErrorKind) -> i32 {
    use clap::error::ErrorKind;

    process_exit_code(match kind {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Termination::Informational,
        _ => Termination::Usage,
    })
}

/// The exit code of a successful run (0).
#[must_use]
pub fn success_exit_code() -> i32 {
    process_exit_code(Termination::Success)
}

/// Resolve one CLI invocation and execute its selected command.
pub fn run(cli: Cli) -> Result<(), error::CliError> {
    // One verbosity, resolved once, drives both the logger and the commands'
    // quiet/verbose behavior: `--quiet` wins over `--verbose`, either flag
    // overrides `TROP_LOG_MODE`, and an invalid `TROP_LOG_MODE` is ignored
    // (with a warning).
    let log_mode =
        std::env::var_os("TROP_LOG_MODE").map(|value| value.to_string_lossy().into_owned());
    let level = trop::logging::resolve_log_level(cli.verbose, cli.quiet, log_mode.as_deref());
    let logger = Logger::new(level);
    if trop::cli_support::invalid_log_mode(log_mode.as_deref()) {
        if let Some(value) = &log_mode {
            logger.warn(&format!(
                "ignoring invalid TROP_LOG_MODE value {}: expected quiet, normal, or verbose",
                trop::output::display_field(value)
            ));
        }
    }

    let global = GlobalOptions {
        verbose: level == LogLevel::Verbose,
        quiet: level == LogLevel::Quiet,
        data_dir: resolve_data_dir(cli.data_dir),
        busy_timeout: cli.busy_timeout,
        disable_autoinit: cli.disable_autoinit,
    };

    let command_name = cli.command.name();
    let request = cli.command.config_request(&global)?;
    let context = InvocationContext::resolve(global, request)?;
    let result = cli.command.execute(&context);
    match context.lock_timeout() {
        Some(timeout) => result.map_err(|error| match error {
            error::CliError::Library(source) => error::CliError::Library(
                source
                    .classify_sqlite_lock(timeout, format!("executing the {command_name} command")),
            ),
            error => error,
        }),
        None => result,
    }
}

/// The data directory: `--data-dir` (or `TROP_DATA_DIR`, which clap reads
/// into it) as given, otherwise the library's default
/// (`trop::database::default_data_dir`: `TROP_DATA_DIR`, then a non-empty
/// absolute `HOME`, then `USERPROFILE`). `None` when no data directory can be
/// determined; commands that need one then fail with exit code 3.
fn resolve_data_dir(cli_value: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    cli_value.or_else(|| trop::database::default_data_dir().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn parse_failures_exit_with_usage_code_except_help_and_version() {
        assert_eq!(parse_failure_exit_code(ErrorKind::DisplayHelp), 0);
        assert_eq!(parse_failure_exit_code(ErrorKind::DisplayVersion), 0);
        for kind in [
            ErrorKind::InvalidValue,
            ErrorKind::UnknownArgument,
            ErrorKind::InvalidSubcommand,
            ErrorKind::NoEquals,
            ErrorKind::ValueValidation,
            ErrorKind::TooManyValues,
            ErrorKind::TooFewValues,
            ErrorKind::WrongNumberOfValues,
            ErrorKind::ArgumentConflict,
            ErrorKind::MissingRequiredArgument,
            ErrorKind::MissingSubcommand,
            ErrorKind::InvalidUtf8,
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
            ErrorKind::Io,
            ErrorKind::Format,
        ] {
            assert_eq!(parse_failure_exit_code(kind), 4, "{kind:?}");
        }
        assert_eq!(success_exit_code(), 0);
    }
}
