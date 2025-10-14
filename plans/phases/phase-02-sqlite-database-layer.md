# Phase 2: SQLite Database Layer - Detailed Implementation Plan

## Overview

This document provides a comprehensive, actionable implementation plan for Phase 2 of the `trop` port reservation tool. This phase implements the SQLite database layer including connection management, schema creation and versioning, CRUD operations for reservations, and proper transaction handling.

## Context from Phase 1

Phase 1 established:
- Workspace structure with `trop` library and `trop-cli` binary
- Core types: `Port`, `PortRange`, `Reservation`, `ReservationKey`
- Error hierarchy with `thiserror`
- Logging infrastructure
- 99 tests passing with clean clippy output

## Success Criteria

Upon completion of Phase 2:
- Database auto-initialization works correctly
- Schema versioning and migration framework in place
- CRUD operations for reservations are atomic and safe
- Concurrent access handled properly via WAL mode
- All database operations use proper transaction semantics
- Integration tests demonstrate concurrent safety
- No regressions in existing functionality

## Task Breakdown

### Task 1: Add Database Dependencies

**Objective**: Add SQLite and related dependencies to the workspace.

**Files to Modify**:
- `/Users/prb/github/trop/Cargo.toml` (workspace)
- `/Users/prb/github/trop/trop/Cargo.toml` (library)

**Implementation Details**:

1. Update workspace `Cargo.toml`:
   ```toml
   [workspace.dependencies]
   rusqlite = { version = "0.32", features = ["bundled", "chrono"] }
   tempfile = "3.8"  # For testing with temporary databases
   ```

2. Update library `Cargo.toml`:
   ```toml
   [dependencies]
   rusqlite = { workspace = true }

   [dev-dependencies]
   tempfile = { workspace = true }
   ```

**Verification**:
- `cargo build` succeeds
- `cargo tree` shows rusqlite with bundled feature

### Task 2: Define Database Schema Module

**Objective**: Create schema definitions and SQL constants.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/schema.rs`

**Implementation Details**:

1. Define schema version constant:
   ```rust
   pub const CURRENT_SCHEMA_VERSION: i32 = 1;
   ```

2. Define table creation SQL:
   ```rust
   pub const CREATE_METADATA_TABLE: &str = r#"
       CREATE TABLE IF NOT EXISTS metadata (
           key TEXT PRIMARY KEY NOT NULL,
           value TEXT NOT NULL
       )"#;

   pub const CREATE_RESERVATIONS_TABLE: &str = r#"
       CREATE TABLE IF NOT EXISTS reservations (
           path TEXT NOT NULL,
           tag TEXT,
           port INTEGER NOT NULL,
           project TEXT,
           task TEXT,
           created_at TEXT NOT NULL,
           last_used_at TEXT NOT NULL,
           PRIMARY KEY (path, tag)
       )"#;
   ```

3. Define index creation SQL:
   ```rust
   pub const CREATE_PORT_INDEX: &str =
       "CREATE INDEX IF NOT EXISTS idx_reservations_port ON reservations(port)";
   pub const CREATE_PROJECT_INDEX: &str =
       "CREATE INDEX IF NOT EXISTS idx_reservations_project ON reservations(project)";
   pub const CREATE_LAST_USED_INDEX: &str =
       "CREATE INDEX IF NOT EXISTS idx_reservations_last_used ON reservations(last_used_at)";
   ```

4. Add schema query constants:
   ```rust
   pub const SELECT_SCHEMA_VERSION: &str =
       "SELECT value FROM metadata WHERE key = 'schema_version'";
   pub const INSERT_SCHEMA_VERSION: &str =
       "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?)";
   ```

**Verification**:
- Module compiles
- SQL syntax is valid (will be tested in integration tests)

### Task 3: Create Database Configuration

**Objective**: Define database configuration and connection parameters.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/config.rs`

**Implementation Details**:

