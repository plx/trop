//! Transaction management utilities.
//!
//! This module provides transaction helpers for complex database operations.

use rusqlite::{params, TransactionBehavior};

use crate::error::Result;
use crate::{Reservation, ReservationKey};

use super::connection::Database;
use super::operations::{encode_reservation, storable_key};
use super::schema::{DELETE_RESERVATION, INSERT_RESERVATION};

impl Database {
    /// Creates multiple reservations in a single transaction.
    ///
    /// This operation is atomic - either all reservations are created
    /// or none are. This is more efficient than creating reservations
    /// individually when you have multiple reservations to create.
    ///
    /// Every reservation is checked by the verified storage kernel before
    /// any SQL runs, exactly as [`Database::create_reservation`] checks one.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any reservation cannot be stored (a validation error; nothing is
    ///   written)
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
        let timeout = self.busy_timeout();
        self.batch_create_reservations_inner(reservations)
            .map_err(|error| error.classify_sqlite_lock(timeout, "creating a reservation batch"))
    }

    fn batch_create_reservations_inner(&mut self, reservations: &[Reservation]) -> Result<()> {
        let rows = reservations
            .iter()
            .map(encode_reservation)
            .collect::<Result<Vec<_>>>()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        {
            let mut delete_stmt = tx.prepare(DELETE_RESERVATION)?;
            let mut insert_stmt = tx.prepare(INSERT_RESERVATION)?;
            for row in &rows {
                delete_stmt.execute(params![row.path, row.tag])?;
                insert_stmt.execute(params![
                    row.path,
                    row.tag,
                    row.port,
                    row.project,
                    row.task,
                    row.created_at,
                    row.last_used_at,
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
    /// Every key is checked by the verified storage kernel before any SQL
    /// runs, exactly as [`Database::delete_reservation`] checks one.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any key cannot be stored (a validation error; nothing is deleted)
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
        let timeout = self.busy_timeout();
        self.batch_delete_reservations_inner(keys)
            .map_err(|error| error.classify_sqlite_lock(timeout, "deleting a reservation batch"))
    }

    fn batch_delete_reservations_inner(&mut self, keys: &[ReservationKey]) -> Result<usize> {
        let stored = keys.iter().map(storable_key).collect::<Result<Vec<_>>>()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let mut total_deleted = 0;
        {
            let mut stmt = tx.prepare(DELETE_RESERVATION)?;
            for key in &stored {
                total_deleted += stmt.execute(params![key.path, key.tag])?;
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
        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_batch_create_empty() {
        let mut db = create_test_database();

        let reservations: Vec<Reservation> = vec![];
        db.batch_create_reservations(&reservations).unwrap();

        let all = Database::list_all_reservations(db.connection()).unwrap();
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
        let all = Database::list_all_reservations(db.connection()).unwrap();
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

        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_batch_create_port_conflict_rolls_back_without_replacing() {
        let mut db = create_test_database();

        let existing = create_test_reservation("/existing", 5000);
        db.create_reservation(&existing).unwrap();

        let conflicting = create_test_reservation("/conflicting", 5000);
        let new_reservation = create_test_reservation("/new", 5001);

        assert!(db
            .batch_create_reservations(&[new_reservation, conflicting])
            .is_err());

        let all = Database::list_all_reservations(db.connection()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].key().path, PathBuf::from("/existing"));
        assert_eq!(all[0].port().value(), 5000);
    }
}
