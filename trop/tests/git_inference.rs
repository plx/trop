//! Comprehensive integration tests for Git-based project and task inference.
//!
//! This test suite validates the git integration feature that automatically
//! infers project and task names from git repository context. The tests cover:
//!
//! - Regular git repositories (project from repo name, task from branch)
//! - Git worktrees (project from main repo, task from worktree directory)
//! - Edge cases: detached HEAD, non-git directories, nested repos
//! - Integration with reserve operations
//!
//! These tests use real git repositories created in temporary directories to
//! ensure the gix library integration works correctly in realistic scenarios.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use trop::operations::inference::{infer_project, infer_task};
use trop::Database;

use trop::operations::ReserveOptions;

use trop::{Port, ReservationKey};

// ============================================================================
// Test Helper Functions
// ============================================================================

mod helpers {
    use super::*;

    /// Creates a minimal git repository in the specified directory.
    ///
    /// This helper initializes a git repo with:
    /// - Initial commit (required for branches to work)
    /// - Default branch (main)
    /// - Git config for test user
    ///
    /// # Arguments
    ///
    /// * `path` - Directory where the repository should be created
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the repository was created successfully
    /// - `Err(String)` with error details if git commands fail
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The directory doesn't exist
    /// - Git commands fail to execute
    /// - Git is not installed on the system
    pub fn create_test_repo(path: &Path) -> Result<(), String> {
        // Initialize the repository
        run_git(path, &["init"])?;

        // Configure git user for commits (required for initial commit)
        run_git(path, &["config", "user.name", "Test User"])?;
        run_git(path, &["config", "user.email", "test@example.com"])?;

        // Create initial commit (required for branches to work properly)
        // Without this, HEAD is unborn and branch detection fails
        std::fs::write(path.join("README.md"), "Test repository\n")
            .map_err(|e| format!("Failed to write README: {e}"))?;
        run_git(path, &["add", "README.md"])?;
        run_git(path, &["commit", "-m", "Initial commit"])?;

        Ok(())
    }

    /// Creates a git worktree from an existing repository.
    ///
    /// Worktrees allow multiple working directories from a single git repository.
    /// This is useful for working on multiple branches simultaneously.
    ///
    /// # Arguments
    ///
    /// * `repo_path` - Path to the main git repository
    /// * `worktree_path` - Path where the worktree should be created
    /// * `branch` - Name of the branch for the worktree (will be created)
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the worktree was created successfully
    /// - `Err(String)` with error details if git commands fail
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The main repository doesn't exist or is invalid
    /// - The worktree path already exists
    /// - Git commands fail to execute
    pub fn create_worktree(
        repo_path: &Path,
        worktree_path: &Path,
        branch: &str,
    ) -> Result<(), String> {
        // Git worktree command creates a new working directory linked to the main repo
        // The -b flag creates a new branch for the worktree
        run_git(
            repo_path,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                worktree_path.to_str().unwrap(),
            ],
        )?;

