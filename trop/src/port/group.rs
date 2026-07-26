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

impl GroupAllocationRequest {
    /// Return a validated request whose service tags use their reservation-key
    /// identity. `ReservationKey` trims tags before storage, so group matching
    /// and duplicate detection must use that same spelling before any write.
    pub(crate) fn normalized(&self) -> Result<Self> {
        if self.services.is_empty() {
            return Err(Error::Validation {
                field: "services".into(),
                message: "Group allocation requires at least one service".into(),
            });
        }

        let mut normalized = self.clone();
        let mut seen_tags = std::collections::HashSet::new();
        for service in &mut normalized.services {
            service.tag = service.tag.trim().to_string();
            if service.tag.is_empty() {
                return Err(Error::Validation {
                    field: "services".into(),
                    message: "Service tags must not be empty or whitespace-only".into(),
                });
            }
            if !seen_tags.insert(service.tag.clone()) {
                return Err(Error::Validation {
                    field: "services".into(),
                    message: "Duplicate service tag after normalization".into(),
                });
            }
            if service.preferred.is_none() && service.offset.is_none() {
                return Err(Error::Validation {
                    field: "services".into(),
                    message: "Every service without a preferred port must have an offset".into(),
                });
            }
        }

        Ok(normalized)
    }
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
    /// 1. Load tagged rows at the exact group path
    /// 2. Reuse and refresh a complete shape-compatible group
    /// 3. Allocate a fresh complete group only when no tagged rows exist
    /// 4. Reject partial or incompatible groups without mutation
    /// 5. Roll back the whole refresh/allocation if any write fails
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The request is invalid (empty services, duplicate tags, etc.)
    /// - No base port can be found for the offset pattern
    /// - Database operations fail
    /// - Preferred ports are unavailable
    /// - Existing tagged rows form a partial or incompatible group
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
        let request = request.normalized()?;

