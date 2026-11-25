//! Port exclusion management for filtering out unavailable ports.
//!
//! This module provides efficient checking and management of excluded port
//! lists, supporting both individual ports and ranges.
//!
//! The decisions are made by the verified `verified::exclusions` kernel:
//! entry validation (with the configuration validator's error precedence),
//! the excluded-port set (stored as its canonical run list), membership,
//! insertion, range queries, counting, and compaction. The code here only
//! converts between [`PortExclusion`] / [`Port`] and the kernel's primitive
//! values.

use super::{Port, PortRange};
use crate::config::PortExclusion;
use crate::error::Error;
use crate::verified::exclusions::{
    compact_exclusions as compact_entries, validate_entries, ExclusionEntry, ExclusionError,
    ExclusionSet,
};

/// Converts a configuration entry to the kernel's field-for-field mirror.
const fn to_entry(exclusion: &PortExclusion) -> ExclusionEntry {
    match *exclusion {
        PortExclusion::Single(port) => ExclusionEntry::Single(port),
        PortExclusion::Range { start, end } => ExclusionEntry::Range { start, end },
    }
}

/// Converts a kernel entry back to a configuration entry.
const fn from_entry(entry: ExclusionEntry) -> PortExclusion {
    match entry {
        ExclusionEntry::Single(port) => PortExclusion::Single(port),
        ExclusionEntry::Range { start, end } => PortExclusion::Range { start, end },
    }
}

fn to_entries(exclusions: &[PortExclusion]) -> Vec<ExclusionEntry> {
    exclusions.iter().map(to_entry).collect()
}

/// The configuration validator's message for an invalid entry.
fn validation_message(error: ExclusionError) -> String {
    match error {
        ExclusionError::ZeroPort => "Invalid port: 0".into(),
        ExclusionError::ZeroStart => "Invalid start port: 0".into(),
        ExclusionError::ZeroEnd => "Invalid end port: 0".into(),
        ExclusionError::Reversed { start, end } => {
            format!("Invalid range: {start}..{end} (end < start)")
        }
    }
}

/// The configuration validator's error for the entry at `index`.
fn validation_error(index: usize, error: ExclusionError) -> Error {
    Error::Validation {
        field: format!("excluded_ports[{index}]"),
        message: validation_message(error),
    }
}

/// The error [`ExclusionManager`] reports for an invalid entry: the same
/// errors `Port::try_from` (port 0) and `PortRange::new` (reversed range)
/// report.
fn manager_error(error: ExclusionError) -> Error {
    match error {
        ExclusionError::ZeroPort | ExclusionError::ZeroStart | ExclusionError::ZeroEnd => {
            Error::InvalidPort {
                value: 0,
                reason: "port 0 is invalid".into(),
            }
        }
        ExclusionError::Reversed { start, end } => Error::InvalidPortRange {
            min: start,
            max: end,
            reason: "max must be greater than or equal to min".into(),
        },
    }
}

/// Validates an `excluded_ports` list.
///
/// Every entry must be a valid port (`1..=65535`) or a valid inclusive range
/// (`1 <= start <= end`; `5000..5000` is valid). Reports the first invalid
/// entry; for a range a zero start is reported before a zero end, which is
/// reported before a reversed range.
///
/// # Errors
///
/// Returns [`Error::Validation`] for field `excluded_ports[i]`, where `i` is
/// the index of the first invalid entry.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::validate_exclusions;
///
/// assert!(validate_exclusions(&[PortExclusion::Range { start: 5000, end: 5000 }]).is_ok());
///
/// let err = validate_exclusions(&[
///     PortExclusion::Single(5001),
///     PortExclusion::Range { start: 5010, end: 5000 },
/// ])
/// .unwrap_err();
/// assert!(err.to_string().contains("excluded_ports[1]"));
/// ```
pub fn validate_exclusions(exclusions: &[PortExclusion]) -> crate::Result<()> {
    validate_entries(&to_entries(exclusions))
        .map_err(|(index, error)| validation_error(index, error))
}

