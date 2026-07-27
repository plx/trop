//! Reserve operation planning and execution.
//!
//! This module implements the reservation planning logic, including
//! idempotency checks, sticky field protection, and path validation.

use crate::config::{CleanupConfig, Config, ConfigValidator};
use crate::database::Database;
use crate::error::{Error, Result};
use crate::port::allocator::{
    allocator_from_config, AllocationOptions, AllocationResult, PortAllocator,
};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::{AutomaticCleanupStatus, Port, PortExhaustionDetails, Reservation, ReservationKey};
use rusqlite::Connection;

use super::cleanup::CleanupOperations;
use super::plan::{OperationPlan, PlanAction};

/// The requested operation for one optional reservation metadata field.
///
/// Absence is deliberately represented as [`Self::Preserve`] rather than as
/// `None`, because omitting metadata from an idempotent reserve request must
/// not clear a value that is already stored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MetadataIntent {
    /// Keep an existing value; infer a default when creating a new reservation.
    #[default]
    Preserve,
    /// Store the supplied value.
    Set(String),
    /// Store no value.
    Clear,
}

impl MetadataIntent {
    /// Create an explicit set intent.
    #[must_use]
    pub fn set(value: impl Into<String>) -> Self {
        Self::Set(value.into())
    }

    fn from_optional_value(value: Option<String>) -> Self {
        value.map_or(Self::Preserve, Self::Set)
    }

    fn resolve_existing(&self, existing: Option<&str>) -> Option<String> {
        match self {
            Self::Preserve => existing.map(ToOwned::to_owned),
            Self::Set(value) => Some(value.trim().to_string()),
            Self::Clear => None,
        }
    }

    fn resolve_new(&self, inferred: Option<String>) -> Option<String> {
        match self {
            Self::Preserve => inferred,
            Self::Set(value) => Some(value.trim().to_string()),
            Self::Clear => None,
        }
    }

    fn validate(&self, field: &str) -> Result<()> {
        if let Self::Set(value) = self {
            ConfigValidator::validate_runtime_identifier(field, value)?;
        }
        Ok(())
    }
}

/// Options for a reserve operation.
///
/// This struct contains all the parameters needed to plan a reserve operation,
/// including validation flags and metadata.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveOptions {
    /// The reservation key (path + optional tag).
    pub key: ReservationKey,

    /// Requested project metadata operation (sticky field).
    pub project: MetadataIntent,

    /// Requested task metadata operation (sticky field).
    pub task: MetadataIntent,

    /// The port to reserve. If None, automatic allocation will be used.
    pub port: Option<Port>,

    /// Preferred port for automatic allocation (hint).
    pub preferred_port: Option<Port>,

    /// Whether an existing reservation may be reallocated.
    pub overwrite: bool,

    /// Whether to ignore system occupancy checks during allocation.
    pub ignore_occupied: bool,

    /// Whether to ignore configured exclusions during allocation.
    pub ignore_exclusions: bool,

    /// Force flag - overrides all protections.
    pub force: bool,

    /// Allow operations on unrelated paths.
    pub allow_unrelated_path: bool,

    /// Allow changing the project field.
    pub allow_project_change: bool,

    /// Allow changing the task field.
    pub allow_task_change: bool,

    /// Disable automatic pruning during allocation.
    pub disable_autoprune: bool,

    /// Disable automatic expiration during allocation.
    pub disable_autoexpire: bool,
}

impl ReserveOptions {
    /// Creates a new `ReserveOptions` with the given key and port.
    ///
    /// All optional fields and flags are set to defaults:
    /// - project: [`MetadataIntent::Preserve`]
    /// - task: [`MetadataIntent::Preserve`]
    /// - `preferred_port`: None
    /// - overwrite: false
    /// - `ignore_occupied`: false
    /// - `ignore_exclusions`: false
    /// - force: false
    /// - `allow_unrelated_path`: false
    /// - `allow_project_change`: false
    /// - `allow_task_change`: false
    /// - `disable_autoprune`: false
    /// - `disable_autoexpire`: false
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::ReserveOptions;
    /// use trop::{ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let options = ReserveOptions::new(key, Some(port));
    /// assert!(!options.force);
    /// ```
    #[must_use]
    pub fn new(key: ReservationKey, port: Option<Port>) -> Self {
        Self {
            key,
            project: MetadataIntent::Preserve,
            task: MetadataIntent::Preserve,
            port,
            preferred_port: None,
            overwrite: false,
            ignore_occupied: false,
            ignore_exclusions: false,
            force: false,
            allow_unrelated_path: false,
            allow_project_change: false,
            allow_task_change: false,
            disable_autoprune: false,
            disable_autoexpire: false,
        }
    }

    /// Sets the project field.
    ///
    /// `Some(value)` explicitly sets the field. `None` preserves an existing
    /// value and permits inference only when a new reservation is created.
    #[must_use]
    pub fn with_project(mut self, project: Option<String>) -> Self {
        self.project = MetadataIntent::from_optional_value(project);
        self
    }

    /// Sets the task field.
    ///
    /// `Some(value)` explicitly sets the field. `None` preserves an existing
    /// value and permits inference only when a new reservation is created.
    #[must_use]
    pub fn with_task(mut self, task: Option<String>) -> Self {
        self.task = MetadataIntent::from_optional_value(task);
        self
    }

    /// Sets the project metadata intent directly.
    #[must_use]
    pub fn with_project_intent(mut self, intent: MetadataIntent) -> Self {
        self.project = intent;
        self
    }

    /// Sets the task metadata intent directly.
    #[must_use]
    pub fn with_task_intent(mut self, intent: MetadataIntent) -> Self {
        self.task = intent;
        self
    }

    /// Explicitly clears project metadata.
    #[must_use]
    pub fn with_clear_project(mut self) -> Self {
        self.project = MetadataIntent::Clear;
        self
    }

    /// Explicitly clears task metadata.
    #[must_use]
    pub fn with_clear_task(mut self) -> Self {
        self.task = MetadataIntent::Clear;
        self
    }

    /// Sets the force flag.
    #[must_use]
    pub const fn with_force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    /// Sets the `allow_unrelated_path` flag.
    #[must_use]
    pub const fn with_allow_unrelated_path(mut self, allow: bool) -> Self {
        self.allow_unrelated_path = allow;
        self
    }

    /// Sets the `allow_project_change` flag.
    #[must_use]
    pub const fn with_allow_project_change(mut self, allow: bool) -> Self {
        self.allow_project_change = allow;
        self
    }

    /// Sets the `allow_task_change` flag.
    #[must_use]
    pub const fn with_allow_task_change(mut self, allow: bool) -> Self {
        self.allow_task_change = allow;
        self
    }

    /// Sets the preferred port for automatic allocation.
    #[must_use]
    pub const fn with_preferred_port(mut self, port: Option<Port>) -> Self {
        self.preferred_port = port;
        self
    }

