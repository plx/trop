//! Reserve operation planning and execution.
//!
//! This module implements the reservation planning logic, including
//! idempotency checks, sticky field protection, and path validation.

use crate::config::Config;
use crate::database::Database;
use crate::error::{Error, Result};
use crate::port::allocator::{allocator_from_config, AllocationOptions, AllocationResult};
use crate::port::occupancy::OccupancyCheckConfig;
use crate::{Port, Reservation, ReservationKey};

use super::plan::{OperationPlan, PlanAction};

/// Options for a reserve operation.
///
/// This struct contains all the parameters needed to plan a reserve operation,
/// including validation flags and metadata.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveOptions {
    /// The reservation key (path + optional tag).
    pub key: ReservationKey,

    /// Optional project identifier (sticky field).
    pub project: Option<String>,

    /// Optional task identifier (sticky field).
    pub task: Option<String>,

    /// The port to reserve. If None, automatic allocation will be used.
    pub port: Option<Port>,

    /// Preferred port for automatic allocation (hint).
    pub preferred_port: Option<Port>,

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
}

impl ReserveOptions {
    /// Creates a new `ReserveOptions` with the given key and port.
    ///
    /// All optional fields and flags are set to defaults:
    /// - project: None
    /// - task: None
    /// - `preferred_port`: None
    /// - `ignore_occupied`: false
    /// - `ignore_exclusions`: false
    /// - force: false
    /// - `allow_unrelated_path`: false
    /// - `allow_project_change`: false
    /// - `allow_task_change`: false
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
            project: None,
            task: None,
            port,
            preferred_port: None,
            ignore_occupied: false,
            ignore_exclusions: false,
            force: false,
            allow_unrelated_path: false,
            allow_project_change: false,
            allow_task_change: false,
        }
    }

    /// Sets the project field.
    #[must_use]
    pub fn with_project(mut self, project: Option<String>) -> Self {
        self.project = project;
        self
    }

    /// Sets the task field.
    #[must_use]
    pub fn with_task(mut self, task: Option<String>) -> Self {
        self.task = task;
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
}

