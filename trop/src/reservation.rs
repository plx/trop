//! Reservation types for tracking port allocations.
//!
//! This module provides types for managing port reservations, including
//! reservation keys, metadata, and builder patterns for construction.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::path::PathResolver;
use crate::Port;

/// A unique identifier for a port reservation.
///
/// Reservations are identified by a filesystem path and an optional tag.
/// The tag allows multiple ports to be reserved for the same path.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use trop::ReservationKey;
///
/// // Untagged reservation
/// let key = ReservationKey::new(PathBuf::from("/path/to/project"), None).unwrap();
/// assert_eq!(format!("{key}"), "/path/to/project");
///
/// // Tagged reservation
/// let key = ReservationKey::new(
///     PathBuf::from("/path/to/project"),
///     Some("web".to_string())
/// ).unwrap();
/// assert_eq!(format!("{key}"), "/path/to/project:web");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReservationKey {
    /// The filesystem path associated with this reservation.
    pub path: PathBuf,
    /// An optional tag to distinguish multiple reservations for the same path.
    pub tag: Option<String>,
}

impl ReservationKey {
    /// Creates a new reservation key.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is provided but is empty after trimming whitespace.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use trop::ReservationKey;
    ///
    /// // Valid untagged key
    /// let key = ReservationKey::new(PathBuf::from("/path"), None);
    /// assert!(key.is_ok());
    ///
    /// // Valid tagged key
    /// let key = ReservationKey::new(PathBuf::from("/path"), Some("web".to_string()));
    /// assert!(key.is_ok());
    ///
    /// // Invalid: empty tag
    /// let key = ReservationKey::new(PathBuf::from("/path"), Some("".to_string()));
    /// assert!(key.is_err());
    ///
    /// // Invalid: whitespace-only tag
    /// let key = ReservationKey::new(PathBuf::from("/path"), Some("  ".to_string()));
    /// assert!(key.is_err());
    /// ```
    pub fn new(path: PathBuf, tag: Option<String>) -> Result<Self, ValidationError> {
        let tag = match tag {
            Some(t) => {
                let trimmed = t.trim();
                if trimmed.is_empty() {
                    return Err(ValidationError {
                        field: "tag".into(),
                        message: "tag must be non-empty after trimming whitespace".into(),
                    });
                }
                Some(trimmed.to_string())
            }
            None => None,
        };

        Ok(Self { path, tag })
    }

    /// Creates a new reservation key with explicit path resolution.
    ///
    /// Explicit paths are normalized but NOT canonicalized, preserving symlinks.
    /// This is appropriate for paths explicitly provided by users via CLI arguments
    /// or environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path normalization fails
    /// - The tag is provided but is empty after trimming whitespace
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::ReservationKey;
    /// use std::path::Path;
    ///
    /// // Create a reservation key with an explicit path
    /// let key = ReservationKey::with_explicit_path(
    ///     Path::new("~/project"),
    ///     None
    /// ).unwrap();
    /// ```
    pub fn with_explicit_path(
        path: impl AsRef<Path>,
        tag: Option<String>,
    ) -> crate::error::Result<Self> {
        let resolver = PathResolver::new();
        let resolved_path = resolver.resolve_explicit(path.as_ref())?;
        Self::new(resolved_path.into_path_buf(), tag).map_err(crate::error::Error::from)
    }

    /// Creates a new reservation key with implicit path resolution.
    ///
    /// Implicit paths are normalized AND canonicalized, following symlinks to their
    /// real paths. This is appropriate for paths inferred from context like the
    /// current working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path normalization fails
    /// - Path canonicalization fails (for existing paths)
    /// - The tag is provided but is empty after trimming whitespace
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::ReservationKey;
    /// use std::path::Path;
    ///
    /// // Create a reservation key with an implicit path
    /// let key = ReservationKey::with_implicit_path(
    ///     Path::new("."),
    ///     None
    /// ).unwrap();
    /// ```
    pub fn with_implicit_path(
        path: impl AsRef<Path>,
        tag: Option<String>,
    ) -> crate::error::Result<Self> {
        let resolver = PathResolver::new();
        let resolved_path = resolver.resolve_implicit(path.as_ref())?;
        Self::new(resolved_path.into_path_buf(), tag).map_err(crate::error::Error::from)
    }