        // The CLI opens an IMMEDIATE outer transaction before execution. This
        // savepoint keeps reconciliation/allocation atomic for library callers
        // too, and rolls timestamp refreshes or inserts back as one unit.
        Database::with_savepoint(conn, "trop_allocate_group", |conn| {
            let existing =
                Database::get_tagged_reservations_by_exact_path(conn, &request.base_path)?;

            if existing.is_empty() {
                return self.allocate_fresh_group(conn, &request, occupancy_config);
            }

            let result = self.compatible_existing_group(&request, &existing)?;
            for reservation in &existing {
                if !Database::update_last_used_simple(conn, reservation.key())? {
                    return Err(Self::group_conflict(
                        &request,
                        "a stored service disappeared while refreshing the group",
                    ));
                }
            }

            Ok(result)
        })
    }

    fn compatible_existing_group(
        &self,
        request: &GroupAllocationRequest,
        existing: &[Reservation],
    ) -> Result<GroupAllocationResult> {
        let existing_by_tag = Self::index_existing_group(request, existing)?;
        let mut allocations = HashMap::with_capacity(request.services.len());
        let mut base_port = None;
        let mut services = request.services.iter().collect::<Vec<_>>();
        services.sort_unstable_by(|left, right| left.tag.cmp(&right.tag));

        for service in services {
            let reservation = existing_by_tag
                .get(service.tag.as_str())
                .copied()
                .ok_or_else(|| {
                    Self::group_conflict(request, "stored group is missing a requested service")
                })?;
            let port = reservation.port();

            if let Some(preferred) = service.preferred {
                if port != preferred {
                    return Err(Self::group_conflict(
                        request,
                        format!("stored port {port} does not match preferred port {preferred}"),
                    ));
                }
            } else {
                let offset = service.offset.ok_or_else(|| Error::Validation {
                    field: "services".into(),
                    message: "An offset-based service is missing its offset".into(),
                })?;
                let candidate = self.compatible_offset_base(request, port, offset)?;

                match base_port {
                    Some(base) if base != candidate => {
                        return Err(Self::group_conflict(
                            request,
                            format!(
                                "stored offset mappings imply different bases \
                                 ({candidate} and {base})"
                            ),
                        ));
                    }
                    None => base_port = Some(candidate),
                    Some(_) => {}
                }
            }

            allocations.insert(service.tag.clone(), port);
        }

        Ok(GroupAllocationResult {
            allocations,
            base_port,
        })
    }

    fn index_existing_group<'a>(
        request: &GroupAllocationRequest,
        existing: &'a [Reservation],
    ) -> Result<HashMap<&'a str, &'a Reservation>> {
        let mut existing_by_tag = HashMap::with_capacity(existing.len());
        for reservation in existing {
            let Some(tag) = reservation.key().tag.as_deref() else {
                return Err(Self::group_conflict(
                    request,
                    "stored group unexpectedly contains an untagged reservation",
                ));
            };
            if existing_by_tag.insert(tag, reservation).is_some() {
                return Err(Self::group_conflict(
                    request,
                    "stored group contains duplicate service tags",
                ));
            }
        }

        let mut requested_tags = request
            .services
            .iter()
            .map(|service| service.tag.as_str())
            .collect::<Vec<_>>();
        requested_tags.sort_unstable();
        let mut existing_tags = existing_by_tag.keys().copied().collect::<Vec<_>>();
        existing_tags.sort_unstable();

        if requested_tags != existing_tags {
            return Err(Self::group_conflict(
                request,
                "requested service set does not match stored service set",
            ));
        }

        Ok(existing_by_tag)
    }

    fn compatible_offset_base(
        &self,
        request: &GroupAllocationRequest,
        port: Port,
        offset: u16,
    ) -> Result<Port> {
        let candidate_value = port.value().checked_sub(offset).ok_or_else(|| {
            Self::group_conflict(
                request,
                format!("stored port {port} cannot satisfy requested offset {offset}"),
            )
        })?;
        let candidate = Port::try_from(candidate_value).map_err(|_| {
            Self::group_conflict(
                request,
                format!("stored port {port} implies invalid base {candidate_value}"),
            )
        })?;

        if !self.range().contains(candidate) || !self.range().contains(port) {
            return Err(Self::group_conflict(
                request,
                "a stored service mapping is outside the current scan range",
            ));
        }

        Ok(candidate)
    }

    fn group_conflict(request: &GroupAllocationRequest, reason: impl Into<String>) -> Error {
        Error::ReservationConflict {
            details: format!(
                "existing group at {} is incompatible: {}",
                request.base_path.display(),
                reason.into()
            ),
        }
    }

    fn allocate_fresh_group(
        &self,
        conn: &rusqlite::Connection,
        request: &GroupAllocationRequest,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<GroupAllocationResult> {
        let (preferred_services, offset_services): (Vec<_>, Vec<_>) =
            request.services.iter().partition(|s| s.preferred.is_some());

        let base_port = if offset_services.is_empty() {
            None
        } else {
            let pattern: Vec<u16> = offset_services.iter().filter_map(|s| s.offset).collect();
            let base = self
                .find_pattern_match(&pattern, conn, occupancy_config)?
                .ok_or_else(|| Error::GroupAllocationFailed {
                    attempted: 0,
                    reason: "No base port found for offset pattern".into(),
                })?;

            Some(base)
        };

        let mut allocations = HashMap::new();
        let mut reservations_to_create = Vec::new();

        for service in &preferred_services {
            let port = service.preferred.ok_or_else(|| Error::Validation {
                field: "services".into(),
                message: "A preferred-port service is missing its preferred port".into(),
            })?;
            let key = ReservationKey::new(request.base_path.clone(), Some(service.tag.clone()))?;

            let options = AllocationOptions {
                preferred: Some(port),
                ignore_occupied: false,
                ignore_exclusions: false,
            };

            match self.allocate_single(conn, &options, occupancy_config)? {
                crate::port::allocator::AllocationResult::Allocated(_) => {}
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

            let reservation = Reservation::builder(key, port)
                .project(request.project.clone())
                .task(request.task.clone())
                .build()?;
            reservations_to_create.push(reservation);
        }

        if let Some(base) = base_port {
            for service in &offset_services {
                let offset = service.offset.ok_or_else(|| Error::Validation {
                    field: "services".into(),
                    message: "An offset-based service is missing its offset".into(),
                })?;

                let port = base.checked_add(offset).ok_or_else(|| Error::Validation {
                    field: "offset".into(),
                    message: format!("Port overflow: {base} + {offset}"),
                })?;

                let key =
                    ReservationKey::new(request.base_path.clone(), Some(service.tag.clone()))?;

                allocations.insert(service.tag.clone(), port);

                let reservation = Reservation::builder(key, port)
                    .project(request.project.clone())
                    .task(request.task.clone())
                    .build()?;
                reservations_to_create.push(reservation);
            }
        }

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
        let db = create_test_database();
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
    fn test_group_allocation_rolls_back_if_late_insert_fails() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        db.connection()
            .execute_batch(
                r"
                CREATE TRIGGER fail_api_insert
                BEFORE INSERT ON reservations
                WHEN NEW.tag = 'api'
                BEGIN
                    SELECT RAISE(ABORT, 'forced late group insert failure');
                END;
                ",
            )
            .unwrap();

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(5000).unwrap()),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: None,
                    preferred: Some(Port::try_from(5001).unwrap()),
                },
            ],
        };

        let config = OccupancyCheckConfig::default();
        assert!(allocator
            .allocate_group(db.connection(), &request, &config)
            .is_err());

        let reservations = Database::list_all_reservations(db.connection()).unwrap();
        assert!(reservations.is_empty());
    }

    #[test]
    fn test_group_allocation_with_gaps() {
        let db = create_test_database();
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
        let db = create_test_database();

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
        let db = create_test_database();
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
        let db = create_test_database();
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
        let db = create_test_database();
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
    fn test_group_allocation_normalizes_tags_before_reuse() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![ServiceAllocationRequest {
                tag: " web ".to_string(),
                offset: Some(0),
                preferred: None,
            }],
        };
        let config = OccupancyCheckConfig::default();

        let first = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect("Padded tag should allocate");
        let second = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect("The same padded tag should reuse its normalized key");

        assert_eq!(second, first);
        assert_eq!(
            first.allocations,
            HashMap::from([("web".to_string(), Port::try_from(5000).unwrap())])
        );
        let reservations = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(reservations.len(), 1);
        assert_eq!(reservations[0].key().tag.as_deref(), Some("web"));
    }

    #[test]
    fn test_group_allocation_rejects_tags_colliding_after_normalization() {
        let db = create_test_database();
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
                    tag: " web ".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
            ],
        };
        let config = OccupancyCheckConfig::default();

        let error = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect_err("Tags with the same normalized identity must conflict");

        assert!(
            matches!(
                error,
                Error::Validation {
                    ref field,
                    ref message
                }
                    if field == "services"
                        && message.contains("Duplicate service tag")
            ),
            "Expected a normalized duplicate-tag validation error, got {error}"
        );
        assert!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .is_empty(),
            "Normalized duplicate tags must fail before any row is written"
        );
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
        let db = create_test_database();
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
        let db = create_test_database();
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
        let db = create_test_database();
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
        let db = create_test_database();
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
        let db = create_test_database();
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
        let db = create_test_database();
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
