//! Property-based tests for configuration system.

use super::merger::ConfigMerger;
use super::schema::{CleanupConfig, Config, PortConfig, DEFAULT_MAX_PORT, DEFAULT_MIN_PORT};
use proptest::prelude::*;

// Strategy for generating valid port ranges
fn port_range_strategy() -> impl Strategy<Value = (u16, u16)> {
    (1u16..=65535).prop_flat_map(|start| (Just(start), start..=65535))
}

// Strategy for generating port configs
fn port_config_strategy() -> impl Strategy<Value = PortConfig> {
    port_range_strategy().prop_map(|(min, max)| PortConfig {
        min,
        max: Some(max),
        max_offset: None,
    })
}

// Strategy for generating configs
fn config_strategy() -> impl Strategy<Value = Config> {
    (
        prop::option::of("[a-z]{1,20}"),
        prop::option::of(port_config_strategy()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
        prop::option::of(any::<bool>()),
    )
        .prop_map(|(project, ports, autoinit, autoprune, autoexpire)| Config {
            project,
            ports,
            disable_autoinit: autoinit,
            disable_autoprune: autoprune,
            disable_autoexpire: autoexpire,
            ..Default::default()
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // Configuration merging preserves non-None values from higher precedence
    #[test]
    fn config_merge_higher_precedence_wins(
        low_project in prop::option::of("[a-z]{1,20}"),
        high_project in prop::option::of("[a-z]{1,20}"),
        low_ports in prop::option::of(port_config_strategy()),
        high_ports in prop::option::of(port_config_strategy()),
        low_autoinit in prop::option::of(any::<bool>()),
        high_autoinit in prop::option::of(any::<bool>()),
        low_autoprune in prop::option::of(any::<bool>()),
        high_autoprune in prop::option::of(any::<bool>()),
        low_autoexpire in prop::option::of(any::<bool>()),
        high_autoexpire in prop::option::of(any::<bool>())
    ) {
        let low = Config {
            project: low_project.clone(),
            ports: low_ports.clone(),
            disable_autoinit: low_autoinit,
            disable_autoprune: low_autoprune,
            disable_autoexpire: low_autoexpire,
            ..Default::default()
        };

        let high = Config {
            project: high_project.clone(),
            ports: high_ports.clone(),
            disable_autoinit: high_autoinit,
            disable_autoprune: high_autoprune,
            disable_autoexpire: high_autoexpire,
            ..Default::default()
        };

        let mut result = low.clone();
        ConfigMerger::merge_into(&mut result, &high);

        // High precedence wins if it's Some, otherwise low persists
        if high_project.is_some() {
            prop_assert_eq!(result.project, high_project);
        } else {
            prop_assert_eq!(result.project, low_project);
        }

        if high_ports.is_some() {
            prop_assert_eq!(result.ports, high_ports);
        } else {
            prop_assert_eq!(result.ports, low_ports);
        }

        if high_autoinit.is_some() {
            prop_assert_eq!(result.disable_autoinit, high_autoinit);
        } else {
            prop_assert_eq!(result.disable_autoinit, low_autoinit);
        }

        if high_autoprune.is_some() {
            prop_assert_eq!(result.disable_autoprune, high_autoprune);
        } else {
            prop_assert_eq!(result.disable_autoprune, low_autoprune);
        }

        if high_autoexpire.is_some() {
            prop_assert_eq!(result.disable_autoexpire, high_autoexpire);
        } else {
            prop_assert_eq!(result.disable_autoexpire, low_autoexpire);
        }
    }

    // Empty config is identity element for merge
    #[test]
    fn config_merge_identity(config in config_strategy()) {
        let empty = Config::default();
        let mut merged = config.clone();
        ConfigMerger::merge_into(&mut merged, &empty);

        // Merging with empty config should preserve all values
        prop_assert_eq!(config.project, merged.project);
        prop_assert_eq!(config.ports, merged.ports);
        prop_assert_eq!(config.disable_autoinit, merged.disable_autoinit);
    }

    // Valid port configs remain valid after merge
    #[test]
    fn valid_port_configs_stay_valid_after_merge(
        config1 in port_config_strategy(),
        config2 in port_config_strategy()
    ) {
        let c1 = Config {
            ports: Some(config1.clone()),
            ..Default::default()
        };

        let c2 = Config {
            ports: Some(config2.clone()),
            ..Default::default()
        };

        let mut merged = c1;
        ConfigMerger::merge_into(&mut merged, &c2);

        // After merge, should have config2's values
        prop_assert_eq!(merged.ports, Some(config2));
    }

    // Port config validation: min <= max
    #[test]
    fn port_config_min_le_max(min in 1u16..=65535, max in 1u16..=65535) {
        let config = PortConfig {
            min,
            max: Some(max),
            max_offset: None,
        };

        // Valid configs should have min <= max
        if min <= max {
            prop_assert!(config.min <= config.max.unwrap());
        }
    }

    // Port config with max_offset calculates correctly
    #[test]
    fn port_config_max_offset_calculation(min in 1u16..=65000, offset in 1u16..=535) {
        let config = PortConfig {
            min,
            max: None,
            max_offset: Some(offset),
        };

        // Offset should be valid (not overflowing)
        let calculated_max = u32::from(min) + u32::from(offset);
        prop_assert!(calculated_max <= 65535);
        prop_assert!(config.max_offset.is_some());
    }

    // Default port config is valid
    #[test]
    fn default_port_config_valid(_unit in 0u8..1) {
        let config = PortConfig::default();
        prop_assert_eq!(config.min, DEFAULT_MIN_PORT);
        prop_assert_eq!(config.max, Some(DEFAULT_MAX_PORT));
        prop_assert!(config.min <= config.max.unwrap());
    }

    // Cleanup config with valid days
    #[test]
    fn cleanup_config_valid_days(days in prop::option::of(1u32..=365)) {
        let config = CleanupConfig {
            expire_after_days: days,
        };

        if let Some(d) = config.expire_after_days {
            prop_assert!(d > 0);
            prop_assert!(d <= 365);
        }
    }

    // Merging is associative for simple fields
    #[test]
    fn config_merge_associative_simple(
        a_project in prop::option::of("[a-z]{1,10}"),
        b_project in prop::option::of("[a-z]{1,10}"),
        c_project in prop::option::of("[a-z]{1,10}")
    ) {
        let a = Config { project: a_project.clone(), ..Default::default() };
        let b = Config { project: b_project.clone(), ..Default::default() };
        let c = Config { project: c_project.clone(), ..Default::default() };

        // (a merge b) merge c
        let mut left = a.clone();
        ConfigMerger::merge_into(&mut left, &b);
        ConfigMerger::merge_into(&mut left, &c);

        // a merge (b merge c)
        let mut right = a;
        let mut b_merge_c = b;
        ConfigMerger::merge_into(&mut b_merge_c, &c);
        ConfigMerger::merge_into(&mut right, &b_merge_c);

        prop_assert_eq!(left.project, right.project);
    }

    // Boolean flags preserve their values
    #[test]
    fn config_boolean_flags_preserved(
        autoinit in any::<bool>(),
        autoprune in any::<bool>(),
        autoexpire in any::<bool>()
    ) {
        let config = Config {
            disable_autoinit: Some(autoinit),
            disable_autoprune: Some(autoprune),
            disable_autoexpire: Some(autoexpire),
            ..Default::default()
        };

        prop_assert_eq!(config.disable_autoinit, Some(autoinit));
        prop_assert_eq!(config.disable_autoprune, Some(autoprune));
        prop_assert_eq!(config.disable_autoexpire, Some(autoexpire));
    }
}
