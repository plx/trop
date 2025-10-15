//! Path relationship checking.
//!
//! This module provides functionality to determine the relationship between
//! two paths, such as whether one is an ancestor or descendant of the other.

use std::path::{Path, PathBuf};

/// Relationship between two paths.
///
/// This enum describes how two paths relate to each other in the filesystem
/// hierarchy.
///
/// # Examples
///
/// ```
/// use trop::path::PathRelationship;
/// use std::path::Path;
///
/// let parent = Path::new("/home/user");
/// let child = Path::new("/home/user/project");
///
/// assert_eq!(
///     PathRelationship::between(parent, child),
///     PathRelationship::Ancestor
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathRelationship {
    /// The first path is an ancestor of the second.
    ///
    /// This means the second path is somewhere beneath the first in the
    /// directory hierarchy.
    Ancestor,

    /// The first path is a descendant of the second.
    ///
    /// This means the first path is somewhere beneath the second in the
    /// directory hierarchy.
    Descendant,

    /// The paths are the same.
    ///
    /// After normalization, the paths point to the same location.
    Same,

    /// The paths are unrelated.
    ///
    /// Neither path is an ancestor or descendant of the other - they are
    /// in different branches of the filesystem tree.
    Unrelated,
}

impl PathRelationship {
    /// Determine the relationship between two paths.
    ///
    /// This function normalizes both paths for comparison by removing trailing
    /// separators and then checks their prefix relationships.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    /// use std::path::Path;
    ///
    /// let rel = PathRelationship::between(
    ///     Path::new("/a"),
    ///     Path::new("/a/b")
    /// );
    /// assert_eq!(rel, PathRelationship::Ancestor);
    ///
    /// let rel = PathRelationship::between(
    ///     Path::new("/a/b"),
    ///     Path::new("/a")
    /// );
    /// assert_eq!(rel, PathRelationship::Descendant);
    ///
    /// let rel = PathRelationship::between(
    ///     Path::new("/a"),
    ///     Path::new("/a")
    /// );
    /// assert_eq!(rel, PathRelationship::Same);
    ///
    /// let rel = PathRelationship::between(
    ///     Path::new("/a"),
    ///     Path::new("/b")
    /// );
    /// assert_eq!(rel, PathRelationship::Unrelated);
    /// ```
    #[must_use]
    pub fn between(path1: &Path, path2: &Path) -> Self {
        // Normalize for comparison
        let p1 = normalize_for_comparison(path1);
        let p2 = normalize_for_comparison(path2);

        if p1 == p2 {
            return Self::Same;
        }

        // Check if path1 is ancestor of path2
        if p2.starts_with(&p1) {
            return Self::Ancestor;
        }

        // Check if path1 is descendant of path2
        if p1.starts_with(&p2) {
            return Self::Descendant;
        }

        Self::Unrelated
    }

    /// Check if the relationship is hierarchical (not unrelated).
    ///
    /// Returns `true` for `Ancestor`, `Descendant`, or `Same`, and `false`
    /// for `Unrelated`.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    ///
    /// assert!(PathRelationship::Ancestor.is_hierarchical());
    /// assert!(PathRelationship::Descendant.is_hierarchical());
    /// assert!(PathRelationship::Same.is_hierarchical());
    /// assert!(!PathRelationship::Unrelated.is_hierarchical());
    /// ```
    #[must_use]
    pub fn is_hierarchical(&self) -> bool {
        matches!(self, Self::Ancestor | Self::Descendant | Self::Same)
    }

    /// Check if this relationship is allowed without force flag.
    ///
    /// Per the specification, operations moving up or down the hierarchy are
    /// allowed, but sideways moves (unrelated paths) require a force flag.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    ///
    /// assert!(PathRelationship::Ancestor.is_allowed_without_force());
    /// assert!(PathRelationship::Descendant.is_allowed_without_force());
    /// assert!(PathRelationship::Same.is_allowed_without_force());
    /// assert!(!PathRelationship::Unrelated.is_allowed_without_force());
    /// ```
    #[must_use]
    pub fn is_allowed_without_force(&self) -> bool {
        self.is_hierarchical()
    }

    /// Check if a path is within a directory (descendant or same).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    /// use std::path::Path;
    ///
    /// let dir = Path::new("/home/user");
    /// let file = Path::new("/home/user/file.txt");
    ///
    /// assert!(PathRelationship::is_within(file, dir));
    /// assert!(PathRelationship::is_within(dir, dir));
    /// ```
    #[must_use]
    pub fn is_within(path: &Path, directory: &Path) -> bool {
        let rel = Self::between(path, directory);
        matches!(rel, Self::Descendant | Self::Same)
    }

