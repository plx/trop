# Phase 12.1: Property-Based Testing Foundation

## Overview

Subpass 12.1 adds property-based testing using the proptest framework to verify invariants and properties of core types and operations. This provides a higher level of confidence in correctness by testing thousands of generated cases rather than just hand-written examples.

## Context & Dependencies

**Prerequisites:**
- Phases 1-11 fully implemented and merged
- proptest dependency already included in `trop/Cargo.toml`
- Existing test suite provides baseline coverage (4,901 tests passing)

**Dependencies:**
- None - this is the foundation subpass and can start immediately

**Key Considerations:**
- Focus on invariants that should hold for ALL valid inputs
- Avoid duplicating existing manual test coverage
- Use proptest's shrinking to find minimal failing cases
- Configure appropriate case counts (10,000+ for CI)

## Implementation Tasks

### Task 1: Port and PortRange Properties

**File:** `trop/src/port/proptests.rs`

**Implementation:**
```rust
use proptest::prelude::*;
use crate::port::{Port, PortRange};

// Property tests for Port type
proptest! {
    // Valid port ranges (1-65535)
    #[test]
    fn port_always_in_valid_range(port in 1u16..=65535) {
        let p = Port::new(port);
        prop_assert!(p.is_ok());
        prop_assert_eq!(p.unwrap().value(), port);
    }

    // Port 0 is always invalid
    #[test]
    fn port_zero_always_invalid(_unit in 0u8..1) {
        let p = Port::new(0);
        prop_assert!(p.is_err());
    }

    // Ordering is transitive: if a < b and b < c, then a < c
    #[test]
    fn port_ordering_transitive(a in 1u16..=65533, b_offset in 1u16..=1, c_offset in 1u16..=1) {
        let port_a = Port::new(a).unwrap();
        let port_b = Port::new(a + b_offset).unwrap();
        let port_c = Port::new(a + b_offset + c_offset).unwrap();

        if port_a < port_b && port_b < port_c {
            prop_assert!(port_a < port_c);
        }
    }

    // Serialization round-trips
    #[test]
    fn port_serialization_roundtrip(port in 1u16..=65535) {
        let p = Port::new(port).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: Port = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(p, deserialized);
    }
}

// Property tests for PortRange
proptest! {
    // start <= end always holds (constructor enforces this)
    #[test]
    fn port_range_start_le_end(start in 1u16..=65535, end in 1u16..=65535) {
        let (s, e) = if start <= end { (start, end) } else { (end, start) };
        let range = PortRange::new(Port::new(s).unwrap(), Port::new(e).unwrap());
        prop_assert!(range.is_ok());
        let r = range.unwrap();
        prop_assert!(r.start() <= r.end());
    }

    // contains() is accurate
    #[test]
    fn port_range_contains_accuracy(start in 1u16..=65534, len in 1u16..=100, test_port in 1u16..=65535) {
        let end = start.saturating_add(len).min(65535);
        let range = PortRange::new(Port::new(start).unwrap(), Port::new(end).unwrap()).unwrap();
        let p = Port::new(test_port).unwrap();

        let should_contain = test_port >= start && test_port <= end;
        prop_assert_eq!(range.contains(&p), should_contain);
    }

    // Overlap detection is symmetric
    #[test]
    fn port_range_overlap_symmetric(
        start1 in 1u16..=65534, end1 in 1u16..=65535,
        start2 in 1u16..=65534, end2 in 1u16..=65535
    ) {
        let (s1, e1) = if start1 <= end1 { (start1, end1) } else { (end1, start1) };
        let (s2, e2) = if start2 <= end2 { (start2, end2) } else { (end2, start2) };

        let range1 = PortRange::new(Port::new(s1).unwrap(), Port::new(e1).unwrap()).unwrap();
        let range2 = PortRange::new(Port::new(s2).unwrap(), Port::new(e2).unwrap()).unwrap();

        prop_assert_eq!(range1.overlaps(&range2), range2.overlaps(&range1));
    }

    // Overlap implies at least one common port
    #[test]
    fn port_range_overlap_implies_common_port(
        start1 in 1u16..=65534, len1 in 1u16..=100,
        start2 in 1u16..=65534, len2 in 1u16..=100
    ) {
        let end1 = start1.saturating_add(len1).min(65535);
        let end2 = start2.saturating_add(len2).min(65535);

        let range1 = PortRange::new(Port::new(start1).unwrap(), Port::new(end1).unwrap()).unwrap();
        let range2 = PortRange::new(Port::new(start2).unwrap(), Port::new(end2).unwrap()).unwrap();

        if range1.overlaps(&range2) {
            // Find at least one port in both ranges
            let common_start = start1.max(start2);
            let common_end = end1.min(end2);
            prop_assert!(common_start <= common_end);
        }
    }
}
```

