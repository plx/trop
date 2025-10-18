# Phase 10: Assertion and Utility Commands - Implementation Plan

## Executive Summary

This plan details the implementation of assertion and utility commands for the trop CLI tool. These commands provide essential testing, debugging, and automation support capabilities. The implementation follows established patterns from phases 7-9, leveraging existing library functionality while adding new capabilities for configuration validation, port scanning, and assertion-based automation.

## Current State Analysis

### Existing Infrastructure

The trop project has mature infrastructure that Phase 10 will leverage:

1. **CLI Framework** (phases 7-9):
   - Command trait pattern with `execute(&self, global: &GlobalOptions)`
   - Structured error handling with semantic exit codes
   - Global options handling (verbose, quiet, data-dir, etc.)
   - Consistent output formatting

2. **Port Scanning** (phase 6):
   - `PortOccupancyChecker` trait with system implementation
   - `OccupancyCheckConfig` for controlling scan parameters
   - `find_occupied_ports` method for range scanning (requires implementation)

3. **Configuration System** (phase 5):
   - `Config` struct with `PortConfig`, `PortExclusion`, etc. (already exist)
   - `ConfigValidator::validate` for configuration validation (already exists)
   - Hierarchical configuration loading
   - YAML parsing with serde

4. **Database Operations** (phases 2-4):
   - `is_port_reserved(port: Port) -> Result<bool>` method (already exists)
   - Need to add: `get_reservation_by_port`, `get_reserved_ports_in_range`
   - Path existence checking utilities

5. **Path Utilities** (existing in `trop-cli/src/utils.rs`):
   - `resolve_path(Option<PathBuf>) -> Result<PathBuf, CliError>`
   - `normalize_path(&Path) -> Result<PathBuf, CliError>`

6. **Output Formatting** (phase 8):
   - JSON, CSV, TSV, and table formatters
   - Shell-friendly output modes

### Commands to Implement

According to the specification, Phase 10 includes:

1. **Assertion Commands** (exit code 0/1 for success/failure):
   - `assert-reservation`: Check if reservation exists for path/tag
   - `assert-port`: Check if specific port is reserved
   - `assert-data-dir`: Validate data directory exists/is valid

2. **Information Commands**:
   - `port-info`: Display detailed information about a port (NOT `show`)
   - `show-data-dir`: Print resolved data directory path
   - `show-path`: Print resolved/canonicalized path

3. **Scanning Commands**:
   - `scan`: Scan port range for occupancy with auto-exclude option

4. **Configuration Commands**:
   - `validate`: Validate trop.yaml/config.yaml files
   - `exclude`: Add port/range to exclusion list
   - `compact-exclusions`: Optimize exclusion list representation

## Implementation Plan

### Part 0: Error Handling Strategy

Add semantic failure variant to CLI error handling for assertion commands:

```rust
// In trop-cli/src/error.rs, add new variant
pub enum CliError {
    // ... existing variants ...

    /// Semantic failure (e.g., assertion failed) - exit code 1
    SemanticFailure(String),
}

// Update exit_code method
impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::SemanticFailure(_) => 1,
            // ... rest as before
        }
    }
}

// Update Display implementation
impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::SemanticFailure(msg) => write!(f, "{msg}"),
            // ... rest as before
        }
    }
}
```

### Part 1: Assertion Commands

#### 1.1 AssertReservation Command (`trop-cli/src/commands/assert_reservation.rs`)

```rust
use crate::error::CliError;
use crate::utils::{normalize_path, resolve_path, open_database, load_configuration, GlobalOptions};
use clap::Args;
use std::path::PathBuf;
use trop::ReservationKey;

/// Assert that a reservation exists for a specific path/tag combination.
#[derive(Args)]
pub struct AssertReservationCommand {
    /// Directory path (default: current directory)
    #[arg(long, value_name = "PATH", env = "TROP_PATH")]
    pub path: Option<PathBuf>,

    /// Service tag
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Invert the assertion (fail if reservation exists)
    #[arg(long)]
    pub not: bool,
}

impl AssertReservationCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve path using existing utilities
        let path = resolve_path(self.path)?;
        let normalized = normalize_path(&path)?;

        // 2. Open database (read-only)
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Build reservation key and query
        let key = ReservationKey::new(normalized, self.tag)
            .map_err(|e| CliError::Library(e))?;

        let reservation = db.get_reservation(&key)
            .map_err(|e| CliError::Library(e))?;

        // 4. Check assertion
        let exists = reservation.is_some();
        let success = if self.not { !exists } else { exists };

        // 5. Output port if found (unless --quiet)
        if !self.not && exists && !global.quiet {
            if let Some(res) = reservation {
                println!("{}", res.port());
            }
        }

        // 6. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!("Assertion failed: reservation exists for {}", key)
            } else {
                format!("Assertion failed: no reservation found for {}", key)
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}
```

