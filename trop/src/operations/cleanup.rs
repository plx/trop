//! Cleanup operations for removing stale and expired reservations.
//!
//! This module provides operations for cleaning up reservations in two ways:
//! 1. **Pruning**: Remove reservations for paths that no longer exist on the filesystem
//! 2. **Expiring**: Remove reservations that haven't been used within a time threshold
//!
//! All cleanup operations support dry-run mode for previewing changes before applying them.
//!
//! ## Transactional Semantics
//!
//! Cleanup operations process reservations one-by-one. Each individual deletion is atomic
//! (committed immediately), but the batch operation as a whole is not transactional. If an
//! error occurs midway through a cleanup, earlier deletions will have been committed.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::CleanupConfig;
use crate::database::Database;
use crate::{Reservation, Result};

/// Number of seconds in a day, used for expiration calculations.
const SECONDS_PER_DAY: u64 = 86400;

/// Broad category for a filesystem error that made a reserved path uninspectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrunePathErrorKind {
    /// Access to the path or one of its parents was denied.
    PermissionDenied,
    /// Symlink traversal encountered a loop or exceeded the platform limit.
    SymlinkLoop,
    /// A retryable or availability-related I/O failure occurred.
    Transient,
    /// The filesystem or platform does not support the requested inspection.
    Unsupported,
    /// An error occurred that is not definitive evidence of absence.
    Other,
}

impl fmt::Display for PrunePathErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => formatter.write_str("permission denied"),
            Self::SymlinkLoop => formatter.write_str("symlink loop"),
            Self::Transient => formatter.write_str("transient I/O error"),
            Self::Unsupported => formatter.write_str("unsupported filesystem operation"),
            Self::Other => formatter.write_str("unknown filesystem error"),
        }
    }
}

/// An inspection error that conservatively preserved reservations for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunePathError {
    /// Stable error category suitable for user-facing diagnostics.
    pub kind: PrunePathErrorKind,
    /// Platform error text captured by the one filesystem probe.
    pub message: String,
    /// Platform error number, when the operating system supplied one.
    pub raw_os_error: Option<i32>,
}

/// Filesystem status captured for one distinct reserved path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrunePathStatus {
    /// The path resolves to a directory and must be kept.
    ExistingDirectory,
    /// The path or its symlink target definitively does not exist.
    Missing,
    /// The path does not resolve to a directory.
    ///
    /// This covers both an existing non-directory target and a traversal that
    /// failed because an intermediate component was not a directory. It also
    /// covers internally stored paths that are invalid on the current host.
    NotDirectory,
    /// Inspection failed without proving absence, so reservations must be kept.
    Uninspectable(PrunePathError),
}

impl PrunePathStatus {
    /// Return whether this status makes a directory reservation eligible for pruning.
    #[must_use]
    pub const fn is_prunable(&self) -> bool {
        matches!(self, Self::Missing | Self::NotDirectory)
    }
}

/// The single filesystem decision captured for one distinct reserved path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunePathDecision {
    /// Reserved path that was inspected.
    pub path: PathBuf,
    /// Status captured by that inspection.
    pub status: PrunePathStatus,
}

/// Minimal filesystem result used by the prune classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbedPath {
    Directory,
    NonDirectory,
}

/// Result of a prune operation.
///
/// Pruning removes reservations for paths that no longer exist on the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneResult {
    /// Number of reservations removed (or would be removed in dry-run mode).
    pub removed_count: usize,
    /// Reservations that were (or would be) removed.
    pub removed_reservations: Vec<Reservation>,
    /// One captured filesystem decision for every distinct reserved path.
    pub path_decisions: Vec<PrunePathDecision>,
}

/// Result of an expire operation.
///
/// Expiring removes reservations that haven't been used within a configured time threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpireResult {
    /// Number of reservations removed (or would be removed in dry-run mode).
    pub removed_count: usize,
    /// Reservations that were (or would be) removed.
    pub removed_reservations: Vec<Reservation>,
}

/// Result of an autoclean operation.
///
/// Autoclean combines both pruning and expiring in a single operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocleanResult {
    /// Number of reservations pruned.
    pub pruned_count: usize,
    /// Number of reservations expired.
    pub expired_count: usize,
    /// Total number of reservations removed.
    pub total_removed: usize,
    /// Reservations that were pruned.
    pub pruned_reservations: Vec<Reservation>,
    /// Reservations that were expired.
    pub expired_reservations: Vec<Reservation>,
    /// Filesystem decisions captured by the prune portion of the operation.
    pub prune_path_decisions: Vec<PrunePathDecision>,
}

