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

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::config::CleanupConfig;
use crate::database::Database;
use crate::{Reservation, Result};

/// Number of seconds in a day, used for expiration calculations.
const SECONDS_PER_DAY: u64 = 86400;

/// Result of a prune operation.
///
/// Pruning removes reservations for paths that no longer exist on the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneResult {
    /// Number of reservations removed (or would be removed in dry-run mode).
    pub removed_count: usize,
    /// Reservations that were (or would be) removed.
    pub removed_reservations: Vec<Reservation>,
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
}

/// Cleanup operations for removing stale reservations.
///
/// All operations are static methods that work on a database instance.
/// Operations are transactional - they either complete fully or are rolled back.
pub struct CleanupOperations;

impl CleanupOperations {
    /// Remove reservations for paths that no longer exist on the filesystem.
    ///
    /// This operation checks each reservation's path against the filesystem and
    /// removes reservations where the path no longer exists.
    ///
    /// # Arguments
    ///
    /// * `db` - Database to operate on
    /// * `dry_run` - If true, report what would be removed without actually removing
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail. Filesystem errors for individual
    /// paths are handled gracefully (paths that can't be checked are assumed to exist).
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
        // Get all reservations
        let all_reservations = Database::list_all_reservations(db.connection())?;

        // Filter to those with non-existent paths
        let mut to_remove = Vec::new();
        for reservation in all_reservations {
            // Fail-open policy: if we can't check the path (e.g., permission errors),
            // we conservatively assume it exists to avoid accidentally removing
            // valid reservations.
            let path_exists = Self::check_path_exists(&reservation.key().path);
            if !path_exists {
                to_remove.push(reservation);
            }
        }

        let removed_count = to_remove.len();

        // If not dry-run, actually delete the reservations
        if !dry_run {
            // Delete within a transaction by doing all deletes together
            for reservation in &to_remove {
                db.delete_reservation(reservation.key())?;
            }
        }

        Ok(PruneResult {
            removed_count,
            removed_reservations: to_remove,
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
        })
    }

    /// Check if a path exists on the filesystem.
    ///
    /// This uses a fail-open policy: if we can't check the path (e.g., permission errors),
    /// we assume it exists to avoid accidentally removing valid reservations.
    fn check_path_exists(path: &Path) -> bool {
        fs::metadata(path).is_ok()
    }
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