#### 1.2 AssertPort Command (`trop-cli/src/commands/assert_port.rs`)

```rust
use crate::error::CliError;
use crate::utils::{open_database, load_configuration, GlobalOptions};
use clap::Args;
use trop::Port;

/// Assert that a specific port is reserved.
#[derive(Args)]
pub struct AssertPortCommand {
    /// Port number to check
    #[arg(value_name = "PORT")]
    pub port: u16,

    /// Invert the assertion (fail if port is reserved)
    #[arg(long)]
    pub not: bool,
}

impl AssertPortCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Parse port
        let port = Port::try_from(self.port)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Check if port is reserved (method already exists)
        let reserved = db.is_port_reserved(port)
            .map_err(|e| CliError::Library(e))?;

        // 4. Check assertion
        let success = if self.not { !reserved } else { reserved };

        // 5. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!("Assertion failed: port {} is reserved", port)
            } else {
                format!("Assertion failed: port {} is not reserved", port)
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}
```

#### 1.3 AssertDataDir Command (`trop-cli/src/commands/assert_data_dir.rs`)

```rust
use crate::error::CliError;
use crate::utils::{resolve_data_dir, GlobalOptions};
use clap::Args;
use std::path::{Path, PathBuf};
use trop::{Database, DatabaseConfig};

/// Assert that the data directory exists and is valid.
#[derive(Args)]
pub struct AssertDataDirCommand {
    /// Data directory path to check (default: ~/.trop)
    #[arg(long, value_name = "PATH", env = "TROP_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Invert the assertion (fail if data dir exists)
    #[arg(long)]
    pub not: bool,

    /// Also validate database contents
    #[arg(long)]
    pub validate: bool,
}

impl AssertDataDirCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Resolve data directory
        let data_dir = self.data_dir.or(global.data_dir.clone())
            .unwrap_or_else(|| resolve_data_dir());

        // 2. Check existence
        let exists = data_dir.exists();

        // 3. If validating, check database integrity
        let valid = if exists && self.validate {
            match validate_database(&data_dir) {
                Ok(()) => true,
                Err(_) => false,
            }
        } else {
            exists
        };

        // 4. Check assertion
        let success = if self.not { !valid } else { valid };

        // 5. Return with appropriate exit code
        if success {
            Ok(())
        } else {
            let msg = if self.not {
                format!("Assertion failed: data directory exists at {}", data_dir.display())
            } else if self.validate && exists {
                format!("Assertion failed: database validation failed")
            } else {
                format!("Assertion failed: data directory not found at {}", data_dir.display())
            };
            Err(CliError::SemanticFailure(msg))
        }
    }
}

fn validate_database(data_dir: &Path) -> Result<(), CliError> {
    let db_path = data_dir.join("trop.db");
    if !db_path.exists() {
        return Err(CliError::InvalidArguments("Database file not found".into()));
    }

    // Open database and run integrity check
    let config = DatabaseConfig::new(db_path);
    let mut db = Database::open(config)
        .map_err(|e| CliError::Library(e))?;

    // Run PRAGMA integrity_check
    db.verify_integrity()
        .map_err(|e| CliError::Library(e))?;

    Ok(())
}
```

### Part 2: Information Commands

#### 2.1 PortInfo Command (`trop-cli/src/commands/port_info.rs`)

**Note**: The command is called `port-info`, not `show`.

