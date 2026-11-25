//! Reserve operation planning and execution.
//!
//! This module implements the reservation planning logic, including
//! idempotency checks, sticky field protection, and path validation.
//!
//! The decisions are made by the verified kernels `verified::reserve` (the
//! reserve policy, metadata-intent resolution, the existing-reservation
//! decision, deferred reconciliation, and the cleanup-then-retry control)
//! and `verified::allocation` (the preferred-port fallback). This module is
//! the glue: it converts options and stored rows into kernel inputs, and it
//! performs the database, allocator, cleanup, and git effects each decision
//! calls for. See `docs/verification.md`.

use crate::config::{CleanupConfig, Config, ConfigValidator};
use crate::database::Database;
use crate::error::{Error, Result};
use crate::port::allocator::{
    allocator_from_config, AllocationOptions, AllocationResult, PortAllocator,
};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::verified::allocation::{self as allocation_kernel, Attempt, Fallback};
use crate::verified::reserve::{
    self as kernel, AllocationRelaxations, CleanupSwitches, DeferredStep, ExistingDecision, Intent,
    MetadataPermissions, ReserveFlags, ReservePolicy, Retry, StoredRow,
};
use crate::{AutomaticCleanupStatus, Port, PortExhaustionDetails, Reservation, ReservationKey};
use rusqlite::Connection;

