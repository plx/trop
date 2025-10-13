# trop (Library Crate)

This is the core library crate for `trop`, a port reservation management tool. It provides the fundamental types and logic for managing ephemeral port allocations in a directory-aware, idempotent manner.

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
- `Database`: SQLite database errors (Phase 2+)
- `Configuration`: Config file parsing errors (Phase 2+)
- `Validation`: Input validation failures
- More variants as needed...

All fallible operations return `Result<T, Error>` or use the type alias `trop::Result<T>`.

## Current Status

**Phase 1 Complete**: Core types and error handling implemented with comprehensive unit tests.

**Coming in Phase 2**: Database layer, configuration parsing, and port allocation algorithms.

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
- `rusqlite`: SQLite database (Phase 2+)
- `log`: Logging facade

## License

MIT

## Contributing

This is part of the larger `trop` project. See the [root README](../README.md) for contribution guidelines and project status.
