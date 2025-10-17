#![deny(missing_docs, unsafe_code)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

//! # trop
//!
//! A library for managing ephemeral port reservations.
//!
//! This library provides core types and functionality for reserving, tracking,
//! and managing port allocations across development projects.
//!
//! ## Core Types
//!
//! - [`Port`] and [`PortRange`]: Network port types with validation
//! - [`Reservation`] and [`ReservationKey`]: Port reservation tracking
//! - [`Error`] and [`Result`]: Error handling types
//! - [`Logger`] and [`LogLevel`]: Logging infrastructure
//!
//! ## Examples
//!
//! ```
//! use trop::{Port, PortRange};
//!
//! // Create a valid port
//! let port = Port::try_from(8080).unwrap();
//! assert_eq!(port.value(), 8080);
//!
//! // Create a port range
//! let min = Port::try_from(5000).unwrap();
//! let max = Port::try_from(5010).unwrap();
//! let range = PortRange::new(min, max).unwrap();
//! assert_eq!(range.len(), 11);
//! ```

pub mod config;
pub mod database;
pub mod error;
pub mod logging;
pub mod operations;
pub mod path;
pub mod port;
pub mod reservation;

// Re-export key types at crate root for convenience
pub use config::{Config, ConfigBuilder};
pub use database::{Database, DatabaseConfig};
pub use error::{Error, PortUnavailableReason, Result};
pub use logging::{init_logger, LogLevel, Logger};
pub use operations::{
    AutocleanResult, CleanupOperations, ExecutionResult, ExpireResult, OperationPlan, PlanAction,
    PlanExecutor, PruneResult, ReleaseOptions, ReleasePlan, ReserveOptions, ReservePlan,
};
pub use path::{PathProvenance, PathRelationship, PathResolver};
pub use port::{Port, PortRange};
pub use reservation::{Reservation, ReservationKey};
