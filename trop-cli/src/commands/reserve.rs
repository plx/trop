//! Reserve command implementation.
//!
//! This module implements the `reserve` command, which reserves a port
//! for a directory with optional metadata and constraints.

use crate::error::CliError;
use crate::utils::{load_configuration, open_database, resolve_path, GlobalOptions};
use clap::Args;
use std::path::PathBuf;
use trop::{PlanExecutor, Port, ReservationKey, ReserveOptions, ReservePlan};

/// Reserve a port for a directory.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Project identifier
    #[arg(long, value_name = "PROJECT", env = "TROP_PROJECT")]
    pub project: Option<String>,

    /// Task identifier
    #[arg(long, value_name = "TASK", env = "TROP_TASK")]
    pub task: Option<String>,

    /// Preferred port number
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

    /// Minimum acceptable port
    #[arg(long, value_name = "MIN", env = "TROP_MIN")]
    pub min: Option<u16>,

    /// Maximum acceptable port
    #[arg(long, value_name = "MAX", env = "TROP_MAX")]
    pub max: Option<u16>,

    /// Overwrite existing reservation
    #[arg(long)]
    pub overwrite: bool,

    /// Ignore if preferred port is occupied
    #[arg(long)]
    pub ignore_occupied: bool,

    /// Ignore excluded ports
    #[arg(long)]
    pub ignore_exclusions: bool,

    /// Force operation (overrides all protections)
    #[arg(long)]
    pub force: bool,

    /// Allow operations on unrelated paths
    #[arg(long, env = "TROP_ALLOW_UNRELATED_PATH")]
    pub allow_unrelated_path: bool,

    /// Allow changing the project field
    #[arg(long, env = "TROP_ALLOW_PROJECT_CHANGE")]
    pub allow_project_change: bool,

    /// Allow changing the task field
    #[arg(long, env = "TROP_ALLOW_TASK_CHANGE")]
    pub allow_task_change: bool,

    /// Allow changing project or task fields
    #[arg(long, env = "TROP_ALLOW_CHANGE")]
    pub allow_change: bool,

    /// Disable automatic pruning
    #[arg(long, env = "TROP_DISABLE_AUTOPRUNE")]
    pub disable_autoprune: bool,

    /// Disable automatic expiration
    #[arg(long, env = "TROP_DISABLE_AUTOEXPIRE")]
    pub disable_autoexpire: bool,

    /// Disable all automatic cleanup
    #[arg(long)]
    pub disable_autoclean: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,

    /// Skip occupancy check
    #[arg(long)]
    pub skip_occupancy_check: bool,

    /// Skip TCP checks
    #[arg(long)]
    pub skip_tcp: bool,

    /// Skip UDP checks
    #[arg(long)]
    pub skip_udp: bool,

    /// Skip IPv6 checks
    #[arg(long)]
    pub skip_ipv6: bool,

    /// Skip IPv4 checks
    #[arg(long)]
    pub skip_ipv4: bool,

    /// Check all network interfaces
    #[arg(long)]
    pub check_all_interfaces: bool,
}

impl ReserveCommand {
    /// Execute the reserve command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve path (use CWD if not specified, canonicalize if implicit)
        let path = resolve_path(self.path)?;

        // 2. Build ReservationKey
        let key = ReservationKey::new(path, self.tag)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 3. Load configuration
        let config = load_configuration(global)?;

        // 4. Convert port number to Port type
        let port = self
            .port
            .map(Port::try_from)
            .transpose()
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 5. Build library ReserveOptions
        let options = ReserveOptions::new(key, port)
            .with_project(self.project)
            .with_task(self.task)
            .with_preferred_port(port)
            .with_ignore_occupied(self.ignore_occupied || self.skip_occupancy_check)
            .with_ignore_exclusions(self.ignore_exclusions)
            .with_force(self.force)
            .with_allow_unrelated_path(self.allow_unrelated_path)
            .with_allow_project_change(self.allow_project_change || self.allow_change)
            .with_allow_task_change(self.allow_task_change || self.allow_change);

        // 6. Open database
        let mut db = open_database(global, &config)?;

        // 7. Build plan
        let plan = ReservePlan::new(options, &config)
            .build_plan(&db)
            .map_err(CliError::from)?;

        // 8. Execute or dry-run
        if self.dry_run {
            // In dry-run mode, just print what would happen
            if !global.quiet {
                eprintln!("Dry run - would perform the following actions:");
                for (i, action) in plan.actions.iter().enumerate() {
                    eprintln!("  {}. {}", i + 1, action.description());
                }
                if !plan.warnings.is_empty() {
                    eprintln!("Warnings:");
                    for warning in &plan.warnings {
                        eprintln!("  - {warning}");
                    }
                }
            }
            return Ok(());
        }

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 9. Output just the port number (shell-friendly) to stdout
        if let Some(port) = result.port {
            println!("{}", port.value());
        }

        // 10. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}
