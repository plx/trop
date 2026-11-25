//! Reserve command implementation.
//!
//! This module implements the `reserve` command, which reserves a port
//! for a directory with optional metadata and constraints.

use super::reserve_group::dry_run_database;
use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::{Path, PathBuf};
use std::time::Duration;
use trop::operations::reserve_group::open_dry_run_snapshot;
use trop::{
    Config, Database, DatabaseConfig, PlanAction, PlanExecutor, Port, ReservationKey,
    ReserveOptions, ReservePlan,
};

/// Reserve a port for a directory.
#[derive(Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct ReserveCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Project identifier
    #[arg(long, value_name = "PROJECT", conflicts_with = "clear_project")]
    pub project: Option<String>,

    /// Clear stored project metadata
    #[arg(long, conflicts_with = "project")]
    pub clear_project: bool,

    /// Task identifier
    #[arg(long, value_name = "TASK", conflicts_with = "clear_task")]
    pub task: Option<String>,

    /// Clear stored task metadata
    #[arg(long, conflicts_with = "task")]
    pub clear_task: bool,

    /// Preferred port number
    #[arg(long, value_name = "PORT")]
    pub port: Option<String>,

    /// Minimum acceptable port
    #[arg(long, value_name = "MIN")]
    pub min: Option<String>,

    /// Maximum acceptable port
    #[arg(long, value_name = "MAX")]
    pub max: Option<String>,

    /// Reallocate an existing reservation for the exact path and tag
    #[arg(long)]
    pub overwrite: bool,

    /// Allow an occupied preferred port
    #[arg(long)]
    pub ignore_occupied: bool,

    /// Allow an excluded preferred port
    #[arg(long)]
    pub ignore_exclusions: bool,

    /// Overwrite and bypass path, metadata, occupancy, and exclusion guards
    #[arg(long)]
    pub force: bool,

    /// Allow operations on unrelated paths
    #[arg(long)]
    pub allow_unrelated_path: bool,

    /// Allow changing the project field
    #[arg(long)]
    pub allow_project_change: bool,

    /// Allow changing the task field
    #[arg(long)]
    pub allow_task_change: bool,

    /// Allow changing project or task fields
    #[arg(long)]
    pub allow_change: bool,

    /// Disable automatic pruning
    #[arg(long)]
    pub disable_autoprune: bool,

    /// Disable automatic expiration
    #[arg(long)]
    pub disable_autoexpire: bool,

    /// Disable all automatic cleanup
    #[arg(long)]
    pub disable_autoclean: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,

    /// Skip occupancy check
    #[arg(long)]
    pub skip_occupancy_check: bool,

    /// Skip TCP checks
    #[arg(long)]
    pub skip_tcp: bool,

    /// Skip UDP checks
    #[arg(long)]
    pub skip_udp: bool,

    /// Skip IPv6 checks
    #[arg(long)]
    pub skip_ipv6: bool,

    /// Skip IPv4 checks
    #[arg(long)]
    pub skip_ipv4: bool,

    /// Check all network interfaces
    #[arg(long)]
    pub check_all_interfaces: bool,
}

impl ReserveCommand {
    /// Execute the reserve command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Resolve path (use CWD if not specified, canonicalize if implicit)
        let path = context.resolve_path(self.path.as_deref())?;

        // 2. Build ReservationKey; a new reservation's tag must satisfy the
        // shared identifier rule (non-blank, no NUL, at most 255 bytes).
        let key = ReservationKey::new_validated(path, self.tag)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 3. Consume the configuration snapshot resolved before dispatch.
        let effective = context.effective()?;
        let config = effective.config();
        let metadata = context.reservation_metadata()?;

        // 4. Parse and validate port arguments
        let port = self
            .port
            .as_deref()
            .map(parse_port_string)
            .transpose()?
            .map(Port::try_from)
            .transpose()
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 7. Build library ReserveOptions
        let options = ReserveOptions::new(key, port)
            .with_project_intent(metadata.project.clone())
            .with_task_intent(metadata.task.clone())
            .with_overwrite(self.overwrite)
            .with_ignore_occupied(self.ignore_occupied)
            .with_ignore_exclusions(self.ignore_exclusions)
            .with_force(self.force)
            .with_allow_unrelated_path(effective.allow_unrelated_path())
            .with_allow_project_change(effective.allow_project_change())
            .with_allow_task_change(effective.allow_task_change())
            .with_disable_autoprune(effective.disable_autoprune())
            .with_disable_autoexpire(effective.disable_autoexpire());

