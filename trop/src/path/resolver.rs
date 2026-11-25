//! Path resolution with provenance-aware canonicalization.
//!
//! This module provides the `PathResolver` type, which is the main interface
//! for resolving paths according to their provenance.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::path::types::{PathProvenance, ResolvedPath};
use crate::path::{canonicalize, normalize};
use crate::verified::path::{resolve_by_provenance, Resolution};

/// Canonicalizes `path`, following at most `max_symlink_depth` symbolic
/// links at the path itself, and pairs a failure with whether it means
/// "not found".
fn canonicalize_reporting_not_found(
    path: &Path,
    max_symlink_depth: usize,
) -> std::result::Result<PathBuf, (Error, bool)> {
    canonicalize::canonicalize_bounded(path, max_symlink_depth).map_err(|error| {
        let not_found = error.is_not_found();
        (error, not_found)
    })
}

/// Resolves paths according to provenance rules.
///
/// The `PathResolver` applies different processing based on how a path was
/// provided:
/// - **Explicit paths** (CLI args, env vars): Normalized only; the target need
///   not exist
/// - **Implicit paths** (CWD, inferred): Normalized and canonicalized; failure
///   to canonicalize is an error
///
/// # Examples
///
/// ```no_run
/// use trop::path::PathResolver;
/// use std::path::Path;
///
/// let resolver = PathResolver::new();
///
/// // Explicit paths are normalized but not canonicalized
/// let explicit = resolver.resolve_explicit(Path::new("~/project")).unwrap();
/// assert!(!explicit.was_canonicalized());
///
/// // Implicit paths are normalized and canonicalized
/// let implicit = resolver.resolve_implicit(Path::new(".")).unwrap();
/// assert!(implicit.was_canonicalized());
/// ```
#[derive(Debug, Clone)]
pub struct PathResolver {
    /// Whether to warn before rejecting a non-existent inferred path.
    warn_on_nonexistent: bool,
    /// Maximum number of symbolic links followed at a canonicalized path
    /// itself (verified kernel `follow_links`).
    max_symlink_depth: usize,
}

impl Default for PathResolver {
    fn default() -> Self {
        Self {
            warn_on_nonexistent: true,
            max_symlink_depth: 40,
        }
    }
}

impl PathResolver {
    /// Create a new path resolver with default settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathResolver;
    ///
    /// let resolver = PathResolver::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure whether to warn on non-existent paths.
    ///
    /// When enabled, the resolver prints a warning to stderr before returning
    /// the canonicalization error for a non-existent implicit path.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathResolver;
    ///
    /// let resolver = PathResolver::new()
    ///     .with_nonexistent_warning(false);
    /// ```
    #[must_use]
    pub fn with_nonexistent_warning(mut self, warn: bool) -> Self {
        self.warn_on_nonexistent = warn;
        self
    }

    /// Configure the maximum symlink depth.
    ///
    /// This limits how many symlinks will be followed during canonicalization
    /// to prevent infinite loops: at most `depth` symbolic links are
    /// followed at the canonicalized path itself (a chain of exactly `depth`
    /// links resolves; with `0` only a path that is not a link does), and a
    /// path that links back to itself is reported as a symlink loop. Links in
    /// parent directories are resolved by the operating system, which applies
    /// its own limit. The default is 40.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathResolver;
    ///
    /// let resolver = PathResolver::new()
    ///     .with_max_symlink_depth(100);
    /// ```
    #[must_use]
    pub fn with_max_symlink_depth(mut self, depth: usize) -> Self {
        self.max_symlink_depth = depth;
        self
    }