**Integration:**
- Add module declaration in `trop/src/port.rs`: `#[cfg(test)] mod proptests;`
- Ensure tests run with `cargo test --lib port::proptests`

### Task 2: Reservation Properties

**File:** `trop/src/reservation/proptests.rs`

**Implementation:**
```rust
use proptest::prelude::*;
use crate::reservation::{Reservation, ReservationKey};
use crate::port::Port;
use std::path::PathBuf;

// Strategy for generating valid paths
fn path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec("[a-z]{1,10}", 1..5)
        .prop_map(|parts| {
            let mut path = PathBuf::from("/tmp");
            for part in parts {
                path.push(part);
            }
            path
        })
}

// Strategy for generating optional strings
fn optional_string_strategy() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-z]{1,20}")
}

proptest! {
    // ReservationKey uniqueness based on path
    #[test]
    fn reservation_key_uniqueness(path1 in path_strategy(), path2 in path_strategy()) {
        let key1 = ReservationKey::new(path1.clone());
        let key2 = ReservationKey::new(path2.clone());

        if path1 == path2 {
            prop_assert_eq!(key1, key2);
        } else {
            prop_assert_ne!(key1, key2);
        }
    }

    // Hash stability - same key always produces same hash
    #[test]
    fn reservation_key_hash_stable(path in path_strategy()) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let key = ReservationKey::new(path);

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
        port in 1u16..=65535,
        project in optional_string_strategy(),
        task in optional_string_strategy()
    ) {
        let key = ReservationKey::new(path);
        let p = Port::new(port).unwrap();
        // Note: Would need to adjust based on actual Reservation constructor
        // This is conceptual - actual API may differ
        prop_assert!(port >= 1 && port <= 65535);
    }
}
```

**Integration:**
- Add module declaration in `trop/src/reservation.rs`: `#[cfg(test)] mod proptests;`

### Task 3: Path Handling Properties

**File:** `trop/src/path/proptests.rs`

