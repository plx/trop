//! Group allocation support for allocating multiple related ports atomically.
//!
//! This module provides functionality for allocating groups of ports with specific
//! offset patterns, useful for microservices or applications that need multiple ports.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::database::Database;
use crate::error::Error;
use crate::port::allocator::{AllocationOptions, PortAllocator};
use crate::port::occupancy::{OccupancyCheckConfig, PortOccupancyChecker};
use crate::{Port, Reservation, ReservationKey, Result};

/// Request for allocating a group of related ports.
///
/// A group allocation request specifies multiple services, each with an optional
/// offset from a base port and/or a preferred absolute port. The allocator will
/// find a base port where all the offsets are available.
///
/// # Examples
///
/// ```
/// use trop::port::group::{GroupAllocationRequest, ServiceAllocationRequest};
/// use std::path::PathBuf;
///
/// let request = GroupAllocationRequest {
///     base_path: PathBuf::from("/my/project"),
///     project: Some("my-app".to_string()),
///     task: Some("dev".to_string()),
///     services: vec![
///         ServiceAllocationRequest {
///             tag: "web".to_string(),
///             offset: Some(0),
///             preferred: None,
///         },
///         ServiceAllocationRequest {
///             tag: "api".to_string(),
///             offset: Some(1),
///             preferred: None,
///         },
///         ServiceAllocationRequest {
///             tag: "admin".to_string(),
///             offset: Some(100),
///             preferred: None,
///         },
///     ],
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAllocationRequest {
    /// Base path for all reservations in the group.
    pub base_path: PathBuf,
    /// Optional project identifier for all reservations.
    pub project: Option<String>,
    /// Optional task identifier for all reservations.
    pub task: Option<String>,
    /// Services to allocate within the group.
    pub services: Vec<ServiceAllocationRequest>,
}

/// Individual service in a group allocation request.
///
/// Each service has a tag (identifier), an optional offset from the base port,
/// and an optional preferred absolute port.
///
/// # Examples
///
/// ```
/// use trop::port::group::ServiceAllocationRequest;
/// use trop::Port;
///
/// // Service with offset from base
/// let web = ServiceAllocationRequest {
///     tag: "web".to_string(),
///     offset: Some(0),
///     preferred: None,
/// };
///
/// // Service with preferred absolute port
/// let api = ServiceAllocationRequest {
///     tag: "api".to_string(),
///     offset: None,
///     preferred: Some(Port::try_from(8080).unwrap()),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAllocationRequest {
    /// Tag identifier for this service.
    pub tag: String,
    /// Optional offset from the base port.
    pub offset: Option<u16>,
    /// Optional preferred absolute port (takes precedence over offset).
    pub preferred: Option<Port>,
}

/// Result of a group allocation operation.
///
/// Contains the mapping of service tags to their allocated ports, and optionally
/// the base port that was used for offset calculations.
///
/// # Examples
///
/// ```
/// use trop::port::group::GroupAllocationResult;
/// use trop::Port;
/// use std::collections::HashMap;
///
/// let mut allocations = HashMap::new();
/// allocations.insert("web".to_string(), Port::try_from(5000).unwrap());
/// allocations.insert("api".to_string(), Port::try_from(5001).unwrap());
///
/// let result = GroupAllocationResult {
///     allocations,
///     base_port: Some(Port::try_from(5000).unwrap()),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupAllocationResult {
    /// Map of service tags to their allocated ports.
    pub allocations: HashMap<String, Port>,
    /// The base port used for offset calculations (if any).
    pub base_port: Option<Port>,
}

