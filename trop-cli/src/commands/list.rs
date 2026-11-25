//! List command implementation.
//!
//! This module implements the `list` command, which displays active
//! reservations in various formats (table, JSON, CSV, TSV).

use crate::error::CliError;
use crate::invocation::InvocationContext;
use crate::utils::{format_timestamp, shorten_path};
use clap::{Args, ValueEnum};
use std::io::Write;
use std::path::PathBuf;
use trop::cli_support::{delimited_record, identifier_filter_matches};
use trop::output::tab_row;
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
    #[arg(long, value_enum, ignore_case = true)]
    pub format: Option<OutputFormat>,

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
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        let format = match context.effective()?.output_format() {
            trop::config::OutputFormat::Table => OutputFormat::Table,
            trop::config::OutputFormat::Json => OutputFormat::Json,
            trop::config::OutputFormat::Csv => OutputFormat::Csv,
            trop::config::OutputFormat::Tsv => OutputFormat::Tsv,
        };

        // 2. Open database (read-only access is fine)
        let db = context.open_database()?;

        // 3. Query reservations
        let mut reservations =
            Database::list_all_reservations(db.connection()).map_err(CliError::from)?;

        // 4. Apply filters. Project and tag filters compare the filter's
        // canonical (trimmed) form with the stored value, the form
        // identifiers are stored in; a blank filter matches nothing (the
        // verified `identifier_filter_matches` decides).
        if let Some(ref project) = self.filter_project {
            reservations.retain(|r| identifier_filter_matches(project, r.project()));
        }

        if let Some(ref tag) = self.filter_tag {
            reservations.retain(|r| identifier_filter_matches(tag, r.key().tag.as_deref()));
        }

        if let Some(ref path) = self.filter_path {
            let lexical_filter = context.resolve_explicit_path(path)?;
            let canonical_filter = context.resolve_canonical_path(path).ok();
            reservations.retain(|reservation| {
                let stored_path = &reservation.key().path;

                // Preserve direct matches for explicit identities, including
                // nonexistent paths. When both paths can be canonicalized,
                // also compare their physical identities as required by the
                // list-filter contract.
                stored_path.starts_with(&lexical_filter)
                    || canonical_filter.as_ref().is_some_and(|canonical_filter| {
                        context
                            .resolve_canonical_path(stored_path)
                            .is_ok_and(|canonical_path| {
                                canonical_path.starts_with(canonical_filter)
                            })
                    })
            });
        }

        // 5. Format and output to stdout
        match format {
            OutputFormat::Table => format_as_table(&reservations, self.show_full_paths)?,
            OutputFormat::Json => format_as_json(&reservations)?,
            OutputFormat::Csv => format_as_csv(&reservations)?,
            OutputFormat::Tsv => format_as_tsv(&reservations)?,
        }

        Ok(())
    }
}

/// Format reservations as a human-readable table.
///
/// Every cell is shown with `trop::output::display_field` (through
/// `tab_row`), so a tag, project, task or path containing a newline, tab or
/// other control character cannot split a reservation across lines or shift
/// its columns: each reservation is exactly one line of seven tab-separated
/// columns.
fn format_as_table(reservations: &[Reservation], show_full: bool) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    write!(handle, "{}", render_table(reservations, show_full))?;
    Ok(())
}

/// The table [`format_as_table`] prints.
fn render_table(reservations: &[Reservation], show_full: bool) -> String {
    // Header (uppercase for table display)
    let headers: Vec<String> = COLUMN_HEADERS.iter().map(|s| s.to_uppercase()).collect();
    let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
    let mut output = tab_row(&headers);
    output.push('\n');

    for res in reservations {
        let path_str = if show_full {
            res.key().path.display().to_string()
        } else {
            shorten_path(&res.key().path)
        };
        let port = res.port().value().to_string();
        let created = format_timestamp(res.created_at());
        let last_used = format_timestamp(res.last_used_at());
        output.push_str(&tab_row(&[
            &port,
            &path_str,
            res.key().tag.as_deref().unwrap_or("-"),
            res.project().unwrap_or("-"),
            res.task().unwrap_or("-"),
            &created,
            &last_used,
        ]));
        output.push('\n');
    }
    output
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

/// Format reservations as delimited output (CSV or TSV).
fn format_as_delimited(reservations: &[Reservation], delimiter: char) -> Result<(), CliError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle.write_all(render_delimited(reservations, delimiter).as_bytes())?;
    handle.flush()?;
    Ok(())
}

