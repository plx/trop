//! Build script for trop-cli.
//!
//! This script generates man pages at build time using clap_mangen.
//! The generated man page is placed in OUT_DIR for inclusion in release builds.
//!
//! Note: We build a minimal command structure here rather than importing from
//! the main crate, since build scripts cannot depend on the crate being built.

use clap::{Arg, Command};
use clap_mangen::Man;
use std::fs;
use std::path::PathBuf;

/// Build the CLI command structure for man page generation.
///
/// IMPORTANT: Keep this structure synchronized with src/cli.rs
/// When adding/removing/modifying commands, update both files.
fn build_cli() -> Command {
    Command::new("trop")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Manage ephemeral port reservations")
        .long_about(
            "Command-line tool for managing ephemeral port reservations for development projects",
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .help("Enable verbose output")
                .global(true)
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .help("Suppress non-essential output")
                .global(true)
                .action(clap::ArgAction::SetTrue),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .help("Override the data directory location")
                .value_name("PATH")
                .global(true)
                .env("TROP_DATA_DIR"),
        )
        .arg(
            Arg::new("busy-timeout")
                .long("busy-timeout")
                .help("Override the default busy timeout (in seconds)")
                .value_name("SECONDS")
                .global(true)
                .env("TROP_BUSY_TIMEOUT"),
        )
        .arg(
            Arg::new("disable-autoinit")
                .long("disable-autoinit")
                .help("Disable automatic database initialization")
                .global(true)
                .action(clap::ArgAction::SetTrue)
                .env("TROP_DISABLE_AUTOINIT"),
        )
        .subcommands(vec![
            Command::new("reserve")
                .about("Reserve a port for a directory")
                .long_about("Reserve a port for the current or specified directory"),
            Command::new("release")
                .about("Release a port reservation")
                .long_about("Release a port reservation by path, port, or project name"),
            Command::new("list")
                .about("List active reservations")
                .long_about("Display all active port reservations in various formats"),
            Command::new("reserve-group")
                .about("Reserve ports for a group of services")
                .long_about("Reserve a group of contiguous ports for multiple services"),
            Command::new("autoreserve")
                .about("Automatically discover and reserve ports")
                .long_about("Discover project structure and automatically reserve required ports"),
            Command::new("prune")
                .about("Remove reservations for non-existent directories")
                .long_about("Clean up reservations where the directory no longer exists"),
            Command::new("expire")
                .about("Remove reservations based on age")
                .long_about(
                    "Remove reservations that haven't been accessed for a specified period",
                ),
            Command::new("autoclean")
                .about("Combined cleanup (prune + expire)")
                .long_about("Perform both pruning and expiration in one operation"),
            Command::new("assert-reservation")
                .about("Assert that a reservation exists for a path/tag")
                .long_about("Check if a reservation exists and exit with appropriate status code"),
            Command::new("assert-port")
                .about("Assert that a specific port is reserved")
                .long_about("Check if a specific port is reserved by trop"),
            Command::new("assert-data-dir")
                .about("Assert that the data directory exists and is valid")
                .long_about("Verify the trop data directory is properly initialized"),
            Command::new("port-info")
                .about("Display information about a specific port")
                .long_about("Show detailed information about a port reservation"),
            Command::new("show-data-dir")
                .about("Show the resolved data directory path")
                .long_about("Display the path to the trop data directory"),
            Command::new("show-path")
                .about("Show the resolved path for a reservation")
                .long_about("Display the path associated with a reservation"),
            Command::new("scan")
                .about("Scan port range for occupied ports")
                .long_about("Scan a range of ports to identify which are currently in use"),
            Command::new("validate")
                .about("Validate a configuration file")
                .long_about("Check a trop configuration file for errors"),
            Command::new("exclude")
                .about("Add port or range to exclusion list")
                .long_about("Exclude specific ports or ranges from allocation"),
            Command::new("compact-exclusions")
                .about("Compact exclusion list to minimal representation")
                .long_about("Merge overlapping port exclusions into minimal ranges"),
            Command::new("init")
                .about("Initialize trop data directory and database")
                .long_about("Set up the trop database and configuration"),
            Command::new("list-projects")
                .about("List all unique project identifiers")
                .long_about("Display all unique project names in the database"),
            Command::new("migrate")
                .about("Migrate reservations between paths")
                .long_about("Move port reservations from one directory to another"),
            Command::new("completions")
                .about("Generate shell completion scripts")
                .long_about("Generate shell completion scripts for bash, zsh, fish, or PowerShell"),
        ])
}

fn main() {
    // Generate man pages at build time
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir).unwrap();

    // Generate main trop.1 man page
    let app = build_cli();
    let man = Man::new(app);
    let mut buffer = Vec::new();
    man.render(&mut buffer).unwrap();

    fs::write(man_dir.join("trop.1"), buffer).unwrap();

    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=src/commands/");
}