```rust
use crate::error::CliError;
use crate::utils::{format_timestamp, open_database, load_configuration, GlobalOptions};
use clap::Args;
use trop::{Port, port::occupancy::{SystemOccupancyChecker, OccupancyCheckConfig}};

/// Display information about a specific port.
#[derive(Args)]
pub struct PortInfoCommand {
    /// Port number to query
    #[arg(value_name = "PORT")]
    pub port: u16,

    /// Include occupancy information
    #[arg(long)]
    pub include_occupancy: bool,
}

impl PortInfoCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Parse port
        let port = Port::try_from(self.port)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        // 2. Open database and query
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Find reservation for this port (need to add this method)
        let reservation = db.get_reservation_by_port(port)
            .map_err(|e| CliError::Library(e))?;

        // 4. Display reservation info
        if let Some(res) = reservation {
            println!("Port: {}", res.port());
            println!("Path: {}", res.key().path().display());
            if let Some(tag) = res.key().tag() {
                println!("Tag: {}", tag);
            }
            if let Some(project) = res.project() {
                println!("Project: {}", project);
            }
            if let Some(task) = res.task() {
                println!("Task: {}", task);
            }
            println!("Created: {}", format_timestamp(res.created_at()));
            println!("Last used: {}", format_timestamp(res.last_used_at()));

            // Check if path exists
            let path_exists = res.key().path().exists();
            println!("Path exists: {}", if path_exists { "yes" } else { "no" });
        } else {
            println!("Port {} is not reserved", port);
        }

        // 5. Check occupancy if requested
        if self.include_occupancy {
            println!();
            println!("Occupancy status:");

            let checker = SystemOccupancyChecker;
            let check_config = OccupancyCheckConfig::default();

            match checker.is_occupied(port, &check_config) {
                Ok(occupied) => {
                    if occupied {
                        println!("  Port is currently in use");
                    } else {
                        println!("  Port is available");
                    }
                }
                Err(e) => {
                    println!("  Unable to check occupancy: {}", e);
                }
            }
        }

        Ok(())
    }
}
```

#### 2.2 ShowDataDir Command (`trop-cli/src/commands/show_data_dir.rs`)

```rust
use crate::error::CliError;
use crate::utils::{resolve_data_dir, GlobalOptions};
use clap::Args;

/// Show the resolved data directory path.
#[derive(Args)]
pub struct ShowDataDirCommand {}

impl ShowDataDirCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Resolve data directory using same logic as other commands
        let data_dir = global.data_dir.clone()
            .unwrap_or_else(|| resolve_data_dir());

        println!("{}", data_dir.display());
        Ok(())
    }
}
```

#### 2.3 ShowPath Command (`trop-cli/src/commands/show_path.rs`)

```rust
use crate::error::CliError;
use crate::utils::{normalize_path, resolve_path, GlobalOptions};
use clap::Args;
use std::path::PathBuf;

/// Show the resolved path that would be used for a reservation.
#[derive(Args)]
pub struct ShowPathCommand {
    /// Path to resolve
    #[arg(long, value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Explicitly request canonicalization
    #[arg(long)]
    pub canonicalize: bool,
}

impl ShowPathCommand {
    pub fn execute(self, _global: &GlobalOptions) -> Result<(), CliError> {
        // Get path using existing resolution logic
        let path = resolve_path(self.path)?;

        // Normalize or canonicalize as requested
        let resolved = if self.canonicalize {
            path.canonicalize()
                .map_err(|e| CliError::Io(e))?
        } else {
            normalize_path(&path)?
        };

        println!("{}", resolved.display());
        Ok(())
    }
}
```

### Part 3: Scanning Commands

#### 3.1 Scan Command (`trop-cli/src/commands/scan.rs`)

