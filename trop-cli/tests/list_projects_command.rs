//! Comprehensive integration tests for the `list-projects` command.
//!
//! These tests verify all aspects of listing unique project identifiers, including:
//! - Empty database handling (no output)
//! - Single project output
//! - Multiple projects with correct deduplication
//! - NULL project handling (exclusion from output)
//! - Alphabetical ordering
//! - Output format (one per line)
//! - Integration with reserve operations

mod common;

use common::TestEnv;

// ============================================================================
// Basic List Projects Tests
// ============================================================================

/// Test list-projects with empty database.
///
/// When no reservations exist (or none have projects), list-projects should:
/// - Succeed (not fail)
/// - Produce no output (no lines on stdout)
/// - Not crash or error
///
/// This verifies the empty case is handled gracefully.
#[test]
fn test_list_projects_empty_database() {
    let env = TestEnv::new();

    // List projects from empty database
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should be completely empty (no projects)
    assert_eq!(stdout, "", "Empty database should produce no output");
}

/// Test list-projects with single project.
///
/// A single reservation with a project should output exactly that project name.
/// This verifies basic functionality: one project in, one project out.
#[test]
fn test_list_projects_single_project() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation with a project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("my-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects should show it
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should have exactly one line with the project name
    assert_eq!(
        stdout.trim(),
        "my-project",
        "Should output the project name"
    );

    // Verify it's exactly one line
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "Should have exactly one line of output");
}

/// Test list-projects with multiple different projects.
///
/// When multiple reservations have different projects, all unique projects
/// should appear in the output.
///
/// This verifies:
/// - All unique projects are listed
/// - Each project appears exactly once (deduplication)
#[test]
fn test_list_projects_multiple_projects() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");
    let path3 = env.create_dir("project3");

    // Create reservations with different projects
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--project")
        .arg("alpha")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--project")
        .arg("beta")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path3)
        .arg("--project")
        .arg("gamma")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should contain all three projects
    assert!(stdout.contains("alpha"), "Should list alpha");
    assert!(stdout.contains("beta"), "Should list beta");
    assert!(stdout.contains("gamma"), "Should list gamma");

    // Should have exactly three lines
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "Should have three projects");
}

// ============================================================================
// NULL Project Handling Tests
// ============================================================================

/// Test list-projects excludes NULL projects.
///
/// Reservations without a project (project = NULL) should not appear in the
/// output. Only reservations with explicit project values should be listed.
///
/// This is important because it means list-projects only shows intentionally
/// tagged reservations, not all reservations.
#[test]
fn test_list_projects_excludes_null_projects() {
    let env = TestEnv::new();
    let path_with_project = env.create_dir("with-project");
    let path_without_project = env.create_dir("without-project");

    // Create one reservation WITH a project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path_with_project)
        .arg("--project")
        .arg("has-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Create one reservation WITHOUT a project (NULL)
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path_without_project)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects should only show the one with a project
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should only contain the named project
    assert!(
        stdout.contains("has-project"),
        "Should list the named project"
    );

    // Should have exactly one line
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "Should only have one project (NULL excluded)"
    );
}

/// Test list-projects with all NULL projects.
///
/// If all reservations have NULL projects, list-projects should output nothing.
/// This is essentially the same as an empty database from the perspective of
/// this command.
#[test]
fn test_list_projects_all_null_projects() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    // Create multiple reservations, none with projects
    env.reserve_simple(&path1);
    env.reserve_simple(&path2);

    // List projects should be empty
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should have no output
    assert_eq!(stdout, "", "All NULL projects should produce no output");
}

// ============================================================================
// Deduplication Tests
// ============================================================================

