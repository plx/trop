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

use crate::verified::render;
use crate::{Port, Result};

pub use formatters::{DotenvFormatter, ExportFormatter, HumanFormatter, JsonFormatter};
pub use shell::ShellType;

/// Escapes user-controlled text (tags, projects, tasks, paths, raw input
/// echoed in diagnostics) for human-readable output.
///
/// Text without control characters that does not start with `"` is
/// returned unchanged. Anything else is wrapped in double quotes, with `"`,
/// `\`, newline, carriage return and tab written as `\"`, `\\`, `\n`, `\r`
/// and `\t`, and every other C0 control, DEL, or C1 control written as
/// `\xHH`. The result never contains a control character, so it cannot
/// start a new line, shift a tab-separated column, or send a terminal
/// escape sequence, and the escaping is reversible, so distinct texts stay
/// distinct.
///
/// # Examples
///
/// ```
/// use trop::output::display_field;
///
/// assert_eq!(display_field("web-server"), "web-server");
/// assert_eq!(display_field("evil\nservice: 1"), "\"evil\\nservice: 1\"");
/// assert_eq!(display_field("\u{1b}[31mred"), "\"\\x1b[31mred\"");
/// ```
#[must_use]
pub fn display_field(text: &str) -> String {
    render::escape_display_field(text)
}

/// Renders one tab-separated table row: each field is shown with
/// [`display_field`] and the fields are joined by tabs.
///
/// Because no shown field contains a tab, newline, or carriage return, the
/// row is a single line, and splitting it on tabs gives back exactly one
/// column per field.
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
