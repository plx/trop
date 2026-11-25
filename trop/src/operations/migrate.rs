//! Migration operation for moving reservations between paths.
//!
//! This module implements the migration planning logic, which allows
//! moving reservations from one path to another while preserving all
//! metadata (ports, project, task, timestamps, tags).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::database::Database;
use crate::error::{Error, Result};
use crate::path::normalize::normalize;
use crate::path::relationship::path_tokens;
use crate::verified::migrate::{
    classify_conflicts, guard_moves, schedule_migration, MigAction, MigrationKeyError,
    MoveGuardError,
};
use crate::verified::path_relation::PathToken;
use crate::{Reservation, ReservationKey};

use super::plan::{OperationPlan, PlanAction};
use super::release::guard_cwd_tokens;

/// Options for migration operation.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // one field per independent CLI flag
pub struct MigrateOptions {
    /// Source path to migrate from.
    pub from_path: PathBuf,
    /// Destination path to migrate to.
    pub to_path: PathBuf,
    /// Migrate all sub-paths recursively.
    pub recursive: bool,
    /// Overwrite existing reservations at destination, and allow migrating
    /// reservations unrelated to the current directory.
    pub force: bool,
    /// Preview changes without applying them.
    pub dry_run: bool,
    /// Allow migrating reservations whose source or destination path is
    /// unrelated to the current working directory.
    pub allow_unrelated_path: bool,
}

impl MigrateOptions {
    /// Creates a new `MigrateOptions` with the given paths.
    ///
    /// All flags are set to defaults:
    /// - recursive: false
    /// - force: false
    /// - `dry_run`: false
    /// - `allow_unrelated_path`: false
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
            allow_unrelated_path: false,
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

    /// Sets the `allow_unrelated_path` flag.
    #[must_use]
    pub const fn with_allow_unrelated_path(mut self, allow: bool) -> Self {
        self.allow_unrelated_path = allow;
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
    /// Destination keys whose existing reservation the migration would
    /// overwrite (destinations that are themselves being moved away are not
    /// conflicts).
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
    /// 4. Checks every source and destination path against the current
    ///    directory (unless `force` or `allow_unrelated_path` is set)
    /// 5. Checks for conflicts: destinations whose existing reservation would
    ///    be overwritten (a destination that is itself being moved away is
    ///    not a conflict)
    /// 6. Returns error if conflicts exist without `--force`
    ///
    /// The plan is a snapshot: [`execute_migrate`] rebuilds it inside its
    /// transaction and refuses to run if the database changed meanwhile.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Paths cannot be normalized
    /// - No reservations found at `from_path` (non-recursive mode only)
    /// - A source or destination path is unrelated to the current directory
    ///   (without `force` or `allow_unrelated_path`)
    /// - Two source reservations would migrate to the same destination (a
    ///   defensive check: rebasing canonical stored paths is injective)
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
        self.build_with_connection(db.connection())
    }

