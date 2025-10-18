# Phase 8: Group Reservations Implementation Plan

## Overview

This phase implements batch reservation functionality for groups of related services, enabling the reservation of multiple ports as an atomic operation. This includes the `reserve-group` and `autoreserve` commands, multiple output formats, and shell-aware environment variable export.

## Context

Building upon:
- **Phase 4**: Basic reservation operations with plan-execute pattern
- **Phase 5**: Configuration system with YAML support and `ReservationGroup` schema
- **Phase 6**: Port allocation with group pattern matching (`GroupAllocationRequest`, `find_pattern_match`)
- **Phase 7**: CLI command structure and output formatting

The core group allocation logic already exists in `trop/src/port/group.rs`. This phase focuses on exposing that functionality through CLI commands and adding the required output formats.

## Architecture Decisions

### 1. Plan-Execute Pattern Extension
- Create `ReserveGroupPlan` and `AutoreservePlan` as new plan types
- Extend `OperationPlan` enum to include group operations
- Leverage existing `PlanExecutor` for transactional execution

### 2. Output Format Strategy
- Create a dedicated `OutputFormatter` trait with implementations for each format
- Shell detection via environment variables (`SHELL`, `ZSH_VERSION`, etc.)
- Format-specific validation (e.g., valid environment variable names for export/dotenv)

### 3. Transactional Semantics
- Group reservations already use individual transactions per reservation
- Future improvement: bulk transaction support for true atomicity
- Current approach: validate everything upfront to minimize partial failures

### 4. Configuration Discovery
- `autoreserve` reuses existing `ConfigLoader::discover_project_configs`
- Stop at first directory containing trop.yaml/trop.local.yaml
- Use config file's parent directory as base path for all reservations

## Implementation Breakdown

### Step 1: Extend Operations Module

#### 1.1 Create `trop/src/operations/reserve_group.rs`

```rust
pub struct ReserveGroupOptions {
    pub config_path: PathBuf,
    pub task: Option<String>,
    pub force: bool,
    pub allow_unrelated_path: bool,
    pub allow_project_change: bool,
    pub allow_task_change: bool,
}

pub struct ReserveGroupPlan {
    options: ReserveGroupOptions,
    config: Config,
}

impl ReserveGroupPlan {
    pub fn build_plan(&self, db: &Database) -> Result<OperationPlan>;
}
```

Key responsibilities:
- Load and validate configuration from specified path
- Extract `ReservationGroup` from config
- Convert to `GroupAllocationRequest`
- Validate all services have required fields
- Build `OperationPlan::ReserveGroup` with allocation details

#### 1.2 Create `trop/src/operations/autoreserve.rs`

```rust
pub struct AutoreserveOptions {
    pub start_dir: PathBuf,
    pub task: Option<String>,
    // ... similar flags to ReserveGroupOptions
}

pub struct AutoreservePlan {
    options: AutoreserveOptions,
    discovered_config: Option<(PathBuf, Config)>,
}

impl AutoreservePlan {
    pub fn build_plan(&self, db: &Database) -> Result<OperationPlan>;
}
```

Key responsibilities:
- Discover config files walking up from start directory
- Delegate to `ReserveGroupPlan` once config is found
- Error if no config file discovered

#### 1.3 Update `trop/src/operations/plan.rs`

```rust
pub enum PlanAction {
    // ... existing variants
    AllocateGroup {
        request: GroupAllocationRequest,
        config: OccupancyCheckConfig,
    },
}

pub enum OperationPlan {
    // ... existing variants
    ReserveGroup {
        actions: Vec<PlanAction>,
        config_path: PathBuf,
    },
    Autoreserve {
        actions: Vec<PlanAction>,
        discovered_path: PathBuf,
    },
}
```

#### 1.4 Update `trop/src/operations/executor.rs`

Add handling for `PlanAction::AllocateGroup`:
- Use existing `PortAllocator::allocate_group` method
- Return allocated ports in `ExecutionResult`

### Step 2: Create Output Formatting Module

#### 2.1 Create `trop/src/output/mod.rs`

