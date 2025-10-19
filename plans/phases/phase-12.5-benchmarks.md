# Phase 12.5: Performance Benchmarks

## Overview

Subpass 12.5 adds comprehensive performance benchmarks for critical operations in trop. This establishes baseline performance metrics and enables detection of performance regressions in future changes.

## Context & Dependencies

**Prerequisites:**
- One benchmark suite already exists (`trop/benches/path_bench.rs`)
- Criterion framework already in use for benchmarking
- All core operations from Phases 1-11 implemented

**Dependencies:**
- Phase 12.2 (Concurrent Tests) should complete first to understand performance characteristics under load

**Key Considerations:**
- Focus on operations that are on the critical path (reserve, get, list)
- Benchmarks should be reproducible across runs
- Establish baseline metrics before future optimizations
- Some benchmarks may need to be gated for CI (too slow)

## Performance Targets

Based on the implementation specification, we aim for:

- **Reservation creation:** < 10ms
- **Port allocation:** < 1ms
- **Database query (indexed):** < 5ms
- **CLI startup:** < 50ms
- **List operation (100 reservations):** < 20ms
- **List operation (10,000 reservations):** < 500ms

## Implementation Tasks

### Task 1: Core Operations Benchmarks

**File:** `trop/benches/operations_bench.rs` (create new)

**Implementation:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use trop::database::{Database, DatabaseConfig};
use trop::operations::reserve::{ReserveOptions, reserve_port};
use trop::operations::release::release_reservation;
use trop::operations::list::list_reservations;
use trop::port::Port;
use trop::config::Config;
use tempfile::TempDir;
use std::path::PathBuf;

fn setup_database() -> (TempDir, Database) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("trop.db");
    let config = DatabaseConfig::new(db_path);
    let db = Database::open(&config).unwrap();
    (temp_dir, db)
}

fn bench_single_reservation(c: &mut Criterion) {
    let (_temp, mut db) = setup_database();
    let config = Config::default();

    c.bench_function("reserve_single", |b| {
        let mut counter = 0;
        b.iter(|| {
            let path = PathBuf::from(format!("/tmp/bench-{}", counter));
            let opts = ReserveOptions {
                path: path.clone(),
                project: Some(format!("bench-project-{}", counter)),
                task: None,
                port: None,
                allow_unrelated_path: true,
                ..Default::default()
            };

            let result = reserve_port(&mut db, &config, &opts);
            counter += 1;
            black_box(result)
        });
    });
}

fn bench_bulk_reservations(c: &mut Criterion) {
    let mut group = c.benchmark_group("reserve_bulk");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.iter(|| {
                let (_temp, mut db) = setup_database();
                let config = Config::default();

                for i in 0..size {
                    let path = PathBuf::from(format!("/tmp/bulk-{}", i));
                    let opts = ReserveOptions {
                        path,
                        project: Some(format!("bulk-{}", i)),
                        task: None,
                        port: None,
                        allow_unrelated_path: true,
                        ..Default::default()
                    };
                    reserve_port(&mut db, &config, &opts).unwrap();
                }
            });
        });
    }

    group.finish();
}

