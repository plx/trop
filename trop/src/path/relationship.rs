//! Path relationship checking.
//!
//! This module provides functionality to determine the relationship between
//! two paths, such as whether one is an ancestor or descendant of the other.
//!
//! The relationship is decided by the verified kernel
//! `verified::path_relation` (`path_relationship`, `path_is_within`,
//! `path_contains`). It proves the relation is reflexive and flip-symmetric,
//! that `Ancestor` is transitive, that `is_within`/`contains` form a partial
//! order, and that an ancestor's descendants extend it by named components
//! only.

use std::ffi::OsStr;
use std::path::{Component, Path};

use crate::path::component_kind;
use crate::verified::path_relation::{
    path_contains, path_is_within, path_relationship, PathToken, Rel,
};

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
    /// Both paths are compared component by component after lexical
    /// resolution: repeated and trailing separators and `.` components are
    /// ignored, and a `..` cancels the preceding named component (`/a/b/..`
    /// is the same as `/a`).
    ///
    /// A `..` with no named component to cancel (at the root, at the start
    /// of a relative path, or after a drive prefix: exactly where
    /// [`resolve_components`](crate::path::normalize::resolve_components)
    /// fails) is kept as part of the path's *head*, together with its root or
    /// prefix. Two paths are related only when their heads agree, so a
    /// relative path (including the empty path) is `Unrelated` to a rooted
    /// one, `/..` is `Unrelated` to `/`, and `../..` is `Unrelated` to `..`.
    /// With equal heads, the named components decide.
    ///
    /// The relation is reflexive (`between(p, p)` is `Same`), flip-symmetric
    /// (`between(a, b)` is `Ancestor` exactly when `between(b, a)` is
    /// `Descendant`), and `Ancestor` is transitive. `Ancestor` means the
    /// second path is the first followed by one or more named components.
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
        match path_relationship(&path_tokens(path1), &path_tokens(path2)) {
            Rel::Ancestor => Self::Ancestor,
            Rel::Descendant => Self::Descendant,
            Rel::Same => Self::Same,
            Rel::Unrelated => Self::Unrelated,
        }
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
        path_is_within(&path_tokens(path), &path_tokens(directory))
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
        path_contains(&path_tokens(path), &path_tokens(other))
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
    /// let p1_str = p1.display().to_string();
    /// let p2_str = p2.display().to_string();
    /// assert!(desc.contains(&p1_str));
    /// assert!(desc.contains(&p2_str));
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

/// A path's components as kernel tokens: each component's kind and an
/// equality key.
///
/// `std` compares named components by their `OsStr` encoding and Windows
/// prefixes by their parsed form (so `c:` equals `C:`), so a prefix is keyed
/// by its parsed `Prefix` value and every other component by its encoded
/// bytes.
fn path_tokens(path: &Path) -> Vec<PathToken> {
    path.components()
        .map(|component| PathToken {
            kind: component_kind(component),
            name: component_key(component),
        })
        .collect()
}

fn component_key(component: Component<'_>) -> Vec<u8> {
    match component {
        Component::Prefix(prefix) => format!("{:?}", prefix.kind()).into_bytes(),
        other => os_str_key(other.as_os_str()),
    }
}

// Each `os_str_key` is an injective encoding of the `OsStr`, so key equality
// is `OsStr` equality. (`OsStr::as_encoded_bytes` would serve every platform
// at once, but it is newer than the crate's minimum supported Rust version.)

