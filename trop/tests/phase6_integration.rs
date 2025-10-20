//! Integration tests for Phase 6: Port Allocation & Occupancy
//!
//! These tests verify end-to-end functionality of the port allocation system,
//! including automatic port selection, occupancy checking, exclusions, cleanup,
//! and group allocations.

mod common;

use common::database::create_test_database;
use trop::Database;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use trop::config::{CleanupConfig, Config, PortConfig, PortExclusion};

use trop::operations::CleanupOperations;

use trop::port::allocator::{AllocationOptions, AllocationResult, PortAllocator};

use trop::port::exclusions::ExclusionManager;

use trop::port::group::{GroupAllocationRequest, ServiceAllocationRequest};

use trop::port::occupancy::{MockOccupancyChecker, OccupancyCheckConfig};

use trop::{Port, PortRange, Reservation, ReservationKey};

// Helper to create a test allocator with mock occupancy checker
fn create_test_allocator_with_config(
    occupied: HashSet<Port>,
    config: &Config,
) -> trop::Result<PortAllocator<MockOccupancyChecker>> {
    let checker = MockOccupancyChecker::new(occupied);

    let port_config = config
        .ports
        .as_ref()
        .ok_or_else(|| trop::Error::Validation {
            field: "ports".into(),
            message: "Port configuration is required".into(),
        })?;

    let min = Port::try_from(port_config.min)?;
    let max_value = if let Some(max) = port_config.max {
        max
    } else if let Some(offset) = port_config.max_offset {
        port_config.min.saturating_add(offset)
    } else {
        return Err(trop::Error::Validation {
            field: "ports".into(),
            message: "Either max or max_offset must be specified".into(),
        });
    };
    let max = Port::try_from(max_value)?;
    let range = PortRange::new(min, max)?;

    let exclusions = if let Some(ref excluded_ports) = config.excluded_ports {
        ExclusionManager::from_config(excluded_ports)?
    } else {
        ExclusionManager::empty()
    };

    Ok(PortAllocator::new(checker, exclusions, range))
}

#[test]
fn test_automatic_allocation_with_no_constraints() {
    // Test automatic port allocation when all ports are free
    // This verifies the basic allocation algorithm selects the first available port
    let mut db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5010),
            max_offset: None,
        }),
        ..Default::default()
    };

    let allocator = create_test_allocator_with_config(HashSet::new(), &config).unwrap();

    let options = AllocationOptions::default();
    let occupancy_config = OccupancyCheckConfig::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();

    // Should allocate the first port in range
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5000).unwrap())
    );

    // Create the reservation
    let key = ReservationKey::new(PathBuf::from("/test/project"), None).unwrap();
    let reservation = Reservation::builder(key.clone(), Port::try_from(5000).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&reservation).unwrap();

    // Second allocation should get next port
    let result2 = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();
    assert_eq!(
        result2,
        AllocationResult::Allocated(Port::try_from(5001).unwrap())
    );
}

#[test]
fn test_allocation_with_exclusions_from_config() {
    // Test that port exclusions from configuration are respected
    // This verifies integration between Config, ExclusionManager, and PortAllocator
    let db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5010),
            max_offset: None,
        }),
        excluded_ports: Some(vec![
            PortExclusion::Single(5000),
            PortExclusion::Single(5001),
            PortExclusion::Range {
                start: 5005,
                end: 5007,
            },
        ]),
        ..Default::default()
    };

    let allocator = create_test_allocator_with_config(HashSet::new(), &config).unwrap();

    let options = AllocationOptions::default();
    let occupancy_config = OccupancyCheckConfig::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();

    // Should skip 5000, 5001, and allocate 5002 (first non-excluded)
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5002).unwrap())
    );
}

#[test]
fn test_allocation_with_occupied_ports() {
    // Test that occupied ports (from mock checker) are skipped
    // This verifies occupancy checking integration
    let db = create_test_database();

    let mut occupied = HashSet::new();
    occupied.insert(Port::try_from(5000).unwrap());
    occupied.insert(Port::try_from(5001).unwrap());
    occupied.insert(Port::try_from(5002).unwrap());

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5010),
            max_offset: None,
        }),
        ..Default::default()
    };

    let allocator = create_test_allocator_with_config(occupied, &config).unwrap();

    let options = AllocationOptions::default();
    let occupancy_config = OccupancyCheckConfig::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();

    // Should skip occupied ports 5000-5002 and allocate 5003
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5003).unwrap())
    );
}

#[test]
fn test_allocation_exhaustion_and_cleanup() {
    // Test exhaustion scenario followed by cleanup and retry
    // This verifies the exhaustion → cleanup → retry flow
    let mut db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5002), // Only 3 ports
            max_offset: None,
        }),
        ..Default::default()
    };

    // Reserve all ports with non-existent paths
    for port in 5000..=5002 {
        let key =
            ReservationKey::new(PathBuf::from(format!("/nonexistent/path/{port}")), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    let allocator = create_test_allocator_with_config(HashSet::new(), &config).unwrap();

    let options = AllocationOptions::default();
    let occupancy_config = OccupancyCheckConfig::default();

    // Should be exhausted
    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();
    assert!(matches!(result, AllocationResult::Exhausted { .. }));

    // Run cleanup to prune non-existent paths
    let cleanup_result = CleanupOperations::prune(&mut db, false).unwrap();
    assert_eq!(cleanup_result.removed_count, 3);

    // Now allocation should succeed
    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5000).unwrap())
    );
}

