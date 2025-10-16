# Phase 7: Essential CLI Commands - Implementation Plan

## Overview

This phase implements the essential CLI commands (`reserve`, `release`, and `list`) with proper argument parsing, output formatting, and error handling. The CLI is designed as a thin wrapper around the library functionality, with all business logic residing in the library.

## Dependencies

- Phase 1-6 complete (core types, database, path handling, reservation operations, configuration, port allocation)
- Library API providing `ReservePlan`, `ReleasePlan`, `PlanExecutor`, etc.
- Clap 4.5+ with derive macros for CLI parsing
- Environment variable support through std::env

## Architecture Decisions

### CLI Structure
- Use clap derive macros for declarative command structure
- Global options struct shared across all commands
- Each subcommand as a separate enum variant with its own options
- Thin CLI layer that delegates to library operations

### Output Strategy
- Stdout for primary output (port numbers, list data)
- Stderr for all logging, warnings, and errors
- Shell-friendly output mode for scripting (just the port number)
- Multiple format options for list command (table, json, csv, tsv)

### Error Handling
- Consistent exit codes across all commands:
  - 0: Success
  - 1: Semantic failure (e.g., assert failed)
  - 2: Timeout (SQLite busy)
  - 3: No data directory found
  - 4+: Other errors as discovered

## Implementation Steps

### Step 1: Global CLI Structure and Options

**File: `trop-cli/src/cli.rs`** (new)

Create the core CLI type definitions with global options:

```rust
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "trop")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Enable verbose output
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Override the data directory location
    #[arg(long, value_name = "PATH", global = true, env = "TROP_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Override the default busy timeout (in seconds)
    #[arg(long, value_name = "SECONDS", global = true, env = "TROP_BUSY_TIMEOUT")]
    pub busy_timeout: Option<u32>,

    /// Disable automatic database initialization
    #[arg(long, global = true, env = "TROP_DISABLE_AUTOINIT")]
    pub disable_autoinit: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Reserve a port for a directory
    Reserve(ReserveCommand),

    /// Release a port reservation
    Release(ReleaseCommand),

    /// List active reservations
    List(ListCommand),
}
```

### Step 2: Reserve Command

**File: `trop-cli/src/commands/reserve.rs`** (new)

Implement the reserve command with all its options:

```rust
use clap::Args;
use std::path::PathBuf;
use trop::{Port, ReservationKey, ReserveOptions as LibReserveOptions};

#[derive(Args)]
pub struct ReserveCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Project identifier
    #[arg(long, value_name = "PROJECT", env = "TROP_PROJECT")]
    pub project: Option<String>,

    /// Task identifier
    #[arg(long, value_name = "TASK", env = "TROP_TASK")]
    pub task: Option<String>,

    /// Preferred port number
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

    /// Minimum acceptable port
    #[arg(long, value_name = "MIN", env = "TROP_MIN")]
    pub min: Option<u16>,

    /// Maximum acceptable port
    #[arg(long, value_name = "MAX", env = "TROP_MAX")]
    pub max: Option<u16>,

    /// Overwrite existing reservation
    #[arg(long)]
    pub overwrite: bool,

    /// Ignore if preferred port is occupied
    #[arg(long)]
    pub ignore_occupied: bool,

    /// Ignore excluded ports
    #[arg(long)]
    pub ignore_exclusions: bool,

    /// Force operation (overrides all protections)
    #[arg(long)]
    pub force: bool,

    /// Allow operations on unrelated paths
    #[arg(long, env = "TROP_ALLOW_UNRELATED_PATH")]
    pub allow_unrelated_path: bool,

    /// Allow changing the project field
    #[arg(long, env = "TROP_ALLOW_PROJECT_CHANGE")]
    pub allow_project_change: bool,

    /// Allow changing the task field
    #[arg(long, env = "TROP_ALLOW_TASK_CHANGE")]
    pub allow_task_change: bool,

    /// Allow changing project or task fields
    #[arg(long, env = "TROP_ALLOW_CHANGE")]
    pub allow_change: bool,

    /// Disable automatic pruning
    #[arg(long, env = "TROP_DISABLE_AUTOPRUNE")]
    pub disable_autoprune: bool,

    /// Disable automatic expiration
    #[arg(long, env = "TROP_DISABLE_AUTOEXPIRE")]
    pub disable_autoexpire: bool,

    /// Disable all automatic cleanup
    #[arg(long)]
    pub disable_autoclean: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,

    /// Skip occupancy check
    #[arg(long)]
    pub skip_occupancy_check: bool,

    /// Skip TCP checks
    #[arg(long)]
    pub skip_tcp: bool,

    /// Skip UDP checks
    #[arg(long)]
    pub skip_udp: bool,

    /// Skip IPv6 checks
    #[arg(long)]
    pub skip_ipv6: bool,

    /// Skip IPv4 checks
    #[arg(long)]
    pub skip_ipv4: bool,

    /// Check all network interfaces
    #[arg(long)]
    pub check_all_interfaces: bool,
}

impl ReserveCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve path (use CWD if not specified, canonicalize if implicit)
        let path = resolve_path(self.path)?;

        // 2. Build ReservationKey
        let key = ReservationKey::new(path, self.tag)?;

        // 3. Load configuration (hierarchical)
        let config = load_configuration(global, &self)?;

        // 4. Apply cleanup flags logic
        let (disable_autoprune, disable_autoexpire) = if self.disable_autoclean {
            (true, true)
        } else {
            (self.disable_autoprune, self.disable_autoexpire)
        };

        // 5. Build library ReserveOptions
        let mut options = LibReserveOptions::new(key, self.port.map(Port::try_from).transpose()?)
            .with_project(self.project)
            .with_task(self.task)
            .with_preferred_port(self.port.map(Port::try_from).transpose()?)
            .with_ignore_occupied(self.ignore_occupied)
            .with_ignore_exclusions(self.ignore_exclusions)
            .with_force(self.force)
            .with_allow_unrelated_path(self.allow_unrelated_path)
            .with_allow_project_change(self.allow_project_change || self.allow_change)
            .with_allow_task_change(self.allow_task_change || self.allow_change);

        // 6. Open database
        let db = open_database(global, &config)?;

        // 7. Build and execute plan
        let plan = ReservePlan::new(options, &config).build_plan(&db)?;

        if self.dry_run {
            print_plan(&plan);
            return Ok(());
        }

        let executor = PlanExecutor::new(&mut db);
        let result = executor.execute(plan)?;

        // 8. Output just the port number (shell-friendly)
        if !global.quiet {
            println!("{}", result.port);
        }

        Ok(())
    }
}
```

### Step 3: Release Command

**File: `trop-cli/src/commands/release.rs`** (new)

Implement the release command:

```rust
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct ReleaseCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Only release untagged reservation
    #[arg(long)]
    pub untagged_only: bool,

    /// Release all reservations under path recursively
    #[arg(long)]
    pub recursive: bool,

    /// Force operation
    #[arg(long)]
    pub force: bool,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl ReleaseCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve path
        let path = resolve_path(self.path)?;

        // 2. Validate option combinations
        if self.tag.is_some() && self.untagged_only {
            return Err(CliError::InvalidArguments(
                "Cannot specify both --tag and --untagged-only".to_string()
            ));
        }

        // 3. Load configuration
        let config = load_configuration(global, &self)?;

        // 4. Open database
        let db = open_database(global, &config)?;

        // 5. Build release options
        let options = ReleaseOptions {
            path,
            tag: self.tag,
            untagged_only: self.untagged_only,
            recursive: self.recursive,
            force: self.force,
        };

        // 6. Build and execute plan
        let plan = ReleasePlan::new(options).build_plan(&db)?;

        if self.dry_run {
            print_plan(&plan);
            return Ok(());
        }

        let executor = PlanExecutor::new(&mut db);
        executor.execute(plan)?;

        if !global.quiet {
            eprintln!("Released reservations successfully");
        }

        Ok(())
    }
}
```

### Step 4: List Command

**File: `trop-cli/src/commands/list.rs`** (new)

Implement the list command with formatting options:

```rust
use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Args)]
pub struct ListCommand {
    /// Output format
    #[arg(long, value_enum, default_value = "table", env = "TROP_OUTPUT_FORMAT")]
    pub format: OutputFormat,

    /// Filter by project
    #[arg(long, value_name = "PROJECT")]
    pub filter_project: Option<String>,

    /// Filter by tag
    #[arg(long, value_name = "TAG")]
    pub filter_tag: Option<String>,

    /// Filter by path prefix
    #[arg(long, value_name = "PATH")]
    pub filter_path: Option<PathBuf>,

    /// Show full paths instead of shortened forms
    #[arg(long)]
    pub show_full_paths: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
    Tsv,
}

impl ListCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration
        let config = load_configuration(global, &self)?;

        // 2. Open database (read-only)
        let db = open_database_readonly(global, &config)?;

        // 3. Query reservations with filters
        let mut reservations = db.list_reservations()?;

        // Apply filters
        if let Some(project) = &self.filter_project {
            reservations.retain(|r| r.project() == Some(project.as_str()));
        }

        if let Some(tag) = &self.filter_tag {
            reservations.retain(|r| r.key().tag() == Some(tag.as_str()));
        }

        if let Some(path) = &self.filter_path {
            let canonical = canonicalize_path(path)?;
            reservations.retain(|r| r.key().path().starts_with(&canonical));
        }

        // 4. Format and output
        match self.format {
            OutputFormat::Table => format_as_table(&reservations, self.show_full_paths),
            OutputFormat::Json => format_as_json(&reservations),
            OutputFormat::Csv => format_as_csv(&reservations),
            OutputFormat::Tsv => format_as_tsv(&reservations),
        }

        Ok(())
    }
}

fn format_as_table(reservations: &[Reservation], show_full: bool) {
    // Table format: PORT PATH TAG PROJECT TASK CREATED LAST_USED
    println!("PORT\tPATH\tTAG\tPROJECT\tTASK\tCREATED\tLAST_USED");

    for res in reservations {
        let path = if show_full {
            res.key().path().display().to_string()
        } else {
            shorten_path(res.key().path())
        };

        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            res.port().value(),
            path,
            res.key().tag().unwrap_or("-"),
            res.project().unwrap_or("-"),
            res.task().unwrap_or("-"),
            format_timestamp(res.created_at()),
            format_timestamp(res.last_used_at()),
        );
    }
}

fn shorten_path(path: &Path) -> String {
    // If within home directory, show as ~/...
    if let Some(home) = home::home_dir() {
        if path.starts_with(&home) {
            if let Ok(relative) = path.strip_prefix(&home) {
                return format!("~/{}", relative.display());
            }
        }
    }
    // Otherwise show full path
    path.display().to_string()
}
```

### Step 5: Main Entry Point Updates

**File: `trop-cli/src/main.rs`** (modify)

Update main.rs to use the new CLI structure:

```rust
mod cli;
mod commands;
mod error;
mod utils;

use clap::Parser;
use cli::Cli;
use error::CliError;
use trop::init_logger;

fn main() {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Initialize logging based on verbosity
    let log_level = if cli.quiet {
        trop::LogLevel::Error
    } else if cli.verbose {
        trop::LogLevel::Debug
    } else {
        trop::LogLevel::Info
    };

    init_logger(log_level);

    // Execute the command
    let result = match cli.command {
        cli::Command::Reserve(cmd) => cmd.execute(&cli.into()),
        cli::Command::Release(cmd) => cmd.execute(&cli.into()),
        cli::Command::List(cmd) => cmd.execute(&cli.into()),
    };

    // Handle errors and set exit code
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(e.exit_code());
        }
    }
}

impl From<&Cli> for GlobalOptions {
    fn from(cli: &Cli) -> Self {
        GlobalOptions {
            verbose: cli.verbose,
            quiet: cli.quiet,
            data_dir: cli.data_dir.clone(),
            busy_timeout: cli.busy_timeout,
            disable_autoinit: cli.disable_autoinit,
        }
    }
}
```

### Step 6: Error Handling

**File: `trop-cli/src/error.rs`** (new)

Create CLI-specific error types with exit codes:

```rust
use std::fmt;
use trop::Error as LibError;

#[derive(Debug)]
pub enum CliError {
    /// Library error
    Library(LibError),

    /// Invalid command-line arguments
    InvalidArguments(String),

    /// I/O error
    Io(std::io::Error),

    /// Timeout waiting for database lock
    Timeout,

    /// Data directory not found (and auto-init disabled)
    NoDataDirectory,
}

impl CliError {
    /// Get the appropriate exit code for this error
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::Library(LibError::AssertionFailed { .. }) => 1,
            CliError::Timeout => 2,
            CliError::NoDataDirectory => 3,
            CliError::InvalidArguments(_) => 4,
            CliError::Io(_) => 5,
            CliError::Library(_) => 6,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Library(e) => write!(f, "{}", e),
            CliError::InvalidArguments(msg) => write!(f, "Invalid arguments: {}", msg),
            CliError::Io(e) => write!(f, "I/O error: {}", e),
            CliError::Timeout => write!(f, "Timeout waiting for database lock"),
            CliError::NoDataDirectory => write!(f, "Data directory not found (use --data-dir or enable auto-init)"),
        }
    }
}

impl From<LibError> for CliError {
    fn from(e: LibError) -> Self {
        // Check for specific error types that need special handling
        if matches!(e, LibError::DatabaseTimeout { .. }) {
            CliError::Timeout
        } else {
            CliError::Library(e)
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}
```

