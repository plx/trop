//! Decisions of the `trop` command-line tool.
//!
//! The `trop-cli` crate cannot reach trop's verified kernels directly, so
//! the decisions it makes (exit codes, assertion verdicts, flag derivations,
//! occupancy layers, list filters, port arguments, path display) are exposed
//! here as thin wrappers over the formally verified kernel `verified::cli`
//! and its siblings (see `docs/verification.md`). This module exists for
//! `trop-cli` and is not part of the library's stable API.

use std::path::{Path, PathBuf};

use crate::config::OccupancyConfig;
use crate::path::relationship::path_tokens;
use crate::port::occupancy::OccupancyCheckConfig;
use crate::verified::cli as kernel;
use crate::verified::occupancy::{OccFlags, OccSettings};
use crate::verified::path_relation::{home_display, HomeDisplay};
use crate::Port;

pub use crate::verified::cli::{
    assertion_outcome, exit_code_for, identifier_filter_matches, prints_reservation_port,
    process_exit_code, AssertionOutcome, ErrorClass, PortArgError, Termination,
};

/// Parses a port argument (`--port`, `--min`, `--max`, a `PORT` operand)
/// with the syntax of `u32::from_str`: a valid port (1 to 65535), or why
/// the text is not one (not a number, zero, or too large, in that order).
///
/// # Errors
///
/// Returns the reason the text is not a valid port.
///
/// # Examples
///
/// ```
/// use trop::cli_support::{parse_port_arg, PortArgError};
///
/// assert_eq!(parse_port_arg("8080"), Ok(8080));
/// assert_eq!(parse_port_arg("+80"), Ok(80));
/// assert_eq!(parse_port_arg("0"), Err(PortArgError::Zero));
/// assert_eq!(parse_port_arg("70000"), Err(PortArgError::TooLarge(70000)));
/// assert_eq!(parse_port_arg(" 80"), Err(PortArgError::NotANumber));
/// ```
pub fn parse_port_arg(text: &str) -> Result<u16, PortArgError> {
    kernel::parse_port_arg(text.as_bytes())
}

/// The `(disable_autoprune, disable_autoexpire)` switches of `trop reserve`:
/// `--disable-autoclean` turns off both automatic cleanups, and each
/// narrower flag turns off only its own.
#[must_use]
pub fn cleanup_switches(
    disable_autoprune: bool,
    disable_autoexpire: bool,
    disable_autoclean: bool,
) -> (bool, bool) {
    let switches =
        kernel::cleanup_switches(disable_autoprune, disable_autoexpire, disable_autoclean);
    (switches.disable_autoprune, switches.disable_autoexpire)
}

/// The metadata intent of a `reserve` field (project or task):
/// `--clear-*` clears it; otherwise an effective value is stored; otherwise
/// the stored value is kept (the verified `metadata_intent` decides).
#[must_use]
pub fn metadata_intent(clear: bool, value: Option<&str>) -> crate::MetadataIntent {
    use crate::verified::reserve::Intent;

    match kernel::metadata_intent(clear, value) {
        Intent::Clear => crate::MetadataIntent::Clear,
        Intent::Set(value) => crate::MetadataIntent::set(value),
        Intent::Preserve => crate::MetadataIntent::Preserve,
    }
}

/// The occupancy switches given on a command line.
// One field per command-line switch.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OccupancySwitches {
    /// `--skip-occupancy-check`.
    pub skip: bool,
    /// `--skip-ipv4`.
    pub skip_ipv4: bool,
    /// `--skip-ipv6`.
    pub skip_ipv6: bool,
    /// `--skip-tcp`.
    pub skip_tcp: bool,
    /// `--skip-udp`.
    pub skip_udp: bool,
    /// `--check-all-interfaces`.
    pub check_all_interfaces: bool,
}

/// The command line's occupancy layer: `None` when no switch is given,
/// otherwise `Some(true)` exactly for the given switches (an absent switch
/// never writes `false`, so the command line only ever disables probes,
/// apart from the wildcard probes `--check-all-interfaces` adds).
#[must_use]
pub fn occupancy_layer(switches: OccupancySwitches) -> Option<OccupancyConfig> {
    kernel::occupancy_layer(kernel::OccupancyFlags {
        skip: switches.skip,
        skip_tcp: switches.skip_tcp,
        skip_udp: switches.skip_udp,
        skip_ipv4: switches.skip_ipv4,
        skip_ipv6: switches.skip_ipv6,
        check_all_interfaces: switches.check_all_interfaces,
    })
    .map(|settings| OccupancyConfig {
        skip: settings.skip,
        skip_ip4: settings.skip_ipv4,
        skip_ip6: settings.skip_ipv6,
        skip_tcp: settings.skip_tcp,
        skip_udp: settings.skip_udp,
        check_all_interfaces: settings.check_all_interfaces,
    })
}

