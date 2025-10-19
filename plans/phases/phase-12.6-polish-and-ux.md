# Phase 12.6: Polish & UX Improvements

## Overview

Subpass 12.6 adds the final polish to trop with enhanced error messages, improved help text, progress indicators, and interactive features. This phase focuses on making trop delightful to use and production-ready.

## Context & Dependencies

**Prerequisites:**
- All previous subpasses completed (12.1-12.5)
- Core functionality tested and benchmarked
- Documentation generated

**Dependencies:**
- All previous Phase 12 subpasses should complete first

**Key Considerations:**
- Error messages should be actionable and helpful
- Progress indicators needed for operations > 1 second
- Interactive features should have non-interactive fallbacks
- All features must work gracefully in both interactive and non-interactive environments

## Implementation Tasks

### Task 1: Enhanced Error Messages

**File:** `trop/src/error.rs` (updates)

**Implementation:**
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TropError {
    // Existing variants...

    #[error("Port {port} is already in use by the system")]
    PortOccupied {
        port: u16,
        #[help]
        hint: String,
    },

    #[error("No available ports in range {start}-{end}")]
    NoAvailablePorts {
        start: u16,
        end: u16,
        #[help]
        hint: String,
    },

    #[error("Path {path} already has a reservation")]
    PathAlreadyReserved {
        path: String,
        existing_port: u16,
        #[help]
        hint: String,
    },

    #[error("Invalid configuration: {message}")]
    InvalidConfig {
        message: String,
        #[help]
        hint: String,
    },

    // Add more error variants with helpful hints...
}

impl TropError {
    /// Create a PortOccupied error with helpful hint
    pub fn port_occupied(port: u16) -> Self {
        Self::PortOccupied {
            port,
            hint: format!(
                "Port {} is being used by another application. Try:\n\
                 1. Using a different port range in your config\n\
                 2. Stopping the application using this port\n\
                 3. Running 'trop exclude {}' to skip this port",
                port, port
            ),
        }
    }

    /// Create NoAvailablePorts error with helpful hint
    pub fn no_available_ports(start: u16, end: u16, allocated: usize, excluded: usize) -> Self {
        let total_range = (end - start + 1) as usize;
        let available = total_range - allocated - excluded;

        Self::NoAvailablePorts {
            start,
            end,
            hint: format!(
                "Port range {}-{} is exhausted.\n\
                 Range size: {} ports\n\
                 Already allocated: {} ports\n\
                 Excluded: {} ports\n\
                 Available: {} ports\n\n\
                 Try:\n\
                 1. Expanding your port range in ~/.config/trop/config.toml\n\
                 2. Releasing unused reservations with 'trop cleanup'\n\
                 3. Reviewing exclusions with 'trop list-exclusions'",
                start, end, total_range, allocated, excluded, available
            ),
        }
    }

    /// Create PathAlreadyReserved error with hint
    pub fn path_already_reserved(path: String, existing_port: u16) -> Self {
        Self::PathAlreadyReserved {
            path: path.clone(),
            existing_port,
            hint: format!(
                "Path '{}' already has port {} reserved.\n\n\
                 Try:\n\
                 1. Use 'trop get' to see the existing reservation\n\
                 2. Use 'trop release' to release the existing port first\n\
                 3. Use a different path if working on a separate task",
                path, existing_port
            ),
        }
    }
}

// Add custom display for errors to show hints
impl std::fmt::Display for TropError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use colored::*;

        match self {
            Self::PortOccupied { port, hint } => {
                writeln!(f, "{}", format!("Error: Port {} is already in use", port).red())?;
                writeln!(f)?;
                write!(f, "{}", hint.yellow())
            }
            Self::NoAvailablePorts { start, end, hint } => {
                writeln!(f, "{}", format!("Error: No available ports in range {}-{}", start, end).red())?;
                writeln!(f)?;
                write!(f, "{}", hint.yellow())
            }
            Self::PathAlreadyReserved { path, existing_port, hint } => {
                writeln!(f, "{}", format!("Error: Path already reserved").red())?;
                writeln!(f, "  Path: {}", path)?;
                writeln!(f, "  Port: {}", existing_port)?;
                writeln!(f)?;
                write!(f, "{}", hint.yellow())
            }
            _ => {
                // Fall back to default Display for other variants
                write!(f, "{:?}", self)
            }
        }
    }
}
```

**Dependencies to add to `trop/Cargo.toml`:**
```toml
[dependencies]
colored = "2.1"  # For colorized error output
```

### Task 2: Improved Help Text

**File:** `trop-cli/src/cli.rs` (updates)

**Implementation:**
```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "trop")]
#[command(about = "Developer port reservation tool", long_about = None)]
#[command(after_help = "COMMON WORKFLOWS:\n\
    \n\
    Get started:\n\
    $ trop init --with-config              # Set up trop\n\
    $ cd ~/projects/my-app                 # Navigate to project\n\
    $ trop reserve                          # Reserve a port\n\
    \n\
    Daily usage:\n\
    $ PORT=$(trop get)                      # Get reserved port\n\
    $ npm start -- --port $PORT            # Use the port\n\
    \n\
    Cleanup:\n\
    $ trop cleanup --orphaned               # Remove orphaned reservations\n\
    $ trop release                          # Release current directory's port\n\
    \n\
    For detailed help:\n\
    $ trop help <command>                   # Get help for specific command\n\
    $ man trop                              # Read the manual (if installed)\n\
    \n\
    Documentation: https://github.com/your-org/trop")]