    /// Converts the path to a string for database operations.
    ///
    /// This helper provides a consistent way to convert paths to strings
    /// for storage in the database. It uses `to_string_lossy()` to handle
    /// paths that may contain invalid UTF-8.
    pub(crate) fn path_as_string(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

impl std::fmt::Display for ReservationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.tag {
            Some(tag) => write!(f, "{}:{}", self.path.display(), tag),
            None => write!(f, "{}", self.path.display()),
        }
    }
}

/// A port reservation with metadata.
///
/// Reservations track which ports are allocated to which paths, along with
/// optional project and task identifiers, timestamps, and persistence settings.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use trop::{Reservation, ReservationKey, Port};
///
/// let key = ReservationKey::new(PathBuf::from("/path/to/project"), None).unwrap();
/// let port = Port::try_from(8080).unwrap();
///
/// let reservation = Reservation::builder(key, port)
///     .project(Some("my-project".to_string()))
///     .build()
///     .unwrap();
///
/// assert_eq!(reservation.port(), port);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    key: ReservationKey,
    port: Port,
    project: Option<String>,
    task: Option<String>,
    sticky: bool,
    created_at: SystemTime,
    last_used_at: SystemTime,
}

impl Reservation {
    /// Creates a new reservation builder.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use trop::{Reservation, ReservationKey, Port};
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    /// ```
    #[must_use]
    pub fn builder(key: ReservationKey, port: Port) -> ReservationBuilder {
        ReservationBuilder {
            key,
            port,
            project: None,
            task: None,
            sticky: false,
            created_at: None,
            last_used_at: None,
        }
    }

    /// Returns the reservation key.
    #[must_use]
    pub const fn key(&self) -> &ReservationKey {
        &self.key
    }

    /// Returns the reserved port.
    #[must_use]
    pub const fn port(&self) -> Port {
        self.port
    }

    /// Returns the optional project identifier.
    #[must_use]
    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    /// Returns the optional task identifier.
    #[must_use]
    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// Returns whether this reservation is sticky.
    #[must_use]
    pub const fn sticky(&self) -> bool {
        self.sticky
    }

    /// Returns the creation timestamp.
    #[must_use]
    pub const fn created_at(&self) -> SystemTime {
        self.created_at
    }

    /// Returns the last used timestamp.
    #[must_use]
    pub const fn last_used_at(&self) -> SystemTime {
        self.last_used_at
    }

    /// Checks if the reservation has expired based on the given maximum age.
    ///
    /// A reservation is considered expired if it hasn't been used for longer
    /// than `max_age`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use std::time::Duration;
    /// use trop::{Reservation, ReservationKey, Port};
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    /// let reservation = Reservation::builder(key, port).build().unwrap();
    ///
    /// // A newly created reservation is not expired
    /// assert!(!reservation.is_expired(Duration::from_secs(1)));
    /// ```
    #[must_use]
    pub fn is_expired(&self, max_age: Duration) -> bool {
        SystemTime::now()
            .duration_since(self.last_used_at)
            .map(|age| age > max_age)
            .unwrap_or(false)
    }
}

/// Builder for creating `Reservation` instances.
///
/// The builder pattern allows for flexible construction of reservations with
/// optional fields and validation.
#[derive(Debug)]
pub struct ReservationBuilder {
    key: ReservationKey,
    port: Port,
    project: Option<String>,
    task: Option<String>,
    sticky: bool,
    created_at: Option<SystemTime>,
    last_used_at: Option<SystemTime>,
}

impl ReservationBuilder {
    /// Sets the project identifier.
    ///
    /// The project string will be trimmed of leading/trailing whitespace.
    #[must_use]
    pub fn project(mut self, project: Option<String>) -> Self {
        self.project = project.map(|p| p.trim().to_string());
        self
    }

    /// Sets the task identifier.
    ///
    /// The task string will be trimmed of leading/trailing whitespace.
    #[must_use]
    pub fn task(mut self, task: Option<String>) -> Self {
        self.task = task.map(|t| t.trim().to_string());
        self
    }

