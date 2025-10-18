//! Reserve group command implementation.
//!
//! This module implements the `reserve-group` command, which reserves
//! ports for a group of services defined in a configuration file.

use crate::error::CliError;
use crate::utils::{format_allocations, load_configuration, open_database, GlobalOptions};
use clap::{Args, ValueEnum};
use std::path::PathBuf;
use trop::operations::{ReserveGroupOptions, ReserveGroupPlan};
use trop::output::{OutputFormat, ShellType};
use trop::PlanExecutor;

/// Reserve ports for a group of services defined in a config file.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveGroupCommand {
    /// Configuration file path
    pub config_path: PathBuf,

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

/// Output format argument for clap.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormatArg {
    /// Shell-specific export statements
    Export,
    /// JSON format
    Json,
    /// Dotenv (.env file) format
    Dotenv,
    /// Human-readable format
    Human,
}

impl OutputFormatArg {
    /// Convert to `OutputFormat`, detecting shell type if needed.
    pub fn to_output_format(self, shell_arg: Option<&str>) -> Result<OutputFormat, CliError> {
        match self {
            Self::Export => {
                let shell = if let Some(shell_str) = shell_arg {
                    ShellType::from_string(shell_str).map_err(CliError::from)?
                } else {
                    ShellType::detect().map_err(CliError::from)?
                };
                Ok(OutputFormat::Export(shell))
            }
            Self::Json => Ok(OutputFormat::Json),
            Self::Dotenv => Ok(OutputFormat::Dotenv),
            Self::Human => Ok(OutputFormat::Human),
        }
    }
}

impl ReserveGroupCommand {
    /// Execute the reserve-group command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Validate config file exists
        if !self.config_path.exists() {
            return Err(CliError::InvalidArguments(format!(
                "Configuration file not found: {}",
                self.config_path.display()
            )));
        }

        if !self.config_path.is_file() {
            return Err(CliError::InvalidArguments(format!(
                "Path is not a file: {}",
                self.config_path.display()
            )));
        }

        // 2. Build ReserveGroupOptions
        let options = ReserveGroupOptions::new(self.config_path.clone())
            .with_task(self.task)
            .with_force(self.force)
            .with_allow_unrelated_path(self.allow_unrelated_path)
            .with_allow_project_change(self.allow_project_change || self.allow_change)
            .with_allow_task_change(self.allow_task_change || self.allow_change);

        // 3. Handle dry-run mode
        if self.dry_run {
            if !global.quiet {
                eprintln!("Dry run - would perform the following actions:");
                eprintln!(
                    "  1. Reserve group of services from {}",
                    self.config_path.display()
                );
            }
            return Ok(());
        }

        // 4. Load configuration and open database
        let config = load_configuration(global)?;
        let mut db = open_database(global, &config)?;

        // 5. Build plan
        let planner = ReserveGroupPlan::new(options).map_err(CliError::from)?;
        let plan = planner.build_plan(&db).map_err(CliError::from)?;

        // 6. Execute plan
        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 7. Extract allocated ports
        let allocated_ports = result.allocated_ports.ok_or_else(|| {
            CliError::InvalidArguments("No ports were allocated - this is unexpected".to_string())
        })?;

        // 8. Format output based on selected format
        let output_format = self.format.to_output_format(self.shell.as_deref())?;

        let formatted_output = format_allocations(&output_format, &allocated_ports, &config)?;

        // 9. Print to stdout (machine-readable)
        println!("{formatted_output}");

        // 10. Print status to stderr (human-readable, unless quiet)
        if !global.quiet {
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

        // 11. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}
