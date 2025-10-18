# `trop`

`trop` is a CLI tool that will be written in `rust`.

## Navigation Guide

Here's an overview of the repository's structure:

@AGENTIC_NAVIGATION_GUIDE.md

## Reference Documents

The `reference` directory contains two important documents:

- `ImplementationSpecification.md`: high-level specification for the `trop` tool
- `ImplementationPlan.md`: high-level implementation plan for the `trop` tool (broken down into a linear sequence of "phases")

These files should be treated as the source of truth for the `trop` tool; agents should consult them for context and clarification. Agents should **NEVER** modify the `ImplementationSpecification.md`; in some circumstances, agents *may* be asked to modify the `ImplementationPlan.md`, and should *only* modify that file if-and-when explcitly directed to do so.

Development will generally follow the plan outlined in `ImplementationPlan.md`, mixed in with occasional ad-hoc directives.

## Meta-Goal: Autonomous Development

The primary purpose of this repository is to bring the `trop` tool into existence, as per the specification documents in the `reference` directory.

Additionally, this repository is an experiment in "hands off", "high-autonomy" agentic delegation: the repo contains multiple agents and slash commands meant to facilitate tackling major blocks of work in a "one-shot-ish" fashion, and agents should keep that meta-goal in mind as they operate.

## Testing

- Run all tests: `cargo test`
- Run clippy: `cargo clippy`
- Format code: `cargo fmt`
- Build release: `cargo build --release`

## GitHub PR Comments

When posting comments to PRs using `gh pr comment`, the comment will be attributed to whatever GitHub account is authenticated with the `gh` CLI (typically the user's account). To ensure proper attribution, add explicit bot attribution in comment text (e.g., "🤖 Posted by Claude Code" at the end of comments).

## Current Status

- Working On Initial Implementation (as per `ImplementationPlan.md`)
- Phase 1 (Project Scaffold) is COMPLETED and merged
- Phase 2 (SQLite Database Layer) is COMPLETED and merged
- Phase 3 (Path Handling System) is COMPLETED and merged
- Phase 4 (Basic Reservation Operations) is COMPLETED and merged
- Phase 5 (Configuration System) is COMPLETED and merged
- Phase 6 (Port Allocation & Occupancy) is COMPLETED and merged
- Phase 7 (Essential CLI Commands) is COMPLETED and merged
- Phase 8 (Group Reservations) is COMPLETED and merged
- Phase 9 (Cleanup Operations) is COMPLETED and ready for merge
