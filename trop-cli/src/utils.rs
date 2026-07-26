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
/// If the path is within the home directory, show it as ~/...
/// Otherwise, show the full path.
pub fn shorten_path(path: &Path) -> String {
    if let Some(home) = home::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            return format!("~/{}", relative.display());
        }
    }
    path.display().to_string()
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
                service
                    .env
                    .as_ref()
                    .map(|env_name| (tag.clone(), env_name.clone()))
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
pub fn resolve_data_dir() -> PathBuf {
    trop::database::default_data_dir().expect(
        "Failed to determine data directory (home directory not found and TROP_DATA_DIR not set)",
    )
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
        assert_eq!(shorten_path(&path), "/usr/local/bin");
    }
}