        // 8. Handle dry-run mode: plan exactly as a live run would, without
        // mutating (or creating) anything.
        if self.dry_run {
            return dry_run(context, options, config, global.quiet);
        }

        // 8. Open database
        let mut db = context.open_database()?;

        // 9. Begin transaction - wraps entire operation (planning + execution)
        let tx = db.begin_transaction().map_err(CliError::from)?;

        // 10. Build plan (inside transaction - sees consistent view)
        let plan = ReservePlan::new(options, config)
            .build_plan(&tx)
            .map_err(CliError::from)?;

        // 11. Execute plan (inside same transaction)
        let mut executor = PlanExecutor::new(&tx);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // 12. Commit transaction - all or nothing
        tx.commit()
            .map_err(trop::Error::from)
            .map_err(CliError::from)?;

        // 11. Output just the port number (shell-friendly) to stdout
        if let Some(port) = result.port {
            println!("{}", port.value());
        }

        // 11. Print warnings to stderr if any
        if !global.quiet && !result.warnings.is_empty() {
            for warning in &result.warnings {
                eprintln!("Warning: {warning}");
            }
        }

        Ok(())
    }
}

/// Runs `reserve --dry-run`.
///
/// The plan is built exactly as a live run builds it, inside a read
/// transaction that is rolled back, and the dry-run executor reports it: the
/// planned actions go to stderr (unless quiet) and the port a live run would
/// report goes to stdout. Planning errors (an unrelated path, a sticky-field
/// change, an exhausted range without automatic cleanup) fail the dry run
/// exactly as they fail a live run. The plan reads:
/// - an existing database through a read-only connection;
/// - when no database exists yet, the empty initialized in-memory database
///   (`open_dry_run_snapshot`) that a live run would create and plan
///   against, after the checks a live run makes before it creates one
///   (`dry_run_database`, shared with `reserve-group`): with auto-init
///   disabled, or when an existing ancestor of the database's directory is
///   not a directory, the dry run fails like the live run (permissions are
///   not checked). Nothing is created.
///
/// What a dry run cannot do, it says (unless quiet) instead of guessing, and
/// succeeds:
/// - a database that needs initialization or a schema migration is never
///   migrated;
/// - an allocation deferred to automatic cleanup is never attempted, so its
///   port and outcome are unknown.
///
/// Opening an existing WAL-mode database, even read-only, can leave the
/// `trop.db-wal` and `trop.db-shm` files beside it until the next writer
/// closes; the database contents never change.
fn dry_run(
    context: &InvocationContext,
    options: ReserveOptions,
    config: &Config,
    quiet: bool,
) -> Result<(), CliError> {
    let db_path = context.database_path()?;
    // The checks a live run makes before it would create a missing
    // database, and the lock wait.
    let (existing, timeout) = dry_run_database(context)?;
    let database;
    let empty;
    let connection = if let Some(db_path) = existing.as_deref() {
        database = open_existing(db_path, timeout, quiet)?;
        let Some(database) = database.as_ref() else {
            return Ok(());
        };
        database.connection()
    } else {
        // The database a live run would create: empty and initialized.
        empty = open_dry_run_snapshot(None, timeout).map_err(CliError::from)?;
        &empty
    };
    // A deferred (read) transaction gives planning one consistent snapshot;
    // dropping it rolls back, and neither connection writes to a file.
    let tx = connection
        .unchecked_transaction()
        .map_err(trop::Error::from)
        .map_err(CliError::from)?;
    let plan = ReservePlan::new(options, config)
        .build_plan(&tx)
        .map_err(CliError::from)?;
    let result = PlanExecutor::new(&tx)
        .dry_run()
        .execute(&plan)
        .map_err(CliError::from)?;
    drop(tx);

    if !quiet {
        eprintln!("Dry run - would perform the following actions:");
        for (i, action) in result.actions_taken.iter().enumerate() {
            eprintln!("  {}. {action}", i + 1);
        }
        for warning in &result.warnings {
            eprintln!("Warning: {warning}");
        }
        if existing.is_none() {
            eprintln!(
                "No database exists yet at {}; a live run would create it.",
                db_path.display()
            );
        }
        if plan.actions.iter().any(allocates_after_cleanup) {
            eprintln!(
                "The range is exhausted: a live run would run automatic cleanup and, \
                 if that freed anything, retry once. A dry run does neither, so it \
                 cannot predict the port or whether the reservation would succeed."
            );
        }
    }
    if let Some(port) = result.port {
        println!("{}", port.value());
    }
    Ok(())
}

