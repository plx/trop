//! Command to show the resolved data directory path.

use crate::error::CliError;
use crate::utils::{resolve_data_dir, GlobalOptions};
use clap::Args;

/// Show the resolved data directory path.
#[derive(Args)]
pub struct ShowDataDirCommand {}

impl ShowDataDirCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Resolve data directory using same logic as other commands
        let data_dir = global.data_dir.clone().unwrap_or_else(resolve_data_dir);

        println!("{}", data_dir.display());
        Ok(())
    }
}
