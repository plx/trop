# Phase 12.2.1: Transaction-Wrapping Concurrency Refactor

## Executive Summary

Phase 12.2 concurrent operation tests revealed a critical architectural misalignment: trop's mutating operations use a two-phase architecture (planning + execution) where planning happens outside any transaction, allowing multiple processes to make conflicting decisions. This causes race conditions detected by our tests (duplicate port allocations, frequent retry failures).

This phase refactors the architecture to align with the original design intent: **use database transactions as coarse-grained inter-process locks**, wrapping the entire operation (planning + execution + non-DB work) in a single transaction. This serializes concurrent operations naturally, eliminating race conditions.

## Problem Statement

### Current Architecture

**Mutating operations** (reserve, release, cleanup, migrate) follow a two-phase pattern:

**Phase 1: Planning** (OUTSIDE transaction)
- File: `trop/src/operations/reserve.rs:298` - `build_plan()`
- Makes multiple database queries without a transaction:
  - `db.validate_path_relationship()`
  - `db.get_reservation()`
  - `allocator.allocate_single()` → calls `db.is_port_reserved()` for each candidate port
  - Potentially dozens of queries during port scanning
- Also performs non-DB work (system occupancy checks)
- Multiple processes can all plan independently and choose the same port

**Phase 2: Execution** (EACH action gets its OWN transaction)
- File: `trop/src/operations/executor.rs:169` - `execute()`
- Each action (CreateReservation, UpdateReservation, etc.) wraps itself in a transaction
- By this point, the port was already chosen during planning
- UNIQUE constraint prevents corruption but causes failures

### The Race Condition

```
Process A                          Process B
---------                          ---------
[NO TRANSACTION]                   [NO TRANSACTION]
build_plan():                      build_plan():
  query: is_port_reserved(5001)      query: is_port_reserved(5001)
  → false                            → false
  decides: use port 5001             decides: use port 5001

execute():                         execute():
  BEGIN TRANSACTION                  BEGIN TRANSACTION
  insert (5001, /path/A)             insert (5001, /path/B)
  COMMIT                             Error: "Port allocated by another process"
```

**Consequences:**
- Frequent failures in concurrent scenarios
- User must retry operations
- Tests show duplicate port allocation attempts
- Poor user experience under concurrent load

### Evidence from Tests

Phase 12.2 tests (`trop/tests/race_conditions.rs:52` - `test_toctou_port_availability`):
- 20 concurrent processes, narrow port range (50000-50010, 11 ports)
- Expected: At most 11 succeed, 9+ fail with clear error
- Actual: 20 allocations recorded but only 11 unique ports (indicating many processes chose the same ports during planning, then some failed and fell back)

## Solution: Transaction-Wrapping Architecture

### Intended Design

Wrap the **entire operation** in a single database transaction:

```
Process A                          Process B
---------                          ---------
BEGIN TRANSACTION                  BEGIN TRANSACTION
                                     ↓
                                   (BLOCKS waiting for A's lock)
build_plan():
  query: is_port_reserved(5001)
  → false
  decides: use port 5001

  [non-DB work: occupancy checks]

execute():
  insert (5001, /path/A)
COMMIT
                                   (Now gets the lock)
                                   build_plan():
                                     query: is_port_reserved(5001)
                                     → true (A has it!)
                                     decides: use port 5002
                                   execute():
                                     insert (5002, /path/B)
                                   COMMIT
```

**Key properties:**
- Transaction acts as a coarse-grained inter-process lock
- Operations serialize naturally
- No race conditions possible
- Each process sees a consistent view of the database
- Non-DB work (occupancy checks) happens inside the transaction (acceptable for our use case)

### Performance Considerations

**Trade-off:** This serializes operations, reducing throughput under heavy concurrent load.

**Justification:**
- trop is a developer tool, not a high-scale service
- Typical usage: 10s of operations/minute, not 1000s/second
- Correctness > throughput for this use case
- Serialization is predictable and doesn't cause user-visible delays (transactions are fast)

## Implementation Strategy

