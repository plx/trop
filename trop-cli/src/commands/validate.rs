//! Command to validate a trop configuration file.

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
            return Err(CliError::InvalidArguments(format!(
                "File not found: {}",
                self.config_path.display()
            )));
        }

        // 2. Determine file type (trop.yaml vs config.yaml)
        let filename = self
            .config_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let is_tropfile = filename == "trop.yaml" || filename == "trop.local.yaml";

        // 3. Parse the file
        let contents = std::fs::read_to_string(&self.config_path)?;
        let config: Config = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Parse error: {e}");
                return Err(CliError::SemanticFailure(
                    "Configuration file is invalid".to_string(),
                ));
            }
        };

        // 4. Validate the configuration (ConfigValidator already exists)
        match ConfigValidator::validate(&config, is_tropfile) {
            Ok(()) => {
                println!("Configuration is valid");
                Ok(())
            }
            Err(e) => {
                eprintln!("Validation error: {e}");
                Err(CliError::SemanticFailure(
                    "Configuration validation failed".to_string(),
                ))
            }
        }
    }
}
