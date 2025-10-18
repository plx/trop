//! Autoreserve command implementation.
//!
//! This module implements the `autoreserve` command, which automatically
//! discovers a config file and reserves ports for the defined group.

use crate::error::CliError;
use crate::utils::{format_allocations, load_configuration, open_database, GlobalOptions};
use clap::Args;
use std::env;
use trop::operations::{AutoreserveOptions, AutoreservePlan};
use trop::PlanExecutor;

use super::reserve_group::OutputFormatArg;

/// Automatically discover and reserve ports from project config.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct AutoreserveCommand {
    /// Task identifier
    #[arg(long, env = "TROP_TASK")]
    pub task: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "export")]
    pub format: OutputFormatArg,

    /// Shell type for export format (auto-detect if not specified)
    #[arg(long)]
    pub shell: Option<String>,

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

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl AutoreserveCommand {
    /// Execute the autoreserve command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Get current working directory as start directory
        let start_dir = env::current_dir().map_err(CliError::Io)?;

        // 2. Build AutoreserveOptions
        let options = AutoreserveOptions::new(start_dir.clone())
            .with_task(self.task)
            .with_force(self.force)
            .with_allow_unrelated_path(self.allow_unrelated_path)
            .with_allow_project_change(self.allow_project_change || self.allow_change)
            .with_allow_task_change(self.allow_task_change || self.allow_change);

        // 3. Discover config file
        let planner = AutoreservePlan::new(options).map_err(|e| match &e {
            trop::Error::InvalidPath { reason, .. }
                if reason.contains("No trop configuration file found") =>
            {
                CliError::InvalidArguments(format!(
                    "No trop configuration file found (searched from: {})",
                    start_dir.display()
                ))
            }
            _ => CliError::from(e),
        })?;

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

        // 5. Load configuration and open database
        let config = load_configuration(global)?;
        let mut db = open_database(global, &config)?;

        // 6. Build plan
        let plan = planner.build_plan(&db).map_err(CliError::from)?;

        // 7. Execute plan
        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 8. Extract allocated ports
        let allocated_ports = result.allocated_ports.ok_or_else(|| {
            CliError::InvalidArguments("No ports were allocated - this is unexpected".to_string())
        })?;

        // 9. Format output based on selected format
        let output_format = self.format.to_output_format(self.shell.as_deref())?;

        let formatted_output = format_allocations(&output_format, &allocated_ports, &config)?;

        // 10. Print to stdout (machine-readable)
        println!("{formatted_output}");

        // 11. Print status to stderr (human-readable, unless quiet)
        if !global.quiet {
            eprintln!("Discovered config: {}", discovered_config.display());
            eprintln!(
                "Reserved {} ports for services: {}",
                allocated_ports.len(),
                allocated_ports
                    .keys()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        // 12. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}
