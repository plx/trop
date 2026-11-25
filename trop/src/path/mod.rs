//! Path handling with provenance-aware canonicalization.
//!
//! This module provides comprehensive path handling for the trop system,
//! including normalization, canonicalization, and relationship checking.
//!
//! # Key Concepts
//!
//! ## Normalization
//!
//! Normalization converts paths to a canonical form by:
//! - Expanding tilde (~) to the home directory
//! - Converting relative paths to absolute paths
//! - Resolving `.` and `..` components
//! - Normalizing path separators for the platform
//!
//! ## Canonicalization
//!
//! Canonicalization follows symlinks to get the "real" path on the filesystem.
//! This is only applied to implicit paths to preserve user intent for explicit
//! paths.
//!
//! ## Provenance
//!
//! Path provenance tracks how a path was provided to the system:
//!
//! - **Explicit** (`PathProvenance::Explicit`): Paths explicitly provided via
//!   CLI arguments or environment variables. These are normalized but NOT
//!   canonicalized, preserving symlinks and permitting nonexistent targets.
//!
//! - **Implicit** (`PathProvenance::Implicit`): Paths inferred from context
//!   like the current working directory. These are both normalized AND
//!   canonicalized for consistency; resolution fails if canonicalization is
//!   impossible.
//!
//! # Examples
//!
//! ```no_run
//! use trop::path::{PathResolver, PathProvenance};
//! use std::path::Path;
//!
//! let resolver = PathResolver::new();
//!
//! // Explicit paths preserve symlinks
//! let explicit = resolver.resolve_explicit(Path::new("~/project")).unwrap();
//! assert!(!explicit.was_canonicalized());
//!
//! // Implicit paths follow symlinks
//! let implicit = resolver.resolve_implicit(Path::new(".")).unwrap();
//! assert!(implicit.was_canonicalized());
//! ```
//!
//! # Path Relationships
//!
//! The module also provides functionality to determine relationships between
//! paths:
//!
//! ```
//! use trop::path::PathRelationship;
//! use std::path::Path;
//!
//! let parent = Path::new("/home/user");
//! let child = Path::new("/home/user/project");
//!
//! let rel = PathRelationship::between(parent, child);
//! assert_eq!(rel, PathRelationship::Ancestor);
//! assert!(rel.is_allowed_without_force());
//! ```

pub mod canonicalize;
pub mod normalize;
pub mod relationship;
pub mod resolver;
mod types;

#[cfg(all(test, feature = "property-tests"))]
mod proptests;

#[cfg(test)]
mod verified_glue_tests;

use std::path::{Component, Path};

use crate::verified::path::CompKind;

// Re-export key types
pub use relationship::PathRelationship;
pub use resolver::PathResolver;
pub use types::{NormalizedPath, PathProvenance, ResolvedPath};

/// The kind of a path component, as the verified path kernels see it.
pub(crate) fn component_kind(component: Component<'_>) -> CompKind {
    match component {
        Component::Prefix(_) => CompKind::Prefix,
        Component::RootDir => CompKind::Root,
        Component::CurDir => CompKind::Cur,
        Component::ParentDir => CompKind::Parent,
        Component::Normal(_) => CompKind::Normal,
    }
}

/// The platform facts the storage kernel (`verified::storage`) needs to judge
/// `path` as a stored reservation key.
///
/// On Unix the byte model decides everything. Elsewhere `std` decides
/// whether the path is absolute and whether it is a fixed point of component
/// resolution ([`is_canonical_absolute`]); the kernel accepts `/`-rooted
/// spellings written on Unix by their portable form.
#[cfg(unix)]
pub(crate) const fn storage_platform(_path: &Path) -> crate::verified::storage::PathPlatform {
    crate::verified::storage::PathPlatform::Unix
}

/// The platform facts the storage kernel (`verified::storage`) needs to judge
/// `path` as a stored reservation key.
///
/// On Unix the byte model decides everything. Elsewhere `std` decides
/// whether the path is absolute and whether it is a fixed point of component
/// resolution ([`is_canonical_absolute`]); the kernel accepts `/`-rooted
/// spellings written on Unix by their portable form.
#[cfg(not(unix))]
pub(crate) fn storage_platform(path: &Path) -> crate::verified::storage::PathPlatform {
    crate::verified::storage::PathPlatform::Native {
        absolute: path.is_absolute(),
        normal: is_canonical_absolute(path),
    }
}

/// Whether `path` is spelled in canonical absolute form: what
/// [`normalize::normalize`] produces.
///
/// On Unix this is the byte-level test `verified::path::is_canonical_abs_path`:
/// `/`, or `/` followed by non-empty segments other than `.` and `..` (so no
/// `//` and no trailing `/`); it accepts exactly the fixed points of
/// normalization. Elsewhere it checks that the path is absolute and that
/// resolving `.` and `..` and re-spelling the components leaves it
/// byte-for-byte unchanged.
#[cfg(unix)]
pub(crate) fn is_canonical_absolute(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    crate::verified::path::is_canonical_abs_path(path.as_os_str().as_bytes(), false)
}

/// Whether `path` is spelled in canonical absolute form: what
/// [`normalize::normalize`] produces.
///
/// Outside Unix this checks that the path is absolute and that resolving `.`
/// and `..` with the verified component stack, and re-spelling the
/// components as `normalize` does, leaves it byte-for-byte unchanged (so `/`
/// for `\`, `.`, repeated separators, and a trailing separator are all
/// rejected, as `normalize` never produces them).
#[cfg(not(unix))]
pub(crate) fn is_canonical_absolute(path: &Path) -> bool {
    path.is_absolute()
        && normalize::resolve_components(path)
            .is_ok_and(|resolved| resolved.as_os_str() == path.as_os_str())
}
