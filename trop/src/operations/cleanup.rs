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
//! Each cleanup invocation reads, evaluates, revalidates, and deletes its
//! candidates inside one `IMMEDIATE` transaction. A live invocation commits
//! every selected deletion together or rolls them all back. Dry-run uses the
//! same transaction-scoped selection path and commits no database changes.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::CleanupConfig;
use crate::database::Database;
use crate::verified::cleanup::{
    self as kernel, AbsenceEvidence, Candidate, CleanupReason, Deletion, Elapsed, ErrorFacts,
    IoKindClass, Observation, PathStatusKind, ProbeOutcome, PruneErrorKind, Selection, Settlement,
};
use crate::{Reservation, Result};
use rusqlite::Connection;

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

impl PrunePathErrorKind {
    /// The kernel's kind, variant for variant.
    const fn from_kernel(kind: PruneErrorKind) -> Self {
        match kind {
            PruneErrorKind::PermissionDenied => Self::PermissionDenied,
            PruneErrorKind::SymlinkLoop => Self::SymlinkLoop,
            PruneErrorKind::Transient => Self::Transient,
            PruneErrorKind::Unsupported => Self::Unsupported,
            PruneErrorKind::Other => Self::Other,
        }
    }

    /// This kind as the kernel's, variant for variant.
    const fn kernel(self) -> PruneErrorKind {
        match self {
            Self::PermissionDenied => PruneErrorKind::PermissionDenied,
            Self::SymlinkLoop => PruneErrorKind::SymlinkLoop,
            Self::Transient => PruneErrorKind::Transient,
            Self::Unsupported => PruneErrorKind::Unsupported,
            Self::Other => PruneErrorKind::Other,
        }
    }
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
        kernel::status_prunable(self.kernel())
    }

    /// This status as the kernel's, variant for variant.
    const fn kernel(&self) -> PathStatusKind {
        match self {
            Self::ExistingDirectory => PathStatusKind::ExistingDirectory,
            Self::Missing => PathStatusKind::Missing,
            Self::NotDirectory => PathStatusKind::NotDirectory,
            Self::Uninspectable(error) => PathStatusKind::Uninspectable(error.kind.kernel()),
        }
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
    /// Number of reservations evaluated by this invocation.
    pub considered_count: usize,
    /// Number of evaluated reservations preserved.
    pub preserved_count: usize,
    /// Number of reservations removed (or would be removed in dry-run mode).
    pub removed_count: usize,
    /// Reservations evaluated by this invocation.
    pub considered_reservations: Vec<Reservation>,
    /// Evaluated reservations that were preserved.
    pub preserved_reservations: Vec<Reservation>,
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
    /// Number of reservations evaluated by this invocation.
    pub considered_count: usize,
    /// Number of evaluated reservations preserved.
    pub preserved_count: usize,
    /// Number of reservations removed (or would be removed in dry-run mode).
    pub removed_count: usize,
    /// Reservations evaluated by this invocation.
    pub considered_reservations: Vec<Reservation>,
    /// Evaluated reservations that were preserved.
    pub preserved_reservations: Vec<Reservation>,
    /// Reservations that were (or would be) removed.
    pub removed_reservations: Vec<Reservation>,
}

/// Result of an autoclean operation.
///
/// Autoclean combines both pruning and expiring in a single operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocleanResult {
    /// Number of reservations evaluated once by the combined invocation.
    pub considered_count: usize,
    /// Number of evaluated reservations preserved.
    pub preserved_count: usize,
    /// Number of reservations pruned.
    pub pruned_count: usize,
    /// Number of reservations expired.
    pub expired_count: usize,
    /// Total number of reservations removed.
    pub total_removed: usize,
    /// Reservations evaluated once by the combined invocation.
    pub considered_reservations: Vec<Reservation>,
    /// Evaluated reservations that were preserved.
    pub preserved_reservations: Vec<Reservation>,
    /// One deduplicated set of reservations removed (or selected in dry-run mode).
    pub removed_reservations: Vec<Reservation>,
    /// Reservations that were pruned.
    pub pruned_reservations: Vec<Reservation>,
    /// Reservations that were expired and were not already selected by prune.
    pub expired_reservations: Vec<Reservation>,
    /// Filesystem decisions captured by the prune portion of the operation.
    pub prune_path_decisions: Vec<PrunePathDecision>,
}

/// Cleanup operations for removing stale reservations.
///
/// All operations are static methods that work on a database instance.
/// Operations are transactional - they either complete fully or are rolled back.
pub struct CleanupOperations;

/// A removal candidate: the reservation as planned, its reason (`Prune` or
/// `Expire`), and whether its path was prunable.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedRemoval {
    reservation: Reservation,
    reason: CleanupReason,
    path_prunable: bool,
}

