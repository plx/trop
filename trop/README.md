# trop (Library Crate)

This is the core library crate for `trop`, a port reservation management tool. It provides the fundamental types and logic for managing ephemeral port allocations in a directory-aware, idempotent manner.

## Security upgrade from 0.1.0

Version 0.1.0 does not safely validate every environment-variable identifier
generated at shell export and dotenv output boundaries. Version 0.2.0 is
published with the fix; depend on it explicitly:

```toml
[dependencies]
trop = "0.2.0"
```

The fix changes `ShellType::format_export` to return `trop::Result<String>` so
invalid identifiers can fail closed. This is a source-breaking change from
0.1.0. Version 0.1.0 is yanked, but yanking does not update existing lockfiles;
consumers must update their dependency resolution explicitly. See
[GHSA-h2jc-jr86-m5vq](https://github.com/plx/trop/security/advisories/GHSA-h2jc-jr86-m5vq)
for affected usage and remediation.

## Overview

The `trop` library provides types for:

- **Port representation and validation**: `Port` and `PortRange` types with compile-time safety
- **Reservation management**: `Reservation` and `ReservationKey` for tracking port allocations
- **Error handling**: Comprehensive error types with clear messages
- **Logging**: Configurable logging infrastructure

This library is designed to be used both by the `trop-cli` binary and potentially by other tools that need port reservation functionality.

## Main Types

### `Port`

A validated network port number (1-65535). Port 0 is explicitly invalid.

```rust
use trop::Port;

// Valid port
let port = Port::try_from(8080)?;
assert_eq!(port.value(), 8080);
assert!(!port.is_privileged()); // < 1024 is privileged

// Invalid port (0)
assert!(Port::try_from(0).is_err());
```

### `PortRange`

An inclusive range of valid ports with iteration support.

```rust
use trop::{Port, PortRange};

let min = Port::try_from(5000)?;
let max = Port::try_from(5010)?;
let range = PortRange::new(min, max)?;

assert_eq!(range.len(), 11);
assert!(range.contains(Port::try_from(5005)?));

// Iterate over ports in range
for port in range {
    println!("Port: {}", port);
}
```

### `ReservationKey`

A unique identifier for a port reservation, combining a filesystem path with an optional tag.

```rust
use std::path::PathBuf;
use trop::ReservationKey;

// Untagged reservation
let key = ReservationKey::new(PathBuf::from("/path/to/project"), None)?;
println!("{}", key); // "/path/to/project"

// Tagged reservation (for multiple ports per directory)
let key = ReservationKey::new(
    PathBuf::from("/path/to/project"),
    Some("web".to_string())
)?;
println!("{}", key); // "/path/to/project:web"
```

### `Reservation`

Complete reservation metadata including port, timestamps, and optional project/task information.

```rust
use std::path::PathBuf;
use trop::{Port, Reservation, ReservationKey};

let key = ReservationKey::new(PathBuf::from("/project"), None)?;
let port = Port::try_from(8080)?;

let reservation = Reservation::builder(key, port)
    .project(Some("my-app".to_string()))
    .task(Some("feature-branch".to_string()))
    .sticky(false)
    .build()?;

println!("Reserved port: {}", reservation.port());
println!("Project: {:?}", reservation.project());
```

## Error Handling

The library uses a custom `Error` type with variants for different failure modes:

- `InvalidPort`: Port number validation failures
- `InvalidPath`: Path-related errors
- `Database`: SQLite database errors
- `Configuration`: Config file parsing errors
- `Validation`: Input validation failures
- More variants as needed...

All fallible operations return `Result<T, Error>` or use the type alias `trop::Result<T>`.

## Current Status

The core library includes the SQLite database layer, path handling, configuration loading, port allocation, reservation operations, cleanup, migration, output formatting, and test utilities used by the CLI.

`ReserveOptions` reconciles one exact `ReservationKey`. Compatible calls keep
the stored port; authorized metadata changes produce an update that preserves
the creation timestamp. `with_overwrite(true)` re-runs allocation while
allowing the key to reuse its own port, and `with_force(true)` additionally
enables both preferred-port availability bypasses and the path and metadata
permissions. A different key's unique port ownership is never bypassed.

Project and task requests use `MetadataIntent::Preserve`, `Set`, or `Clear`.
`ReserveOptions::new` and the existing `with_project(None)` /
`with_task(None)` builders select `Preserve`: an existing exact key keeps its
stored value, while a new reservation uses best-effort Git inference.
`with_clear_project()` and `with_clear_task()` request explicit removal and
remain protected by the matching sticky-field permission. Set and clear
updates keep the stored port unless overwrite or force also requests
reallocation.

Opening a writable schema-v1 database automatically migrates it to schema v2
in one durable transaction. Schema v2 uses strict SQLite tables and enforces
unique reservation keys, globally unique ports in `1..=65535`, and nonnegative
timestamps. The public optional-tag model is unchanged; the empty-string
sentinel used for an untagged row is internal.

Migration preflight reports every detected duplicate or invalid legacy
category and leaves v1 unchanged instead of choosing rows. A read-only v1
database returns `Error::MigrationRequired`; read-only v2 access is supported.
No persistent backup or reverse migration is provided. Stop users of the
database and copy the complete data directory before upgrading if a downgrade
restore point is required.

`Database::validate(&DatabaseConfig)` is the non-mutating validation entry
point. It opens the existing file read-only without initializing or migrating,
then verifies physical integrity, foreign keys, schema/version structure,
metadata, uniqueness, required indexes and constraints, and all reservation
values. Stored row decoding is shared by validation and ordinary reads and is
fully fallible: invalid scalar types or domain values return
`Error::DatabaseCorruption` with table/field/key context instead of panicking.
Diagnostics do not include project/task contents or raw blob bytes. Validation
does not attempt recovery; callers should copy the database before restoring a
known-good copy or recreating disposable reservations.

`DatabaseConfig::busy_timeout` defaults to five seconds. `Duration::ZERO`
disables waiting, while values above 2,147,483,647 milliseconds are rejected
before rusqlite configuration. Database operation boundaries classify only
SQLite Busy and Locked results as `Error::LockTimeout`, preserving the exact
duration and operation context. All other SQLite errors remain
`Error::Database`.

## Usage Example

```rust
use std::path::PathBuf;
use trop::{Port, PortRange, Reservation, ReservationKey};

fn main() -> trop::Result<()> {
    // Create a port
    let port = Port::try_from(8080)?;
    println!("Port: {} (privileged: {})", port, port.is_privileged());

    // Create a port range
    let min = Port::try_from(5000)?;
    let max = Port::try_from(5100)?;
    let range = PortRange::new(min, max)?;
    println!("Range contains {} ports", range.len());

    // Create a reservation key
    let key = ReservationKey::new(
        PathBuf::from("/my/project"),
        Some("api".to_string())
    )?;

    // Build a reservation
    let reservation = Reservation::builder(key, port)
        .project(Some("my-app".to_string()))
        .build()?;

    println!("Created reservation for port {}", reservation.port());

    Ok(())
}
```

## Documentation

Generate and view complete API documentation:

```bash
cargo doc --open
```

The documentation includes:
- Detailed descriptions of all public types
- Usage examples for key operations
- Error handling patterns
- Module organization

## Testing

Run the library's unit tests:

```bash
cargo test --lib
```

Run integration tests (requires full workspace):

```bash
cd ..
cargo test --all
```

## For CLI Users

If you're looking to use the `trop` command-line tool, see the [`trop-cli` crate](../trop-cli/README.md) instead. This library is primarily for programmatic use or for understanding the internals.

## Design Philosophy

The library follows these principles:

- **Type safety**: Use newtypes and the type system to prevent invalid states
- **Validation at construction**: Types validate their inputs when created
- **Immutability by default**: Most types are immutable after construction
- **Builder patterns**: Complex types use builders for flexible construction
- **Clear error messages**: Errors include context about what went wrong
- **Zero-cost abstractions**: Newtypes compile down to their underlying types

## Dependencies

Key dependencies:
- `serde`: Serialization support
- `thiserror`: Error type derivation
- `chrono`: Timestamp handling
- `rusqlite`: SQLite database
- `log`: Logging facade

## License

`trop` is dual-licensed under either:

- [Apache License, Version 2.0](LICENSE-APACHE), or
- [MIT License](LICENSE-MIT),

at your option.

## Contributing

This is part of the larger `trop` project. See the [root README](../README.md) for contribution guidelines and project status.
