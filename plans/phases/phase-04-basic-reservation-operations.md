# Phase 4: Basic Reservation Operations - Detailed Implementation Plan

## Overview

This document provides a comprehensive implementation plan for Phase 4 of the `trop` port reservation tool. This phase implements the core reservation logic using a plan-execute pattern, including idempotent reserve operations, sticky field protection, path relationship checking, and the force override system.

## Context & Dependencies

### Prerequisites from Previous Phases
- **Phase 1**: Core types (`Port`, `PortRange`, `Reservation`, `ReservationKey`), error hierarchy
- **Phase 2**: Database layer with CRUD operations (`create_reservation`, `get_reservation`, `update_last_used`, `delete_reservation`)
- **Phase 3**: Path handling system (`PathResolver`, `PathRelationship`, normalization/canonicalization)

### Key Existing Infrastructure
- `Database::validate_path_relationship()`: Already checks path relationships with `allow_unrelated` flag
- `Reservation` type with `project`, `task`, and `sticky` fields
- `ReservationKey::with_explicit_path()` and `with_implicit_path()` for proper path resolution
- Comprehensive error types including `StickyFieldChange` and `PathRelationshipViolation`

## Success Criteria

Upon completion of Phase 4:
- Idempotent reserve operations return consistent ports for (path, tag) tuples
- Sticky field changes are detected and blocked without `--force`
- Path relationship rules are enforced (ancestor/descendant allowed, unrelated blocked without flag)
- `last_used_at` timestamps are updated on every reserve operation
- Release operations work correctly
- The plan-execute pattern enables dry-run mode and testing
- Comprehensive unit tests cover all reservation scenarios
- All code passes `cargo fmt` and `cargo clippy`

## Architectural Decisions

### Plan-Execute Pattern
Operations are split into two phases:
1. **Planning Phase**: Analyzes the request, validates constraints, and builds a plan describing what actions to take
2. **Execution Phase**: Takes the plan and performs the actual database operations

This separation enables:
- Dry-run mode (generate plan without executing)
- Robust testing (test plan generation separately from execution)
- Clear error messages (know full operation before starting)
- Transaction safety (can validate entire operation before beginning)

### Sticky Field Protection
The `project` and `task` fields are "sticky" - once set, they cannot be changed without explicit override. This prevents accidental modification of reservation metadata that might indicate a configuration error.

### Force Override Behavior
The `--force` flag acts as a master override, enabling:
- Changes to sticky fields (project/task)
- Operations on unrelated paths
- Any other protection bypasses needed in the future

Finer-grained control is available through specific flags like `--allow-unrelated-path`.

## Task Breakdown

### Task 1: Create Plan Types and Infrastructure

**Objective**: Define the plan-execute pattern types and traits.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/operations/mod.rs`
- `/Users/prb/github/trop/trop/src/operations/plan.rs`

**Implementation Details**:

1. Create the operations module structure in `trop/src/operations/mod.rs`:
   ```rust
   pub mod plan;
   pub mod reserve;
   pub mod release;
   pub mod executor;

   pub use plan::{OperationPlan, PlanAction};
   pub use reserve::{ReservePlan, ReserveOptions};
   pub use release::{ReleasePlan, ReleaseOptions};
   pub use executor::PlanExecutor;
   ```

2. Define core plan types in `operations/plan.rs`:
   ```rust
   /// Describes a single action to be taken
   #[derive(Debug, Clone, PartialEq)]
   pub enum PlanAction {
       CreateReservation(Reservation),
       UpdateReservation(Reservation),
       UpdateLastUsed(ReservationKey),
       DeleteReservation(ReservationKey),
   }

   /// A plan describing a complete operation
   #[derive(Debug, Clone)]
   pub struct OperationPlan {
       pub description: String,
       pub actions: Vec<PlanAction>,
       pub warnings: Vec<String>,
   }
   ```

3. Add plan builder methods:
   - `new(description: String) -> Self`
   - `add_action(mut self, action: PlanAction) -> Self`
   - `add_warning(mut self, warning: String) -> Self`
   - `is_empty() -> bool`

**Verification**:
- Types compile and are well-documented
- Can create and manipulate plans

### Task 2: Implement Reserve Planning Logic

**Objective**: Create the reservation planning system with all validation rules.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/operations/reserve.rs`

**Implementation Details**:

1. Define `ReserveOptions` struct:
   ```rust
   pub struct ReserveOptions {
       pub key: ReservationKey,
       pub project: Option<String>,
       pub task: Option<String>,
       pub port: Option<Port>,  // Preferred port (Phase 4 always requires this)
       pub force: bool,
       pub allow_unrelated_path: bool,
       pub allow_project_change: bool,
       pub allow_task_change: bool,
   }
   ```

2. Implement `ReservePlan`:
   ```rust
   pub struct ReservePlan {
       options: ReserveOptions,
   }

   impl ReservePlan {
       pub fn new(options: ReserveOptions) -> Self { ... }

       pub fn build_plan(&self, db: &Database) -> Result<OperationPlan> {
           // 1. Check path relationship (unless force or allow_unrelated_path)
           // 2. Check for existing reservation
           // 3. If exists: validate sticky fields
           // 4. Build appropriate plan actions
       }
   }
   ```

3. Key planning logic:
   - Path validation:
     ```rust
     if !self.options.force && !self.options.allow_unrelated_path {
         db.validate_path_relationship(&self.options.key.path, false)?;
     }
     ```

   - Existing reservation handling:
     ```rust
     if let Some(existing) = db.get_reservation(&self.options.key)? {
         // Check sticky fields
         if !self.can_change_project(&existing) {
             return Err(Error::StickyFieldChange { ... });
         }
         if !self.can_change_task(&existing) {
             return Err(Error::StickyFieldChange { ... });
         }

         // Return existing port with updated last_used_at
         plan.add_action(PlanAction::UpdateLastUsed(self.options.key.clone()));
         return Ok(plan);
     }
     ```

   - New reservation:
     ```rust
     let reservation = Reservation::builder(
         self.options.key.clone(),
         self.options.port.ok_or(Error::PortUnavailable { ... })?,
     )
     .project(self.options.project.clone())
     .task(self.options.task.clone())
     .build()?;

     plan.add_action(PlanAction::CreateReservation(reservation));
     ```

4. Helper methods:
   - `can_change_project(&self, existing: &Reservation) -> bool`
   - `can_change_task(&self, existing: &Reservation) -> bool`
   - These check force flags and compare values

**Verification**:
- Planning handles all cases: new reservation, existing with same metadata, existing with different metadata
- Sticky field protection works correctly
- Path validation is applied appropriately

### Task 3: Implement Release Planning Logic

**Objective**: Create the release operation planning.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/operations/release.rs`

**Implementation Details**:

1. Define `ReleaseOptions`:
   ```rust
   pub struct ReleaseOptions {
       pub key: ReservationKey,
       pub force: bool,
       pub allow_unrelated_path: bool,
   }
   ```

2. Implement `ReleasePlan`:
   ```rust
   pub struct ReleasePlan {
       options: ReleaseOptions,
   }

   impl ReleasePlan {
       pub fn new(options: ReleaseOptions) -> Self { ... }

       pub fn build_plan(&self, db: &Database) -> Result<OperationPlan> {
           // 1. Check path relationship (unless force or allow_unrelated_path)
           // 2. Check if reservation exists
           // 3. If exists, plan deletion
           // 4. If not, return empty plan (idempotent)
       }
   }
   ```

3. Planning logic:
   ```rust
   // Path validation
   if !self.options.force && !self.options.allow_unrelated_path {
       db.validate_path_relationship(&self.options.key.path, false)?;
   }

   // Check existence
   if db.get_reservation(&self.options.key)?.is_some() {
       plan.add_action(PlanAction::DeleteReservation(self.options.key.clone()));
   } else {
       plan.add_warning("No reservation found to release".into());
   }
   ```

**Verification**:
- Release is idempotent (no error if reservation doesn't exist)
- Path validation works correctly
- Plans are generated correctly for both cases

### Task 4: Create Plan Executor

**Objective**: Implement the execution engine that takes plans and applies them to the database.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/operations/executor.rs`

**Implementation Details**:

1. Define `PlanExecutor`:
   ```rust
   pub struct PlanExecutor<'a> {
       db: &'a mut Database,
       dry_run: bool,
   }

   impl<'a> PlanExecutor<'a> {
       pub fn new(db: &'a mut Database) -> Self {
           Self { db, dry_run: false }
       }

       pub fn dry_run(mut self) -> Self {
           self.dry_run = true;
           self
       }

       pub fn execute(&mut self, plan: &OperationPlan) -> Result<ExecutionResult> {
           if self.dry_run {
               return Ok(ExecutionResult::dry_run(plan));
           }

           // Execute each action
           for action in &plan.actions {
               self.execute_action(action)?;
           }

           Ok(ExecutionResult::success(plan))
       }
   }
   ```

