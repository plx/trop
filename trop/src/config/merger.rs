//! Configuration merging and precedence handling.
//!
//! This module implements hierarchical merging of configuration sources,
//! with special handling for accumulated fields like `excluded_ports`.

use crate::config::loader::ConfigSource;
use crate::config::schema::{CleanupConfig, Config, PortConfig};

/// Merges configuration sources according to precedence rules.
///
/// # Examples
///
/// ```
/// use trop::config::{Config, ConfigMerger};
///
/// let low = Config { project: Some("low".to_string()), ..Default::default() };
/// let high = Config { project: Some("high".to_string()), ..Default::default() };
///
/// let mut result = low;
/// ConfigMerger::merge_into(&mut result, &high);
/// assert_eq!(result.project, Some("high".to_string()));
/// ```
pub struct ConfigMerger;

impl ConfigMerger {
    /// Merge multiple configuration sources into final config.
    ///
    /// Sources should be provided in order from lowest to highest precedence.
    /// The final configuration will have higher-precedence values overriding
    /// lower-precedence ones, with special handling for accumulated fields.
    #[must_use]
    pub fn merge(sources: Vec<ConfigSource>) -> Config {
        let mut result = Config::default();

        // Process in order (lowest to highest precedence)
        for source in sources {
            Self::merge_into(&mut result, &source.config);
        }

        result
    }

    /// Merge source config into target (source overwrites target).
    ///
    /// # Merging Rules
    ///
    /// - Simple fields: source overwrites if Some
    /// - Excluded ports: accumulated (union)
    /// - Nested configs: field-by-field merge
    /// - Occupancy config: atomic replacement
    /// - Reservation groups: complete replacement
    pub fn merge_into(target: &mut Config, source: &Config) {
        // Simple fields - source overwrites if Some
        if source.project.is_some() {
            target.project.clone_from(&source.project);
        }

        if source.disable_autoinit.is_some() {
            target.disable_autoinit = source.disable_autoinit;
        }

        if source.disable_autoprune.is_some() {
            target.disable_autoprune = source.disable_autoprune;
        }

        if source.disable_autoexpire.is_some() {
            target.disable_autoexpire = source.disable_autoexpire;
        }

        if source.allow_unrelated_path.is_some() {
            target.allow_unrelated_path = source.allow_unrelated_path;
        }

        if source.allow_change_project.is_some() {
            target.allow_change_project = source.allow_change_project;
        }

        if source.allow_change_task.is_some() {
            target.allow_change_task = source.allow_change_task;
        }

        if source.allow_change.is_some() {
            target.allow_change = source.allow_change;
        }

        if source.maximum_lock_wait_seconds.is_some() {
            target.maximum_lock_wait_seconds = source.maximum_lock_wait_seconds;
        }

        if source.output_format.is_some() {
            target.output_format = source.output_format;
        }

        // Merge ports config
        if let Some(ref source_ports) = source.ports {
            target.ports = Some(match &target.ports {
                Some(target_ports) => Self::merge_port_config(target_ports, source_ports),
                None => source_ports.clone(),
            });
        }

        // Merge excluded_ports (union of all exclusions)
        if let Some(ref source_excluded) = source.excluded_ports {
            match &mut target.excluded_ports {
                Some(target_excluded) => {
                    target_excluded.extend(source_excluded.clone());
                }
                None => {
                    target.excluded_ports.clone_from(&source.excluded_ports);
                }
            }
        }

        // Merge cleanup config
        if let Some(ref source_cleanup) = source.cleanup {
            target.cleanup = Some(match &target.cleanup {
                Some(target_cleanup) => Self::merge_cleanup(target_cleanup, source_cleanup),
                None => source_cleanup.clone(),
            });
        }

        // Occupancy config - full replacement (not field-by-field)
        if source.occupancy_check.is_some() {
            target.occupancy_check.clone_from(&source.occupancy_check);
        }

        // Reservation groups - don't merge, only replace
        if source.reservations.is_some() {
            target.reservations.clone_from(&source.reservations);
        }
    }