struct CleanupExecution {
    considered_reservations: Vec<Reservation>,
    preserved_reservations: Vec<Reservation>,
    removed: Vec<PlannedRemoval>,
    path_decisions: Vec<PrunePathDecision>,
}

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
        let timeout = db.busy_timeout();
        let execution = Self::execute_cleanup(
            db,
            kernel::command_selection(true, None),
            dry_run,
            probe,
            || {},
        )
        .map_err(|error| error.classify_sqlite_lock(timeout, "pruning reservations"))?;
        let removed_reservations = execution
            .removed
            .into_iter()
            .map(|removal| removal.reservation)
            .collect::<Vec<_>>();

        Ok(PruneResult {
            considered_count: execution.considered_reservations.len(),
            preserved_count: execution.preserved_reservations.len(),
            removed_count: removed_reservations.len(),
            considered_reservations: execution.considered_reservations,
            preserved_reservations: execution.preserved_reservations,
            removed_reservations,
            path_decisions: execution.path_decisions,
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
        Self::expire_with_candidate_barrier(db, config, dry_run, || {})
    }

    fn expire_with_candidate_barrier<F>(
        db: &mut Database,
        config: &CleanupConfig,
        dry_run: bool,
        after_candidate_discovery: F,
    ) -> Result<ExpireResult>
    where
        F: FnOnce(),
    {
        // If no expiration configured, return empty result
        let Some(expire_after_days) = config.expire_after_days else {
            return Ok(ExpireResult {
                considered_count: 0,
                preserved_count: 0,
                removed_count: 0,
                considered_reservations: Vec::new(),
                preserved_reservations: Vec::new(),
                removed_reservations: Vec::new(),
            });
        };

        let timeout = db.busy_timeout();
        let execution = Self::execute_cleanup(
            db,
            kernel::command_selection(false, Some(expire_after_days)),
            dry_run,
            Self::probe_path,
            after_candidate_discovery,
        )
        .map_err(|error| error.classify_sqlite_lock(timeout, "expiring reservations"))?;
        let removed_reservations = execution
            .removed
            .into_iter()
            .map(|removal| removal.reservation)
            .collect::<Vec<_>>();

        Ok(ExpireResult {
            considered_count: execution.considered_reservations.len(),
            preserved_count: execution.preserved_reservations.len(),
            removed_count: removed_reservations.len(),
            considered_reservations: execution.considered_reservations,
            preserved_reservations: execution.preserved_reservations,
            removed_reservations,
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
        let timeout = db.busy_timeout();
        let execution = Self::execute_cleanup(
            db,
            kernel::command_selection(true, config.expire_after_days),
            dry_run,
            Self::probe_path,
            || {},
        )
        .map_err(|error| {
            error.classify_sqlite_lock(timeout, "automatically cleaning reservations")
        })?;

        Ok(Self::autoclean_result(execution))
    }

    /// Run the enabled automatic-cleanup phases inside a transaction already
    /// owned by a reserve invocation.
    ///
    /// Enabled expiry uses `config.expire_after_days`, or the built-in 30-day
    /// threshold when it is unset (verified `automatic_selection`), so a
    /// `cleanup` section without `expire_after_days` expires exactly as a
    /// missing section does.
    pub(crate) fn automatic_cleanup_in_connection<B>(
        conn: &Connection,
        config: &CleanupConfig,
        prune: bool,
        expire: bool,
        after_candidate_discovery: B,
    ) -> Result<AutocleanResult>
    where
        B: FnOnce(),
    {
        let execution = Self::execute_cleanup_in_connection(
            conn,
            kernel::automatic_selection(prune, expire, config.expire_after_days),
            false,
            Self::probe_path,
            after_candidate_discovery,
        )?;

        Ok(Self::autoclean_result(execution))
    }

    /// Splits the removals by reason (verified `split_by_reason`: the pruned
    /// and expired lists partition the removals, each in removal order).
    fn autoclean_result(execution: CleanupExecution) -> AutocleanResult {
        let reasons: Vec<CleanupReason> = execution
            .removed
            .iter()
            .map(|removal| removal.reason)
            .collect();
        let (pruned, expired) = kernel::split_by_reason(&reasons);
        let reservations_at = |indices: Vec<usize>| -> Vec<Reservation> {
            indices
                .into_iter()
                .map(|index| execution.removed[index].reservation.clone())
                .collect()
        };
        let pruned_reservations = reservations_at(pruned);
        let expired_reservations = reservations_at(expired);
        let removed_reservations = execution
            .removed
            .into_iter()
            .map(|removal| removal.reservation)
            .collect::<Vec<_>>();

        AutocleanResult {
            considered_count: execution.considered_reservations.len(),
            preserved_count: execution.preserved_reservations.len(),
            pruned_count: pruned_reservations.len(),
            expired_count: expired_reservations.len(),
            total_removed: removed_reservations.len(),
            considered_reservations: execution.considered_reservations,
            preserved_reservations: execution.preserved_reservations,
            removed_reservations,
            pruned_reservations,
            expired_reservations,
            prune_path_decisions: execution.path_decisions,
        }
    }

    fn execute_cleanup<F, B>(
        db: &mut Database,
        selection: Selection,
        dry_run: bool,
        probe: F,
        after_candidate_discovery: B,
    ) -> Result<CleanupExecution>
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
        B: FnOnce(),
    {
        // Acquire the writer slot before reading candidates. This gives the
        // entire invocation one linearization point relative to reserve,
        // reserve-group, and other cleanup writers.
        let transaction = db.begin_transaction()?;
        let execution = Self::execute_cleanup_in_connection(
            &transaction,
            selection,
            dry_run,
            probe,
            after_candidate_discovery,
        )?;
        transaction.commit()?;
        Ok(execution)
    }

    fn execute_cleanup_in_connection<F, B>(
        conn: &Connection,
        selection: Selection,
        dry_run: bool,
        probe: F,
        after_candidate_discovery: B,
    ) -> Result<CleanupExecution>
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
        B: FnOnce(),
    {
        let evaluated_at = Self::captured_evaluation_time();
        let considered_reservations = Database::list_all_reservations(conn)?;

        // Probe each distinct path once inside the transaction so all tags for
        // that path share one observation. The selected reservation snapshot
        // below pairs that observation with the row version it justified.
        let (path_prunable, path_decisions) = if selection.prune {
            Self::probe_distinct_paths(&considered_reservations, &probe)
        } else {
            (vec![false; considered_reservations.len()], Vec::new())
        };

        // The verified kernel decides each reservation's reason (prune takes
        // precedence, so the two sets are disjoint) and splits the scan into
        // the preserved rows and the removal candidates in removal order:
        // prune candidates in path/tag scan order, then expiration
        // candidates from oldest to newest, ties in scan order.
        let candidates: Vec<Candidate> = considered_reservations
            .iter()
            .zip(&path_prunable)
            .map(|(reservation, &prunable)| Candidate {
                reason: kernel::cleanup_reason(
                    selection.prune,
                    prunable,
                    selection.max_age_secs,
                    Self::elapsed_since_last_use(reservation, evaluated_at),
                ),
                last_used_secs: Self::stored_unix_secs(reservation.last_used_at()),
            })
            .collect();
        let plan = kernel::plan_removals(&candidates);
        let mut preserved_reservations: Vec<Reservation> = plan
            .keep
            .iter()
            .map(|&index| considered_reservations[index].clone())
            .collect();
        let planned_removals: Vec<PlannedRemoval> = plan
            .removals
            .iter()
            .map(|&index| PlannedRemoval {
                reservation: considered_reservations[index].clone(),
                reason: candidates[index].reason,
                path_prunable: path_prunable[index],
            })
            .collect();

        after_candidate_discovery();

        // Revalidate the complete persisted row snapshot immediately before
        // each guarded DELETE (verified `revalidate_all`): a changed or
        // vanished row is not deleted, a delete is only requested in a live
        // run for an eligible row a read answered `Unchanged` for, and a dry
        // run reads and deletes nothing. The kernel's spec cannot state call
        // order over `Fn` oracles; that the read is the latest one before
        // the delete rests on the kernel loop's structure (one read per row,
        // its delete right after it) and on the guarded DELETE, which
        // compares the whole row with the planned snapshot in one statement.
        //
        // The eligibility re-check uses the planning evidence (the path
        // status and evaluation time of this transaction, and the planned
        // row, which an `Unchanged` read equals), so it holds for every
        // planned candidate (verified
        // `lemma_unchanged_candidate_still_eligible`): `Fate::Ineligible`
        // cannot occur here, and is kept as defense in depth.
        let eligible: Vec<bool> = planned_removals
            .iter()
            .map(|planned| {
                kernel::still_eligible(
                    planned.reason,
                    planned.path_prunable,
                    selection.max_age_secs,
                    Self::elapsed_since_last_use(&planned.reservation, evaluated_at),
                )
            })
            .collect();
        let observe = |index: usize| -> Result<Observation<Reservation>> {
            let planned = &planned_removals[index].reservation;
            Ok(match Database::get_reservation(conn, planned.key())? {
                // A database trigger may have removed another selected row as
                // part of an earlier guarded delete in this transaction. It is
                // still an invocation removal and must be reported.
                None => Observation::Vanished,
                Some(current) if current == *planned => Observation::Unchanged,
                Some(current) => Observation::Changed(current),
            })
        };
        let delete = |index: usize| -> Result<Deletion<Reservation>> {
            let planned = &planned_removals[index].reservation;
            if Database::delete_reservation_if_unchanged(conn, planned)? {
                return Ok(Deletion::Deleted);
            }
            Ok(match Database::get_reservation(conn, planned.key())? {
                Some(current) => Deletion::StillPresent(current),
                None => Deletion::Vanished,
            })
        };
        let fates =
            kernel::revalidate_all(&eligible, &observe, &delete, dry_run).map_err(|failure| {
                log::debug!(
                    "Cleanup revalidation failed for {}",
                    planned_removals[failure.index].reservation.key()
                );
                failure.error
            })?;

        // The verified `settle` places each fate in exactly one list, so
        // the considered rows are the preserved ones plus the removed ones.
        let mut removed = Vec::with_capacity(planned_removals.len());
        for (planned, fate) in planned_removals.into_iter().zip(fates) {
            match kernel::settle(fate) {
                Settlement::Removed => removed.push(planned),
                Settlement::PreservedAsPlanned => preserved_reservations.push(planned.reservation),
                Settlement::PreservedAs(current) => preserved_reservations.push(current),
            }
        }

        Ok(CleanupExecution {
            considered_reservations,
            preserved_reservations,
            removed,
            path_decisions,
        })
    }

    /// Probes each distinct reserved path once, at its first reservation
    /// (verified `representatives`), and returns whether each reservation's
    /// path is prunable together with one decision per distinct path, in
    /// path order.
    fn probe_distinct_paths<F>(
        considered: &[Reservation],
        probe: &F,
    ) -> (Vec<bool>, Vec<PrunePathDecision>)
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
    {
        // One id per distinct path (`Path` equality), numbered in order of
        // first appearance, so every id is below `considered.len()`.
        let mut ids_by_path: BTreeMap<&Path, usize> = BTreeMap::new();
        let ids: Vec<usize> = considered
            .iter()
            .map(|reservation| {
                let next = ids_by_path.len();
                *ids_by_path
                    .entry(reservation.key().path.as_path())
                    .or_insert(next)
            })
            .collect();
        let representatives = kernel::representatives(&ids);

        let statuses: Vec<Option<PrunePathStatus>> = considered
            .iter()
            .enumerate()
            .map(|(index, reservation)| {
                (representatives[index] == index)
                    .then(|| Self::classify_path(&reservation.key().path, probe))
            })
            .collect();
        let path_prunable = representatives
            .iter()
            .map(|&first| {
                statuses[first]
                    .as_ref()
                    .is_some_and(PrunePathStatus::is_prunable)
            })
            .collect();

        let mut path_decisions: Vec<PrunePathDecision> = considered
            .iter()
            .zip(statuses)
            .filter_map(|(reservation, status)| {
                status.map(|status| PrunePathDecision {
                    path: reservation.key().path.clone(),
                    status,
                })
            })
            .collect();
        path_decisions.sort_by(|left, right| left.path.cmp(&right.path));
        (path_prunable, path_decisions)
    }

    /// The time since `reservation`'s last use, as `SystemTime` measures it,
    /// or `None` when the last use lies after `evaluated_at`.
    fn elapsed_since_last_use(
        reservation: &Reservation,
        evaluated_at: SystemTime,
    ) -> Option<Elapsed> {
        evaluated_at
            .duration_since(reservation.last_used_at())
            .ok()
            .map(|age| Elapsed {
                secs: age.as_secs(),
                nanos: age.subsec_nanos(),
            })
    }

    /// Whole seconds since the Unix epoch of a stored timestamp. Stored
    /// timestamps are whole, non-negative seconds (`unix_secs_to_systemtime`),
    /// so this is exact for every reservation read from the database.
    fn stored_unix_secs(time: SystemTime) -> u64 {
        time.duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since_epoch| since_epoch.as_secs())
    }

    fn captured_evaluation_time() -> SystemTime {
        let now = SystemTime::now();
        now.duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| {
                SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(elapsed.as_secs()))
            })
            .unwrap_or(now)
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

    /// Probes `path` once and classifies the result (verified
    /// `classify_probe`). The glue extracts independent facts from the
    /// `io::Error` (the platform tables below); the kernel decides their
    /// precedence and which outcomes are prunable.
    fn classify_path<F>(path: &Path, probe: &F) -> PrunePathStatus
    where
        F: Fn(&Path) -> io::Result<ProbedPath>,
    {
        let (outcome, failure) = match probe(path) {
            Ok(ProbedPath::Directory) => (ProbeOutcome::Directory, None),
            Ok(ProbedPath::NonDirectory) => (ProbeOutcome::NonDirectory, None),
            Err(error) => (
                ProbeOutcome::Failed(absence_evidence(&error), error_facts(&error)),
                Some(error),
            ),
        };
        match kernel::classify_probe(outcome) {
            PathStatusKind::ExistingDirectory => PrunePathStatus::ExistingDirectory,
            PathStatusKind::Missing => PrunePathStatus::Missing,
            PathStatusKind::NotDirectory => PrunePathStatus::NotDirectory,
            // The kernel reports `Uninspectable` only for a failed probe, so
            // `failure` holds its error.
            PathStatusKind::Uninspectable(kind) => PrunePathStatus::Uninspectable(PrunePathError {
                kind: PrunePathErrorKind::from_kernel(kind),
                message: failure
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                raw_os_error: failure.as_ref().and_then(io::Error::raw_os_error),
            }),
        }
    }
}