1. Define configuration struct:
   ```rust
   use std::path::{Path, PathBuf};
   use std::time::Duration;

   #[derive(Debug, Clone)]
   pub struct DatabaseConfig {
       pub path: PathBuf,
       pub busy_timeout: Duration,
       pub auto_create: bool,
       pub read_only: bool,
   }
   ```

2. Implement default and builder pattern:
   ```rust
   impl DatabaseConfig {
       pub fn new(path: impl AsRef<Path>) -> Self {
           Self {
               path: path.as_ref().to_path_buf(),
               busy_timeout: Duration::from_millis(5000),
               auto_create: true,
               read_only: false,
           }
       }

       pub fn with_busy_timeout(mut self, timeout: Duration) -> Self {
           self.busy_timeout = timeout;
           self
       }

       pub fn read_only(mut self) -> Self {
           self.read_only = true;
           self.auto_create = false;
           self
       }
   }
   ```

3. Add data directory resolution:
   ```rust
   pub fn default_data_dir() -> Result<PathBuf> {
       let home = std::env::var("HOME")
           .or_else(|_| std::env::var("USERPROFILE"))
           .map_err(|_| Error::Configuration("Cannot determine home directory".into()))?;
       Ok(PathBuf::from(home).join(".trop"))
   }

   pub fn resolve_database_path() -> Result<PathBuf> {
       if let Ok(data_dir) = std::env::var("TROP_DATA_DIR") {
           Ok(PathBuf::from(data_dir).join("trop.db"))
       } else {
           Ok(default_data_dir()?.join("trop.db"))
       }
   }
   ```

**Verification**:
- Configuration builder methods work correctly
- Path resolution respects environment variables

### Task 4: Implement Connection Management

**Objective**: Create database connection wrapper with proper PRAGMA settings.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/connection.rs`

**Implementation Details**:

1. Define connection wrapper:
   ```rust
   use rusqlite::{Connection, OpenFlags};
   use crate::database::config::DatabaseConfig;

   pub struct Database {
       conn: Connection,
       config: DatabaseConfig,
   }
   ```

2. Implement connection opening:
   ```rust
   impl Database {
       pub fn open(config: DatabaseConfig) -> Result<Self> {
           // Ensure parent directory exists if auto-creating
           if config.auto_create && !config.path.exists() {
               if let Some(parent) = config.path.parent() {
                   std::fs::create_dir_all(parent)?;
               }
           }

           let flags = if config.read_only {
               OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
           } else if config.auto_create {
               OpenFlags::SQLITE_OPEN_READ_WRITE |
               OpenFlags::SQLITE_OPEN_CREATE |
               OpenFlags::SQLITE_OPEN_NO_MUTEX
           } else {
               OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
           };

           let conn = Connection::open_with_flags(&config.path, flags)?;

           // Set pragmas
           conn.execute("PRAGMA journal_mode = WAL", [])?;
           conn.execute("PRAGMA synchronous = NORMAL", [])?;
           conn.execute(
               &format!("PRAGMA busy_timeout = {}", config.busy_timeout.as_millis()),
               []
           )?;

           Ok(Self { conn, config })
       }
   }
   ```

3. Add connection accessor for raw operations:
   ```rust
   impl Database {
       pub fn connection(&self) -> &Connection {
           &self.conn
       }

       pub fn connection_mut(&mut self) -> &mut Connection {
           &mut self.conn
       }
   }
   ```

**Verification**:
- Connection opens successfully
- PRAGMAs are set correctly
- Parent directory creation works

### Task 5: Implement Schema Management

**Objective**: Create schema initialization and version checking.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/migrations.rs`

**Implementation Details**:

1. Define migration trait:
   ```rust
   pub trait Migration {
       fn version(&self) -> i32;
       fn up(&self, conn: &Connection) -> Result<()>;
       fn down(&self, conn: &Connection) -> Result<()>;
   }
   ```