2. Implement action execution:
   ```rust
   fn execute_action(&mut self, action: &PlanAction) -> Result<()> {
       match action {
           PlanAction::CreateReservation(reservation) => {
               self.db.create_reservation(reservation)
           }
           PlanAction::UpdateReservation(reservation) => {
               self.db.create_reservation(reservation) // create_reservation handles updates
           }
           PlanAction::UpdateLastUsed(key) => {
               self.db.update_last_used(key).map(|_| ())
           }
           PlanAction::DeleteReservation(key) => {
               self.db.delete_reservation(key).map(|_| ())
           }
       }
   }
   ```

3. Define `ExecutionResult`:
   ```rust
   pub struct ExecutionResult {
       pub success: bool,
       pub dry_run: bool,
       pub actions_taken: Vec<String>,
       pub warnings: Vec<String>,
   }
   ```

**Verification**:
- Executor correctly applies all action types
- Dry-run mode works without touching database
- Errors are properly propagated

### Task 5: Integrate Plan-Execute Pattern into Library API

**Objective**: Create high-level API functions that use the plan-execute pattern.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/lib.rs`

**Files to Create**:
- `/Users/prb/github/trop/trop/src/operations.rs` (public API wrapper)

**Implementation Details**:

1. Add operations module to lib.rs:
   ```rust
   pub mod operations;
   ```

2. Create public API in `operations.rs`:
   ```rust
   use crate::database::Database;
   use crate::operations::{ReservePlan, ReserveOptions, ReleasePlan, ReleaseOptions, PlanExecutor};

   /// Reserve a port for the given path and tag
   pub fn reserve(
       db: &mut Database,
       options: ReserveOptions,
   ) -> Result<Port> {
       let plan = ReservePlan::new(options).build_plan(db)?;
       let executor = PlanExecutor::new(db);
       let result = executor.execute(&plan)?;

       // Extract the port from the plan
       // This requires examining the plan actions
       extract_port_from_plan(&plan)
   }

   /// Release a reservation
   pub fn release(
       db: &mut Database,
       options: ReleaseOptions,
   ) -> Result<()> {
       let plan = ReleasePlan::new(options).build_plan(db)?;
       let executor = PlanExecutor::new(db);
       executor.execute(&plan)?;
       Ok(())
   }

   /// Generate a plan without executing (for dry-run)
   pub fn plan_reserve(
       db: &Database,
       options: ReserveOptions,
   ) -> Result<OperationPlan> {
       ReservePlan::new(options).build_plan(db)
   }

   /// Generate a plan without executing (for dry-run)
   pub fn plan_release(
       db: &Database,
       options: ReleaseOptions,
   ) -> Result<OperationPlan> {
       ReleasePlan::new(options).build_plan(db)
   }
   ```

**Verification**:
- Public API is clean and easy to use
- Both planning and execution paths work
- Documentation is clear

### Task 6: Add Idempotency Tests

**Objective**: Ensure reserve operations are truly idempotent.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/idempotency.rs`

**Implementation Details**:

1. Test repeated reservations return same port:
   ```rust
   #[test]
   fn reserve_is_idempotent() {
       let mut db = create_test_database();
       let key = ReservationKey::new(PathBuf::from("/test"), None).unwrap();
       let port = Port::try_from(5000).unwrap();

       let options = ReserveOptions {
           key: key.clone(),
           port: Some(port),
           project: Some("test-project".into()),
           task: None,
           force: false,
           allow_unrelated_path: true, // Testing in temp dir
           allow_project_change: false,
           allow_task_change: false,
       };

       // First reservation
       let port1 = reserve(&mut db, options.clone()).unwrap();
       assert_eq!(port1, port);

       // Second reservation - should return same port
       let port2 = reserve(&mut db, options.clone()).unwrap();
       assert_eq!(port2, port1);

       // Verify only one reservation exists
       let all = db.list_all_reservations().unwrap();
       assert_eq!(all.len(), 1);
   }
   ```

2. Test timestamp updates:
   ```rust
   #[test]
   fn reserve_updates_last_used() {
       // Similar setup
       // Sleep between reservations
       // Verify last_used_at is updated
   }
   ```

