//! Regression tests for path-handling bugs fixed with the verified path
//! kernels. Each test fails on the implementation that preceded them.

use std::path::{Path, PathBuf};

use trop::path::normalize::{expand_tilde, normalize};
use trop::path::{NormalizedPath, PathProvenance, PathRelationship};

/// `~//etc/foo` used to expand to `/etc/foo`: joining an absolute remainder
/// onto the home directory replaced it.
#[test]
fn tilde_with_extra_separators_expands_under_home() {
    let home = home::home_dir().expect("home directory");
    for input in ["~//etc/foo", "~///etc/foo"] {
        let expanded = expand_tilde(Path::new(input)).unwrap();
        assert_ne!(expanded, Path::new("/etc/foo"), "{input}");
        assert_eq!(expanded, home.join("etc").join("foo"), "{input}");
        assert_eq!(
            normalize(Path::new(input)).unwrap(),
            normalize(&home.join("etc").join("foo")).unwrap(),
            "{input}"
        );
    }
}

/// On Unix `~\x` used to be expanded to `$HOME/x`. A backslash is an
/// ordinary filename byte there, so `~\x` names the relative file `~\x`.
#[test]
#[cfg(unix)]
fn tilde_backslash_is_a_relative_name_on_unix() {
    assert_eq!(
        expand_tilde(Path::new("~\\x")).unwrap(),
        PathBuf::from("~\\x")
    );
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(normalize(Path::new("~\\x")).unwrap(), cwd.join("~\\x"));
}

/// `between` used to compare `..` literally: `/a/b/..` was a descendant of
/// `/a` although both name the same directory.
#[test]
fn relationship_resolves_parent_components() {
    assert_eq!(
        PathRelationship::between(Path::new("/a/b/.."), Path::new("/a")),
        PathRelationship::Same
    );
    assert!(PathRelationship::is_within(
        Path::new("/a/b/../c"),
        Path::new("/a")
    ));
    assert!(!PathRelationship::is_within(
        Path::new("/a/../b"),
        Path::new("/a")
    ));
    assert!(!PathRelationship::is_within(
        Path::new("/a/b/.."),
        Path::new("/a/b")
    ));
}

/// The empty (or any relative) path used to be an ancestor of every
/// absolute path.
#[test]
fn relative_path_is_never_an_ancestor_of_an_absolute_one() {
    assert_eq!(
        PathRelationship::between(Path::new(""), Path::new("/a")),
        PathRelationship::Unrelated
    );
    assert!(!PathRelationship::contains(Path::new(""), Path::new("/a")));
    assert_eq!(
        PathRelationship::between(Path::new("a"), Path::new("/a/b")),
        PathRelationship::Unrelated
    );
}

/// `NormalizedPath::new` used to accept any absolute path, including
/// spellings that normalization never produces.
#[test]
#[cfg(unix)]
fn normalized_path_rejects_non_normalized_spellings() {
    for spelling in ["/a/../b", "/a/./b", "/a//b", "/a/b/"] {
        assert!(
            NormalizedPath::new(PathBuf::from(spelling), PathProvenance::Explicit).is_err(),
            "{spelling}"
        );
    }
    assert!(NormalizedPath::new(PathBuf::from("/a/b"), PathProvenance::Explicit).is_ok());
    assert!(NormalizedPath::new(PathBuf::from("/"), PathProvenance::Explicit).is_ok());
}