2. Implement schema initialization:
   ```rust
   use crate::database::schema::*;

   pub fn initialize_schema(conn: &Connection) -> Result<()> {
       conn.execute(CREATE_METADATA_TABLE, [])?;
       conn.execute(CREATE_RESERVATIONS_TABLE, [])?;
       conn.execute(CREATE_PORT_INDEX, [])?;
       conn.execute(CREATE_PROJECT_INDEX, [])?;
       conn.execute(CREATE_LAST_USED_INDEX, [])?;

       // Set initial schema version
       conn.execute(INSERT_SCHEMA_VERSION, [CURRENT_SCHEMA_VERSION])?;

       Ok(())
   }
   ```

3. Implement version checking:
   ```rust
   pub fn get_schema_version(conn: &Connection) -> Result<i32> {
       match conn.query_row(SELECT_SCHEMA_VERSION, [], |row| row.get(0)) {
           Ok(version) => Ok(version),
           Err(rusqlite::Error::QueryReturnedNoRows) => {
               // Database exists but no schema - needs initialization
               Ok(0)
           }
           Err(e) => Err(e.into()),
       }
   }

   pub fn check_schema_compatibility(conn: &Connection) -> Result<()> {
       let version = get_schema_version(conn)?;

       if version == 0 {
           // Fresh database, initialize it
           initialize_schema(conn)?;
       } else if version < CURRENT_SCHEMA_VERSION {
           // Future: apply migrations
           return Err(Error::Database(
               format!("Database schema version {} is older than client version {}",
                       version, CURRENT_SCHEMA_VERSION).into()
           ));
       } else if version > CURRENT_SCHEMA_VERSION {
           return Err(Error::Database(
               format!("Database schema version {} is newer than client version {}",
                       version, CURRENT_SCHEMA_VERSION).into()
           ));
       }

       Ok(())
   }
   ```

4. Add to Database::open:
   ```rust
   impl Database {
       pub fn open(config: DatabaseConfig) -> Result<Self> {
           // ... existing connection opening code ...

           // Check and initialize schema
           check_schema_compatibility(&conn)?;

           Ok(Self { conn, config })
       }
   }
   ```

**Verification**:
- Fresh database gets initialized with schema
- Schema version is correctly stored and retrieved
- Incompatible versions are rejected

### Task 6: Implement Reservation CRUD Operations

