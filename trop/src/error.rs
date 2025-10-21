//! Error types for the trop library.
//!
//! This module provides a comprehensive error hierarchy for all operations
//! in the trop library, using `thiserror` for ergonomic error handling.

use std::path::PathBuf;

use thiserror::Error;

/// Result type alias for operations that may fail with a trop error.
///
/// # Examples
///
/// ```
/// use trop::{Error, Result};
///
/// fn example_operation() -> Result<u16> {
///     Ok(8080)
/// }
/// ```
pub type Result<T> = std::result::Result<T, Error>;

/// The main error type for the trop library.
///
/// This enum encompasses all possible error conditions that can occur
/// during port reservation operations.
#[derive(Debug, Error)]
pub enum Error {
    /// An invalid port number was provided.
    #[error("invalid port {value}: {reason}")]
    InvalidPort {
        /// The invalid port value.
        value: u16,
        /// The reason the port is invalid.
        reason: String,
    },

    /// An invalid filesystem path was provided.
    #[error("invalid path {}: {reason}", path.display())]
    InvalidPath {
        /// The invalid path.
        path: PathBuf,
        /// The reason the path is invalid.
        reason: String,
    },

    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// A configuration error occurred.
    #[error("configuration error: {0}")]
    Configuration(#[from] serde_yaml::Error),

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A validation error occurred.
    #[error("validation error for '{field}': {message}")]
    Validation {
        /// The field that failed validation.
        field: String,
        /// A description of the validation failure.
        message: String,
    },

    /// No ports are available in the requested range.
    #[error("no ports available: {reason}")]
    PortUnavailable {
        /// The reason no ports are available.
        reason: String,
    },

    /// A reservation conflict occurred.
    #[error("reservation conflict: {details}")]
    ReservationConflict {
        /// Details about the conflict.
        details: String,
    },

    /// The requested resource was not found.
    #[error("not found: {resource}")]
    NotFound {
        /// The resource that was not found.
        resource: String,
    },

    /// An invalid port range was specified.
    #[error("invalid port range {min}-{max}: {reason}")]
    InvalidPortRange {
        /// The minimum port in the range.
        min: u16,
        /// The maximum port in the range.
        max: u16,
        /// The reason the range is invalid.
        reason: String,
    },

    /// A database lock timeout occurred.
    #[error("database lock timeout after {seconds}s")]
    LockTimeout {
        /// The number of seconds waited before timing out.
        seconds: u64,
    },

    /// The data directory was not found and auto-initialization is disabled.
    #[error("data directory not found: {}", path.display())]
    DataDirectoryNotFound {
        /// The expected path to the data directory.
        path: PathBuf,
    },

    /// Database corruption was detected.
    #[error("database corruption detected: {details}")]
    DatabaseCorruption {
        /// Details about the corruption.
        details: String,
    },

    /// An unsupported schema version was encountered.
    #[error("unsupported schema version: expected {expected}, found {found}")]
    UnsupportedSchemaVersion {
        /// The expected schema version.
        expected: u32,
        /// The schema version found in the database.
        found: u32,
    },

    /// A path operation attempted to modify an unrelated path.
    #[error("cannot modify unrelated path: {}", path.display())]
    UnrelatedPath {
        /// The path that was attempted to be modified.
        path: PathBuf,
    },

    /// Attempted to change a sticky field without the appropriate flag.
    #[error("cannot change sticky field '{field}': {details}")]
    StickyFieldChange {
        /// The field that was attempted to be changed.
        field: String,
        /// Details about the attempted change.
        details: String,
    },

    /// A path does not exist.
    #[error("path not found: {}", path.display())]
    PathNotFound {
        /// The path that was not found.
        path: PathBuf,
    },

    /// Permission denied accessing a path.
    #[error("permission denied: {}", path.display())]
    PermissionDenied {
        /// The path that could not be accessed.
        path: PathBuf,
    },

    /// A symlink loop was detected.
    #[error("symlink loop detected: {}", path.display())]
    SymlinkLoop {
        /// The path where the loop was detected.
        path: PathBuf,
    },

    /// A path relationship violation occurred.
    #[error("path relationship violation: {details}")]
    PathRelationshipViolation {
        /// Details about the violation.
        details: String,
    },

    /// No ports are available in the specified range.
    #[error("port range {range} exhausted{}", if *.tried_cleanup { " after cleanup" } else { "" })]
    PortExhausted {
        /// The port range that was exhausted.
        range: crate::port::PortRange,
        /// Whether cleanup was attempted.
        tried_cleanup: bool,
    },

    /// Port occupancy check failed.
    #[error("occupancy check failed for port {port}: {source}")]
    OccupancyCheckFailed {
        /// The port that failed the check.
        port: crate::port::Port,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A preferred port is unavailable.
    #[error("preferred port {port} unavailable: {reason}")]
    PreferredPortUnavailable {
        /// The preferred port that was unavailable.
        port: crate::port::Port,
        /// The reason the port is unavailable.
        reason: PortUnavailableReason,
    },

    /// Group allocation failed.
    #[error("group allocation failed after attempting {attempted} service(s): {reason}")]
    GroupAllocationFailed {
        /// Number of services attempted before failure.
        attempted: usize,
        /// The reason for failure.
        reason: String,
    },
}

/// Reason why a port is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortUnavailableReason {
    /// Port is already reserved in the database.
    Reserved,
    /// Port is in the exclusion list.
    Excluded,
    /// Port is currently occupied on the system.
    Occupied,
}

impl std::fmt::Display for PortUnavailableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reserved => write!(f, "reserved"),
            Self::Excluded => write!(f, "excluded"),
            Self::Occupied => write!(f, "occupied"),
        }
    }
}

