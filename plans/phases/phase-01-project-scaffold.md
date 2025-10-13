# Phase 1: Project Scaffold & Core Types - Detailed Implementation Plan

## Overview

This document provides a comprehensive, actionable implementation plan for Phase 1 of the `trop` port reservation tool. This phase establishes the foundational Rust project structure, defines core types, and sets up development infrastructure.

## Success Criteria

Upon completion of Phase 1:
- The project builds successfully with `cargo build`
- All tests pass with `cargo test`
- The binary runs and displays version information
- Code passes `cargo fmt` and `cargo clippy` checks
- Core types are defined and documented
- Error handling infrastructure is in place

## Task Breakdown

### Task 1: Initialize Workspace Structure

**Objective**: Create the Rust workspace with library and binary crates.

**Files to Create/Modify**:
- `/Users/prb/github/trop/Cargo.toml` (workspace manifest)
- `/Users/prb/github/trop/trop/Cargo.toml` (library crate)
- `/Users/prb/github/trop/trop-cli/Cargo.toml` (binary crate)

**Implementation Details**:

1. Create workspace root `Cargo.toml`:
   - Define `[workspace]` section with members `["trop", "trop-cli"]`
   - Set resolver to "2"
   - Configure shared dependencies in `[workspace.dependencies]`
   - Include: clap, rusqlite, serde, serde_yaml, thiserror, chrono, anyhow

