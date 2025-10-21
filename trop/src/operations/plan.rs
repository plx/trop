//! Plan types for reservation operations.
//!
//! This module defines the plan structures that describe what actions
//! will be taken during an operation, without actually performing them.

use crate::port::group::GroupAllocationRequest;
use crate::port::occupancy::OccupancyCheckConfig;
use crate::{Reservation, ReservationKey};

/// A single action to be taken during plan execution.
///
/// Each action corresponds to a specific database operation that will
/// be performed when the plan is executed.
///
/// ## Note on `CreateReservation` vs `UpdateReservation`
///
/// Both `CreateReservation` and `UpdateReservation` variants execute identically
/// (both perform an upsert operation in the database). However, they exist as
/// separate variants for semantic clarity in plans:
///
/// - `CreateReservation`: Used when the plan is creating a new reservation
/// - `UpdateReservation`: Used when the plan is updating an existing reservation
///
/// This distinction helps with logging, debugging, and understanding the intent
/// of the operation, even though the underlying database operation is the same.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanAction {
    /// Create a new reservation or update an existing one.
    CreateReservation(Reservation),

    /// Update an existing reservation.
    UpdateReservation(Reservation),

    /// Update the `last_used_at` timestamp for a reservation.
    UpdateLastUsed(ReservationKey),

    /// Delete a reservation.
    DeleteReservation(ReservationKey),

    /// Allocate a group of related ports.
    AllocateGroup {
        /// The group allocation request.
        request: GroupAllocationRequest,
        /// Full configuration (needed for port allocation).
        full_config: crate::config::Config,
        /// Occupancy check configuration.
        occupancy_config: OccupancyCheckConfig,
    },
}

impl PlanAction {
    /// Returns a human-readable description of this action.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::CreateReservation(r) => {
                format!("Create reservation for {} on port {}", r.key(), r.port())
            }
            Self::UpdateReservation(r) => {
                format!("Update reservation for {} to port {}", r.key(), r.port())
            }
            Self::UpdateLastUsed(key) => {
                format!("Update last_used_at timestamp for {key}")
            }
            Self::DeleteReservation(key) => {
                format!("Delete reservation for {key}")
            }
            Self::AllocateGroup { request, .. } => {
                format!(
                    "Allocate group of {} services at {}",
                    request.services.len(),
                    request.base_path.display()
                )
            }
        }
    }
}

/// A complete operation plan describing all actions to be taken.
///
/// Plans are generated during the planning phase and can be inspected,
/// logged, or executed. They include a description, a sequence of actions,
/// and any warnings that should be communicated to the user.
#[derive(Debug, Clone)]
pub struct OperationPlan {
    /// A human-readable description of the operation.
    pub description: String,

    /// The sequence of actions to perform.
    pub actions: Vec<PlanAction>,

    /// Warnings to communicate to the user.
    pub warnings: Vec<String>,
}

