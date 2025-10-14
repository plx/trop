# Agentic Navigation Guide

This navigation guide helps AI coding assistants understand the structure of this project.

## Project Overview

`trop` is a CLI tool for TCP port reservation management, written in Rust. It's being developed using a "high-autonomy" agentic approach with specialized agents and slash commands to facilitate hands-off development.

**Important**: The specification in `reference/ImplementationSpecification.md` is the source of truth and should NEVER be modified during development.

## Key Locations

- **Specifications**: `reference/` directory contains the authoritative spec and implementation plan
- **Implementation Plans**: `plans/phases/` contains phase-specific implementation plans
- **Library Code**: `trop/` workspace member contains core functionality
- **CLI Binary**: `trop-cli/` workspace member contains the CLI interface
- **Agents**: `.claude/agents/` contains specialized agent definitions for various tasks
- **Commands**: `.claude/commands/trop/` contains slash commands for high-level workflows

<agentic-navigation-guide>
- .claude/ # Claude Code configuration
  - agents/ # Specialized agents for different development tasks
    - github-actions-specialist.md # Handles all GitHub Actions CI/CD work
    - phase-completion-verifier.md # Verifies phase implementation completeness
    - phase-planner.md # Creates detailed implementation plans from high-level goals
    - plan-critic.md # Reviews and validates implementation plans
    - project-administrivia-drone.md # Handles project setup and tooling
    - project-historian.md # Documents completed work retrospectively
    - property-test-writer.md # Adds property-based tests for invariants
    - rust-compilation-fixer.md # Fixes compilation errors and clippy warnings
    - rust-implementer.md # Implements well-defined Rust coding tasks
    - rust-test-writer.md # Writes comprehensive unit and integration tests
    - test-failure-investigator.md # Investigates unexpected test failures
  - commands/
    - trop/ # Project-specific slash commands for workflow automation
      - implement-plan.md # Execute implementation from a plan
      - open-phase-pr.md # Create PR for completed phase
      - prepare-plan.md # Create implementation plan for a phase
      - undertake-phase.md # Full workflow: plan → implement → verify → PR
      - verify-completion.md # Check if phase is complete
  - settings.local.json # Local Claude settings (not in git)
- .clippy.toml # Clippy linter configuration (strict mode enabled)
- .git/ # Git repository internals
- .github/ # GitHub configuration
  - workflows/ # CI/CD workflows
    - claude-code-review.yml # Automated code review on PRs
    - claude.yml # General CI: build, test, clippy, fmt checks
- .gitignore # Git ignore patterns
- .rustfmt.toml # Rustfmt formatter configuration
- AGENTS.md # Documentation about available agents
- AGENTIC_NAVIGATION_GUIDE.md # This file - helps agents navigate the project
- CLAUDE.md # Main instructions for Claude Code (included in context)
- Cargo.lock # Dependency lock file
- Cargo.toml # Workspace root manifest (defines trop and trop-cli members)
- LICENSE # MIT license
- README.md # Project overview and getting started
- plans/ # Implementation planning documents
  - phases/ # Phase-specific implementation plans
    - phase-01-project-scaffold.md # COMPLETED: Foundation, core types, errors
- reference/ # Authoritative project specifications (DO NOT MODIFY)
  - ImplementationPlan.md # High-level phased development plan
  - ImplementationSpecification.md # Complete spec - SOURCE OF TRUTH
- target/ # Rust build output directory (not in git)
- trop/ # Library workspace member - core functionality
  - Cargo.toml # Library manifest with dependencies
  - README.md # Library documentation
  - src/ # Library source code
    - error.rs # Error types using thiserror
    - lib.rs # Library root - exports public API
    - logging.rs # Logging infrastructure (stderr output)
    - port.rs # Port and PortRange types with validation
    - reservation.rs # Reservation and ReservationKey types
  - tests/ # Integration tests
    - common/ # Shared test utilities
      - mod.rs # Common test helpers
    - integration_test.rs # Integration test suite
- trop-cli/ # Binary workspace member - CLI interface
  - Cargo.toml # Binary manifest with dependencies
  - README.md # CLI documentation
  - src/ # CLI source code
    - main.rs # Entry point with argument parsing
  - tests/ # CLI integration tests
    - cli.rs # Command-line interface tests
</agentic-navigation-guide>

## Development Workflow

1. **Planning**: Use `/trop:prepare-plan` to create phase implementation plans
2. **Review**: Plans are reviewed by the plan-critic agent
3. **Implementation**: Use `/trop:undertake-phase` for end-to-end phase completion
4. **Verification**: Use `/trop:verify-completion` to ensure all phase requirements met
5. **PR Creation**: Use `/trop:open-phase-pr` to create pull request for completed phase

## Testing

- Run all tests: `cargo test`
- Run clippy: `cargo clippy`
- Format code: `cargo fmt`
- Build release: `cargo build --release`

## Current Status

- Phase 1 (Project Scaffold) is COMPLETED and merged
- Working on Phase 2+ (see `plans/phases/` for upcoming work)

Note: This guide was automatically generated and manually enhanced with helpful comments.
