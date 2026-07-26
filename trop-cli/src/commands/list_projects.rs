//! List projects command implementation.
//!
//! This module implements the `list-projects` command, which displays
//! all unique project identifiers from the database.

use crate::error::CliError;
use crate::invocation::InvocationContext;
use clap::Parser;
use std::io::Write;
use trop::Database;

/// List all unique project identifiers.
#[derive(Parser)]
#[command(about = "List all unique project identifiers")]
pub struct ListProjectsCommand {
    // Future: could add format options, filters
}

impl ListProjectsCommand {
    /// Execute the list-projects command.
    pub fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        // 2. Open database (read-only access is fine)
        let db = context.open_database()?;

        // 3. Query projects
        let projects = Database::list_projects(db.connection()).map_err(CliError::from)?;

        // 4. Output one per line to stdout
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();

        for project in projects {
            writeln!(handle, "{project}")?;
        }

        Ok(())
    }
}
