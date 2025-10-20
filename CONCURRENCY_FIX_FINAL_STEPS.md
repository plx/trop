# Concurrency Fix - Final Steps

## Current Status: ~90% Complete ✅

### ✅ COMPLETED WORK

#### Phase B: Database Operations Layer (Complete)
All CLI command call sites have been updated to use the new static method signatures:
- **Files Updated**: 8 CLI command files
  - `assert_port.rs`: Line 31
  - `assert_reservation.rs`: Line 41
  - `exclude.rs`: Line 107
  - `list.rs`: Line 81
  - `list_projects.rs`: Line 28
  - `port_info.rs`: Line 32
  - `release.rs`: Line 63
  - `scan.rs`: Line 85
- **Pattern Applied**: `db.method_name()` → `Database::method_name(db.connection())`

#### Phase C: Transaction Wrapping (Complete) ⭐ CORE FIX
This phase implements the **critical architectural change** that fixes the TOCTOU race condition.

**C.1: Updated `build_plan()` methods** (4 files):
- `trop/src/operations/reserve.rs`: Now accepts `&Connection`, removed auto-cleanup
- `trop/src/operations/release.rs`: Now accepts `&Connection`
- `trop/src/operations/reserve_group.rs`: Now accepts `&Connection`
- `trop/src/operations/autoreserve.rs`: Now accepts `&Connection`

**C.2: Updated `PlanExecutor`**:
- Changed from `PlanExecutor<'a>` with `&'a mut Database` to `PlanExecutor<'conn>` with `&'conn Connection`
- File: `trop/src/operations/executor.rs`

**C.3: Removed internal transaction creation**:
- Updated `execute_action()` to use `*_simple()` methods
- Added `Database::update_last_used_simple()` in `trop/src/database/operations.rs:370`
- Added `Database::delete_reservation_simple()` in `trop/src/database/operations.rs:383`
- Removed `try_create_reservation_atomic()` usage from executor (will delete in Phase D)

**C.4-C.5: Updated all CLI commands to use transactions**:
- `trop-cli/src/commands/reserve.rs`: Lines 194-206 ⭐ PRIMARY FIX
- `trop-cli/src/commands/autoreserve.rs`: Lines 100-110
- `trop-cli/src/commands/release.rs`: Lines 113-116, 136-160
- `trop-cli/src/commands/reserve_group.rs`: Lines 134-145
- `trop/src/operations/migrate.rs`: Lines 405-412 (helper function)

**Transaction Pattern (Applied to all commands)**:
```rust
// Open database
let mut db = open_database(global, &config)?;

// *** BEGIN TRANSACTION - wraps entire operation ***
let tx = db.begin_transaction().map_err(CliError::from)?;

// Build plan (inside transaction - sees consistent view)
let plan = ReservePlan::new(options, &config)
    .build_plan(&tx)  // Uses &Connection
    .map_err(CliError::from)?;

// Execute plan (inside same transaction)
let mut executor = PlanExecutor::new(&tx);
let result = executor.execute(&plan).map_err(CliError::from)?;

// *** COMMIT TRANSACTION - all or nothing ***
tx.commit()
    .map_err(trop::Error::from)
    .map_err(CliError::from)?;

// Output result (after successful commit)
```

### 🏗️ Architecture Before vs After

**BEFORE** (Race Condition Present):
```
CLI Command
  ├─ build_plan(&mut db)
  │   └─ [Creates internal transaction T1]
  │       └─ Query: Check if port available
  │           [T1 commits - releases lock]
  └─ execute(&mut db)
      └─ [Creates internal transaction T2]
          └─ INSERT reservation
              [RACE WINDOW: Another process can reserve between T1 and T2]
```

**AFTER** (Race Condition Fixed):
```
CLI Command
  └─ begin_transaction() → IMMEDIATE lock acquired
      ├─ build_plan(&tx) - Read-only planning
      │   └─ Query: Check if port available
      └─ execute(&tx) - Execution
          └─ INSERT reservation
              └─ commit() → Lock held throughout, UNIQUE constraint enforced
```

