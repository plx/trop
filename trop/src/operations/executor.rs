//! Plan execution engine.
//!
//! This module implements the executor that takes operation plans
//! and applies them to the database.

use std::collections::HashMap;

use crate::database::Database;
use crate::error::{Error, Result};
use crate::verified::reserve::{self as reserve_kernel, ReportedPort};
use crate::{Port, ReservationKey};
use rusqlite::Connection;

use super::plan::{OperationPlan, PlanAction};
use super::reserve::execute_reserve_after_cleanup;
use super::reserve_group::execute_group_allocation;

#[derive(Default)]
struct ActionOutcome {
    allocated_ports: Option<HashMap<String, Port>>,
    port: Option<Port>,
    warnings: Vec<String>,
}

/// Result of executing a plan.
///
/// This struct provides information about what happened during execution,
/// including whether it was a dry run and what actions were taken.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Whether the execution was successful.
    pub success: bool,

    /// Whether this was a dry-run (no actual changes made).
    pub dry_run: bool,

    /// Descriptions of actions that were taken (or would be taken in dry-run).
    pub actions_taken: Vec<String>,

    /// Warnings from the plan.
    pub warnings: Vec<String>,

    /// The port that was reserved (if applicable).
    pub port: Option<Port>,

    /// Allocated ports for group operations (tag -> port mapping).
    pub allocated_ports: Option<HashMap<String, Port>>,
}

impl ExecutionResult {
    /// Creates a successful execution result.
    fn success(
        plan: &OperationPlan,
        port: Option<Port>,
        allocated_ports: Option<HashMap<String, Port>>,
    ) -> Self {
        Self {
            success: true,
            dry_run: false,
            actions_taken: plan.actions.iter().map(PlanAction::description).collect(),
            warnings: plan.warnings.clone(),
            port,
            allocated_ports,
        }
    }

    /// Creates a dry-run execution result.
    fn dry_run(
        plan: &OperationPlan,
        port: Option<Port>,
        allocated_ports: Option<HashMap<String, Port>>,
    ) -> Self {
        Self {
            success: true,
            dry_run: true,
            actions_taken: plan.actions.iter().map(PlanAction::description).collect(),
            warnings: plan.warnings.clone(),
            port,
            allocated_ports,
        }
    }
}

/// Executes operation plans against the database.
///
/// The executor can run in normal mode (applying changes) or dry-run mode
/// (validating without changes).
///
/// # Examples
///
/// ```no_run
/// use trop::operations::{PlanExecutor, ReservePlan, ReserveOptions};
/// use trop::{Database, DatabaseConfig, ReservationKey, Port};
/// use trop::config::ConfigBuilder;
/// use std::path::PathBuf;
///
/// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
/// let config = ConfigBuilder::new().build().unwrap();
/// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
/// let port = Port::try_from(8080).unwrap();
///
/// let options = ReserveOptions::new(key, Some(port))
///     .with_allow_unrelated_path(true);
/// let plan = ReservePlan::new(options, &config).build_plan(db.connection()).unwrap();
///
/// // Normal execution
/// let mut executor = PlanExecutor::new(db.connection());
/// let result = executor.execute(&plan).unwrap();
/// assert!(result.success);
///
/// // Dry-run execution
/// let mut executor = PlanExecutor::new(db.connection()).dry_run();
/// let result = executor.execute(&plan).unwrap();
/// assert!(result.dry_run);
/// ```
pub struct PlanExecutor<'conn> {
    conn: &'conn Connection,
    dry_run: bool,
}

impl<'conn> PlanExecutor<'conn> {
    /// Creates a new plan executor.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::PlanExecutor;
    /// use trop::{Database, DatabaseConfig};
    ///
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let executor = PlanExecutor::new(db.connection());
    /// ```
    #[must_use]
    pub const fn new(conn: &'conn Connection) -> Self {
        Self {
            conn,
            dry_run: false,
        }
    }

