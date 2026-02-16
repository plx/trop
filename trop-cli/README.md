# trop-cli

CLI for `trop`, a directory-aware port reservation tool for local development.

## Install

From this repository:

```bash
cargo install --path trop-cli
```

Run directly without installing:

```bash
cargo run -p trop-cli -- --help
```

## Quick Start

```bash
# Reserve a stable port for the current directory
PORT=$(trop reserve)

# Reserve service-specific ports in the same directory
WEB_PORT=$(trop reserve --tag web)
API_PORT=$(trop reserve --tag api)

# Inspect current reservations
trop list

# Release one reservation
trop release --tag web
```

## Group Reservations

Define services in `trop.yaml`:

```yaml
project: my-app
ports:
  min: 5000
  max: 7000
reservations:
  services:
    web:
      offset: 0
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
```

Reserve all services from the file:

```bash
trop reserve-group trop.yaml
```

Auto-discover `trop.yaml`/`trop.local.yaml` from the current directory upward:

```bash
eval "$(trop autoreserve)"
```

## Implemented Commands

Core reservation workflow:

- `trop reserve`
- `trop release`
- `trop list`
- `trop reserve-group`
- `trop autoreserve`

Cleanup and maintenance:

- `trop prune`
- `trop expire`
- `trop autoclean`
- `trop migrate`

Assertions and inspection:

- `trop assert-reservation`
- `trop assert-port`
- `trop assert-data-dir`
- `trop port-info`
- `trop list-projects`
- `trop show-data-dir`
- `trop show-path`

Config and occupancy tooling:

- `trop init`
- `trop validate`
- `trop scan`
- `trop exclude`
- `trop compact-exclusions`

Utility:

- `trop completions`

## Useful Flags

Global flags (place before subcommand):

- `--data-dir <PATH>`
- `--busy-timeout <SECONDS>`
- `--disable-autoinit`
- `--verbose`
- `--quiet`

Common command flags:

- `trop reserve --tag <TAG> --project <PROJECT> --task <TASK>`
- `trop reserve --min <PORT> --max <PORT>`
- `trop list --format <table|json|csv|tsv>`
- `trop release --recursive`
- `trop scan --autoexclude --autocompact`

## Configuration Sources

Configuration precedence (highest to lowest):

1. CLI flags
2. Environment variables
3. `trop.local.yaml`
4. `trop.yaml`
5. `~/.trop/config.yaml`
6. Built-in defaults

## Development

```bash
cargo test
cargo clippy
cargo fmt
cargo build --release
```

## License

MIT