    /// Builds the plan from `conn` (for example inside a transaction).
    fn build_with_connection(&mut self, conn: &Connection) -> Result<()> {
        self.migrations.clear();
        self.conflicts.clear();

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
            Database::get_reservations_by_path_prefix(conn, &from_normalized)?
        } else {
            // Get exact matches only
            Database::list_all_reservations(conn)?
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

        // Step 3: For each reservation, calculate the destination key
        for reservation in reservations {
            let from_key = reservation.key();
            let from_path_normalized = normalize(&from_key.path)?;

            // Calculate new path by replacing from_path prefix with to_path
            let new_path =
                calculate_new_path(&from_path_normalized, &from_normalized, &to_normalized)?;
            // The rebased path is already canonical (proved on Unix), so the
            // normalization the previous implementation repeated here is a
            // no-op.
            #[cfg(unix)]
            debug_assert!(crate::path::is_canonical_absolute(&new_path));

            // Create new reservation key with the new path but same tag
            let to_key = ReservationKey::new(new_path, from_key.tag.clone())?;

            self.migrations.push(MigrationItem {
                from_key: from_key.clone(),
                to_key,
                reservation: reservation.clone(),
            });
        }

        // Step 4: The unrelated-path guard covers every key the migration
        // deletes (sources) or writes (destinations): the kernel checks every
        // id of the same key lists it classifies and schedules, deciding the
        // lexical relation to the current directory itself and consulting the
        // database's single-path check as the physical fallback.
        let keys = KeyTable::intern(&self.migrations);
        let force = self.options.force;
        let allow_unrelated_path = self.options.allow_unrelated_path;
        let cwd = guard_cwd_tokens(force, allow_unrelated_path, !keys.keys.is_empty())?;
        let key_paths: Vec<Vec<PathToken>> =
            keys.keys.iter().map(|key| path_tokens(&key.path)).collect();
        guard_moves(
            force,
            allow_unrelated_path,
            &cwd,
            &key_paths,
            &keys.sources,
            &keys.dests,
            &|id| Database::validate_path_relationship(&keys.keys[id].path, false).err(),
        )
        .map_err(|error| match error {
            MoveGuardError::Refused(failure) => {
                let position = failure.index;
                let id = keys
                    .sources
                    .iter()
                    .chain(&keys.dests)
                    .nth(position)
                    .copied();
                if let Some(id) = id {
                    log::debug!("Unrelated-path guard refused {}", keys.keys[id]);
                }
                failure.error
            }
            MoveGuardError::KeyOutOfRange => inconsistent_plan(&MigrationKeyError::KeyOutOfRange),
        })?;

        // Step 5: Classify conflicts with the verified kernel.
        let mut dest_exists = Vec::with_capacity(self.migrations.len());
        for item in &self.migrations {
            dest_exists.push(Database::get_reservation(conn, &item.to_key)?.is_some());
        }
        let conflicts = classify_conflicts(keys.len(), &keys.sources, &keys.dests, &dest_exists)
            .map_err(|error| key_error(&self.migrations, error))?;
        for index in conflicts {
            let to_key = self.migrations[index].to_key.clone();
            log::debug!("Conflict detected: {to_key} already exists");
            self.conflicts.push(to_key);
        }

        // Step 6: If conflicts exist and not force, return error
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

    /// Whether `other` makes the same moves, with the same reservation data,
    /// and finds the same conflicts as this plan.
    fn same_selection(&self, other: &Self) -> bool {
        self.conflicts == other.conflicts
            && self.migrations.len() == other.migrations.len()
            && self.migrations.iter().zip(&other.migrations).all(|(a, b)| {
                a.from_key == b.from_key
                    && a.to_key == b.to_key
                    && same_reservation_data(&a.reservation, &b.reservation)
            })
    }
}

/// Whether two reservations carry the same data (key, port, project, task,
/// and timestamps): everything a migration moves.
fn same_reservation_data(a: &Reservation, b: &Reservation) -> bool {
    a.key() == b.key()
        && a.port() == b.port()
        && a.project() == b.project()
        && a.task() == b.task()
        && a.created_at() == b.created_at()
        && a.last_used_at() == b.last_used_at()
}

/// Reservation keys interned to dense ids by their database identity (the
/// stored path text and the stored tag text), so equal ids mean the same
/// stored row.
struct KeyTable {
    /// The source id of each item.
    sources: Vec<usize>,
    /// The destination id of each item.
    dests: Vec<usize>,
    /// The key of each id.
    keys: Vec<ReservationKey>,
}

impl KeyTable {
    fn intern(items: &[MigrationItem]) -> Self {
        let mut ids: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut keys = Vec::new();
        let mut id_of = |key: &ReservationKey| {
            let identity = (key.path_as_string(), key.tag.clone().unwrap_or_default());
            *ids.entry(identity).or_insert_with(|| {
                keys.push(key.clone());
                keys.len() - 1
            })
        };
        let sources = items.iter().map(|item| id_of(&item.from_key)).collect();
        let dests = items.iter().map(|item| id_of(&item.to_key)).collect();
        Self {
            sources,
            dests,
            keys,
        }
    }

