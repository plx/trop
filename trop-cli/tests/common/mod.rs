//! Common test utilities for CLI integration tests.
//!
//! This module provides shared helpers for CLI testing, including:
//! - Test environment setup with temporary directories
//! - Command builder helpers for common patterns
//! - Assertion helpers for common checks
//! - Test data fixtures

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Test environment with isolated data directory.
///
/// This struct provides an isolated test environment with:
/// - A temporary directory for test files
/// - A separate data directory for trop database
/// - Helper methods for common CLI operations
pub struct TestEnv {
    /// Temporary directory (kept alive for the duration of the test)
    #[allow(dead_code)]
    temp_dir: TempDir,
    /// Path to the temporary directory
    pub temp_path: PathBuf,
    /// Path to the trop data directory
    pub data_dir: PathBuf,
}

#[allow(dead_code)]
impl TestEnv {
    /// Create a new test environment.
    ///
    /// This creates:
    /// - A temporary directory for test files
    /// - A data directory path (not created yet - trop will create it)
    pub fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let temp_path = temp_dir.path().to_path_buf();
        let data_dir = temp_path.join("trop-data");

        Self {
            temp_dir,
            temp_path,
            data_dir,
        }
    }

    /// Get a bare command builder without pre-configured flags.
    ///
    /// This returns a Command with only the trop binary, allowing tests
    /// to have full control over all flags including --data-dir.
    /// Use this when you need to override the data directory or test
    /// global flag behavior.
    pub fn command_bare(&self) -> Command {
        Command::cargo_bin("trop").expect("Failed to find trop binary")
    }

    /// Get a command builder with the data directory pre-configured.
    ///
    /// This is a convenience method that returns a Command with:
    /// - The trop binary
    /// - The --data-dir flag set to this environment's data directory
    pub fn command(&self) -> Command {
        let mut cmd = self.command_bare();
        cmd.arg("--data-dir").arg(&self.data_dir);
        cmd
    }

    /// Get the temp path.
    pub fn path(&self) -> &Path {
        &self.temp_path
    }

    /// Create a subdirectory in the test environment.
    ///
    /// This creates a directory under the temporary directory and returns its path.
    pub fn create_dir(&self, name: &str) -> PathBuf {
        let path = self.temp_path.join(name);
        std::fs::create_dir_all(&path).expect("Failed to create test directory");
        path
    }

    /// Reserve a port with minimal arguments.
    ///
    /// This is a convenience method that:
    /// - Runs `trop reserve` with the given path
    /// - Uses --allow-unrelated-path to avoid path validation issues
    /// - Returns the allocated port number
    ///
    /// # Panics
    /// Panics if the reserve command fails or doesn't return a valid port.
    pub fn reserve_simple(&self, path: &Path) -> u16 {
        let output = self
            .command()
            .arg("reserve")
            .arg("--path")
            .arg(path)
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to run reserve command");

        assert!(
            output.status.success(),
            "Reserve failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
        stdout
            .trim()
            .parse()
            .expect("Output is not a valid port number")
    }

    /// Reserve a port with a tag.
    ///
    /// Similar to `reserve_simple` but includes a service tag.
    pub fn reserve_with_tag(&self, path: &Path, tag: &str) -> u16 {
        let output = self
            .command()
            .arg("reserve")
            .arg("--path")
            .arg(path)
            .arg("--tag")
            .arg(tag)
            .arg("--allow-unrelated-path")
            .output()
            .expect("Failed to run reserve command");

        assert!(
            output.status.success(),
            "Reserve failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in output");
        stdout
            .trim()
            .parse()
            .expect("Output is not a valid port number")
    }

    /// Release a reservation.
    ///
    /// Runs `trop release` for the given path.
    pub fn release(&self, path: &Path) {
        self.command()
            .arg("release")
            .arg("--path")
            .arg(path)
            .assert()
            .success();
    }

    /// Release a reservation with a tag.
    pub fn release_with_tag(&self, path: &Path, tag: &str) {
        self.command()
            .arg("release")
            .arg("--path")
            .arg(path)
            .arg("--tag")
            .arg(tag)
            .assert()
            .success();
    }

    /// List all reservations and return stdout.
    pub fn list(&self) -> String {
        let output = self
            .command()
            .arg("list")
            .output()
            .expect("Failed to run list command");

        assert!(
            output.status.success(),
            "List failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("Invalid UTF-8 in output")
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to parse a port number from command output.
///
/// This function takes the stdout from a reserve command and extracts
/// the port number, handling common formatting issues.
#[allow(dead_code)]
pub fn parse_port(output: &str) -> u16 {
    output
        .trim()
        .parse()
        .expect("Output is not a valid port number")
}

/// Helper to check if a string contains a port number.
///
/// This is useful for asserting that list output contains a specific port.
#[allow(dead_code)]
pub fn contains_port(haystack: &str, port: u16) -> bool {
    haystack.contains(&port.to_string())
}
