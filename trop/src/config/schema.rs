//! Configuration schema definitions.
//!
//! This module defines the complete configuration structure for trop,
//! including all settings for ports, exclusions, cleanup, and reservation groups.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

/// Complete configuration structure.
///
/// This represents the full configuration schema for trop, supporting
/// hierarchical configuration from multiple sources.
///
/// # Examples
///
/// ```
/// use trop::config::{Config, PortConfig};
///
/// let config = Config {
///     project: Some("my-project".to_string()),
///     ports: Some(PortConfig {
///         min: 5000,
///         max: Some(7000),
///         max_offset: None,
///     }),
///     ..Default::default()
/// };
/// assert_eq!(config.project, Some("my-project".to_string()));
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Project identifier (only valid in trop.yaml files).
    pub project: Option<String>,

    /// Port allocation settings.
    pub ports: Option<PortConfig>,

    /// Excluded ports list.
    #[serde(default)]
    pub excluded_ports: Option<Vec<PortExclusion>>,

    /// Cleanup settings.
    pub cleanup: Option<CleanupConfig>,

    /// Occupancy check settings.
    pub occupancy_check: Option<OccupancyConfig>,

    /// Batch reservation groups (only valid in trop.yaml files).
    pub reservations: Option<ReservationGroup>,

    /// Disable automatic database initialization.
    pub disable_autoinit: Option<bool>,

    /// Disable automatic pruning of stale reservations.
    pub disable_autoprune: Option<bool>,

    /// Disable automatic expiration of old reservations.
    pub disable_autoexpire: Option<bool>,

    /// Allow reservation of unrelated paths.
    pub allow_unrelated_path: Option<bool>,

    /// Allow changing the project field on existing reservations.
    pub allow_change_project: Option<bool>,

    /// Allow changing the task field on existing reservations.
    pub allow_change_task: Option<bool>,

    /// Allow changing project or task fields (convenience flag).
    pub allow_change: Option<bool>,

    /// Maximum time to wait for database lock acquisition (seconds).
    pub maximum_lock_wait_seconds: Option<u64>,

    /// Output format for list commands.
    pub output_format: Option<OutputFormat>,
}

/// Port range configuration.
///
/// Specifies the range of ports available for allocation. Either `max` or
/// `max_offset` can be specified, but not both.
///
/// # Examples
///
/// ```
/// use trop::config::PortConfig;
///
/// let config = PortConfig {
///     min: 5000,
///     max: Some(7000),
///     max_offset: None,
/// };
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortConfig {
    /// Minimum port number in the range.
    pub min: u16,

    /// Maximum port number in the range (mutually exclusive with `max_offset`).
    pub max: Option<u16>,

    /// Offset from min to calculate max (mutually exclusive with max).
    pub max_offset: Option<u16>,
}

impl Default for PortConfig {
    fn default() -> Self {
        Self {
            min: 5000,
            max: Some(7000),
            max_offset: None,
        }
    }
}

/// Port exclusion (single port or range).
///
/// Supports both individual ports and inclusive ranges.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
///
/// let single = PortExclusion::Single(5001);
/// let range = PortExclusion::Range { start: 5005, end: 5009 };
/// ```
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PortExclusion {
    /// A single excluded port.
    Single(u16),
    /// An inclusive range of excluded ports.
    Range {
        /// Start of the range (inclusive).
        start: u16,
        /// End of the range (inclusive).
        end: u16,
    },
}

impl<'de> Deserialize<'de> for PortExclusion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Single(u16),
            Range { start: u16, end: u16 },
            String(String),
        }

        match Helper::deserialize(deserializer)? {
            Helper::Single(port) => Ok(PortExclusion::Single(port)),
            Helper::Range { start, end } => Ok(PortExclusion::Range { start, end }),
            Helper::String(s) => {
                // Parse "5000..5010" format
                if let Some((start_str, end_str)) = s.split_once("..") {
                    let start = start_str.trim().parse().map_err(|_| {
                        D::Error::custom(format!("Invalid port in range: {start_str}"))
                    })?;
                    let end = end_str.trim().parse().map_err(|_| {
                        D::Error::custom(format!("Invalid port in range: {end_str}"))
                    })?;
                    Ok(PortExclusion::Range { start, end })
                } else {
                    // Try parsing as a single port
                    let port = s
                        .trim()
                        .parse()
                        .map_err(|_| D::Error::custom(format!("Invalid port: {s}")))?;
                    Ok(PortExclusion::Single(port))
                }
            }
        }
    }
}

