# Phase 3: Path Handling System - Detailed Implementation Plan

## Overview

This document provides a comprehensive, actionable implementation plan for Phase 3 of the `trop` port reservation tool. This phase implements the path handling system including normalization, canonicalization with provenance awareness, path relationship checking, and comprehensive testing.

## Context from Previous Phases

Phase 1 established:
- Core types: `Port`, `PortRange`, `Reservation`, `ReservationKey`
- Error hierarchy with `thiserror`
- Logging infrastructure

Phase 2 added:
- SQLite database layer with schema versioning
- Transactional CRUD operations
- Concurrent access safety via WAL mode
- Integration testing framework

## Success Criteria

Upon completion of Phase 3:
- Path normalization handles ~, ., .., and relative paths correctly
- Canonicalization follows symlinks with provenance tracking
- Path relationships (ancestor/descendant/unrelated) are correctly identified
- Non-existent paths handled gracefully with appropriate warnings
- Property-based tests validate path operations
- Integration with existing database layer works seamlessly
- No regressions in existing functionality

## Task Breakdown

### Task 1: Define Path Module Structure

**Objective**: Create the foundational module structure for path handling.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/mod.rs`
- `/Users/prb/github/trop/trop/src/path/types.rs`

**Implementation Details**:

1. Create module structure in `path/mod.rs`:
   ```rust
   mod canonicalize;
   mod normalize;
   mod relationship;
   mod resolver;
   mod types;

   pub use resolver::PathResolver;
   pub use types::{NormalizedPath, PathProvenance, ResolvedPath};
   pub use relationship::PathRelationship;
   ```

2. Define core types in `path/types.rs`:
   ```rust
   use std::path::{Path, PathBuf};

   /// Indicates how a path was provided to the system
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum PathProvenance {
       /// Path explicitly provided via CLI argument or environment variable
       Explicit,
       /// Path inferred from current working directory or similar
       Implicit,
   }

   /// A path that has been normalized to absolute form
   #[derive(Debug, Clone, PartialEq, Eq, Hash)]
   pub struct NormalizedPath {
       path: PathBuf,
       provenance: PathProvenance,
   }

   /// A fully resolved path (normalized and optionally canonicalized)
   #[derive(Debug, Clone, PartialEq, Eq, Hash)]
   pub struct ResolvedPath {
       /// The resolved path
       path: PathBuf,
       /// Original path before resolution
       original: PathBuf,
       /// Whether canonicalization was applied
       canonicalized: bool,
       /// Source of the path
       provenance: PathProvenance,
   }
   ```

3. Add validation and conversion methods:
   ```rust
   impl NormalizedPath {
       pub fn new(path: PathBuf, provenance: PathProvenance) -> Result<Self> {
           if !path.is_absolute() {
               return Err(Error::InvalidPath("Path must be absolute".into()));
           }
           Ok(Self { path, provenance })
       }

       pub fn as_path(&self) -> &Path {
           &self.path
       }

       pub fn into_path_buf(self) -> PathBuf {
           self.path
       }
   }
   ```

**Verification**:
- Module compiles without warnings
- Types have appropriate traits derived
- Documentation is clear

### Task 2: Implement Path Normalization

**Objective**: Create normalization logic that converts paths to absolute form.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/normalize.rs`

**Implementation Details**:

1. Define normalization functions:
   ```rust
   use std::path::{Path, PathBuf};
   use std::env;
   use crate::error::Result;

   /// Expand tilde (~) to home directory
   pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
       let path_str = path.to_str()
           .ok_or_else(|| Error::InvalidPath("Path contains invalid UTF-8".into()))?;

       if !path_str.starts_with('~') {
           return Ok(path.to_path_buf());
       }

       let home = env::var("HOME")
           .or_else(|_| env::var("USERPROFILE"))
           .map_err(|_| Error::InvalidPath("Cannot determine home directory".into()))?;

       if path_str == "~" {
           Ok(PathBuf::from(home))
       } else if path_str.starts_with("~/") {
           Ok(PathBuf::from(home).join(&path_str[2..]))
       } else {
           // ~user syntax not supported
           Err(Error::InvalidPath("~user syntax not supported".into()))
       }
   }
   ```