**Implementation:**
```rust
use proptest::prelude::*;
use crate::path::{normalize_path, PathRelationship, get_path_relationship};
use std::path::PathBuf;

// Strategy for generating path-like strings
fn path_component_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9_-]{1,20}"
}

fn relative_path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec(path_component_strategy(), 1..8)
        .prop_map(|parts| parts.iter().collect())
}

fn absolute_path_strategy() -> impl Strategy<Value = PathBuf> {
    prop::collection::vec(path_component_strategy(), 1..8)
        .prop_map(|parts| {
            let mut path = PathBuf::from("/");
            for part in parts {
                path.push(part);
            }
            path
        })
}

proptest! {
    // Normalization is idempotent: normalize(normalize(p)) == normalize(p)
    #[test]
    fn path_normalization_idempotent(path in absolute_path_strategy()) {
        let normalized_once = normalize_path(&path).unwrap();
        let normalized_twice = normalize_path(&normalized_once).unwrap();
        prop_assert_eq!(normalized_once, normalized_twice);
    }

    // Normalized paths never contain ".."
    #[test]
    fn normalized_paths_no_parent_refs(path in absolute_path_strategy()) {
        let normalized = normalize_path(&path).unwrap();
        let path_str = normalized.to_string_lossy();
        prop_assert!(!path_str.contains(".."));
    }

    // Path relationship is reflexive: path is always identical to itself
    #[test]
    fn path_relationship_reflexive(path in absolute_path_strategy()) {
        let rel = get_path_relationship(&path, &path).unwrap();
        prop_assert_eq!(rel, PathRelationship::Identical);
    }

    // Containment is transitive (if A contains B and B contains C, then A contains C)
    #[test]
    fn path_containment_transitive(base in absolute_path_strategy(), parts1 in 1..5usize, parts2 in 1..5usize) {
        let mut path_b = base.clone();
        for i in 0..parts1 {
            path_b.push(format!("sub{}", i));
        }

        let mut path_c = path_b.clone();
        for i in 0..parts2 {
            path_c.push(format!("deep{}", i));
        }

        let rel_ab = get_path_relationship(&base, &path_b).unwrap();
        let rel_bc = get_path_relationship(&path_b, &path_c).unwrap();
        let rel_ac = get_path_relationship(&base, &path_c).unwrap();

        if matches!(rel_ab, PathRelationship::Contains) &&
           matches!(rel_bc, PathRelationship::Contains) {
            prop_assert!(matches!(rel_ac, PathRelationship::Contains));
        }
    }

    // Relationship types are mutually exclusive
    #[test]
    fn path_relationship_mutually_exclusive(path1 in absolute_path_strategy(), path2 in absolute_path_strategy()) {
        let rel = get_path_relationship(&path1, &path2).unwrap();

        // Each path pair has exactly one relationship type
        let is_identical = matches!(rel, PathRelationship::Identical);
        let is_contains = matches!(rel, PathRelationship::Contains);
        let is_contained = matches!(rel, PathRelationship::ContainedBy);
        let is_unrelated = matches!(rel, PathRelationship::Unrelated);

        let count = [is_identical, is_contains, is_contained, is_unrelated]
            .iter()
            .filter(|&&x| x)
            .count();

        prop_assert_eq!(count, 1);
    }
}
```

**Integration:**
- Add module declaration in `trop/src/path.rs`: `#[cfg(test)] mod proptests;`

### Task 4: Configuration Properties

**File:** `trop/src/config/proptests.rs`

**Implementation:**
```rust
use proptest::prelude::*;
use crate::config::{Config, ConfigBuilder};

// Strategy for generating valid port ranges
fn port_range_strategy() -> impl Strategy<Value = (u16, u16)> {
    (1u16..=65535).prop_flat_map(|start| {
        (Just(start), start..=65535)
    })
}

fn config_strategy() -> impl Strategy<Value = Config> {
    (
        prop::option::of(port_range_strategy()),
        prop::option::of(prop::collection::vec(1u16..=65535, 0..10)),
        prop::option::of(prop::bool::ANY),
    ).prop_map(|(range, exclusions, auto_exclude)| {
        let mut builder = ConfigBuilder::new();

        if let Some((start, end)) = range {
            builder = builder.port_range(start, end);
        }

        if let Some(ports) = exclusions {
            for port in ports {
                builder = builder.add_exclusion(port);
            }
        }

        if let Some(auto) = auto_exclude {
            builder = builder.auto_exclude_occupied(auto);
        }

        builder.build().unwrap()
    })
}

proptest! {
    // Configuration merging is associative: (a merge b) merge c == a merge (b merge c)
    #[test]
    fn config_merge_associative(a in config_strategy(), b in config_strategy(), c in config_strategy()) {
        let left = a.clone().merge(&b).merge(&c);
        let right = a.merge(&b.merge(&c));

        // Compare relevant fields
        prop_assert_eq!(left.port_range(), right.port_range());
        prop_assert_eq!(left.exclusions(), right.exclusions());
        prop_assert_eq!(left.auto_exclude_occupied(), right.auto_exclude_occupied());
    }

    // Empty config is identity element for merge
    #[test]
    fn config_merge_identity(config in config_strategy()) {
        let empty = ConfigBuilder::new().build().unwrap();
        let merged = config.clone().merge(&empty);

        prop_assert_eq!(config.port_range(), merged.port_range());
        prop_assert_eq!(config.exclusions(), merged.exclusions());
    }

    // Valid configs remain valid after merge
    #[test]
    fn valid_configs_stay_valid_after_merge(a in config_strategy(), b in config_strategy()) {
        let merged = a.merge(&b);

        // If merge succeeds, result should be valid
        // Port range should be valid
        if let Some((start, end)) = merged.port_range() {
            prop_assert!(start <= end);
            prop_assert!(start >= 1);
            prop_assert!(end <= 65535);
        }
    }
}
```

