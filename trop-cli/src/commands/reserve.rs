//! Reserve command implementation.
//!
//! This module implements the `reserve` command, which reserves a port
//! for a directory with optional metadata and constraints.

use crate::error::CliError;
use crate::invocation::InvocationContext;
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
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,

    /// Task identifier
    #[arg(long, value_name = "TASK", env = "TROP_TASK")]
    pub task: Option<String>,

    /// Preferred port number
    #[arg(long, value_name = "PORT")]
    pub port: Option<String>,

    /// Minimum acceptable port
    #[arg(long, value_name = "MIN")]
    pub min: Option<String>,

    /// Maximum acceptable port
    #[arg(long, value_name = "MAX")]
    pub max: Option<String>,

    /// Reallocate an existing reservation for the exact path and tag
    #[arg(long)]
    pub overwrite: bool,

    /// Allow an occupied preferred port
    #[arg(long)]
    pub ignore_occupied: bool,

    /// Allow an excluded preferred port
    #[arg(long)]
    pub ignore_exclusions: bool,

    /// Overwrite and bypass path, metadata, occupancy, and exclusion guards
    #[arg(long)]
    pub force: bool,

    /// Allow operations on unrelated paths
    #[arg(long)]
    pub allow_unrelated_path: bool,

    /// Allow changing the project field
    #[arg(long)]
    pub allow_project_change: bool,

    /// Allow changing the task field
    #[arg(long)]
    pub allow_task_change: bool,

    /// Allow changing project or task fields
    #[arg(long)]
    pub allow_change: bool,

    /// Disable automatic pruning
    #[arg(long)]
    pub disable_autoprune: bool,

    /// Disable automatic expiration
    #[arg(long)]
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
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Resolve path (use CWD if not specified, canonicalize if implicit)
        let path = context.resolve_path(self.path.as_deref())?;

        // 2. Build ReservationKey
        let key = ReservationKey::new(path, self.tag)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 3. Consume the configuration snapshot resolved before dispatch.
        let effective = context.effective()?;
        let config = effective.config();

        // 4. Parse and validate port arguments
        let port = self
            .port
            .as_deref()
            .map(parse_port_string)
            .transpose()?
            .map(Port::try_from)
            .transpose()
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 7. Build library ReserveOptions
        let options = ReserveOptions::new(key, port)
            .with_project(effective.project().map(ToOwned::to_owned))
            .with_task(self.task)
            .with_overwrite(self.overwrite)
            .with_ignore_occupied(self.ignore_occupied)
            .with_ignore_exclusions(self.ignore_exclusions)
            .with_force(self.force)
            .with_allow_unrelated_path(effective.allow_unrelated_path())
            .with_allow_project_change(effective.allow_project_change())
            .with_allow_task_change(effective.allow_task_change())
            .with_disable_autoprune(effective.disable_autoprune())
            .with_disable_autoexpire(effective.disable_autoexpire());

        // 8. Handle dry-run mode
        if self.dry_run {
            // In dry-run mode, just print what would happen without opening database
            if !global.quiet {
                eprintln!("Dry run - would perform the following actions:");
                eprintln!("  1. Reserve port");
            }
            return Ok(());
        }

        // 8. Open database
        let mut db = context.open_database()?;

        // 9. Begin transaction - wraps entire operation (planning + execution)
        let tx = db.begin_transaction().map_err(CliError::from)?;

        // 10. Build plan (inside transaction - sees consistent view)
        let plan = ReservePlan::new(options, config)
            .build_plan(&tx)
            .map_err(CliError::from)?;

        // 11. Execute plan (inside same transaction)
        let mut executor = PlanExecutor::new(&tx);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 12. Commit transaction - all or nothing
        tx.commit()
            .map_err(trop::Error::from)
            .map_err(CliError::from)?;

        // 11. Output just the port number (shell-friendly) to stdout
        if let Some(port) = result.port {
            println!("{}", port.value());
        }

        // 11. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}

/// Parse a port number from a string, validating it's in the valid range (1-65535).
///
/// Returns an error if the string cannot be parsed as a number or if the number
/// is outside the valid port range.
pub(crate) fn parse_port_string(s: &str) -> Result<u16, CliError> {
    // Try to parse as u32 first to detect values > 65535
    let parsed = s.parse::<u32>().map_err(|_| {
        CliError::InvalidArguments(format!("Invalid port number: '{s}' is not a valid number"))
    })?;

    // Validate port is in valid range (1-65535)
    if parsed == 0 {
        return Err(CliError::InvalidArguments(
            "Invalid port number: port 0 is reserved and cannot be used".to_string(),
        ));
    }

    if parsed > 65535 {
        return Err(CliError::InvalidArguments(format!(
            "Invalid port number: {parsed} is greater than maximum port 65535"
        )));
    }

    Ok(parsed as u16)
}
