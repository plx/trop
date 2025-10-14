//! Shared test utilities for database unit tests.
//!
//! This module provides helper functions used across multiple database test modules.

use std::path::PathBuf;
use tempfile::tempdir;

use crate::database::{Database, DatabaseConfig};
use crate::{Port, Reservation, ReservationKey};

/// Creates a temporary test database that will be cleaned up automatically.
///
/// # Panics
///
/// Panics if the temporary directory or database cannot be created.
/// This is acceptable in test code where we want to fail fast.
#[must_use]
pub fn create_test_database() -> Database {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.db");
    let config = DatabaseConfig::new(path);
    let db = Database::open(config).unwrap();

    // Prevent the TempDir from being dropped immediately
    std::mem::forget(dir);

    db
}

/// Creates a test reservation with the given path and port.
///
/// Uses default values for optional fields (no tag, no project, no task).
///
/// # Panics
///
/// Panics if the reservation key or reservation cannot be created.
/// This is acceptable in test code where we want to fail fast.
#[must_use]
pub fn create_test_reservation(path: &str, port: u16) -> Reservation {
    let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
    let port = Port::try_from(port).unwrap();
    Reservation::builder(key, port).build().unwrap()
}