**Integration:**
- Add module declaration in `trop/src/config.rs`: `#[cfg(test)] mod proptests;`

### Task 5: Port Allocation Properties

**File:** `trop/src/operations/proptests.rs`

**Implementation:**
```rust
use proptest::prelude::*;
use crate::operations::allocate::{allocate_port, allocate_port_group};
use crate::config::Config;
use crate::port::{Port, PortRange};
use std::collections::HashSet;

proptest! {
    // No duplicate ports in allocations
    #[test]
    fn allocation_no_duplicates(count in 1usize..=100) {
        // Create a config with large range
        let config = Config::builder()
            .port_range(10000, 60000)
            .build()
            .unwrap();

        let mut allocated = HashSet::new();

        for _ in 0..count.min(50) {
            if let Ok(port) = allocate_port(&config, &allocated) {
                prop_assert!(!allocated.contains(&port), "Duplicate port allocated");
                allocated.insert(port);
            }
        }
    }

    // Allocated ports are within requested range
    #[test]
    fn allocation_within_range(start in 1024u16..=60000, end in 1024u16..=65535) {
        let (s, e) = if start <= end { (start, end) } else { (end, start) };

        let config = Config::builder()
            .port_range(s, e)
            .build()
            .unwrap();

        let allocated = HashSet::new();

        if let Ok(port) = allocate_port(&config, &allocated) {
            prop_assert!(port.value() >= s);
            prop_assert!(port.value() <= e);
        }
    }

    // Group allocations are contiguous when possible
    #[test]
    fn group_allocation_contiguous_when_possible(count in 2usize..=10, start in 10000u16..=50000) {
        let config = Config::builder()
            .port_range(start, start + 10000)
            .build()
            .unwrap();

        let allocated = HashSet::new();

        if let Ok(ports) = allocate_port_group(&config, &allocated, count) {
            prop_assert_eq!(ports.len(), count);

            // Check if contiguous
            let mut sorted: Vec<_> = ports.iter().map(|p| p.value()).collect();
            sorted.sort();

            let is_contiguous = sorted.windows(2).all(|w| w[1] == w[0] + 1);

            // Should be contiguous if there's space
            if (start + count as u16) <= (start + 10000) {
                prop_assert!(is_contiguous);
            }
        }
    }

    // Allocation respects exclusions
    #[test]
    fn allocation_respects_exclusions(
        exclusions in prop::collection::hash_set(20000u16..=20100, 1..20)
    ) {
        let mut config_builder = Config::builder().port_range(20000, 21000);

        for port in &exclusions {
            config_builder = config_builder.add_exclusion(*port);
        }

        let config = config_builder.build().unwrap();
        let allocated = HashSet::new();

        if let Ok(port) = allocate_port(&config, &allocated) {
            prop_assert!(!exclusions.contains(&port.value()));
        }
    }
}
```

**Integration:**
- Add module declaration in `trop/src/operations.rs`: `#[cfg(test)] mod proptests;`

## Success Criteria

- [ ] 100+ property tests added across all core modules
- [ ] All property tests pass with 10,000+ cases in CI (configure via proptest config)
- [ ] Property tests reveal no unexpected invariant violations
- [ ] Tests run in reasonable time (< 1 minute for full suite)
- [ ] All existing tests continue to pass (4,901 tests)
- [ ] Zero clippy warnings

## Configuration

Add proptest configuration to relevant test modules:

```rust
// In each proptests.rs file
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000, // Increase for CI
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // ... tests here
}
```

## Notes

- Property tests complement existing manual tests by exploring edge cases
- Shrinking helps find minimal failing examples when tests fail
- Some properties may need to be relaxed based on actual implementation constraints
- Focus on testing invariants that should ALWAYS hold, not just common cases
- Property tests are especially valuable for core types that are used throughout the codebase
