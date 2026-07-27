//! Port and port range types for network port management.
//!
//! This module provides types for working with TCP/UDP ports, including
//! validation, range operations, allocation, and occupancy checking.

pub mod allocator;
pub mod exclusions;
pub mod group;
pub mod occupancy;

// Property-based tests
#[cfg(all(test, feature = "property-tests"))]
mod allocator_proptests;
#[cfg(all(test, feature = "property-tests"))]
mod group_proptests;
#[cfg(all(test, feature = "property-tests"))]
mod proptests;

use std::fmt;

use serde::{Deserialize, Serialize};

/// A valid network port number (1-65535).
///
/// Port 0 is considered invalid as it has special meaning in networking contexts.
///
/// # Examples
///
/// ```
/// use trop::Port;
///
/// // Valid port
/// let port = Port::try_from(8080).unwrap();
/// assert_eq!(port.value(), 8080);
///
/// // Invalid port (0)
/// assert!(Port::try_from(0).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Port(u16);

impl Port {
    /// The minimum valid port number.
    pub const MIN: u16 = 1;

    /// The maximum valid port number.
    pub const MAX: u16 = 65535;

    /// Returns the underlying port number.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Port;
    ///
    /// let port = Port::try_from(8080).unwrap();
    /// assert_eq!(port.value(), 8080);
    /// ```
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Returns `true` if this is a privileged port (< 1024).
    ///
    /// Privileged ports typically require elevated permissions to bind to.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Port;
    ///
    /// let http = Port::try_from(80).unwrap();
    /// assert!(http.is_privileged());
    ///
    /// let high_port = Port::try_from(8080).unwrap();
    /// assert!(!high_port.is_privileged());
    /// ```
    #[must_use]
    pub const fn is_privileged(self) -> bool {
        self.0 < 1024
    }

    /// Add an offset to a port, returning a new port if the result is valid.
    ///
    /// Returns `None` if the result would overflow or be invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Port;
    ///
    /// let port = Port::try_from(5000).unwrap();
    /// assert_eq!(port.checked_add(10).unwrap().value(), 5010);
    ///
    /// // Overflow returns None
    /// let high = Port::try_from(65535).unwrap();
    /// assert!(high.checked_add(1).is_none());
    /// ```
    #[must_use]
    pub const fn checked_add(self, offset: u16) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(result) if result > 0 => Some(Self(result)),
            _ => None,
        }
    }

    /// Subtract an offset from a port, returning a new port if the result is valid.
    ///
    /// Returns `None` if the result would underflow or be invalid (port 0).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::Port;
    ///
    /// let port = Port::try_from(5010).unwrap();
    /// assert_eq!(port.checked_sub(10).unwrap().value(), 5000);
    ///
    /// // Underflow returns None
    /// let low = Port::try_from(5).unwrap();
    /// assert!(low.checked_sub(10).is_none());
    /// ```
    #[must_use]
    pub const fn checked_sub(self, offset: u16) -> Option<Self> {
        match self.0.checked_sub(offset) {
            Some(result) if result > 0 => Some(Self(result)),
            _ => None,
        }
    }
}

impl TryFrom<u16> for Port {
    type Error = InvalidPortError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(InvalidPortError {
                value,
                reason: "port 0 is invalid".into(),
            })
        } else {
            Ok(Self(value))
        }
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error type for invalid port numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPortError {
    /// The invalid port value.
    pub value: u16,
    /// The reason the port is invalid.
    pub reason: String,
}

impl fmt::Display for InvalidPortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid port {}: {}", self.value, self.reason)
    }
}

impl std::error::Error for InvalidPortError {}

/// Structured summary of the automatic cleanup attempted after allocation
/// exhaustion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AutomaticCleanupStatus {
    attempted: bool,
    prune_enabled: bool,
    expire_enabled: bool,
    pruned: usize,
    expired: usize,
}

impl AutomaticCleanupStatus {
    pub(crate) const fn new(
        attempted: bool,
        prune_enabled: bool,
        expire_enabled: bool,
        pruned: usize,
        expired: usize,
    ) -> Self {
        Self {
            attempted,
            prune_enabled,
            expire_enabled,
            pruned,
            expired,
        }
    }

    /// Whether automatic cleanup ran.
    #[must_use]
    pub const fn attempted(self) -> bool {
        self.attempted
    }

    /// Whether stale-path pruning was enabled.
    #[must_use]
    pub const fn prune_enabled(self) -> bool {
        self.prune_enabled
    }

    /// Whether age-based expiration was enabled.
    #[must_use]
    pub const fn expire_enabled(self) -> bool {
        self.expire_enabled
    }

    /// Number of reservations removed by pruning.
    #[must_use]
    pub const fn pruned(self) -> usize {
        self.pruned
    }

