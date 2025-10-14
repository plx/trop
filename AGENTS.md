# `trop`

`trop` is a CLI tool that will be written in `rust`.

## Navigation Guide

Here's an overview of the repository's structure:

@AGENTIC_NAVIGATION_GUIDE.md

## Reference Documents

The `reference` directory contains two important documents:

- `specifications/ImplementationSpecification.md`: high-level specification for the `trop` tool
- `specifications/ImplementationPlan.md`: high-level implementation plan for the `trop` tool (broken down into a linear sequence of "phases")

These files should be treated as the source of truth for the `trop` tool; agents should consult them for context and clarification. Agents should **NEVER** modify the `specifications/ImplementationSpecification.md`; in some circumstances, agents *may* be asked to modify the `specifications/ImplementationPlan.md`, and should *only* modify that file if-and-when explcitly directed to do so.

Development will generally follow the plan outlined in `specifications/ImplementationPlan.md`, mixed in with occasional ad-hoc directives.

## Meta-Goal: Autonomous Development

The primary purpose of this repository is to bring the `trop` tool into existence, as per the specification documents in the `reference` directory.

Additionally, this repository is an experiment in "hands off", "high-autonomy" agentic delegation: the repo contains multiple agents and slash commands meant to facilitate tackling major blocks of work in a "one-shot-ish" fashion, and agents should keep that meta-goal in mind as they operate.

## Testing

- Run all tests: `cargo test`
- Run clippy: `cargo clippy`
- Format code: `cargo fmt`
- Build release: `cargo build --release`

## Current Status

- Working On Initial Implementation (as per `specifications/ImplementationPlan.md`)
- Phase 1 (Project Scaffold) is COMPLETED and merged
