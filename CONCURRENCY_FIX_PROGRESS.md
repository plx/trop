# Concurrency Fix Implementation Progress

## Current Branch
`concurrency-fix`

## Overall Goal
Implement transaction-wrapping for the entire reserve operation (planning + execution) to fix TOCTOU race conditions in concurrent port allocation. See `ConcurrencyFixPlan.md` for full context.

---

## ✅ COMPLETED WORK

### Phase A: Database Operations Layer - COMPLETE
**Status**: All library code compiles successfully

**Changes Made**:
1. Added `begin_transaction()` method to `Database` struct (`trop/src/database/connection.rs`)
   - Returns `rusqlite::Transaction<'_>` with IMMEDIATE behavior
   - Acquires write lock immediately to serialize concurrent operations

2. Refactored all database read methods to static methods accepting `&Connection`:
   - `get_reservation(conn: &Connection, key: &ReservationKey)`
   - `list_all_reservations(conn: &Connection)`
   - `get_reserved_ports(conn: &Connection, range: &PortRange)`
   - `get_reservations_by_path_prefix(conn: &Connection, prefix: &Path)`
   - `find_expired_reservations(conn: &Connection, max_age: Duration)`
   - `is_port_reserved(conn: &Connection, port: Port)`
   - `get_reservation_by_port(conn: &Connection, port: Port)`
   - `get_reserved_ports_in_range(conn: &Connection, range: &PortRange)`
   - `list_projects(conn: &Connection)`
   - `validate_path_relationship(target_path: &Path, allow_unrelated: bool)`

3. Added `create_reservation_simple(conn: &Connection, reservation: &Reservation)`
   - For use within existing transactions
   - Does NOT create its own transaction (unlike `create_reservation`)

4. Kept `create_reservation(&mut self, ...)` unchanged for backward compatibility
   - Still creates its own transaction
   - Used by tests and standalone operations

### Phase B: Operations Layer - COMPLETE
**Status**: All library code compiles successfully

**Changes Made**:
1. Updated `PortAllocator` in `trop/src/port/allocator.rs`:
   - `allocate_single(&self, conn: &Connection, ...)`
   - `find_next_available(&self, start: Port, conn: &Connection, ...)`
   - `is_port_available(&self, port: Port, conn: &Connection, ...)`

2. Updated all operation planning code to use new API:
   - `trop/src/operations/cleanup.rs` - uses `Database::list_all_reservations(db.connection())`, etc.
   - `trop/src/operations/executor.rs` - uses `Database::get_reservation(self.db.connection(), ...)`
   - `trop/src/operations/migrate.rs` - uses `Database::get_reservations_by_path_prefix(db.connection(), ...)`
   - `trop/src/operations/release.rs` - uses `Database::validate_path_relationship(...)`, `Database::get_reservation(db.connection(), ...)`
   - `trop/src/operations/reserve.rs` - uses `allocator.allocate_single(db.connection(), ...)`, `Database::get_reservation(db.connection(), ...)`
   - `trop/src/port/group.rs` - uses `self.allocate_single(db.connection(), ...)`, `self.is_port_available(..., db.connection(), ...)`

---

## 🔄 IN PROGRESS

### Phase B (Final Step): Fix CLI Command Call Sites
**Status**: ~7 files need updates, straightforward mechanical changes

**Pattern to Apply**:
```rust
// OLD:
let result = db.method_name(arg)?;

// NEW:
let result = Database::method_name(db.connection(), arg)?;
```

**Files Requiring Updates**:
1. `trop-cli/src/commands/assert_port.rs` - line 31: `db.is_port_reserved(port)` → `Database::is_port_reserved(db.connection(), port)`

2. `trop-cli/src/commands/assert_reservation.rs` - line 41: `db.get_reservation(...)` → `Database::get_reservation(db.connection(), ...)`

3. `trop-cli/src/commands/exclude.rs` - line 107: `db.get_reserved_ports_in_range(...)` → `Database::get_reserved_ports_in_range(db.connection(), ...)`

4. `trop-cli/src/commands/list.rs` - line 81: `db.list_all_reservations()` → `Database::list_all_reservations(db.connection())`

5. `trop-cli/src/commands/list_projects.rs` - line 28: `db.list_projects()` → `Database::list_projects(db.connection())`

6. `trop-cli/src/commands/port_info.rs` - line 32: `db.get_reservation_by_port(...)` → `Database::get_reservation_by_port(db.connection(), ...)`

7. `trop-cli/src/commands/release.rs` - line 63: (check what method call needs updating)

8. `trop-cli/src/commands/scan.rs` - line 85: (check what method call needs updating)

**Verification Command**:
```bash
cargo check --quiet 2>&1 | grep "no method named"
```

Should produce no errors after this phase is complete.

---

## 📋 TODO: Remaining Phases

### Phase C: Update CLI Commands to Use Transactions
**Goal**: Wrap plan + execute in single IMMEDIATE transaction at CLI layer

**Priority Order** (implement in this sequence):
1. Reserve command (most critical for race condition fix)
2. Reserve group command
3. Release command
4. Migrate command
5. Cleanup commands

**Template Pattern for Each Command**:
```rust
// File: trop-cli/src/commands/reserve.rs (example)

impl ReserveCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // ... setup code ...

        let mut db = open_database(global, &config)?;

        // *** BEGIN TRANSACTION ***
        let tx = db.begin_transaction().map_err(CliError::from)?;

        // Build plan (inside transaction - sees consistent view)
        let plan = ReservePlan::new(options, &config)
            .build_plan(&tx)  // TODO: Update build_plan to accept &Connection
            .map_err(CliError::from)?;

        // Execute plan (inside same transaction)
        let mut executor = PlanExecutor::new(&tx);  // TODO: Update PlanExecutor
        let result = executor.execute(&plan).map_err(CliError::from)?;

        // *** COMMIT TRANSACTION ***
        tx.commit().map_err(CliError::from)?;

        // Output result (after commit)
        if let Some(port) = result.port {
            println!("{}", port.value());
        }

        Ok(())
    }
}
```

