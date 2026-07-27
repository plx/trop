//! Port occupancy checking for system-level port availability.
//!
//! This module provides trait-based occupancy checking to determine if ports
//! are actually in use on the system. The design uses traits for testability,
//! allowing both real system checks and mock implementations for testing.
//!
//! The production checker builds an explicit matrix from the enabled
//! transports (TCP and UDP), address families (IPv4 and IPv6), and address
//! scopes. Localhost addresses are always checked; wildcard addresses are
//! added when `check_all_interfaces` is enabled. IPv6 probe sockets set
//! `IPV6_V6ONLY` before binding so operating-system dual-stack defaults cannot
//! make an IPv6 probe accidentally test IPv4 too.

use std::collections::HashSet;
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use socket2::{Domain, Protocol, Socket, Type};

use crate::config::OccupancyConfig;
use crate::{Error, Port, PortRange, Result};

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
        let skip_all = config.skip.unwrap_or(false);
        // Note: Field name divergence between `skip_ip4`/`skip_ip6` (in OccupancyConfig)
        // and `skip_ipv4`/`skip_ipv6` (in OccupancyCheckConfig) is intentional.
        // The config uses abbreviated names for brevity, while the runtime struct uses
        // full names for clarity.
        Self {
            skip_tcp: skip_all || config.skip_tcp.unwrap_or(false),
            skip_udp: skip_all || config.skip_udp.unwrap_or(false),
            skip_ipv4: skip_all || config.skip_ip4.unwrap_or(false),
            skip_ipv6: skip_all || config.skip_ip6.unwrap_or(false),
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

/// Production implementation using explicit socket bind probes.
///
/// Each enabled protocol/address-family pair is checked on localhost. When
/// `check_all_interfaces` is enabled, a wildcard-address probe is added for
/// every enabled pair. Successful binds are immediately closed. An
/// `AddrInUse` result means occupied; any other system error is returned with
/// probe context so callers can fail closed without losing the diagnostic.
///
/// # Examples
///
/// ```no_run
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Tcp,
    Udp,
}

impl Transport {
    fn socket_type(self) -> Type {
        match self {
            Self::Tcp => Type::STREAM,
            Self::Udp => Type::DGRAM,
        }
    }

    fn protocol(self) -> Protocol {
        match self {
            Self::Tcp => Protocol::TCP,
            Self::Udp => Protocol::UDP,
        }
    }
}

impl fmt::Display for Transport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OccupancyProbe {
    transport: Transport,
    address: SocketAddr,
}

impl OccupancyProbe {
    fn domain(self) -> Domain {
        if self.address.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        }
    }

    fn family_name(self) -> &'static str {
        if self.address.is_ipv4() {
            "IPv4"
        } else {
            "IPv6"
        }
    }

    fn scope_name(self) -> &'static str {
        if self.address.ip().is_unspecified() {
            "wildcard"
        } else {
            "localhost"
        }
    }
}

impl fmt::Display for OccupancyProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} {} bind at {}",
            self.transport,
            self.family_name(),
            self.scope_name(),
            self.address
        )
    }
}

#[derive(Debug)]
struct ProbeFailure {
    probe: OccupancyProbe,
    source: io::Error,
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed: {}", self.probe, self.source)
    }
}