2. Implement component resolution:
   ```rust
   /// Resolve . and .. components in an absolute path
   pub fn resolve_components(path: &Path) -> Result<PathBuf> {
       use std::path::Component;

       let mut result = PathBuf::new();

       for component in path.components() {
           match component {
               Component::RootDir => result.push("/"),
               Component::Normal(c) => result.push(c),
               Component::CurDir => {}, // Skip "."
               Component::ParentDir => {
                   if !result.pop() && !result.as_os_str().is_empty() {
                       return Err(Error::InvalidPath(
                           "Path contains too many '..' components".into()
                       ));
                   }
               },
               Component::Prefix(p) => result.push(p.as_os_str()), // Windows only
           }
       }

       Ok(result)
   }
   ```

3. Create main normalization function:
   ```rust
   /// Normalize a path to absolute form
   pub fn normalize(path: &Path) -> Result<PathBuf> {
       // First expand tilde if present
       let expanded = expand_tilde(path)?;

       // Make absolute if not already
       let absolute = if expanded.is_absolute() {
           expanded
       } else {
           env::current_dir()
               .map_err(|e| Error::InvalidPath(format!("Cannot get current directory: {}", e)))?
               .join(expanded)
       };

       // Resolve . and .. components
       resolve_components(&absolute)
   }
   ```

4. Add platform-specific handling:
   ```rust
   #[cfg(windows)]
   fn normalize_separators(path: &Path) -> PathBuf {
       // Convert forward slashes to backslashes on Windows
       let path_str = path.to_str().unwrap_or("");
       PathBuf::from(path_str.replace('/', "\\"))
   }

   #[cfg(not(windows))]
   fn normalize_separators(path: &Path) -> PathBuf {
       path.to_path_buf()
   }
   ```

**Verification**:
- Tilde expansion works correctly
- Relative paths become absolute
- . and .. components are resolved
- Edge cases handled (e.g., "/../..")

### Task 3: Implement Path Canonicalization

**Objective**: Create canonicalization logic that follows symlinks.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/canonicalize.rs`

**Implementation Details**:

1. Define canonicalization with error handling:
   ```rust
   use std::path::{Path, PathBuf};
   use std::fs;
   use crate::error::{Error, Result};

   /// Attempt to canonicalize a path, following symlinks
   pub fn canonicalize(path: &Path) -> Result<PathBuf> {
       fs::canonicalize(path)
           .map_err(|e| {
               if e.kind() == std::io::ErrorKind::NotFound {
                   Error::PathNotFound(format!("Path does not exist: {}", path.display()))
               } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                   Error::PermissionDenied(format!("Permission denied: {}", path.display()))
               } else {
                   Error::Io(e)
               }
           })
   }
   ```

2. Implement partial canonicalization for non-existent paths:
   ```rust
   /// Canonicalize the existing portion of a path
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

           match current.file_name() {
               Some(name) => {
                   non_existent.push(name.to_os_string());
                   current.pop();
               }
               None => {
                   return Err(Error::InvalidPath(
                       "Cannot find any existing portion of path".into()
                   ));
               }
           }
       }
   }
   ```

3. Add symlink loop detection:
   ```rust
   use std::collections::HashSet;

   /// Canonicalize with loop detection
   pub fn canonicalize_safe(path: &Path, max_depth: usize) -> Result<PathBuf> {
       let mut visited = HashSet::new();
       let mut current = path.to_path_buf();
       let mut depth = 0;

       while depth < max_depth {
           // Check for loops
           if !visited.insert(current.clone()) {
               return Err(Error::InvalidPath(
                   format!("Symlink loop detected at: {}", current.display())
               ));
           }

           // Check if it's a symlink
           match fs::read_link(&current) {
               Ok(target) => {
                   current = if target.is_absolute() {
                       target
                   } else {
                       current.parent()
                           .ok_or_else(|| Error::InvalidPath("Invalid symlink target".into()))?
                           .join(target)
                   };
                   depth += 1;
               }
               Err(_) => {
                   // Not a symlink or can't read - we're done
                   return Ok(current);
               }
           }
       }

       Err(Error::InvalidPath(
           format!("Too many symlinks (max {})", max_depth)
       ))
   }
   ```

**Verification**:
- Symlinks are followed correctly
- Non-existent paths handled gracefully
- Symlink loops detected and reported
- Permission errors handled appropriately

### Task 4: Create PathResolver Abstraction

**Objective**: Create the main PathResolver type that combines normalization and canonicalization.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/resolver.rs`