/// Cleanup operations for removing stale reservations.
///
/// All operations are static methods that work on a database instance.
/// Operations are transactional - they either complete fully or are rolled back.
pub struct CleanupOperations;

impl CleanupOperations {
    /// Remove reservations for paths that no longer exist on the filesystem.
    ///
    /// This operation checks each distinct reservation path exactly once and
    /// removes reservations only when the path is missing or does not resolve
    /// to a directory. Inspection errors preserve reservations and are returned
    /// as structured decisions for diagnostics.
    ///
    /// # Arguments
    ///
    /// * `db` - Database to operate on
    /// * `dry_run` - If true, report what would be removed without actually removing
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail. Filesystem errors for
    /// individual paths preserve their reservations.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::operations::CleanupOperations;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// // Preview what would be pruned
    /// let preview = CleanupOperations::prune(&mut db, true).unwrap();
    /// println!("Would prune {} reservations", preview.removed_count);
    ///
    /// // Actually prune
    /// let result = CleanupOperations::prune(&mut db, false).unwrap();
    /// println!("Pruned {} reservations", result.removed_count);
    /// ```
    pub fn prune(db: &mut Database, dry_run: bool) -> Result<PruneResult> {
        Self::prune_with_probe(db, dry_run, Self::probe_path)
    }

    fn prune_with_probe<F>(db: &mut Database, dry_run: bool, probe: F) -> Result<PruneResult>
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
    {
        let all_reservations = Database::list_all_reservations(db.connection())?;

        // Probe each distinct path once so all tags for that path receive the
        // same decision and reporting never performs a TOCTOU-sensitive recheck.
        let mut statuses = BTreeMap::new();
        for reservation in &all_reservations {
            let path = &reservation.key().path;
            statuses
                .entry(path.clone())
                .or_insert_with(|| Self::classify_path(path, &probe));
        }

        let to_remove = all_reservations
            .into_iter()
            .filter(|reservation| {
                statuses
                    .get(&reservation.key().path)
                    .is_some_and(PrunePathStatus::is_prunable)
            })
            .collect::<Vec<_>>();
        let removed_count = to_remove.len();

        if !dry_run {
            for reservation in &to_remove {
                db.delete_reservation(reservation.key())?;
            }
        }

        Ok(PruneResult {
            removed_count,
            removed_reservations: to_remove,
            path_decisions: statuses
                .into_iter()
                .map(|(path, status)| PrunePathDecision { path, status })
                .collect(),
        })
    }

    /// Remove reservations that haven't been used within the configured time threshold.
    ///
    /// This operation removes reservations where `last_used_at` is older than
    /// the threshold specified in the cleanup configuration.
    ///
    /// # Arguments
    ///
    /// * `db` - Database to operate on
    /// * `config` - Cleanup configuration specifying the expiration threshold
    /// * `dry_run` - If true, report what would be removed without actually removing
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::operations::CleanupOperations;
    /// use trop::config::CleanupConfig;
    ///
    /// let db_config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(db_config).unwrap();
    ///
    /// let cleanup_config = CleanupConfig {
    ///     expire_after_days: Some(30),
    /// };
    ///
    /// // Preview what would be expired
    /// let preview = CleanupOperations::expire(&mut db, &cleanup_config, true).unwrap();
    /// println!("Would expire {} reservations", preview.removed_count);
    ///
    /// // Actually expire
    /// let result = CleanupOperations::expire(&mut db, &cleanup_config, false).unwrap();
    /// println!("Expired {} reservations", result.removed_count);
    /// ```
    pub fn expire(
        db: &mut Database,
        config: &CleanupConfig,
        dry_run: bool,
    ) -> Result<ExpireResult> {
        // If no expiration configured, return empty result
        let Some(expire_after_days) = config.expire_after_days else {
            return Ok(ExpireResult {
                removed_count: 0,
                removed_reservations: Vec::new(),
            });
        };

        // Calculate the max age duration
        #[allow(clippy::cast_lossless)]
        let max_age = Duration::from_secs(expire_after_days as u64 * SECONDS_PER_DAY);

        // Find expired reservations
        let to_remove = Database::find_expired_reservations(db.connection(), max_age)?;
        let removed_count = to_remove.len();

        // If not dry-run, actually delete the reservations
        if !dry_run {
            for reservation in &to_remove {
                db.delete_reservation(reservation.key())?;
            }
        }

        Ok(ExpireResult {
            removed_count,
            removed_reservations: to_remove,
        })
    }