impl std::error::Error for ProbeFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl SystemOccupancyChecker {
    fn probes(port: Port, config: &OccupancyCheckConfig) -> Vec<OccupancyProbe> {
        let mut probes = Vec::with_capacity(if config.check_all_interfaces { 8 } else { 4 });

        if !config.skip_ipv4 {
            Self::add_family_probes(
                &mut probes,
                config,
                SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port.value()),
                SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port.value()),
            );
        }
        if !config.skip_ipv6 {
            Self::add_family_probes(
                &mut probes,
                config,
                SocketAddr::new(Ipv6Addr::LOCALHOST.into(), port.value()),
                SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port.value()),
            );
        }

        probes
    }

    fn add_family_probes(
        probes: &mut Vec<OccupancyProbe>,
        config: &OccupancyCheckConfig,
        localhost: SocketAddr,
        wildcard: SocketAddr,
    ) {
        for address in [localhost]
            .into_iter()
            .chain(config.check_all_interfaces.then_some(wildcard))
        {
            if !config.skip_tcp {
                probes.push(OccupancyProbe {
                    transport: Transport::Tcp,
                    address,
                });
            }
            if !config.skip_udp {
                probes.push(OccupancyProbe {
                    transport: Transport::Udp,
                    address,
                });
            }
        }
    }

    fn probe_socket(probe: &OccupancyProbe) -> io::Result<()> {
        let socket = Socket::new(
            probe.domain(),
            probe.transport.socket_type(),
            Some(probe.transport.protocol()),
        )?;

        // Be explicit about the two options that materially affect occupancy
        // results. Address reuse stays disabled, and IPv6 probes never accept
        // IPv4-mapped addresses regardless of the platform default.
        socket.set_reuse_address(false)?;
        if probe.address.is_ipv6() {
            socket.set_only_v6(true)?;
        }

        socket.bind(&probe.address.into())
    }

    fn is_occupied_with<F>(
        port: Port,
        config: &OccupancyCheckConfig,
        mut run_probe: F,
    ) -> Result<bool>
    where
        F: FnMut(&OccupancyProbe) -> io::Result<()>,
    {
        for probe in Self::probes(port, config) {
            match run_probe(&probe) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => return Ok(true),
                Err(source) => {
                    return Err(Error::OccupancyCheckFailed {
                        port,
                        source: Box::new(ProbeFailure { probe, source }),
                    });
                }
            }
        }

        Ok(false)
    }
}

impl PortOccupancyChecker for SystemOccupancyChecker {
    fn is_occupied(&self, port: Port, config: &OccupancyCheckConfig) -> Result<bool> {
        Self::is_occupied_with(port, config, Self::probe_socket)
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
    fn is_occupied(&self, port: Port, config: &OccupancyCheckConfig) -> Result<bool> {
        if (config.skip_tcp && config.skip_udp) || (config.skip_ipv4 && config.skip_ipv6) {
            return Ok(false);
        }
        Ok(self.occupied_ports.contains(&port))
    }

    fn find_occupied_ports(
        &self,
        range: &PortRange,
        config: &OccupancyCheckConfig,
    ) -> Result<Vec<Port>> {
        if (config.skip_tcp && config.skip_udp) || (config.skip_ipv4 && config.skip_ipv6) {
            return Ok(Vec::new());
        }

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
    use serial_test::serial;
    use std::net::{Ipv4Addr, Ipv6Addr, TcpListener, UdpSocket};

    fn bind_ipv6_only_tcp(port: u16) -> io::Result<TcpListener> {
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_reuse_address(false)?;
        socket.set_only_v6(true)?;
        socket.bind(&SocketAddr::from((Ipv6Addr::LOCALHOST, port)).into())?;
        socket.listen(1)?;
        Ok(socket.into())
    }

    fn tcp_ipv4_with_udp_free() -> TcpListener {
        for _ in 0..100 {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            if UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
                return listener;
            }
        }
        panic!("failed to find a TCP/IPv4 port with UDP/IPv4 free");
    }

    fn udp_ipv4_with_tcp_free() -> UdpSocket {
        for _ in 0..100 {
            let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = socket.local_addr().unwrap().port();
            if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
                return socket;
            }
        }
        panic!("failed to find a UDP/IPv4 port with TCP/IPv4 free");
    }

    fn tcp_ipv4_with_ipv6_free() -> io::Result<TcpListener> {
        for _ in 0..100 {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            let port = listener.local_addr()?.port();
            if bind_ipv6_only_tcp(port).is_ok() {
                return Ok(listener);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "failed to find a TCP/IPv4 port with TCP/IPv6 free",
        ))
    }