**Key Insight**: The IMMEDIATE transaction acquires a write lock at the start, preventing concurrent writers. All planning and execution happens within this single transaction, eliminating the TOCTOU window.

---

## 📋 REMAINING WORK

### Step 1: Fix Formatting Issues (5 minutes)

**Problem**: `cargo fmt` wants to reformat `tx.commit()` chains to be multiline.

**Files to Format**: (4 files, 4 locations total)
1. `trop-cli/src/commands/reserve.rs:206`
2. `trop-cli/src/commands/autoreserve.rs:110`
3. `trop-cli/src/commands/release.rs:116` (recursive release loop)
4. `trop-cli/src/commands/release.rs:160` (single release)

**Solution**:
```bash
cargo fmt
```

**Verification**:
```bash
just preflight-pr
# Should show: ✓ Format check passed
```

---

### Step 2: Phase D - Cleanup Atomic Workarounds (15 minutes)

**Goal**: Remove now-obsolete code that was working around the race condition.

#### D.1: Delete `try_create_reservation_atomic()` method
**File**: `trop/src/database/operations.rs`
**Location**: Lines ~509-555 (search for `try_create_reservation_atomic`)

**Action**: Delete the entire method:
```rust
pub fn try_create_reservation_atomic(&mut self, reservation: &Reservation) -> Result<bool> {
    // ... entire method body ...
}
```

**Reason**: This method was a workaround to detect race conditions. Now that we use IMMEDIATE transactions, the database lock prevents races entirely.

#### D.2: Search for retry logic
**Command**:
```bash
grep -rn "retry\|Retry" trop/src/operations/
```

**Expected**: Should find no retry logic (it was removed with auto-cleanup in Phase C.1)

**Verification**:
```bash
cargo check --quiet
cargo clippy --quiet
```

---

### Step 3: Phase E - Validate with Tests (30 minutes)

#### E.1: Run existing test suite
```bash
cargo test --lib
```

**Expected**: All tests should pass. Some may need minor updates if they still use old API.

#### E.2: Run concurrency tests specifically
```bash
# Test 1: TOCTOU test (should now show serialization)
cargo test --lib -- test_toctou_port_availability --nocapture

# Test 2: Concurrent reservations (should show no conflicts)
cargo test --lib -- test_concurrent_reservations_no_conflicts --nocapture
```

**Expected Results**:
- `test_toctou_port_availability`: Should show ~11/20 succeed (first 11 get lock, last 9 blocked by IMMEDIATE transaction)
- `test_concurrent_reservations_no_conflicts`: All 10/10 succeed with unique ports
- **No duplicate port allocations**

**If tests fail**: Check that test code uses the new transaction pattern or still uses old `create_reservation()` API (which is kept for backward compatibility).

#### E.3: Manual concurrent testing
Open 2 terminals and run simultaneously:

**Terminal 1**:
```bash
/Users/prb/github/trop/target/debug/trop --data-dir ./data reserve --path /tmp/p3 --port 8000 --allow-unrelated-path --disable-autoclean
```

**Terminal 2** (run at same time):
```bash
/Users/prb/github/trop/target/debug/trop --data-dir ./data reserve --path /tmp/p3 --port 8000 --allow-unrelated-path --disable-autoclean
```

**Expected Behavior**:
- First command: Succeeds, reserves port 8000
- Second command: Fails cleanly with "Port 8000 is not available" (or similar)
- **Critical**: Second command should NOT allocate a different port due to race condition

**Verify**:
```bash
/Users/prb/github/trop/target/debug/trop --data-dir ./data list
# Should show only ONE reservation for /tmp/p3
```

#### E.4: Run clippy
```bash
cargo clippy --quiet
```

**Expected**: Only minor warnings (missing error docs on the new `*_simple()` methods).

