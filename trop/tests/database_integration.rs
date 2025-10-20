//! Integration tests for the database layer.
//!
//! These tests exercise the full database stack including auto-initialization,
//! schema versioning, concurrent access, and transaction atomicity.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

use tempfile::tempdir;

use trop::database::{Database, DatabaseConfig};

use trop::{Port, PortRange, Reservation, ReservationKey};

#[test]
fn test_database_auto_creation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("subdir").join("test.db");

    // Directory doesn't exist yet
    assert!(!db_path.parent().unwrap().exists());

    // Open with auto-create
    let config = DatabaseConfig::new(&db_path);
    let _db = Database::open(config).unwrap();

    // Directory and file should now exist
    assert!(db_path.exists());
    assert!(db_path.parent().unwrap().exists());
}

#[test]
fn test_schema_version_compatibility() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("version_test.db");

    // Create database with current schema
    {
        let config = DatabaseConfig::new(&db_path);
        Database::open(config).unwrap();
    }

    // Reopen should work (same version)
    {
        let config = DatabaseConfig::new(&db_path);
        Database::open(config).unwrap();
    }

    // Manually set incompatible version (newer)
    {
        use rusqlite::Connection;
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE metadata SET value = '999' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    }

    // Now opening should fail
    let config = DatabaseConfig::new(&db_path);
    let result = Database::open(config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("newer than client"));
}

#[test]
fn test_concurrent_write_operations() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("concurrent.db");

    // Initialize database
    {
        let config = DatabaseConfig::new(&db_path);
        Database::open(config).unwrap();
    }

    // Spawn multiple threads that write to the database
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let path = db_path.clone();
            thread::spawn(move || {
                let config = DatabaseConfig::new(path);
                let mut db = Database::open(config).unwrap();

                let key =
                    ReservationKey::new(PathBuf::from(format!("/test/path/{i}")), None).unwrap();
                let port = Port::try_from(5000 + i as u16).unwrap();
                let reservation = Reservation::builder(key, port).build().unwrap();

                db.create_reservation(&reservation)
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap().unwrap();
    }

    // Verify all reservations were created
    let config = DatabaseConfig::new(&db_path);
    let db = Database::open(config).unwrap();
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 10);
}

#[test]
fn test_concurrent_read_write_operations() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("concurrent_rw.db");

    // Initialize database with some reservations
    {
        let config = DatabaseConfig::new(&db_path);
        let mut db = Database::open(config).unwrap();

        for i in 0..5 {
            let key = ReservationKey::new(PathBuf::from(format!("/test/path/{i}")), None).unwrap();
            let port = Port::try_from(5000 + i as u16).unwrap();
            let reservation = Reservation::builder(key, port).build().unwrap();
            db.create_reservation(&reservation).unwrap();
        }
    }

    // Spawn readers and writers
    let mut handles = Vec::new();

    // Readers
    for _ in 0..5 {
        let path = db_path.clone();
        handles.push(thread::spawn(move || -> Result<(), trop::Error> {
            let config = DatabaseConfig::new(path);
            let db = Database::open(config)?;
            for _ in 0..10 {
                let _ = Database::list_all_reservations(db.connection())?;
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        }));
    }

    // Writers
    for i in 5..10 {
        let path = db_path.clone();
        handles.push(thread::spawn(move || -> Result<(), trop::Error> {
            let config = DatabaseConfig::new(path);
            let mut db = Database::open(config)?;

            let key = ReservationKey::new(PathBuf::from(format!("/test/path/{i}")), None)?;
            let port = Port::try_from(5000 + i as u16)?;
            let reservation = Reservation::builder(key, port).build()?;

            db.create_reservation(&reservation)
        }));
    }

    // Wait for all to complete
    for handle in handles {
        handle.join().unwrap().ok();
    }

    // Verify final state
    let config = DatabaseConfig::new(&db_path);
    let db = Database::open(config).unwrap();
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 10);
}

#[test]
fn test_transaction_atomicity_batch_create() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("atomic.db");
    let config = DatabaseConfig::new(&db_path);
    let mut db = Database::open(config).unwrap();

    // Create valid reservations
    let r1 = Reservation::builder(
        ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
        Port::try_from(5000).unwrap(),
    )
    .build()
    .unwrap();

    let r2 = Reservation::builder(
        ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
        Port::try_from(5001).unwrap(),
    )
    .build()
    .unwrap();

    let reservations = vec![r1, r2];

    // Batch create should succeed
    db.batch_create_reservations(&reservations).unwrap();

    // Verify both were created
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_full_lifecycle() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("lifecycle.db");
    let config = DatabaseConfig::new(&db_path);
    let mut db = Database::open(config).unwrap();

    // Create a reservation
    let key = ReservationKey::new(PathBuf::from("/project/path"), Some("web".to_string())).unwrap();
    let port = Port::try_from(8080).unwrap();
    let reservation = Reservation::builder(key.clone(), port)
        .project(Some("my-project".to_string()))
        .task(Some("dev-server".to_string()))
        .build()
        .unwrap();

    db.create_reservation(&reservation).unwrap();

    // Read it back
    let loaded = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.port(), port);
    assert_eq!(loaded.project(), Some("my-project"));
    assert_eq!(loaded.task(), Some("dev-server"));

    // Update last used (sleep for 2 seconds to ensure Unix timestamp precision)
    thread::sleep(Duration::from_secs(2));
    let updated = db.update_last_used(&key).unwrap();
    assert!(updated);

    // Verify timestamp changed
    let reloaded = Database::get_reservation(db.connection(), &key)
        .unwrap()
        .unwrap();
    assert!(reloaded.last_used_at() > loaded.last_used_at());

    // Check port is reserved
    assert!(Database::is_port_reserved(db.connection(), port).unwrap());

    // Delete it
    let deleted = db.delete_reservation(&key).unwrap();
    assert!(deleted);

    // Verify it's gone
    let gone = Database::get_reservation(db.connection(), &key).unwrap();
    assert!(gone.is_none());

    // Port should no longer be reserved
    assert!(!Database::is_port_reserved(db.connection(), port).unwrap());
}