**Verification**:
- Multiple calls with same parameters return same port
- Database contains only one reservation
- Timestamps are updated appropriately

### Task 7: Add Sticky Field Protection Tests

**Objective**: Verify sticky field protection works correctly.

**Files to Modify**:
- `/Users/prb/github/trop/trop/tests/idempotency.rs` (or create new test file)

**Implementation Details**:

1. Test project field protection:
   ```rust
   #[test]
   fn cannot_change_project_without_force() {
       let mut db = create_test_database();
       let key = ReservationKey::new(PathBuf::from("/test"), None).unwrap();

       // Initial reservation with project
       let options1 = ReserveOptions {
           key: key.clone(),
           port: Some(Port::try_from(5000).unwrap()),
           project: Some("project1".into()),
           // ...
       };
       reserve(&mut db, options1).unwrap();

       // Try to change project
       let options2 = ReserveOptions {
           key: key.clone(),
           port: Some(Port::try_from(5000).unwrap()),
           project: Some("project2".into()),
           force: false,
           // ...
       };

       let result = reserve(&mut db, options2);
       assert!(matches!(result, Err(Error::StickyFieldChange { .. })));
   }

   #[test]
   fn can_change_project_with_force() {
       // Similar but with force: true
       // Should succeed
   }

   #[test]
   fn can_change_project_with_allow_flag() {
       // Similar but with allow_project_change: true
       // Should succeed
   }
   ```

2. Test task field protection (similar pattern)

3. Test combinations:
   - Both fields changing
   - One field changing, one staying same
   - Null to value transitions
   - Value to null transitions

**Verification**:
- Changes are blocked without appropriate flags
- Force flag overrides all protections
- Specific allow flags work correctly
- Error messages are clear

### Task 8: Add Path Relationship Tests

**Objective**: Verify path relationship checking in reservation operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/path_validation.rs`

**Implementation Details**:

1. Test ancestor/descendant paths allowed:
   ```rust
   #[test]
   fn can_reserve_ancestor_path() {
       let mut db = create_test_database();
       let cwd = env::current_dir().unwrap();
       let parent = cwd.parent().unwrap();

       let key = ReservationKey::with_explicit_path(parent, None).unwrap();
       let options = ReserveOptions {
           key,
           port: Some(Port::try_from(5000).unwrap()),
           force: false,
           allow_unrelated_path: false,
           // ...
       };

       // Should succeed - ancestor is allowed
       let result = reserve(&mut db, options);
       assert!(result.is_ok());
   }
   ```

2. Test unrelated paths blocked:
   ```rust
   #[test]
   fn cannot_reserve_unrelated_path_without_flag() {
       let mut db = create_test_database();

       // Create definitely unrelated path
       let key = ReservationKey::with_explicit_path("/some/random/path", None).unwrap();
       let options = ReserveOptions {
           key,
           port: Some(Port::try_from(5000).unwrap()),
           force: false,
           allow_unrelated_path: false,
           // ...
       };

       let result = reserve(&mut db, options);
       assert!(matches!(result, Err(Error::PathRelationshipViolation { .. })));
   }
   ```

3. Test override mechanisms:
   - `allow_unrelated_path: true` allows unrelated
   - `force: true` allows unrelated

**Verification**:
- Hierarchical relationships work correctly
- Unrelated paths are blocked by default
- Override flags work as expected

### Task 9: Add Release Operation Tests

**Objective**: Test release operations thoroughly.

**Files to Modify**:
- `/Users/prb/github/trop/trop/tests/idempotency.rs` (or new file)

**Implementation Details**:

1. Test basic release:
   ```rust
   #[test]
   fn release_removes_reservation() {
       let mut db = create_test_database();

       // Create reservation
       let key = ReservationKey::new(PathBuf::from("/test"), None).unwrap();
       // ... reserve it

       // Release it
       let release_opts = ReleaseOptions {
           key: key.clone(),
           force: false,
           allow_unrelated_path: true,
       };
       release(&mut db, release_opts).unwrap();

       // Verify it's gone
       let result = db.get_reservation(&key).unwrap();
       assert!(result.is_none());
   }
   ```

2. Test idempotent release:
   ```rust
   #[test]
   fn release_is_idempotent() {
       // Release twice - second should succeed with no error
   }
   ```

3. Test path validation in release:
   - Similar to reserve tests
   - Verify unrelated paths need override

**Verification**:
- Release removes reservations
- Multiple releases don't error
- Path validation applies to release

### Task 10: Add Plan Generation Tests

**Objective**: Test the plan generation separately from execution.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/planning.rs`

