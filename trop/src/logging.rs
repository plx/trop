//! Logging infrastructure for the trop library.
//!
//! This module provides a simple stderr-based logging system with
//! configurable log levels.
//!
//! Level names, the CLI/`TROP_LOG_MODE` precedence, and the severity filter
//! are decided by the formally verified kernel `verified::verbosity` (see
//! `docs/verification.md`); this module maps its results onto [`LogLevel`].

use std::env;
use std::fmt;

use crate::output::display_field;
use crate::verified::verbosity::{self, Severity, Verbosity};

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
    /// Suppress all non-essential output: the logger prints nothing, not
    /// even error messages. (The CLI reports a command's fatal error itself,
    /// not through the logger.)
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
    /// The kernel's verbosity.
    const fn verbosity(self) -> Verbosity {
        match self {
            Self::Quiet => Verbosity::Quiet,
            Self::Normal => Verbosity::Normal,
            Self::Verbose => Verbosity::Verbose,
        }
    }

    /// The level of a kernel verbosity.
    const fn from_verbosity(verbosity: Verbosity) -> Self {
        match verbosity {
            Verbosity::Quiet => Self::Quiet,
            Verbosity::Normal => Self::Normal,
            Verbosity::Verbose => Self::Verbose,
        }
    }

    /// Parses a log level from a string.
    ///
    /// Recognizes: "quiet", "normal", "verbose" (ASCII case-insensitive, no
    /// surrounding whitespace).
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not recognized. The error shows the
    /// input with control characters escaped.
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
        verbosity::parse_verbosity(s.as_bytes())
            .map(Self::from_verbosity)
            .ok_or_else(|| format!("invalid log level: {}", display_field(s)))
    }
}

/// A simple stderr-based logger.
///
/// The logger prints a message when the configured level reaches the
/// message's severity: `Quiet` prints nothing, `Normal` prints errors and
/// warnings, and `Verbose` also prints info and debug messages.
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

    /// Whether a message of this severity is printed at this logger's level.
    const fn emits(&self, severity: Severity) -> bool {
        verbosity::should_emit(self.level.verbosity(), severity)
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
        if self.emits(Severity::Error) {
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
        if self.emits(Severity::Warn) {
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
        if self.emits(Severity::Info) {
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
        if self.emits(Severity::Debug) {
            eprintln!("DEBUG: {message}");
        }
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new(LogLevel::Normal)
    }
}

/// Resolves the effective log level from the CLI flags and a raw
/// `TROP_LOG_MODE` value.
///
/// The priority order is:
/// 1. `--verbose`
/// 2. `--quiet`
/// 3. `log_mode`, when it names a level (ASCII case-insensitive); any other
///    value is ignored
/// 4. Default (Normal)
///
/// # Examples
///
/// ```
/// use trop::logging::resolve_log_level;
/// use trop::LogLevel;
///
/// assert_eq!(resolve_log_level(true, true, Some("quiet")), LogLevel::Verbose);
/// assert_eq!(resolve_log_level(false, true, Some("verbose")), LogLevel::Quiet);
/// assert_eq!(resolve_log_level(false, false, Some("QUIET")), LogLevel::Quiet);
/// assert_eq!(resolve_log_level(false, false, Some("loud")), LogLevel::Normal);
/// assert_eq!(resolve_log_level(false, false, None), LogLevel::Normal);
/// ```
#[must_use]
pub fn resolve_log_level(verbose: bool, quiet: bool, log_mode: Option<&str>) -> LogLevel {
    LogLevel::from_verbosity(verbosity::resolve_verbosity(
        verbose,
        quiet,
        log_mode.map(str::as_bytes),
    ))
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
/// If both `verbose` and `quiet` are true, `verbose` takes precedence. See
/// [`resolve_log_level`].
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
    let log_mode = env::var("TROP_LOG_MODE").ok();
    Logger::new(resolve_log_level(verbose, quiet, log_mode.as_deref()))
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

    #[test]
    fn test_log_level_display_round_trips_through_parse() {
        for level in [LogLevel::Quiet, LogLevel::Normal, LogLevel::Verbose] {
            assert_eq!(LogLevel::parse(&level.to_string()).unwrap(), level);
            assert_eq!(
                LogLevel::parse(&level.to_string().to_ascii_uppercase()).unwrap(),
                level
            );
        }
    }

    #[test]
    fn test_log_level_parse_matches_previous_unicode_lowercase_matching() {
        fn previous(s: &str) -> Option<LogLevel> {
            match s.to_lowercase().as_str() {
                "quiet" => Some(LogLevel::Quiet),
                "normal" => Some(LogLevel::Normal),
                "verbose" => Some(LogLevel::Verbose),
                _ => None,
            }
        }
        for input in [
            "quiet",
            "QUIET",
            "Quiet",
            "normal",
            "NoRmAl",
            "verbose",
            "VERBOSE",
            "",
            " quiet",
            "quiet ",
            "quie",
            "quiett",
            "loud",
            "\u{130}",
            "\u{212a}",
            "verbos\u{e9}",
            "nor\nmal",
        ] {
            assert_eq!(LogLevel::parse(input).ok(), previous(input), "{input:?}");
        }
    }

    #[test]
    fn test_log_level_parse_error_escapes_control_characters() {
        let message = LogLevel::parse("loud\n\u{1b}[2J").unwrap_err();
        assert!(!message.contains('\n'));
        assert!(!message.contains('\u{1b}'));
        assert_eq!(message, "invalid log level: \"loud\\n\\x1b[2J\"");
        assert_eq!(
            LogLevel::parse("loud").unwrap_err(),
            "invalid log level: loud"
        );
    }

    #[test]
    fn test_resolve_log_level_precedence_table() {
        let envs = [
            None,
            Some("quiet"),
            Some("NORMAL"),
            Some("Verbose"),
            Some("bogus"),
            Some(""),
        ];
        for verbose in [false, true] {
            for quiet in [false, true] {
                for env in envs {
                    let expected = if verbose {
                        LogLevel::Verbose
                    } else if quiet {
                        LogLevel::Quiet
                    } else {
                        env.and_then(|value| LogLevel::parse(value).ok())
                            .unwrap_or(LogLevel::Normal)
                    };
                    assert_eq!(
                        resolve_log_level(verbose, quiet, env),
                        expected,
                        "{verbose} {quiet} {env:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_logger_severity_filter_table() {
        let cases = [
            (LogLevel::Quiet, [false, false, false, false]),
            (LogLevel::Normal, [true, true, false, false]),
            (LogLevel::Verbose, [true, true, true, true]),
        ];
        for (level, expected) in cases {
            let logger = Logger::new(level);
            let actual = [
                logger.emits(Severity::Error),
                logger.emits(Severity::Warn),
                logger.emits(Severity::Info),
                logger.emits(Severity::Debug),
            ];
            assert_eq!(actual, expected, "{level}");
        }
    }
}
