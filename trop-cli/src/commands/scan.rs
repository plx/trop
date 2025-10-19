//! Command to scan port range for occupied ports.

use crate::commands::compact_exclusions::compact_exclusion_list;
use crate::error::CliError;
use crate::utils::{load_configuration, open_database, resolve_config_file, GlobalOptions};
use clap::{Args, ValueEnum};
use serde::Serialize;
use trop::config::{Config, PortExclusion, DEFAULT_MAX_PORT, DEFAULT_MIN_PORT};
use trop::port::occupancy::{OccupancyCheckConfig, PortOccupancyChecker, SystemOccupancyChecker};
use trop::{Port, PortRange};

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

impl ScanCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration and determine port range
        let mut config = load_configuration(global)?;
        let range = self.determine_range(&config)?;

        // 2. Open database
        let db = open_database(global, &config)?;

        // 3. Scan for occupied ports
        let checker = SystemOccupancyChecker;
        let check_config = OccupancyCheckConfig {
            skip_tcp: self.skip_tcp,
            skip_udp: self.skip_udp,
            skip_ipv4: self.skip_ipv4,
            skip_ipv6: self.skip_ipv6,
            check_all_interfaces: self.check_all_interfaces,
        };

        let occupied_ports = checker
            .find_occupied_ports(&range, &check_config)
            .map_err(CliError::from)?;

        // 4. Get reserved ports from database
        let reserved_ports = db
            .get_reserved_ports_in_range(&range)
            .map_err(CliError::from)?;

        // 5. Find unreserved occupied ports
        let unreserved_occupied: Vec<Port> = occupied_ports
            .iter()
            .filter(|p| !reserved_ports.contains(p))
            .copied()
            .collect();

        // 6. Auto-exclude if requested
        if self.autoexclude && !unreserved_occupied.is_empty() {
            self.add_exclusions(&mut config, &unreserved_occupied, global)?;

            if self.autocompact {
                self.compact_exclusions(&mut config, global)?;
            }
        }

        // 7. Format and output results
        self.output_results(&occupied_ports, &reserved_ports, &unreserved_occupied)?;

        Ok(())
    }

    fn determine_range(&self, config: &Config) -> Result<PortRange, CliError> {
        let min = self
            .min
            .or(config.ports.as_ref().map(|p| p.min))
            .unwrap_or(DEFAULT_MIN_PORT);
        let max = self
            .max
            .or(config.ports.as_ref().and_then(|p| p.max))
            .unwrap_or(DEFAULT_MAX_PORT);

        let min_port =
            Port::try_from(min).map_err(|e| CliError::InvalidArguments(e.to_string()))?;
        let max_port =
            Port::try_from(max).map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        PortRange::new(min_port, max_port).map_err(|e| CliError::Library(e.into()))
    }

    fn add_exclusions(
        &self,
        config: &mut Config,
        ports: &[Port],
        global: &GlobalOptions,
    ) -> Result<(), CliError> {
        // Determine target config file (project or global)
        let config_path = resolve_config_file(global)?;

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
        let yaml = serde_yaml::to_string(config)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(&config_path, yaml)?;

        if !global.quiet {
            eprintln!(
                "Added {} exclusions to {}",
                ports.len(),
                config_path.display()
            );
        }

        Ok(())
    }

    fn compact_exclusions(
        &self,
        config: &mut Config,
        global: &GlobalOptions,
    ) -> Result<(), CliError> {
        if let Some(ref mut exclusions) = config.excluded_ports {
            let original_count = exclusions.len();
            let compacted = compact_exclusion_list(exclusions);
            let new_count = compacted.len();

            if original_count != new_count {
                *exclusions = compacted;

                // Save compacted config
                let config_path = resolve_config_file(global)?;

                let yaml = serde_yaml::to_string(config)
                    .map_err(|e| CliError::Config(format!("Failed to serialize config: {e}")))?;
                std::fs::write(&config_path, yaml)?;

                if !global.quiet {
                    eprintln!("Compacted {original_count} exclusions to {new_count}");
                }
            }
        }

        Ok(())
    }

    fn output_results(
        &self,
        occupied: &[Port],
        reserved: &[Port],
        unreserved: &[Port],
    ) -> Result<(), CliError> {
        #[derive(Serialize)]
        struct ScanResult {
            port: u16,
            status: String,
            reserved: bool,
        }

        let mut results = Vec::new();

        for port in occupied {
            let is_reserved = reserved.contains(port);
            results.push(ScanResult {
                port: port.value(),
                status: if is_reserved {
                    "occupied (reserved)".to_string()
                } else {
                    "occupied".to_string()
                },
                reserved: is_reserved,
            });
        }

        // Format based on requested output format
        match self.format {
            ScanOutputFormat::Table => {
                println!("{:<10} {:<20} Reserved", "Port", "Status");
                println!("{}", "-".repeat(40));
                for result in &results {
                    println!(
                        "{:<10} {:<20} {}",
                        result.port, result.status, result.reserved
                    );
                }
            }
            ScanOutputFormat::Json => {
                let json = serde_json::to_string_pretty(&results)
                    .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
                println!("{json}");
            }
            ScanOutputFormat::Csv => {
                println!("port,status,reserved");
                for result in &results {
                    println!("{},{},{}", result.port, result.status, result.reserved);
                }
            }
            ScanOutputFormat::Tsv => {
                println!("port\tstatus\treserved");
                for result in &results {
                    println!("{}\t{}\t{}", result.port, result.status, result.reserved);
                }
            }
        }

        if !unreserved.is_empty() {
            eprintln!();
            eprintln!("Found {} unreserved occupied port(s)", unreserved.len());
        }

        Ok(())
    }
}