#[test]
fn test_cleanup_with_expiration() {
    // Test automatic expiration of old reservations
    // This verifies time-based cleanup functionality
    let mut db = create_test_database();

    // Create an old reservation (10 days ago)
    let old_time = SystemTime::now() - Duration::from_secs(10 * 86400);
    let key1 = ReservationKey::new(PathBuf::from("/test/old"), None).unwrap();
    let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
        .last_used_at(old_time)
        .build()
        .unwrap();
    db.create_reservation(&r1).unwrap();

    // Create a fresh reservation
    let key2 = ReservationKey::new(PathBuf::from("/test/fresh"), None).unwrap();
    let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&r2).unwrap();

    // Configure cleanup to expire after 7 days
    let cleanup_config = CleanupConfig {
        expire_after_days: Some(7),
    };

    let result = CleanupOperations::expire(&mut db, &cleanup_config, false).unwrap();
    assert_eq!(result.removed_count, 1);

    // Verify only fresh reservation remains
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].key().path, PathBuf::from("/test/fresh"));
}

#[test]
fn test_group_allocation_end_to_end() {
    // Test complete group allocation workflow
    // This verifies group allocation with database persistence
    let db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5100),
            max_offset: None,
        }),
        ..Default::default()
    };

    let allocator = create_test_allocator_with_config(HashSet::new(), &config).unwrap();

    let request = GroupAllocationRequest {
        base_path: PathBuf::from("/test/microservices"),
        project: Some("my-app".to_string()),
        task: Some("dev".to_string()),
        services: vec![
            ServiceAllocationRequest {
                tag: "web".to_string(),
                offset: Some(0),
                preferred: None,
            },
            ServiceAllocationRequest {
                tag: "api".to_string(),
                offset: Some(1),
                preferred: None,
            },
            ServiceAllocationRequest {
                tag: "admin".to_string(),
                offset: Some(100),
                preferred: None,
            },
        ],
    };

    let occupancy_config = OccupancyCheckConfig::default();
    let result = allocator
        .allocate_group(db.connection(), &request, &occupancy_config)
        .unwrap();

    // Verify allocations
    assert_eq!(result.allocations.len(), 3);
    assert!(result.base_port.is_some());

    let base = result.base_port.unwrap();
    assert_eq!(*result.allocations.get("web").unwrap(), base);
    assert_eq!(
        *result.allocations.get("api").unwrap(),
        Port::try_from(base.value() + 1).unwrap()
    );
    assert_eq!(
        *result.allocations.get("admin").unwrap(),
        Port::try_from(base.value() + 100).unwrap()
    );

    // Verify reservations in database
    let all_reservations = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all_reservations.len(), 3);

    for reservation in &all_reservations {
        assert_eq!(reservation.project(), Some("my-app"));
        assert_eq!(reservation.task(), Some("dev"));
        assert!(reservation.key().tag.is_some());
    }
}

#[test]
fn test_group_allocation_with_conflicts() {
    // Test group allocation when some ports in the pattern are unavailable
    // This verifies pattern matching skips conflicting base ports
    let mut db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5100),
            max_offset: None,
        }),
        ..Default::default()
    };

    // Reserve port 5001, which will conflict with pattern [0, 1] starting at 5000
    let key = ReservationKey::new(PathBuf::from("/existing"), None).unwrap();
    let reservation = Reservation::builder(key, Port::try_from(5001).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&reservation).unwrap();

    let allocator = create_test_allocator_with_config(HashSet::new(), &config).unwrap();

    let request = GroupAllocationRequest {
        base_path: PathBuf::from("/test/project"),
        project: None,
        task: None,
        services: vec![
            ServiceAllocationRequest {
                tag: "web".to_string(),
                offset: Some(0),
                preferred: None,
            },
            ServiceAllocationRequest {
                tag: "api".to_string(),
                offset: Some(1),
                preferred: None,
            },
        ],
    };

    let occupancy_config = OccupancyCheckConfig::default();
    let result = allocator
        .allocate_group(db.connection(), &request, &occupancy_config)
        .unwrap();

    // Should skip base port 5000 (because 5000+1=5001 is reserved)
    // Next valid base is 5002
    let base = result.base_port.unwrap();
    assert_eq!(base.value(), 5002);
    assert_eq!(
        *result.allocations.get("web").unwrap(),
        Port::try_from(5002).unwrap()
    );
    assert_eq!(
        *result.allocations.get("api").unwrap(),
        Port::try_from(5003).unwrap()
    );
}

