# Trop Implementation Plan - V1

This document outlines the implementation strategy for the Trop port reservation tool, based on the specification in `ImplementationPlan.md`. The work is broken down into atomic phases that each leave the project in a working state.

## Phase 1: Project Scaffold & Core Types

Set up the basic Rust project structure with library and binary targets, define core types, and establish the foundational abstractions.

- Create workspace with `trop` library crate and `trop` binary crate
- Set up dependencies: clap (with derive), rusqlite, serde, serde_yaml, thiserror, chrono
- Define core types: `ReservationKey`, `Reservation`, `Port`, `PortRange` 
- Implement basic Display/Debug traits and type conversions
- Create error type hierarchy using thiserror
- Set up basic logging infrastructure (to stderr)
- Add rustfmt and clippy configurations with strict settings
- Create placeholder main.rs that just prints version

This phase establishes the project foundation and ensures all team members can build and run the project, even if it doesn't do anything useful yet.

## Phase 2: SQLite Database Layer

Implement the database abstraction layer with proper SQLite setup, schema management, and basic CRUD operations.

- Create `Database` struct with connection management
- Implement schema creation with `reservations` and `metadata` tables
- Set up proper SQLite pragmas (WAL mode, busy timeout, etc.)
- Add schema versioning support via metadata table
- Implement basic reservation CRUD operations (create, read, update, delete)
- Add transaction support with proper IMMEDIATE mode for writes
- Handle database initialization and auto-creation logic
- Create integration tests using temporary databases

This gives us a working persistence layer that we can build upon, with proper concurrent access support from the start.

## Phase 3: Path Handling System

Implement comprehensive path normalization and canonicalization logic according to the spec's requirements.

- Create `PathResolver` abstraction for path operations
- Implement normalization (converting to absolute paths, expanding ~, etc.)
- Implement canonicalization (following symlinks) with provenance awareness
- Add path relationship checking (ancestor, descendant, unrelated)
- Handle non-existent paths gracefully
- Create comprehensive unit tests with property-based testing
- Add special handling for implicit vs explicit paths

Path handling is critical to get right early since reservations are keyed by paths, and changing this later would require data migrations.

## Phase 4: Basic Reservation Operations

Implement the core reservation logic in the library with the plan-execute pattern.

- Create `ReservationPlan` and `ReservationExecutor` abstractions
- Implement idempotent reserve operation (without port allocation yet - just using provided ports)
- Implement release operation
- Add last_used_at timestamp updates
- Implement the sticky field protection (project/task changes)
- Add unrelated path checking
- Create the `--force` override logic
- Write comprehensive unit tests for all reservation scenarios

At this point we have a working reservation system, though without automatic port allocation.

## Phase 5: Configuration System

Build the hierarchical configuration system with YAML support.

- Define configuration structs with serde
- Implement configuration file discovery (walking up directories)
- Build precedence chain: CLI > env > trop.local.yaml > trop.yaml > user config > defaults
- Add configuration validation logic
- Implement environment variable mapping (TROP_* pattern)
- Handle excluded_ports lists with range support
- Parse and validate the `reservations` group definitions
- Create configuration merge logic for hierarchical inheritance

This enables users to configure the tool's behavior at multiple levels, essential for real-world usage.

## Phase 6: Port Allocation & Occupancy

Implement the port allocation algorithm with occupancy checking.

- Research and integrate a cross-platform port scanning library
- Build `PortAllocator` with forward-scanning algorithm  
- Implement occupancy checking with configurable TCP/UDP/IPv4/IPv6 options
- Add excluded ports support during allocation
- Implement the retry-after-cleanup logic
- Handle preferred port specifications
- Add group allocation for reservation groups (pattern matching)
- Create mock occupancy checker for testing
- Write extensive tests for allocation scenarios

This completes the core port reservation functionality, making the tool actually useful.

## Phase 7: Essential CLI Commands

Create the CLI framework and implement the core commands: reserve, release, and list.

- Set up clap with derive macros and subcommand structure
- Implement global options (--verbose, --quiet, --data-dir, etc.)
- Create `reserve` subcommand with all its options
- Create `release` subcommand with tag support
- Create `list` subcommand with formatting options
- Add proper exit code handling (0, 1, 2, 3, etc.)
- Implement stdout/stderr separation
- Add shell-friendly output (just port number for reserve)

At this stage, users can perform basic port reservation workflows from the command line.

## Phase 8: Group Reservations

Implement batch reservation functionality for groups of related services.

- Create `ReservationGroup` model from configuration
- Implement `reserve-group` command with transactional semantics
- Build `autoreserve` with configuration file discovery
- Add multiple output formats (export, json, dotenv, human)
- Implement shell detection for export format
- Handle environment variable injection
- Ensure atomic all-or-nothing group reservations
- Test concurrent group reservations

This enables the key use case of reserving multiple ports for multi-service applications.

## Phase 9: Cleanup Operations

Add maintenance commands for cleaning up stale reservations.

- Implement `prune` command for non-existent directories
- Create `expire` command with configurable days threshold
- Build `autoclean` combining prune and expire
- Add auto-cleanup during allocation when ports exhausted
- Implement dry-run support for all cleanup operations
- Add configuration options for auto-cleanup behavior
- Test cleanup operations with various scenarios

These operations are essential for long-running systems to prevent database bloat.

## Phase 10: Assertion & Utility Commands

Implement the various assertion and utility commands for testing and debugging.

- Create `assert-reservation` and `assert-port` commands
- Implement `assert-data-dir` with validation option
- Add `port-info` command with occupancy details
- Build `scan` command with auto-exclude functionality
- Create `validate` command for configuration files
- Implement `exclude` command for adding exclusions
- Add `compact-exclusions` for cleanup
- Create show commands (show-data-dir, show-path)

These commands support automation, testing, and debugging workflows.

## Phase 11: Advanced Operations

Implement the remaining advanced features and commands.

- Build `migrate` command with recursive support
- Implement `list-projects` command
- Add `init` command for explicit initialization
- Create the project/task inference logic using gix
- Implement all the `--allow-*` flag variations
- Add recursive release functionality
- Build proper canonicalization logic for implicit paths

This completes the full feature set defined in the specification.

## Phase 12: Testing & Polish

Comprehensive testing, documentation, and production readiness.

- Add property-based tests using proptest or quickcheck
- Create integration test suite with concurrent operation tests
- Build example configurations and use cases
- Write man page generation
- Add shell completion generation
- Create CI/CD pipeline configuration
- Implement benchmarks for critical paths
- Add migration tests for future schema changes
- Polish error messages and help text

This phase ensures the tool is production-ready with excellent developer experience and reliability.

## Notes

Each phase should include tests for new functionality as it's added, rather than deferring all testing to the end. The plan-execute pattern should be used throughout the library implementation. After each phase, we should update our implementation journal and adjust subsequent phases based on lessons learned.

The ordering prioritizes getting a minimal working system early (phases 1-7) before adding advanced features. This allows for early testing and validation of core concepts.