```rust
use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::{Args, ValueEnum};
use trop::{Port, PortRange, port::occupancy::{SystemOccupancyChecker, OccupancyCheckConfig}};
use trop::config::{Config, PortExclusion};
use std::path::Path;

/// Scan port range for occupied ports.
#[derive(Args)]
pub struct ScanCommand {
    /// Minimum port (uses config if not specified)
    #[arg(long)]
    pub min: Option<u16>,

    /// Maximum port (uses config if not specified)
    #[arg(long)]
    pub max: Option<u16>,

    /// Automatically add occupied, unreserved ports to exclusion list
    #[arg(long)]
    pub autoexclude: bool,

    /// Automatically compact exclusions after adding
    #[arg(long)]
    pub autocompact: bool,

    /// Output format
    #[arg(long, value_enum, default_value = "table")]
    pub format: ScanOutputFormat,

    // Occupancy check options
    #[arg(long)]
    pub skip_tcp: bool,

    #[arg(long)]
    pub skip_udp: bool,

    #[arg(long)]
    pub skip_ipv4: bool,

    #[arg(long)]
    pub skip_ipv6: bool,

    #[arg(long)]
    pub check_all_interfaces: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ScanOutputFormat {
    Table,
    Json,
    Csv,
    Tsv,
}

impl ScanCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration and determine port range
        let mut config = load_configuration(global)?;
        let range = self.determine_range(&config)?;

        // 2. Open database
        let db = open_database(global, &config)?;

        // 3. Scan for occupied ports
        let checker = SystemOccupancyChecker;
        let check_config = OccupancyCheckConfig {
            skip_tcp: self.skip_tcp,
            skip_udp: self.skip_udp,
            skip_ipv4: self.skip_ipv4,
            skip_ipv6: self.skip_ipv6,
            check_all_interfaces: self.check_all_interfaces,
        };

        // Note: find_occupied_ports needs to be implemented in PortOccupancyChecker trait
        let occupied_ports = checker.find_occupied_ports(&range, &check_config)
            .map_err(|e| CliError::Library(e))?;

        // 4. Get reserved ports from database (need to add this method)
        let reserved_ports = db.get_reserved_ports_in_range(&range)
            .map_err(|e| CliError::Library(e))?;

        // 5. Find unreserved occupied ports
        let unreserved_occupied: Vec<Port> = occupied_ports.iter()
            .filter(|p| !reserved_ports.contains(p))
            .copied()
            .collect();

        // 6. Auto-exclude if requested
        if self.autoexclude && !unreserved_occupied.is_empty() {
            self.add_exclusions(&mut config, &unreserved_occupied, global)?;

            if self.autocompact {
                self.compact_exclusions(&mut config, global)?;
            }
        }

        // 7. Format and output results
        self.output_results(&occupied_ports, &reserved_ports, &unreserved_occupied)?;

        Ok(())
    }

    fn determine_range(&self, config: &Config) -> Result<PortRange, CliError> {
        let min = self.min
            .or(config.ports.as_ref().and_then(|p| Some(p.min)))
            .unwrap_or(5000);
        let max = self.max
            .or(config.ports.as_ref().and_then(|p| p.max))
            .unwrap_or(7000);

        let min_port = Port::try_from(min)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;
        let max_port = Port::try_from(max)
            .map_err(|e| CliError::InvalidArguments(e.to_string()))?;

        PortRange::new(min_port, max_port)
            .map_err(|e| CliError::Library(e))
    }

    fn add_exclusions(&self, config: &mut Config, ports: &[Port], global: &GlobalOptions) -> Result<(), CliError> {
        use crate::utils::{find_project_config, resolve_data_dir};

        // Determine target config file (project or global)
        let config_path = find_project_config()?.unwrap_or_else(|| {
            resolve_data_dir().join("config.yaml")
        });

        // Ensure excluded_ports exists
        if config.excluded_ports.is_none() {
            config.excluded_ports = Some(Vec::new());
        }

        // Add new exclusions
        if let Some(ref mut exclusions) = config.excluded_ports {
            for port in ports {
                let exclusion = PortExclusion::Single(port.value());
                if !exclusions.contains(&exclusion) {
                    exclusions.push(exclusion);
                }
            }
        }

        // Save config
        let yaml = serde_yaml::to_string(config)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {}", e)))?;
        std::fs::write(&config_path, yaml)?;

        if !global.quiet {
            println!("Added {} exclusions to {}", ports.len(), config_path.display());
        }

        Ok(())
    }

    fn compact_exclusions(&self, config: &mut Config, global: &GlobalOptions) -> Result<(), CliError> {
        use crate::commands::compact_exclusions::compact_exclusion_list;

        if let Some(ref mut exclusions) = config.excluded_ports {
            let original_count = exclusions.len();
            let compacted = compact_exclusion_list(exclusions);

            if original_count != compacted.len() {
                *exclusions = compacted;

                // Save compacted config
                let config_path = find_project_config()?.unwrap_or_else(|| {
                    resolve_data_dir().join("config.yaml")
                });

                let yaml = serde_yaml::to_string(config)
                    .map_err(|e| CliError::Config(format!("Failed to serialize config: {}", e)))?;
                std::fs::write(&config_path, yaml)?;

                if !global.quiet {
                    println!("Compacted {} exclusions to {}", original_count, exclusions.len());
                }
            }
        }

        Ok(())
    }

    fn output_results(&self, occupied: &[Port], reserved: &[Port], unreserved: &[Port]) -> Result<(), CliError> {
        use crate::output::{OutputFormatter, OutputRecord};

        #[derive(Serialize)]
        struct ScanResult {
            port: u16,
            status: String,
            reserved: bool,
        }

        let mut results = Vec::new();

        for port in occupied {
            let is_reserved = reserved.contains(port);
            results.push(ScanResult {
                port: port.value(),
                status: if is_reserved { "occupied (reserved)" } else { "occupied" }.to_string(),
                reserved: is_reserved,
            });
        }

        // Format based on requested output format
        match self.format {
            ScanOutputFormat::Table => {
                use prettytable::{Table, row};
                let mut table = Table::new();
                table.add_row(row!["Port", "Status"]);
                for result in &results {
                    table.add_row(row![result.port, result.status]);
                }
                table.printstd();
            }
            ScanOutputFormat::Json => {
                let json = serde_json::to_string_pretty(&results)
                    .map_err(|e| CliError::Config(format!("JSON serialization failed: {}", e)))?;
                println!("{}", json);
            }
            ScanOutputFormat::Csv => {
                for result in &results {
                    println!("{},{}", result.port, result.status);
                }
            }
            ScanOutputFormat::Tsv => {
                for result in &results {
                    println!("{}\t{}", result.port, result.status);
                }
            }
        }

        if !unreserved.is_empty() {
            println!();
            println!("Found {} unreserved occupied port(s)", unreserved.len());
        }

        Ok(())
    }
}
```

