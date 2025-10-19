//! Plan execution engine.
//!
//! This module implements the executor that takes operation plans
//! and applies them to the database.

use std::collections::HashMap;

use crate::database::Database;
use crate::error::Result;
use crate::port::allocator::allocator_from_config;
use crate::Port;

use super::plan::{OperationPlan, PlanAction};

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
/// let plan = ReservePlan::new(options, &config).build_plan(&mut db).unwrap();
///
/// // Normal execution
/// let mut executor = PlanExecutor::new(&mut db);
/// let result = executor.execute(&plan).unwrap();
/// assert!(result.success);
///
/// // Dry-run execution
/// let mut executor = PlanExecutor::new(&mut db).dry_run();
/// let result = executor.execute(&plan).unwrap();
/// assert!(result.dry_run);
/// ```
pub struct PlanExecutor<'a> {
    db: &'a mut Database,
    dry_run: bool,
}

impl<'a> PlanExecutor<'a> {
    /// Creates a new plan executor.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::operations::PlanExecutor;
    /// use trop::{Database, DatabaseConfig};
    ///
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let executor = PlanExecutor::new(&mut db);
    /// ```
    #[must_use]
    pub const fn new(db: &'a mut Database) -> Self {
        Self { db, dry_run: false }
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
    /// let executor = PlanExecutor::new(&mut db).dry_run();
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
    /// let mut executor = PlanExecutor::new(&mut db);
    /// let result = executor.execute(&plan).unwrap();
    /// ```
    pub fn execute(&mut self, plan: &OperationPlan) -> Result<ExecutionResult> {
        if self.dry_run {
            // In dry-run mode, extract port without database queries
            let port = Self::extract_port_from_plan_dry_run(plan);
            let allocated_ports = Self::extract_allocated_ports_dry_run(plan);
            return Ok(ExecutionResult::dry_run(plan, port, allocated_ports));
        }

        // Execute each action and collect any allocated ports
        let mut allocated_ports = None;
        for action in &plan.actions {
            if let Some(ports) = self.execute_action(action)? {
                allocated_ports = Some(ports);
            }
        }

        // Extract the port from the plan after execution
        let port = self.extract_port_from_plan(plan);

        Ok(ExecutionResult::success(plan, port, allocated_ports))
    }

    /// Executes a single action.
    ///
    /// Returns `Ok(Some(ports))` for group allocations, `Ok(None)` for other actions.
    fn execute_action(&mut self, action: &PlanAction) -> Result<Option<HashMap<String, Port>>> {
        match action {
            PlanAction::CreateReservation(reservation) => {
                // Use atomic create to prevent TOCTOU races in concurrent scenarios
                // The UNIQUE constraint on the port column ensures atomicity
                let created = self.db.try_create_reservation_atomic(reservation)?;
                if !created {
                    // Port was allocated by another process between planning and execution
                    // This should be rare but can happen in high-concurrency scenarios
                    return Err(crate::error::Error::Validation {
                        field: "port".into(),
                        message: format!(
                            "Port {} was allocated by another process during execution. Please retry the operation.",
                            reservation.port().value()
                        ),
                    });
                }
                Ok(None)
            }
            PlanAction::UpdateReservation(reservation) => {
                // For updates, use the regular create_reservation (upsert)
                // Updates are for existing reservations where we're changing metadata,
                // not allocating new ports, so atomicity isn't critical
                self.db.create_reservation(reservation)?;
                Ok(None)
            }
            PlanAction::UpdateLastUsed(key) => {
                self.db.update_last_used(key).map(|_| ())?;
                Ok(None)
            }
            PlanAction::DeleteReservation(key) => {
                self.db.delete_reservation(key).map(|_| ())?;
                Ok(None)
            }
            PlanAction::AllocateGroup {
                request,
                full_config,
                occupancy_config,
            } => {
                let allocator = allocator_from_config(full_config)?;
                let result = allocator.allocate_group(self.db, request, occupancy_config)?;
                Ok(Some(result.allocations))
            }
        }
    }

    /// Extracts the port from a plan's actions.
    ///
    /// This is used to return the reserved port to the caller.
    /// This method may perform database queries for `UpdateLastUsed` actions.
    fn extract_port_from_plan(&self, plan: &OperationPlan) -> Option<Port> {
        for action in &plan.actions {
            match action {
                PlanAction::CreateReservation(r) | PlanAction::UpdateReservation(r) => {
                    return Some(r.port());
                }
                PlanAction::UpdateLastUsed(key) => {
                    // For idempotent case, get the existing reservation's port
                    if let Ok(Some(reservation)) = self.db.get_reservation(key) {
                        return Some(reservation.port());
                    }
                }
                PlanAction::DeleteReservation(_) | PlanAction::AllocateGroup { .. } => {
                    // Release operations and group allocations don't return a single port
                }
            }
        }
        None
    }

    /// Extracts the port from a plan's actions without database queries.
    ///
    /// This is used during dry-run mode to avoid unnecessary database access.
    /// For `UpdateLastUsed` actions, this returns None rather than querying
    /// the database, which is acceptable for dry-run reporting.
    fn extract_port_from_plan_dry_run(plan: &OperationPlan) -> Option<Port> {
        for action in &plan.actions {
            match action {
                PlanAction::CreateReservation(r) | PlanAction::UpdateReservation(r) => {
                    return Some(r.port());
                }
                PlanAction::UpdateLastUsed(_)
                | PlanAction::DeleteReservation(_)
                | PlanAction::AllocateGroup { .. } => {
                    // In dry-run mode, we don't query the database.
                    // For UpdateLastUsed and AllocateGroup, return None.
                    // Release operations also don't return a port.
                }
            }
        }
        None
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
    use crate::database::test_util::create_test_database;
    use crate::{Reservation, ReservationKey};
    use std::path::PathBuf;

    #[test]
    fn test_execute_create_reservation() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::CreateReservation(reservation.clone()));

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert!(!result.dry_run);
        assert_eq!(result.actions_taken.len(), 1);

        // Verify reservation was created
        let loaded = db.get_reservation(&key).unwrap();
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

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);