**Sub-tasks for Phase C**:

**C.1**: Update `build_plan()` methods to accept `&Connection` instead of `&Database`
- Files to update:
  - `trop/src/operations/reserve.rs`: `ReservePlan::build_plan(&self, db: &Database)` → `build_plan(&self, conn: &Connection)`
  - `trop/src/operations/release.rs`: `ReleasePlan::build_plan(&self, db: &Database)` → `build_plan(&self, conn: &Connection)`
  - `trop/src/operations/migrate.rs`: `MigratePlan::build_plan(&self, db: &Database)` → `build_plan(&self, conn: &Connection)`
  - Others as needed

**C.2**: Update `PlanExecutor` to accept `&Connection`
- File: `trop/src/operations/executor.rs`
- Change:
  ```rust
  // OLD:
  pub struct PlanExecutor<'a> {
      db: &'a mut Database,
      dry_run: bool,
  }

  // NEW:
  pub struct PlanExecutor<'conn> {
      conn: &'conn Connection,
      dry_run: bool,
  }

  impl<'conn> PlanExecutor<'conn> {
      pub fn new(conn: &'conn Connection) -> Self {
          Self { conn, dry_run: false }
      }

      // Update all internal methods to use self.conn
  }
  ```

**C.3**: Remove internal transaction creation in executor
- File: `trop/src/operations/executor.rs`
- In `execute_action()` method:
  - Remove transaction wrapping around `Database::create_reservation_simple()`
  - Remove transaction wrapping around other database operations
  - Caller (CLI) now owns the transaction boundary

**C.4**: Update each CLI command file
- `trop-cli/src/commands/reserve.rs` - highest priority
- `trop-cli/src/commands/reserve_group.rs`
- `trop-cli/src/commands/release.rs`
- `trop-cli/src/commands/migrate.rs`
- `trop-cli/src/commands/cleanup.rs` (if it calls plan/execute pattern)

### Phase D: Cleanup Atomic Workarounds
**Goal**: Remove temporary workarounds now that transactions solve the problem

**Files to Update**:

**D.1**: Delete `try_create_reservation_atomic()` method
- File: `trop/src/database/operations.rs` (lines ~509-555)
- This method is now redundant - transaction wrapping at CLI layer replaces it

**D.2**: Search for any retry logic
- Search command: `grep -r "retry\|Retry" trop/src/operations/`
- Remove manual retry logic that was working around race conditions
- Transaction serialization makes retries unnecessary

**D.3**: Verify executor no longer has transaction creation
- File: `trop/src/operations/executor.rs`
- Ensure `execute_action()` doesn't create any transactions

### Phase E: Validate with Tests
**Goal**: Ensure concurrency tests pass and race condition is fixed

**E.1**: Run existing tests
```bash
cargo test --lib
```

**E.2**: Run concurrency tests specifically
```bash
cargo test --lib -- test_toctou_port_availability --nocapture
cargo test --lib -- test_concurrent_reservations_no_conflicts --nocapture
```

**Expected Results** (per plan):
- `test_toctou_port_availability`: Exactly 11/20 succeed (first 11 get lock, last 9 fail cleanly due to serialization)
- `test_concurrent_reservations_no_conflicts`: All 10/10 succeed with unique ports
- No duplicate port allocations ever

**E.3**: Run manual concurrent test
```bash
# Terminal 1
/Users/prb/github/trop/target/debug/trop --data-dir ./data reserve --path /tmp/p3 --port 8000 --allow-unrelated-path --disable-autoclean

# Terminal 2 (run simultaneously)
/Users/prb/github/trop/target/debug/trop --data-dir ./data reserve --path /tmp/p3 --port 8000 --allow-unrelated-path --disable-autoclean
```

Expected: Second command should fail cleanly (port already reserved), NOT allocate duplicate port.

**E.4**: Run clippy
```bash
cargo clippy --quiet
```

**E.5**: Run full test suite
```bash
cargo test
```

---

## Quick Reference Commands

**Check compilation status**:
```bash
cargo check --quiet 2>&1 | head -50
```

**Count errors**:
```bash
cargo check --quiet 2>&1 | grep "error\[E" | wc -l
```

**Find specific method call sites**:
```bash
grep -rn "db\.method_name" trop-cli/src/
```

**Build and test**:
```bash
cargo build --quiet --bin trop
cargo test --quiet
```

---

## Notes

- The `conn` field in `Database` is intentionally `pub(super)` - use `db.connection()` accessor
- `rusqlite::Transaction` implements `Deref<Target = Connection>`, so methods accepting `&Connection` work with both `&Database.connection()` and `&Transaction`
- Keep `create_reservation(&mut self, ...)` for backward compatibility (tests, standalone uses)
- New code should use `create_reservation_simple(conn, ...)` within transactions

---

## Success Criteria

Phase complete when:
- ✅ All code compiles without errors
- ✅ All clippy warnings resolved
- ✅ All tests pass
- ✅ Concurrency tests show expected behavior (serialization via transactions)
- ✅ Manual concurrent testing shows no duplicate port allocations
- ✅ No `try_create_reservation_atomic()` method remains
- ✅ CLI commands wrap plan+execute in single transaction