**Objective**: Create the core database operations for reservations.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/operations.rs`

**Implementation Details**:

1. Define SQL statements:
   ```rust
   const INSERT_RESERVATION: &str = r#"
       INSERT OR REPLACE INTO reservations
       (path, tag, port, project, task, created_at, last_used_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)
   "#;

   const SELECT_RESERVATION: &str = r#"
       SELECT port, project, task, created_at, last_used_at
       FROM reservations
       WHERE path = ? AND tag IS ?
   "#;

   const UPDATE_LAST_USED: &str = r#"
       UPDATE reservations
       SET last_used_at = ?
       WHERE path = ? AND tag IS ?
   "#;

   const DELETE_RESERVATION: &str = r#"
       DELETE FROM reservations
       WHERE path = ? AND tag IS ?
   "#;

   const LIST_RESERVATIONS: &str = r#"
       SELECT path, tag, port, project, task, created_at, last_used_at
       FROM reservations
       ORDER BY path, tag
   "#;
   ```

2. Implement create operation:
   ```rust
   impl Database {
       pub fn create_reservation(&mut self, reservation: &Reservation) -> Result<()> {
           let tx = self.conn.transaction_with_behavior(
               rusqlite::TransactionBehavior::Immediate
           )?;

           tx.execute(INSERT_RESERVATION, params![
               reservation.key.path.to_str(),
               reservation.key.tag.as_deref(),
               reservation.port.value(),
               reservation.project.as_deref(),
               reservation.task.as_deref(),
               reservation.created_at.duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
               reservation.last_used_at.duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
           ])?;

           tx.commit()?;
           Ok(())
       }
   }
   ```

3. Implement read operation:
   ```rust
   pub fn get_reservation(&self, key: &ReservationKey) -> Result<Option<Reservation>> {
       let mut stmt = self.conn.prepare(SELECT_RESERVATION)?;

       match stmt.query_row(params![
           key.path.to_str(),
           key.tag.as_deref()
       ], |row| {
           Ok(Reservation {
               key: key.clone(),
               port: Port::try_from(row.get::<_, u16>(0)?).unwrap(),
               project: row.get(1)?,
               task: row.get(2)?,
               created_at: SystemTime::UNIX_EPOCH +
                   Duration::from_secs(row.get::<_, u64>(3)?),
               last_used_at: SystemTime::UNIX_EPOCH +
                   Duration::from_secs(row.get::<_, u64>(4)?),
           })
       }) {
           Ok(reservation) => Ok(Some(reservation)),
           Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
           Err(e) => Err(e.into()),
       }
   }
   ```

4. Implement update operation:
   ```rust
   pub fn update_last_used(&mut self, key: &ReservationKey) -> Result<bool> {
       let tx = self.conn.transaction_with_behavior(
           rusqlite::TransactionBehavior::Immediate
       )?;

       let now = SystemTime::now()
           .duration_since(SystemTime::UNIX_EPOCH)?
           .as_secs();

       let rows_affected = tx.execute(UPDATE_LAST_USED, params![
           now,
           key.path.to_str(),
           key.tag.as_deref()
       ])?;

       tx.commit()?;
       Ok(rows_affected > 0)
   }
   ```

5. Implement delete operation:
   ```rust
   pub fn delete_reservation(&mut self, key: &ReservationKey) -> Result<bool> {
       let tx = self.conn.transaction_with_behavior(
           rusqlite::TransactionBehavior::Immediate
       )?;

       let rows_affected = tx.execute(DELETE_RESERVATION, params![
           key.path.to_str(),
           key.tag.as_deref()
       ])?;

       tx.commit()?;
       Ok(rows_affected > 0)
   }
   ```

6. Implement list operation:
   ```rust
   pub fn list_all_reservations(&self) -> Result<Vec<Reservation>> {
       let mut stmt = self.conn.prepare(LIST_RESERVATIONS)?;

       let reservations = stmt.query_map([], |row| {
           // Parse fields and construct Reservation
           // Similar to get_reservation but with all fields from row
       })?
       .collect::<Result<Vec<_>, _>>()?;

       Ok(reservations)
   }
   ```

**Verification**:
- Each operation correctly serializes/deserializes data
- Transactions use IMMEDIATE mode for writes
- NULL handling works correctly for optional fields

### Task 7: Add Query Operations

**Objective**: Implement specialized queries for port allocation and cleanup.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/database/operations.rs`

**Implementation Details**:

1. Query reserved ports in range:
   ```rust
   const SELECT_RESERVED_PORTS: &str = r#"
       SELECT DISTINCT port
       FROM reservations
       WHERE port >= ? AND port <= ?
       ORDER BY port
   "#;

   pub fn get_reserved_ports(&self, range: &PortRange) -> Result<Vec<Port>> {
       let mut stmt = self.conn.prepare(SELECT_RESERVED_PORTS)?;

       let ports = stmt.query_map(params![
           range.min().value(),
           range.max().value()
       ], |row| {
           Ok(Port::try_from(row.get::<_, u16>(0)?).unwrap())
       })?
       .collect::<Result<Vec<_>, _>>()?;

       Ok(ports)
   }
   ```

2. Query by path prefix:
   ```rust
   const SELECT_BY_PATH_PREFIX: &str = r#"
       SELECT path, tag, port, project, task, created_at, last_used_at
       FROM reservations
       WHERE path LIKE ? || '%'
       ORDER BY path, tag
   "#;

   pub fn get_reservations_by_path_prefix(&self, prefix: &Path) -> Result<Vec<Reservation>> {
       let mut stmt = self.conn.prepare(SELECT_BY_PATH_PREFIX)?;
       // Implementation similar to list_all_reservations but with prefix filter
   }
   ```

