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

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use netstat2::{
    get_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, SocketInfo,
};
use socket2::{Domain, Protocol, Socket, Type};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

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

/// Transport protocol represented by one occupancy probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyProtocol {
    /// Transmission Control Protocol.
    Tcp,
    /// User Datagram Protocol.
    Udp,
}

impl OccupancyProtocol {
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

    /// Stable lowercase representation used by machine-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

impl fmt::Display for OccupancyProtocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        })
    }
}

/// Address family represented by one occupancy probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyAddressFamily {
    /// Internet Protocol version 4.
    Ipv4,
    /// Internet Protocol version 6.
    Ipv6,
}

impl OccupancyAddressFamily {
    /// Stable lowercase representation used by machine-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
}

impl fmt::Display for OccupancyAddressFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ipv4 => "IPv4",
            Self::Ipv6 => "IPv6",
        })
    }
}

/// Bind-address scope represented by one occupancy probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyScope {
    /// The protocol family's localhost address.
    Localhost,
    /// The protocol family's unspecified (all-interfaces) address.
    Wildcard,
}

impl OccupancyScope {
    /// Stable lowercase representation used by machine-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Localhost => "localhost",
            Self::Wildcard => "wildcard",
        }
    }
}

impl fmt::Display for OccupancyScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Availability of best-effort process and user metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyOwnerStatus {
    /// Process ID, process name, and user are all available.
    Available,
    /// At least one owner field is available, but the complete set is not.
    Partial,
    /// No owner fields are safely available.
    Unavailable,
}

impl OccupancyOwnerStatus {
    /// Stable lowercase representation used by machine-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
        }
    }
}

impl fmt::Display for OccupancyOwnerStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Best-effort owner metadata for an occupied socket probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyOwner {
    /// Owning process identifier, when the operating system exposes one.
    pub process_id: Option<u32>,
    /// Owning process name, when safely available.
    pub process_name: Option<String>,
    /// Owning user name, when safely available.
    pub user: Option<String>,
    /// Completeness of the owner fields.
    pub status: OccupancyOwnerStatus,
}

impl Default for OccupancyOwner {
    fn default() -> Self {
        Self {
            process_id: None,
            process_name: None,
            user: None,
            status: OccupancyOwnerStatus::Unavailable,
        }
    }
}

impl OccupancyOwner {
    fn refresh_status(&mut self) {
        let available = [
            self.process_id.is_some(),
            self.process_name.is_some(),
            self.user.is_some(),
        ]
        .into_iter()
        .filter(|value| *value)
        .count();
        self.status = match available {
            0 => OccupancyOwnerStatus::Unavailable,
            3 => OccupancyOwnerStatus::Available,
            _ => OccupancyOwnerStatus::Partial,
        };
    }
}

/// Evidence that one enabled bind probe encountered an occupied address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupiedProbe {
    /// Transport protocol checked by the probe.
    pub protocol: OccupancyProtocol,
    /// Address family checked by the probe.
    pub address_family: OccupancyAddressFamily,
    /// Localhost or wildcard scope checked by the probe.
    pub scope: OccupancyScope,
    /// Exact IP address passed to the bind operation.
    pub address: IpAddr,
    /// Best-effort process and user metadata.
    pub owner: OccupancyOwner,
}

/// Detailed, deterministic result of applying one effective occupancy policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccupancyReport {
    /// Port inspected by this report.
    pub port: Port,
    /// Number of probes enabled by the effective policy.
    pub enabled_probe_count: usize,
    /// Occupied probes in the documented matrix order.
    pub occupied_probes: Vec<OccupiedProbe>,
}

impl OccupancyReport {
    /// Whether at least one enabled probe found the port occupied.
    #[must_use]
    pub fn is_occupied(&self) -> bool {
        !self.occupied_probes.is_empty()
    }

