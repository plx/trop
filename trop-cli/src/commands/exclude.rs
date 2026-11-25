//! Command to add port or range to exclusion list.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Args;
use std::path::Path;
use trop::config::PortExclusion;
use trop::port::exclusions::{
    add_exclusion_to_list, excluded_ports_in, first_reserved_port, rewrite_excluded_ports,
    validate_exclusions, WriteScope,
};
use trop::Database;

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
        let quiet = context.global().quiet;
        // 1. Parse and validate the port or range
        let exclusion = self.parse_exclusion()?;

        // 2. Determine the target config file: without --global, a project
        //    trop.yaml (or trop.local.yaml) is required.
        let scope = if self.global {
            WriteScope::Global
        } else {
            WriteScope::Project
        };
        let config_path = context.config_file_for_write(scope)?;

        // 3. Check if any ports are reserved (unless --force)
        let db = context.open_database()?;
        if !self.force {
            self.check_reserved(&db, &exclusion)?;
        }

        // 4. Add the exclusion (merging overlapping entries) and rewrite only
        //    the `excluded_ports` key of the file.
        let document = read_config_document(&config_path)?;
        let current = current_exclusions(&config_path, &document)?;
        let (updated, changed) = add_exclusion_to_list(&current, &exclusion)
            .map_err(|error| CliError::Config(format!("{}: {error}", config_path.display())))?;

        if changed {
            write_exclusions(&config_path, &document, &updated)?;
            if !quiet {
                println!("Added exclusion to {}", config_path.display());
            }
        } else if !quiet {
            println!("Ports already excluded in {}", config_path.display());
        }

        Ok(())
    }

    /// Parses and validates the argument with the same parser as the YAML
    /// string form and `TROP_EXCLUDED_PORTS`: a port (`"8080"`) or an
    /// inclusive range (`"8080..8090"`). Port 0 and reversed ranges are
    /// rejected here, before anything is written.
    fn parse_exclusion(&self) -> Result<PortExclusion, CliError> {
        let invalid = |reason: &str| {
            CliError::InvalidArguments(format!(
                "Invalid exclusion '{}': {reason}",
                self.port_or_range
            ))
        };
        let exclusion: PortExclusion = self.port_or_range.parse().map_err(|_| {
            invalid("expected a port (1-65535) or an inclusive range such as 8080..8090")
        })?;

        validate_exclusions(std::slice::from_ref(&exclusion)).map_err(|error| match error {
            trop::Error::Validation { message, .. } => invalid(&message),
            other => CliError::Library(other),
        })?;

        Ok(exclusion)
    }

    /// Fails if any port of the (validated) exclusion is reserved, naming the
    /// lowest one. The decision is made by the verified
    /// `trop::port::exclusions::first_reserved_port`, which consults the
    /// database only at the exclusion's ports; database errors are reported,
    /// not treated as "not reserved".
    fn check_reserved(&self, db: &Database, exclusion: &PortExclusion) -> Result<(), CliError> {
        match first_reserved_port(exclusion, |port| {
            Database::is_port_reserved(db.connection(), port)
        })? {
            Some(port) => Err(CliError::InvalidArguments(format!(
                "Port {} is reserved. Use --force to override.",
                port.value()
            ))),
            None => Ok(()),
        }
    }
}

/// Reads a configuration file for rewriting; a missing file reads as an
/// empty document.
pub(crate) fn read_config_document(path: &Path) -> Result<String, CliError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// The description of a library error, without the library's
/// `validation error for '<field>':` prefix, for CLI messages that name
/// their own context.
pub(crate) fn error_detail(error: trop::Error) -> String {
    match error {
        trop::Error::Validation { message, .. } => message,
        other => other.to_string(),
    }
}

/// The `excluded_ports` list of a configuration document (empty when the
/// document has none).
pub(crate) fn current_exclusions(
    path: &Path,
    document: &str,
) -> Result<Vec<PortExclusion>, CliError> {
    excluded_ports_in(document)
        .map(Option::unwrap_or_default)
        .map_err(|error| {
            CliError::Config(format!(
                "Failed to parse config {}: {}",
                path.display(),
                error_detail(error)
            ))
        })
}

/// Writes `exclusions` as the `excluded_ports` list of the configuration
/// document read from `path`, leaving every other top-level key as it was.
///
/// YAML comments and formatting are not preserved.
pub(crate) fn write_exclusions(
    path: &Path,
    document: &str,
    exclusions: &[PortExclusion],
) -> Result<(), CliError> {
    let yaml = rewrite_excluded_ports(document, exclusions).map_err(|error| {
        CliError::Config(format!(
            "Failed to rewrite config {}: {}",
            path.display(),
            error_detail(error)
        ))
    })?;
    std::fs::write(path, yaml)?;
    Ok(())
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

    /// Regression: a parse failure used to be reported twice over
    /// (`Invalid arguments: Invalid port number: Invalid port: abc`).
    #[test]
    fn parse_exclusion_failure_message_is_not_doubled() {
        for text in ["abc", "8080..", "65536"] {
            let error = command(text).parse_exclusion().unwrap_err();
            let message = error.to_string();
            assert_eq!(
                message,
                format!(
                    "Invalid arguments: Invalid exclusion '{text}': expected a port (1-65535) \
                     or an inclusive range such as 8080..8090"
                )
            );
            assert_eq!(message.matches("Invalid").count(), 2, "{message}");
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

    #[test]
    fn read_config_document_treats_a_missing_file_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_config_document(&dir.path().join("missing.yaml")).unwrap(),
            ""
        );
        assert!(current_exclusions(&dir.path().join("missing.yaml"), "")
            .unwrap()
            .is_empty());
    }
}
