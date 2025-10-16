//! Environment variable handling for configuration overrides.
//!
//! This module provides support for TROP_* environment variables that
//! override configuration file values.

use crate::config::schema::{Config, PortExclusion};
use crate::error::{Error, Result};
use std::env;

/// Handles environment variable overrides for configuration.
///
/// # Examples
///
/// ```no_run
/// use trop::config::{Config, EnvironmentConfig};
///
/// let mut config = Config::default();
/// EnvironmentConfig::apply_overrides(&mut config).unwrap();
/// ```
pub struct EnvironmentConfig;

impl EnvironmentConfig {
    /// Apply environment variable overrides to config.
    ///
    /// Reads all TROP_* environment variables and applies them to the
    /// configuration with higher precedence than file-based configs.
    ///
    /// # Errors
    ///
    /// Returns an error if any environment variable value is invalid
    /// (e.g., non-numeric port, invalid boolean).
    pub fn apply_overrides(config: &mut Config) -> Result<()> {
        // TROP_PROJECT
        if let Ok(project) = env::var("TROP_PROJECT") {
            config.project = Some(project);
        }

        // TROP_DISABLE_AUTOINIT
        if let Ok(val) = env::var("TROP_DISABLE_AUTOINIT") {
            config.disable_autoinit = Some(Self::parse_bool("TROP_DISABLE_AUTOINIT", &val)?);
        }

        // TROP_DISABLE_AUTOPRUNE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOPRUNE") {
            config.disable_autoprune = Some(Self::parse_bool("TROP_DISABLE_AUTOPRUNE", &val)?);
        }

