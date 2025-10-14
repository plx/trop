//! Database layer for persistent storage of port reservations.
//!
//! This module provides a SQLite-based storage layer for managing port
//! reservations, including connection management, schema versioning,
//! and CRUD operations.
//!
//! # Examples
//!
//! ```no_run
//! use trop::database::{Database, DatabaseConfig};
//! use trop::{Reservation, ReservationKey, Port};
//! use std::path::PathBuf;
//!
//! // Open a database
//! let config = DatabaseConfig::new("/tmp/trop.db");
//! let mut db = Database::open(config).unwrap();
//!
//! // Create a reservation
//! let key = ReservationKey::new(PathBuf::from("/path/to/project"), None).unwrap();
//! let port = Port::try_from(8080).unwrap();
//! let reservation = Reservation::builder(key, port).build().unwrap();
//! db.create_reservation(&reservation).unwrap();
//!
//! // List all reservations
//! let all = db.list_all_reservations().unwrap();
//! for reservation in all {
//!     println!("{:?}", reservation);
//! }
//! ```

mod config;
mod connection;
pub mod migrations;
mod operations;
mod schema;
mod transaction;

// Re-export public API
pub use config::{default_data_dir, resolve_database_path, DatabaseConfig};
pub use connection::Database;

// Re-export migration functions for advanced use cases
pub use migrations::{check_schema_compatibility, get_schema_version, initialize_schema};