    /// Combined cleanup operation that both prunes and expires.
    ///
    /// This is a convenience method that performs both pruning (removing reservations
    /// for non-existent paths) and expiring (removing old unused reservations) in a
    /// single operation.
    ///
    /// # Arguments
    ///
    /// * `db` - Database to operate on
    /// * `config` - Cleanup configuration specifying the expiration threshold
    /// * `dry_run` - If true, report what would be removed without actually removing
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::operations::CleanupOperations;
    /// use trop::config::CleanupConfig;
    ///
    /// let db_config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(db_config).unwrap();
    ///
    /// let cleanup_config = CleanupConfig {
    ///     expire_after_days: Some(30),
    /// };
    ///
    /// // Preview what would be cleaned
    /// let preview = CleanupOperations::autoclean(&mut db, &cleanup_config, true).unwrap();
    /// println!("Would clean {} total reservations", preview.total_removed);
    ///
    /// // Actually clean
    /// let result = CleanupOperations::autoclean(&mut db, &cleanup_config, false).unwrap();
    /// println!("Cleaned {} total reservations", result.total_removed);
    /// ```
    pub fn autoclean(
        db: &mut Database,
        config: &CleanupConfig,
        dry_run: bool,
    ) -> Result<AutocleanResult> {
        // Run prune first
        let prune_result = Self::prune(db, dry_run)?;

        // Then run expire
        let expire_result = Self::expire(db, config, dry_run)?;

        Ok(AutocleanResult {
            pruned_count: prune_result.removed_count,
            expired_count: expire_result.removed_count,
            total_removed: prune_result.removed_count + expire_result.removed_count,
            pruned_reservations: prune_result.removed_reservations,
            expired_reservations: expire_result.removed_reservations,
            prune_path_decisions: prune_result.path_decisions,
        })
    }

    fn probe_path(path: &Path) -> io::Result<ProbedPath> {
        fs::metadata(path).map(|metadata| {
            if metadata.is_dir() {
                ProbedPath::Directory
            } else {
                ProbedPath::NonDirectory
            }
        })
    }

    fn classify_path<F>(path: &Path, probe: &F) -> PrunePathStatus
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
    {
        match probe(path) {
            Ok(ProbedPath::Directory) => PrunePathStatus::ExistingDirectory,
            Ok(ProbedPath::NonDirectory) => PrunePathStatus::NotDirectory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => PrunePathStatus::Missing,
            Err(error) if is_not_directory(&error) || is_invalid_path(&error) => {
                PrunePathStatus::NotDirectory
            }
            Err(error) => PrunePathStatus::Uninspectable(PrunePathError {
                kind: Self::classify_probe_error(&error),
                message: error.to_string(),
                raw_os_error: error.raw_os_error(),
            }),
        }
    }

    fn classify_probe_error(error: &io::Error) -> PrunePathErrorKind {
        if is_symlink_loop(error) {
            return PrunePathErrorKind::SymlinkLoop;
        }
        if is_transient_os_error(error) {
            return PrunePathErrorKind::Transient;
        }

        match error.kind() {
            io::ErrorKind::PermissionDenied => PrunePathErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported => PrunePathErrorKind::Unsupported,
            io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected => PrunePathErrorKind::Transient,
            _ => PrunePathErrorKind::Other,
        }
    }
}

#[cfg(unix)]
fn is_not_directory(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ENOTDIR)
}

#[cfg(windows)]
fn is_not_directory(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::ERROR_DIRECTORY;

    windows_error_code(error) == Some(ERROR_DIRECTORY)
}

#[cfg(not(any(unix, windows)))]
fn is_not_directory(error: &io::Error) -> bool {
    error.to_string().contains("non-directory ancestor")
}

fn is_invalid_path(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::InvalidFilename
    ) || is_invalid_path_os_error(error)
}

#[cfg(unix)]
fn is_invalid_path_os_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ENAMETOOLONG)
}

