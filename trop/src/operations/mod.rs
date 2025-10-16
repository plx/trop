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
//! let options = ReserveOptions {
//!     key,
//!     project: Some("my-project".to_string()),
//!     task: None,
//!     port: Some(port),
//!     preferred_port: None,
//!     ignore_occupied: false,
//!     ignore_exclusions: false,
//!     force: false,
//!     allow_unrelated_path: true,
//!     allow_project_change: false,
//!     allow_task_change: false,
//! };
//!
//! // Generate plan
//! let plan = ReservePlan::new(options, &config).build_plan(&db).unwrap();
//!
//! // Execute plan
//! let mut executor = PlanExecutor::new(&mut db);
//! let result = executor.execute(&plan).unwrap();
//! ```

pub mod cleanup;
pub mod executor;
pub mod plan;
pub mod release;
pub mod reserve;

pub use cleanup::{AutocleanResult, CleanupOperations, ExpireResult, PruneResult};
pub use executor::{ExecutionResult, PlanExecutor};
pub use plan::{OperationPlan, PlanAction};
pub use release::{ReleaseOptions, ReleasePlan};
pub use reserve::{ReserveOptions, ReservePlan};