/// The delimited output: a header record, then one record per reservation.
///
/// Records are written by the verified `delimited_record` (byte for byte the
/// `csv` crate's default writer): a field containing the delimiter, a quote,
/// or a line break is quoted, so every reservation is exactly one record of
/// seven fields that reads back as its values.
fn render_delimited(reservations: &[Reservation], delimiter: char) -> String {
    let mut output = delimited_record(&COLUMN_HEADERS, delimiter);
    for res in reservations {
        let port = res.port().value().to_string();
        let path = res.key().path.display().to_string();
        let created = format_timestamp(res.created_at());
        let last_used = format_timestamp(res.last_used_at());
        output.push_str(&delimited_record(
            &[
                &port,
                &path,
                res.key().tag.as_deref().unwrap_or(""),
                res.project().unwrap_or(""),
                res.task().unwrap_or(""),
                &created,
                &last_used,
            ],
            delimiter,
        ));
    }
    output
}

/// Format reservations as CSV.
fn format_as_csv(reservations: &[Reservation]) -> Result<(), CliError> {
    format_as_delimited(reservations, ',')
}

/// Format reservations as TSV (tab-separated values).
fn format_as_tsv(reservations: &[Reservation]) -> Result<(), CliError> {
    format_as_delimited(reservations, '\t')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use trop::{Port, ReservationKey};

    fn reservation(
        path: &str,
        tag: Option<&str>,
        project: Option<&str>,
        task: Option<&str>,
    ) -> Reservation {
        let key = ReservationKey::new(PathBuf::from(path), tag.map(String::from)).unwrap();
        let mut builder = Reservation::builder(key, Port::try_from(5001).unwrap())
            .project(project.map(String::from))
            .task(task.map(String::from));
        builder = builder.created_at(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        builder = builder.last_used_at(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        builder.build().unwrap()
    }

    /// The delimited output is exactly the `csv` crate's (the writer it
    /// replaced), and reads back with the `csv` reader as one record of
    /// seven fields per reservation, whatever the field text.
    #[test]
    fn delimited_output_matches_the_csv_crate_and_reads_back() {
        let rows = [
            reservation("/srv/a,b", Some("x\ny"), Some("say \"hi\""), Some("t\tx")),
            reservation("/srv/\r\n", Some("w\u{1b}[31m"), None, None),
            reservation("/srv/plain", Some("web"), Some("naïve"), Some("task")),
        ];
        for delimiter in [',', '\t'] {
            let rendered = render_delimited(&rows, delimiter);
            let mut writer = csv::WriterBuilder::new()
                .delimiter(u8::try_from(delimiter).unwrap())
                .from_writer(Vec::new());
            writer.write_record(COLUMN_HEADERS).unwrap();
            for res in &rows {
                writer
                    .write_record([
                        res.port().value().to_string(),
                        res.key().path.display().to_string(),
                        res.key().tag.clone().unwrap_or_default(),
                        res.project().unwrap_or("").to_string(),
                        res.task().unwrap_or("").to_string(),
                        format_timestamp(res.created_at()),
                        format_timestamp(res.last_used_at()),
                    ])
                    .unwrap();
            }
            let expected = String::from_utf8(writer.into_inner().unwrap()).unwrap();
            assert_eq!(rendered, expected, "{delimiter:?}");

            let mut reader = csv::ReaderBuilder::new()
                .delimiter(u8::try_from(delimiter).unwrap())
                .from_reader(rendered.as_bytes());
            let records: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
            assert_eq!(records.len(), rows.len());
            for (record, res) in records.iter().zip(&rows) {
                assert_eq!(record.len(), 7);
                assert_eq!(&record[1], res.key().path.display().to_string());
                assert_eq!(&record[2], res.key().tag.as_deref().unwrap_or(""));
                assert_eq!(&record[3], res.project().unwrap_or(""));
                assert_eq!(&record[4], res.task().unwrap_or(""));
            }
        }
    }

    /// Every short string over the characters that matter (delimiters,
    /// quotes, line breaks, multibyte text, empty) is written byte for byte
    /// as the `csv` crate writes it, alone and in two-field records, and
    /// reads back unchanged. A record of no fields is written as the `csv`
    /// crate writes it too.
    #[test]
    fn delimited_record_matches_the_csv_crate_exhaustively() {
        let alphabet = ["a", ",", "\t", "\"", "\r", "\n", "é", " "];
        let mut strings = vec![String::new()];
        let mut layer = vec![String::new()];
        for _ in 0..3 {
            layer = layer
                .iter()
                .flat_map(|prefix| alphabet.iter().map(move |c| format!("{prefix}{c}")))
                .collect();
            strings.extend(layer.iter().cloned());
        }
        let short: Vec<&String> = strings.iter().filter(|s| s.chars().count() <= 2).collect();
        let csv_record = |fields: &[&str], delimiter: char| {
            let mut writer = csv::WriterBuilder::new()
                .delimiter(u8::try_from(delimiter).unwrap())
                .from_writer(Vec::new());
            writer.write_record(fields).unwrap();
            String::from_utf8(writer.into_inner().unwrap()).unwrap()
        };
        for delimiter in [',', '\t'] {
            assert_eq!(
                delimited_record(&[], delimiter),
                csv_record(&[], delimiter),
                "no fields {delimiter:?}"
            );
            let check = |fields: &[&str]| {
                let ours = delimited_record(fields, delimiter);
                assert_eq!(
                    ours,
                    csv_record(fields, delimiter),
                    "{fields:?} {delimiter:?}"
                );
                let mut reader = csv::ReaderBuilder::new()
                    .has_headers(false)
                    .delimiter(u8::try_from(delimiter).unwrap())
                    .from_reader(ours.as_bytes());
                let records: Vec<csv::StringRecord> =
                    reader.records().map(Result::unwrap).collect();
                assert_eq!(records.len(), 1, "{fields:?} {delimiter:?}");
                let read: Vec<&str> = records[0].iter().collect();
                assert_eq!(read, fields, "{delimiter:?}");
            };
            for field in &strings {
                check(&[field]);
            }
            for first in &short {
                for second in &short {
                    check(&[first, second]);
                }
            }
        }
    }

    /// Regression: a tag, project or task with a newline or tab printed one
    /// reservation across several lines (or with extra columns).
    #[test]
    fn table_rows_stay_on_one_line_with_seven_columns() {
        let rows = [
            reservation("/srv/a", Some("x\ny"), Some("evil\nproj"), Some("t\tx")),
            reservation("/srv/b\nc", Some("w\u{1b}[31m"), None, None),
            reservation("/srv/plain", Some("web"), Some("proj"), Some("task")),
        ];
        let table = render_table(&rows, true);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 4, "{table}");
        for line in &lines {
            assert_eq!(line.split('\t').count(), 7, "{line:?}");
            assert!(!line.contains('\u{1b}'), "{line:?}");
        }
        assert!(
            lines[1].contains("\t\"x\\ny\"\t\"evil\\nproj\"\t\"t\\tx\"\t"),
            "{}",
            lines[1]
        );
        assert!(
            lines[2].starts_with("5001\t\"/srv/b\\nc\"\t\"w\\x1b[31m\"\t-\t-\t"),
            "{}",
            lines[2]
        );
        // Ordinary values are shown unchanged.
        assert!(
            lines[3].starts_with("5001\t/srv/plain\tweb\tproj\ttask\t"),
            "{}",
            lines[3]
        );
        assert_eq!(
            lines[0],
            "PORT\tPATH\tTAG\tPROJECT\tTASK\tCREATED_AT\tLAST_USED_AT"
        );
    }
}