**Implementation Details**:

1. Define PathResolver struct:
   ```rust
   use std::path::{Path, PathBuf};
   use crate::path::{normalize, canonicalize};
   use crate::path::types::{PathProvenance, ResolvedPath};
   use crate::error::Result;

   /// Resolves paths according to provenance rules
   #[derive(Debug, Clone)]
   pub struct PathResolver {
       /// Whether to warn on non-existent paths
       warn_on_nonexistent: bool,
       /// Maximum symlink depth
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
   ```

2. Implement resolution methods:
   ```rust
   impl PathResolver {
       pub fn new() -> Self {
           Self::default()
       }

       pub fn with_nonexistent_warning(mut self, warn: bool) -> Self {
           self.warn_on_nonexistent = warn;
           self
       }

       /// Resolve a path according to its provenance
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

           Ok(ResolvedPath {
               path: resolved,
               original: path.to_path_buf(),
               canonicalized,
               provenance,
           })
       }
   }
   ```

3. Add convenience methods:
   ```rust
   impl PathResolver {
       /// Resolve with explicit provenance (no canonicalization)
       pub fn resolve_explicit(&self, path: &Path) -> Result<ResolvedPath> {
           self.resolve(path, PathProvenance::Explicit)
       }

       /// Resolve with implicit provenance (with canonicalization)
       pub fn resolve_implicit(&self, path: &Path) -> Result<ResolvedPath> {
           self.resolve(path, PathProvenance::Implicit)
       }

       /// Force canonicalization regardless of provenance
       pub fn resolve_canonical(&self, path: &Path) -> Result<ResolvedPath> {
           let normalized = normalize::normalize(path)?;
           let canonical = canonicalize::canonicalize(&normalized)?;

           Ok(ResolvedPath {
               path: canonical,
               original: path.to_path_buf(),
               canonicalized: true,
               provenance: PathProvenance::Explicit, // Provenance doesn't matter here
           })
       }
   }
   ```

**Verification**:
- Explicit paths are not canonicalized
- Implicit paths are canonicalized when they exist
- Non-existent paths generate warnings
- API is ergonomic and clear

### Task 5: Implement Path Relationship Checking

**Objective**: Create logic to determine relationships between paths.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/relationship.rs`

**Implementation Details**:

1. Define relationship enum:
   ```rust
   use std::path::Path;

   /// Relationship between two paths
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum PathRelationship {
       /// First path is an ancestor of second
       Ancestor,
       /// First path is a descendant of second
       Descendant,
       /// Paths are the same
       Same,
       /// Paths are unrelated (neither ancestor nor descendant)
       Unrelated,
   }
   ```

2. Implement relationship checking:
   ```rust
   impl PathRelationship {
       /// Determine the relationship between two paths
       pub fn between(path1: &Path, path2: &Path) -> Self {
           // Normalize for comparison (basic normalization, not full resolution)
           let p1 = normalize_for_comparison(path1);
           let p2 = normalize_for_comparison(path2);

           if p1 == p2 {
               return PathRelationship::Same;
           }

           // Check if path1 is ancestor of path2
           if p2.starts_with(&p1) {
               return PathRelationship::Ancestor;
           }

           // Check if path1 is descendant of path2
           if p1.starts_with(&p2) {
               return PathRelationship::Descendant;
           }

           PathRelationship::Unrelated
       }

       /// Check if relationship is hierarchical (ancestor/descendant/same)
       pub fn is_hierarchical(&self) -> bool {
           matches!(self,
               PathRelationship::Ancestor |
               PathRelationship::Descendant |
               PathRelationship::Same
           )
       }

       /// Check if this represents an "allowed" relationship for operations
       pub fn is_allowed_without_force(&self) -> bool {
           // Per spec: up/down the hierarchy is ok, sideways is not
           self.is_hierarchical()
       }
   }

   fn normalize_for_comparison(path: &Path) -> PathBuf {
       // Remove trailing slashes and normalize separators
       let mut p = path.to_path_buf();

       // Remove trailing separator if present
       let s = p.to_str().unwrap_or("");
       if s.ends_with('/') || s.ends_with('\\') {
           if s.len() > 1 {
               p = PathBuf::from(&s[..s.len()-1]);
           }
       }

       p
   }
   ```

3. Add helper methods for common checks:
   ```rust
   impl PathRelationship {
       /// Check if path is within directory (descendant or same)
       pub fn is_within(path: &Path, directory: &Path) -> bool {
           let rel = Self::between(path, directory);
           matches!(rel, PathRelationship::Descendant | PathRelationship::Same)
       }

       /// Check if path contains directory (ancestor or same)
       pub fn contains(path: &Path, other: &Path) -> bool {
           let rel = Self::between(path, other);
           matches!(rel, PathRelationship::Ancestor | PathRelationship::Same)
       }

       /// Get human-readable description
       pub fn description(&self, path1: &Path, path2: &Path) -> String {
           match self {
               PathRelationship::Ancestor =>
                   format!("{} is an ancestor of {}",
                       path1.display(), path2.display()),
               PathRelationship::Descendant =>
                   format!("{} is a descendant of {}",
                       path1.display(), path2.display()),
               PathRelationship::Same =>
                   format!("{} and {} are the same path",
                       path1.display(), path2.display()),
               PathRelationship::Unrelated =>
                   format!("{} and {} are unrelated paths",
                       path1.display(), path2.display()),
           }
       }
   }
   ```

**Verification**:
- Ancestor/descendant detection works correctly
- Same path detection handles normalization
- Unrelated paths correctly identified
- Edge cases handled (root, relative paths)

### Task 6: Update Error Types

**Objective**: Add path-specific error variants.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/error.rs`