### Step 7: Utility Functions

**File: `trop-cli/src/utils.rs`** (new)

Common utility functions for the CLI:

```rust
use std::path::{Path, PathBuf};
use std::env;
use trop::{Config, ConfigBuilder, Database, DatabaseConfig};
use crate::error::CliError;

/// Resolve a path, using CWD if not specified
pub fn resolve_path(path: Option<PathBuf>) -> Result<PathBuf, CliError> {
    match path {
        Some(p) => {
            // Explicit path - normalize but don't canonicalize
            Ok(normalize_path(&p))
        }
        None => {
            // Implicit path - use CWD and canonicalize
            let cwd = env::current_dir()?;
            Ok(canonicalize_path(&cwd)?)
        }
    }
}

/// Normalize a path (make absolute, expand ~, etc.)
pub fn normalize_path(path: &Path) -> PathBuf {
    // Implementation using trop::PathResolver
    use trop::PathResolver;
    PathResolver::normalize(path, trop::PathProvenance::Explicit)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Canonicalize a path (follow symlinks)
pub fn canonicalize_path(path: &Path) -> Result<PathBuf, CliError> {
    path.canonicalize().map_err(CliError::from)
}

/// Load hierarchical configuration
pub fn load_configuration(
    global: &GlobalOptions,
    command_opts: &impl CommandOptions,
) -> Result<Config, CliError> {
    let mut builder = ConfigBuilder::new();

    // Apply data directory if specified
    if let Some(data_dir) = &global.data_dir {
        builder = builder.data_dir(data_dir.clone());
    }

    // Apply busy timeout if specified
    if let Some(timeout) = global.busy_timeout {
        builder = builder.busy_timeout(timeout);
    }

    // Apply command-specific options
    command_opts.apply_to_config(&mut builder);

    // Load configuration files (trop.yaml, config.yaml, etc.)
    builder = builder.load_from_environment()?;

    Ok(builder.build()?)
}

/// Open database with configuration
pub fn open_database(
    global: &GlobalOptions,
    config: &Config,
) -> Result<Database, CliError> {
    let db_path = resolve_database_path(global, config)?;

    if !db_path.exists() && global.disable_autoinit {
        return Err(CliError::NoDataDirectory);
    }

    let db_config = DatabaseConfig::new(db_path)
        .with_busy_timeout(config.maximum_lock_wait_seconds)
        .with_auto_init(!global.disable_autoinit);

    Ok(Database::open(db_config)?)
}

/// Format a timestamp for display
pub fn format_timestamp(ts: &chrono::DateTime<chrono::Utc>) -> String {
    ts.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Print a plan in human-readable format
pub fn print_plan(plan: &OperationPlan) {
    eprintln!("Plan: {}", plan.description());
    for (i, action) in plan.actions.iter().enumerate() {
        eprintln!("  {}. {}", i + 1, describe_action(action));
    }
}
```

### Step 8: Integration Tests

**File: `trop-cli/tests/reserve_command.rs`** (new)

Test the reserve command:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_reserve_basic() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // Reserve a port
    Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\d+\n$").unwrap());
}

#[test]
fn test_reserve_with_tag() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // Reserve with tag
    Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(temp.path())
        .arg("--tag").arg("web")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"^\d+\n$").unwrap());
}

#[test]
fn test_reserve_idempotent() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");
    let path = temp.path();

    // First reservation
    let output1 = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(path)
        .output()
        .unwrap();

    let port1 = String::from_utf8(output1.stdout).unwrap();

    // Second reservation - should return same port
    let output2 = Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(path)
        .output()
        .unwrap();

    let port2 = String::from_utf8(output2.stdout).unwrap();

    assert_eq!(port1, port2);
}

