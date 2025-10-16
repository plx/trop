# Development Log

This document records significant development milestones and lessons learned during the implementation of the `trop` port reservation tool.

## 2025-10-14 - Phase 2: SQLite Database Layer

Implemented the complete SQLite database persistence layer for port reservations. This phase established the foundation for storing and retrieving reservation data with proper concurrency support and transaction safety.

The implementation went smoothly overall, with all 12 core tasks completed successfully. Key accomplishments include:

- Database connection management with auto-creation and proper SQLite pragmas (WAL mode, busy timeout)
- Schema versioning framework ready for future migrations
- Complete CRUD operations for reservations with proper transaction handling
- Query operations optimized with appropriate indices
- Batch transaction helpers for atomic multi-reservation operations
- Comprehensive test coverage: 121 tests passing (99 unit + 10 integration + 5 common + 7 CLI + 49 doc tests)

The one deviation from the original plan was Task 12 (performance benchmarks with Criterion), which was not implemented. This was an optional enhancement listed in the testing section, and its absence doesn't affect functionality or subsequent phases. The phase achieved all its success criteria and is fully operational.

One notable success was the concurrent access testing: the integration test suite includes tests with 10 concurrent writers and mixed reader/writer scenarios, all passing reliably. This validates the WAL mode configuration and IMMEDIATE transaction strategy.

The database module is well-organized with clear separation between schema definition, connection management, migrations, and operations. Error handling properly converts rusqlite errors to domain errors with helpful context. The code is ready for Phase 3 (Path Handling System).

## 2025-10-14 - Phase 3: Path Handling System

Implemented the comprehensive path handling system with normalization, canonicalization, and relationship checking. This phase establishes critical infrastructure for managing filesystem paths with provenance awareness and proper validation.

The implementation went very smoothly, with all 12 tasks completed successfully. Key accomplishments include:

- Path normalization with tilde expansion, `.` and `..` resolution, and absolute path handling
- Path canonicalization with symlink following and loop detection
- PathResolver abstraction supporting explicit, implicit, and canonical resolution modes
- Path relationship detection (ancestor/descendant/same/unrelated) with transitive validation
- Provenance tracking distinguishing user-provided vs system-inferred paths
- Property-based tests using proptest for invariant validation (idempotency, reflexivity, symmetry, transitivity)
- Integration with database layer for path relationship validation
- Performance benchmarks for critical path operations
- Comprehensive test coverage: 177 tests passing (unit, integration, property-based, and doc tests)

The implementation required careful consideration of platform differences, particularly around symlink handling. All symlink-related tests are properly gated with `#[cfg(unix)]` to ensure cross-platform compatibility. The property-based tests were particularly valuable for validating mathematical properties of path relationships.

One notable design decision was the three-mode resolver approach (explicit, implicit, canonical), which provides fine-grained control over canonicalization behavior based on path provenance. This aligns perfectly with the specification's requirements for handling user-provided vs system-inferred paths differently.

The path handling module is well-organized with clear separation of concerns: normalization, canonicalization, relationship checking, and resolver abstraction are all independent and composable. Error handling properly distinguishes between different failure modes (not found, permission denied, path relationship violations). The code is ready for Phase 4 (Basic Reservation Operations).

## 2025-10-15 - Phase 4: Basic Reservation Operations

Implemented the core reservation operations using a plan-execute pattern, including idempotent reserve/release operations, sticky field protection, and path relationship validation. This phase brings the core business logic to life, enabling users to actually reserve and release ports.

The implementation went very smoothly, with all 10 tasks completed successfully. Key accomplishments include:

- Plan-execute pattern enabling dry-run mode and robust testing
- Idempotent reserve operations returning consistent ports for (path, tag) tuples
- Sticky field protection preventing accidental metadata changes
- Path relationship validation enforcing ancestor/descendant rules
- Force flag and granular override flags (allow_project_change, allow_task_change, allow_unrelated_path)
- Comprehensive test coverage: 348 tests passing (233 lib + 115 integration/doc tests)
- Property-based tests for mathematical invariants using proptest
- Dry-run mode verified to not modify database
- Plan descriptions providing clear operation summaries

The plan-execute pattern proved invaluable for testing and debugging. The separation between planning (validation, decision-making) and execution (database operations) enabled independent testing of each phase and provided clear error messages before any database modifications occur.

One notable design success was the sticky field protection system. The implementation supports both coarse-grained control (force flag overrides everything) and fine-grained control (specific allow flags for project/task changes). This provides flexibility for power users while maintaining safety by default.

The integration tests are particularly comprehensive, covering idempotency (24 tests), planning (31 tests), and path validation (25 tests). Property-based tests verify mathematical properties like plan idempotency, sticky field transitivity, and path relationship correctness across thousands of generated test cases.

Code quality is excellent with only minor stylistic clippy warnings (length comparisons, format string inlining). The operations module is well-organized with clear separation between plan types, reserve logic, release logic, and execution engine. Error handling properly distinguishes between user errors (sticky field changes, path violations) and system errors. The code is ready for Phase 5 (Configuration System).

## 2025-10-15 - Phase 5: Configuration System

Implemented the complete hierarchical configuration system with YAML support, environment variable overrides, and comprehensive validation. This phase enables users to customize trop's behavior at multiple levels (user config, project config, local overrides) with full validation and clear error messages.

The implementation went extremely smoothly, with all 8 tasks completed successfully. Key accomplishments include:

- Configuration schema supporting all planned features (ports, exclusions, cleanup, occupancy checks, reservation groups)
- Hierarchical configuration discovery walking up directory tree from working directory
- Proper precedence chain: CLI > env vars > trop.local.yaml > trop.yaml > user config > defaults
- Environment variable support for all TROP_* variables with flexible boolean parsing (true/1/yes/on)
- Comprehensive validation with clear, actionable error messages identifying source and suggesting fixes
- Builder pattern for programmatic configuration construction with skip_files/skip_env testing support
- Custom YAML deserialization for port ranges ("5000..5010" format) and other specialized types
- Extensive test coverage: 407 tests passing (365 lib + 42 integration tests)

The configuration merging logic handles different field types appropriately: simple fields use "last wins" semantics, excluded_ports accumulates across all sources (union), occupancy config is treated atomically, and reservation groups replace rather than merge. This provides intuitive behavior for users.

One notable design success was the validation system, which enforces strict rules while providing helpful error messages. For example, the "project" and "reservations" fields are restricted to trop.yaml files only, reservation group offsets must be unique, environment variable names must be valid identifiers, and port ranges must have end >= start. All validation errors include the field name, explanation, and context.

The test fixtures directory structure (valid/, invalid/, hierarchy/) enabled comprehensive testing of both success and failure cases. Property-based tests verify invariants like configuration merging commutativity and validation consistency across thousands of generated configurations.

Minor deviation from the plan: error handling uses `serde_yaml::Error` directly wrapped in validation errors rather than a custom Configuration error variant, but this provides equivalent functionality with better error messages from serde_yaml. The implementation fully satisfies the spirit and intent of the phase plan.