**Implementation Details**:

1. Add new error variants:
   ```rust
   #[derive(Error, Debug)]
   pub enum Error {
       // ... existing variants ...

       /// Path does not exist
       #[error("Path not found: {0}")]
       PathNotFound(String),

       /// Permission denied accessing path
       #[error("Permission denied: {0}")]
       PermissionDenied(String),

       /// Symlink loop detected
       #[error("Symlink loop detected: {0}")]
       SymlinkLoop(String),

       /// Path relationship violation
       #[error("Path relationship violation: {0}")]
       PathRelationshipViolation(String),
   }
   ```

2. Add helper methods:
   ```rust
   impl Error {
       /// Check if error indicates path doesn't exist
       pub fn is_not_found(&self) -> bool {
           matches!(self, Error::PathNotFound(_))
       }

       /// Check if error is permission-related
       pub fn is_permission_denied(&self) -> bool {
           matches!(self, Error::PermissionDenied(_))
       }
   }
   ```

**Verification**:
- Error messages are clear and actionable
- Error types cover all path operations
- Conversions from std::io::Error work

### Task 7: Integration with Database Layer

**Objective**: Update database operations to use normalized paths.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/database/operations.rs`
- `/Users/prb/github/trop/trop/src/reservation.rs`

**Implementation Details**:

1. Update ReservationKey to use resolved paths:
   ```rust
   // In reservation.rs
   use crate::path::{PathResolver, PathProvenance};

   impl ReservationKey {
       /// Create a new reservation key with explicit path
       pub fn with_explicit_path(path: PathBuf, tag: Option<String>) -> Result<Self> {
           let resolver = PathResolver::new();
           let resolved = resolver.resolve_explicit(&path)?;
           Self::new(resolved.path, tag)
       }

       /// Create a new reservation key with implicit path
       pub fn with_implicit_path(path: PathBuf, tag: Option<String>) -> Result<Self> {
           let resolver = PathResolver::new();
           let resolved = resolver.resolve_implicit(&path)?;
           Self::new(resolved.path, tag)
       }
   }
   ```

2. Add path validation in database operations:
   ```rust
   // In database/operations.rs
   use crate::path::PathRelationship;

   impl Database {
       /// Validate path relationship for operations
       pub fn validate_path_relationship(
           &self,
           new_path: &Path,
           current_path: &Path,
           allow_unrelated: bool
       ) -> Result<()> {
           let relationship = PathRelationship::between(new_path, current_path);

           if !relationship.is_allowed_without_force() && !allow_unrelated {
               return Err(Error::PathRelationshipViolation(
                   format!("Cannot operate on unrelated path: {}",
                       relationship.description(new_path, current_path))
               ));
           }

           Ok(())
       }
   }
   ```

**Verification**:
- Database stores normalized paths
- Path validation works in operations
- Existing tests still pass

### Task 8: Create Comprehensive Unit Tests

**Objective**: Write thorough unit tests for all path operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/tests/normalize_tests.rs`
- `/Users/prb/github/trop/trop/src/path/tests/canonicalize_tests.rs`
- `/Users/prb/github/trop/trop/src/path/tests/relationship_tests.rs`
- `/Users/prb/github/trop/trop/src/path/tests/resolver_tests.rs`

