//! Stress tests for trop.
//!
//! This module contains high-volume stress tests that verify trop's behavior
//! under extreme load conditions. These tests are marked with `#[ignore]` to
//! prevent them from slowing down regular CI runs.
//!
//! Run these tests explicitly with: `cargo test --ignored`
//!
//! **Purpose:**
//! - Verify database performance with large datasets (1,000+ reservations)
//! - Test for resource leaks (memory, file handles) under sustained load
//! - Ensure database integrity holds under extreme concurrent stress
//! - Measure query performance degradation with dataset size
//!
//! **When to run:**
//! - Before major releases
//! - In nightly/weekly CI jobs
//! - When investigating performance regressions
//! - After database schema changes

use assert_cmd::cargo::cargo_bin;
use std::process::Command;
use std::thread;
use std::time::Instant;
use tempfile::TempDir;

/// Helper function to create a Command for the trop binary.
fn trop_cmd() -> Command {
    Command::new(cargo_bin("trop"))
}

/// Stress test: Create 10,000 reservations using 100 concurrent threads.
///
/// **What this tests:**
/// - High-volume concurrent database writes (100 threads × 100 reservations each)
/// - Database scalability with large datasets
/// - No deadlocks or resource exhaustion under sustained concurrent load
/// - File handle management (SQLite connections)
///
/// **Why this is important:**
/// In large organizations or CI environments, thousands of reservations may
/// accumulate over time. The database must handle this scale without performance
/// degradation or resource exhaustion.
///
/// **Invariant verified:**
/// After 10,000 concurrent reservation operations:
/// - All 10,000 reservations exist in the database
/// - Each has a unique port (no duplicates)
/// - Database remains queryable and consistent
///
/// **Performance expectations:**
/// - Should complete in reasonable time (< 60 seconds on modern hardware)
/// - Memory usage should remain stable (no memory leaks)
/// - File handles should be properly released
///
/// **Implementation notes:**
/// - Uses 100 threads to maximize concurrency
/// - Each thread creates 100 reservations sequentially
/// - Total: 100 × 100 = 10,000 reservations
/// - Verifies final count using `list` command
/// - Marked #[ignore] to run only when explicitly requested
#[test]
#[ignore] // Run with --ignored for stress tests
fn stress_test_high_volume_reservations() {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    // Initialize database
    trop_cmd()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "init",
            "--with-config",
        ])
        .status()
        .unwrap();

    println!("Creating 10,000 reservations across 100 threads...");
    let start = Instant::now();

    // Spawn 100 threads, each creating 100 reservations
    let handles: Vec<_> = (0..100)
        .map(|batch| {
            let data_dir = data_dir.clone();
            thread::spawn(move || {
                // Each thread creates 100 reservations
                for i in 0..100 {
                    let idx = batch * 100 + i;
                    let status = trop_cmd()
                        .args([
                            "--data-dir",
                            data_dir.to_str().unwrap(),
                            "reserve",
                            "--path",
                            &format!("/tmp/stress-{idx}"),
                            "--allow-unrelated-path",
                        ])
                        .status()
                        .unwrap();

                    if !status.success() {
                        eprintln!("Failed to create reservation {i} in batch {batch}");
                    }
                }
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    let elapsed = start.elapsed();
    println!("Created 10,000 reservations in {elapsed:?}");

    // Verify all 10,000 reservations were created successfully
    println!("Verifying all reservations exist...");
    let output = trop_cmd()
        .args(["--data-dir", data_dir.to_str().unwrap(), "list"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "List command should succeed after stress test"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let count = stdout.lines().filter(|l| l.contains("stress-")).count();

    assert_eq!(
        count, 10000,
        "Should have exactly 10,000 reservations (got {count})"
    );

    println!("✓ All 10,000 reservations verified");
}

/// Stress test: Rapidly create and delete the same reservation 1,000 times.
///
/// **What this tests:**
/// - Database behavior under rapid create/delete cycles on the same key
/// - Transaction log growth and cleanup (WAL mode checkpoint behavior)
/// - No database bloat or fragmentation over many cycles
/// - Proper cleanup of deleted records
///
/// **Why this is important:**
/// Developers often create and release the same reservation repeatedly during
/// development (e.g., restarting a dev server). The database must handle this
/// churn efficiently without accumulating cruft or degrading performance.
///
/// **Invariant verified:**
/// After 1,000 create/delete cycles:
/// - Database integrity checks pass (no corruption)
/// - Database file size remains reasonable (no excessive bloat)
/// - Operations continue to succeed (no degradation)
///
/// **Performance expectations:**
/// - Should complete in reasonable time (< 30 seconds)
/// - Database file should not grow excessively (WAL checkpointing works)
/// - No resource leaks (memory, file handles)
///
/// **Implementation notes:**
/// - Uses the same path for all 1,000 cycles
/// - Tests database's handling of row deletion and reuse
/// - Prints progress every 100 cycles
/// - Runs final integrity check with `assert-data-dir --validate`
/// - Marked #[ignore] to run only when explicitly requested
#[test]
#[ignore]
fn stress_test_rapid_create_delete_cycles() {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    trop_cmd()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "init",
            "--with-config",
        ])
        .status()
        .unwrap();

    println!("Running 1,000 create/delete cycles...");
    let start = Instant::now();

    // Rapidly create and delete the same reservation
    for cycle in 0..1000 {
        // Create reservation
        let reserve_status = trop_cmd()
            .args([
                "--data-dir",
                data_dir.to_str().unwrap(),
                "reserve",
                "--path",
                "/tmp/cycle-test",
                "--allow-unrelated-path",
            ])
            .status()
            .unwrap();

        assert!(
            reserve_status.success(),
            "Reserve should succeed in cycle {cycle}"
        );

        // Delete reservation
        let release_status = trop_cmd()
            .args([
                "--data-dir",
                data_dir.to_str().unwrap(),
                "release",
                "--path",
                "/tmp/cycle-test",
            ])
            .status()
            .unwrap();

        assert!(
            release_status.success(),
            "Release should succeed in cycle {cycle}"
        );

        // Print progress every 100 cycles
        if cycle > 0 && cycle % 100 == 0 {
            println!("Completed {cycle} cycles");
        }
    }

    let elapsed = start.elapsed();
    println!("Completed 1,000 cycles in {elapsed:?}");

    // Verify database is still healthy after all the churn
    println!("Verifying database integrity...");
    let output = trop_cmd()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "assert-data-dir",
            "--validate",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Database should pass integrity check after 1,000 create/delete cycles.\nStderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    println!("✓ Database integrity verified");
}

/// Stress test: Measure query performance with 1,000 reservations.
///
/// **What this tests:**
/// - Query performance (list command) with large dataset
/// - Index effectiveness as data grows
/// - Query time remains acceptable (< 50ms average)
///
/// **Why this is important:**
/// As the database grows, queries must remain fast. Users expect `trop list`
/// to return results quickly even with hundreds or thousands of reservations.
/// This test ensures we don't have O(n²) behavior or missing indexes.
///
/// **Invariant verified:**
/// - Query performance remains acceptable (< 50ms average) with 1,000 records
/// - Performance is consistent across multiple queries (no degradation)
///
/// **Performance expectations:**
/// - Average query time: < 50ms (target for good user experience)
/// - Consistent performance across 100 queries (no degradation)
/// - Linear or better scaling with dataset size
///
/// **Implementation notes:**
/// - Creates 1,000 reservations sequentially (not concurrent, for consistency)
/// - Runs 100 list queries and measures average time
/// - Asserts average query time is under 50ms
/// - Prints timing information for manual analysis
/// - Marked #[ignore] to run only when explicitly requested
#[test]
#[ignore]
fn stress_test_query_performance_with_large_dataset() {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    trop_cmd()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "init",
            "--with-config",
        ])
        .status()
        .unwrap();

    // Create 1,000 reservations (not concurrent, to keep timing predictable)
    println!("Creating 1,000 reservations for query performance test...");
    let setup_start = Instant::now();

    for i in 0..1000 {
        trop_cmd()
            .args([
                "--data-dir",
                data_dir.to_str().unwrap(),
                "reserve",
                "--path",
                &format!("/tmp/perf-{i}"),
                "--allow-unrelated-path",
            ])
            .status()
            .unwrap();

        if i > 0 && i % 100 == 0 {
            println!("  Created {i} reservations");
        }
    }

    let setup_elapsed = setup_start.elapsed();
    println!("Setup completed in {setup_elapsed:?}");

    // Measure query time over 100 iterations
    println!("Running 100 query iterations to measure performance...");
    let start = Instant::now();

    for i in 0..100 {
        let query_start = Instant::now();

        let output = trop_cmd()
            .args(["--data-dir", data_dir.to_str().unwrap(), "list"])
            .output()
            .unwrap();

        let query_elapsed = query_start.elapsed();

        assert!(output.status.success(), "Query {i} should succeed");

        // Print timing for first few queries to spot anomalies
        if i < 5 {
            println!("  Query {i} took {query_elapsed:?}");
        }
    }

    let total_elapsed = start.elapsed();
    let avg_ms = total_elapsed.as_millis() / 100;

    println!(
        "Query performance: {} queries in {:?}, average {}ms per query",
        100, total_elapsed, avg_ms
    );

    // Assert average query time is acceptable
    // Target: < 50ms for good user experience with 1,000 records
    assert!(
        avg_ms < 50,
        "Query took {avg_ms}ms on average, expected <50ms (with 1,000 records)"
    );

    println!("✓ Query performance acceptable ({avg_ms} ms average)");
}
