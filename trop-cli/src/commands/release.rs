//! Release command implementation.
//!
//! This module implements the `release` command, which releases port
//! reservations based on path and tag filters.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::PathBuf;
use trop::{
    Database, ExecutionResult, OperationPlan, PlanExecutor, ReleaseOptions, ReleasePlan,
    ReservationKey,
};

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

    /// Allow operations on unrelated paths
    #[arg(long)]
    pub allow_unrelated_path: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

struct ReleaseExecution {
    plan: OperationPlan,
    result: Option<ExecutionResult>,
}

fn run_release(
    db: &mut Database,
    planner: &ReleasePlan,
    dry_run: bool,
) -> Result<ReleaseExecution, CliError> {
    run_release_with_plan_barrier(db, planner, dry_run, || {})
}

fn run_release_with_plan_barrier<B>(
    db: &mut Database,
    planner: &ReleasePlan,
    dry_run: bool,
    after_plan: B,
) -> Result<ReleaseExecution, CliError>
where
    B: FnOnce(),
{
    // The immediate transaction owns enumeration, planning, and every delete.
    // Dropping it after a dry-run rolls back without ever exposing a different
    // selection path from live execution.
    let tx = db.begin_transaction().map_err(CliError::from)?;
    let plan = planner.build_plan(&tx).map_err(CliError::from)?;
    after_plan();

    if dry_run {
        return Ok(ReleaseExecution { plan, result: None });
    }

    let mut executor = PlanExecutor::new(&tx);
    let result = executor.execute(&plan).map_err(CliError::from)?;
    tx.commit()
        .map_err(trop::Error::from)
        .map_err(CliError::from)?;

    Ok(ReleaseExecution {
        plan,
        result: Some(result),
    })
}

impl ReleaseCommand {
    /// Execute the release command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Resolve path
        let path = context.resolve_path(self.path.as_deref())?;

        // 2. Validate option combinations
        if self.tag.is_some() && self.untagged_only {
            return Err(CliError::InvalidArguments(
                "Cannot specify both --tag and --untagged-only".to_string(),
            ));
        }

        // 3. Consume the path permission from the shared effective configuration.
        let allow_unrelated_path = context.effective()?.allow_unrelated_path();

        // 4. Build one selector with the exact tag semantics shared by exact
        // and recursive release.
        let all_tags = self.tag.is_none() && !self.untagged_only;
        let tag = if self.untagged_only { None } else { self.tag };
        let key = ReservationKey::new(path, tag)
            .map_err(|error| CliError::InvalidArguments(error.to_string()))?;
        let options = ReleaseOptions::new(key)
            .with_force(self.force)
            .with_allow_unrelated_path(allow_unrelated_path);
        let planner = ReleasePlan::new(options)
            .with_all_exact_path_tags(all_tags)
            .with_recursive(self.recursive);

        // 5. Open the database and run the complete selection plus deletion
        // workflow under one immediate transaction.
        let mut db = context.open_database()?;
        let execution = run_release(&mut db, &planner, self.dry_run)?;

        if self.dry_run {
            if !global.quiet {
                if self.recursive {
                    eprintln!(
                        "Dry run - would release {} reservation(s):",
                        execution.plan.actions.len()
                    );
                    for action in &execution.plan.actions {
                        eprintln!("  - {}", action.description());
                    }
                } else {
                    eprintln!("Dry run - would perform the following actions:");
                    for (i, action) in execution.plan.actions.iter().enumerate() {
                        eprintln!("  {}. {}", i + 1, action.description());
                    }
                }
                if !execution.plan.warnings.is_empty() {
                    eprintln!("Warnings:");
                    for warning in &execution.plan.warnings {
                        eprintln!("  - {warning}");
                    }
                }
            }
        } else if !global.quiet {
            if self.recursive {
                eprintln!("Released {} reservation(s)", execution.plan.actions.len());
            } else {
                if execution.plan.actions.is_empty() {
                    eprintln!("No reservation found (already released)");
                } else {
                    eprintln!("Released reservation successfully");
                }
            }
            if let Some(result) = execution.result {
                for warning in result.warnings {
                    eprintln!("Warning: {warning}");
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime};
    use tempfile::tempdir;
    use trop::{DatabaseConfig, Port, Reservation};

    #[test]
    fn recursive_release_serializes_addition_and_refresh_after_planning() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("recursive-release-race.db");
        let database_config =
            DatabaseConfig::new(&database_path).with_busy_timeout(Duration::from_secs(2));
        let mut release_db = Database::open(database_config.clone()).unwrap();
        let mut writer_db = Database::open(database_config).unwrap();

        let project_path = directory.path().join("project");
        let root_key = ReservationKey::new(project_path.clone(), None).unwrap();
        let refresh_key = ReservationKey::new(project_path.join("refresh"), None).unwrap();
        let added_key = ReservationKey::new(project_path.join("added"), None).unwrap();
        let root_port = Port::try_from(5410).unwrap();
        let refresh_port = Port::try_from(5411).unwrap();
        let added_port = Port::try_from(5412).unwrap();
        let stale_time = SystemTime::now() - Duration::from_secs(60);

        release_db
            .create_reservation(
                &Reservation::builder(root_key.clone(), root_port)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        release_db
            .create_reservation(
                &Reservation::builder(refresh_key.clone(), refresh_port)
                    .last_used_at(stale_time)
                    .build()
                    .unwrap(),
            )
            .unwrap();

        let refreshed_reservation = Reservation::builder(refresh_key.clone(), refresh_port)
            .build()
            .unwrap();
        let added_reservation = Reservation::builder(added_key.clone(), added_port)
            .build()
            .unwrap();
        let (start_sender, start_receiver) = mpsc::sync_channel(0);
        let (attempt_sender, attempt_receiver) = mpsc::sync_channel(0);
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        let writer = thread::spawn(move || {
            start_receiver.recv().unwrap();
            attempt_sender.send(()).unwrap();
            let result = (|| -> trop::Result<()> {
                let transaction = writer_db.begin_transaction()?;
                Database::create_reservation_simple(&transaction, &refreshed_reservation)?;
                Database::create_reservation_simple(&transaction, &added_reservation)?;
                transaction.commit().map_err(trop::Error::from)
            })();
            done_sender.send(()).unwrap();
            result
        });

        let options = ReleaseOptions::new(root_key.clone()).with_allow_unrelated_path(true);
        let planner = ReleasePlan::new(options)
            .with_all_exact_path_tags(true)
            .with_recursive(true);
        let execution = run_release_with_plan_barrier(&mut release_db, &planner, false, || {
            start_sender.send(()).unwrap();
            attempt_receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("writer did not attempt its immediate transaction");
            assert!(
                done_receiver
                    .recv_timeout(Duration::from_millis(100))
                    .is_err(),
                "writer completed while recursive release still owned the write transaction"
            );
        })
        .unwrap();

        assert_eq!(execution.plan.actions.len(), 2);
        done_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("writer did not finish after recursive release committed");
        writer.join().unwrap().unwrap();
        assert!(
            Database::get_reservation(release_db.connection(), &root_key)
                .unwrap()
                .is_none(),
            "the preexisting root must be released"
        );
        let refreshed = Database::get_reservation(release_db.connection(), &refresh_key)
            .unwrap()
            .expect("the refresh serialized after release must survive");
        assert!(
            refreshed
                .last_used_at()
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(5)),
            "the surviving row must be the post-release refresh"
        );
        assert!(
            Database::get_reservation(release_db.connection(), &added_key)
                .unwrap()
                .is_some(),
            "the addition serialized after release must survive"
        );
    }
}
