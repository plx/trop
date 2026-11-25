//! Port allocation with occupancy checking and exclusion support.
//!
//! This module implements the core port allocation algorithm, which finds
//! available ports while respecting database reservations, exclusions, and
//! system occupancy.

use rusqlite::Connection;

use crate::config::PortConfig;
use crate::database::Database;
use crate::error::{Error, PortUnavailableReason};
use crate::verified::allocation::{self, Blocker, Checks, SingleOutcome};
use crate::verified::port::PortSpan;
use crate::verified::port_bounds::{resolve_port_bounds, PortBoundsError};
use crate::{PortExhaustionBlockers, PortRange, ReservationKey, Result};

use super::exclusions::ExclusionManager;
use super::occupancy::{OccupancyCheckConfig, PortOccupancyChecker, SystemOccupancyChecker};
use super::{InvalidPortError, Port};

/// Options for port allocation.
///
/// Controls how the allocator selects ports, including preferred ports
/// and flags to bypass certain checks.
///
/// # Examples
///
/// ```
/// use trop::port::allocator::AllocationOptions;
/// use trop::Port;
///
/// let options = AllocationOptions {
///     preferred: Some(Port::try_from(8080).unwrap()),
///     ignore_occupied: false,
///     ignore_exclusions: false,
/// };
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllocationOptions {
    /// Preferred port to allocate if available.
    pub preferred: Option<Port>,
    /// If true, don't fail if the preferred port is occupied.
    pub ignore_occupied: bool,
    /// If true, don't fail if the preferred port is excluded.
    pub ignore_exclusions: bool,
}

/// Result of a port allocation attempt.
///
/// Represents the outcome of trying to allocate a single port.
///
/// # Examples
///
/// ```
/// use trop::port::allocator::AllocationResult;
/// use trop::Port;
///
/// let result = AllocationResult::Allocated(Port::try_from(8080).unwrap());
/// match result {
///     AllocationResult::Allocated(port) => println!("Allocated port {}", port),
///     _ => println!("Allocation failed"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationResult {
    /// Successfully allocated a port.
    Allocated(Port),
    /// The preferred port was unavailable.
    PreferredUnavailable {
        /// The preferred port that was unavailable.
        port: Port,
        /// The reason it was unavailable.
        reason: PortUnavailableReason,
    },
    /// No ports available in the range.
    Exhausted {
        /// Suggests whether cleanup might help free up ports.
        cleanup_suggested: bool,
        /// Whether cleanup was already attempted.
        tried_cleanup: bool,
    },
}

/// Stateless port allocator.
///
/// The `PortAllocator` finds available ports by checking against database
/// reservations, exclusion lists, and actual system occupancy. It's designed
/// to be stateless, with all state coming from the Database and Config.
///
/// # Type Parameters
///
/// * `C` - The port occupancy checker implementation (defaults to `SystemOccupancyChecker`)
///
/// # Examples
///
/// ```no_run
/// use trop::port::allocator::{PortAllocator, AllocationOptions};
/// use trop::port::occupancy::SystemOccupancyChecker;
/// use trop::port::exclusions::ExclusionManager;
/// use trop::database::{Database, DatabaseConfig};
/// use trop::{Port, PortRange};
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
/// // Open database and allocate a port
/// let config = DatabaseConfig::new("/tmp/trop.db");
/// let db = Database::open(config).unwrap();
///
/// let options = AllocationOptions::default();
/// // let result = allocator.allocate_single(db.connection(), &options, &occupancy_config);
/// ```
#[derive(Debug, Clone)]
pub struct PortAllocator<C: PortOccupancyChecker = SystemOccupancyChecker> {
    checker: C,
    exclusions: ExclusionManager,
    range: PortRange,
}

impl<C: PortOccupancyChecker> PortAllocator<C> {
    /// Create a new port allocator.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::allocator::PortAllocator;
    /// use trop::port::occupancy::SystemOccupancyChecker;
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::{Port, PortRange};
    ///
    /// let checker = SystemOccupancyChecker;
    /// let exclusions = ExclusionManager::empty();
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(7000).unwrap(),
    /// ).unwrap();
    ///
    /// let allocator = PortAllocator::new(checker, exclusions, range);
    /// ```
    #[must_use]
    pub fn new(checker: C, exclusions: ExclusionManager, range: PortRange) -> Self {
        Self {
            checker,
            exclusions,
            range,
        }
    }

    /// Get the port range for this allocator.
    #[must_use]
    pub fn range(&self) -> &PortRange {
        &self.range
    }

    /// Get the exclusion manager for this allocator.
    #[must_use]
    pub fn exclusions(&self) -> &ExclusionManager {
        &self.exclusions
    }