/// The facts of a failed probe that can prove the path is not a directory.
fn absence_evidence(error: &io::Error) -> AbsenceEvidence {
    AbsenceEvidence {
        not_found: error.kind() == io::ErrorKind::NotFound,
        not_a_directory: is_not_directory(error),
        invalid_path: is_invalid_path(error),
    }
}

/// The facts that classify a failed probe that proves nothing.
fn error_facts(error: &io::Error) -> ErrorFacts {
    ErrorFacts {
        symlink_loop: is_symlink_loop(error),
        transient_os: is_transient_os_error(error),
        kind: io_kind_class(error.kind()),
    }
}

/// The class of an `io::ErrorKind`.
fn io_kind_class(kind: io::ErrorKind) -> IoKindClass {
    match kind {
        io::ErrorKind::PermissionDenied => IoKindClass::PermissionDenied,
        io::ErrorKind::Unsupported => IoKindClass::Unsupported,
        io::ErrorKind::Interrupted
        | io::ErrorKind::WouldBlock
        | io::ErrorKind::TimedOut
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::NotConnected => IoKindClass::Transient,
        _ => IoKindClass::Other,
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
    use crate::config::ConfigBuilder;
    use crate::database::test_util::create_test_database;
    use crate::database::DatabaseConfig;
    use crate::operations::{PlanExecutor, ReserveOptions, ReservePlan};
    use crate::reservation::ReservationKey;
    use crate::verified::cleanup::SECONDS_PER_DAY;
    use crate::{Port, Reservation};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::SystemTime;
    use tempfile::tempdir;

    #[test]
    fn test_prune_no_reservations() {
        let mut db = create_test_database();

        let result = CleanupOperations::prune(&mut db, false).unwrap();
        assert_eq!(result.removed_count, 0);
        assert!(
            result.removed_reservations.is_empty(),
            "unexpected removals: {:?}",
            result.removed_reservations
        );
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
    fn test_prune_result_partitions_considered_preserved_and_removed_rows() {
        let mut db = create_test_database();
        let existing_path = std::env::current_dir().unwrap();
        let existing = Reservation::builder(
            ReservationKey::new(existing_path, None).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .build()
        .unwrap();
        let missing = Reservation::builder(
            ReservationKey::new(PathBuf::from("/missing-cleanup-result"), None).unwrap(),
            Port::try_from(5001).unwrap(),
        )
        .build()
        .unwrap();
        db.create_reservation(&existing).unwrap();
        db.create_reservation(&missing).unwrap();

        let result = CleanupOperations::prune(&mut db, true).unwrap();

        assert_eq!(result.considered_count, 2);
        assert_eq!(result.preserved_count, 1);
        assert_eq!(result.removed_count, 1);
        assert_eq!(result.considered_reservations.len(), 2);
        assert_eq!(result.preserved_reservations.len(), 1);
        assert_eq!(result.preserved_reservations[0].key(), existing.key());
        assert_eq!(result.removed_reservations.len(), 1);
        assert_eq!(result.removed_reservations[0].key(), missing.key());
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
        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert!(
            remaining.is_empty(),
            "unexpected reservations: {remaining:?}"
        );
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
        assert!(
            result.removed_reservations.is_empty(),
            "unexpected removals: {:?}",
            result.removed_reservations
        );
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
            assert!(
                !error.message.is_empty(),
                "error message should not be empty"
            );
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
    fn test_expire_and_autoclean_preserve_oldest_first_result_order() {
        let mut db = create_test_database();
        let existing_path = std::env::current_dir().unwrap();
        let now = SystemTime::now();
        for (tag, port, age_days) in [
            ("a-newer", 5000, 20),
            ("z-oldest", 5001, 30),
            ("m-middle", 5002, 25),
        ] {
            let key = ReservationKey::new(existing_path.clone(), Some(tag.to_string())).unwrap();
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .last_used_at(now - Duration::from_secs(age_days * SECONDS_PER_DAY))
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };
        let expected_tags = ["z-oldest", "m-middle", "a-newer"];

        let expired = CleanupOperations::expire(&mut db, &config, true).unwrap();
        assert_eq!(
            expired
                .removed_reservations
                .iter()
                .map(|reservation| reservation.key().tag.as_deref().unwrap())
                .collect::<Vec<_>>(),
            expected_tags
        );

        let autoclean = CleanupOperations::autoclean(&mut db, &config, true).unwrap();
        assert_eq!(
            autoclean
                .expired_reservations
                .iter()
                .map(|reservation| reservation.key().tag.as_deref().unwrap())
                .collect::<Vec<_>>(),
            expected_tags
        );
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

    #[test]
    fn test_cleanup_delete_failure_rolls_back_every_candidate() {
        let mut db = create_test_database();

        for (index, path) in [
            "/cleanup-failure/a",
            "/cleanup-failure/b",
            "/cleanup-failure/c",
        ]
        .into_iter()
        .enumerate()
        {
            let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
            let port = Port::try_from(5100 + u16::try_from(index).unwrap()).unwrap();
            db.create_reservation(&Reservation::builder(key, port).build().unwrap())
                .unwrap();
        }

        db.connection()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_second_cleanup_delete
                 BEFORE DELETE ON reservations
                 WHEN OLD.path = '/cleanup-failure/b'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected cleanup delete failure');
                 END;",
            )
            .unwrap();

        let error =
            CleanupOperations::prune_with_probe(&mut db, false, |_| Ok(ProbedPath::NonDirectory))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected cleanup delete failure"),
            "unexpected cleanup error: {error}"
        );

        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(
            remaining.len(),
            3,
            "a failed cleanup invocation must not commit earlier deletions"
        );
    }

    #[test]
    fn test_autoclean_deduplicates_overlap_and_matches_live_selection() {
        let mut db = create_test_database();
        let old_time = SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY);
        let key = ReservationKey::new(PathBuf::from("/missing-and-expired"), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5200).unwrap())
            .last_used_at(old_time)
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };
        let preview = CleanupOperations::autoclean(&mut db, &config, true).unwrap();
        let live = CleanupOperations::autoclean(&mut db, &config, false).unwrap();

        assert_eq!(preview.total_removed, 1);
        assert_eq!(preview.pruned_count, 1);
        assert_eq!(preview.expired_count, 0);
        assert_eq!(preview.total_removed, live.total_removed);
        assert_eq!(preview.removed_reservations.len(), 1);
        assert_eq!(preview.removed_reservations, live.removed_reservations);
        assert_eq!(preview.pruned_reservations, live.pruned_reservations);
        assert_eq!(preview.expired_reservations, live.expired_reservations);
    }

    #[test]
    fn test_expire_serializes_replacement_after_candidate_discovery() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("cleanup-race.db");
        let database_config =
            DatabaseConfig::new(&database_path).with_busy_timeout(Duration::from_secs(2));
        let mut cleanup_db = Database::open(database_config.clone()).unwrap();
        let mut replacement_db = Database::open(database_config).unwrap();

        let key = ReservationKey::new(PathBuf::from("/expire-race"), None).unwrap();
        let old_reservation = Reservation::builder(key.clone(), Port::try_from(5300).unwrap())
            .last_used_at(SystemTime::now() - Duration::from_secs(10 * SECONDS_PER_DAY))
            .build()
            .unwrap();
        cleanup_db.create_reservation(&old_reservation).unwrap();

        let reserve_config = ConfigBuilder::new().build().unwrap();
        let replacement_key = key.clone();
        let (start_sender, start_receiver) = mpsc::sync_channel(0);
        let (attempt_sender, attempt_receiver) = mpsc::sync_channel(0);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let replacement_thread = thread::spawn(move || {
            start_receiver.recv().unwrap();
            attempt_sender.send(()).unwrap();
            let result = (|| -> Result<()> {
                let transaction = replacement_db.begin_transaction()?;
                let options =
                    ReserveOptions::new(replacement_key, Some(Port::try_from(5300).unwrap()))
                        .with_allow_unrelated_path(true)
                        .with_ignore_occupied(true);
                let plan = ReservePlan::new(options, &reserve_config).build_plan(&transaction)?;
                PlanExecutor::new(&transaction).execute(&plan)?;
                transaction.commit()?;
                Ok(())
            })();
            let _ = done_sender.send(());
            result
        });

        let config = CleanupConfig {
            expire_after_days: Some(7),
        };
        CleanupOperations::expire_with_candidate_barrier(
            &mut cleanup_db,
            &config,
            false,
            move || {
                start_sender.send(()).unwrap();
                attempt_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("replacement connection did not start");
                let _ = done_receiver.recv_timeout(Duration::from_millis(100));
            },
        )
        .unwrap();
        replacement_thread.join().unwrap().unwrap();

        let surviving_replacement = Database::get_reservation(cleanup_db.connection(), &key)
            .unwrap()
            .expect("a replacement racing stale cleanup evidence must survive");
        assert_eq!(surviving_replacement.port(), Port::try_from(5300).unwrap());
        assert!(
            surviving_replacement
                .last_used_at()
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(5)),
            "the surviving row must be the fresh replacement"
        );
    }

    // -----------------------------------------------------------------------
    // Glue and trusted-assumption checks for the verified cleanup kernels
    // -----------------------------------------------------------------------

    /// A deterministic pseudo-random sequence (a linear congruential
    /// generator), so the differential tests need no extra dependency.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % bound
        }
    }

    fn at_unix_secs(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn reservation_last_used_at(last_used_at: SystemTime) -> Reservation {
        let key = ReservationKey::new(PathBuf::from("/glue/expiry"), None).unwrap();
        Reservation::builder(key, Port::try_from(5000).unwrap())
            .last_used_at(last_used_at)
            .build()
            .unwrap()
    }

    /// The previous predicate: strictly older than `max_age`, and a last use
    /// in the future never expires.
    fn old_is_expired_at(
        reservation: &Reservation,
        evaluated_at: SystemTime,
        max_age: Duration,
    ) -> bool {
        evaluated_at
            .duration_since(reservation.last_used_at())
            .is_ok_and(|age| age > max_age)
    }

    #[test]
    fn test_glue_expiry_decision_matches_duration_comparison_at_the_boundary() {
        let max_age_secs = 7 * SECONDS_PER_DAY;
        let now_secs = 100 * SECONDS_PER_DAY;
        let evaluated_at = at_unix_secs(now_secs);
        let cases = [
            // Exactly the maximum age: kept.
            (at_unix_secs(now_secs - max_age_secs), false),
            // One second, or one nanosecond, more: expired.
            (at_unix_secs(now_secs - max_age_secs - 1), true),
            (
                at_unix_secs(now_secs - max_age_secs) - Duration::from_nanos(1),
                true,
            ),
            // Used now, or in the future: kept.
            (at_unix_secs(now_secs), false),
            (at_unix_secs(now_secs + 1), false),
            (SystemTime::UNIX_EPOCH, true),
        ];
        for (last_used_at, expected) in cases {
            let reservation = reservation_last_used_at(last_used_at);
            let age = CleanupOperations::elapsed_since_last_use(&reservation, evaluated_at);
            let reason = kernel::cleanup_reason(false, false, Some(max_age_secs), age);
            assert_eq!(
                reason == CleanupReason::Expire,
                expected,
                "{last_used_at:?}"
            );
            assert_eq!(
                old_is_expired_at(
                    &reservation,
                    evaluated_at,
                    Duration::from_secs(max_age_secs)
                ),
                expected
            );
            assert_eq!(
                kernel::still_eligible(CleanupReason::Expire, false, Some(max_age_secs), age),
                expected
            );
            // Without an expire phase nothing expires.
            assert_eq!(
                kernel::cleanup_reason(false, false, None, age),
                CleanupReason::Keep
            );
        }
    }

    #[test]
    fn test_glue_expiry_decision_matches_old_predicate_on_random_times() {
        let mut rng = Lcg(7);
        for _ in 0..5_000 {
            let now_secs = rng.below(1 << 40);
            let evaluated_at = at_unix_secs(now_secs);
            let last_used_at = at_unix_secs(rng.below(1 << 40))
                + Duration::from_nanos(rng.below(2) * rng.below(1_000_000_000));
            let max_age_secs = match rng.below(3) {
                0 => rng.below(4),
                1 => rng.below(1 << 32) * SECONDS_PER_DAY,
                _ => rng.below(1 << 41),
            };
            let reservation = reservation_last_used_at(last_used_at);
            let age = CleanupOperations::elapsed_since_last_use(&reservation, evaluated_at);
            assert_eq!(
                kernel::cleanup_reason(false, false, Some(max_age_secs), age)
                    == CleanupReason::Expire,
                old_is_expired_at(
                    &reservation,
                    evaluated_at,
                    Duration::from_secs(max_age_secs)
                )
            );
        }
    }

    #[test]
    fn test_glue_stored_unix_secs_is_exact_for_storable_times() {
        let max = crate::verified::storage::MAX_STORABLE_UNIX_SECS;
        for secs in [0, 1, 86_400, 1_700_000_000, max - 1, max] {
            assert_eq!(
                CleanupOperations::stored_unix_secs(at_unix_secs(secs)),
                secs
            );
        }
        // Times the database never stores map to the epoch rather than
        // panicking.
        assert_eq!(
            CleanupOperations::stored_unix_secs(SystemTime::UNIX_EPOCH - Duration::from_secs(5)),
            0
        );
    }

    #[test]
    fn test_glue_selection_days_to_seconds() {
        assert_eq!(kernel::days_to_secs(0), 0);
        assert_eq!(kernel::days_to_secs(1), SECONDS_PER_DAY);
        assert_eq!(kernel::days_to_secs(30), 30 * SECONDS_PER_DAY);
        assert_eq!(
            kernel::days_to_secs(u32::MAX),
            u64::from(u32::MAX) * SECONDS_PER_DAY
        );
        assert_eq!(
            kernel::command_selection(true, None),
            Selection {
                prune: true,
                max_age_secs: None
            }
        );
        assert_eq!(
            kernel::command_selection(false, Some(7)).max_age_secs,
            Some(7 * SECONDS_PER_DAY)
        );
        // Automatic cleanup: an unset threshold is the 30-day default, and a
        // disabled expire phase has no threshold whatever is configured.
        for days in [None, Some(30)] {
            assert_eq!(
                kernel::automatic_selection(true, true, days).max_age_secs,
                Some(30 * SECONDS_PER_DAY)
            );
        }
        assert_eq!(
            kernel::automatic_selection(true, true, Some(2)).max_age_secs,
            Some(2 * SECONDS_PER_DAY)
        );
        assert_eq!(
            kernel::automatic_selection(false, false, Some(2)),
            Selection {
                prune: false,
                max_age_secs: None
            }
        );
    }

    #[test]
    fn test_automatic_cleanup_defaults_an_unset_threshold_to_thirty_days() {
        // Regression: `cleanup: Some(CleanupConfig { expire_after_days: None })`
        // reported automatic expiration as enabled but expired nothing.
        let existing = std::env::current_dir().unwrap();
        let now = SystemTime::now();
        for config in [
            CleanupConfig {
                expire_after_days: None,
            },
            CleanupConfig {
                expire_after_days: Some(30),
            },
        ] {
            let mut db = create_test_database();
            for (tag, port, age_days) in [("old", 5000, 31), ("young", 5001, 29)] {
                let key = ReservationKey::new(existing.clone(), Some(tag.to_string())).unwrap();
                let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                    .last_used_at(now - Duration::from_secs(age_days * SECONDS_PER_DAY))
                    .build()
                    .unwrap();
                db.create_reservation(&reservation).unwrap();
            }

            // A disabled expire phase never expires, whatever is configured.
            let disabled = CleanupOperations::automatic_cleanup_in_connection(
                db.connection(),
                &config,
                false,
                false,
                || {},
            )
            .unwrap();
            assert_eq!(disabled.total_removed, 0);

            let result = CleanupOperations::automatic_cleanup_in_connection(
                db.connection(),
                &config,
                false,
                true,
                || {},
            )
            .unwrap();
            assert_eq!(result.expired_count, 1, "{config:?}");
            assert_eq!(
                result.expired_reservations[0].key().tag.as_deref(),
                Some("old")
            );
            assert_eq!(result.preserved_count, 1);
        }
    }

    /// The previous ordering: a stable sort of the planned removals, prune
    /// before expire, expire by last use.
    fn old_removal_order(candidates: &[Candidate]) -> Vec<usize> {
        let mut planned: Vec<usize> = (0..candidates.len())
            .filter(|&index| candidates[index].reason != CleanupReason::Keep)
            .collect();
        planned.sort_by(|&left, &right| {
            match (candidates[left].reason, candidates[right].reason) {
                (CleanupReason::Expire, CleanupReason::Expire) => candidates[left]
                    .last_used_secs
                    .cmp(&candidates[right].last_used_secs),
                (CleanupReason::Prune, CleanupReason::Expire) => std::cmp::Ordering::Less,
                (CleanupReason::Expire, CleanupReason::Prune) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        });
        planned
    }

    fn random_candidates(rng: &mut Lcg, length: usize) -> Vec<Candidate> {
        let key_range = 1 + rng.below(6);
        (0..length)
            .map(|_| Candidate {
                reason: match rng.below(3) {
                    0 => CleanupReason::Keep,
                    1 => CleanupReason::Prune,
                    _ => CleanupReason::Expire,
                },
                last_used_secs: match rng.below(4) {
                    0 => u64::MAX - rng.below(key_range),
                    _ => rng.below(key_range),
                },
            })
            .collect()
    }

    #[test]
    fn test_glue_removal_plan_matches_old_stable_sort() {
        let mut rng = Lcg(11);
        for length in (0..40).chain([200, 1_000]) {
            for _ in 0..20 {
                let candidates = random_candidates(&mut rng, length);
                let plan = kernel::plan_removals(&candidates);
                assert_eq!(plan.removals, old_removal_order(&candidates));
                let keep: Vec<usize> = (0..candidates.len())
                    .filter(|&index| candidates[index].reason == CleanupReason::Keep)
                    .collect();
                assert_eq!(plan.keep, keep);
                assert_eq!(plan.keep.len() + plan.removals.len(), candidates.len());

                let reasons: Vec<CleanupReason> = plan
                    .removals
                    .iter()
                    .map(|&index| candidates[index].reason)
                    .collect();
                let (pruned, expired) = kernel::split_by_reason(&reasons);
                let positions_of = |reason| -> Vec<usize> {
                    (0..reasons.len())
                        .filter(|&index| reasons[index] == reason)
                        .collect()
                };
                assert_eq!(pruned, positions_of(CleanupReason::Prune));
                assert_eq!(expired, positions_of(CleanupReason::Expire));
                assert_eq!(pruned.len() + expired.len(), reasons.len());
            }
        }
    }

    #[test]
    fn test_glue_removal_plan_handles_a_full_port_range() {
        // As many candidates as there are ports, all expiring, newest first:
        // the merge sort reverses them without quadratic work.
        let candidates: Vec<Candidate> = (0..65_535_u64)
            .map(|index| Candidate {
                reason: CleanupReason::Expire,
                last_used_secs: 65_535 - index,
            })
            .collect();
        let plan = kernel::plan_removals(&candidates);
        assert_eq!(plan.keep, Vec::<usize>::new());
        assert_eq!(plan.removals, (0..65_535).rev().collect::<Vec<usize>>());
    }

    #[test]
    fn test_glue_representatives_match_first_seen_paths() {
        let mut rng = Lcg(13);
        for length in 0..60 {
            let distinct = 1 + rng.below(8);
            let paths: Vec<PathBuf> = (0..length)
                .map(|_| PathBuf::from(format!("/probe-once/{}", rng.below(distinct))))
                .collect();
            let mut ids_by_path: BTreeMap<&Path, usize> = BTreeMap::new();
            let ids: Vec<usize> = paths
                .iter()
                .map(|path| {
                    let next = ids_by_path.len();
                    *ids_by_path.entry(path.as_path()).or_insert(next)
                })
                .collect();
            let representatives = kernel::representatives(&ids);
            for (index, &first) in representatives.iter().enumerate() {
                let expected = paths.iter().position(|path| *path == paths[index]).unwrap();
                assert_eq!(first, expected);
            }
            assert_eq!(
                representatives
                    .iter()
                    .enumerate()
                    .filter(|&(index, &first)| index == first)
                    .count(),
                ids_by_path.len(),
                "one probe per distinct path"
            );
        }
        // Total for ids the glue never produces.
        assert_eq!(
            kernel::representatives(&[7, 3, 7, 3, 9]),
            vec![0, 1, 0, 1, 4]
        );
        assert_eq!(
            kernel::representatives(&[usize::MAX, usize::MAX]),
            vec![0, 0]
        );
        assert_eq!(kernel::representatives(&[]), Vec::<usize>::new());
    }

    #[test]
    fn test_glue_io_error_kind_table() {
        for kind in [
            io::ErrorKind::Interrupted,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::TimedOut,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::NotConnected,
        ] {
            assert_eq!(io_kind_class(kind), IoKindClass::Transient, "{kind:?}");
        }
        assert_eq!(
            io_kind_class(io::ErrorKind::PermissionDenied),
            IoKindClass::PermissionDenied
        );
        assert_eq!(
            io_kind_class(io::ErrorKind::Unsupported),
            IoKindClass::Unsupported
        );
        for kind in [
            io::ErrorKind::Other,
            io::ErrorKind::NotFound,
            io::ErrorKind::AlreadyExists,
            io::ErrorKind::InvalidData,
        ] {
            assert_eq!(io_kind_class(kind), IoKindClass::Other, "{kind:?}");
        }
    }

    #[test]
    fn test_glue_probe_classification_precedence() {
        let classify = |result: io::Result<ProbedPath>| {
            let result = std::cell::RefCell::new(Some(result));
            CleanupOperations::classify_path(Path::new("/glue/probe"), &|_| {
                result.borrow_mut().take().unwrap()
            })
        };
        assert_eq!(
            classify(Ok(ProbedPath::Directory)),
            PrunePathStatus::ExistingDirectory
        );
        assert_eq!(
            classify(Ok(ProbedPath::NonDirectory)),
            PrunePathStatus::NotDirectory
        );
        assert_eq!(
            classify(Err(io::Error::from(io::ErrorKind::NotFound))),
            PrunePathStatus::Missing
        );
        for kind in [io::ErrorKind::InvalidInput, io::ErrorKind::InvalidFilename] {
            assert_eq!(
                classify(Err(io::Error::from(kind))),
                PrunePathStatus::NotDirectory
            );
        }
        assert_eq!(
            classify(Err(simulated_not_directory_error())),
            PrunePathStatus::NotDirectory
        );
        let PrunePathStatus::Uninspectable(error) = classify(Err(simulated_symlink_loop_error()))
        else {
            panic!("a symlink loop must be preserved");
        };
        assert_eq!(error.kind, PrunePathErrorKind::SymlinkLoop);
        assert_eq!(
            error.raw_os_error,
            simulated_symlink_loop_error().raw_os_error()
        );
        let PrunePathStatus::Uninspectable(error) = classify(Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "denied",
        ))) else {
            panic!("a denied probe must be preserved");
        };
        assert_eq!(error.kind, PrunePathErrorKind::PermissionDenied);
        assert_eq!(error.message, "denied");
        assert_eq!(error.raw_os_error, None);
    }

    #[test]
    fn test_glue_status_and_error_kind_round_trip_through_the_kernel() {
        for kind in [
            PrunePathErrorKind::PermissionDenied,
            PrunePathErrorKind::SymlinkLoop,
            PrunePathErrorKind::Transient,
            PrunePathErrorKind::Unsupported,
            PrunePathErrorKind::Other,
        ] {
            assert_eq!(PrunePathErrorKind::from_kernel(kind.kernel()), kind);
            let status = PrunePathStatus::Uninspectable(PrunePathError {
                kind,
                message: String::new(),
                raw_os_error: None,
            });
            assert!(!status.is_prunable());
        }
        assert!(PrunePathStatus::Missing.is_prunable());
        assert!(PrunePathStatus::NotDirectory.is_prunable());
        assert!(!PrunePathStatus::ExistingDirectory.is_prunable());
    }

    #[test]
    fn test_glue_settlement_places_each_fate_in_one_list() {
        use crate::verified::cleanup::Fate;
        // Removals: a dry-run selection, a row observed gone, a deleted row,
        // and a row gone after a delete that matched nothing.
        for fate in [
            Fate::Selected,
            Fate::Vanished,
            Fate::Deleted,
            Fate::VanishedAtDelete,
        ] {
            assert_eq!(kernel::settle::<u16>(fate), Settlement::Removed);
        }
        // Preserved with the current row, or as planned.
        assert_eq!(
            kernel::settle(Fate::Changed(7_u16)),
            Settlement::PreservedAs(7)
        );
        assert_eq!(
            kernel::settle(Fate::Replaced(9_u16)),
            Settlement::PreservedAs(9)
        );
        assert_eq!(
            kernel::settle::<u16>(Fate::Ineligible),
            Settlement::PreservedAsPlanned
        );
    }

    #[test]
    fn test_planned_candidates_are_always_still_eligible() {
        // The eligibility re-check uses the planning evidence, so every
        // candidate the plan selects is still eligible (verified
        // `lemma_unchanged_candidate_still_eligible`); exercised over the
        // reason table's corners, including the exact expiry boundary and a
        // last use in the future.
        let ages = [
            None,
            Some(Elapsed { secs: 0, nanos: 0 }),
            Some(Elapsed {
                secs: 86_400,
                nanos: 0,
            }),
            Some(Elapsed {
                secs: 86_400,
                nanos: 1,
            }),
            Some(Elapsed {
                secs: u64::MAX,
                nanos: 999_999_999,
            }),
        ];
        for prune in [false, true] {
            for prunable in [false, true] {
                for max_age in [None, Some(0), Some(86_400), Some(u64::MAX)] {
                    for age in ages {
                        let reason = kernel::cleanup_reason(prune, prunable, max_age, age);
                        if reason != CleanupReason::Keep {
                            assert!(kernel::still_eligible(reason, prunable, max_age, age));
                        }
                    }
                }
            }
        }
    }
}