        Ok(())
    }

    /// Creates a detached HEAD state in a git repository.
    ///
    /// Detached HEAD means the repository is not on any branch - instead it's
    /// pointing directly to a specific commit. This is a common state when
    /// checking out tags or specific commits.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the git repository
    ///
    /// # Returns
    ///
    /// - `Ok(())` if HEAD was successfully detached
    /// - `Err(String)` with error details if git commands fail
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The repository doesn't exist or is invalid
    /// - Git commands fail to execute
    pub fn detach_head(path: &Path) -> Result<(), String> {
        // Get the current commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(path)
            .output()
            .map_err(|e| format!("Failed to get HEAD commit: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "git rev-parse failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Checkout the commit directly (detaches HEAD)
        run_git(path, &["checkout", &commit])?;

        Ok(())
    }

    /// Switches to a specific branch in a git repository.
    ///
    /// Creates the branch if it doesn't exist, or switches to it if it does.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the git repository
    /// * `branch` - Name of the branch to switch to
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the branch was switched successfully
    /// - `Err(String)` with error details if git commands fail
    pub fn switch_branch(path: &Path, branch: &str) -> Result<(), String> {
        // Try to switch to the branch, creating it if it doesn't exist
        run_git(path, &["checkout", "-b", branch]).or_else(|_| {
            // If the branch exists, this will fail, so just switch to it
            run_git(path, &["checkout", branch])
        })
    }

    /// Executes a git command in the specified directory.
    ///
    /// This is a low-level helper used by other git operations. It captures
    /// both stdout and stderr, and returns an error if the command fails.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory where the git command should be executed
    /// * `args` - Git command arguments (without the "git" prefix)
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the command executed successfully (exit code 0)
    /// - `Err(String)` with stderr output if the command failed
    fn run_git(path: &Path, args: &[&str]) -> Result<(), String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .map_err(|e| format!("Failed to execute git: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(())
    }
}

// ============================================================================
// Tests for infer_project()
// ============================================================================

mod infer_project_tests {
    use super::*;

    /// Tests that project name is correctly inferred from a regular git repository.
    ///
    /// SEMANTIC INVARIANT: For a regular git repository, the project name should be
    /// extracted from the repository's directory name. This provides a sensible
    /// default that matches common project organization patterns.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository in a directory named "my-project"
    /// - Call infer_project() from within that directory
    /// - Verify it returns "my-project"
    ///
    /// WHY THIS MATTERS: Project inference allows users to avoid manually specifying
    /// project names when working within a git repository. The project name should
    /// reflect the repository's identity, which is typically its directory name.
    #[test]
    fn test_infer_project_from_regular_repo() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("my-project");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();

        // Test inference from the repository root
        let project = infer_project(&repo_path);
        assert_eq!(
            project,
            Some("my-project".to_string()),
            "Project name should be extracted from repository directory name"
        );
    }

    /// Tests that project inference works from subdirectories within the repository.
    ///
    /// SEMANTIC INVARIANT: Project inference should work from any subdirectory
    /// within a git repository, not just from the repository root. This is crucial
    /// for usability since users often work deep within project directory trees.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "my-project"
    /// - Create nested subdirectories (src/utils/)
    /// - Call infer_project() from the deeply nested directory
    /// - Verify it still returns "my-project"
    ///
    /// WHY THIS MATTERS: Git operations work from anywhere within a repository,
    /// and our inference should too. Users shouldn't need to be at the repo root.
    #[test]
    fn test_infer_project_from_subdirectory() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("my-project");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();

        // Create nested subdirectories
        let subdir = repo_path.join("src").join("utils");
        std::fs::create_dir_all(&subdir).unwrap();

        // Test inference from a deeply nested subdirectory
        let project = infer_project(&subdir);
        assert_eq!(
            project,
            Some("my-project".to_string()),
            "Project name should be inferred even from nested subdirectories"
        );
    }

    /// Tests that project inference works correctly for git worktrees.
    ///
    /// SEMANTIC INVARIANT: For a git worktree, the project name should come from
    /// the MAIN repository, not from the worktree directory. Worktrees are
    /// alternative working directories for the same project, so they should share
    /// the project identifier.
    ///
    /// TEST SCENARIO:
    /// - Create a main repository named "main-repo"
    /// - Create a worktree in a directory named "feature-worktree"
    /// - Call infer_project() from within the worktree
    /// - Verify it returns "main-repo", not "feature-worktree"
    ///
    /// WHY THIS MATTERS: Worktrees are part of the same logical project as the
    /// main repository. Reservations made in different worktrees of the same
    /// project should be grouped under the same project name.
    #[test]
    fn test_infer_project_from_worktree() {
        let temp = TempDir::new().unwrap();
        let main_repo = temp.path().join("main-repo");
        std::fs::create_dir(&main_repo).unwrap();

        helpers::create_test_repo(&main_repo).unwrap();

        // Create a worktree with a different directory name
        let worktree_path = temp.path().join("feature-worktree");
        helpers::create_worktree(&main_repo, &worktree_path, "feature-branch").unwrap();

        // Test inference from the worktree
        let project = infer_project(&worktree_path);
        assert_eq!(
            project,
            Some("main-repo".to_string()),
            "Worktree should infer project name from main repository, not worktree directory"
        );
    }

    /// Tests that project inference returns None for non-git directories.
    ///
    /// SEMANTIC INVARIANT: When called from a directory that is not part of any
    /// git repository, infer_project() should return None rather than guessing
    /// or returning an error. This allows callers to gracefully handle the absence
    /// of git context.
    ///
    /// TEST SCENARIO:
    /// - Create a regular directory (no git initialization)
    /// - Call infer_project() from that directory
    /// - Verify it returns None
    ///
    /// WHY THIS MATTERS: Not all projects use git, and trop should work in
    /// non-git contexts. Returning None allows callers to distinguish between
    /// "no git repo" and "git repo with inference failure".
    #[test]
    fn test_infer_project_non_git_directory() {
        let temp = TempDir::new().unwrap();
        let non_git_dir = temp.path().join("not-a-repo");
        std::fs::create_dir(&non_git_dir).unwrap();

        // Test inference from a non-git directory
        let project = infer_project(&non_git_dir);
        assert_eq!(
            project, None,
            "Non-git directories should return None for project inference"
        );
    }

    /// Tests that project inference handles repository names with special characters.
    ///
    /// SEMANTIC INVARIANT: Repository directory names can contain hyphens, dots,
    /// underscores, and other characters. The inference should preserve these
    /// exactly as they appear in the filesystem.
    ///
    /// TEST SCENARIO:
    /// - Create a repository with a complex name: "my-repo.v2_test"
    /// - Call infer_project() from within it
    /// - Verify it returns the exact name without modification
    ///
    /// WHY THIS MATTERS: Users may have existing naming conventions that include
    /// special characters. We should preserve their project names exactly.
    #[test]
    fn test_infer_project_with_special_chars() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("my-repo.v2_test");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();

        let project = infer_project(&repo_path);
        assert_eq!(
            project,
            Some("my-repo.v2_test".to_string()),
            "Project names with special characters should be preserved exactly"
        );
    }
}