        // Verify timestamp was updated
        let loaded = db.get_reservation(&key).unwrap().unwrap();
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

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);

        // Verify reservation was deleted
        let loaded = db.get_reservation(&key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_dry_run_does_not_modify_database() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::CreateReservation(reservation));

        let mut executor = PlanExecutor::new(&mut db).dry_run();
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert!(result.dry_run);

        // Verify reservation was NOT created
        let loaded = db.get_reservation(&key).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_execute_multiple_actions() {
        let mut db = create_test_database();
        let key1 = ReservationKey::new(PathBuf::from("/test/path1"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/test/path2"), None).unwrap();
        let port1 = Port::try_from(8080).unwrap();
        let port2 = Port::try_from(8081).unwrap();
        let r1 = Reservation::builder(key1.clone(), port1).build().unwrap();
        let r2 = Reservation::builder(key2.clone(), port2).build().unwrap();

        let plan = OperationPlan::new("Test")
            .add_action(PlanAction::CreateReservation(r1))
            .add_action(PlanAction::CreateReservation(r2));

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert!(result.success);
        assert_eq!(result.actions_taken.len(), 2);

        // Verify both reservations were created
        assert!(db.get_reservation(&key1).unwrap().is_some());
        assert!(db.get_reservation(&key2).unwrap().is_some());
    }

    #[test]
    fn test_execution_result_includes_warnings() {
        let mut db = create_test_database();

        let plan = OperationPlan::new("Test")
            .add_warning("Warning 1")
            .add_warning("Warning 2");

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert_eq!(result.warnings.len(), 2);
        assert_eq!(result.warnings[0], "Warning 1");
        assert_eq!(result.warnings[1], "Warning 2");
    }

    #[test]
    fn test_extract_port_from_create_action() {
        let mut db = create_test_database();
        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();

        let plan =
            OperationPlan::new("Test").add_action(PlanAction::CreateReservation(reservation));

        let mut executor = PlanExecutor::new(&mut db);
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

        let mut executor = PlanExecutor::new(&mut db);
        let result = executor.execute(&plan).unwrap();

        assert_eq!(result.port, Some(port));
    }
}