    /// Check if a path contains another path (ancestor or same).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    /// use std::path::Path;
    ///
    /// let dir = Path::new("/home/user");
    /// let file = Path::new("/home/user/file.txt");
    ///
    /// assert!(PathRelationship::contains(dir, file));
    /// assert!(PathRelationship::contains(dir, dir));
    /// ```
    #[must_use]
    pub fn contains(path: &Path, other: &Path) -> bool {
        let rel = Self::between(path, other);
        matches!(rel, Self::Ancestor | Self::Same)
    }

    /// Get a human-readable description of the relationship.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::path::PathRelationship;
    /// use std::path::Path;
    ///
    /// let p1 = Path::new("/a");
    /// let p2 = Path::new("/a/b");
    /// let rel = PathRelationship::Ancestor;
    ///
    /// let desc = rel.description(p1, p2);
    /// assert!(desc.contains("/a"));
    /// assert!(desc.contains("/a/b"));
    /// assert!(desc.contains("ancestor"));
    /// ```
    #[must_use]
    pub fn description(&self, path1: &Path, path2: &Path) -> String {
        match self {
            Self::Ancestor => {
                format!("{} is an ancestor of {}", path1.display(), path2.display())
            }
            Self::Descendant => {
                format!("{} is a descendant of {}", path1.display(), path2.display())
            }
            Self::Same => {
                format!(
                    "{} and {} are the same path",
                    path1.display(),
                    path2.display()
                )
            }
            Self::Unrelated => {
                format!(
                    "{} and {} are unrelated paths",
                    path1.display(),
                    path2.display()
                )
            }
        }
    }
}