/// The probes `trop scan` runs under the effective occupancy settings, or
/// `None` when every probe is disabled (the ports cannot be checked).
///
/// `scan` ignores a configured global `skip` (it has no
/// `--skip-occupancy-check`, since probing is its purpose); the protocol and
/// address-family switches still apply.
#[must_use]
pub fn scan_check_config(settings: Option<&OccupancyConfig>) -> Option<OccupancyCheckConfig> {
    let settings = settings.cloned().unwrap_or_default();
    match kernel::scan_probes(OccSettings {
        skip: settings.skip,
        skip_tcp: settings.skip_tcp,
        skip_udp: settings.skip_udp,
        skip_ipv4: settings.skip_ip4,
        skip_ipv6: settings.skip_ip6,
        check_all_interfaces: settings.check_all_interfaces,
    }) {
        kernel::ScanProbes::NotChecked => None,
        kernel::ScanProbes::Probe(OccFlags {
            skip_tcp,
            skip_udp,
            skip_ipv4,
            skip_ipv6,
            check_all_interfaces,
        }) => Some(OccupancyCheckConfig {
            skip_tcp,
            skip_udp,
            skip_ipv4,
            skip_ipv6,
            check_all_interfaces,
        }),
    }
}

/// The occupied ports `scan --autoexclude` may exclude: those not reserved,
/// in their original order. No reserved port is ever returned.
#[must_use]
pub fn unreserved_ports(occupied: &[Port], reserved: &[Port]) -> Vec<Port> {
    let occupied_values: Vec<u16> = occupied.iter().map(|port| port.value()).collect();
    let reserved_values: Vec<u16> = reserved.iter().map(|port| port.value()).collect();
    crate::verified::exclusions::unreserved_ports(&occupied_values, &reserved_values)
        .into_iter()
        .map(|value| {
            // Every returned value is one of `occupied`'s ports.
            Port::try_from(value).expect("the kernel returns only values of the given ports")
        })
        .collect()
}

/// One CSV or TSV record: the fields joined by `delimiter`, each written
/// verbatim unless it contains the delimiter, `"`, `\r` or `\n`, in which
/// case it is quoted with inner quotes doubled, followed by `\n` (a record
/// of no fields or of one empty field is written `""`). This is byte for
/// byte what the
/// `csv` crate's default writer produces, and the verified kernel proves
/// that a strict RFC 4180 reader reads every such record back as exactly
/// its fields (for a delimiter other than `"`, `\r`, `\n`).
///
/// # Examples
///
/// ```
/// use trop::cli_support::delimited_record;
///
/// assert_eq!(delimited_record(&["5001", "/srv/a,b", "say \"hi\""], ','), "5001,\"/srv/a,b\",\"say \"\"hi\"\"\"\n");
/// assert_eq!(delimited_record(&["a", "b\tc"], '\t'), "a\t\"b\tc\"\n");
/// assert_eq!(delimited_record(&[""], ','), "\"\"\n");
/// assert_eq!(delimited_record(&[], ','), "\"\"\n");
/// ```
#[must_use]
pub fn delimited_record(fields: &[&str], delimiter: char) -> String {
    crate::verified::render::delimited_record(fields, delimiter)
}

/// Removes leading and trailing Unicode whitespace exactly as identifiers
/// are trimmed when stored (identical to `str::trim`).
#[must_use]
pub fn trim_identifier(value: &str) -> &str {
    crate::identifier::trim_identifier(value)
}

/// Whether `TROP_LOG_MODE` is set to something that is not a level name
/// (`quiet`, `normal`, `verbose`, in any ASCII case). Such a value is
/// ignored; the CLI warns about it. An empty value counts as unset (it is
/// ignored like an invalid one but not diagnosed).
#[must_use]
pub fn invalid_log_mode(log_mode: Option<&str>) -> bool {
    crate::verified::verbosity::invalid_log_mode(log_mode.map(str::as_bytes))
}

