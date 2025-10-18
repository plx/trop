//! Git-based inference of project and task names.
//!
//! This module provides automatic inference of project and task identifiers
//! from git repository context, supporting both regular repositories and
//! git worktrees.

use gix::bstr::ByteSlice;
use std::path::Path;

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
    // Use gix to discover repository - returns (Path, Trust)
    let (repo_path, _trust) = gix::discover::upwards(path).ok()?;
    let std_path: &Path = repo_path.as_ref();

    // Open repo to check if it's a worktree
    let repo = gix::open(std_path).ok()?;

    // Get the common dir (main repo location for worktrees, same as git_dir for regular repos)
    let common_dir = repo.common_dir();

    // Canonicalize the common dir to resolve any .. components (important for worktrees)
    // For worktrees, common_dir might be something like ".git/worktrees/name/../.."
    // which needs to be resolved to the actual main repository path
    let canonical_common = common_dir.canonicalize().ok()?;

    // Extract directory name from the common dir's parent
    canonical_common
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(String::from)
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
    // Discover repository - returns (Path, Trust)
    let (repo_path, _trust) = gix::discover::upwards(path).ok()?;

    // Open the repository to get more information
    let std_path: &Path = repo_path.as_ref();
    let repo = gix::open(std_path).ok()?;

    // Check if this is a worktree
    if is_worktree(&repo) {
        // Use worktree directory name
        extract_worktree_name(&repo)
    } else {
        // Use current branch name
        get_current_branch(&repo)
    }
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
    repo.work_dir()
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
    repo.work_dir()
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
            s.strip_prefix("refs/heads/").map(String::from)
        } else {
            log::debug!("Branch name contains non-UTF8 characters: {bstr:?}");
            None
        }
    })
}
