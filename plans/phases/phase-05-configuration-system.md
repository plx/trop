# Phase 5: Configuration System - Detailed Implementation Plan

## Overview

This phase implements the hierarchical configuration system for trop, providing support for YAML configuration files with proper precedence handling, environment variable overrides, and configuration validation. The configuration system is fundamental for user customization and project-specific settings.

## Prerequisites

Completed phases:
- Phase 1: Core types (Port, PortRange, Reservation, ReservationKey)
- Phase 2: Database layer with SQLite support
- Phase 3: Path handling system with normalization and canonicalization
- Phase 4: Basic reservation operations with plan-execute pattern

## Success Criteria

Upon completion:
- Configuration files (trop.yaml, trop.local.yaml, config.yaml) can be parsed and validated
- Hierarchical precedence chain works correctly (CLI > env > trop.local.yaml > trop.yaml > user config > defaults)
- Environment variables (TROP_*) override configuration file values
- Excluded ports configuration supports ranges and individual ports
- Reservation groups can be defined and validated
- Configuration merging produces predictable results
- All configuration validation errors provide clear, actionable messages

## Module Structure

```
trop/src/
├── config/
│   ├── mod.rs           # Module exports and high-level types
│   ├── schema.rs        # Configuration schema definitions
│   ├── loader.rs        # File discovery and loading logic
│   ├── merger.rs        # Configuration merging and precedence
│   ├── validator.rs     # Validation logic for all config fields
│   ├── environment.rs   # Environment variable handling
│   └── builder.rs       # Configuration builder pattern
```

## Task Breakdown

### Task 1: Define Configuration Schema

**Objective**: Create the core configuration data structures with serde support.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/mod.rs`
- `/Users/prb/github/trop/trop/src/config/schema.rs`

**Implementation Details**:

1. Create schema types in `schema.rs`:

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::port::{Port, PortRange};

/// Complete configuration structure
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Project identifier (only valid in trop.yaml)
    pub project: Option<String>,

    /// Port allocation settings
    pub ports: Option<PortConfig>,

    /// Excluded ports list
    pub excluded_ports: Option<Vec<PortExclusion>>,

    /// Cleanup settings
    pub cleanup: Option<CleanupConfig>,

    /// Occupancy check settings
    pub occupancy_check: Option<OccupancyConfig>,

    /// Batch reservation groups (only valid in trop.yaml)
    pub reservations: Option<ReservationGroup>,

    /// Behavioral flags
    pub disable_autoinit: Option<bool>,
    pub disable_autoprune: Option<bool>,
    pub disable_autoexpire: Option<bool>,

    /// Permission flags
    pub allow_unrelated_path: Option<bool>,
    pub allow_change_project: Option<bool>,
    pub allow_change_task: Option<bool>,
    pub allow_change: Option<bool>,

    /// Timeout settings
    pub maximum_lock_wait_seconds: Option<u64>,

    /// Output format
    pub output_format: Option<OutputFormat>,
}

/// Port range configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortConfig {
    pub min: u16,
    pub max: Option<u16>,
    pub max_offset: Option<u16>,
}

/// Port exclusion (single port or range)
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PortExclusion {
    Single(u16),
    Range { start: u16, end: u16 }, // Format: "5000..5010"
}

/// Cleanup configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CleanupConfig {
    pub expire_after_days: Option<u32>,
}

/// Occupancy check configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OccupancyConfig {
    pub skip: Option<bool>,
    pub skip_ip4: Option<bool>,
    pub skip_ip6: Option<bool>,
    pub skip_tcp: Option<bool>,
    pub skip_udp: Option<bool>,
    pub check_all_interfaces: Option<bool>,
}

/// Reservation group definition
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReservationGroup {
    pub base: Option<u16>,
    pub services: HashMap<String, ServiceDefinition>,
}

/// Individual service in a reservation group
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceDefinition {
    pub offset: Option<u16>,
    pub preferred: Option<u16>,
    pub env: Option<String>,
}

/// Output format enum
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
    Csv,
    Tsv,
    Table,
}
```

