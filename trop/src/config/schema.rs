//! Configuration schema definitions.
//!
//! This module defines the complete configuration structure for trop,
//! including all settings for ports, exclusions, cleanup, and reservation groups.

use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::identifier::trim_identifier;
use crate::verified::config::{output_format_keyword, parse_output_format, OutputFormatCode};
use crate::verified::exclusions::{find_range_separator, parse_u16};
use crate::verified::port_bounds::{
    check_override_bounds, default_port_bounds, PortBounds, PortBoundsViolation,
};

/// Default minimum port for port allocation ranges.
///
/// The built-in range `DEFAULT_MIN_PORT..=DEFAULT_MAX_PORT` is defined once,
/// in the verified port-bounds kernel, which proves it valid.
pub const DEFAULT_MIN_PORT: u16 = crate::verified::port_bounds::DEFAULT_MIN_PORT;

/// Default maximum port for port allocation ranges.
pub const DEFAULT_MAX_PORT: u16 = crate::verified::port_bounds::DEFAULT_MAX_PORT;

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
/// When configuration sources are layered (see
/// [`crate::config::ConfigBuilder`]), a `max_offset` that resolves at the
/// `min` in effect at the layer that sets it is stored as the absolute `max`
/// it names, so a higher layer that changes only `min` never moves it.
/// `ConfigBuilder` checks every layer below the command line on its own
/// (configuration files and the programmatic layer are validated, the
/// environment's upper bound must resolve), and validates the result, so
/// every configuration it builds, and every [`crate::config::EffectiveConfig`]
/// after a successful `apply_cli_config`, holds `max` and no `max_offset`.
/// (A bare [`crate::config::ConfigMerger`] merge validates nothing: there, an
/// unresolvable `max_offset` is kept as written, and a later `min`-only
/// section re-evaluates it at its new `min`.)
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
        Self::from_bounds(default_port_bounds())
    }
}

impl PortConfig {
    /// The section's bounds as the verified port-bounds kernel sees them
    /// (a field-for-field copy).
    pub(crate) const fn bounds(&self) -> PortBounds {
        PortBounds {
            min: self.min,
            max: self.max,
            max_offset: self.max_offset,
        }
    }

    /// The section holding `bounds` (a field-for-field copy).
    pub(crate) const fn from_bounds(bounds: PortBounds) -> Self {
        Self {
            min: bounds.min,
            max: bounds.max,
            max_offset: bounds.max_offset,
        }
    }

    /// Check command-line `--min` / `--max` port overrides.
    ///
    /// Each given bound must be a valid port (1-65535) and, when both are
    /// given, `min <= max` — the rules a `ports` section with these bounds
    /// must satisfy. The decision is made by a verified kernel (see
    /// `docs/verification.md`). Used by the `trop` CLI; not a stable API.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Validation`] whose message names the offending
    /// flag.
    #[doc(hidden)]
    pub fn check_command_line_bounds(
        min: Option<u16>,
        max: Option<u16>,
    ) -> crate::error::Result<()> {
        const PORT_ZERO: &str = "Invalid port number: port 0 is reserved and cannot be used";
        check_override_bounds(min, max).map_err(|violation| {
            let (field, message) = match violation {
                PortBoundsViolation::InvalidMin => {
                    ("ports.min", format!("invalid --min value: {PORT_ZERO}"))
                }
                PortBoundsViolation::InvalidMax => {
                    ("ports.max", format!("invalid --max value: {PORT_ZERO}"))
                }
                // `MaxBelowMin` is reported only when both bounds are given.
                // The kernel never reports the `max_offset` violations for
                // command-line overrides (they set no offset).
                PortBoundsViolation::MaxBelowMin
                | PortBoundsViolation::ZeroOffset
                | PortBoundsViolation::OffsetOverflow { .. }
                | PortBoundsViolation::BothMaxAndOffset => (
                    "ports",
                    format!(
                        "Invalid port range: min ({}) must be less than or equal to max ({})",
                        min.unwrap_or_default(),
                        max.unwrap_or_default()
                    ),
                ),
            };
            crate::error::Error::Validation {
                field: field.into(),
                message,
            }
        })
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
            // "5001" or "5000..5010": the shared exclusion-text parser.
            Helper::String(s) => s.parse().map_err(D::Error::custom),
        }
    }
}