### Part 4: Configuration Commands

#### 4.1 Validate Command (`trop-cli/src/commands/validate.rs`)

```rust
use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::Args;
use std::path::PathBuf;
use trop::config::{Config, ConfigValidator};

/// Validate a trop configuration file.
#[derive(Args)]
pub struct ValidateCommand {
    /// Configuration file to validate
    #[arg(value_name = "CONFIG_PATH")]
    pub config_path: PathBuf,
}

impl ValidateCommand {
    pub fn execute(self, _global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Check file exists
        if !self.config_path.exists() {
            return Err(CliError::InvalidArguments(
                format!("File not found: {}", self.config_path.display())
            ));
        }

        // 2. Determine file type (trop.yaml vs config.yaml)
        let filename = self.config_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let is_tropfile = filename == "trop.yaml" || filename == "trop.local.yaml";

        // 3. Parse the file
        let contents = std::fs::read_to_string(&self.config_path)?;
        let config: Config = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Parse error: {}", e);
                return Err(CliError::SemanticFailure("Configuration file is invalid".to_string()));
            }
        };

        // 4. Validate the configuration (ConfigValidator already exists)
        match ConfigValidator::validate(&config, is_tropfile) {
            Ok(()) => {
                println!("Configuration is valid");
                Ok(())
            }
            Err(e) => {
                eprintln!("Validation error: {}", e);
                Err(CliError::SemanticFailure("Configuration validation failed".to_string()))
            }
        }
    }
}
```

#### 4.2 Exclude Command (`trop-cli/src/commands/exclude.rs`)

```rust
use crate::error::CliError;
use crate::utils::{load_configuration, find_project_config, resolve_data_dir, open_database, GlobalOptions};
use clap::Args;
use trop::config::{Config, PortExclusion};
use trop::{Port, PortRange};
use std::path::Path;

/// Add port or range to exclusion list.
#[derive(Args)]
pub struct ExcludeCommand {
    /// Port or port range to exclude (e.g., "8080" or "8080..8090")
    #[arg(value_name = "PORT_OR_RANGE")]
    pub port_or_range: String,

    /// Add to global config instead of project config
    #[arg(long)]
    pub global: bool,

    /// Force exclusion even if port is reserved
    #[arg(long)]
    pub force: bool,
}

impl ExcludeCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Parse port or range
        let exclusion = self.parse_exclusion()?;

        // 2. Load configuration and database
        let config = load_configuration(global)?;
        let db = open_database(global, &config)?;

        // 3. Check if any ports are reserved (unless --force)
        if !self.force {
            self.check_reserved(&db, &exclusion)?;
        }

        // 4. Determine target config file
        let config_path = if self.global {
            resolve_data_dir().join("config.yaml")
        } else {
            find_project_config()?.ok_or_else(|| {
                CliError::Config("No trop.yaml found in current directory or parents".into())
            })?
        };

        // 5. Load, modify, and save configuration
        let mut file_config = self.load_config_file(&config_path)?;
        self.add_exclusion(&mut file_config, exclusion)?;
        self.save_config_file(&config_path, &file_config)?;

        println!("Added exclusion to {}", config_path.display());
        Ok(())
    }

    fn parse_exclusion(&self) -> Result<PortExclusion, CliError> {
        // Parse "8080" or "8080..8090" format
        if let Some(separator_pos) = self.port_or_range.find("..") {
            // Range format
            let min_str = &self.port_or_range[..separator_pos];
            let max_str = &self.port_or_range[separator_pos + 2..];

            let min = min_str.parse::<u16>()
                .map_err(|_| CliError::InvalidArguments("Invalid port number".into()))?;
            let max = max_str.parse::<u16>()
                .map_err(|_| CliError::InvalidArguments("Invalid port number".into()))?;

            Ok(PortExclusion::Range { start: min, end: max })
        } else {
            // Single port
            let port = self.port_or_range.parse::<u16>()
                .map_err(|_| CliError::InvalidArguments("Invalid port number".into()))?;
            Ok(PortExclusion::Single(port))
        }
    }

    fn check_reserved(&self, db: &Database, exclusion: &PortExclusion) -> Result<(), CliError> {
        // Check if any ports in the exclusion are reserved
        let ports_to_check = match exclusion {
            PortExclusion::Single(p) => vec![*p],
            PortExclusion::Range { start, end } => (*start..=*end).collect(),
        };

        for port_value in ports_to_check {
            if let Ok(port) = Port::try_from(port_value) {
                if db.is_port_reserved(port).unwrap_or(false) {
                    return Err(CliError::InvalidArguments(
                        format!("Port {} is reserved. Use --force to override.", port_value)
                    ));
                }
            }
        }

        Ok(())
    }

    fn load_config_file(&self, path: &Path) -> Result<Config, CliError> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            serde_yaml::from_str(&contents)
                .map_err(|e| CliError::Config(format!("Failed to parse config: {}", e)))
        } else {
            Ok(Config::default())
        }
    }

    fn add_exclusion(&self, config: &mut Config, exclusion: PortExclusion) -> Result<(), CliError> {
        if config.excluded_ports.is_none() {
            config.excluded_ports = Some(Vec::new());
        }

        if let Some(ref mut exclusions) = config.excluded_ports {
            // Check for duplicates
            if !exclusions.contains(&exclusion) {
                exclusions.push(exclusion);
            }
        }

        Ok(())
    }

    fn save_config_file(&self, path: &Path, config: &Config) -> Result<(), CliError> {
        // Note: YAML comments will be lost during this process
        // This is a known limitation documented in the plan
        let yaml = serde_yaml::to_string(config)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {}", e)))?;
        std::fs::write(path, yaml)?;
        Ok(())
    }
}
```

