//! Port occupancy checking for system-level port availability.
//!
//! This module provides trait-based occupancy checking to determine if ports
//! are actually in use on the system. The design uses traits for testability,
//! allowing both real system checks and mock implementations for testing.

use std::collections::HashSet;

use crate::config::OccupancyConfig;
use crate::{Port, PortRange, Result};

/// Configuration for a single occupancy check.
///
/// This is derived from `OccupancyConfig` but represents the actual
/// parameters for a specific check operation.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OccupancyCheckConfig {
    /// Skip TCP checks.
    pub skip_tcp: bool,
    /// Skip UDP checks.
    pub skip_udp: bool,
    /// Skip IPv4 checks.
    pub skip_ipv4: bool,
    /// Skip IPv6 checks.
    pub skip_ipv6: bool,
    /// Check all network interfaces (not just localhost).
    pub check_all_interfaces: bool,
}

impl From<&OccupancyConfig> for OccupancyCheckConfig {
    fn from(config: &OccupancyConfig) -> Self {
        // Note: Field name divergence between `skip_ip4`/`skip_ip6` (in OccupancyConfig)
        // and `skip_ipv4`/`skip_ipv6` (in OccupancyCheckConfig) is intentional.
        // The config uses abbreviated names for brevity, while the runtime struct uses
        // full names for clarity.
        Self {
            skip_tcp: config.skip_tcp.unwrap_or(false),
            skip_udp: config.skip_udp.unwrap_or(false),
            skip_ipv4: config.skip_ip4.unwrap_or(false),
            skip_ipv6: config.skip_ip6.unwrap_or(false),
            check_all_interfaces: config.check_all_interfaces.unwrap_or(false),
        }
    }
}

/// Trait for checking port occupancy on the system.
///
/// This trait abstracts port occupancy checking to enable both real system
/// checks and mock implementations for testing.
///
/// # Examples
///
/// ```
/// use trop::port::occupancy::{PortOccupancyChecker, SystemOccupancyChecker, OccupancyCheckConfig};
/// use trop::Port;
///
/// let checker = SystemOccupancyChecker;
/// let config = OccupancyCheckConfig::default();
/// let port = Port::try_from(8080).unwrap();
///
/// // Check if port 8080 is occupied
/// match checker.is_occupied(port, &config) {
///     Ok(occupied) => println!("Port 8080 occupied: {}", occupied),
///     Err(e) => eprintln!("Check failed: {}", e),
/// }
/// ```
pub trait PortOccupancyChecker: Send + Sync {
    /// Check if a specific port is occupied.
    ///
    /// Returns `Ok(true)` if the port is occupied, `Ok(false)` if available.
    /// Returns `Err` if the check itself fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the occupancy check fails due to system issues
    /// or permission problems.
    fn is_occupied(&self, port: Port, config: &OccupancyCheckConfig) -> Result<bool>;

    /// Find all occupied ports in a given range.
    ///
    /// This is an optimization opportunity - implementations may batch checks
    /// for efficiency. The default implementation calls `is_occupied` for each port.
    ///
    /// # Errors
    ///
    /// Returns an error if any occupancy check fails.
    fn find_occupied_ports(
        &self,
        range: &PortRange,
        config: &OccupancyCheckConfig,
    ) -> Result<Vec<Port>> {
        let mut occupied = Vec::new();
        for port in *range {
            if self.is_occupied(port, config)? {
                occupied.push(port);
            }
        }
        Ok(occupied)
    }
}

/// Production implementation using the port-selector crate.
///
/// This checker uses actual system calls to determine port availability.
///
/// # Examples
///
/// ```
/// use trop::port::occupancy::{PortOccupancyChecker, SystemOccupancyChecker, OccupancyCheckConfig};
/// use trop::Port;
///
/// let checker = SystemOccupancyChecker;
/// let config = OccupancyCheckConfig::default();
/// let port = Port::try_from(80).unwrap();
///
/// // Check if port 80 is occupied
/// let occupied = checker.is_occupied(port, &config).unwrap();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct SystemOccupancyChecker;