#[test]
fn test_reserve_dry_run() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // Dry run should not create database
    Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(temp.path())
        .arg("--dry-run")
        .assert()
        .success();

    // Database should not exist
    assert!(!data_dir.exists());
}
```

**File: `trop-cli/tests/list_command.rs`** (new)

Test the list command:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_list_empty() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // List with no reservations
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir").arg(&data_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("PORT\tPATH"));
}

#[test]
fn test_list_formats() {
    let temp = tempdir().unwrap();
    let data_dir = temp.path().join("trop-data");

    // Create a reservation first
    Command::cargo_bin("trop")
        .unwrap()
        .arg("reserve")
        .arg("--data-dir").arg(&data_dir)
        .arg("--path").arg(temp.path())
        .assert()
        .success();

    // Test table format (default)
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir").arg(&data_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("\t"));

    // Test JSON format
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir").arg(&data_dir)
        .arg("--format").arg("json")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["));

    // Test CSV format
    Command::cargo_bin("trop")
        .unwrap()
        .arg("list")
        .arg("--data-dir").arg(&data_dir)
        .arg("--format").arg("csv")
        .assert()
        .success()
        .stdout(predicate::str::contains(","));
}
```

## Testing Strategy

### Unit Tests
- Test argument parsing and validation
- Test format conversion functions
- Test path resolution logic
- Test error handling and exit codes

### Integration Tests
- Test each command with various flag combinations
- Test idempotency of reserve command
- Test filtering in list command
- Test dry-run mode for mutating commands
- Test environment variable handling
- Test configuration file loading

### Manual Testing Checklist
- [ ] Reserve a port without arguments (uses CWD)
- [ ] Reserve with explicit path and tag
- [ ] Reserve with preferred port
- [ ] Reserve with project/task metadata
- [ ] List reservations in different formats
- [ ] Release specific reservations
- [ ] Test --quiet and --verbose flags
- [ ] Test environment variable overrides
- [ ] Test configuration file precedence

## Implementation Order

1. **Core CLI structure** - Set up clap derive structure with global options
2. **Error handling** - Create CliError type with exit codes
3. **Utility functions** - Path resolution, config loading, database opening
4. **Reserve command** - Full implementation with all flags
5. **List command** - With all format options
6. **Release command** - With tag/recursive support
7. **Integration tests** - Comprehensive test coverage
8. **Documentation** - Update help text and add examples

## Non-Obvious Considerations

### Path Handling
- Explicit paths (from --path) are normalized but NOT canonicalized
- Implicit paths (CWD) are both normalized AND canonicalized
- This prevents surprising behavior when users specify symlinks explicitly

### Configuration Precedence
The configuration loading order must be:
1. Command-line flags (highest priority)
2. Environment variables
3. trop.local.yaml (private, not in version control)
4. trop.yaml (project configuration)
5. ~/.trop/config.yaml (user configuration)
6. Built-in defaults (lowest priority)

### Output Separation
- ALL program output (port numbers, lists) goes to stdout
- ALL logging, errors, and informational messages go to stderr
- This enables: `PORT=$(trop reserve)` in shell scripts

### Dry Run Mode
- Must show exactly what would happen without making changes
- Should work even if database doesn't exist
- Useful for testing and debugging configuration

### Force vs Specific Allow Flags
- `--force` overrides ALL protections (path, project, task)
- Specific flags like `--allow-project-change` provide fine-grained control
- `--allow-change` is convenience for allowing both project and task changes

## Dependencies to Add

Update `trop-cli/Cargo.toml`:

```toml
[dependencies]
trop = { path = "../trop" }
clap = { workspace = true }
anyhow = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
home = { workspace = true }
csv = "1.3"  # For CSV output format

[dev-dependencies]
assert_cmd = "2.0"
predicates = "3.1"
tempfile = { workspace = true }
```

## Success Criteria

The phase is complete when:

1. All three commands (reserve, release, list) are fully functional
2. All command-line flags work as specified
3. Environment variable overrides work correctly
4. Output formats (table, json, csv, tsv) work for list command
5. Exit codes are consistent and documented
6. Stdout/stderr separation is correct
7. Dry-run mode works for mutating commands
8. Integration tests pass
9. The CLI is a thin wrapper with business logic in the library

## Notes for Implementer

- Start with the simplest command (list) to establish patterns
- Use the library's plan-execute pattern consistently
- Keep the CLI layer thin - it should only handle I/O and argument parsing
- Test with real shell scripts to ensure shell-friendly output
- Consider adding shell completion generation in a future phase
- Document any deviations from this plan with clear rationale