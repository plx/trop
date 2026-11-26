//! Git-based inference of project and task names.
//!
//! This module provides automatic inference of project and task identifiers
//! from git repository context, supporting both regular repositories and
//! git worktrees.

use gix::bstr::ByteSlice;
use std::ffi::OsStr;
use std::path::Path;

use crate::verified::inference::{self as kernel, NameSource};

// Counts git discoveries made by this thread (test instrumentation: it
// shows that explicit metadata skips inference).
#[cfg(test)]
thread_local! {
    pub(crate) static DISCOVERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Discovers the repository containing `path` and opens it.
fn discover(path: &Path) -> Option<gix::Repository> {
    #[cfg(test)]
    DISCOVERIES.with(|count| count.set(count.get() + 1));
    // Use gix to discover repository - returns (Path, Trust)
    let (repo_path, _trust) = gix::discover::upwards(path).ok()?;
    let std_path: &Path = repo_path.as_ref();
    gix::open(std_path).ok()
}

/// An inferred project or task name in canonical form, or `None` when it is
/// not a valid identifier. The verified identifier kernel both trims
/// (Unicode `White_Space`) and applies the identifier rule (non-blank, no
/// NUL, at most 255 UTF-8 bytes after trimming); the value kept is the
/// kernel's trimmed sub-slice, so it is canonical.
fn normalize_inferred_identifier(value: &str) -> Option<String> {
    crate::identifier::check_identifier(value)
        .ok()
        .map(str::to_owned)
}

/// Infer project name from git repository.
///
/// Returns the repository name extracted from the git directory.
/// This works for both regular repositories and worktrees by examining
/// the common directory (main repository location).
///
/// # Arguments
///
/// * `path` - Path to search upward from for a git repository
///
/// # Returns
///
/// - `Some(String)` containing the repository name if found
/// - `None` if no git repository found or extraction fails
///
/// # Examples
///
/// ```no_run
/// use trop::operations::inference::infer_project;
/// use std::path::Path;
///
/// if let Some(project) = infer_project(Path::new("/home/user/myrepo/src")) {
///     println!("Project: {}", project);
/// }
/// ```
#[must_use]
pub fn infer_project(path: &Path) -> Option<String> {
    // Open the repository (a worktree's common dir is the main repository).
    let repo = discover(path)?;

    // Get the common dir (main repo location for worktrees, same as git_dir for regular repos)
    let common_dir = repo.common_dir();

    // Canonicalize the common dir to resolve any .. components (important for worktrees)
    // For worktrees, common_dir might be something like ".git/worktrees/name/../.."
    // which needs to be resolved to the actual main repository path
    let canonical_common = common_dir.canonicalize().ok()?;

    let common_name = canonical_common.file_name()?.to_str()?;
    let parent_name = canonical_common
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);

    // Discard any candidate that is not a valid identifier.
    normalize_inferred_identifier(repository_name(common_name, parent_name)?)
}

/// The repository name for a canonical common directory whose last
/// component is `common_name` and whose parent's is `parent_name`.
///
/// The verified kernel picks the name - the parent's when the common
/// directory is `.git`, the common directory's own for a bare repository -
/// and the length left after stripping one `.git` suffix, so
/// `/path/to/repo.git` identifies project `repo`. The length sits next to an
/// ASCII byte, so it is a character boundary.
fn repository_name<'a>(common_name: &'a str, parent_name: Option<&'a str>) -> Option<&'a str> {
    let name =
        kernel::repository_name_prefix(common_name.as_bytes(), parent_name.map(str::as_bytes))?;
    let selected = match name.source {
        NameSource::CommonDirectory => common_name,
        NameSource::ParentDirectory => parent_name?,
    };
    selected.get(..name.len)
}

/// The branch a HEAD reference names: the reference without `refs/heads/`
/// (slashes inside the name kept), or `None` for any other reference. The
/// verified kernel finds where the name starts.
fn branch_name(head_ref: &str) -> Option<&str> {
    let start = kernel::branch_name_start(head_ref.as_bytes())?;
    head_ref.get(start..)
}

/// Infer task from git context.
///
/// Determines the appropriate task name based on git context:
/// - In a worktree: uses the worktree directory name
/// - In a regular repo: uses the current branch name
/// - Otherwise: returns None
///
/// # Arguments
///
/// * `path` - Path to search upward from for a git repository
///
/// # Returns
///
/// - `Some(String)` containing the task name if determined
/// - `None` if:
///   - No git repository is found
///   - HEAD is detached (not on a branch)
///   - Branch/worktree name contains non-UTF8 characters
///   - Repository state cannot be determined
///
/// # Examples
///
/// ```no_run
/// use trop::operations::inference::infer_task;
/// use std::path::Path;
///
/// if let Some(task) = infer_task(Path::new("/home/user/myrepo")) {
///     println!("Task: {}", task);
/// }
/// ```
#[must_use]
pub fn infer_task(path: &Path) -> Option<String> {
    // Discover and open the repository.
    let repo = discover(path)?;

    // Check if this is a worktree
    let task = if is_worktree(&repo) {
        // Use worktree directory name
        extract_worktree_name(&repo)
    } else {
        // Use current branch name
        get_current_branch(&repo)
    }?;

    normalize_inferred_identifier(&task)
}

