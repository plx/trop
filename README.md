# trop

`trop` is a lightweight, directory-aware port reservation tool designed for managing ephemeral port allocations in concurrent development workflows. It helps prevent port collisions when multiple processes, agents, or worktrees need to run services on predictable ports.

## Current Status

**Phase 1 Complete** - Core types and project scaffold implemented

This is early-stage development software. The project has completed Phase 1, which includes:

- Core types: `Port`, `PortRange`, `Reservation`, `ReservationKey`
- Error handling infrastructure
- Logging system
- Project structure (library + CLI crates)
- Unit tests for core functionality
- Integration test scaffolding

The CLI currently only displays version information. Full functionality is under active development.

## What is trop?

`trop` provides a CLI tool for idempotent, directory-based port reservation management. Key features (planned):

- **Idempotent reservations**: Repeated requests for the same directory/tag return the same port
- **Directory-based lifecycle**: Automatic cleanup when directories are removed
- **Cross-process safety**: Uses SQLite for ACID properties
- **Hierarchical configuration**: Supports user-level and project-level config files
- **Multiple ports per directory**: Use tags to reserve multiple ports (e.g., "web", "api", "db")

Typical usage (once implemented):

```justfile
# Reserve a port for the current directory
port := $(trop reserve)

preview:
  npm run preview -- --port {{port}}
```

## Documentation

For detailed information about the tool's design and planned features:

- [Implementation Specification](reference/ImplementationSpecification.md) - Complete specification
- [Phase 1 Implementation Plan](plans/phases/phase-01-project-scaffold.md) - Current phase details

## Development

### Prerequisites

- Rust 1.70.0 or later (2021 edition)
- Cargo (comes with Rust)

### Installing Rust

If you don't have Rust installed:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Follow the on-screen instructions, then restart your terminal.

### Building

Build the project in development mode:

```bash
cargo build
```

Build optimized release binary:

```bash
cargo build --release
```

The binary will be located at:
- Debug: `target/debug/trop`
- Release: `target/release/trop`

### Running

Run the CLI directly with cargo:

```bash
# Show version
cargo run -- --version

# Show help
cargo run -- --help
```

Or run the built binary directly:

```bash
./target/debug/trop --version
./target/release/trop --version
```

### Testing

Run all tests (unit and integration):

```bash
cargo test --all
```

Run only unit tests:

```bash
cargo test --lib
```

Run only integration tests:

```bash
cargo test --test '*'
```

Run tests with output:

```bash
cargo test -- --nocapture
```

### Code Quality

Format code with rustfmt:

```bash
cargo fmt --all
```

Check formatting without modifying files:

```bash
cargo fmt --all -- --check
```

Run clippy for linting:

```bash
cargo clippy --all-targets --all-features
```

Run clippy with warnings as errors:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### Documentation

Generate and open documentation:

```bash
cargo doc --open
```

Generate documentation for all dependencies:

```bash
cargo doc --open --document-private-items
```

## Project Structure

The project is organized as a Rust workspace with two crates:

- **`trop/`** - Library crate containing core types and logic
- **`trop-cli/`** - Binary crate providing the CLI interface

This separation allows the core functionality to be used programmatically while providing a convenient CLI tool.

### Agentic Navigation Guide

This project includes an `AGENTIC_NAVIGATION_GUIDE.md` file that helps AI coding assistants navigate the project structure. The guide:

- Lists all important files and directories with helpful comments
- Explains the purpose of agents and slash commands in `.claude/`
- Provides context about the project's development workflow
- Is automatically validated in CI to ensure it stays up-to-date

To maintain the navigation guide:

```bash
# Install the agentic-navigation-guide tool (if not already installed)
cargo install agentic-navigation-guide

# Verify the guide matches the filesystem
agentic-navigation-guide verify

# Regenerate the guide (if needed)
agentic-navigation-guide init --output AGENTIC_NAVIGATION_GUIDE.md --exclude '.git/**' --exclude 'target/**'
```

The CI pipeline automatically checks that the navigation guide is valid on every pull request.

## Installation

Not yet published to crates.io. Once development is further along, installation will be:

```bash
cargo install trop-cli
```

For now, build from source using the instructions above.

## Scope and Limitations

`trop` is designed for single-user, local development coordination:

- **Single-user only**: Manages per-user databases, not system-wide coordination
- **Coordination, not enforcement**: Tracks reservations but doesn't prevent other processes from using ports
- **Localhost focused**: Manages port numbers on localhost, not network-wide allocation

This makes it perfect for coordinating between multiple AI agents, worktrees, or concurrent development processes under a single user account.

## License

MIT

## Contributing

This is an experimental project exploring high-autonomy agentic development patterns. Contributions are welcome once the project reaches a more stable state.

## Warning

**This is alpha-stage development software. It is not ready for production use.**

- The CLI currently has minimal functionality
- Database schema and API are subject to change
- No backward compatibility guarantees yet
- Full documentation pending implementation

Check back as development progresses!