**Implementation Details**:

1. Test normalization:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       use std::env;

       #[test]
       fn test_expand_tilde() {
           let home = env::var("HOME").or(env::var("USERPROFILE")).unwrap();

           assert_eq!(expand_tilde(Path::new("~")).unwrap(), PathBuf::from(&home));
           assert_eq!(
               expand_tilde(Path::new("~/test")).unwrap(),
               PathBuf::from(&home).join("test")
           );
           assert_eq!(
               expand_tilde(Path::new("/absolute")).unwrap(),
               PathBuf::from("/absolute")
           );
       }

       #[test]
       fn test_resolve_components() {
           assert_eq!(
               resolve_components(Path::new("/a/./b/../c")).unwrap(),
               PathBuf::from("/a/c")
           );
           assert_eq!(
               resolve_components(Path::new("/a/b/../../c")).unwrap(),
               PathBuf::from("/c")
           );
       }

       #[test]
       fn test_normalize_relative() {
           let cwd = env::current_dir().unwrap();
           let normalized = normalize(Path::new("relative/path")).unwrap();
           assert!(normalized.is_absolute());
           assert!(normalized.starts_with(&cwd));
       }

       #[test]
       fn test_too_many_parent_components() {
           let result = resolve_components(Path::new("/a/../../.."));
           assert!(result.is_err());
       }
   }
   ```

2. Test canonicalization with temporary files:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       use tempfile::tempdir;
       use std::fs;
       use std::os::unix::fs::symlink;

       #[test]
       fn test_canonicalize_symlink() {
           let dir = tempdir().unwrap();
           let target = dir.path().join("target");
           let link = dir.path().join("link");

           fs::write(&target, "test").unwrap();
           symlink(&target, &link).unwrap();

           let canonical = canonicalize(&link).unwrap();
           assert_eq!(canonical, target);
       }

       #[test]
       fn test_canonicalize_nonexistent() {
           let result = canonicalize(Path::new("/nonexistent/path"));
           assert!(result.is_err());
           assert!(result.unwrap_err().is_not_found());
       }

       #[test]
       fn test_canonicalize_existing_partial() {
           let dir = tempdir().unwrap();
           let existing = dir.path();
           let full = existing.join("nonexistent").join("path");

           let (canonical, remainder) = canonicalize_existing(&full).unwrap();
           assert_eq!(canonical, existing.canonicalize().unwrap());
           assert_eq!(remainder, Some(PathBuf::from("nonexistent/path")));
       }
   }
   ```

3. Test path relationships:
   ```rust
   #[test]
   fn test_path_relationships() {
       use PathRelationship::*;

       assert_eq!(
           PathRelationship::between(Path::new("/a"), Path::new("/a/b")),
           Ancestor
       );
       assert_eq!(
           PathRelationship::between(Path::new("/a/b"), Path::new("/a")),
           Descendant
       );
       assert_eq!(
           PathRelationship::between(Path::new("/a"), Path::new("/a")),
           Same
       );
       assert_eq!(
           PathRelationship::between(Path::new("/a"), Path::new("/b")),
           Unrelated
       );
   }

   #[test]
   fn test_relationship_with_trailing_slash() {
       assert_eq!(
           PathRelationship::between(Path::new("/a/"), Path::new("/a")),
           Same
       );
   }
   ```

**Verification**:
- All test cases pass
- Edge cases covered
- Platform-specific tests work

### Task 9: Add Property-Based Tests

