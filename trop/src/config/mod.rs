//! Configuration system for trop.
//!
//! This module provides hierarchical configuration with support for:
//! - YAML configuration files (user config and project tropfiles)
//! - Environment variable overrides
//! - Programmatic configuration via builder pattern
//! - Comprehensive validation
//!
//! # Configuration Precedence
//!
//! Configuration is merged from multiple sources with the following precedence
//! (highest to lowest):
//!
//! 1. Programmatic overrides (via `ConfigBuilder::with_config`)
//! 2. Environment variables (TROP_*)
//! 3. Private project config (`trop.local.yaml`)
//! 4. Project config (`trop.yaml`)
//! 5. User config (`~/.trop/config.yaml`)
//! 6. Built-in defaults
//!
//! # Examples
//!
//! Basic usage with defaults:
//!
//! ```no_run
//! use trop::config::ConfigBuilder;
//!
//! let config = ConfigBuilder::new()
//!     .build()
//!     .unwrap();
//!
//! println!("Port range: {}-{:?}",
//!     config.ports.as_ref().unwrap().min,
//!     config.ports.as_ref().unwrap().max);
//! ```
//!
//! Loading from a specific directory:
//!
//! ```no_run
//! use trop::config::ConfigBuilder;
//! use std::path::Path;
//!
//! let config = ConfigBuilder::new()
//!     .with_working_dir(Path::new("/path/to/project"))
//!     .build()
//!     .unwrap();
//! ```
//!
//! Programmatic configuration:
//!
//! ```
//! use trop::config::{Config, ConfigBuilder, PortConfig};
//!
//! let custom = Config {
//!     project: Some("my-project".to_string()),
//!     ports: Some(PortConfig {
//!         min: 8000,
//!         max: Some(9000),
//!         max_offset: None,
//!     }),
//!     ..Default::default()
//! };
//!
//! let config = ConfigBuilder::new()
//!     .skip_files()
//!     .skip_env()
//!     .with_config(custom)
//!     .build()
//!     .unwrap();
//!
//! assert_eq!(config.project, Some("my-project".to_string()));
//! ```

pub mod builder;
pub mod environment;
pub mod loader;
pub mod merger;
pub mod schema;
pub mod validator;

// Re-export key types at module root
pub use builder::ConfigBuilder;
pub use environment::EnvironmentConfig;
pub use loader::{ConfigLoader, ConfigSource};
pub use merger::ConfigMerger;
pub use schema::{
    CleanupConfig, Config, OccupancyConfig, OutputFormat, PortConfig, PortExclusion,
    ReservationGroup, ServiceDefinition,
};
pub use validator::ConfigValidator;