impl PortOccupancyChecker for SystemOccupancyChecker {
    fn is_occupied(&self, port: Port, config: &OccupancyCheckConfig) -> Result<bool> {
        // If we're skipping all checks, the port is available
        if config.skip_tcp && config.skip_udp {
            return Ok(false);
        }
        if config.skip_ipv4 && config.skip_ipv6 {
            return Ok(false);
        }

        // Use port-selector to check availability
        // The port-selector crate's is_free checks if we can bind to the port
        let port_u16 = port.value();

        // By default, port-selector checks localhost binding
        // If the port is free, we can bind to it, so it's NOT occupied
        // Note: `check_all_interfaces` is currently reserved for future use when we implement
        // interface-specific binding checks. For now, we always check localhost.
        let is_free = port_selector::is_free(port_u16);

        // Port is occupied if it's NOT free
        Ok(!is_free)
    }
}

/// Mock implementation for testing with configurable occupied ports.
///
/// This checker allows tests to specify exactly which ports should be
/// considered occupied, enabling deterministic testing.
///
/// # Examples
///
/// ```
/// use trop::port::occupancy::{PortOccupancyChecker, MockOccupancyChecker, OccupancyCheckConfig};
/// use trop::Port;
/// use std::collections::HashSet;
///
/// let mut occupied = HashSet::new();
/// occupied.insert(Port::try_from(8080).unwrap());
/// occupied.insert(Port::try_from(8081).unwrap());
///
/// let checker = MockOccupancyChecker::new(occupied);
/// let config = OccupancyCheckConfig::default();
///
/// assert!(checker.is_occupied(Port::try_from(8080).unwrap(), &config).unwrap());
/// assert!(!checker.is_occupied(Port::try_from(8082).unwrap(), &config).unwrap());
/// ```
#[derive(Debug, Clone)]
pub struct MockOccupancyChecker {
    occupied_ports: HashSet<Port>,
}

impl MockOccupancyChecker {
    /// Create a new mock checker with the specified occupied ports.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::occupancy::MockOccupancyChecker;
    /// use trop::Port;
    /// use std::collections::HashSet;
    ///
    /// let mut occupied = HashSet::new();
    /// occupied.insert(Port::try_from(8080).unwrap());
    ///
    /// let checker = MockOccupancyChecker::new(occupied);
    /// ```
    #[must_use]
    pub fn new(occupied_ports: HashSet<Port>) -> Self {
        Self { occupied_ports }
    }

    /// Create an empty mock checker (all ports available).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::occupancy::MockOccupancyChecker;
    ///
    /// let checker = MockOccupancyChecker::empty();
    /// ```
    #[must_use]
    pub fn empty() -> Self {
        Self {
            occupied_ports: HashSet::new(),
        }
    }

    /// Add a port to the occupied set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::occupancy::MockOccupancyChecker;
    /// use trop::Port;
    ///
    /// let mut checker = MockOccupancyChecker::empty();
    /// checker.mark_occupied(Port::try_from(8080).unwrap());
    /// ```
    pub fn mark_occupied(&mut self, port: Port) {
        self.occupied_ports.insert(port);
    }

    /// Remove a port from the occupied set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::occupancy::MockOccupancyChecker;
    /// use trop::Port;
    ///
    /// let mut checker = MockOccupancyChecker::empty();
    /// let port = Port::try_from(8080).unwrap();
    /// checker.mark_occupied(port);
    /// checker.mark_free(port);
    /// ```
    pub fn mark_free(&mut self, port: Port) {
        self.occupied_ports.remove(&port);
    }

    /// Get the set of occupied ports.
    #[must_use]
    pub fn occupied_ports(&self) -> &HashSet<Port> {
        &self.occupied_ports
    }
}

impl PortOccupancyChecker for MockOccupancyChecker {
    fn is_occupied(&self, port: Port, _config: &OccupancyCheckConfig) -> Result<bool> {
        Ok(self.occupied_ports.contains(&port))
    }