use super::cleanup::{AutocleanResult, CleanupOperations};
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

    /// The kernel intent: the same operation, with a `Set` value trimmed.
    ///
    /// Trimming is `str::trim` (trusted). The idempotence lemma of
    /// `verified::reserve` relies on trimming a trimmed value changing
    /// nothing, which `trimmed_intent_values_are_fixed_points` pins.
    fn kernel_intent(&self) -> Intent<&str> {
        match self {
            Self::Preserve => Intent::Preserve,
            Self::Set(value) => Intent::Set(value.trim()),
            Self::Clear => Intent::Clear,
        }
    }

    /// The kernel intent over the UTF-8 bytes of the trimmed value.
    fn kernel_bytes(&self) -> Intent<&[u8]> {
        match self.kernel_intent() {
            Intent::Preserve => Intent::Preserve,
            Intent::Set(value) => Intent::Set(value.as_bytes()),
            Intent::Clear => Intent::Clear,
        }
    }

    /// Resolves the intent against an existing reservation's stored value
    /// (verified `resolve`).
    fn resolve_existing(&self, existing: Option<&str>) -> Option<String> {
        kernel::resolve(&self.kernel_intent(), existing).map(ToOwned::to_owned)
    }

    /// Resolves the intent for a new reservation. `infer` (git discovery)
    /// runs only when the intent can use the inferred value: the verified
    /// `uses_fallback` is `false` exactly for explicit intents, which
    /// resolve alike whatever was inferred
    /// (`lemma_explicit_intent_ignores_fallback`).
    fn resolve_new(&self, infer: impl FnOnce() -> Option<String>) -> Option<String> {
        let intent = self.kernel_intent();
        let inferred = if kernel::uses_fallback(&intent) {
            infer()
        } else {
            None
        };
        kernel::resolve(&intent, inferred.as_deref()).map(ToOwned::to_owned)
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
    /// path only when a new reservation is created, and only for a field the
    /// request leaves unset. Existing reservations keep their stored
    /// metadata, so this method deliberately leaves all intents unchanged.
    /// The `path` argument is retained for source compatibility.
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
    existing_at_planning: bool,
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

    /// The request's flags, field for field, as the kernels take them.
    const fn flags(&self) -> ReserveFlags {
        let options = &self.options;
        ReserveFlags {
            force: options.force,
            overwrite: options.overwrite,
            allow_unrelated_path: options.allow_unrelated_path,
            permissions: MetadataPermissions {
                allow_project_change: options.allow_project_change,
                allow_task_change: options.allow_task_change,
            },
            relaxations: AllocationRelaxations {
                ignore_occupied: options.ignore_occupied,
                ignore_exclusions: options.ignore_exclusions,
            },
            cleanup: CleanupSwitches {
                disable_autoprune: options.disable_autoprune,
                disable_autoexpire: options.disable_autoexpire,
            },
        }
    }

    /// The effective checks of this request (verified `reserve_policy`).
    const fn policy(&self) -> ReservePolicy {
        kernel::reserve_policy(self.flags())
    }

    /// The requested preferred port: an explicit port wins over the
    /// configured hint (verified `choose_preferred`).
    fn preferred_port(&self) -> Option<Port> {
        kernel::choose_preferred(self.options.port, self.options.preferred_port)
    }

    /// Decides what this request does to the stored reservation `existing`
    /// (verified `decide_existing`).
    fn decide_existing(&self, existing: &Reservation) -> ExistingDecision {
        kernel::decide_existing(
            existing.project().map(str::as_bytes),
            existing.task().map(str::as_bytes),
            &self.options.project.kernel_bytes(),
            &self.options.task.kernel_bytes(),
            self.flags(),
        )
    }

    /// Builds an operation plan for this reserve request.
    ///
    /// This method performs all validation and determines what actions
    /// are needed. When automatic allocation finds the range exhausted and
    /// automatic cleanup is enabled, the plan defers allocation to
    /// execution, which cleans up once and retries once.
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
        let plan = OperationPlan::new(format!("Reserve port for {}", self.options.key));

        self.options.project.validate("project")?;
        self.options.task.validate("task")?;

        // Step 1: Validate path relationship
        if self.policy().check_path_relationship {
            Database::validate_path_relationship(&self.options.key.path, false)?;
        }

        // Step 2: Check for existing reservation
        if let Some(existing) = Database::get_reservation(conn, &self.options.key)? {
            return self.plan_existing(conn, plan, &existing);
        }

        // Git inference runs only for a field the request leaves to it.
        let path = &self.options.key.path;
        let project = self
            .options
            .project
            .resolve_new(|| super::inference::infer_project(path));
        let task = self
            .options
            .task
            .resolve_new(|| super::inference::infer_task(path));

        // Step 3: Determine port (unified allocation with fallback)
        match self.allocate_port(conn, None)? {
            Fallback::Allocated(port) => {
                // Step 4: Create the new reservation
                let reservation = Reservation::builder(self.options.key.clone(), port)
                    .project(project)
                    .task(task)
                    .build()?;
                Ok(plan.add_action(PlanAction::CreateReservation(reservation)))
            }
            Fallback::Exhausted => self.plan_cleanup_retry(conn, plan, project, task, None, None),
        }
    }

    /// Plans the request against the reservation already stored under its
    /// key, as the verified `decide_existing` decides. Every sticky-field
    /// change is validated (project before task) before any write is
    /// planned.
    fn plan_existing(
        &self,
        conn: &Connection,
        plan: OperationPlan,
        existing: &Reservation,
    ) -> Result<OperationPlan> {
        let project = self.options.project.resolve_existing(existing.project());
        let task = self.options.task.resolve_existing(existing.task());

        match self.decide_existing(existing) {
            ExistingDecision::StickyProject => {
                Err(sticky_project_error(existing, project.as_deref()))
            }
            ExistingDecision::StickyTask => Err(sticky_task_error(existing, task.as_deref())),
            ExistingDecision::Reallocate => match self
                .allocate_port(conn, Some(&self.options.key))?
            {
                Fallback::Allocated(port) => {
                    let replacement = self.updated_reservation(existing, port, project, task)?;
                    Ok(plan.add_action(PlanAction::UpdateReservation(replacement)))
                }
                Fallback::Exhausted => self.plan_cleanup_retry(
                    conn,
                    plan,
                    project,
                    task,
                    Some(existing.created_at()),
                    Some(&self.options.key),
                ),
            },
            ExistingDecision::UpdateMetadata => {
                let update = self.updated_reservation(existing, existing.port(), project, task)?;
                Ok(plan.add_action(PlanAction::UpdateReservation(update)))
            }
            // A compatible request without overwrite keeps the stable port and
            // only refreshes its last-used timestamp. An explicit preferred
            // port remains a hint and does not relocate an existing key. The
            // plan records the stored port, which is the port it reports.
            ExistingDecision::Touch => {
                Ok(plan.add_refresh(self.options.key.clone(), existing.port()))
            }
        }
    }

    fn allocate_port(
        &self,
        conn: &Connection,
        replaced_key: Option<&ReservationKey>,
    ) -> Result<Fallback<Port>> {
        let allocator = allocator_from_config(self.config)?;
        let occupancy_config = self.occupancy_config();
        let relaxations = self.policy().allocation;
        let attempt = |preferred: Option<Port>| {
            single_attempt(
                &allocator,
                conn,
                preferred,
                relaxations,
                &occupancy_config,
                replaced_key,
            )
        };
        allocation_kernel::allocate_with_fallback(self.preferred_port(), &attempt)
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
        let cleanup = self.policy().cleanup;
        if !cleanup.retry_possible() {
            let allocator = allocator_from_config(self.config)?;
            let occupancy_config = self.occupancy_config();
            let blockers = allocator.exhaustion_blockers(conn, &occupancy_config, ignored_key)?;
            let details = PortExhaustionDetails::new(
                AutomaticCleanupStatus::new(false, cleanup.prune, cleanup.expire, 0, 0),
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
            existing_at_planning: ignored_key.is_some(),
        });

        if ignored_key.is_some() {
            Ok(plan.add_action(PlanAction::UpdateReservation(reservation)))
        } else {
            Ok(plan.add_action(PlanAction::CreateReservation(reservation)))
        }
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
}

