//! Property-based tests for `Reservation` and `ReservationKey` types.

use super::{Reservation, ReservationKey};
use crate::Port;
use proptest::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

// Strategy for generating valid paths
fn path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec("[a-z]{1,10}", 1..5).prop_map(|parts| {
        let mut path = PathBuf::from("/tmp");
        for part in parts {
            path.push(part);
        }
        path
    })
}

// Strategy for generating optional strings (for project/task)
fn optional_string_strategy() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-z]{1,20}")
}

// Strategy for generating optional tags (consolidated from tag_strategy)
fn optional_tag_strategy() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-z]{1,15}")
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // ReservationKey uniqueness based on path and tag
    #[test]
    fn reservation_key_uniqueness(path1 in path_strategy(), tag1 in optional_tag_strategy(), path2 in path_strategy(), tag2 in optional_tag_strategy()) {
        let key1 = ReservationKey::new(path1.clone(), tag1.clone()).unwrap();
        let key2 = ReservationKey::new(path2.clone(), tag2.clone()).unwrap();

        if path1 == path2 && tag1 == tag2 {
            prop_assert_eq!(key1, key2);
        } else {
            prop_assert_ne!(key1, key2);
        }
    }

    // Hash stability - same key always produces same hash
    #[test]
    fn reservation_key_hash_stable(path in path_strategy(), tag in optional_tag_strategy()) {
        let key = ReservationKey::new(path, tag).unwrap();

        let mut hasher1 = DefaultHasher::new();
        key.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        key.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        prop_assert_eq!(hash1, hash2);
    }

    // Port is always within valid range
    #[test]
    fn reservation_port_always_valid(
        path in path_strategy(),
        tag in optional_tag_strategy(),
        port in 1u16..=65535,
        project in optional_string_strategy(),
        task in optional_string_strategy()
    ) {
        let key = ReservationKey::new(path, tag).unwrap();
        let p = Port::try_from(port).unwrap();

        let reservation = Reservation::builder(key, p)
            .project(project)
            .task(task)
            .build();

        prop_assert!(reservation.is_ok());
        prop_assert_eq!(reservation.unwrap().port().value(), port);
    }

    // Reservation key display format is consistent
    #[test]
    fn reservation_key_display_consistent(path in path_strategy(), tag in optional_tag_strategy()) {
        let key = ReservationKey::new(path.clone(), tag.clone()).unwrap();
        let display = format!("{key}");

        if let Some(tag_str) = tag {
            prop_assert!(display.contains(&tag_str));
            prop_assert!(display.contains(':'));
        }
        prop_assert!(display.contains(&path.to_string_lossy().to_string()));
    }

    // Reservations preserve their key and port
    #[test]
    fn reservation_preserves_key_and_port(
        path in path_strategy(),
        tag in optional_tag_strategy(),
        port in 1u16..=65535,
        sticky in any::<bool>()
    ) {
        let key = ReservationKey::new(path, tag).unwrap();
        let p = Port::try_from(port).unwrap();

        let reservation = Reservation::builder(key.clone(), p)
            .sticky(sticky)
            .build()
            .unwrap();

        prop_assert_eq!(reservation.key(), &key);
        prop_assert_eq!(reservation.port(), p);
        prop_assert_eq!(reservation.sticky(), sticky);
    }

    // Empty project/task strings are rejected
    #[test]
    fn reservation_rejects_empty_project_task(
        path in path_strategy(),
        tag in optional_tag_strategy(),
        port in 1u16..=65535,
        empty_type in 0u8..2
    ) {
        let key = ReservationKey::new(path, tag).unwrap();
        let p = Port::try_from(port).unwrap();

        let builder = Reservation::builder(key, p);

        let result = match empty_type {
            0 => builder.project(Some(String::new())).build(),
            _ => builder.task(Some(String::new())).build(),
        };

        prop_assert!(result.is_err());
    }

    // Whitespace in project/task is trimmed
    #[test]
    fn reservation_trims_project_task(
        path in path_strategy(),
        tag in optional_tag_strategy(),
        port in 1u16..=65535,
        project in "[a-z]{1,10}"
    ) {
        let key = ReservationKey::new(path, tag).unwrap();
        let p = Port::try_from(port).unwrap();

        let padded_project = format!("  {project}  ");
        let reservation = Reservation::builder(key, p)
            .project(Some(padded_project))
            .build()
            .unwrap();

        prop_assert_eq!(reservation.project(), Some(project.as_str()));
    }

    // Reservation serialization round-trips
    #[test]
    fn reservation_serialization_roundtrip(
        path in path_strategy(),
        tag in optional_tag_strategy(),
        port in 1u16..=65535,
        project in optional_string_strategy(),
        task in optional_string_strategy(),
        sticky in any::<bool>()
    ) {
        let key = ReservationKey::new(path, tag).unwrap();
        let p = Port::try_from(port).unwrap();

        let reservation = Reservation::builder(key, p)
            .project(project)
            .task(task)
            .sticky(sticky)
            .build()
            .unwrap();

        let json = serde_json::to_string(&reservation).unwrap();
        let deserialized: Reservation = serde_json::from_str(&json).unwrap();

        prop_assert_eq!(deserialized, reservation);
    }
}