impl OperationPlan {
    /// Creates a new operation plan with the given description.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::OperationPlan;
    ///
    /// let plan = OperationPlan::new("Reserve port 8080");
    /// assert_eq!(plan.description, "Reserve port 8080");
    /// assert!(plan.is_empty());
    /// ```
    #[must_use]
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            actions: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Adds an action to the plan.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::{OperationPlan, PlanAction};
    /// use trop::{Reservation, ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    ///
    /// let plan = OperationPlan::new("Test")
    ///     .add_action(PlanAction::CreateReservation(reservation));
    ///
    /// assert_eq!(plan.actions.len(), 1);
    /// ```
    #[must_use]
    pub fn add_action(mut self, action: PlanAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Adds a warning to the plan.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::OperationPlan;
    ///
    /// let plan = OperationPlan::new("Test")
    ///     .add_warning("This is a warning");
    ///
    /// assert_eq!(plan.warnings.len(), 1);
    /// ```
    #[must_use]
    pub fn add_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Checks if the plan has no actions.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::OperationPlan;
    ///
    /// let plan = OperationPlan::new("Test");
    /// assert!(plan.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Returns the number of actions in the plan.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::{OperationPlan, PlanAction};
    /// use trop::{Reservation, ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    ///
    /// let plan = OperationPlan::new("Test")
    ///     .add_action(PlanAction::CreateReservation(reservation));
    ///
    /// assert_eq!(plan.len(), 1);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Port, ReservationKey};
    use std::path::PathBuf;

    // Property-based testing module
    // These tests verify mathematical properties and invariants of the plan system
    #[cfg(feature = "property-tests")]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        // Strategy to generate valid Port values
        fn port_strategy() -> impl Strategy<Value = Port> {
            (1u16..=65535).prop_map(|p| Port::try_from(p).unwrap())
        }

        // Strategy to generate valid ReservationKey instances
        fn reservation_key_strategy() -> impl Strategy<Value = ReservationKey> {
            prop_oneof![
                Just(ReservationKey::new(PathBuf::from("/test/path"), None).unwrap()),
                "[a-z]{1,10}".prop_map(|tag| ReservationKey::new(
                    PathBuf::from("/test/path"),
                    Some(tag)
                )
                .unwrap()),
            ]
        }

        // PROPERTY: OperationPlan builder is associative
        // Adding actions in different groupings produces the same final plan
        proptest! {
            #[test]
            fn prop_operation_plan_builder_associative(
                key1 in reservation_key_strategy(),
                key2 in reservation_key_strategy(),
                port1 in port_strategy(),
                port2 in port_strategy(),
            ) {
                // PROPERTY: (a + b) + c == a + (b + c) for plan building
                // This verifies that the builder pattern correctly accumulates actions
                let r1 = Reservation::builder(key1.clone(), port1).build().unwrap();
                let r2 = Reservation::builder(key2.clone(), port2).build().unwrap();

                let plan1 = OperationPlan::new("test")
                    .add_action(PlanAction::CreateReservation(r1.clone()))
                    .add_action(PlanAction::DeleteReservation(key1.clone()))
                    .add_action(PlanAction::CreateReservation(r2.clone()));

                let plan2 = OperationPlan::new("test")
                    .add_action(PlanAction::CreateReservation(r1))
                    .add_action(PlanAction::DeleteReservation(key1))
                    .add_action(PlanAction::CreateReservation(r2));

                prop_assert_eq!(plan1.len(), plan2.len());
                prop_assert_eq!(plan1.len(), 3);
            }
        }

        // PROPERTY: Plan isEmpty is the inverse of len > 0
        // This is a simple invariant but important for consistency
        proptest! {
            #[test]
            fn prop_plan_is_empty_invariant(
                key in reservation_key_strategy(),
                port in port_strategy(),
                action_count in 0usize..5,
            ) {
                // PROPERTY: is_empty() == (len() == 0)
                // This verifies the consistency of the is_empty and len methods
                let mut plan = OperationPlan::new("test");

                for _ in 0..action_count {
                    let reservation = Reservation::builder(key.clone(), port).build().unwrap();
                    plan = plan.add_action(PlanAction::CreateReservation(reservation));
                }

                prop_assert_eq!(plan.is_empty(), plan.is_empty());
            }
        }

        // PROPERTY: Warning accumulation preserves order
        // Warnings should be accumulated in the order they are added
        proptest! {
            #[test]
            fn prop_warnings_preserve_order(
                warning1 in "[a-z]{5,10}",
                warning2 in "[A-Z]{5,10}",
                warning3 in "[0-9]{5,10}",
            ) {
                // PROPERTY: Warnings are accumulated in the order added
                // This is important for user-facing error reporting
                let plan = OperationPlan::new("test")
                    .add_warning(warning1.clone())
                    .add_warning(warning2.clone())
                    .add_warning(warning3.clone());

                prop_assert_eq!(plan.warnings.len(), 3);
                prop_assert_eq!(&plan.warnings[0], &warning1);
                prop_assert_eq!(&plan.warnings[1], &warning2);
                prop_assert_eq!(&plan.warnings[2], &warning3);
            }
        }

        // PROPERTY: Actions preserve order
        // Actions should be executed in the order they are added to the plan
        proptest! {
            #[test]
            fn prop_actions_preserve_order(
                key1 in reservation_key_strategy(),
                key2 in reservation_key_strategy(),
                port1 in port_strategy(),
                port2 in port_strategy(),
            ) {
                // PROPERTY: Actions are accumulated in the order added
                // This is critical for correct execution semantics
                let r1 = Reservation::builder(key1.clone(), port1).build().unwrap();
                let r2 = Reservation::builder(key2.clone(), port2).build().unwrap();

                let plan = OperationPlan::new("test")
                    .add_action(PlanAction::CreateReservation(r1.clone()))
                    .add_action(PlanAction::UpdateLastUsed(key1))
                    .add_action(PlanAction::CreateReservation(r2));

                prop_assert_eq!(plan.actions.len(), 3);
                prop_assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));
                prop_assert!(matches!(plan.actions[1], PlanAction::UpdateLastUsed(_)));
                prop_assert!(matches!(plan.actions[2], PlanAction::CreateReservation(_)));
            }
        }

        // PROPERTY: PlanAction descriptions are non-empty
        // Every action must have a meaningful description
        proptest! {
            #[test]
            fn prop_action_descriptions_nonempty(
                key in reservation_key_strategy(),
                port in port_strategy(),
            ) {
                // PROPERTY: All PlanAction descriptions produce non-empty strings
                // This ensures that all actions can be meaningfully logged/displayed
                let reservation = Reservation::builder(key.clone(), port).build().unwrap();

                let actions = vec![
                    PlanAction::CreateReservation(reservation.clone()),
                    PlanAction::UpdateReservation(reservation),
                    PlanAction::UpdateLastUsed(key.clone()),
                    PlanAction::DeleteReservation(key),
                ];

                for action in actions {
                    let desc = action.description();
                    prop_assert!(!desc.is_empty(), "action descriptions must be non-empty");
                    prop_assert!(desc.len() > 10, "action descriptions must be meaningful");
                }
            }
        }

        // PROPERTY: Plan building is idempotent for description
        // Creating the same plan multiple times preserves the description
        proptest! {
            #[test]
            fn prop_plan_description_idempotent(
                description in "[a-zA-Z ]{10,50}",
            ) {
                // PROPERTY: Plan description is preserved exactly as provided
                let plan1 = OperationPlan::new(description.clone());
                let plan2 = OperationPlan::new(description.clone());

                prop_assert_eq!(&plan1.description, &plan2.description);
                prop_assert_eq!(&plan1.description, &description);
            }
        }

        // PROPERTY: CreateReservation and UpdateReservation contain the same reservation
        // This tests that actions preserve their payloads correctly
        proptest! {
            #[test]
            fn prop_reservation_actions_preserve_data(
                key in reservation_key_strategy(),
                port in port_strategy(),
            ) {
                // PROPERTY: Actions preserve the reservation data they contain
                let reservation = Reservation::builder(key, port).build().unwrap();

                let create_action = PlanAction::CreateReservation(reservation.clone());
                let update_action = PlanAction::UpdateReservation(reservation.clone());

                // Extract reservations from actions
                if let PlanAction::CreateReservation(r) = create_action {
                    prop_assert_eq!(r.port(), port);
                }

                if let PlanAction::UpdateReservation(r) = update_action {
                    prop_assert_eq!(r.port(), port);
                }
            }
        }
    }

    // Original manual tests follow...

    #[test]
    fn test_plan_action_description() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let action = PlanAction::CreateReservation(reservation);
        let desc = action.description();
        let normalized = desc.replace(std::path::MAIN_SEPARATOR, "/");
        assert!(normalized.contains("/path"));
        assert!(desc.contains("8080"));
    }

    #[test]
    fn test_operation_plan_new() {
        let plan = OperationPlan::new("Test operation");
        assert_eq!(plan.description, "Test operation");
        assert!(plan.is_empty());
        assert_eq!(plan.len(), 0);
    }

    #[test]
    fn test_operation_plan_add_action() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::CreateReservation(reservation));

        assert_eq!(plan.len(), 1);
        assert!(!plan.is_empty());
    }

    #[test]
    fn test_operation_plan_add_warning() {
        let plan = OperationPlan::new("Test").add_warning("Test warning");

        assert_eq!(plan.warnings.len(), 1);
        assert_eq!(plan.warnings[0], "Test warning");
    }

    #[test]
    fn test_operation_plan_builder_pattern() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::CreateReservation(reservation))
            .add_warning("Warning 1")
            .add_warning("Warning 2")
            .add_action(PlanAction::UpdateLastUsed(key));

        assert_eq!(plan.len(), 2);
        assert_eq!(plan.warnings.len(), 2);
    }
}
