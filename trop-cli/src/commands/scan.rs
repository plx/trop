//! Command to scan port range for occupied ports.

use crate::commands::exclude::{current_exclusions, read_config_document, write_exclusions};
use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::fmt::Write as _;
use trop::cli_support::{delimited_record, scan_check_config, unreserved_ports};
use trop::config::{Config, PortConfig};
use trop::output::display_field;
use trop::port::allocator::port_range_from_config;
use trop::port::exclusions::{add_ports_to_list, compact_exclusions, is_compact, WriteScope};
use trop::port::occupancy::{OccupancyReport, OccupiedProbe, SystemOccupancyChecker};
use trop::{Database, Port, PortRange};

/// Scan port range for occupied ports.
#[derive(Args)]
pub struct ScanCommand {
    /// Minimum port (uses config if not specified)
    #[arg(long, value_name = "MIN")]
    pub min: Option<String>,

    /// Maximum port (uses config if not specified)
    #[arg(long, value_name = "MAX")]
    pub max: Option<String>,

    /// Automatically add occupied, unreserved ports to exclusion list
    #[arg(long)]
    pub autoexclude: bool,

    /// Automatically compact exclusions after adding
    #[arg(long)]
    pub autocompact: bool,

    /// Output format
    #[arg(long, value_enum, default_value = "table")]
    pub format: ScanOutputFormat,

    // Occupancy check options
    #[arg(long)]
    pub skip_tcp: bool,

    #[arg(long)]
    pub skip_udp: bool,

    #[arg(long)]
    pub skip_ipv4: bool,

    #[arg(long)]
    pub skip_ipv6: bool,

