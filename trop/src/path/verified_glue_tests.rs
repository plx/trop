//! Tests for the glue between `crate::path` and the verified path kernels.
//!
//! The kernels are proved correct against their specifications; these tests
//! check the trusted parts around them: the conversion of `Path` values into
//! kernel inputs, the rebuilding of results, and the assumption that the
//! kernels' byte and token models agree with `std::path`. The `legacy`
//! module keeps the pre-kernel implementations so that every intended
//! behavior change is pinned and every other behavior is shown unchanged.

use std::env;
use std::path::{Component, Path, PathBuf};

use super::canonicalize::canonicalize_existing;
use super::normalize::{expand_tilde, normalize, resolve_components};
use super::relationship::PathRelationship;
use super::{is_canonical_absolute, NormalizedPath, PathProvenance};
use crate::error::Error;

/// The implementations this package replaced, kept verbatim as references.
pub(super) mod legacy {
    use std::env;
    use std::path::{Component, Path, PathBuf};

    use crate::error::{Error, Result};
    use crate::path::PathRelationship;

    pub(in crate::path) fn expand_tilde(path: &Path) -> Result<PathBuf> {
        let path_str = path.to_str().ok_or_else(|| Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "Path contains invalid UTF-8".to_string(),
        })?;

        if !path_str.starts_with('~') {
            return Ok(path.to_path_buf());
        }

        let home = home::home_dir().ok_or_else(|| Error::InvalidPath {
            path: path.to_path_buf(),
            reason: "Cannot determine home directory".to_string(),
        })?;

        if path_str == "~" {
            Ok(home)
        } else if path_str.starts_with("~/") || path_str.starts_with("~\\") {
            Ok(home.join(&path_str[2..]))
        } else {
            Err(Error::InvalidPath {
                path: path.to_path_buf(),
                reason: "~user syntax is not supported; use ~ or ~/path".to_string(),
            })
        }
    }

    pub(in crate::path) fn resolve_components(path: &Path) -> Result<PathBuf> {
        let mut result = PathBuf::new();
        let mut has_root = false;

        for component in path.components() {
            match component {
                Component::RootDir => {
                    result.push(component);
                    has_root = true;
                }
                Component::Prefix(prefix) => {
                    result.push(prefix.as_os_str());
                    has_root = true;
                }
                Component::Normal(c) => {
                    result.push(c);
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if !result.pop() {
                        return Err(Error::InvalidPath {
                            path: path.to_path_buf(),
                            reason: "Path contains too many '..' components (escapes root)"
                                .to_string(),
                        });
                    }
                }
            }
        }

        if has_root && result.as_os_str().is_empty() {
            result.push(Component::RootDir);
        }

        Ok(result)
    }

    pub(in crate::path) fn normalize(path: &Path) -> Result<PathBuf> {
        let expanded = expand_tilde(path)?;
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            let cwd = env::current_dir().map_err(|e| Error::InvalidPath {
                path: path.to_path_buf(),
                reason: format!("Cannot get current directory: {e}"),
            })?;
            cwd.join(expanded)
        };
        resolve_components(&absolute)
    }

    pub(in crate::path) fn between(path1: &Path, path2: &Path) -> PathRelationship {
        let p1: PathBuf = path1.components().collect();
        let p2: PathBuf = path2.components().collect();
        if p1 == p2 {
            return PathRelationship::Same;
        }
        if p2.starts_with(&p1) {
            return PathRelationship::Ancestor;
        }
        if p1.starts_with(&p2) {
            return PathRelationship::Descendant;
        }
        PathRelationship::Unrelated
    }

    pub(in crate::path) fn canonicalize_existing(
        path: &Path,
    ) -> Result<(PathBuf, Option<PathBuf>)> {
        if let Ok(canonical) = crate::path::canonicalize::canonicalize(path) {
            return Ok((canonical, None));
        }
        let mut current = path.to_path_buf();
        let mut non_existent = Vec::new();
        loop {
            if current.exists() {
                let canonical = crate::path::canonicalize::canonicalize(&current)?;
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
                    return Err(Error::InvalidPath {
                        path: path.to_path_buf(),
                        reason: "Cannot find any existing portion of path".to_string(),
                    });
                }
            }
        }
    }
}