2. Add custom deserialization for port ranges:
   - Support "5000..5010" string format
   - Parse into PortExclusion::Range variant
   - Validate range ordering (start <= end)

**Testing**:
- Unit tests for each struct deserialization
- Test invalid YAML rejection (unknown fields)
- Test optional field handling
- Test port range parsing edge cases

### Task 2: Implement Configuration Loading

**Objective**: Create file discovery and loading logic.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/loader.rs`

**Implementation Details**:

```rust
use std::path::{Path, PathBuf};
use std::fs;
use crate::config::schema::Config;
use crate::error::{Error, Result};

/// Configuration source with its precedence level
#[derive(Debug, Clone)]
pub struct ConfigSource {
    pub path: PathBuf,
    pub precedence: u8,
    pub config: Config,
}

/// Loads configuration from various sources
pub struct ConfigLoader {
    sources: Vec<ConfigSource>,
}

impl ConfigLoader {
    /// Discover and load all configuration files
    pub fn load_all(working_dir: &Path) -> Result<Vec<ConfigSource>> {
        let mut sources = Vec::new();

        // Load user config (~/.trop/config.yaml)
        if let Some(user_config) = Self::load_user_config()? {
            sources.push(user_config);
        }

        // Walk up directory tree looking for trop.yaml/trop.local.yaml
        let project_configs = Self::discover_project_configs(working_dir)?;
        sources.extend(project_configs);

        // Sort by precedence (higher precedence first)
        sources.sort_by_key(|s| std::cmp::Reverse(s.precedence));

        Ok(sources)
    }

    /// Load user configuration file
    fn load_user_config() -> Result<Option<ConfigSource>> {
        let config_path = Self::user_config_path()?;

        if !config_path.exists() {
            return Ok(None);
        }

        let config = Self::load_file(&config_path)?;
        Ok(Some(ConfigSource {
            path: config_path,
            precedence: 1, // Lowest precedence
            config,
        }))
    }

    /// Discover project configurations by walking up directories
    fn discover_project_configs(start_dir: &Path) -> Result<Vec<ConfigSource>> {
        let mut configs = Vec::new();
        let mut current = start_dir.to_path_buf();

        loop {
            // Check for trop.yaml
            let trop_yaml = current.join("trop.yaml");
            if trop_yaml.exists() {
                let config = Self::load_file(&trop_yaml)?;
                configs.push(ConfigSource {
                    path: trop_yaml,
                    precedence: 2,
                    config,
                });
            }

            // Check for trop.local.yaml (higher precedence)
            let trop_local = current.join("trop.local.yaml");
            if trop_local.exists() {
                let config = Self::load_file(&trop_local)?;
                configs.push(ConfigSource {
                    path: trop_local,
                    precedence: 3,
                    config,
                });
            }

            // Stop if we found configs or can't go up anymore
            if !configs.is_empty() || !current.pop() {
                break;
            }
        }

        Ok(configs)
    }

    /// Load and parse a YAML configuration file
    fn load_file(path: &Path) -> Result<Config> {
        let contents = fs::read_to_string(path)
            .map_err(|e| Error::Configuration {
                path: path.to_path_buf(),
                message: format!("Failed to read file: {}", e),
            })?;

        serde_yaml::from_str(&contents)
            .map_err(|e| Error::Configuration {
                path: path.to_path_buf(),
                message: format!("Invalid YAML: {}", e),
            })
    }

    /// Get user config directory path
    fn user_config_path() -> Result<PathBuf> {
        let data_dir = crate::database::default_data_dir()?;
        Ok(data_dir.join("config.yaml"))
    }
}
```

**Testing**:
- Test discovery with nested directory structures
- Test missing configuration files
- Test malformed YAML error handling
- Test precedence ordering

### Task 3: Implement Configuration Merging

**Objective**: Build the configuration merging logic respecting precedence.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/merger.rs`

**Implementation Details**:

```rust
use crate::config::schema::*;

/// Merges configuration sources according to precedence rules
pub struct ConfigMerger;

impl ConfigMerger {
    /// Merge multiple configuration sources into final config
    pub fn merge(sources: Vec<ConfigSource>) -> Config {
        let mut result = Config::default();

        // Process in reverse order (lowest to highest precedence)
        for source in sources.iter().rev() {
            Self::merge_into(&mut result, &source.config);
        }

        result
    }

    /// Merge source config into target (source overwrites target)
    fn merge_into(target: &mut Config, source: &Config) {
        // Simple fields - source overwrites if Some
        if source.project.is_some() {
            target.project = source.project.clone();
        }

        if source.disable_autoinit.is_some() {
            target.disable_autoinit = source.disable_autoinit;
        }

        // Similar for other boolean flags...

        // Merge ports config
        if let Some(ref source_ports) = source.ports {
            target.ports = Some(match &target.ports {
                Some(target_ports) => Self::merge_port_config(target_ports, source_ports),
                None => source_ports.clone(),
            });
        }

        // Merge excluded_ports (union of all exclusions)
        if let Some(ref source_excluded) = source.excluded_ports {
            match &mut target.excluded_ports {
                Some(target_excluded) => {
                    target_excluded.extend(source_excluded.clone());
                },
                None => {
                    target.excluded_ports = source.excluded_ports.clone();
                }
            }
        }

        // Merge cleanup config
        if let Some(ref source_cleanup) = source.cleanup {
            target.cleanup = Some(match &target.cleanup {
                Some(target_cleanup) => Self::merge_cleanup(target_cleanup, source_cleanup),
                None => source_cleanup.clone(),
            });
        }

        // Occupancy config - full replacement (not field-by-field)
        if source.occupancy_check.is_some() {
            target.occupancy_check = source.occupancy_check.clone();
        }

        // Reservation groups - don't merge, only replace
        if source.reservations.is_some() {
            target.reservations = source.reservations.clone();
        }
    }

    fn merge_port_config(target: &PortConfig, source: &PortConfig) -> PortConfig {
        PortConfig {
            min: source.min, // Always use source
            max: source.max.or(target.max),
            max_offset: source.max_offset.or(target.max_offset),
        }
    }

    fn merge_cleanup(target: &CleanupConfig, source: &CleanupConfig) -> CleanupConfig {
        CleanupConfig {
            expire_after_days: source.expire_after_days.or(target.expire_after_days),
        }
    }
}
```

**Key Decisions**:
- Simple fields: higher precedence completely overwrites
- Excluded ports: union of all sources (accumulative)
- Occupancy config: treated as atomic unit
- Reservation groups: no merging, only replacement

**Testing**:
- Test merging with multiple sources
- Test excluded_ports accumulation
- Test partial config merging
- Test precedence ordering

### Task 4: Environment Variable Support

**Objective**: Implement TROP_* environment variable mapping.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/environment.rs`

**Implementation Details**:

```rust
use std::env;
use crate::config::schema::*;
use crate::error::Result;

/// Handles environment variable overrides for configuration
pub struct EnvironmentConfig;

impl EnvironmentConfig {
    /// Apply environment variable overrides to config
    pub fn apply_overrides(config: &mut Config) -> Result<()> {
        // TROP_PROJECT
        if let Ok(project) = env::var("TROP_PROJECT") {
            config.project = Some(project);
        }

        // TROP_DISABLE_AUTOINIT
        if let Ok(val) = env::var("TROP_DISABLE_AUTOINIT") {
            config.disable_autoinit = Some(Self::parse_bool(&val)?);
        }

        // TROP_DISABLE_AUTOPRUNE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOPRUNE") {
            config.disable_autoprune = Some(Self::parse_bool(&val)?);
        }

        // TROP_DISABLE_AUTOEXPIRE
        if let Ok(val) = env::var("TROP_DISABLE_AUTOEXPIRE") {
            config.disable_autoexpire = Some(Self::parse_bool(&val)?);
        }

        // Port range from TROP_PORT_MIN and TROP_PORT_MAX
        let mut port_config = config.ports.clone().unwrap_or_default();
        if let Ok(min) = env::var("TROP_PORT_MIN") {
            port_config.min = min.parse().map_err(|_| Error::Validation {
                field: "TROP_PORT_MIN".into(),
                message: "Invalid port number".into(),
            })?;
            config.ports = Some(port_config.clone());
        }

