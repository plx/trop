//! Core types for path handling.
//!
//! This module defines the fundamental types used throughout the path handling
//! system, including path provenance tracking and resolved path representations.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Indicates how a path was provided to the system.
///
/// Path provenance determines how the path is processed:
/// - **Explicit** paths (CLI args, env vars) are normalized but NOT canonicalized
/// - **Implicit** paths (CWD, inferred) are normalized AND canonicalized
///
/// # Examples
///
/// ```
/// use trop::path::PathProvenance;
///
/// let explicit = PathProvenance::Explicit;
/// let implicit = PathProvenance::Implicit;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathProvenance {
    /// Path explicitly provided via CLI argument or environment variable.
    ///
    /// These paths are normalized (made absolute, tilde expanded, etc.) but
    /// NOT canonicalized (symlinks are preserved).
    Explicit,

    /// Path inferred from current working directory or similar context.
    ///
    /// These paths are both normalized AND canonicalized (symlinks are followed
    /// to their real paths).
    Implicit,
}

/// A path that has been normalized to absolute form.
///
/// Normalization includes:
/// - Tilde expansion (~)
/// - Conversion to absolute path
/// - Resolution of `.` and `..` components
/// - Platform-specific path separator normalization
///
/// # Examples
///
/// ```
/// use trop::path::{NormalizedPath, PathProvenance};
/// use std::path::PathBuf;
///
/// let path = PathBuf::from("/absolute/path");
/// let normalized = NormalizedPath::new(path, PathProvenance::Explicit).unwrap();
/// assert!(normalized.as_path().is_absolute());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedPath {
    path: PathBuf,
    provenance: PathProvenance,
}

impl NormalizedPath {
    /// Create a new normalized path.
    ///
    /// # Errors
    ///
    /// Returns an error if the path is not absolute.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{NormalizedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = PathBuf::from("/absolute/path");
    /// let normalized = NormalizedPath::new(path, PathProvenance::Explicit).unwrap();
    /// ```
    pub fn new(path: PathBuf, provenance: PathProvenance) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::InvalidPath {
                path,
                reason: "Path must be absolute after normalization".to_string(),
            });
        }
        Ok(Self { path, provenance })
    }

    /// Get a reference to the path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{NormalizedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = PathBuf::from("/absolute/path");
    /// let normalized = NormalizedPath::new(path.clone(), PathProvenance::Explicit).unwrap();
    /// assert_eq!(normalized.as_path(), path.as_path());
    /// ```
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// Get the provenance of this path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{NormalizedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = PathBuf::from("/absolute/path");
    /// let normalized = NormalizedPath::new(path, PathProvenance::Explicit).unwrap();
    /// assert_eq!(normalized.provenance(), PathProvenance::Explicit);
    /// ```
    #[must_use]
    pub fn provenance(&self) -> PathProvenance {
        self.provenance
    }

    /// Convert into the underlying `PathBuf`.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{NormalizedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = PathBuf::from("/absolute/path");
    /// let normalized = NormalizedPath::new(path.clone(), PathProvenance::Explicit).unwrap();
    /// assert_eq!(normalized.into_path_buf(), path);
    /// ```
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

/// A fully resolved path with metadata about the resolution process.
///
/// This type represents a path that has been both normalized and optionally
/// canonicalized based on its provenance.
///
/// # Examples
///
/// ```
/// use trop::path::{ResolvedPath, PathProvenance};
/// use std::path::PathBuf;
///
/// let original = PathBuf::from("~/project");
/// let resolved = PathBuf::from("/home/user/project");
/// let path = ResolvedPath::new(
///     resolved.clone(),
///     original,
///     false,
///     PathProvenance::Explicit
/// );
/// assert_eq!(path.path(), &resolved);
/// assert!(!path.was_canonicalized());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedPath {
    /// The resolved path (normalized and possibly canonicalized).
    path: PathBuf,
    /// The original path before any processing.
    original: PathBuf,
    /// Whether canonicalization was applied.
    canonicalized: bool,
    /// The source of the path.
    provenance: PathProvenance,
}

impl ResolvedPath {
    /// Create a new resolved path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let resolved = ResolvedPath::new(
    ///     PathBuf::from("/resolved/path"),
    ///     PathBuf::from("~/path"),
    ///     false,
    ///     PathProvenance::Explicit
    /// );
    /// ```
    #[must_use]
    pub fn new(
        path: PathBuf,
        original: PathBuf,
        canonicalized: bool,
        provenance: PathProvenance,
    ) -> Self {
        Self {
            path,
            original,
            canonicalized,
            provenance,
        }
    }

    /// Get a reference to the resolved path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let resolved_path = PathBuf::from("/resolved/path");
    /// let path = ResolvedPath::new(
    ///     resolved_path.clone(),
    ///     PathBuf::from("~/path"),
    ///     false,
    ///     PathProvenance::Explicit
    /// );
    /// assert_eq!(path.path(), &resolved_path);
    /// ```
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get a reference to the original path before resolution.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let original = PathBuf::from("~/path");
    /// let path = ResolvedPath::new(
    ///     PathBuf::from("/resolved/path"),
    ///     original.clone(),
    ///     false,
    ///     PathProvenance::Explicit
    /// );
    /// assert_eq!(path.original(), &original);
    /// ```
    #[must_use]
    pub fn original(&self) -> &Path {
        &self.original
    }

    /// Check if this path was canonicalized.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = ResolvedPath::new(
    ///     PathBuf::from("/resolved/path"),
    ///     PathBuf::from("~/path"),
    ///     true,
    ///     PathProvenance::Implicit
    /// );
    /// assert!(path.was_canonicalized());
    /// ```
    #[must_use]
    pub fn was_canonicalized(&self) -> bool {
        self.canonicalized
    }

    /// Get the provenance of this path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let path = ResolvedPath::new(
    ///     PathBuf::from("/resolved/path"),
    ///     PathBuf::from("~/path"),
    ///     false,
    ///     PathProvenance::Explicit
    /// );
    /// assert_eq!(path.provenance(), PathProvenance::Explicit);
    /// ```
    #[must_use]
    pub fn provenance(&self) -> PathProvenance {
        self.provenance
    }

    /// Convert into the underlying resolved `PathBuf`.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::{ResolvedPath, PathProvenance};
    /// use std::path::PathBuf;
    ///
    /// let resolved_path = PathBuf::from("/resolved/path");
    /// let path = ResolvedPath::new(
    ///     resolved_path.clone(),
    ///     PathBuf::from("~/path"),
    ///     false,
    ///     PathProvenance::Explicit
    /// );
    /// assert_eq!(path.into_path_buf(), resolved_path);
    /// ```
    #[must_use]
    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_path_requires_absolute() {
        let relative = PathBuf::from("relative/path");
        let result = NormalizedPath::new(relative, PathProvenance::Explicit);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_normalized_path_accepts_absolute() {
        let absolute = PathBuf::from("/absolute/path");
        let result = NormalizedPath::new(absolute.clone(), PathProvenance::Explicit);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_path(), absolute.as_path());
    }

    #[test]
    fn test_resolved_path_construction() {
        let resolved = PathBuf::from("/home/user/project");
        let original = PathBuf::from("~/project");
        let path = ResolvedPath::new(
            resolved.clone(),
            original.clone(),
            false,
            PathProvenance::Explicit,
        );

        assert_eq!(path.path(), resolved.as_path());
        assert_eq!(path.original(), original.as_path());
        assert!(!path.was_canonicalized());
        assert_eq!(path.provenance(), PathProvenance::Explicit);
    }
}
