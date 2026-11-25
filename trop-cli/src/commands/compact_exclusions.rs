//! Command to compact exclusion list to minimal representation.

use crate::commands::exclude::write_exclusions;
use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::Args;
use std::path::PathBuf;
use trop::config::PortExclusion;
use trop::port::exclusions::{compact_exclusions, excluded_ports_in, is_compact};

/// Compact exclusion list to minimal representation.
#[derive(Args)]
pub struct CompactExclusionsCommand {
    /// Configuration file path
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Dry run (show changes without applying)
    #[arg(long)]
    pub dry_run: bool,
}

impl CompactExclusionsCommand {
    pub fn execute(self, _global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration
        if !self.path.exists() {
            return Err(CliError::InvalidArguments(format!(
                "File not found: {}",
                self.path.display()
            )));
        }

        let document = std::fs::read_to_string(&self.path)?;
        let Some(exclusions) = excluded_ports_in(&document)
            .map_err(|e| CliError::Config(format!("Parse error: {e}")))?
        else {
            println!("No exclusions to compact");
            return Ok(());
        };

        // 2. Compact exclusions. Invalid entries (port 0, reversed ranges)
        //    are reported, never dropped: compaction must not change the set
        //    of excluded ports.
        let compacted =
            compact_exclusions(&exclusions).map_err(|e| CliError::Config(e.to_string()))?;

        // The list is rewritten exactly when it is not already in canonical
        // form (`is_compact` holds exactly when compaction returns the list
        // unchanged), even when compaction keeps the number of entries.
        if is_compact(&exclusions) {
            println!("Exclusions already optimal");
            return Ok(());
        }
        println!(
            "Compacted {} exclusions to {}",
            exclusions.len(),
            compacted.len()
        );

        if self.dry_run {
            println!("Dry run - no changes made");
            println!("Would save: {compacted:?}");
        } else {
            // 3. Rewrite only `excluded_ports` (YAML comments will be lost)
            write_exclusions(&self.path, &document, &compacted)?;
            println!("Updated {}", self.path.display());
        }

        Ok(())
    }
}

