//! Command to assert that the data directory exists and is valid.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use trop::cli_support::{assertion_outcome, AssertionOutcome};

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

        // 2. Observe the directory, telling an absent one (a clean false
        // predicate) from one whose existence cannot be determined, with or
        // without validation.
        let exists = observe_data_dir(data_dir)?;

        // 3. Validation failures are internal errors, not a false predicate
        // that --not may invert into success.
        if exists && self.validate {
            validate_database(context)?;
        }

        // 4. Check the assertion (verified): a failure names what was
        // observed. (A failed validation never reaches this point.)
        match assertion_outcome(exists, self.not) {
            AssertionOutcome::Holds => Ok(()),
            AssertionOutcome::FailsPresent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: data directory exists at {}",
                data_dir.display()
            ))),
            AssertionOutcome::FailsAbsent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: data directory not found at {}",
                data_dir.display()
            ))),
        }
    }
}

/// Whether the data directory exists: `Ok(true)` for a directory (following
/// symbolic links), `Ok(false)` when nothing is there. A path that cannot be
/// inspected (for example, permission denied on an ancestor) or that holds
/// something other than a directory is neither a present nor an absent data
/// directory, so it is an internal error that `--not` cannot invert, never a
/// verdict. (`Path::exists` would report the first as absent and the second
/// as present.)
fn observe_data_dir(data_dir: &Path) -> Result<bool, CliError> {
    let inaccessible = |reason: &dyn std::fmt::Display| {
        CliError::from(trop::Error::DatabaseCorruption {
            details: format!(
                "cannot access the selected data directory {}: {reason}; \
                 verify its path and permissions before retrying. \
                 trop did not initialize or modify the data directory",
                data_dir.display()
            ),
        })
    };
    match fs::metadata(data_dir) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(inaccessible(&"it is not a directory")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(inaccessible(&error)),
    }
}

fn validate_database(context: &InvocationContext) -> Result<(), CliError> {
    context.validate_existing_database()
}