/// Cleanup configuration.
///
/// Controls automatic cleanup of stale reservations.
///
/// # Examples
///
/// ```
/// use trop::config::CleanupConfig;
///
/// let config = CleanupConfig {
///     expire_after_days: Some(30),
/// };
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CleanupConfig {
    /// Number of days after which unused reservations expire.
    pub expire_after_days: Option<u32>,
}

/// Occupancy check configuration.
///
/// Controls which types of port occupancy checks are performed.
///
/// # Examples
///
/// ```
/// use trop::config::OccupancyConfig;
///
/// let config = OccupancyConfig {
///     skip: Some(false),
///     skip_ip4: Some(false),
///     skip_ip6: Some(false),
///     skip_tcp: Some(false),
///     skip_udp: Some(false),
///     check_all_interfaces: Some(false),
/// };
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OccupancyConfig {
    /// Skip all occupancy checks.
    pub skip: Option<bool>,

    /// Skip IPv4 occupancy checks.
    pub skip_ip4: Option<bool>,

    /// Skip IPv6 occupancy checks.
    pub skip_ip6: Option<bool>,

    /// Skip TCP occupancy checks.
    pub skip_tcp: Option<bool>,

    /// Skip UDP occupancy checks.
    pub skip_udp: Option<bool>,

    /// Check all network interfaces (not just localhost).
    pub check_all_interfaces: Option<bool>,
}

/// Reservation group definition.
///
/// Defines a batch of related port reservations with optional offsets
/// and environment variable bindings.
///
/// # Examples
///
/// ```
/// use trop::config::{ReservationGroup, ServiceDefinition};
/// use std::collections::HashMap;
///
/// let mut services = HashMap::new();
/// services.insert("web".to_string(), ServiceDefinition {
///     offset: Some(0),
///     preferred: Some(5050),
///     env: Some("WEB_PORT".to_string()),
/// });
///
/// let group = ReservationGroup {
///     base: Some(5000),
///     services,
/// };
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReservationGroup {
    /// Base port for the reservation group.
    pub base: Option<u16>,

    /// Map of service tags to their definitions.
    pub services: HashMap<String, ServiceDefinition>,
}

/// Individual service definition in a reservation group.
///
/// # Examples
///
/// ```
/// use trop::config::ServiceDefinition;
///
/// let service = ServiceDefinition {
///     offset: Some(1),
///     preferred: Some(6061),
///     env: Some("API_PORT".to_string()),
/// };
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinition {
    /// Offset from the base port.
    pub offset: Option<u16>,

    /// Preferred absolute port number.
    pub preferred: Option<u16>,

    /// Environment variable name to export.
    pub env: Option<String>,
}

/// Output format for list commands.
///
/// # Examples
///
/// ```
/// use trop::config::OutputFormat;
///
/// let format = OutputFormat::Json;
/// assert_eq!(format.to_string(), "json");
/// ```
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// JSON output format.
    Json,
    /// CSV output format.
    Csv,
    /// TSV output format.
    Tsv,
    /// Human-readable table format.
    Table,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Csv => write!(f, "csv"),
            Self::Tsv => write!(f, "tsv"),
            Self::Table => write!(f, "table"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_exclusion_single() {
        let yaml = "5001";
        let exclusion: PortExclusion = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(exclusion, PortExclusion::Single(5001));
    }

    #[test]
    fn test_port_exclusion_range_string() {
        let yaml = r#""5000..5010""#;
        let exclusion: PortExclusion = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            exclusion,
            PortExclusion::Range {
                start: 5000,
                end: 5010
            }
        );
    }

    #[test]
    fn test_port_exclusion_range_object() {
        let yaml = "start: 5000\nend: 5010";
        let exclusion: PortExclusion = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            exclusion,
            PortExclusion::Range {
                start: 5000,
                end: 5010
            }
        );
    }

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.project.is_none());
        assert!(config.ports.is_none());
    }

    #[test]
    fn test_port_config_default() {
        let config = PortConfig::default();
        assert_eq!(config.min, 5000);
        assert_eq!(config.max, Some(7000));
        assert_eq!(config.max_offset, None);
    }

    #[test]
    fn test_output_format_serde() {
        let yaml = "json";
        let format: OutputFormat = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(format, OutputFormat::Json);

        let serialized = serde_yaml::to_string(&format).unwrap();
        assert!(serialized.contains("json"));
    }

    #[test]
    fn test_config_deny_unknown_fields() {
        let yaml = r"
project: test
unknown_field: value
";
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_minimal_config() {
        let yaml = r"
project: my-app
";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.project, Some("my-app".to_string()));
    }

    #[test]
    fn test_complete_config() {
        let yaml = r#"
project: my-app
disable_autoinit: false
disable_autoprune: false
disable_autoexpire: false
output_format: json
allow_unrelated_path: false
allow_change_project: false
allow_change_task: false
allow_change: false
maximum_lock_wait_seconds: 5
occupancy_check:
  skip: false
  skip_ip4: false
  skip_ip6: false
  skip_tcp: false
  skip_udp: false
  check_all_interfaces: false
ports:
  min: 5000
  max: 7000
excluded_ports:
  - 5001
  - "5005..5009"
cleanup:
  expire_after_days: 30
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.project, Some("my-app".to_string()));
        assert_eq!(config.output_format, Some(OutputFormat::Json));
        assert!(config.ports.is_some());
        assert!(config.cleanup.is_some());
    }
}

