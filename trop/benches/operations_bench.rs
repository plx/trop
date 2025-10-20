use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use tempfile::TempDir;

use trop::config::{Config, ConfigBuilder};
use trop::database::{Database, DatabaseConfig};
use trop::operations::{
    ExecutionResult, PlanExecutor, ReleaseOptions, ReleasePlan, ReserveOptions, ReservePlan,
};
use trop::{Port, ReservationKey};

const LOOKUP_SIZES: &[usize] = &[10, 100, 500, 1000];
const BULK_RESERVATION_SIZES: &[usize] = &[10, 100, 250];

fn benchmark_config() -> Config {
    let overrides = Config {
        allow_unrelated_path: Some(true),
        ..Config::default()
    };

    ConfigBuilder::new()
        .skip_env()
        .skip_files()
        .with_config(overrides)
        .build()
        .expect("failed to build benchmark configuration")
}

fn setup_database() -> (TempDir, Database) {
    let temp_dir = TempDir::new().expect("failed to create temporary directory");
    let db_path = temp_dir.path().join("trop.db");
    let config = DatabaseConfig::new(&db_path);
    let db = Database::open(config).expect("failed to open temporary database");
    (temp_dir, db)
}

fn perform_reserve(db: &Database, config: &Config, key: ReservationKey) -> ExecutionResult {
    let options = ReserveOptions::new(key.clone(), None).with_allow_unrelated_path(true);
    let plan = ReservePlan::new(options, config)
        .build_plan(db.connection())
        .expect("failed to plan reservation");
    let mut executor = PlanExecutor::new(db.connection());
    executor
        .execute(&plan)
        .expect("failed to execute reservation plan")
}

fn perform_release(db: &Database, key: ReservationKey) -> ExecutionResult {
    let options = ReleaseOptions::new(key).with_allow_unrelated_path(true);
    let plan = ReleasePlan::new(options)
        .build_plan(db.connection())
        .expect("failed to plan release");
    let mut executor = PlanExecutor::new(db.connection());
    executor
        .execute(&plan)
        .expect("failed to execute release plan")
}

fn populate_reservations(
    db: &Database,
    config: &Config,
    count: usize,
    prefix: &str,
) -> (ReservationKey, Port) {
    let mut last_entry = None;

    for index in 0..count {
        let key = ReservationKey::new(PathBuf::from(format!("/tmp/{prefix}-{index}")), None)
            .expect("failed to build reservation key");

        let result = perform_reserve(db, config, key.clone());
        let port = result
            .port
            .expect("reserve operation should allocate a port");
        last_entry = Some((key, port));
    }

    last_entry.expect("at least one reservation should be created")
}

fn bench_reserve_single(c: &mut Criterion) {
    let config = benchmark_config();

    c.bench_function("reserve_single", |b| {
        b.iter_batched(
            setup_database,
            |(temp_dir, db)| {
                let _temp_dir = temp_dir;
                let key = ReservationKey::new(PathBuf::from("/tmp/bench-single"), None)
                    .expect("failed to build key");
                let result = perform_reserve(&db, &config, key);
                black_box(result);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_reserve_bulk(c: &mut Criterion) {
    let config = benchmark_config();
    let mut group = c.benchmark_group("reserve_bulk");

    for &size in BULK_RESERVATION_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &count| {
            b.iter_batched(
                setup_database,
                |(temp_dir, db)| {
                    let _temp_dir = temp_dir;
                    for index in 0..count {
                        let key =
                            ReservationKey::new(PathBuf::from(format!("/tmp/bulk-{index}")), None)
                                .expect("failed to build key");
                        let result = perform_reserve(&db, &config, key);
                        black_box(result.port);
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_lookup_by_path(c: &mut Criterion) {
    let config = benchmark_config();
    let mut group = c.benchmark_group("lookup_by_path");

    for &size in LOOKUP_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &count| {
            b.iter_batched(
                || {
                    let (temp_dir, db) = setup_database();
                    let (key, _port) = populate_reservations(&db, &config, count, "lookup");
                    (temp_dir, db, key)
                },
                |(temp_dir, db, key)| {
                    let _temp_dir = temp_dir;
                    let reservation =
                        Database::get_reservation(db.connection(), &key).expect("lookup failed");
                    black_box(reservation);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_lookup_by_port(c: &mut Criterion) {
    let config = benchmark_config();
    let mut group = c.benchmark_group("lookup_by_port");

    for &size in LOOKUP_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &count| {
            b.iter_batched(
                || {
                    let (temp_dir, db) = setup_database();
                    let (_key, port) = populate_reservations(&db, &config, count, "lookup-port");
                    (temp_dir, db, port)
                },
                |(temp_dir, db, port)| {
                    let _temp_dir = temp_dir;
                    let reservation = Database::get_reservation_by_port(db.connection(), port)
                        .expect("lookup by port failed");
                    black_box(reservation);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_list_reservations(c: &mut Criterion) {
    let config = benchmark_config();
    let mut group = c.benchmark_group("list_reservations");

    for &size in LOOKUP_SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &count| {
            b.iter_batched(
                || {
                    let (temp_dir, db) = setup_database();
                    let _ = populate_reservations(&db, &config, count, "list");
                    (temp_dir, db)
                },
                |(temp_dir, db)| {
                    let _temp_dir = temp_dir;
                    let reservations = Database::list_all_reservations(db.connection())
                        .expect("failed to list reservations");
                    black_box(reservations);
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_release_reservation(c: &mut Criterion) {
    let config = benchmark_config();

    c.bench_function("release_reservation", |b| {
        b.iter_batched(
            || {
                let (temp_dir, db) = setup_database();
                let key = ReservationKey::new(PathBuf::from("/tmp/release"), None)
                    .expect("failed to build key");
                let result = perform_reserve(&db, &config, key.clone());
                black_box(result.port);
                (temp_dir, db, key)
            },
            |(temp_dir, db, key)| {
                let _temp_dir = temp_dir;
                let result = perform_release(&db, key);
                black_box(result.actions_taken);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    operations_bench,
    bench_reserve_single,
    bench_reserve_bulk,
    bench_lookup_by_path,
    bench_lookup_by_port,
    bench_list_reservations,
    bench_release_reservation
);
criterion_main!(operations_bench);