**Objective**: Use property-based testing to validate path operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/path/tests/property_tests.rs`

**Implementation Details**:

1. Add proptest dependency:
   ```toml
   [dev-dependencies]
   proptest = "1.0"
   ```

2. Create property tests:
   ```rust
   #[cfg(test)]
   mod property_tests {
       use proptest::prelude::*;
       use super::*;

       proptest! {
           #[test]
           fn test_normalize_always_absolute(path in any::<String>()) {
               if let Ok(normalized) = normalize(Path::new(&path)) {
                   prop_assert!(normalized.is_absolute());
               }
           }

           #[test]
           fn test_normalize_idempotent(path in any::<String>()) {
               if let Ok(normalized) = normalize(Path::new(&path)) {
                   let normalized2 = normalize(&normalized).unwrap();
                   prop_assert_eq!(normalized, normalized2);
               }
           }

           #[test]
           fn test_relationship_reflexive(path in any::<String>()) {
               let p = Path::new(&path);
               let rel = PathRelationship::between(p, p);
               prop_assert_eq!(rel, PathRelationship::Same);
           }

           #[test]
           fn test_relationship_antisymmetric(
               path1 in any::<String>(),
               path2 in any::<String>()
           ) {
               let p1 = Path::new(&path1);
               let p2 = Path::new(&path2);
               let rel1 = PathRelationship::between(p1, p2);
               let rel2 = PathRelationship::between(p2, p1);

               use PathRelationship::*;
               match (rel1, rel2) {
                   (Ancestor, Descendant) | (Descendant, Ancestor) => {},
                   (Same, Same) | (Unrelated, Unrelated) => {},
                   _ => prop_assert!(false, "Invalid relationship symmetry"),
               }
           }
       }
   }
   ```

3. Test resolver properties:
   ```rust
   proptest! {
       #[test]
       fn test_resolver_preserves_explicit(path in any::<String>()) {
           let resolver = PathResolver::new();
           if let Ok(resolved) = resolver.resolve_explicit(Path::new(&path)) {
               prop_assert!(!resolved.canonicalized);
               prop_assert_eq!(resolved.provenance, PathProvenance::Explicit);
           }
       }

       #[test]
       fn test_resolver_canonicalizes_implicit_when_exists(path in any::<String>()) {
           let resolver = PathResolver::new();
           // Use actual existing path for this test
           if let Ok(resolved) = resolver.resolve_implicit(Path::new("/tmp")) {
               prop_assert!(resolved.canonicalized || !Path::new("/tmp").exists());
           }
       }
   }
   ```

**Verification**:
- Property tests discover no violations
- Good coverage of path space
- Tests run in reasonable time

### Task 10: Create Integration Tests

**Objective**: Write integration tests that combine path operations with database.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/path_integration.rs`

**Implementation Details**:

1. Test path resolution in reservation workflow:
   ```rust
   use trop::{Database, DatabaseConfig, Reservation, ReservationKey};
   use trop::path::{PathResolver, PathProvenance};
   use tempfile::tempdir;

   #[test]
   fn test_reservation_with_path_resolution() {
       let dir = tempdir().unwrap();
       let db_path = dir.path().join("test.db");
       let mut db = Database::open(DatabaseConfig::new(db_path)).unwrap();

       // Create reservation with explicit path
       let resolver = PathResolver::new();
       let explicit_path = dir.path().join("explicit");
       let resolved = resolver.resolve_explicit(&explicit_path).unwrap();

       let key = ReservationKey::new(resolved.path, None).unwrap();
       let reservation = Reservation::builder()
           .key(key.clone())
           .port(5000.try_into().unwrap())
           .build()
           .unwrap();

       db.create_reservation(&reservation).unwrap();

       // Retrieve and verify
       let loaded = db.get_reservation(&key).unwrap().unwrap();
       assert_eq!(loaded.key, key);
   }
   ```

2. Test path relationship validation:
   ```rust
   #[test]
   fn test_unrelated_path_rejection() {
       let dir = tempdir().unwrap();
       let db_path = dir.path().join("test.db");
       let db = Database::open(DatabaseConfig::new(db_path)).unwrap();

       let current = Path::new("/users/me/project");
       let unrelated = Path::new("/users/other/project");

       let result = db.validate_path_relationship(unrelated, current, false);
       assert!(result.is_err());
       assert!(result.unwrap_err().to_string().contains("unrelated"));

       // Should work with allow_unrelated
       let result = db.validate_path_relationship(unrelated, current, true);
       assert!(result.is_ok());
   }
   ```