#### 4.3 CompactExclusions Command (`trop-cli/src/commands/compact_exclusions.rs`)

```rust
use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::Args;
use std::path::PathBuf;
use trop::config::{Config, PortExclusion};

/// Compact exclusion list to minimal representation.
#[derive(Args)]
pub struct CompactExclusionsCommand {
    /// Configuration file path
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Dry run (show changes without applying)
    #[arg(long)]
    pub dry_run: bool,
}

impl CompactExclusionsCommand {
    pub fn execute(self, _global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration
        if !self.path.exists() {
            return Err(CliError::InvalidArguments(
                format!("File not found: {}", self.path.display())
            ));
        }

        let contents = std::fs::read_to_string(&self.path)?;
        let mut config: Config = serde_yaml::from_str(&contents)
            .map_err(|e| CliError::Config(format!("Parse error: {}", e)))?;

        // 2. Compact exclusions
        if let Some(ref mut exclusions) = config.excluded_ports {
            let original_count = exclusions.len();
            let compacted = compact_exclusion_list(exclusions);
            let new_count = compacted.len();

            if original_count != new_count {
                println!("Compacted {} exclusions to {}", original_count, new_count);

                if !self.dry_run {
                    *exclusions = compacted;

                    // 3. Save configuration (YAML comments will be lost)
                    let yaml = serde_yaml::to_string(&config)
                        .map_err(|e| CliError::Config(format!("Serialize error: {}", e)))?;
                    std::fs::write(&self.path, yaml)?;
                    println!("Updated {}", self.path.display());
                } else {
                    println!("Dry run - no changes made");
                    println!("Would save: {:?}", compacted);
                }
            } else {
                println!("Exclusions already optimal");
            }
        } else {
            println!("No exclusions to compact");
        }

        Ok(())
    }
}

/// Compact a list of port exclusions to minimal representation.
pub fn compact_exclusion_list(exclusions: &[PortExclusion]) -> Vec<PortExclusion> {
    use std::collections::BTreeSet;

    // Collect all excluded ports
    let mut ports = BTreeSet::new();
    for exclusion in exclusions {
        match exclusion {
            PortExclusion::Single(p) => {
                ports.insert(*p);
            }
            PortExclusion::Range { start, end } => {
                for p in *start..=*end {
                    ports.insert(p);
                }
            }
        }
    }

    // Build minimal ranges
    let mut result = Vec::new();
    let mut current_start: Option<u16> = None;
    let mut current_end: Option<u16> = None;

    for &port in &ports {
        match (current_start, current_end) {
            (None, None) => {
                current_start = Some(port);
                current_end = Some(port);
            }
            (Some(start), Some(end)) => {
                if port == end + 1 {
                    // Extend current range
                    current_end = Some(port);
                } else {
                    // Save current range and start new one
                    if start == end {
                        result.push(PortExclusion::Single(start));
                    } else {
                        result.push(PortExclusion::Range { start, end });
                    }
                    current_start = Some(port);
                    current_end = Some(port);
                }
            }
            _ => unreachable!(),
        }
    }

    // Save final range
    if let (Some(start), Some(end)) = (current_start, current_end) {
        if start == end {
            result.push(PortExclusion::Single(start));
        } else {
            result.push(PortExclusion::Range { start, end });
        }
    }

    result
}
```

