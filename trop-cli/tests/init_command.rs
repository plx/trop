//! Comprehensive integration tests for the `init` command.
//!
//! These tests verify all aspects of database initialization, including:
//! - Fresh initialization in empty directory
//! - Existing directory handling
//! - Existing database error handling
//! - Overwrite mode (--overwrite flag)
//! - Config file creation (--with-config flag)
//! - Config file preservation (not overwriting existing)
//! - Dry-run mode (--dry-run flag)
//! - Custom data-dir handling (--data-dir flag)
//! - Global data-dir flag respect
//! - Database validation (created database is functional)

mod common;

use common::TestEnv;
use std::fs;

// ============================================================================
// Basic Initialization Tests
// ============================================================================

/// Test fresh initialization in empty directory.
///
/// When init is run in an empty location, it should:
/// - Create the data directory if it doesn't exist
/// - Create the database file (trop.db)
/// - Initialize the database schema
/// - Report success
///
/// This is the most common use case: setting up trop for the first time.
#[test]
fn test_init_fresh_initialization() {
    let env = TestEnv::new();

    // Data directory should not exist yet
    assert!(
        !env.data_dir.exists(),
        "Data directory should not exist initially"
    );

    // Run init
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Init should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should report what was created
    assert!(
        stdout.contains("Initialized trop"),
        "Should report initialization: {stdout}"
    );
    assert!(
        stdout.contains("Created data directory") || stdout.contains("Created database"),
        "Should report what was created: {stdout}"
    );

    // Data directory should now exist
    assert!(env.data_dir.exists(), "Data directory should be created");

    // Database file should exist
    let db_path = env.data_dir.join("trop.db");
    assert!(db_path.exists(), "Database file should be created");
}

/// Test initialization when directory already exists.
///
/// If the data directory exists but is empty (no database), init should:
/// - Not fail
/// - Create the database in the existing directory
/// - Report that it created the database
///
/// This handles the case where the directory was manually created.
#[test]
fn test_init_existing_directory() {
    let env = TestEnv::new();

    // Create the directory manually (but no database)
    fs::create_dir_all(&env.data_dir).expect("Failed to create directory");
    assert!(env.data_dir.exists());

    let db_path = env.data_dir.join("trop.db");
    assert!(!db_path.exists(), "Database should not exist yet");

    // Run init
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Init should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should report database creation (but not directory creation)
    assert!(
        stdout.contains("Created database"),
        "Should report database creation: {stdout}"
    );

    // Should NOT say it created the directory (it already existed)
    assert!(
        !stdout.contains("Created data directory"),
        "Should not claim to create existing directory: {stdout}"
    );

    // Database should now exist
    assert!(db_path.exists(), "Database should be created");
}

// ============================================================================
// Existing Database Error Handling
// ============================================================================