    fn len(&self) -> usize {
        self.keys.len()
    }
}

/// Maps a key-list error from the kernel to a user-facing error.
fn key_error(items: &[MigrationItem], error: MigrationKeyError) -> Error {
    match error {
        MigrationKeyError::DuplicateDest { first, second } => Error::ReservationConflict {
            details: format!(
                "Reservations {} and {} would both migrate to {}",
                items[first].from_key, items[second].from_key, items[first].to_key
            ),
        },
        other => inconsistent_plan(&other),
    }
}

/// An internal inconsistency between the migration items and their key ids.
fn inconsistent_plan(error: &MigrationKeyError) -> Error {
    Error::Validation {
        field: "migration".to_string(),
        message: format!("inconsistent migration plan: {error:?}"),
    }
}

/// Calculates the new path for a reservation by replacing the `from_path` prefix with `to_path`.
///
/// On Unix the three paths are normalized (canonical) byte strings, and the
/// verified kernel `verified::migrate::rebase_canonical` computes the
/// result: `to` followed by the components of `original_path` below
/// `from_prefix`, itself canonical (so normalizing it again is a no-op).
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
/// Returns an error if `from_prefix` is not a component prefix of
/// `original_path` (or, on Unix, if a path is not in canonical form).
#[cfg(unix)]
fn calculate_new_path(
    original_path: &Path,
    from_prefix: &Path,
    to_prefix: &Path,
) -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    crate::verified::migrate::rebase_canonical(
        original_path.as_os_str().as_bytes(),
        from_prefix.as_os_str().as_bytes(),
        to_prefix.as_os_str().as_bytes(),
    )
    .map(|bytes| PathBuf::from(OsString::from_vec(bytes)))
    .ok_or_else(|| not_under_prefix(original_path, from_prefix))
}

/// Calculates the new path for a reservation by replacing the `from_path` prefix with `to_path`.
///
/// Outside Unix this uses `Path::strip_prefix` and `Path::join`, then
/// normalizes the result (joining an empty remainder adds a separator).
///
/// # Errors
///
/// Returns an error if `from_prefix` is not a component prefix of
/// `original_path`.
#[cfg(not(unix))]
fn calculate_new_path(
    original_path: &Path,
    from_prefix: &Path,
    to_prefix: &Path,
) -> Result<PathBuf> {
    let relative = original_path
        .strip_prefix(from_prefix)
        .map_err(|_| not_under_prefix(original_path, from_prefix))?;
    normalize(&to_prefix.join(relative))
}

fn not_under_prefix(original_path: &Path, from_prefix: &Path) -> Error {
    Error::InvalidPath {
        path: original_path.to_path_buf(),
        reason: format!(
            "Path {} does not start with prefix {}",
            original_path.display(),
            from_prefix.display()
        ),
    }
}

/// Converts a migration plan to an operation plan.
///
/// The actions come from the verified schedule
/// (`verified::migrate::schedule_migration`): every source reservation is
/// deleted, then every destination that is not itself a source (overwritten
/// conflicts, or absent keys), and only then is every destination created.
/// No create can be undone by a later delete, so every migrated reservation
/// survives even when the destination tree overlaps the source tree.
///
/// # Arguments
///
/// * `migrate_plan` - The migration plan to convert
///
/// # Returns
///
/// An operation plan with all the necessary actions
///
/// # Errors
///
/// Returns an error if two items share a source or a destination key (which
/// [`MigratePlan::build`] never produces), or if a preserved reservation
/// cannot be rebuilt at its destination.
pub fn to_operation_plan(migrate_plan: &MigratePlan) -> Result<OperationPlan> {
    let mut plan = OperationPlan::new(format!(
        "Migrate reservations from {} to {}",
        migrate_plan.options.from_path.display(),
        migrate_plan.options.to_path.display()
    ));

    // Add warnings if there are no migrations
    if migrate_plan.migrations.is_empty() && migrate_plan.conflicts.is_empty() {
        plan = plan.add_warning("No reservations to migrate".to_string());
        return Ok(plan);
    }

    let keys = KeyTable::intern(&migrate_plan.migrations);
    let actions = schedule_migration(keys.len(), &keys.sources, &keys.dests)
        .map_err(|error| key_error(&migrate_plan.migrations, error))?;

    for action in actions {
        match action {
            MigAction::DeleteKey(id) => {
                plan = plan.add_action(PlanAction::DeleteReservation(keys.keys[id].clone()));
            }
            MigAction::Create(index) => {
                // Create the new reservation with preserved metadata
                let item = &migrate_plan.migrations[index];
                let new_reservation =
                    Reservation::builder(item.to_key.clone(), item.reservation.port())
                        .project(item.reservation.project().map(String::from))
                        .task(item.reservation.task().map(String::from))
                        .created_at(item.reservation.created_at())
                        .last_used_at(item.reservation.last_used_at())
                        .build()?;
                plan = plan.add_action(PlanAction::CreateReservation(new_reservation));
            }
        }
    }

    Ok(plan)
}