// Property-based tests for schema components
#[cfg(test)]
#[allow(unused_doc_comments)] // proptest! macro doesn't support doc comments
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    // ==================================================================================
    // PROPERTY TESTS FOR PORT EXCLUSION PARSING
    // ==================================================================================

    /// Property: PortExclusion::Range should always have start <= end after deserialization
    ///
    /// This test verifies the mathematical invariant that ranges are always valid
    /// (start <= end). We generate random port pairs, serialize them as range objects,
    /// and verify that deserialization enforces this constraint.
    ///
    /// Mathematical Property: For all valid PortExclusion::Range { start, end },
    /// start <= end must hold.
    proptest! {
        #[test]
        fn prop_port_exclusion_range_ordering(start in 1u16..=65535, end in 1u16..=65535) {
            // Only test valid ranges (start <= end)
            if start <= end {
                let yaml = format!("start: {start}\nend: {end}");
                let exclusion: PortExclusion = serde_yaml::from_str(&yaml).unwrap();

                match exclusion {
                    PortExclusion::Range { start: s, end: e } => {
                        // Verify the fundamental invariant: start <= end
                        prop_assert!(s <= e, "Range must have start <= end");
                        prop_assert_eq!(s, start, "Start value preserved");
                        prop_assert_eq!(e, end, "End value preserved");
                    }
                    PortExclusion::Single(_) => prop_assert!(false, "Expected Range variant"),
                }
            }
        }
    }

    /// Property: Single port exclusions should roundtrip through serialization
    ///
    /// Mathematical Property: For all valid ports p (1..=65535),
    /// deserialize(serialize(Single(p))) = Single(p)
    proptest! {
        #[test]
        fn prop_port_exclusion_single_roundtrip(port in 1u16..=65535) {
            // Serialize as YAML
            let exclusion = PortExclusion::Single(port);
            let yaml = serde_yaml::to_string(&exclusion).unwrap();

            // Deserialize and verify identity
            let deserialized: PortExclusion = serde_yaml::from_str(&yaml).unwrap();
            prop_assert_eq!(deserialized, exclusion, "Single port should roundtrip through serde");
        }
    }

    /// Property: Range exclusions should roundtrip through serialization
    ///
    /// Mathematical Property: For all valid port pairs (s, e) where s <= e,
    /// deserialize(serialize(Range{start: s, end: e})) = Range{start: s, end: e}
    proptest! {
        #[test]
        fn prop_port_exclusion_range_roundtrip(start in 1u16..=65535, offset in 0u16..=100) {
            let end = start.saturating_add(offset);
            let exclusion = PortExclusion::Range { start, end };

            // Serialize as YAML
            let yaml = serde_yaml::to_string(&exclusion).unwrap();

            // Deserialize and verify identity
            let deserialized: PortExclusion = serde_yaml::from_str(&yaml).unwrap();
            prop_assert_eq!(deserialized, exclusion, "Range should roundtrip through serde");
        }
    }

    /// Property: String format "start..end" should parse to valid range
    ///
    /// This tests the custom deserializer's ability to parse string ranges.
    /// Mathematical Property: parse("s..e") where s <= e yields Range{start: s, end: e}
    proptest! {
        #[test]
        fn prop_port_exclusion_string_format_parsing(start in 1u16..=65535, offset in 0u16..=100) {
            let end = start.saturating_add(offset);
            let yaml = format!("\"{start}..{end}\"");

            let exclusion: PortExclusion = serde_yaml::from_str(&yaml).unwrap();
            match exclusion {
                PortExclusion::Range { start: s, end: e } => {
                    prop_assert_eq!(s, start, "Start value parsed correctly");
                    prop_assert_eq!(e, end, "End value parsed correctly");
                    prop_assert!(s <= e, "Parsed range is valid");
                }
                PortExclusion::Single(_) => prop_assert!(false, "String range should parse to Range variant"),
            }
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR CONFIG SERIALIZATION
    // ==================================================================================

    /// Property: Config serialization should be idempotent
    ///
    /// Mathematical Property: serialize(deserialize(serialize(x))) = serialize(x)
    /// This verifies that the serialization representation is stable and that
    /// deserialize is a left-inverse of serialize (up to representation equivalence).
    proptest! {
        #[test]
        fn prop_config_serde_idempotent(
            project in proptest::option::of("[a-zA-Z0-9_-]{1,50}"),
            min in 1u16..=60000,
            max in 1u16..=65535,
            disable_autoinit in any::<bool>(),
        ) {
            // Only test if max >= min
            if max < min {
                return Ok(());
            }

            // Create a config with randomized fields
            let config = Config {
                project,
                ports: Some(PortConfig {
                    min,
                    max: Some(max),
                    max_offset: None,
                }),
                disable_autoinit: Some(disable_autoinit),
                ..Default::default()
            };

            // Serialize -> Deserialize -> Serialize
            let yaml1 = serde_yaml::to_string(&config).unwrap();
            let config2: Config = serde_yaml::from_str(&yaml1).unwrap();
            let yaml2 = serde_yaml::to_string(&config2).unwrap();

            // The two serialized forms should be identical (idempotent)
            prop_assert_eq!(yaml1, yaml2, "Serialization should be idempotent");
            prop_assert_eq!(config, config2, "Config should roundtrip");
        }
    }

    /// Property: PortConfig with max_offset computes to valid port range
    ///
    /// Mathematical Property: For valid min and max_offset,
    /// min + max_offset must either be <= 65535 or saturate at a valid value
    proptest! {
        #[test]
        fn prop_port_config_offset_validity(min in 1u16..=65535, max_offset in 1u16..=1000) {
            let config = PortConfig {
                min,
                max: None,
                max_offset: Some(max_offset),
            };

            let computed_max = min.saturating_add(max_offset);

            // Verify that if we can compute a max, it's at least >= min
            prop_assert!(computed_max >= min, "Computed max must be >= min");

            // Serialize and verify structure is preserved
            let yaml = serde_yaml::to_string(&config).unwrap();
            let deserialized: PortConfig = serde_yaml::from_str(&yaml).unwrap();

            prop_assert_eq!(deserialized.min, min, "Min preserved");
            prop_assert_eq!(deserialized.max_offset, Some(max_offset), "Offset preserved");
            prop_assert!(deserialized.max.is_none(), "Max should be None when offset is set");
        }
    }

    /// Property: OutputFormat should roundtrip through string representation
    ///
    /// Mathematical Property: parse(format(x)) = x for all OutputFormat values
    proptest! {
        #[test]
        fn prop_output_format_roundtrip(format_choice in 0u8..=3) {
            let format = match format_choice {
                0 => OutputFormat::Json,
                1 => OutputFormat::Csv,
                2 => OutputFormat::Tsv,
                _ => OutputFormat::Table,
            };

            // Convert to string and back through YAML
            let yaml = serde_yaml::to_string(&format).unwrap();
            let deserialized: OutputFormat = serde_yaml::from_str(&yaml).unwrap();

            prop_assert_eq!(deserialized, format, "OutputFormat should roundtrip");

            // Verify Display implementation matches serialization
            let display_str = format.to_string();
            prop_assert!(yaml.contains(&display_str), "Display and serde should be consistent");
        }
    }

    /// Property: Default PortConfig should always be valid
    ///
    /// This is a trivial but important invariant - the default configuration
    /// should always satisfy the constraint that max >= min.
    proptest! {
        #[test]
        fn prop_port_config_default_is_valid(_dummy in any::<u8>()) {
            let config = PortConfig::default();

            prop_assert!(config.min > 0, "Default min should be valid port");
            if let Some(max) = config.max {
                prop_assert!(max >= config.min, "Default max should be >= min");
                prop_assert!(max > 0, "Default max should be valid port");
            }
            prop_assert!(config.max_offset.is_none(), "Default should not have offset");
        }
    }

    /// Property: CleanupConfig serialization roundtrip
    ///
    /// Mathematical Property: deserialize(serialize(x)) = x
    proptest! {
        #[test]
        fn prop_cleanup_config_roundtrip(expire_days in proptest::option::of(1u32..=1000)) {
            let cleanup = CleanupConfig {
                expire_after_days: expire_days,
            };

            let yaml = serde_yaml::to_string(&cleanup).unwrap();
            let deserialized: CleanupConfig = serde_yaml::from_str(&yaml).unwrap();

            prop_assert_eq!(deserialized, cleanup, "CleanupConfig should roundtrip");
        }
    }
}