/// Compacts an exclusion list to its minimal representation.
///
/// The result excludes exactly the same ports as `exclusions`. It is sorted,
/// merges overlapping and adjacent entries into maximal ranges, and uses
/// `Single` exactly for one-port runs. No exclusion list for the same ports
/// has fewer entries, and the result is the only list with these properties,
/// so it depends only on the excluded ports (not on entry order, duplicates,
/// or overlaps), and compacting it again returns it unchanged. These
/// properties are verified by the `verified::exclusions` kernel.
///
/// # Errors
///
/// Returns the same [`Error::Validation`] as [`validate_exclusions`] if any
/// entry is invalid: invalid entries are never dropped or changed.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::compact_exclusions;
///
/// let compacted = compact_exclusions(&[
///     PortExclusion::Single(5003),
///     PortExclusion::Range { start: 5000, end: 5002 },
///     PortExclusion::Single(6000),
/// ])
/// .unwrap();
/// assert_eq!(
///     compacted,
///     vec![
///         PortExclusion::Range { start: 5000, end: 5003 },
///         PortExclusion::Single(6000),
///     ]
/// );
///
/// assert!(compact_exclusions(&[PortExclusion::Range { start: 5010, end: 5000 }]).is_err());
/// ```
pub fn compact_exclusions(exclusions: &[PortExclusion]) -> crate::Result<Vec<PortExclusion>> {
    compact_entries(&to_entries(exclusions))
        .map(|runs| runs.into_iter().map(from_entry).collect())
        .map_err(|(index, error)| validation_error(index, error))
}

/// Manages excluded port lists with efficient checking.
///
/// The `ExclusionManager` stores ports and port ranges that should not be
/// allocated. It provides efficient checking using sorted data structures
/// and supports compaction to merge overlapping ranges.
///
/// # Examples
///
/// ```
/// use trop::port::exclusions::ExclusionManager;
/// use trop::config::PortExclusion;
/// use trop::Port;
///
/// let exclusions = vec![
///     PortExclusion::Single(5001),
///     PortExclusion::Range { start: 5005, end: 5009 },
/// ];
///
/// let manager = ExclusionManager::from_config(&exclusions).unwrap();
///
/// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
/// assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
/// assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
/// ```
#[derive(Debug, Clone)]
pub struct ExclusionManager {
    /// The excluded ports, stored as their canonical (sorted, disjoint,
    /// non-adjacent) run list by the verified `ExclusionSet` kernel type.
    excluded: ExclusionSet,
}

impl ExclusionManager {
    /// Create a new `ExclusionManager` from a configuration.
    ///
    /// The excluded ports are stored as sorted, merged ranges, so memory use
    /// is proportional to the number of distinct ranges, not ports.
    ///
    /// # Errors
    ///
    /// Returns an error for the first invalid entry: a port of 0
    /// ([`Error::InvalidPort`]) or a range whose end precedes its start
    /// ([`Error::InvalidPortRange`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Range { start: 5005, end: 5009 },
    /// ];
    ///
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    /// ```
    pub fn from_config(exclusions: &[PortExclusion]) -> crate::Result<Self> {
        ExclusionSet::from_entries(&to_entries(exclusions))
            .map(|excluded| Self { excluded })
            .map_err(|(_, error)| manager_error(error))
    }

    /// Create an empty exclusion manager.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let manager = ExclusionManager::empty();
    /// assert!(!manager.is_excluded(Port::try_from(8080).unwrap()));
    /// ```
    #[must_use]
    pub fn empty() -> Self {
        Self {
            excluded: ExclusionSet::empty(),
        }
    }

    /// Check if a specific port is excluded.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    /// use trop::Port;
    ///
    /// let exclusions = vec![PortExclusion::Single(5001)];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
    /// assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
    /// ```
    #[must_use]
    pub fn is_excluded(&self, port: Port) -> bool {
        self.excluded.contains(port.value())
    }

    /// Get all excluded ports within a given range.
    ///
    /// Returns a vector of excluded ports that fall within the specified
    /// range (inclusive on both ends), in ascending order.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    /// use trop::{Port, PortRange};
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Single(5005),
    ///     PortExclusion::Single(5020),
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(5010).unwrap(),
    /// ).unwrap();
    ///
    /// let excluded = manager.excluded_in_range(&range);
    /// assert_eq!(excluded.len(), 2); // 5001 and 5005, not 5020
    /// ```
    #[must_use]
    pub fn excluded_in_range(&self, range: &PortRange) -> Vec<Port> {
        // The kernel guarantees every member is a valid port (never 0).
        self.excluded
            .members_between(range.min().value(), range.max().value())
            .into_iter()
            .map(Port)
            .collect()
    }