// ============================================================================
// Tests for infer_task()
// ============================================================================

mod infer_task_tests {
    use super::*;

    /// Tests that task name is inferred from the current branch in a regular repo.
    ///
    /// SEMANTIC INVARIANT: For a regular git repository (not a worktree), the task
    /// name should be extracted from the current branch name. This reflects the
    /// common workflow where each branch represents a specific task or feature.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository
    /// - Switch to a branch named "feature-auth"
    /// - Call infer_task() from within the repository
    /// - Verify it returns "feature-auth"
    ///
    /// WHY THIS MATTERS: Branch names typically correspond to tasks or features
    /// (e.g., "feature-auth", "bugfix-123", "refactor-api"). Using the branch
    /// name as the task provides automatic, meaningful task identification.
    #[test]
    fn test_infer_task_from_branch() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "feature-auth").unwrap();

        let task = infer_task(&repo_path);
        assert_eq!(
            task,
            Some("feature-auth".to_string()),
            "Task should be inferred from current branch name in regular repo"
        );
    }

    /// Tests that task inference works from subdirectories within the repository.
    ///
    /// SEMANTIC INVARIANT: Like project inference, task inference should work
    /// from any subdirectory within the repository. The git context (current branch)
    /// is repository-wide, not directory-specific.
    ///
    /// TEST SCENARIO:
    /// - Create a repository on branch "feature-api"
    /// - Create a nested subdirectory
    /// - Call infer_task() from the subdirectory
    /// - Verify it returns "feature-api"
    ///
    /// WHY THIS MATTERS: Users work in various subdirectories within a project.
    /// The current branch (and thus task) should be accessible from anywhere.
    #[test]
    fn test_infer_task_from_branch_in_subdirectory() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "feature-api").unwrap();

        // Create a subdirectory
        let subdir = repo_path.join("src");
        std::fs::create_dir(&subdir).unwrap();

        let task = infer_task(&subdir);
        assert_eq!(
            task,
            Some("feature-api".to_string()),
            "Task should be inferred from branch even in subdirectories"
        );
    }

    /// Tests that task inference returns None when HEAD is detached.
    ///
    /// SEMANTIC INVARIANT: When the repository is in a detached HEAD state (not
    /// on any branch), there is no meaningful task name to infer. The function
    /// should return None rather than trying to use the commit hash or other
    /// non-semantic identifier.
    ///
    /// TEST SCENARIO:
    /// - Create a repository with a commit
    /// - Detach HEAD (checkout a specific commit, not a branch)
    /// - Call infer_task()
    /// - Verify it returns None
    ///
    /// WHY THIS MATTERS: Detached HEAD is a temporary state, often used during
    /// rebase, bisect, or tag checkout. There's no stable task identifier in
    /// this state, so returning None is the correct behavior.
    #[test]
    fn test_infer_task_detached_head() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::detach_head(&repo_path).unwrap();

        let task = infer_task(&repo_path);
        assert_eq!(
            task, None,
            "Task inference should return None for detached HEAD state"
        );
    }

    /// Tests that task is inferred from worktree directory name, not branch.
    ///
    /// SEMANTIC INVARIANT: For git worktrees, the task name should come from the
    /// worktree's directory name, not the branch name. This is because worktrees
    /// are often created with meaningful directory names that represent the task,
    /// and multiple worktrees can exist on the same branch with different purposes.
    ///
    /// TEST SCENARIO:
    /// - Create a main repository
    /// - Create a worktree with branch "feature-x" in directory "bugfix-worktree"
    /// - Call infer_task() from the worktree
    /// - Verify it returns "bugfix-worktree", not "feature-x"
    ///
    /// WHY THIS MATTERS: Worktrees are created with specific directory names that
    /// often better represent the current task than the branch name. For example,
    /// you might have worktrees "review-pr-123" and "testing-main" both on the
    /// main branch but serving different purposes.
    #[test]
    fn test_infer_task_from_worktree() {
        let temp = TempDir::new().unwrap();
        let main_repo = temp.path().join("main-repo");
        std::fs::create_dir(&main_repo).unwrap();

        helpers::create_test_repo(&main_repo).unwrap();

        // Create a worktree with a specific directory name and branch
        let worktree_path = temp.path().join("bugfix-worktree");
        helpers::create_worktree(&main_repo, &worktree_path, "feature-branch").unwrap();

        let task = infer_task(&worktree_path);
        assert_eq!(
            task,
            Some("bugfix-worktree".to_string()),
            "Task should be inferred from worktree directory name, not branch name"
        );
    }

    /// Tests that task inference returns None for non-git directories.
    ///
    /// SEMANTIC INVARIANT: When called from a directory that is not part of any
    /// git repository, infer_task() should return None. This mirrors the behavior
    /// of infer_project() and allows graceful handling of non-git contexts.
    ///
    /// TEST SCENARIO:
    /// - Create a regular directory (no git initialization)
    /// - Call infer_task() from that directory
    /// - Verify it returns None
    ///
    /// WHY THIS MATTERS: Trop should work in non-git contexts. Returning None
    /// allows the application to proceed without git-inferred metadata.
    #[test]
    fn test_infer_task_non_git_directory() {
        let temp = TempDir::new().unwrap();
        let non_git_dir = temp.path().join("not-a-repo");
        std::fs::create_dir(&non_git_dir).unwrap();

        let task = infer_task(&non_git_dir);
        assert_eq!(
            task, None,
            "Non-git directories should return None for task inference"
        );
    }

    /// Tests that task inference handles branch names with slashes (e.g., "feature/auth").
    ///
    /// SEMANTIC INVARIANT: Git allows branch names to contain slashes, creating
    /// a hierarchical naming scheme (e.g., "feature/auth", "bugfix/issue-123").
    /// The inference should preserve the full branch name, including slashes.
    ///
    /// TEST SCENARIO:
    /// - Create a repository
    /// - Switch to a branch with slashes: "feature/user-auth"
    /// - Call infer_task()
    /// - Verify it returns the full branch name with slashes preserved
    ///
    /// WHY THIS MATTERS: Many teams use hierarchical branch naming conventions.
    /// Preserving the slashes maintains the semantic meaning of the branch name.
    #[test]
    fn test_infer_task_branch_with_slashes() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "feature/user-auth").unwrap();

        let task = infer_task(&repo_path);
        assert_eq!(
            task,
            Some("feature/user-auth".to_string()),
            "Branch names with slashes should be preserved in task inference"
        );
    }

    /// Tests that task inference handles the default branch name correctly.
    ///
    /// SEMANTIC INVARIANT: The default branch (typically "main" or "master")
    /// should be returned as a valid task name, even though it might not
    /// represent a specific feature. This allows users working on the default
    /// branch to still have automatic task identification.
    ///
    /// TEST SCENARIO:
    /// - Create a repository (which defaults to "main" branch after first commit)
    /// - Don't switch to any other branch
    /// - Call infer_task()
    /// - Verify it returns "main"
    ///
    /// WHY THIS MATTERS: Users often work directly on the main branch for small
    /// changes or in simple projects. The task inference should work even in
    /// this common scenario.
    #[test]
    fn test_infer_task_default_branch() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        // Don't switch branches - stay on default "main" or "master"

        let task = infer_task(&repo_path);
        // The task should be the default branch name
        assert!(
            task.is_some(),
            "Default branch should be inferred as a valid task"
        );
        // Common default branch names
        let task_name = task.unwrap();
        assert!(
            task_name == "main" || task_name == "master",
            "Default branch should be 'main' or 'master', got: {task_name}"
        );
    }
}

