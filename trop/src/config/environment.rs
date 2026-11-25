//! Environment variable handling for configuration overrides.
//!
//! This module provides support for TROP_* environment variables that
//! override configuration file values.

use crate::config::effective::ConfigField;
use crate::config::schema::{Config, OutputFormat, PortConfig, PortExclusion};
use crate::error::{Error, Result};
use crate::verified::config::{parse_bool_word, parse_u32_decimal, parse_u64_decimal};
use crate::verified::exclusions::parse_u16;
use crate::verified::port_bounds::{
    check_upper_declaration, overlay_port_bounds, PortBoundsViolation, PortLayer,
};
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
        Self::apply_overrides_with(config, |_, _| {})
    }

    pub(crate) fn apply_overrides_with(
        config: &mut Config,
        mut on_field: impl FnMut(ConfigField, &'static str),
    ) -> Result<()> {
        // TROP_PROJECT
        if let Ok(project) = env::var("TROP_PROJECT") {
            config.project = Some(project);
            on_field(ConfigField::Project, "TROP_PROJECT");
        }

        // TROP_DISABLE_AUTOINIT
        if let Ok(val) = env::var("TROP_DISABLE_AUTOINIT") {
            config.disable_autoinit = Some(Self::parse_bool("TROP_DISABLE_AUTOINIT", &val)?);
            on_field(ConfigField::DisableAutoinit, "TROP_DISABLE_AUTOINIT");
        }

        // TROP_DISABLE_AUTOPRUNE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOPRUNE") {
            config.disable_autoprune = Some(Self::parse_bool("TROP_DISABLE_AUTOPRUNE", &val)?);
            on_field(ConfigField::DisableAutoprune, "TROP_DISABLE_AUTOPRUNE");
        }

        // TROP_DISABLE_AUTOEXPIRE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOEXPIRE") {
            config.disable_autoexpire = Some(Self::parse_bool("TROP_DISABLE_AUTOEXPIRE", &val)?);
            on_field(ConfigField::DisableAutoexpire, "TROP_DISABLE_AUTOEXPIRE");
        }

        // Port range from TROP_PORT_MIN, TROP_PORT_MAX, and TROP_PORT_MAX_OFFSET
        Self::apply_port_overrides(config, &mut on_field)?;

        // TROP_EXCLUDED_PORTS (comma-separated)
        if let Ok(excluded) = env::var("TROP_EXCLUDED_PORTS") {
            let exclusions = Self::parse_excluded_ports(&excluded)?;
            match &mut config.excluded_ports {
                Some(existing) => existing.extend(exclusions),
                None => config.excluded_ports = Some(exclusions),
            }
            on_field(ConfigField::ExcludedPorts, "TROP_EXCLUDED_PORTS");
        }

        // TROP_EXPIRE_AFTER_DAYS (`u32` decimal text; validation rejects 0)
        if let Ok(days) = env::var("TROP_EXPIRE_AFTER_DAYS") {
            let days = parse_u32_decimal(days.as_bytes()).ok_or_else(|| Error::Validation {
                field: "TROP_EXPIRE_AFTER_DAYS".into(),
                message: "Must be a positive integer".into(),
            })?;

            let cleanup = config.cleanup.get_or_insert_with(Default::default);
            cleanup.expire_after_days = Some(days);
            on_field(
                ConfigField::CleanupExpireAfterDays,
                "TROP_EXPIRE_AFTER_DAYS",
            );
        }

        // Canonical TROP_BUSY_TIMEOUT wins over its historical library alias.
        if let Some((seconds, variable)) =
            Self::read_with_alias("TROP_BUSY_TIMEOUT", &["TROP_MAXIMUM_LOCK_WAIT_SECONDS"])
        {
            config.maximum_lock_wait_seconds = Some(
                parse_u64_decimal(seconds.as_bytes()).ok_or_else(|| Error::Validation {
                    field: variable.into(),
                    message: "Must be a positive integer".into(),
                })?,
            );
            on_field(ConfigField::MaximumLockWaitSeconds, variable);
        }

        if let Ok(format) = env::var("TROP_OUTPUT_FORMAT") {
            config.output_format =
                Some(
                    OutputFormat::parse_keyword(&format).ok_or_else(|| Error::Validation {
                        field: "TROP_OUTPUT_FORMAT".into(),
                        message: format!(
                            "Invalid output format: '{format}' (expected table/json/csv/tsv)"
                        ),
                    })?,
                );
            on_field(ConfigField::OutputFormat, "TROP_OUTPUT_FORMAT");
        }

        // Permission flags
        if let Ok(val) = env::var("TROP_ALLOW_UNRELATED_PATH") {
            config.allow_unrelated_path =
                Some(Self::parse_bool("TROP_ALLOW_UNRELATED_PATH", &val)?);
            on_field(ConfigField::AllowUnrelatedPath, "TROP_ALLOW_UNRELATED_PATH");
        }

        if let Some((val, variable)) =
            Self::read_with_alias("TROP_ALLOW_PROJECT_CHANGE", &["TROP_ALLOW_CHANGE_PROJECT"])
        {
            config.allow_change_project = Some(Self::parse_bool(variable, &val)?);
            on_field(ConfigField::AllowChangeProject, variable);
        }

        if let Some((val, variable)) =
            Self::read_with_alias("TROP_ALLOW_TASK_CHANGE", &["TROP_ALLOW_CHANGE_TASK"])
        {
            config.allow_change_task = Some(Self::parse_bool(variable, &val)?);
            on_field(ConfigField::AllowChangeTask, variable);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE") {
            config.allow_change = Some(Self::parse_bool("TROP_ALLOW_CHANGE", &val)?);
            on_field(ConfigField::AllowChange, "TROP_ALLOW_CHANGE");
        }

        // Occupancy check flags
        Self::apply_occupancy_overrides(config, &mut on_field)?;

        Ok(())
    }

    /// Apply port-related environment variable overrides.
    ///
    /// `TROP_PORT_MIN`, `TROP_PORT_MAX`, and `TROP_PORT_MAX_OFFSET` form one
    /// layer. Like a YAML document, the layer must not set both `max` and
    /// `max_offset`, and its `max_offset` must land on a valid port above the
    /// `min` in effect at this layer; it is then resolved to an absolute
    /// `max`. An upper bound the layer does not set stays where it is when
    /// only `TROP_PORT_MIN` is set. The checks and the overlay are decided by
    /// the verified port-bounds kernel.
    fn apply_port_overrides(
        config: &mut Config,
        on_field: &mut impl FnMut(ConfigField, &'static str),
    ) -> Result<()> {
        let min = Self::read_port_variable("TROP_PORT_MIN", "Invalid port number")?;
        let max = Self::read_port_variable("TROP_PORT_MAX", "Invalid port number")?;
        let max_offset = Self::read_port_variable("TROP_PORT_MAX_OFFSET", "Invalid port offset")?;

        let lower = config.ports.clone().unwrap_or_default().bounds();
        let layer = PortLayer {
            min,
            max,
            max_offset,
        };
        let (bounds, leaves) = overlay_port_bounds(lower, layer);
        // The overlay's `min` is the one in effect at this layer.
        let anchor = bounds.min;
        check_upper_declaration(anchor, max, max_offset).map_err(|violation| {
            let (field, message) = match violation {
                PortBoundsViolation::OffsetOverflow { offset } => (
                    "ports.max_offset (TROP_PORT_MAX_OFFSET)",
                    format!("Offset {offset} would overflow when added to min port {anchor}"),
                ),
                PortBoundsViolation::BothMaxAndOffset => (
                    "ports (TROP_PORT_MAX, TROP_PORT_MAX_OFFSET)",
                    "Cannot specify both max and max_offset".to_string(),
                ),
                // `ZeroOffset` is the only other violation the kernel reports
                // for an upper-bound declaration.
                PortBoundsViolation::ZeroOffset
                | PortBoundsViolation::InvalidMin
                | PortBoundsViolation::InvalidMax
                | PortBoundsViolation::MaxBelowMin => (
                    "ports.max_offset (TROP_PORT_MAX_OFFSET)",
                    "max_offset must be > 0".to_string(),
                ),
            };
            Error::Validation {
                field: field.into(),
                message,
            }
        })?;

        if leaves.min || leaves.upper {
            config.ports = Some(PortConfig::from_bounds(bounds));
        }
        if leaves.min {
            on_field(ConfigField::PortsMin, "TROP_PORT_MIN");
        }
        if leaves.upper {
            // Exactly one upper-bound variable is set (the check above
            // rejects both).
            let upper_variable = if max_offset.is_some() {
                "TROP_PORT_MAX_OFFSET"
            } else {
                "TROP_PORT_MAX"
            };
            on_field(ConfigField::PortsMax, upper_variable);
            on_field(ConfigField::PortsMaxOffset, upper_variable);
        }

        Ok(())
    }

    /// Read and parse one port-valued `TROP_PORT_*` variable, if it is set.
    ///
    /// The value is `u16` decimal text (`u16::from_str`'s syntax), decided by
    /// the verified decimal parser that every numeric `TROP_*` variable uses.
    fn read_port_variable(variable: &'static str, message: &str) -> Result<Option<u16>> {
        env::var(variable)
            .ok()
            .map(|value| {
                parse_u16(value.as_bytes()).ok_or_else(|| Error::Validation {
                    field: variable.into(),
                    message: message.into(),
                })
            })
            .transpose()
    }

    /// Apply occupancy check environment variable overrides.
    fn apply_occupancy_overrides(
        config: &mut Config,
        on_field: &mut impl FnMut(ConfigField, &'static str),
    ) -> Result<()> {
        let mut occupancy = config.occupancy_check.clone().unwrap_or_default();
        let mut modified = false;

        if let Ok(val) = env::var("TROP_SKIP_OCCUPANCY_CHECK") {
            occupancy.skip = Some(Self::parse_bool("TROP_SKIP_OCCUPANCY_CHECK", &val)?);
            modified = true;
            on_field(ConfigField::OccupancySkip, "TROP_SKIP_OCCUPANCY_CHECK");
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV4") {
            occupancy.skip_ip4 = Some(Self::parse_bool("TROP_SKIP_IPV4", &val)?);
            modified = true;
            on_field(ConfigField::OccupancySkipIp4, "TROP_SKIP_IPV4");
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV6") {
            occupancy.skip_ip6 = Some(Self::parse_bool("TROP_SKIP_IPV6", &val)?);
            modified = true;
            on_field(ConfigField::OccupancySkipIp6, "TROP_SKIP_IPV6");
        }

        if let Ok(val) = env::var("TROP_SKIP_TCP") {
            occupancy.skip_tcp = Some(Self::parse_bool("TROP_SKIP_TCP", &val)?);
            modified = true;
            on_field(ConfigField::OccupancySkipTcp, "TROP_SKIP_TCP");
        }

        if let Ok(val) = env::var("TROP_SKIP_UDP") {
            occupancy.skip_udp = Some(Self::parse_bool("TROP_SKIP_UDP", &val)?);
            modified = true;
            on_field(ConfigField::OccupancySkipUdp, "TROP_SKIP_UDP");
        }

        if let Ok(val) = env::var("TROP_CHECK_ALL_INTERFACES") {
            occupancy.check_all_interfaces =
                Some(Self::parse_bool("TROP_CHECK_ALL_INTERFACES", &val)?);
            modified = true;
            on_field(
                ConfigField::OccupancyCheckAllInterfaces,
                "TROP_CHECK_ALL_INTERFACES",
            );
        }

        if modified {
            config.occupancy_check = Some(occupancy);
        }

        Ok(())
    }

    fn read_with_alias(
        canonical: &'static str,
        aliases: &[&'static str],
    ) -> Option<(String, &'static str)> {
        if let Ok(value) = env::var(canonical) {
            return Some((value, canonical));
        }

        aliases
            .iter()
            .find_map(|alias| env::var(alias).ok().map(|value| (value, *alias)))
    }

    /// Parse a boolean value from a string.
    ///
    /// Accepts: true/1/yes/on for true, false/0/no/off for false
    /// (case-insensitive). The decision is the verified kernel's
    /// `parse_bool_word`, which ignores ASCII case. The words are ASCII, so
    /// this is exactly matching `s.to_lowercase()` against them: the only
    /// non-ASCII char whose lowercase is ASCII is the Kelvin sign (`k`), and
    /// no word contains `k` (pinned by
    /// `test_parse_bool_ascii_folding_matches_unicode_lowercase`).
    fn parse_bool(field: &str, s: &str) -> Result<bool> {
        parse_bool_word(s.as_bytes()).ok_or_else(|| Error::Validation {
            field: field.into(),
            message: format!(
                "Invalid boolean value: '{s}' (expected true/false/1/0/yes/no/on/off)"
            ),
        })
    }

    /// Parse excluded ports from comma-separated string.
    ///
    /// Supports both individual ports (e.g., "5001") and ranges (e.g., "5000..5010").
    /// Each item uses the same parser as the YAML string form and
    /// `trop exclude` (`PortExclusion`'s `FromStr`), so whitespace around
    /// each side of `..` is ignored here too.
    fn parse_excluded_ports(s: &str) -> Result<Vec<PortExclusion>> {
        let mut exclusions = Vec::new();

        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let exclusion = part.parse().map_err(|error| Error::Validation {
                field: "TROP_EXCLUDED_PORTS".into(),
                message: format!("{error}"),
            })?;
            exclusions.push(exclusion);
        }

        Ok(exclusions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

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
        assert!(result.is_empty(), "unexpected exclusions: {result:?}");
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

    /// Regression: the env var used to reject whitespace around `..`, which
    /// the YAML string form accepts. All sources now share one parser.
    #[test]
    fn test_parse_excluded_ports_whitespace_around_range_separator() {
        let result = EnvironmentConfig::parse_excluded_ports("5000 .. 5010, 6000 ..6001").unwrap();
        assert_eq!(
            result,
            vec![
                PortExclusion::Range {
                    start: 5000,
                    end: 5010
                },
                PortExclusion::Range {
                    start: 6000,
                    end: 6001
                },
            ]
        );
    }

    #[test]
    fn test_parse_excluded_ports_error_messages_unchanged() {
        let message = |s: &str| match EnvironmentConfig::parse_excluded_ports(s) {
            Err(Error::Validation { field, message }) => {
                assert_eq!(field, "TROP_EXCLUDED_PORTS");
                message
            }
            other => panic!("expected a validation error, got {other:?}"),
        };
        assert_eq!(message("5001, abc"), "Invalid port: abc");
        assert_eq!(message("x..5010"), "Invalid port in range: x");
        assert_eq!(message("5000..y "), "Invalid port in range: y");
        assert_eq!(message("70000"), "Invalid port: 70000");
    }

    #[test]
    fn test_apply_overrides_no_env_vars() {
        // This test doesn't set any env vars, just ensures no crashes
        let mut config = Config::default();
        let result = EnvironmentConfig::apply_overrides(&mut config);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_apply_port_max_offset_override_clears_max() {
        let mut config = Config {
            ports: Some(crate::config::PortConfig {
                min: 8000,
                max: Some(9000),
                max_offset: None,
            }),
            ..Default::default()
        };

        std::env::set_var("TROP_PORT_MAX_OFFSET", "25");
        EnvironmentConfig::apply_overrides(&mut config).unwrap();
        std::env::remove_var("TROP_PORT_MAX_OFFSET");

        // The offset replaces the explicit max and is resolved against the
        // `min` in effect (8000) into the absolute max it names.
        let ports = config.ports.unwrap();
        assert_eq!(ports.min, 8000);
        assert_eq!(ports.max, Some(8025));
        assert_eq!(ports.max_offset, None);
    }

    /// The pre-kernel boolean parser (Unicode lowercasing), as an oracle.
    pub(super) fn reference_parse_bool(s: &str) -> Option<bool> {
        match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    /// The kernel folds only ASCII case; the old parser used Unicode
    /// `to_lowercase`. They agree because the only non-ASCII char whose
    /// lowercase is all-ASCII is the Kelvin sign (to `k`), and no boolean word
    /// contains `k`.
    #[test]
    fn test_parse_bool_ascii_folding_matches_unicode_lowercase() {
        let lowers_to_ascii: Vec<char> = (0..=0x10_FFFF_u32)
            .filter_map(char::from_u32)
            .filter(|c| !c.is_ascii() && c.to_lowercase().all(|l| l.is_ascii()))
            .collect();
        assert_eq!(lowers_to_ascii, ['\u{212A}']);
        assert_eq!('\u{212A}'.to_lowercase().to_string(), "k");
        let words = ["true", "1", "yes", "on", "false", "0", "no", "off"];
        assert!(words.iter().all(|word| !word.contains('k')));

        let mut inputs: Vec<String> = Vec::new();
        for word in words {
            inputs.push(word.to_string());
            inputs.push(word.to_ascii_uppercase());
            inputs.push(format!(" {word}"));
            inputs.push(format!("{word} "));
            inputs.push(format!("{word}\n"));
            inputs.push(word.replace('o', "\u{d8}"));
            inputs.push(word.replace('e', "\u{c9}"));
        }
        inputs.extend(
            [
                "",
                "\u{212A}",
                "o\u{212A}",
                "\u{130}",
                "TrUe",
                "yEs",
                "ON",
                "nO",
                "OfF",
                "2",
                "10",
                "00",
                "truee",
                "fals",
                "y",
                "n",
                "t",
                "f",
            ]
            .map(str::to_string),
        );
        for input in &inputs {
            assert_eq!(
                EnvironmentConfig::parse_bool("TEST", input).ok(),
                reference_parse_bool(input),
                "{input:?}"
            );
        }
    }

    /// The verified decimal parsers accept exactly what the unsigned
    /// `from_str` implementations accept, with the same values.
    #[test]
    fn test_decimal_parsers_match_std_at_the_edges() {
        use crate::verified::config::{parse_u32_decimal, parse_u64_decimal};
        use crate::verified::exclusions::parse_u16;

        for text in [
            "",
            "+",
            "-",
            "-0",
            "0",
            "+0",
            "00",
            "007",
            "1",
            "65535",
            "65536",
            "+65535",
            "0065535",
            "4294967295",
            "4294967296",
            "18446744073709551615",
            "18446744073709551616",
            "99999999999999999999999",
            " 1",
            "1 ",
            "1_000",
            "1e3",
            "0x10",
            "++1",
            "+-1",
            "\u{663}",
            "\u{ff11}",
            "1\0",
            "/",
            ":",
        ] {
            assert_eq!(
                parse_u16(text.as_bytes()),
                text.parse::<u16>().ok(),
                "{text:?}"
            );
            assert_eq!(
                parse_u32_decimal(text.as_bytes()),
                text.parse::<u32>().ok(),
                "{text:?}"
            );
            assert_eq!(
                parse_u64_decimal(text.as_bytes()),
                text.parse::<u64>().ok(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn test_output_format_keywords_round_trip_and_ignore_ascii_case() {
        use crate::config::OutputFormat;

        let reference = |text: &str| match text.to_ascii_lowercase().as_str() {
            "table" => Some(OutputFormat::Table),
            "json" => Some(OutputFormat::Json),
            "csv" => Some(OutputFormat::Csv),
            "tsv" => Some(OutputFormat::Tsv),
            _ => None,
        };
        for format in [
            OutputFormat::Json,
            OutputFormat::Csv,
            OutputFormat::Tsv,
            OutputFormat::Table,
        ] {
            let keyword = format.to_string();
            assert_eq!(OutputFormat::parse_keyword(&keyword), Some(format));
            assert_eq!(
                OutputFormat::parse_keyword(&keyword.to_ascii_uppercase()),
                Some(format)
            );
            // Display agrees with the YAML (serde) spelling.
            assert_eq!(
                serde_yaml::to_string(&format).unwrap().trim(),
                keyword.as_str()
            );
        }
        for text in [
            "json",
            "JSON",
            "Csv",
            "tSv",
            "TABLE",
            "",
            "jso",
            "jsonn",
            " json",
            "json ",
            "csv\n",
            "t\u{e1}ble",
            "\u{212A}",
            "yaml",
        ] {
            assert_eq!(
                OutputFormat::parse_keyword(text),
                reference(text),
                "{text:?}"
            );
        }
    }
}

// Property-based tests for environment variable parsing
#[cfg(all(test, feature = "property-tests"))]
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

    /// Property: the verified decimal parsers are differentially equal to
    /// `u16`/`u32`/`u64::from_str` on arbitrary text, including digit-heavy
    /// text near the overflow boundaries.
    proptest! {
        #[test]
        fn prop_decimal_parsers_match_std(
            text in prop_oneof![
                any::<String>(),
                "[+-]?[0-9]{0,25}",
                "[ +0-9a-f_.-]{0,8}",
            ]
        ) {
            use crate::verified::config::{parse_u32_decimal, parse_u64_decimal};
            use crate::verified::exclusions::parse_u16;
            prop_assert_eq!(parse_u16(text.as_bytes()), text.parse::<u16>().ok());
            prop_assert_eq!(parse_u32_decimal(text.as_bytes()), text.parse::<u32>().ok());
            prop_assert_eq!(parse_u64_decimal(text.as_bytes()), text.parse::<u64>().ok());
        }
    }

    /// Property: the verified boolean parser agrees with the pre-kernel
    /// (Unicode-lowercasing) parser on arbitrary text and on case mixes of
    /// the words.
    proptest! {
        #[test]
        fn prop_bool_parser_matches_reference(
            text in prop_oneof![
                any::<String>(),
                "(?i)(true|false|yes|no|on|off|1|0)",
                "[tTrRuUeEfFaAlLsSyYnNoO01\u{212A}\u{130} ]{0,6}",
            ]
        ) {
            prop_assert_eq!(
                EnvironmentConfig::parse_bool("TEST", &text).ok(),
                super::tests::reference_parse_bool(&text)
            );
        }
    }
}
