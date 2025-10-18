# Phase 11: Advanced Operations

## Overview

Phase 11 completes the trop implementation by adding the remaining advanced features and commands as specified in the authoritative `reference/ImplementationPlan.md`. This phase focuses on filling the gaps in the current implementation:

1. **Path Migration**: `migrate` command for moving reservations between paths
2. **Project Listing**: `list-projects` command to enumerate unique projects
3. **Explicit Initialization**: `init` command for non-auto initialization scenarios
4. **Git Integration**: Project/task inference using gix library
5. **Permission Flags**: `--allow-*` flag variations for reservation operations
6. **Recursive Release**: Already implemented, needs verification
7. **Path Canonicalization**: Already implemented for implicit paths, needs verification

## Context and Prerequisites

- **Completed**: Phases 1-10 provide full foundation including database, reservations, configuration, port allocation, cleanup, and utility commands
- **Current State**:
  - Database at schema v1 with basic versioning support
  - Recursive release already implemented (`--recursive` flag on release command)
  - Path canonicalization already implemented in `PathResolver` for implicit paths
  - Some `--allow-*` flags already exist but may need extension
- **Key Integration Points**:
  - `trop/src/operations/` - existing plan-execute pattern modules
  - `trop-cli/src/commands/` - existing command implementations
  - `trop/src/path/` - path resolution and canonicalization

## Part 1: `migrate` Command

### 1.1 Operation Module

**Purpose**: Move reservations from one path to another, preserving all metadata.

#### Data Structures

```rust
// trop/src/operations/migrate.rs

use std::path::PathBuf;
use crate::{Database, ReservationKey, Reservation, Result};

/// Options for migration operation
#[derive(Debug, Clone)]
pub struct MigrateOptions {
    pub from_path: PathBuf,
    pub to_path: PathBuf,
    pub recursive: bool,
    pub force: bool,
    pub dry_run: bool,
}

/// Migration plan describing what will be moved
#[derive(Debug)]
pub struct MigratePlan {
    pub options: MigrateOptions,
    pub migrations: Vec<MigrationItem>,
    pub conflicts: Vec<ReservationKey>,
}

/// Single item to migrate
#[derive(Debug)]
pub struct MigrationItem {
    pub from_key: ReservationKey,
    pub to_key: ReservationKey,
    pub reservation: Reservation,
}

/// Result of migration execution
#[derive(Debug)]
pub struct MigrateResult {
    pub migrated_count: usize,
    pub conflicts_resolved: usize,
    pub from_path: PathBuf,
    pub to_path: PathBuf,
}
```

#### Implementation Strategy

```rust
impl MigratePlan {
    pub fn new(options: MigrateOptions) -> Self { ... }

    pub fn build_plan(&mut self, db: &Database) -> Result<()> {
        // 1. Normalize both paths (but don't canonicalize - preserve as given)
        // 2. Find all reservations at from_path (exact or recursive)
        // 3. Check for conflicts at to_path
        // 4. Build migration items with new paths
        // 5. Return error if conflicts exist without --force
    }
}

impl PlanExecutor {
    pub fn execute_migrate(&mut self, plan: &MigratePlan) -> Result<MigrateResult> {
        // Transaction:
        // 1. If force: delete all conflicts
        // 2. For each migration item:
        //    a. Delete old reservation
        //    b. Insert with new path, preserving ALL other fields
        // 3. Commit and return result
    }
}
```

**Key Behaviors**:
- Preserves all metadata: port, project, task, timestamps, tags
- Non-recursive fails if no reservation at exact source
- Recursive succeeds even with no reservations (no-op)
- Warning if destination doesn't exist (but proceed)
- All-or-nothing transactional execution

### 1.2 CLI Command

```rust
// trop-cli/src/commands/migrate.rs

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Migrate reservations between paths")]
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

    /// Overwrite existing reservations at destination
    #[arg(long)]
    force: bool,

    /// Preview changes without applying them
    #[arg(long)]
    dry_run: bool,
}

impl MigrateCommand {
    pub fn execute(&self, global: &GlobalOptions, context: &mut CommandContext) -> Result<()> {
        // 1. Build migrate options
        // 2. Create plan and build it
        // 3. Display what would be migrated
        // 4. If not dry-run: execute
        // 5. Report results
    }
}
```

## Part 2: `list-projects` Command

### 2.1 Operation Module

```rust
// trop/src/operations/list.rs (extend existing)

impl Database {
    /// Get all unique project identifiers.
    pub fn list_projects(&self) -> Result<Vec<String>> {
        let query = "SELECT DISTINCT project FROM reservations
                     WHERE project IS NOT NULL
                     ORDER BY project";
        // Execute query and return sorted list
    }
}
```

### 2.2 CLI Command

```rust
// trop-cli/src/commands/list_projects.rs

use clap::Parser;

#[derive(Parser)]
#[command(about = "List all unique project identifiers")]
pub struct ListProjectsCommand {
    // Future: could add format options, filters
}

impl ListProjectsCommand {
    pub fn execute(&self, global: &GlobalOptions, context: &mut CommandContext) -> Result<()> {
        let config = load_config(global)?;
        let db = open_database(global, &config)?;

        let projects = db.list_projects()?;

        // Output one per line to stdout
        for project in projects {
            println!("{}", project);
        }

        Ok(())
    }
}
```

