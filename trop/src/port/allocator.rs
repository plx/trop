//! Port allocation with occupancy checking and exclusion support.
//!
//! This module implements the core port allocation algorithm, which finds
//! available ports while respecting database reservations, exclusions, and
//! system occupancy.

use rusqlite::Connection;

use crate::database::Database;
use crate::error::{Error, PortUnavailableReason};
use crate::{Port, PortRange, ReservationKey, Result};

use super::exclusions::ExclusionManager;
use super::occupancy::{OccupancyCheckConfig, PortOccupancyChecker, SystemOccupancyChecker};

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

    fn allocate_single_ignoring(
        &self,
        conn: &Connection,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<AllocationResult> {
        // If a preferred port is specified, try it first
        if let Some(preferred) = options.preferred {
            // Check if the preferred port is in range
            if !self.range.contains(preferred) {
                return Ok(AllocationResult::PreferredUnavailable {
                    port: preferred,
                    reason: PortUnavailableReason::Excluded,
                });
            }

            // Reservation ownership is absolute, while the exclusion and
            // occupancy guards are independently bypassable. Checking them
            // separately prevents one narrow ignore flag from accidentally
            // hiding the other reason a preferred port is unavailable.
            if Self::is_reserved_by_other(conn, preferred, ignored_key)? {
                return Ok(AllocationResult::PreferredUnavailable {
                    port: preferred,
                    reason: PortUnavailableReason::Reserved,
                });
            }
            if self.exclusions.is_excluded(preferred) && !options.ignore_exclusions {
                return Ok(AllocationResult::PreferredUnavailable {
                    port: preferred,
                    reason: PortUnavailableReason::Excluded,
                });
            }
            if !options.ignore_occupied && self.checker.is_occupied(preferred, occupancy_config)? {
                return Ok(AllocationResult::PreferredUnavailable {
                    port: preferred,
                    reason: PortUnavailableReason::Occupied,
                });
            }
            return Ok(AllocationResult::Allocated(preferred));
        }

        // Forward scan from minimum
        if let Some(port) = self.find_next_available_ignoring(
            self.range.min(),
            conn,
            occupancy_config,
            ignored_key,
        )? {
            Ok(AllocationResult::Allocated(port))
        } else {
            // No ports available - suggest cleanup might help
            Ok(AllocationResult::Exhausted {
                cleanup_suggested: true,
                tried_cleanup: false,
            })
        }
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
        // Scan from start to range max
        let scan_range = PortRange::new(start, self.range.max())?;

        for port in scan_range {
            if let PortAvailability::Available =
                self.port_availability(port, conn, occupancy_config, true, ignored_key)?
            {
                return Ok(Some(port));
            }
        }

        Ok(None)
    }

    /// Find the next available port that can be atomically allocated.
    ///
    /// This method performs allocation without checking occupancy or reservations
    /// in advance. Instead, it relies on the atomic insertion with UNIQUE constraint
    /// to detect conflicts. This is suitable for use within a reservation transaction.
    ///
    /// The method scans forward from `start` checking only exclusions and occupancy,
    /// then returns the first candidate port. The caller should attempt to insert
    /// this port atomically and retry if it fails due to UNIQUE constraint.
    ///
    /// # Errors
    ///
    /// Returns an error if occupancy checks fail.
    pub fn find_next_allocatable(
        &self,
        start: Port,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<Option<Port>> {
        // Scan from start to range max
        let scan_range = PortRange::new(start, self.range.max())?;

        for port in scan_range {
            // Check if in range (already guaranteed by scan_range, but explicit)
            if !self.range.contains(port) {
                continue;
            }

            // Check if excluded
            if self.exclusions.is_excluded(port) {
                continue;
            }

            // Check if occupied on system. Probe failures are returned with
            // their diagnostic instead of being erased; this remains
            // fail-closed because no port is allocated after an uncertain
            // result.
            let occupied = self.checker.is_occupied(port, occupancy_config)?;

            if !occupied {
                return Ok(Some(port));
            }
        }

        Ok(None)
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
        if require_in_range && !self.range.contains(port) {
            return Ok(PortAvailability::Excluded);
        }

        // Check if reserved in database
        if Self::is_reserved_by_other(conn, port, ignored_key)? {
            return Ok(PortAvailability::Reserved);
        }

        // Check if excluded
        if self.exclusions.is_excluded(port) {
            return Ok(PortAvailability::Excluded);
        }

        // Check if occupied on system. Returning a probe failure is
        // fail-closed (the port cannot become Available) and preserves the
        // actionable protocol/family/scope/address diagnostic.
        let occupied = self.checker.is_occupied(port, occupancy_config)?;

        if occupied {
            Ok(PortAvailability::Occupied)
        } else {
            Ok(PortAvailability::Available)
        }
    }

    fn is_reserved_by_other(
        conn: &Connection,
        port: Port,
        ignored_key: Option<&ReservationKey>,
    ) -> Result<bool> {
        Ok(Database::get_reservation_by_port(conn, port)?
            .is_some_and(|reservation| ignored_key != Some(reservation.key())))
    }
}

/// Internal enum for tracking why a port is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PortAvailability {
    /// Port is available for allocation.
    Available,
    /// Port is already reserved in the database.
    Reserved,
    /// Port is in the exclusion list.
    Excluded,
    /// Port is occupied on the system.
    Occupied,
}

impl PortAvailability {
    /// Return the public reason for an unavailable port.
    pub(super) const fn unavailable_reason(self) -> Option<PortUnavailableReason> {
        match self {
            Self::Available => None,
            Self::Reserved => Some(PortUnavailableReason::Reserved),
            Self::Excluded => Some(PortUnavailableReason::Excluded),
            Self::Occupied => Some(PortUnavailableReason::Occupied),
        }
    }
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
    let range = PortRange::new(min, max)?;

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
