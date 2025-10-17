# trop-cli

Command-line interface for `trop`, a lightweight port reservation management tool for concurrent development workflows.

## Overview

`trop` helps prevent port collisions when running multiple development servers, test suites, or other services that need to bind to network ports. It provides idempotent, directory-based port reservations that work across multiple processes and concurrent agents.

## Current Status

**Early Development - Phase 1 Complete**

Currently, the CLI only provides version and help output. Full functionality is under active development.

Phase 1 accomplishments:
- Project scaffold and build system
- Core library types implemented
- CLI framework in place
- Integration test infrastructure

Coming in Phase 2:
- Database layer
- Port reservation commands
- Configuration system

## Installation

### Building from Source

This is currently the only installation method:

```bash
# Clone the repository
git clone https://github.com/prb/trop.git
cd trop

# Build in release mode
cargo build --release

# The binary will be at: target/release/trop
```

Optionally, copy to a directory in your PATH:

```bash
cp target/release/trop ~/.local/bin/  # or /usr/local/bin/, etc.
```

### Future: Install from crates.io

Once published (not yet available):

```bash
cargo install trop-cli
```

## Basic Usage

Currently available commands:

```bash
# Show version
trop --version

# Show help
trop --help
```

## Planned Usage

Once fully implemented, typical workflows will include:

### Simple Reservation

Reserve a port for the current directory:

```bash
port=$(trop reserve)
echo "Using port: $port"
```

### Tagged Reservations

Reserve multiple ports for the same directory:

```bash
web_port=$(trop reserve --tag web)
api_port=$(trop reserve --tag api)
db_port=$(trop reserve --tag db)
```

### Use in Build Scripts

Example `justfile`:

```justfile
port := `trop reserve`

dev:
  npm run dev -- --port {{port}}

test:
  npm test -- --port {{port}}
```

### Group Reservations

Reserve multiple ports at once from a `trop.yaml` file:

```bash
# Create trop.yaml with port definitions
trop reserve-group ./trop.yaml

# Or auto-discover trop.yaml
trop autoreserve
```

## Configuration

`trop` will support hierarchical configuration:

1. Command-line arguments (highest priority)
2. Environment variables
3. `trop.local.yaml` (project-specific, not in source control)
4. `trop.yaml` (project config, checked into source control)
5. `~/.trop/config.yaml` (user-level defaults)
6. Built-in defaults (lowest priority)

### Example trop.yaml

```yaml
project: my-app

ports:
  min: 5000
  max: 7000

excluded_ports:
  - 5432  # PostgreSQL default

reservations:
  base: 5000
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
```

See the [implementation specification](../reference/ImplementationSpecification.md) for complete configuration details.

## Environment Variables

Key environment variables (planned):

- `TROP_DATA_DIR`: Override data directory location (default: `~/.trop`)
- `TROP_LOG_MODE`: Control logging verbosity (`quiet`, `normal`, `verbose`)
- `TROP_PROJECT`: Set project identifier
- `TROP_DISABLE_AUTOINIT`: Disable automatic database initialization

## Planned Commands

### Core Operations

- `trop reserve` - Reserve a port for current directory
- `trop release` - Release a reservation
- `trop list` - List all active reservations
- `trop reserve-group` - Reserve multiple ports from config
- `trop autoreserve` - Auto-discover and reserve port group

### Inspection

- `trop port-info <port>` - Show reservation info for a port
- `trop assert-reservation` - Check if reservation exists (exit code 0/1)
- `trop assert-port <port>` - Check if port is reserved (exit code 0/1)
- `trop list-projects` - List all active projects

### Management

- `trop prune` - Remove reservations for deleted directories
- `trop expire` - Remove stale reservations
- `trop autoclean` - Combined prune and expire
- `trop migrate` - Move reservations between paths

### Configuration

- `trop init` - Initialize data directory and config
- `trop validate <config>` - Validate a trop.yaml file
- `trop scan` - Scan for occupied ports
- `trop exclude <port>` - Add port to exclusion list

### Utility

- `trop show-data-dir` - Print data directory path
- `trop show-path` - Print resolved path for reservation

## Exit Codes

- `0` - Success
- `1` - Semantic error (e.g., assertion failed)
- `2` - Timeout (e.g., database lock)
- `3` - No data directory found
- `4+` - Other errors

## Logging

All diagnostic output goes to stderr. Port numbers and structured output go to stdout, making the tool suitable for command substitution:

```bash
port=$(trop reserve)  # Only the port number goes to stdout
```

Control verbosity:

```bash
trop --quiet reserve      # Minimal output
trop reserve              # Normal output
trop --verbose reserve    # Detailed output
```

Or via environment:

```bash
export TROP_LOG_MODE=verbose
trop reserve
```

## Use Cases

### Multiple Worktrees

```bash
# Each worktree gets its own port automatically
cd ~/projects/myapp/feature-1
port=$(trop reserve)

cd ~/projects/myapp/feature-2
port=$(trop reserve)  # Different port!
```

### Concurrent AI Agents

Multiple agents working on the same codebase in different directories automatically get non-conflicting ports.

### Test Isolation

Each test suite run reserves its own ports, preventing interference between parallel test runs.

## Limitations

- **Single-user coordination only**: Not designed for multi-user systems
- **Coordination, not enforcement**: Doesn't prevent other processes from using ports
- **Localhost focused**: Manages port numbers, not network interfaces

See the [root README](../README.md) for more details.

## Documentation

- [Implementation Specification](../reference/ImplementationSpecification.md) - Complete design
- [Library API Documentation](../trop/README.md) - Core library types
- [Root README](../README.md) - Project overview and development setup

Generate API documentation:

```bash
cargo doc --open
```

## Development

### Running Tests

```bash
# Run all tests (unit + integration)
cargo test

# Run only CLI integration tests
cargo test --test cli
```

### Development Mode

```bash
# Run without building first
cargo run -- --version

# With arguments
cargo run -- --help
```

## Binary Details

- **Binary name**: `trop`
- **Crate name**: `trop-cli`
- **Depends on**: `trop` library crate

The CLI is a thin wrapper around the `trop` library, providing command-line parsing and output formatting.

## License

MIT

## Support

This is an experimental project in active development. For issues or questions, see the [main repository](https://github.com/prb/trop).

## Warning

This is alpha-stage software. The CLI interface and behavior will change. Not recommended for production use yet.
