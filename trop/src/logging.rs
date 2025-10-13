//! Logging infrastructure for the trop library.
//!
//! This module provides a simple stderr-based logging system with
//! configurable log levels.

use std::env;
use std::fmt;

/// Logging level for controlling output verbosity.
///
/// Log levels are ordered from least verbose (Quiet) to most verbose (Verbose).
///
/// # Examples
///
/// ```
/// use trop::LogLevel;
///
/// let quiet = LogLevel::Quiet;
/// let normal = LogLevel::Normal;
/// let verbose = LogLevel::Verbose;
///
/// assert!(quiet < normal);
/// assert!(normal < verbose);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Suppress all non-essential output.
    Quiet,
    /// Normal output level (errors and warnings).
    Normal,
    /// Verbose output (errors, warnings, info, and debug messages).
    Verbose,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Quiet => write!(f, "quiet"),
            Self::Normal => write!(f, "normal"),
            Self::Verbose => write!(f, "verbose"),
        }
    }
}

impl LogLevel {
    /// Parses a log level from a string.
    ///
    /// Recognizes: "quiet", "normal", "verbose" (case-insensitive).
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not recognized.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::LogLevel;
    ///
    /// assert_eq!(LogLevel::parse("quiet").unwrap(), LogLevel::Quiet);
    /// assert_eq!(LogLevel::parse("VERBOSE").unwrap(), LogLevel::Verbose);
    /// assert!(LogLevel::parse("invalid").is_err());
    /// ```
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "quiet" => Ok(Self::Quiet),
            "normal" => Ok(Self::Normal),
            "verbose" => Ok(Self::Verbose),
            _ => Err(format!("invalid log level: {s}")),
        }
    }
}

/// A simple stderr-based logger.
///
/// The logger respects the configured log level and only outputs messages
/// at or above that level.
///
/// # Examples
///
/// ```
/// use trop::{Logger, LogLevel};
///
/// let logger = Logger::new(LogLevel::Normal);
/// logger.error("This is an error message");
/// logger.info("This will not be printed (requires Verbose)");
/// ```
pub struct Logger {
    level: LogLevel,
}

impl Logger {
    /// Creates a new logger with the specified log level.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Logger, LogLevel};
    ///
    /// let logger = Logger::new(LogLevel::Verbose);
    /// ```
    #[must_use]
    pub const fn new(level: LogLevel) -> Self {
        Self { level }
    }

    /// Returns the current log level.
    #[must_use]
    pub const fn level(&self) -> LogLevel {
        self.level
    }

    /// Logs an error message.
    ///
    /// Error messages are always displayed unless the level is Quiet.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Logger, LogLevel};
    ///
    /// let logger = Logger::new(LogLevel::Normal);
    /// logger.error("Something went wrong");
    /// ```
    pub fn error(&self, message: &str) {
        if self.level >= LogLevel::Normal {
            eprintln!("ERROR: {message}");
        }
    }

    /// Logs a warning message.
    ///
    /// Warning messages are displayed at Normal and Verbose levels.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Logger, LogLevel};
    ///
    /// let logger = Logger::new(LogLevel::Normal);
    /// logger.warn("This might be a problem");
    /// ```
    pub fn warn(&self, message: &str) {
        if self.level >= LogLevel::Normal {
            eprintln!("WARN: {message}");
        }
    }

    /// Logs an informational message.
    ///
    /// Info messages are only displayed at Verbose level.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Logger, LogLevel};
    ///
    /// let logger = Logger::new(LogLevel::Verbose);
    /// logger.info("Processing started");
    /// ```
    pub fn info(&self, message: &str) {
        if self.level >= LogLevel::Verbose {
            eprintln!("INFO: {message}");
        }
    }

    /// Logs a debug message.
    ///
    /// Debug messages are only displayed at Verbose level.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Logger, LogLevel};
    ///
    /// let logger = Logger::new(LogLevel::Verbose);
    /// logger.debug("Port 8080 checked and available");
    /// ```
    pub fn debug(&self, message: &str) {
        if self.level >= LogLevel::Verbose {
            eprintln!("DEBUG: {message}");
        }
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new(LogLevel::Normal)
    }
}

