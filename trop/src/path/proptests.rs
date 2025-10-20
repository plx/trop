//! Property-based tests for path handling.
//!
//! Note: The normalize module already has property tests for normalization.
//! This module focuses on path relationships and resolver behavior.

use super::normalize::normalize;
use super::relationship::PathRelationship;
use proptest::prelude::*;
use std::path::PathBuf;

// Strategy for generating path-like strings
fn path_component_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9_-]{1,20}"
}

fn absolute_path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec(path_component_strategy(), 1..8).prop_map(|parts| {
        let mut path = PathBuf::from("/");
        for part in parts {
            path.push(part);
        }
        path
    })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // Normalization is idempotent: normalize(normalize(p)) == normalize(p)
    #[test]
    fn path_normalization_idempotent(path in absolute_path_strategy()) {
        if let Ok(normalized_once) = normalize(&path) {
            if let Ok(normalized_twice) = normalize(&normalized_once) {
                prop_assert_eq!(normalized_once, normalized_twice);
            }
        }
    }

    // Normalized paths never contain ".."
    #[test]
    fn normalized_paths_no_parent_refs(path in absolute_path_strategy()) {
        if let Ok(normalized) = normalize(&path) {
            let path_str = normalized.to_string_lossy();
            prop_assert!(!path_str.contains(".."));
        }
    }

    // Path relationship is reflexive: path is always identical to itself
    #[test]
    fn path_relationship_reflexive(path in absolute_path_strategy()) {
        let rel = PathRelationship::between(&path, &path);
        prop_assert_eq!(rel, PathRelationship::Same);
    }

    // Containment is transitive (if A contains B and B contains C, then A contains C)
    #[test]
    fn path_containment_transitive(base in absolute_path_strategy(), parts1 in 1..5usize, parts2 in 1..5usize) {
        let mut path_b = base.clone();
        for i in 0..parts1 {
            path_b.push(format!("sub{i}"));
        }

        let mut path_c = path_b.clone();
        for i in 0..parts2 {
            path_c.push(format!("deep{i}"));
        }

        let rel_ab = PathRelationship::between(&base, &path_b);
        let rel_bc = PathRelationship::between(&path_b, &path_c);
        let rel_ac = PathRelationship::between(&base, &path_c);

        if matches!(rel_ab, PathRelationship::Ancestor) &&
           matches!(rel_bc, PathRelationship::Ancestor) {
            prop_assert!(matches!(rel_ac, PathRelationship::Ancestor));
        }
    }

    // Relationship types are mutually exclusive
    #[test]
    fn path_relationship_mutually_exclusive(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let rel = PathRelationship::between(&path1, &path2);

        // Each path pair has exactly one relationship type
        let is_same = matches!(rel, PathRelationship::Same);
        let is_ancestor = matches!(rel, PathRelationship::Ancestor);
        let is_descendant = matches!(rel, PathRelationship::Descendant);
        let is_unrelated = matches!(rel, PathRelationship::Unrelated);

        let count = [is_same, is_ancestor, is_descendant, is_unrelated]
            .iter()
            .filter(|&&x| x)
            .count();

        prop_assert_eq!(count, 1);
    }

    // Relationship symmetry: if A is ancestor of B, then B is descendant of A
    #[test]
    fn path_relationship_symmetric(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let rel_12 = PathRelationship::between(&path1, &path2);
        let rel_21 = PathRelationship::between(&path2, &path1);

        let is_symmetric = matches!(
            (rel_12, rel_21),
            (PathRelationship::Ancestor, PathRelationship::Descendant)
                | (PathRelationship::Descendant, PathRelationship::Ancestor)
                | (PathRelationship::Same, PathRelationship::Same)
                | (PathRelationship::Unrelated, PathRelationship::Unrelated)
        );

        prop_assert!(is_symmetric, "Invalid symmetry: {:?} <-> {:?}", rel_12, rel_21);
    }

    // is_hierarchical is consistent with relationship type
    #[test]
    fn path_is_hierarchical_consistent(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let rel = PathRelationship::between(&path1, &path2);
        let hierarchical = rel.is_hierarchical();
        let is_unrelated = matches!(rel, PathRelationship::Unrelated);

        prop_assert_eq!(hierarchical, !is_unrelated);
    }

    // is_allowed_without_force matches is_hierarchical
    #[test]
    fn path_allowed_matches_hierarchical(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let rel = PathRelationship::between(&path1, &path2);
        prop_assert_eq!(rel.is_allowed_without_force(), rel.is_hierarchical());
    }

    // is_within and contains are consistent
    #[test]
    fn path_is_within_contains_consistent(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let within = PathRelationship::is_within(&path1, &path2);
        let contains = PathRelationship::contains(&path2, &path1);

        prop_assert_eq!(within, contains);
    }

    // Ancestor relationship implies contains
    #[test]
    fn path_ancestor_implies_contains(base in absolute_path_strategy(), depth in 1..5usize) {
        let mut child = base.clone();
        for i in 0..depth {
            child.push(format!("level{i}"));
        }

        let rel = PathRelationship::between(&base, &child);
        prop_assert_eq!(rel, PathRelationship::Ancestor);

        let contains = PathRelationship::contains(&base, &child);
        prop_assert!(contains);
    }

    // Descendant relationship implies is_within
    #[test]
    fn path_descendant_implies_within(base in absolute_path_strategy(), depth in 1..5usize) {
        let mut child = base.clone();
        for i in 0..depth {
            child.push(format!("level{i}"));
        }

        let rel = PathRelationship::between(&child, &base);
        prop_assert_eq!(rel, PathRelationship::Descendant);

        let within = PathRelationship::is_within(&child, &base);
        prop_assert!(within);
    }
}
