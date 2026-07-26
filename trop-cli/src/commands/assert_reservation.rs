//! Command to assert that a reservation exists for a specific path/tag combination.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use crate::utils::{normalize_path, resolve_path};
use clap::Args;
use std::path::PathBuf;
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
        // 1. Resolve path using existing utilities
        let path = resolve_path(self.path)?;
        let normalized = normalize_path(&path)?;

        // 2. Open database (read-only)
        let db = context.open_database()?;

        // 3. Build reservation key and query
        let key =
            ReservationKey::new(normalized, self.tag).map_err(|e| CliError::Library(e.into()))?;

        let reservation =
            Database::get_reservation(db.connection(), &key).map_err(CliError::from)?;

        // 4. Check assertion
        let exists = reservation.is_some();
        let success = if self.not { !exists } else { exists };

        // 5. Output port if found (unless --quiet)
        if !self.not && exists && !global.quiet {
            if let Some(res) = reservation {
                println!("{}", res.port());
            }
        }

        // 6. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!("Assertion failed: reservation exists for {key}")
            } else {
                format!("Assertion failed: no reservation found for {key}")
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}