/// The error for a project change without permission.
fn sticky_project_error(existing: &Reservation, project: Option<&str>) -> Error {
    Error::StickyFieldChange {
        field: "project".to_string(),
        details: format!(
            "Cannot change project from {:?} to {:?} without --force or --allow-project-change",
            existing.project(),
            project
        ),
    }
}

/// The error for a task change without permission.
fn sticky_task_error(existing: &Reservation, task: Option<&str>) -> Error {
    Error::StickyFieldChange {
        field: "task".to_string(),
        details: format!(
            "Cannot change task from {:?} to {:?} without --force or --allow-task-change",
            existing.task(),
            task
        ),
    }
}

/// One single-port allocation, as the fallback kernel's oracle: the
/// allocator's result with the port kept as a `Port`.
fn single_attempt(
    allocator: &PortAllocator,
    conn: &Connection,
    preferred: Option<Port>,
    relaxations: AllocationRelaxations,
    occupancy_config: &OccupancyCheckConfig,
    replaced_key: Option<&ReservationKey>,
) -> Result<Attempt<Port>> {
    let options = AllocationOptions {
        preferred,
        ignore_occupied: relaxations.ignore_occupied,
        ignore_exclusions: relaxations.ignore_exclusions,
    };
    let result = match replaced_key {
        Some(key) => allocator.allocate_single_replacing(conn, &options, occupancy_config, key),
        None => allocator.allocate_single(conn, &options, occupancy_config),
    }?;
    Ok(match result {
        AllocationResult::Allocated(port) => Attempt::Allocated(port),
        AllocationResult::PreferredUnavailable { .. } => Attempt::PreferredUnavailable,
        AllocationResult::Exhausted { .. } => Attempt::Exhausted,
    })
}