### Part 5: Database Extensions

Add new query methods to `trop/src/database/operations.rs`:

```rust
// Add to existing SQL constants
const SELECT_BY_PORT: &str = r"
    SELECT path, tag, port, project, task, created_at, last_used_at
    FROM reservations
    WHERE port = ?
";

const GET_RESERVED_PORTS_IN_RANGE: &str = r"
    SELECT DISTINCT port
    FROM reservations
    WHERE port >= ? AND port <= ?
    ORDER BY port
";

impl Database {
    /// Get reservation by port number.
    pub fn get_reservation_by_port(&self, port: Port) -> Result<Option<Reservation>> {
        let mut stmt = self.conn.prepare_cached(SELECT_BY_PORT)?;
        let mut rows = stmt.query_map(params![port.value()], row_to_reservation)?;

        match rows.next() {
            Some(Ok(reservation)) => Ok(Some(reservation)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Get all reserved ports in a range.
    pub fn get_reserved_ports_in_range(&self, range: &PortRange) -> Result<Vec<Port>> {
        let mut stmt = self.conn.prepare_cached(GET_RESERVED_PORTS_IN_RANGE)?;
        let rows = stmt.query_map(
            params![range.min().value(), range.max().value()],
            |row| {
                let port_value: u16 = row.get(0)?;
                Ok(Port::try_from(port_value).unwrap())
            },
        )?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.into())
    }

    /// Verify database integrity using PRAGMA integrity_check.
    /// This is compatible with existing transaction patterns as it's a read-only operation.
    pub fn verify_integrity(&mut self) -> Result<()> {
        let result: String = self.conn.query_row(
            "PRAGMA integrity_check",
            [],
            |row| row.get(0),
        )?;

        if result == "ok" {
            Ok(())
        } else {
            Err(Error::Database {
                message: format!("Integrity check failed: {}", result),
                source: None,
            })
        }
    }
}
```

### Part 6: Port Occupancy Extensions

Add to `trop/src/port/occupancy.rs`:

```rust
impl PortOccupancyChecker for SystemOccupancyChecker {
    // ... existing is_occupied method ...

    /// Find all occupied ports in a range.
    fn find_occupied_ports(&self, range: &PortRange, config: &OccupancyCheckConfig) -> Result<Vec<Port>> {
        let mut occupied = Vec::new();

        for port_value in range.min().value()..=range.max().value() {
            if let Ok(port) = Port::try_from(port_value) {
                if self.is_occupied(port, config)? {
                    occupied.push(port);
                }
            }
        }

        Ok(occupied)
    }
}
```

### Part 7: Utility Functions

Add new utility functions to `trop-cli/src/utils.rs`:

```rust
/// Find project configuration file (trop.yaml) starting from current directory.
pub fn find_project_config() -> Result<Option<PathBuf>, CliError> {
    let mut current = std::env::current_dir()?;

    loop {
        // Check for trop.local.yaml first (higher precedence)
        let local_config = current.join("trop.local.yaml");
        if local_config.exists() {
            return Ok(Some(local_config));
        }

        // Check for trop.yaml
        let config = current.join("trop.yaml");
        if config.exists() {
            return Ok(Some(config));
        }

        // Move up one directory
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Ok(None)
}
```

### Part 8: CLI Integration

Update `trop-cli/src/cli.rs` to add new commands:

```rust
#[derive(Subcommand)]
pub enum Command {
    // ... existing commands ...

    /// Assert that a reservation exists for a path/tag
    AssertReservation(AssertReservationCommand),

    /// Assert that a specific port is reserved
    AssertPort(AssertPortCommand),

    /// Assert that the data directory exists and is valid
    AssertDataDir(AssertDataDirCommand),

    /// Display information about a specific port
    #[command(name = "port-info")]
    PortInfo(PortInfoCommand),

    /// Show the resolved data directory path
    ShowDataDir(ShowDataDirCommand),

    /// Show the resolved path for a reservation
    ShowPath(ShowPathCommand),

    /// Scan port range for occupied ports
    Scan(ScanCommand),

    /// Validate a configuration file
    Validate(ValidateCommand),

    /// Add port or range to exclusion list
    Exclude(ExcludeCommand),

    /// Compact exclusion list to minimal representation
    CompactExclusions(CompactExclusionsCommand),
}
```