    /// Resolve a path according to its provenance.
    ///
    /// This is the main resolution method that applies different processing
    /// based on whether the path is explicit or implicit.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Normalization fails
    /// - Canonicalization fails (for implicit paths)
    /// - An I/O error occurs
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::path::{PathResolver, PathProvenance};
    /// use std::path::Path;
    ///
    /// let resolver = PathResolver::new();
    ///
    /// // Resolve with explicit provenance
    /// let resolved = resolver.resolve(
    ///     Path::new("~/project"),
    ///     PathProvenance::Explicit
    /// ).unwrap();
    /// assert!(!resolved.was_canonicalized());
    /// ```
    pub fn resolve(&self, path: &Path, provenance: PathProvenance) -> Result<ResolvedPath> {
        // Always normalize first
        let normalized = normalize::normalize(path)?;

        // The verified provenance policy: explicit paths are only normalized
        // (the filesystem is not consulted); implicit paths are canonicalized
        // and never silently fall back to the normalized path.
        let resolution = resolve_by_provenance(
            provenance == PathProvenance::Implicit,
            self.warn_on_nonexistent,
            &|| canonicalize_reporting_not_found(&normalized, self.max_symlink_depth),
        );
        let (resolved, canonicalized) = match resolution {
            Resolution::Normalized => (normalized, false),
            Resolution::Canonical(canonical) => (canonical, true),
            Resolution::Failed {
                failure: (error, _),
                warn,
            } => {
                if warn {
                    eprintln!(
                        "Warning: Cannot canonicalize inferred path because it does not exist: {}",
                        normalized.display()
                    );
                }
                return Err(error);
            }
        };

        Ok(ResolvedPath::new(
            resolved,
            path.to_path_buf(),
            canonicalized,
            provenance,
        ))
    }

    /// Resolve a path with explicit provenance (no canonicalization).
    ///
    /// This is a convenience method for `resolve(path, PathProvenance::Explicit)`.
    ///
    /// # Errors
    ///
    /// Returns an error if normalization fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::path::PathResolver;
    /// use std::path::Path;
    ///
    /// let resolver = PathResolver::new();
    /// let resolved = resolver.resolve_explicit(Path::new("~/project")).unwrap();
    /// assert!(!resolved.was_canonicalized());
    /// ```
    pub fn resolve_explicit(&self, path: &Path) -> Result<ResolvedPath> {
        self.resolve(path, PathProvenance::Explicit)
    }

    /// Resolve a path with implicit provenance (with canonicalization).
    ///
    /// This is a convenience method for `resolve(path, PathProvenance::Implicit)`.
    ///
    /// # Errors
    ///
    /// Returns an error if normalization or canonicalization fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::path::PathResolver;
    /// use std::path::Path;
    ///
    /// let resolver = PathResolver::new();
    /// let resolved = resolver.resolve_implicit(Path::new(".")).unwrap();
    /// ```
    pub fn resolve_implicit(&self, path: &Path) -> Result<ResolvedPath> {
        self.resolve(path, PathProvenance::Implicit)
    }