/// Initializes a logger based on environment variables and CLI flags.
///
/// The priority order is:
/// 1. CLI flags (verbose/quiet)
/// 2. `TROP_LOG_MODE` environment variable
/// 3. Default (Normal)
///
/// # Arguments
///
/// * `verbose` - If true, sets level to Verbose
/// * `quiet` - If true, sets level to Quiet
///
/// If both `verbose` and `quiet` are true, `verbose` takes precedence.
///
/// # Examples
///
/// ```
/// use trop::init_logger;
///
/// // Use default (Normal) level
/// let logger = init_logger(false, false);
///
/// // Force verbose
/// let logger = init_logger(true, false);
///
/// // Force quiet
/// let logger = init_logger(false, true);
/// ```
#[must_use]
pub fn init_logger(verbose: bool, quiet: bool) -> Logger {
    // CLI flags take precedence
    if verbose {
        return Logger::new(LogLevel::Verbose);
    }
    if quiet {
        return Logger::new(LogLevel::Quiet);
    }

    // Check environment variable
    if let Ok(env_value) = env::var("TROP_LOG_MODE") {
        if let Ok(level) = LogLevel::parse(&env_value) {
            return Logger::new(level);
        }
    }

    // Default to Normal
    Logger::new(LogLevel::Normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Quiet < LogLevel::Normal);
        assert!(LogLevel::Normal < LogLevel::Verbose);
        assert!(LogLevel::Quiet < LogLevel::Verbose);
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(format!("{}", LogLevel::Quiet), "quiet");
        assert_eq!(format!("{}", LogLevel::Normal), "normal");
        assert_eq!(format!("{}", LogLevel::Verbose), "verbose");
    }

    #[test]
    fn test_log_level_parse() {
        assert_eq!(LogLevel::parse("quiet").unwrap(), LogLevel::Quiet);
        assert_eq!(LogLevel::parse("normal").unwrap(), LogLevel::Normal);
        assert_eq!(LogLevel::parse("verbose").unwrap(), LogLevel::Verbose);

        // Case insensitive
        assert_eq!(LogLevel::parse("QUIET").unwrap(), LogLevel::Quiet);
        assert_eq!(LogLevel::parse("Normal").unwrap(), LogLevel::Normal);
        assert_eq!(LogLevel::parse("VERBOSE").unwrap(), LogLevel::Verbose);

        // Invalid
        assert!(LogLevel::parse("invalid").is_err());
        assert!(LogLevel::parse("").is_err());
    }

    #[test]
    fn test_logger_creation() {
        let logger = Logger::new(LogLevel::Verbose);
        assert_eq!(logger.level(), LogLevel::Verbose);
    }

    #[test]
    fn test_logger_default() {
        let logger = Logger::default();
        assert_eq!(logger.level(), LogLevel::Normal);
    }

    #[test]
    fn test_init_logger_defaults() {
        // Save current env var if it exists
        let saved_env = env::var("TROP_LOG_MODE").ok();

        // Clear env var for this test
        env::remove_var("TROP_LOG_MODE");

        let logger = init_logger(false, false);
        assert_eq!(logger.level(), LogLevel::Normal);

        // Restore env var if it existed
        if let Some(val) = saved_env {
            env::set_var("TROP_LOG_MODE", val);
        }
    }

    #[test]
    fn test_init_logger_verbose_flag() {
        let logger = init_logger(true, false);
        assert_eq!(logger.level(), LogLevel::Verbose);
    }

    #[test]
    fn test_init_logger_quiet_flag() {
        let logger = init_logger(false, true);
        assert_eq!(logger.level(), LogLevel::Quiet);
    }

    #[test]
    fn test_init_logger_verbose_takes_precedence() {
        let logger = init_logger(true, true);
        assert_eq!(logger.level(), LogLevel::Verbose);
    }

    #[test]
    fn test_init_logger_from_env() {
        // Save current env var if it exists
        let saved_env = env::var("TROP_LOG_MODE").ok();

        env::set_var("TROP_LOG_MODE", "verbose");
        let logger = init_logger(false, false);
        assert_eq!(logger.level(), LogLevel::Verbose);

        env::set_var("TROP_LOG_MODE", "quiet");
        let logger = init_logger(false, false);
        assert_eq!(logger.level(), LogLevel::Quiet);

        // Restore env var if it existed, or remove if it didn't
        match saved_env {
            Some(val) => env::set_var("TROP_LOG_MODE", val),
            None => env::remove_var("TROP_LOG_MODE"),
        }
    }

    #[test]
    fn test_init_logger_env_invalid_fallback() {
        // Save current env var if it exists
        let saved_env = env::var("TROP_LOG_MODE").ok();

        env::set_var("TROP_LOG_MODE", "invalid");
        let logger = init_logger(false, false);
        // Should fall back to default (Normal)
        assert_eq!(logger.level(), LogLevel::Normal);

        // Restore env var if it existed, or remove if it didn't
        match saved_env {
            Some(val) => env::set_var("TROP_LOG_MODE", val),
            None => env::remove_var("TROP_LOG_MODE"),
        }
    }

    #[test]
    fn test_init_logger_cli_overrides_env() {
        // Save current env var if it exists
        let saved_env = env::var("TROP_LOG_MODE").ok();

        env::set_var("TROP_LOG_MODE", "normal");
        let logger = init_logger(true, false);
        // CLI flag should override env
        assert_eq!(logger.level(), LogLevel::Verbose);

        // Restore env var if it existed, or remove if it didn't
        match saved_env {
            Some(val) => env::set_var("TROP_LOG_MODE", val),
            None => env::remove_var("TROP_LOG_MODE"),
        }
    }

    // Note: We can't easily test the actual output of the logging methods
    // without capturing stderr, which is complex in unit tests. The methods
    // are simple enough that visual/integration testing is more appropriate.
}
