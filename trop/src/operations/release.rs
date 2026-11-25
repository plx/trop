//! Release operation planning and execution.
//!
//! This module implements the release planning logic, including
//! path validation and idempotent behavior.

use std::path::PathBuf;

use crate::database::Database;
use crate::error::Result;
use crate::path::relationship::path_tokens;
use crate::verified::path_relation::PathToken;
use crate::verified::release::{
    guard_paths, plan_flags, requires_path_check, select_and_guard, selector_from_plan,
    tag_selector, ReleaseRow, TagSelector,
};
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
    /// The candidate rows come from the database (recursively, every row
    /// whose path starts with the key's path; with all tags, every row at
    /// the path; otherwise the key's own row). The verified kernel
    /// `verified::release::select_and_guard` then selects exactly the rows
    /// at the path (or a component descendant of it) whose tag the request
    /// selects. Unless `force` or `allow_unrelated_path` is set, the key's
    /// path and the path of every selected reservation must pass the
    /// unrelated-path check, so a recursive release cannot delete a
    /// reservation sideways of the current directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path relationship validation fails for the key's path or for the
    ///   path of any reservation the release would delete
    /// - The database query fails
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
        let target = &self.options.key.path;
        let force = self.options.force;
        let allow_unrelated_path = self.options.allow_unrelated_path;

        // Step 1: Validate the requested path's relationship. The kernel
        // decides the lexical relation to the current directory; the
        // database's single-path check is the physical fallback for a path
        // that is lexically unrelated.
        let cwd = guard_cwd_tokens(force, allow_unrelated_path, true)?;
        guard_paths(
            force,
            allow_unrelated_path,
            &cwd,
            &[path_tokens(target)],
            &|_index| Database::validate_path_relationship(target, false).err(),
        )
        .map_err(|failure| failure.error)?;

        // Step 2: Read the candidate rows: every row under the path
        // (recursive), every row at the path (all tags), or the requested
        // key's row.
        let candidates = if self.recursive {
            Database::get_reservations_by_path_prefix(conn, target)?
        } else if self.all_exact_path_tags {
            Database::get_reservations_by_exact_path(conn, target)?
        } else {
            Database::get_reservation(conn, &self.options.key)?
                .into_iter()
                .collect()
        };

        // Step 3: The kernel selects the rows the release deletes and applies
        // the unrelated-path guard to exactly those rows. Selection compares
        // stored identities: the target as the database stores its path, and
        // the requested tag as the database stores it (an empty tag is the
        // untagged sentinel).
        let stored_target = PathBuf::from(self.options.key.path_as_string());
        let selector = selector_from_plan(
            stored_tag(self.options.key.tag.as_deref()),
            self.all_exact_path_tags,
        );
        let rows: Vec<ReleaseRow> = candidates
            .iter()
            .map(|reservation| ReleaseRow {
                path: path_tokens(&reservation.key().path),
                tag: stored_tag(reservation.key().tag.as_deref()),
            })
            .collect();
        let selected = select_and_guard(
            &selector,
            self.recursive,
            &path_tokens(&stored_target),
            &rows,
            force,
            allow_unrelated_path,
            &cwd,
            &|index| {
                Database::validate_path_relationship(&candidates[index].key().path, false).err()
            },
        )
        .map_err(|failure| {
            log::debug!(
                "Unrelated-path guard refused {}",
                candidates[failure.index].key()
            );
            failure.error
        })?;

        for &index in &selected {
            plan = plan.add_action(PlanAction::DeleteReservation(
                candidates[index].key().clone(),
            ));
        }
        if selected.is_empty() {
            let warning = if self.recursive {
                format!(
                    "No reservations found under {} (already released)",
                    target.display()
                )
            } else if self.all_exact_path_tags {
                format!(
                    "No reservations found for {} (already released)",
                    target.display()
                )
            } else {
                format!(
                    "No reservation found for {} (already released)",
                    self.options.key
                )
            };
            plan = plan.add_warning(warning);
        }

        Ok(plan)
    }
}

/// A tag's bytes as the database stores it: the empty tag is the untagged
/// sentinel, so it denotes the untagged reservation.
fn stored_tag(tag: Option<&str>) -> Option<Vec<u8>> {
    tag.filter(|tag| !tag.is_empty())
        .map(|tag| tag.as_bytes().to_vec())
}