## Part 3: `init` Command

### 3.1 Operation Module

```rust
// trop/src/operations/init.rs

use std::path::PathBuf;
use crate::{Database, Result};

pub struct InitOptions {
    pub data_dir: PathBuf,
    pub overwrite: bool,
    pub create_config: bool,
}

pub struct InitResult {
    pub data_dir_created: bool,
    pub database_created: bool,
    pub config_created: bool,
    pub data_dir: PathBuf,
}

pub fn init_database(options: InitOptions) -> Result<InitResult> {
    // 1. Create data directory if needed
    // 2. Check if database exists
    // 3. Create/initialize database if needed or overwrite
    // 4. Optionally create default trop.yaml
    // 5. Return summary of actions
}
```

### 3.2 CLI Command

```rust
// trop-cli/src/commands/init.rs

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Initialize trop data directory and database")]
pub struct InitCommand {
    /// Data directory to initialize
    #[arg(long, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Overwrite existing database
    #[arg(long)]
    overwrite: bool,

    /// Create default configuration file
    #[arg(long)]
    with_config: bool,

    /// Preview actions without executing
    #[arg(long)]
    dry_run: bool,
}

impl InitCommand {
    pub fn execute(&self, global: &GlobalOptions) -> Result<()> {
        // NOTE: Does NOT accept --disable-autoinit (would be paradoxical)
        // NOTE: --data-dir has different meaning here (where to create, not where to find)

        let data_dir = self.data_dir.clone()
            .or_else(|| global.data_dir.clone())
            .unwrap_or_else(|| default_data_dir());

        // Show what would be done in dry-run mode
        // Otherwise execute initialization
        // Report what was created
    }
}
```

## Part 4: Git Integration for Project/Task Inference

### 4.1 Add gix Dependency

```toml
# trop/Cargo.toml
[dependencies]
gix = { version = "0.68", default-features = false, features = ["revision"] }
```

### 4.2 Inference Module

```rust
// trop/src/operations/inference.rs

use std::path::Path;
use gix::ThreadSafeRepository;

/// Infer project name from git repository.
///
/// Returns the repository name extracted from the git directory.
pub fn infer_project(path: &Path) -> Option<String> {
    // Use gix to discover repository
    let repo = gix::discover::upwards(path).ok()?;

    // Extract repository name from path
    // Handle both regular repos and worktrees
    let repo_path = repo.path();

    // Get parent directory name as project
    repo_path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// Infer task from git context.
///
/// - In worktree: use worktree directory name
/// - In regular repo: use current branch name
/// - Otherwise: None
pub fn infer_task(path: &Path) -> Option<String> {
    let repo = gix::discover::upwards(path).ok()?;

    // Check if this is a worktree
    if is_worktree(&repo) {
        // Use worktree directory name
        extract_worktree_name(&repo)
    } else {
        // Use current branch name
        get_current_branch(&repo)
    }
}

fn is_worktree(repo: &ThreadSafeRepository) -> bool {
    // Check if .git is a file (indicates worktree)
    repo.work_dir()
        .and_then(|wd| wd.join(".git").metadata().ok())
        .map(|m| m.is_file())
        .unwrap_or(false)
}

fn extract_worktree_name(repo: &ThreadSafeRepository) -> Option<String> {
    repo.work_dir()
        .and_then(|wd| wd.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

fn get_current_branch(repo: &ThreadSafeRepository) -> Option<String> {
    repo.to_thread_local()
        .head_name()
        .ok()
        .flatten()
        .and_then(|name| {
            // Extract branch name from refs/heads/...
            name.as_bstr()
                .to_str()
                .ok()
                .and_then(|s| s.strip_prefix("refs/heads/"))
                .map(|s| s.to_string())
        })
}
```

### 4.3 Integration with Reserve Operations

Modify `trop/src/operations/reserve.rs` to use inference when project/task not provided:

```rust
impl ReserveOptions {
    pub fn with_git_inference(&mut self, path: &Path) -> &mut Self {
        // Only infer if not explicitly provided
        if self.project.is_none() {
            self.project = infer_project(path);
        }
        if self.task.is_none() {
            self.task = infer_task(path);
        }
        self
    }
}
```

## Part 5: Enhanced `--allow-*` Flags

### 5.1 Extend Existing Operations

The current implementation already has some `--allow-*` flags. Ensure comprehensive support:

```rust
// trop/src/operations/reserve.rs (verify/extend existing)

pub struct ReserveOptions {
    // ... existing fields ...

    /// Allow operations on unrelated paths
    pub allow_unrelated_path: bool,

    /// Allow changing the project field
    pub allow_change_project: bool,

    /// Allow changing the task field
    pub allow_change_task: bool,
}

impl ReserveOptions {
    /// Shorthand for allowing both project and task changes
    pub fn with_allow_change(&mut self, allow: bool) -> &mut Self {
        self.allow_change_project = allow;
        self.allow_change_task = allow;
        self
    }
}
```

