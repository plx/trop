# Phase 11: Migration System and Advanced Operations

## Overview

Phase 11 completes the trop implementation by adding database migration infrastructure and the remaining advanced CLI commands. This phase focuses on two major subsystems:

1. **Database Migration System**: Robust schema versioning and migration infrastructure for future evolution
2. **Advanced CLI Operations**: The remaining commands including `migrate`, `list-projects`, and `init`

## Context and Prerequisites

- **Completed**: Phases 1-10 provide the foundation including database layer, reservations, configuration, port allocation, and essential commands
- **Current State**: Database has basic schema versioning (v1) but no migration infrastructure
- **Key Files**:
  - `trop/src/database/migrations.rs` - existing migration stub
  - `trop/src/database/schema.rs` - current schema definitions
  - `trop-cli/src/commands/` - existing command implementations

## Part 1: Database Migration System

### 1.1 Migration Infrastructure

**Goal**: Create a robust migration system capable of evolving the database schema over time while maintaining backward compatibility.

#### Data Structures

```rust
// trop/src/database/migrations/types.rs

/// Represents a single schema migration
pub struct Migration {
    /// Version this migration upgrades TO
    pub version: i32,
    /// Human-readable description
    pub description: String,
    /// SQL statements to apply the migration
    pub up_sql: Vec<String>,
    /// Optional SQL to rollback (for development only)
    pub down_sql: Option<Vec<String>>,
    /// Validation function to verify migration success
    pub validate: Box<dyn Fn(&Connection) -> Result<()>>,
}

/// Migration execution plan
pub struct MigrationPlan {
    pub current_version: i32,
    pub target_version: i32,
    pub migrations: Vec<Migration>,
    pub dry_run: bool,
}

/// Migration execution result
pub struct MigrationResult {
    pub from_version: i32,
    pub to_version: i32,
    pub migrations_applied: Vec<i32>,
    pub duration_ms: u64,
}
```

#### Module Structure

```
trop/src/database/migrations/
├── mod.rs           # Public API and coordination
├── types.rs         # Migration data structures
├── registry.rs      # Migration registry and version mapping
├── executor.rs      # Migration execution engine
├── validator.rs     # Pre/post migration validation
└── v1_to_v2.rs     # Example migration module (for testing)
```

### 1.2 Migration Registry

**Purpose**: Central registry of all migrations with version ordering and dependency tracking.

```rust
// trop/src/database/migrations/registry.rs

pub struct MigrationRegistry {
    migrations: BTreeMap<i32, Migration>,
}

impl MigrationRegistry {
    /// Build the registry with all known migrations
    pub fn new() -> Self {
        let mut registry = Self::default();

        // Register all migrations here
        // For now, empty - but structure ready for future migrations
        // Example:
        // registry.register(v2::migration());
        // registry.register(v3::migration());

        registry
    }

    /// Get migrations needed to go from current to target version
    pub fn plan_migration(&self, from: i32, to: i32) -> Result<Vec<&Migration>> {
        // Validate version range
        // Build ordered list of migrations
        // Return error if any migration missing in chain
    }
}
```

### 1.3 Migration Executor

**Purpose**: Transactional execution of migration plans with rollback capability.

```rust
// trop/src/database/migrations/executor.rs

pub struct MigrationExecutor<'a> {
    conn: &'a mut Connection,
    registry: &'a MigrationRegistry,
}

impl<'a> MigrationExecutor<'a> {
    /// Execute a migration plan
    pub fn execute(&mut self, plan: MigrationPlan) -> Result<MigrationResult> {
        // 1. Begin transaction (EXCLUSIVE mode)
        // 2. Verify current version matches expected
        // 3. For each migration in plan:
        //    a. Execute up_sql statements
        //    b. Update schema_version
        //    c. Run validation
        // 4. Commit transaction
        // 5. Return result with timing info
    }

    /// Dry run - validate without executing
    pub fn validate_plan(&self, plan: &MigrationPlan) -> Result<()> {
        // Check migrations exist
        // Verify version continuity
        // Run pre-migration validation
    }
}
```

### 1.4 Migration Testing Strategy

**Critical**: Migration code must be thoroughly tested as it handles irreversible schema changes.

```rust
// trop/src/database/migrations/test_util.rs

/// Create test database at specific version
pub fn create_test_db_at_version(version: i32) -> Connection {
    // Create in-memory database
    // Apply migrations up to specified version
    // Return connection
}

/// Test migration roundtrip (up and down)
pub fn test_migration_roundtrip(migration: &Migration) {
    // Apply up migration
    // Verify schema changes
    // Apply down migration (if available)
    // Verify restoration
}
```

**Test Cases**:
- Migration from v0 (uninitialized) to v1
- Future migration from v1 to v2 (with test migration)
- Attempting migration with incompatible version
- Transaction rollback on migration failure
- Concurrent migration attempts (busy timeout)
- Migration with data preservation

