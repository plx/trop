//! Group allocation support for allocating multiple related ports atomically.
//!
//! This module provides functionality for allocating groups of ports with specific
//! offset patterns, useful for microservices or applications that need multiple ports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::database::Database;
use crate::error::Error;
use crate::port::allocator::{PortAllocator, PortAvailability};
use crate::port::occupancy::{OccupancyCheckConfig, PortOccupancyChecker};
use crate::verified::group::{
    decide_group_reconciliation, derive_compatible_base, find_pattern_base, match_stored_tags,
    plan_fresh_group, validate_group_request, CompatClass, CompatError, FreshGroupError,
    GroupDecision, IdentifierIssue, PreferredVerdict, RawService, ShapeError, StoredService,
    TagMatchError, ValidatedGroupShape,
};
use crate::verified::sticky::{
    plan_group_metadata, replacement_metadata, ReplacementSource, StickyField, StickyResolution,
    StoredMetadata,
};
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
    ///
    /// Tags, project, and task are trimmed here; every validation decision is
    /// made by the verified kernel `verified::group::validate_group_request`.
    pub(crate) fn normalized(&self) -> Result<Self> {
        let mut normalized = self.clone();
        normalized.project = normalized
            .project
            .as_deref()
            .map(|value| value.trim().to_string());
        normalized.task = normalized
            .task
            .as_deref()
            .map(|value| value.trim().to_string());
        for service in &mut normalized.services {
            service.tag = service.tag.trim().to_string();
        }

        validated_shape(
            normalized.services.iter(),
            normalized.project.as_deref(),
            normalized.task.as_deref(),
        )?;
        Ok(normalized)
    }

    /// The services of a normalized request sorted by tag, with their
    /// validated shape in the same order.
    ///
    /// Only the services are validated again: a forced replacement may carry
    /// project and task values inherited from stored rows, which are kept as
    /// they are.
    fn sorted_shape(&self) -> Result<(Vec<&ServiceAllocationRequest>, ValidatedGroupShape)> {
        let mut services = self.services.iter().collect::<Vec<_>>();
        services.sort_unstable_by(|left, right| left.tag.cmp(&right.tag));
        let shape = validated_shape(services.iter().copied(), None, None)?;
        Ok((services, shape))
    }
}

/// Validate `services` (already trimmed) together with `project` and `task`,
/// keeping the given service order.
fn validated_shape<'a>(
    services: impl Iterator<Item = &'a ServiceAllocationRequest>,
    project: Option<&str>,
    task: Option<&str>,
) -> Result<ValidatedGroupShape> {
    let raw = services
        .map(|service| RawService {
            tag: service.tag.as_bytes().to_vec(),
            offset: service.offset,
            preferred: service.preferred.map(Port::value),
        })
        .collect();
    validate_group_request(raw, project.map(str::as_bytes), task.map(str::as_bytes))
        .map_err(shape_error)
}

/// The validation message of an identifier rule (the same wording as the
/// configuration validator).
const fn identifier_message(issue: IdentifierIssue) -> &'static str {
    match issue {
        IdentifierIssue::Empty => "Cannot be empty or only whitespace",
        IdentifierIssue::ContainsNul => "Cannot contain null bytes",
        IdentifierIssue::TooLong => "Cannot exceed 255 characters",
    }
}