    /// Allocate a single port.
    ///
    /// This implements the forward-scanning algorithm:
    /// 1. If a preferred port is specified and available, use it
    /// 2. Otherwise, scan forward from range minimum
    /// 3. For each candidate port, check it's not reserved, excluded, or occupied
    /// 4. If exhausted and cleanup is enabled, try cleanup and retry
    /// 5. Return the first available port or exhaustion
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database queries fail
    /// - Occupancy checks fail (system errors)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::port::allocator::{PortAllocator, AllocationOptions, AllocationResult};
    /// use trop::port::occupancy::{SystemOccupancyChecker, OccupancyCheckConfig};
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Port, PortRange};
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
    /// let db = Database::open(config).unwrap();
    ///
    /// let options = AllocationOptions::default();
    /// let occupancy_config = OccupancyCheckConfig::default();
    ///
    /// let result = allocator.allocate_single(db.connection(), &options, &occupancy_config).unwrap();
    /// match result {
    ///     AllocationResult::Allocated(port) => println!("Allocated {}", port),
    ///     AllocationResult::Exhausted { .. } => println!("No ports available"),
    ///     _ => println!("Preferred port unavailable"),
    /// }
    /// ```
    pub fn allocate_single(
        &self,
        conn: &Connection,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<AllocationResult> {
        self.allocate_single_ignoring(conn, options, occupancy_config, None)
    }

    /// Allocate a replacement while treating the reservation currently owned
    /// by `ignored_key` as available to itself.
    pub(crate) fn allocate_single_replacing(
        &self,
        conn: &Connection,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
        ignored_key: &ReservationKey,
    ) -> Result<AllocationResult> {
        self.allocate_single_ignoring(conn, options, occupancy_config, Some(ignored_key))
    }

    /// Classify the reasons the configured fallback range remains exhausted.
    ///
    /// This repeats the same ordered availability checks as the forward scan
    /// and is used only to attach typed diagnostics to a terminal failure.
    /// The fold is the verified `verified::allocation::exhaustion_blockers`
    /// kernel: a flag is set only if some port of the range was classified
    /// that way, every port's classification is reflected in the flags, and
    /// errors propagate.
    pub(crate) fn exhaustion_blockers(
        &self,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<PortExhaustionBlockers> {
        let blockers = allocation::exhaustion_blockers(
            self.range.span,
            &Self::reservation_oracle(conn, ignored_key),
            &self.exclusion_oracle(),
            &self.occupancy_oracle(occupancy_config),
        )
        .map_err(|failure| failure.error)?;

        Ok(PortExhaustionBlockers::new(
            blockers.reserved,
            blockers.excluded,
            blockers.occupied,
        ))
    }

    fn allocate_single_ignoring(
        &self,
        conn: &Connection,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<AllocationResult> {
        // The verified `verified::allocation::allocate_single` kernel decides:
        // a preferred port is checked for being in range, then for a
        // reservation by another key (absolute: no flag bypasses it), then -
        // each independently bypassable - against the exclusions and the
        // occupancy probe. Without a preferred port, the forward scan from the
        // range minimum applies, with every check.
        let outcome = allocation::allocate_single(
            self.range.span,
            options.preferred.map(Port::value),
            options.ignore_exclusions,
            options.ignore_occupied,
            &Self::reservation_oracle(conn, ignored_key),
            &self.exclusion_oracle(),
            &self.occupancy_oracle(occupancy_config),
        )
        .map_err(|failure| failure.error)?;

        // Both ports below are valid: an allocated port lies in the range,
        // and an unavailable one is the preferred `Port` itself.
        Ok(match outcome {
            SingleOutcome::Allocated(port) => AllocationResult::Allocated(Port(port)),
            SingleOutcome::PreferredUnavailable(port, blocker) => {
                AllocationResult::PreferredUnavailable {
                    port: Port(port),
                    reason: reason_for(blocker),
                }
            }
            // No ports available - suggest cleanup might help
            SingleOutcome::Exhausted => AllocationResult::Exhausted {
                cleanup_suggested: true,
                tried_cleanup: false,
            },
        })
    }

    /// Find the next available port starting from the given port.
    ///
    /// Scans forward from `start` to find the first port that is:
    /// - Within the range
    /// - Not reserved in the database
    /// - Not in the exclusion list
    /// - Not occupied on the system
    ///
    /// # Errors
    ///
    /// Returns an error if database queries or occupancy checks fail.
    pub fn find_next_available(
        &self,
        start: Port,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<Option<Port>> {
        self.find_next_available_ignoring(start, conn, occupancy_config, None)
    }

    fn find_next_available_ignoring(
        &self,
        start: Port,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<Option<Port>> {
        // A start above the range maximum is reported as an invalid range.
        PortRange::new(start, self.range.max())?;

        // Verified forward scan: the lowest port in `start..=max` whose
        // classification (with every check) is `Available`.
        let found = allocation::first_available(
            start.value(),
            self.range.span,
            &Self::reservation_oracle(conn, ignored_key),
            &self.exclusion_oracle(),
            &self.occupancy_oracle(occupancy_config),
        )
        .map_err(|failure| failure.error)?;

        // A found port lies in the range, so it is a valid port.
        Ok(found.map(Port))
    }

    /// Find the next available port that can be atomically allocated.
    ///
    /// This method performs allocation without checking reservations in
    /// advance (its reservation oracle never reports a port reserved).
    /// Instead, it relies on the atomic insertion with UNIQUE constraint to
    /// detect conflicts. This is suitable for use within a reservation
    /// transaction.
    ///
    /// The method scans forward from `start` checking only exclusions and occupancy,
    /// then returns the first candidate port. The caller should attempt to insert
    /// this port atomically and retry if it fails due to UNIQUE constraint.
    ///
    /// # Errors
    ///
    /// Returns an error if `start` is above the range maximum, or if an
    /// occupancy check fails: the scan stops at the first port whose probe
    /// failed.
    pub fn find_next_allocatable(
        &self,
        start: Port,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<Option<Port>> {
        // A start above the range maximum is reported as an invalid range.
        PortRange::new(start, self.range.max())?;

        // The verified forward scan with a reservation oracle that never
        // blocks: ports outside the range or excluded are skipped without a
        // probe, and the first port the probe reports free is returned. Probe
        // failures are returned with their diagnostic instead of being erased;
        // this remains fail-closed because no port is allocated after an
        // uncertain result.
        let never_reserved = |_port: u16| -> Result<bool> { Ok(false) };
        let found = allocation::first_available(
            start.value(),
            self.range.span,
            &never_reserved,
            &self.exclusion_oracle(),
            &self.occupancy_oracle(occupancy_config),
        )
        .map_err(|failure| failure.error)?;

        // A found port lies in the range, so it is a valid port.
        Ok(found.map(Port))
    }

    /// Check if a specific port is available for allocation.
    ///
    /// A port is available if it is:
    /// 1. Within the configured range
    /// 2. Not reserved in the database
    /// 3. Not in the exclusion list
    /// 4. Not occupied on the system
    ///
    /// # Errors
    ///
    /// Returns an error if database queries or occupancy checks fail.
    pub(super) fn is_port_available(
        &self,
        port: Port,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<PortAvailability> {
        self.port_availability(port, conn, occupancy_config, true, None)
    }

    /// Check a preferred port against reservations, exclusions, and operating
    /// system occupancy without applying the ordinary scan-range boundary.
    ///
    /// Preferred ports use the full valid [`Port`] domain. Only fallback scans
    /// are constrained by [`Self::range`].
    pub(super) fn is_preferred_port_available(
        &self,
        port: Port,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<PortAvailability> {
        self.port_availability(port, conn, occupancy_config, false, None)
    }

    fn port_availability(
        &self,
        port: Port,
        conn: &Connection,
        occupancy_config: &OccupancyCheckConfig,
        require_in_range: bool,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<PortAvailability> {
        // Verified classification: out of range (when required) is
        // `Excluded`; then reservations by another key, then exclusions, then
        // the occupancy probe, each consulted only if the earlier ones passed.
        // Returning a probe failure is fail-closed (the port cannot become
        // Available) and preserves the actionable
        // protocol/family/scope/address diagnostic.
        allocation::classify_port(
            port.value(),
            Checks {
                require_in_range,
                ignore_exclusions: false,
                ignore_occupied: false,
            },
            self.range.span,
            &Self::reservation_oracle(conn, ignored_key),
            &self.exclusion_oracle(),
            &self.occupancy_oracle(occupancy_config),
        )
    }

    fn is_reserved_by_other(
        conn: &Connection,
        port: Port,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<bool> {
        Ok(Database::get_reservation_by_port(conn, port)?
            .is_some_and(|reservation| ignored_key != Some(reservation.key())))
    }

    // Oracles for the verified kernels. A kernel consults its oracles only at
    // valid ports (its precondition grants the oracles' preconditions on
    // `1..=65535` alone), so rebuilding a `Port` from the `u16` is sound.

    /// Reservation oracle: whether a port is reserved by a key other than
    /// `ignored_key`.
    fn reservation_oracle<'a>(
        conn: &'a Connection,
        ignored_key: Option<&'a ReservationKey>,
    ) -> impl Fn(u16) -> Result<bool> + 'a {
        move |port| Self::is_reserved_by_other(conn, Port(port), ignored_key)
    }

    /// Exclusion oracle: whether a port is on the exclusion list.
    fn exclusion_oracle(&self) -> impl Fn(u16) -> bool + '_ {
        |port| self.exclusions.is_excluded(Port(port))
    }

    /// Occupancy oracle: the operating-system probe.
    fn occupancy_oracle<'a>(
        &'a self,
        occupancy_config: &'a OccupancyCheckConfig,
    ) -> impl Fn(u16) -> Result<bool> + 'a {
        move |port| self.checker.is_occupied(Port(port), occupancy_config)
    }
}

/// Why a port is (un)available: the verified kernel's classification
/// (`Available`, `Reserved`, `Excluded` - on the exclusion list, or outside
/// the range when the range applies - or `Occupied`).
pub(super) use crate::verified::allocation::Availability as PortAvailability;

impl PortAvailability {
    /// Return the public reason for an unavailable port.
    ///
    /// Verified total: `None` exactly for `Available`.
    pub(super) const fn unavailable_reason(self) -> Option<PortUnavailableReason> {
        match self.blocker() {
            None => None,
            Some(blocker) => Some(reason_for(blocker)),
        }
    }
}

/// The public reason for a kernel blocker (a one-to-one mapping).
const fn reason_for(blocker: Blocker) -> PortUnavailableReason {
    match blocker {
        Blocker::Reserved => PortUnavailableReason::Reserved,
        Blocker::Excluded => PortUnavailableReason::Excluded,
        Blocker::Occupied => PortUnavailableReason::Occupied,
    }
}

/// Resolve a port configuration to the range it describes.
///
/// An explicit `max` wins; otherwise the range ends at `min + max_offset`
/// (computed without overflow). This is the one resolution
/// [`allocator_from_config`], `trop scan`, and `reserve-group`'s
/// `reservations.base` narrowing use, decided by a verified kernel (see
/// `docs/verification.md`). It is more permissive than configuration
/// validation, which also rejects giving both `max` and `max_offset`, and a
/// zero `max_offset`. The `ports` section of an effective configuration
/// (built by [`crate::config::ConfigBuilder`]) always holds exactly one
/// absolute `max`, so the "neither bound" error below arises only for raw
/// sections.
///
/// # Errors
///
/// The first applicable of:
/// - `min` is 0: [`Error::InvalidPort`];
/// - an explicit `max` is 0: [`Error::InvalidPort`];
/// - an explicit `max` is below `min`: [`Error::InvalidPortRange`];
/// - `min + max_offset` exceeds 65535: [`Error::Validation`] on
///   `ports.max_offset`;
/// - neither `max` nor `max_offset` is given: [`Error::Validation`] on
///   `ports`.
///
/// # Examples
///
/// ```
/// use trop::config::PortConfig;
/// use trop::port::allocator::port_range_from_config;
///
/// let ports = PortConfig { min: 8000, max: None, max_offset: Some(100) };
/// let range = port_range_from_config(&ports).unwrap();
/// assert_eq!((range.min().value(), range.max().value()), (8000, 8100));
///
/// let overflow = PortConfig { min: 65000, max: None, max_offset: Some(1000) };
/// assert!(port_range_from_config(&overflow).is_err());
/// ```
pub fn port_range_from_config(port_config: &PortConfig) -> Result<PortRange> {
    port_span_from_config(port_config).map(|span| PortRange {
        span,
        exhaustion_details: None,
    })
}

/// [`port_range_from_config`]'s resolution, as the verified span (for glue
/// that hands the range to another kernel, such as `reserve-group`'s base
/// narrowing).
pub(crate) fn port_span_from_config(port_config: &PortConfig) -> Result<PortSpan> {
    resolve_port_bounds(port_config.min, port_config.max, port_config.max_offset)
        .map_err(|error| port_bounds_error(port_config.min, error))
}

/// The error value `allocator_from_config` has always reported for each
/// port-bounds resolution failure.
fn port_bounds_error(min: u16, error: PortBoundsError) -> Error {
    match error {
        // The kernel reports `InvalidMin` only for `min == 0`, and
        // `InvalidMax` only for an explicit `max` of 0.
        PortBoundsError::InvalidMin => invalid_port_error(min),
        PortBoundsError::InvalidMax => invalid_port_error(0),
        PortBoundsError::MaxBelowMin { max } => Error::InvalidPortRange {
            min,
            max,
            reason: "max must be greater than or equal to min".into(),
        },
        PortBoundsError::OffsetOverflow { offset } => Error::Validation {
            field: "ports.max_offset".into(),
            message: format!("Offset {offset} would overflow when added to min port {min}"),
        },
        PortBoundsError::MissingMax => Error::Validation {
            field: "ports".into(),
            message: "Either max or max_offset must be specified".into(),
        },
    }
}

/// The error `Port::try_from` reports for an invalid port number.
fn invalid_port_error(value: u16) -> Error {
    InvalidPortError {
        value,
        reason: "port 0 is invalid".into(),
    }
    .into()
}

/// Helper to create an allocator from configuration.
///
/// This is a convenience function that constructs all the pieces needed
/// for an allocator from configuration objects.
///
/// # Errors
///
/// Returns an error if:
/// - Port range is invalid
/// - Exclusions are invalid
pub fn allocator_from_config(
    config: &crate::config::Config,
) -> Result<PortAllocator<SystemOccupancyChecker>> {
    // Extract port range from config
    let port_config = config.ports.as_ref().ok_or_else(|| Error::Validation {
        field: "ports".into(),
        message: "Port configuration is required".into(),
    })?;

    let range = port_range_from_config(port_config)?;

    // Create exclusion manager
    let exclusions = if let Some(ref excluded_ports) = config.excluded_ports {
        ExclusionManager::from_config(excluded_ports)?
    } else {
        ExclusionManager::empty()
    };

    // Create checker
    let checker = SystemOccupancyChecker;

    Ok(PortAllocator::new(checker, exclusions, range))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::create_test_database;
    use crate::port::occupancy::{MockOccupancyChecker, PortOccupancyChecker};
    use crate::reservation::{Reservation, ReservationKey};
    use std::collections::HashSet;
    use std::io;
    use std::path::PathBuf;

    #[derive(Debug, Clone, Copy)]
    struct FailingOccupancyChecker;

    impl PortOccupancyChecker for FailingOccupancyChecker {
        fn is_occupied(&self, port: Port, _config: &OccupancyCheckConfig) -> Result<bool> {
            Err(Error::OccupancyCheckFailed {
                port,
                source: Box::new(io::Error::other(
                    "TCP IPv6 localhost bind at [::1] failed: synthetic probe failure",
                )),
            })
        }
    }

    fn create_failing_allocator(min: u16, max: u16) -> PortAllocator<FailingOccupancyChecker> {
        let range =
            PortRange::new(Port::try_from(min).unwrap(), Port::try_from(max).unwrap()).unwrap();
        PortAllocator::new(FailingOccupancyChecker, ExclusionManager::empty(), range)
    }

    fn create_test_allocator(
        occupied: HashSet<Port>,
        exclusions: ExclusionManager,
        min: u16,
        max: u16,
    ) -> PortAllocator<MockOccupancyChecker> {
        let checker = MockOccupancyChecker::new(occupied);
        let range =
            PortRange::new(Port::try_from(min).unwrap(), Port::try_from(max).unwrap()).unwrap();
        PortAllocator::new(checker, exclusions, range)
    }

    #[test]
    fn test_allocate_single_first_port() {
        let db = create_test_database();
        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5000).unwrap())
        );
    }

    #[test]
    fn test_allocation_propagates_actionable_occupancy_probe_failure() {
        let db = create_test_database();
        let allocator = create_failing_allocator(5000, 5001);
        let config = OccupancyCheckConfig::default();

        let error = allocator
            .allocate_single(db.connection(), &AllocationOptions::default(), &config)
            .unwrap_err();
        let diagnostic = error.to_string();
        assert!(matches!(error, Error::OccupancyCheckFailed { .. }));
        assert!(diagnostic.contains("port 5000"));
        assert!(diagnostic.contains("possibly occupied"));
        assert!(diagnostic.contains("TCP IPv6 localhost bind at [::1]"));

        let preferred = AllocationOptions {
            preferred: Some(Port::try_from(5001).unwrap()),
            ..Default::default()
        };
        let error = allocator
            .allocate_single(db.connection(), &preferred, &config)
            .unwrap_err();
        assert!(error.to_string().contains("port 5001"));

        let error = allocator
            .find_next_allocatable(Port::try_from(5000).unwrap(), &config)
            .unwrap_err();
        assert!(error.to_string().contains("synthetic probe failure"));

        let error = allocator
            .is_port_available(Port::try_from(5000).unwrap(), db.connection(), &config)
            .unwrap_err();
        assert!(error.to_string().contains("synthetic probe failure"));
    }

    #[test]
    fn test_ignore_occupied_bypasses_occupancy_probe_failure() {
        let db = create_test_database();
        let allocator = create_failing_allocator(5000, 5000);
        let options = AllocationOptions {
            preferred: Some(Port::try_from(5000).unwrap()),
            ignore_occupied: true,
            ..Default::default()
        };

        assert_eq!(
            allocator
                .allocate_single(db.connection(), &options, &OccupancyCheckConfig::default(),)
                .unwrap(),
            AllocationResult::Allocated(Port::try_from(5000).unwrap())
        );
    }

    #[test]
    fn test_allocate_single_preferred_available() {
        let db = create_test_database();
        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5005).unwrap())
        );
    }

    #[test]
    fn test_allocate_single_preferred_occupied() {
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5005).unwrap());

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::PreferredUnavailable {
                port: Port::try_from(5005).unwrap(),
                reason: PortUnavailableReason::Occupied,
            }
        );
    }

    #[test]
    fn test_allocate_single_preferred_excluded() {
        let db = create_test_database();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5005).unwrap());

        let allocator = create_test_allocator(HashSet::new(), exclusions, 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::PreferredUnavailable {
                port: Port::try_from(5005).unwrap(),
                reason: PortUnavailableReason::Excluded,
            }
        );
    }

    #[test]
    fn test_allocate_single_skip_occupied() {
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5002).unwrap())
        );
    }

    #[test]
    fn test_allocate_single_skip_reserved() {
        let mut db = create_test_database();

        // Create reservation for port 5000
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5000).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5001).unwrap())
        );
    }

    #[test]
    fn test_allocate_single_skip_excluded() {
        let db = create_test_database();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5000).unwrap());
        exclusions.add_port(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(HashSet::new(), exclusions, 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5002).unwrap())
        );
    }

    #[test]
    fn test_allocate_single_exhausted() {
        let db = create_test_database();
        let mut occupied = HashSet::new();
        // Mark all ports as occupied
        for port in 5000..=5010 {
            occupied.insert(Port::try_from(port).unwrap());
        }

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Exhausted {
                cleanup_suggested: true,
                tried_cleanup: false
            }
        );
    }

    #[test]
    fn test_find_next_available() {
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(5001).unwrap());

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let config = OccupancyCheckConfig::default();

        let port = allocator
            .find_next_available(Port::try_from(5000).unwrap(), db.connection(), &config)
            .unwrap();

        assert_eq!(port, Some(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_is_port_available() {
        let db = create_test_database();
        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let config = OccupancyCheckConfig::default();

        let availability = allocator
            .is_port_available(Port::try_from(5000).unwrap(), db.connection(), &config)
            .unwrap();

        assert_eq!(availability, PortAvailability::Available);
    }

    #[test]
    fn test_allocation_with_ignore_occupied() {
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5005).unwrap());

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ignore_occupied: true,
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5005).unwrap())
        );
    }

    #[test]
    fn test_ignore_occupied_does_not_ignore_reserved_preferred_port() {
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5005).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);
        let options = AllocationOptions {
            preferred: Some(port),
            ignore_occupied: true,
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::PreferredUnavailable {
                port,
                reason: PortUnavailableReason::Reserved,
            }
        );
    }

    #[test]
    fn test_allocation_with_ignore_exclusions() {
        let db = create_test_database();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5005).unwrap());

        let allocator = create_test_allocator(HashSet::new(), exclusions, 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ignore_exclusions: true,
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5005).unwrap())
        );
    }

    #[test]
    fn test_preferred_ignore_flags_are_independent() {
        let db = create_test_database();
        let preferred = Port::try_from(5005).unwrap();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(preferred);
        let allocator = create_test_allocator(HashSet::from([preferred]), exclusions, 5000, 5010);
        let config = OccupancyCheckConfig::default();

        let ignore_exclusions = allocator
            .allocate_single(
                db.connection(),
                &AllocationOptions {
                    preferred: Some(preferred),
                    ignore_exclusions: true,
                    ..Default::default()
                },
                &config,
            )
            .unwrap();
        assert_eq!(
            ignore_exclusions,
            AllocationResult::PreferredUnavailable {
                port: preferred,
                reason: PortUnavailableReason::Occupied,
            }
        );

        let ignore_occupied = allocator
            .allocate_single(
                db.connection(),
                &AllocationOptions {
                    preferred: Some(preferred),
                    ignore_occupied: true,
                    ..Default::default()
                },
                &config,
            )
            .unwrap();
        assert_eq!(
            ignore_occupied,
            AllocationResult::PreferredUnavailable {
                port: preferred,
                reason: PortUnavailableReason::Excluded,
            }
        );

        let ignore_both = allocator
            .allocate_single(
                db.connection(),
                &AllocationOptions {
                    preferred: Some(preferred),
                    ignore_occupied: true,
                    ignore_exclusions: true,
                },
                &config,
            )
            .unwrap();
        assert_eq!(ignore_both, AllocationResult::Allocated(preferred));
    }

    #[test]
    fn test_replacement_ignores_only_the_current_keys_port() {
        let mut db = create_test_database();
        let current_key =
            ReservationKey::new(PathBuf::from("/test/current"), Some("web".to_string())).unwrap();
        let other_key =
            ReservationKey::new(PathBuf::from("/test/other"), Some("web".to_string())).unwrap();
        let current_port = Port::try_from(5000).unwrap();
        let other_port = Port::try_from(5001).unwrap();
        db.create_reservation(
            &Reservation::builder(current_key.clone(), current_port)
                .build()
                .unwrap(),
        )
        .unwrap();
        db.create_reservation(&Reservation::builder(other_key, other_port).build().unwrap())
            .unwrap();

        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5001);
        let config = OccupancyCheckConfig::default();

        let own_result = allocator
            .allocate_single_replacing(
                db.connection(),
                &AllocationOptions {
                    preferred: Some(current_port),
                    ..Default::default()
                },
                &config,
                &current_key,
            )
            .unwrap();
        assert_eq!(own_result, AllocationResult::Allocated(current_port));

        let other_result = allocator
            .allocate_single_replacing(
                db.connection(),
                &AllocationOptions {
                    preferred: Some(other_port),
                    ..Default::default()
                },
                &config,
                &current_key,
            )
            .unwrap();
        assert_eq!(
            other_result,
            AllocationResult::PreferredUnavailable {
                port: other_port,
                reason: PortUnavailableReason::Reserved,
            }
        );
    }

    #[test]
    fn test_preferred_port_out_of_range() {
        // Test that a preferred port outside the allocator's range is rejected
        // This ensures proper range validation for preferred ports
        let db = create_test_database();
        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(6000).unwrap()), // Outside range
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::PreferredUnavailable {
                port: Port::try_from(6000).unwrap(),
                reason: PortUnavailableReason::Excluded,
            }
        );
    }

    #[test]
    fn test_forward_scan_boundary_conditions() {
        // Test forward scanning behavior at range boundaries
        // Ensures the scanner correctly handles the last available port
        let db = create_test_database();
        let mut occupied = HashSet::new();
        // Occupy all but the last port
        for port in 5000..5010 {
            occupied.insert(Port::try_from(port).unwrap());
        }

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5010).unwrap())
        );
    }

    #[test]
    fn test_allocation_with_all_three_constraints() {
        // Test allocation when a port is reserved, excluded, AND occupied
        // Verifies correct prioritization and interaction of all availability checks
        let mut db = create_test_database();

        // Reserve port 5000
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        // Exclude port 5001
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5001).unwrap());

        // Occupy port 5002
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5002).unwrap());

        let allocator = create_test_allocator(occupied, exclusions, 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        // Should skip 5000 (reserved), 5001 (excluded), 5002 (occupied) and allocate 5003
        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5003).unwrap())
        );
    }

    #[test]
    fn test_preferred_port_reserved() {
        // Test preferred port that's already reserved in database
        // Ensures database reservations are properly checked for preferred ports
        let mut db = create_test_database();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(5005).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();

        let allocator =
            create_test_allocator(HashSet::new(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(port),
            ..Default::default()
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::PreferredUnavailable {
                port,
                reason: PortUnavailableReason::Reserved,
            }
        );
    }

    #[test]
    fn test_find_next_available_from_middle() {
        // Test finding next available port starting from middle of range
        // Verifies forward scanning can start from any point in the range
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5005).unwrap());
        occupied.insert(Port::try_from(5006).unwrap());

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);
        let config = OccupancyCheckConfig::default();

        // Start scan from middle of range
        let port = allocator
            .find_next_available(Port::try_from(5005).unwrap(), db.connection(), &config)
            .unwrap();

        assert_eq!(port, Some(Port::try_from(5007).unwrap()));
    }

    #[test]
    fn test_find_next_available_none_available() {
        // Test find_next_available when no ports are available
        // Ensures proper handling of exhaustion in range scanning
        let db = create_test_database();
        let mut occupied = HashSet::new();
        for port in 5000..=5010 {
            occupied.insert(Port::try_from(port).unwrap());
        }

        let allocator = create_test_allocator(occupied, ExclusionManager::empty(), 5000, 5010);
        let config = OccupancyCheckConfig::default();

        let port = allocator
            .find_next_available(Port::try_from(5000).unwrap(), db.connection(), &config)
            .unwrap();

        assert_eq!(port, None);
    }

    #[test]
    fn test_is_port_available_all_conditions() {
        // Test is_port_available helper with each unavailability condition
        // This is a unit test for the core availability checking logic
        let mut db = create_test_database();

        // Set up: reserve 5000, exclude 5001, occupy 5002
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let reservation = Reservation::builder(key, Port::try_from(5000).unwrap())
            .build()
            .unwrap();
        db.create_reservation(&reservation).unwrap();

        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5001).unwrap());

        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5002).unwrap());

        let allocator = create_test_allocator(occupied, exclusions, 5000, 5010);
        let config = OccupancyCheckConfig::default();

        // Test reserved port
        let availability = allocator
            .is_port_available(Port::try_from(5000).unwrap(), db.connection(), &config)
            .unwrap();
        assert_eq!(availability, PortAvailability::Reserved);

        // Test excluded port
        let availability = allocator
            .is_port_available(Port::try_from(5001).unwrap(), db.connection(), &config)
            .unwrap();
        assert_eq!(availability, PortAvailability::Excluded);

        // Test occupied port
        let availability = allocator
            .is_port_available(Port::try_from(5002).unwrap(), db.connection(), &config)
            .unwrap();
        assert_eq!(availability, PortAvailability::Occupied);

        // Test available port
        let availability = allocator
            .is_port_available(Port::try_from(5003).unwrap(), db.connection(), &config)
            .unwrap();
        assert_eq!(availability, PortAvailability::Available);

        // Test port out of range
        let availability = allocator
            .is_port_available(Port::try_from(6000).unwrap(), db.connection(), &config)
            .unwrap();
        assert_eq!(availability, PortAvailability::Excluded);
    }

    #[test]
    fn test_allocation_deterministic() {
        // Test that allocation is deterministic: same state produces same result
        // This is a key invariant for predictable behavior
        let db = create_test_database();
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(5002).unwrap());

        let allocator =
            create_test_allocator(occupied.clone(), ExclusionManager::empty(), 5000, 5010);

        let options = AllocationOptions::default();
        let config = OccupancyCheckConfig::default();

        // Allocate multiple times
        let result1 = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        let result2 = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        let result3 = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();

        // All should return the same port (5001, the first available)
        assert_eq!(result1, result2);
        assert_eq!(result2, result3);
        assert_eq!(
            result1,
            AllocationResult::Allocated(Port::try_from(5001).unwrap())
        );
    }

    #[test]
    fn test_allocation_range_accessors() {
        // Test that allocator provides correct access to its configuration
        // Verifies getter methods return expected values
        let exclusions = ExclusionManager::empty();
        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(6000).unwrap()).unwrap();
        let allocator =
            PortAllocator::new(MockOccupancyChecker::empty(), exclusions.clone(), range);

        assert_eq!(allocator.range().min().value(), 5000);
        assert_eq!(allocator.range().max().value(), 6000);
    }

    #[test]
    fn test_ignore_flags_combination() {
        // Test that ignore flags can be combined correctly
        // Verifies both ignore_occupied and ignore_exclusions work together
        let db = create_test_database();
        let mut exclusions = ExclusionManager::empty();
        exclusions.add_port(Port::try_from(5005).unwrap());

        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5005).unwrap()); // Same port excluded AND occupied

        let allocator = create_test_allocator(occupied, exclusions, 5000, 5010);

        let options = AllocationOptions {
            preferred: Some(Port::try_from(5005).unwrap()),
            ignore_occupied: true,
            ignore_exclusions: true,
        };
        let config = OccupancyCheckConfig::default();

        let result = allocator
            .allocate_single(db.connection(), &options, &config)
            .unwrap();
        assert_eq!(
            result,
            AllocationResult::Allocated(Port::try_from(5005).unwrap())
        );
    }
}