/// Every path built from up to `max_len` of the given pieces, joined with
/// `/`, both as written and with a leading `/`.
pub(super) fn generated_paths(pieces: &[&str], max_len: usize) -> Vec<String> {
    let mut layer: Vec<Vec<&str>> = vec![Vec::new()];
    let mut out = Vec::new();
    for _ in 0..=max_len {
        let mut next = Vec::new();
        for parts in &layer {
            let joined = parts.join("/");
            out.push(joined.clone());
            out.push(format!("/{joined}"));
            for piece in pieces {
                let mut longer = parts.clone();
                longer.push(piece);
                next.push(longer);
            }
        }
        layer = next;
    }
    out.sort();
    out.dedup();
    out
}

/// Every string of length at most `max_len` over `alphabet`.
// Its remaining users are Unix-only; the stored-path tests that also used it
// moved to `database::operations` with the decoder they check.
#[cfg(unix)]
fn generated_strings(alphabet: &[char], max_len: usize) -> Vec<String> {
    let mut layer = vec![String::new()];
    let mut out = Vec::new();
    for _ in 0..=max_len {
        let mut next = Vec::new();
        for s in &layer {
            out.push(s.clone());
            for c in alphabet {
                next.push(format!("{s}{c}"));
            }
        }
        layer = next;
    }
    out
}

fn same_outcome(new: &crate::Result<PathBuf>, old: &crate::Result<PathBuf>) -> bool {
    match (new, old) {
        (Ok(a), Ok(b)) => a.as_os_str() == b.as_os_str(),
        (Err(a), Err(b)) => a.to_string() == b.to_string(),
        _ => false,
    }
}

/// Tilde forms whose expansion this package deliberately changed: `~`
/// followed by two or more separators (the legacy code joined an absolute
/// remainder, which replaced the home directory), and, where backslash is not
/// a separator, `~\` (no longer home-relative). The separators are those
/// `expand_tilde` uses: `/`, plus `\` on Windows.
fn is_changed_tilde_form(path: &str) -> bool {
    let backslash_is_sep = cfg!(windows);
    let is_sep = |byte: u8| byte == b'/' || (backslash_is_sep && byte == b'\\');
    match path.as_bytes() {
        [b'~', b'\\', ..] if !backslash_is_sep => true,
        [b'~', first, second, ..] => is_sep(*first) && is_sep(*second),
        _ => false,
    }
}

/// The leading root/prefix components of a path (its anchor).
fn anchor(path: &Path) -> Vec<Component<'_>> {
    path.components()
        .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        .collect()
}

// ---------------------------------------------------------------------------
// Regression tests for the bugs fixed with the kernels
// ---------------------------------------------------------------------------

// Regression: `~//etc/foo` used to expand to `/etc/foo` because joining an
// absolute remainder replaces the home directory.
#[test]
fn tilde_with_repeated_separators_stays_under_home() {
    let home = home::home_dir().unwrap();
    let expanded = expand_tilde(Path::new("~//etc/foo")).unwrap();
    assert_ne!(expanded, Path::new("/etc/foo"));
    assert!(expanded.starts_with(&home), "{}", expanded.display());
    assert_eq!(expanded, home.join("etc/foo"));

    let normalized = normalize(Path::new("~//etc/foo")).unwrap();
    assert_eq!(normalized, normalize(&home.join("etc/foo")).unwrap());

    assert_eq!(
        normalize(Path::new("~///")).unwrap(),
        normalize(&home).unwrap()
    );
}

// Regression: on Unix `~\x` used to be expanded to `$HOME/x`, although `\`
// is an ordinary filename byte there; it is now the relative name `~\x`.
#[test]
#[cfg(unix)]
fn tilde_backslash_is_a_literal_name_on_unix() {
    assert_eq!(expand_tilde(Path::new("~\\x")).unwrap(), Path::new("~\\x"));
    let cwd = env::current_dir().unwrap();
    assert_eq!(normalize(Path::new("~\\x")).unwrap(), cwd.join("~\\x"));
    // A backslash after a slash is part of the name under the home directory.
    let home = home::home_dir().unwrap();
    let backslash_name = String::from("\\x");
    assert_eq!(
        expand_tilde(Path::new("~/\\x")).unwrap(),
        home.join(&backslash_name)
    );
    // Other `~name` forms are still rejected.
    assert!(expand_tilde(Path::new("~user")).is_err());
    assert!(expand_tilde(Path::new("~a\\b")).is_err());
}

// ---------------------------------------------------------------------------
// Differential tests against the replaced implementations
// ---------------------------------------------------------------------------

