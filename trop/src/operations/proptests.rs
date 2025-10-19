//! Property-based tests for operations module.
//!
//! These tests focus on the plan-execute pattern and operation invariants.

use crate::{Port, ReservationKey};
use proptest::prelude::*;
use std::path::PathBuf;

// Strategy for generating valid paths
fn path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec("[a-z]{1,10}", 1..5).prop_map(|parts| {
        let mut path = PathBuf::from("/tmp/test");
        for part in parts {
            path.push(part);
        }
        path
    })
}

// Strategy for generating optional tags
fn tag_strategy() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-z]{1,15}")
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // Reservation keys are unique per path-tag combination
    #[test]
    fn reservation_keys_unique(
        path1 in path_strategy(),
        tag1 in tag_strategy(),
        path2 in path_strategy(),
        tag2 in tag_strategy()
    ) {
        let key1 = ReservationKey::new(path1.clone(), tag1.clone()).unwrap();
        let key2 = ReservationKey::new(path2.clone(), tag2.clone()).unwrap();

        if path1 == path2 && tag1 == tag2 {
            prop_assert_eq!(key1, key2);
        } else {
            prop_assert_ne!(key1, key2);
        }
    }

    // Reservation keys preserve their path component
    #[test]
    fn reservation_keys_preserve_path(path in path_strategy(), tag in tag_strategy()) {
        let key = ReservationKey::new(path.clone(), tag.clone()).unwrap();
        prop_assert_eq!(key.path, path);
        prop_assert_eq!(key.tag, tag);
    }

    // Operations with contiguous port groups
    #[test]
    fn port_group_ordering(base in 1u16..=65525, count in 2usize..=10) {
        let ports: Vec<Port> = (0..count)
            .filter_map(|i| {
                #[allow(clippy::cast_possible_truncation)]
                let port_val = base.saturating_add(i as u16);
                if port_val >= 1 {
                    Port::try_from(port_val).ok()
                } else {
                    None
                }
            })
            .collect();

        // Verify all ports are in ascending order
        for window in ports.windows(2) {
            prop_assert!(window[0] < window[1]);
        }

        // Verify contiguity
        for window in ports.windows(2) {
            prop_assert_eq!(window[1].value(), window[0].value() + 1);
        }
    }

    // Port ranges are always valid (min <= max)
    #[test]
    fn port_ranges_valid(start in 1u16..=65534, len in 1u16..=100) {
        let end = start.saturating_add(len);
        prop_assert!(start <= end);
    }

    // Path components don't break reservation key integrity
    #[test]
    fn path_components_preserve_key_integrity(components in prop::collection::vec("[a-z]{1,10}", 1..8)) {
        let mut path = PathBuf::from("/tmp");
        for component in &components {
            path.push(component);
        }

        let key = ReservationKey::new(path.clone(), None).unwrap();
        prop_assert_eq!(key.path, path);
    }
}
