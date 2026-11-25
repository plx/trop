//! Command to add port or range to exclusion list.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use crate::utils::GlobalOptions;
use clap::Args;
use std::path::Path;
use trop::config::{Config, PortExclusion};
use trop::port::exclusions::validate_exclusions;
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
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let global = context.global();
        // 1. Parse port or range
        let exclusion = self.parse_exclusion()?;

        // 2. Load configuration and database
        let db = context.open_database()?;

        // 3. Check if any ports are reserved (unless --force)
        if !self.force {
            self.check_reserved(&db, &exclusion)?;
        }

        // 4. Determine target config file
        let config_path = context.config_file_for_write(self.global)?;

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

    /// Parses and validates the argument with the same parser as the YAML
    /// string form and `TROP_EXCLUDED_PORTS`: a port (`"8080"`) or an
    /// inclusive range (`"8080..8090"`). Port 0 and reversed ranges are
    /// rejected here, before anything is written.
    fn parse_exclusion(&self) -> Result<PortExclusion, CliError> {
        let exclusion: PortExclusion = self
            .port_or_range
            .parse()
            .map_err(|e| CliError::InvalidArguments(format!("Invalid port number: {e}")))?;

        validate_exclusions(std::slice::from_ref(&exclusion)).map_err(|error| match error {
            trop::Error::Validation { message, .. } => CliError::InvalidArguments(format!(
                "Invalid exclusion '{}': {message}",
                self.port_or_range
            )),
            other => CliError::Library(other),
        })?;

        Ok(exclusion)
    }

    /// Fails if any port of the (validated) exclusion is reserved. Database
    /// errors are reported, not treated as "not reserved".
    fn check_reserved(&self, db: &Database, exclusion: &PortExclusion) -> Result<(), CliError> {
        let (start, end) = match *exclusion {
            PortExclusion::Single(port) => (port, port),
            PortExclusion::Range { start, end } => (start, end),
        };

        for port_value in start..=end {
            let port = Port::try_from(port_value)
                .map_err(|e| CliError::InvalidArguments(e.to_string()))?;
            if Database::is_port_reserved(db.connection(), port)? {
                return Err(CliError::InvalidArguments(format!(
                    "Port {port_value} is reserved. Use --force to override."
                )));
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

#[cfg(test)]
mod tests {
    use super::*;
    use trop::database::DatabaseConfig;

    fn command(port_or_range: &str) -> ExcludeCommand {
        ExcludeCommand {
            port_or_range: port_or_range.to_string(),
            global: false,
            force: false,
        }
    }

    #[test]
    fn parse_exclusion_accepts_ports_and_ranges() {
        assert_eq!(
            command("8080").parse_exclusion().unwrap(),
            PortExclusion::Single(8080)
        );
        assert_eq!(
            command("8080..8090").parse_exclusion().unwrap(),
            PortExclusion::Range {
                start: 8080,
                end: 8090
            }
        );
        // `5000..5000` is a valid one-port range.
        assert_eq!(
            command("5000..5000").parse_exclusion().unwrap(),
            PortExclusion::Range {
                start: 5000,
                end: 5000
            }
        );
        // Same whitespace policy as the YAML string form.
        assert_eq!(
            command(" 8080 .. 8090 ").parse_exclusion().unwrap(),
            PortExclusion::Range {
                start: 8080,
                end: 8090
            }
        );
        assert_eq!(
            command("1..65535").parse_exclusion().unwrap(),
            PortExclusion::Range {
                start: 1,
                end: 65535
            }
        );
    }

    /// Regression: `trop exclude` used to persist reversed ranges and port 0,
    /// which broke every later configuration load.
    #[test]
    fn parse_exclusion_rejects_invalid_exclusions() {
        for (text, reason) in [
            ("8090..8080", "Invalid range: 8090..8080 (end < start)"),
            ("0", "Invalid port: 0"),
            ("0..5", "Invalid start port: 0"),
            ("5..0", "Invalid end port: 0"),
        ] {
            match command(text).parse_exclusion() {
                Err(CliError::InvalidArguments(message)) => {
                    assert!(message.contains(reason), "{text}: {message}");
                }
                other => panic!("{text}: expected InvalidArguments, got {other:?}"),
            }
        }
        for text in ["", "abc", "65536", "8080..", "..8080", "1..2..3", "-1"] {
            assert!(
                matches!(
                    command(text).parse_exclusion(),
                    Err(CliError::InvalidArguments(_))
                ),
                "{text}"
            );
        }
    }

    /// Regression: a database error while checking for reservations used to
    /// be treated as "not reserved" (fail open).
    #[test]
    fn check_reserved_fails_closed_on_database_errors() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(DatabaseConfig::new(dir.path().join("trop.db"))).unwrap();
        db.connection()
            .execute_batch("DROP TABLE reservations")
            .unwrap();

        let result = command("8080").check_reserved(&db, &PortExclusion::Single(8080));
        assert!(matches!(result, Err(CliError::Library(_))), "{result:?}");

        let result = command("8080..8082").check_reserved(
            &db,
            &PortExclusion::Range {
                start: 8080,
                end: 8082,
            },
        );
        assert!(matches!(result, Err(CliError::Library(_))), "{result:?}");
    }

    #[test]
    fn check_reserved_passes_when_nothing_is_reserved() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(DatabaseConfig::new(dir.path().join("trop.db"))).unwrap();
        command("65530..65535")
            .check_reserved(
                &db,
                &PortExclusion::Range {
                    start: 65530,
                    end: 65535,
                },
            )
            .unwrap();
    }
}
