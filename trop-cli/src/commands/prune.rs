//! Prune command implementation.
//!
//! This module implements the `prune` command, which removes reservations
//! for non-existent directories.

use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::operations::CleanupOperations;

/// Remove reservations for non-existent directories.
#[derive(Args)]
pub struct PruneCommand {
    /// Perform a dry run (show what would be removed without removing)
    #[arg(long)]
    pub dry_run: bool,
}

impl PruneCommand {
    /// Execute the prune command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Load configuration for database location
        let config = load_configuration(global)?;

        // Handle dry-run output
        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Scanning for reservations with non-existent paths...");
        }

        // Open database with write access
        let mut db = open_database(global, &config)?;

        // Perform pruning operation
        let result = CleanupOperations::prune(&mut db, self.dry_run).map_err(CliError::from)?;

        // Format and output results
        if global.quiet {
            // Quiet mode: just the count to stdout
            if result.removed_count > 0 {
                println!("{}", result.removed_count);
            }
        } else if global.verbose {
            // Verbose mode: detailed output to stderr
            if self.dry_run {
                eprintln!(
                    "[DRY RUN] Would remove {} reservation(s):",
                    result.removed_count
                );
            } else {
                eprintln!("Removed {} reservation(s):", result.removed_count);
            }

            for reservation in &result.removed_reservations {
                eprintln!(
                    "  - Port {}: {} (tag: {:?}, project: {:?})",
                    reservation.port().value(),
                    reservation.key().path.display(),
                    reservation.key().tag,
                    reservation.project()
                );
            }
        } else {
            // Normal mode: summary to stderr
            if self.dry_run {
                eprintln!(
                    "[DRY RUN] Would remove {} reservation(s) for non-existent paths",
                    result.removed_count
                );
            } else {
                eprintln!(
                    "Removed {} reservation(s) for non-existent paths",
                    result.removed_count
                );
            }
        }

        Ok(())
    }
}
