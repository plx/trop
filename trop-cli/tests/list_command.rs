//! Comprehensive integration tests for the `list` command.
//!
//! These tests verify all aspects of listing reservations, including:
//! - Empty database handling
//! - Various output formats (table, json, csv, tsv)
//! - Filtering by project, tag, and path
//! - Path display options (full vs shortened)
//! - Sorting and ordering
//! - Integration with reserve/release commands

mod common;

use common::TestEnv;
use serde_json::Value;

// ============================================================================
// Basic List Tests
// ============================================================================

/// Test list with empty database.
///
/// When no reservations exist, list should:
/// - Succeed (not fail)
/// - Show table header (in table format)
/// - Have empty content (no reservation rows)
#[test]
fn test_list_empty_database() {
    let env = TestEnv::new();

    // List with no reservations
    let output = env.list();

    // Should have header but no data rows
    assert!(output.contains("PORT"));
    assert!(output.contains("PATH"));

    // Should not have any port numbers (only header)
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 1, "Should have only header line when empty");
}

/// Test list with single reservation.
///
/// After creating one reservation, list should display it with all fields.
#[test]
fn test_list_single_reservation() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    let port = env.reserve_simple(&test_path);

    // List should show it
    let output = env.list();
    assert!(output.contains(&port.to_string()));
    assert!(output.contains(test_path.to_str().unwrap()));
}

/// Test list with multiple reservations.
///
/// All reservations should appear in the list output.
#[test]
fn test_list_multiple_reservations() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");
    let path3 = env.create_dir("project3");

    // Create multiple reservations
    let port1 = env.reserve_simple(&path1);
    let port2 = env.reserve_simple(&path2);
    let port3 = env.reserve_simple(&path3);

    // All should appear in list
    let output = env.list();
    assert!(output.contains(&port1.to_string()));
    assert!(output.contains(&port2.to_string()));
    assert!(output.contains(&port3.to_string()));
}

/// Test list shows tags.
///
/// Tagged reservations should display their tags in the output.
#[test]
fn test_list_shows_tags() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservations with tags
    env.reserve_with_tag(&test_path, "web");
    env.reserve_with_tag(&test_path, "api");

    // Tags should appear in list
    let output = env.list();
    assert!(output.contains("web"));
    assert!(output.contains("api"));
}

/// Test list shows metadata (project, task).
///
/// Reservation metadata should be visible in list output.
#[test]
fn test_list_shows_metadata() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservation with metadata
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("my-project")
        .arg("--task")
        .arg("task-123")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Metadata should appear
    let output = env.list();
    assert!(output.contains("my-project"));
    assert!(output.contains("task-123"));
}

// ============================================================================
// Output Format Tests
// ============================================================================

/// Test default table format.
///
/// Without specifying a format, list should output in table format
/// (tab-separated values with header).
#[test]
fn test_list_default_table_format() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    // Default format should be table (contains tabs)
    let output = env.list();
    assert!(output.contains('\t'), "Table format should use tabs");
    assert!(output.contains("PORT"), "Should have header");
}

/// Test JSON format.
///
/// JSON format should produce valid JSON with all reservation details.
#[test]
fn test_list_json_format() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    let port = env.reserve_simple(&test_path);

    // Get JSON output
    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let json_str = String::from_utf8(output.stdout).unwrap();

    // Should be valid JSON
    let json: Value = serde_json::from_str(&json_str).expect("Should be valid JSON");

    // Should be an array
    assert!(json.is_array(), "JSON output should be an array");

    // Should contain our reservation
    let array = json.as_array().unwrap();
    assert_eq!(array.len(), 1, "Should have one reservation");

    // Check the reservation has expected fields
    let reservation = &array[0];
    assert!(reservation.get("port").is_some());
    assert!(reservation.get("path").is_some());

    // Port should match
    let json_port = reservation["port"].as_u64().unwrap() as u16;
    assert_eq!(json_port, port);
}

/// Test JSON format with empty database.
///
/// Empty database should produce an empty JSON array.
#[test]
fn test_list_json_empty() {
    let env = TestEnv::new();

    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let json_str = String::from_utf8(output.stdout).unwrap();

    // Should be empty array
    let json: Value = serde_json::from_str(&json_str).expect("Should be valid JSON");
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

/// Test CSV format.
///
/// CSV format should produce comma-separated values with proper escaping.
#[test]
fn test_list_csv_format() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("csv")
        .output()
        .unwrap();

    assert!(output.status.success());
    let csv = String::from_utf8(output.stdout).unwrap();

    // Should have commas
    assert!(csv.contains(','), "CSV should contain commas");

    // Should have header row
    assert!(csv.contains("port") || csv.contains("PORT"));
}