**Implementation Details**:

1. Test reserve plan generation:
   ```rust
   #[test]
   fn reserve_generates_correct_plan_for_new() {
       let db = create_test_database();
       let key = ReservationKey::new(PathBuf::from("/test"), None).unwrap();

       let options = ReserveOptions {
           key,
           port: Some(Port::try_from(5000).unwrap()),
           // ...
       };

       let plan = plan_reserve(&db, options).unwrap();

       assert_eq!(plan.actions.len(), 1);
       assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));
   }

   #[test]
   fn reserve_generates_correct_plan_for_existing() {
       // Create existing reservation
       // Generate plan for same key
       // Should be UpdateLastUsed action
   }
   ```

2. Test dry-run execution:
   ```rust
   #[test]
   fn dry_run_does_not_modify_database() {
       let mut db = create_test_database();

       // Generate plan
       let plan = // ...

       // Execute in dry-run
       let executor = PlanExecutor::new(&mut db).dry_run();
       let result = executor.execute(&plan).unwrap();

       assert!(result.dry_run);
       // Verify database unchanged
   }
   ```

**Verification**:
- Plans accurately reflect operations
- Dry-run mode doesn't modify database
- Plan descriptions are helpful

## Integration Points

### With Database Module
- Uses existing CRUD operations
- Leverages `validate_path_relationship()`
- All database operations go through `Database` type

### With Path Module
- Uses `ReservationKey::with_explicit_path()` for user-provided paths
- Uses `ReservationKey::with_implicit_path()` for inferred paths
- Relies on `PathRelationship` for validation

### With Error Module
- Uses `StickyFieldChange` for sticky field violations
- Uses `PathRelationshipViolation` for path issues
- All errors properly propagate through `Result<T>`

## Testing Strategy

### Unit Tests
- Each operation type has comprehensive unit tests
- Test both success and failure cases
- Test edge cases (empty strings, null values, etc.)
- Use property-based testing for validation logic

### Integration Tests
- Test full reserve/release cycles
- Test concurrent operations (in later phases)
- Test with real database operations
- Test error recovery scenarios

### Plan-Execute Tests
- Test plan generation independently
- Test execution independently
- Test that plans accurately describe operations
- Verify dry-run mode

## Risk Mitigations

### Backward Compatibility
Since we're still in early phases, we can modify the `Reservation` type if needed. However, we should avoid breaking changes to types defined in Phase 1-3.

### Performance
The plan-execute pattern adds a small overhead, but provides significant benefits in testing and debugging. The overhead is negligible compared to database operations.

### Complexity
The plan-execute pattern might seem overengineered for simple operations, but it will pay dividends when we add:
- Group reservations (Phase 8)
- Migration operations (Phase 11)
- Complex cleanup operations (Phase 9)

## Validation Checklist

Before considering Phase 4 complete:

- [ ] Reserve operations are idempotent
- [ ] Sticky fields are protected correctly
- [ ] Path relationships are validated
- [ ] Force flag overrides all protections
- [ ] Release operations work correctly
- [ ] Plan generation is accurate
- [ ] Dry-run mode works
- [ ] All tests pass
- [ ] Code is documented
- [ ] No clippy warnings

## Next Phase Preparation

Phase 5 (Configuration System) will need:
- Options structs that can be populated from config
- Plan types that can incorporate config-derived defaults
- Validation that works with hierarchical configuration

Ensure our types are designed to be easily populated from configuration files.

## Notes for Implementer

### Design Principles
- Make illegal states unrepresentable
- Validate early, execute confidently
- Prefer explicit options over implicit behavior
- Keep plans immutable once created

### Error Handling
- Return specific error types
- Include context in error messages
- Distinguish between user errors and system errors
- Make errors actionable

### Documentation
- Document why sticky fields exist
- Explain path relationship rules
- Provide examples of force flag usage
- Document idempotency guarantees

### Testing
- Test the "happy path" and error cases equally
- Use realistic test data
- Test combinations of flags
- Verify error messages are helpful

This plan provides a complete blueprint for implementing Phase 4's reservation operations with the plan-execute pattern, comprehensive validation, and robust testing.