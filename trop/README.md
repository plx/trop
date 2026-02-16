# trop (Library Crate)

Core library for `trop`, a directory-aware port reservation system.

## Overview

The `trop` library is the implementation behind the `trop` CLI. It includes:

- Typed port and reservation models (`Port`, `PortRange`, `Reservation`, `ReservationKey`)
- Config loading and merging (`ConfigBuilder`) across `~/.trop/config.yaml`, `trop.yaml`, `trop.local.yaml`, and env vars
- SQLite storage with migrations and transactional operations (`Database`, `DatabaseConfig`)
- Path normalization and relationship checks (`PathResolver`, `PathRelationship`)
- Operation planning and execution (`ReservePlan`, `ReleasePlan`, `ReserveGroupPlan`, `AutoreservePlan`, `MigratePlan`, `PlanExecutor`)
- Cleanup operations (`CleanupOperations`, `PruneResult`, `ExpireResult`, `AutocleanResult`)
- Output helpers for human and machine-readable formats

## Main Types

### `Port`

A validated network port number (1-65535).

```rust
use trop::Port;

let port = Port::try_from(8080)?;
assert_eq!(port.value(), 8080);
assert!(!port.is_privileged());
# Ok::<(), trop::Error>(())
```

### `PortRange`

An inclusive range of valid ports.

```rust
use trop::{Port, PortRange};

let min = Port::try_from(5000)?;
let max = Port::try_from(5010)?;
let range = PortRange::new(min, max)?;

assert_eq!(range.len(), 11);
assert!(range.contains(Port::try_from(5005)?));
# Ok::<(), trop::Error>(())
```

### `ReservationKey`

A unique reservation key composed of a normalized path plus an optional tag.

```rust
use std::path::PathBuf;
use trop::ReservationKey;

let untagged = ReservationKey::new(PathBuf::from("/path/to/project"), None)?;
let tagged = ReservationKey::new(
    PathBuf::from("/path/to/project"),
    Some("web".to_string()),
)?;

assert_eq!(untagged.tag, None);
assert_eq!(tagged.tag.as_deref(), Some("web"));
# Ok::<(), trop::Error>(())
```

## Operations API

For programmatic workflows, use option + plan pairs and execute inside a transaction:

- `ReserveOptions` + `ReservePlan`
- `ReleaseOptions` + `ReleasePlan`
- `ReserveGroupOptions` + `ReserveGroupPlan`
- `AutoreserveOptions` + `AutoreservePlan`
- `MigrateOptions` + `MigratePlan`

All fallible operations return `trop::Result<T>`.

## Documentation

Generate API docs:

```bash
cargo doc --package trop --open
```

## Testing

```bash
cargo test --package trop
cargo test --workspace
```

## For CLI Users

If you want the command-line tool, see [`trop-cli`](../trop-cli/README.md).

## License

MIT
