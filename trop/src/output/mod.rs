//! Output formatting module for port allocations.
//!
//! This module provides various output formats for displaying port allocations,
//! including shell-specific export statements, JSON, dotenv format, and
//! human-readable output.

mod formatters;
mod shell;

use std::collections::HashMap;

use crate::{Port, Result};

pub use formatters::{DotenvFormatter, ExportFormatter, HumanFormatter, JsonFormatter};
pub use shell::ShellType;

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
    ///   If None, service tags are converted to uppercase for variable names.
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