        if let Ok(max) = env::var("TROP_PORT_MAX") {
            port_config.max = Some(max.parse().map_err(|_| Error::Validation {
                field: "TROP_PORT_MAX".into(),
                message: "Invalid port number".into(),
            })?);
            config.ports = Some(port_config);
        }

        // TROP_EXCLUDED_PORTS (comma-separated)
        if let Ok(excluded) = env::var("TROP_EXCLUDED_PORTS") {
            let exclusions = Self::parse_excluded_ports(&excluded)?;
            match &mut config.excluded_ports {
                Some(existing) => existing.extend(exclusions),
                None => config.excluded_ports = Some(exclusions),
            }
        }

        // TROP_EXPIRE_AFTER_DAYS
        if let Ok(days) = env::var("TROP_EXPIRE_AFTER_DAYS") {
            let days = days.parse().map_err(|_| Error::Validation {
                field: "TROP_EXPIRE_AFTER_DAYS".into(),
                message: "Must be a positive integer".into(),
            })?;

            config.cleanup = Some(config.cleanup.clone().unwrap_or_default());
            if let Some(ref mut cleanup) = config.cleanup {
                cleanup.expire_after_days = Some(days);
            }
        }

        // TROP_MAXIMUM_LOCK_WAIT_SECONDS
        if let Ok(seconds) = env::var("TROP_MAXIMUM_LOCK_WAIT_SECONDS") {
            config.maximum_lock_wait_seconds = Some(seconds.parse().map_err(|_| {
                Error::Validation {
                    field: "TROP_MAXIMUM_LOCK_WAIT_SECONDS".into(),
                    message: "Must be a positive integer".into(),
                }
            })?);
        }

        // Permission flags
        if let Ok(val) = env::var("TROP_ALLOW_UNRELATED_PATH") {
            config.allow_unrelated_path = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE_PROJECT") {
            config.allow_change_project = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE_TASK") {
            config.allow_change_task = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_ALLOW_CHANGE") {
            config.allow_change = Some(Self::parse_bool(&val)?);
        }

        // Occupancy check flags
        Self::apply_occupancy_overrides(config)?;

        Ok(())
    }

    fn apply_occupancy_overrides(config: &mut Config) -> Result<()> {
        let mut occupancy = config.occupancy_check.clone().unwrap_or_default();

        if let Ok(val) = env::var("TROP_SKIP_OCCUPANCY_CHECK") {
            occupancy.skip = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV4") {
            occupancy.skip_ip4 = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_SKIP_IPV6") {
            occupancy.skip_ip6 = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_SKIP_TCP") {
            occupancy.skip_tcp = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_SKIP_UDP") {
            occupancy.skip_udp = Some(Self::parse_bool(&val)?);
        }

        if let Ok(val) = env::var("TROP_CHECK_ALL_INTERFACES") {
            occupancy.check_all_interfaces = Some(Self::parse_bool(&val)?);
        }

        config.occupancy_check = Some(occupancy);
        Ok(())
    }

    fn parse_bool(s: &str) -> Result<bool> {
        match s.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(Error::Validation {
                field: "boolean".into(),
                message: format!("Invalid boolean value: {}", s),
            }),
        }
    }

    fn parse_excluded_ports(s: &str) -> Result<Vec<PortExclusion>> {
        let mut exclusions = Vec::new();

        for part in s.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some((start, end)) = part.split_once("..") {
                // Range format: "5000..5010"
                let start: u16 = start.parse().map_err(|_| Error::Validation {
                    field: "excluded_ports".into(),
                    message: format!("Invalid port in range: {}", start),
                })?;

                let end: u16 = end.parse().map_err(|_| Error::Validation {
                    field: "excluded_ports".into(),
                    message: format!("Invalid port in range: {}", end),
                })?;

                exclusions.push(PortExclusion::Range { start, end });
            } else {
                // Single port
                let port: u16 = part.parse().map_err(|_| Error::Validation {
                    field: "excluded_ports".into(),
                    message: format!("Invalid port: {}", part),
                })?;

                exclusions.push(PortExclusion::Single(port));
            }
        }

        Ok(exclusions)
    }
}
```

**Testing**:
- Test each environment variable mapping
- Test boolean parsing variations
- Test excluded ports parsing (single, range, multiple)
- Test invalid value error handling

### Task 5: Configuration Validation

**Objective**: Implement comprehensive validation for all configuration fields.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/validator.rs`