    /// Merge port configuration.
    ///
    /// Source values take precedence over target values.
    fn merge_port_config(target: &PortConfig, source: &PortConfig) -> PortConfig {
        PortConfig {
            min: source.min, // Always use source
            max: source.max.or(target.max),
            max_offset: source.max_offset.or(target.max_offset),
        }
    }

    /// Merge cleanup configuration.
    ///
    /// Source values take precedence over target values.
    fn merge_cleanup(target: &CleanupConfig, source: &CleanupConfig) -> CleanupConfig {
        CleanupConfig {
            expire_after_days: source.expire_after_days.or(target.expire_after_days),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{OccupancyConfig, PortExclusion};
    use std::path::PathBuf;

    fn make_source(precedence: u8, config: Config) -> ConfigSource {
        ConfigSource {
            path: PathBuf::from(format!("test-{precedence}.yaml")),
            precedence,
            config,
        }
    }

    #[test]
    fn test_merge_simple_fields() {
        let mut target = Config::default();
        let source = Config {
            project: Some("test".to_string()),
            disable_autoinit: Some(true),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        assert_eq!(target.project, Some("test".to_string()));
        assert_eq!(target.disable_autoinit, Some(true));
    }

    #[test]
    fn test_merge_overwrites() {
        let mut target = Config {
            project: Some("old".to_string()),
            ..Default::default()
        };
        let source = Config {
            project: Some("new".to_string()),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        assert_eq!(target.project, Some("new".to_string()));
    }

    #[test]
    fn test_merge_excluded_ports_accumulates() {
        let mut target = Config {
            excluded_ports: Some(vec![PortExclusion::Single(5001)]),
            ..Default::default()
        };
        let source = Config {
            excluded_ports: Some(vec![PortExclusion::Single(5002)]),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        assert_eq!(target.excluded_ports.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_merge_port_config() {
        let mut target = Config {
            ports: Some(PortConfig {
                min: 5000,
                max: Some(6000),
                max_offset: None,
            }),
            ..Default::default()
        };
        let source = Config {
            ports: Some(PortConfig {
                min: 7000,
                max: None,
                max_offset: Some(100),
            }),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        let ports = target.ports.unwrap();
        assert_eq!(ports.min, 7000); // Always use source
        assert_eq!(ports.max, Some(6000)); // Source didn't specify, use target
        assert_eq!(ports.max_offset, Some(100)); // Use source
    }

    #[test]
    fn test_merge_cleanup_config() {
        let mut target = Config {
            cleanup: Some(CleanupConfig {
                expire_after_days: Some(30),
            }),
            ..Default::default()
        };
        let source = Config {
            cleanup: Some(CleanupConfig {
                expire_after_days: Some(60),
            }),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        let cleanup = target.cleanup.unwrap();
        assert_eq!(cleanup.expire_after_days, Some(60));
    }

    #[test]
    fn test_merge_occupancy_atomic_replacement() {
        let mut target = Config {
            occupancy_check: Some(OccupancyConfig {
                skip: Some(false),
                skip_ip4: Some(true),
                skip_ip6: Some(false),
                skip_tcp: Some(false),
                skip_udp: Some(false),
                check_all_interfaces: Some(false),
            }),
            ..Default::default()
        };
        let source = Config {
            occupancy_check: Some(OccupancyConfig {
                skip: Some(true),
                skip_ip4: None,
                skip_ip6: None,
                skip_tcp: None,
                skip_udp: None,
                check_all_interfaces: None,
            }),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        let occ = target.occupancy_check.unwrap();
        // Complete replacement - target values are lost
        assert_eq!(occ.skip, Some(true));
        assert_eq!(occ.skip_ip4, None);
    }

    #[test]
    fn test_merge_multiple_sources() {
        let sources = vec![
            make_source(
                1,
                Config {
                    project: Some("base".to_string()),
                    disable_autoinit: Some(false),
                    ..Default::default()
                },
            ),
            make_source(
                2,
                Config {
                    project: Some("override".to_string()),
                    disable_autoprune: Some(true),
                    ..Default::default()
                },
            ),
            make_source(
                3,
                Config {
                    disable_autoinit: Some(true),
                    ..Default::default()
                },
            ),
        ];

        let result = ConfigMerger::merge(sources);
        assert_eq!(result.project, Some("override".to_string()));
        assert_eq!(result.disable_autoinit, Some(true));
        assert_eq!(result.disable_autoprune, Some(true));
    }

    #[test]
    fn test_merge_none_values_dont_overwrite() {
        let mut target = Config {
            project: Some("existing".to_string()),
            ..Default::default()
        };
        let source = Config {
            project: None,
            disable_autoinit: Some(true),
            ..Default::default()
        };

        ConfigMerger::merge_into(&mut target, &source);
        assert_eq!(target.project, Some("existing".to_string()));
        assert_eq!(target.disable_autoinit, Some(true));
    }
}

// Property-based tests for configuration merging
#[cfg(test)]
#[allow(unused_doc_comments)] // proptest! macro doesn't support doc comments
mod property_tests {
    use super::*;
    use crate::config::schema::{OccupancyConfig, PortExclusion};
    use proptest::prelude::*;

    // ==================================================================================
    // PROPERTY TESTS FOR MERGING IDENTITY ELEMENT
    // ==================================================================================

    /// Property: Merging with empty Config is identity operation for simple fields
    ///
    /// Mathematical Property: For all configs c, merge(c, empty) = c and merge(empty, c) = c
    /// This verifies that Config::default() acts as an identity element for the merge operation.
    ///
    /// WHY THIS MATTERS: Empty configs should never corrupt existing configuration data.
    /// If a config file is missing or empty, it shouldn't change existing values.
    proptest! {
        #[test]
        fn prop_merge_empty_is_right_identity(
            project in proptest::option::of("[a-z]{1,20}"),
            disable_autoinit in proptest::option::of(any::<bool>()),
            disable_autoprune in proptest::option::of(any::<bool>()),
        ) {
            let mut config = Config {
                project: project.clone(),
                disable_autoinit,
                disable_autoprune,
                ..Default::default()
            };

            let original = config.clone();
            let empty = Config::default();

            // Merge with empty should not change anything
            ConfigMerger::merge_into(&mut config, &empty);

            prop_assert_eq!(config.project, original.project, "Project unchanged by empty merge");
            prop_assert_eq!(config.disable_autoinit, original.disable_autoinit, "disable_autoinit unchanged");
            prop_assert_eq!(config.disable_autoprune, original.disable_autoprune, "disable_autoprune unchanged");
        }
    }

    /// Property: Merging empty config into target is left identity for Some fields
    ///
    /// Mathematical Property: merge_into(empty, c) leaves empty with values from c
    proptest! {
        #[test]
        fn prop_merge_into_empty_copies_values(
            project in proptest::option::of("[a-z]{1,20}"),
            max_lock_wait in proptest::option::of(1u64..=60),
        ) {
            let mut empty = Config::default();
            let source = Config {
                project: project.clone(),
                maximum_lock_wait_seconds: max_lock_wait,
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut empty, &source);

            prop_assert_eq!(empty.project, project, "Project copied from source");
            prop_assert_eq!(empty.maximum_lock_wait_seconds, max_lock_wait, "Lock wait copied");
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR MERGING PRECEDENCE
    // ==================================================================================

    /// Property: Source always overwrites target for Some values (simple fields)
    ///
    /// Mathematical Property: For all configs c1, c2 where c2.field = Some(x),
    /// merge(c1, c2).field = Some(x)
    ///
    /// WHY THIS MATTERS: Higher precedence configs must override lower precedence ones.
    /// This is fundamental to the configuration hierarchy.
    proptest! {
        #[test]
        fn prop_merge_source_overwrites_for_simple_fields(
            target_proj in "[a-z]{1,10}",
            source_proj in "[a-z]{1,10}",
            target_bool in any::<bool>(),
            source_bool in any::<bool>(),
        ) {
            let mut target = Config {
                project: Some(target_proj),
                disable_autoinit: Some(target_bool),
                ..Default::default()
            };

            let source = Config {
                project: Some(source_proj.clone()),
                disable_autoinit: Some(source_bool),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            // Source values must win
            prop_assert_eq!(target.project, Some(source_proj), "Source project wins");
            prop_assert_eq!(target.disable_autoinit, Some(source_bool), "Source bool wins");
        }
    }

    /// Property: None values in source don't overwrite Some values in target
    ///
    /// Mathematical Property: For config c1 where c1.field = Some(x),
    /// merge(c1, c2) where c2.field = None should preserve c1.field = Some(x)
    ///
    /// WHY THIS MATTERS: Missing values in higher-precedence configs shouldn't
    /// delete values from lower-precedence configs. Only explicit values override.
    proptest! {
        #[test]
        fn prop_merge_none_preserves_existing(
            existing_project in "[a-z]{1,20}",
            existing_timeout in 1u64..=600,
        ) {
            let mut target = Config {
                project: Some(existing_project.clone()),
                maximum_lock_wait_seconds: Some(existing_timeout),
                ..Default::default()
            };

            let source = Config {
                project: None,
                maximum_lock_wait_seconds: None,
                // But set something else
                disable_autoinit: Some(true),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            // Original values should be preserved
            prop_assert_eq!(target.project, Some(existing_project), "Existing project preserved");
            prop_assert_eq!(target.maximum_lock_wait_seconds, Some(existing_timeout), "Existing timeout preserved");
            // New value should be set
            prop_assert_eq!(target.disable_autoinit, Some(true), "New value set");
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR EXCLUDED PORTS ACCUMULATION
    // ==================================================================================

    /// Property: Excluded ports accumulate (union behavior)
    ///
    /// Mathematical Property: excluded_ports is accumulated, not replaced:
    /// merge(c1, c2).excluded_ports = c1.excluded_ports ∪ c2.excluded_ports
    ///
    /// WHY THIS MATTERS: Port exclusions from all config sources should be combined,
    /// not replaced. This ensures all exclusions are respected regardless of source.
    proptest! {
        #[test]
        fn prop_merge_excluded_ports_accumulate(
            port1 in 1u16..=65535,
            port2 in 1u16..=65535,
            port3 in 1u16..=65535,
        ) {
            let mut target = Config {
                excluded_ports: Some(vec![
                    PortExclusion::Single(port1),
                    PortExclusion::Single(port2),
                ]),
                ..Default::default()
            };

            let source = Config {
                excluded_ports: Some(vec![
                    PortExclusion::Single(port3),
                ]),
                ..Default::default()
            };

            let original_count = target.excluded_ports.as_ref().unwrap().len();
            let source_count = source.excluded_ports.as_ref().unwrap().len();

            ConfigMerger::merge_into(&mut target, &source);

            // Should have sum of both lists
            let final_count = target.excluded_ports.as_ref().unwrap().len();
            prop_assert_eq!(final_count, original_count + source_count, "Exclusions accumulated");

            // Should contain all original exclusions
            let excluded = target.excluded_ports.as_ref().unwrap();
            prop_assert!(excluded.contains(&PortExclusion::Single(port1)), "Original port1 present");
            prop_assert!(excluded.contains(&PortExclusion::Single(port2)), "Original port2 present");
            prop_assert!(excluded.contains(&PortExclusion::Single(port3)), "Source port3 present");
        }
    }

    /// Property: Merging N configs accumulates all excluded ports
    ///
    /// Mathematical Property: For configs c1, c2, ..., cn,
    /// merge([c1, c2, ..., cn]).excluded_ports contains all exclusions from all configs
    proptest! {
        #[test]
        fn prop_merge_multiple_accumulates_all_exclusions(
            ports in prop::collection::vec(1u16..=65535, 1..=10),
        ) {
            // Create multiple configs, each with one exclusion
            let configs: Vec<Config> = ports.iter().map(|&port| Config {
                excluded_ports: Some(vec![PortExclusion::Single(port)]),
                ..Default::default()
            }).collect();

            let mut result = Config::default();
            for config in configs {
                ConfigMerger::merge_into(&mut result, &config);
            }

            let final_exclusions = result.excluded_ports.as_ref().unwrap();
            prop_assert_eq!(final_exclusions.len(), ports.len(), "All exclusions accumulated");

            // Verify all ports are present
            for port in ports {
                prop_assert!(
                    final_exclusions.contains(&PortExclusion::Single(port)),
                    "Port {} should be in exclusions", port
                );
            }
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR PORT CONFIG MERGING
    // ==================================================================================

    /// Property: Port config min always uses source value
    ///
    /// Mathematical Property: merge(c1, c2).ports.min = c2.ports.min
    /// The min value is NOT merged field-by-field; source completely replaces for min.
    ///
    /// WHY THIS MATTERS: The min port defines the base of the allocation range.
    /// It should come from the highest-precedence config that specifies it.
    proptest! {
        #[test]
        fn prop_merge_port_config_min_from_source(
            target_min in 1u16..=60000,
            source_min in 1u16..=60000,
        ) {
            let mut target = Config {
                ports: Some(PortConfig {
                    min: target_min,
                    max: Some(65000),
                    max_offset: None,
                }),
                ..Default::default()
            };

            let source = Config {
                ports: Some(PortConfig {
                    min: source_min,
                    max: None,
                    max_offset: None,
                }),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            let merged_ports = target.ports.unwrap();
            prop_assert_eq!(merged_ports.min, source_min, "Min always from source");
        }
    }

    /// Property: Port config max uses source value if present, else target
    ///
    /// Mathematical Property: merge(c1, c2).ports.max = c2.ports.max OR c1.ports.max
    /// Uses optional field merging: source takes precedence only if Some.
    proptest! {
        #[test]
        fn prop_merge_port_config_max_optional(
            target_min in 1u16..=50000,
            target_max in 50001u16..=65535,
            source_min in 1u16..=50000,
            has_source_max in any::<bool>(),
        ) {
            let mut target = Config {
                ports: Some(PortConfig {
                    min: target_min,
                    max: Some(target_max),
                    max_offset: None,
                }),
                ..Default::default()
            };

            let source = Config {
                ports: Some(PortConfig {
                    min: source_min,
                    max: if has_source_max { Some(60000) } else { None },
                    max_offset: None,
                }),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            let merged_ports = target.ports.unwrap();

            if has_source_max {
                prop_assert_eq!(merged_ports.max, Some(60000), "Source max used when present");
            } else {
                prop_assert_eq!(merged_ports.max, Some(target_max), "Target max preserved when source None");
            }
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR CLEANUP CONFIG MERGING
    // ==================================================================================

    /// Property: Cleanup config merges with optional field semantics
    ///
    /// Mathematical Property: For cleanup fields, source.Some(x) overwrites, None preserves
    proptest! {
        #[test]
        fn prop_merge_cleanup_config_optional_fields(
            target_days in 1u32..=365,
            source_days_opt in proptest::option::of(1u32..=365),
        ) {
            let mut target = Config {
                cleanup: Some(CleanupConfig {
                    expire_after_days: Some(target_days),
                }),
                ..Default::default()
            };

            let source = Config {
                cleanup: Some(CleanupConfig {
                    expire_after_days: source_days_opt,
                }),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            let merged_cleanup = target.cleanup.unwrap();

            match source_days_opt {
                Some(days) => prop_assert_eq!(
                    merged_cleanup.expire_after_days,
                    Some(days),
                    "Source value used when present"
                ),
                None => prop_assert_eq!(
                    merged_cleanup.expire_after_days,
                    Some(target_days),
                    "Target value preserved when source None"
                ),
            }
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR OCCUPANCY CONFIG (ATOMIC REPLACEMENT)
    // ==================================================================================

    /// Property: Occupancy config is atomically replaced, not field-merged
    ///
    /// Mathematical Property: merge(c1, c2).occupancy_check = c2.occupancy_check (complete)
    /// Unlike other nested configs, occupancy is replaced as a unit.
    ///
    /// WHY THIS MATTERS: Occupancy checks are tightly coupled. Mixing fields from
    /// different sources could create inconsistent check behavior. Atomic replacement
    /// ensures clarity about which config source controls occupancy checking.
    proptest! {
        #[test]
        fn prop_merge_occupancy_atomic_replacement(
            target_skip in any::<bool>(),
            target_skip_ip4 in any::<bool>(),
            source_skip in any::<bool>(),
        ) {
            let mut target = Config {
                occupancy_check: Some(OccupancyConfig {
                    skip: Some(target_skip),
                    skip_ip4: Some(target_skip_ip4),
                    skip_ip6: Some(true),
                    skip_tcp: Some(false),
                    skip_udp: Some(true),
                    check_all_interfaces: Some(false),
                }),
                ..Default::default()
            };

            let source = Config {
                occupancy_check: Some(OccupancyConfig {
                    skip: Some(source_skip),
                    // All other fields are None
                    skip_ip4: None,
                    skip_ip6: None,
                    skip_tcp: None,
                    skip_udp: None,
                    check_all_interfaces: None,
                }),
                ..Default::default()
            };

            ConfigMerger::merge_into(&mut target, &source);

            let merged = target.occupancy_check.unwrap();

            // Source completely replaces target (atomic)
            prop_assert_eq!(merged.skip, Some(source_skip), "Source skip used");
            prop_assert_eq!(merged.skip_ip4, None, "Target skip_ip4 replaced with None");
            prop_assert_eq!(merged.skip_ip6, None, "Target skip_ip6 replaced with None");
            // Not field-by-field merge - complete replacement
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR MERGE ORDER (TRANSITIVITY)
    // ==================================================================================

    /// Property: Sequential merges respect order
    ///
    /// Mathematical Property: merge(merge(a, b), c) processes in order a < b < c
    /// Each successive merge can override previous values.
    ///
    /// WHY THIS MATTERS: Configuration precedence must be deterministic and follow
    /// a clear hierarchy (e.g., user config > project config > defaults).
    proptest! {
        #[test]
        fn prop_merge_order_matters_for_overwrites(
            val1 in "[a-z]{1,10}",
            val2 in "[a-z]{1,10}",
            val3 in "[a-z]{1,10}",
        ) {
            // Assume val1, val2, val3 are different
            prop_assume!(val1 != val2);
            prop_assume!(val2 != val3);
            prop_assume!(val1 != val3);

            let c1 = Config {
                project: Some(val1),
                ..Default::default()
            };

            let c2 = Config {
                project: Some(val2.clone()),
                ..Default::default()
            };

            let c3 = Config {
                project: Some(val3.clone()),
                ..Default::default()
            };

            let mut result = c1;
            ConfigMerger::merge_into(&mut result, &c2);
            ConfigMerger::merge_into(&mut result, &c3);

            // Final value should be from c3 (last/highest precedence)
            prop_assert_eq!(result.project, Some(val3), "Last merge wins");
        }
    }
}