    /// Sets the executor to dry-run mode.
    ///
    /// In dry-run mode, the executor validates the plan but does not
    /// actually modify the database.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::PlanExecutor;
    /// use trop::{Database, DatabaseConfig};
    ///
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let executor = PlanExecutor::new(db.connection()).dry_run();
    /// ```
    #[must_use]
    pub const fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }

    /// Executes the given plan.
    ///
    /// If in dry-run mode, validates the plan but makes no database changes.
    /// Otherwise, applies all actions in the plan to the database.
    ///
    /// # Errors
    ///
    /// Returns an error if any action fails to execute.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::{PlanExecutor, OperationPlan};
    /// use trop::{Database, DatabaseConfig};
    ///
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let plan = OperationPlan::new("Test operation");
    ///
    /// let mut executor = PlanExecutor::new(db.connection());
    /// let result = executor.execute(&plan).unwrap();
    /// ```
    pub fn execute(&mut self, plan: &OperationPlan) -> Result<ExecutionResult> {
        if self.dry_run {
            // A dry run executes nothing: deferred allocations have produced
            // no port; a written reservation's port comes from the plan and
            // a refreshed reservation's port is read as stored.
            let produced = vec![None; plan.actions.len()];
            let port = self.reported_port(plan, &produced)?;
            let allocated_ports = Self::extract_allocated_ports_dry_run(plan);
            return Ok(ExecutionResult::dry_run(plan, port, allocated_ports));
        }

        // Execute each action and collect any allocated ports
        let mut allocated_ports = None;
        let mut produced = Vec::with_capacity(plan.actions.len());
        let mut dynamic_warnings = Vec::new();
        for action in &plan.actions {
            let outcome = self.execute_action(action)?;
            if let Some(ports) = outcome.allocated_ports {
                allocated_ports = Some(ports);
            }
            produced.push(outcome.port);
            dynamic_warnings.extend(outcome.warnings);
        }

        // The same verified selection as a dry run, with the ports deferred
        // actions produced.
        let port = self.reported_port(plan, &produced)?;
        let mut result = ExecutionResult::success(plan, port, allocated_ports);
        result.warnings.extend(dynamic_warnings);

        Ok(result)
    }

    /// Executes a single action.
    ///
    fn execute_action(&mut self, action: &PlanAction) -> Result<ActionOutcome> {
        match action {
            PlanAction::CreateReservation(reservation) => {
                if reservation.requires_allocation_at_execution() {
                    let outcome = execute_reserve_after_cleanup(self.conn, reservation)?;
                    return Ok(ActionOutcome {
                        port: Some(outcome.port),
                        warnings: outcome.warning.into_iter().collect(),
                        ..ActionOutcome::default()
                    });
                }
                // Use simple create - transaction is managed by caller (CLI layer)
                // The UNIQUE constraint on the port column ensures we can't double-allocate
                Database::create_reservation_simple(self.conn, reservation)?;
                Ok(ActionOutcome::default())
            }
            PlanAction::UpdateReservation(reservation) => {
                if reservation.requires_allocation_at_execution() {
                    let outcome = execute_reserve_after_cleanup(self.conn, reservation)?;
                    return Ok(ActionOutcome {
                        port: Some(outcome.port),
                        warnings: outcome.warning.into_iter().collect(),
                        ..ActionOutcome::default()
                    });
                }
                // For updates, use the simple create (upsert) - no transaction needed here
                // Updates are for existing reservations where we're changing metadata
                Database::create_reservation_simple(self.conn, reservation)?;
                Ok(ActionOutcome::default())
            }
            PlanAction::UpdateLastUsed(key) => {
                // Refreshing a reservation that no longer exists must not
                // report success.
                if !Database::update_last_used_simple(self.conn, key)? {
                    return Err(Self::missing_reservation(key));
                }
                // The refresh reports the port the row holds as it is
                // refreshed (the port planning saw, unless the key was
                // reallocated after planning). A failed read is an error.
                Ok(ActionOutcome {
                    port: Some(self.stored_port(key)?),
                    ..ActionOutcome::default()
                })
            }
            PlanAction::DeleteReservation(key) => {
                Database::delete_reservation_simple(self.conn, key)?;
                Ok(ActionOutcome::default())
            }
            PlanAction::AllocateGroup {
                request,
                policy,
                full_config,
                occupancy_config,
            } => {
                // Reconciliation, then a fresh allocation with one automatic
                // cleanup and retry on exhaustion, as for single reservations.
                let outcome = execute_group_allocation(
                    self.conn,
                    request,
                    *policy,
                    full_config,
                    occupancy_config,
                )?;
                Ok(ActionOutcome {
                    allocated_ports: Some(outcome.result.allocations),
                    warnings: outcome.warning.into_iter().collect(),
                    ..ActionOutcome::default()
                })
            }
        }
    }

    /// The port a plan reports, chosen by the verified
    /// `verified::reserve::reported_port`: the last port a deferred
    /// allocation produced at execution (`produced[i]` for action `i`),
    /// otherwise the port of the first action that carries one in plan
    /// order. Live execution and dry runs both use it.
    ///
    /// A refresh carries the port its reservation holds: in live execution
    /// as read when it was refreshed (`produced[i]`), in a dry run as stored
    /// now. A failed read is an error, never a silently missing port.
    fn reported_port(
        &self,
        plan: &OperationPlan,
        produced: &[Option<Port>],
    ) -> Result<Option<Port>> {
        let mut actions = Vec::with_capacity(plan.actions.len());
        for (index, action) in plan.actions.iter().enumerate() {
            actions.push(match action {
                PlanAction::CreateReservation(r) | PlanAction::UpdateReservation(r) => {
                    if r.requires_allocation_at_execution() {
                        ReportedPort::Deferred(produced.get(index).copied().flatten())
                    } else {
                        ReportedPort::Static(r.port())
                    }
                }
                PlanAction::UpdateLastUsed(key) => {
                    ReportedPort::Touch(match produced.get(index).copied().flatten() {
                        Some(port) => port,
                        None => self.stored_port(key)?,
                    })
                }
                PlanAction::DeleteReservation(_) | PlanAction::AllocateGroup { .. } => {
                    // Release operations and group allocations don't return a single port
                    ReportedPort::NoPort
                }
            });
        }
        Ok(reserve_kernel::reported_port(&actions))
    }

    /// The port stored for `key`; a missing reservation or a failed read is
    /// an error.
    fn stored_port(&self, key: &ReservationKey) -> Result<Port> {
        Database::get_reservation(self.conn, key)?
            .map(|reservation| reservation.port())
            .ok_or_else(|| Self::missing_reservation(key))
    }

    fn missing_reservation(key: &ReservationKey) -> Error {
        Error::NotFound {
            resource: format!("reservation for {key}"),
        }
    }

    /// Extracts allocated ports from a plan's group allocation actions (dry-run).
    fn extract_allocated_ports_dry_run(_plan: &OperationPlan) -> Option<HashMap<String, Port>> {
        // In dry-run mode, we don't actually allocate ports
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PortConfig};
    use crate::database::test_util::create_test_database;
    use crate::operations::{ReserveOptions, ReservePlan};
    use crate::Reservation;
    use std::path::PathBuf;

    #[test]
    fn test_execute_create_reservation() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::CreateReservation(reservation.clone()));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert!(!result.dry_run);
        assert_eq!(result.actions_taken.len(), 1);

        // Verify reservation was created
        let loaded = Database::get_reservation(db.connection(), &key).unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().port(), port);
    }

    #[test]
    fn test_execute_update_last_used() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        // Create initial reservation
        db.create_reservation(&reservation).unwrap();

        // Wait to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_secs(2));

        let plan = OperationPlan::new("Test").add_action(PlanAction::UpdateLastUsed(key.clone()));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);

        // Verify timestamp was updated
        let loaded = Database::get_reservation(db.connection(), &key)
            .unwrap()
            .unwrap();
        assert!(loaded.last_used_at() > reservation.last_used_at());
    }

    #[test]
    fn test_execute_delete_reservation() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        // Create reservation
        db.create_reservation(&reservation).unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::DeleteReservation(key.clone()));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);

        // Verify reservation was deleted
        let loaded = Database::get_reservation(db.connection(), &key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_dry_run_does_not_modify_database() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::CreateReservation(reservation));

        let mut executor = PlanExecutor::new(db.connection()).dry_run();
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert!(result.dry_run);

        // Verify reservation was NOT created
        let loaded = Database::get_reservation(db.connection(), &key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_execute_multiple_actions() {
        let db = create_test_database();
        let key1 = ReservationKey::new(PathBuf::from("/test/path1"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/test/path2"), None).unwrap();
        let port1 = Port::try_from(8080).unwrap();
        let port2 = Port::try_from(8081).unwrap();
        let r1 = Reservation::builder(key1.clone(), port1).build().unwrap();
        let r2 = Reservation::builder(key2.clone(), port2).build().unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::CreateReservation(r1))
            .add_action(PlanAction::CreateReservation(r2));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert_eq!(result.actions_taken.len(), 2);

        // Verify both reservations were created
        assert!(Database::get_reservation(db.connection(), &key1)
            .unwrap()
            .is_some());
        assert!(Database::get_reservation(db.connection(), &key2)
            .unwrap()
            .is_some());
    }

    #[test]
    fn test_execution_result_includes_warnings() {
        let db = create_test_database();

        let plan = OperationPlan::new("Test")
            .add_warning("Warning 1")
            .add_warning("Warning 2");

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.warnings[0], "Warning 1");
        assert_eq!(result.warnings[1], "Warning 2");
    }

    #[test]
    fn test_extract_port_from_create_action() {
        let db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::CreateReservation(reservation));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert_eq!(result.port, Some(port));
    }

    #[test]
    fn test_extract_port_from_update_last_used() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        // Create initial reservation
        db.create_reservation(&reservation).unwrap();

        let plan = OperationPlan::new("Test").add_action(PlanAction::UpdateLastUsed(key));

        let mut executor = PlanExecutor::new(db.connection());
        let result = executor.execute(&plan).unwrap();

        assert_eq!(result.port, Some(port));
    }

    // ---------------------------------------------------------------------
    // The reported port (verified `reported_port`) and refresh failures
    // ---------------------------------------------------------------------

    fn refresh_plan(db: &Database, key: &ReservationKey) -> OperationPlan {
        let config = Config {
            ports: Some(PortConfig {
                min: 5000,
                max: Some(5010),
                max_offset: None,
            }),
            ..Default::default()
        };
        let options = ReserveOptions::new(key.clone(), None).with_allow_unrelated_path(true);
        let plan = ReservePlan::new(options, &config)
            .build_plan(db.connection())
            .unwrap();
        assert!(matches!(
            plan.actions.as_slice(),
            [PlanAction::UpdateLastUsed(_)]
        ));
        plan
    }

    fn stored(db: &mut Database, key: &ReservationKey, port: u16) -> Port {
        let port = Port::try_from(port).unwrap();
        db.create_reservation(&Reservation::builder(key.clone(), port).build().unwrap())
            .unwrap();
        port
    }

    /// Regression: the refreshed reservation's port used to be re-read after
    /// execution with every read error swallowed, so a successful `trop
    /// reserve` could report no port at all (exit 0, empty stdout). The
    /// refresh now reports the port the row holds, and a failed read fails
    /// the execution.
    #[test]
    fn refresh_reports_the_stored_port_and_propagates_read_errors() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/refresh"), None).unwrap();
        let port = stored(&mut db, &key, 5003);
        let plan = refresh_plan(&db, &key);
        let result = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(result.port, Some(port));

        // Make the stored row unreadable (an invalid identifier) while the
        // refresh itself still succeeds.
        db.connection()
            .execute("UPDATE reservations SET project = ''", [])
            .unwrap();
        assert!(Database::get_reservation(db.connection(), &key).is_err());

        assert!(PlanExecutor::new(db.connection()).execute(&plan).is_err());
    }

    /// A refresh planned before the key was reallocated (a plan executed
    /// outside the snapshot it was built in) reports the port the
    /// reservation holds when it is refreshed, never the stale port planning
    /// saw; so does a dry run of that plan.
    #[test]
    fn refresh_after_a_reallocation_reports_the_current_port() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/moved"), None).unwrap();
        stored(&mut db, &key, 5003);
        let plan = refresh_plan(&db, &key);
        db.connection()
            .execute("UPDATE reservations SET port = 5009", [])
            .unwrap();
        let moved = Some(Port::try_from(5009).unwrap());

        let dry = PlanExecutor::new(db.connection())
            .dry_run()
            .execute(&plan)
            .unwrap();
        assert_eq!(dry.port, moved);
        let live = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(live.port, moved);
    }

    /// A hand-built plan that refreshes a reservation and then deletes it
    /// reports the port the reservation held when it was refreshed.
    #[test]
    fn hand_built_refresh_then_delete_reports_the_refreshed_port() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/refdel"), None).unwrap();
        let port = stored(&mut db, &key, 5002);
        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::UpdateLastUsed(key.clone()))
            .add_action(PlanAction::DeleteReservation(key.clone()));

        let result = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(result.port, Some(port));
        assert!(Database::get_reservation(db.connection(), &key)
            .unwrap()
            .is_none());
    }

    /// Regression: a refresh of a reservation that disappeared after
    /// planning used to succeed and report no port.
    #[test]
    fn refresh_of_a_vanished_reservation_fails() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/vanished"), None).unwrap();
        stored(&mut db, &key, 5004);
        let plan = refresh_plan(&db, &key);
        Database::delete_reservation_simple(db.connection(), &key).unwrap();

        let error = PlanExecutor::new(db.connection())
            .execute(&plan)
            .unwrap_err();
        assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
    }

    /// Regression: for a plan assembled by hand, a failed read of the
    /// refreshed reservation is an error, not a missing port, in both
    /// modes.
    #[test]
    fn hand_built_refresh_propagates_read_errors() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/corrupt"), None).unwrap();
        stored(&mut db, &key, 5005);
        db.connection()
            .execute("UPDATE reservations SET project = ''", [])
            .unwrap();
        let plan = OperationPlan::new("Test").add_action(PlanAction::UpdateLastUsed(key));

        assert!(PlanExecutor::new(db.connection()).execute(&plan).is_err());
        assert!(PlanExecutor::new(db.connection())
            .dry_run()
            .execute(&plan)
            .is_err());
    }

    /// Regression: a dry run of a refresh used to report no port; it now
    /// reports the same port as live execution.
    #[test]
    fn dry_run_and_live_report_the_same_refresh_port() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/agree"), None).unwrap();
        let port = stored(&mut db, &key, 5006);
        let plan = refresh_plan(&db, &key);

        let dry = PlanExecutor::new(db.connection())
            .dry_run()
            .execute(&plan)
            .unwrap();
        let live = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(dry.port, Some(port));
        assert_eq!(live.port, Some(port));

        // A hand-built refresh of an existing row reads the stored port in
        // both modes.
        let hand_built = OperationPlan::new("Test").add_action(PlanAction::UpdateLastUsed(key));
        let dry = PlanExecutor::new(db.connection())
            .dry_run()
            .execute(&hand_built)
            .unwrap();
        assert_eq!(dry.port, Some(port));
    }

    #[test]
    fn reported_port_is_the_first_planned_port_in_plan_order() {
        let mut db = create_test_database();
        let touched = ReservationKey::new(PathBuf::from("/test/touched"), None).unwrap();
        let created = ReservationKey::new(PathBuf::from("/test/created"), None).unwrap();
        let deleted = ReservationKey::new(PathBuf::from("/test/deleted"), None).unwrap();
        let touched_port = stored(&mut db, &touched, 5007);
        let created_port = Port::try_from(5008).unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::DeleteReservation(deleted))
            .add_action(PlanAction::UpdateLastUsed(touched))
            .add_action(PlanAction::CreateReservation(
                Reservation::builder(created, created_port).build().unwrap(),
            ));
        let dry = PlanExecutor::new(db.connection())
            .dry_run()
            .execute(&plan)
            .unwrap();
        let live = PlanExecutor::new(db.connection()).execute(&plan).unwrap();
        assert_eq!(dry.port, Some(touched_port));
        assert_eq!(live.port, Some(touched_port));

        // No action carries a port: none is reported.
        let empty = OperationPlan::new("Test");
        let result = PlanExecutor::new(db.connection()).execute(&empty).unwrap();
        assert_eq!(result.port, None);
    }
}