2. Create library crate at `trop/`:
   - Package name: `trop`
   - Version: "0.1.0"
   - Edition: "2021" (note: spec says 2024 but that's not available yet)
   - Add workspace dependency references

3. Create binary crate at `trop-cli/`:
   - Package name: `trop-cli`
   - Binary name: `trop` (via `[[bin]]` section)
   - Depend on `trop` library crate
   - Add clap with derive feature

**Verification**:
- Run `cargo build` at workspace root - should create both crates
- Check that `target/debug/trop` binary exists

### Task 2: Configure Development Tools

**Objective**: Set up rustfmt and clippy with strict configurations.

**Files to Create**:
- `/Users/prb/github/trop/.rustfmt.toml`
- `/Users/prb/github/trop/.clippy.toml`

**Implementation Details**:

1. Create `.rustfmt.toml`:
   ```toml
   edition = "2021"
   max_width = 100
   use_field_init_shorthand = true
   use_try_shorthand = true
   imports_granularity = "Crate"
   group_imports = "StdExternalCrate"
   ```

2. Create `.clippy.toml`:
   ```toml
   msrv = "1.70.0"
   ```

3. Add clippy lints to library `trop/src/lib.rs`:
   - Add standard deny/warn attributes for common issues
   - Include: `missing_docs`, `unsafe_code`, `clippy::all`, `clippy::pedantic`
   - Allow specific lints where appropriate for ergonomics

**Verification**:
- Run `cargo fmt --all -- --check` - should pass
- Run `cargo clippy --all-targets --all-features` - should pass

### Task 3: Define Core Port Types

**Objective**: Create the foundational port-related types.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/lib.rs`
- `/Users/prb/github/trop/trop/src/port.rs`

**Implementation Details**:

1. In `trop/src/port.rs`, define:
   - `Port`: newtype wrapper around `u16` with validation
     - Implement `TryFrom<u16>` with range checking (1-65535)
     - Implement `Display`, `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`
     - Add methods: `value() -> u16`, `is_privileged() -> bool`

   - `PortRange`: struct with `min: Port` and `max: Port`
     - Implement validation (max >= min)
     - Add methods: `contains(&self, port: Port) -> bool`, `len() -> u16`
     - Add iterator support for iterating over ports in range
     - Implement `Display` showing "min-max" format

2. Add comprehensive unit tests:
   - Port validation edge cases (0, 1, 65535, 65536)
   - PortRange validation and contains logic
   - Display formatting

**Verification**:
- Run `cargo test port` - all tests should pass
- Documentation builds with `cargo doc --open`

### Task 4: Define Reservation Types

**Objective**: Create the core reservation data structures.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/reservation.rs`

**Implementation Details**:

1. Define `ReservationKey`:
   - Fields: `path: PathBuf`, `tag: Option<String>`
   - Implement `Display` showing "path" or "path:tag"
   - Add validation for tag (non-empty after trimming)
   - Implement `PartialEq`, `Eq`, `Hash` for use as map keys

2. Define `Reservation`:
   - Include all fields from spec: key, port, project, task, created_at, last_used_at
   - Use `SystemTime` for timestamps (from std)
   - Implement builder pattern for construction
   - Add method `is_expired(&self, max_age: Duration) -> bool`

3. Add field validation helpers:
   - Function to validate and normalize tags/projects/tasks
   - Strip whitespace, check non-empty
   - Return `Result` with clear error messages

**Verification**:
- Unit tests for key equality and hashing
- Tests for builder pattern and validation
- Tests for expiry logic

### Task 5: Create Error Type Hierarchy

**Objective**: Establish comprehensive error handling using thiserror.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/error.rs`

**Implementation Details**:

1. Define `Error` enum with thiserror derives:
   - `InvalidPort`: for port range violations
   - `InvalidPath`: for path-related errors
   - `Database`: wrapping rusqlite errors
   - `Configuration`: for config file issues
   - `Io`: wrapping std::io errors
   - `Validation`: for input validation failures
   - `PortUnavailable`: when no ports can be allocated
   - `ReservationConflict`: for conflicting operations

2. Implement conversions:
   - From `rusqlite::Error` to `Error::Database`
   - From `std::io::Error` to `Error::Io`
   - From `serde_yaml::Error` to `Error::Configuration`

3. Define type alias: `pub type Result<T> = std::result::Result<T, Error>`

**Verification**:
- Error messages are clear and actionable
- All variants have appropriate Display implementations
- Conversions work correctly

### Task 6: Set Up Logging Infrastructure

**Objective**: Implement stderr-based logging system.

**Files to Create**:
- `/Users/prb/github/trop/trop/src/logging.rs`

**Implementation Details**:

1. Define `LogLevel` enum:
   - Variants: `Quiet`, `Normal`, `Verbose`
   - Add comparison traits

2. Create `Logger` struct:
   - Field for current log level
   - Methods: `error()`, `warn()`, `info()`, `debug()`
   - All output goes to stderr via `eprintln!`
   - Respect log level for filtering

3. Add initialization function:
   - Parse `TROP_LOG_MODE` environment variable
   - Accept CLI flags for verbose/quiet
   - Return configured Logger instance

4. Consider using `log` crate facade:
   - Implement custom logger backend
   - Allows library users to integrate with their logging

**Verification**:
- Test output appears on stderr, not stdout
- Log levels filter appropriately
- Environment variable parsing works

### Task 7: Create Module Structure

**Objective**: Organize library code into logical modules.

**Files to Modify**:
- `/Users/prb/github/trop/trop/src/lib.rs`

**Implementation Details**:

1. Set up module declarations:
   ```rust
   pub mod error;
   pub mod logging;
   pub mod port;
   pub mod reservation;

   // Re-export key types at crate root
   pub use error::{Error, Result};
   pub use port::{Port, PortRange};
   pub use reservation::{Reservation, ReservationKey};
   ```

2. Add crate-level documentation:
   - Overview of trop's purpose
   - Link to binary crate for CLI usage
   - Examples of library usage

3. Configure crate attributes:
   - Documentation settings
   - Linting rules
   - Feature flags if needed

**Verification**:
- `cargo doc` generates complete documentation
- Public API is accessible from crate root

### Task 8: Implement Minimal CLI Binary

**Objective**: Create placeholder CLI that prints version.

**Files to Create**:
- `/Users/prb/github/trop/trop-cli/src/main.rs`
- `/Users/prb/github/trop/trop-cli/build.rs` (optional, for version info)

**Implementation Details**:

1. Define CLI structure with clap derive:
   ```rust
   #[derive(Parser)]
   #[command(name = "trop")]
   #[command(version, about, long_about = None)]
   struct Cli {
       #[command(subcommand)]
       command: Option<Commands>,
   }

   #[derive(Subcommand)]
   enum Commands {
       // Placeholder - will be filled in later phases
   }
   ```

2. Implement main function:
   - Parse CLI arguments
   - If no subcommand, print help
   - Set up panic handler for better error messages
   - Initialize logging based on flags

3. Add version information:
   - Use env!("CARGO_PKG_VERSION")
   - Consider adding git commit hash in build.rs

**Verification**:
- `cargo run -- --version` prints version
- `cargo run -- --help` shows help text
- Binary size is reasonable

### Task 9: Add Integration Test Scaffolding

**Objective**: Create test infrastructure for future integration tests.

**Files to Create**:
- `/Users/prb/github/trop/trop/tests/common/mod.rs`
- `/Users/prb/github/trop/trop-cli/tests/cli.rs`

**Implementation Details**:

1. Create test utilities module:
   - Helper for creating temporary directories
   - Helper for isolated database creation
   - Fixture data builders

2. Add basic CLI integration test:
   - Test that binary runs without arguments
   - Test version flag output
   - Test help text generation

3. Set up test organization:
   - Common module for shared utilities
   - Separate files for different test categories

**Verification**:
- `cargo test --all` runs all tests
- Tests are isolated and don't interfere

### Task 10: Create Basic Documentation

**Objective**: Add initial README and documentation.

**Files to Create/Modify**:
- `/Users/prb/github/trop/README.md` (update existing)
- `/Users/prb/github/trop/trop/README.md`
- `/Users/prb/github/trop/trop-cli/README.md`

**Implementation Details**:

1. Update root README:
   - Brief description of trop
   - Build instructions
   - Link to specification
   - Development setup guide

2. Create library README:
   - API overview
   - Usage examples (even if placeholder)
   - Link to documentation

3. Create CLI README:
   - Installation instructions
   - Basic usage (coming soon)
   - Configuration overview

**Verification**:
- READMEs render correctly on GitHub
- Links work correctly

## Dependencies Between Tasks

```
Task 1 (Workspace)
    ├── Task 2 (Dev Tools)
    ├── Task 3 (Port Types)
    ├── Task 4 (Reservation Types)
    ├── Task 5 (Error Types)
    ├── Task 6 (Logging)
    └── Task 7 (Module Structure)
         ├── Task 8 (CLI Binary)
         └── Task 9 (Test Scaffolding)
              └── Task 10 (Documentation)
```

Tasks 3-6 can be implemented in parallel after Task 1. Task 7 requires Tasks 3-6. Tasks 8-9 require Task 7. Task 10 can be done anytime after Task 1.

## Testing Strategy

### Unit Tests
- Each type should have comprehensive unit tests in its module
- Use `#[cfg(test)]` modules at the bottom of each file
- Test edge cases, especially for validation logic
- Aim for >80% code coverage

### Doc Tests
- Add examples in documentation comments
- Ensure all public APIs have usage examples
- Examples should be runnable

### Integration Tests
- Create foundation for integration testing
- Even if minimal, establish the pattern

## Validation Checklist

Before considering Phase 1 complete:

- [ ] Workspace builds without warnings
- [ ] All tests pass
- [ ] Code formatted with rustfmt
- [ ] No clippy warnings at default level
- [ ] Public APIs have documentation
- [ ] Error messages are helpful
- [ ] Binary runs and shows version
- [ ] README files are present and accurate
- [ ] Core types are ergonomic to use
- [ ] Module structure is logical

## Risk Mitigations

### Rust Edition
The spec mentions Rust 2024, but this doesn't exist yet. Use 2021 edition and note this for future update.

### Dependency Versions
Pin major versions but allow minor updates. Use workspace dependencies to ensure consistency.

### Platform Differences
Even though Phase 1 is platform-agnostic, set up CI to test on Linux, macOS, and Windows early.

### API Stability
Mark library as 0.1.0 and note that API is unstable. Don't publish to crates.io yet.

## Next Phase Preparation

Phase 2 will build the SQLite database layer. Ensure:
- Error type can wrap SQLite errors
- Reservation types are serialization-friendly
- Path handling is prepared for database storage

## Notes for Implementer

### Code Style
- Prefer explicit over implicit
- Use semantic newtype wrappers
- Validate at construction when possible
- Make illegal states unrepresentable

### Documentation
- Every public item needs documentation
- Include examples where helpful
- Explain "why" not just "what"
- Link between related items

### Testing Philosophy
- Test behavior, not implementation
- Each test should test one thing
- Test names should describe the scenario
- Use property-based testing for validators

### Performance
- Don't optimize prematurely in Phase 1
- Focus on correctness and API design
- Profile before optimizing later

This plan provides sufficient detail for an experienced Rust developer to implement Phase 1 without making architectural decisions that would conflict with the specification. Each task has clear outputs and verification steps.