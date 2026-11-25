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
//!
//! The policy decisions are made by the verified kernel in
//! `crate::verified::occupancy`: combining configuration into effective flags,
//! selecting and ordering the probe matrix, deciding when checking is
//! disabled, folding probe outcomes into a report (fail closed on the first
//! probe failure), and classifying owner metadata completeness. This module
//! supplies the effects: socket binds, error classification, and owner
//! lookups.

use std::cell::RefCell;
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
use crate::verified::occupancy as kernel;
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

impl OccupancyCheckConfig {
    /// The verified kernel's view of this policy (a field-for-field copy).
    const fn flags(&self) -> kernel::OccFlags {
        kernel::OccFlags {
            skip_tcp: self.skip_tcp,
            skip_udp: self.skip_udp,
            skip_ipv4: self.skip_ipv4,
            skip_ipv6: self.skip_ipv6,
            check_all_interfaces: self.check_all_interfaces,
        }
    }

    /// The policy described by the kernel's flags (a field-for-field copy).
    const fn from_flags(flags: kernel::OccFlags) -> Self {
        Self {
            skip_tcp: flags.skip_tcp,
            skip_udp: flags.skip_udp,
            skip_ipv4: flags.skip_ipv4,
            skip_ipv6: flags.skip_ipv6,
            check_all_interfaces: flags.check_all_interfaces,
        }
    }
}

