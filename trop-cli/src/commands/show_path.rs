//! Command to show the resolved path for a reservation.

use crate::error::CliError;
use crate::invocation::InvocationContext;
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
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let resolved = if self.canonicalize {
            let path = self
                .path
                .as_deref()
                .unwrap_or_else(|| context.working_dir());
            context.resolve_canonical_path(path)?
        } else {
            context.resolve_path(self.path.as_deref())?
        };

        println!("{}", resolved.display());
        Ok(())
    }
}