#[test]
fn expand_tilde_matches_legacy_outside_the_fixed_forms() {
    let inputs = generated_paths(&["~", "", "a", ".", "..", "\\", "~a"], 3);
    for input in &inputs {
        let path = Path::new(input);
        if is_changed_tilde_form(input) {
            continue;
        }
        assert!(
            same_outcome(&expand_tilde(path), &legacy::expand_tilde(path)),
            "{input:?}"
        );
    }
}

#[test]
fn normalize_matches_legacy_outside_the_fixed_forms() {
    let inputs = generated_paths(&["~", "", "a", "b", ".", "..", "a.b"], 4);
    for input in &inputs {
        if is_changed_tilde_form(input) {
            continue;
        }
        let path = Path::new(input);
        let new = normalize(path);
        let old = legacy::normalize(path);
        assert!(same_outcome(&new, &old), "{input:?}: {new:?} vs {old:?}");
    }
}

#[test]
fn resolve_components_matches_legacy() {
    let inputs = generated_paths(&["", "a", "b", ".", ".."], 5);
    for input in &inputs {
        let path = Path::new(input);
        let new = resolve_components(path);
        let old = legacy::resolve_components(path);
        assert!(same_outcome(&new, &old), "{input:?}: {new:?} vs {old:?}");
        // Absolute input keeps its root (the legacy fallback is unreachable).
        if let Ok(resolved) = new {
            assert_eq!(resolved.is_absolute(), path.is_absolute(), "{input:?}");
        }
    }
}