    /// Sets the overwrite flag.
    #[must_use]
    pub const fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Sets the `ignore_occupied` flag.
    #[must_use]
    pub const fn with_ignore_occupied(mut self, ignore: bool) -> Self {
        self.ignore_occupied = ignore;
        self
    }

    /// Sets the `ignore_exclusions` flag.
    #[must_use]
    pub const fn with_ignore_exclusions(mut self, ignore: bool) -> Self {
        self.ignore_exclusions = ignore;
        self
    }

    /// Sets the `disable_autoprune` flag.
    #[must_use]
    pub const fn with_disable_autoprune(mut self, disable: bool) -> Self {
        self.disable_autoprune = disable;
        self
    }

    /// Sets the `disable_autoexpire` flag.
    #[must_use]
    pub const fn with_disable_autoexpire(mut self, disable: bool) -> Self {
        self.disable_autoexpire = disable;
        self
    }

    /// Retains deferred Git inference for compatibility with earlier callers.
    ///
    /// Planning performs best-effort inference from the reservation's target
    /// path only when a new reservation is created. Existing reservations keep
    /// their stored metadata, so this method deliberately leaves all intents
    /// unchanged. The `path` argument is retained for source compatibility.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to search upward from for git repository
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::ReserveOptions;
    /// use trop::ReservationKey;
    /// use std::path::{Path, PathBuf};
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let options = ReserveOptions::new(key, None)
    ///     .with_git_inference(Path::new("/path"));
    /// // project and task will be inferred from git if available
    /// ```
    #[must_use]
    pub fn with_git_inference(self, _path: &std::path::Path) -> Self {
        self
    }
}

/// A reservation plan generator.
///
/// This struct is responsible for analyzing a reserve request and
/// generating a plan that describes what actions to take.
pub struct ReservePlan<'a> {
    options: ReserveOptions,
    config: &'a Config,
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredReserve {
    options: ReserveOptions,
    full_config: Config,
    occupancy_config: OccupancyCheckConfig,
}

pub(crate) struct AutomaticReserveOutcome {
    pub port: Port,
    pub warning: Option<String>,
}

struct PendingDeferredAllocation {
    project: Option<String>,
    task: Option<String>,
    created_at: Option<std::time::SystemTime>,
    ignored_key: Option<ReservationKey>,
}

enum DeferredKeyReconciliation {
    Complete(AutomaticReserveOutcome),
    Allocate(PendingDeferredAllocation),
}

impl<'a> ReservePlan<'a> {
    /// Creates a new reserve plan with the given options and config.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{ReservePlan, ReserveOptions};
    /// use trop::{ReservationKey, Port};
    /// use trop::config::ConfigBuilder;
    /// use std::path::PathBuf;
    ///
    /// let config = ConfigBuilder::new().build().unwrap();
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let options = ReserveOptions::new(key, Some(port));
    /// let planner = ReservePlan::new(options, &config);
    /// ```
    #[must_use]
    pub const fn new(options: ReserveOptions, config: &'a Config) -> Self {
        Self { options, config }
    }

    /// Gets the occupancy check configuration from the overall config.
    fn occupancy_config(&self) -> OccupancyCheckConfig {
        if let Some(ref occ_config) = self.config.occupancy_check {
            OccupancyCheckConfig::from(occ_config)
        } else {
            OccupancyCheckConfig::default()
        }
    }

    /// Builds an operation plan for this reserve request.
    ///
    /// This method performs all validation and determines what actions
    /// are needed. It may perform cleanup operations if allocation fails
    /// due to exhaustion and cleanup is enabled.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path relationship validation fails
    /// - Sticky field changes are attempted without permission
    /// - No port is available/specified
    /// - Port allocation fails (exhausted or preferred unavailable)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{ReservePlan, ReserveOptions};
    /// use trop::{Database, DatabaseConfig, ReservationKey, Port};
    /// use trop::config::ConfigBuilder;
    /// use std::path::PathBuf;
    ///
    /// let config = ConfigBuilder::new().build().unwrap();
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let options = ReserveOptions::new(key, Some(port))
    ///     .with_allow_unrelated_path(true);
    ///
    /// let plan = ReservePlan::new(options, &config).build_plan(db.connection()).unwrap();
    /// ```
    pub fn build_plan(&self, conn: &Connection) -> Result<OperationPlan> {
        let mut plan = OperationPlan::new(format!("Reserve port for {}", self.options.key));

        self.options.project.validate("project")?;
        self.options.task.validate("task")?;

        // Step 1: Validate path relationship
        if !self.options.force && !self.options.allow_unrelated_path {
            Database::validate_path_relationship(&self.options.key.path, false)?;
        }

        // Step 2: Check for existing reservation
        if let Some(existing) = Database::get_reservation(conn, &self.options.key)? {
            let project = self.options.project.resolve_existing(existing.project());
            let task = self.options.task.resolve_existing(existing.task());

            // Reservation exists - validate every requested sticky-field change
            // before planning any write.
            self.validate_sticky_fields(&existing, project.as_deref(), task.as_deref())?;

            if self.options.overwrite || self.options.force {
                return match self.allocate_port(conn, Some(&self.options.key))? {
                    AllocationResult::Allocated(port) => {
                        let replacement = self.updated_reservation(
                            &existing,
                            port,
                            project.clone(),
                            task.clone(),
                        )?;
                        Ok(plan.add_action(PlanAction::UpdateReservation(replacement)))
                    }
                    AllocationResult::Exhausted { .. } => self.plan_cleanup_retry(
                        conn,
                        plan,
                        project,
                        task,
                        Some(existing.created_at()),
                        Some(&self.options.key),
                    ),
                    AllocationResult::PreferredUnavailable { .. } => unreachable!(),
                };
            }

            if Self::metadata_changed(&existing, project.as_deref(), task.as_deref()) {
                let update = self.updated_reservation(&existing, existing.port(), project, task)?;
                plan = plan.add_action(PlanAction::UpdateReservation(update));
                return Ok(plan);
            }

            // A compatible request without overwrite keeps the stable port and
            // only refreshes its last-used timestamp. An explicit preferred
            // port remains a hint and does not relocate an existing key.
            plan = plan.add_action(PlanAction::UpdateLastUsed(self.options.key.clone()));
            return Ok(plan);
        }

        let project = self
            .options
            .project
            .resolve_new(super::inference::infer_project(&self.options.key.path));
        let task = self
            .options
            .task
            .resolve_new(super::inference::infer_task(&self.options.key.path));

        // Step 3: Determine port (unified allocation with fallback)
        match self.allocate_port(conn, None)? {
            AllocationResult::Allocated(port) => {
                // Step 4: Create the new reservation
                let reservation = Reservation::builder(self.options.key.clone(), port)
                    .project(project)
                    .task(task)
                    .build()?;
                Ok(plan.add_action(PlanAction::CreateReservation(reservation)))
            }
            AllocationResult::Exhausted { .. } => {
                self.plan_cleanup_retry(conn, plan, project, task, None, None)
            }
            AllocationResult::PreferredUnavailable { .. } => unreachable!(),
        }
    }