3. Find expired reservations:
   ```rust
   const SELECT_EXPIRED: &str = r#"
       SELECT path, tag, port, project, task, created_at, last_used_at
       FROM reservations
       WHERE last_used_at < ?
       ORDER BY last_used_at
   "#;

   pub fn find_expired_reservations(&self, max_age: Duration) -> Result<Vec<Reservation>> {
       let cutoff = SystemTime::now()
           .duration_since(SystemTime::UNIX_EPOCH)?
           .saturating_sub(max_age)
           .as_secs();

       let mut stmt = self.conn.prepare(SELECT_EXPIRED)?;
       // Query and construct reservations
   }
   ```

4. Check port availability:
   ```rust
   const CHECK_PORT_RESERVED: &str = r#"
       SELECT COUNT(*) FROM reservations WHERE port = ?
   "#;

   pub fn is_port_reserved(&self, port: Port) -> Result<bool> {
       let count: i32 = self.conn.query_row(
           CHECK_PORT_RESERVED,
           params![port.value()],
           |row| row.get(0)
       )?;
       Ok(count > 0)
   }
   ```

**Verification**:
- Queries return expected results
- Path prefix matching works correctly
- Expiry calculation is accurate

### Task 8: Implement Transaction Helpers

**Objective**: Create transaction management utilities for complex operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/transaction.rs`

**Implementation Details**:

1. Define transaction wrapper:
   ```rust
   use rusqlite::{Connection, Transaction};

   pub struct TransactionGuard<'a> {
       tx: Transaction<'a>,
       committed: bool,
   }

   impl<'a> TransactionGuard<'a> {
       pub fn new(conn: &'a mut Connection) -> Result<Self> {
           let tx = conn.transaction_with_behavior(
               rusqlite::TransactionBehavior::Immediate
           )?;
           Ok(Self { tx, committed: false })
       }

       pub fn commit(mut self) -> Result<()> {
           self.tx.commit()?;
           self.committed = true;
           Ok(())
       }
   }

   impl<'a> Drop for TransactionGuard<'a> {
       fn drop(&mut self) {
           if !self.committed {
               // Automatic rollback on drop
               let _ = self.tx.rollback();
           }
       }
   }
   ```

2. Add batch operations:
   ```rust
   impl Database {
       pub fn batch_create_reservations(&mut self, reservations: &[Reservation]) -> Result<()> {
           let tx = self.conn.transaction_with_behavior(
               rusqlite::TransactionBehavior::Immediate
           )?;

           {
               let mut stmt = tx.prepare(INSERT_RESERVATION)?;
               for reservation in reservations {
                   stmt.execute(/* params */)?;
               }
           }

           tx.commit()?;
           Ok(())
       }

       pub fn batch_delete_reservations(&mut self, keys: &[ReservationKey]) -> Result<usize> {
           let tx = self.conn.transaction_with_behavior(
               rusqlite::TransactionBehavior::Immediate
           )?;

           let mut total_deleted = 0;
           {
               let mut stmt = tx.prepare(DELETE_RESERVATION)?;
               for key in keys {
                   total_deleted += stmt.execute(/* params */)?;
               }
           }

           tx.commit()?;
           Ok(total_deleted)
       }
   }
   ```

**Verification**:
- Transactions auto-rollback on error
- Batch operations are atomic
- IMMEDIATE mode prevents deadlocks

### Task 9: Create Database Module Structure

**Objective**: Organize database code into a cohesive module.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/mod.rs`
- `/Users/prb/github/trop/trop/src/database.rs` (re-export module)

**Implementation Details**:

1. Create module structure in `database/mod.rs`:
   ```rust
   mod config;
   mod connection;
   mod migrations;
   mod operations;
   mod schema;
   mod transaction;

   pub use config::{DatabaseConfig, default_data_dir, resolve_database_path};
   pub use connection::Database;
   pub use migrations::{check_schema_compatibility, initialize_schema};

   // Re-export commonly used items
   pub use operations::*;  // All CRUD operations are on Database impl
   ```

