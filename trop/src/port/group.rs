//! Group allocation support for allocating multiple related ports atomically.
//!
//! This module provides functionality for allocating groups of ports with specific
//! offset patterns, useful for microservices or applications that need multiple ports.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::database::Database;
use crate::error::Error;
use crate::port::allocator::{PortAllocator, PortAvailability};
use crate::port::occupancy::{OccupancyCheckConfig, PortOccupancyChecker};
use crate::{Port, Reservation, ReservationKey, Result};

/// Request for allocating a group of related ports.
///
/// A group allocation request specifies multiple services, each with an optional
/// offset from a base port and/or a preferred absolute port. Available preferred
/// ports are pinned first across the full valid port domain. Services whose
/// preference is unavailable use their offset fallback, and the allocator finds
/// the lowest in-range base where the complete fallback pattern is available and
/// does not collide with a pinned preference.
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
        normalized.project = Self::normalize_metadata("project", normalized.project.as_deref())?;
        normalized.task = Self::normalize_metadata("task", normalized.task.as_deref())?;
        let mut seen_tags = HashSet::new();
        let mut seen_offsets = HashSet::new();
        let mut seen_preferred = HashSet::new();
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
            if let Some(offset) = service.offset {
                if !seen_offsets.insert(offset) {
                    return Err(Error::Validation {
                        field: "services".into(),
                        message: format!("Duplicate service offset: {offset}"),
                    });
                }
            }
            if let Some(preferred) = service.preferred {
                if !seen_preferred.insert(preferred) {
                    return Err(Error::Validation {
                        field: "services".into(),
                        message: format!("Duplicate preferred port: {preferred}"),
                    });
                }
            }
        }

        Ok(normalized)
    }

    fn normalize_metadata(field: &str, value: Option<&str>) -> Result<Option<String>> {
        value
            .map(str::trim)
            .map(|value| {
                if value.is_empty() {
                    Err(Error::Validation {
                        field: field.to_string(),
                        message: "Cannot be empty or only whitespace".to_string(),
                    })
                } else {
                    Ok(value.to_string())
                }
            })
            .transpose()
    }
}

/// Safety policy for reconciling an existing reservation group.
///
/// Field-specific permissions authorize only their named metadata update.
/// `force` additionally authorizes both metadata fields and one atomic
/// replacement of an incompatible group shape. It deliberately does not relax
/// database uniqueness, exclusion, or operating-system occupancy checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroupReconciliationPolicy {
    /// Authorize project metadata changes.
    pub allow_project_change: bool,
    /// Authorize task metadata changes.
    pub allow_task_change: bool,
    /// Authorize every group safety override, including shape replacement.
    pub force: bool,
}

/// Individual service in a group allocation request.
///
/// Each service has a tag (identifier), an optional offset from the base port,
/// and an optional preferred absolute port. The lower-level library API permits
/// `offset: None` for a preferred-only request; unlike configuration, that
/// request has no fallback and reports why an unavailable preference failed.
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
/// // Service with preferred absolute port and offset fallback
/// let api = ServiceAllocationRequest {
///     tag: "api".to_string(),
///     offset: Some(1),
///     preferred: Some(Port::try_from(8080).unwrap()),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAllocationRequest {
    /// Tag identifier for this service.
    pub tag: String,
    /// Optional offset from the base port and fallback when preferred is unavailable.
    pub offset: Option<u16>,
    /// Optional preferred absolute port (tried before the offset fallback).
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

#[derive(Debug, Clone)]
struct GroupMetadataUpdate {
    key: ReservationKey,
    project: Option<String>,
    task: Option<String>,
}

enum GroupReconciliation {
    AllocateFresh(GroupAllocationRequest),
    Refresh {
        result: GroupAllocationResult,
        metadata: Vec<GroupMetadataUpdate>,
    },
    Replace {
        request: GroupAllocationRequest,
        existing_keys: Vec<ReservationKey>,
    },
}