    fn tcp_ipv6_with_ipv4_free() -> io::Result<TcpListener> {
        for _ in 0..100 {
            let listener = bind_ipv6_only_tcp(0)?;
            let port = listener.local_addr()?.port();
            if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
                return Ok(listener);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "failed to find a TCP/IPv6 port with TCP/IPv4 free",
        ))
    }

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
    fn test_occupancy_check_config_skip_all() {
        let occ_config = OccupancyConfig {
            skip: Some(true),
            ..Default::default()
        };

        let config = OccupancyCheckConfig::from(&occ_config);
        assert!(config.skip_tcp);
        assert!(config.skip_udp);
        assert!(config.skip_ipv4);
        assert!(config.skip_ipv6);
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
    fn test_mock_checker_respects_skip_all_config() {
        let mut occupied = HashSet::new();
        occupied.insert(Port::try_from(8080).unwrap());
        let checker = MockOccupancyChecker::new(occupied);
        let config = OccupancyCheckConfig {
            skip_tcp: true,
            skip_udp: true,
            ..Default::default()
        };

        assert!(!checker
            .is_occupied(Port::try_from(8080).unwrap(), &config)
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
    fn test_system_checker_builds_exact_selected_probe_matrix() {
        let port = Port::try_from(5050).unwrap();
        let config = OccupancyCheckConfig {
            skip_tcp: true,
            skip_ipv6: true,
            check_all_interfaces: true,
            ..Default::default()
        };

        assert_eq!(
            SystemOccupancyChecker::probes(port, &config),
            vec![
                OccupancyProbe {
                    transport: Transport::Udp,
                    address: SocketAddr::from((Ipv4Addr::LOCALHOST, 5050)),
                },
                OccupancyProbe {
                    transport: Transport::Udp,
                    address: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5050)),
                },
            ]
        );
    }

    #[test]
    fn test_system_checker_all_interfaces_adds_wildcards_to_localhost_matrix() {
        let port = Port::try_from(5050).unwrap();
        let config = OccupancyCheckConfig {
            check_all_interfaces: true,
            ..Default::default()
        };
        let probes = SystemOccupancyChecker::probes(port, &config);

        assert_eq!(probes.len(), 8);
        for transport in [Transport::Tcp, Transport::Udp] {
            for address in [
                SocketAddr::from((Ipv4Addr::LOCALHOST, 5050)),
                SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5050)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, 5050)),
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, 5050)),
            ] {
                assert!(probes.contains(&OccupancyProbe { transport, address }));
            }
        }
    }

    #[test]
    fn test_system_checker_addr_in_use_is_occupied() {
        let port = Port::try_from(5050).unwrap();
        let mut probes_run = 0;
        let occupied = SystemOccupancyChecker::is_occupied_with(
            port,
            &OccupancyCheckConfig::default(),
            |_| {
                probes_run += 1;
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "synthetic bind conflict",
                ))
            },
        )
        .unwrap();

        assert!(occupied);
        assert_eq!(probes_run, 1);
    }

    #[test]
    fn test_system_checker_permission_error_fails_closed_with_probe_context() {
        let port = Port::try_from(5050).unwrap();
        let error = SystemOccupancyChecker::is_occupied_with(
            port,
            &OccupancyCheckConfig::default(),
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic permission failure",
                ))
            },
        )
        .unwrap_err();
        let diagnostic = error.to_string();

        assert!(matches!(error, Error::OccupancyCheckFailed { .. }));
        assert!(diagnostic.contains("TCP IPv4 localhost bind at 127.0.0.1:5050"));
        assert!(diagnostic.contains("synthetic permission failure"));
    }

    #[test]
    fn test_system_checker_unknown_error_fails_closed_with_probe_context() {
        let port = Port::try_from(5050).unwrap();
        let error = SystemOccupancyChecker::is_occupied_with(
            port,
            &OccupancyCheckConfig::default(),
            |_| Err(io::Error::other("synthetic unknown failure")),
        )
        .unwrap_err();
        let diagnostic = error.to_string();

        assert!(matches!(error, Error::OccupancyCheckFailed { .. }));
        assert!(diagnostic.contains("TCP IPv4 localhost bind at 127.0.0.1:5050"));
        assert!(diagnostic.contains("synthetic unknown failure"));
    }

    #[test]
    fn test_system_checker_skipped_unsupported_family_is_never_probed() {
        let port = Port::try_from(5050).unwrap();
        let unsupported_ipv6 = |probe: &OccupancyProbe| {
            if probe.address.is_ipv6() {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "synthetic IPv6 unavailable",
                ))
            } else {
                Ok(())
            }
        };

        let error = SystemOccupancyChecker::is_occupied_with(
            port,
            &OccupancyCheckConfig::default(),
            unsupported_ipv6,
        )
        .unwrap_err();
        assert!(error.to_string().contains("IPv6"));
        assert!(error.to_string().contains("synthetic IPv6 unavailable"));

        let skip_ipv6 = OccupancyCheckConfig {
            skip_ipv6: true,
            ..Default::default()
        };
        assert!(
            !SystemOccupancyChecker::is_occupied_with(port, &skip_ipv6, unsupported_ipv6).unwrap()
        );
    }

    #[test]
    #[serial]
    fn test_system_checker_respects_tcp_and_udp_skips() {
        let checker = SystemOccupancyChecker;
        let tcp = tcp_ipv4_with_udp_free();
        let tcp_port = Port::try_from(tcp.local_addr().unwrap().port()).unwrap();
        let udp = udp_ipv4_with_tcp_free();
        let udp_port = Port::try_from(udp.local_addr().unwrap().port()).unwrap();
        let ipv4_only = OccupancyCheckConfig {
            skip_ipv6: true,
            ..Default::default()
        };

        assert!(checker.is_occupied(tcp_port, &ipv4_only).unwrap());
        assert!(checker.is_occupied(udp_port, &ipv4_only).unwrap());

        let skip_tcp = OccupancyCheckConfig {
            skip_tcp: true,
            skip_ipv6: true,
            ..Default::default()
        };
        let skip_udp = OccupancyCheckConfig {
            skip_udp: true,
            skip_ipv6: true,
            ..Default::default()
        };

        assert!(!checker.is_occupied(tcp_port, &skip_tcp).unwrap());
        assert!(!checker.is_occupied(udp_port, &skip_udp).unwrap());
    }

    #[test]
    #[serial]
    fn test_system_checker_respects_ipv4_and_ipv6_skips() {
        let checker = SystemOccupancyChecker;
        let Ok(ipv4) = tcp_ipv4_with_ipv6_free() else {
            return;
        };
        let ipv4_port = Port::try_from(ipv4.local_addr().unwrap().port()).unwrap();
        let Ok(ipv6) = tcp_ipv6_with_ipv4_free() else {
            return;
        };
        let ipv6_port = Port::try_from(ipv6.local_addr().unwrap().port()).unwrap();
        let tcp_only = OccupancyCheckConfig {
            skip_udp: true,
            ..Default::default()
        };

        assert!(checker.is_occupied(ipv4_port, &tcp_only).unwrap());
        assert!(checker.is_occupied(ipv6_port, &tcp_only).unwrap());

        let skip_ipv4 = OccupancyCheckConfig {
            skip_ipv4: true,
            skip_udp: true,
            ..Default::default()
        };
        let skip_ipv6 = OccupancyCheckConfig {
            skip_ipv6: true,
            skip_udp: true,
            ..Default::default()
        };

        assert!(!checker.is_occupied(ipv4_port, &skip_ipv4).unwrap());
        assert!(!checker.is_occupied(ipv6_port, &skip_ipv6).unwrap());
    }

    #[test]
    #[serial]
    fn test_system_checker_adds_wildcard_probes_for_all_interfaces() {
        let checker = SystemOccupancyChecker;
        let mut listener = None;
        for _ in 0..100 {
            let Ok(candidate) = TcpListener::bind((Ipv4Addr::new(127, 0, 0, 2), 0)) else {
                return;
            };
            let port = candidate.local_addr().unwrap().port();
            if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
                listener = Some(candidate);
                break;
            }
        }
        let Some(listener) = listener else {
            return;
        };
        let port = Port::try_from(listener.local_addr().unwrap().port()).unwrap();
        let localhost_only = OccupancyCheckConfig {
            skip_udp: true,
            skip_ipv6: true,
            ..Default::default()
        };
        let all_interfaces = OccupancyCheckConfig {
            check_all_interfaces: true,
            ..localhost_only.clone()
        };

        assert!(!checker.is_occupied(port, &localhost_only).unwrap());
        assert!(checker.is_occupied(port, &all_interfaces).unwrap());
    }
}