impl<C: PortOccupancyChecker> PortAllocator<C> {
    /// Allocate a group of related ports atomically.
    ///
    /// This method implements group allocation with the following semantics:
    /// 1. Separate services into those with preferred ports and those with offsets
    /// 2. For offset-based services, find a base port where all offsets are available
    /// 3. Create reservations for all services
    /// 4. Each individual reservation creation is atomic (transactional)
    /// 5. The group as a whole is validated upfront, so failures should be rare
    ///
    /// Note: Currently, reservations are created one-by-one. If a later creation fails
    /// (e.g., due to database errors), earlier reservations will have been committed.
    /// This could be improved with bulk transaction support in the future.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The request is invalid (empty services, duplicate tags, etc.)
    /// - No base port can be found for the offset pattern
    /// - Database operations fail
    /// - Preferred ports are unavailable
    ///
    /// # Panics
    ///
    /// Does not panic. The code uses `unwrap()` on `service.preferred` at line 253,
    /// but this is safe because `preferred_services` is filtered to only contain
    /// services where `preferred.is_some()`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::port::allocator::PortAllocator;
    /// use trop::port::group::{GroupAllocationRequest, ServiceAllocationRequest};
    /// use trop::port::occupancy::{SystemOccupancyChecker, OccupancyCheckConfig};
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Port, PortRange};
    /// use std::path::PathBuf;
    ///
    /// let checker = SystemOccupancyChecker;
    /// let exclusions = ExclusionManager::empty();
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(7000).unwrap(),
    /// ).unwrap();
    ///
    /// let allocator = PortAllocator::new(checker, exclusions, range);
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let request = GroupAllocationRequest {
    ///     base_path: PathBuf::from("/my/project"),
    ///     project: Some("my-app".to_string()),
    ///     task: None,
    ///     services: vec![
    ///         ServiceAllocationRequest {
    ///             tag: "web".to_string(),
    ///             offset: Some(0),
    ///             preferred: None,
    ///         },
    ///         ServiceAllocationRequest {
    ///             tag: "api".to_string(),
    ///             offset: Some(1),
    ///             preferred: None,
    ///         },
    ///     ],
    /// };
    ///
    /// let occupancy_config = OccupancyCheckConfig::default();
    /// let result = allocator.allocate_group(db.connection(), &request, &occupancy_config).unwrap();
    /// println!("Allocated {} ports", result.allocations.len());
    /// ```
    pub fn allocate_group(
        &self,
        conn: &rusqlite::Connection,
        request: &GroupAllocationRequest,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<GroupAllocationResult> {
        // Validate request
        if request.services.is_empty() {
            return Err(Error::Validation {
                field: "services".into(),
                message: "Group allocation requires at least one service".into(),
            });
        }

        // Check for duplicate tags
        let mut seen_tags = std::collections::HashSet::new();
        for service in &request.services {
            if !seen_tags.insert(&service.tag) {
                return Err(Error::Validation {
                    field: "services".into(),
                    message: format!("Duplicate service tag: {}", service.tag),
                });
            }
        }

        // Separate services into those with preferred ports and those with offsets
        let (preferred_services, offset_services): (Vec<_>, Vec<_>) =
            request.services.iter().partition(|s| s.preferred.is_some());

        // Determine the base port
        let base_port = if offset_services.is_empty() {
            None
        } else {
            // Extract offset pattern using two-phase validation:
            // 1. First, filter_map extracts only services with Some(offset)
            // 2. Later, we validate that the pattern is non-empty
            // This allows us to provide a specific error message if all services
            // without preferred ports are also missing offsets.
            let pattern: Vec<u16> = offset_services.iter().filter_map(|s| s.offset).collect();

            if pattern.is_empty() {
                return Err(Error::Validation {
                    field: "services".into(),
                    message: "Services without preferred ports must have offsets".into(),
                });
            }

            // Find a base port where all offsets are available
            let base = self
                .find_pattern_match(&pattern, conn, occupancy_config)?
                .ok_or_else(|| Error::GroupAllocationFailed {
                    attempted: 0,
                    reason: "No base port found for offset pattern".into(),
                })?;

            Some(base)
        };

        // Build allocation map
        let mut allocations = HashMap::new();
        let mut reservations_to_create = Vec::new();

        // Allocate preferred ports
        for service in &preferred_services {
            let port = service.preferred.unwrap(); // Safe because we filtered for Some
            let key = ReservationKey::new(request.base_path.clone(), Some(service.tag.clone()))?;

            // Check if port is available
            let options = AllocationOptions {
                preferred: Some(port),
                ignore_occupied: false,
                ignore_exclusions: false,
            };

            match self.allocate_single(conn, &options, occupancy_config)? {
                crate::port::allocator::AllocationResult::Allocated(_) => {
                    // Good, port is available
                }
                crate::port::allocator::AllocationResult::PreferredUnavailable { port, reason } => {
                    return Err(Error::PreferredPortUnavailable { port, reason });
                }
                crate::port::allocator::AllocationResult::Exhausted { .. } => {
                    return Err(Error::GroupAllocationFailed {
                        attempted: allocations.len(),
                        reason: format!("Preferred port {port} not available"),
                    });
                }
            }

            allocations.insert(service.tag.clone(), port);

            // Prepare reservation
            let reservation = Reservation::builder(key, port)
                .project(request.project.clone())
                .task(request.task.clone())
                .build()?;
            reservations_to_create.push(reservation);
        }

        // Allocate offset-based ports
        if let Some(base) = base_port {
            for service in &offset_services {
                let offset = service.offset.ok_or_else(|| Error::Validation {
                    field: "services".into(),
                    message: format!("Service {} missing offset", service.tag),
                })?;

                let port = base.checked_add(offset).ok_or_else(|| Error::Validation {
                    field: "offset".into(),
                    message: format!("Port overflow: {base} + {offset}"),
                })?;

                let key =
                    ReservationKey::new(request.base_path.clone(), Some(service.tag.clone()))?;

                allocations.insert(service.tag.clone(), port);

                // Prepare reservation
                let reservation = Reservation::builder(key, port)
                    .project(request.project.clone())
                    .task(request.task.clone())
                    .build()?;
                reservations_to_create.push(reservation);
            }
        }

        // Create all reservations using simple create (within transaction managed by caller)
        // If any creation fails, the transaction managed by the caller will roll back
        for reservation in &reservations_to_create {
            Database::create_reservation_simple(conn, reservation)?;
        }

        Ok(GroupAllocationResult {
            allocations,
            base_port,
        })
    }

    /// Find a base port where all offsets in the pattern are available.
    ///
    /// This scans forward from the range minimum looking for a base port where
    /// base+offset is available for every offset in the pattern.
    ///
    /// # Errors
    ///
    /// Returns an error if database queries or occupancy checks fail.
    pub fn find_pattern_match(
        &self,
        pattern: &[u16],
        conn: &rusqlite::Connection,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<Option<Port>> {
        if pattern.is_empty() {
            return Ok(None);
        }

        // Scan from range minimum
        let start = self.range().min();
        let end = self.range().max();

        // Calculate the maximum offset to ensure we don't scan too far
        let max_offset = pattern.iter().copied().max().unwrap_or(0);

        // We need to ensure base + max_offset <= range.max()
        let scan_end = end.checked_sub(max_offset).unwrap_or(start);

        for base_value in start.value()..=scan_end.value() {
            let base = Port::try_from(base_value)?;

            // Check if all offsets are available from this base
            let mut all_available = true;
            for &offset in pattern {
                if let Some(port) = base.checked_add(offset) {
                    if self.is_port_available(port, conn, occupancy_config)?
                        == super::allocator::PortAvailability::Available
                    {
                        // Good, continue checking
                    } else {
                        all_available = false;
                        break;
                    }
                } else {
                    // Port overflow
                    all_available = false;
                    break;
                }
            }

            if all_available {
                return Ok(Some(base));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::port::allocator::PortAllocator;
    use crate::port::exclusions::ExclusionManager;
    use crate::port::occupancy::{MockOccupancyChecker, OccupancyCheckConfig};
    use crate::PortRange;
    use std::collections::HashSet;

    fn create_test_allocator(
        occupied: HashSet<Port>,
        min: u16,
        max: u16,
    ) -> PortAllocator<MockOccupancyChecker> {
        let checker = MockOccupancyChecker::new(occupied);
        let range =
            PortRange::new(Port::try_from(min).unwrap(), Port::try_from(max).unwrap()).unwrap();
        PortAllocator::new(checker, ExclusionManager::empty(), range)
    }

    #[test]
    fn test_group_allocation_simple() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
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

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        assert_eq!(result.allocations.len(), 2);
        assert!(result.base_port.is_some());

        let web_port = result.allocations.get("web").unwrap();
        let api_port = result.allocations.get("api").unwrap();

        // Ports should be consecutive
        assert_eq!(api_port.value(), web_port.value() + 1);
    }

    #[test]
    fn test_group_allocation_with_gaps() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
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
                ServiceAllocationRequest {
                    tag: "admin".to_string(),
                    offset: Some(100),
                    preferred: None,
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        assert_eq!(result.allocations.len(), 3);

        let web_port = result.allocations.get("web").unwrap();
        let api_port = result.allocations.get("api").unwrap();
        let admin_port = result.allocations.get("admin").unwrap();

        assert_eq!(api_port.value(), web_port.value() + 1);
        assert_eq!(admin_port.value(), web_port.value() + 100);
    }

    #[test]
    fn test_group_allocation_skips_occupied() {
        let mut db = create_test_database();

        // Mark port 5000 and 5001 as occupied
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(occupied, 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
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

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        let web_port = result.allocations.get("web").unwrap();
        // Should skip 5000 and 5001, allocate starting from 5002
        assert_eq!(web_port.value(), 5002);
    }

    #[test]
    fn test_group_allocation_with_preferred() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(8080).unwrap()),
                },
            ],
        };

        let config = OccupancyCheckConfig::default();

        // This should fail because 8080 is outside the range
        let result = allocator.allocate_group(db.connection(), &request, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_group_allocation_empty_services() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator.allocate_group(db.connection(), &request, &config);

        assert!(result.is_err());
        match result {
            Err(Error::Validation { field, .. }) => {
                assert_eq!(field, "services");
            }
            _ => panic!("Expected validation error"),
        }
    }

    #[test]
    fn test_group_allocation_duplicate_tags() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "web".to_string(), // Duplicate!
                    offset: Some(1),
                    preferred: None,
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator.allocate_group(db.connection(), &request, &config);

        assert!(result.is_err());
        match result {
            Err(Error::Validation { message, .. }) => {
                assert!(message.contains("Duplicate"));
            }
            _ => panic!("Expected validation error"),
        }
    }

    #[test]
    fn test_find_pattern_match_simple() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let pattern = vec![0, 1, 2];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        assert_eq!(result, Some(Port::try_from(5000).unwrap()));
    }

    #[test]
    fn test_find_pattern_match_with_occupied() {
        let db = create_test_database();

        // Mark some ports as occupied
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(occupied, 5000, 5100);

        let pattern = vec![0, 1];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        // Should find first available base where both 0 and 1 offsets are free
        assert_eq!(result, Some(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_find_pattern_match_with_gaps() {
        let db = create_test_database();

        // Occupy port 5001 (but not 5000 or 5002)
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(occupied, 5000, 5100);

        // Pattern needs 0 and 1 offsets
        let pattern = vec![0, 1];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        // Can't use 5000 (because 5000+1=5001 is occupied), should use 5002
        assert_eq!(result, Some(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_find_pattern_match_exhausted() {
        let db = create_test_database();

        // Occupy all ports
        let mut occupied = HashSet::new();
        for port in 5000..=5100 {
            occupied.insert(Port::try_from(port).unwrap());
        }

        let allocator = create_test_allocator(occupied, 5000, 5100);

        let pattern = vec![0, 1];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_group_allocation_creates_reservations() {
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
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
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        // Verify reservations were created in database
        let all_reservations = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all_reservations.len(), 2);

        for reservation in &all_reservations {
            assert_eq!(reservation.key().path, PathBuf::from("/test/project"));
            assert_eq!(reservation.project(), Some("test"));
            assert_eq!(reservation.task(), Some("dev"));

            let port = result
                .allocations
                .get(reservation.key().tag.as_ref().unwrap())
                .unwrap();
            assert_eq!(reservation.port(), *port);
        }
    }

    #[test]
    fn test_group_allocation_mixed_offset_and_preferred() {
        // Test group allocation with both offset-based and preferred ports
        // Verifies correct handling of mixed allocation strategies
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: None,
                },
                ServiceAllocationRequest {
                    tag: "admin".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(5050).unwrap()),
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        assert_eq!(result.allocations.len(), 2);
        assert_eq!(
            *result.allocations.get("admin").unwrap(),
            Port::try_from(5050).unwrap()
        );
    }

    #[test]
    fn test_group_allocation_no_base_port_only_preferred() {
        // Test group allocation when all services use preferred ports (no base port)
        // Ensures base_port is None when not needed
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(5010).unwrap()),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(5020).unwrap()),
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        assert_eq!(result.allocations.len(), 2);
        assert!(result.base_port.is_none());
    }

    #[test]
    fn test_find_pattern_match_at_range_boundary() {
        // Test pattern matching at the end of the port range
        // Ensures correct boundary handling when max_offset is considered
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5010);

        // Pattern with offset 5 - can only use base ports up to 5005
        let pattern = vec![0, 5];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        assert_eq!(result, Some(Port::try_from(5000).unwrap()));
    }

    #[test]
    fn test_find_pattern_match_overflow_protection() {
        // Test that pattern matching correctly handles port overflow scenarios
        // Ensures u16 overflow is prevented when calculating offsets
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 65530, 65535);

        // Pattern that would overflow if not checked
        let pattern = vec![0, 10]; // 65530 + 10 = 65540 (overflow!)
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        // Should not find a match because pattern would overflow
        assert!(result.is_none());
    }

    #[test]
    fn test_group_allocation_service_without_offset_or_preferred() {
        // Test validation error when service has neither offset nor preferred port
        // Ensures proper error handling for misconfigured services
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![ServiceAllocationRequest {
                tag: "web".to_string(),
                offset: None,    // No offset
                preferred: None, // No preferred either!
            }],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator.allocate_group(db.connection(), &request, &config);

        assert!(result.is_err());
    }

    #[test]
    fn test_find_pattern_match_empty_pattern() {
        // Test pattern matching with empty pattern
        // Ensures graceful handling of edge case
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let pattern: Vec<u16> = vec![];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_pattern_match_partial_availability() {
        // Test pattern matching when some base ports work and others don't
        // Verifies correct scanning and recovery from partial matches
        let db = create_test_database();

        let mut occupied = HashSet::new();
        // Occupy 5001, which breaks the pattern starting at 5000 (needs 0,1,2)
        occupied.insert(Port::try_from(5001).unwrap());
        // Occupy 5003, which breaks pattern starting at 5002 (needs 0,1)
        occupied.insert(Port::try_from(5003).unwrap());

        let allocator = create_test_allocator(occupied, 5000, 5100);
        let pattern = vec![0, 1];
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .find_pattern_match(&pattern, db.connection(), &config)
            .unwrap();
        // Should skip 5000 (5000+1=5001 occupied) and 5002 (5002+1=5003 occupied)
        // First valid base is 5004
        assert_eq!(result, Some(Port::try_from(5004).unwrap()));
    }

    #[test]
    fn test_group_allocation_large_offset_gap() {
        // Test group allocation with large gaps between offsets
        // Ensures correct handling of sparse port patterns
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 10000);

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
                    tag: "admin".to_string(),
                    offset: Some(1000),
                    preferred: None,
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        let web_port = result.allocations.get("web").unwrap();
        let admin_port = result.allocations.get("admin").unwrap();
        assert_eq!(admin_port.value(), web_port.value() + 1000);
    }

    #[test]
    fn test_group_allocation_result_base_port() {
        // Test that result correctly includes base_port for offset-based allocations
        // Verifies base_port tracking is accurate
        let mut db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

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

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        assert!(result.base_port.is_some());
        let base = result.base_port.unwrap();
        assert_eq!(*result.allocations.get("web").unwrap(), base);
        assert_eq!(
            *result.allocations.get("api").unwrap(),
            Port::try_from(base.value() + 1).unwrap()
        );
    }
}
