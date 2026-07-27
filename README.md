# trop

[![CI](https://github.com/plx/trop/actions/workflows/ci.yml/badge.svg)](https://github.com/plx/trop/actions/workflows/ci.yml)

> **Note:** GitHub only renders a status badge after the referenced workflow has produced at least one run. If the badge above
> shows "no status" or a gray placeholder, trigger the workflow manually via the "Run workflow" button or push a commit to
> `main` or `develop` so the CI pipeline records an initial result that the badge can display.

A lightweight, directory-aware port reservation tool for managing ephemeral port allocations in concurrent development workflows.

Website and docs: https://plx.github.io/trop/

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
API_PORT=$(trop reserve --tag api)
DB_PORT=$(trop reserve --tag db)
```

As with `trop reserve`, these reservations will be associated with the current directory, and thus will be automatically pruned when the directory is removed. 

For recurring reservation patterns, you add a "tropfile" (`trop.yaml`) file to your project root, which can then define a "reservation group" like so:

```yaml
reservations:
  services:
    web:
      offset: 0
      preferred: 8080
      env: WEB_PORT
    api:
      offset: 1
      env: API_PORT
    db:
      offset: 2
      env: DB_PORT
```

Each service offset is unique within the group and defaults to `0` when
omitted, including for a service that also has `preferred`. A preferred port
may be any valid port from 1 through 65535; it does not need to be inside the
configured scan range or match the service's offset. Trop pins available
preferred ports first. If a preference is reserved, excluded, or occupied, the
service joins the offset fallback pattern instead. The fallback base is the
lowest candidate in the configured scan range that fits the complete pattern
without colliding with another reservation, the operating system, an
exclusion, or a preferred port pinned by the same request.

Every reservation service must resolve to a portable `export`/`dotenv`
identifier, regardless of the selected output format. Explicit `env` names must
be at most 255 bytes and match `[A-Za-z_][A-Za-z0-9_]*`. When `env` is omitted,
trop accepts ASCII service tags that become valid names after converting ASCII
letters to uppercase and replacing `-` with `_`; all other tags require an
explicit valid mapping.
Resolved names must also be unique when compared without ASCII case.

With that file in place, reserve all ports and inspect the resulting mapping by
choosing one non-executable output format:

```bash
trop autoreserve --format human
# or
trop autoreserve --format json
```

Compatible group requests are idempotent: repeated `reserve-group`,
`autoreserve`, or alternating invocations return the same service-to-port
mapping, preserve creation timestamps, and refresh each service's last-used
time in one transaction. A stored group is compatible when its complete tagged
service set matches the configuration and its ports still satisfy the requested
preferred/offset shape. Partial groups and changed service shapes fail without
modifying any group row.

Group metadata and paths use the same safety model as single reservations.
Explicit project or task changes require `--allow-project-change`,
`--allow-task-change`, their combined `--allow-change` form, or `--force`;
omitting either value preserves the stored value because there is no metadata
clearing interface. The group path must be the current directory, an ancestor,
or a descendant unless `--allow-unrelated-path` or `--force` is supplied.
Narrow flags authorize only their named check.

For groups, `--force` combines the path and metadata permissions with
authorization to replace an incompatible exact-path tagged group atomically.
Replacement may choose a new mapping, resets creation times for the replacement
set, and leaves same-path untagged reservations and descendant reservations
alone. It does not bypass invalid configuration, exclusions, operating-system
occupancy, range exhaustion, or another reservation key's ownership of a port.
If replacement fails, the original exact-path tagged state is restored.

Version 0.1.0 does not safely validate every generated variable name in
`export` or `dotenv` output from `autoreserve` and `reserve-group`. Both 0.1.0
crates are yanked, but yanking does not remove installed binaries or update
existing lockfiles. Upgrade the CLI explicitly:

```bash
cargo install trop-cli --version 0.2.0 --locked --force
```

Version 0.2.0 rejects invalid identifiers. See
[GHSA-h2jc-jr86-m5vq](https://github.com/plx/trop/security/advisories/GHSA-h2jc-jr86-m5vq)
for affected usage and remediation. Until upgraded, use human or JSON output,
inspect the result, and set only trusted variables manually.

### Configuration overlays

At the nearest project boundary, `trop.yaml` and `trop.local.yaml` compose as
one effective configuration. Ordinary values merge by documented precedence,
occupancy settings merge one explicit leaf at a time, and exclusions accumulate.
An omitted value inherits the next lower layer.

Reservation groups are the deliberate exception because merging service maps
could create an accidental group shape. An omitted `reservations` key inherits
the lower-precedence group, a non-null mapping replaces the complete group, and
`reservations: null` explicitly clears it. A cleared group makes
`reserve-group` and `autoreserve` fail without changing stored reservations.
In user-wide `config.yaml`, a generated `reservations: null` remains inert
because that source is not permitted to define project reservation groups.

Explicitly naming `trop.yaml` or `trop.local.yaml` with `reserve-group` loads
both sibling files when present. An arbitrarily named configuration file is a
standalone project source. Both group entrypoints otherwise consume the same
built-ins, user configuration, project layers, environment, and command-line
overrides.

### Path identity

`trop` stores reservation paths as lexically normalized absolute paths. An
explicit path from `--path` or `TROP_PATH` keeps the spelling supplied by the
user and does not follow symbolic links. The explicit target does not need to
exist when it is resolved, although an individual command may impose its own
existence checks.

When no path is supplied, `trop` infers the path from the current working
directory and canonicalizes it. This makes physical and symbolic-link routes to
the same working directory share one reservation identity. If that inferred
path cannot be canonicalized, the command reports an error instead of storing
an unstable identity. `trop show-path --canonicalize` also forces
canonicalization, so its target must exist.

Project configuration discovery follows the same inferred-path rule: it starts
from the canonical absolute working directory and walks its real parents,
stopping at the nearest directory containing `trop.yaml` or
`trop.local.yaml`.

Group commands retain the resolved configuration filename for diagnostics, but
reservation identity is inferred from that file's containing directory and is
always canonicalized. Consequently, bare, relative, absolute, and
symbolic-link directory routes to the same group configuration share one
absolute stored path.

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
git clone https://github.com/plx/trop
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

`trop` is dual-licensed under either:

- [Apache License, Version 2.0](LICENSE-APACHE), or
- [MIT License](LICENSE-MIT),

at your option.

The licenses of all third-party crates that ship with `trop` and `trop-cli`
are enumerated in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). That file
is auto-generated by [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)
from `about.toml` and `about.hbs`; CI rejects any change that leaves it stale.
Regenerate locally with `just licenses`.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `trop` by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
