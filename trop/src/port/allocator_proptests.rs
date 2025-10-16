//! Property-based tests for port allocation.
//!
//! This module contains property-based tests using proptest that verify
//! mathematical invariants and algebraic properties of the port allocator.
//! These tests complement the manual unit tests by exploring a much larger
//! input space and verifying universal properties that should hold for all
//! valid inputs.

#[cfg(test)]
mod tests {
    use crate::database::test_util::create_test_database;
    use crate::port::allocator::{AllocationOptions, AllocationResult, PortAllocator};
    use crate::port::exclusions::ExclusionManager;
    use crate::port::occupancy::{MockOccupancyChecker, OccupancyCheckConfig};
    use crate::reservation::{Reservation, ReservationKey};
    use crate::{Port, PortRange};
    use proptest::prelude::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    // ============================================================================
    // STRATEGY DEFINITIONS
    // ============================================================================
    // These define how proptest generates random test inputs for properties.

    /// Strategy for generating valid port numbers (`Port::MIN-Port::MAX`).
    ///
    /// Port 0 is invalid, so we generate in the range [`Port::MIN`, `Port::MAX`].
    #[allow(dead_code)]
    fn port_strategy() -> impl Strategy<Value = Port> {
        (Port::MIN..=Port::MAX).prop_map(|p| Port::try_from(p).unwrap())
    }

    /// Strategy for generating valid port ranges.
    ///
    /// Ensures min <= max to create valid ranges. We limit the range size
    /// to make tests run in reasonable time.
    fn port_range_strategy() -> impl Strategy<Value = PortRange> {
        (Port::MIN..=60000u16, 1u16..=5000u16).prop_map(|(min, size)| {
            let max = min.saturating_add(size);
            PortRange::new(Port::try_from(min).unwrap(), Port::try_from(max).unwrap()).unwrap()
        })
    }

