//! Release operation planning and execution.
//!
//! This module implements the release planning logic, including
//! path validation and idempotent behavior.

use crate::database::Database;
use crate::error::Result;
use crate::ReservationKey;
use rusqlite::Connection;

use super::plan::{OperationPlan, PlanAction};

/// Options for a release operation.
///
/// This struct contains all the parameters needed to plan a release operation.
#[derive(Debug, Clone)]
pub struct ReleaseOptions {
    /// The reservation key to release.
    pub key: ReservationKey,

    /// Force flag - overrides all protections.
    pub force: bool,

    /// Allow operations on unrelated paths.
    pub allow_unrelated_path: bool,
}

impl ReleaseOptions {
    /// Creates a new `ReleaseOptions` with the given key.
    ///
    /// All flags are set to defaults:
    /// - force: false
    /// - `allow_unrelated_path`: false
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::ReleaseOptions;
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let options = ReleaseOptions::new(key);
    /// assert!(!options.force);
    /// ```
    #[must_use]
    pub const fn new(key: ReservationKey) -> Self {
        Self {
            key,
            force: false,
            allow_unrelated_path: false,
        }
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
}

/// A release plan generator.
///
/// This struct is responsible for analyzing a release request and
/// generating a plan that describes what actions to take.
pub struct ReleasePlan {
    options: ReleaseOptions,
    all_exact_path_tags: bool,
    recursive: bool,
}

impl ReleasePlan {
    /// Creates a new release plan with the given options.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::operations::{ReleasePlan, ReleaseOptions};
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let options = ReleaseOptions::new(key);
    /// let planner = ReleasePlan::new(options);
    /// ```
    #[must_use]
    pub const fn new(options: ReleaseOptions) -> Self {
        Self {
            options,
            all_exact_path_tags: false,
            recursive: false,
        }
    }

    /// Selects every tagged and untagged reservation at the key's exact path.
    ///
    /// The default planner behavior remains an exact-key release. CLI callers
    /// use this mode when neither `--tag` nor `--untagged-only` is supplied.
    #[must_use]
    pub const fn with_all_exact_path_tags(mut self, enabled: bool) -> Self {
        self.all_exact_path_tags = enabled;
        self
    }

    /// Selects reservations at the key path and every component descendant.
    ///
    /// Tag selection follows the exact-release contract at every selected
    /// path: all tags when [`Self::with_all_exact_path_tags`] is enabled,
    /// otherwise only the key's exact optional tag.
    #[must_use]
    pub const fn with_recursive(mut self, enabled: bool) -> Self {
        self.recursive = enabled;
        self
    }

