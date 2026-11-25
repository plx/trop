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
/// let display = format!("{key}").replace(std::path::MAIN_SEPARATOR, "/");
/// assert_eq!(display, "/path/to/project");
///
/// // Tagged reservation
/// let key = ReservationKey::new(
///     PathBuf::from("/path/to/project"),
///     Some("web".to_string())
/// ).unwrap();
/// let display = format!("{key}").replace(std::path::MAIN_SEPARATOR, "/");
/// assert_eq!(display, "/path/to/project:web");
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
    /// A provided tag is trimmed of leading and trailing Unicode whitespace,
    /// and the trimmed value is stored. Tags follow the same rule as project
    /// and task identifiers.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is provided and, after trimming whitespace,
    /// it is empty, contains a NUL character, or is longer than 255 UTF-8
    /// bytes (checked in that order).
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
    ///
    /// // Invalid: longer than 255 bytes
    /// let key = ReservationKey::new(PathBuf::from("/path"), Some("a".repeat(256)));
    /// assert!(key.is_err());
    /// ```
    pub fn new(path: PathBuf, tag: Option<String>) -> Result<Self, ValidationError> {
        let tag = match tag {
            Some(t) => Some(
                crate::identifier::check_identifier(&t)
                    .map_err(tag_validation_error)?
                    .to_owned(),
            ),
            None => None,
        };

        Ok(Self { path, tag })
    }

    /// Rebuilds a key from a stored database row.
    ///
    /// Stored tags may have been written by earlier versions of trop, which
    /// did not limit tag length, so this only requires the canonical stored
    /// form (non-empty and already trimmed) and keeps every such row
    /// readable. Unlike [`Self::new`], it neither trims nor applies the
    /// NUL and length limits.
    ///
    /// # Errors
    ///
    /// Returns an error if the tag is provided but is empty or not already
    /// trimmed.
    #[doc(hidden)]
    pub fn from_stored(path: PathBuf, tag: Option<String>) -> Result<Self, ValidationError> {
        if let Some(tag) = &tag {
            if !crate::identifier::is_canonical_identifier(tag) {
                return Err(ValidationError {
                    field: "tag".into(),
                    message: "stored tag must be non-empty and already trimmed".into(),
                });
            }
        }
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
    /// - The tag is provided but violates the tag rule (see [`Self::new`])
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
    /// - Path canonicalization fails, including when the inferred path does not exist
    /// - The tag is provided but violates the tag rule (see [`Self::new`])
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
#[derive(Clone, Serialize, Deserialize)]
pub struct Reservation {
    key: ReservationKey,
    port: Port,
    project: Option<String>,
    task: Option<String>,
    sticky: bool,
    created_at: SystemTime,
    last_used_at: SystemTime,
    #[serde(skip)]
    deferred_reserve: Option<Box<crate::operations::reserve::DeferredReserve>>,
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

    /// Whether this reservation is a plan placeholder whose final port will be
    /// selected atomically by [`crate::operations::PlanExecutor`].
    ///
    /// This is true only for a reserve plan created after initial allocation
    /// exhaustion. In that case [`Self::port`] is a non-persistent placeholder,
    /// and the executor's result contains the selected port.
    #[must_use]
    pub const fn requires_allocation_at_execution(&self) -> bool {
        self.deferred_reserve.is_some()
    }

    pub(crate) fn with_deferred_reserve(
        mut self,
        deferred_reserve: crate::operations::reserve::DeferredReserve,
    ) -> Self {
        self.deferred_reserve = Some(Box::new(deferred_reserve));
        self
    }

    pub(crate) fn deferred_reserve(&self) -> Option<&crate::operations::reserve::DeferredReserve> {
        self.deferred_reserve.as_deref()
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
            .is_ok_and(|age| age > max_age)
    }
}

// Preserve the published persisted-record debug representation; deferred
// executor intent is neither stored reservation data nor safe diagnostic text.
#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("key", &self.key)
            .field("port", &self.port)
            .field("project", &self.project)
            .field("task", &self.task)
            .field("sticky", &self.sticky)
            .field("created_at", &self.created_at)
            .field("last_used_at", &self.last_used_at)
            .finish()
    }
}

impl PartialEq for Reservation {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.port == other.port
            && self.project == other.project
            && self.task == other.task
            && self.sticky == other.sticky
            && self.created_at == other.created_at
            && self.last_used_at == other.last_used_at
    }
}

impl Eq for Reservation {}

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
        self.project = project.map(|p| crate::identifier::trim_identifier(&p).to_owned());
        self
    }

    /// Sets the task identifier.
    ///
    /// The task string will be trimmed of leading/trailing whitespace.
    #[must_use]
    pub fn task(mut self, task: Option<String>) -> Self {
        self.task = task.map(|t| crate::identifier::trim_identifier(&t).to_owned());
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
            deferred_reserve: None,
        })
    }
}

