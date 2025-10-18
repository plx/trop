//! Command to add port or range to exclusion list.

use crate::error::CliError;
use crate::utils::{
    load_configuration, open_database, resolve_config_file, resolve_data_dir, GlobalOptions,
};
use clap::Args;
use std::path::Path;
use trop::config::{Config, PortExclusion};
use trop::{Database, Port};

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
            // Use resolve_config_file which returns project config if exists, otherwise global
            resolve_config_file()?
        };

        // 5. Load, modify, and save configuration
        let mut file_config = self.load_config_file(&config_path)?;
        let was_added = self.add_exclusion(&mut file_config, exclusion, global)?;

        if was_added {
            self.save_config_file(&config_path, &file_config)?;
            if !global.quiet {
                println!("Added exclusion to {}", config_path.display());
            }
        } else if !global.quiet {
            println!("Exclusion already exists in {}", config_path.display());
        }

        Ok(())
    }

    fn parse_exclusion(&self) -> Result<PortExclusion, CliError> {
        // Parse "8080" or "8080..8090" format
        if let Some(separator_pos) = self.port_or_range.find("..") {
            // Range format
            let min_str = &self.port_or_range[..separator_pos];
            let max_str = &self.port_or_range[separator_pos + 2..];

            let min = min_str
                .parse::<u16>()
                .map_err(|_| CliError::InvalidArguments("Invalid port number".into()))?;
            let max = max_str
                .parse::<u16>()
                .map_err(|_| CliError::InvalidArguments("Invalid port number".into()))?;

            Ok(PortExclusion::Range {
                start: min,
                end: max,
            })
        } else {
            // Single port
            let port = self
                .port_or_range
                .parse::<u16>()
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
                    return Err(CliError::InvalidArguments(format!(
                        "Port {port_value} is reserved. Use --force to override."
                    )));
                }
            }
        }

        Ok(())
    }

    fn load_config_file(&self, path: &Path) -> Result<Config, CliError> {
        if path.exists() {
            let contents = std::fs::read_to_string(path)?;
            serde_yaml::from_str(&contents)
                .map_err(|e| CliError::Config(format!("Failed to parse config: {e}")))
        } else {
            Ok(Config::default())
        }
    }

    fn add_exclusion(
        &self,
        config: &mut Config,
        exclusion: PortExclusion,
        _global: &GlobalOptions,
    ) -> Result<bool, CliError> {
        if config.excluded_ports.is_none() {
            config.excluded_ports = Some(Vec::new());
        }

        if let Some(ref mut exclusions) = config.excluded_ports {
            // Check for duplicates
            if !exclusions.contains(&exclusion) {
                exclusions.push(exclusion);
                Ok(true) // Was added
            } else {
                Ok(false) // Already existed
            }
        } else {
            Ok(false)
        }
    }

    fn save_config_file(&self, path: &Path, config: &Config) -> Result<(), CliError> {
        // Note: YAML comments will be lost during this process
        // This is a known limitation documented in the plan
        let yaml = serde_yaml::to_string(config)
            .map_err(|e| CliError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(path, yaml)?;
        Ok(())
    }
}