### High-Level Approach

1. **Introduce Transaction abstraction** - wrapper around `rusqlite::Transaction` with trop-specific helpers
2. **Refactor database operations** - accept `Transaction` OR `Connection` (for backwards compatibility during transition)
3. **Update planning phase** - accept `Transaction` instead of `Database`
4. **Update execution phase** - work within existing transaction, don't create new ones
5. **Update CLI commands** - wrap operations in transactions
6. **Update tests** - verify transaction behavior
7. **Remove atomic creation workaround** - `try_create_reservation_atomic()` becomes unnecessary

### Migration Path

**Phase 1: Add Transaction support (backwards-compatible)**
- Add `Transaction` wrapper type
- Database methods accept `&Transaction` OR `&Connection` (using trait)
- Old code continues to work

**Phase 2: Update operations**
- Planning and execution accept `Transaction`
- CLI commands create transactions

**Phase 3: Remove workarounds**
- Delete `try_create_reservation_atomic()`
- Simplify executor logic
- UNIQUE constraint remains (defense-in-depth)

## Detailed Implementation Steps

### Step 1: Create Transaction Abstraction

**File:** `trop/src/database/transaction.rs` (new)

```rust
//! Transaction wrapper for trop operations.
//!
//! This module provides a Transaction type that wraps rusqlite::Transaction
//! with trop-specific helpers and ensures proper usage patterns.

use rusqlite::{Connection, Transaction as RusqliteTransaction, TransactionBehavior};
use crate::error::Result;

/// Transaction wrapper for atomic operations.
///
/// This type provides RAII-based transaction management with trop-specific
/// helpers. Transactions use IMMEDIATE locking to serialize concurrent
/// operations across processes.
///
/// # Examples
///
/// ```no_run
/// use trop::database::{Database, DatabaseConfig};
///
/// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
/// let tx = db.begin_transaction().unwrap();
///
/// // ... perform operations within transaction ...
///
/// tx.commit().unwrap();
/// ```
pub struct Transaction<'conn> {
    inner: RusqliteTransaction<'conn>,
}

impl<'conn> Transaction<'conn> {
    /// Creates a new transaction with IMMEDIATE locking behavior.
    ///
    /// IMMEDIATE transactions acquire a write lock immediately, which
    /// serializes concurrent operations across processes. This is the
    /// intended concurrency model for trop.
    pub fn new(conn: &'conn mut Connection) -> Result<Self> {
        let inner = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Ok(Self { inner })
    }

    /// Commits the transaction.
    ///
    /// This consumes the transaction and commits all changes to the database.
    pub fn commit(self) -> Result<()> {
        self.inner.commit()?;
        Ok(())
    }

    /// Rolls back the transaction.
    ///
    /// This is rarely needed explicitly since Drop will rollback if not committed.
    pub fn rollback(self) -> Result<()> {
        self.inner.rollback()?;
        Ok(())
    }

    /// Returns a reference to the underlying rusqlite transaction.
    ///
    /// This provides access for database operation methods that need
    /// the raw transaction.
    pub fn inner(&self) -> &RusqliteTransaction<'conn> {
        &self.inner
    }
}
```

**File:** `trop/src/database/mod.rs` (update)

```rust
// Add new module
pub mod transaction;

// Re-export Transaction type
pub use transaction::Transaction;
```

**File:** `trop/src/database/connection.rs` (update)

```rust
use super::transaction::Transaction;

impl Database {
    // ... existing methods ...

    /// Begins a new transaction with IMMEDIATE locking.
    ///
    /// This transaction will serialize with other concurrent operations,
    /// providing the coarse-grained locking behavior intended for trop.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::database::{Database, DatabaseConfig};
    ///
    /// let mut db = Database::open(DatabaseConfig::new("/tmp/trop.db")).unwrap();
    /// let tx = db.begin_transaction().unwrap();
    ///
    /// // Perform operations...
    ///
    /// tx.commit().unwrap();
    /// ```
    pub fn begin_transaction(&mut self) -> Result<Transaction> {
        Transaction::new(&mut self.conn)
    }
}
```

### Step 2: Create Trait for Connection-or-Transaction

**File:** `trop/src/database/operations.rs` (update)

```rust
use rusqlite::{Connection, Transaction as RusqliteTransaction};