/// Normalize a path for comparison purposes.
///
/// This function removes trailing slashes and normalizes separators to ensure
/// consistent comparison results.
fn normalize_for_comparison(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();

    // Remove trailing separator if present (but not for root)
    if let Some(s) = p.to_str() {
        if s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) {
            p = PathBuf::from(&s[..s.len() - 1]);
        }
    }

    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relationship_ancestor() {
        assert_eq!(
            PathRelationship::between(Path::new("/a"), Path::new("/a/b")),
            PathRelationship::Ancestor
        );
        assert_eq!(
            PathRelationship::between(Path::new("/a/b"), Path::new("/a/b/c/d")),
            PathRelationship::Ancestor
        );
    }

    #[test]
    fn test_relationship_descendant() {
        assert_eq!(
            PathRelationship::between(Path::new("/a/b"), Path::new("/a")),
            PathRelationship::Descendant
        );
        assert_eq!(
            PathRelationship::between(Path::new("/a/b/c/d"), Path::new("/a/b")),
            PathRelationship::Descendant
        );
    }

    #[test]
    fn test_relationship_same() {
        assert_eq!(
            PathRelationship::between(Path::new("/a"), Path::new("/a")),
            PathRelationship::Same
        );
        assert_eq!(
            PathRelationship::between(Path::new("/a/b/c"), Path::new("/a/b/c")),
            PathRelationship::Same
        );
    }

    #[test]
    fn test_relationship_unrelated() {
        assert_eq!(
            PathRelationship::between(Path::new("/a"), Path::new("/b")),
            PathRelationship::Unrelated
        );
        assert_eq!(
            PathRelationship::between(Path::new("/a/b"), Path::new("/a/c")),
            PathRelationship::Unrelated
        );
    }

    #[test]
    fn test_relationship_with_trailing_slash() {
        assert_eq!(
            PathRelationship::between(Path::new("/a/"), Path::new("/a")),
            PathRelationship::Same
        );
        assert_eq!(
            PathRelationship::between(Path::new("/a"), Path::new("/a/")),
            PathRelationship::Same
        );
    }

    #[test]
    fn test_is_hierarchical() {
        assert!(PathRelationship::Ancestor.is_hierarchical());
        assert!(PathRelationship::Descendant.is_hierarchical());
        assert!(PathRelationship::Same.is_hierarchical());
        assert!(!PathRelationship::Unrelated.is_hierarchical());
    }

    #[test]
    fn test_is_allowed_without_force() {
        assert!(PathRelationship::Ancestor.is_allowed_without_force());
        assert!(PathRelationship::Descendant.is_allowed_without_force());
        assert!(PathRelationship::Same.is_allowed_without_force());
        assert!(!PathRelationship::Unrelated.is_allowed_without_force());
    }

    #[test]
    fn test_is_within() {
        assert!(PathRelationship::is_within(
            Path::new("/a/b"),
            Path::new("/a")
        ));
        assert!(PathRelationship::is_within(
            Path::new("/a"),
            Path::new("/a")
        ));
        assert!(!PathRelationship::is_within(
            Path::new("/a"),
            Path::new("/a/b")
        ));
        assert!(!PathRelationship::is_within(
            Path::new("/a"),
            Path::new("/b")
        ));
    }

    #[test]
    fn test_contains() {
        assert!(PathRelationship::contains(
            Path::new("/a"),
            Path::new("/a/b")
        ));
        assert!(PathRelationship::contains(Path::new("/a"), Path::new("/a")));
        assert!(!PathRelationship::contains(
            Path::new("/a/b"),
            Path::new("/a")
        ));
        assert!(!PathRelationship::contains(
            Path::new("/a"),
            Path::new("/b")
        ));
    }

    #[test]
    fn test_description() {
        let desc = PathRelationship::Ancestor.description(Path::new("/a"), Path::new("/a/b"));
        assert!(desc.contains("/a"));
        assert!(desc.contains("/a/b"));
        assert!(desc.contains("ancestor"));

        let desc = PathRelationship::Descendant.description(Path::new("/a/b"), Path::new("/a"));
        assert!(desc.contains("/a"));
        assert!(desc.contains("/a/b"));
        assert!(desc.contains("descendant"));

        let desc = PathRelationship::Same.description(Path::new("/a"), Path::new("/a"));
        assert!(desc.contains("/a"));
        assert!(desc.contains("same"));

        let desc = PathRelationship::Unrelated.description(Path::new("/a"), Path::new("/b"));
        assert!(desc.contains("/a"));
        assert!(desc.contains("/b"));
        assert!(desc.contains("unrelated"));
    }

    #[test]
    fn test_normalize_for_comparison() {
        assert_eq!(
            normalize_for_comparison(Path::new("/a/")),
            PathBuf::from("/a")
        );
        assert_eq!(
            normalize_for_comparison(Path::new("/a")),
            PathBuf::from("/a")
        );
        assert_eq!(normalize_for_comparison(Path::new("/")), PathBuf::from("/"));
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
            /// Relationship checking is reflexive (path related to itself is Same)
            #[test]
            fn relationship_reflexive(s in path_strategy()) {
                let path = Path::new(&s);
                let rel = PathRelationship::between(path, path);
                prop_assert_eq!(rel, PathRelationship::Same);
            }

            /// Relationship checking has proper symmetry
            /// If A is ancestor of B, then B is descendant of A
            #[test]
            fn relationship_symmetric(s1 in path_strategy(), s2 in path_strategy()) {
                let p1 = Path::new(&s1);
                let p2 = Path::new(&s2);
                let rel1 = PathRelationship::between(p1, p2);
                let rel2 = PathRelationship::between(p2, p1);

                match (rel1, rel2) {
                    (PathRelationship::Ancestor, PathRelationship::Descendant) => {},
                    (PathRelationship::Descendant, PathRelationship::Ancestor) => {},
                    (PathRelationship::Same, PathRelationship::Same) => {},
                    (PathRelationship::Unrelated, PathRelationship::Unrelated) => {},
                    _ => prop_assert!(false, "Invalid relationship symmetry: {:?} vs {:?}", rel1, rel2),
                }
            }

            /// Hierarchical relationships are transitive
            /// If A is ancestor of B and B is ancestor of C, then A is ancestor of C
            #[test]
            fn relationship_transitive(s1 in path_strategy()) {
                // Construct paths with guaranteed hierarchical relationship
                let p1 = Path::new(&s1);
                let p2 = PathBuf::from(&s1).join("subdir");
                let p3 = p2.join("nested");

                // p1 should be ancestor of p2
                let rel12 = PathRelationship::between(p1, &p2);
                prop_assert_eq!(rel12, PathRelationship::Ancestor);

                // p2 should be ancestor of p3
                let rel23 = PathRelationship::between(&p2, &p3);
                prop_assert_eq!(rel23, PathRelationship::Ancestor);

                // p1 should be ancestor of p3 (transitivity)
                let rel13 = PathRelationship::between(p1, &p3);
                prop_assert_eq!(rel13, PathRelationship::Ancestor);
            }

            /// is_hierarchical returns true for non-Unrelated relationships
            #[test]
            fn is_hierarchical_consistent(s1 in path_strategy(), s2 in path_strategy()) {
                let p1 = Path::new(&s1);
                let p2 = Path::new(&s2);
                let rel = PathRelationship::between(p1, p2);

                let hierarchical = rel.is_hierarchical();
                let is_unrelated = matches!(rel, PathRelationship::Unrelated);

                prop_assert_eq!(hierarchical, !is_unrelated);
            }

            /// is_allowed_without_force matches is_hierarchical
            #[test]
            fn allowed_matches_hierarchical(s1 in path_strategy(), s2 in path_strategy()) {
                let p1 = Path::new(&s1);
                let p2 = Path::new(&s2);
                let rel = PathRelationship::between(p1, p2);

                prop_assert_eq!(rel.is_allowed_without_force(), rel.is_hierarchical());
            }

            /// is_within and contains are consistent
            #[test]
            fn is_within_contains_consistent(s1 in path_strategy(), s2 in path_strategy()) {
                let p1 = Path::new(&s1);
                let p2 = Path::new(&s2);

                let within = PathRelationship::is_within(p1, p2);
                let contains = PathRelationship::contains(p2, p1);

                prop_assert_eq!(within, contains);
            }
        }
    }
}