#[test]
fn test_combined_constraints_integration() {
    // Test allocation with all constraints active: reserved, excluded, and occupied
    // This is a comprehensive integration test of all availability checks
    let mut db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5020),
            max_offset: None,
        }),
        excluded_ports: Some(vec![PortExclusion::Range {
            start: 5005,
            end: 5007,
        }]),
        ..Default::default()
    };

    // Reserve some ports
    for port in [5000, 5001, 5002] {
        let key = ReservationKey::new(PathBuf::from(format!("/reserved/{port}")), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(port).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    // Mark some ports as occupied
    let mut occupied = HashSet::new();
    for port in [5003, 5004] {
        occupied.insert(Port::try_from(port).unwrap());
    }

    let allocator = create_test_allocator_with_config(occupied, &config).unwrap();

    let options = AllocationOptions::default();
    let occupancy_config = OccupancyCheckConfig::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();

    // Should skip:
    // - 5000, 5001, 5002 (reserved)
    // - 5003, 5004 (occupied)
    // - 5005, 5006, 5007 (excluded)
    // First available is 5008
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5008).unwrap())
    );
}

#[test]
fn test_autoclean_integration() {
    // Test the complete autoclean workflow: prune + expire
    // This verifies that cleanup operations work correctly together
    let mut db = create_test_database();

    // Create reservation with non-existent path (will be pruned)
    let key1 = ReservationKey::new(PathBuf::from("/nonexistent"), None).unwrap();
    let r1 = Reservation::builder(key1, Port::try_from(5000).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&r1).unwrap();

    // Create old reservation (will be expired)
    let old_time = SystemTime::now() - Duration::from_secs(10 * 86400);
    let key2 =
        ReservationKey::new(std::env::current_dir().unwrap(), Some("old".to_string())).unwrap();
    let r2 = Reservation::builder(key2, Port::try_from(5001).unwrap())
        .last_used_at(old_time)
        .build()
        .unwrap();
    db.create_reservation(&r2).unwrap();

    // Create fresh, valid reservation (will remain)
    let key3 =
        ReservationKey::new(std::env::current_dir().unwrap(), Some("fresh".to_string())).unwrap();
    let r3 = Reservation::builder(key3, Port::try_from(5002).unwrap())
        .build()
        .unwrap();
    db.create_reservation(&r3).unwrap();

    let cleanup_config = CleanupConfig {
        expire_after_days: Some(7),
    };

    let result = CleanupOperations::autoclean(&mut db, &cleanup_config, false).unwrap();

    assert_eq!(result.pruned_count, 1);
    assert_eq!(result.expired_count, 1);
    assert_eq!(result.total_removed, 2);

    // Verify only fresh reservation remains
    let remaining = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].key().tag, Some("fresh".to_string()));
}

#[test]
fn test_preferred_port_with_occupancy_check() {
    // Test preferred port allocation with occupancy checking
    // This verifies that preferred ports are validated against occupancy
    let db = create_test_database();

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5100),
            max_offset: None,
        }),
        ..Default::default()
    };

    let mut occupied = HashSet::new();
    occupied.insert(Port::try_from(5050).unwrap());

    let allocator = create_test_allocator_with_config(occupied, &config).unwrap();

    // Try to allocate occupied preferred port
    let options = AllocationOptions {
        preferred: Some(Port::try_from(5050).unwrap()),
        ignore_occupied: false,
        ignore_exclusions: false,
    };
    let occupancy_config = OccupancyCheckConfig::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occupancy_config)
        .unwrap();

    assert_eq!(
        result,
        AllocationResult::PreferredUnavailable {
            port: Port::try_from(5050).unwrap(),
            reason: trop::error::PortUnavailableReason::Occupied,
        }
    );

    // Try with ignore_occupied flag
    let options_ignore = AllocationOptions {
        preferred: Some(Port::try_from(5050).unwrap()),
        ignore_occupied: true,
        ignore_exclusions: false,
    };

    let result2 = allocator
        .allocate_single(db.connection(), &options_ignore, &occupancy_config)
        .unwrap();

    assert_eq!(
        result2,
        AllocationResult::Allocated(Port::try_from(5050).unwrap())
    );
}

#[test]
fn test_occupancy_config_integration() {
    // Test that OccupancyCheckConfig controls the SystemOccupancyChecker behavior
    // Note: MockOccupancyChecker ignores the config by design (for deterministic testing)
    // This test verifies that the MockChecker correctly simulates occupied ports
    let db = create_test_database();

    let mut occupied = HashSet::new();
    occupied.insert(Port::try_from(5000).unwrap());

    let config = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(5100),
            max_offset: None,
        }),
        ..Default::default()
    };

    let allocator = create_test_allocator_with_config(occupied, &config).unwrap();

    // MockOccupancyChecker returns true for port 5000 regardless of config
    // (this is intentional for deterministic testing)
    let occ_config = OccupancyCheckConfig::default();

    let options = AllocationOptions::default();

    let result = allocator
        .allocate_single(db.connection(), &options, &occ_config)
        .unwrap();

    // Should skip occupied port 5000 and allocate 5001
    assert_eq!(
        result,
        AllocationResult::Allocated(Port::try_from(5001).unwrap())
    );
}