/// Maps a rejected tag to its validation error.
fn tag_validation_error(error: crate::identifier::IdentifierError) -> ValidationError {
    use crate::identifier::IdentifierError;

    let message = match error {
        IdentifierError::Blank => "tag must be non-empty after trimming whitespace",
        IdentifierError::ContainsNul => "tag must not contain null bytes",
        IdentifierError::TooLong => "tag must not exceed 255 bytes after trimming whitespace",
    };
    ValidationError {
        field: "tag".into(),
        message: message.into(),
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

#[cfg(all(test, feature = "property-tests"))]
mod proptests;

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

    /// Regression: tags used to bypass the shared identifier length limit.
    #[test]
    fn test_reservation_key_rejects_tag_over_255_bytes() {
        let path = PathBuf::from("/path/to/project");
        let err = ReservationKey::new(path.clone(), Some("a".repeat(256))).unwrap_err();
        assert_eq!(err.field, "tag");
        assert!(err.message.contains("255 bytes"), "{}", err.message);

        // Bytes, not characters: 128 two-byte characters are 256 bytes.
        assert!(ReservationKey::new(path.clone(), Some("é".repeat(128))).is_err());
        assert!(ReservationKey::new(path, Some("é".repeat(127))).is_ok());
    }

    /// Regression: tags used to bypass the shared identifier NUL check.
    #[test]
    fn test_reservation_key_rejects_tag_with_nul() {
        let path = PathBuf::from("/path/to/project");
        for tag in ["web\0", "\0", " we\0b ", "\0 "] {
            let err = ReservationKey::new(path.clone(), Some(tag.to_string())).unwrap_err();
            assert_eq!(err.field, "tag");
            assert!(err.message.contains("null"), "{}", err.message);
        }
    }

    #[test]
    fn test_reservation_key_tag_rule_boundaries() {
        let path = PathBuf::from("/path");
        let maximal = "a".repeat(255);
        let key =
            ReservationKey::new(path.clone(), Some(format!("\u{3000} {maximal}\u{a0}"))).unwrap();
        assert_eq!(key.tag.as_deref(), Some(maximal.as_str()));

        // Blank is reported before NUL and length.
        let err = ReservationKey::new(path.clone(), Some("\u{2003}\t".to_string())).unwrap_err();
        assert!(err.message.contains("non-empty"), "{}", err.message);
        // NUL is reported before length.
        let err =
            ReservationKey::new(path.clone(), Some(format!("\0{}", "a".repeat(300)))).unwrap_err();
        assert!(err.message.contains("null"), "{}", err.message);

        // Unicode whitespace is trimmed exactly like `str::trim`; other
        // invisible characters (zero-width space) are kept.
        let key = ReservationKey::new(path, Some("\u{200b}web\u{85}".to_string())).unwrap();
        assert_eq!(key.tag.as_deref(), Some("\u{200b}web"));
    }

    /// Stored rows stay readable: the stored-row constructor accepts any
    /// canonical tag, including legacy tags the write-side rule now rejects.
    #[test]
    fn test_reservation_key_from_stored_is_lenient_but_canonical() {
        let path = PathBuf::from("/path");
        for tag in ["web".to_string(), "a".repeat(300), "we\0b".to_string()] {
            let key = ReservationKey::from_stored(path.clone(), Some(tag.clone())).unwrap();
            assert_eq!(key.tag, Some(tag));
        }
        assert_eq!(
            ReservationKey::from_stored(path.clone(), None).unwrap().tag,
            None
        );
        for tag in ["", " web", "web ", "\u{3000}"] {
            assert!(ReservationKey::from_stored(path.clone(), Some(tag.to_string())).is_err());
        }
    }

    #[test]
    fn test_reservation_builder_trims_unicode_whitespace() {
        let key = ReservationKey::new(PathBuf::from("/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();
        let reservation = Reservation::builder(key.clone(), port)
            .project(Some("\u{3000}my-project\u{a0}".to_string()))
            .task(Some("\u{2028}task\n".to_string()))
            .build()
            .unwrap();
        assert_eq!(reservation.project(), Some("my-project"));
        assert_eq!(reservation.task(), Some("task"));

        let err = Reservation::builder(key, port)
            .project(Some("\u{2009}".to_string()))
            .build()
            .unwrap_err();
        assert_eq!(err.field, "project");
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
        let dir = tempfile::tempdir().unwrap();
        let key = ReservationKey::with_implicit_path(dir.path(), None).unwrap();

        assert!(key.path.is_absolute());
        assert_eq!(key.path, dir.path().canonicalize().unwrap());
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
        let dir = tempfile::tempdir().unwrap();
        let key = ReservationKey::with_implicit_path(dir.path(), Some("web".to_string())).unwrap();
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
