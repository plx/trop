# Justfile for trop development
# See: https://github.com/casey/just

TROP_CONCURRENCY_TEST_THREADS:=env("TROP_CONCURRENCY_TEST_THREADS", "4")

# Default recipe to display help
default:
    @just --list

# Run all tests
test:
    cargo test

# Run config integration tests
test-config:
    cargo test --test config_integration

# Run all tests with verbose output
test-all:
    cargo test -- --nocapture

# Run clippy linter
clippy:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# Format code with rustfmt
fmt:
    cargo fmt

# Check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# Build the project in debug mode
build:
    cargo build

# Build the project in release mode
build-release:
    cargo build --release

# Run all checks (fmt, clippy, tests)
check: fmt-check clippy test

# Pre-flight checks for PR submission (minimal output)
preflight-pr:
    #!/usr/bin/env bash
    echo "Running pre-flight checks..."
    echo ""

    # Build check
    if cargo build --quiet 2>&1 | grep -q .; then
        echo "✗ Build failed"
        cargo build 2>&1 | tail -20
        exit 1
    fi
    echo "✓ Build succeeded"

    # Format check
    if ! cargo fmt -- --check &>/dev/null; then
        echo "✗ Format check failed"
        cargo fmt -- --check 2>&1
        exit 1
    fi
    echo "✓ Format check passed"

    # Clippy check
    output=$(cargo clippy -- -D warnings 2>&1) || true
    if echo "$output" | grep -q "error:\|warning:"; then
        echo "✗ Clippy failed"
        echo "$output" | grep -v "Checking\|Finished"
        exit 1
    fi
    echo "✓ Clippy passed"

    # Test check
    output=$(cargo test 2>&1) || true
    if echo "$output" | grep -q "test result: FAILED"; then
        echo "✗ Tests failed"
        echo "$output" | grep -A 20 "failures:" | head -40
        exit 1
    fi
    echo "✓ Tests passed"

    echo ""
    echo "All pre-flight checks passed!"

# Clean build artifacts
clean:
    cargo clean

# Generate documentation
doc:
    cargo doc --no-deps --open

# Run benchmarks
bench:
    cargo bench

# Build and run the CLI tool
run *ARGS:
    cargo run --bin trop-cli -- {{ARGS}}

# CI: build all (debug)
ci-build-all-debug:
    cargo build --all-targets --all-features --workspace

# CI: build all (release)
ci-build-all-release:
    cargo build --all-targets --all-features --workspace --release

# CI: build all (debug and release)
ci-build-all: ci-build-all-debug ci-build-all-release

# CI: run clippy (debug)
ci-run-clippy-debug:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# CI: run clippy (release)
ci-run-clippy-release:
    cargo clippy --all-targets --all-features --workspace -- -D warnings

# CI: run clippy (debug and release)
ci-run-clippy: ci-run-clippy-debug ci-run-clippy-release

# CI: run tests (debug)
ci-run-tests-debug:
    cargo test --all-targets --all-features --workspace --verbose

# CI: run tests (release)
ci-run-tests-release:
    cargo test --all-targets --all-features --workspace  --verbose --release

# CI: run tests (debug and release)
ci-run-tests: ci-run-tests-debug ci-run-tests-release

# CI: check format
ci-check-format:
    cargo fmt --all -- --check

# CI: check documentation
ci-check-docs:
    cargo doc --all --no-deps

# CI: run doc tests (debug)
ci-run-doc-tests-debug:
    cargo test --doc --workspace

# CI: check doc (release)
ci-run-doc-tests-release:
    cargo test --doc --workspace --release

# CI: check doc (debug and release)
ci-run-doc-tests: ci-run-doc-tests-debug ci-run-doc-tests-release

# CI: run concurrent tests (debug)
ci-run-concurrency-tests-debug threads=TROP_CONCURRENCY_TEST_THREADS:
    cargo test concurrent -- --test-threads={{threads}} --nocapture

# CI: run concurrent tests (release)
ci-run-concurrency-tests-release threads=TROP_CONCURRENCY_TEST_THREADS:
    cargo test --release concurrent -- --test-threads={{threads}} --nocapture

# CI: run concurrent tests (debug and release)
ci-run-concurrency-tests threads=TROP_CONCURRENCY_TEST_THREADS: (ci-run-concurrency-tests-debug threads) (ci-run-concurrency-tests-release threads)

# CI: build (debug)
ci-build-debug:
    cargo build --workspace

# CI: build (release)
ci-build-release:
    cargo build --workspace --release

# CI: run property tests (debug)
ci-run-property-tests-debug:
    cargo test --workspace --features property-tests -- --nocapture

# CI: run property tests (release)
ci-run-property-tests-release:
    cargo test --workspace --release --features property-tests -- --nocapture

# CI: run property tests (debug and release)
ci-run-property-tests: ci-run-property-tests-debug ci-run-property-tests-release