/// Test error when database already exists (without --overwrite).
///
/// If a database already exists and --overwrite is not specified, init should:
/// - Fail with an error
/// - Not modify the existing database
/// - Provide a helpful error message mentioning --overwrite
///
/// This prevents accidentally destroying existing data.
#[test]
fn test_init_existing_database_error() {
    let env = TestEnv::new();

    // Initialize once
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    let db_path = env.data_dir.join("trop.db");
    assert!(db_path.exists());

    // Get the original database's modification time
    let original_metadata = fs::metadata(&db_path).expect("Failed to get metadata");
    let original_modified = original_metadata.modified().expect("Failed to get mtime");

    // Try to init again without --overwrite
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .output()
        .expect("Failed to run init");

    // Should fail
    assert!(
        !output.status.success(),
        "Init should fail when database exists"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Error message should mention the problem
    assert!(
        stderr.contains("already exists") || stderr.contains("exist"),
        "Error should mention database exists: {stderr}"
    );

    // Database should be unchanged (check modification time)
    let new_metadata = fs::metadata(&db_path).expect("Failed to get metadata");
    let new_modified = new_metadata.modified().expect("Failed to get mtime");
    assert_eq!(
        original_modified, new_modified,
        "Database should not be modified"
    );
}

// ============================================================================
// Overwrite Mode Tests
// ============================================================================

/// Test successful recreation with --overwrite.
///
/// When --overwrite is specified and a database exists, init should:
/// - Remove the old database
/// - Create a fresh database
/// - Report that it recreated the database
///
/// This is useful for resetting trop to a clean state.
#[test]
fn test_init_overwrite_mode() {
    let env = TestEnv::new();

    // Initialize once and create a reservation
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    let test_path = env.create_dir("test");
    env.reserve_simple(&test_path);

    // Verify reservation exists
    let list_before = env.list();
    let lines_before: Vec<&str> = list_before.lines().collect();
    assert!(
        lines_before.len() > 1, // More than just header
        "Should have a reservation"
    );

    // Init with --overwrite
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .output()
        .expect("Failed to run init");

    assert!(
        output.status.success(),
        "Init with overwrite should succeed"
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should report recreation
    assert!(
        stdout.contains("Recreated database") || stdout.contains("Created database"),
        "Should report database recreation: {stdout}"
    );

    // Database should be empty now (reservation should be gone)
    let list_after = env.list();
    let lines_after: Vec<&str> = list_after.lines().collect();
    assert_eq!(
        lines_after.len(),
        1,
        "After overwrite, should only have header (no reservations)"
    );
}

/// Test --overwrite with non-existent database is harmless.
///
/// If --overwrite is specified but no database exists, init should:
/// - Succeed (not fail)
/// - Create the database normally
/// - Not report an error
///
/// This ensures --overwrite is safe to use unconditionally in scripts.
#[test]
fn test_init_overwrite_nonexistent_database() {
    let env = TestEnv::new();

    // No database exists yet
    let db_path = env.data_dir.join("trop.db");
    assert!(!db_path.exists());

    // Init with --overwrite (even though nothing to overwrite)
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .output()
        .expect("Failed to run init");

    assert!(
        output.status.success(),
        "Init with overwrite should succeed even if nothing to overwrite"
    );

    // Database should be created
    assert!(db_path.exists(), "Database should be created");
}

// ============================================================================
// Config File Creation Tests
// ============================================================================

/// Test --with-config creates config file.
///
/// When --with-config is specified, init should:
/// - Create config.yaml in the data directory
/// - Populate it with default/template content
/// - Report that it created the config file
///
/// This helps users get started with configuration.
#[test]
fn test_init_creates_config_file() {
    let env = TestEnv::new();

    // Init with --with-config
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--with-config")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Init should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should report config creation
    assert!(
        stdout.contains("Created default configuration file")
            || stdout.contains("Created") && stdout.contains("config"),
        "Should report config creation: {stdout}"
    );

    // Config file should exist
    let config_path = env.data_dir.join("config.yaml");
    assert!(config_path.exists(), "Config file should be created");

    // Config should contain YAML content (basic validation)
    let config_content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert!(!config_content.is_empty(), "Config should have content");

    // Should look like YAML (basic check)
    assert!(
        config_content.contains(':') || config_content.contains('#'),
        "Config should contain YAML syntax: {config_content}"
    );
}

/// Test --with-config preserves existing config file.
///
/// If config.yaml already exists, init --with-config should:
/// - Not overwrite the existing file
/// - Report that the config already exists
/// - Still succeed (not fail)
///
/// This prevents accidentally destroying user configuration.
#[test]
fn test_init_preserves_existing_config() {
    let env = TestEnv::new();

    // Create data directory and config file
    fs::create_dir_all(&env.data_dir).expect("Failed to create directory");
    let config_path = env.data_dir.join("config.yaml");
    let original_content = "# My custom config\nport_range: 10000-20000\n";
    fs::write(&config_path, original_content).expect("Failed to write config");

    // Init with --with-config
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--with-config")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Init should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should report that config already exists
    assert!(
        stdout.contains("already exists") || stdout.contains("not overwritten"),
        "Should report existing config: {stdout}"
    );

    // Config should be unchanged
    let config_content = fs::read_to_string(&config_path).expect("Failed to read config");
    assert_eq!(
        config_content, original_content,
        "Config should not be modified"
    );
}

/// Test init without --with-config doesn't create config.
///
/// By default (without --with-config), init should:
/// - Not create config.yaml
/// - Only create the database
///
/// This is the normal behavior: config is optional.
#[test]
fn test_init_without_config_flag() {
    let env = TestEnv::new();

    // Init without --with-config
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Database should exist
    let db_path = env.data_dir.join("trop.db");
    assert!(db_path.exists(), "Database should be created");

    // Config should NOT exist
    let config_path = env.data_dir.join("config.yaml");
    assert!(
        !config_path.exists(),
        "Config should not be created without --with-config"
    );
}

// ============================================================================
// Dry-Run Mode Tests
// ============================================================================

/// Test --dry-run shows planned actions without executing.
///
/// In dry-run mode, init should:
/// - Show what it would do
/// - Not actually create any files
/// - Exit successfully
///
/// This allows users to preview the init operation.
#[test]
fn test_init_dry_run_mode() {
    let env = TestEnv::new();

    // Run init with --dry-run
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--dry-run")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Dry-run should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should mention dry-run mode
    assert!(
        stdout.contains("Dry-run") || stdout.contains("dry-run"),
        "Should indicate dry-run mode: {stdout}"
    );

    // Should describe what would be done
    assert!(
        stdout.contains("Would") || stdout.contains("would"),
        "Should describe planned actions: {stdout}"
    );

    // Should mention the data directory path
    assert!(
        stdout.contains(&env.data_dir.to_string_lossy().to_string()),
        "Should mention data directory: {stdout}"
    );

    // Nothing should actually be created
    assert!(
        !env.data_dir.exists(),
        "Dry-run should not create data directory"
    );

    let db_path = env.data_dir.join("trop.db");
    assert!(!db_path.exists(), "Dry-run should not create database");
}

/// Test --dry-run with existing database and --overwrite.
///
/// Dry-run should show that it would recreate the database, but not actually
/// do so. This tests the interaction between flags in dry-run mode.
#[test]
fn test_init_dry_run_with_overwrite() {
    let env = TestEnv::new();

    // Create existing database
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    let db_path = env.data_dir.join("trop.db");
    let original_metadata = fs::metadata(&db_path).expect("Failed to get metadata");
    let original_modified = original_metadata.modified().expect("Failed to get mtime");

    // Dry-run with --overwrite
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--dry-run")
        .arg("--overwrite")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Dry-run should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should mention removing and recreating
    assert!(
        stdout.contains("Remove") || stdout.contains("remove"),
        "Should mention removing old database: {stdout}"
    );

    // Database should be unchanged
    let new_metadata = fs::metadata(&db_path).expect("Failed to get metadata");
    let new_modified = new_metadata.modified().expect("Failed to get mtime");
    assert_eq!(
        original_modified, new_modified,
        "Dry-run should not modify database"
    );
}

/// Test --dry-run with --with-config.
///
/// Dry-run should show that it would create config, but not actually do so.
#[test]
fn test_init_dry_run_with_config() {
    let env = TestEnv::new();

    // Dry-run with --with-config
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--dry-run")
        .arg("--with-config")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Dry-run should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should mention config creation
    assert!(
        stdout.contains("configuration") || stdout.contains("config"),
        "Should mention config: {stdout}"
    );

    // Config should not be created
    let config_path = env.data_dir.join("config.yaml");
    assert!(!config_path.exists(), "Dry-run should not create config");
}

/// Test --dry-run shows error for existing database (without --overwrite).
///
/// Even in dry-run mode, we should indicate what the error would be.
#[test]
fn test_init_dry_run_shows_error_for_existing_db() {
    let env = TestEnv::new();

    // Create database
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Dry-run without --overwrite (should show error)
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--dry-run")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Dry-run should succeed");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should mention an error would occur
    assert!(
        stdout.contains("ERROR") || stdout.contains("already exists"),
        "Should show error would occur: {stdout}"
    );
}

// ============================================================================
// Data Directory Flag Tests
// ============================================================================

/// Test init with command-line --data-dir flag.
///
/// The init command's --data-dir flag should specify where to initialize.
/// This is the primary way to specify a custom location.
#[test]
fn test_init_custom_data_dir_flag() {
    let env = TestEnv::new();
    let custom_dir = env.temp_path.join("custom-trop-location");

    // Init with custom path
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&custom_dir)
        .assert()
        .success();

    // Custom location should be created
    assert!(custom_dir.exists(), "Custom directory should be created");

    let db_path = custom_dir.join("trop.db");
    assert!(db_path.exists(), "Database should be in custom location");

    // Default location should NOT be created
    assert!(
        !env.data_dir.exists(),
        "Default directory should not be created"
    );
}