/// The current directory's component tokens for the unrelated-path guard.
///
/// The directory is read only when the guard applies (neither `force` nor
/// `allow_unrelated_path`) and there is a path to check; otherwise the
/// kernels never consult it. The per-path physical fallback,
/// [`Database::validate_path_relationship`], reads the same directory.
pub(crate) fn guard_cwd_tokens(
    force: bool,
    allow_unrelated_path: bool,
    has_paths: bool,
) -> Result<Vec<PathToken>> {
    if has_paths && requires_path_check(force, allow_unrelated_path) {
        Ok(path_tokens(&std::env::current_dir()?))
    } else {
        Ok(Vec::new())
    }
}

/// Which tags `trop release` selects, from its `--tag` and
/// `--untagged-only` flags (the specification's table, decided by the
/// verified kernel `verified::release::tag_selector`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseTagSelection {
    /// No flag: every reservation at each selected path, tagged or not.
    AllTags,
    /// `--untagged-only`: only the untagged reservation.
    UntaggedOnly,
    /// `--tag`: only the reservation with this tag.
    Tag(String),
}

impl ReleaseTagSelection {
    /// Resolves the `--tag` / `--untagged-only` flags.
    ///
    /// # Errors
    ///
    /// Returns an error when both flags are given.
    pub fn from_flags(
        tag: Option<String>,
        untagged_only: bool,
    ) -> std::result::Result<Self, ConflictingReleaseTagFlags> {
        match tag_selector(tag, untagged_only) {
            Ok(TagSelector::AllTags) => Ok(Self::AllTags),
            Ok(TagSelector::UntaggedOnly) => Ok(Self::UntaggedOnly),
            Ok(TagSelector::Exact(tag)) => Ok(Self::Tag(tag)),
            Err(_) => Err(ConflictingReleaseTagFlags),
        }
    }

    /// The planner's representation: the key's tag, and whether every tag
    /// at a path is selected (see [`ReleasePlan::with_all_exact_path_tags`]).
    #[must_use]
    pub fn into_key_tag_and_all_tags(self) -> (Option<String>, bool) {
        plan_flags(match self {
            Self::AllTags => TagSelector::AllTags,
            Self::UntaggedOnly => TagSelector::UntaggedOnly,
            Self::Tag(tag) => TagSelector::Exact(tag),
        })
    }
}

/// `--tag` and `--untagged-only` were both given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictingReleaseTagFlags;

impl std::fmt::Display for ConflictingReleaseTagFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cannot specify both --tag and --untagged-only")
    }
}

impl std::error::Error for ConflictingReleaseTagFlags {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::{Port, Reservation};
    use std::path::{Path, PathBuf};

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
            assert!(!plan1.warnings.is_empty(), "must have at least one warning");

            // Second release - should produce identical result
            let plan2 = ReleasePlan::new(options)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(plan2.len(), 0);
            assert!(!plan2.warnings.is_empty(), "must have at least one warning");
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

        assert!(
            plan.actions.is_empty(),
            "unexpected actions: {:?}",
            plan.actions
        );
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

        assert!(
            plan.actions.is_empty(),
            "unexpected actions: {:?}",
            plan.actions
        );
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

    // ------------------------------------------------------------------
    // Verified selection, tag table, and the guard over the affected set
    // ------------------------------------------------------------------

    fn planned_keys(plan: &OperationPlan) -> Vec<ReservationKey> {
        plan.actions
            .iter()
            .map(|action| match action {
                PlanAction::DeleteReservation(key) => key.clone(),
                other => panic!("unexpected release action: {other:?}"),
            })
            .collect()
    }

