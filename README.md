# trop

[![CI](https://github.com/prb/trop/actions/workflows/ci.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/ci.yml)
[![Multi-Platform Tests](https://github.com/prb/trop/actions/workflows/multi-platform.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/multi-platform.yml)
[![Code Coverage](https://github.com/prb/trop/actions/workflows/coverage.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/coverage.yml)

A lightweight, directory-aware port reservation tool for managing ephemeral port allocations in concurrent development workflows.

## Overview

`trop` is a port-reservation management tool meant to act as a "drop-in" replacement for hardcoded ports, like so:

- without `trop`:
  ```bash
  # Reserve a port for the current directory
  PORT=4040
  npm start -- --port $PORT
  ```
- with `trop`:
  ```bash
  # Reserve a port for the current directory
  PORT=$(trop reserve)
  npm start -- --port $PORT
  ```

Key features:

- **Idempotent reservations**: Same directory always gets the same port
- **Directory-based lifecycle**: Automatic cleanup when directories are removed
- **Cross-process safety**: SQLite-backed ACID properties
- **Port occupancy detection**: Automatically avoid ports already in use
- **Easy Integration**: hardcoded port numbers can generally be replaced by calls to `trop reserve`

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

## Testing

The project includes comprehensive test coverage:

- Unit tests for all core functionality
- Integration tests for CLI commands
- Property-based tests for correctness guarantees
- Concurrency tests for race condition detection
- Benchmarks for performance regression testing

## License

MIT
