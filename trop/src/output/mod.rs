//! Output formatting module for port allocations.
//!
//! This module provides various output formats for displaying port allocations,
//! including shell-specific export statements, JSON, dotenv format, and
//! human-readable output.
//!
//! Ordering, name resolution checks, line rendering, shell detection, and
//! the escaping of user text are decided by the formally verified kernels
//! `verified::render` and `verified::shell` (see `docs/verification.md`).

mod formatters;
mod shell;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::verified::{civil_time, render};
use crate::{Port, Result};

pub use formatters::{DotenvFormatter, ExportFormatter, HumanFormatter, JsonFormatter};
pub use shell::ShellType;

/// Escapes user-controlled text (tags, projects, tasks, paths, raw input
/// echoed in diagnostics) for human-readable output.
///
/// The characters that are escaped are the control characters (C0, DEL
/// and C1, which include newline, carriage return, tab, ESC and NEL), the
/// line and paragraph separators `U+2028` and `U+2029`, and the Unicode bidi
/// controls (`U+061C`, `U+200E`, `U+200F`, `U+202A..=U+202E`,
/// `U+2066..=U+2069`).
///
/// Text without any of these that does not start with `"` is returned
/// unchanged. Anything else is wrapped in double quotes, with `"`, `\`,
/// newline, carriage return and tab written as `\"`, `\\`, `\n`, `\r` and
/// `\t`, every other control character written as `\xHH`, and each
/// separator or bidi control written as `\u{HHHH}`. The result contains
/// none of the escaped characters, so it cannot start a new line (for
/// terminals, or for readers such as Python's `str.splitlines` and
/// JavaScript that also split on `U+2028`/`U+2029`), shift a tab-separated
/// column, send a terminal control sequence, or carry a bidi control that
/// reorders the rest of its line. The escaping is reversible, so distinct
/// texts stay distinct. Every other character is shown as it is: right-to-
/// left letters are still laid out by the terminal's bidi rules, and other
/// invisible or look-alike characters are not marked.
///
/// # Examples
///
/// ```
/// use trop::output::display_field;
///
/// assert_eq!(display_field("web-server"), "web-server");
/// assert_eq!(display_field("evil\nservice: 1"), "\"evil\\nservice: 1\"");
/// assert_eq!(display_field("\u{1b}[31mred"), "\"\\x1b[31mred\"");
/// assert_eq!(display_field("web\u{202e}"), "\"web\\u{202e}\"");
/// ```
#[must_use]
pub fn display_field(text: &str) -> String {
    render::escape_display_field(text)
}

/// Renders one tab-separated table row: each field is shown with
/// [`display_field`] and the fields are joined by tabs.
///
/// Because no shown field contains a tab, newline, or carriage return, the
/// row is a single line. For one or more fields, splitting the row on tabs
/// gives back exactly one column per field. (No fields and a single empty
/// field both render as `""`, so callers should use a fixed, non-empty
/// column list.)
///
/// # Examples
///
/// ```
/// use trop::output::tab_row;
///
/// assert_eq!(tab_row(&["8080", "web", "-"]), "8080\tweb\t-");
/// assert_eq!(tab_row(&["a\tb", "c\nd"]), "\"a\\tb\"\t\"c\\nd\"");
/// assert_eq!(tab_row(&[]), "");
/// ```
#[must_use]
pub fn tab_row(fields: &[&str]) -> String {
    render::tab_row(fields)
}

/// Formats a point in time as UTC `YYYY-MM-DD HH:MM:SS` (chrono's
/// `%Y-%m-%d %H:%M:%S`), truncated to the second.
///
/// Dates are proleptic Gregorian with astronomical years. A year outside
/// `0..=9999` is written with a sign, as chrono writes it (`+10000-01-01
/// 00:00:00`, `-0001-12-31 23:59:59`). Every `SystemTime` is accepted, including
/// times before the epoch and beyond chrono's range. The conversion and the
/// text are decided by the formally verified kernel `verified::civil_time`
/// (see `docs/verification.md`).
///
/// # Examples
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
/// use trop::output::format_utc_timestamp;
///
/// assert_eq!(format_utc_timestamp(UNIX_EPOCH), "1970-01-01 00:00:00");
/// let leap_day = UNIX_EPOCH + Duration::from_secs(951_782_400);
/// assert_eq!(format_utc_timestamp(leap_day), "2000-02-29 00:00:00");
/// let before = UNIX_EPOCH - Duration::from_millis(1);
/// assert_eq!(format_utc_timestamp(before), "1969-12-31 23:59:59");
/// ```
#[must_use]
pub fn format_utc_timestamp(time: SystemTime) -> String {
    let (before_epoch, offset) = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => (false, after),
        Err(before) => (true, before.duration()),
    };
    civil_time::format_unix_instant(before_epoch, offset.as_secs(), offset.subsec_nanos())
}

/// Trait for formatting port allocations into different output formats.
pub trait OutputFormatter {
    /// Format the given port allocations into a string.
    ///
    /// # Arguments
    ///
    /// * `allocations` - Map from service tags to allocated ports
    ///
    /// # Errors
    ///
    /// Returns an error if the formatting fails (e.g., invalid environment variable names).
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String>;
}

/// Available output formats for port allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Shell-specific export statements.
    Export(ShellType),
    /// JSON format.
    Json,
    /// Dotenv (.env file) format.
    Dotenv,
    /// Human-readable format.
    Human,
}

impl OutputFormat {
    /// Create a formatter for this output format.
    ///
    /// # Arguments
    ///
    /// * `env_mappings` - Optional mapping from service tags to environment variable names.
    ///   Unmapped tags use ASCII-only uppercase/hyphen-to-underscore derivation,
    ///   and formatting fails if the result is not portable.
    #[must_use]
    pub fn create_formatter(
        &self,
        env_mappings: Option<HashMap<String, String>>,
    ) -> Box<dyn OutputFormatter> {
        match self {
            Self::Export(shell) => Box::new(ExportFormatter::new(*shell, env_mappings)),
            Self::Json => Box::new(JsonFormatter),
            Self::Dotenv => Box::new(DotenvFormatter::new(env_mappings)),
            Self::Human => Box::new(HumanFormatter),
        }
    }
}
