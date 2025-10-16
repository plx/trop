//! Utility functions for CLI operations.
//!
//! This module provides common utility functions used across CLI commands,
//! including path resolution, configuration loading, database management,
//! and output formatting.

use crate::error::CliError;
use std::env;
use std::path::{Path, PathBuf};
use trop::{Config, ConfigBuilder, Database, DatabaseConfig, PathResolver};

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

/// Canonicalize a path (follow symlinks).
pub fn canonicalize_path(path: &Path) -> Result<PathBuf, CliError> {
    let resolver = PathResolver::new();
    let resolved = resolver.resolve_implicit(path).map_err(CliError::from)?;
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