// ============================================================================
// Integration Tests: with_git_inference()
// ============================================================================

mod integration_tests {
    use super::*;

    /// Tests that with_git_inference() only sets fields that are currently None.
    ///
    /// SEMANTIC INVARIANT: Git inference is a fallback mechanism - it should
    /// only provide values when the user hasn't explicitly specified them.
    /// Explicit user-provided values must always take precedence over inferred
    /// values.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "repo-project" on branch "repo-branch"
    /// - Create ReserveOptions with explicit project "my-project" and task "my-task"
    /// - Call with_git_inference()
    /// - Verify the explicit values are preserved, not overwritten by git inference
    ///
    /// WHY THIS MATTERS: Users need the ability to override git-based inference.
    /// For example, they might want to group multiple repos under one project name,
    /// or use a task name that differs from the branch name. Explicit values must
    /// always win.
    #[test]
    fn test_with_git_inference_preserves_explicit_values() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("repo-project");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "repo-branch").unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create options with EXPLICIT project and task
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("my-project".to_string()))
            .with_task(Some("my-task".to_string()))
            .with_git_inference(&repo_path);

        // Verify explicit values are preserved
        assert_eq!(
            options.project,
            Some("my-project".to_string()),
            "Explicit project value must be preserved by git inference"
        );
        assert_eq!(
            options.task,
            Some("my-task".to_string()),
            "Explicit task value must be preserved by git inference"
        );
    }

    /// Tests that with_git_inference() infers values when fields are None.
    ///
    /// SEMANTIC INVARIANT: When project and task are not explicitly specified
    /// (i.e., they are None), with_git_inference() should populate them from
    /// git context if available. This is the primary purpose of the inference
    /// feature.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "inferred-project" on branch "inferred-task"
    /// - Create ReserveOptions with project and task set to None
    /// - Call with_git_inference()
    /// - Verify project is set to "inferred-project" and task to "inferred-task"
    ///
    /// WHY THIS MATTERS: This is the main use case - automatic population of
    /// project and task from git context, saving users from manual specification.
    #[test]
    fn test_with_git_inference_infers_when_none() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("inferred-project");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "inferred-task").unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Create options with NO explicit project or task
        let options = ReserveOptions::new(key, Some(port)).with_git_inference(&repo_path);

        // Verify values were inferred from git
        assert_eq!(
            options.project,
            Some("inferred-project".to_string()),
            "Project should be inferred from repository name when not explicitly set"
        );
        assert_eq!(
            options.task,
            Some("inferred-task".to_string()),
            "Task should be inferred from branch name when not explicitly set"
        );
    }

    /// Tests partial inference: explicit project, inferred task.
    ///
    /// SEMANTIC INVARIANT: Project and task inference should be independent -
    /// one can be explicitly set while the other is inferred. This allows users
    /// to mix explicit and inferred values as needed.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository on branch "feature-branch"
    /// - Create ReserveOptions with explicit project but no task
    /// - Call with_git_inference()
    /// - Verify explicit project is preserved and task is inferred from git
    ///
    /// WHY THIS MATTERS: Users might want to override the project name (e.g., to
    /// group multiple repos) while still benefiting from automatic task inference
    /// based on the current branch.
    #[test]
    fn test_with_git_inference_partial_explicit_project() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("some-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "feature-branch").unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // Explicit project, no task
        let options = ReserveOptions::new(key, Some(port))
            .with_project(Some("explicit-project".to_string()))
            .with_git_inference(&repo_path);

        assert_eq!(
            options.project,
            Some("explicit-project".to_string()),
            "Explicit project should be preserved"
        );
        assert_eq!(
            options.task,
            Some("feature-branch".to_string()),
            "Task should be inferred when not explicitly set"
        );
    }

    /// Tests partial inference: inferred project, explicit task.
    ///
    /// SEMANTIC INVARIANT: Similar to the previous test, but in the opposite
    /// direction - project is inferred while task is explicit. This demonstrates
    /// the symmetry and independence of the two inference mechanisms.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "inferred-repo"
    /// - Create ReserveOptions with explicit task but no project
    /// - Call with_git_inference()
    /// - Verify project is inferred from git and explicit task is preserved
    ///
    /// WHY THIS MATTERS: Users might want to use a specific task name (e.g., a
    /// ticket number) while still benefiting from automatic project inference
    /// based on the repository name.
    #[test]
    fn test_with_git_inference_partial_explicit_task() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("inferred-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::switch_branch(&repo_path, "some-branch").unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        // No project, explicit task
        let options = ReserveOptions::new(key, Some(port))
            .with_task(Some("explicit-task".to_string()))
            .with_git_inference(&repo_path);

        assert_eq!(
            options.project,
            Some("inferred-repo".to_string()),
            "Project should be inferred when not explicitly set"
        );
        assert_eq!(
            options.task,
            Some("explicit-task".to_string()),
            "Explicit task should be preserved"
        );
    }

    /// Tests that with_git_inference() gracefully handles non-git directories.
    ///
    /// SEMANTIC INVARIANT: When called from a non-git directory, with_git_inference()
    /// should not fail or error. Instead, it should simply leave project and task
    /// as None if they weren't explicitly set. This allows the same code path to
    /// work in both git and non-git contexts.
    ///
    /// TEST SCENARIO:
    /// - Create a regular directory (no git initialization)
    /// - Create ReserveOptions with no explicit project/task
    /// - Call with_git_inference()
    /// - Verify project and task remain None (no crash, no error)
    ///
    /// WHY THIS MATTERS: Not all projects use git. The inference feature should
    /// degrade gracefully, allowing trop to work normally in non-git contexts.
    #[test]
    fn test_with_git_inference_non_git_directory() {
        let temp = TempDir::new().unwrap();
        let non_git_dir = temp.path().join("not-a-repo");
        std::fs::create_dir(&non_git_dir).unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        let options = ReserveOptions::new(key, Some(port)).with_git_inference(&non_git_dir);

        // Values should remain None, but no error should occur
        assert_eq!(
            options.project, None,
            "Project should remain None for non-git directory"
        );
        assert_eq!(
            options.task, None,
            "Task should remain None for non-git directory"
        );
    }

    /// Tests that with_git_inference() handles detached HEAD gracefully.
    ///
    /// SEMANTIC INVARIANT: In a detached HEAD state, project can still be inferred
    /// (from repository name), but task cannot (no branch). The function should
    /// handle this mixed state correctly without errors.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "test-project"
    /// - Detach HEAD
    /// - Create ReserveOptions with no explicit project/task
    /// - Call with_git_inference()
    /// - Verify project is inferred, task remains None
    ///
    /// WHY THIS MATTERS: Detached HEAD is a valid git state. The inference should
    /// work partially, providing what information is available (project) while
    /// gracefully handling what isn't (task).
    #[test]
    fn test_with_git_inference_detached_head() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("test-project");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();
        helpers::detach_head(&repo_path).unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        let options = ReserveOptions::new(key, Some(port)).with_git_inference(&repo_path);

        // Project should be inferred, task should be None (detached HEAD)
        assert_eq!(
            options.project,
            Some("test-project".to_string()),
            "Project should be inferred even in detached HEAD state"
        );
        assert_eq!(
            options.task, None,
            "Task should be None for detached HEAD (no branch)"
        );
    }

    /// Tests inference in a worktree: project from main repo, task from worktree dir.
    ///
    /// SEMANTIC INVARIANT: In a worktree context, both project and task inference
    /// should work, but with different sources:
    /// - Project comes from the main repository's directory name
    /// - Task comes from the worktree's directory name (not the branch)
    ///
    /// TEST SCENARIO:
    /// - Create a main repository named "main-project"
    /// - Create a worktree in directory "feature-work" on branch "some-branch"
    /// - Create ReserveOptions with no explicit project/task
    /// - Call with_git_inference() from the worktree
    /// - Verify project="main-project" and task="feature-work"
    ///
    /// WHY THIS MATTERS: This tests the complete integration of worktree support,
    /// ensuring both inference functions work correctly together in the worktree
    /// scenario, which is a key use case for the feature.
    #[test]
    fn test_with_git_inference_in_worktree() {
        let temp = TempDir::new().unwrap();
        let main_repo = temp.path().join("main-project");
        std::fs::create_dir(&main_repo).unwrap();

        helpers::create_test_repo(&main_repo).unwrap();

        let worktree_path = temp.path().join("feature-work");
        helpers::create_worktree(&main_repo, &worktree_path, "some-branch").unwrap();

        let key = ReservationKey::new(PathBuf::from("/test/path"), None).unwrap();
        let port = Port::try_from(8080).unwrap();

        let options = ReserveOptions::new(key, Some(port)).with_git_inference(&worktree_path);

        assert_eq!(
            options.project,
            Some("main-project".to_string()),
            "Project should be inferred from main repository in worktree"
        );
        assert_eq!(
            options.task,
            Some("feature-work".to_string()),
            "Task should be inferred from worktree directory name"
        );
    }
}