pub struct Cli {
    // ... existing fields ...
}

#[derive(Subcommand)]
pub enum Commands {
    /// Reserve a port for a directory
    ///
    /// Allocates an available port and associates it with a directory path.
    /// The port number is stored in a database and can be retrieved later.
    ///
    /// Examples:
    ///   # Reserve for current directory
    ///   trop reserve
    ///
    ///   # Reserve for specific directory
    ///   trop reserve --path ~/projects/my-app
    ///
    ///   # Reserve with specific project name
    ///   trop reserve --project my-app
    ///
    ///   # Request specific port (if available)
    ///   trop reserve --port 8080
    #[command(verbatim_doc_comment)]
    Reserve {
        /// Path to reserve port for (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,

        /// Project name (default: auto-detect from git)
        #[arg(short = 'P', long)]
        project: Option<String>,

        /// Task name (default: auto-detect from git branch/worktree)
        #[arg(short = 'T', long)]
        task: Option<String>,

        /// Specific port to request (if available)
        #[arg(long)]
        port: Option<u16>,

        /// Allow reserving path unrelated to project
        #[arg(long)]
        allow_unrelated_path: bool,
    },

    /// Get the reserved port for a directory
    ///
    /// Retrieves the port number associated with a directory.
    /// Useful for scripts and shell integrations.
    ///
    /// Examples:
    ///   # Get port for current directory
    ///   trop get
    ///
    ///   # Use in scripts
    ///   PORT=$(trop get)
    ///   npm start -- --port $PORT
    ///
    ///   # Get port for specific directory
    ///   trop get --path ~/projects/api-server
    #[command(verbatim_doc_comment)]
    Get {
        /// Path to get port for (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },

    // ... other commands with enhanced help text ...
}
```

### Task 3: Progress Indicators

**File:** `trop-cli/src/ui.rs` (create new)

**Dependencies to add to `trop-cli/Cargo.toml`:**
```toml
[dependencies]
indicatif = "0.17"  # Progress bars and spinners
```

**Implementation:**
```rust
use indicatif::{ProgressBar, ProgressStyle, ProgressDrawTarget};
use std::time::Duration;

pub struct ProgressIndicator {
    bar: Option<ProgressBar>,
}

impl ProgressIndicator {
    /// Create a new progress indicator
    pub fn new(message: &str, quiet: bool) -> Self {
        if quiet || !atty::is(atty::Stream::Stderr) {
            return Self { bar: None };
        }

        let bar = ProgressBar::new_spinner();
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
        );
        bar.set_message(message.to_string());
        bar.enable_steady_tick(Duration::from_millis(100));

        Self { bar: Some(bar) }
    }

    /// Create a progress bar with known length
    pub fn new_bar(total: u64, message: &str, quiet: bool) -> Self {
        if quiet || !atty::is(atty::Stream::Stderr) {
            return Self { bar: None };
        }

        let bar = ProgressBar::new(total);
        bar.set_draw_target(ProgressDrawTarget::stderr());
        bar.set_style(
            ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=>-")
        );
        bar.set_message(message.to_string());

        Self { bar: Some(bar) }
    }

    /// Update the message
    pub fn set_message(&self, message: &str) {
        if let Some(bar) = &self.bar {
            bar.set_message(message.to_string());
        }
    }

    /// Increment progress (for progress bars)
    pub fn inc(&self, delta: u64) {
        if let Some(bar) = &self.bar {
            bar.inc(delta);
        }
    }

    /// Finish and clear the indicator
    pub fn finish(&self) {
        if let Some(bar) = &self.bar {
            bar.finish_and_clear();
        }
    }

    /// Finish with a success message
    pub fn finish_with_message(&self, message: &str) {
        if let Some(bar) = &self.bar {
            bar.finish_with_message(message.to_string());
        }
    }
}

impl Drop for ProgressIndicator {
    fn drop(&mut self) {
        self.finish();
    }
}