    fn allocate_port(
        &self,
        conn: &Connection,
        replaced_key: Option<&ReservationKey>,
    ) -> Result<AllocationResult> {
        let allocator = allocator_from_config(self.config)?;
        let allocation_options = self.allocation_options();
        let occupancy_config = self.occupancy_config();

        let allocate = |options: &AllocationOptions| match replaced_key {
            Some(key) => allocator.allocate_single_replacing(conn, options, &occupancy_config, key),
            None => allocator.allocate_single(conn, options, &occupancy_config),
        };

        allocate_with_fallback(&allocation_options, &allocate)
    }

    fn allocation_options(&self) -> AllocationOptions {
        AllocationOptions {
            preferred: self.options.port.or(self.options.preferred_port),
            ignore_occupied: self.options.ignore_occupied || self.options.force,
            ignore_exclusions: self.options.ignore_exclusions || self.options.force,
        }
    }

    fn plan_cleanup_retry(
        &self,
        conn: &Connection,
        plan: OperationPlan,
        project: Option<String>,
        task: Option<String>,
        created_at: Option<std::time::SystemTime>,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<OperationPlan> {
        let prune = !self.options.disable_autoprune;
        let expire = !self.options.disable_autoexpire;
        if !prune && !expire {
            let allocator = allocator_from_config(self.config)?;
            let occupancy_config = self.occupancy_config();
            let blockers = allocator.exhaustion_blockers(conn, &occupancy_config, ignored_key)?;
            let details = PortExhaustionDetails::new(
                AutomaticCleanupStatus::new(false, prune, expire, 0, 0),
                blockers,
            );
            return Err(Error::PortExhausted {
                range: (*allocator.range()).with_exhaustion_details(details),
                tried_cleanup: false,
            });
        }

        let allocator = allocator_from_config(self.config)?;
        let mut builder = Reservation::builder(self.options.key.clone(), allocator.range().min())
            .project(project)
            .task(task);
        if let Some(created_at) = created_at {
            builder = builder.created_at(created_at);
        }
        let reservation = builder.build()?.with_deferred_reserve(DeferredReserve {
            options: self.options.clone(),
            full_config: self.config.clone(),
            occupancy_config: self.occupancy_config(),
        });

        if ignored_key.is_some() {
            Ok(plan.add_action(PlanAction::UpdateReservation(reservation)))
        } else {
            Ok(plan.add_action(PlanAction::CreateReservation(reservation)))
        }
    }

    fn metadata_changed(existing: &Reservation, project: Option<&str>, task: Option<&str>) -> bool {
        project != existing.project() || task != existing.task()
    }

    fn updated_reservation(
        &self,
        existing: &Reservation,
        port: Port,
        project: Option<String>,
        task: Option<String>,
    ) -> Result<Reservation> {
        Ok(Reservation::builder(self.options.key.clone(), port)
            .project(project)
            .task(task)
            .created_at(existing.created_at())
            .build()?)
    }

    /// Validates that sticky fields aren't being changed without permission.
    fn validate_sticky_fields(
        &self,
        existing: &Reservation,
        project: Option<&str>,
        task: Option<&str>,
    ) -> Result<()> {
        // Check project field
        if !can_change_field(
            project,
            existing.project(),
            self.options.force,
            self.options.allow_project_change,
        ) {
            return Err(Error::StickyFieldChange {
                field: "project".to_string(),
                details: format!(
                    "Cannot change project from {:?} to {:?} without --force or --allow-project-change",
                    existing.project(),
                    project
                ),
            });
        }

        // Check task field
        if !can_change_field(
            task,
            existing.task(),
            self.options.force,
            self.options.allow_task_change,
        ) {
            return Err(Error::StickyFieldChange {
                field: "task".to_string(),
                details: format!(
                    "Cannot change task from {:?} to {:?} without --force or --allow-task-change",
                    existing.task(),
                    task
                ),
            });
        }

        Ok(())
    }
}

fn reconcile_deferred_key(
    conn: &Connection,
    placeholder: &Reservation,
    deferred: &DeferredReserve,
    reserve_plan: &ReservePlan<'_>,
) -> Result<DeferredKeyReconciliation> {
    let options = &deferred.options;
    let Some(existing) = Database::get_reservation(conn, &options.key)? else {
        return Ok(DeferredKeyReconciliation::Allocate(
            PendingDeferredAllocation {
                project: placeholder.project().map(ToOwned::to_owned),
                task: placeholder.task().map(ToOwned::to_owned),
                created_at: None,
                ignored_key: None,
            },
        ));
    };

    let project = options.project.resolve_existing(existing.project());
    let task = options.task.resolve_existing(existing.task());
    reserve_plan.validate_sticky_fields(&existing, project.as_deref(), task.as_deref())?;

    if options.overwrite || options.force {
        return Ok(DeferredKeyReconciliation::Allocate(
            PendingDeferredAllocation {
                project,
                task,
                created_at: Some(existing.created_at()),
                ignored_key: Some(options.key.clone()),
            },
        ));
    }

    if ReservePlan::metadata_changed(&existing, project.as_deref(), task.as_deref()) {
        let update = reserve_plan.updated_reservation(&existing, existing.port(), project, task)?;
        Database::create_reservation_simple(conn, &update)?;
    } else {
        Database::update_last_used_simple(conn, &options.key)?;
    }
    Ok(DeferredKeyReconciliation::Complete(
        AutomaticReserveOutcome {
            port: existing.port(),
            warning: None,
        },
    ))
}

fn port_exhausted_error(
    allocator: &PortAllocator,
    conn: &Connection,
    occupancy_config: &OccupancyCheckConfig,
    ignored_key: Option<&ReservationKey>,
    cleanup: AutomaticCleanupStatus,
) -> Result<Error> {
    let blockers = allocator.exhaustion_blockers(conn, occupancy_config, ignored_key)?;
    let details = PortExhaustionDetails::new(cleanup, blockers);
    Ok(Error::PortExhausted {
        range: (*allocator.range()).with_exhaustion_details(details),
        tried_cleanup: cleanup.attempted(),
    })
}

pub(crate) fn execute_reserve_after_cleanup(
    conn: &Connection,
    placeholder: &Reservation,
) -> Result<AutomaticReserveOutcome> {
    execute_reserve_after_cleanup_with_barrier(conn, placeholder, || {})
}

fn execute_reserve_after_cleanup_with_barrier<B>(
    conn: &Connection,
    placeholder: &Reservation,
    after_candidate_discovery: B,
) -> Result<AutomaticReserveOutcome>
where
    B: FnOnce(),
{
    let deferred = placeholder
        .deferred_reserve()
        .ok_or_else(|| Error::Validation {
            field: "plan".into(),
            message: "deferred reserve execution requires a deferred reservation".into(),
        })?;

    let execute = |conn: &Connection| {
        let options = &deferred.options;
        let full_config = &deferred.full_config;
        let occupancy_config = &deferred.occupancy_config;
        let reserve_plan = ReservePlan::new(options.clone(), full_config);
        let pending = match reconcile_deferred_key(conn, placeholder, deferred, &reserve_plan)? {
            DeferredKeyReconciliation::Complete(outcome) => return Ok(outcome),
            DeferredKeyReconciliation::Allocate(pending) => pending,
        };
        let allocator = allocator_from_config(full_config)?;
        let allocation_options = reserve_plan.allocation_options();
        let allocate = |options: &AllocationOptions| match pending.ignored_key {
            Some(ref ignored_key) => {
                allocator.allocate_single_replacing(conn, options, occupancy_config, ignored_key)
            }
            None => allocator.allocate_single(conn, options, occupancy_config),
        };

        if let AllocationResult::Allocated(port) =
            allocate_with_fallback(&allocation_options, &allocate)?
        {
            return persist_deferred_outcome(conn, &options.key, &pending, port, None);
        }

        let prune = !options.disable_autoprune;
        let expire = !options.disable_autoexpire;
        if !prune && !expire {
            return Err(port_exhausted_error(
                &allocator,
                conn,
                occupancy_config,
                pending.ignored_key.as_ref(),
                AutomaticCleanupStatus::new(false, prune, expire, 0, 0),
            )?);
        }

        let default_cleanup = CleanupConfig {
            expire_after_days: Some(30),
        };
        let cleanup_config = full_config.cleanup.as_ref().unwrap_or(&default_cleanup);
        let cleanup = CleanupOperations::automatic_cleanup_in_connection(
            conn,
            cleanup_config,
            prune,
            expire,
            after_candidate_discovery,
        )?;

        match allocate_with_fallback(&allocation_options, &allocate)? {
            AllocationResult::Allocated(port) => persist_deferred_outcome(
                conn,
                &options.key,
                &pending,
                port,
                Some(format!(
                    "Automatic cleanup after initial port exhaustion pruned {} and expired \
                         {} reservation(s); allocation succeeded on the single retry.",
                    cleanup.pruned_count, cleanup.expired_count
                )),
            ),
            AllocationResult::Exhausted { .. } => Err(port_exhausted_error(
                &allocator,
                conn,
                occupancy_config,
                pending.ignored_key.as_ref(),
                AutomaticCleanupStatus::new(
                    true,
                    prune,
                    expire,
                    cleanup.pruned_count,
                    cleanup.expired_count,
                ),
            )?),
            AllocationResult::PreferredUnavailable { .. } => unreachable!(),
        }
    };

    Database::with_immediate_transaction_or_savepoint(
        conn,
        "trop_reserve_after_cleanup",
        "automatic cleanup and reserve retry",
        execute,
    )
}

fn persist_deferred_outcome(
    conn: &Connection,
    key: &ReservationKey,
    pending: &PendingDeferredAllocation,
    port: Port,
    warning: Option<String>,
) -> Result<AutomaticReserveOutcome> {
    persist_deferred_reservation(
        conn,
        key,
        pending.project.as_deref(),
        pending.task.as_deref(),
        pending.created_at,
        port,
    )?;
    Ok(AutomaticReserveOutcome { port, warning })
}

fn persist_deferred_reservation(
    conn: &Connection,
    key: &ReservationKey,
    project: Option<&str>,
    task: Option<&str>,
    created_at: Option<std::time::SystemTime>,
    port: Port,
) -> Result<()> {
    let mut builder = Reservation::builder(key.clone(), port)
        .project(project.map(ToOwned::to_owned))
        .task(task.map(ToOwned::to_owned));
    if let Some(created_at) = created_at {
        builder = builder.created_at(created_at);
    }
    Database::create_reservation_simple(conn, &builder.build()?)
}

fn allocate_with_fallback(
    allocation_options: &AllocationOptions,
    allocate: &(impl Fn(&AllocationOptions) -> Result<AllocationResult> + ?Sized),
) -> Result<AllocationResult> {
    match allocate(allocation_options)? {
        AllocationResult::PreferredUnavailable { .. } => {
            let fallback_options = AllocationOptions {
                preferred: None,
                ignore_occupied: allocation_options.ignore_occupied,
                ignore_exclusions: allocation_options.ignore_exclusions,
            };
            allocate(&fallback_options)
        }
        result => Ok(result),
    }
}

/// Generic helper to check if a sticky field can be changed.
///
/// This function encapsulates the common logic for validating sticky field changes:
/// - If force or the field-specific allow flag is set, allow the change
/// - Otherwise, only allow if the value isn't actually changing
///
/// # Arguments
///
/// * `new_value` - The new value being proposed (as `Option<&String>`)
/// * `existing_value` - The existing value in the database (as `Option<&str>`)
/// * `force` - Whether the force flag is set (overrides all checks)
/// * `allow_change` - Whether the field-specific allow flag is set
///
/// # Returns
///
/// `true` if the change is allowed, `false` otherwise
fn can_change_field(
    new_value: Option<&str>,
    existing_value: Option<&str>,
    force: bool,
    allow_change: bool,
) -> bool {
    // If force or specific allow flag is set, allow change
    if force || allow_change {
        return true;
    }

    // Otherwise, only allow if the value isn't actually changing
    match (new_value, existing_value) {
        (None, None) => true,
        (Some(new), Some(old)) => new == old,
        _ => false, // One is Some, other is None - this is a change
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PortConfig};
    use crate::database::test_util::create_test_database;
    use crate::database::DatabaseConfig;
    use crate::operations::PlanExecutor;
    use std::net::{Ipv4Addr, TcpListener};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;

    // Helper to create a test config with reasonable defaults
    fn create_test_config() -> Config {
        Config {
            ports: Some(PortConfig {
                min: 5000,
                max: Some(7000),
                max_offset: None,
            }),
            ..Default::default()
        }
    }

    // Property-based testing module
    // These tests verify mathematical properties and invariants of the reservation system
    #[cfg(feature = "property-tests")]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        // Strategy to generate valid Port values (1-65535)
        fn port_strategy() -> impl Strategy<Value = Port> {
            (1u16..=65535).prop_map(|p| Port::try_from(p).unwrap())
        }

        // Strategy to generate valid ReservationKey instances
        fn reservation_key_strategy() -> impl Strategy<Value = ReservationKey> {
            prop_oneof![
                // Untagged keys
                Just(ReservationKey::new(PathBuf::from("/test/path"), None).unwrap()),
                // Tagged keys
                "[a-z]{1,10}".prop_map(|tag| ReservationKey::new(
                    PathBuf::from("/test/path"),
                    Some(tag)
                )
                .unwrap()),
            ]
        }

        // Strategy to generate optional non-empty strings for project/task
        fn optional_string_strategy() -> impl Strategy<Value = Option<String>> {
            prop_oneof![Just(None), "[a-zA-Z0-9_-]{1,20}".prop_map(Some),]
        }

        // PROPERTY: ReserveOptions builder is idempotent
        // Building options with the same values multiple times produces equal structures
        proptest! {
            #[test]
            fn prop_reserve_options_builder_idempotent(
                key in reservation_key_strategy(),
                port in port_strategy(),
                project in optional_string_strategy(),
                task in optional_string_strategy(),
                force in any::<bool>(),
                overwrite in any::<bool>(),
                allow_unrelated in any::<bool>(),
                allow_project_change in any::<bool>(),
                allow_task_change in any::<bool>(),
            ) {
                // PROPERTY: Building options twice with identical parameters yields identical results
                // This verifies that the builder has no hidden state or side effects
                let opts1 = ReserveOptions::new(key.clone(), Some(port))
                    .with_project(project.clone())
                    .with_task(task.clone())
                    .with_force(force)
                    .with_overwrite(overwrite)
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_allow_project_change(allow_project_change)
                    .with_allow_task_change(allow_task_change);

                let opts2 = ReserveOptions::new(key.clone(), Some(port))
                    .with_project(project.clone())
                    .with_task(task.clone())
                    .with_force(force)
                    .with_overwrite(overwrite)
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_allow_project_change(allow_project_change)
                    .with_allow_task_change(allow_task_change);

                // Compare all fields
                prop_assert_eq!(opts1.port, opts2.port);
                prop_assert_eq!(opts1.project, opts2.project);
                prop_assert_eq!(opts1.task, opts2.task);
                prop_assert_eq!(opts1.force, opts2.force);
                prop_assert_eq!(opts1.overwrite, opts2.overwrite);
                prop_assert_eq!(opts1.allow_unrelated_path, opts2.allow_unrelated_path);
                prop_assert_eq!(opts1.allow_project_change, opts2.allow_project_change);
                prop_assert_eq!(opts1.allow_task_change, opts2.allow_task_change);
            }
        }

        // PROPERTY: Builder methods are commutative
        // The order of setting flags doesn't affect the final result
        proptest! {
            #[test]
            fn prop_reserve_options_builder_commutative(
                key in reservation_key_strategy(),
                port in port_strategy(),
                force in any::<bool>(),
                allow_unrelated in any::<bool>(),
            ) {
                // PROPERTY: Order of builder calls doesn't matter (commutativity)
                // This is a critical property for builder patterns - users should be able
                // to chain methods in any order
                let opts1 = ReserveOptions::new(key.clone(), Some(port))
                    .with_force(force)
                    .with_allow_unrelated_path(allow_unrelated);

                let opts2 = ReserveOptions::new(key.clone(), Some(port))
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_force(force);

                prop_assert_eq!(opts1.force, opts2.force);
                prop_assert_eq!(opts1.allow_unrelated_path, opts2.allow_unrelated_path);
            }
        }

        // PROPERTY: can_change_field logic - force always allows changes
        // This is the master override property of the force flag
        proptest! {
            #[test]
            fn prop_can_change_field_force_overrides_all(
                new_value in optional_string_strategy(),
                existing_value in optional_string_strategy(),
                allow_change in any::<bool>(),
            ) {
                // PROPERTY: When force=true, can_change_field ALWAYS returns true
                // This is the key invariant of the force flag - it overrides all protections
                let result = can_change_field(
                    new_value.as_deref(),
                    existing_value.as_deref(),
                    true,  // force = true
                    allow_change,
                );

                prop_assert!(result, "force=true must always allow changes regardless of values");
            }
        }

        // PROPERTY: can_change_field logic - specific allow flag permits changes
        // This tests the fine-grained control mechanism
        proptest! {
            #[test]
            fn prop_can_change_field_allow_flag_permits(
                new_value in optional_string_strategy(),
                existing_value in optional_string_strategy(),
            ) {
                // PROPERTY: When the specific allow flag is true, changes are permitted
                // This verifies the fine-grained override mechanism
                let result = can_change_field(
                    new_value.as_deref(),
                    existing_value.as_deref(),
                    false,  // force = false
                    true,   // allow_change = true
                );

                prop_assert!(result, "specific allow flag must permit changes");
            }
        }

        // PROPERTY: can_change_field logic - unchanged values are always allowed
        // This is the idempotency foundation
        proptest! {
            #[test]
            fn prop_can_change_field_same_value_allowed(
                value in optional_string_strategy(),
            ) {
                // PROPERTY: Applying the same value is always allowed (idempotent)
                // This is crucial for reservation idempotency - reapplying the same
                // reservation parameters should never fail due to sticky fields
                let result = can_change_field(
                    value.as_deref(),
                    value.as_deref(),
                    false,  // force = false
                    false,  // allow_change = false
                );

                prop_assert!(result, "setting the same value must always be allowed (idempotency)");
            }
        }

        // PROPERTY: can_change_field logic - actual changes are blocked without permission
        // This tests the sticky field protection mechanism
        proptest! {
            #[test]
            fn prop_can_change_field_different_value_blocked(
                value1 in "[a-z]{1,5}",
                value2 in "[A-Z]{1,5}",  // Different case ensures different values
            ) {
                // PROPERTY: Changing from one non-None value to a different non-None value
                // is blocked when both force and allow_change are false
                // This is the core sticky field protection
                let result = can_change_field(
                    Some(&value1),
                    Some(value2.as_str()),
                    false,  // force = false
                    false,  // allow_change = false
                );

                prop_assert!(!result, "changing to a different value must be blocked without permission");
            }
        }

        // PROPERTY: can_change_field logic - None to Some transitions are blocked
        // This tests that setting a field that was previously unset is considered a change
        proptest! {
            #[test]
            fn prop_can_change_field_none_to_some_blocked(
                new_value in "[a-z]{1,10}",
            ) {
                // PROPERTY: Setting a field from None to Some is a change and must be blocked
                // without permission. This prevents accidentally setting metadata on
                // existing reservations that didn't have it.
                let result = can_change_field(
                    Some(&new_value),
                    None,  // existing is None
                    false,  // force = false
                    false,  // allow_change = false
                );

                prop_assert!(!result, "None -> Some transition must be blocked without permission");
            }
        }

        // PROPERTY: can_change_field logic - Some to None transitions are blocked
        // This tests that clearing a field is also considered a change
        proptest! {
            #[test]
            fn prop_can_change_field_some_to_none_blocked(
                existing_value in "[a-z]{1,10}",
            ) {
                // PROPERTY: Clearing a field from Some to None is a change and must be blocked
                // without permission. This prevents accidentally removing metadata.
                let result = can_change_field(
                    None,  // new is None
                    Some(existing_value.as_str()),
                    false,  // force = false
                    false,  // allow_change = false
                );

                prop_assert!(!result, "Some -> None transition must be blocked without permission");
            }
        }

        // PROPERTY: Multiple reserves with same parameters generate same plan type
        // This verifies idempotency at the plan generation level
        proptest! {
            #[test]
            fn prop_idempotent_reserve_generates_update_plan(
                port in port_strategy(),
            ) {
                // PROPERTY: Once a reservation exists, subsequent reserves with the same
                // parameters generate UpdateLastUsed actions (not CreateReservation)
                // This is the core idempotency guarantee
                let mut db = create_test_database();
                let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();

                // Create initial reservation
                let reservation = Reservation::builder(key.clone(), port).build().unwrap();
                db.create_reservation(&reservation).unwrap();

                // Plan a second reservation with same parameters
                let config = super::create_test_config();
                let options = ReserveOptions::new(key, Some(port))
                    .with_allow_unrelated_path(true);

                let plan = ReservePlan::new(options, &config).build_plan(db.connection()).unwrap();

                // Must generate UpdateLastUsed, not CreateReservation
                prop_assert_eq!(plan.len(), 1);
                prop_assert!(matches!(plan.actions[0], PlanAction::UpdateLastUsed(_)),
                    "idempotent reserve must generate UpdateLastUsed action");
            }
        }

        // PROPERTY: Force flag overrides path relationship validation
        // This tests the force flag's path validation override behavior
        proptest! {
            #[test]
            fn prop_force_overrides_path_validation(
                port in port_strategy(),
            ) {
                // PROPERTY: With force=true, path relationship validation is skipped
                // This allows operations on unrelated paths without explicit permission
                let db = create_test_database();
                let config = super::create_test_config();
                let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

                let options_without_force = ReserveOptions::new(key.clone(), Some(port))
                    .with_force(false)
                    .with_allow_unrelated_path(false);

                let options_with_force = ReserveOptions::new(key, Some(port))
                    .with_force(true)
                    .with_allow_unrelated_path(false);

                // Without force, should fail path validation
                let result_without = ReservePlan::new(options_without_force, &config).build_plan(db.connection());
                prop_assert!(result_without.is_err(), "unrelated path must fail without force");

                // With force, should succeed
                let result_with = ReservePlan::new(options_with_force, &config).build_plan(db.connection());
                prop_assert!(result_with.is_ok(), "force must override path validation");
            }
        }

        // PROPERTY: allow_unrelated_path flag enables unrelated path operations
        // This tests the specific path override mechanism
        proptest! {
            #[test]
            fn prop_allow_unrelated_path_enables_operations(
                port in port_strategy(),
            ) {
                // PROPERTY: The allow_unrelated_path flag specifically enables operations
                // on paths unrelated to the current working directory
                let db = create_test_database();
                let config = super::create_test_config();
                let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

                let options = ReserveOptions::new(key, Some(port))
                    .with_force(false)
                    .with_allow_unrelated_path(true);

                let result = ReservePlan::new(options, &config).build_plan(db.connection());
                prop_assert!(result.is_ok(), "allow_unrelated_path must enable unrelated path operations");
            }
        }
    }

