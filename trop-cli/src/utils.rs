//! Utility functions for CLI operations.
//!
//! This module provides common utility functions used across CLI commands,
//! including path resolution, configuration loading, database management,
//! and output formatting.

use crate::error::CliError;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use trop::output::OutputFormat;
use trop::{Config, ConfigBuilder, Database, DatabaseConfig, PathResolver, Port};

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

/// Resolve a path, using CWD if not specified.
///
/// # Path Handling Rules
///
/// - Explicit paths (provided by user) are normalized but NOT canonicalized
/// - Implicit paths (CWD) are normalized from the current directory
///
/// Normalization makes paths absolute and expands ~, but doesn't follow symlinks.
/// This allows paths that don't exist yet and avoids issues with temp directories.
pub fn resolve_path(path: Option<PathBuf>) -> Result<PathBuf, CliError> {
    let path_to_resolve = match path {
        Some(p) => p,
        None => env::current_dir()?,
    };

    // Normalize to make absolute, but don't canonicalize (allows non-existent paths)
    normalize_path(&path_to_resolve)
}

/// Normalize a path (make absolute, expand ~, etc.) without following symlinks.
pub fn normalize_path(path: &Path) -> Result<PathBuf, CliError> {
    let resolver = PathResolver::new();
    let resolved = resolver.resolve_explicit(path).map_err(CliError::from)?;
    Ok(resolved.into_path_buf())
}

/// Load hierarchical configuration.
///
/// Configuration is merged from multiple sources with precedence:
/// 1. Global options (highest priority)
/// 2. Environment variables
/// 3. Configuration files
/// 4. Built-in defaults (lowest priority)
pub fn load_configuration(_global: &GlobalOptions) -> Result<Config, CliError> {
    let builder = ConfigBuilder::new();

    // Build configuration from environment and files
    let config = builder
        .build()
        .map_err(|e| CliError::Config(e.to_string()))?;

    Ok(config)
}

/// Resolve the database path from global options.
fn resolve_database_path(global: &GlobalOptions) -> Result<PathBuf, CliError> {
    // Priority: global option > default
    if let Some(ref data_dir) = global.data_dir {
        return Ok(data_dir.join("trop.db"));
    }

    // Default: ~/.trop/trop.db
    let home_dir = home::home_dir()
        .ok_or_else(|| CliError::Config("Could not determine home directory".to_string()))?;

    Ok(home_dir.join(".trop").join("trop.db"))
}

/// Open database with configuration.
///
/// # Errors
///
/// Returns `NoDataDirectory` if the database doesn't exist and auto-init is disabled.
pub fn open_database(global: &GlobalOptions, config: &Config) -> Result<Database, CliError> {
    let db_path = resolve_database_path(global)?;

    if !db_path.exists() && global.disable_autoinit {
        return Err(CliError::NoDataDirectory);
    }

    let mut db_config = DatabaseConfig::new(db_path);

    // Set busy timeout if specified
    if let Some(timeout_seconds) = global.busy_timeout {
        db_config =
            db_config.with_busy_timeout(std::time::Duration::from_secs(timeout_seconds.into()));
    } else if let Some(timeout_seconds) = config.maximum_lock_wait_seconds {
        db_config = db_config.with_busy_timeout(std::time::Duration::from_secs(timeout_seconds));
    }

    Database::open(db_config).map_err(CliError::from)
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

/// Resolve the data directory path.
///
/// Returns the default data directory location: `~/.trop`
pub fn resolve_data_dir() -> PathBuf {
    home::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".trop")
}

/// Find project configuration file (trop.yaml) starting from current directory.
///
/// Searches up the directory tree for `trop.local.yaml` or `trop.yaml`.
/// Returns the first match found, with `trop.local.yaml` taking precedence.
///
/// # Returns
///
/// - `Ok(Some(path))` if a configuration file is found
/// - `Ok(None)` if no configuration file is found
/// - `Err(_)` if there's an error accessing the file system
pub fn find_project_config() -> Result<Option<PathBuf>, CliError> {
    let mut current = env::current_dir()?;

    loop {
        // Check for trop.local.yaml first (higher precedence)
        let local_config = current.join("trop.local.yaml");
        if local_config.exists() {
            return Ok(Some(local_config));
        }

        // Check for trop.yaml
        let config = current.join("trop.yaml");
        if config.exists() {
            return Ok(Some(config));
        }

        // Move up one directory
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Ok(None)
}

/// Resolve the configuration file to use (project or global).
///
/// Returns the path to the project configuration file (trop.yaml or trop.local.yaml)
/// if one exists, otherwise returns the path to the global configuration file.
///
/// # Returns
///
/// Path to the configuration file to use (may not exist yet).
pub fn resolve_config_file() -> Result<PathBuf, CliError> {
    Ok(find_project_config()?.unwrap_or_else(|| resolve_data_dir().join("config.yaml")))
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
