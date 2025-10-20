//! Reservation operations using the plan-execute pattern.
//!
//! This module provides a plan-execute pattern for reservation operations,
//! separating planning from execution to enable dry-run mode, better testing,
//! and clear error messages.
//!
//! # Architecture
//!
//! Operations are split into two phases:
//! 1. **Planning**: Analyzes the request, validates constraints, builds a plan
//! 2. **Execution**: Takes the plan and performs actual database operations
//!
//! # Examples
//!
//! ```no_run
//! use trop::operations::{ReservePlan, ReserveOptions, PlanExecutor};
//! use trop::{Database, DatabaseConfig, ReservationKey, Port};
//! use trop::config::ConfigBuilder;
//! use std::path::PathBuf;
//!
//! let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
//! let config = ConfigBuilder::new().build().unwrap();
//! let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
//! let port = Port::try_from(8080).unwrap();
//!
//! let options = ReserveOptions::new(key, Some(port))
//!     .with_project(Some("my-project".to_string()))
//!     .with_allow_unrelated_path(true);
//!
//! // Generate plan
//! let plan = ReservePlan::new(options, &config).build_plan(db.connection()).unwrap();
//!
//! // Execute plan
//! let mut executor = PlanExecutor::new(db.connection());
//! let result = executor.execute(&plan).unwrap();
//! ```

pub mod autoreserve;
pub mod cleanup;
pub mod executor;
pub mod inference;
pub mod init;
pub mod migrate;
pub mod plan;
pub mod release;
pub mod reserve;
pub mod reserve_group;

#[cfg(test)]
mod proptests;

pub use autoreserve::{AutoreserveOptions, AutoreservePlan};
pub use cleanup::{AutocleanResult, CleanupOperations, ExpireResult, PruneResult};
pub use executor::{ExecutionResult, PlanExecutor};
pub use init::{init_database, InitOptions, InitResult};
pub use migrate::{execute_migrate, MigrateOptions, MigratePlan, MigrateResult, MigrationItem};
pub use plan::{OperationPlan, PlanAction};
pub use release::{ReleaseOptions, ReleasePlan};
pub use reserve::{ReserveOptions, ReservePlan};
pub use reserve_group::{ReserveGroupOptions, ReserveGroupPlan};