    /// Sets whether the reservation is sticky.
    #[must_use]
    pub const fn sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }

    /// Sets the creation timestamp.
    #[must_use]
    pub fn created_at(mut self, created_at: SystemTime) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the last used timestamp.
    #[must_use]
    pub fn last_used_at(mut self, last_used_at: SystemTime) -> Self {
        self.last_used_at = Some(last_used_at);
        self
    }

    /// Builds the reservation.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The project is provided but is empty after trimming
    /// - The task is provided but is empty after trimming
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use trop::{Reservation, ReservationKey, Port};
    ///
    /// let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
    /// let port = Port::try_from(8080).unwrap();
    ///
    /// // Valid reservation
    /// let reservation = Reservation::builder(key.clone(), port)
    ///     .project(Some("my-project".to_string()))
    ///     .build();
    /// assert!(reservation.is_ok());
    ///
    /// // Invalid: empty project
    /// let reservation = Reservation::builder(key, port)
    ///     .project(Some("".to_string()))
    ///     .build();
    /// assert!(reservation.is_err());
    /// ```
    pub fn build(self) -> Result<Reservation, ValidationError> {
        // Validate project
        if let Some(ref project) = self.project {
            if project.is_empty() {
                return Err(ValidationError {
                    field: "project".into(),
                    message: "project must be non-empty after trimming whitespace".into(),
                });
            }
        }

        // Validate task
        if let Some(ref task) = self.task {
            if task.is_empty() {
                return Err(ValidationError {
                    field: "task".into(),
                    message: "task must be non-empty after trimming whitespace".into(),
                });
            }
        }

        let now = SystemTime::now();
        Ok(Reservation {
            key: self.key,
            port: self.port,
            project: self.project,
            task: self.task,
            sticky: self.sticky,
            created_at: self.created_at.unwrap_or(now),
            last_used_at: self.last_used_at.unwrap_or(now),
        })
    }
}

