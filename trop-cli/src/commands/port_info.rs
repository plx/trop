//! Command to display information about a specific port.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use crate::utils::format_timestamp;
use clap::Args;
use std::fmt::Write as _;
use trop::port::occupancy::{OccupancyCheckConfig, SystemOccupancyChecker};
use trop::{Database, Port};

/// Display information about a specific port.
#[derive(Args)]
pub struct PortInfoCommand {
    /// Port number to query
    #[arg(value_name = "PORT")]
    pub port: u16,

    /// Include occupancy information
    #[arg(long)]
    pub include_occupancy: bool,

    /// Skip occupancy checks
    #[arg(long, requires = "include_occupancy")]
    pub skip_occupancy_check: bool,

    /// Skip TCP occupancy probes
    #[arg(long, requires = "include_occupancy")]
    pub skip_tcp: bool,

    /// Skip UDP occupancy probes
    #[arg(long, requires = "include_occupancy")]
    pub skip_udp: bool,

    /// Skip IPv6 occupancy probes
    #[arg(long, requires = "include_occupancy")]
    pub skip_ipv6: bool,

    /// Skip IPv4 occupancy probes
    #[arg(long, requires = "include_occupancy")]
    pub skip_ipv4: bool,

    /// Add wildcard probes for all network interfaces
    #[arg(long, requires = "include_occupancy")]
    pub check_all_interfaces: bool,
}

impl PortInfoCommand {
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 1. Parse port
        let port =
            Port::try_from(self.port).map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database and query
        let db = context.open_database()?;

        // 3. Find reservation for this port
        let reservation =
            Database::get_reservation_by_port(db.connection(), port).map_err(CliError::from)?;

        // 4. Display reservation info
        if let Some(res) = reservation {
            println!("Port: {}", res.port());
            println!("Path: {}", res.key().path.display());
            if let Some(tag) = &res.key().tag {
                println!("Tag: {tag}");
            }
            if let Some(project) = res.project() {
                println!("Project: {project}");
            }
            if let Some(task) = res.task() {
                println!("Task: {task}");
            }
            println!("Created: {}", format_timestamp(res.created_at()));
            println!("Last used: {}", format_timestamp(res.last_used_at()));

            // Check if path exists
            let path_exists = res.key().path.exists();
            println!("Path exists: {}", if path_exists { "yes" } else { "no" });
        } else {
            println!("Port {port} is not reserved");
        }

        // 5. Check occupancy if requested
        if self.include_occupancy {
            println!();
            println!("Occupancy status:");

            let checker = SystemOccupancyChecker;
            let check_config = context
                .config()?
                .occupancy_check
                .as_ref()
                .map(OccupancyCheckConfig::from)
                .unwrap_or_default();

            let report = checker
                .inspect_port(port, &check_config)
                .map_err(CliError::from)?;
            print!("{}", render_occupancy_report(&report));
        }

        Ok(())
    }
}

fn render_occupancy_report(report: &trop::port::occupancy::OccupancyReport) -> String {
    let mut output = String::new();
    if report.was_skipped() {
        writeln!(
            output,
            "  Not checked: the effective policy disabled every probe"
        )
        .expect("writing to a String cannot fail");
    } else if report.is_occupied() {
        writeln!(output, "  Port is currently in use").expect("writing to a String cannot fail");
        for probe in &report.occupied_probes {
            writeln!(
                output,
                "  - {} {} {} at {}",
                probe.protocol, probe.address_family, probe.scope, probe.address
            )
            .expect("writing to a String cannot fail");
            writeln!(
                output,
                "    Process ID: {}",
                optional_display(probe.owner.process_id)
            )
            .expect("writing to a String cannot fail");
            writeln!(
                output,
                "    Process: {}",
                optional_display(probe.owner.process_name.as_deref())
            )
            .expect("writing to a String cannot fail");
            writeln!(
                output,
                "    User: {}",
                optional_display(probe.owner.user.as_deref())
            )
            .expect("writing to a String cannot fail");
            writeln!(output, "    Owner metadata: {}", probe.owner.status)
                .expect("writing to a String cannot fail");
        }
    } else {
        writeln!(output, "  Port is available").expect("writing to a String cannot fail");
    }
    output
}

fn optional_display<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use trop::port::occupancy::{
        OccupancyAddressFamily, OccupancyOwner, OccupancyProtocol, OccupancyReport, OccupancyScope,
        OccupiedProbe,
    };

    #[test]
    fn occupancy_output_snapshot_explains_probe_and_unavailable_owner() {
        let report = OccupancyReport {
            port: Port::try_from(5050).unwrap(),
            enabled_probe_count: 4,
            occupied_probes: vec![OccupiedProbe {
                protocol: OccupancyProtocol::Tcp,
                address_family: OccupancyAddressFamily::Ipv4,
                scope: OccupancyScope::Localhost,
                address: Ipv4Addr::LOCALHOST.into(),
                owner: OccupancyOwner::default(),
            }],
        };

        assert_eq!(
            render_occupancy_report(&report),
            concat!(
                "  Port is currently in use\n",
                "  - TCP IPv4 localhost at 127.0.0.1\n",
                "    Process ID: unavailable\n",
                "    Process: unavailable\n",
                "    User: unavailable\n",
                "    Owner metadata: unavailable\n",
            )
        );
    }

    #[test]
    fn occupancy_output_snapshot_distinguishes_a_disabled_policy() {
        let report = OccupancyReport {
            port: Port::try_from(5050).unwrap(),
            enabled_probe_count: 0,
            occupied_probes: Vec::new(),
        };

        assert_eq!(
            render_occupancy_report(&report),
            "  Not checked: the effective policy disabled every probe\n"
        );
    }
}