2. Update library root `lib.rs`:
   ```rust
   pub mod database;

   // Optionally re-export at crate root
   pub use database::{Database, DatabaseConfig};
   ```

3. Update error types to include database errors:
   ```rust
   // In error.rs
   #[derive(Error, Debug)]
   pub enum Error {
       // ... existing variants ...

       #[error("Database error: {0}")]
       Database(String),

       #[error("SQLite error: {0}")]
       Sqlite(#[from] rusqlite::Error),
   }
   ```

**Verification**:
- Module structure is logical and navigable
- Public API is clean and well-documented
- Error conversions work properly

### Task 10: Write Unit Tests

**Objective**: Create comprehensive unit tests for database operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/database/tests.rs`

**Implementation Details**:

1. Create test helpers:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       use tempfile::tempdir;

       fn create_test_database() -> Result<Database> {
           let dir = tempdir()?;
           let path = dir.path().join("test.db");
           let config = DatabaseConfig::new(path);
           Database::open(config)
       }

       fn create_test_reservation(path: &str, port: u16) -> Reservation {
           Reservation::builder()
               .key(ReservationKey::new(path.into(), None)?)
               .port(Port::try_from(port)?)
               .build()
               .unwrap()
       }
   }
   ```

2. Test schema initialization:
   ```rust
   #[test]
   fn test_database_initialization() {
       let db = create_test_database().unwrap();
       let version = get_schema_version(db.connection()).unwrap();
       assert_eq!(version, CURRENT_SCHEMA_VERSION);
   }
   ```

3. Test CRUD operations:
   ```rust
   #[test]
   fn test_reservation_crud() {
       let mut db = create_test_database().unwrap();
       let reservation = create_test_reservation("/test/path", 5000);

       // Create
       db.create_reservation(&reservation).unwrap();

       // Read
       let loaded = db.get_reservation(&reservation.key).unwrap();
       assert!(loaded.is_some());
       assert_eq!(loaded.unwrap().port, reservation.port);

       // Update
       db.update_last_used(&reservation.key).unwrap();

       // Delete
       let deleted = db.delete_reservation(&reservation.key).unwrap();
       assert!(deleted);

       // Verify deleted
       let loaded = db.get_reservation(&reservation.key).unwrap();
       assert!(loaded.is_none());
   }
   ```

4. Test concurrent access:
   ```rust
   #[test]
   fn test_concurrent_operations() {
       use std::sync::Arc;
       use std::thread;

       let dir = tempdir().unwrap();
       let db_path = dir.path().join("concurrent.db");

       // Initialize database
       Database::open(DatabaseConfig::new(&db_path)).unwrap();

       let handles: Vec<_> = (0..10).map(|i| {
           let path = db_path.clone();
           thread::spawn(move || {
               let mut db = Database::open(DatabaseConfig::new(path)).unwrap();
               let reservation = create_test_reservation(
                   &format!("/test/path/{}", i),
                   5000 + i
               );
               db.create_reservation(&reservation)
           })
       }).collect();

       for handle in handles {
           handle.join().unwrap().unwrap();
       }

       // Verify all reservations were created
       let db = Database::open(DatabaseConfig::new(&db_path)).unwrap();
       let all = db.list_all_reservations().unwrap();
       assert_eq!(all.len(), 10);
   }
   ```

**Verification**:
- All tests pass reliably
- Tests cover edge cases
- Concurrent tests don't have race conditions

### Task 11: Create Integration Tests