/// Test TSV format.
///
/// TSV format should produce tab-separated values.
#[test]
fn test_list_tsv_format() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("tsv")
        .output()
        .unwrap();

    assert!(output.status.success());
    let tsv = String::from_utf8(output.stdout).unwrap();

    // Should have tabs
    assert!(tsv.contains('\t'), "TSV should contain tabs");

    // Should have header row
    assert!(tsv.contains("PORT") || tsv.contains("port"));
}

/// Test format case-insensitivity.
///
/// Format names should be case-insensitive (JSON, json, Json all work).
#[test]
fn test_list_format_case_insensitive() {
    let env = TestEnv::new();

    // These should all work
    for format in [
        "json", "JSON", "Json", "csv", "CSV", "tsv", "TSV", "table", "TABLE",
    ] {
        env.command()
            .arg("list")
            .arg("--format")
            .arg(format)
            .assert()
            .success();
    }
}

// ============================================================================
// Filter Tests
// ============================================================================

/// Test filter by project.
///
/// --filter-project should show only reservations with that project.
#[test]
fn test_list_filter_by_project() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    // Create reservations with different projects
    let port1 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--project")
        .arg("proj-a")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    let port1 = common::parse_port(&String::from_utf8(port1.stdout).unwrap());

    let port2 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--project")
        .arg("proj-b")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    let port2 = common::parse_port(&String::from_utf8(port2.stdout).unwrap());

    // Filter by proj-a
    let output = env
        .command()
        .arg("list")
        .arg("--filter-project")
        .arg("proj-a")
        .output()
        .unwrap();

    let filtered = String::from_utf8(output.stdout).unwrap();

    // Should show port1 but not port2
    assert!(filtered.contains(&port1.to_string()));
    assert!(!filtered.contains(&port2.to_string()));
    assert!(filtered.contains("proj-a"));
    assert!(!filtered.contains("proj-b"));
}

/// Test filter by tag.
///
/// --filter-tag should show only reservations with that tag.
#[test]
fn test_list_filter_by_tag() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservations with different tags
    let port_web = env.reserve_with_tag(&test_path, "web");
    let port_api = env.reserve_with_tag(&test_path, "api");

    // Filter by "web" tag
    let output = env
        .command()
        .arg("list")
        .arg("--filter-tag")
        .arg("web")
        .output()
        .unwrap();

    let filtered = String::from_utf8(output.stdout).unwrap();

    // Should show web but not api
    assert!(filtered.contains(&port_web.to_string()));
    assert!(!filtered.contains(&port_api.to_string()));
}

/// Test filter by path prefix.
///
/// --filter-path should show only reservations under that path.
#[test]
fn test_list_filter_by_path() {
    let env = TestEnv::new();
    let parent = env.create_dir("parent");
    let child = env.create_dir("parent/child");
    let sibling = env.create_dir("sibling");

    // Create reservations at different paths
    let port_parent = env.reserve_simple(&parent);
    let port_child = env.reserve_simple(&child);
    let port_sibling = env.reserve_simple(&sibling);

    // Filter by parent path (should include parent and child)
    let output = env
        .command()
        .arg("list")
        .arg("--filter-path")
        .arg(&parent)
        .output()
        .unwrap();

    let filtered = String::from_utf8(output.stdout).unwrap();

    // Should show parent and child, not sibling
    assert!(filtered.contains(&port_parent.to_string()));
    assert!(filtered.contains(&port_child.to_string()));
    assert!(!filtered.contains(&port_sibling.to_string()));
}

/// Test combining multiple filters.
///
/// Multiple filters should be AND'ed together (all must match).
#[test]
fn test_list_multiple_filters() {
    let env = TestEnv::new();
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    // Create various reservations
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--tag")
        .arg("web")
        .arg("--project")
        .arg("proj-a")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    let port2 = env
        .command()
        .arg("reserve")
        .arg("--path")
        .arg(&path1)
        .arg("--tag")
        .arg("api")
        .arg("--project")
        .arg("proj-a")
        .arg("--allow-unrelated-path")
        .output()
        .unwrap();
    let port2 = common::parse_port(&String::from_utf8(port2.stdout).unwrap());

    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&path2)
        .arg("--tag")
        .arg("api")
        .arg("--project")
        .arg("proj-b")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Filter by tag=api AND project=proj-a
    let output = env
        .command()
        .arg("list")
        .arg("--filter-tag")
        .arg("api")
        .arg("--filter-project")
        .arg("proj-a")
        .output()
        .unwrap();

    let filtered = String::from_utf8(output.stdout).unwrap();

    // Should only show port2 (api + proj-a)
    assert!(filtered.contains(&port2.to_string()));
    assert!(filtered.contains("api"));
    assert!(filtered.contains("proj-a"));

    // Should not show the web tag or proj-b
    assert!(!filtered.contains("web"));
    assert!(!filtered.contains("proj-b"));
}

