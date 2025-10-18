//! Command to assert that the data directory exists and is valid.

use crate::error::CliError;
use crate::utils::{resolve_data_dir, GlobalOptions};
use clap::Args;
use std::path::{Path, PathBuf};
use trop::{Database, DatabaseConfig};

/// Assert that the data directory exists and is valid.
#[derive(Args)]
pub struct AssertDataDirCommand {
    /// Data directory path to check (default: ~/.trop)
    #[arg(long, value_name = "PATH", env = "TROP_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Invert the assertion (fail if data dir exists)
    #[arg(long)]
    pub not: bool,

    /// Also validate database contents
    #[arg(long)]
    pub validate: bool,
}

impl AssertDataDirCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve data directory
        let data_dir = self
            .data_dir
            .or(global.data_dir.clone())
            .unwrap_or_else(resolve_data_dir);

        // 2. Check existence
        let exists = data_dir.exists();

        // 3. If validating, check database integrity
        let valid = if exists && self.validate {
            match validate_database(&data_dir) {
                Ok(()) => true,
                Err(_) => false,
            }
        } else {
            exists
        };

        // 4. Check assertion
        let success = if self.not { !valid } else { valid };

        // 5. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!(
                    "Assertion failed: data directory exists at {}",
                    data_dir.display()
                )
            } else if self.validate && exists {
                "Assertion failed: database validation failed".to_string()
            } else {
                format!(
                    "Assertion failed: data directory not found at {}",
                    data_dir.display()
                )
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}

fn validate_database(data_dir: &Path) -> Result<(), CliError> {
    let db_path = data_dir.join("trop.db");
    if !db_path.exists() {
        return Err(CliError::InvalidArguments("Database file not found".into()));
    }

    // Open database and run integrity check
    let config = DatabaseConfig::new(db_path);
    let mut db = Database::open(config).map_err(CliError::from)?;

    // Run PRAGMA integrity_check
    db.verify_integrity().map_err(CliError::from)?;

    Ok(())
}
