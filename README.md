# trop

[![CI](https://github.com/prb/trop/actions/workflows/ci.yml/badge.svg)](https://github.com/prb/trop/actions/workflows/ci.yml)

> **Note:** GitHub only renders a status badge after the referenced workflow has produced at least one run. If the badge above
> shows "no status" or a gray placeholder, trigger the workflow manually via the "Run workflow" button or push a commit to
> `main` or `develop` so the CI pipeline records an initial result that the badge can display.

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

- **Idempotent reservations**: Reservations are sticky and keyed by directory—repeated invocations in the same directory receive a stable port.
- **Directory-based lifecycle**: Reservations can be automatically pruned once their associated directory has been removed—no need to register hooks or perform manual clean up.
- **Cross-process safety**: Safe to invoke `trop` concurrently from multiple processes (e.g. by multiple concurrent, indenently-operating agents).
- **Port occupancy detection & Exclusion Management**: `trop` avoids conflict with non-`trop` managed ports by:
  - verifying a prospective port is unoccupied before creating the reservation
  - allowing users to explicitly exclude specific ports and port-ranges from `trop` 
- **Easy Integration**: hardcoded port numbers can generally be replaced by calls to `trop reserve`

See `trop --help` or `man trop` for complete usage details.

## Advanced Usage

Full documentation for `trop` advanced's features is forthcoming, but here's a brief overview of `trop`'s advanced features.

### Tags & `trop autoreserve`

For projects with multiple services, you can reserve a distinct port for each service, like so:

```bash
WEB_PORT=$(trop reserve --tag web)
API_PORT=$(trop reserve --api db)
DB_PORT=$(trop reserve --tag db)
```

As with `trop reserve`, these reservations will be associated with the current directory, and thus will be automatically pruned when the directory is removed. 

For recurring reservation patterns, you add a "tropfile" (`trop.yaml`) file to your project root, which can then define a "reservation group" like so:

```yaml
reservations:
  services:
    web:
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    db:
      offset: 2
      env: DB_PORT
```

With that file in place, you can then reserve all ports and inject them into the environment like so:

```bash
eval "$(trop autoreserve)"
```

### Projects and Tasks

`trop` reservations are *keyed* by a path and optional tag, but support two additional metadata fields:

- `project`: A human-readable name for the *project* associated with the reservation
- `task`: A human-readable name for the *task* associated with the reservation

Although you *can* supply these values via the `--project` and `--task` flags, convenient defaults have been provided for the "multiple agents in multiple worktrees" scenario:

- `project` defaults to the name of the associated git repo 
- `task` defaults to the name of the current worktree or branch

Both of these fields are optional and have no impact on port-reservation behavior, but can be useful for inspection and debugging.

## Installation

### From crates.io

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

## Status

This release should be considered a "preview" release: the core functionality is implemented, heavily-tested, and appears to work, but has not *yet* been heavily used in real-world scenarios. 
As such, expect potential bugs and breaking changes—appreciate all early adopters and welcome any feedback!

## License

MIT