/// `path` shortened relative to the home directory `home`, compared
/// component by component (like `Path::strip_prefix`): `~/` for the home
/// directory itself, `~/rest` below it, and `None` for any other path (and
/// for an empty `home`).
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use trop::cli_support::home_relative_display;
///
/// let home = Path::new("/home/al");
/// assert_eq!(home_relative_display(Path::new("/home/al/src/x"), home), Some("~/src/x".into()));
/// assert_eq!(home_relative_display(Path::new("/home/al"), home), Some("~/".into()));
/// assert_eq!(home_relative_display(Path::new("/home/alice/x"), home), None);
/// ```
#[must_use]
pub fn home_relative_display(path: &Path, home: &Path) -> Option<String> {
    match home_display(&path_tokens(path), &path_tokens(home)) {
        // The home directory itself, shown as before: `~/`.
        HomeDisplay::Home => Some("~/".to_string()),
        HomeDisplay::Below { rest_start } => {
            let rest: PathBuf = path.components().skip(rest_start).collect();
            Some(format!("~/{}", rest.display()))
        }
        HomeDisplay::Elsewhere => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_arguments_match_u32_from_str_and_the_port_range() {
        for (text, expected) in [
            ("1", Ok(1)),
            ("65535", Ok(65535)),
            ("+65535", Ok(65535)),
            ("0080", Ok(80)),
            ("0", Err(PortArgError::Zero)),
            ("+0", Err(PortArgError::Zero)),
            ("000", Err(PortArgError::Zero)),
            ("65536", Err(PortArgError::TooLarge(65536))),
            ("4294967295", Err(PortArgError::TooLarge(u32::MAX))),
            ("4294967296", Err(PortArgError::NotANumber)),
            ("", Err(PortArgError::NotANumber)),
            ("+", Err(PortArgError::NotANumber)),
            ("-1", Err(PortArgError::NotANumber)),
            (" 1", Err(PortArgError::NotANumber)),
            ("1 ", Err(PortArgError::NotANumber)),
            ("0x10", Err(PortArgError::NotANumber)),
            ("８０", Err(PortArgError::NotANumber)),
        ] {
            assert_eq!(parse_port_arg(text), expected, "{text:?}");
            // Differential check against `u32::from_str` and the port range.
            let reference = match text.parse::<u32>() {
                Err(_) => Err(PortArgError::NotANumber),
                Ok(0) => Err(PortArgError::Zero),
                Ok(value) => u16::try_from(value).map_err(|_| PortArgError::TooLarge(value)),
            };
            assert_eq!(parse_port_arg(text), reference, "{text:?}");
        }
    }

    #[test]
    fn cleanup_switches_combine_autoclean_with_each_flag() {
        for prune in [false, true] {
            for expire in [false, true] {
                for clean in [false, true] {
                    assert_eq!(
                        cleanup_switches(prune, expire, clean),
                        (prune || clean, expire || clean)
                    );
                }
            }
        }
    }

    #[test]
    fn metadata_intents_follow_the_clear_flag_then_the_value() {
        use crate::MetadataIntent;
        assert_eq!(metadata_intent(true, Some("p")), MetadataIntent::Clear);
        assert_eq!(metadata_intent(true, None), MetadataIntent::Clear);
        assert_eq!(metadata_intent(false, Some("p")), MetadataIntent::set("p"));
        assert_eq!(metadata_intent(false, None), MetadataIntent::Preserve);
    }

    #[test]
    fn occupancy_layer_sets_exactly_the_given_switches() {
        assert_eq!(occupancy_layer(OccupancySwitches::default()), None);
        let layer = occupancy_layer(OccupancySwitches {
            skip_tcp: true,
            check_all_interfaces: true,
            ..OccupancySwitches::default()
        })
        .unwrap();
        assert_eq!(
            layer,
            OccupancyConfig {
                skip_tcp: Some(true),
                check_all_interfaces: Some(true),
                ..OccupancyConfig::default()
            }
        );
        // Every switch maps to its own leaf (ip4/ip6 naming included).
        for bit in 0..6 {
            let switches = OccupancySwitches {
                skip: bit == 0,
                skip_ipv4: bit == 1,
                skip_ipv6: bit == 2,
                skip_tcp: bit == 3,
                skip_udp: bit == 4,
                check_all_interfaces: bit == 5,
            };
            let layer = occupancy_layer(switches).unwrap();
            let leaves = [
                layer.skip,
                layer.skip_ip4,
                layer.skip_ip6,
                layer.skip_tcp,
                layer.skip_udp,
                layer.check_all_interfaces,
            ];
            for (index, leaf) in leaves.iter().enumerate() {
                assert_eq!(*leaf, (index == bit).then_some(true), "{switches:?}");
            }
        }
    }

    #[test]
    fn scan_ignores_only_the_global_skip() {
        let all = OccupancyCheckConfig::default();
        assert_eq!(scan_check_config(None), Some(all.clone()));
        let skip_all = OccupancyConfig {
            skip: Some(true),
            ..OccupancyConfig::default()
        };
        assert_eq!(scan_check_config(Some(&skip_all)), Some(all));
        let skip_tcp = OccupancyConfig {
            skip: Some(true),
            skip_tcp: Some(true),
            skip_ip6: Some(true),
            check_all_interfaces: Some(true),
            ..OccupancyConfig::default()
        };
        assert_eq!(
            scan_check_config(Some(&skip_tcp)),
            Some(OccupancyCheckConfig {
                skip_tcp: true,
                skip_ipv6: true,
                check_all_interfaces: true,
                ..OccupancyCheckConfig::default()
            })
        );
        for nothing_left in [
            OccupancyConfig {
                skip_tcp: Some(true),
                skip_udp: Some(true),
                ..OccupancyConfig::default()
            },
            OccupancyConfig {
                skip_ip4: Some(true),
                skip_ip6: Some(true),
                skip: Some(false),
                ..OccupancyConfig::default()
            },
        ] {
            assert_eq!(scan_check_config(Some(&nothing_left)), None);
        }
    }

    #[test]
    fn unreserved_ports_keep_order_and_drop_reserved() {
        let ports = |values: &[u16]| -> Vec<Port> {
            values
                .iter()
                .map(|value| Port::try_from(*value).unwrap())
                .collect()
        };
        assert_eq!(
            unreserved_ports(&ports(&[7, 3, 5, 3, 65535, 1]), &ports(&[5, 1])),
            ports(&[7, 3, 3, 65535])
        );
        assert_eq!(unreserved_ports(&[], &ports(&[1])), Vec::<Port>::new());
        assert_eq!(unreserved_ports(&ports(&[2]), &[]), ports(&[2]));
    }

    #[test]
    fn identifier_filters_compare_canonical_values() {
        assert!(identifier_filter_matches("web", Some("web")));
        assert!(identifier_filter_matches(" web\t", Some("web")));
        assert!(identifier_filter_matches("\u{3000}web\u{a0}", Some("web")));
        assert!(!identifier_filter_matches("web", Some("webs")));
        assert!(!identifier_filter_matches("web", None));
        assert!(!identifier_filter_matches("", Some("")));
        assert!(!identifier_filter_matches("  ", Some("")));
        assert!(!identifier_filter_matches(" ", None));
        assert!(identifier_filter_matches("naïve proj", Some("naïve proj")));
    }

    #[test]
    fn log_mode_diagnosis() {
        assert!(!invalid_log_mode(None));
        for valid in ["quiet", "NORMAL", "Verbose"] {
            assert!(!invalid_log_mode(Some(valid)), "{valid}");
        }
        assert!(!invalid_log_mode(Some("")), "empty counts as unset");
        for invalid in [" ", " quiet", "loud", "quiet\n", "\u{fffd}"] {
            assert!(invalid_log_mode(Some(invalid)), "{invalid:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn home_relative_display_is_component_wise() {
        let home = Path::new("/home/al");
        let show = |path: &str| home_relative_display(Path::new(path), home);
        assert_eq!(show("/home/al"), Some("~/".to_string()));
        assert_eq!(show("/home/al/"), Some("~/".to_string()));
        assert_eq!(show("/home/al/x/y"), Some("~/x/y".to_string()));
        assert_eq!(show("/home/al/./x"), Some("~/x".to_string()));
        assert_eq!(show("/home/alice"), None);
        assert_eq!(show("/home"), None);
        assert_eq!(show("/usr/local/bin"), None);
        assert_eq!(home_relative_display(Path::new("/x"), Path::new("")), None);
        assert_eq!(
            home_relative_display(Path::new("/usr/bin"), Path::new("/")),
            Some("~/usr/bin".to_string())
        );
        // The shortened form agrees with `Path::strip_prefix`.
        for path in ["/home/al/a", "/home/al/a/b/c", "/home/alx", "/home/al"] {
            let path = Path::new(path);
            // Exactly the shortening `trop list` used before.
            let expected = path
                .strip_prefix(home)
                .ok()
                .map(|rest| format!("~/{}", rest.display()));
            assert_eq!(home_relative_display(path, home), expected, "{path:?}");
        }
    }
}