#[test]
fn between_matches_legacy_without_dots_and_with_equal_anchors() {
    let inputs = generated_paths(&["", "a", "b", "ab"], 3);
    for left in &inputs {
        for right in &inputs {
            let (p1, p2) = (Path::new(left), Path::new(right));
            // Compare anchors rather than `is_absolute()`: on Windows `/a` is
            // rooted but not absolute.
            if anchor(p1) != anchor(p2)
                || left.split('/').any(|piece| piece == "." || piece == "..")
                || right.split('/').any(|piece| piece == "." || piece == "..")
            {
                continue;
            }
            assert_eq!(
                PathRelationship::between(p1, p2),
                legacy::between(p1, p2),
                "{left:?} vs {right:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Properties of the glue over exhaustive small inputs
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn normalize_output_is_canonical_and_idempotent() {
    let inputs = generated_paths(&["", "a", "b", ".", ".."], 5);
    let other_cwd = env::temp_dir();
    for input in &inputs {
        if let Ok(normalized) = normalize(Path::new(input)) {
            assert!(is_canonical_absolute(&normalized), "{input:?}");
            assert_eq!(normalize(&normalized).unwrap(), normalized, "{input:?}");
            // Absolute input does not depend on the working directory, and
            // relative input is resolved against it.
            if Path::new(input).is_absolute() {
                assert_eq!(
                    crate::verified::path::normalize_unix(
                        input.as_bytes(),
                        other_cwd.to_str().unwrap().as_bytes()
                    )
                    .unwrap(),
                    normalized.to_str().unwrap().as_bytes()
                );
            }
        }
    }
}

#[test]
#[cfg(unix)]
fn canonical_recognizer_accepts_exactly_the_fixed_points_of_normalize() {
    for input in generated_strings(&['/', '.', 'a', '\\'], 7) {
        let path = Path::new(&input);
        let fixed_point = path.is_absolute()
            && normalize(path).is_ok_and(|normalized| normalized.as_os_str() == path.as_os_str());
        assert_eq!(is_canonical_absolute(path), fixed_point, "{input:?}");
        assert_eq!(
            NormalizedPath::new(path.to_path_buf(), PathProvenance::Explicit).is_ok(),
            fixed_point,
            "{input:?}"
        );
    }
}

#[test]
fn between_is_a_hierarchy_relation_on_small_paths() {
    let inputs = generated_paths(&["", "a", "b", ".", ".."], 2);
    for a in &inputs {
        let pa = Path::new(a);
        assert_eq!(PathRelationship::between(pa, pa), PathRelationship::Same);
        for b in &inputs {
            let pb = Path::new(b);
            let forward = PathRelationship::between(pa, pb);
            let backward = PathRelationship::between(pb, pa);
            let flipped = match backward {
                PathRelationship::Ancestor => PathRelationship::Descendant,
                PathRelationship::Descendant => PathRelationship::Ancestor,
                other => other,
            };
            assert_eq!(forward, flipped, "{a:?} vs {b:?}");
            assert_eq!(
                PathRelationship::is_within(pa, pb),
                PathRelationship::contains(pb, pa)
            );
            // The within/contains kernels agree with `between`.
            assert_eq!(
                PathRelationship::is_within(pa, pb),
                matches!(
                    forward,
                    PathRelationship::Descendant | PathRelationship::Same
                ),
                "{a:?} within {b:?}"
            );
            assert_eq!(
                PathRelationship::contains(pa, pb),
                matches!(forward, PathRelationship::Ancestor | PathRelationship::Same),
                "{a:?} contains {b:?}"
            );
        }
    }
}

// `between` never treats a `..` it cannot resolve as a named child: a path is
// an ancestor of another only if the other one, resolved, extends it by
// named components. Checked here against `normalize`'s resolution, which
// rejects exactly the paths with such a `..`.
#[test]
fn between_ancestors_extend_by_names_on_small_paths() {
    let inputs = generated_paths(&["", "a", "b", ".", ".."], 3);
    for a in &inputs {
        for b in &inputs {
            let (pa, pb) = (Path::new(a), Path::new(b));
            if PathRelationship::between(pa, pb) != PathRelationship::Ancestor {
                continue;
            }
            match (resolve_components(pa), resolve_components(pb)) {
                (Ok(ra), Ok(rb)) => {
                    assert!(rb.starts_with(&ra), "{a:?} vs {b:?}");
                    assert!(
                        rb.strip_prefix(&ra)
                            .unwrap()
                            .components()
                            .all(|c| matches!(c, Component::Normal(_))),
                        "{a:?} vs {b:?}"
                    );
                    assert_ne!(ra, rb, "{a:?} vs {b:?}");
                }
                // Both contain the same unresolvable `..` (equal heads).
                (Err(_), Err(_)) => {}
                (left, right) => panic!("{a:?} vs {b:?}: {left:?} vs {right:?}"),
            }
        }
    }
}

// The kernel's `tilde_join` agrees with `PathBuf::push` of a relative path
// (the trusted model behind `expand_tilde` on Unix) for unusual home
// directories, independently of the real `$HOME`.
#[test]
#[cfg(unix)]
fn tilde_join_matches_pathbuf_push() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let homes = [
        "", "/", "//", "/h", "/h/", "/h//", "h", "h/", "/h/.", "/h/..", "/h\\", "\\", "/h/x y",
    ];
    let rests = [
        "", "a", "a/", "a//b", ".", "..", "\\", " ", "\\a", "a\\", "a/./b", "~", "C:x",
    ];
    for home in homes {
        for rest in rests {
            let expected = Path::new(home).join(rest);
            let joined = crate::verified::path::tilde_join(home.as_bytes(), rest.as_bytes());
            assert_eq!(
                OsStr::from_bytes(&joined),
                expected.as_os_str(),
                "{home:?} + {rest:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// canonicalize_existing
// ---------------------------------------------------------------------------

#[test]
fn canonicalize_existing_keeps_the_remainder_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir(dir.path().join("present")).unwrap();

    let full = dir.path().join("present").join("x").join("y").join("z");
    let (canonical, remainder) = canonicalize_existing(&full).unwrap();
    assert_eq!(canonical, base.join("present"));
    assert_eq!(remainder, Some(PathBuf::from("x/y/z")));
    assert_eq!(
        canonicalize_existing(&full).unwrap(),
        legacy::canonicalize_existing(&full).unwrap()
    );
}

#[test]
fn canonicalize_existing_matches_legacy_on_edge_cases() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file"), "x").unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let cases = [
        format!("{root}/missing"),
        format!("{root}/missing/"),
        format!("{root}/missing/./deeper"),
        format!("{root}//missing//deeper/"),
        format!("{root}/file/"),
        format!("{root}/file/below"),
        format!("{root}/missing/.."),
        format!("{root}/missing/../other"),
        "relative-missing/x".to_string(),
        String::new(),
    ];
    for case in &cases {
        let path = Path::new(case);
        let new = canonicalize_existing(path);
        let old = legacy::canonicalize_existing(path);
        match (&new, &old) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "{case:?}"),
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{case:?}"),
            _ => panic!("{case:?}: {new:?} vs {old:?}"),
        }
    }
}

#[test]
fn canonicalize_existing_cannot_step_over_parent_components() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing").join("..");
    assert!(matches!(
        canonicalize_existing(&path),
        Err(Error::InvalidPath { .. })
    ));
    assert!(matches!(
        canonicalize_existing(Path::new("")),
        Err(Error::InvalidPath { .. })
    ));
}