/// Trait for types that can execute database queries.
///
/// This allows database operations to work with either a Connection
/// (for read-only operations or when no transaction is needed) or
/// a Transaction (for atomic multi-step operations).
pub trait DbExecutor {
    /// Returns a reference to the underlying connection for query execution.
    fn conn(&self) -> &Connection;
}

impl DbExecutor for Connection {
    fn conn(&self) -> &Connection {
        self
    }
}

impl<'conn> DbExecutor for RusqliteTransaction<'conn> {
    fn conn(&self) -> &Connection {
        self
    }
}

impl<'conn> DbExecutor for super::transaction::Transaction<'conn> {
    fn conn(&self) -> &Connection {
        self.inner()
    }
}

impl DbExecutor for Database {
    fn conn(&self) -> &Connection {
        &self.conn
    }
}
```

### Step 3: Refactor Database Operations to Use Trait

**File:** `trop/src/database/operations.rs` (update)

Update all database operation methods to accept `&impl DbExecutor` instead of `&self`:

```rust
impl Database {
    // Current signature:
    pub fn get_reservation(&self, key: &ReservationKey) -> Result<Option<Reservation>> {
        let (path_str, tag) = key.as_query_params();
        // ...
    }

    // New signature:
    pub fn get_reservation(
        executor: &impl DbExecutor,
        key: &ReservationKey,
    ) -> Result<Option<Reservation>> {
        let (path_str, tag) = key.as_query_params();
        let mut stmt = executor.conn().prepare_cached(SELECT_RESERVATION)?;
        // ...
    }
}
```

This requires updating:
- `get_reservation()`
- `list_all_reservations()`
- `is_port_reserved()`
- `create_reservation()`
- `update_last_used()`
- `delete_reservation()`
- `cleanup_orphaned()`
- `cleanup_expired()`
- `get_reservation_by_port()`
- `get_reserved_ports_in_range()`
- All other database methods

**Pattern for conversion:**
```rust
// Old:
self.conn.prepare_cached(QUERY)?

// New:
executor.conn().prepare_cached(QUERY)?
```

### Step 4: Update Planning Phase to Accept Transaction

**File:** `trop/src/operations/reserve.rs` (update)

```rust
impl ReservePlan {
    // Current signature:
    pub fn build_plan(&self, db: &mut Database) -> Result<OperationPlan> {
        // ...
    }

    // New signature (backwards-compatible via trait):
    pub fn build_plan(&self, executor: &impl DbExecutor) -> Result<OperationPlan> {
        // Step 1: Validate path relationship
        if !self.options.force && !self.options.allow_unrelated_path {
            Database::validate_path_relationship(
                executor,
                &self.options.key.path,
                false
            )?;
        }

        // Step 2: Check for existing reservation
        if let Some(existing) = Database::get_reservation(executor, &self.options.key)? {
            // ...
        }

        // Step 3: Determine port
        let port = {
            let allocator = allocator_from_config(self.config)?;
            // allocator methods also need to accept executor
            let result = allocator.allocate_single(executor, &options, &config)?;
            // ...
        };

        // ...
    }
}
```

**File:** `trop/src/port/allocator.rs` (update)

```rust
impl PortAllocator {
    // Update allocate_single to accept executor trait
    pub fn allocate_single(
        &self,
        executor: &impl DbExecutor,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<AllocationResult> {
        // ...
        if Database::is_port_reserved(executor, port)? {
            return Ok(PortAvailability::Reserved);
        }
        // ...
    }

    // Similar updates for other allocation methods
}
```

### Step 5: Update Execution Phase

**File:** `trop/src/operations/executor.rs` (update)

```rust
pub struct PlanExecutor<'conn> {
    executor: &'conn dyn DbExecutor,
    dry_run: bool,
}