/// A reservation plan generator.
///
/// This struct is responsible for analyzing a reserve request and
/// generating a plan that describes what actions to take.
pub struct ReservePlan<'a> {
    options: ReserveOptions,
    config: &'a Config,
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
    /// are needed. It does NOT modify the database.
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
    /// let db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let options = ReserveOptions::new(key, Some(port))
    ///     .with_allow_unrelated_path(true);
    ///
    /// let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();
    /// ```
    pub fn build_plan(&self, db: &Database) -> Result<OperationPlan> {
        let mut plan = OperationPlan::new(format!("Reserve port for {}", self.options.key));

        // Step 1: Validate path relationship
        if !self.options.force && !self.options.allow_unrelated_path {
            db.validate_path_relationship(&self.options.key.path, false)?;
        }

        // Step 2: Check for existing reservation
        if let Some(existing) = db.get_reservation(&self.options.key)? {
            // Reservation exists - validate sticky fields and return idempotent result
            self.validate_sticky_fields(&existing)?;

            // Idempotent case: reservation exists with compatible metadata
            // Just update the timestamp
            plan = plan.add_action(PlanAction::UpdateLastUsed(self.options.key.clone()));
            return Ok(plan);
        }

        // Step 3: Determine port (unified allocation with fallback)
        // All ports (whether from --port or automatic) go through the allocator.
        // If a preferred port is specified but unavailable, we fall back to
        // automatic scanning. This implements the "try preferred first, then scan"
        // algorithm described in the specification.
        let port = {
            let allocator = allocator_from_config(self.config)?;

            let allocation_options = AllocationOptions {
                // Merge explicit --port and --preferred-port options
                preferred: self.options.port.or(self.options.preferred_port),
                ignore_occupied: self.options.ignore_occupied,
                ignore_exclusions: self.options.ignore_exclusions,
            };

            let occupancy_config = self.occupancy_config();

            match allocator.allocate_single(db, &allocation_options, &occupancy_config)? {
                AllocationResult::Allocated(port) => port,

                AllocationResult::PreferredUnavailable { .. } => {
                    // Preferred port unavailable - fall back to automatic scanning
                    // This implements the specification's "preferentially reserved if available"
                    // behavior: try the preferred port first, but scan if unavailable.

                    let fallback_options = AllocationOptions {
                        preferred: None, // No preference for fallback scan
                        ignore_occupied: self.options.ignore_occupied,
                        ignore_exclusions: self.options.ignore_exclusions,
                    };

                    match allocator.allocate_single(db, &fallback_options, &occupancy_config)? {
                        AllocationResult::Allocated(fallback_port) => fallback_port,
                        AllocationResult::Exhausted { tried_cleanup } => {
                            return Err(Error::PortExhausted {
                                range: *allocator.range(),
                                tried_cleanup,
                            });
                        }
                        // Should not get PreferredUnavailable without a preferred port
                        AllocationResult::PreferredUnavailable { .. } => unreachable!(),
                    }
                }

                AllocationResult::Exhausted { tried_cleanup } => {
                    return Err(Error::PortExhausted {
                        range: *allocator.range(),
                        tried_cleanup,
                    });
                }
            }
        };

        // Step 4: Create the new reservation
        let reservation = Reservation::builder(self.options.key.clone(), port)
            .project(self.options.project.clone())
            .task(self.options.task.clone())
            .build()?;

        plan = plan.add_action(PlanAction::CreateReservation(reservation));

        Ok(plan)
    }

    /// Validates that sticky fields aren't being changed without permission.
    fn validate_sticky_fields(&self, existing: &Reservation) -> Result<()> {
        // Check project field
        if !self.can_change_project(existing) {
            return Err(Error::StickyFieldChange {
                field: "project".to_string(),
                details: format!(
                    "Cannot change project from {:?} to {:?} without --force or --allow-project-change",
                    existing.project(),
                    self.options.project
                ),
            });
        }

        // Check task field
        if !self.can_change_task(existing) {
            return Err(Error::StickyFieldChange {
                field: "task".to_string(),
                details: format!(
                    "Cannot change task from {:?} to {:?} without --force or --allow-task-change",
                    existing.task(),
                    self.options.task
                ),
            });
        }

        Ok(())
    }

    /// Checks if the project field can be changed.
    fn can_change_project(&self, existing: &Reservation) -> bool {
        can_change_field(
            self.options.project.as_ref(),
            existing.project(),
            self.options.force,
            self.options.allow_project_change,
        )
    }

    /// Checks if the task field can be changed.
    fn can_change_task(&self, existing: &Reservation) -> bool {
        can_change_field(
            self.options.task.as_ref(),
            existing.task(),
            self.options.force,
            self.options.allow_task_change,
        )
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
    new_value: Option<&String>,
    existing_value: Option<&str>,
    force: bool,
    allow_change: bool,
) -> bool {
    // If force or specific allow flag is set, allow change
    if force || allow_change {
        return true;
    }

    // Otherwise, only allow if the value isn't actually changing
    match (new_value.map(String::as_str), existing_value) {
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
    use std::path::PathBuf;

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
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_allow_project_change(allow_project_change)
                    .with_allow_task_change(allow_task_change);

                let opts2 = ReserveOptions::new(key.clone(), Some(port))
                    .with_project(project.clone())
                    .with_task(task.clone())
                    .with_force(force)
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_allow_project_change(allow_project_change)
                    .with_allow_task_change(allow_task_change);

                // Compare all fields
                prop_assert_eq!(opts1.port, opts2.port);
                prop_assert_eq!(opts1.project, opts2.project);
                prop_assert_eq!(opts1.task, opts2.task);
                prop_assert_eq!(opts1.force, opts2.force);
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
                    new_value.as_ref(),
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
                    new_value.as_ref(),
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
                    value.as_ref(),
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

                let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();

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
                let result_without = ReservePlan::new(options_without_force, &config).build_plan(&db);
                prop_assert!(result_without.is_err(), "unrelated path must fail without force");

                // With force, should succeed
                let result_with = ReservePlan::new(options_with_force, &config).build_plan(&db);
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

                let result = ReservePlan::new(options, &config).build_plan(&db);
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

        assert!(!options.force);
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
            .with_allow_unrelated_path(true);

        assert!(options.force);
        assert!(options.allow_unrelated_path);
        assert_eq!(options.project, Some("test-project".to_string()));
        assert_eq!(options.task, Some("test-task".to_string()));
    }

    #[test]
    fn test_plan_new_reservation() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        let options = ReserveOptions::new(key, Some(port)).with_allow_unrelated_path(true);

        let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();

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

        let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();

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

        let result = ReservePlan::new(options, &config).build_plan(&db);

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

        let result = ReservePlan::new(options, &config).build_plan(&db);

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

        let result = ReservePlan::new(options, &config).build_plan(&db);

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

        let result = ReservePlan::new(options, &config).build_plan(&db);

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

        let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();

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
        let options = ReserveOptions::new(key3, None).with_allow_unrelated_path(true);

        let result = ReservePlan::new(options, &config).build_plan(&db);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::PortExhausted { .. }));
    }

    #[test]
    fn test_plan_path_relationship_denied() {
        let db = create_test_database();
        let config = create_test_config();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Don't allow unrelated path
        let options = ReserveOptions::new(key, Some(port));

        let result = ReservePlan::new(options, &config).build_plan(&db);

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

        let result = ReservePlan::new(options, &config).build_plan(&db);

        assert!(result.is_ok());
    }
}
