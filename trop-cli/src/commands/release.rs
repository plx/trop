//! Release command implementation.
//!
//! This module implements the `release` command, which releases port
//! reservations based on path and tag filters.

use crate::error::CliError;
use crate::utils::{load_configuration, open_database, resolve_path, GlobalOptions};
use clap::Args;
use std::path::PathBuf;
use trop::{Database, PlanExecutor, ReleaseOptions, ReleasePlan, ReservationKey};

/// Release a port reservation.
#[derive(Args)]
pub struct ReleaseCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Only release untagged reservation
    #[arg(long)]
    pub untagged_only: bool,

    /// Release all reservations under path recursively
    #[arg(long)]
    pub recursive: bool,

    /// Force operation
    #[arg(long)]
    pub force: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl ReleaseCommand {
    /// Execute the release command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve path
        let path = resolve_path(self.path)?;

        // 2. Validate option combinations
        if self.tag.is_some() && self.untagged_only {
            return Err(CliError::InvalidArguments(
                "Cannot specify both --tag and --untagged-only".to_string(),
            ));
        }

        // 3. Load configuration
        let config = load_configuration(global)?;

        // 4. Open database
        let mut db = open_database(global, &config)?;

        // 5. Handle recursive release or single release
        if self.recursive {
            // For recursive release, we need to find all reservations under this path
            // and release them one by one
            let all_reservations =
                Database::list_all_reservations(db.connection()).map_err(CliError::from)?;

            let mut released_count = 0;
            let mut plans = Vec::new();

            for reservation in all_reservations {
                // Check if this reservation is under the target path
                if !reservation.key().path.starts_with(&path) {
                    continue;
                }

                // Apply tag filter
                if self.untagged_only && reservation.key().tag.is_some() {
                    continue;
                }

                if let Some(ref tag) = self.tag {
                    if reservation.key().tag.as_deref() != Some(tag.as_str()) {
                        continue;
                    }
                }

                // Build release options for this reservation
                let options = ReleaseOptions::new(reservation.key().clone())
                    .with_force(self.force)
                    .with_allow_unrelated_path(true); // Already validated

                // Build plan using database connection for reading
                let plan = ReleasePlan::new(options)
                    .build_plan(db.connection())
                    .map_err(CliError::from)?;

                plans.push((reservation.key().clone(), plan));
            }

            // Execute all plans
            if self.dry_run {
                if !global.quiet {
                    eprintln!("Dry run - would release {} reservation(s):", plans.len());
                    for (key, plan) in &plans {
                        eprintln!("  {key}");
                        for action in &plan.actions {
                            eprintln!("    - {}", action.description());
                        }
                    }
                }
            } else {
                for (_, plan) in plans {
                    // Each release in its own transaction
                    let tx = db.begin_transaction().map_err(CliError::from)?;
                    let mut executor = PlanExecutor::new(&tx);
                    executor.execute(&plan).map_err(CliError::from)?;
                    tx.commit()
                        .map_err(trop::Error::from)
                        .map_err(CliError::from)?;
                    released_count += 1;
                }

                if !global.quiet {
                    eprintln!("Released {released_count} reservation(s)");
                }
            }
        } else {
            // Single release: build key and release it
            let tag = if self.untagged_only { None } else { self.tag };

            let key = ReservationKey::new(path, tag)
                .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

            let options = ReleaseOptions::new(key)
                .with_force(self.force)
                .with_allow_unrelated_path(true); // Path was resolved from CWD

            // Begin transaction for single release
            let tx = db.begin_transaction().map_err(CliError::from)?;

            let plan = ReleasePlan::new(options)
                .build_plan(&tx)
                .map_err(CliError::from)?;

            if self.dry_run {
                if !global.quiet {
                    eprintln!("Dry run - would perform the following actions:");
                    for (i, action) in plan.actions.iter().enumerate() {
                        eprintln!("  {}. {}", i + 1, action.description());
                    }
                    if !plan.warnings.is_empty() {
                        eprintln!("Warnings:");
                        for warning in &plan.warnings {
                            eprintln!("  - {warning}");
                        }
                    }
                }
            } else {
                let mut executor = PlanExecutor::new(&tx);
                let result = executor.execute(&plan).map_err(CliError::from)?;

                // Commit transaction
                tx.commit()
                    .map_err(trop::Error::from)
                    .map_err(CliError::from)?;

                if !global.quiet {
                    if plan.actions.is_empty() {
                        eprintln!("No reservation found (already released)");
                    } else {
                        eprintln!("Released reservation successfully");
                    }

                    // Print warnings if any
                    if !result.warnings.is_empty() {
                        for warning in &result.warnings {
                            eprintln!("Warning: {warning}");
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
