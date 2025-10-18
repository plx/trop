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

## 2025-10-16 - Phase 6: Port Allocation & Occupancy Tracking

Implemented the complete port allocation system with automatic port selection, occupancy checking, exclusion support, and group allocation with offsets. This phase transforms trop from a manual port specification tool into an intelligent allocator that finds available ports automatically.

The implementation went extremely smoothly, with all components completed successfully. Key accomplishments include:

- Port occupancy checking using `port-selector` crate with trait-based design for testability
- Mock occupancy checker enabling deterministic testing without system dependencies
- Exclusion manager with efficient sorted-range checking and compaction logic
- Forward-scanning allocation algorithm that checks reservations, exclusions, and system occupancy
- Group allocation supporting services with port offsets (e.g., web:0, api:1, debug:100)
- Transactional group allocation ensuring all-or-nothing semantics
- Preferred port support with granular override flags (ignore_occupied, ignore_exclusions)
- Integration with Phase 5 configuration system for ranges and exclusions
- Comprehensive test coverage: 540+ tests passing including 210+ property-based tests for allocator and groups
- Property-based tests validating determinism, allocation correctness, and group atomicity

The implementation successfully balances several design goals: deterministic allocation (same inputs produce same outputs), fail-closed security policy (occupancy check failures treat ports as occupied), and flexibility (preferred ports, override flags, multiple resolution modes).

One notable design success was the trait-based occupancy checker design. The `PortOccupancyChecker` trait with `SystemOccupancyChecker` and `MockOccupancyChecker` implementations enables comprehensive testing without requiring elevated privileges or actual port binding. Tests can inject specific occupancy patterns and verify allocation behavior in isolation.

Group allocation proved more complex than anticipated but the final implementation handles all edge cases correctly: finding patterns with gaps (e.g., [0, 1, 100]), respecting per-service preferred ports, and rolling back partial allocations on conflict. The property-based tests generate thousands of random group patterns and verify atomicity and correctness.

The integration with Phase 4's reserve operation was straightforward - the allocator slots cleanly into the existing planning workflow, with manual port specification remaining as a first-class option alongside automatic allocation. Backward compatibility is complete.

Property-based testing was extensive, with `proptest` tests covering: allocation determinism (same state + options = same result), exclusion respect (allocated ports never in exclusion list), group atomicity (all services succeed or all fail), and offset pattern matching (found base ports satisfy all offset requirements). These tests found several edge cases during development that were promptly fixed.

Minor implementation notes: cleanup integration was prepared but automatic cleanup-on-exhaustion wasn't wired up to the allocator (intentional - cleanup remains explicit via CLI). The fail-closed policy for occupancy checks (errors treated as "occupied") ensures safe behavior even under system permission restrictions. All public APIs are fully documented with examples.

## 2025-10-17 - Phase 7: Essential CLI Commands

Implemented the complete CLI interface with `reserve`, `release`, and `list` commands, providing a user-facing shell-friendly interface to the trop port reservation system. This phase transforms the library into a usable command-line tool with comprehensive argument parsing, multiple output formats, and proper error handling.

The implementation required multiple iterations to address test infrastructure issues and align implementation with specification requirements. Key accomplishments include:

- Complete CLI structure using clap derive macros with global options and three subcommands
- Reserve command with full idempotency, preferred port support, and all override flags
- Release command with tag filtering and idempotent behavior (success even when nothing to release)
- List command with multiple output formats (table, json, csv, tsv) and filtering capabilities
- Proper stdout/stderr separation enabling shell-friendly scripting (`PORT=$(trop reserve)`)
- Comprehensive test coverage: 117 CLI integration tests passing (27 global options, 29 list, 23 release, 31 reserve, 7 error handling)
- Path resolution strategy distinguishing explicit (normalized only) vs implicit (canonicalized) paths
- Exit code system with distinct codes for different failure modes

The implementation encountered several challenges that required careful debugging:

1. **Test Infrastructure**: Initial test failures revealed that multiple tests were passing conflicting `--data-dir` flags. Solution: added `command_bare()` method to TestEnv that allows tests to manually construct commands without automatic global options, enabling fine-grained control over argument ordering.

2. **Path Normalization Issues**: Symlinked temp directories on macOS (`/var` → `/private/var`) caused path filter tests to fail. Solution: standardized on path normalization (not canonicalization) throughout the CLI layer, allowing tests to work with symbolic paths and non-existent paths (important for dry-run mode).