/// Tests for the glue between the allocator and its verified kernels, at the
/// boundaries the kernels' specifications do not reach (error values, `Port`
/// reconstruction, oracle wiring).
#[cfg(test)]
mod kernel_glue_tests {
    use super::*;
    use crate::config::{Config, ConfigValidator};
    use crate::database::test_util::create_test_database;
    use crate::port::occupancy::MockOccupancyChecker;
    use crate::reservation::{Reservation, ReservationKey};
    use crate::verified::port_bounds::{check_port_bounds, PortBoundsViolation};
    use std::collections::HashSet;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Interesting `u16` values for port-bound grids.
    const EDGES: [u16; 10] = [0, 1, 2, 999, 1000, 5000, 64535, 65000, 65534, 65535];

    fn options() -> impl Iterator<Item = Option<u16>> {
        std::iter::once(None).chain(EDGES.into_iter().map(Some))
    }

    /// `allocator_from_config`'s range resolution before it was delegated to
    /// the verified kernel (reference model).
    fn historical_range(port_config: &PortConfig) -> Result<PortRange> {
        let min = Port::try_from(port_config.min)?;
        let max_value = if let Some(max) = port_config.max {
            max
        } else if let Some(offset) = port_config.max_offset {
            port_config
                .min
                .checked_add(offset)
                .ok_or_else(|| Error::Validation {
                    field: "ports.max_offset".into(),
                    message: format!(
                        "Offset {offset} would overflow when added to min port {}",
                        port_config.min
                    ),
                })?
        } else {
            return Err(Error::Validation {
                field: "ports".into(),
                message: "Either max or max_offset must be specified".into(),
            });
        };
        let max = Port::try_from(max_value)?;
        Ok(PortRange::new(min, max)?)
    }

