//! Expire command implementation.
//!
//! This module implements the `expire` command, which removes reservations
//! based on age.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use trop::config::CleanupConfig;
use trop::operations::CleanupOperations;

/// Remove reservations based on age.
#[derive(Args)]
pub struct ExpireCommand {
    /// Remove reservations unused for N days
    #[arg(long, value_name = "DAYS")]
    pub days: Option<u32>,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl ExpireCommand {
    /// Execute the expire command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        let expire_days = context.effective()?.cleanup().expire_after_days;

        // Validate we have a threshold
        let Some(days) = expire_days else {
            return Err(CliError::InvalidArguments(
                "No expiration threshold specified. Use --days or configure expire_after_days"
                    .into(),
            ));
        };

        // Create cleanup config
        let cleanup_config = CleanupConfig {
            expire_after_days: Some(days),
        };

        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Scanning for reservations unused for {days} days...");
        }

        // Open database
        let mut db = context.open_database()?;

        // Perform expiration
        let result = CleanupOperations::expire(&mut db, &cleanup_config, self.dry_run)
            .map_err(CliError::from)?;

        // Format output
        if global.quiet {
            if result.removed_count > 0 {
                println!("{}", result.removed_count);
            }
        } else if global.verbose {
            if self.dry_run {
                eprintln!(
                    "[DRY RUN] Would expire {} reservation(s) older than {} days:",
                    result.removed_count, days
                );
            } else {
                eprintln!(
                    "Expired {} reservation(s) older than {} days:",
                    result.removed_count, days
                );
            }

            for reservation in &result.removed_reservations {
                let age_days = reservation
                    .last_used_at()
                    .elapsed()
                    .unwrap_or_default()
                    .as_secs()
                    / 86400;
                eprintln!(
                    "  - Port {}: {} ({} days old, project: {:?})",
                    reservation.port().value(),
                    reservation.key().path.display(),
                    age_days,
                    reservation.project()
                );
            }
        } else if self.dry_run {
            eprintln!(
                "[DRY RUN] Would expire {} reservation(s) older than {} days",
                result.removed_count, days
            );
        } else {
            eprintln!(
                "Expired {} reservation(s) older than {} days",
                result.removed_count, days
            );
        }

        Ok(())
    }
}