    // Original manual tests follow...

    #[test]
    fn test_reserve_options_new() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let options = ReserveOptions::new(key, Some(port));

        assert_eq!(options.project, MetadataIntent::Preserve);
        assert_eq!(options.task, MetadataIntent::Preserve);
        assert!(!options.force);
        assert!(!options.overwrite);
        assert!(!options.allow_unrelated_path);
        assert!(!options.allow_project_change);
        assert!(!options.allow_task_change);
    }

    #[test]
    fn test_reserve_options_builder() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("test-project".to_string()))
            .with_task(Some("test-task".to_string()))
            .with_force(true)
            .with_overwrite(true)
            .with_allow_unrelated_path(true);

        assert!(options.force);
        assert!(options.overwrite);
        assert!(options.allow_unrelated_path);
        assert_eq!(
            options.project,
            MetadataIntent::Set("test-project".to_string())
        );
        assert_eq!(options.task, MetadataIntent::Set("test-task".to_string()));

        let cleared = ReserveOptions::new(
            ReservationKey::new(PathBuf::from("/path"), None).unwrap(),
            Some(port),
        )
        .with_clear_project()
        .with_clear_task();
        assert_eq!(cleared.project, MetadataIntent::Clear);
        assert_eq!(cleared.task, MetadataIntent::Clear);
    }

    #[test]
    fn test_plan_new_reservation() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

        let plan = ReservePlan::new(options, &config)
            .build_plan(db.connection())
            .unwrap();

        assert_eq!(plan.len(), 1);
        assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));
    }

    #[test]
    fn test_plan_existing_reservation_idempotent() {
        let mut db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create initial reservation
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        // Plan second reservation with same parameters
        let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

        let plan = ReservePlan::new(options, &config)
            .build_plan(db.connection())
            .unwrap();

        // Should just update timestamp
        assert_eq!(plan.len(), 1);
        assert!(matches!(plan.actions[0], PlanAction::UpdateLastUsed(_)));
    }

    #[test]
    fn test_plan_sticky_field_project_change_denied() {
        let mut db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create initial reservation with project
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("project1".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Try to change project without permission
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("project2".to_string()))
            .with_allow_unrelated_path(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::StickyFieldChange { .. }
        ));
    }

    #[test]
    fn test_plan_sticky_field_project_change_with_force() {
        let mut db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create initial reservation with project
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("project1".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Change project with force flag
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("project2".to_string()))
            .with_force(true)
            .with_allow_unrelated_path(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        // Should succeed with force
        assert!(result.is_ok());
    }

    #[test]
    fn test_plan_sticky_field_project_change_with_allow_flag() {
        let mut db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create initial reservation with project
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("project1".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Change project with specific allow flag
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("project2".to_string()))
            .with_allow_project_change(true)
            .with_allow_unrelated_path(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        // Should succeed with allow flag
        assert!(result.is_ok());
    }

    #[test]
    fn test_plan_sticky_field_task_change_denied() {
        let mut db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create initial reservation with task
        let reservation = Reservation::builder(key.clone(), port)
            .task(Some("task1".to_string()))
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Try to change task without permission
        let options = ReserveOptions::new(key, Some(port))
            .with_task(Some("task2".to_string()))
            .with_allow_unrelated_path(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::StickyFieldChange { .. }
        ));
    }

    #[test]
    fn test_plan_automatic_allocation() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();

        // No port specified - use automatic allocation
        let options = ReserveOptions::new(key.clone(), None).with_allow_unrelated_path(true);

        let plan = ReservePlan::new(options, &config)
            .build_plan(db.connection())
            .unwrap();

        assert_eq!(plan.len(), 1);
        // Should create a reservation with an automatically allocated port
        match &plan.actions[0] {
            PlanAction::CreateReservation(res) => {
                assert_eq!(res.key(), &key);
                // Port should be within configured range (5000-7000)
                assert!(res.port().value() >= 5000);
                assert!(res.port().value() <= 7000);
            }
            _ => panic!("Expected CreateReservation action"),
        }
    }

    #[test]
    fn test_plan_automatic_allocation_exhausted() {
        let mut db = create_test_database();
        let config = Config {
            ports: Some(PortConfig {
                min: 5000,
                max: Some(5001), // Only 2 ports available
                max_offset: None,
            }),
            ..Default::default()
        };

        // Reserve all ports in the range
        let key1 = ReservationKey::new(PathBuf::from("/test/path1"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/test/path2"), None).unwrap();
        let res1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        let res2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&res1).unwrap();
        db.create_reservation(&res2).unwrap();

        // Try to allocate another port - should fail with exhaustion
        let key3 = ReservationKey::new(PathBuf::from("/test/path3"), None).unwrap();
        let options = ReserveOptions::new(key3, None)
            .with_allow_unrelated_path(true)
            .with_disable_autoprune(true)  // Disable autoclean to test exhaustion
            .with_disable_autoexpire(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        let error = result.unwrap_err();
        match &error {
            Error::PortExhausted {
                range,
                tried_cleanup,
            } => {
                assert_eq!(range.min().value(), 5000);
                assert_eq!(range.max().value(), 5001);
                assert!(!tried_cleanup);
            }
            other => panic!("expected PortExhausted, got {other:?}"),
        }
        let details = error
            .port_exhaustion_details()
            .expect("reserve exhaustion must expose typed details");
        assert!(!details.cleanup().attempted());
        assert!(!details.cleanup().prune_enabled());
        assert!(!details.cleanup().expire_enabled());
        assert!(details.blockers().reserved());
        assert!(!details.blockers().excluded());
        assert!(!details.blockers().occupied());
    }

    #[test]
    fn test_deferred_reserve_reconciles_same_key_created_after_planning() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("same-key-reconciliation.db");
        let database_config =
            DatabaseConfig::new(&database_path).with_busy_timeout(Duration::from_secs(2));
        let mut planning_db = Database::open(database_config.clone()).unwrap();
        let mut competing_db = Database::open(database_config).unwrap();
        let first_port = Port::try_from(5450).unwrap();
        let second_port = Port::try_from(5451).unwrap();

        let first_blocker_path = directory.path().join("first-blocker");
        let second_blocker_path = directory.path().join("second-blocker");
        let target_path = directory.path().join("target");
        std::fs::create_dir(&first_blocker_path).unwrap();
        std::fs::create_dir(&second_blocker_path).unwrap();
        std::fs::create_dir(&target_path).unwrap();
        let first_blocker_key = ReservationKey::new(first_blocker_path, None).unwrap();
        let second_blocker_key = ReservationKey::new(second_blocker_path, None).unwrap();
        let target_key = ReservationKey::new(target_path, None).unwrap();

        planning_db
            .create_reservation(
                &Reservation::builder(first_blocker_key.clone(), first_port)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        planning_db
            .create_reservation(
                &Reservation::builder(second_blocker_key.clone(), second_port)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let config = Config {
            ports: Some(PortConfig {
                min: first_port.value(),
                max: Some(second_port.value()),
                max_offset: None,
            }),
            ..Default::default()
        };
        let options = ReserveOptions::new(target_key.clone(), None).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(options, &config)
            .build_plan(planning_db.connection())
            .unwrap();
        assert!(matches!(
            plan.actions.as_slice(),
            [PlanAction::CreateReservation(reservation)]
                if reservation.requires_allocation_at_execution()
        ));

        Database::delete_reservation_simple(competing_db.connection(), &first_blocker_key).unwrap();
        Database::delete_reservation_simple(competing_db.connection(), &second_blocker_key)
            .unwrap();
        competing_db
            .create_reservation(
                &Reservation::builder(target_key.clone(), first_port)
                    .project(Some("concurrent-owner".into()))
                    .task(Some("concurrent-task".into()))
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let result = PlanExecutor::new(planning_db.connection())
            .execute(&plan)
            .unwrap();
        assert_eq!(result.port, Some(first_port));
        assert!(result.warnings.is_empty());

        let stored = Database::get_reservation(planning_db.connection(), &target_key)
            .unwrap()
            .expect("concurrent same-key reservation must survive");
        assert_eq!(stored.port(), first_port);
        assert_eq!(stored.project(), Some("concurrent-owner"));
        assert_eq!(stored.task(), Some("concurrent-task"));
        assert_eq!(
            Database::list_all_reservations(planning_db.connection())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_cleanup_retry_exposes_typed_aggregate_blockers_and_rolls_back() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let occupied_port = Port::try_from(listener.local_addr().unwrap().port()).unwrap();
        let directory = tempdir().unwrap();
        let stale_path = directory.path().join("stale");
        let target_path = directory.path().join("target");
        std::fs::create_dir(&stale_path).unwrap();
        std::fs::create_dir(&target_path).unwrap();
        let stale_key = ReservationKey::new(stale_path.clone(), None).unwrap();
        let target_key = ReservationKey::new(target_path, None).unwrap();
        let mut db = create_test_database();
        db.create_reservation(
            &Reservation::builder(stale_key.clone(), occupied_port)
                .build()
                .unwrap(),
        )
        .unwrap();
        std::fs::remove_dir(&stale_path).unwrap();

        let config = Config {
            ports: Some(PortConfig {
                min: occupied_port.value(),
                max: Some(occupied_port.value()),
                max_offset: None,
            }),
            ..Default::default()
        };
        let options =
            ReserveOptions::new(target_key, Some(occupied_port)).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(options, &config)
            .build_plan(db.connection())
            .unwrap();
        let error = PlanExecutor::new(db.connection())
            .execute(&plan)
            .unwrap_err();

        match &error {
            Error::PortExhausted {
                range,
                tried_cleanup,
            } => {
                assert_eq!(range.min(), occupied_port);
                assert_eq!(range.max(), occupied_port);
                assert!(tried_cleanup);
            }
            other => panic!("expected PortExhausted, got {other:?}"),
        }
        let details = error.port_exhaustion_details().unwrap();
        assert!(details.cleanup().attempted());
        assert!(details.cleanup().prune_enabled());
        assert!(details.cleanup().expire_enabled());
        assert_eq!(details.cleanup().pruned(), 1);
        assert_eq!(details.cleanup().expired(), 0);
        assert!(!details.blockers().reserved());
        assert!(!details.blockers().excluded());
        assert!(details.blockers().occupied());
        assert!(
            Database::get_reservation(db.connection(), &stale_key)
                .unwrap()
                .is_some(),
            "terminal exhaustion must roll automatic cleanup back"
        );
        drop(listener);
    }

    #[test]
    fn test_deferred_reserve_maps_transaction_contention_to_lock_timeout() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("deferred-lock-timeout.db");
        let database_config = DatabaseConfig::new(&database_path).with_busy_timeout(Duration::ZERO);
        let mut planning_db = Database::open(database_config.clone()).unwrap();
        let competing_db = Database::open(database_config).unwrap();
        let only_port = Port::try_from(5460).unwrap();

        let blocker_path = directory.path().join("blocker");
        let target_path = directory.path().join("target");
        std::fs::create_dir(&blocker_path).unwrap();
        std::fs::create_dir(&target_path).unwrap();
        let blocker_key = ReservationKey::new(blocker_path, None).unwrap();
        let target_key = ReservationKey::new(target_path, None).unwrap();
        planning_db
            .create_reservation(
                &Reservation::builder(blocker_key, only_port)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let config = Config {
            ports: Some(PortConfig {
                min: only_port.value(),
                max: Some(only_port.value()),
                max_offset: None,
            }),
            ..Default::default()
        };
        let plan = ReservePlan::new(
            ReserveOptions::new(target_key, None).with_allow_unrelated_path(true),
            &config,
        )
        .build_plan(planning_db.connection())
        .unwrap();
        assert!(matches!(
            plan.actions.as_slice(),
            [PlanAction::CreateReservation(reservation)]
                if reservation.requires_allocation_at_execution()
        ));

        competing_db
            .connection()
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        let error = PlanExecutor::new(planning_db.connection())
            .execute(&plan)
            .unwrap_err();
        assert!(matches!(
            error,
            Error::LockTimeout {
                timeout,
                ref operation
            } if timeout == Duration::ZERO
                && operation
                    == "starting an immediate transaction for automatic cleanup and reserve retry"
        ));
        competing_db.connection().execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn test_cleanup_retry_serializes_refresh_after_candidate_discovery() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("reserve-cleanup-race.db");
        let database_config =
            DatabaseConfig::new(&database_path).with_busy_timeout(Duration::from_secs(2));
        let mut reserve_db = Database::open(database_config.clone()).unwrap();
        let mut refresh_db = Database::open(database_config).unwrap();

        let refresh_path = directory.path().join("refresh-target");
        let other_expired_path = directory.path().join("other-expired");
        let replacement_path = directory.path().join("replacement");
        std::fs::create_dir(&refresh_path).unwrap();
        std::fs::create_dir(&other_expired_path).unwrap();
        std::fs::create_dir(&replacement_path).unwrap();

        let refresh_key = ReservationKey::new(refresh_path, None).unwrap();
        let other_expired_key = ReservationKey::new(other_expired_path, None).unwrap();
        let replacement_key = ReservationKey::new(replacement_path, None).unwrap();
        let first_port = Port::try_from(5400).unwrap();
        let second_port = Port::try_from(5401).unwrap();
        let expired_at = SystemTime::now() - Duration::from_secs(31 * 24 * 60 * 60);
        reserve_db
            .create_reservation(
                &Reservation::builder(refresh_key.clone(), first_port)
                    .last_used_at(expired_at)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        reserve_db
            .create_reservation(
                &Reservation::builder(other_expired_key.clone(), second_port)
                    .last_used_at(expired_at)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let config = Config {
            ports: Some(PortConfig {
                min: first_port.value(),
                max: Some(second_port.value()),
                max_offset: None,
            }),
            cleanup: Some(CleanupConfig {
                expire_after_days: Some(30),
            }),
            ..Default::default()
        };
        let refresh_config = config.clone();
        let refresh_thread_key = refresh_key.clone();
        let (start_sender, start_receiver) = mpsc::sync_channel(0);
        let (attempt_sender, attempt_receiver) = mpsc::sync_channel(0);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let refresh_thread = thread::spawn(move || {
            start_receiver.recv().unwrap();
            attempt_sender.send(()).unwrap();
            let result = (|| -> Result<()> {
                let transaction = refresh_db.begin_transaction()?;
                let options = ReserveOptions::new(refresh_thread_key, Some(first_port))
                    .with_allow_unrelated_path(true)
                    .with_ignore_occupied(true)
                    .with_disable_autoprune(true)
                    .with_disable_autoexpire(true);
                let plan = ReservePlan::new(options, &refresh_config).build_plan(&transaction)?;
                PlanExecutor::new(&transaction).execute(&plan)?;
                transaction.commit()?;
                Ok(())
            })();
            let _ = done_sender.send(());
            result
        });

        let planned = Reservation::builder(replacement_key.clone(), first_port)
            .build()
            .unwrap()
            .with_deferred_reserve(DeferredReserve {
                options: ReserveOptions::new(replacement_key.clone(), None)
                    .with_ignore_occupied(true),
                full_config: config,
                occupancy_config: OccupancyCheckConfig::default(),
            });
        let transaction = reserve_db.begin_transaction().unwrap();
        let outcome =
            execute_reserve_after_cleanup_with_barrier(&transaction, &planned, move || {
                start_sender.send(()).unwrap();
                attempt_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("refresh connection did not start");
                let _ = done_receiver.recv_timeout(Duration::from_millis(100));
            })
            .unwrap();
        assert_eq!(outcome.port, first_port);
        transaction.commit().unwrap();
        refresh_thread.join().unwrap().unwrap();

        let replacement = Database::get_reservation(reserve_db.connection(), &replacement_key)
            .unwrap()
            .expect("cleanup retry must persist the replacement");
        assert_eq!(replacement.port(), first_port);

        let refreshed = Database::get_reservation(reserve_db.connection(), &refresh_key)
            .unwrap()
            .expect("the refresh serialized after cleanup/retry must survive");
        assert_eq!(refreshed.port(), second_port);
        assert!(
            refreshed
                .last_used_at()
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(5)),
            "the surviving refresh must be fresh"
        );
        assert!(
            Database::get_reservation(reserve_db.connection(), &other_expired_key)
                .unwrap()
                .is_none(),
            "the unrelated expired candidate should remain removed"
        );
    }

    #[test]
    fn test_plan_path_relationship_denied() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Don't allow unrelated path
        let options = ReserveOptions::new(key, Some(port));

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            Error::PathRelationshipViolation { .. }
        ));
    }

    #[test]
    fn test_plan_path_relationship_with_force() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Force allows unrelated path
        let options = ReserveOptions::new(key, Some(port)).with_force(true);

        let result = ReservePlan::new(options, &config).build_plan(db.connection());

        assert!(result.is_ok());
    }
}