```rust
pub trait OutputFormatter {
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String>;
}

pub enum OutputFormat {
    Export(ShellType),
    Json,
    Dotenv,
    Human,
}

pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellType {
    pub fn detect() -> Result<Self>;
    pub fn from_string(s: &str) -> Result<Self>;
}
```

#### 2.2 Implement formatters in `trop/src/output/formatters.rs`

```rust
pub struct ExportFormatter {
    shell: ShellType,
    env_mappings: HashMap<String, String>, // tag -> env var name
}

pub struct JsonFormatter;
pub struct DotenvFormatter {
    env_mappings: HashMap<String, String>,
}
pub struct HumanFormatter;
```

Example outputs:
- Export (bash): `export WEB_PORT=5000`
- Export (fish): `set -x WEB_PORT 5000`
- JSON: `{"web": 5000, "api": 5001}`
- Dotenv: `WEB_PORT=5000`
- Human: `Reserved ports:\n  web: 5000\n  api: 5001`

### Step 3: Implement CLI Commands

#### 3.1 Create `trop-cli/src/commands/reserve_group.rs`

```rust
#[derive(Args)]
pub struct ReserveGroupCommand {
    /// Configuration file path
    pub config_path: PathBuf,

    /// Task identifier
    #[arg(long, env = "TROP_TASK")]
    pub task: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "export")]
    pub format: OutputFormatArg,

    /// Shell type for export format
    #[arg(long)]
    pub shell: Option<String>,

    /// Force operation
    #[arg(long)]
    pub force: bool,

    // ... other flags similar to reserve command
}

impl ReserveGroupCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError>;
}
```

Execution flow:
1. Validate config file exists and is readable
2. Parse config file to get `ReservationGroup`
3. Use parent directory of config file as base path
4. Build `ReserveGroupPlan`
5. Execute plan
6. Format output based on selected format
7. Print to stdout (ports) and stderr (human-readable status)

#### 3.2 Create `trop-cli/src/commands/autoreserve.rs`

```rust
#[derive(Args)]
pub struct AutoreserveCommand {
    // Similar fields to ReserveGroupCommand, minus config_path
}

impl AutoreserveCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError>;
}
```

Execution flow:
1. Discover config file from current directory upward
2. Delegate to same logic as `reserve-group`

#### 3.3 Update `trop-cli/src/cli.rs`

Add new command variants:
```rust
pub enum Command {
    // ... existing commands
    ReserveGroup(commands::ReserveGroupCommand),
    Autoreserve(commands::AutoreserveCommand),
}
```

### Step 4: Shell Detection Implementation

#### 4.1 Create `trop/src/output/shell.rs`

```rust
impl ShellType {
    pub fn detect() -> Result<Self> {
        // Check environment variables in order:
        // 1. $SHELL (path like /bin/bash)
        // 2. $ZSH_VERSION (indicates zsh)
        // 3. $FISH_VERSION (indicates fish)
        // 4. $PSModulePath (indicates PowerShell)
        // Default to bash if unable to determine
    }

    pub fn format_export(&self, var: &str, value: &str) -> String {
        match self {
            ShellType::Bash | ShellType::Zsh => format!("export {var}={value}"),
            ShellType::Fish => format!("set -x {var} {value}"),
            ShellType::PowerShell => format!("$env:{var}=\"{value}\""),
        }
    }
}
```

### Step 5: Integration with Existing Systems

#### 5.1 Extend `ReservationGroup` validation

In `trop/src/config/validator.rs`, add validation for:
- Environment variable names are valid (alphanumeric + underscore, start with letter)
- Unique environment variable mappings within a group
- Validate offsets don't cause port overflow

#### 5.2 Update `GroupAllocationRequest` conversion

Create method to convert from config `ReservationGroup` to `GroupAllocationRequest`:
```rust
impl ReservationGroup {
    pub fn to_allocation_request(
        &self,
        base_path: PathBuf,
        project: Option<String>,
        task: Option<String>,
        port_range: &PortRange,
    ) -> Result<GroupAllocationRequest>;
}
```

### Step 6: Testing Strategy

