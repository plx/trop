# trop

[![CI](https://github.com/prb/trop/actions/workflows/ci.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/ci.yml)
[![Multi-Platform Tests](https://github.com/prb/trop/actions/workflows/multi-platform.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/multi-platform.yml)
[![Code Coverage](https://github.com/prb/trop/actions/workflows/coverage.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/coverage.yml)

A lightweight, directory-aware port reservation tool for managing ephemeral port allocations in concurrent development workflows.

## Overview

`trop` helps prevent port collisions when multiple processes, agents, or worktrees need to run services on predictable ports. Key features:

- **Idempotent reservations**: Same directory always gets the same port
- **Directory-based lifecycle**: Automatic cleanup when directories are removed
- **Cross-process safety**: SQLite-backed ACID properties
- **Group reservations**: Reserve multiple contiguous ports for multi-service projects
- **Port occupancy detection**: Automatically avoid ports already in use

## Usage

```bash
# Reserve a port for the current directory
PORT=$(trop reserve)
npm start -- --port $PORT

# List all reservations
trop list

# Release the current directory's reservation
trop release

# Reserve a group of 3 contiguous ports
trop reserve --group myapp --count 3
trop get myapp
```

Integration with build tools:

```justfile
# In your justfile
port := $(trop reserve)

preview:
  npm run preview -- --port {{port}}
```

See `trop --help` or `man trop` for complete usage details.

## Installation

### From crates.io (coming soon)

```bash
cargo install trop-cli
```

### From source

```bash
git clone https://github.com/prb/trop
cd trop
cargo install --path trop-cli
```

After installation, initialize trop:

```bash
trop init --with-config
```

## Testing

The project includes comprehensive test coverage:

- Unit tests for all core functionality
- Integration tests for CLI commands
- Property-based tests for correctness guarantees
- Concurrency tests for race condition detection
- Benchmarks for performance regression testing

Run the full test suite:

```bash
cargo test --all
```

## License

MIT