**Implementation Details**:

```rust
use crate::config::schema::*;
use crate::error::{Error, Result};
use crate::port::Port;

/// Validates configuration according to spec rules
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate a complete configuration
    pub fn validate(config: &Config, is_tropfile: bool) -> Result<()> {
        // Validate project field (only in trop.yaml)
        if let Some(ref project) = config.project {
            if !is_tropfile {
                return Err(Error::Validation {
                    field: "project".into(),
                    message: "project field is only valid in trop.yaml files".into(),
                });
            }
            Self::validate_identifier("project", project)?;
        }

        // Validate reservations (only in trop.yaml)
        if let Some(ref reservations) = config.reservations {
            if !is_tropfile {
                return Err(Error::Validation {
                    field: "reservations".into(),
                    message: "reservations field is only valid in trop.yaml files".into(),
                });
            }
            Self::validate_reservation_group(reservations)?;
        }

        // Validate port configuration
        if let Some(ref ports) = config.ports {
            Self::validate_port_config(ports)?;
        }

        // Validate excluded ports
        if let Some(ref excluded) = config.excluded_ports {
            Self::validate_excluded_ports(excluded)?;
        }

        // Validate cleanup config
        if let Some(ref cleanup) = config.cleanup {
            Self::validate_cleanup(cleanup)?;
        }

        // Validate lock timeout
        if let Some(timeout) = config.maximum_lock_wait_seconds {
            if timeout == 0 {
                return Err(Error::Validation {
                    field: "maximum_lock_wait_seconds".into(),
                    message: "Timeout must be greater than 0".into(),
                });
            }
        }

        Ok(())
    }

    /// Validate string identifiers (project, task, tags)
    fn validate_identifier(field: &str, value: &str) -> Result<()> {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot be empty or only whitespace".into(),
            });
        }

        // Additional checks for security/safety
        if trimmed.contains('\0') {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot contain null bytes".into(),
            });
        }

        if trimmed.len() > 255 {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot exceed 255 characters".into(),
            });
        }

        Ok(())
    }

    /// Validate port configuration
    fn validate_port_config(config: &PortConfig) -> Result<()> {
        // Validate min port
        Port::try_from(config.min).map_err(|_| Error::Validation {
            field: "ports.min".into(),
            message: format!("Invalid port number: {}", config.min),
        })?;

        // Validate max if present
        if let Some(max) = config.max {
            Port::try_from(max).map_err(|_| Error::Validation {
                field: "ports.max".into(),
                message: format!("Invalid port number: {}", max),
            })?;

            if max < config.min {
                return Err(Error::Validation {
                    field: "ports".into(),
                    message: "max must be >= min".into(),
                });
            }
        }

        // Validate max_offset if present
        if let Some(offset) = config.max_offset {
            if offset == 0 {
                return Err(Error::Validation {
                    field: "ports.max_offset".into(),
                    message: "max_offset must be > 0".into(),
                });
            }

            let computed_max = config.min.saturating_add(offset);
            Port::try_from(computed_max).map_err(|_| Error::Validation {
                field: "ports.max_offset".into(),
                message: format!("Offset would create invalid max port: {}", computed_max),
            })?;
        }

        // Can't have both max and max_offset
        if config.max.is_some() && config.max_offset.is_some() {
            return Err(Error::Validation {
                field: "ports".into(),
                message: "Cannot specify both max and max_offset".into(),
            });
        }

        Ok(())
    }

    /// Validate excluded ports list
    fn validate_excluded_ports(excluded: &[PortExclusion]) -> Result<()> {
        for (i, exclusion) in excluded.iter().enumerate() {
            match exclusion {
                PortExclusion::Single(port) => {
                    Port::try_from(*port).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{}]", i),
                        message: format!("Invalid port: {}", port),
                    })?;
                }
                PortExclusion::Range { start, end } => {
                    Port::try_from(*start).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{}]", i),
                        message: format!("Invalid start port: {}", start),
                    })?;

                    Port::try_from(*end).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{}]", i),
                        message: format!("Invalid end port: {}", end),
                    })?;

                    if end < start {
                        return Err(Error::Validation {
                            field: format!("excluded_ports[{}]", i),
                            message: format!("Invalid range: {}..{} (end < start)", start, end),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Validate cleanup configuration
    fn validate_cleanup(cleanup: &CleanupConfig) -> Result<()> {
        if let Some(days) = cleanup.expire_after_days {
            if days == 0 {
                return Err(Error::Validation {
                    field: "cleanup.expire_after_days".into(),
                    message: "Must be > 0".into(),
                });
            }
        }
        Ok(())
    }

    /// Validate reservation group
    fn validate_reservation_group(group: &ReservationGroup) -> Result<()> {
        // Validate base port if present
        if let Some(base) = group.base {
            Port::try_from(base).map_err(|_| Error::Validation {
                field: "reservations.base".into(),
                message: format!("Invalid port: {}", base),
            })?;
        }

        // Track uniqueness constraints
        let mut seen_offsets = std::collections::HashSet::new();
        let mut seen_preferred = std::collections::HashSet::new();
        let mut seen_env_vars = std::collections::HashSet::new();
        let mut has_default_offset = false;

        for (tag, service) in &group.services {
            // Validate tag
            Self::validate_identifier(&format!("reservations.services.{}", tag), tag)?;

            // Check offset uniqueness
            let offset = service.offset.unwrap_or(0);
            if offset == 0 && has_default_offset {
                return Err(Error::Validation {
                    field: format!("reservations.services.{}.offset", tag),
                    message: "Only one service can omit offset (default to 0)".into(),
                });
            }
            if offset == 0 {
                has_default_offset = true;
            }

            if !seen_offsets.insert(offset) {
                return Err(Error::Validation {
                    field: format!("reservations.services.{}.offset", tag),
                    message: format!("Duplicate offset: {}", offset),
                });
            }

            // Check preferred port uniqueness
            if let Some(preferred) = service.preferred {
                Port::try_from(preferred).map_err(|_| Error::Validation {
                    field: format!("reservations.services.{}.preferred", tag),
                    message: format!("Invalid port: {}", preferred),
                })?;

                if !seen_preferred.insert(preferred) {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{}.preferred", tag),
                        message: format!("Duplicate preferred port: {}", preferred),
                    });
                }
            }

            // Check env var uniqueness and validity
            if let Some(ref env) = service.env {
                if env.is_empty() {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{}.env", tag),
                        message: "Environment variable name cannot be empty".into(),
                    });
                }

                // Validate env var name (alphanumeric + underscore, starts with letter)
                if !env.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{}.env", tag),
                        message: "Invalid environment variable name".into(),
                    });
                }

                if !env.chars().next().unwrap().is_ascii_alphabetic() {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{}.env", tag),
                        message: "Environment variable must start with a letter".into(),
                    });
                }

                if !seen_env_vars.insert(env.clone()) {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{}.env", tag),
                        message: format!("Duplicate environment variable: {}", env),
                    });
                }
            }
        }

        Ok(())
    }
}
```