### 1.5 Integration Points

**Modifications to existing code**:

1. **`database/connection.rs`**:
   - Add `migrate_to_latest()` method
   - Enhance `open()` to handle migration on connection

2. **`database/schema.rs`**:
   - Move current schema definition to `migrations/v1.rs`
   - Make schema module coordinate with migration system

3. **Error handling**:
   - Add `MigrationError` variant with detailed context
   - Include version information in error messages

## Part 2: Advanced CLI Commands

### 2.1 `trop migrate` Command

**Purpose**: Move reservations between paths, preserving port assignments.

#### Implementation Structure

```rust
// trop/src/operations/migrate.rs

pub struct MigratePlan {
    pub from_path: PathBuf,
    pub to_path: PathBuf,
    pub recursive: bool,
    pub force: bool,
    pub affected_reservations: Vec<ReservationKey>,
    pub conflicts: Vec<ReservationKey>,
}

impl MigratePlan {
    pub fn new(from: PathBuf, to: PathBuf, recursive: bool, force: bool) -> Result<Self> {
        // 1. Validate paths (normalize, check relationships)
        // 2. Find affected reservations
        // 3. Check for conflicts at destination
        // 4. Build plan
    }

    pub fn validate(&self, db: &Database) -> Result<()> {
        // Check source has reservations (unless recursive)
        // Verify destination path doesn't conflict
        // Ensure no data loss without --force
    }
}

pub struct MigrateExecutor;

impl MigrateExecutor {
    pub fn execute(plan: &MigratePlan, db: &mut Database) -> Result<MigrateResult> {
        // Transaction:
        // 1. Delete conflicts if --force
        // 2. Update all affected reservation paths
        // 3. Maintain all other fields (port, project, task, timestamps)
        // 4. Return summary
    }
}
```

#### CLI Command

```rust
// trop-cli/src/commands/migrate.rs

#[derive(Parser)]
pub struct MigrateCommand {
    /// Source path to migrate from
    #[arg(long, value_name = "PATH")]
    from: PathBuf,

    /// Destination path to migrate to
    #[arg(long, value_name = "PATH")]
    to: PathBuf,

    /// Migrate all sub-paths recursively
    #[arg(long)]
    recursive: bool,

    /// Force migration, overwriting existing reservations
    #[arg(long)]
    force: bool,

    /// Preview changes without applying them
    #[arg(long)]
    dry_run: bool,
}
```

**Key Behaviors**:
- Non-recursive fails if no reservation at exact source path
- Recursive succeeds even with no reservations (no-op)
- All migrations are transactional
- Preserves all metadata (only path changes)
- Tags migrate with their paths

### 2.2 `trop list-projects` Command

**Purpose**: List all unique project identifiers in the database.

#### Implementation

```rust
// trop/src/operations/list.rs

pub fn list_projects(db: &Database) -> Result<Vec<String>> {
    // Query distinct non-null project values
    // Sort alphabetically
    // Return list
}
```

#### CLI Command

```rust
// trop-cli/src/commands/list_projects.rs

#[derive(Parser)]
pub struct ListProjectsCommand {
    // Future: add format options, filters, etc.
}

impl ListProjectsCommand {
    pub fn execute(&self, context: &mut CommandContext) -> Result<()> {
        // Get projects from database
        // Print one per line to stdout
        // Simple format for now, extensible later
    }
}
```

### 2.3 `trop init` Command

**Purpose**: Explicitly initialize the data directory and database.

#### Implementation

```rust
// trop/src/operations/init.rs

pub struct InitPlan {
    pub data_dir: PathBuf,
    pub overwrite: bool,
    pub create_config: bool,
}

pub fn init_database(plan: &InitPlan) -> Result<InitResult> {
    // 1. Create data directory if needed
    // 2. Create/open database file
    // 3. Initialize schema if needed
    // 4. Optionally create default config.yaml
    // 5. Return summary of actions taken
}
```

#### CLI Command

```rust
// trop-cli/src/commands/init.rs

#[derive(Parser)]
pub struct InitCommand {
    /// Data directory to initialize
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Overwrite existing files
    #[arg(long)]
    overwrite: bool,

    /// Create default configuration file
    #[arg(long)]
    with_config: bool,

    /// Preview changes without applying them
    #[arg(long)]
    dry_run: bool,
}
```

**Special Behaviors**:
- Does NOT accept `--disable-autoinit` (would be paradoxical)
- `--data-dir` has different meaning (where to create, not where to find)
- Useful for CI/CD and containerized environments

### 2.4 Additional Enhancements

#### Project/Task Inference

**Location**: `trop/src/operations/inference.rs`

```rust
use gix::discover::upwards;

pub fn infer_project(path: &Path) -> Option<String> {
    // Use gix to discover git repository
    // Extract repository name from .git path
    // Handle worktrees vs regular repos
}

pub fn infer_task(path: &Path) -> Option<String> {
    // In worktree: use worktree directory name
    // In regular repo: use current branch name
    // Otherwise: None
}
```

