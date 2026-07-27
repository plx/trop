//! Command to scan port range for occupied ports.

use crate::commands::compact_exclusions::compact_exclusion_list;
use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::fmt::Write as _;
use trop::config::{Config, PortExclusion, DEFAULT_MAX_PORT, DEFAULT_MIN_PORT};
use trop::port::occupancy::{
    OccupancyCheckConfig, OccupancyReport, OccupiedProbe, SystemOccupancyChecker,
};
use trop::{Database, Port, PortRange};

/// Scan port range for occupied ports.
#[derive(Args)]
pub struct ScanCommand {
    /// Minimum port (uses config if not specified)
    #[arg(long)]
    pub min: Option<u16>,

    /// Maximum port (uses config if not specified)
    #[arg(long)]
    pub max: Option<u16>,

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

        // 3. Scan for occupied ports
        let checker = SystemOccupancyChecker;
        let check_config = config
            .occupancy_check
            .as_ref()
            .map(OccupancyCheckConfig::from)
            .unwrap_or_default();

        let reports = checker
            .find_occupied_reports(&range, &check_config)
            .map_err(CliError::from)?;
        let occupied_ports = reports.iter().map(|report| report.port).collect::<Vec<_>>();

        // 4. Get reserved ports from database
        let reserved_ports = Database::get_reserved_ports_in_range(db.connection(), &range)
            .map_err(CliError::from)?;

        // 5. Find unreserved occupied ports
        let unreserved_occupied: Vec<Port> = occupied_ports
            .iter()
            .filter(|p| !reserved_ports.contains(p))
            .copied()
            .collect();

        // 6. Auto-exclude if requested
        if self.autoexclude && !unreserved_occupied.is_empty() {
            self.add_exclusions(&unreserved_occupied, context)?;

            if self.autocompact {
                self.compact_exclusions(context)?;
            }
        }

        // 7. Format and output results
        self.output_results(&reports, &reserved_ports, &unreserved_occupied)?;

        Ok(())
    }

    fn determine_range(&self, config: &Config) -> Result<PortRange, CliError> {
        let min = config
            .ports
            .as_ref()
            .map(|p| p.min)
            .unwrap_or(DEFAULT_MIN_PORT);
        let max = config
            .ports
            .as_ref()
            .and_then(|p| p.max)
            .unwrap_or(DEFAULT_MAX_PORT);

        let min_port =
            Port::try_from(min).map_err(|e| CliError::InvalidArguments(e.to_string()))?;
        let max_port =
            Port::try_from(max).map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        PortRange::new(min_port, max_port).map_err(|e| CliError::Library(e.into()))
    }

    fn add_exclusions(&self, ports: &[Port], context: &InvocationContext) -> Result<(), CliError> {
        let config_path = context.config_file_for_write(false)?;
        let mut config = load_raw_config(&config_path)?;

        // Ensure excluded_ports exists
        if config.excluded_ports.is_none() {
            config.excluded_ports = Some(Vec::new());
        }

        // Add new exclusions
        if let Some(ref mut exclusions) = config.excluded_ports {
            for port in ports {
                let exclusion = PortExclusion::Single(port.value());
                if !exclusions.contains(&exclusion) {
                    exclusions.push(exclusion);
                }
            }
        }

        // Save config
        let yaml = serde_yaml::to_string(&config)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(&config_path, yaml)?;

        if !context.global().quiet {
            eprintln!(
                "Added {} exclusions to {}",
                ports.len(),
                config_path.display()
            );
        }

        Ok(())
    }

    fn compact_exclusions(&self, context: &InvocationContext) -> Result<(), CliError> {
        let config_path = context.config_file_for_write(false)?;
        let mut config = load_raw_config(&config_path)?;
        if let Some(ref mut exclusions) = config.excluded_ports {
            let original_count = exclusions.len();
            let compacted = compact_exclusion_list(exclusions);
            let new_count = compacted.len();

            if original_count != new_count {
                *exclusions = compacted;

                // Save compacted config
                let yaml = serde_yaml::to_string(&config)
                    .map_err(|e| CliError::Config(format!("Failed to serialize config: {e}")))?;
                std::fs::write(&config_path, yaml)?;

                if !context.global().quiet {
                    eprintln!("Compacted {original_count} exclusions to {new_count}");
                }
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
                print!("{}", render_delimited(&results, b',')?);
            }
            ScanOutputFormat::Tsv => {
                print!("{}", render_delimited(&results, b'\t')?);
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
            optional_display(result.process_name.as_deref()),
            optional_display(result.user.as_deref()),
            result.owner_status,
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn render_delimited(results: &[ScanResult], delimiter: u8) -> Result<String, CliError> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(Vec::new());
    writer
        .write_record([
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
        ])
        .map_err(|error| CliError::Config(format!("Delimited output failed: {error}")))?;
    for result in results {
        writer
            .write_record([
                result.port.to_string(),
                result.status.clone(),
                result.reserved.to_string(),
                result.protocol.clone(),
                result.address_family.clone(),
                result.scope.clone(),
                result.address.clone(),
                optional_display(result.process_id),
                optional_display(result.process_name.as_deref()),
                optional_display(result.user.as_deref()),
                result.owner_status.clone(),
            ])
            .map_err(|error| CliError::Config(format!("Delimited output failed: {error}")))?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|error| CliError::Config(format!("Delimited output failed: {error}")))?;
    let output = String::from_utf8(bytes)
        .map_err(|error| CliError::Config(format!("Delimited output was not UTF-8: {error}")))?;
    Ok(output)
}

fn load_raw_config(path: &std::path::Path) -> Result<Config, CliError> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let contents = std::fs::read_to_string(path)?;
    serde_yaml::from_str(&contents)
        .map_err(|error| CliError::Config(format!("Failed to parse config: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            render_delimited(&results, b',').unwrap(),
            "port,status,reserved,protocol,address_family,scope,address,process_id,process_name,user,owner_status\n\
             5050,occupied,false,tcp,ipv4,localhost,127.0.0.1,unavailable,unavailable,unavailable,unavailable\n"
        );
        assert_eq!(
            render_delimited(&results, b'\t').unwrap(),
            "port\tstatus\treserved\tprotocol\taddress_family\tscope\taddress\tprocess_id\tprocess_name\tuser\towner_status\n\
             5050\toccupied\tfalse\ttcp\tipv4\tlocalhost\t127.0.0.1\tunavailable\tunavailable\tunavailable\tunavailable\n"
        );
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