/// Map a verified group-shape error to the public validation error.
fn shape_error(error: ShapeError) -> Error {
    let (field, message) = match error {
        ShapeError::NoServices => (
            "services",
            "Group allocation requires at least one service".to_string(),
        ),
        ShapeError::Project(issue) => ("project", identifier_message(issue).to_string()),
        ShapeError::Task(issue) => ("task", identifier_message(issue).to_string()),
        ShapeError::Tag { issue, .. } => (
            "services",
            match issue {
                IdentifierIssue::Empty => "Service tags must not be empty or whitespace-only",
                IdentifierIssue::ContainsNul => "Service tags cannot contain null bytes",
                IdentifierIssue::TooLong => "Service tags cannot exceed 255 characters",
            }
            .to_string(),
        ),
        ShapeError::DuplicateTag { .. } => (
            "services",
            "Duplicate service tag after normalization".to_string(),
        ),
        ShapeError::MissingOffsetAndPreferred { .. } => (
            "services",
            "Every service without a preferred port must have an offset".to_string(),
        ),
        ShapeError::ZeroPreferred { .. } => ("services", "Preferred port 0 is invalid".to_string()),
        ShapeError::DuplicateOffset { offset, .. } => {
            ("services", format!("Duplicate service offset: {offset}"))
        }
        ShapeError::DuplicatePreferred { port, .. } => {
            ("services", format!("Duplicate preferred port: {port}"))
        }
    };
    Error::Validation {
        field: field.to_string(),
        message,
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
        let compat = self.compatible_existing_group(request, existing);
        let class = match &compat {
            Ok(_) => CompatClass::Compatible,
            Err(Error::ReservationConflict { .. }) => CompatClass::Conflict,
            Err(_) => CompatClass::OtherError,
        };

        match decide_group_reconciliation(existing.len(), class, policy.force) {
            GroupDecision::AllocateFresh => Ok(GroupReconciliation::AllocateFresh(request.clone())),
            GroupDecision::Replace => {
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
            // The verified decision is `Refresh` only for a compatible group
            // and `Fail` only for a compatibility error, so `compat?` yields
            // the group to refresh or the error to report.
            GroupDecision::Refresh | GroupDecision::Fail => {
                let result = compat?;
                let metadata = Self::plan_metadata_refresh(request, existing, policy)?;
                Ok(GroupReconciliation::Refresh { result, metadata })
            }
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

    /// Plan the sticky metadata of every stored row before any write; the
    /// verified kernel `verified::sticky::plan_group_metadata` decides.
    fn plan_metadata_refresh(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
        policy: GroupReconciliationPolicy,
    ) -> Result<Vec<GroupMetadataUpdate>> {
        let rows = existing
            .iter()
            .map(|reservation| StoredMetadata {
                project: reservation.project().map(|value| value.as_bytes().to_vec()),
                task: reservation.task().map(|value| value.as_bytes().to_vec()),
            })
            .collect::<Vec<_>>();
        let resolutions = plan_group_metadata(
            request.project.as_deref().map(str::as_bytes),
            request.task.as_deref().map(str::as_bytes),
            &rows,
            policy.allow_project_change,
            policy.allow_task_change,
            policy.force,
        )
        .map_err(|violation| {
            let reservation = &existing[violation.row];
            match violation.field {
                StickyField::Project => Self::sticky_change_error(
                    request,
                    "project",
                    request.project.as_deref(),
                    reservation.project(),
                ),
                StickyField::Task => Self::sticky_change_error(
                    request,
                    "task",
                    request.task.as_deref(),
                    reservation.task(),
                ),
            }
        })?;

        Ok(existing
            .iter()
            .zip(resolutions)
            .map(|(reservation, (project, task))| GroupMetadataUpdate {
                key: reservation.key().clone(),
                project: resolved_metadata(
                    project,
                    request.project.as_deref(),
                    reservation.project(),
                ),
                task: resolved_metadata(task, request.task.as_deref(), reservation.task()),
            })
            .collect())
    }

    fn sticky_change_error(
        request: &GroupAllocationRequest,
        field: &str,
        requested: Option<&str>,
        existing: Option<&str>,
    ) -> Error {
        // The kernel reports a violation only for an explicit requested value.
        let requested = requested.unwrap_or_default();
        let permission = match field {
            "project" => "--allow-project-change",
            "task" => "--allow-task-change",
            _ => "--force",
        };
        Error::StickyFieldChange {
            field: field.to_string(),
            details: format!(
                "Cannot change group {field} from {existing:?} to {requested:?} \
                 without --force or {permission} (group at {})",
                request.base_path.display()
            ),
        }
    }

    fn replacement_request(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
    ) -> Result<GroupAllocationRequest> {
        let mut replacement = request.clone();
        replacement.project = Self::replacement_metadata(
            request,
            existing,
            "project",
            request.project.as_deref(),
            Reservation::project,
        )?;
        replacement.task = Self::replacement_metadata(
            request,
            existing,
            "task",
            request.task.as_deref(),
            Reservation::task,
        )?;
        Ok(replacement)
    }

    /// The sticky value of a forced replacement: the explicit request, or the
    /// value every stored row agrees on (decided by the verified kernel
    /// `verified::sticky::replacement_metadata`).
    fn replacement_metadata(
        request: &GroupAllocationRequest,
        existing: &[Reservation],
        field: &str,
        requested: Option<&str>,
        get: fn(&Reservation) -> Option<&str>,
    ) -> Result<Option<String>> {
        let stored = existing
            .iter()
            .map(|reservation| get(reservation).map(|value| value.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        match replacement_metadata(requested.is_some(), &stored) {
            Ok(ReplacementSource::Requested) => Ok(requested.map(str::to_string)),
            Ok(ReplacementSource::InheritStored) => {
                Ok(existing.first().and_then(get).map(str::to_string))
            }
            Err(_) => Err(Self::group_conflict(
                request,
                format!(
                    "stored rows disagree on {field}; provide an explicit value \
                     when forcing a shape replacement"
                ),
            )),
        }
    }

    /// Check that the stored rows form the requested group under the current
    /// scan range. Matching rows to services and the shape check are decided
    /// by the verified kernels `verified::group::match_stored_tags` and
    /// `verified::group::derive_compatible_base`.
    fn compatible_existing_group(
        &self,
        request: &GroupAllocationRequest,
        existing: &[Reservation],
    ) -> Result<GroupAllocationResult> {
        let (services, shape) = request.sorted_shape()?;
        let stored_tags = existing
            .iter()
            .map(|reservation| {
                reservation
                    .key()
                    .tag
                    .as_ref()
                    .map(|tag| tag.as_bytes().to_vec())
            })
            .collect::<Vec<_>>();
        let rows = match_stored_tags(&shape, &stored_tags).map_err(|error| {
            Self::group_conflict(
                request,
                match error {
                    TagMatchError::Untagged { .. } => {
                        "stored group unexpectedly contains an untagged reservation"
                    }
                    TagMatchError::DuplicateStored { .. } => {
                        "stored group contains duplicate service tags"
                    }
                    TagMatchError::SetMismatch => {
                        "requested service set does not match stored service set"
                    }
                },
            )
        })?;

        // The kernel guarantees `rows` is a bijection onto `existing`.
        let stored = services
            .iter()
            .zip(&rows)
            .map(|(service, &row)| StoredService {
                stored: existing[row].port().value(),
                preferred: service.preferred.map(Port::value),
                offset: service.offset,
            })
            .collect::<Vec<_>>();
        let range = self.range();
        let base = derive_compatible_base(&stored, range.min().value(), range.max().value())
            .map_err(|error| Self::compat_conflict(request, &stored, error))?;

        let allocations = services
            .iter()
            .zip(&rows)
            .map(|(service, &row)| (service.tag.clone(), existing[row].port()))
            .collect();
        Ok(GroupAllocationResult {
            allocations,
            base_port: base.map(Port::try_from).transpose()?,
        })
    }

    /// Map a verified shape-compatibility error to a reservation conflict.
    fn compat_conflict(
        request: &GroupAllocationRequest,
        stored: &[StoredService],
        error: CompatError,
    ) -> Error {
        let reason = match error {
            CompatError::NoOffsetFallback { index } => format!(
                "stored port {} does not match the preferred port and the service \
                 has no offset fallback",
                stored[index].stored
            ),
            CompatError::OffsetUnderflow { index } => format!(
                "stored port {} cannot satisfy requested offset {}",
                stored[index].stored,
                stored[index].offset.unwrap_or_default()
            ),
            CompatError::ZeroBase { index } => format!(
                "stored port {} implies invalid base 0",
                stored[index].stored
            ),
            CompatError::OutOfScanRange { .. } => {
                "a stored service mapping is outside the current scan range".to_string()
            }
            CompatError::BaseMismatch {
                candidate, base, ..
            } => format!("stored offset mappings imply different bases ({candidate} and {base})"),
        };
        Self::group_conflict(request, reason)
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

    /// Allocate a fresh group. The verified kernel
    /// `verified::group::plan_fresh_group` decides every port (pinning,
    /// fallback classification, and the lowest-base pattern search); this
    /// glue supplies the probes and performs the inserts.
    fn allocate_fresh_group(
        &self,
        conn: &rusqlite::Connection,
        request: &GroupAllocationRequest,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<GroupAllocationResult> {
        let (services, shape) = request.sorted_shape()?;
        let preferred_probe = |port: u16| -> Result<PreferredVerdict> {
            let availability =
                self.is_preferred_port_available(Port::try_from(port)?, conn, occupancy_config)?;
            Ok(match availability {
                PortAvailability::Available => PreferredVerdict::Available,
                PortAvailability::Reserved => PreferredVerdict::Reserved,
                PortAvailability::Excluded => PreferredVerdict::Excluded,
                PortAvailability::Occupied => PreferredVerdict::Occupied,
            })
        };
        let scan_probe = |port: u16| self.scan_probe(port, conn, occupancy_config);

        let plan = match plan_fresh_group(&shape, self.range().span, &preferred_probe, &scan_probe)
        {
            Ok(plan) => plan,
            Err(FreshGroupError::Probe(error)) => return Err(error),
            Err(FreshGroupError::PreferredOnlyUnavailable { port, verdict }) => {
                let reason = match verdict {
                    PreferredVerdict::Available => PortAvailability::Available,
                    PreferredVerdict::Reserved => PortAvailability::Reserved,
                    PreferredVerdict::Excluded => PortAvailability::Excluded,
                    PreferredVerdict::Occupied => PortAvailability::Occupied,
                }
                .unavailable_reason()
                .expect("verified: the reported preferred-port verdict is never Available");
                return Err(Error::PreferredPortUnavailable {
                    port: Port::try_from(port)?,
                    reason,
                });
            }
            Err(FreshGroupError::PatternExhausted { fallback_count }) => {
                return Err(Error::GroupAllocationFailed {
                    attempted: fallback_count,
                    reason: "No scanned base can satisfy the complete offset fallback pattern"
                        .into(),
                });
            }
        };

        let mut allocations = HashMap::new();
        let mut reservations_to_create = Vec::new();

        // Pinned services first, then fallback services, each in tag order.
        for pinned in [true, false] {
            for (index, service) in services.iter().enumerate() {
                if plan.pinned[index] != pinned {
                    continue;
                }
                // Infallible: the kernel proves every planned port is >= 1.
                let port = Port::try_from(plan.ports[index])?;
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
            base_port: plan.base.map(Port::try_from).transpose()?,
        })
    }

    /// Whether `port` is available to a fallback service (in range, not
    /// reserved, not excluded, not occupied).
    fn scan_probe(
        &self,
        port: u16,
        conn: &rusqlite::Connection,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<bool> {
        Ok(
            self.is_port_available(Port::try_from(port)?, conn, occupancy_config)?
                == PortAvailability::Available,
        )
    }

    /// Find a base port where all offsets in the pattern are available.
    ///
    /// This scans forward from the range minimum looking for the lowest base
    /// port where `base + offset` is available for every offset in the pattern.
    /// The search (bounds, order, early stop, and minimality) is the verified
    /// kernel `verified::group::find_pattern_base`.
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
        let probe = |port: u16| self.scan_probe(port, conn, occupancy_config);
        let range = self.range();
        let base = find_pattern_base(
            range.min().value(),
            range.max().value(),
            pattern,
            &[],
            &probe,
        )?;
        // Infallible: the kernel returns a base >= the range minimum (>= 1).
        Ok(base.map(Port::try_from).transpose()?)
    }
}

/// The value a verified sticky resolution selects.
fn resolved_metadata(
    resolution: StickyResolution,
    requested: Option<&str>,
    existing: Option<&str>,
) -> Option<String> {
    match resolution {
        StickyResolution::KeepExisting => existing.map(str::to_string),
        StickyResolution::UseRequested => requested.map(str::to_string),
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
        assert!(
            reservations.is_empty(),
            "unexpected reservations: {reservations:?}"
        );
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
        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert!(
            remaining.is_empty(),
            "unexpected reservations: {remaining:?}"
        );
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
        let remaining = Database::list_all_reservations(db.connection()).unwrap();
        assert!(
            remaining.is_empty(),
            "unexpected reservations: {remaining:?}"
        );
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

    fn service(tag: &str, offset: Option<u16>, preferred: Option<u16>) -> ServiceAllocationRequest {
        ServiceAllocationRequest {
            tag: tag.to_string(),
            offset,
            preferred: preferred.map(|port| Port::try_from(port).unwrap()),
        }
    }

    fn group_request(services: Vec<ServiceAllocationRequest>) -> GroupAllocationRequest {
        GroupAllocationRequest {
            base_path: PathBuf::from("/test/project"),
            project: None,
            task: None,
            services,
        }
    }

    fn validation_message(result: Result<GroupAllocationRequest>) -> (String, String) {
        match result {
            Err(Error::Validation { field, message }) => (field, message),
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    /// Regression: a preferred-only service whose stored port no longer
    /// matches its preference is an incompatible group (a reservation
    /// conflict), not a validation error, so `force` can replace it.
    #[test]
    fn test_preferred_only_mismatch_is_conflict_and_force_replaces_it() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let config = OccupancyCheckConfig::default();

        let original = group_request(vec![service("web", None, Some(5050))]);
        allocator
            .allocate_group(db.connection(), &original, &config)
            .expect("A free preferred-only port should be allocated");

        let changed = group_request(vec![service("web", None, Some(5060))]);
        let error = allocator
            .allocate_group(db.connection(), &changed, &config)
            .expect_err("A moved preference must not silently refresh the stored group");
        assert!(
            matches!(error, Error::ReservationConflict { .. }),
            "expected a reservation conflict, got {error:?}"
        );

        let replaced = allocator
            .allocate_group_with_policy(
                db.connection(),
                &changed,
                &config,
                GroupReconciliationPolicy {
                    force: true,
                    ..GroupReconciliationPolicy::default()
                },
            )
            .expect("Force should replace the incompatible preferred-only group");
        assert_eq!(replaced.allocations["web"], Port::try_from(5060).unwrap());
        assert_eq!(replaced.base_port, None);
        let stored = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].port(), Port::try_from(5060).unwrap());
    }

    /// Regression: group project, task, and tags follow the identifier rules
    /// of a single reservation (no NUL bytes, at most 255 bytes).
    #[test]
    fn test_group_request_enforces_identifier_rules() {
        let long = "x".repeat(256);
        let limit = "x".repeat(255);

        let mut request = group_request(vec![service("web", Some(0), None)]);
        request.task = Some(long.clone());
        assert_eq!(
            validation_message(request.normalized()),
            (
                "task".to_string(),
                "Cannot exceed 255 characters".to_string()
            )
        );

        request.task = Some(format!("  {limit}  "));
        let normalized = request
            .normalized()
            .expect("255 bytes after trimming is valid");
        assert_eq!(normalized.task.as_deref(), Some(limit.as_str()));

        request.task = Some("dev\0".to_string());
        assert_eq!(
            validation_message(request.normalized()),
            ("task".to_string(), "Cannot contain null bytes".to_string())
        );

        request.task = None;
        request.project = Some(long.clone());
        assert_eq!(
            validation_message(request.normalized()),
            (
                "project".to_string(),
                "Cannot exceed 255 characters".to_string()
            )
        );

        request.project = Some("  ".to_string());
        assert_eq!(
            validation_message(request.normalized()),
            (
                "project".to_string(),
                "Cannot be empty or only whitespace".to_string()
            )
        );

        let request = group_request(vec![service(&long, Some(0), None)]);
        assert_eq!(
            validation_message(request.normalized()),
            (
                "services".to_string(),
                "Service tags cannot exceed 255 characters".to_string()
            )
        );

        let request = group_request(vec![service("we\0b", Some(0), None)]);
        assert_eq!(
            validation_message(request.normalized()),
            (
                "services".to_string(),
                "Service tags cannot contain null bytes".to_string()
            )
        );
    }

    #[test]
    fn test_group_request_reports_first_error_in_service_order() {
        // Service 1 repeats an offset and service 2 repeats a preferred port;
        // the earlier service's error wins.
        let request = group_request(vec![
            service("a", Some(0), Some(6000)),
            service("b", Some(0), Some(6001)),
            service("c", Some(1), Some(6000)),
        ]);
        assert_eq!(
            validation_message(request.normalized()),
            (
                "services".to_string(),
                "Duplicate service offset: 0".to_string()
            )
        );

        // Within one service, a duplicate tag is reported before anything else.
        let request = group_request(vec![
            service("a", Some(0), Some(6000)),
            service(" a ", None, None),
        ]);
        assert_eq!(
            validation_message(request.normalized()),
            (
                "services".to_string(),
                "Duplicate service tag after normalization".to_string()
            )
        );

        // Group-level errors come first.
        let mut request = group_request(Vec::new());
        request.project = Some(String::new());
        assert_eq!(
            validation_message(request.normalized()),
            (
                "services".to_string(),
                "Group allocation requires at least one service".to_string()
            )
        );
    }

    #[test]
    fn test_find_pattern_match_at_top_of_port_domain() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 65530, 65535);
        let config = OccupancyCheckConfig::default();

        // The widest pattern that fits: base 65530, ports 65530 and 65535.
        assert_eq!(
            allocator
                .find_pattern_match(&[5, 0], db.connection(), &config)
                .unwrap(),
            Some(Port::try_from(65530).unwrap())
        );
        // One wider and no base keeps every port in range.
        assert_eq!(
            allocator
                .find_pattern_match(&[0, 6], db.connection(), &config)
                .unwrap(),
            None
        );
        // Offset 65535 can never fit, even from base 1.
        assert_eq!(
            allocator
                .find_pattern_match(&[u16::MAX], db.connection(), &config)
                .unwrap(),
            None
        );
    }

    #[test]
    fn test_find_pattern_match_at_bottom_of_port_domain() {
        let db = create_test_database();
        let occupied: HashSet<Port> = [Port::try_from(2).unwrap()].into_iter().collect();
        let allocator = create_test_allocator(occupied, 1, 5);
        let config = OccupancyCheckConfig::default();

        // Base 1 needs port 2 (occupied); base 2 is itself occupied; base 3
        // works and is the lowest.
        assert_eq!(
            allocator
                .find_pattern_match(&[0, 1], db.connection(), &config)
                .unwrap(),
            Some(Port::try_from(3).unwrap())
        );
        // A single-port range with a zero offset.
        let single = create_test_allocator(HashSet::new(), 1, 1);
        assert_eq!(
            single
                .find_pattern_match(&[0], db.connection(), &config)
                .unwrap(),
            Some(Port::try_from(1).unwrap())
        );
    }

    #[test]
    fn test_fresh_group_pins_preferred_ports_at_domain_edges() {
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5010);
        let config = OccupancyCheckConfig::default();
        let request = group_request(vec![
            service("high", Some(0), Some(65535)),
            service("low", Some(1), Some(1)),
            service("mid", Some(2), None),
        ]);

        let result = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();
        assert_eq!(result.allocations["high"], Port::try_from(65535).unwrap());
        assert_eq!(result.allocations["low"], Port::try_from(1).unwrap());
        assert_eq!(result.allocations["mid"], Port::try_from(5002).unwrap());
        assert_eq!(result.base_port, Some(Port::try_from(5000).unwrap()));

        // The repeated request takes the refresh path with the same mapping.
        let repeated = allocator
            .allocate_group(db.connection(), &request, &config)
            .unwrap();
        assert_eq!(repeated, result);
    }

    #[test]
    fn test_stored_base_out_of_scan_range_is_conflict() {
        let db = create_test_database();
        let config = OccupancyCheckConfig::default();
        let request = group_request(vec![
            service("web", Some(0), None),
            service("api", Some(1), None),
        ]);
        create_test_allocator(HashSet::new(), 5000, 5100)
            .allocate_group(db.connection(), &request, &config)
            .unwrap();

        // The configured range moved: the stored base 5000 is now outside it.
        let error = create_test_allocator(HashSet::new(), 5001, 5100)
            .allocate_group(db.connection(), &request, &config)
            .expect_err("A stored group outside the scan range is incompatible");
        assert!(
            matches!(&error, Error::ReservationConflict { details } if details.contains("outside the current scan range")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn test_forced_replacement_keeps_inherited_legacy_metadata() {
        // Rows stored before identifier limits applied to groups may carry a
        // project the request validation would reject; a forced replacement
        // still inherits it unchanged when the request omits the project.
        let db = create_test_database();
        let allocator = create_test_allocator(HashSet::new(), 5000, 5100);
        let config = OccupancyCheckConfig::default();
        let legacy_project = "p".repeat(300);
        let stored = Reservation::builder(
            ReservationKey::new(PathBuf::from("/test/project"), Some("old".to_string())).unwrap(),
            Port::try_from(5000).unwrap(),
        )
        .project(Some(legacy_project.clone()))
        .build()
        .unwrap();
        Database::create_reservation_simple(db.connection(), &stored).unwrap();

        let request = group_request(vec![service("web", Some(0), None)]);
        let result = allocator
            .allocate_group_with_policy(
                db.connection(),
                &request,
                &config,
                GroupReconciliationPolicy {
                    force: true,
                    ..GroupReconciliationPolicy::default()
                },
            )
            .expect("Force should replace the incompatible stored group");

        assert_eq!(result.allocations["web"], Port::try_from(5000).unwrap());
        let rows = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key().tag.as_deref(), Some("web"));
        assert_eq!(rows[0].project(), Some(legacy_project.as_str()));
    }
}
