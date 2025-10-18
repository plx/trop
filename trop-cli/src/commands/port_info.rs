//! Command to display information about a specific port.

use crate::error::CliError;
use crate::utils::{format_timestamp, load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::port::occupancy::{OccupancyCheckConfig, PortOccupancyChecker, SystemOccupancyChecker};
use trop::Port;

/// Display information about a specific port.
#[derive(Args)]
pub struct PortInfoCommand {
    /// Port number to query
    #[arg(value_name = "PORT")]
    pub port: u16,

    /// Include occupancy information
    #[arg(long)]
    pub include_occupancy: bool,
}

impl PortInfoCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Parse port
        let port =
            Port::try_from(self.port).map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database and query
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Find reservation for this port
        let reservation = db.get_reservation_by_port(port).map_err(CliError::from)?;

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
            let check_config = OccupancyCheckConfig::default();

            match checker.is_occupied(port, &check_config) {
                Ok(occupied) => {
                    if occupied {
                        println!("  Port is currently in use");
                    } else {
                        println!("  Port is available");
                    }
                }
                Err(e) => {
                    println!("  Unable to check occupancy: {e}");
                }
            }
        }

        Ok(())
    }
}