impl<'conn> PlanExecutor<'conn> {
    // Accept any DbExecutor (Connection or Transaction)
    pub fn new(executor: &'conn impl DbExecutor) -> Self {
        Self {
            executor,
            dry_run: false,
        }
    }

    fn execute_action(&mut self, action: &PlanAction) -> Result<Option<HashMap<String, Port>>> {
        match action {
            PlanAction::CreateReservation(reservation) => {
                // No longer need atomic creation - we're already in a transaction!
                // Just insert directly
                Database::create_reservation(self.executor, reservation)?;
                Ok(None)
            }
            PlanAction::UpdateReservation(reservation) => {
                Database::create_reservation(self.executor, reservation)?;
                Ok(None)
            }
            PlanAction::UpdateLastUsed(key) => {
                Database::update_last_used(self.executor, key)?;
                Ok(None)
            }
            PlanAction::DeleteReservation(key) => {
                Database::delete_reservation(self.executor, key)?;
                Ok(None)
            }
            // ... other actions
        }
    }
}
```

### Step 6: Update CLI Commands to Use Transactions

**File:** `trop-cli/src/commands/reserve.rs` (update)

```rust
impl ReserveCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // ... argument parsing ...

        // Open database
        let mut db = open_database(global, &config)?;

        // BEGIN TRANSACTION (wraps entire operation)
        let tx = db.begin_transaction().map_err(CliError::from)?;

        // Build plan (inside transaction)
        let plan = ReservePlan::new(options, &config)
            .build_plan(&tx)  // Pass transaction
            .map_err(CliError::from)?;

        // Execute plan (inside same transaction)
        let mut executor = PlanExecutor::new(&tx);
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // COMMIT TRANSACTION (only if everything succeeded)
        tx.commit().map_err(CliError::from)?;

        // Output result
        if let Some(port) = result.port {
            println!("{}", port.value());
        }

        Ok(())
    }
}
```

Apply similar pattern to:
- `trop-cli/src/commands/release.rs`
- `trop-cli/src/commands/reserve_group.rs`
- `trop-cli/src/commands/cleanup.rs`
- `trop-cli/src/commands/migrate.rs`
- Any other mutating commands

**Read-only commands** (list, list-projects, port-info, etc.) can continue using `&db` directly - no transaction needed.

### Step 7: Remove Atomic Creation Workaround

**File:** `trop/src/database/operations.rs` (delete method)

Delete the `try_create_reservation_atomic()` method entirely - it's no longer needed since we're always in a transaction when creating reservations.

**File:** `trop/src/operations/executor.rs` (simplify)

The executor no longer needs special handling for concurrent conflicts - the transaction serializes everything:

```rust
// OLD:
PlanAction::CreateReservation(reservation) => {
    let created = self.db.try_create_reservation_atomic(reservation)?;
    if !created {
        return Err(Error::Validation {
            field: "port".into(),
            message: format!("Port {} was allocated...", ...),
        });
    }
    Ok(None)
}