    #[test]
    fn test_tag_selection_table() {
        assert_eq!(
            ReleaseTagSelection::from_flags(None, false),
            Ok(ReleaseTagSelection::AllTags)
        );
        assert_eq!(
            ReleaseTagSelection::from_flags(None, true),
            Ok(ReleaseTagSelection::UntaggedOnly)
        );
        assert_eq!(
            ReleaseTagSelection::from_flags(Some("web".to_string()), false),
            Ok(ReleaseTagSelection::Tag("web".to_string()))
        );
        let conflict = ReleaseTagSelection::from_flags(Some("web".to_string()), true).unwrap_err();
        assert_eq!(
            conflict.to_string(),
            "Cannot specify both --tag and --untagged-only"
        );

        assert_eq!(
            ReleaseTagSelection::AllTags.into_key_tag_and_all_tags(),
            (None, true)
        );
        assert_eq!(
            ReleaseTagSelection::UntaggedOnly.into_key_tag_and_all_tags(),
            (None, false)
        );
        assert_eq!(
            ReleaseTagSelection::Tag("web".to_string()).into_key_tag_and_all_tags(),
            (Some("web".to_string()), false)
        );
    }

    /// The component tokens the verified selection compares agree with
    /// `Path::starts_with` and component equality.
    #[test]
    fn test_path_tokens_agree_with_std() {
        let paths = [
            "/", "/a", "/a/", "/a/b", "/a/bc", "/a/b/c", "/ab", "a", "a/b", "./a", "/a/./b",
            "/a/../b",
        ];
        let views = |path: &Path| -> Vec<_> {
            path_tokens(path)
                .into_iter()
                .map(|token| (token.kind, token.name))
                .collect()
        };
        for first in paths {
            for second in paths {
                let (p, s) = (Path::new(first), Path::new(second));
                let (kp, ks) = (views(p), views(s));
                assert_eq!(
                    kp == ks,
                    p.components().eq(s.components()),
                    "{first} == {second}"
                );
                assert_eq!(
                    ks.len() >= kp.len() && ks[..kp.len()] == kp[..],
                    s.starts_with(p),
                    "{second} starts with {first}"
                );
            }
        }
    }

    /// Inserts a row directly, bypassing the write-side validation.
    fn insert_raw_row(db: &Database, path: &str, tag: &str, port: u16) {
        db.connection()
            .execute(
                "INSERT INTO reservations (path, tag, port, project, task, created_at, last_used_at) \
                 VALUES (?1, ?2, ?3, NULL, NULL, 1, 1)",
                rusqlite::params![path, tag, port],
            )
            .unwrap();
    }

    /// Exact-key release reads only the requested key's row, so a corrupt
    /// row with another tag at the same path does not block it.
    #[test]
    fn test_exact_key_release_ignores_a_corrupt_sibling_row() {
        let mut db = create_test_database();
        let path = PathBuf::from("/test/path");
        for key in [
            ReservationKey::new(path.clone(), None).unwrap(),
            ReservationKey::new(path.clone(), Some("api".to_string())).unwrap(),
        ] {
            let port = if key.tag.is_some() { 8081 } else { 8080 };
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }
        insert_raw_row(&db, "/test/path", " web ", 8082);

        for tag in [None, Some("api".to_string())] {
            let key = ReservationKey::new(path.clone(), tag).unwrap();
            let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
            let plan = ReleasePlan::new(options)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(planned_keys(&plan), vec![key]);
        }

        // Selecting every tag at the path reads the corrupt row, as before.
        let key = ReservationKey::new(path, None).unwrap();
        let result = ReleasePlan::new(ReleaseOptions::new(key).with_allow_unrelated_path(true))
            .with_all_exact_path_tags(true)
            .build_plan(db.connection());
        assert!(matches!(
            result,
            Err(crate::Error::DatabaseCorruption { .. })
        ));
    }

    /// A key whose tag is the empty string denotes the untagged reservation,
    /// as it does in storage.
    #[test]
    fn test_empty_tag_key_releases_the_untagged_reservation() {
        let mut db = create_test_database();
        let path = PathBuf::from("/test/path");
        let untagged = ReservationKey::new(path.clone(), None).unwrap();
        let reservation = Reservation::builder(untagged.clone(), Port::try_from(8080).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let empty_tag = ReservationKey {
            path,
            tag: Some(String::new()),
        };
        let options = ReleaseOptions::new(empty_tag).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .build_plan(db.connection())
            .unwrap();
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.warnings, Vec::<String>::new());
        let mut executor = crate::operations::PlanExecutor::new(db.connection());
        executor.execute(&plan).unwrap();
        assert!(Database::get_reservation(db.connection(), &untagged)
            .unwrap()
            .is_none());
    }

