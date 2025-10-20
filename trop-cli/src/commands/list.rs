//! List command implementation.
//!
//! This module implements the `list` command, which displays active
//! reservations in various formats (table, JSON, CSV, TSV).

use crate::error::CliError;
use crate::utils::{
    format_timestamp, load_configuration, normalize_path, open_database, shorten_path,
    GlobalOptions,
};
use clap::{Args, ValueEnum};
use std::io::Write;
use std::path::PathBuf;
use trop::{Database, Reservation};

/// Column headers for CSV/TSV output.
const COLUMN_HEADERS: [&str; 7] = [
    "port",
    "path",
    "tag",
    "project",
    "task",
    "created_at",
    "last_used_at",
];

/// List active reservations.
#[derive(Args)]
pub struct ListCommand {
    /// Output format
    #[arg(
        long,
        value_enum,
        default_value = "table",
        env = "TROP_OUTPUT_FORMAT",
        ignore_case = true
    )]
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

/// Output format for list command.
#[derive(Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Tab-separated table format (human-readable)
    Table,
    /// JSON format
    Json,
    /// CSV format
    Csv,
    /// TSV format (tab-separated values)
    Tsv,
}

impl ListCommand {
    /// Execute the list command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // 1. Load configuration
        let config = load_configuration(global)?;

        // 2. Open database (read-only access is fine)
        let db = open_database(global, &config)?;

        // 3. Query reservations
        let mut reservations =
            Database::list_all_reservations(db.connection()).map_err(CliError::from)?;

        // 4. Apply filters
        if let Some(ref project) = self.filter_project {
            reservations.retain(|r| r.project() == Some(project.as_str()));
        }

        if let Some(ref tag) = self.filter_tag {
            reservations.retain(|r| r.key().tag == Some(tag.clone()));
        }

        if let Some(ref path) = self.filter_path {
            let normalized = normalize_path(path)?;
            reservations.retain(|r| r.key().path.starts_with(&normalized));
        }

        // 5. Format and output to stdout
        match self.format {
            OutputFormat::Table => format_as_table(&reservations, self.show_full_paths)?,
            OutputFormat::Json => format_as_json(&reservations)?,
            OutputFormat::Csv => format_as_csv(&reservations)?,
            OutputFormat::Tsv => format_as_tsv(&reservations)?,
        }

        Ok(())
    }
}

/// Format reservations as a human-readable table.
fn format_as_table(reservations: &[Reservation], show_full: bool) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    // Print header (uppercase for table display)
    let header_line = COLUMN_HEADERS
        .iter()
        .map(|s| s.to_uppercase())
        .collect::<Vec<_>>()
        .join("\t");
    writeln!(handle, "{header_line}")?;

    // Print each reservation
    for res in reservations {
        let path_str = if show_full {
            res.key().path.display().to_string()
        } else {
            shorten_path(&res.key().path)
        };

        writeln!(
            handle,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            res.port().value(),
            path_str,
            res.key().tag.as_deref().unwrap_or("-"),
            res.project().unwrap_or("-"),
            res.task().unwrap_or("-"),
            format_timestamp(res.created_at()),
            format_timestamp(res.last_used_at()),
        )?;
    }

    Ok(())
}

/// Format reservations as JSON.
fn format_as_json(reservations: &[Reservation]) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    // Build a JSON array of reservation objects
    let json_data: Vec<serde_json::Value> = reservations
        .iter()
        .map(|r| {
            serde_json::json!({
                "port": r.port().value(),
                "path": r.key().path.display().to_string(),
                "tag": r.key().tag,
                "project": r.project(),
                "task": r.task(),
                "created_at": format_timestamp(r.created_at()),
                "last_used_at": format_timestamp(r.last_used_at()),
            })
        })
        .collect();

    serde_json::to_writer_pretty(&mut handle, &json_data)
        .map_err(|e| CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    writeln!(handle)?;

    Ok(())
}

/// Convert csv::Error to CliError.
fn csv_error(e: csv::Error) -> CliError {
    CliError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
}

/// Format reservations as delimited output (CSV or TSV).
fn format_as_delimited(reservations: &[Reservation], delimiter: u8) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let handle = stdout.lock();
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(handle);

    // Write header
    writer.write_record(COLUMN_HEADERS).map_err(csv_error)?;

    // Write each reservation
    for res in reservations {
        writer
            .write_record(&[
                res.port().value().to_string(),
                res.key().path.display().to_string(),
                res.key().tag.as_deref().unwrap_or("").to_string(),
                res.project().unwrap_or("").to_string(),
                res.task().unwrap_or("").to_string(),
                format_timestamp(res.created_at()),
                format_timestamp(res.last_used_at()),
            ])
            .map_err(csv_error)?;
    }

    writer.flush()?;

    Ok(())
}

/// Format reservations as CSV.
fn format_as_csv(reservations: &[Reservation]) -> Result<(), CliError> {
    format_as_delimited(reservations, b',')
}

/// Format reservations as TSV (tab-separated values).
fn format_as_tsv(reservations: &[Reservation]) -> Result<(), CliError> {
    format_as_delimited(reservations, b'\t')
}