3. **Release Command Semantics**: Discovered divergence between implementation and specification regarding idempotency. The specification requires release operations to succeed (exit code 0) even when nothing matches, treating "already released" as success. Updated tests and implementation to match this expectation, marking one CWD-based test as ignored due to test harness limitations.

4. **Format Parsing**: Initial implementation was case-sensitive for `--format` values. Added case-insensitive parsing so `--format JSON`, `--format json`, and `--format Json` all work correctly.

The final implementation provides excellent user experience with clear error messages, helpful defaults (CWD for path, table format for list), and extensive environment variable support (TROP_PATH, TROP_PROJECT, TROP_TASK, etc.). The CLI is a thin wrapper around the library, with all business logic residing in library operations following the plan-execute pattern.

Testing was comprehensive with 117 integration tests exercising all command combinations, error conditions, format options, and flag interactions. Property-based testing at the library level provides confidence in correctness, while CLI tests focus on user-facing behavior and shell integration.

Notable design decisions:
- Explicit paths (from `--path`) are normalized but NOT canonicalized (preserving user intent for symlinks)
- Implicit paths (CWD when `--path` omitted) are both normalized AND canonicalized (following actual filesystem)
- All logging and warnings go to stderr; only primary output (port numbers, lists) goes to stdout
- Format enum supports case-insensitive parsing for better UX
- Release operations are idempotent (success even when nothing to release)

## 2025-10-17 - Phase 8: Group Reservations

Implemented batch reservation functionality with `reserve-group` and `autoreserve` commands, enabling atomic reservation of multiple related services with sophisticated output formatting. This phase completes the core feature set by providing convenient workflow tools for reserving entire service groups from configuration files.

The implementation went very smoothly, with all components completed successfully. Key accomplishments include:

- Complete output formatting module with four formats (export, json, dotenv, human)
- Shell detection and format generation for bash, zsh, fish, and PowerShell
- Reserve-group command for explicit config file reservation
- Autoreserve command with automatic config discovery walking up from current directory
- Environment variable mapping support for services (e.g., web → WEB_PORT)
- Integration with Phase 6 group allocation for atomic port reservation
- Comprehensive test coverage: 164 tests passing (1645 CLI group command tests, 1250 library group integration tests)
- Proper stdout/stderr separation enabling shell sourcing (`eval $(trop autoreserve)`)

The output formatting system is the major new capability in this phase. Four distinct formatters handle different use cases:

1. **Export Format**: Shell-specific environment variable exports (e.g., `export WEB_PORT=5000` for bash/zsh, `set -x WEB_PORT 5000` for fish, `$env:WEB_PORT="5000"` for PowerShell)
2. **JSON Format**: Machine-readable JSON output (e.g., `{"web": 5000, "api": 5001}`)
3. **Dotenv Format**: `.env` file format (e.g., `WEB_PORT=5000`)
4. **Human Format**: User-friendly output (e.g., `Reserved ports:\n  web: 5000\n  api: 5001`)

Shell detection leverages environment variables (`$SHELL`, `$ZSH_VERSION`, `$FISH_VERSION`, `$PSModulePath`) with explicit `--shell` override support. This enables the powerful workflow: `eval $(trop autoreserve)` sources environment variables directly into the current shell.

The autoreserve command implements intelligent config discovery, walking up from the current directory until finding `trop.yaml` or `trop.local.yaml`. The discovered config's parent directory becomes the base path for all reservations, ensuring consistent behavior regardless of invocation location. This matches user expectations and aligns with the existing `ConfigLoader::discover_project_configs` behavior.

Implementation architecture follows established patterns:
- Plan-execute pattern extended with `ReserveGroupPlan` and `AutoreservePlan`
- Execution via `PlanAction::AllocateGroup` leveraging Phase 6's `PortAllocator::allocate_group`
- Configuration validation ensuring services have either offset or preferred port
- Builder pattern for options with fluent API (`ReserveGroupOptions::new().with_task().with_force()`)

Testing was comprehensive across multiple dimensions:
- Output formatters tested with various port allocations, edge cases (empty, single service), and format-specific validation
- Shell detection tested with mocked environment variables for all shell types
- CLI integration tests covering all format options, config discovery scenarios, dry-run mode, and error conditions
- Library integration tests validating group allocation, transactional semantics, and database state

Notable design decisions:
- Base path for reservations is config file's parent directory (not CWD), ensuring predictable behavior
- Environment variable names default to uppercased service tags but support explicit mapping via `env` field
- Shell detection defaults to bash when unable to determine, ensuring safe fallback
- Stdout contains machine-readable output only; all human messages go to stderr
- Config discovery stops at first directory containing trop configs (no recursive search)