fn bench_reservation_lookup(c: &mut Criterion) {
    // Create database with varying numbers of reservations
    let mut group = c.benchmark_group("reservation_lookup");

    for size in [10, 100, 1000, 10000].iter() {
        let (_temp, mut db) = setup_database();
        let config = Config::default();

        // Pre-populate database
        for i in 0..*size {
            let path = PathBuf::from(format!("/tmp/lookup-{}", i));
            let opts = ReserveOptions {
                path,
                project: Some(format!("lookup-{}", i)),
                task: None,
                port: None,
                allow_unrelated_path: true,
                ..Default::default()
            };
            reserve_port(&mut db, &config, &opts).unwrap();
        }

        // Benchmark lookup by path (should use index)
        group.bench_with_input(BenchmarkId::new("by_path", size), size, |b, _| {
            let lookup_path = PathBuf::from(format!("/tmp/lookup-{}", size / 2));
            b.iter(|| {
                let result = db.get_reservation_by_path(&lookup_path);
                black_box(result)
            });
        });

        // Benchmark lookup by port (should use index)
        let sample_port = {
            let path = PathBuf::from(format!("/tmp/lookup-{}", size / 2));
            db.get_reservation_by_path(&path).unwrap().unwrap().port
        };

        group.bench_with_input(BenchmarkId::new("by_port", size), size, |b, _| {
            b.iter(|| {
                let result = db.get_reservation_by_port(&sample_port);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_list_reservations(c: &mut Criterion) {
    let mut group = c.benchmark_group("list_reservations");

    for size in [10, 100, 1000, 10000].iter() {
        let (_temp, mut db) = setup_database();
        let config = Config::default();

        // Pre-populate
        for i in 0..*size {
            let path = PathBuf::from(format!("/tmp/list-{}", i));
            let opts = ReserveOptions {
                path,
                project: Some(format!("project-{}", i % 10)), // 10 different projects
                task: None,
                port: None,
                allow_unrelated_path: true,
                ..Default::default()
            };
            reserve_port(&mut db, &config, &opts).unwrap();
        }

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let result = list_reservations(&db, None, None);
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_port_allocation(c: &mut Criterion) {
    use trop::operations::allocate::allocate_port;
    use std::collections::HashSet;

    let config = Config::builder()
        .port_range(50000, 60000)
        .build()
        .unwrap();

    let mut group = c.benchmark_group("port_allocation");

    // Benchmark allocation with varying numbers of already-allocated ports
    for allocated_count in [0, 100, 1000, 5000].iter() {
        let mut allocated = HashSet::new();
        for i in 0..*allocated_count {
            allocated.insert(Port::new(50000 + i as u16).unwrap());
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(allocated_count),
            allocated_count,
            |b, _| {
                b.iter(|| {
                    let result = allocate_port(&config, &allocated);
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn bench_group_allocation(c: &mut Criterion) {
    use trop::operations::allocate::allocate_port_group;
    use std::collections::HashSet;

    let config = Config::builder()
        .port_range(50000, 60000)
        .build()
        .unwrap();

    let mut group = c.benchmark_group("group_allocation");

    for group_size in [5, 10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(group_size),
            group_size,
            |b, &size| {
                b.iter(|| {
                    let allocated = HashSet::new();
                    let result = allocate_port_group(&config, &allocated, size);
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn bench_cleanup_operations(c: &mut Criterion) {
    let (_temp, mut db) = setup_database();
    let config = Config::default();

    // Create 100 reservations
    for i in 0..100 {
        let path = PathBuf::from(format!("/tmp/cleanup-{}", i));
        let opts = ReserveOptions {
            path,
            project: Some(format!("cleanup-{}", i)),
            task: None,
            port: None,
            allow_unrelated_path: true,
            ..Default::default()
        };
        reserve_port(&mut db, &config, &opts).unwrap();
    }

    c.bench_function("release_reservation", |b| {
        let mut counter = 0;
        b.iter(|| {
            let path = PathBuf::from(format!("/tmp/cleanup-{}", counter % 100));
            let result = release_reservation(&mut db, &path);
            counter += 1;
            black_box(result)
        });
    });
}

criterion_group!(
    benches,
    bench_single_reservation,
    bench_bulk_reservations,
    bench_reservation_lookup,
    bench_list_reservations,
    bench_port_allocation,
    bench_group_allocation,
    bench_cleanup_operations
);

criterion_main!(benches);
```

**Integration:**
Add to `trop/Cargo.toml`:
```toml
[[bench]]
name = "operations_bench"
harness = false
```

### Task 2: Database Benchmarks

**File:** `trop/benches/database_bench.rs` (create new)

**Implementation:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use trop::database::{Database, DatabaseConfig};
use trop::reservation::Reservation;
use trop::port::Port;
use tempfile::TempDir;
use std::path::PathBuf;

fn setup_populated_db(size: usize) -> (TempDir, Database) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("trop.db");
    let config = DatabaseConfig::new(db_path);
    let mut db = Database::open(&config).unwrap();

    // Populate with reservations
    for i in 0..size {
        let path = PathBuf::from(format!("/tmp/db-bench-{}", i));
        let port = Port::new((10000 + i as u16) % 65535 + 1).unwrap();
        db.insert_reservation(&path, port, Some(format!("project-{}", i % 10)), None, vec![])
            .unwrap();
    }

    (temp_dir, db)
}

fn bench_insert_query_performance(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_insert");

    for size in [10, 100, 1000].iter() {
        let (_temp, mut db) = setup_populated_db(*size);

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let mut counter = size;
            b.iter(|| {
                let path = PathBuf::from(format!("/tmp/insert-{}", counter));
                let port = Port::new(((20000 + counter) % 65535 + 1) as u16).unwrap();
                let result = db.insert_reservation(&path, port, Some(format!("p-{}", counter)), None, vec![]);
                counter += 1;
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_index_effectiveness(c: &mut Criterion) {
    // Benchmark queries that should use indexes vs those that don't
    let (_temp, db) = setup_populated_db(10000);

    let mut group = c.benchmark_group("index_effectiveness");

    // Indexed query: lookup by path
    group.bench_function("indexed_path_lookup", |b| {
        let path = PathBuf::from("/tmp/db-bench-5000");
        b.iter(|| {
            let result = db.get_reservation_by_path(&path);
            black_box(result)
        });
    });

    // Indexed query: lookup by port
    group.bench_function("indexed_port_lookup", |b| {
        let port = Port::new(15000).unwrap();
        b.iter(|| {
            let result = db.get_reservation_by_port(&port);
            black_box(result)
        });
    });

    // Non-indexed query: filter by project (may require full scan depending on implementation)
    group.bench_function("filter_by_project", |b| {
        b.iter(|| {
            let result = db.list_reservations_by_project("project-5");
            black_box(result)
        });
    });

    group.finish();
}

fn bench_transaction_overhead(c: &mut Criterion) {
    let (_temp, mut db) = setup_populated_db(100);

    let mut group = c.benchmark_group("transaction_overhead");

    // Single operation without explicit transaction
    group.bench_function("single_operation", |b| {
        let mut counter = 1000;
        b.iter(|| {
            let path = PathBuf::from(format!("/tmp/txn-single-{}", counter));
            let port = Port::new(((30000 + counter) % 65535 + 1) as u16).unwrap();
            db.insert_reservation(&path, port, None, None, vec![]).unwrap();
            counter += 1;
        });
    });

    // Multiple operations in explicit transaction
    group.bench_function("batched_transaction", |b| {
        let mut counter = 2000;
        b.iter(|| {
            let tx = db.begin_transaction().unwrap();

            for i in 0..10 {
                let path = PathBuf::from(format!("/tmp/txn-batch-{}-{}", counter, i));
                let port = Port::new(((40000 + counter + i) % 65535 + 1) as u16).unwrap();
                tx.insert_reservation(&path, port, None, None, vec![]).unwrap();
            }

            tx.commit().unwrap();
            counter += 10;
        });
    });

    group.finish();
}

fn bench_concurrent_reads(c: &mut Criterion) {
    use std::sync::Arc;
    use std::thread;

    let (_temp, db) = setup_populated_db(1000);
    let db = Arc::new(db);

    c.bench_function("concurrent_reads_10_threads", |b| {
        b.iter(|| {
            let handles: Vec<_> = (0..10)
                .map(|i| {
                    let db_clone = Arc::clone(&db);
                    thread::spawn(move || {
                        for j in 0..100 {
                            let path = PathBuf::from(format!("/tmp/db-bench-{}", (i * 100 + j) % 1000));
                            let _ = db_clone.get_reservation_by_path(&path);
                        }
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap();
            }
        });
    });
}

criterion_group!(
    benches,
    bench_insert_query_performance,
    bench_index_effectiveness,
    bench_transaction_overhead,
    bench_concurrent_reads
);

criterion_main!(benches);
```

**Integration:**
Add to `trop/Cargo.toml`:
```toml
[[bench]]
name = "database_bench"
harness = false
```

### Task 3: CLI Benchmarks

**File:** `trop-cli/benches/cli_bench.rs` (create new)

**Implementation:**
```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::process::Command;
use std::time::Instant;
use tempfile::TempDir;

fn bench_cli_startup(c: &mut Criterion) {
    // Measure time to just print version (minimal work)
    c.bench_function("cli_startup_version", |b| {
        b.iter(|| {
            let output = Command::new(env!("CARGO_BIN_EXE_trop"))
                .arg("--version")
                .output()
                .unwrap();
            black_box(output)
        });
    });
}

fn bench_cli_end_to_end_reserve(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    // Initialize once
    Command::new(env!("CARGO_BIN_EXE_trop"))
        .args(["--data-dir", data_dir.to_str().unwrap(), "init", "--with-config"])
        .status()
        .unwrap();

    c.bench_function("cli_reserve_e2e", |b| {
        let mut counter = 0;
        b.iter(|| {
            let output = Command::new(env!("CARGO_BIN_EXE_trop"))
                .args([
                    "--data-dir", data_dir.to_str().unwrap(),
                    "reserve",
                    "--path", &format!("/tmp/cli-bench-{}", counter),
                    "--allow-unrelated-path",
                ])
                .output()
                .unwrap();
            counter += 1;
            black_box(output)
        });
    });
}

fn bench_cli_list_performance(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    Command::new(env!("CARGO_BIN_EXE_trop"))
        .args(["--data-dir", data_dir.to_str().unwrap(), "init", "--with-config"])
        .status()
        .unwrap();

    // Create 1000 reservations
    for i in 0..1000 {
        Command::new(env!("CARGO_BIN_EXE_trop"))
            .args([
                "--data-dir", data_dir.to_str().unwrap(),
                "reserve",
                "--path", &format!("/tmp/list-bench-{}", i),
                "--allow-unrelated-path",
            ])
            .output()
            .unwrap();
    }

    c.bench_function("cli_list_1000_reservations", |b| {
        b.iter(|| {
            let output = Command::new(env!("CARGO_BIN_EXE_trop"))
                .args([
                    "--data-dir", data_dir.to_str().unwrap(),
                    "list",
                ])
                .output()
                .unwrap();
            black_box(output)
        });
    });
}

fn bench_output_formats(c: &mut Criterion) {
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");

    Command::new(env!("CARGO_BIN_EXE_trop"))
        .args(["--data-dir", data_dir.to_str().unwrap(), "init", "--with-config"])
        .status()
        .unwrap();

    // Create 100 reservations
    for i in 0..100 {
        Command::new(env!("CARGO_BIN_EXE_trop"))
            .args([
                "--data-dir", data_dir.to_str().unwrap(),
                "reserve",
                "--path", &format!("/tmp/format-bench-{}", i),
                "--allow-unrelated-path",
            ])
            .output()
            .unwrap();
    }

    let mut group = c.benchmark_group("output_formats");

    for format in ["table", "json", "csv"].iter() {
        group.bench_function(*format, |b| {
            b.iter(|| {
                let output = Command::new(env!("CARGO_BIN_EXE_trop"))
                    .args([
                        "--data-dir", data_dir.to_str().unwrap(),
                        "list",
                        "--format", format,
                    ])
                    .output()
                    .unwrap();
                black_box(output)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_cli_startup,
    bench_cli_end_to_end_reserve,
    bench_cli_list_performance,
    bench_output_formats
);

criterion_main!(benches);
```

**Integration:**
Create `trop-cli/benches/` directory and add to `trop-cli/Cargo.toml`:
```toml
[dev-dependencies]
criterion = "0.5"

[[bench]]
name = "cli_bench"
harness = false
```

## Running Benchmarks

```bash
# Run all benchmarks
cargo bench

# Run specific benchmark suite
cargo bench --bench operations_bench
cargo bench --bench database_bench
cargo bench --bench cli_bench

# Run and save baseline
cargo bench -- --save-baseline initial

# Compare against baseline
cargo bench -- --baseline initial

# Generate HTML report
cargo bench -- --plotting-backend gnuplot
```

## Success Criteria

- [ ] All critical paths benchmarked (reserve, get, list, allocate)
- [ ] Baseline performance documented in project README
- [ ] Benchmarks integrated into CI for regression detection
- [ ] Performance meets targets:
  - Reserve: < 10ms
  - Port allocation: < 1ms
  - Database lookup: < 5ms
  - CLI startup: < 50ms
- [ ] Benchmark results are reproducible (variance < 10%)

## Integration with CI

Add to `.github/workflows/coverage.yml`:
```yaml
- name: Run benchmarks
  run: cargo bench --no-fail-fast -- --output-format bencher | tee benchmark-results.txt

- name: Store benchmark result
  uses: benchmark-action/github-action-benchmark@v1
  with:
    tool: 'cargo'
    output-file-path: benchmark-results.txt
    github-token: ${{ secrets.GITHUB_TOKEN }}
    auto-push: true
    alert-threshold: '150%'
    comment-on-alert: true
```

## Notes

- Benchmarks establish baseline for future optimization work
- Some benchmarks (especially CLI) may be slow; consider gating for CI
- Criterion provides statistical analysis and regression detection
- Results should be documented in project README
- Consider separate benchmark runs for different hardware (x86_64 vs ARM64)
