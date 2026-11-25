//! Command to show the resolved data directory path.

use crate::error::CliError;
use crate::utils::{resolve_data_dir, GlobalOptions};
use clap::Args;

/// Show the resolved data directory path.
#[derive(Args)]
pub struct ShowDataDirCommand {}

impl ShowDataDirCommand {
    /// Prints the data directory other commands use. When none can be
    /// determined (no `--data-dir` or `TROP_DATA_DIR`, and no non-empty
    /// absolute `HOME` or `USERPROFILE`), fails with exit code 3.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Resolve data directory using same logic as other commands
        let data_dir = match &global.data_dir {
            Some(data_dir) => data_dir.clone(),
            None => resolve_data_dir()?,
        };

        println!("{}", data_dir.display());
        Ok(())
    }
}
