//! CLI-specific error types with exit codes.
//!
//! This module defines error types specific to the CLI layer,
//! wrapping library errors and providing appropriate exit codes.

use std::fmt;
use trop::Error as LibError;

/// CLI-specific error type with exit code mapping.
#[derive(Debug)]
pub enum CliError {
    /// Library error (wrapped).
    Library(LibError),

    /// Invalid command-line arguments.
    InvalidArguments(String),

    /// I/O error.
    Io(std::io::Error),

    /// Timeout waiting for database lock.
    Timeout,

    /// Data directory not found (and auto-init disabled).
    NoDataDirectory,

    /// Configuration error.
    Config(String),

    /// Semantic failure (e.g., assertion failed) - exit code 1.
    SemanticFailure(String),
}

impl CliError {
    /// Get the appropriate exit code for this error.
    ///
    /// Exit codes:
    /// - 0: Success (not an error)
    /// - 1: Semantic failure (e.g., assertion failed)
    /// - 2: Timeout waiting for database lock
    /// - 3: No data directory found
    /// - 4: Invalid arguments
    /// - 5: I/O error
    /// - 6: Other library error
    /// - 7: Configuration error
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::SemanticFailure(_) => 1,
            CliError::Library(lib_err) => match lib_err {
                LibError::StickyFieldChange { .. } => 1,
                LibError::PathRelationshipViolation { .. } => 1,
                _ => 6,
            },
            CliError::Timeout => 2,
            CliError::NoDataDirectory => 3,
            CliError::InvalidArguments(_) => 4,
            CliError::Io(_) => 5,
            CliError::Config(_) => 7,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Library(e) => write!(f, "{e}"),
            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {msg}"),
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::Timeout => write!(f, "Timeout waiting for database lock"),
            CliError::NoDataDirectory => {
                write!(
                    f,
                    "Data directory not found (use --data-dir or enable auto-init)"
                )
            }
            CliError::Config(msg) => write!(f, "Configuration error: {msg}"),
            CliError::SemanticFailure(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CliError::Library(e) => Some(e),
            CliError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<LibError> for CliError {
    fn from(e: LibError) -> Self {
        // Check for specific error types that need special handling
        if matches!(e, LibError::LockTimeout { .. }) {
            CliError::Timeout
        } else {
            CliError::Library(e)
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}