/// Test filter with no matches.
///
/// If filters match nothing, list should show empty result (header only).
#[test]
fn test_list_filter_no_matches() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create a reservation
    env.reserve_simple(&test_path);

    // Filter by non-existent project
    let output = env
        .command()
        .arg("list")
        .arg("--filter-project")
        .arg("nonexistent")
        .output()
        .unwrap();

    let filtered = String::from_utf8(output.stdout).unwrap();

    // Should have header but no data
    assert!(filtered.contains("PORT"));
    let lines: Vec<&str> = filtered.lines().collect();
    assert_eq!(lines.len(), 1, "Should only have header line");
}

// ============================================================================
// Path Display Tests
// ============================================================================

/// Test default path display (shortened).
///
/// By default, paths should be shortened (e.g., ~/... for home directory).
#[test]
fn test_list_default_path_display() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env.list();

    // Should contain the path (exact format may vary)
    assert!(output.contains(test_path.file_name().unwrap().to_str().unwrap()));
}

/// Test --show-full-paths flag.
///
/// With --show-full-paths, absolute paths should be displayed without shortening.
#[test]
fn test_list_show_full_paths() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env
        .command()
        .arg("list")
        .arg("--show-full-paths")
        .output()
        .unwrap();

    let list_output = String::from_utf8(output.stdout).unwrap();

    // Should contain the full absolute path
    assert!(list_output.contains(test_path.to_str().unwrap()));
}

/// Test path display in JSON always uses full paths.
///
/// JSON output should always use complete, unambiguous paths regardless
/// of --show-full-paths flag.
#[test]
fn test_list_json_uses_full_paths() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    let json_str = String::from_utf8(output.stdout).unwrap();
    let json: Value = serde_json::from_str(&json_str).unwrap();

    // Path in JSON should be full absolute path
    let path_in_json = json[0]["path"].as_str().unwrap();
    assert!(
        path_in_json.starts_with('/') || path_in_json.contains(':'),
        "JSON path should be absolute"
    );
}

// ============================================================================
// Environment Variable Tests
// ============================================================================

/// Test TROP_OUTPUT_FORMAT environment variable.
///
/// The output format can be set via environment variable.
#[test]
fn test_list_respects_output_format_env() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    // Set format via env var
    let output = env
        .command()
        .arg("list")
        .env("TROP_OUTPUT_FORMAT", "json")
        .output()
        .unwrap();

    let output_str = String::from_utf8(output.stdout).unwrap();

    // Should be JSON
    assert!(output_str.trim().starts_with('['));
    serde_json::from_str::<Value>(&output_str).expect("Should be valid JSON");
}

/// Test --format flag overrides environment variable.
///
/// CLI flag should take precedence over env var.
#[test]
fn test_cli_format_overrides_env() {
    let env = TestEnv::new();

    // Set env to json but use --format table
    let output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("table")
        .env("TROP_OUTPUT_FORMAT", "json")
        .output()
        .unwrap();

    let output_str = String::from_utf8(output.stdout).unwrap();

    // Should be table format (tabs), not JSON
    assert!(output_str.contains('\t'));
    assert!(!output_str.trim().starts_with('['));
}

// ============================================================================
// Output Ordering Tests
// ============================================================================

/// Test that reservations are listed in consistent order.
///
/// The order should be deterministic (e.g., by port number or creation time).
#[test]
fn test_list_consistent_ordering() {
    let env = TestEnv::new();

    // Create several reservations
    let path1 = env.create_dir("p1");
    let path2 = env.create_dir("p2");
    let path3 = env.create_dir("p3");

    env.reserve_simple(&path1);
    env.reserve_simple(&path2);
    env.reserve_simple(&path3);

    // List multiple times - order should be consistent
    let output1 = env.list();
    let output2 = env.list();

    assert_eq!(output1, output2, "List order should be consistent");
}