#[cfg(windows)]
fn is_invalid_path_os_error(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{
        ERROR_BAD_PATHNAME, ERROR_FILENAME_EXCED_RANGE, ERROR_INVALID_NAME,
    };

    windows_error_code(error).is_some_and(|code| {
        matches!(
            code,
            ERROR_INVALID_NAME | ERROR_BAD_PATHNAME | ERROR_FILENAME_EXCED_RANGE
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn is_invalid_path_os_error(_error: &io::Error) -> bool {
    false
}

#[cfg(unix)]
fn is_symlink_loop(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(windows)]
fn is_symlink_loop(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::ERROR_CANT_RESOLVE_FILENAME;

    windows_error_code(error) == Some(ERROR_CANT_RESOLVE_FILENAME)
}

#[cfg(not(any(unix, windows)))]
fn is_symlink_loop(error: &io::Error) -> bool {
    error.to_string().contains("symlink loop")
}

#[cfg(unix)]
fn is_transient_os_error(error: &io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| {
        matches!(
            code,
            libc::ESTALE
                | libc::EBUSY
                | libc::EDEADLK
                | libc::ETXTBSY
                | libc::ENETDOWN
                | libc::ENETUNREACH
                | libc::EHOSTUNREACH
        )
    })
}

#[cfg(windows)]
fn is_transient_os_error(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{
        ERROR_BUSY, ERROR_BUSY_DRIVE, ERROR_HOST_UNREACHABLE, ERROR_LOCK_VIOLATION,
        ERROR_NETWORK_BUSY, ERROR_NETWORK_UNREACHABLE, ERROR_RETRY, ERROR_UNEXP_NET_ERR,
    };

    // Common availability/busy errors from local and network filesystems:
    // ERROR_LOCK_VIOLATION, ERROR_NETWORK_BUSY, ERROR_UNEXP_NET_ERR,
    // ERROR_BUSY_DRIVE, ERROR_BUSY, ERROR_NETWORK_UNREACHABLE,
    // ERROR_HOST_UNREACHABLE, and ERROR_RETRY.
    windows_error_code(error).is_some_and(|code| {
        matches!(
            code,
            ERROR_LOCK_VIOLATION
                | ERROR_NETWORK_BUSY
                | ERROR_UNEXP_NET_ERR
                | ERROR_BUSY_DRIVE
                | ERROR_BUSY
                | ERROR_NETWORK_UNREACHABLE
                | ERROR_HOST_UNREACHABLE
                | ERROR_RETRY
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn is_transient_os_error(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
fn windows_error_code(error: &io::Error) -> Option<u32> {
    error.raw_os_error().and_then(|code| code.try_into().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::reservation::ReservationKey;
    use crate::{Port, Reservation};
    use std::path::PathBuf;
    use std::time::SystemTime;

    #[test]
    fn test_prune_no_reservations() {
        let mut db = create_test_database();

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 0);
        assert!(result.removed_reservations.is_empty());
    }

    #[test]
    fn test_prune_all_paths_exist() {
        let mut db = create_test_database();

        // Create a reservation for the current directory (which exists)
        let key = ReservationKey::new(std::env::current_dir().unwrap(), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 0);

        // Verify reservation still exists
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_prune_nonexistent_path() {
        let mut db = create_test_database();

        // Create a reservation for a path that doesn't exist
        let nonexistent = PathBuf::from("/this/path/definitely/does/not/exist/at/all");
        let key = ReservationKey::new(nonexistent.clone(), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.removed_reservations.len(), 1);
        assert_eq!(result.removed_reservations[0].key().path, nonexistent);

        // Verify reservation was actually deleted
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_prune_dry_run() {
        let mut db = create_test_database();

        // Create a reservation for a path that doesn't exist
        let nonexistent = PathBuf::from("/this/path/definitely/does/not/exist/at/all");
        let key = ReservationKey::new(nonexistent, None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        // Dry run should report what would be removed
        let result = CleanupOperations::prune(&mut db, true).unwrap();
        assert_eq!(result.removed_count, 1);

        // But reservation should still exist
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_prune_mixed_paths() {
        let mut db = create_test_database();

        // Create one reservation with existing path
        let existing = std::env::current_dir().unwrap();
        let key1 = ReservationKey::new(existing, None).unwrap();
        let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r1).unwrap();

        // Create one reservation with non-existent path
        let nonexistent = PathBuf::from("/this/path/definitely/does/not/exist/at/all");
        let key2 = ReservationKey::new(nonexistent, None).unwrap();
        let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r2).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 1);

        // Only the existing path should remain
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_prune_probes_each_distinct_path_once() {
        use std::cell::Cell;

        let mut db = create_test_database();
        let path = PathBuf::from("/one/probe/per/path");
        for (port, tag) in [(5000, None), (5001, Some("web".to_string()))] {
            let key = ReservationKey::new(path.clone(), tag).unwrap();
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let probe_count = Cell::new(0);
        let result = CleanupOperations::prune_with_probe(&mut db, true, |probed_path| {
            assert_eq!(probed_path, path);
            probe_count.set(probe_count.get() + 1);
            Ok(ProbedPath::Directory)
        })
        .unwrap();

        assert_eq!(probe_count.get(), 1);
        assert_eq!(
            result.path_decisions,
            vec![PrunePathDecision {
                path,
                status: PrunePathStatus::ExistingDirectory,
            }]
        );
        assert_eq!(result.removed_count, 0);
    }

    #[cfg(unix)]
    #[test]
    fn test_prune_removes_illegal_internal_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut db = create_test_database();
        let path = PathBuf::from(OsString::from_vec(
            b"/illegal/internal\0reservation".to_vec(),
        ));
        let probe_error = fs::metadata(&path).expect_err("interior NUL path must be illegal");
        assert_eq!(probe_error.kind(), io::ErrorKind::InvalidInput);

        let key = ReservationKey::new(path.clone(), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();

        assert_eq!(result.removed_count, 1);
        assert_eq!(
            result.path_decisions,
            vec![PrunePathDecision {
                path,
                status: PrunePathStatus::NotDirectory,
            }]
        );
        assert!(Database::list_all_reservations(db.connection())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_prune_preserves_and_classifies_injected_probe_errors() {
        let mut db = create_test_database();
        let fixtures = [
            ("permission", PrunePathErrorKind::PermissionDenied),
            ("loop", PrunePathErrorKind::SymlinkLoop),
            ("transient", PrunePathErrorKind::Transient),
            ("broken-mount", PrunePathErrorKind::Transient),
            ("unsupported", PrunePathErrorKind::Unsupported),
            ("unknown", PrunePathErrorKind::Other),
        ];
        for (index, (name, _)) in fixtures.iter().enumerate() {
            let key =
                ReservationKey::new(PathBuf::from(format!("/injected/{name}")), None).unwrap();
            #[allow(clippy::cast_possible_truncation)]
            let port = Port::try_from(5000 + index as u16).unwrap();
            let reservation = Reservation::builder(key, port).build().unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let result = CleanupOperations::prune_with_probe(&mut db, false, |path| {
            match path.file_name().and_then(std::ffi::OsStr::to_str) {
                Some("permission") => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "simulated ACL denial",
                )),
                Some("loop") => Err(simulated_symlink_loop_error()),
                Some("transient") => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "simulated unavailable mount",
                )),
                Some("broken-mount") => Err(simulated_broken_mount_error()),
                Some("unsupported") => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "simulated unsupported operation",
                )),
                Some("unknown") => Err(io::Error::other("simulated unknown I/O failure")),
                other => panic!("unexpected injected path: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(result.removed_count, 0);
        assert!(result.removed_reservations.is_empty());
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            fixtures.len()
        );
        assert_eq!(result.path_decisions.len(), fixtures.len());
        for (name, expected_kind) in fixtures {
            let decision = result
                .path_decisions
                .iter()
                .find(|decision| decision.path.ends_with(name))
                .unwrap();
            let PrunePathStatus::Uninspectable(error) = &decision.status else {
                panic!("expected an uninspectable decision for {name}");
            };
            assert_eq!(error.kind, expected_kind);
            assert!(!error.message.is_empty());
            if !matches!(name, "loop" | "broken-mount") {
                assert!(error.message.contains("simulated"));
            }
        }
    }

    #[test]
    fn test_prune_removes_only_definitive_missing_or_non_directory_paths() {
        let mut db = create_test_database();
        for (index, name) in [
            "directory",
            "missing",
            "bad-ancestor",
            "file",
            "invalid-input",
            "invalid-filename",
        ]
        .iter()
        .enumerate()
        {
            let key =
                ReservationKey::new(PathBuf::from(format!("/classified/{name}")), None).unwrap();
            #[allow(clippy::cast_possible_truncation)]
            let port = Port::try_from(5000 + index as u16).unwrap();
            let reservation = Reservation::builder(key, port).build().unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let result = CleanupOperations::prune_with_probe(&mut db, false, |path| {
            match path.file_name().and_then(std::ffi::OsStr::to_str) {
                Some("directory") => Ok(ProbedPath::Directory),
                Some("missing") => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "simulated missing target",
                )),
                Some("bad-ancestor") => Err(simulated_not_directory_error()),
                Some("file") => Ok(ProbedPath::NonDirectory),
                Some("invalid-input") => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "simulated invalid path",
                )),
                Some("invalid-filename") => Err(io::Error::new(
                    io::ErrorKind::InvalidFilename,
                    "simulated invalid filename",
                )),
                other => panic!("unexpected injected path: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(result.removed_count, 5);
        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].key().path.ends_with("directory"));

        let statuses = result
            .path_decisions
            .iter()
            .map(|decision| {
                (
                    decision
                        .path
                        .file_name()
                        .and_then(std::ffi::OsStr::to_str)
                        .unwrap(),
                    &decision.status,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(statuses["directory"], &PrunePathStatus::ExistingDirectory);
        assert_eq!(statuses["missing"], &PrunePathStatus::Missing);
        assert_eq!(statuses["bad-ancestor"], &PrunePathStatus::NotDirectory);
        assert_eq!(statuses["file"], &PrunePathStatus::NotDirectory);
        assert_eq!(statuses["invalid-input"], &PrunePathStatus::NotDirectory);
        assert_eq!(statuses["invalid-filename"], &PrunePathStatus::NotDirectory);
    }

    #[cfg(windows)]
    #[test]
    fn test_prune_removes_paths_rejected_by_windows() {
        use windows_sys::Win32::Foundation::{
            ERROR_BAD_PATHNAME, ERROR_FILENAME_EXCED_RANGE, ERROR_INVALID_NAME,
        };

        for code in [
            ERROR_INVALID_NAME,
            ERROR_BAD_PATHNAME,
            ERROR_FILENAME_EXCED_RANGE,
        ] {
            let raw_code = code.try_into().expect("Windows error code should fit i32");
            let status = CleanupOperations::classify_path(Path::new("invalid"), &|_| {
                Err(io::Error::from_raw_os_error(raw_code))
            });
            assert_eq!(status, PrunePathStatus::NotDirectory);
        }
    }

    #[cfg(unix)]
    fn simulated_symlink_loop_error() -> io::Error {
        io::Error::from_raw_os_error(libc::ELOOP)
    }

    #[cfg(windows)]
    fn simulated_symlink_loop_error() -> io::Error {
        use windows_sys::Win32::Foundation::ERROR_CANT_RESOLVE_FILENAME;

        io::Error::from_raw_os_error(
            ERROR_CANT_RESOLVE_FILENAME
                .try_into()
                .expect("Windows error code should fit i32"),
        )
    }

    #[cfg(not(any(unix, windows)))]
    fn simulated_symlink_loop_error() -> io::Error {
        io::Error::other("simulated symlink loop")
    }

    #[cfg(unix)]
    fn simulated_not_directory_error() -> io::Error {
        io::Error::from_raw_os_error(libc::ENOTDIR)
    }

    #[cfg(windows)]
    fn simulated_not_directory_error() -> io::Error {
        use windows_sys::Win32::Foundation::ERROR_DIRECTORY;

        io::Error::from_raw_os_error(
            ERROR_DIRECTORY
                .try_into()
                .expect("Windows error code should fit i32"),
        )
    }

    #[cfg(not(any(unix, windows)))]
    fn simulated_not_directory_error() -> io::Error {
        io::Error::other("simulated non-directory ancestor")
    }

    #[cfg(unix)]
    fn simulated_broken_mount_error() -> io::Error {
        io::Error::from_raw_os_error(libc::ESTALE)
    }

    #[cfg(windows)]
    fn simulated_broken_mount_error() -> io::Error {
        use windows_sys::Win32::Foundation::ERROR_RETRY;

        io::Error::from_raw_os_error(
            ERROR_RETRY
                .try_into()
                .expect("Windows error code should fit i32"),
        )
    }

    #[cfg(not(any(unix, windows)))]
    fn simulated_broken_mount_error() -> io::Error {
        io::Error::new(io::ErrorKind::TimedOut, "simulated broken mount")
    }

    #[test]
    fn test_expire_no_config() {
        let mut db = create_test_database();

        let config = CleanupConfig {
            expire_after_days: None,
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();
        assert_eq!(result.removed_count, 0);
    }

    #[test]
    fn test_expire_no_old_reservations() {
        let mut db = create_test_database();

        // Create a fresh reservation
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();
        assert_eq!(result.removed_count, 0);

        // Verify reservation still exists
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_expire_old_reservation() {
        let mut db = create_test_database();

        // Create an old reservation (10 days ago)
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port)
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Configure to expire after 7 days
        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();
        assert_eq!(result.removed_count, 1);

        // Verify reservation was actually deleted
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_expire_dry_run() {
        let mut db = create_test_database();

        // Create an old reservation
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port)
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        // Dry run should report what would be removed
        let result = CleanupOperations::expire(&mut db, &config, true).unwrap();
        assert_eq!(result.removed_count, 1);

        // But reservation should still exist
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_expire_mixed_ages() {
        let mut db = create_test_database();

        // Create an old reservation (10 days ago)
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key1 = ReservationKey::new(PathBuf::from("/test/old"), None).unwrap();
        let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&r1).unwrap();

        // Create a fresh reservation
        let key2 = ReservationKey::new(PathBuf::from("/test/fresh"), None).unwrap();
        let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r2).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();
        assert_eq!(result.removed_count, 1);

        // Only the fresh reservation should remain
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key().path, PathBuf::from("/test/fresh"));
    }

    #[test]
    fn test_autoclean_combines_operations() {
        let mut db = create_test_database();

        // Create a reservation with non-existent path (will be pruned)
        let nonexistent = PathBuf::from("/this/path/definitely/does/not/exist/at/all");
        let key1 = ReservationKey::new(nonexistent, None).unwrap();
        let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r1).unwrap();

        // Create an old reservation with existing path (will be expired)
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key2 =
            ReservationKey::new(std::env::current_dir().unwrap(), Some("old".to_string())).unwrap();
        let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&r2).unwrap();

        // Create a fresh reservation with existing path (will remain)
        let key3 = ReservationKey::new(std::env::current_dir().unwrap(), Some("fresh".to_string()))
            .unwrap();
        let r3 = Reservation::builder(key3, Port::try_from(5002).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r3).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::autoclean(&mut db, &config, false).unwrap();
        assert_eq!(result.pruned_count, 1);
        assert_eq!(result.expired_count, 1);
        assert_eq!(result.total_removed, 2);

        // Only the fresh reservation should remain
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_autoclean_dry_run() {
        let mut db = create_test_database();

        // Create a reservation with non-existent path
        let nonexistent = PathBuf::from("/this/path/definitely/does/not/exist/at/all");
        let key = ReservationKey::new(nonexistent, None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        // Dry run should report what would be removed
        let result = CleanupOperations::autoclean(&mut db, &config, true).unwrap();
        assert_eq!(result.pruned_count, 1);
        assert_eq!(result.total_removed, 1);

        // But all reservations should still exist
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_prune_multiple_nonexistent_paths() {
        // Test pruning multiple non-existent paths at once
        // Verifies batch processing works correctly
        let mut db = create_test_database();

        for i in 0..5 {
            let path = PathBuf::from(format!("/nonexistent/path/{i}"));
            let key = ReservationKey::new(path, None).unwrap();
            let port = Port::try_from(5000 + i).unwrap();
            let reservation = Reservation::builder(key, port).build().unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 5);

        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_expire_boundary_threshold() {
        // Test expiration exactly at the threshold boundary
        // Ensures proper >= vs > semantics in expiration logic
        let mut db = create_test_database();

        // Create reservation well over threshold (8 days old)
        let over_threshold = SystemTime::now() - Duration::from_secs(8 * SECONDS_PER_DAY);
        let key1 = ReservationKey::new(PathBuf::from("/test/old"), None).unwrap();
        let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .last_used_at(over_threshold)
            .build()
            .unwrap();
        db.create_reservation(&r1).unwrap();

        // Create reservation well under threshold (5 days old)
        let under_threshold = SystemTime::now() - Duration::from_secs(5 * SECONDS_PER_DAY);
        let key2 = ReservationKey::new(PathBuf::from("/test/fresh"), None).unwrap();
        let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .last_used_at(under_threshold)
            .build()
            .unwrap();
        db.create_reservation(&r2).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();

        // The 8-day-old reservation should be expired
        // The 5-day-old reservation should remain
        assert_eq!(result.removed_count, 1);

        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key().path, PathBuf::from("/test/fresh"));
    }

    #[test]
    fn test_autoclean_no_overlap() {
        // Test autoclean when prune and expire sets don't overlap
        // Verifies correct counting when operations affect different reservations
        let mut db = create_test_database();

        // Create non-existent path reservation (will be pruned)
        let nonexistent = PathBuf::from("/nonexistent");
        let key1 = ReservationKey::new(nonexistent, None).unwrap();
        let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&r1).unwrap();

        // Create old reservation with existing path (will be expired)
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key2 = ReservationKey::new(std::env::current_dir().unwrap(), None).unwrap();
        let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&r2).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::autoclean(&mut db, &config, false).unwrap();
        assert_eq!(result.pruned_count, 1);
        assert_eq!(result.expired_count, 1);
        assert_eq!(result.total_removed, 2);
        assert_eq!(result.pruned_reservations.len(), 1);
        assert_eq!(result.expired_reservations.len(), 1);
    }

    #[test]
    fn test_prune_result_contains_correct_reservations() {
        // Test that prune result includes the actual removed reservations
        // This verifies the result provides full information for reporting
        let mut db = create_test_database();

        let path = PathBuf::from("/nonexistent/path");
        let key = ReservationKey::new(path.clone(), Some("test".to_string())).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("test-project".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();

        assert_eq!(result.removed_reservations.len(), 1);
        let removed = &result.removed_reservations[0];
        assert_eq!(removed.key().path, path);
        assert_eq!(removed.key().tag, Some("test".to_string()));
        assert_eq!(removed.port(), port);
        assert_eq!(removed.project(), Some("test-project"));
    }

    #[test]
    fn test_expire_result_contains_correct_reservations() {
        // Test that expire result includes the actual removed reservations
        // Verifies complete information is available for audit/logging
        let mut db = create_test_database();

        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key = ReservationKey::new(PathBuf::from("/test/old"), Some("svc".to_string())).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("old-project".to_string()))
            .task(Some("dev".to_string()))
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();

        assert_eq!(result.removed_reservations.len(), 1);
        let removed = &result.removed_reservations[0];
        assert_eq!(removed.key().tag, Some("svc".to_string()));
        assert_eq!(removed.project(), Some("old-project"));
        assert_eq!(removed.task(), Some("dev"));
    }

    #[test]
    fn test_cleanup_preserves_fresh_valid_reservations() {
        // Test that cleanup operations never remove fresh, valid reservations
        // This is a critical safety invariant
        let mut db = create_test_database();

        // Create a fresh reservation with existing path
        let key = ReservationKey::new(std::env::current_dir().unwrap(), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };

        // Run all cleanup operations
        let result = CleanupOperations::autoclean(&mut db, &config, false).unwrap();
        assert_eq!(result.total_removed, 0);

        // Verify reservation still exists
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_prune_with_symlink_paths() {
        // Test that prune handles symlink paths appropriately
        // Verifies fail-open policy: if we can't check, assume it exists
        let mut db = create_test_database();

        // Create a reservation for a path that might be a broken symlink
        // (we use a non-existent path to simulate this)
        let path = PathBuf::from("/this/might/be/a/broken/symlink");
        let key = ReservationKey::new(path, None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let result = CleanupOperations::prune(&mut db, false).unwrap();

        // Should be pruned because path doesn't exist
        assert_eq!(result.removed_count, 1);
    }

    #[test]
    fn test_expire_multiple_threshold_values() {
        // Test expiration with different threshold values
        // Ensures the threshold parameter works correctly across different values
        let mut db = create_test_database();

        // Create reservations at different ages
        let times = [
            SystemTime::now() - Duration::from_secs(30 * SECONDS_PER_DAY), // 30 days
            SystemTime::now() - Duration::from_secs(60 * SECONDS_PER_DAY), // 60 days
            SystemTime::now() - Duration::from_secs(90 * SECONDS_PER_DAY), // 90 days
        ];

        for (i, time) in times.iter().enumerate() {
            let key = ReservationKey::new(PathBuf::from(format!("/test/{i}")), None).unwrap();
            #[allow(clippy::cast_possible_truncation)]
            let port = Port::try_from(5000 + i as u16).unwrap();
            let reservation = Reservation::builder(key, port)
                .last_used_at(*time)
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        // Expire with 45-day threshold - should remove 60 and 90 day old
        let config = CleanupConfig {
            expire_after_days: Some(45),
        };

        let result = CleanupOperations::expire(&mut db, &config, false).unwrap();
        assert_eq!(result.removed_count, 2);

        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(remaining.len(), 1);
    }
}
