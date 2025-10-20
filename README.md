# trop

[![CI](https://github.com/prb/trop/actions/workflows/ci.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/ci.yml)
[![Multi-Platform Tests](https://github.com/prb/trop/actions/workflows/multi-platform.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/multi-platform.yml)
[![Code Coverage](https://github.com/prb/trop/actions/workflows/coverage.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/coverage.yml)

`trop` is a lightweight, directory-aware port reservation tool designed for managing ephemeral port allocations in concurrent development workflows. It helps prevent port collisions when multiple processes, agents, or worktrees need to run services on predictable ports.

## Current Status

**Phase 12 (Testing and Polish)** - Core functionality complete, documentation added

This project has completed Phases 1-11, implementing all core functionality:

- ✅ Core types and project scaffold
- ✅ SQLite database layer with ACID properties
- ✅ Path handling and canonicalization
- ✅ Reservation operations (create, query, release)
- ✅ Configuration system with TOML support
- ✅ Port allocation and occupancy checking
- ✅ Essential CLI commands (reserve, release, list, get)
- ✅ Group reservations for multi-service projects
- ✅ Cleanup operations
- ✅ Assertion and utility commands
- ✅ Migration and advanced operations
- ✅ Property-based tests and concurrency tests
- ✅ Man pages and shell completions

The tool is now feature-complete and undergoing final polish before 1.0 release.

## What is trop?

`trop` provides a CLI tool for idempotent, directory-based port reservation management. Key features:

- **Idempotent reservations**: Repeated requests for the same directory return the same port
- **Directory-based lifecycle**: Automatic cleanup when directories are removed
- **Cross-process safety**: Uses SQLite for ACID properties
- **Hierarchical configuration**: Supports user-level and project-level config files
- **Group reservations**: Reserve multiple contiguous ports for multi-service projects
- **Port occupancy detection**: Automatically avoid ports already in use
- **Shell completions**: First-class support for bash, zsh, fish, and PowerShell

Typical usage:

```justfile
# Reserve a port for the current directory
port := $(trop reserve)

preview:
  npm run preview -- --port {{port}}
```

## Quick Start

Once built, initialize trop and reserve your first port:

```bash
# Initialize trop (creates database and default config)
trop init --with-config

# Reserve a port for the current directory
cd ~/projects/my-app
trop reserve

# Use the port in your application
PORT=$(trop reserve)
npm start -- --port $PORT

# List all reservations
trop list

# Release when done
trop release
```

See the [examples/](examples/) directory for more detailed usage guides.

## Documentation

### User Documentation

- [Basic Usage Guide](examples/basic_usage.md) - Get started in 5 minutes
- [Examples](examples/) - Practical examples for common scenarios
- Man page: `man trop` (after installation)
- Shell completions: `trop completions <shell>`

### Developer Documentation

- [Implementation Specification](reference/ImplementationSpecification.md) - Complete specification
- [Implementation Plan](reference/ImplementationPlan.md) - Phased development plan
- API documentation: `cargo doc --open`

## Installation

### Install from source

```bash
git clone https://github.com/your-org/trop
cd trop
cargo install --path trop-cli
```

### Install man pages (optional)

```bash
# After building
sudo cp target/release/build/trop-cli-*/out/man/trop.1 /usr/local/share/man/man1/
sudo mandb
```

### Install shell completions (optional)

For bash:
```bash
trop completions bash > ~/.local/share/bash-completion/completions/trop
```

For zsh:
```bash
trop completions zsh > ~/.zsh/completions/_trop
# Add ~/.zsh/completions to your $fpath in .zshrc
```

For fish:
```bash
trop completions fish > ~/.config/fish/completions/trop.fish
```

For PowerShell:
```powershell
trop completions powershell > $PROFILE
```

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

### Benchmarks

`trop` ships Criterion benchmarks covering both the core library and the CLI to guard against performance regressions. Run the smoke suite locally with:

```bash
cargo bench -p trop --bench operations_bench -- --sample-size 10 --measurement-time 2
cargo bench -p trop-cli --bench cli_bench -- --sample-size 10 --measurement-time 2
```

Recent baseline numbers (median of 10 samples on a shared CI runner):

| Library benchmark | Scenario | Median time |
| ----------------- | -------- | ----------- |
| `reserve_single` | Allocate one reservation | 476 µs 【c65538†L1-L3】 |
| `reserve_bulk/250` | Allocate 250 sequential reservations | 213 ms 【b60669†L1-L2】 |
| `lookup_by_path/1000` | Fetch reservation from a 1000-row dataset | 1.05 ms 【966eac†L1-L3】 |
| `list_reservations/1000` | List 1000 reservations | 1.82 ms 【47e436†L1-L3】 |
| `release_reservation` | Delete an existing reservation | 383 µs 【04e778†L1-L2】 |

| CLI benchmark | Scenario | Median time |
| ------------- | -------- | ----------- |
| `cli_startup_version` | Process startup + `--version` | 8.39 ms 【4dae56†L1-L3】 |
| `cli_reserve` | `trop reserve` against a fresh data dir | 19.9 ms 【c8c021†L1-L3】 |
| `cli_list` | `trop list --format json` with 50 reservations | 17.8 ms 【54929a†L1-L3】 |

These figures meet the phase targets (<10 ms for library reserve, <50 ms for CLI startup, <20 ms for list operations) and establish a baseline for future changes.【c65538†L1-L3】【47e436†L1-L3】【4dae56†L1-L3】

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

## Project Status

**Pre-1.0 Release**

The core functionality is feature-complete and stable, but trop is still in pre-release status:

- ✅ All planned features implemented and tested
- ⚠️ Minor API refinements may occur before 1.0
- ✅ Backward compatibility maintained after 1.0 release
- 📝 Please report any issues or unexpected behavior

Consider this software "beta" quality - ready for real use with the understanding that the 1.0 API is not yet final.
