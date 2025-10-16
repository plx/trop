# Phase 6: Port Allocation & Occupancy - Implementation Plan

## Overview

This phase implements the port allocation algorithm with occupancy checking capabilities. The system will automatically find available ports while respecting reservations, exclusions, and actual port occupancy on the system.

## Context & Current State

### Completed Components
- **Phase 1-3**: Core types (Port, PortRange), database layer, path handling
- **Phase 4**: Basic reservation operations with manual port specification
- **Phase 5**: Configuration system with exclusion lists and port ranges

### Current Limitations
- Ports must be manually specified when creating reservations
- No checking if a port is actually in use on the system
- No automatic port allocation algorithm
- No support for group allocations with offsets

## Architectural Decisions

### Port Occupancy Checking Library

After researching available options, we'll use **`port-selector`** as the primary occupancy checking library with a fallback strategy:

**Rationale:**
- Provides cross-platform support (Windows, Linux, macOS)
- Offers granular TCP/UDP checking
- Simple API for checking port availability
- Active maintenance and good documentation

**Fallback Strategy:**
If `port-selector` doesn't meet all needs, implement a two-tier approach:
1. Primary: Use `port-selector` for basic occupancy checks
2. Fallback: Attempt to bind to the port as a last resort (with immediate cleanup)

### Allocation Algorithm Design

The allocation will use a forward-scanning algorithm with the following characteristics:
- Deterministic: Same inputs produce same outputs
- Efficient: Skip occupied/excluded ports quickly
- Flexible: Support preferred ports and group patterns
- Recoverable: Automatic cleanup and retry on exhaustion

## Implementation Breakdown

### 1. Port Occupancy Checker Module

**Location:** `trop/src/port/occupancy.rs`

**Components:**

```rust
/// Trait for checking port occupancy
pub trait PortOccupancyChecker: Send + Sync {
    fn is_occupied(&self, port: Port, config: &OccupancyCheckConfig) -> Result<bool>;
    fn find_occupied_ports(&self, range: &PortRange, config: &OccupancyCheckConfig) -> Result<Vec<Port>>;
}

/// Configuration for occupancy checks
pub struct OccupancyCheckConfig {
    pub skip_tcp: bool,
    pub skip_udp: bool,
    pub skip_ipv4: bool,
    pub skip_ipv6: bool,
    pub check_all_interfaces: bool,
}

/// Production implementation using port-selector
pub struct SystemOccupancyChecker;

/// Mock implementation for testing
pub struct MockOccupancyChecker {
    occupied_ports: HashSet<Port>,
}
```

**Key Methods:**
- `is_occupied()`: Check if a specific port is in use
- `find_occupied_ports()`: Batch check for a range (optimization)
- Configuration respects the OccupancyConfig from Phase 5

**Integration Points:**
- Integrate with existing `OccupancyConfig` from `config/schema.rs`
- Use existing `Port` and `PortRange` types

### 2. Port Exclusion Manager

**Location:** `trop/src/port/exclusions.rs`

**Components:**

```rust
/// Manages excluded port lists
pub struct ExclusionManager {
    exclusions: Vec<PortExclusion>,
}

impl ExclusionManager {
    pub fn from_config(exclusions: &[PortExclusion]) -> Self;
    pub fn is_excluded(&self, port: Port) -> bool;
    pub fn excluded_in_range(&self, range: &PortRange) -> Vec<Port>;
    pub fn add_exclusion(&mut self, exclusion: PortExclusion);
    pub fn compact(&mut self);  // Merge overlapping ranges
}
```

**Key Features:**
- Efficient checking using sorted ranges
- Support for single ports and ranges (from Phase 5 config)
- Compaction to optimize exclusion lists

### 3. Port Allocator Core

**Location:** `trop/src/port/allocator.rs`

**Components:**

```rust
/// Main port allocator
pub struct PortAllocator<C: PortOccupancyChecker = SystemOccupancyChecker> {
    checker: C,
    exclusions: ExclusionManager,
    range: PortRange,
}

/// Options for allocation
pub struct AllocationOptions {
    pub preferred: Option<Port>,
    pub ignore_occupied: bool,
    pub ignore_exclusions: bool,
}

/// Result of allocation attempt
pub enum AllocationResult {
    Allocated(Port),
    PreferredUnavailable { port: Port, reason: String },
    Exhausted { tried_cleanup: bool },
}

impl<C: PortOccupancyChecker> PortAllocator<C> {
    pub fn allocate_single(&self, db: &Database, key: &ReservationKey, options: AllocationOptions) -> Result<AllocationResult>;
    pub fn allocate_group(&self, db: &Database, group: &GroupAllocationRequest) -> Result<GroupAllocationResult>;

    // Internal methods
    fn find_next_available(&self, start: Port, db: &Database) -> Result<Option<Port>>;
    fn is_port_available(&self, port: Port, db: &Database) -> Result<bool>;
    fn trigger_cleanup(&self, db: &mut Database, config: &CleanupConfig) -> Result<usize>;
}
```