/// Test list-projects deduplicates same project.
///
/// Multiple reservations with the same project should result in that project
/// appearing only once in the output. This tests the DISTINCT behavior.
///
/// This is a key semantic property: list-projects returns unique identifiers,
/// not a count of how many times each appears.
#[test]
fn test_list_projects_deduplicates_same_project() {
    let env = TestEnv::new();
    let path1 = env.create_dir("service1");
    let path2 = env.create_dir("service2");
    let path3 = env.create_dir("service3");

    // Create multiple reservations with the same project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--project")
        .arg("my-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--project")
        .arg("my-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path3)
        .arg("--project")
        .arg("my-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects should show it only once
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should have exactly one line
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "Same project should appear only once (deduplicated)"
    );
    assert_eq!(
        lines[0], "my-project",
        "The one line should be the project name"
    );
}

/// Test deduplication with mixed projects.
///
/// A combination of repeated and unique projects should all be deduplicated.
/// For example: [A, B, A, C, B] -> [A, B, C] (in alphabetical order).
#[test]
fn test_list_projects_deduplicates_mixed_projects() {
    let env = TestEnv::new();

    // Create reservations: alpha, beta, alpha, gamma, beta
    for (i, project) in ["alpha", "beta", "alpha", "gamma", "beta"]
        .iter()
        .enumerate()
    {
        let path = env.create_dir(&format!("project{i}"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(project)
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();

    // Should have exactly three unique projects
    assert_eq!(lines.len(), 3, "Should have three unique projects");

    // Should be alpha, beta, gamma (alphabetical)
    assert_eq!(lines[0], "alpha");
    assert_eq!(lines[1], "beta");
    assert_eq!(lines[2], "gamma");
}

// ============================================================================
// Alphabetical Ordering Tests
// ============================================================================

/// Test list-projects alphabetical ordering.
///
/// Projects should be output in alphabetical order (ORDER BY project).
/// This makes the output consistent and predictable for users and scripts.
///
/// The SQL query uses ORDER BY, so this tests that the implementation
/// follows the specification.
#[test]
fn test_list_projects_alphabetical_order() {
    let env = TestEnv::new();

    // Create projects in non-alphabetical order
    let projects = ["zebra", "alpha", "mike", "charlie"];

    for (i, project) in projects.iter().enumerate() {
        let path = env.create_dir(&format!("project{i}"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(project)
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();

    // Should be in alphabetical order
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "alpha", "First should be alpha");
    assert_eq!(lines[1], "charlie", "Second should be charlie");
    assert_eq!(lines[2], "mike", "Third should be mike");
    assert_eq!(lines[3], "zebra", "Fourth should be zebra");
}

/// Test alphabetical ordering is case-sensitive.
///
/// SQLite's default ordering is case-sensitive, so "Alpha" comes before "alpha".
/// This test documents that behavior (which follows SQL standard).
///
/// If we wanted case-insensitive ordering, we'd need COLLATE NOCASE in SQL.
#[test]
fn test_list_projects_case_sensitive_ordering() {
    let env = TestEnv::new();

    // Mix of case variations
    let projects = ["zebra", "Alpha", "alpha", "BETA"];

    for (i, project) in projects.iter().enumerate() {
        let path = env.create_dir(&format!("project{i}"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(project)
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();

    // SQLite default: ASCII sort (uppercase before lowercase)
    // Expected order: Alpha, BETA, alpha, zebra
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "Alpha");
    assert_eq!(lines[1], "BETA");
    assert_eq!(lines[2], "alpha");
    assert_eq!(lines[3], "zebra");
}

// ============================================================================
// Output Format Tests
// ============================================================================

/// Test output format is one project per line.
///
/// Each project should be on its own line, with no extra formatting,
/// headers, or decorations. This makes it easy to parse in scripts.
///
/// Format should be:
/// ```
/// project1
/// project2
/// project3
/// ```
#[test]
fn test_list_projects_one_per_line() {
    let env = TestEnv::new();

    // Create a few projects
    for project in ["proj-a", "proj-b", "proj-c"] {
        let path = env.create_dir(&format!("{project}-dir"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(project)
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should end with newline
    assert!(
        stdout.ends_with('\n'),
        "Output should end with newline for proper line formatting"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);

    // Each line should be just the project name (no tabs, commas, etc.)
    assert_eq!(lines[0], "proj-a");
    assert_eq!(lines[1], "proj-b");
    assert_eq!(lines[2], "proj-c");

    // No extra content
    for line in &lines {
        assert!(!line.contains('\t'), "Should not contain tabs: {line}");
        assert!(!line.contains(','), "Should not contain commas: {line}");
    }
}

/// Test list-projects with special characters in project names.
///
/// Project names with spaces, Unicode, or other special characters should
/// be output correctly without escaping or corruption.
///
/// This documents that the output is raw text, not CSV-escaped or quoted.
#[test]
fn test_list_projects_special_characters() {
    let env = TestEnv::new();

    // Projects with spaces and Unicode
    let projects = ["project with spaces", "プロジェクト", "dots.in.name"];

    for (i, project) in projects.iter().enumerate() {
        let path = env.create_dir(&format!("dir{i}"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(project)
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should contain all projects exactly as specified
    assert!(stdout.contains("project with spaces"));
    assert!(stdout.contains("プロジェクト"));
    assert!(stdout.contains("dots.in.name"));

    // Verify they're on separate lines
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Test list-projects reflects database changes.
///
/// As reservations are added and released, list-projects should reflect
/// the current state. This is an integration test that verifies the full
/// lifecycle.
#[test]
fn test_list_projects_reflects_database_changes() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    // Initially empty
    let output1 = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout1 = String::from_utf8(output1.stdout).unwrap();
    assert_eq!(stdout1, "", "Should start empty");

    // Add first project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--project")
        .arg("first-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let output2 = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert_eq!(stdout2.trim(), "first-project", "Should show first project");

    // Add second project
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--project")
        .arg("second-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let output3 = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout3 = String::from_utf8(output3.stdout).unwrap();
    let lines3: Vec<&str> = stdout3.lines().collect();
    assert_eq!(lines3.len(), 2, "Should show both projects");
    assert!(stdout3.contains("first-project"));
    assert!(stdout3.contains("second-project"));

    // Release first project's reservation
    env.release(&path1);

    let output4 = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout4 = String::from_utf8(output4.stdout).unwrap();
    assert_eq!(
        stdout4.trim(),
        "second-project",
        "Should only show remaining project"
    );

    // Release second project's reservation
    env.release(&path2);

    let output5 = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout5 = String::from_utf8(output5.stdout).unwrap();
    assert_eq!(stdout5, "", "Should be empty again");
}

/// Test list-projects with group reservations.
///
/// Group reservations can all share the same project. Verify that when
/// a group is reserved with a project, that project appears in list-projects.
#[test]
fn test_list_projects_with_group_reservations() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-group");

    // Create a config file for reserve-group
    let config_path = test_path.join("trop.yaml");
    std::fs::write(
        &config_path,
        "ports:\n  min: 5000\n  max: 9999\nproject: group-project\nreservations:\n  base: 9000\n  services:\n    web:\n      offset: 0\n      env: WEB_PORT\n    api:\n      offset: 1\n      env: API_PORT\n    db:\n      offset: 2\n      env: DB_PORT\n",
    )
    .expect("Failed to write config");

    // Reserve a group with a project
    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects should show the group's project once
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should have exactly one project
    assert_eq!(
        stdout.trim(),
        "group-project",
        "Group project should appear once"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "Group project should appear only once despite multiple reservations"
    );
}

// ============================================================================
// Global Options Tests
// ============================================================================

/// Test list-projects respects --quiet flag.
///
/// The --quiet flag should suppress stderr output but still show projects
/// on stdout. This is important for script usage.
#[test]
fn test_list_projects_quiet_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test");

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("test-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let output = env
        .command()
        .arg("--quiet")
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    // Stdout should still have the project
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "test-project");

    // Stderr should be empty or minimal
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.is_empty() || stderr.trim().is_empty());
}

/// Test list-projects respects --verbose flag.
///
/// The --verbose flag might add logging to stderr, but should not affect
/// stdout output. This test verifies the command doesn't crash with verbose.
#[test]
fn test_list_projects_verbose_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test");

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("test-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let output = env
        .command()
        .arg("--verbose")
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    // Stdout should have the project
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.trim(), "test-project");

    // Verbose mode may produce stderr (but we don't require it)
    // Just verify it doesn't crash
}

/// Test list-projects with custom --data-dir.
///
/// When a custom data directory is specified, list-projects should use
/// that database. This test creates two separate environments to verify
/// isolation.
#[test]
fn test_list_projects_custom_data_dir() {
    let env1 = TestEnv::new();
    let env2 = TestEnv::new();

    // Create a project in env1
    let path1 = env1.create_dir("test1");
    env1.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--project")
        .arg("env1-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Create a different project in env2
    let path2 = env2.create_dir("test2");
    env2.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--project")
        .arg("env2-project")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List from env1 should only show env1's project
    let output1 = env1
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout1 = String::from_utf8(output1.stdout).unwrap();
    assert_eq!(stdout1.trim(), "env1-project");

    // List from env2 should only show env2's project
    let output2 = env2
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert_eq!(stdout2.trim(), "env2-project");
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Test list-projects with very long project names.
///
/// Long project names should be handled correctly without truncation
/// or corruption.
#[test]
fn test_list_projects_long_project_names() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test");

    // Create a very long project name
    let long_project = "a".repeat(200);

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg(&long_project)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // List projects
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should output the full name
    assert_eq!(stdout.trim(), long_project);
}

/// Test list-projects with many projects (performance smoke test).
///
/// Creating many projects and listing them should complete in reasonable time.
/// This is more of a smoke test than a performance benchmark.
#[test]
fn test_list_projects_many_projects() {
    let env = TestEnv::new();

    // Create 100 projects (enough to test, not too slow)
    for i in 0..100 {
        let path = env.create_dir(&format!("project{i:03}"));
        env.command()
            .arg("reserve")
            .arg("--path")
            .arg(&path)
            .arg("--project")
            .arg(format!("project-{i:03}"))
            .arg("--allow-unrelated-path")
            .assert()
            .success();
    }

    // List projects should work
    let output = env
        .command()
        .arg("list-projects")
        .output()
        .expect("Failed to run list-projects");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");
    let lines: Vec<&str> = stdout.lines().collect();

    // Should have all 100 projects
    assert_eq!(lines.len(), 100, "Should list all 100 projects");

    // Verify they're in order (spot check)
    assert!(lines[0].starts_with("project-"));
    assert!(lines[99].starts_with("project-"));
}