3. Test with symlinks:
   ```rust
   #[test]
   #[cfg(unix)]
   fn test_implicit_path_with_symlink() {
       use std::os::unix::fs::symlink;

       let dir = tempdir().unwrap();
       let real_dir = dir.path().join("real");
       let link_dir = dir.path().join("link");

       std::fs::create_dir(&real_dir).unwrap();
       symlink(&real_dir, &link_dir).unwrap();

       let resolver = PathResolver::new();
       let resolved = resolver.resolve_implicit(&link_dir).unwrap();

       // Should be canonicalized to real path
       assert!(resolved.canonicalized);
       assert_eq!(resolved.path, real_dir.canonicalize().unwrap());
   }
   ```

**Verification**:
- Integration tests pass
- Path resolution works with database
- Symlink handling correct

### Task 11: Add Benchmarks

**Objective**: Create performance benchmarks for path operations.

**Files to Create**:
- `/Users/prb/github/trop/trop/benches/path_bench.rs`

**Implementation Details**:

1. Add to `Cargo.toml`:
   ```toml
   [[bench]]
   name = "path_bench"
   harness = false
   ```

2. Create benchmarks:
   ```rust
   use criterion::{black_box, criterion_group, criterion_main, Criterion};
   use trop::path::{PathResolver, normalize, PathRelationship};
   use std::path::Path;

   fn bench_normalize(c: &mut Criterion) {
       c.bench_function("normalize_absolute", |b| {
           b.iter(|| {
               normalize::normalize(black_box(Path::new("/absolute/path/to/file")))
           });
       });

       c.bench_function("normalize_relative", |b| {
           b.iter(|| {
               normalize::normalize(black_box(Path::new("./relative/path")))
           });
       });

       c.bench_function("normalize_with_dots", |b| {
           b.iter(|| {
               normalize::normalize(black_box(Path::new("/a/b/../c/./d")))
           });
       });
   }

   fn bench_relationship(c: &mut Criterion) {
       let path1 = Path::new("/users/test/projects/trop/src");
       let path2 = Path::new("/users/test/projects/other");

       c.bench_function("relationship_check", |b| {
           b.iter(|| {
               PathRelationship::between(black_box(path1), black_box(path2))
           });
       });
   }

   fn bench_resolver(c: &mut Criterion) {
       let resolver = PathResolver::new();
       let path = Path::new("/tmp/test");

       c.bench_function("resolve_explicit", |b| {
           b.iter(|| {
               resolver.resolve_explicit(black_box(path))
           });
       });
   }

   criterion_group!(benches, bench_normalize, bench_relationship, bench_resolver);
   criterion_main!(benches);
   ```

**Verification**:
- Benchmarks run successfully
- No obvious performance issues
- Results are reasonable

### Task 12: Update Documentation

**Objective**: Add comprehensive documentation for path module.

**Files to Modify**:
- All path module files
- `/Users/prb/github/trop/trop/src/lib.rs`

**Implementation Details**:

1. Add module documentation:
   ```rust
   //! # Path Handling Module
   //!
   //! This module provides comprehensive path handling for the trop system,
   //! including normalization, canonicalization, and relationship checking.
   //!
   //! ## Key Concepts
   //!
   //! - **Normalization**: Converting paths to absolute form, expanding ~ and
   //!   resolving . and .. components
   //! - **Canonicalization**: Following symlinks to get the "real" path
   //! - **Provenance**: Whether a path was explicitly provided or implicitly
   //!   inferred affects how it's processed
   //!
   //! ## Examples
   //!
   //! ```
   //! use trop::path::{PathResolver, PathProvenance};
   //!
   //! let resolver = PathResolver::new();
   //!
   //! // Explicit paths are normalized but not canonicalized
   //! let explicit = resolver.resolve_explicit(Path::new("~/project")).unwrap();
   //! assert!(!explicit.canonicalized);
   //!
   //! // Implicit paths are both normalized and canonicalized
   //! let implicit = resolver.resolve_implicit(Path::new(".")).unwrap();
   //! assert!(implicit.canonicalized);
   //! ```
   ```

2. Update crate root exports:
   ```rust
   // In lib.rs
   pub mod path;
   pub use path::{PathResolver, PathRelationship, PathProvenance};
   ```

**Verification**:
- Documentation builds without warnings
- Examples compile and run
- API documentation is complete

## Dependencies Between Tasks

```
Task 1 (Module Structure)
    ├── Task 2 (Normalization)
    ├── Task 3 (Canonicalization)
    └── Task 5 (Relationships)
         └── Task 4 (PathResolver)
              └── Task 6 (Error Types)
                   └── Task 7 (Database Integration)
                        ├── Task 8 (Unit Tests)
                        ├── Task 9 (Property Tests)
                        ├── Task 10 (Integration Tests)
                        ├── Task 11 (Benchmarks)
                        └── Task 12 (Documentation)