/// Test init respects global --data-dir flag.
///
/// The global flag (before the subcommand) should also work.
/// Priority is: init's --data-dir > global --data-dir > default.
#[test]
fn test_init_respects_global_data_dir() {
    let env = TestEnv::new();
    let custom_dir = env.temp_path.join("global-custom");

    // Use global --data-dir flag
    env.command_bare()
        .arg("--data-dir")
        .arg(&custom_dir)
        .arg("init")
        .assert()
        .success();

    // Custom location should be created
    assert!(custom_dir.exists(), "Custom directory should be created");

    let db_path = custom_dir.join("trop.db");
    assert!(db_path.exists(), "Database should be in custom location");
}

/// Test init command flag overrides global flag.
///
/// If both global and command flags are specified, the command flag should win.
/// This tests the priority: command > global > default.
#[test]
fn test_init_command_flag_overrides_global() {
    let env = TestEnv::new();
    let global_dir = env.temp_path.join("global-dir");
    let command_dir = env.temp_path.join("command-dir");

    // Specify both global and command flags
    env.command_bare()
        .arg("--data-dir")
        .arg(&global_dir)
        .arg("init")
        .arg("--data-dir")
        .arg(&command_dir)
        .assert()
        .success();

    // Command flag location should be used
    assert!(
        command_dir.exists(),
        "Command flag directory should be created"
    );
    let db_path = command_dir.join("trop.db");
    assert!(
        db_path.exists(),
        "Database should be in command flag location"
    );

    // Global flag location should NOT be created
    assert!(
        !global_dir.exists(),
        "Global flag directory should not be created (overridden)"
    );
}

