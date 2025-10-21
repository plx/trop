use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use assert_cmd::prelude::*;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use tempfile::TempDir;

static CLI_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn initialize_data_dir(data_dir: &TempDir) {
    let mut cmd = Command::cargo_bin("trop").expect("failed to locate trop binary");
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let status = cmd
        .args([
            "--data-dir",
            data_dir.path().to_str().unwrap(),
            "--quiet",
            "init",
        ])
        .status()
        .expect("failed to execute trop init");
    assert!(status.success(), "trop init command failed");
}

fn reserve_path(data_dir: &TempDir, path: &str) {
    let mut cmd = Command::cargo_bin("trop").expect("failed to locate trop binary");
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let status = cmd
        .args([
            "--data-dir",
            data_dir.path().to_str().unwrap(),
            "reserve",
            "--path",
            path,
            "--allow-unrelated-path",
            "--quiet",
        ])
        .status()
        .expect("failed to execute trop reserve");
    assert!(status.success(), "trop reserve command failed");
}

fn bench_cli_startup(c: &mut Criterion) {
    c.bench_function("cli_startup_version", |b| {
        b.iter(|| {
            let mut cmd = Command::cargo_bin("trop").expect("failed to locate trop binary");
            let output = cmd.arg("--version").output().expect("failed to run trop");
            black_box(output);
        });
    });
}

fn bench_cli_reserve(c: &mut Criterion) {
    c.bench_function("cli_reserve", |b| {
        b.iter_batched(
            || {
                let data_dir = TempDir::new().expect("failed to create temp dir");
                initialize_data_dir(&data_dir);
                data_dir
            },
            |data_dir| {
                let counter = CLI_COUNTER.fetch_add(1, Ordering::Relaxed);
                let path = data_dir.path().join(format!("cli-reserve-{counter}"));

                std::fs::create_dir_all(&path).expect("failed to create bench path");

                let mut cmd = Command::cargo_bin("trop").expect("failed to locate trop binary");
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
                let status = cmd
                    .args([
                        "--data-dir",
                        data_dir.path().to_str().unwrap(),
                        "reserve",
                        "--path",
                        path.to_str().unwrap(),
                        "--allow-unrelated-path",
                        "--quiet",
                    ])
                    .status()
                    .expect("failed to execute trop reserve");

                black_box(status.success());
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_cli_list(c: &mut Criterion) {
    c.bench_function("cli_list", |b| {
        b.iter_batched(
            || {
                let data_dir = TempDir::new().expect("failed to create temp dir");
                initialize_data_dir(&data_dir);

                for i in 0..50 {
                    let path = data_dir.path().join(format!("cli-list-{i}"));
                    std::fs::create_dir_all(&path).expect("failed to create bench path");
                    reserve_path(&data_dir, path.to_str().unwrap());
                }

                data_dir
            },
            |data_dir| {
                let mut cmd = Command::cargo_bin("trop").expect("failed to locate trop binary");
                let output = cmd
                    .args([
                        "--data-dir",
                        data_dir.path().to_str().unwrap(),
                        "list",
                        "--format",
                        "json",
                    ])
                    .output()
                    .expect("failed to execute trop list");

                black_box(output);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    cli_benches,
    bench_cli_startup,
    bench_cli_reserve,
    bench_cli_list
);
criterion_main!(cli_benches);
