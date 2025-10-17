//! Property-based tests for group port allocation.
//!
//! This module contains property-based tests that verify the atomic and
//! transactional properties of group allocation. Group allocation is more
//! complex than single allocation because it must satisfy multiple constraints
//! simultaneously across multiple ports.

#[cfg(test)]
mod tests {
    use crate::database::test_util::create_test_database;
    use crate::port::allocator::PortAllocator;
    use crate::port::exclusions::ExclusionManager;
    use crate::port::group::{GroupAllocationRequest, ServiceAllocationRequest};
    use crate::port::occupancy::{MockOccupancyChecker, OccupancyCheckConfig};
    use crate::{Port, PortRange};
    use proptest::prelude::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    // ============================================================================
    // STRATEGY DEFINITIONS
    // ============================================================================

    /// Strategy for generating service allocation requests with offsets.
    ///
    /// This creates services with varying offset patterns to test the
    /// pattern-matching algorithm.
    #[allow(dead_code)]
    fn service_request_strategy() -> impl Strategy<Value = Vec<ServiceAllocationRequest>> {
        proptest::collection::vec(
            (
                "[a-z]{3,8}",  // Tag name
                0u16..1000u16, // Offset
            ),
            1..=5, // Number of services
        )
        .prop_map(|services| {
            services
                .into_iter()
                .map(|(tag, offset)| ServiceAllocationRequest {
                    tag,
                    offset: Some(offset),
                    preferred: None,
                })
                .collect()
        })
    }

    // ============================================================================
    // PROPERTY 1: GROUP ALLOCATION ATOMICITY
    // ============================================================================
    // Mathematical property: All-or-nothing allocation
    //
    // When allocating a group, either all services get ports or none do.
    // There should never be partial allocations in the database after a
    // group allocation operation.
    //
    // This is a critical transactional property: partial allocations would
    // leave the system in an inconsistent state.