// ============================================================================
// Database Validation Tests
// ============================================================================

/// Test created database is functional.
///
/// After init, the database should be fully functional:
/// - Can connect and query
/// - Can create reservations
/// - Has proper schema
///
/// This is an end-to-end validation that init actually works.
#[test]
fn test_init_database_is_functional() {
    let env = TestEnv::new();

    // Initialize
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Should be able to reserve a port
    let test_path = env.create_dir("test-project");
    let port = env.reserve_simple(&test_path);

    // Port should be valid
    assert!(port >= 1024, "Port should be valid: {port}");

    // Should be able to list reservations
    let list_output = env.list();
    assert!(
        list_output.contains(&port.to_string()),
        "Should be able to list reservations"
    );

    // Should be able to release
    env.release(&test_path);

    let list_after_release = env.list();
    assert!(
        !list_after_release.contains(&port.to_string()),
        "Should be able to release reservations"
    );
}

/// Test init creates all necessary tables.
///
/// The database should have all required tables for trop to function.
/// We verify this by checking that various operations work.
#[test]
fn test_init_creates_complete_schema() {
    let env = TestEnv::new();

    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    let test_path = env.create_dir("test");

    // Test reservations table exists and works
    let port = env.reserve_simple(&test_path);
    assert!(port > 0);

    // Test we can query with filters (list command)
    env.command()
        .arg("list")
        .arg("--filter-path")
        .arg(&test_path)
        .assert()
        .success();

    // Test groups functionality (reserve-group command)
    let group_path = env.create_dir("group-test");
    let config_path = group_path.join("trop.yaml");
    std::fs::write(
        &config_path,
        "ports:\n  min: 5000\n  max: 9000\nreservations:\n  base: 8000\n  services:\n    web:\n      offset: 0\n      env: WEB_PORT\n    api:\n      offset: 1\n      env: API_PORT\n",
    )
    .expect("Failed to write config");

    env.command()
        .arg("reserve-group")
        .arg(&config_path)
        .arg("--allow-unrelated-path")
        .assert()
        .success();

    // All operations should work if schema is complete
}

/// Test init with overwrite preserves schema integrity.
///
/// After overwriting a database, it should have the same schema as a fresh init.
/// This verifies that overwrite truly recreates the database properly.
#[test]
fn test_init_overwrite_preserves_schema() {
    let env = TestEnv::new();

    // Initialize once
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Add some data
    let path1 = env.create_dir("test1");
    env.reserve_simple(&path1);

    // Overwrite
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .assert()
        .success();

    // Database should still be fully functional
    let path2 = env.create_dir("test2");
    let port = env.reserve_simple(&path2);
    assert!(port > 0, "Database should be functional after overwrite");

    // Old data should be gone
    let list = env.list();
    assert!(
        !list.contains(path1.to_str().unwrap()),
        "Old data should be cleared"
    );
}

// ============================================================================
// Output Messages Tests
// ============================================================================

/// Test init output is user-friendly.
///
/// The output should clearly communicate what was done, helping users
/// understand the initialization process.
#[test]
fn test_init_output_is_clear() {
    let env = TestEnv::new();

    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .output()
        .expect("Failed to run init");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should show where it initialized
    assert!(
        stdout.contains("Initialized")
            && stdout.contains(&env.data_dir.to_string_lossy().to_string()),
        "Should show initialization location: {stdout}"
    );

    // Should list what was created
    assert!(
        stdout.contains("Created") || stdout.contains("created"),
        "Should list created items: {stdout}"
    );

    // Output should be concise (not too verbose)
    let line_count = stdout.lines().count();
    assert!(
        line_count <= 10,
        "Output should be concise (was {line_count} lines)"
    );
}

// ============================================================================
// Error Cases
// ============================================================================

