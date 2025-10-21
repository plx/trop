//! Command to show the resolved path for a reservation.

use crate::error::CliError;
use crate::utils::{normalize_path, resolve_path, GlobalOptions};
use clap::Args;
use std::path::PathBuf;

/// Show the resolved path that would be used for a reservation.
#[derive(Args)]
pub struct ShowPathCommand {
    /// Path to resolve
    #[arg(long, value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Explicitly request canonicalization
    #[arg(long)]
    pub canonicalize: bool,
}

impl ShowPathCommand {
    pub fn execute(self, _global: &GlobalOptions) -> Result<(), CliError> {
        // Get path using existing resolution logic
        let path = resolve_path(self.path)?;

        // Normalize or canonicalize as requested
        let resolved = if self.canonicalize {
            path.canonicalize().map_err(CliError::from)?
        } else {
            normalize_path(&path)?
        };

        println!("{}", resolved.display());
        Ok(())
    }
}