### 5.2 Update CLI Commands

Ensure all relevant commands support these flags:

```rust
// trop-cli/src/commands/reserve.rs (verify/extend)

#[derive(Parser)]
pub struct ReserveCommand {
    // ... existing fields ...

    /// Allow operations on unrelated paths
    #[arg(long)]
    allow_unrelated_path: bool,

    /// Allow changing the project field
    #[arg(long)]
    allow_change_project: bool,

    /// Allow changing the task field
    #[arg(long)]
    allow_change_task: bool,

    /// Allow changing both project and task fields
    #[arg(long)]
    allow_change: bool,
}
```

## Part 6: Verification of Existing Features

### 6.1 Recursive Release

**Already Implemented**: The `--recursive` flag on the release command is fully functional. Verification tasks:

- Review `trop-cli/src/commands/release.rs` for correctness
- Ensure tests in `trop-cli/tests/release_command.rs` are comprehensive
- No additional implementation needed

### 6.2 Path Canonicalization for Implicit Paths

**Already Implemented**: The `PathResolver` properly canonicalizes implicit paths. Verification tasks:

- Review `trop/src/path/resolver.rs` implementation
- Confirm implicit paths (CWD, inferred) are canonicalized
- Confirm explicit paths (CLI args) are NOT canonicalized
- Tests exist in `trop/tests/path_validation.rs`

## Part 7: Testing Requirements

### 7.1 Integration Tests

**New test files**:

1. `trop-cli/tests/migrate_command.rs`:
   - Simple migration (single reservation)
   - Recursive migration
   - Conflict handling with/without --force
   - Non-existent source path handling
   - Dry-run mode
   - Preservation of all metadata

2. `trop-cli/tests/list_projects_command.rs`:
   - Empty database
   - Multiple projects
   - Null project handling
   - Alphabetical ordering

3. `trop-cli/tests/init_command.rs`:
   - Fresh initialization
   - Existing directory/database handling
   - Overwrite mode
   - Config file creation
   - Dry-run mode

4. `trop/tests/git_inference.rs`:
   - Regular repository project/branch inference
   - Worktree detection and naming
   - Non-git directory handling
   - Nested repository scenarios

### 7.2 Unit Tests

- Migration plan building and validation
- Git inference functions
- Enhanced --allow-* flag validation

## Part 8: Implementation Order

### Step 1: Git Integration (2 days)
1. Add gix dependency
2. Implement inference module
3. Add comprehensive tests
4. Integrate with reserve operations

### Step 2: Simple Commands (2 days)
1. Implement `list-projects`:
   - Database query method
   - CLI command
   - Tests
2. Implement `init`:
   - Init operations
   - CLI command
   - Tests

### Step 3: Migrate Command (3 days)
1. Design migration plan types
2. Implement plan building
3. Implement execution
4. CLI integration
5. Comprehensive testing

### Step 4: Flag Enhancement (1 day)
1. Verify existing --allow-* implementations
2. Add missing flags
3. Update CLI commands
4. Add tests for flag combinations

### Step 5: Integration & Polish (2 days)
1. End-to-end testing
2. Documentation updates
3. Performance verification
4. Code review and cleanup

## Part 9: Risk Mitigation

### Key Risks

1. **Data Loss During Migration**
   - Mitigation: Transactional operations
   - Comprehensive dry-run mode
   - Clear warnings for destructive operations

2. **Git Integration Complexity**
   - Mitigation: Graceful fallback when git unavailable
   - Clear documentation of inference rules
   - Optional feature (doesn't break existing usage)

3. **Path Handling Edge Cases**
   - Mitigation: Extensive testing with symlinks
   - Clear normalization vs canonicalization rules
   - Consistent behavior across commands

## Deliverables Checklist

- [ ] `trop migrate` command with recursive support
- [ ] `trop list-projects` command
- [ ] `trop init` command for explicit initialization
- [ ] Git-based project/task inference using gix
- [ ] Complete `--allow-*` flag implementations
- [x] Recursive release functionality (already complete)
- [x] Proper canonicalization for implicit paths (already complete)
- [ ] Comprehensive test coverage for new features
- [ ] Updated documentation

## Success Criteria

1. **Completeness**: All Phase 11 requirements from ImplementationPlan.md implemented
2. **Robustness**: No data loss during migrations, proper transaction handling
3. **Usability**: Clear error messages, intuitive command behavior
4. **Performance**: Operations complete quickly even with many reservations
5. **Testing**: >90% code coverage for new features
6. **Integration**: Seamless integration with existing commands

## Notes for Implementation

- Follow existing plan-execute pattern consistently
- Ensure all database operations are transactional
- Match existing code style and conventions
- Prioritize correctness over performance
- Add logging at appropriate verbosity levels
- Update CLI help text to be clear and comprehensive

This plan provides a focused implementation of Phase 11 requirements without scope creep. The database migration system from the previous plan has been deferred as it wasn't in the original requirements. All seven required features are addressed, with two already complete and verified.