    /// Number of reservations removed by expiration.
    #[must_use]
    pub const fn expired(self) -> usize {
        self.expired
    }

    fn policy_description(self) -> &'static str {
        match (self.prune_enabled, self.expire_enabled) {
            (true, true) => "automatic pruning and expiration enabled",
            (true, false) => "automatic expiration disabled",
            (false, true) => "automatic pruning disabled",
            (false, false) => "automatic pruning and expiration disabled",
        }
    }
}

/// Typed reasons that prevented every port in a configured range from being
/// allocated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortExhaustionBlockers {
    reserved: bool,
    excluded: bool,
    occupied: bool,
}

impl PortExhaustionBlockers {
    pub(crate) const fn new(reserved: bool, excluded: bool, occupied: bool) -> Self {
        Self {
            reserved,
            excluded,
            occupied,
        }
    }

    /// Whether stored reservations blocked at least one candidate.
    #[must_use]
    pub const fn reserved(self) -> bool {
        self.reserved
    }

    /// Whether configured exclusions blocked at least one candidate.
    #[must_use]
    pub const fn excluded(self) -> bool {
        self.excluded
    }

    /// Whether operating-system occupancy blocked at least one candidate.
    #[must_use]
    pub const fn occupied(self) -> bool {
        self.occupied
    }
}

impl fmt::Display for PortExhaustionBlockers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (blocked, description) in [
            (self.reserved, "reserved"),
            (self.excluded, "excluded"),
            (self.occupied, "occupied"),
        ] {
            if blocked {
                write!(f, "{separator}{description}")?;
                separator = ", ";
            }
        }
        if separator.is_empty() {
            write!(f, "unavailable")
        } else {
            Ok(())
        }
    }
}

/// Machine-readable context for a [`crate::Error::PortExhausted`] failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PortExhaustionDetails {
    cleanup: AutomaticCleanupStatus,
    blockers: PortExhaustionBlockers,
}

impl PortExhaustionDetails {
    pub(crate) const fn new(
        cleanup: AutomaticCleanupStatus,
        blockers: PortExhaustionBlockers,
    ) -> Self {
        Self { cleanup, blockers }
    }

    /// Automatic-cleanup policy and aggregate outcome.
    #[must_use]
    pub const fn cleanup(self) -> AutomaticCleanupStatus {
        self.cleanup
    }

    /// Typed reasons that kept the range exhausted.
    #[must_use]
    pub const fn blockers(self) -> PortExhaustionBlockers {
        self.blockers
    }
}

impl fmt::Display for PortExhaustionDetails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.cleanup.attempted {
            write!(
                f,
                ": remaining blockers after the single retry: {}; automatic cleanup pruned {} \
                 and expired {} reservation(s); {}",
                self.blockers,
                self.cleanup.pruned,
                self.cleanup.expired,
                self.cleanup.policy_description()
            )
        } else {
            write!(
                f,
                ": cleanup was skipped ({}); remaining blockers: {}",
                self.cleanup.policy_description(),
                self.blockers
            )
        }
    }
}

/// A range of ports (inclusive on both ends).
///
/// # Examples
///
/// ```
/// use trop::{Port, PortRange};
///
/// let min = Port::try_from(5000).unwrap();
/// let max = Port::try_from(5010).unwrap();
/// let range = PortRange::new(min, max).unwrap();
///
/// assert_eq!(range.len(), 11);
/// assert!(range.contains(Port::try_from(5005).unwrap()));
/// assert!(!range.contains(Port::try_from(4999).unwrap()));
/// ```
#[derive(Clone, Copy)]
pub struct PortRange {
    min: Port,
    max: Port,
    exhaustion_details: Option<PortExhaustionDetails>,
}