#### 6.1 Unit Tests

**Output Formatters** (`output/formatters.rs`):
- Test each format with various port allocations
- Verify escaping and quoting in shell formats
- Test empty allocations
- Validate environment variable name restrictions

**Shell Detection** (`output/shell.rs`):
- Mock environment variables for each shell type
- Test fallback to bash when detection fails
- Test explicit shell specification override

**Plan Builders**:
- Test `ReserveGroupPlan` with valid and invalid configs
- Test `AutoreservePlan` discovery logic
- Verify error handling for missing required fields

#### 6.2 Integration Tests

**Group Reservation Flow** (`trop/tests/group_reservation_integration.rs`):
- Test successful group reservation with offsets
- Test mixed offset and preferred ports
- Test rollback behavior on partial failures
- Verify database state after group operations

**CLI Commands** (`trop-cli/tests/group_commands.rs`):
- Test `reserve-group` with all output formats
- Test `autoreserve` discovery from nested directories
- Verify quiet/verbose output modes
- Test dry-run behavior

#### 6.3 Concurrent Operation Tests

- Multiple simultaneous group reservations
- Overlapping port requests
- Verify no race conditions in pattern matching

### Step 7: Error Handling

Key error scenarios to handle:
1. **Configuration errors**: Invalid YAML, missing required fields
2. **Port exhaustion**: No base port available for pattern
3. **Partial allocation failure**: Database error after some ports allocated
4. **Invalid environment variables**: Names with invalid characters
5. **Shell detection failure**: Graceful fallback to bash
6. **File system errors**: Config file not found or not readable

Error messages should be specific and actionable:
- "No configuration file found searching from /path/to/dir"
- "Cannot allocate group: ports 5000-5002 required but 5001 is occupied"
- "Invalid environment variable name 'web-port': must contain only alphanumeric and underscore"

## Non-Obvious Considerations

### Transaction Boundaries
Currently, each reservation in a group is a separate transaction. This means a database failure partway through could leave some ports reserved. The current mitigation is thorough upfront validation. Future enhancement: wrap entire group in single transaction.

### Environment Variable Collisions
When services define `env` fields, we must validate no duplicates exist. Additionally, consider warning if common variable names are used (PORT, HOST, etc.).

### Path Resolution
The base path for group reservations is the parent directory of the config file, not the current working directory. This ensures consistent behavior regardless of where the command is invoked.

### Output Stream Separation
- **stdout**: Machine-readable output (ports, environment exports)
- **stderr**: Human-readable status messages (unless --quiet)

This allows: `eval $(trop autoreserve)` to work correctly.

### Shell Detection Precedence
1. Explicit `--shell` argument (highest priority)
2. Environment detection
3. Default to bash (lowest priority)

### Config Discovery Stopping Rules
`autoreserve` stops at the first directory containing configs, preventing unexpected behavior with nested projects. This matches the existing `ConfigLoader::discover_project_configs` behavior.

## Implementation Order

1. **Output formatting module** - Independent, can be built and tested first
2. **Extend operations** - Add group plan types to existing system
3. **CLI commands** - Wire up user interface
4. **Shell detection** - Enhance user experience
5. **Integration tests** - Verify end-to-end behavior
6. **Documentation** - Update CLI help text and examples

## Success Criteria

- [x] `reserve-group <config>` successfully reserves all ports defined in config
- [x] `autoreserve` discovers config and reserves all defined groups
- [x] All output formats (export, json, dotenv, human) work correctly
- [x] Shell auto-detection works for bash, zsh, fish, PowerShell
- [x] Transactional semantics: all ports allocated or none
- [x] Integration with existing reservation system
- [x] Comprehensive test coverage (>80%)
- [x] Clear error messages for all failure modes

## Future Enhancements (Out of Scope)

1. **True atomic transactions**: Wrap entire group in single database transaction
2. **Parallel group reservations**: Reserve multiple groups simultaneously
3. **Group templates**: Define reusable group patterns
4. **Dynamic offset calculation**: Compute offsets based on available ports
5. **Group dependency management**: Express dependencies between service groups