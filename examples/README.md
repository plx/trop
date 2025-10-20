# Trop Examples

This directory contains practical examples and tutorials for using trop.

## Quick Start

1. [Basic Usage](basic_usage.md) - Get started with trop in 5 minutes
2. [Docker Compose](docker_example/) - Managing ports across containerized services

## Configuration Examples

See the [`configs/`](configs/) directory for example configuration files:

- `simple.toml` - Minimal configuration for single developers
- `team.toml` - Team environment with shared exclusions

## Installation

If you haven't installed trop yet:

```bash
# Build from source
git clone https://github.com/prb/trop
cd trop
cargo build --release

# The binary will be at target/release/trop
```

## Getting Help

- Run `trop --help` for a list of all commands
- Run `trop <command> --help` for command-specific help
- Check the generated man page: `man trop` (after installation)
- Generate shell completions: `trop completions <shell>`