    #[arg(long)]
    pub check_all_interfaces: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ScanOutputFormat {
    Table,
    Json,
    Csv,
    Tsv,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ScanResult {
    port: u16,
    status: String,
    reserved: bool,
    protocol: String,
    address_family: String,
    scope: String,
    address: String,
    process_id: Option<u32>,
    process_name: Option<String>,
    user: Option<String>,
    owner_status: String,
}

impl ScanResult {
    fn from_probe(report: &OccupancyReport, probe: &OccupiedProbe, reserved: bool) -> Self {
        Self {
            port: report.port.value(),
            status: if reserved {
                "occupied (reserved)".to_string()
            } else {
                "occupied".to_string()
            },
            reserved,
            protocol: probe.protocol.as_str().to_string(),
            address_family: probe.address_family.as_str().to_string(),
            scope: probe.scope.as_str().to_string(),
            address: probe.address.to_string(),
            process_id: probe.owner.process_id,
            process_name: probe.owner.process_name.clone(),
            user: probe.owner.user.clone(),
            owner_status: probe.owner.status.as_str().to_string(),
        }
    }
}

impl ScanCommand {
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 1. Load configuration and determine port range
        let config = context.config()?;
        let range = self.determine_range(config)?;

        // 2. Open database
        let db = context.open_database()?;

        // 3. Scan for occupied ports. Scanning is this command's purpose, so
        // a configured global `skip` (meant for allocations) does not apply;
        // the protocol and family switches do. When they leave no probe, the
        // ports cannot be checked, and saying "nothing is occupied" would be
        // false, so the scan fails instead (the verified
        // `scan_check_config` decides both).
        let checker = SystemOccupancyChecker;
        let check_config = scan_check_config(config.occupancy_check.as_ref()).ok_or_else(|| {
            CliError::Config(
                "ports were not checked: the effective occupancy settings disable every \
                 probe (both TCP and UDP, or both IPv4 and IPv6, are skipped)"
                    .to_string(),
            )
        })?;

        let reports = checker
            .find_occupied_reports(&range, &check_config)
            .map_err(CliError::from)?;
        let occupied_ports = reports.iter().map(|report| report.port).collect::<Vec<_>>();

        // 4. Get reserved ports from database
        let reserved_ports = Database::get_reserved_ports_in_range(db.connection(), &range)
            .map_err(CliError::from)?;

        // 5. Find unreserved occupied ports (verified: never a reserved port,
        // so `--autoexclude` never excludes a reserved port).
        let unreserved_occupied: Vec<Port> = unreserved_ports(&occupied_ports, &reserved_ports);

        // 6. Auto-exclude (and auto-compact) if requested
        if self.autoexclude && !unreserved_occupied.is_empty() {
            self.add_exclusions(&unreserved_occupied, context)?;
        }

        // 7. Format and output results
        self.output_results(&reports, &reserved_ports, &unreserved_occupied)?;

        Ok(())
    }

    fn determine_range(&self, config: &Config) -> Result<PortRange, CliError> {
        // Resolve the configured bounds exactly as the allocator does: an
        // explicit `max`, or `min + max_offset`. Without a `ports` section the
        // built-in default range applies.
        let default_ports = PortConfig::default();
        let ports = config.ports.as_ref().unwrap_or(&default_ports);
        port_range_from_config(ports).map_err(|error| match error {
            trop::Error::InvalidPort { .. } => CliError::InvalidArguments(error.to_string()),
            other => CliError::Library(other),
        })
    }

    /// Excludes the occupied, unreserved `ports` in the write target (the
    /// loaded `trop.local.yaml` or `trop.yaml`, else the global
    /// `config.yaml`) and, with `--autocompact`, compacts the list when it
    /// changed. The file is rewritten at most once, and only its
    /// `excluded_ports` key changes.
    fn add_exclusions(&self, ports: &[Port], context: &InvocationContext) -> Result<(), CliError> {
        let config_path = context.config_file_for_write(WriteScope::ProjectOrGlobal)?;
        let document = read_config_document(&config_path)?;
        let current = current_exclusions(&config_path, &document)?;
        let quiet = context.global().quiet;

        // Ports the list already excludes are skipped; `added` counts the
        // ports newly excluded (verified by the exclusions kernel).
        let (mut updated, added) = add_ports_to_list(&current, ports);
        if added == 0 {
            if !quiet {
                eprintln!("Added 0 exclusions to {}", config_path.display());
            }
            return Ok(());
        }

        // `--autocompact` compacts the list because it changed. Invalid
        // entries are reported; the list is rewritten in canonical form when
        // it is not already canonical. (The loader validates the write
        // target's list before any command runs, so the error is a defensive
        // check.)
        let mut compaction = None;
        if self.autocompact {
            let compacted = compact_exclusions(&updated).map_err(|error| {
                CliError::Config(format!(
                    "cannot compact the exclusions in {}: {error}",
                    config_path.display()
                ))
            })?;
            if !is_compact(&updated) {
                compaction = Some((updated.len(), compacted.len()));
                updated = compacted;
            }
        }

        // Report only once the file has been written.
        write_exclusions(&config_path, &document, &updated)?;
        if !quiet {
            eprintln!("Added {added} exclusions to {}", config_path.display());
            if let Some((before, after)) = compaction {
                eprintln!("Compacted {before} exclusions to {after}");
            }
        }
        Ok(())
    }

    fn output_results(
        &self,
        reports: &[OccupancyReport],
        reserved: &[Port],
        unreserved: &[Port],
    ) -> Result<(), CliError> {
        let mut results = Vec::new();
        for report in reports {
            let is_reserved = reserved.contains(&report.port);
            results.extend(
                report
                    .occupied_probes
                    .iter()
                    .map(|probe| ScanResult::from_probe(report, probe, is_reserved)),
            );
        }

        // Format based on requested output format
        match self.format {
            ScanOutputFormat::Table => {
                print!("{}", render_table(&results));
            }
            ScanOutputFormat::Json => {
                let json = serde_json::to_string_pretty(&results)
                    .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
                println!("{json}");
            }
            ScanOutputFormat::Csv => {
                print!("{}", render_delimited(&results, ','));
            }
            ScanOutputFormat::Tsv => {
                print!("{}", render_delimited(&results, '\t'));
            }
        }

        if !unreserved.is_empty() {
            eprintln!();
            eprintln!("Found {} unreserved occupied port(s)", unreserved.len());
        }

        Ok(())
    }
}

fn optional_display<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
}

/// An owner field reported by the operating system (process name, user),
/// escaped for the table so it cannot break the row's line or columns.
fn owner_cell(value: Option<&str>) -> String {
    value.map_or_else(|| "unavailable".to_string(), display_field)
}