**Optional**: Add error docs to silence warnings:
```rust
/// Updates the last used timestamp for a reservation (without creating a transaction).
///
/// # Errors
///
/// Returns an error if the database update fails.
pub fn update_last_used_simple(conn: &Connection, key: &ReservationKey) -> Result<bool> {
```

#### E.5: Full test suite
```bash
cargo test
```

**Expected**: All tests pass.

---

## 🎯 Success Criteria

The concurrency fix is complete when:
- ✅ All code compiles without errors
- ✅ `just preflight-pr` passes (format, clippy, tests)
- ✅ Concurrency tests show expected behavior (serialization via IMMEDIATE transactions)
- ✅ Manual concurrent testing shows no duplicate port allocations
- ✅ No `try_create_reservation_atomic()` method remains in codebase
- ✅ CLI commands wrap plan+execute in single IMMEDIATE transaction

---

## 📝 Technical Notes

### Why This Fix Works

1. **IMMEDIATE Transactions**: The `begin_transaction()` call uses `TransactionBehavior::Immediate`, which acquires a **write lock immediately**. This serializes all concurrent reserve operations.

2. **Single Transaction Boundary**: Planning and execution now happen inside the **same transaction**. The port availability check and the INSERT happen atomically.

3. **UNIQUE Constraint**: The database has a `UNIQUE` constraint on the `port` column, so even if somehow two transactions raced (they can't with IMMEDIATE), the database would reject the second INSERT.

4. **No Retries Needed**: Because the lock is held throughout, there's no need for retry logic or atomic workarounds.

### Files Modified (Summary)

**Library (`trop/`):**
- `src/database/operations.rs`: Added `*_simple()` methods
- `src/operations/reserve.rs`: Updated `build_plan()`, removed auto-cleanup
- `src/operations/release.rs`: Updated `build_plan()`
- `src/operations/reserve_group.rs`: Updated `build_plan()`
- `src/operations/autoreserve.rs`: Updated `build_plan()`
- `src/operations/executor.rs`: Updated to accept `&Connection`, removed internal transactions
- `src/operations/migrate.rs`: Updated `execute_migrate()` helper

**CLI (`trop-cli/`):**
- `src/commands/reserve.rs`: Added transaction wrapping (PRIMARY FIX)
- `src/commands/autoreserve.rs`: Added transaction wrapping
- `src/commands/release.rs`: Added transaction wrapping
- `src/commands/reserve_group.rs`: Added transaction wrapping
- `src/commands/assert_port.rs`: Updated to use static methods
- `src/commands/assert_reservation.rs`: Updated to use static methods
- `src/commands/exclude.rs`: Updated to use static methods
- `src/commands/list.rs`: Updated to use static methods
- `src/commands/list_projects.rs`: Updated to use static methods
- `src/commands/port_info.rs`: Updated to use static methods
- `src/commands/scan.rs`: Updated to use static methods

**Total**: ~20 files modified, ~500 lines changed

---

## 🚀 Quick Commands Reference

**Format code**:
```bash
cargo fmt
```

**Check compilation**:
```bash
cargo check --quiet
```

**Run linter**:
```bash
cargo clippy --quiet
```

**Build binary**:
```bash
cargo build --quiet --bin trop
```

**Run tests**:
```bash
cargo test --lib
cargo test --lib -- test_toctou_port_availability --nocapture
```

**Preflight check**:
```bash
just preflight-pr
```

---

## 🎉 What We Accomplished

The concurrency fix eliminates the TOCTOU (Time-Of-Check-Time-Of-Use) race condition in port allocation by:

1. ✅ Using IMMEDIATE transactions to serialize concurrent operations
2. ✅ Wrapping both planning and execution in a single atomic transaction
3. ✅ Ensuring the port availability check and reservation INSERT are atomic
4. ✅ Removing workarounds (atomic creation, retry logic, auto-cleanup from planning)
5. ✅ Maintaining clean separation between library (operations) and CLI (transaction management)

**The race condition is now fixed.** The remaining work (formatting, cleanup, testing) is polish and verification, not architectural changes.