// ============================================================================
// Edge Case Tests
// ============================================================================

mod edge_cases {
    use super::*;

    /// Tests handling of nested git repositories (submodules scenario).
    ///
    /// SEMANTIC INVARIANT: When git repositories are nested, the discovery should
    /// find the closest repository upward from the current directory. This matches
    /// git's own behavior for finding the repository.
    ///
    /// TEST SCENARIO:
    /// - Create an outer repository named "outer-repo"
    /// - Create an inner repository in a subdirectory named "inner-repo"
    /// - Call infer_project() from within the inner repository
    /// - Verify it returns "inner-repo", not "outer-repo"
    ///
    /// WHY THIS MATTERS: Git submodules and monorepo setups can create nested
    /// repositories. The inference should respect git's discovery rules and work
    /// with the repository that actually contains the current directory.
    #[test]
    fn test_nested_repositories() {
        let temp = TempDir::new().unwrap();
        let outer_repo = temp.path().join("outer-repo");
        std::fs::create_dir(&outer_repo).unwrap();

        helpers::create_test_repo(&outer_repo).unwrap();

        // Create a nested repository
        let inner_repo = outer_repo.join("subdir").join("inner-repo");
        std::fs::create_dir_all(&inner_repo).unwrap();
        helpers::create_test_repo(&inner_repo).unwrap();

        // Inference should find the inner (closest) repository
        let project = infer_project(&inner_repo);
        assert_eq!(
            project,
            Some("inner-repo".to_string()),
            "Nested repositories should infer from the closest repository"
        );
    }