/// Opens the existing database at `db_path` read-only for a dry run, or
/// returns `None`, after saying so (unless quiet), when a live run would
/// first initialize or migrate it, which a dry run must not do.
fn open_existing(
    db_path: &Path,
    timeout: Duration,
    quiet: bool,
) -> Result<Option<Database>, CliError> {
    match Database::open(
        DatabaseConfig::new(db_path)
            .with_busy_timeout(timeout)
            .read_only(),
    ) {
        Ok(db) => Ok(Some(db)),
        // A live run initializes or migrates the database before it plans;
        // a dry run must not, so it cannot plan.
        Err(trop::Error::MigrationRequired { found, target, .. }) => {
            if !quiet {
                eprintln!("Dry run - would perform the following actions:");
                eprintln!("  1. Reserve port");
                if found == 0 {
                    eprintln!(
                        "The database at {} is not initialized yet; a live run would \
                         initialize it first.",
                        db_path.display()
                    );
                } else {
                    eprintln!(
                        "The database at {} has schema version {found} and needs a \
                         migration to version {target}, which a dry run does not perform; \
                         a live run would attempt the migration first, so this dry run \
                         cannot predict its outcome.",
                        db_path.display()
                    );
                }
            }
            Ok(None)
        }
        Err(error) => Err(CliError::from(error)),
    }
}

/// The action allocates at execution, after automatic cleanup.
fn allocates_after_cleanup(action: &PlanAction) -> bool {
    match action {
        PlanAction::CreateReservation(reservation) | PlanAction::UpdateReservation(reservation) => {
            reservation.requires_allocation_at_execution()
        }
        PlanAction::UpdateLastUsed(_)
        | PlanAction::DeleteReservation(_)
        | PlanAction::AllocateGroup { .. } => false,
    }
}

/// Parse a port number from a string, validating it's in the valid range (1-65535).
///
/// Returns an error if the string cannot be parsed as a number or if the number
/// is outside the valid port range. The verified kernel
/// (`trop::cli_support::parse_port_arg`) decides, with `u32::from_str`
/// syntax and the precedence not-a-number, then zero, then too large; every
/// command that takes a port argument uses this function, so they all accept
/// the same text and report the same messages. The rejected text is shown
/// with control characters escaped.
pub(crate) fn parse_port_string(s: &str) -> Result<u16, CliError> {
    use trop::cli_support::{parse_port_arg, PortArgError};

    parse_port_arg(s).map_err(|error| {
        CliError::InvalidArguments(match error {
            PortArgError::NotANumber => format!(
                "Invalid port number: '{}' is not a valid number",
                trop::output::display_field(s)
            ),
            PortArgError::Zero => {
                "Invalid port number: port 0 is reserved and cannot be used".to_string()
            }
            PortArgError::TooLarge(value) => {
                format!("Invalid port number: {value} is greater than maximum port 65535")
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The messages and precedence of the historical `u32` parser.
    #[test]
    fn port_strings_keep_their_messages() {
        assert_eq!(parse_port_string("8080").unwrap(), 8080);
        assert_eq!(parse_port_string("+1").unwrap(), 1);
        assert_eq!(parse_port_string("65535").unwrap(), 65535);
        for (text, message) in [
            ("abc", "Invalid arguments: Invalid port number: 'abc' is not a valid number"),
            ("", "Invalid arguments: Invalid port number: '' is not a valid number"),
            ("99999999999", "Invalid arguments: Invalid port number: '99999999999' is not a valid number"),
            ("a\nb", "Invalid arguments: Invalid port number: '\"a\\nb\"' is not a valid number"),
            ("0", "Invalid arguments: Invalid port number: port 0 is reserved and cannot be used"),
            ("00", "Invalid arguments: Invalid port number: port 0 is reserved and cannot be used"),
            ("65536", "Invalid arguments: Invalid port number: 65536 is greater than maximum port 65535"),
            ("4294967295", "Invalid arguments: Invalid port number: 4294967295 is greater than maximum port 65535"),
        ] {
            let error = parse_port_string(text).unwrap_err();
            assert_eq!(error.to_string(), message, "{text:?}");
            assert_eq!(error.exit_code(), 4);
        }
    }
}