// Usage example in commands
pub fn reserve_with_progress(/* ... */) -> Result<(), CliError> {
    let progress = ProgressIndicator::new("Allocating port...", quiet_mode);

    // ... do work ...

    progress.finish_with_message("✓ Port reserved");
    Ok(())
}
```

**Dependencies to add:**
```toml
atty = "0.2"  # Check if output is a TTY
```

### Task 4: Interactive Features

**File:** `trop-cli/src/interactive.rs` (create new)

**Dependencies to add to `trop-cli/Cargo.toml`:**
```toml
[dependencies]
dialoguer = "0.11"  # Interactive prompts
```

**Implementation:**
```rust
use dialoguer::{Confirm, Select, Input};
use crate::error::CliError;

/// Confirm a potentially dangerous operation
pub fn confirm_dangerous_operation(message: &str, force: bool) -> Result<bool, CliError> {
    if force {
        return Ok(true);
    }

    if !atty::is(atty::Stream::Stdin) {
        // Not interactive, require --force
        return Err(CliError::RequiresConfirmation {
            operation: message.to_string(),
            hint: "Use --force to confirm this operation non-interactively".to_string(),
        });
    }

    Ok(Confirm::new()
        .with_prompt(message)
        .default(false)
        .interact()?)
}

/// Interactive conflict resolution
pub fn resolve_conflict(
    path: &str,
    existing_port: u16,
    options: &[&str],
) -> Result<usize, CliError> {
    if !atty::is(atty::Stream::Stdin) {
        return Err(CliError::RequiresInteraction {
            message: "Conflict resolution requires interactive terminal".to_string(),
        });
    }

    println!("Conflict: Path '{}' already has port {} reserved", path, existing_port);
    println!();

    Ok(Select::new()
        .with_prompt("How would you like to proceed?")
        .items(options)
        .default(0)
        .interact()?)
}

/// First-run configuration wizard
pub fn configuration_wizard() -> Result<(), CliError> {
    println!("Welcome to trop! Let's set up your configuration.\n");

    let port_range_start: u16 = Input::new()
        .with_prompt("Port range start")
        .default(50000)
        .interact()?;

    let port_range_end: u16 = Input::new()
        .with_prompt("Port range end")
        .default(60000)
        .validate_with(|input: &u16| -> Result<(), &str> {
            if *input <= port_range_start {
                Err("End must be greater than start")
            } else {
                Ok(())
            }
        })
        .interact()?;

    let auto_exclude = Confirm::new()
        .with_prompt("Automatically exclude occupied ports?")
        .default(true)
        .interact()?;

    println!("\nConfiguration summary:");
    println!("  Port range: {}-{}", port_range_start, port_range_end);
    println!("  Auto-exclude occupied: {}", auto_exclude);

    if Confirm::new()
        .with_prompt("Save this configuration?")
        .default(true)
        .interact()?
    {
        // Save configuration...
        println!("✓ Configuration saved to ~/.config/trop/config.toml");
    }

    Ok(())
}

/// Batch operation confirmation
pub fn confirm_batch_operation(
    operation: &str,
    count: usize,
    force: bool,
) -> Result<bool, CliError> {
    if force {
        return Ok(true);
    }

    if !atty::is(atty::Stream::Stdin) {
        return Err(CliError::RequiresConfirmation {
            operation: format!("{} {} items", operation, count),
            hint: "Use --force to confirm batch operations non-interactively".to_string(),
        });
    }

    Ok(Confirm::new()
        .with_prompt(format!("{} {} items. Continue?", operation, count))
        .default(false)
        .interact()?)
}
```

## Success Criteria

- [ ] All errors have actionable hints and suggestions
- [ ] Help text includes examples for all major commands
- [ ] Progress indicators shown for operations > 1 second
- [ ] Interactive confirmations for dangerous operations (with --force override)
- [ ] First-run configuration wizard works
- [ ] All new features work in both interactive and non-interactive modes
- [ ] Colorized output respects NO_COLOR environment variable

## Testing

**Error Messages:**
- Trigger each error condition and verify hint is helpful
- Test colorized vs plain output

**Interactive Features:**
- Test with interactive terminal (TTY)
- Test with non-interactive input (CI environment)
- Verify --force flag bypasses prompts

**Progress Indicators:**
- Test with and without TTY
- Verify quiet mode suppresses indicators

## Notes

- Interactive features must gracefully degrade in non-interactive environments
- Error hints should guide users to solutions, not just describe problems
- Progress indicators improve perceived performance for long operations
- Colorized output should respect NO_COLOR environment variable and TTY detection
- Consider adding error codes for scripting (exit codes and error identifiers)