    fn describe(result: &Result<PortRange>) -> String {
        match result {
            Ok(range) => format!("Ok({range})"),
            Err(error) => format!("Err({error:?})"),
        }
    }

    #[test]
    fn port_range_from_config_matches_historical_resolution() {
        for min in EDGES {
            for max in options() {
                for max_offset in options() {
                    let ports = PortConfig {
                        min,
                        max,
                        max_offset,
                    };
                    assert_eq!(
                        describe(&port_range_from_config(&ports)),
                        describe(&historical_range(&ports)),
                        "{ports:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn port_range_from_config_boundaries() {
        let resolve = |min, max, max_offset| {
            port_range_from_config(&PortConfig {
                min,
                max,
                max_offset,
            })
            .map(|range| (range.min().value(), range.max().value()))
        };
        assert_eq!(resolve(1, Some(65535), None).unwrap(), (1, 65535));
        assert_eq!(resolve(65535, Some(65535), None).unwrap(), (65535, 65535));
        assert_eq!(resolve(65000, None, Some(535)).unwrap(), (65000, 65535));
        assert_eq!(resolve(1, None, Some(65534)).unwrap(), (1, 65535));
        // Lenient shapes kept from `allocator_from_config`: a zero offset, and
        // an explicit max that wins over an offset.
        assert_eq!(resolve(5000, None, Some(0)).unwrap(), (5000, 5000));
        assert_eq!(resolve(5000, Some(6000), Some(1)).unwrap(), (5000, 6000));
        assert!(matches!(
            resolve(65000, None, Some(536)),
            Err(Error::Validation { field, .. }) if field == "ports.max_offset"
        ));
        assert!(matches!(
            resolve(0, Some(10), None),
            Err(Error::InvalidPort { value: 0, .. })
        ));
        assert!(matches!(
            resolve(10, Some(0), None),
            Err(Error::InvalidPort { value: 0, .. })
        ));
        assert!(matches!(
            resolve(10, Some(9), None),
            Err(Error::InvalidPortRange {
                min: 10,
                max: 9,
                ..
            })
        ));
        assert!(matches!(
            resolve(10, None, None),
            Err(Error::Validation { field, .. }) if field == "ports"
        ));
        // The error values are the ones `Port::try_from` reports.
        assert_eq!(
            invalid_port_error(0).to_string(),
            Error::from(Port::try_from(0).unwrap_err()).to_string()
        );
    }

    /// The (field, message) `ConfigValidator::validate_port_config` reports
    /// for each violation.
    fn validator_error(
        min: u16,
        max: Option<u16>,
        violation: PortBoundsViolation,
    ) -> (String, String) {
        match violation {
            PortBoundsViolation::InvalidMin => {
                ("ports.min".into(), format!("Invalid port number: {min}"))
            }
            PortBoundsViolation::InvalidMax => (
                "ports.max".into(),
                format!("Invalid port number: {}", max.unwrap()),
            ),
            PortBoundsViolation::MaxBelowMin => ("ports".into(), "max must be >= min".into()),
            PortBoundsViolation::ZeroOffset => {
                ("ports.max_offset".into(), "max_offset must be > 0".into())
            }
            PortBoundsViolation::OffsetOverflow { offset } => (
                "ports.max_offset".into(),
                format!("Offset {offset} would overflow when added to min port {min}"),
            ),
            PortBoundsViolation::BothMaxAndOffset => (
                "ports".into(),
                "Cannot specify both max and max_offset".into(),
            ),
        }
    }

    #[test]
    fn strict_port_bounds_kernel_agrees_with_config_validator() {
        for min in EDGES {
            for max in options() {
                for max_offset in options() {
                    let ports = PortConfig {
                        min,
                        max,
                        max_offset,
                    };
                    let config = Config {
                        ports: Some(ports.clone()),
                        ..Default::default()
                    };
                    let validated = ConfigValidator::validate(&config, true);
                    match check_port_bounds(min, max, max_offset) {
                        Ok(span) => {
                            assert!(validated.is_ok(), "{ports:?}: {validated:?}");
                            // Every validated shape that names a bound
                            // resolves to the same range.
                            let resolved = port_range_from_config(&ports).ok();
                            assert_eq!(
                                span.map(|span| (span.min(), span.max())),
                                resolved.map(|range| (range.min().value(), range.max().value())),
                                "{ports:?}"
                            );
                        }
                        Err(violation) => match validated {
                            Err(Error::Validation { field, message }) => assert_eq!(
                                (field, message),
                                validator_error(min, max, violation),
                                "{ports:?}"
                            ),
                            other => {
                                panic!("{ports:?}: kernel {violation:?}, validator {other:?}")
                            }
                        },
                    }
                }
            }
        }
    }

    /// An occupancy checker that records every probed port, fails at
    /// `failing`, and reports `occupied` ports as occupied.
    #[derive(Debug, Default)]
    struct RecordingChecker {
        probed: Mutex<Vec<u16>>,
        occupied: HashSet<u16>,
        failing: HashSet<u16>,
    }

    impl RecordingChecker {
        fn probed(&self) -> Vec<u16> {
            self.probed.lock().unwrap().clone()
        }
    }

    impl PortOccupancyChecker for RecordingChecker {
        fn is_occupied(&self, port: Port, _config: &OccupancyCheckConfig) -> Result<bool> {
            self.probed.lock().unwrap().push(port.value());
            if self.failing.contains(&port.value()) {
                return Err(Error::OccupancyCheckFailed {
                    port,
                    source: Box::new(io::Error::other("synthetic probe failure")),
                });
            }
            Ok(self.occupied.contains(&port.value()))
        }
    }

    fn port(value: u16) -> Port {
        Port::try_from(value).unwrap()
    }

    fn recording_allocator(
        min: u16,
        max: u16,
        occupied: &[u16],
        failing: &[u16],
        excluded: &[u16],
    ) -> PortAllocator<RecordingChecker> {
        let checker = RecordingChecker {
            probed: Mutex::default(),
            occupied: occupied.iter().copied().collect(),
            failing: failing.iter().copied().collect(),
        };
        let mut exclusions = ExclusionManager::empty();
        for &value in excluded {
            exclusions.add_port(port(value));
        }
        PortAllocator::new(
            checker,
            exclusions,
            PortRange::new(port(min), port(max)).unwrap(),
        )
    }

    fn reserve(db: &mut Database, path: &str, value: u16) {
        let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
        db.create_reservation(&Reservation::builder(key, port(value)).build().unwrap())
            .unwrap();
    }

    #[test]
    fn scan_probes_each_candidate_once_in_order_and_skips_blocked_ports() {
        let mut db = create_test_database();
        reserve(&mut db, "/test/reserved", 5001);
        let allocator = recording_allocator(5000, 5005, &[5000, 5003], &[], &[5002]);
        let result = allocator
            .allocate_single(
                db.connection(),
                &AllocationOptions::default(),
                &OccupancyCheckConfig::default(),
            )
            .unwrap();
        assert_eq!(result, AllocationResult::Allocated(port(5004)));
        // 5001 (reserved) and 5002 (excluded) are never probed.
        assert_eq!(allocator.checker.probed(), vec![5000, 5003, 5004]);
    }

    #[test]
    fn probe_failures_on_blocked_ports_do_not_abort_allocation() {
        let mut db = create_test_database();
        reserve(&mut db, "/test/reserved", 5000);
        // The probe would fail at 5000 (reserved) and 5001 (excluded).
        let allocator = recording_allocator(5000, 5002, &[], &[5000, 5001], &[5001]);
        let config = OccupancyCheckConfig::default();
        assert_eq!(
            allocator
                .allocate_single(db.connection(), &AllocationOptions::default(), &config)
                .unwrap(),
            AllocationResult::Allocated(port(5002))
        );
        for (preferred, reason) in [
            (5000, PortUnavailableReason::Reserved),
            (5001, PortUnavailableReason::Excluded),
        ] {
            let options = AllocationOptions {
                preferred: Some(port(preferred)),
                ..Default::default()
            };
            assert_eq!(
                allocator
                    .allocate_single(db.connection(), &options, &config)
                    .unwrap(),
                AllocationResult::PreferredUnavailable {
                    port: port(preferred),
                    reason,
                }
            );
        }
        let blockers = allocator
            .exhaustion_blockers(db.connection(), &config, None)
            .unwrap();
        assert!(blockers.reserved() && blockers.excluded() && !blockers.occupied());
        assert_eq!(allocator.checker.probed(), vec![5002, 5002]);
    }

    #[test]
    fn ignored_probe_is_never_run_and_ignored_exclusion_still_probes() {
        let db = create_test_database();
        let allocator = recording_allocator(5000, 5002, &[], &[5001], &[5001]);
        let config = OccupancyCheckConfig::default();
        let forced = AllocationOptions {
            preferred: Some(port(5001)),
            ignore_occupied: true,
            ignore_exclusions: true,
        };
        assert_eq!(
            allocator
                .allocate_single(db.connection(), &forced, &config)
                .unwrap(),
            AllocationResult::Allocated(port(5001))
        );
        assert_eq!(allocator.checker.probed(), Vec::<u16>::new());

        let ignore_exclusions_only = AllocationOptions {
            preferred: Some(port(5001)),
            ignore_exclusions: true,
            ..Default::default()
        };
        let error = allocator
            .allocate_single(db.connection(), &ignore_exclusions_only, &config)
            .unwrap_err();
        assert!(matches!(error, Error::OccupancyCheckFailed { .. }));
        assert_eq!(allocator.checker.probed(), vec![5001]);
    }

    #[test]
    fn scan_errors_stop_at_the_first_failing_candidate() {
        let db = create_test_database();
        let allocator = recording_allocator(5000, 5004, &[5000], &[5001, 5003], &[]);
        let config = OccupancyCheckConfig::default();
        let error = allocator
            .allocate_single(db.connection(), &AllocationOptions::default(), &config)
            .unwrap_err();
        assert!(matches!(error, Error::OccupancyCheckFailed { port: p, .. } if p == port(5001)));
        assert_eq!(allocator.checker.probed(), vec![5000, 5001]);

        let error = allocator
            .exhaustion_blockers(db.connection(), &config, None)
            .unwrap_err();
        assert!(matches!(error, Error::OccupancyCheckFailed { port: p, .. } if p == port(5001)));
    }

    #[test]
    fn scans_reach_the_top_of_the_port_domain() {
        let db = create_test_database();
        let config = OccupancyCheckConfig::default();
        let allocator = recording_allocator(65533, 65535, &[65533, 65534], &[], &[]);
        assert_eq!(
            allocator
                .allocate_single(db.connection(), &AllocationOptions::default(), &config)
                .unwrap(),
            AllocationResult::Allocated(port(65535))
        );
        assert_eq!(
            allocator
                .find_next_allocatable(port(65535), &config)
                .unwrap(),
            Some(port(65535))
        );

        let full = recording_allocator(65534, 65535, &[65534, 65535], &[], &[]);
        assert_eq!(
            full.allocate_single(db.connection(), &AllocationOptions::default(), &config)
                .unwrap(),
            AllocationResult::Exhausted {
                cleanup_suggested: true,
                tried_cleanup: false,
            }
        );
        assert_eq!(
            full.find_next_allocatable(port(65534), &config).unwrap(),
            None
        );
        let blockers = full
            .exhaustion_blockers(db.connection(), &config, None)
            .unwrap();
        assert!(!blockers.reserved() && !blockers.excluded() && blockers.occupied());

        let single = recording_allocator(1, 1, &[], &[], &[]);
        assert_eq!(
            single
                .allocate_single(db.connection(), &AllocationOptions::default(), &config)
                .unwrap(),
            AllocationResult::Allocated(port(1))
        );
    }

    #[test]
    fn scan_starts_are_handled_as_before() {
        let db = create_test_database();
        let config = OccupancyCheckConfig::default();
        let allocator = recording_allocator(5000, 5010, &[], &[], &[]);
        // Below the range: the ports below are skipped without probing.
        assert_eq!(
            allocator
                .find_next_available(port(1), db.connection(), &config)
                .unwrap(),
            Some(port(5000))
        );
        assert_eq!(
            allocator
                .find_next_allocatable(port(4000), &config)
                .unwrap(),
            Some(port(5000))
        );
        assert_eq!(allocator.checker.probed(), vec![5000, 5000]);
        // At the top of the range, and above it (an invalid range, as before).
        assert_eq!(
            allocator
                .find_next_available(port(5010), db.connection(), &config)
                .unwrap(),
            Some(port(5010))
        );
        assert!(matches!(
            allocator.find_next_available(port(5011), db.connection(), &config),
            Err(Error::InvalidPortRange {
                min: 5011,
                max: 5010,
                ..
            })
        ));
        assert!(matches!(
            allocator.find_next_allocatable(port(6000), &config),
            Err(Error::InvalidPortRange {
                min: 6000,
                max: 5010,
                ..
            })
        ));
    }

    #[test]
    fn find_next_allocatable_ignores_reservations() {
        let mut db = create_test_database();
        reserve(&mut db, "/test/reserved", 5000);
        let allocator = recording_allocator(5000, 5002, &[], &[], &[]);
        assert_eq!(
            allocator
                .find_next_allocatable(port(5000), &OccupancyCheckConfig::default())
                .unwrap(),
            Some(port(5000))
        );
    }

    #[test]
    fn preferred_availability_uses_the_full_port_domain() {
        let db = create_test_database();
        let allocator = PortAllocator::new(
            MockOccupancyChecker::empty(),
            ExclusionManager::empty(),
            PortRange::new(port(5000), port(5010)).unwrap(),
        );
        let config = OccupancyCheckConfig::default();
        assert_eq!(
            allocator
                .is_preferred_port_available(port(65535), db.connection(), &config)
                .unwrap(),
            PortAvailability::Available
        );
        assert_eq!(
            allocator
                .is_port_available(port(65535), db.connection(), &config)
                .unwrap(),
            PortAvailability::Excluded
        );
    }

    #[test]
    fn unavailable_reason_is_none_exactly_for_available() {
        assert_eq!(PortAvailability::Available.unavailable_reason(), None);
        assert_eq!(
            PortAvailability::Reserved.unavailable_reason(),
            Some(PortUnavailableReason::Reserved)
        );
        assert_eq!(
            PortAvailability::Excluded.unavailable_reason(),
            Some(PortUnavailableReason::Excluded)
        );
        assert_eq!(
            PortAvailability::Occupied.unavailable_reason(),
            Some(PortUnavailableReason::Occupied)
        );
    }
}