/// Executes a migration plan.
///
/// `plan` must come from [`MigratePlan::build`]. The migration runs in one
/// `IMMEDIATE` transaction. Inside it, the plan is rebuilt from
/// `plan.options` against the locked database, and the migration is refused
/// (with no changes) unless the rebuilt plan makes the same moves, carrying
/// the same reservation data (port, project, task, timestamps), and finds
/// the same conflicts as `plan`. So the moves that run are the moves `plan`
/// describes, and they are a snapshot of the state they modify: a
/// destination reserved after planning is never overwritten without
/// `--force`, a source released after planning is never resurrected, and a
/// source changed after planning is never moved with data `plan` did not
/// show. A plan that was never built, or was edited by hand, is refused the
/// same way (or fails as building it would, for example with `NotFound`).
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
/// - Building the plan inside the transaction fails (for example, conflicts
///   without force, or a path unrelated to the current directory)
/// - The reservations changed since the plan was built
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
    let timeout = db.busy_timeout();
    execute_migrate_inner(plan, db)
        .map_err(|error| error.classify_sqlite_lock(timeout, "executing a path migration"))
}

fn execute_migrate_inner(plan: &MigratePlan, db: &mut Database) -> Result<MigrateResult> {
    use super::executor::PlanExecutor;

    // Begin transaction for atomic migration
    let tx = db.begin_transaction()?;

    // Rebuild the plan against the locked database: the executed schedule is
    // derived from this snapshot, which must match the plan the caller saw.
    let mut current = MigratePlan::new(plan.options.clone());
    current.build_with_connection(&tx)?;
    if !current.same_selection(plan) {
        return Err(Error::ReservationConflict {
            details: "Reservations changed while the migration was being planned; \
                      no changes were made. Re-run the migration."
                .to_string(),
        });
    }

    // Convert to operation plan
    let op_plan = to_operation_plan(&current)?;

    // Execute the plan inside transaction
    let mut executor = PlanExecutor::new(&tx);
    executor.execute(&op_plan)?;

    // Commit transaction
    tx.commit()?;

    // Build result
    Ok(MigrateResult {
        migrated_count: current.migrations.len(),
        conflicts_resolved: if current.options.force {
            current.conflicts.len()
        } else {
            0
        },
        from_path: current.options.from_path.clone(),
        to_path: current.options.to_path.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::Port;

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
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"))
            .with_allow_unrelated_path(true);
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
        let options = MigrateOptions::new(PathBuf::from("/old/path"), PathBuf::from("/new/path"))
            .with_allow_unrelated_path(true);
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
            .with_recursive(true)
            .with_allow_unrelated_path(true);
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

    // ------------------------------------------------------------------
    // Verified schedule, conflict classification, and snapshot glue
    // ------------------------------------------------------------------

    #[cfg(unix)]
    fn reserve(db: &mut Database, path: &str, port: u16) {
        let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    #[cfg(unix)]
    fn port_at(db: &Database, path: &str) -> Option<u16> {
        let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
        Database::get_reservation(db.connection(), &key)
            .unwrap()
            .map(|reservation| reservation.port().value())
    }

    #[cfg(unix)]
    fn migrate(db: &mut Database, options: MigrateOptions) -> Result<MigrateResult> {
        let mut plan = MigratePlan::new(options);
        plan.build(db)?;
        execute_migrate(&plan, db)
    }

    #[cfg(unix)]
    fn options(from: &str, to: &str) -> MigrateOptions {
        MigrateOptions::new(PathBuf::from(from), PathBuf::from(to)).with_allow_unrelated_path(true)
    }

    /// Regression: with `/w/a` and `/w/a/b` reserved, moving `/w/a` onto
    /// `/w/a/b` with `--force` used to delete the row it had just created
    /// at `/w/a/b`, leaving only `/w/a/b/b`.
    #[test]
    #[cfg(unix)]
    fn test_destination_inside_source_tree_keeps_every_reservation() {
        let mut db = create_test_database();
        reserve(&mut db, "/w/a", 5000);
        reserve(&mut db, "/w/a/b", 5001);

        let result = migrate(
            &mut db,
            options("/w/a", "/w/a/b")
                .with_recursive(true)
                .with_force(true),
        )
        .unwrap();

        assert_eq!(result.migrated_count, 2);
        // `/w/a/b` is itself moved away, so nothing was overwritten.
        assert_eq!(result.conflicts_resolved, 0);
        assert_eq!(port_at(&db, "/w/a"), None);
        assert_eq!(port_at(&db, "/w/a/b"), Some(5000));
        assert_eq!(port_at(&db, "/w/a/b/b"), Some(5001));
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            2
        );
    }

    /// Regression: a destination that is itself a migration source is not a
    /// conflict, so `migrate --from P --to P` is a no-op without `--force`.
    #[test]
    #[cfg(unix)]
    fn test_self_migration_is_a_noop_without_force() {
        let mut db = create_test_database();
        reserve(&mut db, "/w/p", 5000);

        let mut plan = MigratePlan::new(options("/w/p", "/w/p"));
        plan.build(&db).unwrap();
        assert_eq!(plan.conflict_count(), 0);
        let result = execute_migrate(&plan, &mut db).unwrap();

        assert_eq!(result.migrated_count, 1);
        assert_eq!(port_at(&db, "/w/p"), Some(5000));
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            1
        );
    }

    /// Regression: re-rooting a subtree onto its parent overwrites nothing
    /// (the only existing destination is itself moved away), so it succeeds
    /// without `--force`.
    #[test]
    #[cfg(unix)]
    fn test_migrate_to_ancestor_without_force() {
        let mut db = create_test_database();
        reserve(&mut db, "/w/a/b", 5000);
        reserve(&mut db, "/w/a/b/b", 5001);

        let result = migrate(&mut db, options("/w/a/b", "/w/a").with_recursive(true)).unwrap();

        assert_eq!(result.migrated_count, 2);
        assert_eq!(port_at(&db, "/w/a"), Some(5000));
        assert_eq!(port_at(&db, "/w/a/b"), Some(5001));
        assert_eq!(port_at(&db, "/w/a/b/b"), None);
    }

    /// A real overwrite is still a conflict (and `--force` resolves it).
    #[test]
    #[cfg(unix)]
    fn test_overlapping_migration_with_a_real_conflict() {
        let mut db = create_test_database();
        reserve(&mut db, "/w/a", 5000);
        reserve(&mut db, "/w/a/b", 5001);
        reserve(&mut db, "/w/a/b/b", 5002);
        reserve(&mut db, "/w/a/b/b/b", 5003);

        // Moving only `/w/a/b` (non-recursive) onto `/w/a/b/b` overwrites the
        // reservation there, which is not moved.
        let mut plan = MigratePlan::new(options("/w/a/b", "/w/a/b/b"));
        assert!(matches!(
            plan.build(&db),
            Err(Error::ReservationConflict { .. })
        ));

        let result = migrate(&mut db, options("/w/a/b", "/w/a/b/b").with_force(true)).unwrap();
        assert_eq!(result.conflicts_resolved, 1);
        assert_eq!(port_at(&db, "/w/a/b"), None);
        assert_eq!(port_at(&db, "/w/a/b/b"), Some(5001));
        assert_eq!(port_at(&db, "/w/a/b/b/b"), Some(5003));
        assert_eq!(port_at(&db, "/w/a"), Some(5000));
    }

    /// The operation plan deletes every source and non-source destination
    /// before it creates anything, and creates each item exactly once.
    #[test]
    #[cfg(unix)]
    fn test_operation_plan_deletes_before_creates() {
        let mut db = create_test_database();
        reserve(&mut db, "/w/a", 5000);
        reserve(&mut db, "/w/a/b", 5001);
        reserve(&mut db, "/w/x", 5002);

        let mut plan = MigratePlan::new(options("/w/a", "/w/a/b").with_recursive(true));
        plan.build(&db).unwrap();
        let op_plan = to_operation_plan(&plan).unwrap();

        let first_create = op_plan
            .actions
            .iter()
            .position(|action| matches!(action, PlanAction::CreateReservation(_)))
            .unwrap();
        assert!(op_plan.actions[..first_create]
            .iter()
            .all(|action| matches!(action, PlanAction::DeleteReservation(_))));
        let created: Vec<PathBuf> = op_plan.actions[first_create..]
            .iter()
            .map(|action| match action {
                PlanAction::CreateReservation(reservation) => reservation.key().path.clone(),
                other => panic!("unexpected action after the first create: {other:?}"),
            })
            .collect();
        assert_eq!(
            created,
            vec![PathBuf::from("/w/a/b"), PathBuf::from("/w/a/b/b")]
        );
        let deleted: Vec<PathBuf> = op_plan.actions[..first_create]
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key.path.clone(),
                other => panic!("unexpected action: {other:?}"),
            })
            .collect();
        // Sources, then the destination that is not a source.
        assert_eq!(
            deleted,
            vec![
                PathBuf::from("/w/a"),
                PathBuf::from("/w/a/b"),
                PathBuf::from("/w/a/b/b")
            ]
        );
    }

    /// Two items sharing a destination key cannot come from
    /// `MigratePlan::build` (stored paths are canonical, and rebasing
    /// canonical paths is injective: `lemma_rebase_canonical_injective`), but
    /// a hand-built plan can contain them. The kernel refuses to schedule it
    /// (`DuplicateDest`), and the error names both sources and the shared
    /// destination instead of silently keeping one reservation.
    #[test]
    fn test_plan_with_a_shared_destination_is_refused() {
        let item = |from: &str, port: u16| {
            let from_key = ReservationKey::new(PathBuf::from(from), None).unwrap();
            let to_key = ReservationKey::new(PathBuf::from("/w/b"), None).unwrap();
            let reservation = Reservation::builder(from_key.clone(), Port::try_from(port).unwrap())
                .build()
                .unwrap();
            MigrationItem {
                from_key,
                to_key,
                reservation,
            }
        };
        let mut plan = MigratePlan::new(options("/w", "/w/b"));
        plan.migrations = vec![item("/w/a", 5000), item("/w/c", 5001)];

        match to_operation_plan(&plan) {
            Err(Error::ReservationConflict { details }) => {
                assert_eq!(
                    details,
                    "Reservations /w/a and /w/c would both migrate to /w/b"
                );
            }
            other => panic!("expected a reservation conflict, got {other:?}"),
        }
    }

    /// A hand-built plan whose items repeat a source key is an internal
    /// inconsistency, reported without scheduling anything.
    #[test]
    fn test_plan_with_a_repeated_source_is_refused() {
        let key = ReservationKey::new(PathBuf::from("/w/a"), None).unwrap();
        let reservation = Reservation::builder(key.clone(), Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        let mut plan = MigratePlan::new(options("/w/a", "/w/b"));
        plan.migrations = ["/w/b", "/w/c"]
            .iter()
            .map(|to| MigrationItem {
                from_key: key.clone(),
                to_key: ReservationKey::new(PathBuf::from(to), None).unwrap(),
                reservation: reservation.clone(),
            })
            .collect();

        match to_operation_plan(&plan) {
            Err(Error::Validation { field, message }) => {
                assert_eq!(field, "migration");
                assert!(message.contains("DuplicateSource"), "{message}");
            }
            other => panic!("expected an inconsistent-plan error, got {other:?}"),
        }
    }

    /// Stored paths must already be canonical: a legacy row spelled `/w/a/`
    /// is reported as database corruption before any migration item is
    /// built, so two spellings of one path never reach the planner.
    #[test]
    #[cfg(unix)]
    fn test_non_canonical_stored_path_is_reported_as_corruption() {
        let db = create_test_database();
        for (path, port) in [("/w/a", 5000), ("/w/a/", 5001)] {
            db.connection()
                .execute(
                    "INSERT INTO reservations (path, tag, port, project, task, created_at, last_used_at) \
                     VALUES (?1, '', ?2, NULL, NULL, 0, 0)",
                    rusqlite::params![path, port],
                )
                .unwrap();
        }

        for recursive in [false, true] {
            let mut plan = MigratePlan::new(options("/w/a", "/w/b").with_recursive(recursive));
            assert!(
                matches!(plan.build(&db), Err(Error::DatabaseCorruption { .. })),
                "recursive={recursive}"
            );
        }
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM reservations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    fn two_connections() -> (tempfile::TempDir, Database, Database) {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::DatabaseConfig::new(directory.path().join("race.db"));
        let first = Database::open(config.clone()).unwrap();
        let second = Database::open(config).unwrap();
        (directory, first, second)
    }

    /// Regression: a destination reserved after the plan was built is not
    /// silently overwritten (the plan is rebuilt inside the transaction).
    #[test]
    #[cfg(unix)]
    fn test_execute_refuses_destination_reserved_after_planning() {
        let (_directory, mut db, mut other) = two_connections();
        reserve(&mut db, "/w/src", 5000);

        let mut plan = MigratePlan::new(options("/w/src", "/w/dst"));
        plan.build(&db).unwrap();
        assert_eq!(plan.conflict_count(), 0);

        // A concurrent writer reserves the destination after planning.
        reserve(&mut other, "/w/dst", 5001);

        assert!(execute_migrate(&plan, &mut db).is_err());
        assert_eq!(port_at(&db, "/w/dst"), Some(5001));
        assert_eq!(port_at(&db, "/w/src"), Some(5000));
    }

    /// Regression: a source released after the plan was built is not
    /// resurrected at the destination.
    #[test]
    #[cfg(unix)]
    fn test_execute_refuses_source_released_after_planning() {
        let (_directory, mut db, mut other) = two_connections();
        reserve(&mut db, "/w/src", 5000);

        let mut plan = MigratePlan::new(options("/w/src", "/w/dst"));
        plan.build(&db).unwrap();

        let key = ReservationKey::new(PathBuf::from("/w/src"), None).unwrap();
        assert!(other.delete_reservation(&key).unwrap());

        assert!(execute_migrate(&plan, &mut db).is_err());
        assert_eq!(port_at(&db, "/w/dst"), None);
        assert_eq!(port_at(&db, "/w/src"), None);
    }

    /// A source whose reservation data changed after planning (here: the
    /// same key re-reserved at another port) is not moved with data the plan
    /// never showed; the migration is refused without changes.
    #[test]
    #[cfg(unix)]
    fn test_execute_refuses_source_changed_after_planning() {
        let (_directory, mut db, mut other) = two_connections();
        reserve(&mut db, "/w/src", 5300);

        let mut plan = MigratePlan::new(options("/w/src", "/w/dst"));
        plan.build(&db).unwrap();
        assert_eq!(plan.migrations[0].reservation.port().value(), 5300);

        let key = ReservationKey::new(PathBuf::from("/w/src"), None).unwrap();
        assert!(other.delete_reservation(&key).unwrap());
        reserve(&mut other, "/w/src", 5301);

        match execute_migrate(&plan, &mut db) {
            Err(Error::ReservationConflict { details }) => {
                assert!(details.contains("changed while the migration"), "{details}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(port_at(&db, "/w/src"), Some(5301));
        assert_eq!(port_at(&db, "/w/dst"), None);
    }

    /// The plan executed is rebuilt against the locked database; an
    /// unchanged database executes exactly the plan that was built.
    #[test]
    #[cfg(unix)]
    fn test_execute_runs_the_planned_moves() {
        let (_directory, mut db, _other) = two_connections();
        reserve(&mut db, "/w/src", 5000);
        reserve(&mut db, "/w/src/child", 5001);

        let mut plan = MigratePlan::new(options("/w/src", "/w/dst").with_recursive(true));
        plan.build(&db).unwrap();
        let result = execute_migrate(&plan, &mut db).unwrap();

        assert_eq!(result.migrated_count, 2);
        assert_eq!(port_at(&db, "/w/dst"), Some(5000));
        assert_eq!(port_at(&db, "/w/dst/child"), Some(5001));
    }

    /// The unrelated-path guard covers sources and destinations; force or
    /// `allow_unrelated_path` bypass it.
    #[test]
    #[cfg(unix)]
    fn test_migrate_guard_checks_sources_and_destinations() {
        let mut db = create_test_database();
        let cwd = std::env::current_dir().unwrap();
        let related = cwd.join("trop-migrate-guard-source");
        let related_str = related.to_str().unwrap();
        reserve(&mut db, related_str, 5000);
        let unrelated = "/trop-migrate-guard-unrelated/destination";

        let guarded = MigrateOptions::new(related.clone(), PathBuf::from(unrelated));
        let mut plan = MigratePlan::new(guarded.clone());
        assert!(matches!(
            plan.build(&db),
            Err(Error::PathRelationshipViolation { .. })
        ));

        reserve(&mut db, unrelated, 5001);
        let back = MigrateOptions::new(PathBuf::from(unrelated), cwd.join("trop-migrate-guard-b"));
        let mut plan = MigratePlan::new(back);
        assert!(matches!(
            plan.build(&db),
            Err(Error::PathRelationshipViolation { .. })
        ));

        let mut plan = MigratePlan::new(guarded.clone().with_allow_unrelated_path(true));
        assert!(matches!(
            plan.build(&db),
            Err(Error::ReservationConflict { .. })
        ));
        let mut plan = MigratePlan::new(guarded.with_force(true));
        plan.build(&db).unwrap();
        assert_eq!(plan.conflict_count(), 1);
    }

    #[test]
    #[cfg(unix)]
    fn test_calculate_new_path_root_and_boundaries() {
        let cases = [
            ("/", "/", "/", Some("/")),
            ("/", "/", "/x", Some("/x")),
            ("/a", "/", "/", Some("/a")),
            ("/a/b", "/", "/x", Some("/x/a/b")),
            ("/a/b", "/a", "/", Some("/b")),
            ("/a", "/a", "/", Some("/")),
            ("/a/b", "/a/b", "/c", Some("/c")),
            ("/a/bc", "/a/b", "/c", None),
            ("/a", "/a/b", "/c", None),
        ];
        for (original, from, to, expected) in cases {
            let result = calculate_new_path(Path::new(original), Path::new(from), Path::new(to));
            match expected {
                Some(path) => assert_eq!(result.unwrap(), PathBuf::from(path), "{original}"),
                None => assert!(result.is_err(), "{original} under {from}"),
            }
        }
    }

    /// The rebased path agrees with `strip_prefix` + `join` + `normalize`
    /// (the previous implementation) on canonical inputs.
    #[test]
    #[cfg(unix)]
    fn test_calculate_new_path_matches_std() {
        let paths = [
            "/", "/a", "/a/b", "/a/bc", "/a/b/c", "/x", "/x/y", "/a/b/c/d",
        ];
        for original in paths {
            for from in paths {
                for to in paths {
                    let expected = Path::new(original)
                        .strip_prefix(from)
                        .ok()
                        .map(|relative| normalize(&Path::new(to).join(relative)).unwrap());
                    let actual =
                        calculate_new_path(Path::new(original), Path::new(from), Path::new(to))
                            .ok();
                    assert_eq!(actual, expected, "{original} from {from} to {to}");
                }
            }
        }
    }
}
