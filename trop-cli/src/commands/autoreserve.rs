//! Autoreserve command implementation.
//!
//! This module implements the `autoreserve` command, which automatically
//! discovers a config file and reserves ports for the defined group.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use crate::utils::format_allocations;
use clap::Args;
use trop::operations::{AutoreserveOptions, AutoreservePlan};
use trop::PlanExecutor;

use super::reserve_group::OutputFormatArg;

/// Automatically discover and reserve ports from project config.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct AutoreserveCommand {
    /// Task identifier; omission preserves an existing value (clearing unsupported)
    #[arg(long, env = "TROP_TASK")]
    pub task: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "export")]
    pub format: OutputFormatArg,

    /// Shell type for export format (auto-detect if not specified)
    #[arg(long)]
    pub shell: Option<String>,

    /// Allow unrelated paths, metadata changes, and atomic shape replacement
    ///
    /// This does not bypass exclusions, OS occupancy, invalid configuration,
    /// exhaustion, or another reservation key's port ownership.
    #[arg(long)]
    pub force: bool,

    /// Allow an unrelated config parent without changing other protections
    #[arg(long)]
    pub allow_unrelated_path: bool,

    /// Allow changing only the project field
    #[arg(long)]
    pub allow_project_change: bool,

    /// Allow changing only the task field
    #[arg(long)]
    pub allow_task_change: bool,

    /// Allow changing project and task fields, but not paths or group shape
    #[arg(long)]
    pub allow_change: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,

    /// Skip occupancy checks
    #[arg(long)]
    pub skip_occupancy_check: bool,

    /// Skip TCP occupancy probes
    #[arg(long)]
    pub skip_tcp: bool,

    /// Skip UDP occupancy probes
    #[arg(long)]
    pub skip_udp: bool,

    /// Skip IPv6 occupancy probes
    #[arg(long)]
    pub skip_ipv6: bool,

    /// Skip IPv4 occupancy probes
    #[arg(long)]
    pub skip_ipv4: bool,

    /// Add wildcard probes for all network interfaces
    #[arg(long)]
    pub check_all_interfaces: bool,
}

impl AutoreserveCommand {
    /// Execute the autoreserve command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Use the canonical working directory selected for this invocation.
        let start_dir = context.working_dir().to_path_buf();

        // 2. Build AutoreserveOptions
        let options = AutoreserveOptions::new(start_dir.clone())
            .with_task(self.task)
            .with_force(self.force)
            .with_allow_unrelated_path(context.effective()?.allow_unrelated_path())
            .with_allow_project_change(context.effective()?.allow_project_change())
            .with_allow_task_change(context.effective()?.allow_task_change());

        // 3. Discover config file
        let planner = AutoreservePlan::from_effective(options, context.effective()?).map_err(
            |e| match &e {
                trop::Error::InvalidPath { reason, .. }
                    if reason.contains("No trop configuration file found") =>
                {
                    CliError::InvalidArguments(format!(
                        "No trop configuration file found (searched from: {})",
                        start_dir.display()
                    ))
                }
                _ => CliError::from(e),
            },
        )?;

        let discovered_config = planner.discovered_config_path();

        // 4. Handle dry-run mode
        if self.dry_run {
            if !global.quiet {
                eprintln!("Dry run - would perform the following actions:");
                eprintln!("  1. Discovered config: {}", discovered_config.display());
                eprintln!("  2. Reserve group of services from config");
            }
            return Ok(());
        }

        // 5. Validate the selected output format before opening a transaction.
        let output_format = self.format.to_output_format(self.shell.as_deref())?;

        // 6. Load configuration and open database
        let mut db = context.open_database()?;

        // 7. Begin transaction
        let tx = db.begin_transaction().map_err(CliError::from)?;

        // 8. Build plan (inside transaction)
        let plan = planner.build_plan(&tx).map_err(CliError::from)?;

        // 9. Execute plan (inside transaction)
        let mut executor = PlanExecutor::new(&tx);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 10. Extract and completely format the result before committing.
        let allocated_ports = result.allocated_ports.ok_or_else(|| {
            CliError::InvalidArguments("No ports were allocated - this is unexpected".to_string())
        })?;
        let formatted_output =
            format_allocations(&output_format, &allocated_ports, planner.config())?;

        // 11. Commit only after every fallible output step has succeeded.
        tx.commit()
            .map_err(trop::Error::from)
            .map_err(CliError::from)?;

        // 12. Print the retained output exactly once after a successful commit.
        println!("{formatted_output}");

        // 13. Print status to stderr (human-readable, unless quiet)
        if !global.quiet {
            eprintln!("Discovered config: {discovered_config:?}");
            eprintln!("Reserved {} ports.", allocated_ports.len());
        }

        // 14. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}