**Allocation Algorithm (Single Port):**
1. If preferred port specified and available → use it
2. Otherwise, scan forward from range minimum
3. For each candidate port:
   - Check not in reservations (database)
   - Check not in exclusions (config)
   - Check not occupied (system)
4. If exhausted and cleanup enabled:
   - Run prune (remove reservations for non-existent paths)
   - Run expire (remove old unused reservations)
   - Retry from step 2
5. Return exhaustion error if still no ports

### 4. Group Allocation Support

**Location:** `trop/src/port/group.rs`

**Components:**

```rust
/// Request for group allocation
pub struct GroupAllocationRequest {
    pub base_path: PathBuf,
    pub project: Option<String>,
    pub task: Option<String>,
    pub services: Vec<ServiceAllocationRequest>,
}

/// Individual service in a group
pub struct ServiceAllocationRequest {
    pub tag: String,
    pub offset: u16,
    pub preferred: Option<Port>,
}

/// Result of group allocation
pub struct GroupAllocationResult {
    pub allocations: HashMap<String, Port>,
    pub base_port: Option<Port>,
}

impl PortAllocator {
    fn allocate_group(&self, db: &Database, request: &GroupAllocationRequest) -> Result<GroupAllocationResult>;
    fn find_pattern_match(&self, pattern: &[u16], db: &Database) -> Result<Option<Port>>;
}
```

**Group Allocation Algorithm:**
1. Extract pattern from offsets (e.g., [0, 1, 100])
2. Scan forward looking for base ports where ALL offsets are available
3. Apply same availability checks as single allocation
4. Transaction: allocate all or none
5. Support preferred ports for individual services

### 5. Integration with Reserve Operation

**Location:** Update `trop/src/operations/reserve.rs`

**Changes to `ReservePlan::build_plan()`:**

```rust
impl ReservePlan {
    pub fn build_plan(&self, db: &Database) -> Result<OperationPlan> {
        // ... existing validation ...

        // Step 3: Port allocation (updated)
        let port = if let Some(port) = self.options.port {
            // Manual port specified - validate it
            self.validate_manual_port(port, db)?;
            port
        } else {
            // Automatic allocation
            let allocator = self.create_allocator()?;
            let allocation_options = AllocationOptions {
                preferred: self.options.preferred_port,
                ignore_occupied: self.options.ignore_occupied,
                ignore_exclusions: self.options.ignore_exclusions,
            };

            match allocator.allocate_single(db, &self.options.key, allocation_options)? {
                AllocationResult::Allocated(port) => port,
                AllocationResult::PreferredUnavailable { port, reason } => {
                    if !self.options.ignore_occupied {
                        return Err(Error::PortUnavailable { reason });
                    }
                    port
                }
                AllocationResult::Exhausted { .. } => {
                    return Err(Error::PortExhausted {
                        range: self.config.port_range(),
                        tried_cleanup: true,
                    });
                }
            }
        };

        // ... continue with reservation creation ...
    }
}
```

### 6. Cleanup Integration

**Location:** `trop/src/operations/cleanup.rs` (new file)

**Components:**

```rust
/// Cleanup operations
pub struct CleanupOperations;

impl CleanupOperations {
    /// Remove reservations for non-existent paths
    pub fn prune(db: &mut Database, dry_run: bool) -> Result<PruneResult>;

    /// Remove expired reservations
    pub fn expire(db: &mut Database, config: &CleanupConfig, dry_run: bool) -> Result<ExpireResult>;

    /// Combined cleanup
    pub fn autoclean(db: &mut Database, config: &CleanupConfig, dry_run: bool) -> Result<AutocleanResult>;
}
```

**Integration with Allocator:**
- Allocator triggers cleanup when exhausted (if not disabled)
- Cleanup is transactional - all or nothing
- Returns count of freed reservations

## Testing Strategy

### 1. Unit Tests

**Occupancy Checker Tests** (`port/occupancy.rs`):
- Mock checker returns predictable results
- System checker handles permission errors gracefully
- Configuration flags properly control checks

**Exclusion Manager Tests** (`port/exclusions.rs`):
- Single port exclusions
- Range exclusions with overlaps
- Compaction produces minimal representation
- Edge cases (port 1, port 65535)

**Allocator Tests** (`port/allocator.rs`):
- Forward scanning from minimum
- Preferred port selection
- Exhaustion handling
- Cleanup retry logic