impl PortRange {
    /// Creates a new port range.
    ///
    /// Returns an error if `max` < `min`.
    ///
    /// # Errors
    ///
    /// Returns an error if `max` is less than `min`.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Port, PortRange};
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5010).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    /// assert_eq!(range.len(), 11);
    /// ```
    pub fn new(min: Port, max: Port) -> Result<Self, InvalidPortRangeError> {
        if max < min {
            Err(InvalidPortRangeError {
                min,
                max,
                reason: "max must be greater than or equal to min".into(),
            })
        } else {
            Ok(Self {
                min,
                max,
                exhaustion_details: None,
            })
        }
    }

    pub(crate) const fn with_exhaustion_details(mut self, details: PortExhaustionDetails) -> Self {
        self.exhaustion_details = Some(details);
        self
    }

    pub(crate) const fn exhaustion_details(self) -> Option<PortExhaustionDetails> {
        self.exhaustion_details
    }

    /// Returns the minimum port in the range.
    #[must_use]
    pub const fn min(&self) -> Port {
        self.min
    }

    /// Returns the maximum port in the range.
    #[must_use]
    pub const fn max(&self) -> Port {
        self.max
    }

    /// Returns `true` if the range contains the given port.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Port, PortRange};
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5010).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    ///
    /// assert!(range.contains(Port::try_from(5000).unwrap()));
    /// assert!(range.contains(Port::try_from(5005).unwrap()));
    /// assert!(range.contains(Port::try_from(5010).unwrap()));
    /// assert!(!range.contains(Port::try_from(4999).unwrap()));
    /// assert!(!range.contains(Port::try_from(5011).unwrap()));
    /// ```
    #[must_use]
    pub const fn contains(&self, port: Port) -> bool {
        port.value() >= self.min.value() && port.value() <= self.max.value()
    }

    /// Returns the number of ports in the range (inclusive).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Port, PortRange};
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5010).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    /// assert_eq!(range.len(), 11);
    /// ```
    #[must_use]
    pub const fn len(&self) -> u16 {
        self.max.value() - self.min.value() + 1
    }

    /// Returns `true` if the range contains no ports.
    ///
    /// Note: This should never be true for a valid `PortRange` since we validate
    /// that max >= min, but the method is provided for completeness.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Returns an iterator over all ports in this range.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::{Port, PortRange};
    ///
    /// let min = Port::try_from(5000).unwrap();
    /// let max = Port::try_from(5002).unwrap();
    /// let range = PortRange::new(min, max).unwrap();
    ///
    /// let ports: Vec<Port> = range.iter().collect();
    /// assert_eq!(ports.len(), 3);
    /// assert_eq!(ports[0].value(), 5000);
    /// assert_eq!(ports[1].value(), 5001);
    /// assert_eq!(ports[2].value(), 5002);
    /// ```
    #[must_use]
    pub fn iter(self) -> PortRangeIter {
        PortRangeIter {
            range: self,
            current: u32::from(self.min.value()),
        }
    }
}

// Preserve the published range-only debug representation; the private
// exhaustion annotation is error context, not part of range identity.
#[allow(clippy::missing_fields_in_debug)]
impl fmt::Debug for PortRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PortRange")
            .field("min", &self.min)
            .field("max", &self.max)
            .finish()
    }
}

impl PartialEq for PortRange {
    fn eq(&self, other: &Self) -> bool {
        self.min == other.min && self.max == other.max
    }
}

impl Eq for PortRange {}

impl fmt::Display for PortRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.min, self.max)
    }
}

impl IntoIterator for PortRange {
    type Item = Port;
    type IntoIter = PortRangeIter;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over ports in a `PortRange`.
#[derive(Debug)]
pub struct PortRangeIter {
    range: PortRange,
    current: u32,
}

impl Iterator for PortRangeIter {
    type Item = Port;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current <= u32::from(self.range.max.value()) {
            let port =
                Port(u16::try_from(self.current).expect("iterator current cannot exceed u16::MAX"));
            self.current += 1;
            Some(port)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.current <= u32::from(self.range.max.value()) {
            let remaining = (u32::from(self.range.max.value()) - self.current + 1) as usize;
            (remaining, Some(remaining))
        } else {
            (0, Some(0))
        }
    }
}

impl ExactSizeIterator for PortRangeIter {
    fn len(&self) -> usize {
        self.size_hint().0
    }
}

/// Error type for invalid port ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPortRangeError {
    /// The minimum port.
    pub min: Port,
    /// The maximum port.
    pub max: Port,
    /// The reason the range is invalid.
    pub reason: String,
}

impl fmt::Display for InvalidPortRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid port range {}-{}: {}",
            self.min, self.max, self.reason
        )
    }
}

impl std::error::Error for InvalidPortRangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_validation() {
        // Port 0 is invalid
        assert!(Port::try_from(0).is_err());

        // Port 1 is valid (minimum)
        assert!(Port::try_from(1).is_ok());

        // Port 65535 is valid (maximum)
        assert!(Port::try_from(65535).is_ok());