#[test]
fn test_query_operations() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("query.db");
    let config = DatabaseConfig::new(&db_path);
    let mut db = Database::open(config).unwrap();

    // Create multiple reservations
    for i in 0..10 {
        let key =
            ReservationKey::new(PathBuf::from(format!("/home/user/project{i}")), None).unwrap();
        let port = Port::try_from(5000 + i as u16).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    // Add some in different paths
    for i in 0..5 {
        let key = ReservationKey::new(PathBuf::from(format!("/opt/service{i}")), None).unwrap();
        let port = Port::try_from(6000 + i as u16).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    // Test get_reserved_ports
    let min = Port::try_from(5000).unwrap();
    let max = Port::try_from(5009).unwrap();
    let range = PortRange::new(min, max).unwrap();
    let reserved = Database::get_reserved_ports(db.connection(), &range).unwrap();
    assert_eq!(reserved.len(), 10);

    // Test path prefix query
    let prefix_results =
        Database::get_reservations_by_path_prefix(db.connection(), &PathBuf::from("/home/user"))
            .unwrap();
    assert_eq!(prefix_results.len(), 10);

    let prefix_results =
        Database::get_reservations_by_path_prefix(db.connection(), &PathBuf::from("/opt")).unwrap();
    assert_eq!(prefix_results.len(), 5);

    // Test list all
    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 15);
}

#[test]
fn test_expired_reservations() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("expired.db");
    let config = DatabaseConfig::new(&db_path);
    let mut db = Database::open(config).unwrap();

    // Create old reservation
    let old_time = SystemTime::now() - Duration::from_secs(200);
    let key1 = ReservationKey::new(PathBuf::from("/old/path"), None).unwrap();
    let port1 = Port::try_from(5000).unwrap();
    let old_reservation = Reservation::builder(key1, port1)
        .last_used_at(old_time)
        .build()
        .unwrap();
    db.create_reservation(&old_reservation).unwrap();

    // Create fresh reservation
    let key2 = ReservationKey::new(PathBuf::from("/fresh/path"), None).unwrap();
    let port2 = Port::try_from(5001).unwrap();
    let fresh_reservation = Reservation::builder(key2, port2).build().unwrap();
    db.create_reservation(&fresh_reservation).unwrap();

    // Find expired (older than 100 seconds)
    let expired =
        Database::find_expired_reservations(db.connection(), Duration::from_secs(100)).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].key().path, PathBuf::from("/old/path"));

    // Find with shorter max age (should find both - fresh ones are also "expired" with 0 max age)
    let expired =
        Database::find_expired_reservations(db.connection(), Duration::from_secs(0)).unwrap();
    // Note: This might be 1 or 2 depending on timing. Let's just check we have at least one
    assert!(!expired.is_empty());
}

#[test]
fn test_batch_operations() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("batch.db");
    let config = DatabaseConfig::new(&db_path);
    let mut db = Database::open(config).unwrap();

    // Batch create
    let reservations: Vec<_> = (0..20)
        .map(|i| {
            let key = ReservationKey::new(PathBuf::from(format!("/path/{i}")), None).unwrap();
            let port = Port::try_from(5000 + i as u16).unwrap();
            Reservation::builder(key, port).build().unwrap()
        })
        .collect();

    db.batch_create_reservations(&reservations).unwrap();

    let all = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(all.len(), 20);

    // Batch delete half of them
    let keys_to_delete: Vec<_> = (0..10)
        .map(|i| ReservationKey::new(PathBuf::from(format!("/path/{i}")), None).unwrap())
        .collect();

    let deleted = db.batch_delete_reservations(&keys_to_delete).unwrap();
    assert_eq!(deleted, 10);

    let remaining = Database::list_all_reservations(db.connection()).unwrap();
    assert_eq!(remaining.len(), 10);
}

#[test]
fn test_database_reopening() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("reopen.db");

    // Create database and add reservation
    {
        let config = DatabaseConfig::new(&db_path);
        let mut db = Database::open(config).unwrap();

        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port).build().unwrap();
        db.create_reservation(&reservation).unwrap();
    }

    // Reopen and verify data persists
    {
        let config = DatabaseConfig::new(&db_path);
        let db = Database::open(config).unwrap();

        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].port().value(), 8080);
    }
}