### 2. Integration Tests

**Single Allocation Scenarios**:
- Allocate with all ports free
- Allocate with some reserved
- Allocate with exclusions
- Allocate with occupied ports
- Exhaustion and cleanup

**Group Allocation Scenarios**:
- Simple group (consecutive ports)
- Complex group (gaps in offsets)
- Partial conflicts
- Transaction rollback on failure

### 3. Property-Based Tests

```rust
proptest! {
    // Allocation is deterministic
    fn prop_allocation_deterministic(seed: u64) {
        // Same state + same request = same result
    }

    // Allocated ports are never excluded
    fn prop_never_allocates_excluded(exclusions: Vec<PortExclusion>) {
        // Verify allocated port ∉ exclusions
    }

    // Group allocation is atomic
    fn prop_group_allocation_atomic(group: GroupRequest) {
        // Either all succeed or none
    }
}
```

### 4. Mock Testing Strategy

Create `MockOccupancyChecker` that:
- Can be configured with specific occupied ports
- Records which ports were checked
- Supports deterministic testing
- Can simulate system errors

## Migration & Compatibility

### Database Changes
No schema changes required - Phase 6 adds functionality without modifying storage.

### Configuration Compatibility
- Existing configs continue to work
- New allocation uses existing exclusions and ranges
- Occupancy check config from Phase 5 is respected

### API Compatibility
- `ReserveOptions` gains optional fields for allocation
- Existing manual port specification still works
- New allocation is opt-in via omitting port

## Error Handling

### New Error Types

```rust
pub enum Error {
    // ... existing ...

    PortExhausted {
        range: PortRange,
        tried_cleanup: bool,
    },

    OccupancyCheckFailed {
        port: Port,
        source: Box<dyn Error>,
    },

    PreferredPortUnavailable {
        port: Port,
        reason: PortUnavailableReason,
    },

    GroupAllocationFailed {
        partial: Vec<String>,  // Tags that were attempted
        reason: String,
    },
}

pub enum PortUnavailableReason {
    Reserved,
    Excluded,
    Occupied,
}
```

### Error Recovery
- Occupancy check failures are treated as "port occupied"
- System errors during allocation trigger cleanup attempt
- Partial group allocations are rolled back

## Dependencies

### New Crate Dependencies

```toml
[dependencies]
port-selector = "0.1"  # Port occupancy checking

[dev-dependencies]
mockall = "0.11"  # For mock testing
```

### Internal Dependencies
- Builds on `Port`, `PortRange` from Phase 1
- Uses `Database` operations from Phase 2
- Integrates with `Config` types from Phase 5
- Extends `ReserveOptions` from Phase 4

## Implementation Order

1. **Port Occupancy Module** (2-3 hours)
   - Trait definition
   - SystemOccupancyChecker with port-selector
   - MockOccupancyChecker for testing

2. **Exclusion Manager** (2 hours)
   - Basic implementation
   - Compaction logic
   - Tests

3. **Basic Allocator** (3-4 hours)
   - Single port allocation
   - Forward scanning
   - Availability checking

4. **Cleanup Integration** (2 hours)
   - Prune operation
   - Expire operation
   - Integration with allocator

5. **Group Allocation** (3-4 hours)
   - Pattern matching
   - Transactional allocation
   - Rollback handling

6. **Reserve Operation Updates** (2 hours)
   - Integrate allocator
   - Update options handling
   - Backward compatibility

7. **Testing** (3-4 hours)
   - Unit tests for each module
   - Integration test scenarios
   - Property-based tests

## Success Criteria

- [ ] Can automatically allocate available ports
- [ ] Respects exclusion lists from configuration
- [ ] Checks actual system port occupancy
- [ ] Supports preferred port hints
- [ ] Implements cleanup-and-retry on exhaustion
- [ ] Group allocation is transactional
- [ ] Mock checker enables deterministic testing
- [ ] All tests pass including property tests
- [ ] Backward compatible with Phase 4 manual specification

## Open Questions & Decisions

### Resolved Decisions

1. **Library Choice**: Use `port-selector` for occupancy checking
2. **Algorithm**: Forward scanning with deterministic behavior
3. **Cleanup**: Automatic with opt-out via config
4. **Mocking**: Use trait-based design for testability

### Deferred Decisions

1. **Performance Optimization**: May add caching of occupied ports in future
2. **Parallel Checking**: Could batch check ports for performance
3. **Custom Allocators**: Might support pluggable allocation strategies

## Notes for Implementation

- Keep allocator stateless - all state in Database/Config
- Ensure thread-safety for concurrent allocations
- Log allocation attempts for debugging
- Consider adding metrics/stats in future phase
- Document allocation behavior clearly for users