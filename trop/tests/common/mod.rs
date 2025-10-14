//! Common test utilities for integration tests.
//!
//! This module provides helper functions and fixture builders for testing
//! the trop library.

pub mod database;

use std::path::PathBuf;
use std::time::SystemTime;

use trop::{Port, Reservation, ReservationKey};

/// Creates a temporary directory for testing.
///
/// The directory will be automatically cleaned up when the returned
/// `TempDir` is dropped.
#[allow(dead_code)]
pub fn create_temp_dir() -> std::io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}

/// Creates a test database in a temporary location.
///
/// This is a placeholder for Phase 2 when database functionality is implemented.
/// For now, it just returns a path where the database should be created.
#[allow(dead_code)]
pub fn create_test_database() -> std::io::Result<PathBuf> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("test.db");
    // Keep the temp_dir alive by forgetting it - this is a test helper
    std::mem::forget(temp_dir);
    Ok(db_path)
}

/// Builder for creating test reservations with sensible defaults.
///
/// # Examples
///
/// ```no_run
/// # use common::ReservationFixture;
/// let reservation = ReservationFixture::new()
///     .with_path("/test/project")
///     .with_port(8080)
///     .build();
/// ```
#[allow(dead_code)]
pub struct ReservationFixture {
    path: PathBuf,
    tag: Option<String>,
    port: u16,
    project: Option<String>,
    task: Option<String>,
    sticky: bool,
    created_at: Option<SystemTime>,
    last_used_at: Option<SystemTime>,
}

impl ReservationFixture {
    /// Creates a new fixture builder with default values.
    ///
    /// Defaults:
    /// - path: "/test/project"
    /// - tag: None
    /// - port: 8080
    /// - project: None
    /// - task: None
    /// - sticky: false
    /// - timestamps: current time
    pub fn new() -> Self {
        Self {
            path: PathBuf::from("/test/project"),
            tag: None,
            port: 8080,
            project: None,
            task: None,
            sticky: false,
            created_at: None,
            last_used_at: None,
        }
    }

    /// Sets the path for the reservation.
    pub fn with_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = path.into();
        self
    }

    /// Sets the tag for the reservation.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Sets the port number for the reservation.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the project identifier for the reservation.
    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }

    /// Sets the task identifier for the reservation.
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Sets whether the reservation is sticky.
    pub fn with_sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }

    /// Sets the creation timestamp.
    #[allow(dead_code)]
    pub fn with_created_at(mut self, created_at: SystemTime) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the last used timestamp.
    #[allow(dead_code)]
    pub fn with_last_used_at(mut self, last_used_at: SystemTime) -> Self {
        self.last_used_at = Some(last_used_at);
        self
    }

    /// Builds the reservation.
    ///
    /// # Panics
    ///
    /// Panics if the port is invalid (0) or if the reservation fails validation.
    /// This is acceptable in test code where we want to fail fast on invalid fixtures.
    pub fn build(self) -> Reservation {
        let key = ReservationKey::new(self.path, self.tag)
            .expect("fixture should have valid reservation key");
        let port = Port::try_from(self.port).expect("fixture should have valid port");

        let mut builder = Reservation::builder(key, port)
            .project(self.project)
            .task(self.task)
            .sticky(self.sticky);

        if let Some(created_at) = self.created_at {
            builder = builder.created_at(created_at);
        }

        if let Some(last_used_at) = self.last_used_at {
            builder = builder.last_used_at(last_used_at);
        }

        builder
            .build()
            .expect("fixture should build valid reservation")
    }
}

impl Default for ReservationFixture {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_default() {
        let reservation = ReservationFixture::new().build();
        assert_eq!(reservation.port().value(), 8080);
        assert_eq!(reservation.key().path, PathBuf::from("/test/project"));
        assert_eq!(reservation.key().tag, None);
    }

    #[test]
    fn test_fixture_custom() {
        let reservation = ReservationFixture::new()
            .with_path("/custom/path")
            .with_tag("web")
            .with_port(9090)
            .with_project("test-project")
            .with_task("test-task")
            .with_sticky(true)
            .build();

        assert_eq!(reservation.port().value(), 9090);
        assert_eq!(reservation.key().path, PathBuf::from("/custom/path"));
        assert_eq!(reservation.key().tag, Some("web".to_string()));
        assert_eq!(reservation.project(), Some("test-project"));
        assert_eq!(reservation.task(), Some("test-task"));
        assert!(reservation.sticky());
    }

    #[test]
    fn test_temp_dir_creation() {
        let temp_dir = create_temp_dir().expect("should create temp dir");
        assert!(temp_dir.path().exists());
    }
}
