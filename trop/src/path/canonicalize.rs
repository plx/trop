//! Path canonicalization functions.
//!
//! This module provides functionality to canonicalize paths by following
//! symlinks to their real paths, with support for:
//! - Full canonicalization of existing paths
//! - Partial canonicalization for non-existent paths
//! - Symlink loop detection

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Attempt to canonicalize a path by following symlinks.
///
/// This function uses the standard library's `canonicalize` to resolve all
/// symlinks in the path. The path must exist for canonicalization to succeed.
///
/// # Errors
///
/// Returns an error if:
/// - The path does not exist (`PathNotFound`)
/// - Permission is denied (`PermissionDenied`)
/// - An I/O error occurs
///
/// # Examples
///
/// ```no_run
/// use trop::path::canonicalize::canonicalize;
/// use std::path::Path;
///
/// // Canonicalize an existing path
/// let canonical = canonicalize(Path::new("/tmp")).unwrap();
/// assert!(canonical.is_absolute());
/// ```
pub fn canonicalize(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => Error::PathNotFound {
            path: path.to_path_buf(),
        },
        ErrorKind::PermissionDenied => Error::PermissionDenied {
            path: path.to_path_buf(),
        },
        _ => Error::Io(e),
    })
}

/// Canonicalize a path with symlink loop detection.
///
/// This function provides more control over canonicalization by detecting
/// symlink loops and limiting the depth of symlink following.
///
/// # Errors
///
/// Returns an error if:
/// - A symlink loop is detected (`SymlinkLoop`)
/// - The maximum symlink depth is exceeded
/// - An I/O error occurs
///
/// # Examples
///
/// ```no_run
/// use trop::path::canonicalize::canonicalize_safe;
/// use std::path::Path;
///
/// // Canonicalize with loop detection
/// let canonical = canonicalize_safe(Path::new("/tmp"), 40).unwrap();
/// ```
pub fn canonicalize_safe(path: &Path, max_depth: usize) -> Result<PathBuf> {
    let mut visited = HashSet::new();
    let mut current = path.to_path_buf();
    let mut depth = 0;

    loop {
        // Check for loops
        if !visited.insert(current.clone()) {
            return Err(Error::SymlinkLoop {
                path: current.clone(),
            });
        }

        // Check depth
        if depth >= max_depth {
            return Err(Error::InvalidPath {
                path: path.to_path_buf(),
                reason: format!("Too many symlinks (max {max_depth})"),
            });
        }

        // Try to read the symlink
        match fs::read_link(&current) {
            Ok(target) => {
                // It's a symlink - resolve it
                current = if target.is_absolute() {
                    target
                } else {
                    // Relative symlink - resolve relative to parent
                    current
                        .parent()
                        .ok_or_else(|| Error::InvalidPath {
                            path: current.clone(),
                            reason: "Symlink has no parent directory".to_string(),
                        })?
                        .join(target)
                };
                depth += 1;
            }
            Err(e) if e.kind() == ErrorKind::InvalidInput => {
                // Not a symlink - use fs::canonicalize to handle any parent symlinks
                return fs::canonicalize(&current).map_err(|e| match e.kind() {
                    ErrorKind::NotFound => Error::PathNotFound {
                        path: current.clone(),
                    },
                    ErrorKind::PermissionDenied => Error::PermissionDenied {
                        path: current.clone(),
                    },
                    _ => Error::Io(e),
                });
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // Path doesn't exist - return as-is
                return Ok(current);
            }
            Err(e) => {
                // Other error
                return Err(Error::Io(e));
            }
        }
    }
}

/// Canonicalize the existing portion of a path.
///
/// For non-existent paths, this function finds the longest existing ancestor
/// and canonicalizes it, then appends the non-existent components.
///
/// # Returns
///
/// Returns a tuple of:
/// - The canonicalized existing portion
/// - The remaining non-existent components (if any)
///
/// # Errors
///
/// Returns an error if:
/// - No existing ancestor can be found
/// - Canonicalization of the existing portion fails
/// - An I/O error occurs
///
/// # Examples
///
/// ```no_run
/// use trop::path::canonicalize::canonicalize_existing;
/// use std::path::{Path, PathBuf};
///
/// // For a path where /tmp exists but /tmp/nonexistent/file does not:
/// let (canonical, remainder) =
///     canonicalize_existing(Path::new("/tmp/nonexistent/file")).unwrap();
/// // canonical will be the canonicalized /tmp
/// // remainder will be Some(PathBuf::from("nonexistent/file"))
/// ```
pub fn canonicalize_existing(path: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    // Try full canonicalization first
    if let Ok(canonical) = canonicalize(path) {
        return Ok((canonical, None));
    }

    // Walk up the path to find the existing portion
    let mut current = path.to_path_buf();
    let mut non_existent = Vec::new();

    loop {
        if current.exists() {
            // Found an existing ancestor - canonicalize it
            let canonical = canonicalize(&current)?;

            // Rebuild the non-existent portion
            let remainder = if non_existent.is_empty() {
                None
            } else {
                non_existent.reverse();
                Some(non_existent.into_iter().collect())
            };

            return Ok((canonical, remainder));
        }

        // Save this component and move to parent
        match current.file_name() {
            Some(name) => {
                non_existent.push(name.to_os_string());
                current.pop();
            }
            None => {
                // Reached root without finding existing path
                return Err(Error::InvalidPath {
                    path: path.to_path_buf(),
                    reason: "Cannot find any existing portion of path".to_string(),
                });
            }
        }
    }
}

