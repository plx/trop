//! Migrate command implementation.
//!
//! This module implements the `migrate` command, which moves reservations
//! from one path to another while preserving all metadata.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::PathBuf;
use trop::{execute_migrate, MigrateOptions, MigratePlan};

/// Migrate reservations between paths.
#[derive(Args)]
pub struct MigrateCommand {
    /// Source path to migrate from
    #[arg(long, value_name = "PATH")]
    pub from: PathBuf,

    /// Destination path to migrate to
    #[arg(long, value_name = "PATH")]
    pub to: PathBuf,

    /// Migrate all sub-paths recursively
    #[arg(long)]
    pub recursive: bool,

    /// Overwrite existing reservations at destination
    #[arg(long)]
    pub force: bool,

    /// Preview changes without applying them
    #[arg(long)]
    pub dry_run: bool,
}

impl MigrateCommand {
    /// Execute the migrate command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();

        // 1. Open database
        let mut db = context.open_database()?;

        // 2. Resolve both explicitly supplied paths without following symlinks.
        let from = context.resolve_explicit_path(&self.from)?;
        let to = context.resolve_explicit_path(&self.to)?;

        // 3. Build migrate options
        let options = MigrateOptions::new(from.clone(), to.clone())
            .with_recursive(self.recursive)
            .with_force(self.force)
            .with_dry_run(self.dry_run);

        // 4. Create and build migration plan
        let mut plan = MigratePlan::new(options);
        plan.build(&db).map_err(CliError::from)?;

        // 5. Display migration plan
        if !global.quiet {
            eprintln!("Migration plan:");
            eprintln!("  From: {}", from.display());
            eprintln!("  To:   {}", to.display());
            eprintln!(
                "  Mode: {}",
                if self.recursive { "recursive" } else { "exact" }
            );
            eprintln!();

            if plan.migration_count() == 0 && plan.conflict_count() == 0 {
                eprintln!("No reservations to migrate.");
                return Ok(());
            }

            // Show migrations
            if plan.migration_count() > 0 {
                eprintln!("Reservations to migrate ({}):", plan.migration_count());
                for item in &plan.migrations {
                    eprintln!(
                        "  {} -> {} (port {}{}{})",
                        item.from_key.path.display(),
                        item.to_key.path.display(),
                        item.reservation.port().value(),
                        if let Some(ref tag) = item.from_key.tag {
                            format!(", tag: {tag}")
                        } else {
                            String::new()
                        },
                        if let Some(ref project) = item.reservation.project() {
                            format!(", project: {project}")
                        } else {
                            String::new()
                        }
                    );
                }
                eprintln!();
            }

            // Show conflicts
            if plan.conflict_count() > 0 {
                eprintln!(
                    "Conflicts at destination ({}){}:",
                    plan.conflict_count(),
                    if self.force {
                        " (will be overwritten)"
                    } else {
                        ""
                    }
                );
                for conflict_key in &plan.conflicts {
                    eprintln!("  {}", conflict_key.path.display());
                    if let Some(ref tag) = conflict_key.tag {
                        eprintln!("    (tag: {tag})");
                    }
                }
                eprintln!();

                if !self.force {
                    eprintln!("Use --force to overwrite conflicting reservations.");
                    return Err(CliError::InvalidArguments(
                        "Migration would conflict with existing reservations".to_string(),
                    ));
                }
            }
        }

        // 6. Execute migration if not dry-run
        if self.dry_run {
            if !global.quiet {
                eprintln!("Dry run - no changes made.");
            }
        } else {
            let result = execute_migrate(&plan, &mut db).map_err(CliError::from)?;

            if !global.quiet {
                eprintln!("Migration complete:");
                eprintln!("  Migrated: {} reservation(s)", result.migrated_count);
                if result.conflicts_resolved > 0 {
                    eprintln!("  Conflicts resolved: {}", result.conflicts_resolved);
                }
            }

            // Output the destination path to stdout for scripting
            if global.quiet {
                println!("{}", result.to_path.display());
            }
        }

        Ok(())
    }
}