    /// Whether the effective policy disabled every probe.
    #[must_use]
    pub const fn was_skipped(&self) -> bool {
        self.enabled_probe_count == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OccupancyProbe {
    protocol: OccupancyProtocol,
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

    fn address_family(self) -> OccupancyAddressFamily {
        if self.address.is_ipv4() {
            OccupancyAddressFamily::Ipv4
        } else {
            OccupancyAddressFamily::Ipv6
        }
    }

    fn scope(self) -> OccupancyScope {
        if self.address.ip().is_unspecified() {
            OccupancyScope::Wildcard
        } else {
            OccupancyScope::Localhost
        }
    }

    fn occupied_evidence(self) -> OccupiedProbe {
        OccupiedProbe {
            protocol: self.protocol,
            address_family: self.address_family(),
            scope: self.scope(),
            address: self.address.ip(),
            owner: OccupancyOwner::default(),
        }
    }
}

impl fmt::Display for OccupancyProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} {} bind at {}",
            self.protocol,
            self.address_family(),
            self.scope(),
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
                    protocol: OccupancyProtocol::Tcp,
                    address,
                });
            }
            if !config.skip_udp {
                probes.push(OccupancyProbe {
                    protocol: OccupancyProtocol::Udp,
                    address,
                });
            }
        }
    }

    fn probe_socket(probe: &OccupancyProbe) -> io::Result<()> {
        let socket = Socket::new(
            probe.domain(),
            probe.protocol.socket_type(),
            Some(probe.protocol.protocol()),
        )?;

        // Be explicit about the options that materially affect occupancy
        // results. Address reuse stays disabled, Windows wildcard probes use
        // exclusive address binding, and IPv6 probes never accept IPv4-mapped
        // addresses regardless of the platform default.
        socket.set_reuse_address(false)?;
        #[cfg(windows)]
        if probe.address.ip().is_unspecified() {
            Self::set_exclusive_address_use(&socket)?;
        }
        if probe.address.is_ipv6() {
            socket.set_only_v6(true)?;
        }

        socket.bind(&probe.address.into())
    }

    #[cfg(windows)]
    #[allow(unsafe_code)]
    fn set_exclusive_address_use(socket: &Socket) -> io::Result<()> {
        use std::mem::size_of_val;
        use std::os::windows::io::AsRawSocket;

        use windows_sys::Win32::Networking::WinSock::{
            setsockopt, WSAGetLastError, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
        };

        let exclusive = 1_i32;
        let raw_socket = usize::try_from(socket.as_raw_socket()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket handle does not fit a Winsock SOCKET",
            )
        })?;
        // SAFETY: `socket` owns a valid Winsock SOCKET, `exclusive` remains
        // alive for the call, and the pointer/length describe that i32 exactly.
        let result = unsafe {
            setsockopt(
                raw_socket,
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                std::ptr::from_ref(&exclusive).cast::<u8>(),
                size_of_val(&exclusive) as i32,
            )
        };

        if result == SOCKET_ERROR {
            // SAFETY: Winsock records the calling thread's last socket error.
            let error = unsafe { WSAGetLastError() };
            Err(io::Error::from_raw_os_error(error))
        } else {
            Ok(())
        }
    }

    fn report_with<F>(
        port: Port,
        config: &OccupancyCheckConfig,
        mut run_probe: F,
    ) -> Result<OccupancyReport>
    where
        F: FnMut(&OccupancyProbe) -> io::Result<()>,
    {
        let probes = Self::probes(port, config);
        let enabled_probe_count = probes.len();
        let mut occupied_probes = Vec::new();

        for probe in probes {
            match run_probe(&probe) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    occupied_probes.push(probe.occupied_evidence());
                }
                Err(source) => {
                    return Err(Error::OccupancyCheckFailed {
                        port,
                        source: Box::new(ProbeFailure { probe, source }),
                    });
                }
            }
        }

        Ok(OccupancyReport {
            port,
            enabled_probe_count,
            occupied_probes,
        })
    }

    fn is_occupied_with<F>(port: Port, config: &OccupancyCheckConfig, run_probe: F) -> Result<bool>
    where
        F: FnMut(&OccupancyProbe) -> io::Result<()>,
    {
        Self::report_with(port, config, run_probe).map(|report| report.is_occupied())
    }

    /// Inspect one port and return every enabled probe that found occupancy.
    ///
    /// Bind probes remain authoritative. Process and user metadata is
    /// best-effort enrichment: lookup failures leave the stable owner fields
    /// explicitly unavailable and never change the occupancy result.
    ///
    /// # Errors
    ///
    /// Returns an error when an enabled bind probe fails for a reason other
    /// than address-in-use.
    pub fn inspect_port(
        &self,
        port: Port,
        config: &OccupancyCheckConfig,
    ) -> Result<OccupancyReport> {
        let mut report = Self::report_with(port, config, Self::probe_socket)?;
        Self::enrich_owner_metadata(std::slice::from_mut(&mut report));
        Ok(report)
    }

    /// Inspect a range and return detailed reports for occupied ports only.
    ///
    /// Reports and their probe evidence follow ascending port order and the
    /// documented probe-matrix order, respectively. Owner lookup is batched
    /// across the range so reporting does not repeatedly enumerate the host's
    /// socket and process tables.
    ///
    /// # Errors
    ///
    /// Returns an error when any enabled bind probe fails for a reason other
    /// than address-in-use.
    pub fn find_occupied_reports(
        &self,
        range: &PortRange,
        config: &OccupancyCheckConfig,
    ) -> Result<Vec<OccupancyReport>> {
        let mut reports = Vec::new();
        for port in *range {
            let report = Self::report_with(port, config, Self::probe_socket)?;
            if report.is_occupied() {
                reports.push(report);
            }
        }
        Self::enrich_owner_metadata(&mut reports);
        Ok(reports)
    }

    fn enrich_owner_metadata(reports: &mut [OccupancyReport]) {
        if !reports.iter().any(OccupancyReport::is_occupied) {
            return;
        }

        let Ok(sockets) = get_sockets_info(
            AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6,
            ProtocolFlags::TCP | ProtocolFlags::UDP,
        ) else {
            return;
        };

        for report in reports.iter_mut() {
            for probe in &mut report.occupied_probes {
                probe.owner.process_id = sockets
                    .iter()
                    .filter(|socket| Self::socket_matches(report.port, probe, socket))
                    .flat_map(|socket| socket.associated_pids.iter().copied())
                    .min();
            }
        }

        let pids = reports
            .iter()
            .flat_map(|report| &report.occupied_probes)
            .filter_map(|probe| probe.owner.process_id)
            .map(Pid::from_u32)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if pids.is_empty() {
            return;
        }

        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_user(UpdateKind::OnlyIfNotSet),
        );
        let users = Users::new_with_refreshed_list();

        for probe in reports
            .iter_mut()
            .flat_map(|report| &mut report.occupied_probes)
        {
            if let Some(pid) = probe.owner.process_id.map(Pid::from_u32) {
                if let Some(process) = system.process(pid) {
                    probe.owner.process_name = Some(process.name().to_string_lossy().into_owned());
                    probe.owner.user = process
                        .user_id()
                        .and_then(|user_id| users.get_user_by_id(user_id))
                        .map(|user| user.name().to_string());
                }
            }
            probe.owner.refresh_status();
        }
    }

    fn socket_matches(port: Port, probe: &OccupiedProbe, socket: &SocketInfo) -> bool {
        let (protocol, address, socket_port) = match &socket.protocol_socket_info {
            ProtocolSocketInfo::Tcp(info) => {
                (OccupancyProtocol::Tcp, info.local_addr, info.local_port)
            }
            ProtocolSocketInfo::Udp(info) => {
                (OccupancyProtocol::Udp, info.local_addr, info.local_port)
            }
        };

        protocol == probe.protocol
            && socket_port == port.value()
            && address.is_ipv4() == probe.address.is_ipv4()
            && (address == probe.address
                || address.is_unspecified()
                || probe.address.is_unspecified())
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

    fn tcp_udp_ipv4_pair() -> io::Result<(TcpListener, UdpSocket)> {
        for _ in 0..100 {
            // Select through UDP first, then prove the same port is also
            // bindable by TCP. Some Windows hosts do not make a
            // TCP-selected ephemeral port eligible for UDP.
            let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
            let port = udp.local_addr()?.port();
            if let Ok(tcp) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                return Ok((tcp, udp));
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "failed to find a port bindable by both TCP/IPv4 and UDP/IPv4",
        ))
    }

    fn tcp_ipv4_with_udp_free() -> io::Result<TcpListener> {
        let (tcp, udp) = tcp_udp_ipv4_pair()?;
        drop(udp);
        Ok(tcp)
    }

    fn udp_ipv4_with_tcp_free() -> io::Result<UdpSocket> {
        let (tcp, udp) = tcp_udp_ipv4_pair()?;
        drop(tcp);
        Ok(udp)
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
                    protocol: OccupancyProtocol::Udp,
                    address: SocketAddr::from((Ipv4Addr::LOCALHOST, 5050)),
                },
                OccupancyProbe {
                    protocol: OccupancyProtocol::Udp,
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
        for protocol in [OccupancyProtocol::Tcp, OccupancyProtocol::Udp] {
            for address in [
                SocketAddr::from((Ipv4Addr::LOCALHOST, 5050)),
                SocketAddr::from((Ipv4Addr::UNSPECIFIED, 5050)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, 5050)),
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, 5050)),
            ] {
                assert!(probes.contains(&OccupancyProbe { protocol, address }));
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
        assert_eq!(probes_run, 4);
    }

    #[test]
    fn test_detailed_report_records_all_occupied_probes_in_matrix_order() {
        let port = Port::try_from(5050).unwrap();
        let config = OccupancyCheckConfig {
            check_all_interfaces: true,
            ..Default::default()
        };
        let report = SystemOccupancyChecker::report_with(port, &config, |probe| {
            let occupied = matches!(
                (probe.protocol, probe.address),
                (
                    OccupancyProtocol::Udp,
                    SocketAddr::V4(address)
                ) if address.ip().is_loopback()
            ) || matches!(
                (probe.protocol, probe.address),
                (
                    OccupancyProtocol::Tcp,
                    SocketAddr::V6(address)
                ) if address.ip().is_unspecified()
            );
            if occupied {
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "synthetic bind conflict",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap();

        assert_eq!(report.enabled_probe_count, 8);
        assert_eq!(
            report.occupied_probes,
            vec![
                OccupiedProbe {
                    protocol: OccupancyProtocol::Udp,
                    address_family: OccupancyAddressFamily::Ipv4,
                    scope: OccupancyScope::Localhost,
                    address: Ipv4Addr::LOCALHOST.into(),
                    owner: OccupancyOwner::default(),
                },
                OccupiedProbe {
                    protocol: OccupancyProtocol::Tcp,
                    address_family: OccupancyAddressFamily::Ipv6,
                    scope: OccupancyScope::Wildcard,
                    address: Ipv6Addr::UNSPECIFIED.into(),
                    owner: OccupancyOwner::default(),
                },
            ]
        );
    }

    #[test]
    fn test_detailed_report_distinguishes_disabled_policy_from_available_port() {
        let port = Port::try_from(5050).unwrap();
        let config = OccupancyCheckConfig {
            skip_tcp: true,
            skip_udp: true,
            ..Default::default()
        };
        let report =
            SystemOccupancyChecker::report_with(port, &config, |_| unreachable!()).unwrap();

        assert!(report.was_skipped());
        assert!(!report.is_occupied());
        assert!(report.occupied_probes.is_empty());
    }

    #[test]
    fn test_detailed_report_does_not_hide_later_probe_failures() {
        let port = Port::try_from(5050).unwrap();
        let mut probes_run = 0;
        let error =
            SystemOccupancyChecker::report_with(port, &OccupancyCheckConfig::default(), |_| {
                probes_run += 1;
                match probes_run {
                    1 => Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "synthetic bind conflict",
                    )),
                    2 => Err(io::Error::other("synthetic later failure")),
                    _ => Ok(()),
                }
            })
            .unwrap_err();

        assert_eq!(probes_run, 2);
        assert!(error.to_string().contains("synthetic later failure"));
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
        let Ok(tcp) = tcp_ipv4_with_udp_free() else {
            return;
        };
        let tcp_port = Port::try_from(tcp.local_addr().unwrap().port()).unwrap();
        let Ok(udp) = udp_ipv4_with_tcp_free() else {
            return;
        };
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