**Testing**:
- Test valid configurations pass
- Test each validation rule with invalid data
- Test uniqueness constraints in reservation groups
- Test error messages are clear and specific

### Task 6: Configuration Builder Pattern

**Objective**: Create a builder for constructing configurations programmatically.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/config/builder.rs`

**Implementation Details**:

```rust
use std::path::Path;
use crate::config::schema::*;
use crate::config::loader::ConfigLoader;
use crate::config::merger::ConfigMerger;
use crate::config::environment::EnvironmentConfig;
use crate::config::validator::ConfigValidator;
use crate::error::Result;

/// Builder for loading and constructing configuration
pub struct ConfigBuilder {
    working_dir: Option<PathBuf>,
    skip_env: bool,
    skip_files: bool,
    additional_config: Option<Config>,
}

impl ConfigBuilder {
    /// Create a new configuration builder
    pub fn new() -> Self {
        Self {
            working_dir: None,
            skip_env: false,
            skip_files: false,
            additional_config: None,
        }
    }

    /// Set the working directory for config discovery
    pub fn with_working_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Skip loading configuration files
    pub fn skip_files(mut self) -> Self {
        self.skip_files = true;
        self
    }

    /// Skip environment variable overrides
    pub fn skip_env(mut self) -> Self {
        self.skip_env = true;
        self
    }