        // Standard ports are valid
        assert!(Port::try_from(80).is_ok());
        assert!(Port::try_from(443).is_ok());
        assert!(Port::try_from(8080).is_ok());
    }

    #[test]
    fn test_port_invalid_error_message() {
        let err = Port::try_from(0).unwrap_err();
        assert_eq!(err.value, 0);
        assert!(err.reason.contains("invalid"));
    }

    #[test]
    fn test_port_value() {
        let port = Port::try_from(8080).unwrap();
        assert_eq!(port.value(), 8080);
    }

    #[test]
    fn test_port_is_privileged() {
        // Privileged ports (< 1024)
        assert!(Port::try_from(80).unwrap().is_privileged());
        assert!(Port::try_from(443).unwrap().is_privileged());
        assert!(Port::try_from(1023).unwrap().is_privileged());

        // Non-privileged ports
        assert!(!Port::try_from(1024).unwrap().is_privileged());
        assert!(!Port::try_from(8080).unwrap().is_privileged());
        assert!(!Port::try_from(65535).unwrap().is_privileged());
    }

    #[test]
    fn test_port_display() {
        let port = Port::try_from(8080).unwrap();
        assert_eq!(format!("{port}"), "8080");
    }

    #[test]
    fn test_port_ordering() {
        let p1 = Port::try_from(80).unwrap();
        let p2 = Port::try_from(443).unwrap();
        let p3 = Port::try_from(8080).unwrap();

        assert!(p1 < p2);
        assert!(p2 < p3);
        assert!(p1 < p3);
    }

    #[test]
    fn test_port_serde() {
        let port = Port::try_from(8080).unwrap();
        let json = serde_json::to_string(&port).unwrap();
        assert_eq!(json, "8080");

        let deserialized: Port = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, port);
    }

    #[test]
    fn test_port_range_creation() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        assert_eq!(range.min(), min);
        assert_eq!(range.max(), max);
    }

    #[test]
    fn test_port_range_invalid() {
        let min = Port::try_from(5010).unwrap();
        let max = Port::try_from(5000).unwrap();
        let result = PortRange::new(min, max);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.reason.contains("greater than or equal"));
    }

    #[test]
    fn test_port_range_single_port() {
        let port = Port::try_from(5000).unwrap();
        let range = PortRange::new(port, port).unwrap();

        assert_eq!(range.len(), 1);
        assert!(range.contains(port));
    }

    #[test]
    fn test_port_range_contains() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        // Ports in range
        assert!(range.contains(Port::try_from(5000).unwrap()));
        assert!(range.contains(Port::try_from(5005).unwrap()));
        assert!(range.contains(Port::try_from(5010).unwrap()));

        // Ports outside range
        assert!(!range.contains(Port::try_from(4999).unwrap()));
        assert!(!range.contains(Port::try_from(5011).unwrap()));
    }

    #[test]
    fn test_port_range_len() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        assert_eq!(range.len(), 11);
    }

    #[test]
    fn test_port_range_display() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        assert_eq!(format!("{range}"), "5000-5010");
    }

    #[test]
    fn test_port_range_iterator() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5002).unwrap();
        let range = PortRange::new(min, max).unwrap();

        let ports: Vec<Port> = range.iter().collect();
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0].value(), 5000);
        assert_eq!(ports[1].value(), 5001);
        assert_eq!(ports[2].value(), 5002);
    }

    #[test]
    fn test_port_range_iterator_exact_size() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5010).unwrap();
        let range = PortRange::new(min, max).unwrap();

        let mut iter = range.iter();
        assert_eq!(iter.len(), 11);

        iter.next();
        assert_eq!(iter.len(), 10);

        iter.next();
        assert_eq!(iter.len(), 9);
    }

    #[test]
    fn test_port_range_into_iter() {
        let min = Port::try_from(5000).unwrap();
        let max = Port::try_from(5002).unwrap();
        let range = PortRange::new(min, max).unwrap();

        let ports: Vec<Port> = range.into_iter().collect();
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn test_port_range_large() {
        let min = Port::try_from(1).unwrap();
        let max = Port::try_from(65535).unwrap();
        let range = PortRange::new(min, max).unwrap();

        assert_eq!(range.len(), 65535);
        assert!(range.contains(Port::try_from(32768).unwrap()));
    }

    #[test]
    fn test_port_range_iterator_includes_max_port_without_overflow() {
        let min = Port::try_from(65534).unwrap();
        let max = Port::try_from(65535).unwrap();
        let range = PortRange::new(min, max).unwrap();

        let ports: Vec<Port> = range.iter().collect();
        assert_eq!(ports, vec![min, max]);
    }

    #[test]
    fn test_port_checked_add_overflow() {
        // Test overflow with high port numbers
        let high = Port::try_from(65535).unwrap();
        assert!(high.checked_add(1).is_none());
        assert!(high.checked_add(100).is_none());

        // Test near-overflow that should work
        let near_max = Port::try_from(65534).unwrap();
        assert_eq!(near_max.checked_add(1).unwrap().value(), 65535);
        assert!(near_max.checked_add(2).is_none());
    }

    #[test]
    fn test_port_checked_sub_underflow() {
        // Test underflow with low port numbers
        let low = Port::try_from(1).unwrap();
        assert!(low.checked_sub(1).is_none());
        assert!(low.checked_sub(100).is_none());

        // Test near-underflow that should work
        let near_min = Port::try_from(2).unwrap();
        assert_eq!(near_min.checked_sub(1).unwrap().value(), 1);
        assert!(near_min.checked_sub(2).is_none());
    }
}