    proptest! {
        #[test]
        fn prop_group_allocation_is_atomic(
            min in 5000u16..=6000u16,
            num_services in 1usize..5usize,
        ) {
            // PROPERTY: After allocate_group completes (success or failure),
            // either all requested services have reservations or none do.
            // There must never be partial allocations in the database.
            //
            // WHY THIS MATTERS: Atomicity is fundamental for:
            // - Consistency: the database reflects either complete success or failure
            // - Reliability: no orphaned reservations from failed allocations
            // - Predictability: users can trust all-or-nothing semantics
            // Violating this property would require manual cleanup of partial state.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            // Create request with consecutive offsets
            let services: Vec<ServiceAllocationRequest> = (0..num_services)
                .map(|i| {
                    #[allow(clippy::cast_possible_truncation)]
                    let offset = i as u16;
                    ServiceAllocationRequest {
                        tag: format!("service{i}"),
                        offset: Some(offset),
                        preferred: None,
                    }
                })
                .collect();

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/group"),
                project: Some("test".to_string()),
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();
            let result = allocator.allocate_group(&mut db, &request, &config);

            // Count how many reservations exist in the database for this path
            let reservations = db.list_all_reservations().unwrap();
            let group_reservations: Vec<_> = reservations
                .iter()
                .filter(|r| r.key().path == PathBuf::from("/test/group"))
                .collect();

            match result {
                Ok(allocation_result) => {
                    // Success: all services should have reservations
                    prop_assert_eq!(
                        group_reservations.len(),
                        num_services,
                        "Expected {} reservations but found {}",
                        num_services,
                        group_reservations.len()
                    );

                    // Verify all services are in the result
                    prop_assert_eq!(
                        allocation_result.allocations.len(),
                        num_services,
                        "Result should contain all {} services",
                        num_services
                    );
                }
                Err(_) => {
                    // Failure: no reservations should exist
                    // Note: Current implementation may leave partial reservations
                    // due to lack of bulk transaction support. This is a known
                    // limitation documented in the code.
                    // For now, we check that either all or none exist.
                    prop_assert!(
                        group_reservations.is_empty() || group_reservations.len() == num_services,
                        "Partial allocation detected: {} of {} services have reservations",
                        group_reservations.len(),
                        num_services
                    );
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 2: OFFSET PATTERN CONSISTENCY
    // ============================================================================
    // Mathematical property: All ports respect offset pattern
    //
    // If group allocation succeeds with offset pattern [o1, o2, ..., on],
    // then there exists a base port such that all allocated ports equal
    // base + offset_i.
    //
    // This property ensures the pattern-matching algorithm correctly identifies
    // compatible base ports.

    proptest! {
        #[test]
        fn prop_group_allocation_respects_offset_pattern(
            min in 5000u16..=6000u16,
            offsets in proptest::collection::vec(0u16..100u16, 1..=5),
        ) {
            // PROPERTY: If allocate_group succeeds for services with offsets
            // [o1, o2, ..., on], then allocated_ports[i] - allocated_ports[0] == offsets[i]
            // for all i.
            //
            // WHY THIS MATTERS: Offset patterns allow microservices to reserve
            // related ports (e.g., web=base, api=base+1, metrics=base+100).
            // This property ensures the allocator correctly maintains these
            // relationships, which is critical for applications that depend on
            // specific port layouts.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services: Vec<ServiceAllocationRequest> = offsets
                .iter()
                .enumerate()
                .map(|(i, &offset)| ServiceAllocationRequest {
                    tag: format!("svc{i}"),
                    offset: Some(offset),
                    preferred: None,
                })
                .collect();

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/offsets"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();

            if let Ok(result) = allocator.allocate_group(&mut db, &request, &config) {
                // Verify base port exists
                let base = result.base_port.expect("Base port should exist for offset-based allocation");

                // Verify each service has correct offset from base
                for (i, &expected_offset) in offsets.iter().enumerate() {
                    let tag = format!("svc{i}");
                    let allocated = result.allocations.get(&tag)
                        .expect("Service should have allocation");

                    let actual_offset = allocated.value().saturating_sub(base.value());

                    prop_assert_eq!(
                        actual_offset,
                        expected_offset,
                        "Service {} should have offset {} but has offset {}",
                        tag,
                        expected_offset,
                        actual_offset
                    );
                }
            } else {
                // Failure is acceptable if range is exhausted
            }
        }
    }

    // ============================================================================
    // PROPERTY 3: GROUP ALLOCATION SKIPS OBSTACLES
    // ============================================================================
    // Mathematical property: Pattern matching finds available base port
    //
    // If there exists a valid base port (one where all offsets are available),
    // the allocator should find it, even if earlier base ports are blocked.
    //
    // This ensures the forward-scanning algorithm correctly handles gaps and
    // obstacles in the port space.

    proptest! {
        #[test]
        fn prop_group_allocation_skips_occupied_patterns(
            min in 5000u16..=5500u16,
            obstacle_base in 0u16..10u16,
        ) {
            // PROPERTY: If ports [base, base+1, base+2] are occupied but ports
            // [base+gap, base+gap+1, base+gap+2] are free, then group allocation
            // for offsets [0,1,2] should succeed and use base+gap.
            //
            // WHY THIS MATTERS: Partial occupancy is common in real environments.
            // The allocator must scan past blocked patterns to find available ones.
            // This property ensures resilience to fragmentation in the port space.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            // Block the first potential base port
            let blocked_base = min + obstacle_base;
            let mut occupied = HashSet::new();
            occupied.insert(Port::try_from(blocked_base).unwrap());
            occupied.insert(Port::try_from(blocked_base + 1).unwrap());
            occupied.insert(Port::try_from(blocked_base + 2).unwrap());

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(occupied);
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services = vec![
                ServiceAllocationRequest {
                    tag: "a".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "b".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "c".to_string(),
                    offset: Some(2),
                    preferred: None,
                },
            ];

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/skip"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();
            let result = allocator.allocate_group(&mut db, &request, &config);

            // Should succeed and skip the blocked pattern
            prop_assert!(result.is_ok(), "Allocation should succeed by skipping occupied pattern");

            if let Ok(alloc) = result {
                let base = alloc.base_port.expect("Should have base port");

                // Base should not be the blocked base
                prop_assert_ne!(
                    base.value(),
                    blocked_base,
                    "Should skip blocked base {}", blocked_base
                );

                // Base should be either before the block or after it
                prop_assert!(
                    base.value() < blocked_base || base.value() > blocked_base + 2,
                    "Base {} should avoid blocked range [{}, {}]",
                    base.value(), blocked_base, blocked_base + 2
                );
            }
        }
    }

    // ============================================================================
    // PROPERTY 4: UNIQUE TAG ENFORCEMENT
    // ============================================================================
    // Mathematical property: Duplicate tags cause validation error
    //
    // Group allocation requests with duplicate service tags should always
    // fail with a validation error, never succeed.
    //
    // This ensures data integrity and prevents ambiguous allocations.

    proptest! {
        #[test]
        fn prop_group_allocation_rejects_duplicate_tags(
            min in 5000u16..=6000u16,
            duplicate_tag in "[a-z]{3,8}",
        ) {
            // PROPERTY: If a GroupAllocationRequest contains duplicate service tags,
            // allocate_group must return a validation error.
            //
            // WHY THIS MATTERS: Duplicate tags would create ambiguity:
            // - Which service gets which port?
            // - How to reference services in the result?
            // This property ensures input validation catches configuration errors
            // early, before attempting allocation.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            // Create request with duplicate tags
            let services = vec![
                ServiceAllocationRequest {
                    tag: duplicate_tag.clone(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: duplicate_tag.clone(),  // Duplicate!
                    offset: Some(1),
                    preferred: None,
                },
            ];

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/dup"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();
            let result = allocator.allocate_group(&mut db, &request, &config);

            // Must fail with validation error
            prop_assert!(result.is_err(), "Duplicate tags should cause error");

            if let Err(e) = result {
                let error_msg = format!("{e}");
                prop_assert!(
                    error_msg.to_lowercase().contains("duplicate"),
                    "Error should mention 'duplicate', got: {}", error_msg
                );
            }
        }
    }

    // ============================================================================
    // PROPERTY 5: GROUP SIZE INVARIANT
    // ============================================================================
    // Mathematical property: |result.allocations| == |request.services|
    //
    // On success, the number of allocated ports must equal the number of
    // requested services.
    //
    // This is a completeness property ensuring all services are allocated.

    proptest! {
        #[test]
        fn prop_group_allocation_result_completeness(
            min in 5000u16..=6000u16,
            num_services in 1usize..=10usize,
        ) {
            // PROPERTY: If allocate_group succeeds, then
            // result.allocations.len() == request.services.len()
            //
            // WHY THIS MATTERS: Partial allocation would be a logic error.
            // This property ensures that successful allocation is truly complete -
            // every requested service gets exactly one port.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services: Vec<ServiceAllocationRequest> = (0..num_services)
                .map(|i| {
                    #[allow(clippy::cast_possible_truncation)]
                    let offset = i as u16;
                    ServiceAllocationRequest {
                        tag: format!("service_{i}"),
                        offset: Some(offset),
                        preferred: None,
                    }
                })
                .collect();

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/complete"),
                project: None,
                task: None,
                services: services.clone(),
            };

            let config = OccupancyCheckConfig::default();

            if let Ok(result) = allocator.allocate_group(&mut db, &request, &config) {
                // Result must have exactly the same number of allocations
                prop_assert_eq!(
                    result.allocations.len(),
                    num_services,
                    "Expected {} allocations, got {}",
                    num_services,
                    result.allocations.len()
                );

                // Verify each service has an allocation
                for service in services {
                    prop_assert!(
                        result.allocations.contains_key(&service.tag),
                        "Missing allocation for service '{}'",
                        service.tag
                    );
                }
            }
        }
    }

    // ============================================================================
    // PROPERTY 6: NO OVERLAP IN GROUP
    // ============================================================================
    // Mathematical property: Allocated ports are distinct
    //
    // Within a successful group allocation, all allocated ports must be
    // distinct - no two services should get the same port.
    //
    // This ensures proper isolation between services in a group.

    proptest! {
        #[test]
        fn prop_group_allocation_ports_are_distinct(
            min in 5000u16..=6000u16,
            offsets in proptest::collection::hash_set(0u16..100u16, 2..=5),
        ) {
            // PROPERTY: If allocate_group succeeds with UNIQUE offsets, then all ports in
            // result.allocations.values() are unique.
            //
            // WHY THIS MATTERS: Port conflicts within a group would cause:
            // - Bind failures when services start
            // - Data corruption from multiple services on same port
            // This property ensures proper isolation within service groups.
            //
            // NOTE: We use a HashSet for offsets to ensure uniqueness. If services
            // have the same offset, they will naturally get the same port from base+offset.
            // The allocator doesn't validate offset uniqueness - it's the user's
            // responsibility to provide meaningful offsets.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services: Vec<ServiceAllocationRequest> = offsets
                .iter()
                .enumerate()
                .map(|(i, &offset)| ServiceAllocationRequest {
                    tag: format!("svc{i}"),
                    offset: Some(offset),
                    preferred: None,
                })
                .collect();

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/distinct"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();

            if let Ok(result) = allocator.allocate_group(&mut db, &request, &config) {
                // Collect all allocated ports
                let ports: Vec<Port> = result.allocations.values().copied().collect();

                // Convert to set to check uniqueness
                let port_set: HashSet<Port> = ports.iter().copied().collect();

                prop_assert_eq!(
                    ports.len(),
                    port_set.len(),
                    "Allocated ports are not distinct: {:?}",
                    ports.iter().map(|p| p.value()).collect::<Vec<_>>()
                );
            }
        }
    }

    // ============================================================================
    // PROPERTY 7: EMPTY SERVICES REJECTION
    // ============================================================================
    // Mathematical property: Empty service list causes validation error
    //
    // Group allocation with zero services should always fail validation.
    //
    // This ensures meaningful requests and prevents degenerate cases.

    proptest! {
        #[test]
        fn prop_group_allocation_rejects_empty_services(
            min in 5000u16..=6000u16,
        ) {
            // PROPERTY: If request.services is empty, allocate_group must
            // return a validation error.
            //
            // WHY THIS MATTERS: An empty group allocation is meaningless and
            // likely indicates a programming error. Early validation prevents
            // wasted work and provides clear error messages.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 100).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/empty"),
                project: None,
                task: None,
                services: vec![],  // Empty!
            };

            let config = OccupancyCheckConfig::default();
            let result = allocator.allocate_group(&mut db, &request, &config);

            prop_assert!(result.is_err(), "Empty services should cause error");

            if let Err(e) = result {
                let error_msg = format!("{e}");
                prop_assert!(
                    error_msg.to_lowercase().contains("service") ||
                    error_msg.to_lowercase().contains("empty") ||
                    error_msg.to_lowercase().contains("at least one"),
                    "Error should mention services/empty, got: {}", error_msg
                );
            }
        }
    }

    // ============================================================================
    // PROPERTY 8: PORT OVERFLOW PROTECTION
    // ============================================================================
    // Mathematical property: Large offsets don't cause overflow
    //
    // Group allocation with offsets that would overflow u16 should fail
    // gracefully, not panic or produce invalid ports.
    //
    // This ensures robustness against edge case inputs.

    proptest! {
        #[test]
        fn prop_group_allocation_handles_port_overflow(
            min in 60000u16..=65000u16,
            large_offset in 1000u16..=10000u16,
        ) {
            // PROPERTY: If a service offset would cause base + offset > 65535,
            // allocate_group either:
            // - Skips that base port (finding one where no overflow occurs), or
            // - Returns an error (no valid base port exists)
            // It must never panic or produce invalid port numbers.
            //
            // WHY THIS MATTERS: Port numbers are u16 (max 65535). Large offsets
            // near the top of the range could overflow. This property ensures
            // safe arithmetic and graceful failure.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(Port::MAX).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services = vec![
                ServiceAllocationRequest {
                    tag: "base".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "overflow".to_string(),
                    offset: Some(large_offset),
                    preferred: None,
                },
            ];

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/overflow"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();
            let result = allocator.allocate_group(&mut db, &request, &config);

            // Either succeeds (found valid base) or fails (no valid base)
            // Should never panic
            if let Ok(alloc) = result {
                // If succeeded, verify no port is invalid
                // Note: All Port values are guaranteed to be Port::MIN-Port::MAX by construction
                // (Port::try_from validates this), so this is just a safety check.
                #[allow(clippy::absurd_extreme_comparisons)]
                for port in alloc.allocations.values() {
                    prop_assert!(
                        port.value() <= Port::MAX,
                        "Port {} is out of valid range", port.value()
                    );
                }
            } else {
                // Failure is acceptable when no valid base exists
            }
        }
    }

    // ============================================================================
    // PROPERTY 9: MIXED PREFERRED AND OFFSET HANDLING
    // ============================================================================
    // Mathematical property: Mixed allocation strategies coexist
    //
    // Group allocation can handle services with preferred ports alongside
    // services with offsets, allocating each according to its strategy.
    //
    // This ensures the allocator supports flexible allocation patterns.

    proptest! {
        #[test]
        fn prop_group_allocation_handles_mixed_strategies(
            min in 5000u16..=6000u16,
            preferred_port_offset in 100u16..200u16,
        ) {
            // PROPERTY: A group with both preferred-port services and offset-based
            // services should succeed, with each service getting a port according
            // to its specified strategy.
            //
            // WHY THIS MATTERS: Real applications may need both:
            // - Specific ports for external services (8080 for web)
            // - Offset patterns for internal services
            // This property ensures mixed strategies work correctly together.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let preferred_port = Port::try_from(min + preferred_port_offset).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let services = vec![
                ServiceAllocationRequest {
                    tag: "preferred".to_string(),
                    offset: None,
                    preferred: Some(preferred_port),
                },
                ServiceAllocationRequest {
                    tag: "offset1".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "offset2".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
            ];

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/mixed"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();

            if let Ok(result) = allocator.allocate_group(&mut db, &request, &config) {
                // Verify preferred service got its preferred port
                let preferred_allocated = result.allocations.get("preferred").unwrap();
                prop_assert_eq!(
                    *preferred_allocated,
                    preferred_port,
                    "Preferred service should get preferred port"
                );

                // Verify offset services have correct relationship
                let offset1 = result.allocations.get("offset1").unwrap();
                let offset2 = result.allocations.get("offset2").unwrap();
                prop_assert_eq!(
                    offset2.value(),
                    offset1.value() + 1,
                    "Offset services should maintain offset relationship"
                );
            }
        }
    }

    // ============================================================================
    // PROPERTY 10: BASE PORT CORRECTNESS
    // ============================================================================
    // Mathematical property: Base port represents first offset service
    //
    // When group allocation succeeds with offset-based services, the base_port
    // in the result should equal the port allocated to the service with offset 0
    // (if one exists).
    //
    // This ensures the base_port field accurately represents the allocation.

    proptest! {
        #[test]
        fn prop_group_allocation_base_port_is_accurate(
            min in 5000u16..=6000u16,
            has_zero_offset in proptest::bool::ANY,
        ) {
            // PROPERTY: If a group has a service with offset 0, then
            // result.base_port == result.allocations[that_service]
            //
            // WHY THIS MATTERS: The base_port field helps users understand the
            // allocation pattern. This property ensures it's accurately reported
            // and matches the actual allocations.

            let range = PortRange::new(
                Port::try_from(min).unwrap(),
                Port::try_from(min + 1000).unwrap()
            ).unwrap();

            let mut db = create_test_database();
            let checker = MockOccupancyChecker::new(HashSet::new());
            let allocator = PortAllocator::new(checker, ExclusionManager::empty(), range);

            let mut services = vec![
                ServiceAllocationRequest {
                    tag: "svc1".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "svc2".to_string(),
                    offset: Some(2),
                    preferred: None,
                },
            ];

            if has_zero_offset {
                services.insert(0, ServiceAllocationRequest {
                    tag: "base".to_string(),
                    offset: Some(0),
                    preferred: None,
                });
            }

            let request = GroupAllocationRequest {
                base_path: PathBuf::from("/test/base"),
                project: None,
                task: None,
                services,
            };

            let config = OccupancyCheckConfig::default();

            if let Ok(result) = allocator.allocate_group(&mut db, &request, &config) {
                if has_zero_offset {
                    // Should have a base port
                    let base = result.base_port.expect("Should have base port");
                    let base_service_port = result.allocations.get("base").unwrap();

                    prop_assert_eq!(
                        base,
                        *base_service_port,
                        "Base port should equal the port of the service with offset 0"
                    );
                }
            }
        }
    }
}