/// Check if the repository is a git worktree.
///
/// Worktrees have a `.git` file instead of a `.git` directory.
///
/// # Arguments
///
/// * `repo` - The repository to check
///
/// # Returns
///
/// `true` if this is a worktree, `false` otherwise
fn is_worktree(repo: &gix::Repository) -> bool {
    // Check if .git is a file (indicates worktree)
    repo.workdir()
        .and_then(|wd| wd.join(".git").metadata().ok())
        .is_some_and(|m| m.is_file())
}

/// Extract the worktree name from the repository.
///
/// The worktree name is taken from the working directory's file name.
///
/// # Arguments
///
/// * `repo` - The repository to extract from
///
/// # Returns
///
/// - `Some(String)` containing the worktree directory name
/// - `None` if extraction fails
fn extract_worktree_name(repo: &gix::Repository) -> Option<String> {
    repo.workdir()
        .and_then(|wd| wd.file_name())
        .and_then(|n| n.to_str())
        .map(std::string::ToString::to_string)
}

/// Get the current branch name from the repository.
///
/// Extracts the branch name from the HEAD reference, stripping the
/// "refs/heads/" prefix if present.
///
/// # Arguments
///
/// * `repo` - The repository to query
///
/// # Returns
///
/// - `Some(String)` containing the branch name
/// - `None` if HEAD is detached or extraction fails
fn get_current_branch(repo: &gix::Repository) -> Option<String> {
    repo.head_name().ok().flatten().and_then(|name| {
        let bstr = name.as_bstr();
        if let Ok(s) = bstr.to_str() {
            branch_name(s).map(String::from)
        } else {
            log::debug!("Branch name contains non-UTF8 characters: {bstr:?}");
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_git_project_inference_is_best_effort() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(infer_project(directory.path()), None);
    }

    #[test]
    fn non_git_task_inference_is_best_effort() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(infer_task(directory.path()), None);
    }

    #[test]
    fn repository_names_follow_the_git_directory_rules() {
        // A `.git` common directory names its parent.
        assert_eq!(repository_name(".git", Some("repo")), Some("repo"));
        assert_eq!(repository_name(".git", Some("repo.git")), Some("repo"));
        // Exactly one suffix is stripped.
        assert_eq!(
            repository_name(".git", Some("repo.git.git")),
            Some("repo.git")
        );
        assert_eq!(repository_name("repo.git.git", None), Some("repo.git"));
        // A bare repository names itself; its parent is irrelevant.
        assert_eq!(repository_name("repo.git", Some("ignored")), Some("repo"));
        assert_eq!(repository_name("repo", None), Some("repo"));
        // No usable parent name: no project.
        assert_eq!(repository_name(".git", None), None);
        // Suffix-only and short names.
        assert_eq!(repository_name(".git", Some(".git")), Some(""));
        assert_eq!(repository_name("git", None), Some("git"));
        assert_eq!(repository_name("", None), Some(""));
        assert_eq!(repository_name("x.GIT", None), Some("x.GIT"));
        // Non-ASCII names are cut on a character boundary.
        assert_eq!(repository_name("répo.git", None), Some("répo"));
        assert_eq!(repository_name(".git", Some("仓库.git")), Some("仓库"));
        assert_eq!(repository_name("é", None), Some("é"));
    }

    #[test]
    fn branch_names_keep_slashes_and_reject_other_refs() {
        assert_eq!(branch_name("refs/heads/main"), Some("main"));
        assert_eq!(branch_name("refs/heads/feature/x"), Some("feature/x"));
        assert_eq!(branch_name("refs/heads/é/ü"), Some("é/ü"));
        assert_eq!(branch_name("refs/heads/"), Some(""));
        assert_eq!(branch_name("refs/heads"), None);
        assert_eq!(branch_name("refs/tags/v1"), None);
        assert_eq!(branch_name("refs/remotes/origin/main"), None);
        assert_eq!(branch_name("HEAD"), None);
        assert_eq!(branch_name(""), None);
        assert_eq!(branch_name("REFS/HEADS/main"), None);
    }

    #[test]
    fn invalid_inferred_identifiers_are_ignored() {
        assert_eq!(normalize_inferred_identifier("   "), None);
        assert_eq!(normalize_inferred_identifier(""), None);
        assert_eq!(normalize_inferred_identifier(&"x".repeat(256)), None);
        assert_eq!(normalize_inferred_identifier("a\0b"), None);
        assert_eq!(
            normalize_inferred_identifier("  valid-project  "),
            Some("valid-project".to_string())
        );
        // The 255-byte limit applies after trimming.
        let longest = "x".repeat(255);
        assert_eq!(
            normalize_inferred_identifier(&format!(" {longest}\u{3000}")),
            Some(longest)
        );
    }

    /// The inferred name is trimmed by the verified kernel. Pins that this is
    /// exactly the previous `str::trim` + rule behavior for every Unicode
    /// scalar value used as padding, and that the result is canonical.
    #[test]
    fn inferred_identifier_trim_matches_str_trim() {
        for c in (0..=0x0010_FFFF_u32).filter_map(char::from_u32) {
            for text in [format!("{c}name{c}"), format!("{c}"), format!("a{c}b")] {
                let reference = Some(text.trim())
                    .filter(|value| {
                        !value.is_empty() && !value.contains('\0') && value.len() <= 255
                    })
                    .map(str::to_owned);
                let normalized = normalize_inferred_identifier(&text);
                assert_eq!(normalized, reference, "{text:?}");
                if let Some(value) = normalized {
                    assert!(crate::identifier::is_canonical_identifier(&value));
                }
            }
        }
    }
}
