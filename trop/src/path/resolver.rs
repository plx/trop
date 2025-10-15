//! Path resolution with provenance-aware canonicalization.
//!
//! This module provides the `PathResolver` type, which is the main interface
//! for resolving paths according to their provenance.

use std::path::Path;

use crate::error::Result;
use crate::path::types::{PathProvenance, ResolvedPath};
use crate::path::{canonicalize, normalize};

/// Resolves paths according to provenance rules.
///
/// The `PathResolver` applies different processing based on how a path was
/// provided:
/// - **Explicit paths** (CLI args, env vars): Normalized only
/// - **Implicit paths** (CWD, inferred): Normalized and canonicalized
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
/// assert!(implicit.was_canonicalized() || !Path::new(".").exists());
/// ```
#[derive(Debug, Clone)]
pub struct PathResolver {
    /// Whether to warn on non-existent paths.
    warn_on_nonexistent: bool,
    /// Maximum symlink depth for safe canonicalization.
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
    /// When enabled, the resolver will print a warning to stderr if it
    /// encounters a non-existent path during implicit resolution.
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
    /// to prevent infinite loops.
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

        // Decide whether to canonicalize based on provenance
        let (resolved, canonicalized) = match provenance {
            PathProvenance::Explicit => {
                // Explicit paths: normalize only
                (normalized.clone(), false)
            }
            PathProvenance::Implicit => {
                // Implicit paths: normalize and canonicalize
                match canonicalize::canonicalize(&normalized) {
                    Ok(canonical) => (canonical, true),
                    Err(e) if e.is_not_found() => {
                        // Path doesn't exist - use normalized version
                        if self.warn_on_nonexistent {
                            eprintln!("Warning: Path does not exist: {}", normalized.display());
                        }
                        (normalized.clone(), false)
                    }
                    Err(e) => return Err(e),
                }
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
        let canonical = canonicalize::canonicalize(&normalized)?;

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
    fn test_resolve_implicit_nonexistent() {
        let resolver = PathResolver::new().with_nonexistent_warning(false);
        let path = Path::new("/nonexistent/path/xyz");
        let result = resolver.resolve_implicit(path).unwrap();

        // Should be normalized
        assert!(result.path().is_absolute());

        // Should NOT be canonicalized (path doesn't exist)
        assert!(!result.was_canonicalized());

        // Should have implicit provenance
        assert_eq!(result.provenance(), PathProvenance::Implicit);
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