    fn find_occupied_ports(
        &self,
        range: &PortRange,
        _config: &OccupancyCheckConfig,
    ) -> Result<Vec<Port>> {
        let mut occupied = Vec::new();
        for port in *range {
            if self.occupied_ports.contains(&port) {
                occupied.push(port);
            }
        }
        Ok(occupied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_occupancy_check_config_default() {
        let config = OccupancyCheckConfig::default();
        assert!(!config.skip_tcp);
        assert!(!config.skip_udp);
        assert!(!config.skip_ipv4);
        assert!(!config.skip_ipv6);
        assert!(!config.check_all_interfaces);
    }

    #[test]
    fn test_occupancy_check_config_from_occupancy_config() {
        let occ_config = OccupancyConfig {
            skip_tcp: Some(true),
            skip_ip4: Some(true),
            check_all_interfaces: Some(true),
            ..Default::default()
        };

        let config = OccupancyCheckConfig::from(&occ_config);
        assert!(config.skip_tcp);
        assert!(config.skip_ipv4);
        assert!(config.check_all_interfaces);
        assert!(!config.skip_udp);
        assert!(!config.skip_ipv6);
    }

    #[test]
    fn test_mock_checker_empty() {
        let checker = MockOccupancyChecker::empty();
        let config = OccupancyCheckConfig::default();
        let port = Port::try_from(8080).unwrap();

        assert!(!checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_mock_checker_with_occupied_ports() {
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(8080).unwrap());
        occupied.insert(Port::try_from(8081).unwrap());

        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();

        assert!(checker
            .is_occupied(Port::try_from(8080).unwrap(), &config)
            .unwrap());
        assert!(checker
            .is_occupied(Port::try_from(8081).unwrap(), &config)
            .unwrap());
        assert!(!checker
            .is_occupied(Port::try_from(8082).unwrap(), &config)
            .unwrap());
    }

    #[test]
    fn test_mock_checker_mark_occupied() {
        let mut checker = MockOccupancyChecker::empty();
        let config = OccupancyCheckConfig::default();
        let port = Port::try_from(8080).unwrap();

        assert!(!checker.is_occupied(port, &config).unwrap());

        checker.mark_occupied(port);
        assert!(checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_mock_checker_mark_free() {
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(8080).unwrap());

        let mut checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();
        let port = Port::try_from(8080).unwrap();

        assert!(checker.is_occupied(port, &config).unwrap());

        checker.mark_free(port);
        assert!(!checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_mock_checker_find_occupied_ports() {
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5001).unwrap());
        occupied.insert(Port::try_from(5005).unwrap());
        occupied.insert(Port::try_from(5009).unwrap());

        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let occupied_in_range = checker.find_occupied_ports(&range, &config).unwrap();

        assert_eq!(occupied_in_range.len(), 3);
        assert!(occupied_in_range.contains(&Port::try_from(5001).unwrap()));
        assert!(occupied_in_range.contains(&Port::try_from(5005).unwrap()));
        assert!(occupied_in_range.contains(&Port::try_from(5009).unwrap()));
    }

    #[test]
    fn test_system_checker_skip_all_tcp_udp() {
        let checker = SystemOccupancyChecker;
        let config = OccupancyCheckConfig {
            skip_tcp: true,
            skip_udp: true,
            ..Default::default()
        };
        let port = Port::try_from(8080).unwrap();

        // Should return false (available) when all checks are skipped
        assert!(!checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_system_checker_skip_all_ip_versions() {
        let checker = SystemOccupancyChecker;
        let config = OccupancyCheckConfig {
            skip_ipv4: true,
            skip_ipv6: true,
            ..Default::default()
        };
        let port = Port::try_from(8080).unwrap();

        // Should return false (available) when all IP versions are skipped
        assert!(!checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_occupancy_check_config_from_occupancy_config_with_all_fields() {
        // Test that all fields are correctly converted from OccupancyConfig
        // This verifies the mapping between abbreviated config names and full runtime names
        let occ_config = OccupancyConfig {
            skip: None,
            skip_tcp: Some(true),
            skip_udp: Some(true),
            skip_ip4: Some(true),
            skip_ip6: Some(true),
            check_all_interfaces: Some(true),
        };

        let config = OccupancyCheckConfig::from(&occ_config);
        assert!(config.skip_tcp);
        assert!(config.skip_udp);
        assert!(config.skip_ipv4);
        assert!(config.skip_ipv6);
        assert!(config.check_all_interfaces);
    }

    #[test]
    fn test_occupancy_check_config_partial_none_values() {
        // Test that None values default to false
        // This ensures proper handling of optional configuration fields
        let occ_config = OccupancyConfig {
            skip: None,
            skip_tcp: None,
            skip_udp: Some(true),
            skip_ip4: None,
            skip_ip6: None,
            check_all_interfaces: None,
        };

        let config = OccupancyCheckConfig::from(&occ_config);
        assert!(!config.skip_tcp);
        assert!(config.skip_udp);
        assert!(!config.skip_ipv4);
        assert!(!config.skip_ipv6);
        assert!(!config.check_all_interfaces);
    }

    #[test]
    fn test_mock_checker_multiple_mark_operations() {
        // Test that marking the same port multiple times is idempotent
        // This verifies the HashSet semantics work correctly
        let mut checker = MockOccupancyChecker::empty();
        let port = Port::try_from(8080).unwrap();
        let config = OccupancyCheckConfig::default();

        checker.mark_occupied(port);
        checker.mark_occupied(port); // Mark again
        assert!(checker.is_occupied(port, &config).unwrap());

        // Verify only one entry exists
        assert_eq!(checker.occupied_ports().len(), 1);
    }

    #[test]
    fn test_mock_checker_mark_free_nonexistent() {
        // Test that marking a non-occupied port as free is safe (no-op)
        // This verifies fail-safe behavior for cleanup operations
        let mut checker = MockOccupancyChecker::empty();
        let port = Port::try_from(8080).unwrap();
        let config = OccupancyCheckConfig::default();

        checker.mark_free(port); // Free a port that was never occupied
        assert!(!checker.is_occupied(port, &config).unwrap());
    }

    #[test]
    fn test_mock_checker_boundary_ports() {
        // Test occupancy checking at port number boundaries
        // Ensures correct behavior at minimum and maximum valid port values
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(1).unwrap()); // Minimum valid port
        occupied.insert(Port::try_from(65535).unwrap()); // Maximum valid port

        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();

        assert!(checker
            .is_occupied(Port::try_from(1).unwrap(), &config)
            .unwrap());
        assert!(checker
            .is_occupied(Port::try_from(65535).unwrap(), &config)
            .unwrap());
        assert!(!checker
            .is_occupied(Port::try_from(1000).unwrap(), &config)
            .unwrap());
    }

    #[test]
    fn test_mock_checker_find_occupied_ports_empty_range() {
        // Test finding occupied ports when none exist in range
        // Verifies correct handling of empty result sets
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(5000).unwrap());
        occupied.insert(Port::try_from(6000).unwrap());

        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();

        // Search in a range that contains no occupied ports
        let range =
            PortRange::new(Port::try_from(5500).unwrap(), Port::try_from(5999).unwrap()).unwrap();

        let occupied_in_range = checker.find_occupied_ports(&range, &config).unwrap();
        assert!(occupied_in_range.is_empty());
    }

    #[test]
    fn test_mock_checker_find_occupied_ports_all_occupied() {
        // Test finding occupied ports when all ports in range are occupied
        // Verifies correct handling of fully occupied ranges
        let mut occupied = HashSet::new();
        for port in 5000..=5010 {
            occupied.insert(Port::try_from(port).unwrap());
        }

        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig::default();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let occupied_in_range = checker.find_occupied_ports(&range, &config).unwrap();
        assert_eq!(occupied_in_range.len(), 11); // 5000-5010 inclusive
    }

    #[test]
    fn test_mock_checker_occupied_ports_accessor() {
        // Test that the occupied_ports() accessor returns correct state
        // This verifies the getter provides accurate information for test assertions
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(8080).unwrap());
        occupied.insert(Port::try_from(8081).unwrap());

        let checker = MockOccupancyChecker::new(occupied.clone());

        let ports = checker.occupied_ports();
        assert_eq!(ports.len(), 2);
        assert!(ports.contains(&Port::try_from(8080).unwrap()));
        assert!(ports.contains(&Port::try_from(8081).unwrap()));
    }

    #[test]
    fn test_system_checker_partial_skip_combinations() {
        // Test various partial skip configurations to ensure correct logic
        // Verifies that only when ALL checks are skipped do we report available
        let checker = SystemOccupancyChecker;
        let port = Port::try_from(8080).unwrap();

        // Skip TCP but not UDP - should still check
        let config1 = OccupancyCheckConfig {
            skip_tcp: true,
            skip_udp: false,
            ..Default::default()
        };
        // This will actually check the port (system-dependent result)
        let result1 = checker.is_occupied(port, &config1);
        assert!(result1.is_ok()); // Should not error

        // Skip IPv4 but not IPv6 - should still check
        let config2 = OccupancyCheckConfig {
            skip_ipv4: true,
            skip_ipv6: false,
            ..Default::default()
        };
        let result2 = checker.is_occupied(port, &config2);
        assert!(result2.is_ok());
    }
}
