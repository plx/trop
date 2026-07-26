//! Autoclean command implementation.
//!
//! This module implements the `autoclean` command, which combines
//! prune and expire operations.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use trop::operations::CleanupOperations;

/// Combined cleanup (prune + expire).
#[derive(Args)]
pub struct AutocleanCommand {
    /// Override expiration threshold in days
    #[arg(long, value_name = "DAYS")]
    pub days: Option<u32>,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl AutocleanCommand {
    /// Execute the autoclean command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        let cleanup_config = context.effective()?.cleanup().clone();

        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Performing combined cleanup...");
        }

        // Open database
        let mut db = context.open_database()?;

        // Perform combined cleanup
        let result = CleanupOperations::autoclean(&mut db, &cleanup_config, self.dry_run)
            .map_err(CliError::from)?;

        // Format output
        if global.quiet {
            if result.total_removed > 0 {
                println!("{}", result.total_removed);
            }
        } else if global.verbose {
            let prefix = if self.dry_run {
                "[DRY RUN] Would remove"
            } else {
                "Removed"
            };

            eprintln!("{} {} total reservation(s):", prefix, result.total_removed);
            eprintln!("  Pruned: {} (non-existent paths)", result.pruned_count);

            if cleanup_config.expire_after_days.is_some() {
                eprintln!("  Expired: {} (old reservations)", result.expired_count);
            }

            if !result.pruned_reservations.is_empty() {
                eprintln!("\nPruned reservations:");
                for res in &result.pruned_reservations {
                    eprintln!(
                        "  - Port {}: {}",
                        res.port().value(),
                        res.key().path.display()
                    );
                }
            }

            if !result.expired_reservations.is_empty() {
                eprintln!("\nExpired reservations:");
                for res in &result.expired_reservations {
                    let age_days =
                        res.last_used_at().elapsed().unwrap_or_default().as_secs() / 86400;
                    eprintln!(
                        "  - Port {}: {} ({} days old)",
                        res.port().value(),
                        res.key().path.display(),
                        age_days
                    );
                }
            }
        } else {
            let prefix = if self.dry_run {
                "[DRY RUN] Would remove"
            } else {
                "Removed"
            };
            eprintln!(
                "{} {} total reservation(s) (pruned: {}, expired: {})",
                prefix, result.total_removed, result.pruned_count, result.expired_count
            );
        }

        Ok(())
    }
}