// Additional conversions for better ergonomics

impl From<crate::port::InvalidPortError> for Error {
    fn from(err: crate::port::InvalidPortError) -> Self {
        Self::InvalidPort {
            value: err.value,
            reason: err.reason,
        }
    }
}

impl From<crate::port::InvalidPortRangeError> for Error {
    fn from(err: crate::port::InvalidPortRangeError) -> Self {
        Self::InvalidPortRange {
            min: err.min.value(),
            max: err.max.value(),
            reason: err.reason,
        }
    }
}

impl From<crate::reservation::ValidationError> for Error {
    fn from(err: crate::reservation::ValidationError) -> Self {
        Self::Validation {
            field: err.field,
            message: err.message,
        }
    }
}

impl Error {
    /// Check if error indicates a path does not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Error;
    /// use std::path::PathBuf;
    ///
    /// let err = Error::PathNotFound { path: PathBuf::from("/nonexistent") };
    /// assert!(err.is_not_found());
    /// ```
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::PathNotFound { .. })
    }

    /// Check if error is permission-related.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Error;
    /// use std::path::PathBuf;
    ///
    /// let err = Error::PermissionDenied { path: PathBuf::from("/restricted") };
    /// assert!(err.is_permission_denied());
    /// ```
    #[must_use]
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::PermissionDenied { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_port_error() {
        let err = Error::InvalidPort {
            value: 0,
            reason: "port 0 is invalid".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("invalid port"));
        assert!(display.contains('0'));
    }

    #[test]
    fn test_invalid_path_error() {
        let err = Error::InvalidPath {
            path: PathBuf::from("/invalid/path"),
            reason: "does not exist".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("invalid path"));
        let normalized = display.replace(std::path::MAIN_SEPARATOR, "/");
        assert!(normalized.contains("/invalid/path"));
        assert!(display.contains("does not exist"));
    }

    #[test]
    fn test_validation_error() {
        let err = Error::Validation {
            field: "project".to_string(),
            message: "must be non-empty".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("validation error"));
        assert!(display.contains("project"));
        assert!(display.contains("must be non-empty"));
    }

    #[test]
    fn test_port_unavailable_error() {
        let err = Error::PortUnavailable {
            reason: "all ports in range are occupied".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("no ports available"));
        assert!(display.contains("all ports in range are occupied"));
    }

    #[test]
    fn test_reservation_conflict_error() {
        let err = Error::ReservationConflict {
            details: "port already reserved for different project".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("reservation conflict"));
        assert!(display.contains("already reserved"));
    }

    #[test]
    fn test_not_found_error() {
        let err = Error::NotFound {
            resource: "reservation for /path/to/project".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("not found"));
        assert!(display.contains("reservation"));
    }

    #[test]
    fn test_invalid_port_range_error() {
        let err = Error::InvalidPortRange {
            min: 5000,
            max: 4000,
            reason: "max must be >= min".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("invalid port range"));
        assert!(display.contains("5000-4000"));
    }

    #[test]
    fn test_lock_timeout_error() {
        let err = Error::LockTimeout { seconds: 5 };
        let display = format!("{err}");
        assert!(display.contains("lock timeout"));
        assert!(display.contains('5'));
    }

    #[test]
    fn test_data_directory_not_found_error() {
        let err = Error::DataDirectoryNotFound {
            path: PathBuf::from("/home/user/.trop"),
        };
        let display = format!("{err}");
        assert!(display.contains("data directory not found"));
        assert!(display.contains(".trop"));
    }

    #[test]
    fn test_database_corruption_error() {
        let err = Error::DatabaseCorruption {
            details: "invalid schema version".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("corruption"));
        assert!(display.contains("invalid schema version"));
    }

    #[test]
    fn test_unsupported_schema_version_error() {
        let err = Error::UnsupportedSchemaVersion {
            expected: 1,
            found: 2,
        };
        let display = format!("{err}");
        assert!(display.contains("unsupported schema version"));
        assert!(display.contains("expected 1"));
        assert!(display.contains("found 2"));
    }

    #[test]
    fn test_unrelated_path_error() {
        let err = Error::UnrelatedPath {
            path: PathBuf::from("/unrelated/path"),
        };
        let display = format!("{err}");
        assert!(display.contains("unrelated path"));
        let normalized = display.replace(std::path::MAIN_SEPARATOR, "/");
        assert!(normalized.contains("/unrelated/path"));
    }

    #[test]
    fn test_sticky_field_change_error() {
        let err = Error::StickyFieldChange {
            field: "project".to_string(),
            details: "use --force to override".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("sticky field"));
        assert!(display.contains("project"));
        assert!(display.contains("--force"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        let display = format!("{err}");
        assert!(display.contains("I/O error"));
    }

    #[test]
    fn test_result_type_alias() {
        fn returns_result() -> Result<u16> {
            Err(Error::InvalidPort {
                value: 0,
                reason: "test".to_string(),
            })
        }

        assert!(returns_result().is_err());
    }
}