/// The bytes of a Unix `OsStr`.
#[cfg(unix)]
fn os_str_key(value: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

/// The UTF-16 code units of a Windows `OsStr`, as little-endian bytes
/// (fixed-width, so injective).
#[cfg(windows)]
fn os_str_key(value: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}

/// Other platforms: the `Debug` spelling, which quotes the string and escapes
/// backslashes and every byte or code unit that is not valid Unicode, so
/// distinct values never share a spelling.
#[cfg(not(any(unix, windows)))]
fn os_str_key(value: &OsStr) -> Vec<u8> {
    format!("{value:?}").into_bytes()
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
        let path_a = Path::new("/a");
        let path_ab = Path::new("/a/b");
        let desc = PathRelationship::Ancestor.description(path_a, path_ab);
        let path_a_str = path_a.display().to_string();
        let path_ab_str = path_ab.display().to_string();
        assert!(desc.contains(&path_a_str));
        assert!(desc.contains(&path_ab_str));
        assert!(desc.contains("ancestor"));

        let desc = PathRelationship::Descendant.description(path_ab, path_a);
        assert!(desc.contains(&path_a_str));
        assert!(desc.contains(&path_ab_str));
        assert!(desc.contains("descendant"));

        let desc = PathRelationship::Same.description(path_a, path_a);
        assert!(desc.contains(&path_a_str));
        assert!(desc.contains("same"));

        let path_b = Path::new("/b");
        let path_b_str = path_b.display().to_string();
        let desc = PathRelationship::Unrelated.description(path_a, path_b);
        assert!(desc.contains(&path_a_str));
        assert!(desc.contains(&path_b_str));
        assert!(desc.contains("unrelated"));
    }

    #[test]
    fn test_separators_and_dots_do_not_matter() {
        for (left, right) in [
            ("/a/", "/a"),
            ("/a//b", "/a/b"),
            ("/a/./b", "/a/b"),
            ("/", "/"),
            ("//", "/"),
        ] {
            assert_eq!(
                PathRelationship::between(Path::new(left), Path::new(right)),
                PathRelationship::Same,
                "{left} vs {right}"
            );
        }
    }

    // Regression: `..` used to be compared as a literal component, so
    // `/a/b/..` was reported as a descendant of `/a`.
    #[test]
    fn test_parent_components_are_resolved_before_comparing() {
        let cases = [
            ("/a/b/..", "/a", PathRelationship::Same),
            ("/a/b/../c", "/a/c", PathRelationship::Same),
            ("/a/b/../c", "/a", PathRelationship::Descendant),
            ("/a/b/..", "/a/b", PathRelationship::Ancestor),
            ("/a/../b", "/a", PathRelationship::Unrelated),
            ("a/..", "", PathRelationship::Same),
            ("../a", "../a/b", PathRelationship::Ancestor),
            ("../a/..", "..", PathRelationship::Same),
            ("a/../..", "..", PathRelationship::Same),
            ("../a", "a", PathRelationship::Unrelated),
            ("./a", "a", PathRelationship::Same),
        ];
        for (left, right, expected) in cases {
            assert_eq!(
                PathRelationship::between(Path::new(left), Path::new(right)),
                expected,
                "{left} vs {right}"
            );
        }
    }

    // A `..` that cannot be resolved belongs to the path's head: it is never
    // compared as if it were a named child component. (It used to be, so
    // `../..` was reported as a descendant of `..`, and `/..` of `/`.)
    #[test]
    fn test_unresolvable_parent_components_are_part_of_the_head() {
        let cases = [
            ("..", ".", PathRelationship::Unrelated),
            ("..", "", PathRelationship::Unrelated),
            ("../..", "..", PathRelationship::Unrelated),
            ("../../a", "../a", PathRelationship::Unrelated),
            ("/..", "/", PathRelationship::Unrelated),
            ("/../a", "/a", PathRelationship::Unrelated),
            ("/..", "/..", PathRelationship::Same),
            ("/../a", "/../a/b", PathRelationship::Ancestor),
            ("../..", "../../a", PathRelationship::Ancestor),
            ("..", "../a", PathRelationship::Ancestor),
        ];
        for (left, right, expected) in cases {
            assert_eq!(
                PathRelationship::between(Path::new(left), Path::new(right)),
                expected,
                "{left} vs {right}"
            );
        }
        assert!(!PathRelationship::is_within(
            Path::new(".."),
            Path::new(".")
        ));
        assert!(!PathRelationship::is_within(Path::new(".."), Path::new("")));
        assert!(!PathRelationship::is_within(
            Path::new("../.."),
            Path::new("..")
        ));
        assert!(!PathRelationship::contains(
            Path::new("/"),
            Path::new("/..")
        ));
        assert!(PathRelationship::is_within(
            Path::new("../a"),
            Path::new("..")
        ));
    }

    // Windows prefixes are compared in their parsed form, and a `..` right
    // after a drive-relative prefix belongs to the head.
    #[test]
    #[cfg(windows)]
    fn test_windows_prefixes() {
        let cases = [
            (r"C:\a", r"c:\a\b", PathRelationship::Ancestor),
            (r"C:\a", r"D:\a", PathRelationship::Unrelated),
            (r"C:\a", r"\a", PathRelationship::Unrelated),
            ("C:..", "C:", PathRelationship::Unrelated),
            (r"C:..\a", "C:..", PathRelationship::Descendant),
        ];
        for (left, right, expected) in cases {
            assert_eq!(
                PathRelationship::between(Path::new(left), Path::new(right)),
                expected,
                "{left} vs {right}"
            );
        }
    }

    // Regression: the empty path used to be an ancestor of every absolute
    // path. A relative path and a rooted path are never hierarchically
    // related.
    #[test]
    fn test_relative_paths_are_unrelated_to_rooted_paths() {
        for relative in ["", ".", "a", "a/b", ".."] {
            for rooted in ["/", "/a", "/a/b"] {
                assert_eq!(
                    PathRelationship::between(Path::new(relative), Path::new(rooted)),
                    PathRelationship::Unrelated,
                    "{relative:?} vs {rooted:?}"
                );
                assert_eq!(
                    PathRelationship::between(Path::new(rooted), Path::new(relative)),
                    PathRelationship::Unrelated,
                    "{rooted:?} vs {relative:?}"
                );
            }
        }
        assert!(!PathRelationship::contains(Path::new(""), Path::new("/a")));
        assert!(!PathRelationship::is_within(Path::new("/a"), Path::new("")));
        // Relative paths still relate to each other.
        assert_eq!(
            PathRelationship::between(Path::new(""), Path::new("a")),
            PathRelationship::Ancestor
        );
    }

    #[test]
    fn test_component_boundaries() {
        assert_eq!(
            PathRelationship::between(Path::new("/a/b"), Path::new("/a/bc")),
            PathRelationship::Unrelated
        );
        assert_eq!(
            PathRelationship::between(Path::new("/"), Path::new("/a/b")),
            PathRelationship::Ancestor
        );
    }

    // Property-based tests
    #[cfg(feature = "property-tests")]
    mod property_tests {
        use super::*;
        use proptest::prelude::*;
        use std::path::PathBuf;

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
                    (PathRelationship::Ancestor, PathRelationship::Descendant)
                    | (PathRelationship::Descendant, PathRelationship::Ancestor)
                    | (PathRelationship::Same, PathRelationship::Same)
                    | (PathRelationship::Unrelated, PathRelationship::Unrelated) => {},
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
