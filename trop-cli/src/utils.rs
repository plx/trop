//! Utility functions for CLI operations.
//!
//! This module provides common utility functions used across CLI commands,
//! including path resolution, configuration loading, database management,
//! and output formatting.

use crate::error::CliError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use trop::output::OutputFormat;
use trop::{Config, Port};

/// Global CLI options shared across all commands.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used via pattern matching in main.rs
pub struct GlobalOptions {
    /// Enable verbose output.
    pub verbose: bool,

    /// Suppress non-essential output.
    pub quiet: bool,

    /// Override the data directory location.
    pub data_dir: Option<PathBuf>,

    /// Override the default busy timeout (in seconds).
    pub busy_timeout: Option<u32>,

    /// Disable automatic database initialization.
    pub disable_autoinit: bool,
}

/// Format a timestamp for display.
pub fn format_timestamp(ts: std::time::SystemTime) -> String {
    use chrono::{DateTime, Utc};
    let dt: DateTime<Utc> = ts.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Shorten a path for display.
///
/// If the path is within the home directory, show it as `~/...` (and the
/// home directory itself as `~`); otherwise, show the full path. Paths are
/// compared component by component, so `/home/al` never shortens
/// `/home/alice` (the verified `trop::cli_support::home_relative_display`
/// decides).
pub fn shorten_path(path: &Path) -> String {
    home::home_dir()
        .and_then(|home| trop::cli_support::home_relative_display(path, &home))
        .unwrap_or_else(|| path.display().to_string())
}

/// Format port allocations using the specified output format.
///
/// This function extracts environment variable mappings from the config
/// and uses the output formatter to generate the appropriate output format.
///
/// # Arguments
///
/// * `output_format` - The desired output format (export, json, dotenv, human)
/// * `allocations` - Map of service tags to allocated ports
/// * `config` - Configuration containing service definitions with env mappings
///
/// # Returns
///
/// Formatted string representation of the allocations
pub fn format_allocations(
    output_format: &OutputFormat,
    allocations: &HashMap<String, Port>,
    config: &Config,
) -> Result<String, CliError> {
    // Extract environment variable mappings from config if present
    let env_mappings = config.reservations.as_ref().map(|group| {
        group
            .services
            .iter()
            .filter_map(|(tag, service)| {
                service.env.as_ref().map(|env_name| {
                    // Allocations are keyed by the canonical (trimmed)
                    // tag; trim with the same verified rule.
                    (
                        trop::cli_support::trim_identifier(tag).to_string(),
                        env_name.clone(),
                    )
                })
            })
            .collect::<HashMap<String, String>>()
    });

    let formatter = output_format.create_formatter(env_mappings);
    formatter.format(allocations).map_err(CliError::from)
}

/// Resolve the default data directory.
///
/// Data-directory selection is deliberately outside the YAML configuration
/// schema and is shared by the special data-directory commands.
///
/// # Errors
///
/// Returns [`CliError::DataDirectoryUndetermined`] (exit code 3) when no
/// data directory can be determined.
pub fn resolve_data_dir() -> Result<PathBuf, CliError> {
    trop::database::default_data_dir()
        .map_err(|error| CliError::DataDirectoryUndetermined(error_message(error)))
}

/// The error for a data directory that cannot be determined (exit code 3),
/// with the library's explanation.
pub(crate) fn undetermined_data_dir_error() -> CliError {
    match resolve_data_dir() {
        Err(error) => error,
        // `--data-dir` and the default were both unavailable when the
        // invocation was resolved; the environment does not change.
        Ok(dir) => CliError::DataDirectoryUndetermined(format!(
            "no data directory was selected (the default would be {})",
            dir.display()
        )),
    }
}

/// A library error's message without its `validation error for '<field>':`
/// prefix (for errors whose field is already clear from the context).
fn error_message(error: trop::Error) -> String {
    match error {
        trop::Error::Validation { message, .. } => message,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_timestamp() {
        use std::time::{Duration, UNIX_EPOCH};
        // Create a known SystemTime
        let st = UNIX_EPOCH + Duration::from_secs(1705323045); // 2024-01-15 10:30:45 UTC
        let formatted = format_timestamp(st);
        assert!(formatted.contains("2024-01-15"));
    }

    #[test]
    fn test_shorten_path_outside_home() {
        let path = PathBuf::from("/usr/local/bin");
        if home::home_dir().is_some_and(|home| home != Path::new("/")) {
            assert_eq!(shorten_path(&path), "/usr/local/bin");
        }
    }

    #[test]
    fn test_shorten_path_inside_home() {
        if let Some(home) = home::home_dir() {
            assert_eq!(shorten_path(&home), "~");
            assert_eq!(
                shorten_path(&home.join("a").join("b")),
                format!("~/{}", Path::new("a").join("b").display())
            );
        }
    }
}
