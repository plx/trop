//! Property-based tests for path handling.
//!
//! Note: The normalize module already has property tests for normalization.
//! This module focuses on path relationships and resolver behavior.

use super::normalize::{normalize, resolve_components};
use super::relationship::PathRelationship;
use super::verified_glue_tests::legacy;
use proptest::prelude::*;
use std::path::{Path, PathBuf};

// Paths mixing names, `.`, `..`, empty pieces (repeated separators), and
// an optional leading root.
fn messy_path_strategy() -> impl Strategy<Value = String> {
    (
        any::<bool>(),
        prop::collection::vec(
            prop_oneof![
                Just(String::new()),
                Just(".".to_string()),
                Just("..".to_string()),
                "[a-z0-9_.-]{1,6}",
            ],
            0..10,
        ),
    )
        .prop_map(|(rooted, parts)| {
            let joined = parts.join("/");
            if rooted {
                format!("/{joined}")
            } else {
                joined
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 5000,
        .. ProptestConfig::default()
    })]

    // Differential check of the trusted `std::path` model: outside the
    // deliberately changed tilde forms, the verified normalizer agrees
    // byte-for-byte with the implementation it replaced.
    #[test]
    fn normalize_agrees_with_legacy(path in messy_path_strategy()) {
        let new = normalize(Path::new(&path));
        let old = legacy::normalize(Path::new(&path));
        match (&new, &old) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a.as_os_str(), b.as_os_str()),
            (Err(a), Err(b)) => prop_assert_eq!(a.to_string(), b.to_string()),
            _ => prop_assert!(false, "{:?}: {:?} vs {:?}", path, new, old),
        }
    }

    #[test]
    fn resolve_components_agrees_with_legacy(path in messy_path_strategy()) {
        let new = resolve_components(Path::new(&path));
        let old = legacy::resolve_components(Path::new(&path));
        match (&new, &old) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a.as_os_str(), b.as_os_str()),
            (Err(a), Err(b)) => prop_assert_eq!(a.to_string(), b.to_string()),
            _ => prop_assert!(false, "{:?}: {:?} vs {:?}", path, new, old),
        }
    }

    // Normalized paths are canonical and normalization is idempotent.
    #[test]
    fn normalize_output_is_a_fixed_point(path in messy_path_strategy()) {
        if let Ok(normalized) = normalize(Path::new(&path)) {
            prop_assert!(super::is_canonical_absolute(&normalized));
            let again = normalize(&normalized).unwrap();
            prop_assert_eq!(again.as_os_str(), normalized.as_os_str());
        }
    }

    // Outside `.`/`..` and with equal anchors, the verified relationship
    // agrees with the implementation it replaced.
    #[test]
    fn between_agrees_with_legacy_on_plain_paths(
        rooted in any::<bool>(),
        left in prop::collection::vec("[a-c]{1,2}", 0..5),
        right in prop::collection::vec("[a-c]{1,2}", 0..5),
    ) {
        let root = if rooted { "/" } else { "" };
        let left = format!("{root}{}", left.join("/"));
        let right = format!("{root}{}", right.join("/"));
        prop_assert_eq!(
            PathRelationship::between(Path::new(&left), Path::new(&right)),
            legacy::between(Path::new(&left), Path::new(&right))
        );
    }
}

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
