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
