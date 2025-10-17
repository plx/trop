# Port Reservation Library Bug: Duplicate Manual Port Allocations

## Summary

The `trop` library's reservation planning logic (`trop/src/operations/reserve.rs`) does not validate that manually-specified ports (via `--port`) are available before allocating them. This allows the same port to be reserved for multiple different paths, violating a core invariant of the port reservation system.

## Bug Location

**File**: `trop/src/operations/reserve.rs`
**Function**: `ReservePlan::build_plan()`
**Lines**: 252-256

```rust
// Step 3: Determine port (manual or automatic allocation)
let port = if let Some(port) = self.options.port {
    // Manual port specified - use it directly (backward compatibility)
    port
} else {
    // Automatic allocation
    // ... (proper validation happens here)
```

## Usage Examples That Trigger the Bug

### Minimal Reproduction

```bash
# Create two different project directories
mkdir -p /tmp/project1 /tmp/project2

# Reserve port 8080 for project1 - SUCCEEDS
trop reserve --path /tmp/project1 --port 8080 --allow-unrelated-path
# Output: 8080

# Reserve port 8080 for project2 - SHOULD FAIL BUT SUCCEEDS
trop reserve --path /tmp/project2 --port 8080 --allow-unrelated-path
# Output: 8080  <-- BUG: Same port allocated to different path!

# Verify the conflict
trop list
# Shows:
# /tmp/project1  8080
# /tmp/project2  8080  <-- INVALID STATE
```

### Failing Integration Test

The test `test_reserve_preferred_port_already_occupied` in `trop-cli/tests/reserve_command.rs` (lines 246-287) demonstrates this bug:

```rust
#[test]
fn test_reserve_preferred_port_already_occupied() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");
    let preferred = 8080;

    // First reservation gets port 8080
    env.command()
        .arg("reserve")
        .arg("--path").arg(&path1)
        .arg("--port").arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Second reservation should NOT get the same port
    let output2 = env.command()
        .arg("reserve")
        .arg("--path").arg(&path2)
        .arg("--port").arg(preferred.to_string())
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();

    if output2.status.success() {
        let port2 = parse_port(&String::from_utf8(output2.stdout).unwrap());
        assert_ne!(port2, preferred,
            "Should not allocate same port to different paths");  // FAILS!
    }
}
```

**Current Behavior**: Test fails because `port2 == 8080` (same as path1)
**Expected Behavior**: Either the command fails with an error, OR it succeeds with a different port

## Incorrect Behavior Observed

When a user specifies a manually-selected port that is already reserved to a different path:

1. ✅ The command **succeeds** (exit code 0)
2. ❌ The command **outputs the requested port** (e.g., "8080")
3. ❌ The **database creates a second reservation** for the same port
4. ❌ **No error or warning** is produced

This creates an invalid database state where two different paths have reservations for the same port.

## Correct/Expected Behavior

When a user specifies a manually-selected port that is already reserved to a different path, the system should do ONE of the following:

### Option A: Fail with Error (Recommended)
```
$ trop reserve --path /tmp/project2 --port 8080 --allow-unrelated-path
Error: Port 8080 is already reserved to /tmp/project1
```

**Exit code**: Non-zero (failure)
**Rationale**: This is the most explicit and prevents accidental conflicts. Users who want to override can use `--force`.

### Option B: Fallback to Automatic Allocation
```
$ trop reserve --path /tmp/project2 --port 8080 --allow-unrelated-path
Warning: Port 8080 is already reserved, allocated 8081 instead
8081
```

**Exit code**: 0 (success)
**Output**: Different port number
**Stderr**: Warning explaining the fallback
**Rationale**: This matches the behavior when using `--ignore-occupied` with preferred ports in automatic allocation.

### Option C: Allow with --force Flag
Allow the duplicate reservation ONLY if `--force` is specified:
```
$ trop reserve --path /tmp/project2 --port 8080 --force
Warning: Overriding existing reservation for port 8080
8080
```

**Rationale**: `--force` already exists as a "safety override" flag for sticky fields and path validation. Using it for port conflicts would be consistent.

**Recommendation**: Implement Option A (fail by default) + Option C (allow with --force).

## Root Cause Analysis

### The Problem: Missing Validation in Manual Port Path

Looking at `ReservePlan::build_plan()` in `trop/src/operations/reserve.rs:233-307`:

