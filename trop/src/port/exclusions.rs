//! Port exclusion management for filtering out unavailable ports.
//!
//! This module provides efficient checking and management of excluded port
//! lists, supporting both individual ports and ranges.

use std::collections::BTreeSet;

use crate::config::PortExclusion;
use crate::{Port, PortRange};

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
    /// Sorted set of all excluded ports.
    /// Using `BTreeSet` for efficient range queries and iteration.
    excluded: BTreeSet<Port>,
}

impl ExclusionManager {
    /// Create a new `ExclusionManager` from a configuration.
    ///
    /// # Performance Note
    ///
    /// For large port ranges (e.g., excluding 10,000+ ports), this method materializes
    /// all individual ports in memory. This is acceptable for typical exclusion lists
    /// but may use significant memory for very large ranges. Future optimizations could
    /// store ranges directly for better space efficiency.
    ///
    /// # Errors
    ///
    /// Returns an error if any port value is invalid (port 0).
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
        let mut excluded = BTreeSet::new();

        for exclusion in exclusions {
            match exclusion {
                PortExclusion::Single(port) => {
                    let port = Port::try_from(*port)?;
                    excluded.insert(port);
                }
                PortExclusion::Range { start, end } => {
                    // Validate ports
                    let start_port = Port::try_from(*start)?;
                    let end_port = Port::try_from(*end)?;

                    // Create range and add all ports
                    let range = PortRange::new(start_port, end_port)?;
                    for port in range {
                        excluded.insert(port);
                    }
                }
            }
        }

        Ok(Self { excluded })
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
            excluded: BTreeSet::new(),
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
        self.excluded.contains(&port)
    }

    /// Get all excluded ports within a given range.
    ///
    /// Returns a vector of excluded ports that fall within the specified range.
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
        self.excluded
            .range(range.min()..=range.max())
            .copied()
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
        self.excluded.insert(port);
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
        for port in *range {
            self.excluded.insert(port);
        }
    }

    /// Add an exclusion from configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if any port value is invalid.
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
        match *exclusion {
            PortExclusion::Single(port) => {
                let port = Port::try_from(port)?;
                self.excluded.insert(port);
            }
            PortExclusion::Range { start, end } => {
                let start_port = Port::try_from(start)?;
                let end_port = Port::try_from(end)?;
                let range = PortRange::new(start_port, end_port)?;
                self.add_range(&range);
            }
        }
        Ok(())
    }

    /// Get a compacted representation of exclusions.
    ///
    /// This returns a minimal set of `PortExclusion` entries by merging
    /// adjacent and overlapping ports into ranges.
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
        if self.excluded.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut ports: Vec<Port> = self.excluded.iter().copied().collect();
        ports.sort_by_key(|p| p.value());

        let mut range_start = ports[0];
        let mut range_end = ports[0];

        for &port in &ports[1..] {
            if port.value() == range_end.value() + 1 {
                // Extend the current range
                range_end = port;
            } else {
                // Emit the current range and start a new one
                if range_start == range_end {
                    result.push(PortExclusion::Single(range_start.value()));
                } else {
                    result.push(PortExclusion::Range {
                        start: range_start.value(),
                        end: range_end.value(),
                    });
                }
                range_start = port;
                range_end = port;
            }
        }

        // Emit the final range
        if range_start == range_end {
            result.push(PortExclusion::Single(range_start.value()));
        } else {
            result.push(PortExclusion::Range {
                start: range_start.value(),
                end: range_end.value(),
            });
        }

        result
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
        self.excluded.iter().copied()
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
}