    /// Add a single port to the exclusion set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_port(Port::try_from(8080).unwrap());
    ///
    /// assert!(manager.is_excluded(Port::try_from(8080).unwrap()));
    /// ```
    pub fn add_port(&mut self, port: Port) {
        // The kernel rejects only port 0, which no `Port` holds.
        let inserted = self.excluded.insert_port(port.value());
        debug_assert!(inserted, "a Port is never 0");
    }

    /// Add a range of ports to the exclusion set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::{Port, PortRange};
    ///
    /// let mut manager = ExclusionManager::empty();
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(5002).unwrap(),
    /// ).unwrap();
    ///
    /// manager.add_range(&range);
    ///
    /// assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
    /// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
    /// assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
    /// assert!(!manager.is_excluded(Port::try_from(5003).unwrap()));
    /// ```
    pub fn add_range(&mut self, range: &PortRange) {
        self.excluded.insert_span(range.span);
    }

    /// Add an exclusion from configuration.
    ///
    /// On error the exclusion set is unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is invalid: a port of 0
    /// ([`Error::InvalidPort`]) or a range whose end precedes its start
    /// ([`Error::InvalidPortRange`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_exclusion(&PortExclusion::Single(8080)).unwrap();
    /// ```
    pub fn add_exclusion(&mut self, exclusion: &PortExclusion) -> crate::Result<()> {
        self.excluded
            .insert_entry(to_entry(exclusion))
            .map_err(manager_error)
    }

    /// Get a compacted representation of exclusions.
    ///
    /// This returns the minimal list of `PortExclusion` entries for the
    /// excluded ports: sorted, with adjacent and overlapping ports merged
    /// into ranges, and `Single` exactly for one-port runs (see
    /// [`compact_exclusions`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_port(Port::try_from(5000).unwrap());
    /// manager.add_port(Port::try_from(5001).unwrap());
    /// manager.add_port(Port::try_from(5002).unwrap());
    /// manager.add_port(Port::try_from(5010).unwrap());
    ///
    /// let compacted = manager.compact();
    /// // Should produce: Range(5000..5002) and Single(5010)
    /// assert_eq!(compacted.len(), 2);
    /// ```
    #[must_use]
    pub fn compact(&self) -> Vec<PortExclusion> {
        self.excluded.runs().into_iter().map(from_entry).collect()
    }