#### Path Relationship Flags

**Enhance existing commands** to support:
- `--allow-unrelated-path`: Allow operations on paths outside current hierarchy
- `--allow-change-project`: Allow modifying project field
- `--allow-change-task`: Allow modifying task field
- `--allow-change`: Shorthand for both project and task

Implementation in `operations/reserve.rs` and related modules.

## Part 3: Testing Requirements

### 3.1 Migration System Tests

**Integration Tests** (`tests/migration_integration.rs`):
- Fresh database initialization
- Migration from older version to current
- Migration with existing data preservation
- Rollback on failure scenarios
- Concurrent migration attempts

**Unit Tests**:
- Registry: migration ordering, version validation
- Executor: transaction handling, validation
- Each future migration module

### 3.2 CLI Command Tests

**Integration Tests** (`trop-cli/tests/`):

1. **`migrate` command**:
   - Simple path migration
   - Recursive migration with sub-paths
   - Conflict handling with/without --force
   - Dry-run mode
   - Error cases (missing source, invalid paths)

2. **`list-projects` command**:
   - Empty database
   - Multiple projects
   - Null project handling

3. **`init` command**:
   - Fresh initialization
   - Existing directory handling
   - Overwrite behavior
   - Config file creation

### 3.3 Property-Based Tests

Using `proptest` or `quickcheck`:
- Path migration preserves all non-path fields
- Migration + reverse migration = identity
- Registry always returns valid migration chains

## Part 4: Implementation Order

### Week 1: Migration Infrastructure
1. Create migration module structure
2. Implement registry and types
3. Build executor with transaction support
4. Add validation framework
5. Create test migration (v1 to v2)
6. Comprehensive testing

### Week 2: CLI Commands - Part 1
1. Implement `migrate` command:
   - Path validation and normalization
   - Conflict detection
   - Transactional execution
   - CLI integration
2. Implement `list-projects` command:
   - Database query
   - Output formatting
   - CLI integration

### Week 3: CLI Commands - Part 2
1. Implement `init` command:
   - Directory creation
   - Database initialization
   - Config file template
2. Add project/task inference using gix:
   - Git repository detection
   - Worktree handling
   - Branch name extraction
3. Implement `--allow-*` flags:
   - Enhance validation logic
   - Update existing commands

### Week 4: Integration and Polish
1. End-to-end testing of all new features
2. Performance optimization (migration speed)
3. Documentation updates
4. Error message refinement
5. Logging enhancements

## Part 5: Risk Mitigation

### Critical Risks

1. **Data Loss During Migration**
   - Mitigation: All operations in transactions
   - Validation before and after migration
   - Comprehensive backup recommendation in docs

2. **Schema Version Conflicts**
   - Mitigation: Strict version checking
   - Clear error messages about version mismatch
   - Never attempt partial migrations

3. **Path Normalization Edge Cases**
   - Mitigation: Extensive testing with symlinks, relative paths
   - Clear documentation of path handling
   - Validation at every layer

### Performance Considerations

1. **Large-Scale Migrations**
   - Use SQL bulk updates where possible
   - Index on path column for efficient queries
   - Progress reporting for long operations

2. **Migration Execution Speed**
   - Pre-compile migration SQL
   - Minimize validation overhead
   - Use prepared statements

## Part 6: Documentation Requirements

### User Documentation
- Migration command examples and use cases
- When to use `init` vs auto-initialization
- Project/task inference behavior
- Troubleshooting guide for migrations

### Developer Documentation
- Migration system architecture
- How to add new migrations
- Testing migration changes
- Schema evolution guidelines

## Deliverables Checklist

- [ ] Migration infrastructure with registry and executor
- [ ] Test migration demonstrating system capabilities
- [ ] `trop migrate` command with recursive support
- [ ] `trop list-projects` command
- [ ] `trop init` command for explicit initialization
- [ ] Git-based project/task inference
- [ ] `--allow-*` flag implementations
- [ ] Comprehensive test coverage (>90%)
- [ ] Performance benchmarks for migrations
- [ ] Updated documentation

## Success Criteria

1. **Robustness**: Zero data loss in migration operations
2. **Performance**: Migrations complete in <100ms for typical datasets
3. **Usability**: Clear error messages and recovery paths
4. **Compatibility**: Smooth upgrade path for existing databases
5. **Testing**: All edge cases covered with tests
6. **Documentation**: Complete examples for all new commands

## Notes for Implementation

- Start with migration system as it's foundational
- Use plan-execute pattern consistently
- Ensure all operations are transactional
- Focus on error handling and recovery
- Consider future extensibility in design decisions
- Coordinate with existing team members on integration points

This plan provides a solid foundation for implementing Phase 11. The migration system will ensure the project can evolve safely over time, while the advanced commands complete the full feature set defined in the specification.