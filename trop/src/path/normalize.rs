//! Path normalization functions.
//!
//! This module provides functionality to normalize paths by:
//! - Expanding tilde (~) to the home directory
//! - Converting relative paths to absolute paths
//! - Resolving `.` and `..` components
//! - Normalizing path separators for the platform

use std::env;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Expand tilde (~) to the home directory.
///
/// This function handles `~` and `~/path` but does not support `~user` syntax.
///
/// # Errors
///
/// Returns an error if:
/// - The path contains invalid UTF-8
/// - The home directory cannot be determined
/// - The path uses `~user` syntax (not supported)
///
/// # Examples
///
/// ```
/// use trop::path::normalize::expand_tilde;
/// use std::path::Path;
///
/// // Expands ~ to home directory
/// let expanded = expand_tilde(Path::new("~")).unwrap();
/// assert!(expanded.is_absolute());
///
/// // Expands ~/path to home/path
/// let expanded = expand_tilde(Path::new("~/project")).unwrap();
/// assert!(expanded.is_absolute());
/// assert!(expanded.ends_with("project"));
///
/// // Leaves absolute paths unchanged
/// let expanded = expand_tilde(Path::new("/absolute")).unwrap();
/// assert_eq!(expanded, Path::new("/absolute"));
/// ```
pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let path_str = path.to_str().ok_or_else(|| Error::InvalidPath {
        path: path.to_path_buf(),
        reason: "Path contains invalid UTF-8".to_string(),
    })?;

    if !path_str.starts_with('~') {
        return Ok(path.to_path_buf());
    }

    // Get home directory using the home crate
    let home = home::home_dir().ok_or_else(|| Error::InvalidPath {
        path: path.to_path_buf(),
        reason: "Cannot determine home directory".to_string(),
    })?;

    if path_str == "~" {
        Ok(home)
    } else if path_str.starts_with("~/") || path_str.starts_with("~\\") {
        Ok(home.join(&path_str[2..]))
    } else {
        // ~user syntax not supported
        Err(Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "~user syntax is not supported; use ~ or ~/path".to_string(),
        })
    }
}

/// Resolve `.` and `..` components in an absolute path.
///
/// This function processes path components to remove current directory (`.`)
/// references and resolve parent directory (`..`) references.
///
/// # Errors
///
/// Returns an error if the path contains too many `..` components that would
/// escape the root directory.
///
/// # Examples
///
/// ```
/// use trop::path::normalize::resolve_components;
/// use std::path::{Path, PathBuf};
///
/// // Resolves . and ..
/// let resolved = resolve_components(Path::new("/a/./b/../c")).unwrap();
/// assert_eq!(resolved, PathBuf::from("/a/c"));
///
/// // Handles multiple ..
/// let resolved = resolve_components(Path::new("/a/b/../../c")).unwrap();
/// assert_eq!(resolved, PathBuf::from("/c"));
/// ```
pub fn resolve_components(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::RootDir => {
                result.push(component);
                has_root = true;
            }
            Component::Prefix(prefix) => {
                // Windows prefix
                result.push(prefix.as_os_str());
                has_root = true;
            }
            Component::Normal(c) => {
                result.push(c);
            }
            Component::CurDir => {
                // Skip "." - it doesn't change the path
            }
            Component::ParentDir => {
                // Try to pop the last component for ".."
                if !result.pop() {
                    // Already at root - can't go up further
                    return Err(Error::InvalidPath {
                        path: path.to_path_buf(),
                        reason: "Path contains too many '..' components (escapes root)".to_string(),
                    });
                }
            }
        }
    }

    // Ensure we at least have a root if we started with one
    if has_root && result.as_os_str().is_empty() {
        result.push(Component::RootDir);
    }

    Ok(result)
}