    /// Builds an operation plan for this release request.
    ///
    /// This method performs validation and determines what actions are needed.
    /// It does NOT modify the database. Release operations are idempotent -
    /// if the reservation doesn't exist, a warning is added but no error occurs.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path relationship validation fails
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{ReleasePlan, ReleaseOptions};
    /// use trop::{Database, DatabaseConfig, ReservationKey};
    /// use std::path::PathBuf;
    ///
    /// let db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    ///
    /// let plan = ReleasePlan::new(options).build_plan(db.connection()).unwrap();
    /// ```
    pub fn build_plan(&self, conn: &Connection) -> Result<OperationPlan> {
        let mut plan = OperationPlan::new(format!("Release reservation for {}", self.options.key));

        // Step 1: Validate path relationship
        if !self.options.force && !self.options.allow_unrelated_path {
            Database::validate_path_relationship(&self.options.key.path, false)?;
        }

        // Step 2: Select the recursive set, exact-path set, or exact key
        // requested by the caller.
        if self.recursive {
            let reservations =
                Database::get_reservations_by_path_prefix(conn, &self.options.key.path)?;
            let matching = reservations.into_iter().filter(|reservation| {
                self.all_exact_path_tags
                    || reservation.key().tag.as_deref() == self.options.key.tag.as_deref()
            });
            for reservation in matching {
                plan = plan.add_action(PlanAction::DeleteReservation(reservation.key().clone()));
            }
            if plan.actions.is_empty() {
                plan = plan.add_warning(format!(
                    "No reservations found under {} (already released)",
                    self.options.key.path.display()
                ));
            }
        } else if self.all_exact_path_tags {
            let reservations =
                Database::get_reservations_by_exact_path(conn, &self.options.key.path)?;
            if reservations.is_empty() {
                plan = plan.add_warning(format!(
                    "No reservations found for {} (already released)",
                    self.options.key.path.display()
                ));
            } else {
                for reservation in reservations {
                    plan =
                        plan.add_action(PlanAction::DeleteReservation(reservation.key().clone()));
                }
            }
        } else if Database::get_reservation(conn, &self.options.key)?.is_some() {
            plan = plan.add_action(PlanAction::DeleteReservation(self.options.key.clone()));
        } else {
            plan = plan.add_warning(format!(
                "No reservation found for {} (already released)",
                self.options.key
            ));
        }

        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::{Port, Reservation};
    use std::path::PathBuf;

    // Property-based testing module
    // These tests verify mathematical properties and invariants of the release system
    #[cfg(feature = "property-tests")]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

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

        // PROPERTY: ReleaseOptions builder is idempotent
        // Building options with the same values multiple times produces equal structures
        proptest! {
            #[test]
            fn prop_release_options_builder_idempotent(
                key in reservation_key_strategy(),
                force in any::<bool>(),
                allow_unrelated in any::<bool>(),
            ) {
                // PROPERTY: Building options twice with identical parameters yields identical results
                // This verifies that the builder has no hidden state or side effects
                let opts1 = ReleaseOptions::new(key.clone())
                    .with_force(force)
                    .with_allow_unrelated_path(allow_unrelated);

                let opts2 = ReleaseOptions::new(key.clone())
                    .with_force(force)
                    .with_allow_unrelated_path(allow_unrelated);

                // Compare all fields
                prop_assert_eq!(opts1.force, opts2.force);
                prop_assert_eq!(opts1.allow_unrelated_path, opts2.allow_unrelated_path);
            }
        }

        // PROPERTY: Builder methods are commutative
        // The order of setting flags doesn't affect the final result
        proptest! {
            #[test]
            fn prop_release_options_builder_commutative(
                key in reservation_key_strategy(),
                force in any::<bool>(),
                allow_unrelated in any::<bool>(),
            ) {
                // PROPERTY: Order of builder calls doesn't matter (commutativity)
                // This is a critical property for builder patterns
                let opts1 = ReleaseOptions::new(key.clone())
                    .with_force(force)
                    .with_allow_unrelated_path(allow_unrelated);

                let opts2 = ReleaseOptions::new(key.clone())
                    .with_allow_unrelated_path(allow_unrelated)
                    .with_force(force);

                prop_assert_eq!(opts1.force, opts2.force);
                prop_assert_eq!(opts1.allow_unrelated_path, opts2.allow_unrelated_path);
            }
        }

        // PROPERTY: Release is idempotent - releasing a non-existent reservation succeeds
        // This is a critical property for safe, repeatable operations
        proptest! {
            #[test]
            fn prop_release_nonexistent_is_idempotent(
                key in reservation_key_strategy(),
            ) {
                // PROPERTY: Releasing a non-existent reservation generates an empty plan
                // with a warning (not an error). This is the idempotency guarantee.
                let db = create_test_database();

                let options = ReleaseOptions::new(key)
                    .with_allow_unrelated_path(true);

                let plan = ReleasePlan::new(options).build_plan(db.connection()).unwrap();

                // Must succeed with empty actions and a warning
                prop_assert_eq!(plan.len(), 0, "releasing non-existent reservation must have no actions");
                prop_assert!(!plan.warnings.is_empty(), "must have at least one warning");
            }
        }

        // PROPERTY: Releasing existing reservation generates delete action
        // This verifies the core release behavior
        proptest! {
            #[test]
            fn prop_release_existing_generates_delete(
                port in (1u16..=65535).prop_map(|p| Port::try_from(p).unwrap()),
            ) {
                // PROPERTY: When a reservation exists, release generates a DeleteReservation action
                let mut db = create_test_database();
                let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();

                // Create a reservation
                let reservation = Reservation::builder(key.clone(), port).build().unwrap();
                db.create_reservation(&reservation).unwrap();

                // Plan to release it
                let options = ReleaseOptions::new(key)
                    .with_allow_unrelated_path(true);

                let plan = ReleasePlan::new(options).build_plan(db.connection()).unwrap();

                // Must generate DeleteReservation action
                prop_assert_eq!(plan.len(), 1);
                prop_assert!(matches!(plan.actions[0], PlanAction::DeleteReservation(_)),
                    "releasing existing reservation must generate DeleteReservation action");
            }
        }

        // PROPERTY: Force flag overrides path relationship validation
        // This tests the force flag's path validation override behavior
        #[test]
        fn prop_release_force_overrides_path_validation() {
            // PROPERTY: With force=true, path relationship validation is skipped
            // This is a constant property test - behavior doesn't depend on random inputs
            let db = create_test_database();
            let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

            let options_without_force = ReleaseOptions::new(key.clone())
                .with_force(false)
                .with_allow_unrelated_path(false);

            let options_with_force = ReleaseOptions::new(key.clone())
                .with_force(true)
                .with_allow_unrelated_path(false);

            // Without force, should fail path validation
            let result_without =
                ReleasePlan::new(options_without_force).build_plan(db.connection());
            assert!(
                result_without.is_err(),
                "unrelated path must fail without force"
            );

            // With force, should succeed
            let result_with = ReleasePlan::new(options_with_force).build_plan(db.connection());
            assert!(result_with.is_ok(), "force must override path validation");
        }

        // PROPERTY: allow_unrelated_path flag enables unrelated path operations
        // This tests the specific path override mechanism
        #[test]
        fn prop_release_allow_unrelated_path_enables_operations() {
            // PROPERTY: The allow_unrelated_path flag specifically enables operations
            // on paths unrelated to the current working directory
            // This is a constant property test - behavior doesn't depend on random inputs
            let db = create_test_database();
            let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

            let options = ReleaseOptions::new(key)
                .with_force(false)
                .with_allow_unrelated_path(true);

            let result = ReleasePlan::new(options).build_plan(db.connection());
            assert!(
                result.is_ok(),
                "allow_unrelated_path must enable unrelated path operations"
            );
        }

        // PROPERTY: Multiple releases generate the same result (idempotency)
        // Releasing the same key multiple times always produces the same outcome
        #[test]
        fn prop_multiple_releases_are_idempotent() {
            // PROPERTY: Releasing a key multiple times produces consistent results
            // The first release deletes, subsequent releases produce warnings
            // This is a constant property test - behavior doesn't depend on random inputs
            let db = create_test_database();
            let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();

            let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);

            // First release - should have no actions (nothing to delete)
            let plan1 = ReleasePlan::new(options.clone())
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(plan1.len(), 0);
            assert!(!plan1.warnings.is_empty());

            // Second release - should produce identical result
            let plan2 = ReleasePlan::new(options)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(plan2.len(), 0);
            assert!(!plan2.warnings.is_empty());
        }
    }