```

Tasks 2, 3, and 5 can be developed in parallel after Task 1. Task 4 requires Tasks 2 and 3. Testing tasks (8-11) can proceed in parallel after Task 7.

## Testing Strategy

### Unit Tests
- Test each path operation in isolation
- Cover edge cases (empty paths, root, .., symlinks)
- Test platform-specific behavior
- Validate error conditions

### Property-Based Tests
- Verify invariants (normalization idempotent, relationships antisymmetric)
- Test with randomly generated paths
- Ensure no panics on arbitrary input
- Validate algorithm properties

### Integration Tests
- Test path resolution with database operations
- Verify symlink handling in real filesystem
- Test permission and non-existence handling
- Validate provenance-based behavior

## Validation Checklist

Before considering Phase 3 complete:

- [ ] Path normalization handles all special cases
- [ ] Canonicalization follows symlinks correctly
- [ ] Provenance rules properly enforced
- [ ] Path relationships accurately determined
- [ ] Non-existent paths handled gracefully
- [ ] Database integration seamless
- [ ] All tests pass on all platforms
- [ ] No performance regressions
- [ ] Documentation complete
- [ ] API is ergonomic and clear

## Risk Mitigations

### Platform Differences
- Test extensively on Windows, macOS, and Linux
- Handle path separator differences
- Account for case-sensitivity variations
- Test symlink behavior on each platform

### Symlink Complexity
- Implement loop detection
- Handle broken symlinks gracefully
- Test with deeply nested symlinks
- Consider permission issues

### Performance Concerns
- Benchmark critical operations
- Cache normalized paths where appropriate
- Avoid unnecessary filesystem operations
- Profile before optimizing

### Security Considerations
- Validate paths don't escape expected directories
- Handle permission errors gracefully
- Don't follow symlinks for explicit paths
- Log suspicious path operations

## Implementation Decisions

### Provenance Tracking
- Explicit vs implicit determines canonicalization
- Tracked through entire path lifecycle
- Preserved in database storage
- Clear API separation

### Error Handling Strategy
- Specific error types for each failure mode
- Non-existent paths don't fail normalization
- Warnings for non-existent implicit paths
- Clear error messages with paths

### Normalization Approach
- Always expand ~ first
- Then make absolute
- Finally resolve . and ..
- Platform-specific separator handling

### Relationship Algorithm
- Use path prefix matching
- Normalize before comparison
- Handle trailing separators
- Case-sensitive on Unix, insensitive on Windows

## Next Phase Preparation

Phase 4 will implement basic reservation operations. Ensure:
- Paths properly normalized before storage
- Path validation integrated into operations
- Relationship checking available for access control
- Error types support reservation failures

## Notes for Implementer

### Code Organization
- Keep normalization and canonicalization separate
- Use builder pattern for complex types
- Validate at construction when possible
- Provide both strict and lenient APIs

### Error Philosophy
- Fail fast for invalid input
- Warn for suspicious but valid operations
- Provide detailed error context
- Make errors actionable

### Testing Approach
- Test both success and failure paths
- Use real filesystem for integration tests
- Mock where filesystem would be slow
- Cover platform-specific code

### Performance Notes
- Path operations can be expensive
- Consider caching normalized paths
- Batch operations where possible
- Profile before optimizing

This plan provides comprehensive guidance for implementing the path handling system while maintaining compatibility with existing code and preparing for future phases.