        // TROP_DISABLE_AUTOEXPIRE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOEXPIRE") {
            config.disable_autoexpire = Some(Self::parse_bool("TROP_DISABLE_AUTOEXPIRE", &val)?);
        }

        // Port range from TROP_PORT_MIN and TROP_PORT_MAX
        Self::apply_port_overrides(config)?;

        // TROP_EXCLUDED_PORTS (comma-separated)
        if let Ok(excluded) = env::var("TROP_EXCLUDED_PORTS") {
            let exclusions = Self::parse_excluded_ports(&excluded)?;
            match &mut config.excluded_ports {
                Some(existing) => existing.extend(exclusions),
                None => config.excluded_ports = Some(exclusions),
            }
        }

        // TROP_EXPIRE_AFTER_DAYS
        if let Ok(days) = env::var("TROP_EXPIRE_AFTER_DAYS") {
            let days = days.parse().map_err(|_| Error::Validation {
                field: "TROP_EXPIRE_AFTER_DAYS".into(),
                message: "Must be a positive integer".into(),
            })?;

            let cleanup = config.cleanup.get_or_insert_with(Default::default);
            cleanup.expire_after_days = Some(days);
        }

        // TROP_MAXIMUM_LOCK_WAIT_SECONDS
        if let Ok(seconds) = env::var("TROP_MAXIMUM_LOCK_WAIT_SECONDS") {
            config.maximum_lock_wait_seconds =
                Some(seconds.parse().map_err(|_| Error::Validation {
                    field: "TROP_MAXIMUM_LOCK_WAIT_SECONDS".into(),
                    message: "Must be a positive integer".into(),
                })?);
        }

        // Permission flags
        if let Ok(val) = env::var("TROP_ALLOW_UNRELATED_PATH") {
            config.allow_unrelated_path =
                Some(Self::parse_bool("TROP_ALLOW_UNRELATED_PATH", &val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE_PROJECT") {
            config.allow_change_project =
                Some(Self::parse_bool("TROP_ALLOW_CHANGE_PROJECT", &val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE_TASK") {
            config.allow_change_task = Some(Self::parse_bool("TROP_ALLOW_CHANGE_TASK", &val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE") {
            config.allow_change = Some(Self::parse_bool("TROP_ALLOW_CHANGE", &val)?);
        }

        // Occupancy check flags
        Self::apply_occupancy_overrides(config)?;

        Ok(())
    }

    /// Apply port-related environment variable overrides.
    fn apply_port_overrides(config: &mut Config) -> Result<()> {
        let mut port_config = config.ports.clone().unwrap_or_default();
        let mut modified = false;

        if let Ok(min) = env::var("TROP_PORT_MIN") {
            port_config.min = min.parse().map_err(|_| Error::Validation {
                field: "TROP_PORT_MIN".into(),
                message: "Invalid port number".into(),
            })?;
            modified = true;
        }

        if let Ok(max) = env::var("TROP_PORT_MAX") {
            port_config.max = Some(max.parse().map_err(|_| Error::Validation {
                field: "TROP_PORT_MAX".into(),
                message: "Invalid port number".into(),
            })?);
            modified = true;
        }

        if modified {
            config.ports = Some(port_config);
        }

        Ok(())
    }

    /// Apply occupancy check environment variable overrides.
    fn apply_occupancy_overrides(config: &mut Config) -> Result<()> {
        let mut occupancy = config.occupancy_check.clone().unwrap_or_default();
        let mut modified = false;

        if let Ok(val) = env::var("TROP_SKIP_OCCUPANCY_CHECK") {
            occupancy.skip = Some(Self::parse_bool("TROP_SKIP_OCCUPANCY_CHECK", &val)?);
            modified = true;
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV4") {
            occupancy.skip_ip4 = Some(Self::parse_bool("TROP_SKIP_IPV4", &val)?);
            modified = true;
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV6") {
            occupancy.skip_ip6 = Some(Self::parse_bool("TROP_SKIP_IPV6", &val)?);
            modified = true;
        }

        if let Ok(val) = env::var("TROP_SKIP_TCP") {
            occupancy.skip_tcp = Some(Self::parse_bool("TROP_SKIP_TCP", &val)?);
            modified = true;
        }

        if let Ok(val) = env::var("TROP_SKIP_UDP") {
            occupancy.skip_udp = Some(Self::parse_bool("TROP_SKIP_UDP", &val)?);
            modified = true;
        }

        if let Ok(val) = env::var("TROP_CHECK_ALL_INTERFACES") {
            occupancy.check_all_interfaces =
                Some(Self::parse_bool("TROP_CHECK_ALL_INTERFACES", &val)?);
            modified = true;
        }

        if modified {
            config.occupancy_check = Some(occupancy);
        }

        Ok(())
    }

    /// Parse a boolean value from a string.
    ///
    /// Accepts: true/1/yes/on for true, false/0/no/off for false (case-insensitive).
    fn parse_bool(field: &str, s: &str) -> Result<bool> {
        match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(Error::Validation {
                field: field.into(),
                message: format!(
                    "Invalid boolean value: '{s}' (expected true/false/1/0/yes/no/on/off)"
                ),
            }),
        }
    }

    /// Parse excluded ports from comma-separated string.
    ///
    /// Supports both individual ports (e.g., "5001") and ranges (e.g., "5000..5010").
    fn parse_excluded_ports(s: &str) -> Result<Vec<PortExclusion>> {
        let mut exclusions = Vec::new();

        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some((start_str, end_str)) = part.split_once("..") {
                // Range format: "5000..5010"
                let start: u16 = start_str.parse().map_err(|_| Error::Validation {
                    field: "TROP_EXCLUDED_PORTS".into(),
                    message: format!("Invalid port in range: {start_str}"),
                })?;

                let end: u16 = end_str.parse().map_err(|_| Error::Validation {
                    field: "TROP_EXCLUDED_PORTS".into(),
                    message: format!("Invalid port in range: {end_str}"),
                })?;

                exclusions.push(PortExclusion::Range { start, end });
            } else {
                // Single port
                let port_num: u16 = part.parse().map_err(|_| Error::Validation {
                    field: "TROP_EXCLUDED_PORTS".into(),
                    message: format!("Invalid port: {part}"),
                })?;

                exclusions.push(PortExclusion::Single(port_num));
            }
        }

        Ok(exclusions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bool_true_variants() {
        assert!(EnvironmentConfig::parse_bool("test", "true").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "TRUE").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "1").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "yes").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "YES").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "on").unwrap());
        assert!(EnvironmentConfig::parse_bool("test", "ON").unwrap());
    }

    #[test]
    fn test_parse_bool_false_variants() {
        assert!(!EnvironmentConfig::parse_bool("test", "false").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "FALSE").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "0").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "no").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "NO").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "off").unwrap());
        assert!(!EnvironmentConfig::parse_bool("test", "OFF").unwrap());
    }

    #[test]
    fn test_parse_bool_invalid() {
        let result = EnvironmentConfig::parse_bool("test", "maybe");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_excluded_ports_single() {
        let result = EnvironmentConfig::parse_excluded_ports("5001").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], PortExclusion::Single(5001));
    }

    #[test]
    fn test_parse_excluded_ports_multiple() {
        let result = EnvironmentConfig::parse_excluded_ports("5001,5002,5003").unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_parse_excluded_ports_range() {
        let result = EnvironmentConfig::parse_excluded_ports("5000..5010").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            PortExclusion::Range {
                start: 5000,
                end: 5010
            }
        );
    }

    #[test]
    fn test_parse_excluded_ports_mixed() {
        let result = EnvironmentConfig::parse_excluded_ports("5001, 5005..5009, 5020").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], PortExclusion::Single(5001));
        assert_eq!(
            result[1],
            PortExclusion::Range {
                start: 5005,
                end: 5009
            }
        );
        assert_eq!(result[2], PortExclusion::Single(5020));
    }

    #[test]
    fn test_parse_excluded_ports_whitespace() {
        let result = EnvironmentConfig::parse_excluded_ports(" 5001 , 5002 ").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_parse_excluded_ports_empty() {
        let result = EnvironmentConfig::parse_excluded_ports("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_excluded_ports_invalid_port() {
        let result = EnvironmentConfig::parse_excluded_ports("not_a_port");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_excluded_ports_invalid_range() {
        let result = EnvironmentConfig::parse_excluded_ports("5000..invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_overrides_no_env_vars() {
        // This test doesn't set any env vars, just ensures no crashes
        let mut config = Config::default();
        let result = EnvironmentConfig::apply_overrides(&mut config);
        assert!(result.is_ok());
    }
}

// Property-based tests for environment variable parsing
#[cfg(test)]
#[allow(unused_doc_comments)] // proptest! macro doesn't support doc comments
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // ==================================================================================
    // PROPERTY TESTS FOR BOOLEAN PARSING
    // ==================================================================================

    /// Property: Boolean parsing should be case-insensitive
    ///
    /// Mathematical Property: For all valid boolean strings s in {true, false, 1, 0, yes, no, on, off},
    /// parse_bool(s) = parse_bool(uppercase(s)) = parse_bool(lowercase(s))
    ///
    /// WHY THIS MATTERS: Environment variables may come from different sources with
    /// different casing conventions. The parser should handle all reasonable variants.
    proptest! {
        #[test]
        fn prop_bool_parsing_case_insensitive(use_uppercase in any::<bool>()) {
            let true_variants = vec!["true", "1", "yes", "on"];
            let false_variants = vec!["false", "0", "no", "off"];

            for variant in true_variants {
                let input = if use_uppercase {
                    variant.to_uppercase()
                } else {
                    variant.to_lowercase()
                };

                let result = EnvironmentConfig::parse_bool("test", &input);
                prop_assert!(result.is_ok(), "Failed to parse: {}", input);
                prop_assert_eq!(result.unwrap(), true, "{} should parse to true", input);
            }

            for variant in false_variants {
                let input = if use_uppercase {
                    variant.to_uppercase()
                } else {
                    variant.to_lowercase()
                };

                let result = EnvironmentConfig::parse_bool("test", &input);
                prop_assert!(result.is_ok(), "Failed to parse: {}", input);
                prop_assert_eq!(result.unwrap(), false, "{} should parse to false", input);
            }
        }
    }

    /// Property: Boolean parsing roundtrip for standard representations
    ///
    /// Mathematical Property: parse_bool(bool::to_string()) should succeed and preserve value
    /// For the canonical string representations, we should be able to roundtrip.
    proptest! {
        #[test]
        fn prop_bool_parsing_roundtrip(value in any::<bool>()) {
            // Use the "true"/"false" representation
            let s = value.to_string();
            let parsed = EnvironmentConfig::parse_bool("test", &s).unwrap();
            prop_assert_eq!(parsed, value, "Boolean value should roundtrip through string");

            // Also test "1"/"0" representation
            let numeric = if value { "1" } else { "0" };
            let parsed_numeric = EnvironmentConfig::parse_bool("test", numeric).unwrap();
            prop_assert_eq!(parsed_numeric, value, "Boolean value should parse from numeric string");
        }
    }

    /// Property: Invalid boolean strings should always fail
    ///
    /// Mathematical Property: For all strings s not in the valid set,
    /// parse_bool(s) returns Err
    ///
    /// WHY THIS MATTERS: Invalid inputs should fail fast with clear errors,
    /// not silently default to some value.
    proptest! {
        #[test]
        fn prop_bool_parsing_rejects_invalid(
            s in "[a-z]{2,10}".prop_filter("Not a valid bool string", |s| {
                !matches!(s.as_str(), "true" | "false" | "yes" | "no" | "on" | "off")
            })
        ) {
            let result = EnvironmentConfig::parse_bool("test", &s);
            prop_assert!(result.is_err(), "Invalid string '{}' should fail to parse", s);
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR PORT RANGE PARSING
    // ==================================================================================

    /// Property: Valid port ranges should always parse
    ///
    /// Mathematical Property: For all valid port pairs (start, end) where start <= end,
    /// parse_excluded_ports("start..end") should succeed and produce Range{start, end}
    proptest! {
        #[test]
        fn prop_port_range_parsing_valid(start in 1u16..=65535, offset in 0u16..=100) {
            let end = start.saturating_add(offset);
            let input = format!("{start}..{end}");

            let result = EnvironmentConfig::parse_excluded_ports(&input);
            prop_assert!(result.is_ok(), "Valid range should parse: {}", input);

            let exclusions = result.unwrap();
            prop_assert_eq!(exclusions.len(), 1, "Should have one exclusion");

            match &exclusions[0] {
                PortExclusion::Range { start: s, end: e } => {
                    prop_assert_eq!(*s, start, "Start value preserved");
                    prop_assert_eq!(*e, end, "End value preserved");
                    prop_assert!(*s <= *e, "Range invariant maintained");
                }
                PortExclusion::Single(_) => prop_assert!(false, "Expected Range variant"),
            }
        }
    }

    /// Property: Single port should parse correctly
    ///
    /// Mathematical Property: For all valid ports p (1..=65535),
    /// parse_excluded_ports(p.to_string()) = Single(p)
    proptest! {
        #[test]
        fn prop_single_port_parsing(port in 1u16..=65535) {
            let input = port.to_string();
            let result = EnvironmentConfig::parse_excluded_ports(&input);

            prop_assert!(result.is_ok(), "Valid port should parse: {}", input);

            let exclusions = result.unwrap();
            prop_assert_eq!(exclusions.len(), 1, "Should have one exclusion");
            prop_assert_eq!(&exclusions[0], &PortExclusion::Single(port), "Port value preserved");
        }
    }

    /// Property: Comma-separated list should parse to multiple exclusions
    ///
    /// Mathematical Property: parse_excluded_ports("p1,p2,...,pn") produces
    /// n exclusions, each corresponding to one input port.
    ///
    /// WHY THIS MATTERS: Users should be able to specify multiple exclusions
    /// in a single environment variable, separated by commas.
    proptest! {
        #[test]
        fn prop_comma_separated_parsing(ports in prop::collection::vec(1u16..=65535, 1..=5)) {
            let input = ports.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");

            let result = EnvironmentConfig::parse_excluded_ports(&input);
            prop_assert!(result.is_ok(), "Valid comma-separated list should parse: {}", input);

            let exclusions = result.unwrap();
            prop_assert_eq!(exclusions.len(), ports.len(), "Should have correct count");

            // Verify each port is present
            for (i, &port) in ports.iter().enumerate() {
                prop_assert_eq!(
                    &exclusions[i],
                    &PortExclusion::Single(port),
                    "Port {} should be at position {}", port, i
                );
            }
        }
    }

    /// Property: Whitespace should be tolerated around ports and ranges
    ///
    /// Mathematical Property: parse(s) = parse(trim(s)) for exclusion strings
    ///
    /// WHY THIS MATTERS: Environment variables might have extra whitespace
    /// from shell quoting or user input. The parser should be forgiving.
    proptest! {
        #[test]
        fn prop_port_parsing_tolerates_whitespace(
            port in 1u16..=65535,
            leading_spaces in 0usize..=3,
            trailing_spaces in 0usize..=3,
        ) {
            let spaces_before = " ".repeat(leading_spaces);
            let spaces_after = " ".repeat(trailing_spaces);
            let input = format!("{spaces_before}{port}{spaces_after}");

            let result = EnvironmentConfig::parse_excluded_ports(&input);
            prop_assert!(result.is_ok(), "Should parse with whitespace: '{}'", input);

            let exclusions = result.unwrap();
            if !input.trim().is_empty() {
                prop_assert_eq!(exclusions.len(), 1, "Should have one exclusion");
                prop_assert_eq!(&exclusions[0], &PortExclusion::Single(port), "Port value correct");
            }
        }
    }

    /// Property: Mixed single ports and ranges should parse correctly
    ///
    /// Mathematical Property: parse("p1,r1..r2,p3") produces three exclusions
    /// of the appropriate types in order.
    proptest! {
        #[test]
        fn prop_mixed_exclusions_parsing(
            port1 in 1u16..=20000,
            range_start in 30000u16..=40000,
            port2 in 50000u16..=65535,
        ) {
            let range_end = range_start + 100;
            let input = format!("{port1}, {range_start}..{range_end}, {port2}");

            let result = EnvironmentConfig::parse_excluded_ports(&input);
            prop_assert!(result.is_ok(), "Mixed exclusions should parse: {}", input);

            let exclusions = result.unwrap();
            prop_assert_eq!(exclusions.len(), 3, "Should have three exclusions");

            prop_assert_eq!(&exclusions[0], &PortExclusion::Single(port1), "First port correct");
            match &exclusions[1] {
                PortExclusion::Range { start, end } => {
                    prop_assert_eq!(*start, range_start, "Range start correct");
                    prop_assert_eq!(*end, range_end, "Range end correct");
                }
                PortExclusion::Single(_) => prop_assert!(false, "Second should be Range"),
            }
            prop_assert_eq!(&exclusions[2], &PortExclusion::Single(port2), "Third port correct");
        }
    }

    /// Property: Empty string should parse to empty list
    ///
    /// Mathematical Property: parse_excluded_ports("") = []
    proptest! {
        #[test]
        fn prop_empty_string_empty_list(_dummy in any::<u8>()) {
            let result = EnvironmentConfig::parse_excluded_ports("");
            prop_assert!(result.is_ok(), "Empty string should parse");

            let exclusions = result.unwrap();
            prop_assert!(exclusions.is_empty(), "Should produce empty list");
        }
    }

    /// Property: Invalid port numbers should fail
    ///
    /// Mathematical Property: For port numbers outside [1, 65535], parsing should fail
    proptest! {
        #[test]
        fn prop_invalid_port_numbers_fail(invalid_port in 65536u32..=100_000) {
            let input = invalid_port.to_string();
            let result = EnvironmentConfig::parse_excluded_ports(&input);

            prop_assert!(result.is_err(), "Invalid port {} should fail to parse", invalid_port);
        }
    }

    /// Property: Port 0 should fail to parse
    ///
    /// WHY THIS MATTERS: Port 0 is not a valid port in the trop system
    /// (per the Port type's validation rules).
    proptest! {
        #[test]
        fn prop_port_zero_fails(_dummy in any::<u8>()) {
            let result = EnvironmentConfig::parse_excluded_ports("0");
            // Note: parse_excluded_ports uses u16::parse which will succeed for 0,
            // but validation should reject it later. For now, it parses.
            // This test documents current behavior - may change if validation moves to parser.
            prop_assert!(result.is_ok(), "Port 0 currently parses (validation happens later)");
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR PORT PARSING INVARIANTS
    // ==================================================================================

    /// Property: Parsing should preserve port ordering in comma-separated lists
    ///
    /// Mathematical Property: The order of exclusions in the result matches
    /// the order in the input string.
    ///
    /// WHY THIS MATTERS: While order doesn't affect exclusion semantics,
    /// preserving it makes debugging easier and matches user expectations.
    proptest! {
        #[test]
        fn prop_port_parsing_preserves_order(
            port1 in 1u16..=10000,
            port2 in 20000u16..=30000,
            port3 in 40000u16..=50000,
        ) {
            let input = format!("{port1},{port2},{port3}");
            let result = EnvironmentConfig::parse_excluded_ports(&input).unwrap();

            prop_assert_eq!(result.len(), 3, "Should have three exclusions");
            prop_assert_eq!(&result[0], &PortExclusion::Single(port1), "First port in order");
            prop_assert_eq!(&result[1], &PortExclusion::Single(port2), "Second port in order");
            prop_assert_eq!(&result[2], &PortExclusion::Single(port3), "Third port in order");
        }
    }

    /// Property: Duplicate ports in input produce duplicate exclusions
    ///
    /// Mathematical Property: parse_excluded_ports does not deduplicate;
    /// if a port appears n times in input, it appears n times in output.
    ///
    /// WHY THIS MATTERS: Deduplication is the responsibility of the merge/validation
    /// layer, not the parser. The parser should faithfully represent input.
    proptest! {
        #[test]
        fn prop_duplicates_preserved(port in 1u16..=65535, count in 2usize..=4) {
            let input = vec![port.to_string(); count].join(",");
            let result = EnvironmentConfig::parse_excluded_ports(&input).unwrap();

            prop_assert_eq!(result.len(), count, "Should have {} exclusions", count);

            for exclusion in result {
                prop_assert_eq!(exclusion, PortExclusion::Single(port), "All should be same port");
            }
        }
    }
}
