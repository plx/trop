# Future Directions: Database Migration Testing

## Overview

This document outlines a potential future enhancement to trop: a comprehensive database migration and schema versioning system. This work is **not part of the v1.0 scope** but is documented here for consideration in future versions (v2.0+).

## Context

As of v1.0, trop uses a SQLite database with a fixed schema. While this works well for the initial release, future versions may need to evolve the database schema to support new features or optimize existing operations. A migration system would enable:

- Schema evolution across versions
- Backward compatibility for users upgrading from older versions
- Safe rollback in case of migration failures
- Testing of migration paths before release

## Proposed Implementation

### Schema Versioning Framework

**File:** `trop/src/database/migration.rs` (future work)

**Basic Infrastructure:**
```rust
//! Database migration framework
//!
//! This module provides infrastructure for managing database schema versions
//! and applying migrations between versions.

use crate::database::Database;
use crate::error::TropError;

/// Database schema version
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    pub const V1: Self = Self(1);
    pub const CURRENT: Self = Self::V1;
}

/// A migration from one schema version to another
pub trait Migration {
    fn from_version(&self) -> SchemaVersion;
    fn to_version(&self) -> SchemaVersion;
    fn up(&self, db: &mut Database) -> Result<(), TropError>;
    fn down(&self, db: &mut Database) -> Result<(), TropError>;
}

/// Check the current schema version
pub fn get_schema_version(db: &Database) -> Result<SchemaVersion, TropError> {
    // Query schema_version table
    // If table doesn't exist, assume v1 (initial schema)
    Ok(SchemaVersion::V1)
}

/// Apply all migrations to bring database to target version
pub fn migrate_to(
    db: &mut Database,
    target: SchemaVersion,
) -> Result<(), TropError> {
    let current = get_schema_version(db)?;

    if current == target {
        return Ok(());
    }

    // Find migration path
    // Apply migrations in order
    // Update schema_version table
    // Create backup before migration

    todo!("Implement migration execution")
}

/// Verify schema is compatible with current binary
pub fn verify_schema(db: &Database) -> Result<(), TropError> {
    let version = get_schema_version(db)?;

    if version > SchemaVersion::CURRENT {
        return Err(TropError::SchemaVersionTooNew {
            current: version.0,
            supported: SchemaVersion::CURRENT.0,
            hint: "This database was created by a newer version of trop. Please upgrade.".into(),
        });
    }

    if version < SchemaVersion::CURRENT {
        return Err(TropError::SchemaVersionTooOld {
            current: version.0,
            latest: SchemaVersion::CURRENT.0,
            hint: "Run 'trop migrate up' to upgrade the database schema.".into(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseConfig;
    use tempfile::TempDir;

    #[test]
    fn test_schema_version_check() {
        let temp_dir = TempDir::new().unwrap();
        let config = DatabaseConfig::new(temp_dir.path().join("test.db"));
        let db = Database::open(&config).unwrap();

        let version = get_schema_version(&db).unwrap();
        assert_eq!(version, SchemaVersion::CURRENT);
    }

    #[test]
    fn test_verify_schema_succeeds_on_current() {
        let temp_dir = TempDir::new().unwrap();
        let config = DatabaseConfig::new(temp_dir.path().join("test.db"));
        let db = Database::open(&config).unwrap();

        verify_schema(&db).unwrap();
    }
}
```

### CLI Migration Commands

**File:** `trop-cli/src/commands/migrate_schema.rs` (future work)

**Proposed Commands:**
```rust
/// Database schema migration commands
#[derive(Subcommand)]
pub enum MigrateSchemaCommands {
    /// Show current schema version and available migrations
    Status,

    /// Upgrade database to latest schema version
    Up {
        /// Target version (default: latest)
        #[arg(long)]
        to: Option<u32>,

        /// Create backup before migration
        #[arg(long, default_value = "true")]
        backup: bool,
    },

    /// Downgrade database to previous schema version
    Down {
        /// Target version
        #[arg(long)]
        to: Option<u32>,

        /// Create backup before migration
        #[arg(long, default_value = "true")]
        backup: bool,
    },

    /// Test migration without applying changes (dry run)
    Test {
        /// Target version
        #[arg(long)]
        to: Option<u32>,
    },
}

// Example usage:
// $ trop migrate status
// Current schema version: 1
// Latest schema version: 2
// Available migrations: 1 -> 2
//
// $ trop migrate up
// Creating backup: ~/.local/share/trop/backups/trop-v1-2024-01-15.db
// Applying migration 1 -> 2...
// Migration complete. Database is now at version 2.
//
// $ trop migrate down --to 1
// Warning: This will downgrade your database from v2 to v1.
// Some data may be lost. Continue? [y/N]
```

