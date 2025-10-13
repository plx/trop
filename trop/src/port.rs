//! Port and port range types for network port management.
//!
//! This module provides types for working with TCP/UDP ports, including
//! validation and range operations.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRange {
    min: Port,
    max: Port,
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
            Ok(Self { min, max })
        }
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
            current: self.min.value(),
        }
    }
}

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
    current: u16,
}

impl Iterator for PortRangeIter {
    type Item = Port;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current <= self.range.max.value() {
            let port = Port(self.current);
            self.current += 1;
            Some(port)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.current <= self.range.max.value() {
            let remaining = (self.range.max.value() - self.current + 1) as usize;
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
}