    /// Get the total number of excluded ports.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Range { start: 5005, end: 5009 },
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// assert_eq!(manager.len(), 6); // 5001 plus 5005-5009 (5 ports)
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.excluded.len()
    }

    /// Check if there are no exclusions.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    ///
    /// let manager = ExclusionManager::empty();
    /// assert!(manager.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.excluded.is_empty()
    }

    /// Get an iterator over all excluded ports in sorted order.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Single(5003),
    ///     PortExclusion::Single(5002),
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// let mut iter = manager.iter();
    /// assert_eq!(iter.next().unwrap().value(), 5001);
    /// assert_eq!(iter.next().unwrap().value(), 5002);
    /// assert_eq!(iter.next().unwrap().value(), 5003);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = Port> + '_ {
        // The kernel guarantees every member is a valid port (never 0).
        self.excluded
            .members_between(Port::MIN, Port::MAX)
            .into_iter()
            .map(Port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_manager() {
        let manager = ExclusionManager::empty();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert!(!manager.is_excluded(Port::try_from(8080).unwrap()));
    }

    #[test]
    fn test_from_config_single_ports() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5009),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5009).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_from_config_range() {
        let exclusions = vec![PortExclusion::Range {
            start: 5000,
            end: 5002,
        }];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5003).unwrap()));
    }

    #[test]
    fn test_from_config_mixed() {
        let exclusions = vec![
            PortExclusion::Single(5000),
            PortExclusion::Range {
                start: 5005,
                end: 5009,
            },
            PortExclusion::Single(5020),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 7); // 1 + 5 + 1
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5009).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5020).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5010).unwrap()));
    }

    #[test]
    fn test_excluded_in_range() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5020),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 2);
        assert!(excluded.contains(&Port::try_from(5001).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
        assert!(!excluded.contains(&Port::try_from(5020).unwrap()));
    }

    #[test]
    fn test_add_port() {
        let mut manager = ExclusionManager::empty();
        let port = Port::try_from(8080).unwrap();

        assert!(!manager.is_excluded(port));
        manager.add_port(port);
        assert!(manager.is_excluded(port));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_add_range() {
        let mut manager = ExclusionManager::empty();
        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5002).unwrap()).unwrap();

        manager.add_range(&range);

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_compact_single_ports() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5009),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);
    }

    #[test]
    fn test_compact_adjacent_ports() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5001).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 1);
        assert_eq!(
            compacted[0],
            PortExclusion::Range {
                start: 5000,
                end: 5002
            }
        );
    }

    #[test]
    fn test_compact_range_ending_at_max_port() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(65534).unwrap());
        manager.add_port(Port::try_from(65535).unwrap());

        let compacted = manager.compact();
        assert_eq!(
            compacted,
            vec![PortExclusion::Range {
                start: 65534,
                end: 65535,
            }]
        );
    }

    #[test]
    fn test_compact_mixed() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5001).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());
        manager.add_port(Port::try_from(5010).unwrap());
        manager.add_port(Port::try_from(5020).unwrap());
        manager.add_port(Port::try_from(5021).unwrap());

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);

        // Should have: Range(5000..5002), Single(5010), Range(5020..5021)
        assert!(compacted.contains(&PortExclusion::Range {
            start: 5000,
            end: 5002
        }));
        assert!(compacted.contains(&PortExclusion::Single(5010)));
        assert!(compacted.contains(&PortExclusion::Range {
            start: 5020,
            end: 5021
        }));
    }

    #[test]
    fn test_iter() {
        let exclusions = vec![
            PortExclusion::Single(5003),
            PortExclusion::Single(5001),
            PortExclusion::Single(5002),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let ports: Vec<u16> = manager.iter().map(Port::value).collect();
        assert_eq!(ports, vec![5001, 5002, 5003]); // Should be sorted
    }

    #[test]
    fn test_overlapping_exclusions() {
        // Test that overlapping ranges are correctly merged into the exclusion set
        // This verifies BTreeSet properly handles duplicates
        let exclusions = vec![
            PortExclusion::Range {
                start: 5000,
                end: 5005,
            },
            PortExclusion::Range {
                start: 5003,
                end: 5008,
            },
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        // Should have ports 5000-5008 (9 ports total)
        assert_eq!(manager.len(), 9);
        for port in 5000..=5008 {
            assert!(manager.is_excluded(Port::try_from(port).unwrap()));
        }
    }

    #[test]
    fn test_compaction_single_port_at_boundary() {
        // Test compaction when a single port exists at range boundaries
        // Ensures proper handling of single-element ranges that shouldn't be compacted
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5005).unwrap()); // Gap, shouldn't merge
        manager.add_port(Port::try_from(5010).unwrap()); // Gap, shouldn't merge

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);
        assert!(compacted.contains(&PortExclusion::Single(5000)));
        assert!(compacted.contains(&PortExclusion::Single(5005)));
        assert!(compacted.contains(&PortExclusion::Single(5010)));
    }

    #[test]
    fn test_compaction_large_continuous_range() {
        // Test compaction on a large continuous range to verify efficiency
        // This ensures the compaction algorithm scales properly
        let mut manager = ExclusionManager::empty();
        for port in 5000..=5100 {
            manager.add_port(Port::try_from(port).unwrap());
        }

        let compacted = manager.compact();
        // Should compact to a single range
        assert_eq!(compacted.len(), 1);
        assert_eq!(
            compacted[0],
            PortExclusion::Range {
                start: 5000,
                end: 5100
            }
        );
    }

    #[test]
    fn test_compaction_alternating_pattern() {
        // Test compaction with alternating excluded/available ports
        // Verifies correct handling of non-consecutive patterns
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());
        manager.add_port(Port::try_from(5004).unwrap());
        manager.add_port(Port::try_from(5006).unwrap());

        let compacted = manager.compact();
        // All should be single ports (no consecutive pairs)
        assert_eq!(compacted.len(), 4);
        for exclusion in compacted {
            assert!(matches!(exclusion, PortExclusion::Single(_)));
        }
    }

    #[test]
    fn test_excluded_in_range_partial_overlap() {
        // Test range query when exclusions partially overlap the query range
        // Verifies correct boundary handling in range queries
        let exclusions = vec![
            PortExclusion::Single(4995), // Before range
            PortExclusion::Single(5001), // In range
            PortExclusion::Single(5005), // In range
            PortExclusion::Single(6000), // After range
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 2);
        assert!(excluded.contains(&Port::try_from(5001).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
    }

    #[test]
    fn test_excluded_in_range_exact_boundaries() {
        // Test range query with exclusions exactly at range boundaries
        // Ensures inclusive boundary semantics work correctly
        let exclusions = vec![
            PortExclusion::Single(5000), // Exactly at min
            PortExclusion::Single(5005), // Middle
            PortExclusion::Single(5010), // Exactly at max
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 3);
        assert!(excluded.contains(&Port::try_from(5000).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
        assert!(excluded.contains(&Port::try_from(5010).unwrap()));
    }

    #[test]
    fn test_add_exclusion_error_handling() {
        // Test that add_exclusion properly handles invalid port values
        // Verifies error propagation from port validation
        let mut manager = ExclusionManager::empty();

        // Port 0 is invalid and should cause an error
        let invalid_exclusion = PortExclusion::Single(0);
        let result = manager.add_exclusion(&invalid_exclusion);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_config_invalid_range() {
        // Test that from_config properly validates range endpoints
        // Ensures invalid ranges are rejected during construction
        let exclusions = vec![PortExclusion::Range {
            start: 5010,
            end: 5000, // Invalid: max < min
        }];

        let result = ExclusionManager::from_config(&exclusions);
        assert!(result.is_err());
    }

    #[test]
    fn test_exclusion_manager_boundary_ports() {
        // Test exclusion manager with port number boundaries
        // Verifies correct handling of extreme valid port values
        let exclusions = vec![
            PortExclusion::Single(1),     // Minimum valid port
            PortExclusion::Single(65535), // Maximum valid port
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 2);
        assert!(manager.is_excluded(Port::try_from(1).unwrap()));
        assert!(manager.is_excluded(Port::try_from(65535).unwrap()));
    }

    #[test]
    fn test_compaction_preserves_coverage() {
        // Test that compaction doesn't change which ports are excluded
        // This is an invariant test: compaction should only change representation
        let mut manager = ExclusionManager::empty();
        for port in 5000..=5002 {
            manager.add_port(Port::try_from(port).unwrap());
        }
        manager.add_port(Port::try_from(5005).unwrap());
        for port in 5010..=5012 {
            manager.add_port(Port::try_from(port).unwrap());
        }

        // Save pre-compaction state
        let original_exclusions: Vec<Port> = manager.iter().collect();

        // Compact and reconstruct
        let compacted = manager.compact();
        let new_manager = ExclusionManager::from_config(&compacted).unwrap();

        // Verify same ports are excluded
        let new_exclusions: Vec<Port> = new_manager.iter().collect();
        assert_eq!(original_exclusions, new_exclusions);
    }

    #[test]
    fn test_add_range_large_range() {
        // Test adding a large range to verify performance characteristics
        // Ensures the implementation can handle large exclusion lists
        let mut manager = ExclusionManager::empty();
        let range = PortRange::new(
            Port::try_from(10000).unwrap(),
            Port::try_from(20000).unwrap(),
        )
        .unwrap();

        manager.add_range(&range);

        // Should have 10,001 ports (10000-20000 inclusive)
        assert_eq!(manager.len(), 10001);
        assert!(manager.is_excluded(Port::try_from(10000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(15000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(20000).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(20001).unwrap()));
    }

    // ------------------------------------------------------------------
    // Glue tests for the verified kernel (boundaries, error mapping, and
    // agreement with the previous `BTreeSet` implementation).
    // ------------------------------------------------------------------

    fn single(port: u16) -> PortExclusion {
        PortExclusion::Single(port)
    }

    fn range(start: u16, end: u16) -> PortExclusion {
        PortExclusion::Range { start, end }
    }

    /// The previous implementation: materialize every port, then sweep the
    /// sorted set into maximal runs. Valid input only.
    fn reference_ports(exclusions: &[PortExclusion]) -> std::collections::BTreeSet<u16> {
        let mut ports = std::collections::BTreeSet::new();
        for exclusion in exclusions {
            match *exclusion {
                PortExclusion::Single(port) => {
                    ports.insert(port);
                }
                PortExclusion::Range { start, end } => ports.extend(start..=end),
            }
        }
        ports
    }

    fn reference_compact(exclusions: &[PortExclusion]) -> Vec<PortExclusion> {
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for port in reference_ports(exclusions) {
            match runs.last_mut() {
                Some((_, end)) if u32::from(*end) + 1 == u32::from(port) => *end = port,
                _ => runs.push((port, port)),
            }
        }
        runs.into_iter()
            .map(|(start, end)| {
                if start == end {
                    single(start)
                } else {
                    range(start, end)
                }
            })
            .collect()
    }

    /// Deterministic pseudo-random valid exclusion lists, biased towards
    /// overlaps, adjacency, duplicates, and the ends of the port space.
    fn sample_lists() -> Vec<Vec<PortExclusion>> {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        let mut lists = Vec::new();
        for case in 0..400 {
            let base: u16 = match case % 4 {
                0 => 1,
                1 => 65_500,
                2 => 5000,
                _ => u16::try_from(1 + next(65_000)).unwrap(),
            };
            let len = next(8);
            let mut list = Vec::new();
            for _ in 0..len {
                let start = base.saturating_add(u16::try_from(next(40)).unwrap()).max(1);
                let width = u16::try_from(next(6)).unwrap();
                let end = start.saturating_add(width);
                if next(3) == 0 {
                    list.push(single(start));
                } else {
                    list.push(range(start, end));
                }
            }
            lists.push(list);
        }
        lists
    }

    #[test]
    fn test_kernel_agrees_with_previous_implementation() {
        for list in sample_lists() {
            let manager = ExclusionManager::from_config(&list).unwrap();
            let expected_ports = reference_ports(&list);
            let actual_ports: Vec<u16> = manager.iter().map(Port::value).collect();
            assert_eq!(
                actual_ports,
                expected_ports.iter().copied().collect::<Vec<_>>(),
                "{list:?}"
            );
            assert_eq!(manager.len(), expected_ports.len(), "{list:?}");
            assert_eq!(manager.is_empty(), expected_ports.is_empty(), "{list:?}");
            assert_eq!(manager.compact(), reference_compact(&list), "{list:?}");
            assert_eq!(
                compact_exclusions(&list).unwrap(),
                reference_compact(&list),
                "{list:?}"
            );
            for port in [1, 2, 4999, 5000, 5001, 5039, 5045, 65_500, 65_534, 65_535] {
                let port = Port::try_from(port).unwrap();
                assert_eq!(
                    manager.is_excluded(port),
                    expected_ports.contains(&port.value()),
                    "{list:?} {port}"
                );
            }
            let query = PortRange::new(
                Port::try_from(5010).unwrap(),
                Port::try_from(65_510).unwrap(),
            )
            .unwrap();
            let in_range: Vec<u16> = manager
                .excluded_in_range(&query)
                .into_iter()
                .map(Port::value)
                .collect();
            let expected: Vec<u16> = expected_ports.range(5010..=65_510).copied().collect();
            assert_eq!(in_range, expected, "{list:?}");
        }
    }

    #[test]
    fn test_incremental_inserts_agree_with_from_config() {
        for list in sample_lists() {
            let mut manager = ExclusionManager::empty();
            for exclusion in &list {
                manager.add_exclusion(exclusion).unwrap();
            }
            assert_eq!(manager.compact(), reference_compact(&list), "{list:?}");
        }
    }

    #[test]
    fn test_full_port_space_boundaries() {
        let manager = ExclusionManager::from_config(&[range(1, 65_535)]).unwrap();
        assert_eq!(manager.len(), 65_535);
        assert_eq!(manager.iter().next().map(Port::value), Some(1));
        assert_eq!(manager.iter().last().map(Port::value), Some(65_535));
        assert_eq!(manager.iter().count(), 65_535);
        assert_eq!(manager.compact(), vec![range(1, 65_535)]);

        let edges = ExclusionManager::from_config(&[single(65_535), single(1)]).unwrap();
        assert_eq!(edges.compact(), vec![single(1), single(65_535)]);
        let all =
            PortRange::new(Port::try_from(1).unwrap(), Port::try_from(65_535).unwrap()).unwrap();
        let members: Vec<u16> = edges
            .excluded_in_range(&all)
            .into_iter()
            .map(Port::value)
            .collect();
        assert_eq!(members, vec![1, 65_535]);
        let single_port = PortRange::new(
            Port::try_from(65_535).unwrap(),
            Port::try_from(65_535).unwrap(),
        )
        .unwrap();
        assert_eq!(
            edges.excluded_in_range(&single_port),
            vec![Port::try_from(65_535).unwrap()]
        );

        // A one-port range compacts to a single port.
        assert_eq!(
            compact_exclusions(&[range(5000, 5000)]).unwrap(),
            [single(5000)]
        );
        assert_eq!(
            compact_exclusions(&[]).unwrap(),
            Vec::<PortExclusion>::new()
        );
    }

    #[test]
    fn test_from_config_errors_match_port_and_range_constructors() {
        // The kernel's error is mapped back to exactly the errors the
        // previous `Port::try_from` / `PortRange::new` calls produced.
        let zero = Error::from(Port::try_from(0).unwrap_err()).to_string();
        for invalid in [single(0), range(0, 5000), range(5000, 0), range(0, 0)] {
            let err = ExclusionManager::from_config(&[single(80), invalid.clone()]).unwrap_err();
            assert!(
                matches!(err, Error::InvalidPort { value: 0, .. }),
                "{invalid:?}: {err:?}"
            );
            assert_eq!(err.to_string(), zero, "{invalid:?}");
        }

        let reversed = Error::from(
            PortRange::new(Port::try_from(5010).unwrap(), Port::try_from(5000).unwrap())
                .unwrap_err(),
        )
        .to_string();
        let err = ExclusionManager::from_config(&[range(5010, 5000)]).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidPortRange {
                min: 5010,
                max: 5000,
                ..
            }
        ));
        assert_eq!(err.to_string(), reversed);
    }

    #[test]
    fn test_add_exclusion_is_atomic_on_error() {
        let mut manager = ExclusionManager::from_config(&[range(5000, 5002)]).unwrap();
        for invalid in [single(0), range(5010, 5000), range(0, 5005), range(5003, 0)] {
            assert!(manager.add_exclusion(&invalid).is_err(), "{invalid:?}");
            assert_eq!(manager.compact(), vec![range(5000, 5002)], "{invalid:?}");
            assert_eq!(manager.len(), 3);
        }
        // Valid insertions merge with adjacent runs.
        manager.add_exclusion(&single(5003)).unwrap();
        manager.add_exclusion(&range(4998, 4999)).unwrap();
        assert_eq!(manager.compact(), vec![range(4998, 5003)]);
    }

    #[test]
    fn test_validate_exclusions_reports_first_invalid_entry_with_precedence() {
        let message = |exclusions: &[PortExclusion]| match validate_exclusions(exclusions) {
            Err(Error::Validation { field, message }) => format!("{field}: {message}"),
            other => panic!("expected a validation error, got {other:?}"),
        };
        assert!(validate_exclusions(&[]).is_ok());
        assert!(validate_exclusions(&[single(1), single(65_535), range(1, 65_535)]).is_ok());
        assert!(validate_exclusions(&[range(5000, 5000)]).is_ok());
        assert_eq!(message(&[single(0)]), "excluded_ports[0]: Invalid port: 0");
        assert_eq!(
            message(&[single(1), range(0, 0)]),
            "excluded_ports[1]: Invalid start port: 0"
        );
        assert_eq!(
            message(&[range(0, 5)]),
            "excluded_ports[0]: Invalid start port: 0"
        );
        assert_eq!(
            message(&[range(5, 0)]),
            "excluded_ports[0]: Invalid end port: 0"
        );
        assert_eq!(
            message(&[single(5), range(5010, 5000), single(0)]),
            "excluded_ports[1]: Invalid range: 5010..5000 (end < start)"
        );
    }

    /// Regression: compaction used to drop reversed ranges (changing the
    /// excluded set) and keep port 0. It now rejects such lists.
    #[test]
    fn test_compact_exclusions_rejects_invalid_entries() {
        let err = compact_exclusions(&[range(5010, 5000), single(6000)]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation error for 'excluded_ports[0]': Invalid range: 5010..5000 (end < start)"
        );
        assert!(compact_exclusions(&[single(0), single(1)]).is_err());
        assert!(compact_exclusions(&[single(6000), range(0, 3)]).is_err());
    }

    #[test]
    fn test_compaction_is_order_independent_and_idempotent() {
        let list = vec![
            single(5005),
            range(5000, 5003),
            single(5004),
            range(5001, 5002),
            single(7000),
            single(7000),
        ];
        let mut reversed = list.clone();
        reversed.reverse();
        let compacted = compact_exclusions(&list).unwrap();
        assert_eq!(compacted, vec![range(5000, 5005), single(7000)]);
        assert_eq!(compact_exclusions(&reversed).unwrap(), compacted);
        assert_eq!(compact_exclusions(&compacted).unwrap(), compacted);
    }
}
