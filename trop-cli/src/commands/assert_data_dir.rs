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

    /// Read-only physical, schema, and logical database validation
    #[arg(long)]
    pub validate: bool,
}

impl AssertDataDirCommand {
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 1. Use the data directory selected once for this invocation.
        let data_dir = context.data_dir()?;

        // 2. Validation must distinguish an absent directory (a clean false
        // predicate) from a directory whose existence cannot be determined.
        let exists = if self.validate {
            data_dir.try_exists().map_err(|error| {
                CliError::from(trop::Error::DatabaseCorruption {
                    details: format!(
                        "cannot access the selected data directory {}: {error}; \
                         verify its path and permissions before retrying. \
                         trop did not initialize or modify the data directory",
                        data_dir.display()
                    ),
                })
            })?
        } else {
            data_dir.exists()
        };

        // 3. Validation failures are internal errors, not a false predicate
        // that --not may invert into success.
        if exists && self.validate {
            validate_database(context)?;
        }
        let valid = exists;

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
    context.validate_existing_database()
}