/// Attempt to canonicalize a path, returning the original if it doesn't exist.
///
/// This is a convenience function that attempts canonicalization but falls back
/// to returning the original path (normalized) if the path doesn't exist.
///
/// # Errors
///
/// Returns an error only for I/O errors other than "not found".
///
/// # Examples
///
/// ```no_run
/// use trop::path::canonicalize::try_canonicalize;
/// use std::path::Path;
///
/// // For existing paths, returns canonicalized version
/// let result = try_canonicalize(Path::new("/tmp")).unwrap();
///
/// // For non-existent paths, returns the original
/// let result = try_canonicalize(Path::new("/nonexistent")).unwrap();
/// ```
pub fn try_canonicalize(path: &Path) -> Result<PathBuf> {
    match canonicalize(path) {
        Ok(canonical) => Ok(canonical),
        Err(Error::PathNotFound { .. }) => Ok(path.to_path_buf()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Maximum symlink depth used in tests.
    const MAX_SYMLINK_DEPTH: usize = 40;

    #[test]
    fn test_canonicalize_nonexistent() {
        let result = canonicalize(Path::new("/nonexistent/path/xyz"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::PathNotFound { .. }));
    }

    #[test]
    fn test_canonicalize_safe_nonexistent() {
        let result = canonicalize_safe(Path::new("/nonexistent/path"), MAX_SYMLINK_DEPTH);
        // Should return the path as-is for non-existent paths
        assert!(result.is_ok());
    }

    #[test]
    fn test_canonicalize_existing_full_path_exists() {
        let dir = tempdir().unwrap();
        let path = dir.path();

        let (canonical, remainder) = canonicalize_existing(path).unwrap();
        assert_eq!(canonical, fs::canonicalize(path).unwrap());
        assert!(remainder.is_none());
    }

    #[test]
    fn test_canonicalize_existing_partial() {
        let dir = tempdir().unwrap();
        let existing = dir.path();
        let full = existing.join("nonexistent").join("path");

        let (canonical, remainder) = canonicalize_existing(&full).unwrap();
        assert_eq!(canonical, fs::canonicalize(existing).unwrap());
        assert_eq!(remainder, Some(PathBuf::from("nonexistent").join("path")));
    }

    #[test]
    fn test_try_canonicalize_existing() {
        let dir = tempdir().unwrap();
        let result = try_canonicalize(dir.path()).unwrap();
        assert_eq!(result, fs::canonicalize(dir.path()).unwrap());
    }

    #[test]
    fn test_try_canonicalize_nonexistent() {
        let path = Path::new("/nonexistent/path");
        let result = try_canonicalize(path).unwrap();
        assert_eq!(result, path);
    }

    #[cfg(unix)]
    #[test]
    fn test_canonicalize_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::write(&target, "test").unwrap();
        symlink(&target, &link).unwrap();

        let canonical = canonicalize(&link).unwrap();
        assert_eq!(canonical, fs::canonicalize(&target).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_canonicalize_safe_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");

        fs::create_dir(&target).unwrap();
        symlink(&target, &link).unwrap();

        let canonical = canonicalize_safe(&link, MAX_SYMLINK_DEPTH).unwrap();
        assert_eq!(canonical, fs::canonicalize(&target).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_canonicalize_safe_detects_loop() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let link1 = dir.path().join("link1");
        let link2 = dir.path().join("link2");

        symlink(&link2, &link1).unwrap();
        symlink(&link1, &link2).unwrap();

        let result = canonicalize_safe(&link1, MAX_SYMLINK_DEPTH);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::SymlinkLoop { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn test_canonicalize_safe_respects_max_depth() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();

        // Create a chain of symlinks longer than the limit
        let mut current = dir.path().join("target");
        fs::create_dir(&current).unwrap();

        for i in 0..5 {
            let link = dir.path().join(format!("link{i}"));
            symlink(&current, &link).unwrap();
            current = link;
        }

        // Should succeed with sufficient depth
        let result = canonicalize_safe(&current, 10);
        assert!(result.is_ok());

        // Should fail with insufficient depth
        let result = canonicalize_safe(&current, 2);
        assert!(result.is_err());
    }
}