    /// Add additional configuration to merge (highest precedence)
    pub fn with_config(mut self, config: Config) -> Self {
        self.additional_config = Some(config);
        self
    }

    /// Build the final configuration
    pub fn build(self) -> Result<Config> {
        let mut sources = Vec::new();

        // Load configuration files
        if !self.skip_files {
            let working_dir = self.working_dir
                .as_deref()
                .unwrap_or_else(|| Path::new("."));
            sources = ConfigLoader::load_all(working_dir)?;
        }

        // Add default configuration at lowest precedence
        sources.insert(0, ConfigSource {
            path: PathBuf::from("<defaults>"),
            precedence: 0,
            config: Self::default_config(),
        });

        // Merge all file-based configs
        let mut config = ConfigMerger::merge(sources);

        // Apply environment overrides
        if !self.skip_env {
            EnvironmentConfig::apply_overrides(&mut config)?;
        }

        // Apply additional config if provided
        if let Some(additional) = self.additional_config {
            ConfigMerger::merge_into(&mut config, &additional);
        }

        // Validate final configuration
        let is_tropfile = sources.iter()
            .any(|s| s.path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "trop.yaml" || n == "trop.local.yaml")
                .unwrap_or(false));

        ConfigValidator::validate(&config, is_tropfile)?;

        Ok(config)
    }

    /// Create default configuration
    fn default_config() -> Config {
        Config {
            project: None,
            ports: Some(PortConfig {
                min: 5000,
                max: Some(7000),
                max_offset: None,
            }),
            excluded_ports: None,
            cleanup: Some(CleanupConfig {
                expire_after_days: Some(30),
            }),
            occupancy_check: Some(OccupancyConfig {
                skip: Some(false),
                skip_ip4: Some(false),
                skip_ip6: Some(false),
                skip_tcp: Some(false),
                skip_udp: Some(false),
                check_all_interfaces: Some(false),
            }),
            reservations: None,
            disable_autoinit: Some(false),
            disable_autoprune: Some(false),
            disable_autoexpire: Some(false),
            allow_unrelated_path: Some(false),
            allow_change_project: Some(false),
            allow_change_task: Some(false),
            allow_change: Some(false),
            maximum_lock_wait_seconds: Some(5),
            output_format: Some(OutputFormat::Table),
        }
    }
}
```

**Testing**:
- Test builder with various combinations of options
- Test configuration precedence chain
- Test validation occurs after building
- Test default values are applied

### Task 7: Integration with Existing Code

**Objective**: Update existing modules to use the configuration system.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/lib.rs`
- `/Users/prb/github/trop/trop/src/database/config.rs`
- `/Users/prb/github/trop/trop/src/operations/reserve.rs`
- `/Users/prb/github/trop/trop/src/error.rs`

**Implementation Details**:

1. Update `error.rs` to add configuration-specific error variants:
```rust
#[derive(Debug, thiserror::Error)]
pub enum Error {
    // ... existing variants ...

    /// Configuration file error
    #[error("Configuration error in {path:?}: {message}")]
    Configuration {
        path: PathBuf,
        message: String,
    },
}
```

2. Update `lib.rs` to export configuration module:
```rust
pub mod config;

pub use config::{Config, ConfigBuilder};
```

3. Update `DatabaseConfig` to accept configuration:
```rust
impl From<&Config> for DatabaseConfig {
    fn from(config: &Config) -> Self {
        let timeout = config.maximum_lock_wait_seconds
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_millis(5000));

        Self {
            // ... existing fields ...
            busy_timeout: timeout,
        }
    }
}
```