fn reconcile_deferred_key(
    conn: &Connection,
    placeholder: &Reservation,
    deferred: &DeferredReserve,
    reserve_plan: &ReservePlan<'_>,
) -> Result<DeferredKeyReconciliation> {
    let options = &deferred.options;
    let current = Database::get_reservation(conn, &options.key)?;
    let stored = current.as_ref().map(|existing| StoredRow {
        reservation: existing,
        project: existing.project().map(str::as_bytes),
        task: existing.task().map(str::as_bytes),
    });
    let step = kernel::deferred_step(
        stored,
        &options.project.kernel_bytes(),
        &options.task.kernel_bytes(),
        reserve_plan.flags(),
        deferred.existing_at_planning,
    );

    let (existing, decision) = match step {
        DeferredStep::AllocateFresh { reinfer } => {
            let (project, task) = if reinfer {
                let path = &options.key.path;
                (
                    options
                        .project
                        .resolve_new(|| super::inference::infer_project(path)),
                    options
                        .task
                        .resolve_new(|| super::inference::infer_task(path)),
                )
            } else {
                (
                    placeholder.project().map(ToOwned::to_owned),
                    placeholder.task().map(ToOwned::to_owned),
                )
            };
            return Ok(DeferredKeyReconciliation::Allocate(
                PendingDeferredAllocation {
                    project,
                    task,
                    created_at: None,
                    ignored_key: None,
                },
            ));
        }
        DeferredStep::Existing(existing, decision) => (existing, decision),
    };

    let project = options.project.resolve_existing(existing.project());
    let task = options.task.resolve_existing(existing.task());
    match decision {
        ExistingDecision::StickyProject => Err(sticky_project_error(existing, project.as_deref())),
        ExistingDecision::StickyTask => Err(sticky_task_error(existing, task.as_deref())),
        ExistingDecision::Reallocate => Ok(DeferredKeyReconciliation::Allocate(
            PendingDeferredAllocation {
                project,
                task,
                created_at: Some(existing.created_at()),
                ignored_key: Some(options.key.clone()),
            },
        )),
        ExistingDecision::UpdateMetadata => {
            let update =
                reserve_plan.updated_reservation(existing, existing.port(), project, task)?;
            Database::create_reservation_simple(conn, &update)?;
            Ok(DeferredKeyReconciliation::Complete(
                AutomaticReserveOutcome {
                    port: existing.port(),
                    warning: None,
                },
            ))
        }
        ExistingDecision::Touch => {
            Database::update_last_used_simple(conn, &options.key)?;
            Ok(DeferredKeyReconciliation::Complete(
                AutomaticReserveOutcome {
                    port: existing.port(),
                    warning: None,
                },
            ))
        }
    }
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
        let policy = reserve_plan.policy();
        let preferred = reserve_plan.preferred_port();
        let ignored_key = pending.ignored_key.as_ref();
        let attempt = |preferred: Option<Port>| {
            single_attempt(
                &allocator,
                conn,
                preferred,
                policy.allocation,
                occupancy_config,
                ignored_key,
            )
        };
        let allocate =
            |_after_cleanup: bool| allocation_kernel::allocate_with_fallback(preferred, &attempt);
        let cleanup = || {
            let default_cleanup = CleanupConfig {
                expire_after_days: Some(30),
            };
            let cleanup_config = full_config.cleanup.as_ref().unwrap_or(&default_cleanup);
            CleanupOperations::automatic_cleanup_in_connection(
                conn,
                cleanup_config,
                policy.cleanup.prune,
                policy.cleanup.expire,
                after_candidate_discovery,
            )
        };

        // The verified kernel runs the first allocation, and only after an
        // exhausted one with cleanup enabled the single cleanup and the
        // single retry.
        match kernel::allocate_with_cleanup_retry(
            policy.cleanup.retry_possible(),
            &allocate,
            cleanup,
        )? {
            Retry::Allocated {
                port,
                cleanup: None,
            } => persist_deferred_outcome(conn, &options.key, &pending, port, None),
            Retry::Allocated {
                port,
                cleanup: Some(cleanup),
            } => persist_deferred_outcome(
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
            Retry::Exhausted { cleanup } => Err(port_exhausted_error(
                &allocator,
                conn,
                occupancy_config,
                ignored_key,
                cleanup_status(policy, cleanup.as_ref()),
            )?),
        }
    };

    Database::with_immediate_transaction_or_savepoint(
        conn,
        "trop_reserve_after_cleanup",
        "automatic cleanup and reserve retry",
        execute,
    )
}