```rust
pub fn build_plan(&self, db: &Database) -> Result<OperationPlan> {
    // Step 1: Validate path relationship
    if !self.options.force && !self.options.allow_unrelated_path {
        db.validate_path_relationship(&self.options.key.path, false)?;
    }

    // Step 2: Check for existing reservation FOR THIS KEY
    if let Some(existing) = db.get_reservation(&self.options.key)? {
        // Idempotent case: same path+tag already has a reservation
        self.validate_sticky_fields(&existing)?;
        plan = plan.add_action(PlanAction::UpdateLastUsed(self.options.key.clone()));
        return Ok(plan);
    }

    // Step 3: Determine port (manual or automatic allocation)
    let port = if let Some(port) = self.options.port {
        // 🐛 BUG: Manual port - NO VALIDATION!
        port  // Just use it directly
    } else {
        // ✅ Automatic allocation - PROPER VALIDATION
        let allocator = allocator_from_config(self.config)?;
        // ... allocator checks if ports are reserved and occupied
        match allocator.allocate_single(db, &allocation_options, &occupancy_config)? {
            AllocationResult::PreferredUnavailable { port, reason } => {
                match reason {
                    PortUnavailableReason::Reserved => {
                        // Automatic allocation DOES check for reserved ports!
                        return Err(Error::PreferredPortUnavailable { port, reason });
                    }
                    // ...
                }
            }
        }
    };

    // Step 4: Create the reservation with the (unvalidated) port
    let reservation = Reservation::builder(self.options.key.clone(), port)
        .project(self.options.project.clone())
        .task(self.options.task.clone())
        .build()?;

    plan = plan.add_action(PlanAction::CreateReservation(reservation));
    Ok(plan)
}
```

### Why This Happens

The code has **two paths** for determining which port to use:

1. **Manual path** (`if let Some(port) = self.options.port`):
   - Uses port directly
   - Comment says "backward compatibility"
   - **No validation** of any kind
   - Assumes the port is available

2. **Automatic path** (`else { allocator.allocate_single(...) }`):
   - Uses the allocator
   - Allocator checks `db.is_port_reserved()`
   - Allocator checks system occupancy
   - Returns `PortUnavailableReason::Reserved` if port is taken
   - **Proper validation** throughout

The asymmetry exists because manual port specification was likely implemented first as a simple "use what the user asked for" feature, before the more sophisticated automatic allocation with validation was added.

### Step 2 Is Insufficient

The existing check at Step 2 only prevents duplicate reservations **for the same ReservationKey** (path + tag combination). It does NOT prevent:
- Path1 reserving port 8080
- Path2 reserving port 8080

Because Path1 and Path2 have different keys, Step 2's `get_reservation(&self.options.key)` returns `None` for Path2, even though port 8080 is already reserved.

## Suggested Fix Strategy

### Strategy 1: Validate Manual Ports Against Database (Recommended)

Add validation for manually-specified ports similar to automatic allocation:

```rust
// Step 3: Determine port (manual or automatic allocation)
let port = if let Some(port) = self.options.port {
    // Manual port specified - validate it's available
    if !self.options.force && db.is_port_reserved(port)? {
        return Err(Error::PreferredPortUnavailable {
            port,
            reason: PortUnavailableReason::Reserved,
        });
    }
    port
} else {
    // Automatic allocation (unchanged)
    // ...
};
```

**Pros**:
- Minimal change (4 lines added)
- Reuses existing `is_port_reserved()` method
- Reuses existing `Error::PreferredPortUnavailable` error type
- Consistent with automatic allocation behavior
- Respects `--force` flag (existing pattern)

**Cons**:
- Doesn't check system occupancy (only database reservations)
- May want different error message for manual vs automatic

### Strategy 2: Treat Manual Ports as Preferred Ports

Refactor to treat manual port specification as automatic allocation with a strongly-preferred port:

```rust
// Step 3: Determine port
let port = {
    let allocator = allocator_from_config(self.config)?;

    let allocation_options = if let Some(manual_port) = self.options.port {
        // Manual port becomes a preferred port with strict validation
        AllocationOptions {
            preferred: Some(manual_port),
            ignore_occupied: false,  // Don't ignore - we want strict validation
            ignore_exclusions: self.options.ignore_exclusions,
        }
    } else {
        // Standard automatic allocation
        AllocationOptions {
            preferred: self.options.preferred_port,
            ignore_occupied: self.options.ignore_occupied,
            ignore_exclusions: self.options.ignore_exclusions,
        }
    };

    let occupancy_config = self.occupancy_config();

    match allocator.allocate_single(db, &allocation_options, &occupancy_config)? {
        AllocationResult::Allocated(port) => port,
        AllocationResult::PreferredUnavailable { port, reason } => {
            if self.options.port.is_some() {
                // Manual port was unavailable - this is an error
                return Err(Error::PreferredPortUnavailable { port, reason });
            } else {
                // Preferred port unavailable - handle based on flags
                // ... (existing logic)
            }
        }
        // ...
    }
};
```

**Pros**:
- Unifies manual and automatic allocation logic
- Automatically gets all validation (reserved, occupied, excluded)
- More maintainable (single code path)
- Removes special-case handling

