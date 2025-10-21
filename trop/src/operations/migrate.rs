//! Migration operation for moving reservations between paths.
//!
//! This module implements the migration planning logic, which allows
//! moving reservations from one path to another while preserving all
//! metadata (ports, project, task, timestamps, tags).

use std::path::{Path, PathBuf};

use crate::database::Database;
use crate::error::{Error, Result};
use crate::path::normalize::normalize;
use crate::{Reservation, ReservationKey};

use super::plan::{OperationPlan, PlanAction};

/// Options for migration operation.
#[derive(Debug, Clone)]
pub struct MigrateOptions {
    /// Source path to migrate from.
    pub from_path: PathBuf,
    /// Destination path to migrate to.
    pub to_path: PathBuf,
    /// Migrate all sub-paths recursively.
    pub recursive: bool,
    /// Overwrite existing reservations at destination.
    pub force: bool,
    /// Preview changes without applying them.
    pub dry_run: bool,
}

impl MigrateOptions {
    /// Creates a new `MigrateOptions` with the given paths.
    ///
    /// All flags are set to defaults:
    /// - recursive: false
    /// - force: false
    /// - `dry_run`: false
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::MigrateOptions;
    /// use std::path::PathBuf;
    ///
    /// let options = MigrateOptions::new(
    ///     PathBuf::from("/old/path"),
    ///     PathBuf::from("/new/path")
    /// );
    /// assert!(!options.recursive);
    /// assert!(!options.force);
    /// ```
    #[must_use]
    pub fn new(from_path: PathBuf, to_path: PathBuf) -> Self {
        Self {
            from_path,
            to_path,
            recursive: false,
            force: false,
            dry_run: false,
        }
    }

    /// Sets the recursive flag.
    #[must_use]
    pub const fn with_recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// Sets the force flag.
    #[must_use]
    pub const fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Sets the `dry_run` flag.
    #[must_use]
    pub const fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }
}

/// Migration plan describing what will be moved.
#[derive(Debug)]
pub struct MigratePlan {
    /// The migration options.
    pub options: MigrateOptions,
    /// Items to migrate (`from_key` -> `to_key` with reservation data).
    pub migrations: Vec<MigrationItem>,
    /// Reservation keys that would conflict at destination.
    pub conflicts: Vec<ReservationKey>,
}

/// Single item to migrate.
#[derive(Debug)]
pub struct MigrationItem {
    /// The original reservation key.
    pub from_key: ReservationKey,
    /// The new reservation key.
    pub to_key: ReservationKey,
    /// The reservation data to preserve.
    pub reservation: Reservation,
}

/// Result of migration execution.
#[derive(Debug)]
pub struct MigrateResult {
    /// Number of reservations migrated.
    pub migrated_count: usize,
    /// Number of conflicts resolved (if force was used).
    pub conflicts_resolved: usize,
    /// Source path.
    pub from_path: PathBuf,
    /// Destination path.
    pub to_path: PathBuf,
}