**Objective**: Write integration tests that exercise the full database stack.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/database_integration.rs`

**Implementation Details**:

1. Test database lifecycle:
   ```rust
   use trop::database::{Database, DatabaseConfig};
   use tempfile::tempdir;

   #[test]
   fn test_database_auto_creation() {
       let dir = tempdir().unwrap();
       let db_path = dir.path().join("subdir").join("test.db");

       // Directory doesn't exist yet
       assert!(!db_path.parent().unwrap().exists());

       // Open with auto-create
       let config = DatabaseConfig::new(&db_path);
       let db = Database::open(config).unwrap();

       // Directory and file should now exist
       assert!(db_path.exists());
       assert!(db_path.parent().unwrap().exists());
   }
   ```

2. Test schema version checking:
   ```rust
   #[test]
   fn test_schema_version_compatibility() {
       let dir = tempdir().unwrap();
       let db_path = dir.path().join("version_test.db");

       // Create database with current schema
       {
           let db = Database::open(DatabaseConfig::new(&db_path)).unwrap();
           // Database created and closed
       }

       // Reopen should work (same version)
       {
           let db = Database::open(DatabaseConfig::new(&db_path)).unwrap();
           // Should succeed
       }

       // Manually set incompatible version
       {
           use rusqlite::Connection;
           let conn = Connection::open(&db_path).unwrap();
           conn.execute(
               "UPDATE metadata SET value = '999' WHERE key = 'schema_version'",
               []
           ).unwrap();
       }

       // Now opening should fail
       let result = Database::open(DatabaseConfig::new(&db_path));
       assert!(result.is_err());
       assert!(result.unwrap_err().to_string().contains("newer than client"));
   }
   ```

3. Test transaction atomicity:
   ```rust
   #[test]
   fn test_transaction_atomicity() {
       let mut db = create_test_database().unwrap();

       // Create some initial reservations
       let r1 = create_test_reservation("/path1", 5000);
       let r2 = create_test_reservation("/path2", 5001);
       db.create_reservation(&r1).unwrap();

       // Try batch operation that will fail partway through
       let invalid_reservations = vec![
           r2.clone(),  // This will succeed
           Reservation::builder()
               .key(ReservationKey::new("/path3".into(), None).unwrap())
               .port(Port::try_from(0).unwrap())  // This will fail (port 0 invalid)
               .build()
               .unwrap()
       ];

       let result = db.batch_create_reservations(&invalid_reservations);
       assert!(result.is_err());

       // Verify r2 was NOT created (transaction rolled back)
       let loaded = db.get_reservation(&r2.key).unwrap();
       assert!(loaded.is_none());
   }
   ```

**Verification**:
- Integration tests demonstrate real-world usage
- Error conditions are properly tested
- Database state is predictable after operations

### Task 12: Add Performance Benchmarks

**Objective**: Create benchmarks for critical database operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/benches/database_bench.rs`

**Implementation Details**:

1. Add benchmark dependencies to `Cargo.toml`:
   ```toml
   [dev-dependencies]
   criterion = "0.5"

   [[bench]]
   name = "database_bench"
   harness = false
   ```

2. Create benchmark suite:
   ```rust
   use criterion::{black_box, criterion_group, criterion_main, Criterion};
   use trop::database::{Database, DatabaseConfig};
   use tempfile::tempdir;

   fn bench_create_reservation(c: &mut Criterion) {
       let dir = tempdir().unwrap();
       let mut db = Database::open(
           DatabaseConfig::new(dir.path().join("bench.db"))
       ).unwrap();

       c.bench_function("create_reservation", |b| {
           let mut counter = 0u32;
           b.iter(|| {
               let reservation = create_test_reservation(
                   &format!("/bench/path/{}", counter),
                   5000 + (counter % 1000) as u16
               );
               db.create_reservation(black_box(&reservation)).unwrap();
               counter += 1;
           });
       });
   }

   fn bench_query_reserved_ports(c: &mut Criterion) {
       // Setup database with many reservations
       // Benchmark querying port ranges
   }

   criterion_group!(benches, bench_create_reservation, bench_query_reserved_ports);
   criterion_main!(benches);
   ```

**Verification**:
- Benchmarks run with `cargo bench`
- Performance baselines are established
- No obvious performance bottlenecks

## Dependencies Between Tasks