// ============================================================================
// Edge Cases
// ============================================================================

/// Test list with very long paths.
///
/// Long paths should be handled gracefully, possibly truncated in table format.
#[test]
fn test_list_with_long_paths() {
    let env = TestEnv::new();

    // Create a deeply nested path
    let mut deep_path = env.path().to_path_buf();
    for i in 0..10 {
        deep_path = deep_path.join(format!("level{i}"));
    }
    std::fs::create_dir_all(&deep_path).unwrap();

    env.reserve_simple(&deep_path);

    // Should not crash or produce malformed output
    let output = env.list();
    assert!(output.contains("PORT"));
}

/// Test list with special characters in paths.
///
/// Paths with spaces, quotes, or other special characters should be
/// properly escaped in CSV/TSV output.
#[test]
fn test_list_with_special_characters_in_path() {
    let env = TestEnv::new();

    // Create path with spaces
    let special_path = env.create_dir("path with spaces");
    env.reserve_simple(&special_path);

    // Table format should handle it
    let output = env.list();
    assert!(output.contains("spaces") || output.contains("path with spaces"));

    // CSV should properly escape/quote
    let csv_output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("csv")
        .output()
        .unwrap();

    let csv = String::from_utf8(csv_output.stdout).unwrap();
    // Should either quote the path or handle it somehow
    assert!(csv.contains("spaces"));
}

/// Test list with Unicode in metadata.
///
/// Project names, tasks, and tags with Unicode should display correctly.
#[test]
fn test_list_with_unicode_metadata() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Create reservation with Unicode metadata
    env.command()
        .arg("reserve")
        .arg("--path")
        .arg(&test_path)
        .arg("--project")
        .arg("プロジェクト")
        .arg("--task")
        .arg("タスク-1")
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // Should display Unicode correctly
    let output = env.list();
    assert!(output.contains("プロジェクト"));
    assert!(output.contains("タスク-1"));

    // JSON should handle Unicode
    let json_output = env
        .command()
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();

    let json_str = String::from_utf8(json_output.stdout).unwrap();
    assert!(json_str.contains("プロジェクト") || json_str.contains("\\u"));
}

// ============================================================================
// Quiet/Verbose Mode Tests
// ============================================================================

/// Test list with --quiet flag.
///
/// --quiet should suppress stderr output but still show list on stdout.
#[test]
fn test_list_quiet_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env.command().arg("--quiet").arg("list").output().unwrap();

    assert!(output.status.success());

    // Stdout should still have list
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("PORT"));

    // Stderr should be minimal/empty
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.is_empty() || stderr.trim().is_empty());
}

/// Test list with --verbose flag.
///
/// --verbose might add additional logging to stderr.
#[test]
fn test_list_verbose_mode() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");
    env.reserve_simple(&test_path);

    let output = env.command().arg("--verbose").arg("list").output().unwrap();

    assert!(output.status.success());

    // Stdout should have list
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("PORT"));

    // Verbose might produce stderr output (but not required)
    // Just verify it doesn't crash
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Test that list reflects reserve/release operations.
///
/// This integration test verifies the full lifecycle: reserve, list, release, list.
#[test]
fn test_list_reflects_reserve_and_release() {
    let env = TestEnv::new();
    let test_path = env.create_dir("test-project");

    // Initially empty
    let list1 = env.list();
    assert!(!list1.contains(test_path.to_str().unwrap()));

    // Reserve a port
    let port = env.reserve_simple(&test_path);

    // Should appear in list
    let list2 = env.list();
    assert!(list2.contains(&port.to_string()));
    assert!(list2.contains(test_path.to_str().unwrap()));

    // Release it
    env.release(&test_path);

    // Should disappear from list
    let list3 = env.list();
    assert!(!list3.contains(&port.to_string()));
}

/// Test list performance with many reservations.
///
/// Creating many reservations and listing them should complete in reasonable time.
/// This is more of a smoke test than a performance benchmark.
#[test]
fn test_list_with_many_reservations() {
    let env = TestEnv::new();

    // Create 50 reservations (enough to test, not too slow)
    for i in 0..50 {
        let path = env.create_dir(&format!("project{i}"));
        env.reserve_simple(&path);
    }

    // List should work and show all of them
    let output = env.list();
    let lines: Vec<&str> = output.lines().collect();

    // Should have header + 50 data lines
    assert_eq!(lines.len(), 51, "Should have 1 header + 50 reservations");
}