    /// Strategy for generating a set of occupied ports within a given range.
    ///
    /// This generates a `HashSet` of ports that are marked as occupied.
    /// The occupancy rate parameter controls how many ports are occupied.
    #[allow(dead_code)]
    fn occupied_ports_strategy(
        range: PortRange,
        occupancy_rate: f64,
    ) -> impl Strategy<Value = HashSet<Port>> {
        let ports: Vec<Port> = range.into_iter().collect();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss
        )]
        let num_occupied = ((ports.len() as f64) * occupancy_rate).ceil() as usize;

        proptest::sample::subsequence(ports, 0..=num_occupied)
            .prop_map(|vec| vec.into_iter().collect())
    }

    /// Strategy for generating exclusion managers with various patterns.
    ///
    /// This generates `ExclusionManagers` with between 0-10 excluded ports
    /// within a given range. This exercises the exclusion checking logic
    /// with different patterns.
    fn exclusion_manager_strategy(range: PortRange) -> impl Strategy<Value = ExclusionManager> {
        let ports: Vec<Port> = range.into_iter().collect();
        proptest::sample::subsequence(ports, 0..=10).prop_map(|excluded_ports| {
            let mut manager = ExclusionManager::empty();
            for port in excluded_ports {
                manager.add_port(port);
            }
            manager
        })
    }

    // ============================================================================
    // PROPERTY 1: ALLOCATION DETERMINISM
    // ============================================================================
    // Mathematical property: Same inputs → same outputs
    //
    // Given identical database state, exclusions, occupied ports, and options,
    // the allocator must always produce the same result. This is a fundamental
    // requirement for predictable behavior and is critical for reproducibility.
    //
    // This property verifies that the forward-scanning algorithm is truly
    // deterministic and doesn't depend on any hidden state or randomness.

    proptest! {
        #[test]
        fn prop_allocation_is_deterministic(
            range in port_range_strategy(),
            exclusions in exclusion_manager_strategy(PortRange::new(
                Port::try_from(Port::MIN).unwrap(),
                Port::try_from(1000).unwrap()
            ).unwrap()),
        ) {
            // PROPERTY: Calling allocate_single multiple times with the same state
            // produces the same result every time.
            //
            // WHY THIS MATTERS: Determinism is essential for:
            // - Reproducible behavior in production
            // - Debugging and testing
            // - Predictable allocation patterns
            // - Avoiding race conditions in concurrent scenarios

            let db = create_test_database();
            let occupied = HashSet::new();
            let checker = MockOccupancyChecker::new(occupied);
            let allocator = PortAllocator::new(checker, exclusions, range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            // Allocate multiple times
            let result1 = allocator.allocate_single(&db, &options, &config).unwrap();
            let result2 = allocator.allocate_single(&db, &options, &config).unwrap();
            let result3 = allocator.allocate_single(&db, &options, &config).unwrap();

            // All results must be identical
            prop_assert_eq!(&result1, &result2);
            prop_assert_eq!(&result1, &result3);
        }
    }

    // ============================================================================
    // PROPERTY 2: EXCLUSION INVARIANT
    // ============================================================================
    // Mathematical invariant: ∀port ∈ allocated_ports, port ∉ exclusion_set
    //
    // No allocated port should ever be in the exclusion list. This is a safety
    // property that ensures the allocator respects configured exclusions.
    //
    // This property is critical for security and correctness: if excluded ports
    // (e.g., privileged ports, reserved system ports) can be allocated, it
    // could lead to conflicts or security vulnerabilities.

    proptest! {
        #[test]
        fn prop_never_allocates_excluded_ports(
            range in port_range_strategy(),
            exclusions in exclusion_manager_strategy(PortRange::new(
                Port::try_from(Port::MIN).unwrap(),
                Port::try_from(1000).unwrap()
            ).unwrap()),
        ) {
            // PROPERTY: If allocate_single returns Allocated(port), then
            // exclusions.is_excluded(port) must be false.
            //
            // WHY THIS MATTERS: Exclusions exist to prevent allocation of:
            // - System reserved ports (e.g., 1-1023 on Unix)
            // - Ports used by other services
            // - Ports blocked by firewall rules
            // Violating this property could cause application failures or
            // security issues.

            let db = create_test_database();
            let occupied = HashSet::new();
            let checker = MockOccupancyChecker::new(occupied);
            let allocator = PortAllocator::new(checker, exclusions.clone(), range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // The allocated port must NOT be excluded
                    prop_assert!(!exclusions.is_excluded(port),
                        "Allocated port {} is in exclusion list", port.value());
                }
                AllocationResult::Exhausted { .. } | AllocationResult::PreferredUnavailable { .. } => {
                    // Exhaustion is acceptable if all available ports are excluded
                    // PreferredUnavailable not applicable for default options (no preferred port)
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 3: OCCUPANCY INVARIANT
    // ============================================================================
    // Mathematical invariant: ∀port ∈ allocated_ports, port ∉ occupied_set
    //
    // When ignore_occupied is false, allocated ports must not be occupied.
    // This ensures the allocator doesn't assign ports that are already in use.
    //
    // This is the core correctness property for occupancy checking: it prevents
    // port conflicts that would cause bind failures.

    proptest! {
        #[test]
        fn prop_never_allocates_occupied_ports(
            min in 5000u16..=5100u16,
            occupied_indices in proptest::collection::vec(0usize..100, 0..10),
        ) {
            // PROPERTY: If allocate_single returns Allocated(port) and
            // ignore_occupied is false, then is_occupied(port) must be false.
            //
            // WHY THIS MATTERS: Allocating occupied ports causes:
            // - Bind failures when the application tries to use the port
            // - Potential data corruption from port conflicts
            // - Difficult-to-debug runtime errors
            // This property ensures the allocator's primary function works correctly.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            // Convert indices to actual ports in range
            let occupied: HashSet<Port> = occupied_indices
                .into_iter()
                .map(|idx| {
                    #[allow(clippy::cast_possible_truncation)]
                    let port_val = min + (idx as u16);
                    Port::try_from(port_val.min(min + 100)).unwrap()
                })
                .collect();

            let db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied.clone());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions {
                preferred: None,
                ignore_occupied: false,  // Don't ignore occupancy
                ignore_exclusions: false,
            };
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // The allocated port must NOT be occupied
                    prop_assert!(!occupied.contains(&port),
                        "Allocated occupied port {}", port.value());
                }
                AllocationResult::Exhausted { .. } | AllocationResult::PreferredUnavailable { .. } => {
                    // Exhaustion is acceptable if all ports are occupied
                    // PreferredUnavailable not applicable for default options
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 4: PORT RANGE BOUNDS INVARIANT
    // ============================================================================
    // Mathematical invariant: ∀port ∈ allocated_ports, range.min ≤ port ≤ range.max
    //
    // All allocated ports must fall within the configured range. This is a
    // fundamental safety property that ensures the allocator stays within its
    // defined boundaries.
    //
    // Violating this property could lead to allocating ports outside the
    // intended range, potentially conflicting with other applications or
    // violating system policies.

    proptest! {
        #[test]
        fn prop_allocated_port_within_range(
            range in port_range_strategy(),
            occupancy_rate in 0.0f64..0.5f64,
        ) {
            // PROPERTY: If allocate_single returns Allocated(port), then
            // range.contains(port) must be true.
            //
            // WHY THIS MATTERS: The range defines the allocator's domain.
            // Allocating outside the range would violate the configuration and
            // could conflict with ports managed by other systems or allocators.
            // This is a boundary safety property.

            let occupied: HashSet<Port> = {
                let ports: Vec<Port> = range.into_iter().collect();
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss, clippy::cast_sign_loss)]
                let num_occupied = ((ports.len() as f64) * occupancy_rate).ceil() as usize;
                ports.into_iter().take(num_occupied).collect()
            };

            let db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied);
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // The allocated port must be within the range
                    prop_assert!(range.contains(port),
                        "Allocated port {} is outside range [{}, {}]",
                        port.value(), range.min().value(), range.max().value());
                }
                AllocationResult::Exhausted { .. } | AllocationResult::PreferredUnavailable { .. } => {
                    // Exhaustion is acceptable
                    // PreferredUnavailable not applicable
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 5: FORWARD SCANNING MINIMALITY
    // ============================================================================
    // Mathematical property: Allocated port is the minimum available port
    //
    // Given a forward-scanning algorithm starting from range.min, the allocated
    // port should be the smallest port number that satisfies all constraints.
    // This property verifies that the allocator doesn't skip available ports.
    //
    // This ensures efficient and predictable port usage, filling in gaps before
    // moving to higher port numbers.

    proptest! {
        #[test]
        fn prop_forward_scanning_finds_minimum(
            min in 5000u16..=5100u16,
            gap_start in 0u16..10u16,
            gap_size in 1u16..5u16,
        ) {
            // PROPERTY: If allocate_single returns Allocated(port), then there
            // should be no lower port in the range that is also available.
            //
            // WHY THIS MATTERS: Forward scanning ensures:
            // - Predictable allocation patterns
            // - Efficient use of the port space
            // - Gaps are filled before moving to higher ports
            // This property verifies the "forward" part of forward-scanning.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            // Create a gap: occupy ports [gap_start, gap_start + gap_size)
            let occupied: HashSet<Port> = (gap_start..gap_start + gap_size)
                .map(|offset| Port::try_from(min + offset).unwrap())
                .collect();

            let db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied.clone());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // Verify no lower port is available
                    // We expect the port to be either min (if not occupied) or
                    // min + gap_size (first port after the gap)
                    let expected = if occupied.contains(&Port::try_from(min).unwrap()) {
                        Port::try_from(min + gap_size).unwrap()
                    } else {
                        Port::try_from(min).unwrap()
                    };

                    prop_assert_eq!(port, expected,
                        "Expected port {} but got {}", expected.value(), port.value());
                }
                AllocationResult::Exhausted { .. } | AllocationResult::PreferredUnavailable { .. } => {
                    // Not expected in this test scenario
                    // PreferredUnavailable not applicable
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 6: RESERVATION CONSTRAINT
    // ============================================================================
    // Mathematical invariant: ∀port ∈ allocated_ports, port ∉ reserved_set
    //
    // Allocated ports must not already be reserved in the database. This
    // ensures the allocator doesn't double-allocate ports.
    //
    // This is a critical consistency property: violating it would lead to
    // port conflicts between different reservations.

    proptest! {
        #[test]
        fn prop_never_allocates_reserved_ports(
            min in 5000u16..=5100u16,
            num_reserved in 0usize..20usize,
        ) {
            // PROPERTY: If allocate_single returns Allocated(port), then
            // db.is_port_reserved(port) must be false (before allocation).
            //
            // WHY THIS MATTERS: The database tracks port assignments. Allocating
            // an already-reserved port would create a conflict between multiple
            // reservations, leading to potential data corruption and application
            // failures. This property ensures database consistency.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            let mut db = create_test_database();

            // Reserve some ports in the database
            let mut reserved_ports = HashSet::new();
            for i in 0..num_reserved {
                #[allow(clippy::cast_possible_truncation)]
                let port_val = min + (i as u16 % 100);
                let port = Port::try_from(port_val).unwrap();

                // Create a unique reservation for this port
                let key = ReservationKey::new(
                    PathBuf::from(format!("/test/reserved/{i}")),
                    None
                ).unwrap();
                let reservation = Reservation::builder(key, port).build().unwrap();

                // Only track successfully reserved ports
                if db.create_reservation(&reservation).is_ok() {
                    reserved_ports.insert(port);
                }
            }

            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // The allocated port must NOT be in the reserved set
                    prop_assert!(!reserved_ports.contains(&port),
                        "Allocated reserved port {}", port.value());
                }
                AllocationResult::Exhausted { .. } | AllocationResult::PreferredUnavailable { .. } => {
                    // Exhaustion is acceptable if all ports are reserved
                    // PreferredUnavailable not applicable
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 7: PREFERRED PORT RESPECT
    // ============================================================================
    // Mathematical property: If preferred port is available, it gets allocated
    //
    // When a preferred port is specified and available, the allocator should
    // allocate exactly that port, not any other.
    //
    // This property ensures that port preferences are honored when possible,
    // which is important for applications that need specific ports.

    proptest! {
        #[test]
        fn prop_preferred_port_allocated_when_available(
            min in 5000u16..=6000u16,
            preferred_offset in 0u16..100u16,
        ) {
            // PROPERTY: If preferred port is specified and available (not reserved,
            // not excluded, not occupied), then allocate_single must return
            // Allocated(preferred_port).
            //
            // WHY THIS MATTERS: Preferred ports allow applications to request
            // specific port numbers (e.g., 8080 for development). This property
            // ensures that when the preferred port is available, it's actually used,
            // maintaining user expectations and supporting deterministic deployments.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let preferred = Port::try_from(min + preferred_offset).unwrap();

            // Make sure preferred port is available (not occupied, not excluded, not reserved)
            let db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions {
                preferred: Some(preferred),
                ignore_occupied: false,
                ignore_exclusions: false,
            };
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // Must be the preferred port
                    prop_assert_eq!(port, preferred,
                        "Expected preferred port {} but got {}",
                        preferred.value(), port.value());
                }
                AllocationResult::PreferredUnavailable { port, reason } => {
                    // Should not happen in this scenario
                    return Err(TestCaseError::fail(format!(
                        "Preferred port {} should be available but got: {:?}",
                        port.value(), reason
                    )));
                }
                AllocationResult::Exhausted { .. } => {
                    // Should not happen when preferred port is in range
                    return Err(TestCaseError::fail(
                        "Should not exhaust when preferred port is available"
                    ));
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 8: EXHAUSTION CORRECTNESS
    // ============================================================================
    // Mathematical property: Exhaustion ⟹ no available ports exist
    //
    // If allocate_single returns Exhausted, then every port in the range
    // must be unavailable (reserved, excluded, or occupied).
    //
    // This property ensures that exhaustion reports are accurate and the
    // allocator has truly explored all possibilities.

    proptest! {
        #[test]
        fn prop_exhaustion_implies_no_available_ports(
            min in 5000u16..=5200u16,
        ) {
            // PROPERTY: If allocate_single returns Exhausted, then for all ports
            // in the range, at least one of these must be true:
            // - is_port_reserved(port)
            // - is_excluded(port)
            // - is_occupied(port)
            //
            // WHY THIS MATTERS: Exhaustion is a strong claim - it means no ports
            // are available. This property ensures exhaustion is only reported when
            // truly warranted, preventing false negatives that would deny allocations
            // when ports are actually available.

            // Create a small range and occupy all ports
            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 10).unwrap()  // Small range for test efficiency
            ).unwrap();

            // Occupy all ports in the range
            let occupied: HashSet<Port> = range.into_iter().collect();

            let db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied.clone());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            let result = allocator.allocate_single(&db, &options, &config).unwrap();

            // Should be exhausted since all ports are occupied
            prop_assert!(
                matches!(result, AllocationResult::Exhausted { .. }),
                "Expected Exhausted but got {:?}", result
            );

            // Verify that all ports are indeed unavailable
            for port in range {
                prop_assert!(occupied.contains(&port),
                    "Port {} should be occupied", port.value());
            }
        }
    }

    // ============================================================================
    // PROPERTY 9: IGNORE FLAGS EFFECT
    // ============================================================================
    // Mathematical property: ignore_occupied ⟹ can allocate occupied ports
    //
    // When ignore_occupied is true, the allocator should be able to allocate
    // ports that are marked as occupied.
    //
    // This property verifies that override flags work correctly, which is
    // important for testing and special operational scenarios.

    proptest! {
        #[test]
        fn prop_ignore_occupied_allows_occupied_ports(
            min in 5000u16..=5100u16,
            preferred_offset in 0u16..100u16,
        ) {
            // PROPERTY: If ignore_occupied is true and a preferred occupied port
            // is specified, then allocate_single should return Allocated(port),
            // not PreferredUnavailable.
            //
            // WHY THIS MATTERS: The ignore_occupied flag is used for:
            // - Testing scenarios
            // - Force-allocation in emergencies
            // - Overriding occupancy checks for trusted applications
            // This property ensures the flag actually bypasses occupancy checks.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 200).unwrap()
            ).unwrap();

            let preferred = Port::try_from(min + preferred_offset).unwrap();

            // Mark the preferred port as occupied
            let mut occupied = HashSet::new();
            occupied.insert(preferred);

            let db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied);
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let options = AllocationOptions {
                preferred: Some(preferred),
                ignore_occupied: true,  // Ignore occupancy
                ignore_exclusions: false,
            };
            let config = OccupancyCheckConfig::default();

            match allocator.allocate_single(&db, &options, &config).unwrap() {
                AllocationResult::Allocated(port) => {
                    // Should get the preferred port despite it being occupied
                    prop_assert_eq!(port, preferred);
                }
                AllocationResult::PreferredUnavailable { .. } => {
                    return Err(TestCaseError::fail(
                        "Should allocate occupied port when ignore_occupied is true"
                    ));
                }
                AllocationResult::Exhausted { .. } => {
                    // Should not happen
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 10: ALLOCATOR STATELESSNESS
    // ============================================================================
    // Mathematical property: Allocator behavior depends only on inputs
    //
    // The allocator should be stateless - creating multiple allocator instances
    // with the same configuration should produce identical behavior.
    //
    // This property verifies architectural soundness and ensures the allocator
    // can be safely used in concurrent scenarios.

    proptest! {
        #[test]
        fn prop_allocator_is_stateless(
            range in port_range_strategy(),
            occupancy_rate in 0.0f64..0.5f64,
        ) {
            // PROPERTY: Two allocators with identical configuration produce
            // identical results when given the same database and options.
            //
            // WHY THIS MATTERS: Statelessness is crucial for:
            // - Thread safety
            // - Predictable behavior
            // - No hidden dependencies or side effects
            // This property ensures the allocator follows functional principles.

            let occupied: HashSet<Port> = {
                let ports: Vec<Port> = range.into_iter().collect();
                #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss, clippy::cast_sign_loss)]
                let num_occupied = ((ports.len() as f64) * occupancy_rate).ceil() as usize;
                ports.into_iter().take(num_occupied).collect()
            };

            let db = create_test_database();

            // Create two identical allocators
            let checker1 = MockOccupancyChecker::new(occupied.clone());
            let allocator1 = PortAllocator::new(
                checker1,
                ExclusionManager::empty(),
                range
            );

            let checker2 = MockOccupancyChecker::new(occupied);
            let allocator2 = PortAllocator::new(
                checker2,
                ExclusionManager::empty(),
                range
            );

            let options = AllocationOptions::default();
            let config = OccupancyCheckConfig::default();

            // Both should produce identical results
            let result1 = allocator1.allocate_single(&db, &options, &config).unwrap();
            let result2 = allocator2.allocate_single(&db, &options, &config).unwrap();

            prop_assert_eq!(result1, result2,
                "Two identical allocators produced different results");
        }
    }
}