// NEW:
PlanAction::CreateReservation(reservation) => {
    Database::create_reservation(self.executor, reservation)?;
    Ok(None)
}
```

### Step 8: Update Tests

**File:** `trop/tests/concurrent_operations.rs` (update expectations)

With transactions serializing operations, the behavior changes:

```rust
#[test]
fn test_concurrent_reservations_no_conflicts() {
    // ... setup ...

    // All 10 reservations should succeed (serialized by transactions)
    let success_count = results.iter().filter(|(success, _)| *success).count();
    assert_eq!(success_count, 10, "All concurrent reservations should succeed");

    // Extract allocated ports
    let ports: Vec<u16> = results.iter()
        .filter_map(|(_, stdout)| stdout.trim().parse().ok())
        .collect();

    // All ports should be unique (no conflicts possible)
    let unique_ports: HashSet<_> = ports.iter().collect();
    assert_eq!(
        ports.len(),
        unique_ports.len(),
        "All allocated ports must be unique"
    );
    assert_eq!(ports.len(), 10, "Should allocate 10 distinct ports");
}
```

**File:** `trop/tests/race_conditions.rs` (update expectations)

```rust
#[test]
fn test_toctou_port_availability() {
    // ... setup with narrow range (50000-50010, 11 ports) ...

    // Spawn 20 processes
    let handles: Vec<_> = (0..20).map(|i| {
        // ... spawn processes ...
    }).collect();

    let results: Vec<_> = handles.into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    let success_count = results.iter().filter(|r| r.status.success()).count();

    // With transactions serializing operations:
    // - First 11 succeed (one per available port)
    // - Last 9 fail cleanly with "No available ports" error
    assert_eq!(success_count, 11, "Exactly 11 reservations should succeed");

    let ports: Vec<u16> = results.iter()
        .filter(|r| r.status.success())
        .filter_map(|r| String::from_utf8_lossy(&r.stdout).trim().parse().ok())
        .collect();

    // All successful allocations have unique ports
    let unique_ports: HashSet<_> = ports.iter().collect();
    assert_eq!(
        ports.len(),
        unique_ports.len(),
        "All allocated ports must be unique"
    );

    // Verify failures are clean
    let failures: Vec<_> = results.iter().filter(|r| !r.status.success()).collect();
    assert_eq!(failures.len(), 9, "Exactly 9 reservations should fail");

    for result in failures {
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("No available ports") || stderr.contains("exhausted"),
            "Failures should report port exhaustion"
        );
    }
}
```

**File:** `trop/src/database/operations.rs` (update tests)

Tests that create reservations directly need to use transactions:

```rust
#[test]
fn test_create_reservation() {
    let mut db = create_test_database();
    let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
    let port = Port::try_from(8080).unwrap();
    let reservation = Reservation::builder(key.clone(), port).build().unwrap();

    // Use transaction
    let tx = db.begin_transaction().unwrap();
    Database::create_reservation(&tx, &reservation).unwrap();
    tx.commit().unwrap();

    // Verify
    let loaded = Database::get_reservation(&db, &key).unwrap();
    assert!(loaded.is_some());
}
```

### Step 9: Update Documentation

**Files to update:**
- `trop/src/database/operations.rs` - Update doc comments to reflect transaction usage
- `trop/src/operations/reserve.rs` - Update examples in doc comments
- `trop-cli/README.md` - Add note about concurrent behavior (operations serialize)
- Any architecture docs

## Testing Strategy

### Unit Tests

1. **Transaction creation and commit:**
   - `trop/src/database/transaction.rs` - test transaction lifecycle
   - Verify IMMEDIATE locking behavior

2. **DbExecutor trait:**
   - Verify all database methods work with `Transaction` and `Connection`
   - Test backwards compatibility

3. **Operation planning with transactions:**
   - `trop/src/operations/reserve.rs` - test build_plan() with transaction
   - Verify planning queries execute within transaction

### Integration Tests

1. **Serial execution verification:**
   - Create test that spawns concurrent processes
   - Verify operations serialize (measure timing)
   - Confirm no conflicts occur

2. **Concurrent operation tests (existing, update expectations):**
   - `test_concurrent_reservations_no_conflicts` - all should succeed
   - `test_toctou_port_availability` - exactly 11 succeed, 9 fail cleanly
   - `test_concurrent_readers_during_write` - readers may block during writes
   - `test_database_consistency_after_concurrent_ops` - verify integrity

3. **Stress tests (update expectations):**
   - Operations take longer (serialized) but never conflict
   - Verify no deadlocks or starvation

### Manual Testing

1. **Concurrent reserve operations:**
   ```bash
   for i in {1..20}; do
     trop reserve --path /tmp/test-$i --allow-unrelated-path &
   done
   wait
   # All should succeed with unique ports
   ```

2. **Port exhaustion scenario:**
   ```bash
   trop init --with-config
   # Edit config to narrow range: port_range = [60000, 60005]
   for i in {1..10}; do
     trop reserve --path /tmp/test-$i --allow-unrelated-path
   done
   # First 6 succeed, rest fail with clear error
   ```

3. **Performance under concurrent load:**
   - Measure latency with varying concurrency (1, 5, 10, 20 processes)
   - Verify acceptable performance (< 100ms per operation even with serialization)

## Migration Considerations

### Backwards Compatibility

The refactoring maintains API compatibility for library users:

**Old code (still works):**
```rust
let mut db = Database::open(config)?;
let plan = ReservePlan::new(options, &config).build_plan(&db)?;
```

**New code (recommended):**
```rust
let mut db = Database::open(config)?;
let tx = db.begin_transaction()?;
let plan = ReservePlan::new(options, &config).build_plan(&tx)?;
tx.commit()?;
```

Both work because of the `DbExecutor` trait.

### Breaking Changes

**None for CLI users** - commands work identically, just with better concurrency behavior.

**Possible for library users who:**
- Directly call `Database::try_create_reservation_atomic()` (method deleted)
- Rely on specific transaction boundaries in operations
- Implement custom operations outside the standard patterns

These are acceptable since trop is still pre-1.0.

### Database Compatibility

No schema changes required - the UNIQUE constraint added in Phase 12.2 remains (defense-in-depth).

Existing databases work without migration.

## Success Criteria

### Must-Have

- [ ] All database operations accept `DbExecutor` trait (Connection or Transaction)
- [ ] All mutating CLI commands wrap operations in transactions
- [ ] `try_create_reservation_atomic()` deleted
- [ ] All 637+ existing tests pass with new architecture
- [ ] Phase 12.2 concurrent tests pass with updated expectations:
  - `test_concurrent_reservations_no_conflicts` - 10/10 succeed, all unique
  - `test_toctou_port_availability` - 11/20 succeed, 9 fail cleanly, all unique
  - `test_database_consistency_after_concurrent_ops` - integrity maintained
  - `test_transaction_isolation` - group allocations atomic
- [ ] No clippy warnings
- [ ] No regressions in single-process performance (< 5% slower)

### Nice-to-Have

- [ ] Concurrent operation benchmark showing serialization overhead
- [ ] Documentation updates explaining concurrency model
- [ ] Examples showing transaction usage patterns
- [ ] Performance test showing acceptable latency under concurrent load (< 100ms p95)

## Implementation Order

1. **Day 1: Foundation**
   - Create `Transaction` type and `DbExecutor` trait
   - Update 2-3 database methods to use trait (prove concept)
   - Add basic tests

2. **Day 2: Database Layer**
   - Update all remaining database methods
   - Update allocator to use trait
   - Verify all library tests pass

3. **Day 3: Operations Layer**
   - Update planning phase (`build_plan()` methods)
   - Update execution phase (executor)
   - Verify operation tests pass

4. **Day 4: CLI Layer**
   - Update all CLI commands to use transactions
   - Remove `try_create_reservation_atomic()`
   - Update integration tests

5. **Day 5: Testing & Polish**
   - Update Phase 12.2 test expectations
   - Run stress tests
   - Performance validation
   - Documentation updates

## Risk Mitigation

**Risk:** Performance regression from serialization

**Mitigation:**
- Measure baseline performance first
- Monitor transaction duration
- If needed, add timeout configuration
- Acceptable trade-off: correctness > throughput for trop use case

**Risk:** Deadlocks from complex operations

**Mitigation:**
- Use IMMEDIATE transactions (explicit locking, no upgrades)
- Keep transactions short (no long-running operations inside)
- Document that non-DB work (occupancy checks) happens inside transaction
- SQLite's busy timeout prevents infinite blocking

**Risk:** Breaking library users

**Mitigation:**
- `DbExecutor` trait maintains backwards compatibility
- Old patterns continue to work
- Document migration path
- Provide examples of new patterns

## Notes

- This refactoring aligns trop's implementation with its original design intent
- The UNIQUE constraint from Phase 12.2 remains as defense-in-depth
- Transactions serialize operations, which is acceptable for trop's use case
- Phase 12.2 tests correctly identified the architectural issue
- This is a pure refactoring - no new features, just better concurrency model