    /// Force canonicalization regardless of provenance.
    ///
    /// This method always canonicalizes the path, even for explicit provenance.
    /// This is useful when you need the real path regardless of how it was
    /// provided.
    ///
    /// # Errors
    ///
    /// Returns an error if normalization or canonicalization fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use trop::path::PathResolver;
    /// use std::path::Path;
    ///
    /// let resolver = PathResolver::new();
    /// let resolved = resolver.resolve_canonical(Path::new("~/link")).unwrap();
    /// assert!(resolved.was_canonicalized());
    /// ```
    pub fn resolve_canonical(&self, path: &Path) -> Result<ResolvedPath> {
        let normalized = normalize::normalize(path)?;
        let canonical = canonicalize::canonicalize_bounded(&normalized, self.max_symlink_depth)?;

        Ok(ResolvedPath::new(
            canonical,
            path.to_path_buf(),
            true,
            PathProvenance::Explicit, // Provenance doesn't matter here
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::tempdir;

    #[test]
    fn test_resolver_default() {
        let resolver = PathResolver::default();
        assert!(resolver.warn_on_nonexistent);
        assert_eq!(resolver.max_symlink_depth, 40);
    }

    #[test]
    fn test_resolver_new() {
        let resolver = PathResolver::new();
        assert!(resolver.warn_on_nonexistent);
        assert_eq!(resolver.max_symlink_depth, 40);
    }

    #[test]
    fn test_resolver_with_nonexistent_warning() {
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        assert!(!resolver.warn_on_nonexistent);
    }

    #[test]
    fn test_resolver_with_max_symlink_depth() {
        let resolver = PathResolver::new().with_max_symlink_depth(100);
        assert_eq!(resolver.max_symlink_depth, 100);
    }

    #[cfg(unix)]
    #[test]
    fn max_symlink_depth_bounds_implicit_and_forced_canonicalization() {
        use std::fs;
        use std::os::unix::fs::symlink;

        // Before the option was wired in, it was stored but had no effect.
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        symlink(&target, &first).unwrap();
        symlink(&first, &second).unwrap();
        let canonical_target = fs::canonicalize(&target).unwrap();

        let shallow = PathResolver::new()
            .with_nonexistent_warning(false)
            .with_max_symlink_depth(1);
        assert!(matches!(
            shallow.resolve_implicit(&second),
            Err(Error::InvalidPath { ref reason, .. }) if reason == "Too many symlinks (max 1)"
        ));
        assert!(shallow.resolve_canonical(&second).is_err());
        assert_eq!(
            shallow.resolve_implicit(&first).unwrap().path(),
            canonical_target
        );
        // Explicit paths are never canonicalized, so the bound does not apply.
        assert!(shallow.resolve_explicit(&second).is_ok());

        let exact = PathResolver::new()
            .with_nonexistent_warning(false)
            .with_max_symlink_depth(2);
        assert_eq!(
            exact.resolve_implicit(&second).unwrap().path(),
            canonical_target
        );
        assert_eq!(
            exact.resolve_canonical(&second).unwrap().path(),
            canonical_target
        );

        // A directory that is not a link resolves even with depth 0.
        let none = PathResolver::new()
            .with_nonexistent_warning(false)
            .with_max_symlink_depth(0);
        assert_eq!(
            none.resolve_implicit(&target).unwrap().path(),
            canonical_target
        );
    }

    #[cfg(unix)]
    #[test]
    fn implicit_symlink_loop_is_reported_as_a_loop() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        symlink(&b, &a).unwrap();
        symlink(&a, &b).unwrap();
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        assert!(matches!(
            resolver.resolve_implicit(&a),
            Err(Error::SymlinkLoop { .. })
        ));
    }

    #[test]
    fn test_resolve_explicit_normalizes_only() {
        let resolver = PathResolver::new();
        let cwd = env::current_dir().unwrap();
        let result = resolver.resolve_explicit(Path::new("./test")).unwrap();

        // Should be normalized (absolute)
        assert!(result.path().is_absolute());
        assert!(result.path().starts_with(&cwd));

        // Should NOT be canonicalized
        assert!(!result.was_canonicalized());

        // Should have explicit provenance
        assert_eq!(result.provenance(), PathProvenance::Explicit);
    }

    #[test]
    fn test_resolve_implicit_canonicalizes() {
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        let dir = tempdir().unwrap();
        let result = resolver.resolve_implicit(dir.path()).unwrap();

        // Should be normalized (absolute)
        assert!(result.path().is_absolute());

        // Should be canonicalized (for existing path)
        assert!(result.was_canonicalized());

        // Should have implicit provenance
        assert_eq!(result.provenance(), PathProvenance::Implicit);
    }

    #[test]
    fn test_resolve_implicit_nonexistent_is_an_error() {
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        let path = Path::new("/nonexistent/path/xyz");
        let result = resolver.resolve_implicit(path);

        assert!(
            matches!(result, Err(crate::Error::PathNotFound { .. })),
            "inferred paths must fail closed when canonicalization is impossible"
        );
    }

    #[test]
    fn test_resolve_canonical_forces_canonicalization() {
        let resolver = PathResolver::new();
        let dir = tempdir().unwrap();
        let result = resolver.resolve_canonical(dir.path()).unwrap();

        // Should be canonicalized
        assert!(result.was_canonicalized());

        // Should be absolute
        assert!(result.path().is_absolute());
    }

    #[test]
    fn test_resolve_preserves_original() {
        let resolver = PathResolver::new();
        let original = Path::new("./test");
        let result = resolver.resolve_explicit(original).unwrap();

        // Original should be preserved
        assert_eq!(result.original(), original);

        // But resolved should be different (absolute)
        assert_ne!(result.path(), original);
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_explicit_preserves_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        let resolver = PathResolver::new();
        let result = resolver.resolve_explicit(&link).unwrap();

        // Explicit should not canonicalize - should end with "link"
        assert!(result.path().ends_with("link"));
        assert!(!result.was_canonicalized());
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_implicit_follows_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        let resolver = PathResolver::new();
        let result = resolver.resolve_implicit(&link).unwrap();

        // Implicit should canonicalize - should end with "target"
        assert!(result.path().ends_with("target"));
        assert!(result.was_canonicalized());
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_canonical_follows_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        let resolver = PathResolver::new();
        let result = resolver.resolve_canonical(&link).unwrap();

        // Should always canonicalize
        assert!(result.path().ends_with("target"));
        assert!(result.was_canonicalized());
    }

    // Property-based tests
    #[cfg(feature = "property-tests")]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        // Strategy to generate valid path strings
        fn path_strategy() -> impl Strategy<Value = String> {
            prop::collection::vec("[a-zA-Z0-9_-]{1,10}", 1..=5)
                .prop_map(|parts| format!("/{}", parts.join("/")))
        }

        proptest! {
            /// Explicit resolution never sets canonicalized flag to true
            #[test]
            fn resolve_explicit_never_canonicalizes(s in path_strategy()) {
                let resolver = PathResolver::new();
                let path = Path::new(&s);
                if let Ok(resolved) = resolver.resolve_explicit(path) {
                    prop_assert!(!resolved.was_canonicalized());
                }
            }

            /// Explicit resolution preserves explicit provenance
            #[test]
            fn resolve_explicit_preserves_provenance(s in path_strategy()) {
                let resolver = PathResolver::new();
                let path = Path::new(&s);
                if let Ok(resolved) = resolver.resolve_explicit(path) {
                    prop_assert_eq!(resolved.provenance(), PathProvenance::Explicit);
                }
            }

            /// Implicit resolution has implicit provenance
            #[test]
            fn resolve_implicit_preserves_provenance(s in path_strategy()) {
                let resolver = PathResolver::new().with_nonexistent_warning(false);
                let path = Path::new(&s);
                if let Ok(resolved) = resolver.resolve_implicit(path) {
                    prop_assert_eq!(resolved.provenance(), PathProvenance::Implicit);
                }
            }

            /// Resolution preserves the original path
            #[test]
            fn resolution_preserves_original(s in path_strategy()) {
                let resolver = PathResolver::new();
                let path = Path::new(&s);
                if let Ok(resolved) = resolver.resolve_explicit(path) {
                    prop_assert_eq!(resolved.original(), path);
                }
            }

            /// Resolved paths are always absolute
            #[test]
            fn resolved_paths_always_absolute(s in path_strategy()) {
                let resolver = PathResolver::new();
                let path = Path::new(&s);
                if let Ok(resolved) = resolver.resolve_explicit(path) {
                    prop_assert!(resolved.path().is_absolute());
                }
            }

            /// resolve_canonical always canonicalizes (for existing paths)
            #[test]
            fn resolve_canonical_always_canonicalizes_existing(s in "[a-zA-Z0-9_-]{1,10}") {
                let resolver = PathResolver::new();
                // Use tempdir to ensure path exists
                let dir = tempdir().unwrap();
                let test_path = dir.path().join(&s);
                std::fs::create_dir_all(&test_path).unwrap();

                if let Ok(resolved) = resolver.resolve_canonical(&test_path) {
                    prop_assert!(resolved.was_canonicalized());
                }
            }
        }
    }
}