### Migration Testing Scenarios

**File:** `trop/tests/migration_scenarios.rs` (future work)

**Test Coverage:**
```rust
//! Migration scenario tests
//!
//! These tests verify that database migrations work correctly across versions.

use trop::database::{Database, DatabaseConfig};
use trop::database::migration::{SchemaVersion, migrate_to};
use tempfile::TempDir;
use std::path::PathBuf;

/// Test forward migration from v1 to v2
#[test]
fn test_migrate_v1_to_v2() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create v1 database with data
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let port = trop::port::Port::new(5000).unwrap();
        db.insert_reservation(&path, port, Some("project".into()), None, vec![]).unwrap();
    }

    // Apply migration to v2
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        migrate_to(&mut db, SchemaVersion::V2).unwrap();
    }

    // Verify data integrity after migration
    {
        let config = DatabaseConfig::new(db_path);
        let db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let reservation = db.get_reservation_by_path(&path).unwrap().unwrap();
        assert_eq!(reservation.port.value(), 5000);
    }
}

/// Test rollback capability
#[test]
fn test_migrate_rollback_on_failure() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create database with data
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let port = trop::port::Port::new(5000).unwrap();
        db.insert_reservation(&path, port, Some("project".into()), None, vec![]).unwrap();
    }

    // Attempt migration that will fail (simulate)
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        // Inject failure condition
        let result = migrate_to(&mut db, SchemaVersion::V2);
        assert!(result.is_err());
    }

    // Verify database is unchanged (rollback worked)
    {
        let config = DatabaseConfig::new(db_path);
        let db = Database::open(&config).unwrap();

        let version = get_schema_version(&db).unwrap();
        assert_eq!(version, SchemaVersion::V1);

        let path = PathBuf::from("/tmp/test");
        let reservation = db.get_reservation_by_path(&path).unwrap().unwrap();
        assert_eq!(reservation.port.value(), 5000);
    }
}

/// Test that database can be opened and used after recreation
#[test]
fn test_database_recreation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // Create and use database
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let port = trop::port::Port::new(5000).unwrap();
        db.insert_reservation(&path, port, Some("project".into()), None, vec![]).unwrap();
    }

    // Reopen and verify
    {
        let config = DatabaseConfig::new(db_path.clone());
        let db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let reservation = db.get_reservation_by_path(&path).unwrap().unwrap();
        assert_eq!(reservation.port.value(), 5000);
    }
}

/// Test data preservation across database close/reopen
#[test]
fn test_data_preservation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let config = DatabaseConfig::new(db_path);

    // Insert data
    {
        let mut db = Database::open(&config).unwrap();
        for i in 0..100 {
            let path = PathBuf::from(format!("/tmp/test-{}", i));
            let port = trop::port::Port::new(5000 + i).unwrap();
            db.insert_reservation(&path, port, Some(format!("project-{}", i)), None, vec![]).unwrap();
        }
    }

    // Verify all data present
    {
        let db = Database::open(&config).unwrap();
        let reservations = db.list_all_reservations().unwrap();
        assert_eq!(reservations.len(), 100);
    }
}

/// Test backup and restore capability
#[test]
fn test_backup_restore() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let backup_path = temp_dir.path().join("backup.db");

    // Create database with data
    {
        let config = DatabaseConfig::new(db_path.clone());
        let mut db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let port = trop::port::Port::new(5000).unwrap();
        db.insert_reservation(&path, port, Some("project".into()), None, vec![]).unwrap();
    }

    // Copy database file (simple backup)
    std::fs::copy(&db_path, &backup_path).unwrap();

    // Verify backup is valid
    {
        let config = DatabaseConfig::new(backup_path);
        let db = Database::open(&config).unwrap();

        let path = PathBuf::from("/tmp/test");
        let reservation = db.get_reservation_by_path(&path).unwrap().unwrap();
        assert_eq!(reservation.port.value(), 5000);
    }
}

/// Test adding new columns with defaults
#[test]
fn test_migration_add_column_with_default() {
    // Simulate adding a new column to the schema
    // Existing rows should get the default value
    todo!("Implement when migrations exist")
}

/// Test removing deprecated columns
#[test]
fn test_migration_remove_column() {
    // Simulate removing a column from the schema
    // Verify data integrity for remaining columns
    todo!("Implement when migrations exist")
}

/// Test index additions/modifications
#[test]
fn test_migration_add_index() {
    // Simulate adding a new index
    // Verify query performance improves
    todo!("Implement when migrations exist")
}
```

