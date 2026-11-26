//! CLI-specific error types with exit codes.
//!
//! This module defines error types specific to the CLI layer,
//! wrapping library errors and providing appropriate exit codes.

use std::fmt;
use trop::cli_support::{exit_code_for, ErrorClass};
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

    /// Data directory not found (and auto-init disabled).
    NoDataDirectory,

    /// No data directory could be determined (no `--data-dir` or
    /// `TROP_DATA_DIR`, and no usable home directory).
    DataDirectoryUndetermined(String),

    /// Configuration error.
    Config(String),

    /// Semantic failure (e.g., assertion failed) - exit code 1.
    SemanticFailure(String),
}

impl CliError {
    /// The failure class of this error (the input to the exit-code table).
    pub fn class(&self) -> ErrorClass {
        match self {
            CliError::SemanticFailure(_) => ErrorClass::Semantic,
            // Every variant is listed (no wildcard), so a new library error
            // must be classified before `trop-cli` compiles.
            CliError::Library(lib_err) => match lib_err {
                LibError::LockTimeout { .. } => ErrorClass::LockTimeout,
                LibError::StickyFieldChange { .. } => ErrorClass::StickyField,
                LibError::PathRelationshipViolation { .. } | LibError::UnrelatedPath { .. } => {
                    ErrorClass::PathRelationship
                }
                LibError::DataDirectoryNotFound { .. } => ErrorClass::NoDataDir,
                LibError::InvalidPort { .. }
                | LibError::InvalidPath { .. }
                | LibError::Database(_)
                | LibError::Configuration(_)
                | LibError::Io(_)
                | LibError::Validation { .. }
                | LibError::PortUnavailable { .. }
                | LibError::ReservationConflict { .. }
                | LibError::NotFound { .. }
                | LibError::InvalidPortRange { .. }
                | LibError::DatabaseCorruption { .. }
                | LibError::UnsupportedSchemaVersion { .. }
                | LibError::MigrationRequired { .. }
                | LibError::MigrationBlocked { .. }
                | LibError::PathNotFound { .. }
                | LibError::PermissionDenied { .. }
                | LibError::SymlinkLoop { .. }
                | LibError::PortExhausted { .. }
                | LibError::OccupancyCheckFailed { .. }
                | LibError::PreferredPortUnavailable { .. }
                | LibError::GroupAllocationFailed { .. } => ErrorClass::OtherLibrary,
            },
            CliError::NoDataDirectory | CliError::DataDirectoryUndetermined(_) => {
                ErrorClass::NoDataDir
            }
            CliError::InvalidArguments(_) => ErrorClass::InvalidArguments,
            CliError::Io(_) => ErrorClass::Io,
            CliError::Config(_) => ErrorClass::Config,
        }
    }

    /// Get the appropriate exit code for this error.
    ///
    /// The verified kernel `verified::cli::exit_code_for` maps the error's
    /// class to the specification's table:
    /// - 0: Success (never produced for an error)
    /// - 1: Semantic failure (e.g., assertion failed, refused sticky-field or
    ///   unrelated-path change)
    /// - 2: Timeout waiting for database lock (and nothing else)
    /// - 3: No data directory found or determinable
    /// - 4: Invalid arguments (including command-line usage errors)
    /// - 5: I/O error
    /// - 6: Other library error
    /// - 7: Configuration error
    pub fn exit_code(&self) -> i32 {
        exit_code_for(self.class())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Library(e) => write!(f, "{e}"),
            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {msg}"),
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::NoDataDirectory => {
                write!(
                    f,
                    "Data directory not found (use --data-dir or enable auto-init)"
                )
            }
            CliError::DataDirectoryUndetermined(msg) => {
                write!(f, "Data directory not determined: {msg}")
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
        CliError::Library(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The specification's exit-code table, for every CLI error class.
    #[test]
    fn exit_codes_follow_the_specification_table() {
        let cases: Vec<(CliError, i32)> = vec![
            (CliError::SemanticFailure("no".into()), 1),
            (
                CliError::Library(LibError::LockTimeout {
                    timeout: std::time::Duration::from_secs(1),
                    operation: "x".into(),
                }),
                2,
            ),
            (CliError::NoDataDirectory, 3),
            (CliError::DataDirectoryUndetermined("no home".into()), 3),
            (CliError::InvalidArguments("bad".into()), 4),
            (CliError::Io(std::io::Error::other("io")), 5),
            (
                CliError::Library(LibError::Validation {
                    field: "f".into(),
                    message: "m".into(),
                }),
                6,
            ),
            (CliError::Config("bad".into()), 7),
            // The library's own "data directory not found" is the
            // specification's exit 3, and its "unrelated path" is a path
            // relationship violation (exit 1), like
            // `PathRelationshipViolation`.
            (
                CliError::Library(LibError::DataDirectoryNotFound {
                    path: "/nowhere".into(),
                }),
                3,
            ),
            (
                CliError::Library(LibError::UnrelatedPath {
                    path: "/elsewhere".into(),
                }),
                1,
            ),
            (
                CliError::Library(LibError::PathRelationshipViolation {
                    details: "unrelated".into(),
                }),
                1,
            ),
            (
                CliError::Library(LibError::StickyFieldChange {
                    field: "project".into(),
                    details: "d".into(),
                }),
                1,
            ),
        ];
        for (error, code) in cases {
            assert_eq!(error.exit_code(), code, "{error:?}");
        }
    }
}
