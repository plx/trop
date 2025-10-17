//! Transaction management utilities.
//!
//! This module provides transaction helpers for complex database operations.

use rusqlite::{params, TransactionBehavior};

use crate::error::Result;
use crate::{Reservation, ReservationKey};

use super::connection::Database;
use super::operations::systemtime_to_unix_secs;
use super::schema::{DELETE_RESERVATION, INSERT_RESERVATION};

impl Database {
    /// Creates multiple reservations in a single transaction.
    ///
    /// This operation is atomic - either all reservations are created
    /// or none are. This is more efficient than creating reservations
    /// individually when you have multiple reservations to create.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transaction cannot be started
    /// - Any insert fails
    /// - The transaction cannot be committed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::{Reservation, ReservationKey, Port};
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let reservations = vec![
    ///     Reservation::builder(
    ///         ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
    ///         Port::try_from(8080).unwrap()
    ///     ).build().unwrap(),
    ///     Reservation::builder(
    ///         ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
    ///         Port::try_from(8081).unwrap()
    ///     ).build().unwrap(),
    /// ];
    ///
    /// db.batch_create_reservations(&reservations).unwrap();
    /// ```
    pub fn batch_create_reservations(&mut self, reservations: &[Reservation]) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        {
            let mut stmt = tx.prepare(INSERT_RESERVATION)?;
            for reservation in reservations {
                let created_secs = systemtime_to_unix_secs(reservation.created_at())?;
                let last_used_secs = systemtime_to_unix_secs(reservation.last_used_at())?;

                stmt.execute(params![
                    reservation.key().path.to_string_lossy().to_string(),
                    reservation.key().tag,
                    reservation.port().value(),
                    reservation.project(),
                    reservation.task(),
                    created_secs,
                    last_used_secs,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Deletes multiple reservations in a single transaction.
    ///
    /// This operation is atomic - either all reservations are deleted
    /// or none are. Returns the total number of reservations actually deleted.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The transaction cannot be started
    /// - Any delete fails
    /// - The transaction cannot be committed
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    /// use trop::ReservationKey;
    /// use std::path::PathBuf;
    ///
    /// let config = DatabaseConfig::new("/tmp/trop.db");
    /// let mut db = Database::open(config).unwrap();
    ///
    /// let keys = vec![
    ///     ReservationKey::new(PathBuf::from("/path1"), None).unwrap(),
    ///     ReservationKey::new(PathBuf::from("/path2"), None).unwrap(),
    /// ];
    ///
    /// let deleted = db.batch_delete_reservations(&keys).unwrap();
    /// println!("Deleted {} reservations", deleted);
    /// ```
    pub fn batch_delete_reservations(&mut self, keys: &[ReservationKey]) -> Result<usize> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let mut total_deleted = 0;
        {
            let mut stmt = tx.prepare(DELETE_RESERVATION)?;
            for key in keys {
                let rows_affected =
                    stmt.execute(params![key.path.to_string_lossy().to_string(), key.tag])?;
                total_deleted += rows_affected;
            }
        }

        tx.commit()?;
        Ok(total_deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_util::{create_test_database, create_test_reservation};
    use crate::ReservationKey;
    use std::path::PathBuf;

    #[test]
    fn test_batch_create_reservations() {
        let mut db = create_test_database();

        let reservations = vec![
            create_test_reservation("/path1", 5000),
            create_test_reservation("/path2", 5001),
            create_test_reservation("/path3", 5002),
        ];

        db.batch_create_reservations(&reservations).unwrap();

        // Verify all were created
        let all = db.list_all_reservations().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_batch_create_empty() {
        let mut db = create_test_database();

        let reservations: Vec<Reservation> = vec![];
        db.batch_create_reservations(&reservations).unwrap();

        let all = db.list_all_reservations().unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_batch_delete_reservations() {
        let mut db = create_test_database();

        // Create some reservations
        let r1 = create_test_reservation("/path1", 5000);
        let r2 = create_test_reservation("/path2", 5001);
        let r3 = create_test_reservation("/path3", 5002);

        db.create_reservation(&r1).unwrap();
        db.create_reservation(&r2).unwrap();
        db.create_reservation(&r3).unwrap();

        // Delete two of them
        let keys = vec![r1.key().clone(), r2.key().clone()];
        let deleted = db.batch_delete_reservations(&keys).unwrap();

        assert_eq!(deleted, 2);

        // Verify only one remains
        let all = db.list_all_reservations().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key().path, PathBuf::from("/path3"));
    }

    #[test]
    fn test_batch_delete_nonexistent() {
        let mut db = create_test_database();

        let keys = vec![
            ReservationKey::new(PathBuf::from("/nonexistent1"), None).unwrap(),
            ReservationKey::new(PathBuf::from("/nonexistent2"), None).unwrap(),
        ];

        let deleted = db.batch_delete_reservations(&keys).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_batch_delete_mixed() {
        let mut db = create_test_database();

        // Create one reservation
        let r1 = create_test_reservation("/path1", 5000);
        db.create_reservation(&r1).unwrap();

        // Try to delete it and a nonexistent one
        let keys = vec![
            r1.key().clone(),
            ReservationKey::new(PathBuf::from("/nonexistent"), None).unwrap(),
        ];

        let deleted = db.batch_delete_reservations(&keys).unwrap();
        assert_eq!(deleted, 1);

        let all = db.list_all_reservations().unwrap();
        assert_eq!(all.len(), 0);
    }
}
