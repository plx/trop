//! Init command implementation.
//!
//! This module implements the `init` command for explicitly initializing
//! the trop data directory and database.

use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::Parser;
use std::path::PathBuf;
use trop::database::default_data_dir;
use trop::operations::init::{init_database, InitOptions};

/// Initialize trop data directory and database.
#[derive(Parser)]
#[command(about = "Initialize trop data directory and database")]
pub struct InitCommand {
    /// Data directory to initialize
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

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
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Determine data directory to initialize
        // Priority: command flag > global flag > default
        let data_dir = self
            .data_dir
            .or_else(|| global.data_dir.clone())
            .or_else(|| default_data_dir().ok())
            .ok_or_else(|| {
                CliError::Config(
                    "Could not determine data directory (home directory not found)".to_string(),
                )
            })?;

        if self.dry_run {
            // Dry-run mode: show what would be done
            println!("Dry-run mode: no changes will be made");
            println!();
            println!("Would initialize trop in: {}", data_dir.display());

            if !data_dir.exists() {
                println!("  - Create data directory: {}", data_dir.display());
            } else {
                println!("  - Data directory already exists: {}", data_dir.display());
            }

            let db_path = data_dir.join("trop.db");
            if db_path.exists() {
                if self.overwrite {
                    println!("  - Remove existing database: {}", db_path.display());
                    println!("  - Create new database: {}", db_path.display());
                } else {
                    println!(
                        "  - ERROR: Database already exists (use --overwrite to replace): {}",
                        db_path.display()
                    );
                }
            } else {
                println!("  - Create database: {}", db_path.display());
            }

            if self.with_config {
                let config_path = data_dir.join("config.yaml");
                if config_path.exists() {
                    println!(
                        "  - Configuration file already exists (will not overwrite): {}",
                        config_path.display()
                    );
                } else {
                    println!("  - Create configuration file: {}", config_path.display());
                }
            }

            return Ok(());
        }

        // Build initialization options
        let options = InitOptions::new(data_dir.clone())
            .with_overwrite(self.overwrite)
            .with_create_config(self.with_config);

        // Execute initialization
        let result = init_database(&options).map_err(CliError::from)?;

        // Report what was created
        println!("Initialized trop in: {}", result.data_dir.display());

        if result.data_dir_created {
            println!("  - Created data directory");
        }

        if result.database_created {
            if self.overwrite {
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
