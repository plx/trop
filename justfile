# Justfile for trop development
# See: https://github.com/casey/just

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
    cargo clippy -- -D warnings

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