impl MigratePlan {
    /// Creates a new empty migration plan.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::{MigratePlan, MigrateOptions};
    /// use std::path::PathBuf;
    ///
    /// let options = MigrateOptions::new(
    ///     PathBuf::from("/old"),
    ///     PathBuf::from("/new")
    /// );
    /// let plan = MigratePlan::new(options);
    /// ```
    #[must_use]
    pub fn new(options: MigrateOptions) -> Self {
        Self {
            options,
            migrations: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    /// Builds the migration plan by analyzing what needs to be moved.
    ///
    /// This method:
    /// 1. Normalizes both paths
    /// 2. Finds all reservations at `from_path` (exact or recursive)
    /// 3. Calculates new paths by replacing the `from_path` prefix with `to_path`
    /// 4. Checks for conflicts at destination paths
    /// 5. Returns error if conflicts exist without `--force`
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Paths cannot be normalized
    /// - No reservations found at `from_path` (non-recursive mode only)
    /// - Conflicts exist at destination without `--force`
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{MigratePlan, MigrateOptions};
    /// use trop::{Database, DatabaseConfig};
    /// use std::path::PathBuf;
    ///
    /// let db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let options = MigrateOptions::new(
    ///     PathBuf::from("/old"),
    ///     PathBuf::from("/new")
    /// );
    /// let mut plan = MigratePlan::new(options);
    /// plan.build(&db).unwrap();
    /// ```
    pub fn build(&mut self, db: &Database) -> Result<()> {
        // Step 1: Normalize both paths
        let from_normalized = normalize(&self.options.from_path)?;
        let to_normalized = normalize(&self.options.to_path)?;

        log::debug!(
            "Normalized paths: from={} to={}",
            from_normalized.display(),
            to_normalized.display()
        );

        // Step 2: Find all reservations at from_path
        let reservations = if self.options.recursive {
            // Get all descendants (including exact match)
            Database::get_reservations_by_path_prefix(db.connection(), &from_normalized)?
        } else {
            // Get exact matches only
            Database::list_all_reservations(db.connection())?
                .into_iter()
                .filter(|r| normalize(&r.key().path).ok() == Some(from_normalized.clone()))
                .collect()
        };

        log::debug!(
            "Found {} reservation(s) at source path (recursive={})",
            reservations.len(),
            self.options.recursive
        );

        // Error if non-recursive and no exact match found
        if !self.options.recursive && reservations.is_empty() {
            return Err(Error::NotFound {
                resource: format!("reservation at {}", from_normalized.display()),
            });
        }

        // Step 3: For each reservation, calculate new path and check for conflicts
        for reservation in reservations {
            let from_key = reservation.key();
            let from_path_normalized = normalize(&from_key.path)?;

            // Calculate new path by replacing from_path prefix with to_path
            let new_path_raw =
                calculate_new_path(&from_path_normalized, &from_normalized, &to_normalized)?;

            // Normalize the new path to ensure consistent format (no trailing slashes)
            let new_path = normalize(&new_path_raw)?;

            // Create new reservation key with the new path but same tag
            let to_key = ReservationKey::new(new_path, from_key.tag.clone())?;

            // Check if reservation already exists at destination
            let has_conflict = Database::get_reservation(db.connection(), &to_key)?.is_some();
            if has_conflict {
                // Conflict detected - track it
                self.conflicts.push(to_key.clone());
                log::debug!("Conflict detected: {to_key} already exists");
            }

            // Always add to migration list - conflicts are resolved by force flag later
            self.migrations.push(MigrationItem {
                from_key: from_key.clone(),
                to_key,
                reservation: reservation.clone(),
            });
        }

        // Step 4: If conflicts exist and not force, return error
        if !self.conflicts.is_empty() && !self.options.force {
            return Err(Error::ReservationConflict {
                details: format!(
                    "Found {} conflicting reservation(s) at destination. Use --force to overwrite.",
                    self.conflicts.len()
                ),
            });
        }

        Ok(())
    }

    /// Returns the number of reservations that will be migrated.
    #[must_use]
    pub fn migration_count(&self) -> usize {
        self.migrations.len()
    }

    /// Returns the number of conflicts detected.
    #[must_use]
    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }
}

/// Calculates the new path for a reservation by replacing the `from_path` prefix with `to_path`.
///
/// # Arguments
///
/// * `original_path` - The original normalized path of the reservation
/// * `from_prefix` - The normalized source prefix to replace
/// * `to_prefix` - The normalized destination prefix
///
/// # Returns
///
/// The new path with the prefix replaced
///
/// # Errors
///
/// Returns an error if the path transformation fails
fn calculate_new_path(
    original_path: &Path,
    from_prefix: &Path,
    to_prefix: &Path,
) -> Result<PathBuf> {
    // Get the relative path from from_prefix to original_path
    let relative = original_path
        .strip_prefix(from_prefix)
        .map_err(|_| Error::InvalidPath {
            path: original_path.to_path_buf(),
            reason: format!(
                "Path {} does not start with prefix {}",
                original_path.display(),
                from_prefix.display()
            ),
        })?;

    // Join to_prefix with the relative path
    Ok(to_prefix.join(relative))
}

/// Converts a migration plan to an operation plan.
///
/// This creates the plan actions that will be executed, including
/// deleting old reservations and creating new ones at the destination.
///
/// # Arguments
///
/// * `migrate_plan` - The migration plan to convert
///
/// # Returns
///
/// An operation plan with all the necessary actions
///
/// # Panics
///
/// Panics if building a reservation from valid existing data fails,
/// which should never happen in practice.
pub fn to_operation_plan(migrate_plan: &MigratePlan) -> OperationPlan {
    let mut plan = OperationPlan::new(format!(
        "Migrate reservations from {} to {}",
        migrate_plan.options.from_path.display(),
        migrate_plan.options.to_path.display()
    ));

    // Add warnings if there are no migrations
    if migrate_plan.migrations.is_empty() && migrate_plan.conflicts.is_empty() {
        plan = plan.add_warning("No reservations to migrate".to_string());
        return plan;
    }

    // If force mode, delete conflicts first
    if migrate_plan.options.force {
        for conflict_key in &migrate_plan.conflicts {
            plan = plan.add_action(PlanAction::DeleteReservation(conflict_key.clone()));
        }
    }

    // For each migration: delete old, create new
    for item in &migrate_plan.migrations {
        // Delete old reservation
        plan = plan.add_action(PlanAction::DeleteReservation(item.from_key.clone()));

        // Create new reservation with preserved metadata
        let new_reservation = Reservation::builder(item.to_key.clone(), item.reservation.port())
            .project(item.reservation.project().map(String::from))
            .task(item.reservation.task().map(String::from))
            .created_at(item.reservation.created_at())
            .last_used_at(item.reservation.last_used_at())
            .build()
            .expect("Building reservation from valid data should succeed");

        plan = plan.add_action(PlanAction::CreateReservation(new_reservation));
    }

    plan
}

/// Executes a migration plan.
///
/// This is a convenience function that converts the migration plan to an operation plan
/// and executes it using the plan executor.
///
/// # Arguments
///
/// * `plan` - The migration plan to execute
/// * `db` - Mutable reference to the database
///
/// # Returns
///
/// A `MigrateResult` with statistics about the migration
///
/// # Errors
///
/// Returns an error if:
/// - The migration plan contains conflicts without force
/// - Database operations fail
///
/// # Examples
///
/// ```no_run
/// use trop::operations::{MigratePlan, MigrateOptions, execute_migrate};
/// use trop::{Database, DatabaseConfig};
/// use std::path::PathBuf;
///
/// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
/// let options = MigrateOptions::new(
///     PathBuf::from("/old"),
///     PathBuf::from("/new")
/// );
/// let mut plan = MigratePlan::new(options);
/// plan.build(&db).unwrap();
/// let result = execute_migrate(&plan, &mut db).unwrap();
/// println!("Migrated {} reservations", result.migrated_count);
/// ```
pub fn execute_migrate(plan: &MigratePlan, db: &mut Database) -> Result<MigrateResult> {
    use super::executor::PlanExecutor;

    // Convert to operation plan
    let op_plan = to_operation_plan(plan);

    // Begin transaction for atomic migration
    let tx = db.begin_transaction()?;

    // Execute the plan inside transaction
    let mut executor = PlanExecutor::new(&tx);
    executor.execute(&op_plan)?;

    // Commit transaction
    tx.commit()?;

    // Build result
    Ok(MigrateResult {
        migrated_count: plan.migrations.len(),
        conflicts_resolved: if plan.options.force {
            plan.conflicts.len()
        } else {
            0
        },
        from_path: plan.options.from_path.clone(),
        to_path: plan.options.to_path.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    #[cfg(unix)]
    use crate::{Port, Reservation};

    #[test]
    fn test_migrate_options_new() {
        let options = MigrateOptions::new(PathBuf::from("/old"), PathBuf::from("/new"));
        assert!(!options.recursive);
        assert!(!options.force);
        assert!(!options.dry_run);
    }

    #[test]
    fn test_migrate_options_builder() {
        let options = MigrateOptions::new(PathBuf::from("/old"), PathBuf::from("/new"))
            .with_recursive(true)
            .with_force(true)
            .with_dry_run(true);

        assert!(options.recursive);
        assert!(options.force);
        assert!(options.dry_run);
    }

    #[test]
    fn test_calculate_new_path_exact_match() {
        let original = Path::new("/old/path");
        let from = Path::new("/old/path");
        let to = Path::new("/new/path");

        let result = calculate_new_path(original, from, to).unwrap();
        assert_eq!(result, PathBuf::from("/new/path"));
    }

    #[test]
    fn test_calculate_new_path_with_suffix() {
        let original = Path::new("/old/path/subdir/file");
        let from = Path::new("/old/path");
        let to = Path::new("/new/path");

        let result = calculate_new_path(original, from, to).unwrap();
        assert_eq!(result, PathBuf::from("/new/path/subdir/file"));
    }

    #[test]
    fn test_calculate_new_path_invalid_prefix() {
        let original = Path::new("/different/path");
        let from = Path::new("/old/path");
        let to = Path::new("/new/path");

        let result = calculate_new_path(original, from, to);
        assert!(result.is_err());
    }

    #[test]
    fn test_migrate_plan_nonrecursive_not_found() {
        let db = create_test_database();
        let options = MigrateOptions::new(PathBuf::from("/nonexistent"), PathBuf::from("/new"));
        let mut plan = MigratePlan::new(options);

        let result = plan.build(&db);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::NotFound { .. }));
    }

    #[test]
    fn test_migrate_plan_recursive_empty_ok() {
        let db = create_test_database();
        let options = MigrateOptions::new(PathBuf::from("/nonexistent"), PathBuf::from("/new"))
            .with_recursive(true);
        let mut plan = MigratePlan::new(options);

        let result = plan.build(&db);
        assert!(result.is_ok());
        assert_eq!(plan.migration_count(), 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_migrate_plan_single_reservation() {
        let mut db = create_test_database();

        // Create a reservation
        let key = ReservationKey::new(PathBuf::from("/old/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("test-project".to_string()))
            .task(Some("test-task".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Build migration plan
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"));
        let mut plan = MigratePlan::new(options);
        plan.build(&db).unwrap();

        // Should have one migration
        assert_eq!(plan.migration_count(), 1);
        assert_eq!(plan.conflict_count(), 0);

        let item = &plan.migrations[0];
        assert_eq!(item.from_key.path, PathBuf::from("/old/path"));
        assert_eq!(item.to_key.path, PathBuf::from("/new/path"));
        assert_eq!(item.reservation.port(), port);
        assert_eq!(item.reservation.project(), Some("test-project"));
        assert_eq!(item.reservation.task(), Some("test-task"));
    }

    #[test]
    #[cfg(unix)]
    fn test_migrate_plan_with_conflict() {
        let mut db = create_test_database();

        // Create reservations at both source and destination
        let from_key = ReservationKey::new(PathBuf::from("/old/path"), None).unwrap();
        let to_key = ReservationKey::new(PathBuf::from("/new/path"), None).unwrap();
        let port1 = Port::try_from(5000).unwrap();
        let port2 = Port::try_from(5001).unwrap();

        let r1 = Reservation::builder(from_key, port1).build().unwrap();
        let r2 = Reservation::builder(to_key, port2).build().unwrap();

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();

        // Build migration plan without force - should error
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"));
        let mut plan = MigratePlan::new(options);
        let result = plan.build(&db);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::ReservationConflict { .. }
        ));
    }

    #[test]
    #[cfg(unix)]
    fn test_migrate_plan_with_conflict_force() {
        let mut db = create_test_database();

        // Create reservations at both source and destination
        let from_key = ReservationKey::new(PathBuf::from("/old/path"), None).unwrap();
        let to_key = ReservationKey::new(PathBuf::from("/new/path"), None).unwrap();
        let port1 = Port::try_from(5000).unwrap();
        let port2 = Port::try_from(5001).unwrap();

        let r1 = Reservation::builder(from_key, port1).build().unwrap();
        let r2 = Reservation::builder(to_key, port2).build().unwrap();

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();

        // Build migration plan with force - should succeed by overwriting the conflict
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"))
            .with_force(true);
        let mut plan = MigratePlan::new(options);
        plan.build(&db).unwrap();

        // Should have both migration and conflict (force allows overwrite)
        assert_eq!(plan.migration_count(), 1); // Migration happens, overwriting conflict
        assert_eq!(plan.conflict_count(), 1); // Conflict detected but will be resolved
    }

    #[test]
    #[cfg(unix)]
    fn test_migrate_plan_recursive() {
        let mut db = create_test_database();

        // Create reservations at different levels
        let keys = [
            ReservationKey::new(PathBuf::from("/old/path"), None).unwrap(),
            ReservationKey::new(PathBuf::from("/old/path/sub1"), None).unwrap(),
            ReservationKey::new(PathBuf::from("/old/path/sub2"), None).unwrap(),
        ];

        for (i, key) in keys.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let port = Port::try_from(5000 + i as u16).unwrap();
            let reservation = Reservation::builder(key.clone(), port).build().unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        // Build recursive migration plan
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"))
            .with_recursive(true);
        let mut plan = MigratePlan::new(options);
        plan.build(&db).unwrap();

        // Should migrate all three
        assert_eq!(plan.migration_count(), 3);
        assert_eq!(plan.conflict_count(), 0);

        // Verify paths are transformed correctly
        let new_paths: Vec<PathBuf> = plan
            .migrations
            .iter()
            .map(|m| m.to_key.path.clone())
            .collect();
        assert!(new_paths.contains(&PathBuf::from("/new/path")));
        assert!(new_paths.contains(&PathBuf::from("/new/path/sub1")));
        assert!(new_paths.contains(&PathBuf::from("/new/path/sub2")));
    }
}
