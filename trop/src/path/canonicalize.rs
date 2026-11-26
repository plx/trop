//! Path canonicalization functions.
//!
//! This module provides functionality to canonicalize paths by following
//! symlinks to their real paths, with support for:
//! - Full canonicalization of existing paths
//! - Partial canonicalization for non-existent paths
//! - Symlink loop detection

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::path::component_kind;
use crate::verified::path::{deepest_existing_prefix, CompKind};
use crate::verified::symlink::{follow_links, Followed, LinkStep};

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
/// symlink loops and limiting the depth of symlink following: it follows at
/// most `max_depth` symbolic links starting at `path` itself (a relative link
/// target is joined to the link's parent directory), then canonicalizes the
/// final, non-link path with the operating system. A chain of exactly
/// `max_depth` links resolves, and `max_depth == 0` accepts a path that is
/// not a link. Symbolic links in parent directories are resolved by the
/// operating system, which applies its own limit.
///
/// If the chain ends at a path that does not exist, the deepest existing
/// ancestor of that path is canonicalized by the operating system (so `..`
/// and links in it are resolved physically, as the filesystem would) and the
/// missing components are appended (see [`canonicalize_existing`]). If no
/// ancestor can be canonicalized, the path is returned as reached.
///
/// # Errors
///
/// Returns an error if:
/// - A symlink loop is detected (`SymlinkLoop`): a link chain at `path`
///   reaches a path equal (component by component) to one it already
///   visited, within the first `max_depth` links. The error names the
///   repeated path. A loop that never repeats a path exactly (for example
///   one spelled differently through `..`) exceeds the depth instead.
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
    // The verified follower decides the depth bound and loop detection.
    match follow_symlinks(path, max_depth, &Error::Io) {
        Followed::Resolved(current) => fs::canonicalize(&current).map_err(|e| match e.kind() {
            ErrorKind::NotFound => Error::PathNotFound { path: current },
            ErrorKind::PermissionDenied => Error::PermissionDenied { path: current },
            _ => Error::Io(e),
        }),
        Followed::Missing(current) => Ok(resolve_missing(current)),
        Followed::Loop(state) => Err(Error::SymlinkLoop { path: state }),
        Followed::TooDeep => Err(too_many_symlinks(path, max_depth)),
        Followed::Failed(_, error) => Err(error),
    }
}

/// Where a link chain ends when its last path `current` does not exist: its
/// deepest existing ancestor, canonicalized, with the missing components
/// appended (they are all named components). `current` was joined
/// lexically, so a `..` in it must be resolved by the filesystem, not
/// dropped with the component before it: that component may be a link. If
/// no ancestor can be canonicalized, `current` is returned unchanged.
fn resolve_missing(current: PathBuf) -> PathBuf {
    match canonicalize_existing(&current) {
        Ok((canonical, Some(missing))) => canonical.join(missing),
        Ok((canonical, None)) => canonical,
        Err(_) => current,
    }
}

/// Canonicalizes an existing path like [`canonicalize`], but follows at most
/// `max_depth` symbolic links at the path itself, and reports a symlink loop
/// there as `SymlinkLoop` (see [`canonicalize_safe`]). `PathNotFound`,
/// `PermissionDenied` and `InvalidPath` name `path`, as [`canonicalize`]'s
/// errors do (a chain ending at a path that does not exist is
/// `PathNotFound`); `SymlinkLoop` names the repeated path, which may be a
/// link the chain reached rather than `path` itself.
pub(crate) fn canonicalize_bounded(path: &Path, max_depth: usize) -> Result<PathBuf> {
    let io_error = |error: std::io::Error| match error.kind() {
        ErrorKind::NotFound => Error::PathNotFound {
            path: path.to_path_buf(),
        },
        ErrorKind::PermissionDenied => Error::PermissionDenied {
            path: path.to_path_buf(),
        },
        _ => Error::Io(error),
    };
    match follow_symlinks(path, max_depth, &io_error) {
        Followed::Resolved(current) => fs::canonicalize(current).map_err(io_error),
        Followed::Missing(_) => Err(Error::PathNotFound {
            path: path.to_path_buf(),
        }),
        Followed::Loop(state) => Err(Error::SymlinkLoop { path: state }),
        Followed::TooDeep => Err(too_many_symlinks(path, max_depth)),
        Followed::Failed(_, error) => Err(error),
    }
}

fn too_many_symlinks(path: &Path, max_depth: usize) -> Error {
    Error::InvalidPath {
        path: path.to_path_buf(),
        reason: format!("Too many symlinks (max {max_depth})"),
    }
}