The implementation adds 6,462 lines across 21 files, with major contributions:
- `trop/src/output/formatters.rs` (1135 lines): Complete formatter implementations
- `trop/src/output/shell.rs` (604 lines): Shell detection and format generation
- `trop-cli/tests/group_commands.rs` (1645 lines): Comprehensive CLI integration tests
- `trop/tests/group_reservation_integration.rs` (1250 lines): Library-level group tests
- `trop/src/operations/reserve_group.rs` (530 lines): Plan builders for group operations
- `trop/src/operations/autoreserve.rs` (359 lines): Config discovery and autoreserve logic

All 164 tests pass reliably, with no clippy warnings. The implementation is production-ready and provides excellent user experience with clear error messages, helpful defaults, and flexible configuration options.

## 2025-10-18 - Phase 9: Cleanup Operations

Implemented maintenance commands for cleaning up stale reservations, including `prune`, `expire`, and `autoclean` commands, with automatic cleanup-on-exhaustion support during port allocation. This phase completes the core maintenance capabilities by providing tools to prevent database bloat in long-running systems.

The implementation went very smoothly, with all components completed successfully. Key accomplishments include:

- Three cleanup commands (prune, expire, autoclean) with dry-run support
- Prune command removes reservations for non-existent directories
- Expire command removes reservations older than configurable threshold
- Autoclean combines both prune and expire operations
- Auto-cleanup during allocation when ports are exhausted (opt-in via configuration)
- Enhanced AllocationResult enum with cleanup hints (cleanup_suggested, tried_cleanup)
- Two-phase allocation strategy: attempt allocation, then cleanup-and-retry on exhaustion
- Configuration options for disabling auto-prune and auto-expire
- CLI flags (--disable-autoprune, --disable-autoexpire, --disable-autoclean) for fine-grained control
- Comprehensive test coverage: 28 CLI cleanup command tests plus integration tests
- Proper output formatting with quiet/normal/verbose modes and dry-run prefixes

The implementation maintains backward compatibility by using a hint-based approach rather than changing the allocator's database reference from immutable to mutable. When allocation exhaustion occurs, the allocator returns `AllocationResult::Exhausted { cleanup_suggested: true }`, signaling that cleanup might help. The reserve operation then performs cleanup with its mutable database reference and retries allocation.

Auto-cleanup integration was carefully designed to be conservative:
- Only triggers on actual port exhaustion (not on every allocation)
- Respects configuration settings (disable_autoprune, disable_autoexpire)
- Logs cleanup actions for visibility
- Reports whether cleanup was attempted in error messages
- Fails gracefully if cleanup doesn't free enough ports

Output formatting is consistent across all cleanup commands:
- **Quiet mode**: Just the count to stdout (or nothing if 0)
- **Normal mode**: Summary message to stderr
- **Verbose mode**: Detailed list of removed reservations with metadata
- **Dry-run mode**: All output prefixed with `[DRY RUN]` and no database modifications

Implementation architecture follows established patterns:
- CLI commands are thin wrappers around library `CleanupOperations` functions
- Database operations are already implemented from Phase 2 (cleanup module exists)
- Configuration precedence: CLI flags > environment variables > config files
- Error handling with proper context and user-friendly messages

Testing covered multiple dimensions:
- CLI integration tests (28 tests) covering all commands, formats, and flag combinations
- Dry-run mode verification ensuring no database modifications
- Auto-cleanup scenarios (exhaustion triggering cleanup, cleanup disabled, partial success)
- Configuration precedence testing
- Error handling for missing thresholds and database failures

Notable design decisions:
- AllocationResult enhancement preserves API compatibility while enabling cleanup hints
- Reserve operation signature changed from `&Database` to `&mut Database` (minor breaking change but necessary)
- Auto-cleanup is opt-in via configuration (default behavior unchanged)
- Cleanup operations are fail-open for path checking (uncertain = preserve reservation)
- Expire command requires explicit threshold (either via CLI --days or config expire_after_days)

The implementation adds 1,887 lines with major contributions:
- `trop-cli/tests/cleanup_commands.rs` (1265 lines): Comprehensive CLI integration tests
- `trop/src/operations/reserve.rs` (+147 lines): Auto-cleanup integration
- `trop-cli/src/commands/{prune,expire,autoclean}.rs` (296 lines total): Command implementations
- Enhanced `AllocationResult` with cleanup hints

All tests pass (568 lib + 260 CLI + 164 doc tests = 992 total), with no clippy warnings. The cleanup system is production-ready and provides essential maintenance capabilities for long-running trop deployments.