4. Update reservation operations to respect configuration:
   - Check `allow_unrelated_path` flag
   - Check `allow_change_project` and `allow_change_task`
   - Use configured port range
   - Respect excluded ports

**Testing**:
- Integration tests with real configuration files
- Test configuration affects operation behavior
- Test error propagation

### Task 8: Comprehensive Testing

**Objective**: Create thorough test coverage for the configuration system.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/config_integration.rs`
- Test fixtures in `/Users/prb/github/trop/trop/tests/fixtures/configs/`

**Test Scenarios**:

1. **File Discovery Tests**:
   - Test upward directory traversal
   - Test stopping at first config found
   - Test trop.local.yaml precedence over trop.yaml

2. **Merging Tests**:
   - Test complete precedence chain
   - Test excluded_ports accumulation
   - Test partial config merging

3. **Environment Variable Tests**:
   - Test each TROP_* variable
   - Test env vars override file configs
   - Test invalid env var values

4. **Validation Tests**:
   - Test all validation rules
   - Test helpful error messages
   - Test edge cases (empty strings, max values)

5. **YAML Parsing Tests**:
   - Test port range formats ("5000..5010")
   - Test unknown fields rejection
   - Test malformed YAML errors

6. **Reservation Group Tests**:
   - Test offset uniqueness
   - Test preferred port validation
   - Test env var name validation

7. **Integration Tests**:
   - Test with real file system
   - Test with actual environment variables
   - Test complete workflow from files to final config

**Test Fixtures**:
```
tests/fixtures/configs/
├── valid/
│   ├── minimal.yaml
│   ├── complete.yaml
│   ├── with_reservations.yaml
│   └── with_exclusions.yaml
├── invalid/
│   ├── bad_port_range.yaml
│   ├── duplicate_offset.yaml
│   ├── unknown_field.yaml
│   └── invalid_project.yaml
└── hierarchy/
    ├── parent/
    │   └── trop.yaml
    └── child/
        ├── trop.yaml
        └── trop.local.yaml
```

## Error Handling Strategy

All configuration errors should:
- Clearly identify the source (file path, env var name)
- Explain what's wrong
- Suggest how to fix it
- Use appropriate error variants

Example error messages:
- "Configuration error in /path/to/trop.yaml: Invalid port range 7000..5000 (end < start)"
- "Environment variable TROP_PORT_MIN: Invalid port number: 70000"
- "Configuration error: project field is only valid in trop.yaml files"

## Performance Considerations

- Cache parsed configurations when possible
- Use lazy evaluation for expensive validations
- Minimize file system access during discovery
- Consider using `once_cell` for singleton config

## Security Considerations

- Validate all string inputs for path traversal attempts
- Sanitize identifiers to prevent injection attacks
- Limit configuration file size to prevent DoS
- Never log sensitive configuration values

## Migration Path

Since this is a new feature:
1. Existing code continues to work with defaults
2. Configuration is opt-in initially
3. Future phases will integrate more deeply
4. Document configuration options thoroughly

## Deliverables Checklist

- [ ] Configuration schema types with serde
- [ ] File discovery and loading
- [ ] Hierarchical merging logic
- [ ] Environment variable support
- [ ] Comprehensive validation
- [ ] Builder pattern implementation
- [ ] Integration with existing modules
- [ ] Complete test coverage
- [ ] Documentation and examples

## Next Phase Preparation

Phase 6 (Port Allocation & Occupancy) will need:
- Access to excluded_ports configuration
- Port range configuration
- Occupancy check settings
- Auto-cleanup flags

Ensure configuration module provides easy access to these values.

## Implementation Notes

**For the Implementer**:
- Start with schema definition and work outward
- Build validation incrementally with tests
- Use property-based testing for validators where appropriate
- Keep error messages user-friendly
- Consider future extensibility in design
- Document all public APIs thoroughly
- Make configuration immutable after construction
- Use type-safe builders where possible

This plan provides a complete roadmap for implementing the configuration system. Each component has clear responsibilities, and the testing strategy ensures reliability.