    /// Tests that inference works correctly with symlinked repositories.
    ///
    /// SEMANTIC INVARIANT: Git follows symlinks when discovering repositories.
    /// The inference should work through symlinks, finding the actual repository.
    ///
    /// TEST SCENARIO:
    /// - Create a git repository named "real-repo"
    /// - Create a symlink pointing to the repository
    /// - Call infer_project() from the symlink path
    /// - Verify the project name is correctly inferred
    ///
    /// WHY THIS MATTERS: Users sometimes work with symlinked directories, and
    /// git supports this. The inference should work transparently through symlinks.
    ///
    /// NOTE: This test may behave differently on Windows vs Unix systems due to
    /// differences in symlink support. The test is marked to skip on Windows if
    /// symlink creation fails.
    #[test]
    fn test_symlinked_repository() {
        let temp = TempDir::new().unwrap();
        let real_repo = temp.path().join("real-repo");
        std::fs::create_dir(&real_repo).unwrap();

        helpers::create_test_repo(&real_repo).unwrap();

        // Create a symlink to the repository
        let symlink_path = temp.path().join("repo-link");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_repo, &symlink_path).unwrap();

            let project = infer_project(&symlink_path);
            assert!(
                project.is_some(),
                "Project inference should work through symlinks"
            );
        }

        #[cfg(windows)]
        {
            // Windows symlink creation requires admin privileges or developer mode
            // Skip this test on Windows if we can't create the symlink
            if std::os::windows::fs::symlink_dir(&real_repo, &symlink_path).is_ok() {
                let project = infer_project(&symlink_path);
                assert!(
                    project.is_some(),
                    "Project inference should work through symlinks"
                );
            }
        }
    }

    /// Tests that inference handles repository names with Unicode characters.
    ///
    /// SEMANTIC INVARIANT: Repository names can contain Unicode characters in
    /// filesystems that support them. The inference should preserve these
    /// characters correctly without corruption or errors.
    ///
    /// TEST SCENARIO:
    /// - Create a repository with Unicode characters in the name: "my-项目-repo"
    /// - Call infer_project()
    /// - Verify the Unicode characters are preserved in the returned name
    ///
    /// WHY THIS MATTERS: International users may have repository names in their
    /// native language. The inference should support this without issues.
    ///
    /// NOTE: This test may fail on filesystems that don't support Unicode
    /// filenames, but should work on modern systems.
    #[test]
    fn test_unicode_repository_name() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("my-项目-repo");
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();

        let project = infer_project(&repo_path);
        assert_eq!(
            project,
            Some("my-项目-repo".to_string()),
            "Repository names with Unicode should be preserved"
        );
    }

    /// Tests that inference handles very long repository and branch names.
    ///
    /// SEMANTIC INVARIANT: While there are practical limits on filesystem and
    /// git name lengths, the inference should handle reasonably long names
    /// without truncation or errors.
    ///
    /// TEST SCENARIO:
    /// - Create a repository with a long name (255 characters, near filesystem limit)
    /// - Create a branch with a long name
    /// - Call infer_project() and infer_task()
    /// - Verify the full names are preserved
    ///
    /// WHY THIS MATTERS: While unusual, some projects or workflows might use
    /// long, descriptive names. The inference should handle these robustly.
    #[test]
    fn test_very_long_names() {
        let temp = TempDir::new().unwrap();

        // Create a long but valid repository name (not too long to exceed filesystem limits)
        let long_name = "a".repeat(100);
        let repo_path = temp.path().join(&long_name);
        std::fs::create_dir(&repo_path).unwrap();

        helpers::create_test_repo(&repo_path).unwrap();

        // Create a long branch name
        let long_branch = "feature-".to_string() + &"b".repeat(100);
        helpers::switch_branch(&repo_path, &long_branch).unwrap();

        let project = infer_project(&repo_path);
        assert_eq!(
            project,
            Some(long_name.clone()),
            "Long repository names should be fully preserved"
        );

        let task = infer_task(&repo_path);
        assert_eq!(
            task,
            Some(long_branch),
            "Long branch names should be fully preserved"
        );
    }
}
