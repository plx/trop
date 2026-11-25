//! Path normalization functions.
//!
//! This module provides functionality to normalize paths by:
//! - Expanding tilde (~) to the home directory
//! - Converting relative paths to absolute paths
//! - Resolving `.` and `..` components
//! - Normalizing path separators for the platform
//!
//! The decisions are made by the verified kernel `verified::path`: the tilde
//! classifier, the `.`/`..` stack over component kinds, and (on Unix) the
//! byte-level normalizer whose results are canonical, idempotent, and
//! independent of the working directory for absolute input.

use std::env;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::path::component_kind;
use crate::verified::path::{classify_tilde, resolve_component_kinds, CompKind, TildeForm};

/// Expand tilde (~) to the home directory.
///
/// This function handles `~` and `~/path` but does not support `~user` syntax.
/// Every separator after `~` is skipped, so `~//etc` names `etc` under the
/// home directory (never `/etc`). On Unix a backslash is an ordinary filename
/// byte, so `~\x` is not a tilde form and is returned unchanged; on Windows
/// `~\x` is home-relative, and a remainder that starts with a drive prefix
/// (`~/C:x`) is rejected, because joining it would replace the home
/// directory.
///
/// # Errors
///
/// Returns an error if:
/// - The path contains invalid UTF-8
/// - The home directory cannot be determined
/// - The path uses `~user` syntax (not supported)
/// - On Windows, the part after `~/` starts with a drive prefix
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

    let form = classify_tilde(path_str.as_bytes(), cfg!(windows));
    if matches!(form, TildeForm::NotTilde) {
        return Ok(path.to_path_buf());
    }

    // Get home directory using the home crate
    let home = home::home_dir().ok_or_else(|| Error::InvalidPath {
        path: path.to_path_buf(),
        reason: "Cannot determine home directory".to_string(),
    })?;

    match form {
        TildeForm::HomeOnly => Ok(home),
        // `rest_start` is past every separator that follows `~`. Those bytes
        // are ASCII, so it is a char boundary, and the rest never starts with
        // a separator.
        TildeForm::HomeRelative { rest_start } => {
            let rest = &path_str[rest_start..];
            // Without a leading separator the rest can only replace the home
            // directory on `join` by starting with a prefix (`C:x` on
            // Windows); std never parses a prefix on Unix.
            if matches!(
                Path::new(rest).components().next(),
                Some(Component::Prefix(_))
            ) {
                return Err(Error::InvalidPath {
                    path: path.to_path_buf(),
                    reason: "The path after '~' names a drive; use ~/path relative to the home \
                             directory"
                        .to_string(),
                });
            }
            Ok(join_home(&home, rest))
        }
        TildeForm::NotTilde | TildeForm::UserSyntax => Err(Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "~user syntax is not supported; use ~ or ~/path".to_string(),
        }),
    }
}

/// Joins the separator-free `rest` of a `~/rest` path onto `home`, exactly as
/// `PathBuf::push` joins a relative path (verified kernel `tilde_join`).
#[cfg(unix)]
fn join_home(home: &Path, rest: &str) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let joined = crate::verified::path::tilde_join(home.as_os_str().as_bytes(), rest.as_bytes());
    PathBuf::from(OsString::from_vec(joined))
}

/// Joins the separator-free `rest` of a `~/rest` path onto `home`.
#[cfg(not(unix))]
fn join_home(home: &Path, rest: &str) -> PathBuf {
    home.join(rest)
}

/// Resolve `.` and `..` components in a path.
///
/// This function processes path components to remove current directory (`.`)
/// references and resolve parent directory (`..`) references. A `..` cancels
/// the preceding named component; the root (or prefix) is always kept.
///
/// # Errors
///
/// Returns an error if a `..` has no named component to cancel: it would
/// escape the root (`/..`), or climb above the start of a relative path
/// (`a/../..`).
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
    let components: Vec<Component<'_>> = path.components().collect();
    let kinds: Vec<CompKind> = components.iter().map(|c| component_kind(*c)).collect();
    // The kernel returns the surviving component indices, in order; it keeps
    // every root/prefix and only roots, prefixes, and named components.
    let kept = resolve_component_kinds(&kinds).map_err(|_| too_many_parents(path))?;
    Ok(kept.into_iter().map(|index| components[index]).collect())
}

fn too_many_parents(path: &Path) -> Error {
    Error::InvalidPath {
        path: path.to_path_buf(),
        reason: "Path contains too many '..' components (escapes root)".to_string(),
    }
}

fn current_dir(path: &Path) -> Result<PathBuf> {
    env::current_dir().map_err(|e| Error::InvalidPath {
        path: path.to_path_buf(),
        reason: format!("Cannot get current directory: {e}"),
    })
}

/// Normalize a path to absolute form.
///
/// This is the main normalization function that:
/// 1. Expands tilde (~) if present
/// 2. Converts relative paths to absolute (using current directory)
/// 3. Resolves `.` and `..` components
///
/// On Unix the result is the canonical spelling `/` or `/a/b/c` (no `.`,
/// `..`, empty components, or trailing separator), normalizing it again
/// returns it unchanged, and the current directory is consulted only for
/// relative paths.
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
    normalize_expanded(path, expanded)
}

/// Unix: the verified byte-level normalizer `normalize_unix`.
#[cfg(unix)]
fn normalize_expanded(path: &Path, expanded: PathBuf) -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use crate::verified::path::{is_absolute_unix, normalize_unix, NormError};

    let bytes = expanded.as_os_str().as_bytes();
    // The kernel ignores the working directory for absolute paths, so it is
    // only read for relative ones.
    let cwd = if is_absolute_unix(bytes) {
        None
    } else {
        Some(current_dir(path)?)
    };
    let cwd_bytes = cwd
        .as_deref()
        .map_or(&[][..], |dir| dir.as_os_str().as_bytes());
    match normalize_unix(bytes, cwd_bytes) {
        Ok(normalized) => Ok(PathBuf::from(OsString::from_vec(normalized))),
        Err(NormError::EscapesRoot) => Err(too_many_parents(&match cwd {
            Some(dir) => dir.join(&expanded),
            None => expanded,
        })),
        Err(NormError::BaseNotAbsolute) => Err(Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "Current directory is not absolute".to_string(),
        }),
    }
}

/// Other platforms: make the path absolute, then resolve `.` and `..` with the
/// verified component-kind stack.
#[cfg(not(unix))]
fn normalize_expanded(path: &Path, expanded: PathBuf) -> Result<PathBuf> {
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        current_dir(path)?.join(expanded)
    };
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

    // On Windows a drive prefix right after the separators that follow `~`
    // would make `join` replace the home directory, so it is rejected.
    #[test]
    #[cfg(windows)]
    fn test_expand_tilde_rejects_a_drive_prefix_on_windows() {
        for input in [r"~/C:x", r"~\C:\x", r"~//D:", r"~\/c:"] {
            assert!(
                matches!(
                    expand_tilde(Path::new(input)),
                    Err(Error::InvalidPath { .. })
                ),
                "{input}"
            );
        }
    }

    // On Unix `C:` is an ordinary name, so it stays under the home directory.
    #[test]
    #[cfg(unix)]
    fn test_expand_tilde_drive_like_name_on_unix() {
        let home = home::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~/C:x")).unwrap(), home.join("C:x"));
        assert_eq!(expand_tilde(Path::new("~//C:")).unwrap(), home.join("C:"));
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
    #[cfg(all(unix, feature = "property-tests"))]
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
