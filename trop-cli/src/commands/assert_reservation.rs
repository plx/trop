//! Command to assert that a reservation exists for a specific path/tag combination.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::PathBuf;
use trop::cli_support::{assertion_outcome, prints_reservation_port, AssertionOutcome};
use trop::{Database, ReservationKey};

/// Assert that a reservation exists for a specific path/tag combination.
#[derive(Args)]
pub struct AssertReservationCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Invert the assertion (fail if reservation exists)
    #[arg(long)]
    pub not: bool,
}

impl AssertReservationCommand {
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Resolve the path through the invocation-wide provenance policy.
        let path = context.resolve_path(self.path.as_deref())?;

        // 2. Build the reservation key. A blank tag is an invalid argument
        // (exit 4), exactly as `reserve` and `release` report it.
        let key = ReservationKey::new(path, self.tag)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 3. Open database (read-only) and query
        let db = context.open_database()?;
        let reservation =
            Database::get_reservation(db.connection(), &key).map_err(CliError::from)?;
        let exists = reservation.is_some();

        // 4. Output the port when found, unless inverted or quiet (verified:
        // only ever when the assertion holds).
        if prints_reservation_port(exists, self.not, global.quiet) {
            if let Some(res) = reservation {
                println!("{}", res.port());
            }
        }

        // 5. Check the assertion (verified): a failure names what was observed.
        match assertion_outcome(exists, self.not) {
            AssertionOutcome::Holds => Ok(()),
            AssertionOutcome::FailsPresent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: reservation exists for {key}"
            ))),
            AssertionOutcome::FailsAbsent => Err(CliError::SemanticFailure(format!(
                "Assertion failed: no reservation found for {key}"
            ))),
        }
    }
}