/// Test init with invalid path fails gracefully.
///
/// If the data directory path is invalid (e.g., permission denied),
/// init should fail with a clear error message.
#[test]
#[cfg(unix)] // This test is Unix-specific (permission handling differs on Windows)
fn test_init_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let env = TestEnv::new();

    // Skip this test when running as root. A privileged user can bypass
    // the permission restrictions we set up here, which would cause the
    // initialization to succeed unexpectedly and lead to false negatives
    // in environments (like some CI runners) that execute the tests as
    // root.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipping test_init_permission_denied as root user");
        return;
    }

    // Create a directory with no write permission
    let readonly_parent = env.temp_path.join("readonly");
    fs::create_dir_all(&readonly_parent).expect("Failed to create directory");
    let mut perms = fs::metadata(&readonly_parent)
        .expect("Failed to get metadata")
        .permissions();
    perms.set_mode(0o444); // Read-only
    fs::set_permissions(&readonly_parent, perms).expect("Failed to set permissions");

    let inaccessible_dir = readonly_parent.join("trop-data");

    // Try to init in inaccessible location
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&inaccessible_dir)
        .output()
        .expect("Failed to run init");

    // Should fail
    assert!(
        !output.status.success(),
        "Init should fail with permission denied"
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8");

    // Error should be meaningful
    assert!(!stderr.is_empty(), "Should have error message");

    // Clean up: restore permissions so tempdir can be deleted
    let mut restore_perms = fs::metadata(&readonly_parent)
        .expect("Failed to get metadata")
        .permissions();
    restore_perms.set_mode(0o755);
    fs::set_permissions(&readonly_parent, restore_perms).expect("Failed to restore permissions");
}

// ============================================================================
// Flag Combination Tests
// ============================================================================

/// Test all flags together.
///
/// Combining --overwrite, --with-config, and --dry-run should work correctly.
/// This tests that flag parsing and handling works for complex combinations.
#[test]
fn test_init_all_flags_together() {
    let env = TestEnv::new();

    // Create existing database
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Run with all flags
    let output = env
        .command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .arg("--with-config")
        .arg("--dry-run")
        .output()
        .expect("Failed to run init");

    assert!(output.status.success(), "Should handle all flags");

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8");

    // Should mention dry-run
    assert!(
        stdout.contains("Dry-run"),
        "Should show dry-run mode: {stdout}"
    );

    // Should mention overwrite
    assert!(
        stdout.contains("Remove") || stdout.contains("overwrite"),
        "Should mention overwrite: {stdout}"
    );

    // Should mention config
    assert!(stdout.contains("config"), "Should mention config: {stdout}");

    // Nothing should actually change (dry-run)
    let config_path = env.data_dir.join("config.yaml");
    assert!(
        !config_path.exists(),
        "Config should not be created in dry-run"
    );
}

// ============================================================================
// Integration Tests
// ============================================================================

/// Test init followed by normal usage.
///
/// A complete workflow: init, reserve, list, release should all work.
/// This validates that init properly sets up trop for real use.
#[test]
fn test_init_followed_by_normal_usage() {
    let env = TestEnv::new();

    // Step 1: Initialize
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    // Step 2: Reserve some ports
    let path1 = env.create_dir("project1");
    let path2 = env.create_dir("project2");

    let port1 = env.reserve_simple(&path1);
    let port2 = env.reserve_simple(&path2);

    // Step 3: List them
    let list_output = env.list();
    assert!(list_output.contains(&port1.to_string()));
    assert!(list_output.contains(&port2.to_string()));

    // Step 4: Release one
    env.release(&path1);

    // Step 5: List again
    let list_output2 = env.list();
    assert!(!list_output2.contains(&port1.to_string()));
    assert!(list_output2.contains(&port2.to_string()));

    // Everything should work smoothly
}

/// Test init can be used to reset trop state.
///
/// Init with --overwrite provides a way to clear all reservations.
/// This tests that workflow.
#[test]
fn test_init_reset_workflow() {
    let env = TestEnv::new();

    // Setup: initialize and create reservations
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .assert()
        .success();

    for i in 0..5 {
        let path = env.create_dir(&format!("project{i}"));
        env.reserve_simple(&path);
    }

    // Verify we have reservations
    let list_before = env.list();
    let lines_before: Vec<&str> = list_before.lines().collect();
    assert!(lines_before.len() > 1, "Should have multiple reservations");

    // Reset with init --overwrite
    env.command_bare()
        .arg("init")
        .arg("--data-dir")
        .arg(&env.data_dir)
        .arg("--overwrite")
        .assert()
        .success();

    // Everything should be cleared
    let list_after = env.list();
    let lines_after: Vec<&str> = list_after.lines().collect();
    assert_eq!(lines_after.len(), 1, "After reset, should only have header");
}