/// Error type for validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The field that failed validation.
    pub field: String,
    /// A description of the validation failure.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "validation error for '{}': {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reservation_key_untagged() {
        let key = ReservationKey::new(PathBuf::from("/path/to/project"), None).unwrap();
        assert_eq!(key.path, PathBuf::from("/path/to/project"));
        assert_eq!(key.tag, None);
        assert_eq!(format!("{key}"), "/path/to/project");
    }

    #[test]
    fn test_reservation_key_tagged() {
        let key = ReservationKey::new(PathBuf::from("/path/to/project"), Some("web".to_string()))
            .unwrap();
        assert_eq!(key.path, PathBuf::from("/path/to/project"));
        assert_eq!(key.tag, Some("web".to_string()));
        assert_eq!(format!("{key}"), "/path/to/project:web");
    }

    #[test]
    fn test_reservation_key_tag_trimming() {
        let key = ReservationKey::new(
            PathBuf::from("/path/to/project"),
            Some("  web  ".to_string()),
        )
        .unwrap();
        assert_eq!(key.tag, Some("web".to_string()));
    }

    #[test]
    fn test_reservation_key_empty_tag() {
        let result = ReservationKey::new(PathBuf::from("/path/to/project"), Some(String::new()));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.field, "tag");
        assert!(err.message.contains("non-empty"));
    }

    #[test]
    fn test_reservation_key_whitespace_only_tag() {
        let result =
            ReservationKey::new(PathBuf::from("/path/to/project"), Some("   ".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_reservation_key_equality() {
        let key1 = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        assert_eq!(key1, key2);

        let key3 = ReservationKey::new(PathBuf::from("/path"), Some("web".to_string())).unwrap();
        let key4 = ReservationKey::new(PathBuf::from("/path"), Some("web".to_string())).unwrap();
        assert_eq!(key3, key4);

        assert_ne!(key1, key3);
    }

    #[test]
    fn test_reservation_key_hash() {
        use std::collections::HashMap;

        let key1 = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let key2 = ReservationKey::new(PathBuf::from("/path"), Some("web".to_string())).unwrap();

        let mut map = HashMap::new();
        map.insert(key1, 8080);
        map.insert(key2, 8081);

        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_reservation_builder_basic() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();

        assert_eq!(reservation.key(), &key);
        assert_eq!(reservation.port(), port);
        assert_eq!(reservation.project(), None);
        assert_eq!(reservation.task(), None);
        assert!(!reservation.sticky());
    }

    #[test]
    fn test_reservation_builder_with_project() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("my-project".to_string()))
            .build()
            .unwrap();

        assert_eq!(reservation.project(), Some("my-project"));
    }

    #[test]
    fn test_reservation_builder_with_task() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port)
            .task(Some("my-task".to_string()))
            .build()
            .unwrap();

        assert_eq!(reservation.task(), Some("my-task"));
    }

    #[test]
    fn test_reservation_builder_sticky() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port)
            .sticky(true)
            .build()
            .unwrap();

        assert!(reservation.sticky());
    }

    #[test]
    fn test_reservation_builder_empty_project() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let result = Reservation::builder(key, port)
            .project(Some(String::new()))
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.field, "project");
    }

    #[test]
    fn test_reservation_builder_empty_task() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let result = Reservation::builder(key, port)
            .task(Some(String::new()))
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.field, "task");
    }

    #[test]
    fn test_reservation_builder_project_trimming() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("  my-project  ".to_string()))
            .build()
            .unwrap();

        assert_eq!(reservation.project(), Some("my-project"));
    }

    #[test]
    fn test_reservation_is_expired() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Newly created reservation is not expired
        let reservation = Reservation::builder(key.clone(), port).build().unwrap();
        assert!(!reservation.is_expired(Duration::from_secs(1)));
        assert!(!reservation.is_expired(Duration::from_secs(60)));

        // Reservation with old last_used_at is expired
        let old_time = SystemTime::now() - Duration::from_secs(100);
        let reservation = Reservation::builder(key, port)
            .last_used_at(old_time)
            .build()
            .unwrap();
        assert!(reservation.is_expired(Duration::from_secs(50)));
        assert!(!reservation.is_expired(Duration::from_secs(150)));
    }

    #[test]
    fn test_reservation_timestamps() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let now = SystemTime::now();

        let reservation = Reservation::builder(key, port)
            .created_at(now)
            .last_used_at(now)
            .build()
            .unwrap();

        assert_eq!(reservation.created_at(), now);
        assert_eq!(reservation.last_used_at(), now);
    }

    #[test]
    fn test_reservation_serde() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key, port)
            .project(Some("my-project".to_string()))
            .build()
            .unwrap();

        let json = serde_json::to_string(&reservation).unwrap();
        let deserialized: Reservation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, reservation);
    }

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError {
            field: "project".to_string(),
            message: "must be non-empty".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("project"));
        assert!(display.contains("must be non-empty"));
    }

    #[test]
    fn test_with_explicit_path_normalizes() {
        use std::env;

        // Test with a relative path
        let cwd = env::current_dir().unwrap();
        let key = ReservationKey::with_explicit_path(Path::new("./test"), None).unwrap();

        // Should be normalized to absolute
        assert!(key.path.is_absolute());
        assert!(key.path.starts_with(&cwd));
    }

    #[test]
    fn test_with_implicit_path_normalizes() {
        // Test with a relative path
        let key = ReservationKey::with_implicit_path(Path::new("./test"), None).unwrap();

        // Should be normalized to absolute
        assert!(key.path.is_absolute());
        // Note: May or may not start with cwd depending on canonicalization
    }

    #[cfg(unix)]
    #[test]
    fn test_with_explicit_path_preserves_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        // Explicit path should NOT canonicalize - should preserve "link"
        let key = ReservationKey::with_explicit_path(&link, None).unwrap();
        assert!(key.path.ends_with("link"));
    }

    #[cfg(unix)]
    #[test]
    fn test_with_implicit_path_follows_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        // Implicit path should canonicalize - should follow to "target"
        let key = ReservationKey::with_implicit_path(&link, None).unwrap();
        assert!(key.path.ends_with("target"));
    }

    #[test]
    fn test_with_explicit_path_with_tag() {
        let key =
            ReservationKey::with_explicit_path(Path::new("/test/path"), Some("web".to_string()))
                .unwrap();
        assert_eq!(key.tag, Some("web".to_string()));
    }

    #[test]
    fn test_with_implicit_path_with_tag() {
        let key =
            ReservationKey::with_implicit_path(Path::new("/test/path"), Some("web".to_string()))
                .unwrap();
        assert_eq!(key.tag, Some("web".to_string()));
    }

    #[test]
    fn test_with_explicit_path_empty_tag_fails() {
        let result =
            ReservationKey::with_explicit_path(Path::new("/test/path"), Some(String::new()));
        assert!(result.is_err());
    }

    #[test]
    fn test_with_implicit_path_empty_tag_fails() {
        let result =
            ReservationKey::with_implicit_path(Path::new("/test/path"), Some(String::new()));
        assert!(result.is_err());
    }
}