/// The typed cleanup diagnostics of a terminal exhaustion: attempted exactly
/// when a cleanup ran, with its counts (zero when it did not run).
fn cleanup_status(
    policy: ReservePolicy,
    cleanup: Option<&AutocleanResult>,
) -> AutomaticCleanupStatus {
    match cleanup {
        Some(cleanup) => AutomaticCleanupStatus::new(
            true,
            policy.cleanup.prune,
            policy.cleanup.expire,
            cleanup.pruned_count,
            cleanup.expired_count,
        ),
        None => {
            AutomaticCleanupStatus::new(false, policy.cleanup.prune, policy.cleanup.expire, 0, 0)
        }
    }
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

/// The sticky-field rule (a delegate to the verified
/// `verified::sticky::can_change_field`, which `decide_existing` applies to
/// each field); kept for the property tests below.
#[cfg(all(test, feature = "property-tests"))]
fn can_change_field(
    new_value: Option<&str>,
    existing_value: Option<&str>,
    force: bool,
    allow_change: bool,
) -> bool {
    crate::verified::sticky::can_change_field(
        new_value.map(str::as_bytes),
        existing_value.map(str::as_bytes),
        force,
        allow_change,
    )
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
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );

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
    fn test_deferred_overwrite_reinfers_metadata_when_target_disappears() {
        let directory = tempdir().unwrap();
        let blocker_path = directory.path().join("blocker");
        let target_path = directory.path().join("target");
        std::fs::create_dir(&blocker_path).unwrap();
        std::fs::create_dir(&target_path).unwrap();
        let blocker_key = ReservationKey::new(blocker_path, None).unwrap();
        let target_key = ReservationKey::new(target_path, None).unwrap();
        let available_port = Port::try_from(5452).unwrap();
        let old_target_port = Port::try_from(5453).unwrap();
        let mut db = create_test_database();

        db.create_reservation(
            &Reservation::builder(blocker_key.clone(), available_port)
                .build()
                .unwrap(),
        )
        .unwrap();
        db.create_reservation(
            &Reservation::builder(target_key.clone(), old_target_port)
                .project(Some("deleted-project".into()))
                .task(Some("deleted-task".into()))
                .build()
                .unwrap(),
        )
        .unwrap();

        let config = Config {
            ports: Some(PortConfig {
                min: available_port.value(),
                max: Some(available_port.value()),
                max_offset: None,
            }),
            ..Default::default()
        };
        let plan = ReservePlan::new(
            ReserveOptions::new(target_key.clone(), None)
                .with_overwrite(true)
                .with_allow_unrelated_path(true),
            &config,
        )
        .build_plan(db.connection())
        .unwrap();
        assert!(matches!(
            plan.actions.as_slice(),
            [PlanAction::UpdateReservation(reservation)]
                if reservation.requires_allocation_at_execution()
        ));

        Database::delete_reservation_simple(db.connection(), &target_key).unwrap();
        Database::delete_reservation_simple(db.connection(), &blocker_key).unwrap();

        let result = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(result.port, Some(available_port));
        let stored = Database::get_reservation(db.connection(), &target_key)
            .unwrap()
            .expect("the disappeared target should be recreated");
        assert_eq!(stored.project(), None);
        assert_eq!(stored.task(), None);
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
                existing_at_planning: false,
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

    // ---------------------------------------------------------------------
    // Glue of the verified `verified::reserve` kernel
    // ---------------------------------------------------------------------

    use crate::config::OccupancyConfig;
    use crate::verified::reserve::{allocate_with_cleanup_retry, reserve_policy};
    use std::cell::Cell;

    /// The decision table as it was hand-written before it moved into the
    /// verified kernel (`build_plan`'s existing-key branch). `flags` holds
    /// force, overwrite, allow-project-change, and allow-task-change.
    fn legacy_decision(
        stored_project: Option<&str>,
        stored_task: Option<&str>,
        project: &MetadataIntent,
        task: &MetadataIntent,
        (force, overwrite, allow_project, allow_task): (bool, bool, bool, bool),
    ) -> (ExistingDecision, Option<String>, Option<String>) {
        fn resolve(intent: &MetadataIntent, existing: Option<&str>) -> Option<String> {
            match intent {
                MetadataIntent::Preserve => existing.map(ToOwned::to_owned),
                MetadataIntent::Set(value) => Some(value.trim().to_string()),
                MetadataIntent::Clear => None,
            }
        }
        fn can_change(new: Option<&str>, old: Option<&str>, force: bool, allow: bool) -> bool {
            if force || allow {
                return true;
            }
            match (new, old) {
                (None, None) => true,
                (Some(new), Some(old)) => new == old,
                _ => false,
            }
        }
        let p = resolve(project, stored_project);
        let t = resolve(task, stored_task);
        let decision = if !can_change(p.as_deref(), stored_project, force, allow_project) {
            ExistingDecision::StickyProject
        } else if !can_change(t.as_deref(), stored_task, force, allow_task) {
            ExistingDecision::StickyTask
        } else if overwrite || force {
            ExistingDecision::Reallocate
        } else if p.as_deref() != stored_project || t.as_deref() != stored_task {
            ExistingDecision::UpdateMetadata
        } else {
            ExistingDecision::Touch
        };
        (decision, p, t)
    }

    fn stored_values() -> Vec<Option<&'static str>> {
        vec![None, Some("alpha"), Some("beta"), Some("ünïcode")]
    }

    fn intents() -> Vec<MetadataIntent> {
        vec![
            MetadataIntent::Preserve,
            MetadataIntent::Clear,
            MetadataIntent::set("alpha"),
            MetadataIntent::set("  alpha\t"),
            MetadataIntent::set("beta"),
            MetadataIntent::set("\u{3000}ünïcode\u{a0}"),
        ]
    }

    /// Differential test: the kernel decision and the glue's resolved
    /// metadata agree with the previous hand-written rules on every
    /// combination of stored values, intents (including values that need
    /// trimming and non-ASCII values), and the four deciding flags.
    #[test]
    fn existing_decision_matches_the_previous_rules() {
        let key = ReservationKey::new(PathBuf::from("/test/differential"), None).unwrap();
        let config = create_test_config();
        let mut cases = 0;
        for stored_project in stored_values() {
            for stored_task in stored_values() {
                let existing = Reservation::builder(key.clone(), Port::try_from(5000).unwrap())
                    .project(stored_project.map(ToOwned::to_owned))
                    .task(stored_task.map(ToOwned::to_owned))
                    .build()
                    .unwrap();
                for project in intents() {
                    for task in intents() {
                        for bits in 0u8..16 {
                            let (force, overwrite, allow_project, allow_task) =
                                (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
                            let options = ReserveOptions::new(key.clone(), None)
                                .with_project_intent(project.clone())
                                .with_task_intent(task.clone())
                                .with_force(force)
                                .with_overwrite(overwrite)
                                .with_allow_project_change(allow_project)
                                .with_allow_task_change(allow_task);
                            let planner = ReservePlan::new(options, &config);
                            let expected = legacy_decision(
                                stored_project,
                                stored_task,
                                &project,
                                &task,
                                (force, overwrite, allow_project, allow_task),
                            );
                            let actual = (
                                planner.decide_existing(&existing),
                                project.resolve_existing(existing.project()),
                                task.resolve_existing(existing.task()),
                            );
                            assert_eq!(
                                actual, expected,
                                "stored {stored_project:?}/{stored_task:?}, \
                                 intents {project:?}/{task:?}, flags {bits:04b}"
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 4 * 4 * 6 * 6 * 16);
    }

    /// Pins the trusted `str::trim` fact behind the kernel's idempotence
    /// lemma (`lemma_repeat_request_touches`): an explicit value is stored
    /// trimmed, and trimming it again (the reservation builder trims)
    /// changes nothing, so the stored value equals the requested one.
    #[test]
    fn trimmed_intent_values_are_fixed_points() {
        let key = ReservationKey::new(PathBuf::from("/test/trim"), None).unwrap();
        for raw in [
            "plain",
            "  padded  ",
            "\t\ttabs\n",
            "\u{3000}ideographic\u{2003}",
            "\u{a0}nbsp\u{a0}",
            "inner  space",
            "ünïcode",
        ] {
            let intent = MetadataIntent::set(raw);
            let Intent::Set(trimmed) = intent.kernel_intent() else {
                panic!("a set intent must stay a set intent");
            };
            assert_eq!(trimmed.trim(), trimmed);
            let stored = Reservation::builder(key.clone(), Port::try_from(5000).unwrap())
                .project(Some(trimmed.to_string()))
                .build()
                .unwrap();
            assert_eq!(stored.project(), Some(trimmed));
            assert_eq!(
                intent.resolve_existing(stored.project()).as_deref(),
                Some(trimmed)
            );
        }
    }

    /// Regression: explicit metadata used to trigger git discovery anyway.
    /// Inference now runs only for a field the request leaves to it.
    #[test]
    fn explicit_metadata_skips_git_inference() {
        let discoveries = || super::super::inference::DISCOVERIES.with(Cell::get);
        let config = Config {
            occupancy_check: Some(OccupancyConfig {
                skip: Some(true),
                ..OccupancyConfig::default()
            }),
            ..create_test_config()
        };
        let directory = tempdir().unwrap();
        let cases = [
            (MetadataIntent::set("p"), MetadataIntent::set("t"), 0),
            (MetadataIntent::Clear, MetadataIntent::Clear, 0),
            (MetadataIntent::set("p"), MetadataIntent::Preserve, 1),
            (MetadataIntent::Preserve, MetadataIntent::Clear, 1),
            (MetadataIntent::Preserve, MetadataIntent::Preserve, 2),
        ];
        for (index, (project, task, expected)) in cases.into_iter().enumerate() {
            let db = create_test_database();
            let path = directory.path().join(format!("case-{index}"));
            std::fs::create_dir(&path).unwrap();
            let key = ReservationKey::new(path, None).unwrap();
            let options = ReserveOptions::new(key, None)
                .with_project_intent(project.clone())
                .with_task_intent(task.clone())
                .with_allow_unrelated_path(true);
            let before = discoveries();
            let plan = ReservePlan::new(options, &config)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(
                discoveries() - before,
                expected,
                "intents {project:?}/{task:?}"
            );
            let [PlanAction::CreateReservation(reservation)] = plan.actions.as_slice() else {
                panic!("expected one creation, got {:?}", plan.actions);
            };
            let explicit = |intent: &MetadataIntent| match intent {
                MetadataIntent::Set(value) => Some(value.clone()),
                _ => None,
            };
            assert_eq!(
                reservation.project().map(ToOwned::to_owned),
                explicit(&project)
            );
            assert_eq!(reservation.task().map(ToOwned::to_owned), explicit(&task));
        }
    }

    #[test]
    fn explicit_intents_never_evaluate_inference() {
        for intent in [MetadataIntent::set(" value "), MetadataIntent::Clear] {
            let resolved = intent.resolve_new(|| panic!("inference must not run"));
            let expected = match &intent {
                MetadataIntent::Set(_) => Some("value".to_string()),
                _ => None,
            };
            assert_eq!(resolved, expected);
        }
        assert_eq!(
            MetadataIntent::Preserve.resolve_new(|| Some("inferred".into())),
            Some("inferred".to_string())
        );
    }

    #[test]
    fn reserve_policy_matches_the_flag_semantics() {
        for bits in 0u16..512 {
            let bit = |n: u16| bits & (1 << n) != 0;
            let key = ReservationKey::new(PathBuf::from("/test/policy"), None).unwrap();
            let options = ReserveOptions::new(key, None)
                .with_force(bit(0))
                .with_overwrite(bit(1))
                .with_allow_unrelated_path(bit(2))
                .with_allow_project_change(bit(3))
                .with_allow_task_change(bit(4))
                .with_ignore_occupied(bit(5))
                .with_ignore_exclusions(bit(6))
                .with_disable_autoprune(bit(7))
                .with_disable_autoexpire(bit(8));
            let config = create_test_config();
            let planner = ReservePlan::new(options.clone(), &config);
            let policy = reserve_policy(planner.flags());
            assert_eq!(
                policy.check_path_relationship,
                !options.force && !options.allow_unrelated_path
            );
            assert_eq!(
                policy.reallocate_existing,
                options.overwrite || options.force
            );
            assert_eq!(
                policy.allocation.ignore_occupied,
                options.ignore_occupied || options.force
            );
            assert_eq!(
                policy.allocation.ignore_exclusions,
                options.ignore_exclusions || options.force
            );
            assert_eq!(policy.cleanup.prune, !options.disable_autoprune);
            assert_eq!(policy.cleanup.expire, !options.disable_autoexpire);
            assert_eq!(
                policy.cleanup.retry_possible(),
                !options.disable_autoprune || !options.disable_autoexpire
            );
        }
    }

    #[test]
    fn preferred_port_prefers_the_explicit_port() {
        let key = ReservationKey::new(PathBuf::from("/test/preferred"), None).unwrap();
        let config = create_test_config();
        let explicit = Port::try_from(6000).unwrap();
        let hint = Port::try_from(6001).unwrap();
        for (port, preferred, expected) in [
            (None, None, None),
            (Some(explicit), None, Some(explicit)),
            (None, Some(hint), Some(hint)),
            (Some(explicit), Some(hint), Some(explicit)),
        ] {
            let options = ReserveOptions::new(key.clone(), port).with_preferred_port(preferred);
            assert_eq!(
                ReservePlan::new(options, &config).preferred_port(),
                expected
            );
        }
    }

    /// The fallback kernel over a scripted allocator: at most two requests,
    /// the second without a preference and only after the preference was
    /// reported unavailable; a contract violation is exhaustion, not a panic.
    #[test]
    fn fallback_asks_again_only_after_an_unavailable_preference() {
        let preferred = Some(Port::try_from(6000).unwrap());
        let scanned = Port::try_from(5000).unwrap();
        let script = |first: Attempt<Port>, second: Attempt<Port>| {
            let calls = Cell::new(Vec::new());
            let outcome = {
                let attempt = |requested: Option<Port>| -> Result<Attempt<Port>> {
                    let mut seen = calls.take();
                    seen.push(requested);
                    let answer = if seen.len() == 1 { &first } else { &second };
                    let answer = match answer {
                        Attempt::Allocated(port) => Attempt::Allocated(*port),
                        Attempt::PreferredUnavailable => Attempt::PreferredUnavailable,
                        Attempt::Exhausted => Attempt::Exhausted,
                    };
                    calls.set(seen);
                    Ok(answer)
                };
                allocation_kernel::allocate_with_fallback(preferred, &attempt).unwrap()
            };
            (outcome, calls.take())
        };

        let (outcome, calls) = script(Attempt::Allocated(scanned), Attempt::Exhausted);
        assert!(matches!(outcome, Fallback::Allocated(port) if port == scanned));
        assert_eq!(calls, vec![preferred]);

        let (outcome, calls) = script(Attempt::PreferredUnavailable, Attempt::Allocated(scanned));
        assert!(matches!(outcome, Fallback::Allocated(port) if port == scanned));
        assert_eq!(calls, vec![preferred, None]);

        let (outcome, calls) = script(Attempt::PreferredUnavailable, Attempt::Exhausted);
        assert!(matches!(outcome, Fallback::Exhausted));
        assert_eq!(calls, vec![preferred, None]);

        // The allocator contract forbids this; it is reported as exhaustion.
        let (outcome, calls) = script(Attempt::PreferredUnavailable, Attempt::PreferredUnavailable);
        assert!(matches!(outcome, Fallback::Exhausted));
        assert_eq!(calls, vec![preferred, None]);
    }

    /// The retry kernel over scripted oracles: cleanup runs at most once,
    /// only after an exhausted first allocation and only when possible;
    /// allocation runs at most twice.
    #[test]
    fn cleanup_retry_runs_cleanup_at_most_once() {
        let port = Port::try_from(5000).unwrap();
        let run = |possible: bool, first: Fallback<Port>, second: Fallback<Port>| {
            let allocations = Cell::new(Vec::new());
            let cleanups = Cell::new(0);
            let outcome = {
                let allocate = |after_cleanup: bool| -> Result<Fallback<Port>> {
                    let mut seen = allocations.take();
                    seen.push(after_cleanup);
                    let answer = if seen.len() == 1 { &first } else { &second };
                    let answer = match answer {
                        Fallback::Allocated(port) => Fallback::Allocated(*port),
                        Fallback::Exhausted => Fallback::Exhausted,
                    };
                    allocations.set(seen);
                    Ok(answer)
                };
                let cleanup = || -> Result<(usize, usize)> {
                    cleanups.set(cleanups.get() + 1);
                    Ok((1, 2))
                };
                allocate_with_cleanup_retry(possible, &allocate, cleanup).unwrap()
            };
            (outcome, allocations.take(), cleanups.get())
        };

        for possible in [false, true] {
            let (outcome, allocations, cleanups) =
                run(possible, Fallback::Allocated(port), Fallback::Exhausted);
            assert!(matches!(outcome, Retry::Allocated { cleanup: None, .. }));
            assert_eq!((allocations, cleanups), (vec![false], 0));
        }

        let (outcome, allocations, cleanups) =
            run(false, Fallback::Exhausted, Fallback::Allocated(port));
        assert!(matches!(outcome, Retry::Exhausted { cleanup: None }));
        assert_eq!((allocations, cleanups), (vec![false], 0));

        let (outcome, allocations, cleanups) =
            run(true, Fallback::Exhausted, Fallback::Allocated(port));
        assert!(matches!(
            outcome,
            Retry::Allocated { port: allocated, cleanup: Some((1, 2)) } if allocated == port
        ));
        assert_eq!((allocations, cleanups), (vec![false, true], 1));

        let (outcome, allocations, cleanups) = run(true, Fallback::Exhausted, Fallback::Exhausted);
        assert!(matches!(
            outcome,
            Retry::Exhausted {
                cleanup: Some((1, 2))
            }
        ));
        assert_eq!((allocations, cleanups), (vec![false, true], 1));
    }

    #[test]
    fn cleanup_status_reports_counts_only_after_a_cleanup() {
        let key = ReservationKey::new(PathBuf::from("/test/status"), None).unwrap();
        let config = create_test_config();
        let planner = ReservePlan::new(
            ReserveOptions::new(key, None).with_disable_autoexpire(true),
            &config,
        );
        let policy = planner.policy();
        let skipped = cleanup_status(policy, None);
        assert!(!skipped.attempted());
        assert!(skipped.prune_enabled());
        assert!(!skipped.expire_enabled());
        assert_eq!((skipped.pruned(), skipped.expired()), (0, 0));

        let cleanup = AutocleanResult {
            considered_count: 4,
            preserved_count: 1,
            pruned_count: 2,
            expired_count: 1,
            total_removed: 3,
            considered_reservations: Vec::new(),
            preserved_reservations: Vec::new(),
            removed_reservations: Vec::new(),
            pruned_reservations: Vec::new(),
            expired_reservations: Vec::new(),
            prune_path_decisions: Vec::new(),
        };
        let ran = cleanup_status(policy, Some(&cleanup));
        assert!(ran.attempted());
        assert_eq!((ran.pruned(), ran.expired()), (2, 1));
    }
}