/// Normalize a path to absolute form.
///
/// This is the main normalization function that:
/// 1. Expands tilde (~) if present
/// 2. Converts relative paths to absolute (using current directory)
/// 3. Resolves `.` and `..` components
///
/// # Errors
///
/// Returns an error if:
/// - Tilde expansion fails
/// - Current directory cannot be determined
/// - Path contains too many `..` components
/// - Path is invalid for the platform
///
/// # Examples
///
/// ```no_run
/// use trop::path::normalize::normalize;
/// use std::path::Path;
///
/// // Normalize tilde path
/// let normalized = normalize(Path::new("~/project")).unwrap();
/// assert!(normalized.is_absolute());
///
/// // Normalize relative path
/// let normalized = normalize(Path::new("./src")).unwrap();
/// assert!(normalized.is_absolute());
///
/// // Resolve . and ..
/// let normalized = normalize(Path::new("/a/./b/../c")).unwrap();
/// assert_eq!(normalized, Path::new("/a/c"));
/// ```
pub fn normalize(path: &Path) -> Result<PathBuf> {
    // First expand tilde if present
    let expanded = expand_tilde(path)?;

    // Make absolute if not already
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        let cwd = env::current_dir().map_err(|e| Error::InvalidPath {
            path: path.to_path_buf(),
            reason: format!("Cannot get current directory: {e}"),
        })?;
        cwd.join(expanded)
    };

    // Resolve . and .. components
    resolve_components(&absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde_home() {
        let home = home::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~")).unwrap(), home);
    }

    #[test]
    fn test_expand_tilde_with_path() {
        let home = home::home_dir().unwrap();
        let expanded = expand_tilde(Path::new("~/test")).unwrap();
        assert_eq!(expanded, home.join("test"));
    }

    #[test]
    fn test_expand_tilde_absolute_unchanged() {
        let path = Path::new("/absolute/path");
        assert_eq!(expand_tilde(path).unwrap(), path);
    }

    #[test]
    fn test_expand_tilde_user_syntax_not_supported() {
        let result = expand_tilde(Path::new("~user/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_components_simple() {
        let resolved = resolve_components(Path::new("/a/./b/../c")).unwrap();
        assert_eq!(resolved, PathBuf::from("/a/c"));
    }

    #[test]
    fn test_resolve_components_multiple_parent() {
        let resolved = resolve_components(Path::new("/a/b/../../c")).unwrap();
        assert_eq!(resolved, PathBuf::from("/c"));
    }

    #[test]
    fn test_resolve_components_root_only() {
        let resolved = resolve_components(Path::new("/")).unwrap();
        assert_eq!(resolved, PathBuf::from("/"));
    }

    #[test]
    fn test_resolve_components_too_many_parent() {
        let result = resolve_components(Path::new("/a/../.."));
        assert!(result.is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_normalize_absolute() {
        let path = Path::new("/a/./b/../c");
        let normalized = normalize(path).unwrap();
        assert_eq!(normalized, PathBuf::from("/a/c"));
        assert!(normalized.is_absolute());
    }

    #[test]
    fn test_normalize_relative() {
        let cwd = env::current_dir().unwrap();
        let normalized = normalize(Path::new("relative/path")).unwrap();
        assert!(normalized.is_absolute());
        assert!(normalized.starts_with(&cwd));
        assert!(normalized.ends_with("relative/path"));
    }

    #[test]
    fn test_normalize_tilde() {
        let home = home::home_dir().unwrap();
        let normalized = normalize(Path::new("~/test")).unwrap();
        assert_eq!(normalized, home.join("test"));
        assert!(normalized.is_absolute());
    }

    #[test]
    fn test_normalize_current_dir() {
        let cwd = env::current_dir().unwrap();
        let normalized = normalize(Path::new(".")).unwrap();
        assert_eq!(normalized, cwd);
    }

    // Property-based tests
    #[cfg(unix)]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        // Strategy to generate valid path strings (Unix-like paths)
        fn path_strategy() -> impl Strategy<Value = String> {
            prop::collection::vec("[a-zA-Z0-9_-]{1,10}", 1..=5)
                .prop_map(|parts| format!("/{}", parts.join("/")))
        }

        // Strategy for paths with . and .. components
        fn path_with_dots_strategy() -> impl Strategy<Value = String> {
            prop::collection::vec(
                prop_oneof![
                    Just(".".to_string()),
                    Just("..".to_string()),
                    "[a-zA-Z0-9_-]{1,10}".prop_map(|s| s),
                ],
                1..=8,
            )
            .prop_map(|parts| format!("/{}", parts.join("/")))
        }

        proptest! {
            /// Normalization always produces absolute paths
            #[test]
            fn normalize_always_absolute(s in path_strategy()) {
                let path = Path::new(&s);
                if let Ok(normalized) = normalize(path) {
                    prop_assert!(normalized.is_absolute());
                }
            }

            /// Normalization is idempotent (normalizing twice gives same result)
            #[test]
            fn normalize_idempotent(s in path_strategy()) {
                let path = Path::new(&s);
                if let Ok(norm1) = normalize(path) {
                    if let Ok(norm2) = normalize(&norm1) {
                        prop_assert_eq!(norm1, norm2);
                    }
                }
            }

            /// Normalized paths don't contain . components
            #[test]
            fn normalize_no_current_dir(s in path_with_dots_strategy()) {
                let path = Path::new(&s);
                if let Ok(normalized) = normalize(path) {
                    // Check no component is "."
                    for component in normalized.components() {
                        prop_assert_ne!(component, std::path::Component::CurDir);
                    }
                }
            }

            /// Normalized paths don't contain .. components
            #[test]
            fn normalize_no_parent_dir(s in path_with_dots_strategy()) {
                let path = Path::new(&s);
                if let Ok(normalized) = normalize(path) {
                    // Check no component is ".."
                    for component in normalized.components() {
                        prop_assert_ne!(component, std::path::Component::ParentDir);
                    }
                }
            }

            /// resolve_components preserves absolute paths
            #[test]
            fn resolve_components_preserves_absolute(s in path_strategy()) {
                let path = Path::new(&s);
                if let Ok(resolved) = resolve_components(path) {
                    prop_assert!(resolved.is_absolute());
                }
            }
        }
    }
}
