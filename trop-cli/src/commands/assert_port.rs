//! Command to assert that a specific port is reserved.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use trop::cli_support::{assertion_outcome, AssertionOutcome};
use trop::{Database, Port};

/// Assert that a specific port is reserved.
#[derive(Args)]
pub struct AssertPortCommand {
    /// Port number to check
    #[arg(value_name = "PORT")]
    pub port: String,

    /// Invert the assertion (fail if port is reserved)
    #[arg(long)]
    pub not: bool,
}

impl AssertPortCommand {
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 1. Parse port (exactly as `reserve --port` parses it)
        let port = Port::try_from(crate::commands::reserve::parse_port_string(&self.port)?)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database
        let db = context.open_database()?;

        // 3. Check if port is reserved (method already exists)
        let reserved = Database::is_port_reserved(db.connection(), port).map_err(CliError::from)?;

        // 4. Check the assertion (verified): a failure names what was observed.
        match assertion_outcome(reserved, self.not) {
            AssertionOutcome::Holds => Ok(()),
            AssertionOutcome::FailsPresent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: port {port} is reserved"
            ))),
            AssertionOutcome::FailsAbsent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: port {port} is not reserved"
            ))),
        }
    }
}
