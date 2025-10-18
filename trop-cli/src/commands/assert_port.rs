//! Command to assert that a specific port is reserved.

use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::Port;

/// Assert that a specific port is reserved.
#[derive(Args)]
pub struct AssertPortCommand {
    /// Port number to check
    #[arg(value_name = "PORT")]
    pub port: u16,

    /// Invert the assertion (fail if port is reserved)
    #[arg(long)]
    pub not: bool,
}

impl AssertPortCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Parse port
        let port =
            Port::try_from(self.port).map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Check if port is reserved (method already exists)
        let reserved = db.is_port_reserved(port).map_err(CliError::from)?;

        // 4. Check assertion
        let success = if self.not { !reserved } else { reserved };

        // 5. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!("Assertion failed: port {port} is reserved")
            } else {
                format!("Assertion failed: port {port} is not reserved")
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}