/// Follows symbolic links from `path`, at most `max_depth` of them (verified
/// kernel `follow_links`). `TooDeep` only when exactly `max_depth` links were
/// followed and the path reached is a link again. `Loop` when a path reached
/// within the first `max_depth` links equals (as a `PathBuf`, component by
/// component) one visited before; a repeat that would only be reached by the
/// link that is not followed is reported as `TooDeep`, and a loop spelled
/// differently (for example through `..`) is never an exact repeat, so it
/// also ends as `TooDeep`. I/O errors other than "not found" are mapped by
/// `io_error`.
fn follow_symlinks(
    path: &Path,
    max_depth: usize,
    io_error: &dyn Fn(std::io::Error) -> Error,
) -> Followed<PathBuf, Error> {
    let (followed, _) = follow_links(
        path.to_path_buf(),
        max_depth,
        &|current: &PathBuf| link_step(current, io_error),
        &|a: &PathBuf, b: &PathBuf| a == b,
    );
    followed
}

/// Inspects one path for the symlink follower: a symbolic link (to its
/// target, joined lexically to the link's parent directory when relative, so
/// the operating system still resolves `..` physically), not a link,
/// missing, or an error.
fn link_step(
    current: &Path,
    io_error: &dyn Fn(std::io::Error) -> Error,
) -> LinkStep<PathBuf, Error> {
    match fs::symlink_metadata(current) {
        Ok(metadata) if metadata.file_type().is_symlink() => match fs::read_link(current) {
            Ok(target) if target.is_absolute() => LinkStep::Link(target),
            Ok(target) => current.parent().map_or_else(
                || {
                    LinkStep::Failed(Error::InvalidPath {
                        path: current.to_path_buf(),
                        reason: "Symlink has no parent directory".to_string(),
                    })
                },
                |parent| LinkStep::Link(parent.join(target)),
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => LinkStep::Missing,
            Err(error) => LinkStep::Failed(io_error(error)),
        },
        Ok(_) => LinkStep::NotLink,
        Err(error) if error.kind() == ErrorKind::NotFound => LinkStep::Missing,
        Err(error) => LinkStep::Failed(io_error(error)),
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

    let components: Vec<Component<'_>> = path.components().collect();
    let kinds: Vec<CompKind> = components.iter().map(|c| component_kind(*c)).collect();
    // The prefix of the first `k` components. The whole path is used as
    // written (a trailing separator matters to the filesystem).
    let prefix = |k: usize| -> PathBuf {
        if k == components.len() {
            path.to_path_buf()
        } else {
            components[..k].iter().collect()
        }
    };

    // The verified search walks up from the whole path, one named component
    // at a time, and stops at the deepest prefix observed to exist. Every
    // component after that prefix is a named component.
    match deepest_existing_prefix(&kinds, &|k| prefix(k).exists()) {
        Ok(k) => {
            let canonical = canonicalize(&prefix(k))?;
            let remainder = (k < components.len()).then(|| components[k..].iter().collect());
            Ok((canonical, remainder))
        }
        Err(_) => Err(Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "Cannot find any existing portion of path".to_string(),
        }),
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

    /// A chain `link{n-1} -> ... -> link0 -> target` of `n` links to a
    /// directory, returning the last link and the canonical target.
    #[cfg(unix)]
    fn link_chain(dir: &Path, n: usize) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::symlink;

        let target = dir.join("target");
        fs::create_dir(&target).unwrap();
        let mut current = target.clone();
        for i in 0..n {
            let link = dir.join(format!("link{i}"));
            symlink(&current, &link).unwrap();
            current = link;
        }
        (current, fs::canonicalize(&target).unwrap())
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_follows_exactly_max_depth_links() {
        let dir = tempdir().unwrap();
        let (start, target) = link_chain(dir.path(), 5);

        // Before the verified follower, the depth was checked before reading
        // each link, so exactly `max_depth` links failed.
        assert_eq!(canonicalize_safe(&start, 5).unwrap(), target);
        assert!(matches!(
            canonicalize_safe(&start, 4),
            Err(Error::InvalidPath { ref reason, .. }) if reason == "Too many symlinks (max 4)"
        ));
    }

    #[test]
    fn canonicalize_safe_with_zero_depth_accepts_a_directory() {
        // Before the verified follower, `max_depth == 0` rejected every path.
        let dir = tempdir().unwrap();
        assert_eq!(
            canonicalize_safe(dir.path(), 0).unwrap(),
            fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_with_zero_depth_rejects_a_link() {
        let dir = tempdir().unwrap();
        let (start, _) = link_chain(dir.path(), 1);
        assert!(matches!(
            canonicalize_safe(&start, 0),
            Err(Error::InvalidPath { .. })
        ));
        assert!(canonicalize_safe(&start, 1).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_reports_a_self_loop_at_any_depth() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let link = dir.path().join("self");
        symlink(&link, &link).unwrap();
        // One link followed reaches the same path again.
        assert!(matches!(
            canonicalize_safe(&link, 1),
            Err(Error::SymlinkLoop { ref path }) if *path == link
        ));
        assert!(matches!(
            canonicalize_safe(&link, 0),
            Err(Error::InvalidPath { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_joins_relative_targets_to_the_link_directory() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        fs::create_dir(dir.path().join("links")).unwrap();
        symlink("../real", dir.path().join("links").join("rel")).unwrap();
        assert_eq!(
            canonicalize_safe(&dir.path().join("links").join("rel"), 1).unwrap(),
            fs::canonicalize(dir.path().join("real")).unwrap()
        );
        // A dangling relative link ends at its (missing) target, in the
        // link's (canonical) directory.
        symlink("missing", dir.path().join("links").join("dangling")).unwrap();
        assert_eq!(
            canonicalize_safe(&dir.path().join("links").join("dangling"), 1).unwrap(),
            fs::canonicalize(dir.path().join("links"))
                .unwrap()
                .join("missing")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_resolves_a_dangling_target_physically() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        // `symdir -> x/realdir`, and inside it `rm -> ../missing`. The joined
        // target `symdir/../missing` names `x/missing` physically, although
        // it reads lexically as `missing` next to `symdir`.
        fs::create_dir_all(base.join("x").join("realdir")).unwrap();
        symlink(base.join("x").join("realdir"), base.join("symdir")).unwrap();
        symlink("../missing", base.join("x").join("realdir").join("rm")).unwrap();
        // Before, the joined path was returned unchanged.
        assert_eq!(
            canonicalize_safe(&base.join("symdir").join("rm"), 2).unwrap(),
            base.join("x").join("missing")
        );
        // `up -> ../nope`: the parent directory is resolved, not kept as `..`.
        symlink("../nope", base.join("up")).unwrap();
        assert_eq!(
            canonicalize_safe(&base.join("up"), 1).unwrap(),
            base.parent().unwrap().join("nope")
        );
        // A path that is missing from the start is unchanged when it is
        // already canonical.
        assert_eq!(
            canonicalize_safe(&base.join("absent").join("deeper"), 0).unwrap(),
            base.join("absent").join("deeper")
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_safe_reports_a_loop_by_exact_repeat_within_the_depth() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let (a, b, c) = (base.join("a"), base.join("b"), base.join("c"));
        symlink(&b, &a).unwrap();
        symlink(&a, &b).unwrap();
        symlink(&a, &c).unwrap();
        // `a -> b -> a`: the repeat of `a` is reached by the second link.
        assert!(matches!(
            canonicalize_safe(&a, 2),
            Err(Error::SymlinkLoop { ref path }) if *path == a
        ));
        // With one link allowed, the repeat would need the link that is not
        // followed: too deep.
        assert!(matches!(
            canonicalize_safe(&a, 1),
            Err(Error::InvalidPath { .. })
        ));
        // `c -> a -> b -> a`: the error names the repeated path, not `c`.
        assert!(matches!(
            canonicalize_bounded(&c, 40),
            Err(Error::SymlinkLoop { ref path }) if *path == a
        ));
        // A loop through `..` never repeats a path exactly.
        let name = base.file_name().unwrap();
        symlink(
            Path::new("..").join(name).join("dotloop"),
            base.join("dotloop"),
        )
        .unwrap();
        assert!(matches!(
            canonicalize_safe(&base.join("dotloop"), 5),
            Err(Error::InvalidPath { ref reason, .. }) if reason == "Too many symlinks (max 5)"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_bounded_maps_errors_like_canonicalize() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(matches!(
            canonicalize_bounded(&missing, 40),
            Err(Error::PathNotFound { ref path }) if *path == missing
        ));
        let (start, target) = link_chain(dir.path(), 3);
        assert_eq!(canonicalize_bounded(&start, 3).unwrap(), target);
        assert!(canonicalize_bounded(&start, 2).is_err());
        assert_eq!(
            canonicalize_bounded(dir.path(), 0).unwrap(),
            canonicalize(dir.path()).unwrap()
        );
    }
}