/// Error returned when text is neither a port (`"5001"`) nor an inclusive
/// port range (`"5000..5010"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsePortExclusionError {
    message: String,
}

impl fmt::Display for ParsePortExclusionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParsePortExclusionError {}

impl FromStr for PortExclusion {
    type Err = ParsePortExclusionError;

    /// Parses a port (`"5001"`) or an inclusive port range (`"5000..5010"`).
    ///
    /// This one parser serves the YAML string form, `TROP_EXCLUDED_PORTS`,
    /// and `trop exclude`. The text splits at its first `..`; whitespace
    /// around the whole text and around each side of `..` is ignored. Each
    /// number has the verified kernel's syntax (an optional `+`, then ASCII
    /// digits, at most 65535); that this is also `u16::from_str`'s syntax is
    /// trusted, pinned by `test_parse_u16_kernel_matches_from_str`. The
    /// separator search, the trimming (Unicode `White_Space`, the verified
    /// trim the identifier rule uses, equal to `str::trim`) and the number
    /// parsing are verified kernels. Parsing does not validate:
    /// `"0"` and `"5010..5000"` parse, and validation rejects them.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::config::PortExclusion;
    ///
    /// assert_eq!("5001".parse(), Ok(PortExclusion::Single(5001)));
    /// assert_eq!(
    ///     " 5000 .. 5010 ".parse(),
    ///     Ok(PortExclusion::Range { start: 5000, end: 5010 })
    /// );
    /// assert!("5000..".parse::<PortExclusion>().is_err());
    /// ```
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let port = |part: &str| parse_u16(trim_identifier(part).as_bytes());
        if let Some(at) = find_range_separator(text.as_bytes()) {
            // `text[at..at + 2]` is the ASCII `..`, so both slice bounds are
            // character boundaries.
            let (start_text, end_text) = (&text[..at], &text[at + 2..]);
            let invalid = |part: &str| ParsePortExclusionError {
                message: format!("Invalid port in range: {part}"),
            };
            let start = port(start_text).ok_or_else(|| invalid(start_text))?;
            let end = port(end_text).ok_or_else(|| invalid(end_text))?;
            Ok(Self::Range { start, end })
        } else {
            port(text)
                .map(Self::Single)
                .ok_or_else(|| ParsePortExclusionError {
                    message: format!("Invalid port: {text}"),
                })
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

    /// Map of broad service tags to their definitions.
    ///
    /// When a service omits [`ServiceDefinition::env`], its tag must be ASCII
    /// and produce a portable environment-variable name after ASCII
    /// case conversion and replacing hyphens with underscores. Resolved names
    /// must be unique under ASCII-case-insensitive comparison.
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
    /// Offset from the base port, defaulting to zero when omitted.
    ///
    /// When [`Self::preferred`] is also present, this is the fallback used if
    /// the preferred port is reserved, excluded, or occupied.
    pub offset: Option<u16>,

    /// Preferred absolute port number, which may be outside the scan range.
    pub preferred: Option<u16>,

    /// Explicit environment-variable name to export.
    ///
    /// Names must be at most 255 bytes and match
    /// `[A-Za-z_][A-Za-z0-9_]*`.
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

impl OutputFormat {
    /// The verified kernel's code for this format (a variant-for-variant
    /// copy).
    pub(crate) const fn code(self) -> OutputFormatCode {
        match self {
            Self::Json => OutputFormatCode::Json,
            Self::Csv => OutputFormatCode::Csv,
            Self::Tsv => OutputFormatCode::Tsv,
            Self::Table => OutputFormatCode::Table,
        }
    }

    /// The format with the verified kernel's `code` (a variant-for-variant
    /// copy).
    pub(crate) const fn from_code(code: OutputFormatCode) -> Self {
        match code {
            OutputFormatCode::Json => Self::Json,
            OutputFormatCode::Csv => Self::Csv,
            OutputFormatCode::Tsv => Self::Tsv,
            OutputFormatCode::Table => Self::Table,
        }
    }

    /// Parses a format keyword (`json`, `csv`, `tsv`, `table`), ignoring
    /// ASCII case, as `TROP_OUTPUT_FORMAT` does. The decision is the verified
    /// kernel's, which proves that every format's `Display` text parses back
    /// to it.
    pub(crate) fn parse_keyword(text: &str) -> Option<Self> {
        parse_output_format(text.as_bytes()).map(Self::from_code)
    }
}

impl std::fmt::Display for OutputFormat {
    /// Writes the format's keyword (`json`, `csv`, `tsv`, or `table`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(output_format_keyword(self.code()))
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

    // ------------------------------------------------------------------
    // The unified exclusion-text parser.
    // ------------------------------------------------------------------

    /// The YAML string-form parser before unification.
    fn previous_yaml_parse(text: &str) -> Option<PortExclusion> {
        if let Some((start, end)) = text.split_once("..") {
            Some(PortExclusion::Range {
                start: start.trim().parse().ok()?,
                end: end.trim().parse().ok()?,
            })
        } else {
            text.trim().parse().ok().map(PortExclusion::Single)
        }
    }

    /// The `TROP_EXCLUDED_PORTS` item parser before unification (applied to
    /// the trimmed item).
    fn previous_env_parse(item: &str) -> Option<PortExclusion> {
        if let Some((start, end)) = item.split_once("..") {
            Some(PortExclusion::Range {
                start: start.parse().ok()?,
                end: end.parse().ok()?,
            })
        } else {
            item.parse().ok().map(PortExclusion::Single)
        }
    }

    /// The `trop exclude` argument parser before unification.
    fn previous_cli_parse(text: &str) -> Option<PortExclusion> {
        if let Some(at) = text.find("..") {
            Some(PortExclusion::Range {
                start: text[..at].parse().ok()?,
                end: text[at + 2..].parse().ok()?,
            })
        } else {
            text.parse().ok().map(PortExclusion::Single)
        }
    }

    /// Every string of up to four characters over an alphabet with digits,
    /// signs, dots, ASCII and Unicode whitespace, and a letter, plus longer
    /// hand-picked cases.
    fn exclusion_text_corpus() -> Vec<String> {
        let alphabet = [
            '0', '1', '5', '9', '+', '-', '.', ' ', '\t', 'a', '\u{a0}', '\u{3000}',
        ];
        let mut corpus = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..4 {
            let mut next = Vec::new();
            for prefix in &frontier {
                for c in alphabet {
                    let mut text = prefix.clone();
                    text.push(c);
                    next.push(text);
                }
            }
            corpus.extend(next.iter().cloned());
            frontier = next;
        }
        for text in [
            "5000..5010",
            " 5000 .. 5010 ",
            "5000 ..5010",
            "\u{a0}5000..5010\u{3000}",
            "+5000..+5010",
            "0005000",
            "65535",
            "65536",
            "99999999999999999999",
            "00000000000000065535",
            "1..2..3",
            "...",
            "5000...5010",
            "5000..",
            "..5010",
            "-1",
            "+",
            "5000..5000",
            "5010..5000",
            "0..0",
        ] {
            corpus.push(text.to_string());
        }
        corpus
    }

    #[test]
    fn test_unified_parser_is_exactly_the_previous_yaml_parser() {
        for text in exclusion_text_corpus() {
            assert_eq!(
                text.parse::<PortExclusion>().ok(),
                previous_yaml_parse(&text),
                "{text:?}"
            );
        }
    }

    /// Regression for the divergent parsers: everything the previous
    /// `TROP_EXCLUDED_PORTS` and `trop exclude` parsers accepted still parses
    /// to the same entry, and whitespace around `..` is now accepted by all.
    #[test]
    fn test_unified_parser_accepts_everything_previously_accepted() {
        for text in exclusion_text_corpus() {
            let parsed = text.parse::<PortExclusion>().ok();
            if let Some(previous) = previous_env_parse(text.trim()) {
                assert_eq!(parsed, Some(previous), "env {text:?}");
            }
            if let Some(previous) = previous_cli_parse(&text) {
                assert_eq!(parsed, Some(previous), "cli {text:?}");
            }
        }
        assert_eq!(previous_env_parse("5000 .. 5010"), None);
        assert_eq!(previous_cli_parse(" 5000"), None);
        assert_eq!(
            "5000 .. 5010".parse(),
            Ok(PortExclusion::Range {
                start: 5000,
                end: 5010
            })
        );
        assert_eq!(" 5000".parse(), Ok(PortExclusion::Single(5000)));
    }

    #[test]
    fn test_parse_u16_kernel_matches_from_str() {
        use crate::verified::exclusions::parse_u16;
        let check = |text: &str| {
            assert_eq!(
                parse_u16(text.as_bytes()),
                text.parse::<u16>().ok(),
                "{text:?}"
            );
        };
        for value in 0..=70_000_u32 {
            check(&value.to_string());
            check(&format!("+{value}"));
            check(&format!("00{value}"));
            check(&format!("-{value}"));
            check(&format!(" {value}"));
            check(&format!("{value}."));
        }
        for text in exclusion_text_corpus() {
            check(&text);
        }
    }

    /// The exclusion-text parser trims with the verified whitespace trim
    /// (`crate::identifier::trim_identifier`). Pins that this is exactly the
    /// previous `str::trim` behavior: every Unicode scalar value, as padding
    /// around a port and around each side of `..`, parses as the `str::trim`
    /// reference does.
    #[test]
    fn test_exclusion_text_trim_matches_str_trim() {
        fn reference(text: &str) -> Option<PortExclusion> {
            let port = |part: &str| part.trim().parse::<u16>().ok();
            match text.split_once("..") {
                Some((start, end)) => Some(PortExclusion::Range {
                    start: port(start)?,
                    end: port(end)?,
                }),
                None => port(text).map(PortExclusion::Single),
            }
        }
        let mut whitespace = 0;
        for c in (0..=0x0010_FFFF_u32).filter_map(char::from_u32) {
            if c.is_whitespace() {
                whitespace += 1;
            }
            for text in [
                format!("{c}8080{c}"),
                format!("{c}{c}8080..{c}8090{c}{c}"),
                format!("8080{c}..8090"),
            ] {
                assert_eq!(text.parse().ok(), reference(&text), "{text:?}");
            }
        }
        // Every `White_Space` code point was exercised (and accepted).
        assert_eq!(whitespace, 25);
        assert_eq!(
            "\u{3000}8080\u{85}..\u{2029}8090\u{a0}".parse(),
            Ok(PortExclusion::Range {
                start: 8080,
                end: 8090
            })
        );
        // Zero-width and BOM characters are not whitespace.
        assert!("\u{200b}8080".parse::<PortExclusion>().is_err());
        assert!("\u{feff}8080".parse::<PortExclusion>().is_err());
    }

    #[test]
    fn test_exclusion_text_round_trip() {
        for value in 0..=u16::MAX {
            assert_eq!(value.to_string().parse(), Ok(PortExclusion::Single(value)));
        }
        for (start, end) in [(0, 0), (1, 65_535), (5000, 5010), (65_535, 1), (7, 7)] {
            assert_eq!(
                format!("{start}..{end}").parse(),
                Ok(PortExclusion::Range { start, end })
            );
        }
    }

    #[test]
    fn test_exclusion_text_error_messages() {
        let message = |text: &str| text.parse::<PortExclusion>().unwrap_err().to_string();
        assert_eq!(message("abc"), "Invalid port: abc");
        assert_eq!(message(" 70000 "), "Invalid port:  70000 ");
        assert_eq!(message("x ..5010"), "Invalid port in range: x ");
        assert_eq!(message("5000.. y"), "Invalid port in range:  y");
        assert_eq!(message("1..2..3"), "Invalid port in range: 2..3");

        let yaml_error = serde_yaml::from_str::<PortExclusion>(r#""5000..x""#)
            .unwrap_err()
            .to_string();
        assert!(
            yaml_error.contains("Invalid port in range: x"),
            "{yaml_error}"
        );
    }
}

// Property-based tests for schema components
#[cfg(all(test, feature = "property-tests"))]
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