    /// A key path that is not valid UTF-8 is stored lossily; release selects
    /// its stored row in every non-recursive mode.
    #[test]
    #[cfg(unix)]
    fn test_non_utf8_key_path_is_released() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut db = create_test_database();
        let path = PathBuf::from(OsStr::from_bytes(b"/test/\xff"));
        let key = ReservationKey {
            path: path.clone(),
            tag: None,
        };
        let reservation = Reservation::builder(key.clone(), Port::try_from(8080).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        for all_tags in [false, true] {
            let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
            let plan = ReleasePlan::new(options)
                .with_all_exact_path_tags(all_tags)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(plan.actions.len(), 1, "all_tags={all_tags}");
            assert!(plan.warnings.is_empty(), "all_tags={all_tags}");
        }
        let options = ReleaseOptions::new(key.clone()).with_allow_unrelated_path(true);
        let plan = ReleasePlan::new(options)
            .build_plan(db.connection())
            .unwrap();
        let mut executor = crate::operations::PlanExecutor::new(db.connection());
        executor.execute(&plan).unwrap();
        assert!(Database::get_reservation(db.connection(), &key)
            .unwrap()
            .is_none());
    }

    /// Exact-key release (`--tag` or `--untagged-only`) selects only the
    /// matching tag at the exact path.
    #[test]
    fn test_plan_exact_key_selects_only_matching_tag() {
        let mut db = create_test_database();
        let path = PathBuf::from("/test/path");
        let keys = [
            ReservationKey::new(path.clone(), None).unwrap(),
            ReservationKey::new(path.clone(), Some("web".to_string())).unwrap(),
            ReservationKey::new(path.join("child"), None).unwrap(),
        ];
        for (key, port) in keys.iter().cloned().zip(8080..) {
            let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }

        for selected in [&keys[0], &keys[1]] {
            let options = ReleaseOptions::new(selected.clone()).with_allow_unrelated_path(true);
            let plan = ReleasePlan::new(options)
                .build_plan(db.connection())
                .unwrap();
            assert_eq!(planned_keys(&plan), vec![selected.clone()]);
        }
    }

    /// Regression: the unrelated-path guard covers every reservation a
    /// recursive release deletes, not just the requested path. From the
    /// current directory, a recursive release of its parent must not delete
    /// a reservation in a sibling directory without an override.
    #[test]
    fn test_recursive_release_guards_every_selected_reservation() {
        let mut db = create_test_database();
        let cwd = std::env::current_dir().unwrap();
        let parent = cwd.parent().unwrap().to_path_buf();
        let below_cwd = ReservationKey::new(cwd.join("trop-release-guard-child"), None).unwrap();
        let sideways =
            ReservationKey::new(parent.join("trop-release-guard-sibling"), None).unwrap();
        for (key, port) in [(&below_cwd, 8080), (&sideways, 8081)] {
            let reservation = Reservation::builder(key.clone(), Port::try_from(port).unwrap())
                .build()
                .unwrap();
            db.create_reservation(&reservation).unwrap();
        }
        let target = ReservationKey::new(parent, None).unwrap();

        // The requested path (an ancestor of the cwd) passes on its own, but
        // the sibling's reservation does not.
        let result = ReleasePlan::new(ReleaseOptions::new(target.clone()))
            .with_all_exact_path_tags(true)
            .with_recursive(true)
            .build_plan(db.connection());
        assert!(matches!(
            result,
            Err(crate::Error::PathRelationshipViolation { .. })
        ));

        for options in [
            ReleaseOptions::new(target.clone()).with_force(true),
            ReleaseOptions::new(target.clone()).with_allow_unrelated_path(true),
        ] {
            let plan = ReleasePlan::new(options)
                .with_all_exact_path_tags(true)
                .with_recursive(true)
                .build_plan(db.connection())
                .unwrap();
            let keys = planned_keys(&plan);
            assert!(keys.contains(&below_cwd));
            assert!(keys.contains(&sideways));
        }

        // A recursive release of the cwd itself only selects related rows.
        let options = ReleaseOptions::new(ReservationKey::new(cwd, None).unwrap());
        let plan = ReleasePlan::new(options)
            .with_all_exact_path_tags(true)
            .with_recursive(true)
            .build_plan(db.connection())
            .unwrap();
        assert_eq!(planned_keys(&plan), vec![below_cwd]);
    }
}