fn render_table(results: &[ScanResult]) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "{:<6} {:<20} {:<9} {:<9} {:<7} {:<10} {:<15} {:<12} {:<16} {:<16} Owner Status",
        "Port",
        "Status",
        "Reserved",
        "Protocol",
        "Family",
        "Scope",
        "Address",
        "Process ID",
        "Process",
        "User",
    )
    .expect("writing to a String cannot fail");
    writeln!(output, "{}", "-".repeat(145)).expect("writing to a String cannot fail");
    for result in results {
        writeln!(
            output,
            "{:<6} {:<20} {:<9} {:<9} {:<7} {:<10} {:<15} {:<12} {:<16} {:<16} {}",
            result.port,
            result.status,
            result.reserved,
            result.protocol,
            result.address_family,
            result.scope,
            result.address,
            optional_display(result.process_id),
            owner_cell(result.process_name.as_deref()),
            owner_cell(result.user.as_deref()),
            result.owner_status,
        )
        .expect("writing to a String cannot fail");
    }
    output
}

/// The delimited (CSV/TSV) output, written by the verified
/// `delimited_record` (byte for byte the `csv` crate's default writer): a
/// field containing the delimiter, a quote, or a line break is quoted, so
/// every probe is one record of eleven fields.
fn render_delimited(results: &[ScanResult], delimiter: char) -> String {
    let mut output = delimited_record(
        &[
            "port",
            "status",
            "reserved",
            "protocol",
            "address_family",
            "scope",
            "address",
            "process_id",
            "process_name",
            "user",
            "owner_status",
        ],
        delimiter,
    );
    for result in results {
        let port = result.port.to_string();
        let reserved = result.reserved.to_string();
        let process_id = optional_display(result.process_id);
        let process_name = optional_display(result.process_name.as_deref());
        let user = optional_display(result.user.as_deref());
        output.push_str(&delimited_record(
            &[
                &port,
                &result.status,
                &reserved,
                &result.protocol,
                &result.address_family,
                &result.scope,
                &result.address,
                &process_id,
                &process_name,
                &user,
                &result.owner_status,
            ],
            delimiter,
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_command() -> ScanCommand {
        ScanCommand {
            min: None,
            max: None,
            autoexclude: false,
            autocompact: false,
            format: ScanOutputFormat::Json,
            skip_tcp: false,
            skip_udp: false,
            skip_ipv4: false,
            skip_ipv6: false,
            check_all_interfaces: false,
        }
    }

    fn config_with_ports(min: u16, max: Option<u16>, max_offset: Option<u16>) -> Config {
        Config {
            ports: Some(PortConfig {
                min,
                max,
                max_offset,
            }),
            ..Default::default()
        }
    }

    fn bounds(range: &PortRange) -> (u16, u16) {
        (range.min().value(), range.max().value())
    }

    #[test]
    fn determine_range_honors_max_offset() {
        // Regression: `max_offset` used to be ignored, so this scanned (or,
        // with min above 7000, failed on) the default maximum instead.
        let range = scan_command()
            .determine_range(&config_with_ports(8000, None, Some(100)))
            .unwrap();
        assert_eq!(bounds(&range), (8000, 8100));

        let range = scan_command()
            .determine_range(&config_with_ports(6000, None, Some(100)))
            .unwrap();
        assert_eq!(bounds(&range), (6000, 6100));
    }

    #[test]
    fn determine_range_matches_the_allocator_range() {
        for (min, max, max_offset) in [
            (5000, Some(7000), None),
            (8000, None, Some(100)),
            (1, Some(65535), None),
            (65535, None, Some(0)),
            (65000, None, Some(535)),
            (5000, Some(5100), Some(1)),
        ] {
            let config = config_with_ports(min, max, max_offset);
            let scanned = scan_command().determine_range(&config).unwrap();
            let allocator = trop::port::allocator::allocator_from_config(&config).unwrap();
            assert_eq!(scanned, *allocator.range(), "{min} {max:?} {max_offset:?}");
        }
    }

    #[test]
    fn determine_range_defaults_without_ports_section() {
        let range = scan_command().determine_range(&Config::default()).unwrap();
        assert_eq!(
            bounds(&range),
            (
                trop::config::DEFAULT_MIN_PORT,
                trop::config::DEFAULT_MAX_PORT
            )
        );
    }

    #[test]
    fn determine_range_error_kinds_are_preserved() {
        let error = scan_command()
            .determine_range(&config_with_ports(0, Some(10), None))
            .unwrap_err();
        assert!(
            matches!(&error, CliError::InvalidArguments(message) if message == "invalid port 0: port 0 is invalid"),
            "{error:?}"
        );
        assert_eq!(error.exit_code(), 4);

        let error = scan_command()
            .determine_range(&config_with_ports(10, Some(0), None))
            .unwrap_err();
        assert!(matches!(error, CliError::InvalidArguments(_)), "{error:?}");

        let error = scan_command()
            .determine_range(&config_with_ports(6000, Some(5000), None))
            .unwrap_err();
        assert!(
            matches!(
                &error,
                CliError::Library(trop::Error::InvalidPortRange {
                    min: 6000,
                    max: 5000,
                    ..
                })
            ),
            "{error:?}"
        );

        let error = scan_command()
            .determine_range(&config_with_ports(65000, None, Some(1000)))
            .unwrap_err();
        assert!(
            matches!(&error, CliError::Library(trop::Error::Validation { field, .. }) if field == "ports.max_offset"),
            "{error:?}"
        );
    }

    fn unavailable_result() -> ScanResult {
        ScanResult {
            port: 5050,
            status: "occupied".to_string(),
            reserved: false,
            protocol: "tcp".to_string(),
            address_family: "ipv4".to_string(),
            scope: "localhost".to_string(),
            address: "127.0.0.1".to_string(),
            process_id: None,
            process_name: None,
            user: None,
            owner_status: "unavailable".to_string(),
        }
    }

    #[test]
    fn json_schema_snapshot_keeps_explicit_unavailable_owner_fields() {
        let json = serde_json::to_string_pretty(&[unavailable_result()]).unwrap();
        assert_eq!(
            json,
            r#"[
  {
    "port": 5050,
    "status": "occupied",
    "reserved": false,
    "protocol": "tcp",
    "address_family": "ipv4",
    "scope": "localhost",
    "address": "127.0.0.1",
    "process_id": null,
    "process_name": null,
    "user": null,
    "owner_status": "unavailable"
  }
]"#
        );
    }

    #[test]
    fn delimited_schema_snapshots_append_deterministic_evidence_columns() {
        let results = [unavailable_result()];
        assert_eq!(
            render_delimited(&results, ','),
            "port,status,reserved,protocol,address_family,scope,address,process_id,process_name,user,owner_status\n\
             5050,occupied,false,tcp,ipv4,localhost,127.0.0.1,unavailable,unavailable,unavailable,unavailable\n"
        );
        assert_eq!(
            render_delimited(&results, '\t'),
            "port\tstatus\treserved\tprotocol\taddress_family\tscope\taddress\tprocess_id\tprocess_name\tuser\towner_status\n\
             5050\toccupied\tfalse\ttcp\tipv4\tlocalhost\t127.0.0.1\tunavailable\tunavailable\tunavailable\tunavailable\n"
        );
    }

    /// Regression: an owner process name or user with a newline or tab
    /// broke the scan table's one-line-per-probe layout.
    #[test]
    fn table_cells_from_the_os_are_escaped() {
        let mut result = unavailable_result();
        result.process_name = Some("evil\nproc".to_string());
        result.user = Some("a\tb\u{1b}[31m".to_string());
        let rendered = render_table(&[result]);
        assert_eq!(rendered.lines().count(), 3, "{rendered}");
        assert!(rendered.contains("\"evil\\nproc\""), "{rendered}");
        assert!(rendered.contains("\"a\\tb\\x1b[31m\""), "{rendered}");
        assert!(!rendered.contains('\t'));
        assert!(!rendered.contains('\u{1b}'));
    }

    #[test]
    fn table_schema_snapshot_renders_unavailable_fields_explicitly() {
        let rendered = render_table(&[unavailable_result()]);
        assert_eq!(
            rendered,
            concat!(
                "Port   Status               Reserved  Protocol  Family  Scope      Address         Process ID   Process          User             Owner Status\n",
                "-------------------------------------------------------------------------------------------------------------------------------------------------\n",
                "5050   occupied             false     tcp       ipv4    localhost  127.0.0.1       unavailable  unavailable      unavailable      unavailable\n",
            )
        );
    }
}
