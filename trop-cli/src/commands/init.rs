//! Init command implementation.
//!
//! This module implements the `init` command for explicitly initializing
//! the trop data directory and database.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Parser;
use std::path::PathBuf;
use trop::operations::init::{init_database, plan_init, InitOptions, InitPlan};

/// Initialize trop data directory and database.
#[derive(Parser)]
#[command(about = "Initialize trop data directory and database")]
pub struct InitCommand {
    /// Data directory to initialize
    #[arg(long, value_name = "PATH")]
    pub(crate) data_dir: Option<PathBuf>,

    /// Overwrite existing database
    #[arg(long)]
    overwrite: bool,

    /// Create default configuration file
    #[arg(long)]
    with_config: bool,

    /// Preview actions without executing
    #[arg(long)]
    dry_run: bool,
}

impl InitCommand {
    /// Execute the init command.
    ///
    /// Note: This command does NOT accept --disable-autoinit (would be paradoxical).
    /// The --data-dir flag has a different meaning here (where to create, not where to find).
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let data_dir = context.data_dir()?.to_path_buf();

        // Build initialization options
        let options = InitOptions::new(data_dir.clone())
            .with_overwrite(self.overwrite)
            .with_create_config(self.with_config)
            .with_busy_timeout(std::time::Duration::from_secs(
                context.effective()?.maximum_lock_wait_seconds(),
            ));

        if self.dry_run {
            // The dry run reports the plan the live run would carry out, and
            // fails exactly when it would (an existing database without
            // --overwrite), with the same error and exit code.
            let plan = plan_init(&options).map_err(CliError::from)?;
            print_dry_run(&plan, self.with_config);
            return Ok(());
        }

        // Execute initialization
        let result = init_database(&options).map_err(CliError::from)?;

        // Report what was created
        println!("Initialized trop in: {}", result.data_dir.display());

        if result.data_dir_created {
            println!("  - Created data directory");
        }

        if result.database_created {
            if result.database_replaced {
                println!("  - Recreated database");
            } else {
                println!("  - Created database");
            }
        }

        if result.config_created {
            println!("  - Created default configuration file");
        } else if self.with_config {
            println!("  - Configuration file already exists (not overwritten)");
        }

        Ok(())
    }
}

/// Prints what a live run would do.
fn print_dry_run(plan: &InitPlan, with_config: bool) {
    let data_dir = &plan.data_dir;
    println!("Dry-run mode: no changes will be made");
    println!();
    println!("Would initialize trop in: {}", data_dir.display());

    if plan.create_data_dir {
        println!("  - Create data directory: {}", data_dir.display());
    } else {
        println!("  - Data directory already exists: {}", data_dir.display());
    }

    let db_path = plan.database_path();
    if plan.replace_database {
        println!("  - Remove existing database: {}", db_path.display());
        println!("  - Create new database: {}", db_path.display());
    } else {
        println!("  - Create database: {}", db_path.display());
    }

    let config_path = plan.config_path();
    if plan.create_config {
        println!("  - Create configuration file: {}", config_path.display());
    } else if with_config {
        // Requested but kept: the plan never replaces an existing file.
        println!(
            "  - Configuration file already exists (will not overwrite): {}",
            config_path.display()
        );
    }
}