**Cons**:
- Larger refactoring
- Changes behavior more significantly
- Might break "backward compatibility" mentioned in current code

### Strategy 3: Add New Validation Method

Create a dedicated validation method that checks all constraints:

```rust
impl ReservePlan<'_> {
    fn validate_manual_port(&self, db: &Database, port: Port) -> Result<()> {
        // Check if port is already reserved (unless --force)
        if !self.options.force && db.is_port_reserved(port)? {
            return Err(Error::ManualPortAlreadyReserved { port });
        }

        // Check if port is in excluded list (unless --ignore-exclusions)
        if !self.options.ignore_exclusions {
            if let Some(ref excluded) = self.config.excluded_ports {
                if port_is_excluded(port, excluded) {
                    return Err(Error::PortExcluded { port });
                }
            }
        }

        // Optionally check system occupancy
        if !self.options.ignore_occupied {
            let occupancy_config = self.occupancy_config();
            if is_port_occupied(port, &occupancy_config)? {
                return Err(Error::PortOccupied { port });
            }
        }

        Ok(())
    }
}

// Then in build_plan():
let port = if let Some(port) = self.options.port {
    self.validate_manual_port(db, port)?;
    port
} else {
    // ... automatic allocation
};
```

**Pros**:
- Clear, explicit validation
- Easy to extend with new checks
- Testable in isolation
- Can provide manual-port-specific error messages

**Cons**:
- More code duplication with allocator logic
- Needs to replicate excluded port checking
- Needs to replicate occupancy checking

## Recommended Implementation

**Use Strategy 1** for the initial fix because:

1. **Minimal risk**: Only 4 lines added, minimal behavior change
2. **Quick fix**: Can be implemented and tested immediately
3. **Solves the bug**: Prevents duplicate port reservations
4. **Respects existing patterns**: Uses `--force` flag consistently
5. **Backward compatible**: Only adds validation, doesn't change allocation

Later, consider refactoring to Strategy 2 for better long-term maintainability.

## Testing Strategy

After implementing the fix, verify:

### 1. Unit Tests (in `trop/src/operations/reserve.rs`)

Add test cases:
```rust
#[test]
fn test_plan_manual_port_already_reserved() {
    let mut db = create_test_database();
    let config = create_test_config();

    // Reserve port 8080 to path1
    let key1 = ReservationKey::new(PathBuf::from("/path1"), None).unwrap();
    let port = Port::try_from(8080).unwrap();
    let res1 = Reservation::builder(key1, port).build().unwrap();
    db.create_reservation(&res1).unwrap();

    // Try to manually reserve same port to path2
    let key2 = ReservationKey::new(PathBuf::from("/path2"), None).unwrap();
    let options = ReserveOptions::new(key2, Some(port))
        .with_allow_unrelated_path(true);

    let result = ReservePlan::new(options, &config).build_plan(&db);

    // Should fail with PreferredPortUnavailable
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        Error::PreferredPortUnavailable { .. }
    ));
}

#[test]
fn test_plan_manual_port_already_reserved_with_force() {
    // Same setup as above, but with force=true
    let options = ReserveOptions::new(key2, Some(port))
        .with_allow_unrelated_path(true)
        .with_force(true);  // Should allow duplicate

    let result = ReservePlan::new(options, &config).build_plan(&db);
    assert!(result.is_ok());  // Should succeed with --force
}
```

### 2. Integration Tests (already exists)

The existing test `test_reserve_preferred_port_already_occupied` should pass after the fix.

### 3. Manual Testing

```bash
# Setup
trop reserve --path /tmp/p1 --port 8080 --allow-unrelated-path

# Should fail
trop reserve --path /tmp/p2 --port 8080 --allow-unrelated-path
# Expected: Error message, non-zero exit

# Should succeed with force
trop reserve --path /tmp/p2 --port 8080 --allow-unrelated-path --force
# Expected: Warning, port 8080 allocated
```

## Related Files

- **Bug location**: `trop/src/operations/reserve.rs:252-256`
- **Database API**: `trop/src/database/operations.rs:457-462` (`is_port_reserved`)
- **Error types**: `trop/src/error.rs` (may need new error variant)
- **Failing test**: `trop-cli/tests/reserve_command.rs:246-287`
- **CLI command**: `trop-cli/src/commands/reserve.rs` (no changes needed for Strategy 1)

## Estimated Effort

- **Strategy 1**: ~30 minutes (code + tests)
- **Strategy 2**: ~2-3 hours (refactoring + comprehensive testing)
- **Strategy 3**: ~1-2 hours (new method + tests)

## Priority

**HIGH** - This bug violates a core system invariant (unique port allocations) and can cause port conflicts that break applications.
