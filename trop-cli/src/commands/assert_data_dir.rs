//! Command to assert that the data directory exists and is valid.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::PathBuf;

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
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 1. Use the data directory selected once for this invocation.
        let data_dir = context.data_dir()?;

        // 2. Check existence
        let exists = data_dir.exists();

        // 3. If validating, check database integrity
        let valid = if exists && self.validate {
            match validate_database(context) {
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

fn validate_database(context: &InvocationContext) -> Result<(), CliError> {
    // Open the existing database with the effective lock timeout.
    let mut db = context.open_existing_database()?;

    // Run PRAGMA integrity_check
    db.verify_integrity().map_err(CliError::from)?;

    Ok(())
}
