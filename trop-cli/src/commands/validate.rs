//! Command to validate a trop configuration file.

use crate::error::CliError;
use crate::utils::GlobalOptions;
use clap::Args;
use std::path::PathBuf;
use trop::config::{Config, ConfigFileKind, ConfigValidator, ConfigValueSource};

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
        let kind = if filename == "config.yaml" {
            ConfigFileKind::User
        } else {
            ConfigFileKind::Project
        };

        // 3. Parse the file
        let contents = std::fs::read_to_string(&self.config_path)?;
        let config: Config = match serde_yaml::from_str(&contents) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Parse error in {}: {e}", self.config_path.display());
                return Err(CliError::SemanticFailure(
                    "Configuration file is invalid".to_string(),
                ));
            }
        };

        // 4. Validate the document in the context of its exact source.
        let source = ConfigValueSource::File {
            kind,
            path: self.config_path.clone(),
        };
        match ConfigValidator::validate_source(&config, &source) {
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