impl From<&OccupancyConfig> for OccupancyCheckConfig {
    /// Each skip flag is set when `skip` or its own setting is `Some(true)`;
    /// unset (`None`) settings mean `false`, and `Some(false)` never re-enables
    /// a check that `skip` disabled. `check_all_interfaces` is independent of
    /// `skip` (with every family or protocol skipped, no probe runs anyway).
    fn from(config: &OccupancyConfig) -> Self {
        // Note: Field name divergence between `skip_ip4`/`skip_ip6` (in OccupancyConfig)
        // and `skip_ipv4`/`skip_ipv6` (in OccupancyCheckConfig) is intentional.
        // The config uses abbreviated names for brevity, while the runtime struct uses
        // full names for clarity.
        Self::from_flags(kernel::effective_occupancy_flags(kernel::OccSettings {
            skip: config.skip,
            skip_tcp: config.skip_tcp,
            skip_udp: config.skip_udp,
            skip_ipv4: config.skip_ip4,
            skip_ipv6: config.skip_ip6,
            check_all_interfaces: config.check_all_interfaces,
        }))
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
    /// for efficiency. The default implementation calls `is_occupied` for each
    /// port in ascending order and returns the occupied ones in that order,
    /// stopping at the first failed check.
    ///
    /// # Errors
    ///
    /// Returns an error if any occupancy check fails.
    fn find_occupied_ports(
        &self,
        range: &PortRange,
        config: &OccupancyCheckConfig,
    ) -> Result<Vec<Port>> {
        scan_range(range, |port| {
            Ok(self.is_occupied(port, config)?.then_some(port))
        })
    }
}

/// Inspects every port of `range` in ascending order and collects the values
/// `inspect` keeps, stopping at the first error.
///
/// The scan itself is the verified kernel's `scan_ports`; this glue only
/// converts between `Port` and `u16`.
fn scan_range<T>(range: &PortRange, inspect: impl Fn(Port) -> Result<Option<T>>) -> Result<Vec<T>> {
    kernel::scan_ports(range.min().value(), range.max().value(), &|port| {
        // Never fails: the kernel only inspects ports of the range.
        inspect(Port::try_from(port)?)
    })
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
        self.status = match kernel::owner_status(
            self.process_id.is_some(),
            self.process_name.is_some(),
            self.user.is_some(),
        ) {
            kernel::OwnerStatus::Available => OccupancyOwnerStatus::Available,
            kernel::OwnerStatus::Partial => OccupancyOwnerStatus::Partial,
            kernel::OwnerStatus::Unavailable => OccupancyOwnerStatus::Unavailable,
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
    /// The bind probe that realizes the kernel's `probe` for `port`.
    fn new(port: Port, probe: kernel::Probe) -> Self {
        use kernel::{Family, Scope};

        let protocol = match probe.protocol {
            kernel::Protocol::Tcp => OccupancyProtocol::Tcp,
            kernel::Protocol::Udp => OccupancyProtocol::Udp,
        };
        let ip: IpAddr = match (probe.family, probe.scope) {
            (Family::Ipv4, Scope::Localhost) => Ipv4Addr::LOCALHOST.into(),
            (Family::Ipv4, Scope::Wildcard) => Ipv4Addr::UNSPECIFIED.into(),
            (Family::Ipv6, Scope::Localhost) => Ipv6Addr::LOCALHOST.into(),
            (Family::Ipv6, Scope::Wildcard) => Ipv6Addr::UNSPECIFIED.into(),
        };
        Self {
            protocol,
            address: SocketAddr::new(ip, port.value()),
        }
    }

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
    /// The bind probes the policy selects for `port`, in matrix order.
    #[cfg(test)]
    fn probes(port: Port, config: &OccupancyCheckConfig) -> Vec<OccupancyProbe> {
        kernel::probe_matrix(config.flags())
            .into_iter()
            .map(|probe| OccupancyProbe::new(port, probe))
            .collect()
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

    /// Runs the policy's probe matrix for `port` through `run_probe`.
    ///
    /// The verified kernel selects the probes, runs them in matrix order, and
    /// stops at the first failure; this glue only maps kernel probes to
    /// socket addresses and classifies each bind result: success is free,
    /// `AddrInUse` is occupied, and any other error is a probe failure.
    fn report_with<F>(
        port: Port,
        config: &OccupancyCheckConfig,
        run_probe: F,
    ) -> Result<OccupancyReport>
    where
        F: FnMut(&OccupancyProbe) -> io::Result<()>,
    {
        // The kernel's oracle is a `Fn`; the probe runner may be stateful.
        // The kernel never re-enters the oracle, so the borrow cannot clash.
        let run_probe = RefCell::new(run_probe);
        let outcome = |probe: kernel::Probe| {
            let mut run_probe = run_probe.borrow_mut();
            match (*run_probe)(&OccupancyProbe::new(port, probe)) {
                Ok(()) => kernel::ProbeOutcome::Free,
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                    kernel::ProbeOutcome::InUse
                }
                Err(error) => kernel::ProbeOutcome::Failed(error),
            }
        };

        match kernel::run_probe_matrix(config.flags(), &outcome) {
            Ok(evidence) => Ok(OccupancyReport {
                port,
                enabled_probe_count: evidence.enabled,
                occupied_probes: evidence
                    .occupied
                    .into_iter()
                    .map(|probe| OccupancyProbe::new(port, probe).occupied_evidence())
                    .collect(),
            }),
            Err((probe, source)) => Err(Error::OccupancyCheckFailed {
                port,
                source: Box::new(ProbeFailure {
                    probe: OccupancyProbe::new(port, probe),
                    source,
                }),
            }),
        }
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
        let mut reports = Self::find_occupied_reports_with(range, config, Self::probe_socket)?;
        Self::enrich_owner_metadata(&mut reports);
        Ok(reports)
    }

    /// The range scan behind `find_occupied_reports`, with the bind probes
    /// run by `run_probe`: the verified `scan_ports` inspects the range in
    /// ascending order and stops at the first failure, `report_with` builds
    /// each port's report, and only occupied ports' reports are kept.
    fn find_occupied_reports_with<F>(
        range: &PortRange,
        config: &OccupancyCheckConfig,
        run_probe: F,
    ) -> Result<Vec<OccupancyReport>>
    where
        F: Fn(&OccupancyProbe) -> io::Result<()>,
    {
        scan_range(range, |port| {
            let report = Self::report_with(port, config, &run_probe)?;
            Ok(report.is_occupied().then_some(report))
        })
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
        // Disabled exactly when the system checker's probe matrix is empty.
        if kernel::occupancy_disabled(config.flags()) {
            return Ok(false);
        }
        Ok(self.occupied_ports.contains(&port))
    }

    fn find_occupied_ports(
        &self,
        range: &PortRange,
        config: &OccupancyCheckConfig,
    ) -> Result<Vec<Port>> {
        if kernel::occupancy_disabled(config.flags()) {
            return Ok(Vec::new());
        }

        scan_range(range, |port| {
            Ok(self.occupied_ports.contains(&port).then_some(port))
        })
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
        assert!(
            occupied_in_range.is_empty(),
            "unexpected occupied ports: {occupied_in_range:?}"
        );
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
        assert!(
            report.occupied_probes.is_empty(),
            "unexpected occupied probes: {:?}",
            report.occupied_probes
        );
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

    // -----------------------------------------------------------------------
    // Glue tests for the verified occupancy kernel
    // -----------------------------------------------------------------------

    /// Every combination of the five effective flags.
    fn every_check_config() -> Vec<OccupancyCheckConfig> {
        (0..32_u8)
            .map(|bits| OccupancyCheckConfig {
                skip_tcp: bits & 1 != 0,
                skip_udp: bits & 2 != 0,
                skip_ipv4: bits & 4 != 0,
                skip_ipv6: bits & 8 != 0,
                check_all_interfaces: bits & 16 != 0,
            })
            .collect()
    }

    /// Independent statement of the documented matrix: families (IPv4, IPv6),
    /// then scopes (localhost, wildcard when enabled), then protocols (TCP,
    /// UDP), skipping disabled dimensions.
    fn documented_matrix(port: Port, config: &OccupancyCheckConfig) -> Vec<OccupancyProbe> {
        let mut families: Vec<(IpAddr, IpAddr)> = Vec::new();
        if !config.skip_ipv4 {
            families.push((Ipv4Addr::LOCALHOST.into(), Ipv4Addr::UNSPECIFIED.into()));
        }
        if !config.skip_ipv6 {
            families.push((Ipv6Addr::LOCALHOST.into(), Ipv6Addr::UNSPECIFIED.into()));
        }
        let mut protocols = Vec::new();
        if !config.skip_tcp {
            protocols.push(OccupancyProtocol::Tcp);
        }
        if !config.skip_udp {
            protocols.push(OccupancyProtocol::Udp);
        }

        let mut probes = Vec::new();
        for (localhost, wildcard) in families {
            let mut addresses = vec![localhost];
            if config.check_all_interfaces {
                addresses.push(wildcard);
            }
            for ip in addresses {
                for &protocol in &protocols {
                    probes.push(OccupancyProbe {
                        protocol,
                        address: SocketAddr::new(ip, port.value()),
                    });
                }
            }
        }
        probes
    }

    #[test]
    fn test_occupancy_check_config_from_every_setting_combination() {
        let values = [None, Some(false), Some(true)];
        let on = |setting: Option<bool>| setting == Some(true);
        for code in 0..3_usize.pow(6) {
            let digit = |position: u32| values[code / 3_usize.pow(position) % 3];
            let settings = OccupancyConfig {
                skip: digit(0),
                skip_tcp: digit(1),
                skip_udp: digit(2),
                skip_ip4: digit(3),
                skip_ip6: digit(4),
                check_all_interfaces: digit(5),
            };

            let effective = OccupancyCheckConfig::from(&settings);

            assert_eq!(
                effective,
                OccupancyCheckConfig {
                    skip_tcp: on(settings.skip) || on(settings.skip_tcp),
                    skip_udp: on(settings.skip) || on(settings.skip_udp),
                    skip_ipv4: on(settings.skip) || on(settings.skip_ip4),
                    skip_ipv6: on(settings.skip) || on(settings.skip_ip6),
                    check_all_interfaces: on(settings.check_all_interfaces),
                },
                "{settings:?}"
            );
            if on(settings.skip) {
                assert!(
                    SystemOccupancyChecker::probes(Port::try_from(5050).unwrap(), &effective)
                        .is_empty(),
                    "skip = true must disable every probe: {settings:?}"
                );
            }
        }
    }

    #[test]
    fn test_skip_false_does_not_override_global_skip() {
        let config = OccupancyCheckConfig::from(&OccupancyConfig {
            skip: Some(true),
            skip_tcp: Some(false),
            skip_udp: Some(false),
            skip_ip4: Some(false),
            skip_ip6: Some(false),
            check_all_interfaces: Some(true),
        });

        assert!(config.skip_tcp && config.skip_udp && config.skip_ipv4 && config.skip_ipv6);
        assert!(config.check_all_interfaces);
        assert_eq!(
            SystemOccupancyChecker::probes(Port::try_from(5050).unwrap(), &config),
            Vec::new()
        );
    }

    #[test]
    fn test_report_runs_exactly_the_documented_matrix_for_every_policy() {
        for config in every_check_config() {
            for port_value in [1, 5050, 65535] {
                let port = Port::try_from(port_value).unwrap();
                let expected = documented_matrix(port, &config);
                let mut run = Vec::new();

                let report = SystemOccupancyChecker::report_with(port, &config, |probe| {
                    run.push(*probe);
                    Ok(())
                })
                .unwrap();

                assert_eq!(run, expected, "{config:?} port {port_value}");
                assert_eq!(SystemOccupancyChecker::probes(port, &config), expected);
                assert_eq!(report.port, port);
                assert_eq!(report.enabled_probe_count, expected.len());
                assert!(!report.is_occupied());
                assert_eq!(report.was_skipped(), expected.is_empty());
            }
        }
    }

    #[test]
    fn test_all_interfaces_matrix_is_in_documented_order() {
        let port = Port::try_from(65535).unwrap();
        let config = OccupancyCheckConfig {
            check_all_interfaces: true,
            ..Default::default()
        };
        let probe = |protocol, ip: IpAddr| OccupancyProbe {
            protocol,
            address: SocketAddr::new(ip, 65535),
        };
        let (tcp, udp) = (OccupancyProtocol::Tcp, OccupancyProtocol::Udp);

        assert_eq!(
            SystemOccupancyChecker::probes(port, &config),
            vec![
                probe(tcp, Ipv4Addr::LOCALHOST.into()),
                probe(udp, Ipv4Addr::LOCALHOST.into()),
                probe(tcp, Ipv4Addr::UNSPECIFIED.into()),
                probe(udp, Ipv4Addr::UNSPECIFIED.into()),
                probe(tcp, Ipv6Addr::LOCALHOST.into()),
                probe(udp, Ipv6Addr::LOCALHOST.into()),
                probe(tcp, Ipv6Addr::UNSPECIFIED.into()),
                probe(udp, Ipv6Addr::UNSPECIFIED.into()),
            ]
        );
        assert_eq!(
            SystemOccupancyChecker::probes(port, &OccupancyCheckConfig::default()),
            vec![
                probe(tcp, Ipv4Addr::LOCALHOST.into()),
                probe(udp, Ipv4Addr::LOCALHOST.into()),
                probe(tcp, Ipv6Addr::LOCALHOST.into()),
                probe(udp, Ipv6Addr::LOCALHOST.into()),
            ]
        );
    }

    #[test]
    fn test_mock_checker_disabled_exactly_when_system_matrix_is_empty() {
        let port = Port::try_from(5050).unwrap();
        let checker = MockOccupancyChecker::new(HashSet::from([port]));
        let range = PortRange::new(port, port).unwrap();

        for config in every_check_config() {
            let matrix_empty = SystemOccupancyChecker::probes(port, &config).is_empty();
            let report = SystemOccupancyChecker::report_with(port, &config, |_| Ok(())).unwrap();

            assert_eq!(report.was_skipped(), matrix_empty, "{config:?}");
            assert_eq!(
                checker.is_occupied(port, &config).unwrap(),
                !matrix_empty,
                "{config:?}"
            );
            assert_eq!(
                checker.find_occupied_ports(&range, &config).unwrap(),
                if matrix_empty { vec![] } else { vec![port] },
                "{config:?}"
            );
        }
    }

    #[test]
    fn test_detailed_report_evidence_is_the_in_use_probes_for_every_policy() {
        let port = Port::try_from(5050).unwrap();
        for config in every_check_config() {
            let expected = documented_matrix(port, &config);
            // Alternate in-use and free answers in run order.
            let mut calls = 0;
            let report = SystemOccupancyChecker::report_with(port, &config, |_| {
                calls += 1;
                if calls % 2 == 1 {
                    Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "synthetic bind conflict",
                    ))
                } else {
                    Ok(())
                }
            })
            .unwrap();

            assert_eq!(calls, expected.len(), "{config:?}");
            assert_eq!(
                report.occupied_probes,
                expected
                    .iter()
                    .step_by(2)
                    .map(|probe| probe.occupied_evidence())
                    .collect::<Vec<_>>(),
                "{config:?}"
            );
            assert_eq!(report.is_occupied(), !expected.is_empty(), "{config:?}");
        }
    }

    #[test]
    fn test_failure_on_last_probe_runs_every_earlier_probe_and_fails_closed() {
        let port = Port::try_from(5050).unwrap();
        let config = OccupancyCheckConfig {
            check_all_interfaces: true,
            ..Default::default()
        };
        let mut calls = 0;
        let error = SystemOccupancyChecker::report_with(port, &config, |_| {
            calls += 1;
            if calls == 8 {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic last-probe failure",
                ))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "synthetic bind conflict",
                ))
            }
        })
        .unwrap_err();
        let diagnostic = error.to_string();

        assert_eq!(calls, 8);
        assert!(matches!(error, Error::OccupancyCheckFailed { port: p, .. } if p == port));
        assert!(diagnostic.contains("UDP IPv6 wildcard bind at [::]:5050"));
        assert!(diagnostic.contains("synthetic last-probe failure"));
    }

    /// `find_occupied_reports`' range scan (the glue `trop scan` uses), with
    /// scripted binds over a range ending at 65535: only occupied ports'
    /// reports are kept, in ascending port order, each with the in-use
    /// probes in matrix order; the scan stops at the first failing port and
    /// reports that port's failure, and no later port is probed.
    #[test]
    fn test_find_occupied_reports_keeps_occupied_ports_in_order_and_stops_at_first_failure() {
        let range = PortRange::new(
            Port::try_from(65532).unwrap(),
            Port::try_from(65535).unwrap(),
        )
        .unwrap();
        let config = OccupancyCheckConfig {
            skip_udp: true,
            skip_ipv6: true,
            ..Default::default()
        };
        let in_use = || {
            Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "synthetic bind conflict",
            ))
        };

        // 65532 free, 65533 and 65535 occupied, 65534 free.
        let probed = RefCell::new(Vec::new());
        let reports =
            SystemOccupancyChecker::find_occupied_reports_with(&range, &config, |probe| {
                probed.borrow_mut().push(probe.address.port());
                match probe.address.port() {
                    65533 | 65535 => in_use(),
                    _ => Ok(()),
                }
            })
            .unwrap();
        assert_eq!(probed.take(), vec![65532, 65533, 65534, 65535]);
        assert_eq!(
            reports
                .iter()
                .map(|report| report.port.value())
                .collect::<Vec<_>>(),
            vec![65533, 65535]
        );
        for report in &reports {
            assert_eq!(report.enabled_probe_count, 1);
            assert_eq!(
                report.occupied_probes,
                vec![OccupiedProbe {
                    protocol: OccupancyProtocol::Tcp,
                    address_family: OccupancyAddressFamily::Ipv4,
                    scope: OccupancyScope::Localhost,
                    address: Ipv4Addr::LOCALHOST.into(),
                    owner: OccupancyOwner::default(),
                }]
            );
        }

        // A failure at 65534 ends the scan there: 65535 is never probed and
        // the occupied 65533 is not reported.
        let probed = RefCell::new(Vec::new());
        let error = SystemOccupancyChecker::find_occupied_reports_with(&range, &config, |probe| {
            probed.borrow_mut().push(probe.address.port());
            match probe.address.port() {
                65533 => in_use(),
                65534 => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic scan failure",
                )),
                _ => Ok(()),
            }
        })
        .unwrap_err();
        assert_eq!(probed.take(), vec![65532, 65533, 65534]);
        assert!(
            matches!(error, Error::OccupancyCheckFailed { port, .. } if port.value() == 65534),
            "{error}"
        );
        assert!(error.to_string().contains("synthetic scan failure"));

        // A single-port range at the top of the domain, and a skipped policy
        // that probes nothing and reports nothing.
        let top = PortRange::new(
            Port::try_from(65535).unwrap(),
            Port::try_from(65535).unwrap(),
        )
        .unwrap();
        let reports =
            SystemOccupancyChecker::find_occupied_reports_with(&top, &config, |_| in_use())
                .unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].port.value(), 65535);
        let skipped = OccupancyCheckConfig {
            skip_tcp: true,
            skip_udp: true,
            ..Default::default()
        };
        let reports = SystemOccupancyChecker::find_occupied_reports_with(&range, &skipped, |_| {
            unreachable!("a skipped policy runs no probe")
        })
        .unwrap();
        assert_eq!(reports, Vec::<OccupancyReport>::new());
    }

    #[test]
    fn test_owner_status_reflects_field_completeness() {
        for bits in 0..8_u8 {
            let mut owner = OccupancyOwner {
                process_id: (bits & 1 != 0).then_some(42),
                process_name: (bits & 2 != 0).then(|| "service".to_string()),
                user: (bits & 4 != 0).then(|| "operator".to_string()),
                status: OccupancyOwnerStatus::Available,
            };
            owner.refresh_status();

            let expected = match bits {
                0 => OccupancyOwnerStatus::Unavailable,
                7 => OccupancyOwnerStatus::Available,
                _ => OccupancyOwnerStatus::Partial,
            };
            assert_eq!(owner.status, expected, "fields present: {bits:03b}");
        }
    }

    /// A checker that only implements `is_occupied`, so the trait's default
    /// `find_occupied_ports` is exercised. Odd ports are occupied; checking
    /// `failing` fails. Every checked port is logged.
    struct ScriptedChecker {
        failing: Option<u16>,
        checked: std::sync::Mutex<Vec<u16>>,
    }

    impl ScriptedChecker {
        fn new(failing: Option<u16>) -> Self {
            Self {
                failing,
                checked: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn checked(&self) -> Vec<u16> {
            self.checked.lock().unwrap().clone()
        }
    }

    impl PortOccupancyChecker for ScriptedChecker {
        fn is_occupied(&self, port: Port, _config: &OccupancyCheckConfig) -> Result<bool> {
            self.checked.lock().unwrap().push(port.value());
            if Some(port.value()) == self.failing {
                return Err(Error::OccupancyCheckFailed {
                    port,
                    source: Box::new(io::Error::other("synthetic scan failure")),
                });
            }
            Ok(port.value() % 2 == 1)
        }
    }

    fn port_range(min: u16, max: u16) -> PortRange {
        PortRange::new(Port::try_from(min).unwrap(), Port::try_from(max).unwrap()).unwrap()
    }

    fn ports(values: &[u16]) -> Vec<Port> {
        values
            .iter()
            .map(|&value| Port::try_from(value).unwrap())
            .collect()
    }

    #[test]
    fn test_default_find_occupied_ports_scans_range_in_ascending_order() {
        let config = OccupancyCheckConfig::default();

        let checker = ScriptedChecker::new(None);
        let occupied = checker
            .find_occupied_ports(&port_range(65530, 65535), &config)
            .unwrap();
        assert_eq!(occupied, ports(&[65531, 65533, 65535]));
        assert_eq!(checker.checked(), (65530..=65535).collect::<Vec<_>>());

        let checker = ScriptedChecker::new(None);
        let occupied = checker
            .find_occupied_ports(&port_range(1, 1), &config)
            .unwrap();
        assert_eq!(occupied, ports(&[1]));
        assert_eq!(checker.checked(), vec![1]);
    }

    #[test]
    fn test_default_find_occupied_ports_stops_at_first_failure() {
        let checker = ScriptedChecker::new(Some(5003));
        let error = checker
            .find_occupied_ports(&port_range(5000, 5010), &OccupancyCheckConfig::default())
            .unwrap_err();

        assert!(error.to_string().contains("synthetic scan failure"));
        assert_eq!(checker.checked(), vec![5000, 5001, 5002, 5003]);
    }

    #[test]
    fn test_mock_find_occupied_ports_returns_ascending_ports() {
        let checker =
            MockOccupancyChecker::new(ports(&[65535, 1, 3, 2, 40000]).into_iter().collect());
        let config = OccupancyCheckConfig::default();

        assert_eq!(
            checker
                .find_occupied_ports(&port_range(1, 65535), &config)
                .unwrap(),
            ports(&[1, 2, 3, 40000, 65535])
        );
        assert_eq!(
            checker
                .find_occupied_ports(&port_range(2, 39999), &config)
                .unwrap(),
            ports(&[2, 3])
        );
    }
}