    // Original manual tests follow...

    #[test]
    fn test_release_options_new() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let options = ReleaseOptions::new(key);

        assert!(!options.force);
        assert!(!options.allow_unrelated_path);
    }

    #[test]
    fn test_release_options_builder() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let options = ReleaseOptions::new(key)
            .with_force(true)
            .with_allow_unrelated_path(true);

        assert!(options.force);
        assert!(options.allow_unrelated_path);
    }

    #[test]
    fn test_plan_release_existing_reservation() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create a reservation
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        // Plan to release it
        let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .build_plan(db.connection())
            .unwrap();

        assert_eq!(plan.len(), 1);
        assert!(matches!(plan.actions[0], PlanAction::DeleteReservation(_)));
    }

    #[test]
    fn test_plan_release_all_exact_path_tags_preserves_descendants() {
        let mut db = create_test_database();
        let exact_path = PathBuf::from("/test/path");
        let keys = [
            ReservationKey::new(exact_path.clone(), None).unwrap(),
            ReservationKey::new(exact_path.clone(), Some("api".to_string())).unwrap(),
            ReservationKey::new(exact_path.clone(), Some("web".to_string())).unwrap(),
        ];
        let child_key =
            ReservationKey::new(PathBuf::from("/test/path/child"), Some("web".to_string()))
                .unwrap();

        for (key, port) in keys
            .iter()
            .cloned()
            .chain(std::iter::once(child_key.clone()))
            .zip(8080..)
        {
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let options = ReleaseOptions::new(keys[0].clone()).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .with_all_exact_path_tags(true)
            .build_plan(db.connection())
            .unwrap();

        let planned_keys = plan
            .actions
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key,
                other => panic!("unexpected release action: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(planned_keys.len(), keys.len());
        for key in &keys {
            assert!(planned_keys.contains(&key));
        }
        assert!(!planned_keys.contains(&&child_key));
    }

    #[test]
    fn test_plan_release_all_exact_path_tags_is_idempotent_when_empty() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);

        let plan = ReleasePlan::new(options)
            .with_all_exact_path_tags(true)
            .build_plan(db.connection())
            .unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("No reservations found"));
    }

    #[test]
    fn test_plan_recursive_all_tags_is_component_aware() {
        let mut db = create_test_database();
        let root = PathBuf::from("/work/a");
        let selected_keys = [
            ReservationKey::new(root.clone(), None).unwrap(),
            ReservationKey::new(root.clone(), Some("web".to_string())).unwrap(),
            ReservationKey::new(PathBuf::from("/work/a/child"), Some("api".to_string())).unwrap(),
        ];
        let lexical_sibling = ReservationKey::new(PathBuf::from("/work/ab"), None).unwrap();

        for (key, port) in selected_keys
            .iter()
            .cloned()
            .chain(std::iter::once(lexical_sibling.clone()))
            .zip(8080..)
        {
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let options = ReleaseOptions::new(selected_keys[0].clone()).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .with_all_exact_path_tags(true)
            .with_recursive(true)
            .build_plan(db.connection())
            .unwrap();

        let planned_keys = plan
            .actions
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key,
                other => panic!("unexpected recursive release action: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(planned_keys.len(), selected_keys.len());
        for key in &selected_keys {
            assert!(planned_keys.contains(&key));
        }
        assert!(!planned_keys.contains(&&lexical_sibling));
    }

    #[test]
    fn test_plan_recursive_exact_tag_applies_at_every_path() {
        let mut db = create_test_database();
        let root = PathBuf::from("/work/a");
        let matching_keys = [
            ReservationKey::new(root.clone(), Some("web".to_string())).unwrap(),
            ReservationKey::new(PathBuf::from("/work/a/child"), Some("web".to_string())).unwrap(),
        ];
        let nonmatching_keys = [
            ReservationKey::new(root.clone(), None).unwrap(),
            ReservationKey::new(PathBuf::from("/work/a/child"), Some("api".to_string())).unwrap(),
        ];

        for (key, port) in matching_keys
            .iter()
            .chain(&nonmatching_keys)
            .cloned()
            .zip(8080..)
        {
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let options = ReleaseOptions::new(matching_keys[0].clone()).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .with_recursive(true)
            .build_plan(db.connection())
            .unwrap();

        let planned_keys = plan
            .actions
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key,
                other => panic!("unexpected recursive release action: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(planned_keys.len(), matching_keys.len());
        for key in &matching_keys {
            assert!(planned_keys.contains(&key));
        }
        for key in &nonmatching_keys {
            assert!(!planned_keys.contains(&key));
        }
    }

    #[test]
    fn test_plan_recursive_untagged_applies_at_every_path() {
        let mut db = create_test_database();
        let root = PathBuf::from("/work/a");
        let matching_keys = [
            ReservationKey::new(root.clone(), None).unwrap(),
            ReservationKey::new(PathBuf::from("/work/a/child"), None).unwrap(),
        ];
        let tagged_key =
            ReservationKey::new(PathBuf::from("/work/a/child"), Some("web".to_string())).unwrap();

        for (key, port) in matching_keys
            .iter()
            .cloned()
            .chain(std::iter::once(tagged_key.clone()))
            .zip(8080..)
        {
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        let options = ReleaseOptions::new(matching_keys[0].clone()).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .with_recursive(true)
            .build_plan(db.connection())
            .unwrap();

        let planned_keys = plan
            .actions
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key,
                other => panic!("unexpected recursive release action: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(planned_keys.len(), matching_keys.len());
        for key in &matching_keys {
            assert!(planned_keys.contains(&key));
        }
        assert!(!planned_keys.contains(&&tagged_key));
    }

    #[test]
    fn test_plan_recursive_is_idempotent_when_selection_is_empty() {
        let db = create_test_database();
        let key =
            ReservationKey::new(PathBuf::from("/test/path"), Some("web".to_string())).unwrap();
        let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);

        let plan = ReleasePlan::new(options)
            .with_recursive(true)
            .build_plan(db.connection())
            .unwrap();

        assert!(plan.actions.is_empty());
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("No reservations found under"));
    }

    #[test]
    fn test_plan_release_nonexistent_reservation() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();

        // Plan to release a reservation that doesn't exist
        let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .build_plan(db.connection())
            .unwrap();

        // Should be empty with a warning (idempotent)
        assert_eq!(plan.len(), 0);
        assert_eq!(plan.warnings.len(), 1);
        assert!(plan.warnings[0].contains("No reservation found"));
    }

    #[test]
    fn test_plan_release_path_relationship_denied() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

        // Don't allow unrelated path
        let options = ReleaseOptions::new(key);
        let result = ReleasePlan::new(options).build_plan(db.connection());

        assert!(result.is_err());
    }

    #[test]
    fn test_plan_release_path_relationship_with_force() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

        // Force allows unrelated path
        let options = ReleaseOptions::new(key).with_force(true);
        let result = ReleasePlan::new(options).build_plan(db.connection());

        assert!(result.is_ok());
    }

    #[test]
    fn test_plan_release_path_relationship_with_allow_flag() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/unrelated/path"), None).unwrap();

        // Allow unrelated path flag
        let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
        let result = ReleasePlan::new(options).build_plan(db.connection());

        assert!(result.is_ok());
    }
}