```
Task 1 (Dependencies)
    └── Task 2 (Schema Module)
        └── Task 3 (Config)
            └── Task 4 (Connection)
                └── Task 5 (Schema Management)
                    └── Task 6 (CRUD Operations)
                        └── Task 7 (Query Operations)
                            └── Task 8 (Transaction Helpers)
                                └── Task 9 (Module Structure)
                                    ├── Task 10 (Unit Tests)
                                    ├── Task 11 (Integration Tests)
                                    └── Task 12 (Benchmarks)
```

Tasks should be completed sequentially as each builds upon the previous. Testing tasks (10-12) can be developed in parallel once Task 9 is complete.

## Testing Strategy

### Unit Tests
- Test each database operation in isolation
- Use temporary databases for each test
- Mock file system errors where appropriate
- Test transaction rollback scenarios

### Integration Tests
- Test full database lifecycle
- Verify concurrent access patterns
- Test schema migration framework
- Validate error handling across module boundaries

### Performance Tests
- Establish baselines for common operations
- Test with realistic data volumes
- Measure impact of indices
- Profile transaction overhead

## Validation Checklist

Before considering Phase 2 complete:

- [ ] Database auto-initializes correctly
- [ ] Schema versioning framework functions
- [ ] All CRUD operations are transactional
- [ ] Concurrent access is safe (WAL mode works)
- [ ] Busy timeout is configurable and works
- [ ] Path operations handle all edge cases
- [ ] Integration tests pass consistently
- [ ] No performance regressions from Phase 1
- [ ] Documentation is complete and accurate
- [ ] Error messages are helpful and specific

## Risk Mitigations

### SQLite Version Compatibility
- Use bundled SQLite feature to ensure consistent version
- Test with minimum supported SQLite version
- Document any version-specific features used

### Concurrent Access Patterns
- WAL mode enables reader/writer concurrency
- IMMEDIATE transactions prevent writer deadlocks
- Busy timeout prevents spurious failures
- Integration tests verify concurrent safety

### Data Migration Strategy
- Schema versioning in place from day one
- Migration framework ready for future changes
- Forward compatibility checks prevent data corruption
- Clear error messages for version mismatches

### Platform Differences
- Use rusqlite's bundled feature for consistency
- Test on all target platforms (Linux, macOS, Windows)
- Handle path separators correctly
- Account for filesystem case sensitivity differences

## Implementation Decisions

### Transaction Strategy
- All writes use IMMEDIATE mode to prevent deadlocks
- Reads can use DEFERRED mode (default)
- Batch operations wrapped in single transaction
- Automatic rollback on error via RAII

### NULL Handling
- Use NULL for missing optional fields (tag, project, task)
- rusqlite's `params!` macro handles Option<T> correctly
- Consistent NULL checking in queries

### Timestamp Storage
- Store as Unix epoch seconds (INTEGER)
- Convert to/from SystemTime at boundaries
- Handles time zones consistently
- Simplifies queries and comparisons

### Index Strategy
- Index on port for allocation queries
- Index on project for filtered lists
- Index on last_used_at for cleanup operations
- Primary key index on (path, tag) for lookups

## Next Phase Preparation

Phase 3 will implement path handling. Ensure:
- Database can store any valid path string
- Path comparison logic is prepared
- Hierarchical queries are possible
- Path normalization doesn't affect database

## Notes for Implementer

### Code Organization
- Keep SQL separate from logic where possible
- Use prepared statements for all queries
- Validate inputs at API boundaries
- Use Result<T> consistently

### Error Handling
- Convert rusqlite errors to domain errors
- Provide context in error messages
- Log database operations at debug level
- Handle busy timeout gracefully

### Testing Approach
- Each test uses isolated database
- Tests are deterministic and reproducible
- Concurrent tests use different paths
- Cleanup temporary files properly

### Documentation
- Document transaction semantics
- Explain schema decisions
- Include examples for common operations
- Note any platform-specific behavior

This plan provides comprehensive guidance for implementing the SQLite database layer while maintaining compatibility with the existing Phase 1 code and preparing for future phases.