impl<C: PortOccupancyChecker> PortAllocator<C> {
    /// Allocate a group of related ports atomically.
    ///
    /// This method implements group allocation with the following semantics:
    /// 1. Load tagged rows at the exact group path
    /// 2. Validate every sticky metadata change before writing
    /// 3. Reuse and atomically refresh a complete shape-compatible group
    /// 4. Preserve stored metadata when the request omits it
    /// 5. Allocate a fresh complete group only when no tagged rows exist
    /// 6. Reject partial or incompatible groups without mutation
    /// 7. Roll back the whole refresh/allocation if any write fails
    ///
    /// This compatibility entrypoint uses the default reconciliation policy,
    /// so it never authorizes sticky changes or shape replacement.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The request is invalid (empty services, duplicate tags, etc.)
    /// - No base port can be found for the complete offset fallback pattern
    /// - Database operations fail
    /// - A preferred-only port is reserved, excluded, or occupied
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
        self.allocate_group_with_policy(
            conn,
            request,
            occupancy_config,
            GroupReconciliationPolicy::default(),
        )
    }

    /// Allocate or reconcile a group under an explicit safety policy.
    ///
    /// The existing exact-path tagged rows are classified before any write.
    /// The resulting refresh, fresh allocation, or forced replacement is then
    /// executed inside one savepoint.
    pub(crate) fn allocate_group_with_policy(
        &self,
        conn: &rusqlite::Connection,
        request: &GroupAllocationRequest,
        occupancy_config: &OccupancyCheckConfig,
        policy: GroupReconciliationPolicy,
    ) -> Result<GroupAllocationResult> {
        let request = request.normalized()?;

        // The CLI opens an IMMEDIATE outer transaction before execution. This
        // savepoint keeps reconciliation/allocation atomic for library callers
        // too, and rolls timestamp refreshes or inserts back as one unit.
        Database::with_savepoint(conn, "trop_allocate_group", |conn| {
            let existing =
                Database::get_tagged_reservations_by_exact_path(conn, &request.base_path)?;
            let reconciliation = self.plan_reconciliation(&request, &existing, policy)?;
            self.execute_reconciliation(conn, reconciliation, occupancy_config)
        })
    }

    fn plan_reconciliation(
        &self,
        request: &GroupAllocationRequest,
        existing: &[Reservation],
        policy: GroupReconciliationPolicy,
    ) -> Result<GroupReconciliation> {
        if existing.is_empty() {
            return Ok(GroupReconciliation::AllocateFresh(request.clone()));
        }

        match self.compatible_existing_group(request, existing) {
            Ok(result) => {
                let metadata = Self::plan_metadata_refresh(request, existing, policy)?;
                Ok(GroupReconciliation::Refresh { result, metadata })
            }
            Err(Error::ReservationConflict { .. }) if policy.force => {
                let request = Self::replacement_request(request, existing)?;
                let existing_keys = existing
                    .iter()
                    .map(|reservation| reservation.key().clone())
                    .collect();
                Ok(GroupReconciliation::Replace {
                    request,
                    existing_keys,
                })
            }
            Err(error) => Err(error),
        }
    }

    fn execute_reconciliation(
        &self,
        conn: &rusqlite::Connection,
        reconciliation: GroupReconciliation,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<GroupAllocationResult> {
        match reconciliation {
            GroupReconciliation::AllocateFresh(request) => {
                self.allocate_fresh_group(conn, &request, occupancy_config)
            }
            GroupReconciliation::Refresh { result, metadata } => {
                let refreshed_at = SystemTime::now();
                for update in metadata {
                    if !Database::update_metadata_and_last_used_simple(
                        conn,
                        &update.key,
                        update.project.as_deref(),
                        update.task.as_deref(),
                        refreshed_at,
                    )? {
                        return Err(Self::group_conflict_at_path(
                            &update.key.path,
                            "a stored service disappeared while refreshing the group",
                        ));
                    }
                }
                Ok(result)
            }
            GroupReconciliation::Replace {
                request,
                existing_keys,
            } => {
                for key in existing_keys {
                    if !Database::delete_reservation_simple(conn, &key)? {
                        return Err(Self::group_conflict(
                            &request,
                            "a stored service disappeared while replacing the group",
                        ));
                    }
                }
                self.allocate_fresh_group(conn, &request, occupancy_config)
            }
        }
    }

    fn plan_metadata_refresh(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
        policy: GroupReconciliationPolicy,
    ) -> Result<Vec<GroupMetadataUpdate>> {
        existing
            .iter()
            .map(|reservation| {
                let project = Self::reconciled_metadata_field(
                    request,
                    "project",
                    request.project.as_deref(),
                    reservation.project(),
                    policy.force || policy.allow_project_change,
                )?;
                let task = Self::reconciled_metadata_field(
                    request,
                    "task",
                    request.task.as_deref(),
                    reservation.task(),
                    policy.force || policy.allow_task_change,
                )?;

                Ok(GroupMetadataUpdate {
                    key: reservation.key().clone(),
                    project,
                    task,
                })
            })
            .collect()
    }

    fn reconciled_metadata_field(
        request: &GroupAllocationRequest,
        field: &str,
        requested: Option<&str>,
        existing: Option<&str>,
        allowed: bool,
    ) -> Result<Option<String>> {
        let Some(requested) = requested else {
            return Ok(existing.map(str::to_string));
        };

        if Some(requested) != existing && !allowed {
            let permission = match field {
                "project" => "--allow-project-change",
                "task" => "--allow-task-change",
                _ => "--force",
            };
            return Err(Error::StickyFieldChange {
                field: field.to_string(),
                details: format!(
                    "Cannot change group {field} from {existing:?} to {requested:?} \
                     without --force or {permission} (group at {})",
                    request.base_path.display()
                ),
            });
        }

        Ok(Some(requested.to_string()))
    }

    fn replacement_request(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
    ) -> Result<GroupAllocationRequest> {
        let mut replacement = request.clone();
        if replacement.project.is_none() {
            replacement.project = Self::uniform_existing_metadata(
                request,
                existing,
                "project",
                Reservation::project,
            )?;
        }
        if replacement.task.is_none() {
            replacement.task =
                Self::uniform_existing_metadata(request, existing, "task", Reservation::task)?;
        }
        Ok(replacement)
    }

    fn uniform_existing_metadata(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
        field: &str,
        get: fn(&Reservation) -> Option<&str>,
    ) -> Result<Option<String>> {
        let first = existing.first().and_then(get);
        if existing
            .iter()
            .skip(1)
            .any(|reservation| get(reservation) != first)
        {
            return Err(Self::group_conflict(
                request,
                format!(
                    "stored rows disagree on {field}; provide an explicit value \
                     when forcing a shape replacement"
                ),
            ));
        }
        Ok(first.map(str::to_string))
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

            if service.preferred != Some(port) {
                let offset = service.offset.ok_or_else(|| Error::Validation {
                    field: "services".into(),
                    message: format!(
                        "Stored port {port} does not match the preferred port and \
                         the service has no offset fallback"
                    ),
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
        Self::group_conflict_at_path(&request.base_path, reason)
    }

    fn group_conflict_at_path(path: &std::path::Path, reason: impl Into<String>) -> Error {
        Error::ReservationConflict {
            details: format!(
                "existing group at {} is incompatible: {}",
                path.display(),
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
        let mut services = request.services.iter().collect::<Vec<_>>();
        services.sort_unstable_by(|left, right| left.tag.cmp(&right.tag));
        let mut preferred_services = Vec::new();
        let mut offset_services = Vec::new();

        for service in services {
            let Some(preferred) = service.preferred else {
                offset_services.push(service);
                continue;
            };

            let availability =
                self.is_preferred_port_available(preferred, conn, occupancy_config)?;
            if availability == PortAvailability::Available {
                preferred_services.push((service, preferred));
            } else if service.offset.is_some() {
                offset_services.push(service);
            } else {
                return Err(Error::PreferredPortUnavailable {
                    port: preferred,
                    reason: availability
                        .unavailable_reason()
                        .expect("An unavailable preferred port must have a reason"),
                });
            }
        }

        let preferred_ports = preferred_services
            .iter()
            .map(|(_, port)| *port)
            .collect::<HashSet<_>>();

        let base_port = if offset_services.is_empty() {
            None
        } else {
            let pattern: Vec<u16> = offset_services.iter().filter_map(|s| s.offset).collect();
            let base = self
                .find_pattern_match_avoiding(&pattern, &preferred_ports, conn, occupancy_config)?
                .ok_or_else(|| Error::GroupAllocationFailed {
                    attempted: offset_services.len(),
                    reason: "No scanned base can satisfy the complete offset fallback pattern"
                        .into(),
                })?;

            Some(base)
        };

        let mut allocations = HashMap::new();
        let mut reservations_to_create = Vec::new();

        for (service, port) in &preferred_services {
            let key = ReservationKey::new(request.base_path.clone(), Some(service.tag.clone()))?;

            allocations.insert(service.tag.clone(), *port);

            let reservation = Reservation::builder(key, *port)
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
    /// This scans forward from the range minimum looking for the lowest base
    /// port where `base + offset` is available for every offset in the pattern.
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
        self.find_pattern_match_avoiding(pattern, &HashSet::new(), conn, occupancy_config)
    }

    fn find_pattern_match_avoiding(
        &self,
        pattern: &[u16],
        unavailable_in_request: &HashSet<Port>,
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
        let Some(scan_end_value) = end
            .value()
            .checked_sub(max_offset)
            .filter(|scan_end| *scan_end >= start.value())
        else {
            return Ok(None);
        };

        for base_value in start.value()..=scan_end_value {
            let base = Port::try_from(base_value)?;

            // Check if all offsets are available from this base
            let mut all_available = true;
            for &offset in pattern {
                if let Some(port) = base.checked_add(offset) {
                    if !unavailable_in_request.contains(&port)
                        && self.is_port_available(port, conn, occupancy_config)?
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
    use crate::error::PortUnavailableReason;
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

    fn single_preferred_request(
        base_path: &str,
        preferred: Port,
        offset: Option<u16>,
    ) -> GroupAllocationRequest {
        GroupAllocationRequest {
            base_path: PathBuf::from(base_path),
            project: None,
            task: None,
            services: vec![ServiceAllocationRequest {
                tag: "web".to_string(),
                offset,
                preferred: Some(preferred),
            }],
        }
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
    fn test_forced_shape_replacement_does_not_bypass_occupancy() {
        let db = create_test_database();
        let initial_allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let initial = GroupAllocationRequest {
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
        initial_allocator
            .allocate_group(db.connection(), &initial, &occupancy_config)
            .expect("Initial group should allocate");

        let occupied = HashSet::from([Port::try_from(5002).unwrap()]);
        let replacement_allocator = create_test_allocator(occupied, 5000, 5100);
        let replacement = GroupAllocationRequest {
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
                    tag: "db".to_string(),
                    offset: Some(2),
                    preferred: None,
                },
            ],
            ..initial
        };
        let result = replacement_allocator
            .allocate_group_with_policy(
                db.connection(),
                &replacement,
                &occupancy_config,
                GroupReconciliationPolicy {
                    force: true,
                    ..GroupReconciliationPolicy::default()
                },
            )
            .expect("Force should scan past the occupied replacement pattern");

        assert_eq!(result.allocations["web"], Port::try_from(5003).unwrap());
        assert_eq!(result.allocations["api"], Port::try_from(5004).unwrap());
        assert_eq!(result.allocations["db"], Port::try_from(5005).unwrap());
    }

    #[test]
    fn test_group_allocation_accepts_free_preferred_outside_scan_range() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);

        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: Some("test".to_string()),
            task: None,
            services: vec![ServiceAllocationRequest {
                tag: "api".to_string(),
                offset: Some(0),
                preferred: Some(Port::try_from(8080).unwrap()),
            }],
        };

        let config = OccupancyCheckConfig::default();
        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect("A valid free preferred port is independent of the scan range");
        assert_eq!(result.allocations["api"], Port::try_from(8080).unwrap());
        assert_eq!(result.base_port, None);
    }

    #[test]
    fn test_group_occupied_preferred_falls_back_to_offset_pattern() {
        let db = create_test_database();
        let allocator =
            create_test_allocator(HashSet::from([Port::try_from(5050).unwrap()]), 5000, 5100);
        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: Some(Port::try_from(5050).unwrap()),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
            ],
        };

        let result = allocator
            .allocate_group(db.connection(), &request, &OccupancyCheckConfig::default())
            .expect("An unavailable preferred port should use its offset fallback");

        assert_eq!(result.base_port, Some(Port::try_from(5000).unwrap()));
        assert_eq!(result.allocations["web"], Port::try_from(5000).unwrap());
        assert_eq!(result.allocations["api"], Port::try_from(5001).unwrap());
    }

    #[test]
    fn test_group_excluded_preferred_falls_back_to_offset_pattern() {
        let db = create_test_database();
        let preferred = Port::try_from(5050).unwrap();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(preferred);
        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5100).unwrap()).unwrap();
        let allocator =
            PortAllocator::new(MockOccupancyChecker::new(HashSet::new()), exclusions, range);
        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: Some(preferred),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
            ],
        };

        let result = allocator
            .allocate_group(db.connection(), &request, &OccupancyCheckConfig::default())
            .expect("An excluded preferred port should use its offset fallback");

        assert_eq!(result.base_port, Some(Port::try_from(5000).unwrap()));
        assert_eq!(result.allocations["web"], Port::try_from(5000).unwrap());
        assert_eq!(result.allocations["api"], Port::try_from(5001).unwrap());
    }

    #[test]
    fn test_group_reserved_preferred_fallback_is_reused() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let preferred = Port::try_from(5050).unwrap();
        let blocker = Reservation::builder(
            ReservationKey::new(PathBuf::from("/other/project"), Some("blocker".to_string()))
                .unwrap(),
            preferred,
        )
        .build()
        .unwrap();
        Database::create_reservation_simple(db.connection(), &blocker).unwrap();
        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: Some(preferred),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: Some(1),
                    preferred: None,
                },
            ],
        };
        let config = OccupancyCheckConfig::default();

        let first = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect("A reserved preferred port should use its offset fallback");
        let repeated = allocator
            .allocate_group(db.connection(), &request, &config)
            .expect("A stored fallback pattern should remain shape-compatible");

        assert_eq!(repeated, first);
        assert_eq!(first.base_port, Some(Port::try_from(5000).unwrap()));
        assert_eq!(first.allocations["web"], Port::try_from(5000).unwrap());
        assert_eq!(first.allocations["api"], Port::try_from(5001).unwrap());
        assert_eq!(
            Database::list_all_reservations(db.connection())
                .unwrap()
                .len(),
            3,
            "Repeating a fallback allocation must not insert another group"
        );
    }

    #[test]
    fn test_group_preferred_without_fallback_reports_reserved() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let preferred = Port::try_from(5050).unwrap();
        let blocker = Reservation::builder(
            ReservationKey::new(PathBuf::from("/other/project"), Some("blocker".to_string()))
                .unwrap(),
            preferred,
        )
        .build()
        .unwrap();
        Database::create_reservation_simple(db.connection(), &blocker).unwrap();

        let error = allocator
            .allocate_group(
                db.connection(),
                &single_preferred_request("/test/project", preferred, None),
                &OccupancyCheckConfig::default(),
            )
            .expect_err("A reserved preferred-only port must fail");

        assert!(matches!(
            error,
            Error::PreferredPortUnavailable {
                port,
                reason: PortUnavailableReason::Reserved,
            } if port == preferred
        ));
    }

    #[test]
    fn test_group_preferred_without_fallback_reports_excluded() {
        let db = create_test_database();
        let preferred = Port::try_from(5050).unwrap();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(preferred);
        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5100).unwrap()).unwrap();
        let allocator =
            PortAllocator::new(MockOccupancyChecker::new(HashSet::new()), exclusions, range);

        let error = allocator
            .allocate_group(
                db.connection(),
                &single_preferred_request("/test/project", preferred, None),
                &OccupancyCheckConfig::default(),
            )
            .expect_err("An excluded preferred-only port must fail");

        assert!(matches!(
            error,
            Error::PreferredPortUnavailable {
                port,
                reason: PortUnavailableReason::Excluded,
            } if port == preferred
        ));
    }

    #[test]
    fn test_group_preferred_without_fallback_reports_occupied() {
        let db = create_test_database();
        let preferred = Port::try_from(5050).unwrap();
        let allocator = create_test_allocator(HashSet::from([preferred]), 5000, 5100);

        let error = allocator
            .allocate_group(
                db.connection(),
                &single_preferred_request("/test/project", preferred, None),
                &OccupancyCheckConfig::default(),
            )
            .expect_err("An occupied preferred-only port must fail");

        assert!(matches!(
            error,
            Error::PreferredPortUnavailable {
                port,
                reason: PortUnavailableReason::Occupied,
            } if port == preferred
        ));
    }

    #[test]
    fn test_group_preferred_port_collision_advances_fallback_pattern() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let request = GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "admin".to_string(),
                    offset: Some(2),
                    preferred: Some(Port::try_from(5000).unwrap()),
                },
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

        let result = allocator
            .allocate_group(db.connection(), &request, &OccupancyCheckConfig::default())
            .expect("The allocator should scan past an internal preferred collision");

        assert_eq!(result.allocations["admin"], Port::try_from(5000).unwrap());
        assert_eq!(result.base_port, Some(Port::try_from(5001).unwrap()));
        assert_eq!(result.allocations["web"], Port::try_from(5001).unwrap());
        assert_eq!(result.allocations["api"], Port::try_from(5002).unwrap());
        assert_eq!(
            result
                .allocations
                .values()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            result.allocations.len(),
            "Every successful group allocation must use distinct ports"
        );
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
    fn test_group_allocation_rejects_duplicate_offsets_and_preferred_ports() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let duplicate_offsets = GroupAllocationRequest {
            base_path: PathBuf::from("/test/offsets"),
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
                    offset: Some(0),
                    preferred: None,
                },
            ],
        };
        let preferred = Port::try_from(5050).unwrap();
        let duplicate_preferred = GroupAllocationRequest {
            base_path: PathBuf::from("/test/preferred"),
            project: None,
            task: None,
            services: vec![
                ServiceAllocationRequest {
                    tag: "web".to_string(),
                    offset: Some(0),
                    preferred: Some(preferred),
                },
                ServiceAllocationRequest {
                    tag: "api".to_string(),
                    offset: Some(1),
                    preferred: Some(preferred),
                },
            ],
        };

        for (request, expected) in [
            (duplicate_offsets, "Duplicate service offset"),
            (duplicate_preferred, "Duplicate preferred port"),
        ] {
            let error = allocator
                .allocate_group(db.connection(), &request, &OccupancyCheckConfig::default())
                .expect_err("Invalid group constraints must fail before allocation");
            assert!(
                matches!(
                    error,
                    Error::Validation {
                        ref field,
                        ref message
                    } if field == "services" && message.contains(expected)
                ),
                "Expected a precise validation error containing {expected:?}, got {error}"
            );
        }
        assert!(Database::list_all_reservations(db.connection())
            .unwrap()
            .is_empty());
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
    fn test_group_allocation_reports_complete_pattern_exhaustion() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5001);
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
                    offset: Some(2),
                    preferred: None,
                },
            ],
        };

        let error = allocator
            .allocate_group(db.connection(), &request, &OccupancyCheckConfig::default())
            .expect_err("A pattern wider than the scan range must be exhausted");

        assert!(
            matches!(
                error,
                Error::GroupAllocationFailed {
                    attempted: 2,
                    ref reason,
                } if reason.contains("complete offset fallback pattern")
            ),
            "Expected a precise complete-pattern exhaustion error, got {error}"
        );
        assert!(Database::list_all_reservations(db.connection())
            .unwrap()
            .is_empty());
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
