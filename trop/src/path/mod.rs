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
//!   canonicalized, preserving symlinks.
//!
//! - **Implicit** (`PathProvenance::Implicit`): Paths inferred from context
//!   like the current working directory. These are both normalized AND
//!   canonicalized for consistency.
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
//! // will be canonicalized if the path exists
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

// Re-export key types
pub use relationship::PathRelationship;
pub use resolver::PathResolver;
pub use types::{NormalizedPath, PathProvenance, ResolvedPath};