Update `trop-cli/src/main.rs` to handle new commands:

```rust
let result = match cli.command {
    // ... existing commands ...
    cli::Command::AssertReservation(cmd) => cmd.execute(&global),
    cli::Command::AssertPort(cmd) => cmd.execute(&global),
    cli::Command::AssertDataDir(cmd) => cmd.execute(&global),
    cli::Command::PortInfo(cmd) => cmd.execute(&global),
    cli::Command::ShowDataDir(cmd) => cmd.execute(&global),
    cli::Command::ShowPath(cmd) => cmd.execute(&global),
    cli::Command::Scan(cmd) => cmd.execute(&global),
    cli::Command::Validate(cmd) => cmd.execute(&global),
    cli::Command::Exclude(cmd) => cmd.execute(&global),
    cli::Command::CompactExclusions(cmd) => cmd.execute(&global),
};
```

## Implementation Order

1. **Foundation** (Parts 0, 5, 6, 7, 8):
   - Add `SemanticFailure` error variant
   - Add database query methods
   - Add port occupancy extensions
   - Add utility functions
   - Update CLI structure

2. **Information Commands** (Part 2):
   - `show-data-dir` (simplest)
   - `show-path`
   - `port-info`

3. **Assertion Commands** (Part 1):
   - `assert-data-dir`
   - `assert-port`
   - `assert-reservation`

4. **Configuration Commands** (Part 4):
   - `validate`
   - `exclude`
   - `compact-exclusions`

5. **Scanning Command** (Part 3):
   - `scan` (most complex, builds on all previous work)

6. **Testing**:
   - Unit tests for each command
   - Integration tests for end-to-end scenarios

## Key Design Decisions

1. **Exit Codes**: Assertion commands use `CliError::SemanticFailure` for failed assertions (exit code 1), maintaining compatibility with shell scripting and CI/CD.

2. **Path Handling**: Use existing `resolve_path` and `normalize_path` utilities consistently across all commands.

3. **Database Extensions**: New query methods are added to the existing `Database` struct, maintaining consistency with the existing codebase.

4. **Configuration Mutation**: The `exclude` and `compact-exclusions` commands use YAML parsing/serialization. **YAML comments will be lost** - this is a known limitation that should be documented.

5. **Port Scanning**: The `scan` command leverages the existing `PortOccupancyChecker` trait, adding the `find_occupied_ports` method for efficiency.

6. **Command Naming**: Using `port-info` as specified (not `show`).

## Testing Requirements

### Unit Tests

Each command module should include unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use trop::database::test_util::TestDatabase;

    #[test]
    fn test_assert_reservation_exists() {
        // Test successful assertion
    }

    #[test]
    fn test_assert_reservation_not_exists() {
        // Test failed assertion returns SemanticFailure
    }

    #[test]
    fn test_assert_reservation_inverted() {
        // Test --not flag
    }
}
```

### Integration Tests

Add integration tests in `trop-cli/tests/`:

```rust
// trop-cli/tests/assert_commands.rs
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_assert_reservation_command() {
    let temp = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("trop").unwrap();

    // First create a reservation
    cmd.arg("reserve")
        .arg("--data-dir").arg(temp.path())
        .arg("--path").arg("/test/path")
        .assert()
        .success();

    // Then assert it exists
    let mut cmd = Command::cargo_bin("trop").unwrap();
    cmd.arg("assert-reservation")
        .arg("--data-dir").arg(temp.path())
        .arg("--path").arg("/test/path")
        .assert()
        .success();

    // Assert non-existent fails with exit code 1
    let mut cmd = Command::cargo_bin("trop").unwrap();
    cmd.arg("assert-reservation")
        .arg("--data-dir").arg(temp.path())
        .arg("--path").arg("/other/path")
        .assert()
        .code(1);
}
```

## Success Criteria

- All commands execute successfully with appropriate exit codes
- Assertion commands enable automation and CI/CD workflows
- Configuration validation catches all spec-defined errors
- Port scanning accurately identifies occupied ports
- Exclusion management correctly modifies configuration files
- All commands have comprehensive test coverage
- Output formats are consistent with existing commands
- Error messages are clear and actionable

## Notes for Implementation

- Use existing error types and extend only where necessary
- Maintain consistency with existing command patterns
- Ensure all commands respect global options (verbose, quiet, data-dir)
- Follow the project's established testing patterns
- Consider performance for the `scan` command on large port ranges
- Document that YAML comments are lost when modifying config files
- Ensure thread-safety for all database operations