## Example Migration Scenarios

### Scenario 1: Adding a New Column

**v1 Schema:**
```sql
CREATE TABLE reservations (
    path TEXT PRIMARY KEY,
    port INTEGER NOT NULL,
    project TEXT,
    task TEXT
);
```

**v2 Schema (adds `created_at`):**
```sql
CREATE TABLE reservations (
    path TEXT PRIMARY KEY,
    port INTEGER NOT NULL,
    project TEXT,
    task TEXT,
    created_at INTEGER DEFAULT (strftime('%s', 'now'))
);
```

**Migration:**
```rust
struct AddCreatedAt;

impl Migration for AddCreatedAt {
    fn from_version(&self) -> SchemaVersion { SchemaVersion::V1 }
    fn to_version(&self) -> SchemaVersion { SchemaVersion::V2 }

    fn up(&self, db: &mut Database) -> Result<(), TropError> {
        db.execute(
            "ALTER TABLE reservations ADD COLUMN created_at INTEGER DEFAULT (strftime('%s', 'now'))",
            [],
        )?;
        Ok(())
    }

    fn down(&self, db: &mut Database) -> Result<(), TropError> {
        // SQLite doesn't support DROP COLUMN directly
        // Would need to recreate table without the column
        todo!("Implement down migration")
    }
}
```

### Scenario 2: Index Addition

**Migration:**
```rust
struct AddProjectTaskIndex;

impl Migration for AddProjectTaskIndex {
    fn from_version(&self) -> SchemaVersion { SchemaVersion::V2 }
    fn to_version(&self) -> SchemaVersion { SchemaVersion::V3 }

    fn up(&self, db: &mut Database) -> Result<(), TropError> {
        db.execute(
            "CREATE INDEX idx_project_task ON reservations(project, task)",
            [],
        )?;
        Ok(())
    }

    fn down(&self, db: &mut Database) -> Result<(), TropError> {
        db.execute("DROP INDEX idx_project_task", [])?;
        Ok(())
    }
}
```

## Implementation Considerations

### Safety Features

1. **Automatic Backups**: Always create a backup before migration
2. **Transaction Safety**: All migrations run in a transaction (rollback on failure)
3. **Verification**: Verify schema and data integrity after migration
4. **Dry Run**: Test migrations before applying them

### User Experience

1. **Clear Communication**: Show what will change before migration
2. **Progress Indicators**: Show progress for long migrations
3. **Warnings**: Warn about potential data loss in downgrades
4. **Documentation**: Provide migration guide for each version

### Testing Strategy

1. **Unit Tests**: Test individual migration functions
2. **Integration Tests**: Test full migration paths (v1 → v2 → v3)
3. **Rollback Tests**: Verify down migrations work correctly
4. **Data Integrity**: Verify no data loss during migrations
5. **Performance**: Ensure migrations complete in reasonable time

## When to Implement

This migration system should be implemented **when**:

1. A schema change is needed for a new feature
2. Performance optimization requires schema changes
3. Bug fixes require schema modifications
4. There's a significant user base that needs upgrade support

For v1.0, the current fixed schema is sufficient. This work can be deferred until v2.0 or when the first schema change is actually needed.

## Alternative Approaches

1. **Simple Schema Versioning**: Just track version, require manual migration
2. **Diesel Migrations**: Use Diesel ORM's migration system
3. **Refinery**: Use refinery crate for migration management
4. **No Versioning**: Accept breaking changes, document migration scripts

## References

- [Diesel Migrations](https://docs.diesel.rs/diesel_migrations/index.html)
- [Refinery](https://github.com/rust-db/refinery)
- [SQLite ALTER TABLE](https://www.sqlite.org/lang_altertable.html)
- [Database Migration Best Practices](https://martinfowler.com/articles/evodb.html)
