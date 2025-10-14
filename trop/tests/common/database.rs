//! Shared database test utilities.

use std::path::PathBuf;
use trop::database::{Database, DatabaseConfig};
use trop::{Port, Reservation, ReservationKey};

/// Creates a temporary test database that will be cleaned up when dropped.
///
/// Returns the database instance. The temporary directory is tied to the
/// database's lifetime through the test.
#[allow(dead_code)]
pub fn create_test_database() -> Database {
    let dir = tempfile::tempdir().unwrap();
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
#[allow(dead_code)]
pub fn create_test_reservation(path: &str, port: u16) -> Reservation {
    let key = ReservationKey::new(PathBuf::from(path), None).unwrap();
    let port = Port::try_from(port).unwrap();
    Reservation::builder(key, port).build().unwrap()
}