/// Compact a list of port exclusions to minimal representation.
///
/// Delegates to the verified compaction (`trop::port::exclusions::compact_exclusions`):
/// the result excludes exactly the same ports, is sorted with overlapping and
/// adjacent entries merged, and no shorter list excludes the same ports.
///
/// A list with an invalid entry (port 0 or a reversed range) has no
/// compaction; it is returned unchanged rather than altered, so the excluded
/// set never changes. Callers that must report invalid entries use
/// `compact_exclusions` directly.
pub fn compact_exclusion_list(exclusions: &[PortExclusion]) -> Vec<PortExclusion> {
    compact_exclusions(exclusions).unwrap_or_else(|_| exclusions.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test compaction of empty list.
    ///
    /// **Edge case**: Empty input should produce empty output.
    /// **Invariant**: compact([]) = []
    #[test]
    fn test_compact_empty_list() {
        let exclusions: Vec<PortExclusion> = vec![];
        let compacted = compact_exclusion_list(&exclusions);
        assert_eq!(compacted.len(), 0, "Empty list should remain empty");
    }

    /// Test compaction of single port.
    ///
    /// **Trivial case**: A single port should remain as a single exclusion.
    /// **Invariant**: compact([Single(p)]) = [Single(p)]
    #[test]
    fn test_compact_single_port() {
        let exclusions = vec![PortExclusion::Single(8080)];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        assert_eq!(compacted[0], PortExclusion::Single(8080));
    }

    /// Test compaction of two adjacent ports into a range.
    ///
    /// **Basic range formation**: [8080, 8081] → Range{8080..8081}
    /// **Property**: Adjacent ports (p, p+1) form a range
    /// **Why test this**: This is the minimal case for range formation
    #[test]
    fn test_compact_two_adjacent_ports() {
        let exclusions = vec![PortExclusion::Single(8080), PortExclusion::Single(8081)];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8081);
            }
            _ => panic!("Expected range, got single"),
        }
    }

    /// Test compaction of non-adjacent ports remains as singles.
    ///
    /// **Property**: Non-adjacent ports cannot be merged
    /// **Example**: [8080, 9000] → [8080, 9000] (no change)
    /// **Why test this**: Ensures we don't incorrectly merge distant ports
    #[test]
    fn test_compact_non_adjacent_ports_stay_separate() {
        let exclusions = vec![PortExclusion::Single(8080), PortExclusion::Single(9000)];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0], PortExclusion::Single(8080));
        assert_eq!(compacted[1], PortExclusion::Single(9000));
    }

    /// Test compaction of consecutive sequence into single range.
    ///
    /// **Core algorithm test**: [8080, 8081, 8082, 8083, 8084] → Range{8080..8084}
    /// **Invariant**: All ports in sequence are represented in output
    /// **Why test this**: This is the primary optimization case
    #[test]
    fn test_compact_consecutive_sequence() {
        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
            PortExclusion::Single(8082),
            PortExclusion::Single(8083),
            PortExclusion::Single(8084),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8084);
            }
            _ => panic!("Expected range"),
        }
    }

    /// Test compaction with gaps creates multiple ranges/singles.
    ///
    /// **Property**: Gaps prevent merging
    /// **Example**: [8080, 8081, 8085, 8086] → [Range{8080..8081}, Range{8085..8086}]
    /// **Why test this**: Real-world exclusion lists have gaps
    #[test]
    fn test_compact_with_gaps() {
        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
            PortExclusion::Single(8085),
            PortExclusion::Single(8086),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 2);

        // First range: 8080..8081
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8081);
            }
            _ => panic!("Expected range for first group"),
        }

        // Second range: 8085..8086
        match compacted[1] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8085);
                assert_eq!(end, 8086);
            }
            _ => panic!("Expected range for second group"),
        }
    }

    /// Test compaction of existing ranges is idempotent.
    ///
    /// **Idempotency test**: compact(already_compacted) = already_compacted
    /// **Why test this**: Ensures repeated compaction is safe
    #[test]
    fn test_compact_already_compacted_range() {
        let exclusions = vec![PortExclusion::Range {
            start: 8080,
            end: 8090,
        }];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8090);
            }
            _ => panic!("Expected range"),
        }
    }

    /// Test compaction merges overlapping ranges.
    ///
    /// **Correctness test**: Overlapping ranges must merge
    /// **Example**: [Range{8080..8085}, Range{8083..8090}] → Range{8080..8090}
    /// **Invariant**: Union semantics - all ports from both ranges are covered
    /// **Why test this**: Prevents redundant exclusions after manual editing
    #[test]
    fn test_compact_overlapping_ranges() {
        let exclusions = vec![
            PortExclusion::Range {
                start: 8080,
                end: 8085,
            },
            PortExclusion::Range {
                start: 8083,
                end: 8090,
            },
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8090);
            }
            _ => panic!("Expected merged range"),
        }
    }

    /// Test compaction merges adjacent ranges (ranges that touch).
    ///
    /// **Example**: [Range{8080..8085}, Range{8086..8090}] → Range{8080..8090}
    /// **Property**: Ranges are adjacent if end_1 + 1 = start_2
    /// **Why test this**: Maximizes compaction efficiency
    #[test]
    fn test_compact_adjacent_ranges() {
        let exclusions = vec![
            PortExclusion::Range {
                start: 8080,
                end: 8085,
            },
            PortExclusion::Range {
                start: 8086,
                end: 8090,
            },
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8090);
            }
            _ => panic!("Expected merged range"),
        }
    }

    /// Test compaction merges ranges with singles filling gaps.
    ///
    /// **Complex scenario**: [Range{8080..8082}, Single(8083), Range{8084..8086}]
    ///                       → Range{8080..8086}
    /// **Why test this**: Mixed input types should be handled correctly
    #[test]
    fn test_compact_mixed_ranges_and_singles() {
        let exclusions = vec![
            PortExclusion::Range {
                start: 8080,
                end: 8082,
            },
            PortExclusion::Single(8083),
            PortExclusion::Range {
                start: 8084,
                end: 8086,
            },
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8086);
            }
            _ => panic!("Expected single merged range"),
        }
    }

    /// Test compaction preserves isolated single between ranges.
    ///
    /// **Example**: [Range{8080..8082}, Single(8085), Range{8087..8090}]
    ///              → [Range{8080..8082}, Single(8085), Range{8087..8090}]
    /// **Why test this**: Isolated ports should not be merged into distant ranges
    #[test]
    fn test_compact_preserves_isolated_single_between_ranges() {
        let exclusions = vec![
            PortExclusion::Range {
                start: 8080,
                end: 8082,
            },
            PortExclusion::Single(8085),
            PortExclusion::Range {
                start: 8087,
                end: 8090,
            },
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 3);
    }

    /// Test compaction handles unsorted input correctly.
    ///
    /// **Property**: Compaction should work regardless of input order
    /// **Example**: [8085, 8080, 8081] → Range{8080..8081}, Single(8085)
    /// **Implementation detail**: The verified compaction is order-independent
    /// **Why test this**: Real config files might have unsorted exclusions
    #[test]
    fn test_compact_unsorted_input() {
        let exclusions = vec![
            PortExclusion::Single(8085),
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        // Should be sorted and compacted
        assert_eq!(compacted.len(), 2);

        // First: 8080..8081
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8081);
            }
            _ => panic!("Expected range"),
        }

        // Second: 8085
        assert_eq!(compacted[1], PortExclusion::Single(8085));
    }

    /// Test compaction with duplicate ports is idempotent.
    ///
    /// **Property**: Duplicates should be deduplicated
    /// **Example**: [8080, 8080, 8081] → Range{8080..8081}
    /// **Why test this**: Config might have duplicates after manual editing
    #[test]
    fn test_compact_with_duplicates() {
        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 8080);
                assert_eq!(end, 8081);
            }
            _ => panic!("Expected range"),
        }
    }

    /// Test compaction at minimum port boundary.
    ///
    /// **Edge case**: Ports starting from 1 (minimum valid port) should be handled correctly
    /// **Why test this**: Ensures no underflow or special-case errors at the lower boundary
    /// **Complements**: test_compact_near_max_port (tests upper boundary)
    #[test]
    fn test_compact_at_min_port() {
        let exclusions = vec![
            PortExclusion::Single(1),
            PortExclusion::Single(2),
            PortExclusion::Single(3),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(
            compacted.len(),
            1,
            "Adjacent ports starting at 1 should compact to one range"
        );
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 1, "Range should start at minimum port 1");
                assert_eq!(end, 3, "Range should end at port 3");
            }
            _ => panic!("Expected range for adjacent ports at minimum boundary"),
        }
    }

    /// Test compaction at port boundaries (edge values).
    ///
    /// **Edge case**: Ports near u16::MAX should be handled correctly
    /// **Why test this**: Ensures no overflow in range calculations
    #[test]
    fn test_compact_near_max_port() {
        let exclusions = vec![
            PortExclusion::Single(65533),
            PortExclusion::Single(65534),
            PortExclusion::Single(65535),
        ];
        let compacted = compact_exclusion_list(&exclusions);

        assert_eq!(compacted.len(), 1);
        match compacted[0] {
            PortExclusion::Range { start, end } => {
                assert_eq!(start, 65533);
                assert_eq!(end, 65535);
            }
            _ => panic!("Expected range"),
        }
    }

    /// Test compaction preserves semantic equivalence.
    ///
    /// **Invariant**: The set of excluded ports before and after compaction
    /// must be identical.
    /// **Why test this**: This is the most important property - correctness
    #[test]
    fn test_compact_preserves_port_set() {
        use std::collections::HashSet;

        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Range {
                start: 8082,
                end: 8085,
            },
            PortExclusion::Single(8090),
            PortExclusion::Range {
                start: 8095,
                end: 8097,
            },
        ];

        // Collect all ports from original
        let mut original_ports = HashSet::new();
        for excl in &exclusions {
            match excl {
                PortExclusion::Single(p) => {
                    original_ports.insert(*p);
                }
                PortExclusion::Range { start, end } => {
                    for p in *start..=*end {
                        original_ports.insert(p);
                    }
                }
            }
        }

        // Compact and collect all ports from result
        let compacted = compact_exclusion_list(&exclusions);
        let mut compacted_ports = HashSet::new();
        for excl in &compacted {
            match excl {
                PortExclusion::Single(p) => {
                    compacted_ports.insert(*p);
                }
                PortExclusion::Range { start, end } => {
                    for p in *start..=*end {
                        compacted_ports.insert(p);
                    }
                }
            }
        }

        assert_eq!(
            original_ports, compacted_ports,
            "Compaction must preserve the exact set of excluded ports"
        );
    }

    /// Test compaction is stable (repeated application converges).
    ///
    /// **Property**: compact(compact(x)) = compact(x)
    /// **Why test this**: Ensures algorithm reaches a fixed point
    #[test]
    fn test_compact_is_idempotent() {
        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
            PortExclusion::Single(8085),
        ];

        let compacted_once = compact_exclusion_list(&exclusions);
        let compacted_twice = compact_exclusion_list(&compacted_once);

        assert_eq!(
            compacted_once, compacted_twice,
            "Compaction should be idempotent"
        );
    }

    /// Test compaction reduces list size when possible.
    ///
    /// **Optimization property**: Compacted list should be <= original size
    /// **Example**: [8080, 8081, 8082] (3 items) → [Range{8080..8082}] (1 item)
    /// **Why test this**: The whole point is to reduce config file size
    #[test]
    fn test_compact_reduces_size() {
        let exclusions = vec![
            PortExclusion::Single(8080),
            PortExclusion::Single(8081),
            PortExclusion::Single(8082),
            PortExclusion::Single(8083),
            PortExclusion::Single(8084),
        ];

        let compacted = compact_exclusion_list(&exclusions);

        assert!(
            compacted.len() <= exclusions.len(),
            "Compaction should not increase size"
        );
        assert_eq!(
            compacted.len(),
            1,
            "Should compact 5 consecutive singles to 1 range"
        );
    }

    /// Regression: a reversed range used to vanish (changing the excluded
    /// set) and port 0 used to survive or merge with port 1. A list with an
    /// invalid entry is now returned unchanged, and the fallible compaction
    /// that `compact-exclusions` uses rejects it.
    #[test]
    fn test_compact_never_changes_invalid_lists() {
        for exclusions in [
            vec![
                PortExclusion::Range {
                    start: 5010,
                    end: 5000,
                },
                PortExclusion::Single(6000),
            ],
            vec![PortExclusion::Range {
                start: 5010,
                end: 5000,
            }],
            vec![PortExclusion::Single(0), PortExclusion::Single(1)],
        ] {
            assert_eq!(compact_exclusion_list(&exclusions), exclusions);
            assert!(compact_exclusions(&exclusions).is_err());
        }
    }

    #[test]
    fn test_compact_full_port_space() {
        let exclusions = vec![
            PortExclusion::Range {
                start: 2,
                end: 65535,
            },
            PortExclusion::Single(1),
        ];
        assert_eq!(
            compact_exclusion_list(&exclusions),
            vec![PortExclusion::Range {
                start: 1,
                end: 65535
            }]
        );
    }
}
