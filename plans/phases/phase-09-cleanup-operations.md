# Phase 9: Cleanup Operations - Implementation Plan (Revised)

## Executive Summary

This revised plan addresses the implementation of cleanup commands for the trop CLI tool, incorporating detailed feedback from plan review. The plan resolves the critical database mutability issue, provides complete command implementations, and specifies all integration points. These operations maintain database health by removing stale reservations—either for non-existent directories (prune) or based on age (expire).

## Current State Analysis

### Existing Library Support

The `trop` library already provides comprehensive cleanup functionality via `trop/src/operations/cleanup.rs`:

- **`CleanupOperations::prune`**: Removes reservations for non-existent paths
- **`CleanupOperations::expire`**: Removes reservations older than threshold
- **`CleanupOperations::autoclean`**: Combines prune and expire
- All operations support dry-run mode
- Results include detailed information about removed reservations

### Configuration Support

The configuration system already supports cleanup settings:

```rust
pub struct CleanupConfig {
    pub expire_after_days: Option<u32>,
}

pub struct Config {
    // ...
    pub cleanup: Option<CleanupConfig>,
    pub disable_autoprune: Option<bool>,
    pub disable_autoexpire: Option<bool>,
    // ...
}
```

### Critical Architecture Decision: Database Mutability

The current `PortAllocator::allocate_single` method signature:
```rust
pub fn allocate_single(
    &self,
    db: &Database,  // Immutable reference
    options: &AllocationOptions,
    occupancy_config: &OccupancyCheckConfig,
) -> Result<AllocationResult>
```

The cleanup operations require:
```rust
pub fn prune(db: &mut Database, dry_run: bool) -> Result<PruneResult>
```

#### Solution: Two-Phase Allocation with Cleanup Hint

Instead of changing the allocator's database reference to mutable (a breaking change), we'll implement a two-phase approach:

1. **Phase 1**: Allocator attempts allocation with immutable database reference
2. **Phase 2**: If exhausted, the allocator returns a hint suggesting cleanup
3. **Caller** (ReservePlan) performs cleanup with mutable reference and retries

This maintains backward compatibility while enabling auto-cleanup.

## Implementation Plan

### Part 1: Enhanced Allocation Result

Modify the `AllocationResult` enum to provide cleanup hints:

```rust
// In trop/src/port/allocator.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationResult {
    /// Successfully allocated a port.
    Allocated(Port),

    /// The preferred port was unavailable.
    PreferredUnavailable {
        port: Port,
        reason: PortUnavailableReason,
    },

    /// No ports available in the range.
    Exhausted {
        /// Suggests whether cleanup might help
        cleanup_suggested: bool,
        /// Whether cleanup was already attempted
        tried_cleanup: bool,
    },
}
```

The allocator will set `cleanup_suggested: true` when it detects reservations that might be stale (e.g., for paths that no longer exist or old reservations).

### Part 2: CLI Command Structures

#### 2.1 Prune Command (`trop-cli/src/commands/prune.rs`)

```rust
use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::operations::CleanupOperations;

/// Remove reservations for non-existent directories.
#[derive(Args)]
pub struct PruneCommand {
    /// Perform a dry run (show what would be removed without removing)
    #[arg(long)]
    pub dry_run: bool,
}

impl PruneCommand {
    /// Execute the prune command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Load configuration for database location
        let config = load_configuration(global)?;

        // Handle dry-run output
        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Scanning for reservations with non-existent paths...");
        }

        // Open database with write access
        let mut db = open_database(global, &config)?;

        // Perform pruning operation
        let result = CleanupOperations::prune(&mut db, self.dry_run)
            .map_err(CliError::from)?;

        // Format and output results
        if global.quiet {
            // Quiet mode: just the count to stdout
            if result.removed_count > 0 {
                println!("{}", result.removed_count);
            }
        } else if global.verbose {
            // Verbose mode: detailed output to stderr
            if self.dry_run {
                eprintln!("[DRY RUN] Would remove {} reservation(s):", result.removed_count);
            } else {
                eprintln!("Removed {} reservation(s):", result.removed_count);
            }

            for reservation in &result.removed_reservations {
                eprintln!("  - Port {}: {} (tag: {:?}, project: {:?})",
                    reservation.port().value(),
                    reservation.key().path.display(),
                    reservation.key().tag,
                    reservation.project()
                );
            }
        } else {
            // Normal mode: summary to stderr
            if self.dry_run {
                eprintln!("[DRY RUN] Would remove {} reservation(s) for non-existent paths",
                    result.removed_count);
            } else {
                eprintln!("Removed {} reservation(s) for non-existent paths",
                    result.removed_count);
            }
        }

        Ok(())
    }
}
```

#### 2.2 Expire Command (`trop-cli/src/commands/expire.rs`)

```rust
use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::config::CleanupConfig;
use trop::operations::CleanupOperations;

/// Remove reservations based on age.
#[derive(Args)]
pub struct ExpireCommand {
    /// Remove reservations unused for N days
    #[arg(long, value_name = "DAYS")]
    pub days: Option<u32>,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl ExpireCommand {
    /// Execute the expire command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Load configuration
        let config = load_configuration(global)?;

        // Determine expiration threshold
        let expire_days = self.days.or_else(|| {
            config.cleanup.as_ref()
                .and_then(|c| c.expire_after_days)
        });

        // Validate we have a threshold
        let Some(days) = expire_days else {
            return Err(CliError::InvalidArguments(
                "No expiration threshold specified. Use --days or configure expire_after_days".into()
            ));
        };

        // Create cleanup config
        let cleanup_config = CleanupConfig {
            expire_after_days: Some(days),
        };

        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Scanning for reservations unused for {} days...", days);
        }

        // Open database
        let mut db = open_database(global, &config)?;

        // Perform expiration
        let result = CleanupOperations::expire(&mut db, &cleanup_config, self.dry_run)
            .map_err(CliError::from)?;

        // Format output
        if global.quiet {
            if result.removed_count > 0 {
                println!("{}", result.removed_count);
            }
        } else if global.verbose {
            if self.dry_run {
                eprintln!("[DRY RUN] Would expire {} reservation(s) older than {} days:",
                    result.removed_count, days);
            } else {
                eprintln!("Expired {} reservation(s) older than {} days:",
                    result.removed_count, days);
            }

            for reservation in &result.removed_reservations {
                let age_days = reservation.last_used_at()
                    .elapsed()
                    .unwrap_or_default()
                    .as_secs() / 86400;
                eprintln!("  - Port {}: {} ({} days old, project: {:?})",
                    reservation.port().value(),
                    reservation.key().path.display(),
                    age_days,
                    reservation.project()
                );
            }
        } else {
            if self.dry_run {
                eprintln!("[DRY RUN] Would expire {} reservation(s) older than {} days",
                    result.removed_count, days);
            } else {
                eprintln!("Expired {} reservation(s) older than {} days",
                    result.removed_count, days);
            }
        }

        Ok(())
    }
}
```

#### 2.3 Autoclean Command (`trop-cli/src/commands/autoclean.rs`)

```rust
use crate::error::CliError;
use crate::utils::{load_configuration, open_database, GlobalOptions};
use clap::Args;
use trop::config::CleanupConfig;
use trop::operations::CleanupOperations;

/// Combined cleanup (prune + expire).
#[derive(Args)]
pub struct AutocleanCommand {
    /// Override expiration threshold in days
    #[arg(long, value_name = "DAYS")]
    pub days: Option<u32>,

    /// Perform a dry run
    #[arg(long)]
    pub dry_run: bool,
}

impl AutocleanCommand {
    /// Execute the autoclean command.
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // Load configuration
        let config = load_configuration(global)?;

        // Build cleanup config with overrides
        let cleanup_config = if let Some(days) = self.days {
            CleanupConfig {
                expire_after_days: Some(days),
            }
        } else if let Some(ref cleanup) = config.cleanup {
            cleanup.clone()
        } else {
            CleanupConfig {
                expire_after_days: None,
            }
        };

        if self.dry_run && !global.quiet {
            eprintln!("[DRY RUN] Performing combined cleanup...");
        }

        // Open database
        let mut db = open_database(global, &config)?;

        // Perform combined cleanup
        let result = CleanupOperations::autoclean(&mut db, &cleanup_config, self.dry_run)
            .map_err(CliError::from)?;

        // Format output
        if global.quiet {
            if result.total_removed > 0 {
                println!("{}", result.total_removed);
            }
        } else if global.verbose {
            let prefix = if self.dry_run { "[DRY RUN] Would remove" } else { "Removed" };

            eprintln!("{} {} total reservation(s):", prefix, result.total_removed);
            eprintln!("  Pruned: {} (non-existent paths)", result.pruned_count);

            if cleanup_config.expire_after_days.is_some() {
                eprintln!("  Expired: {} (old reservations)", result.expired_count);
            }

            if !result.pruned_reservations.is_empty() {
                eprintln!("\nPruned reservations:");
                for res in &result.pruned_reservations {
                    eprintln!("  - Port {}: {}",
                        res.port().value(),
                        res.key().path.display()
                    );
                }
            }

            if !result.expired_reservations.is_empty() {
                eprintln!("\nExpired reservations:");
                for res in &result.expired_reservations {
                    let age_days = res.last_used_at()
                        .elapsed()
                        .unwrap_or_default()
                        .as_secs() / 86400;
                    eprintln!("  - Port {}: {} ({} days old)",
                        res.port().value(),
                        res.key().path.display(),
                        age_days
                    );
                }
            }
        } else {
            let prefix = if self.dry_run { "[DRY RUN] Would remove" } else { "Removed" };
            eprintln!("{} {} total reservation(s) (pruned: {}, expired: {})",
                prefix,
                result.total_removed,
                result.pruned_count,
                result.expired_count
            );
        }

        Ok(())
    }
}
```

### Part 3: CLI Integration

#### 3.1 Update Commands Module (`trop-cli/src/commands/mod.rs`)

```rust
//! CLI command implementations.
//!
//! This module contains the implementations of all CLI commands:
//! - `reserve`: Reserve a port for a directory
//! - `release`: Release a port reservation
//! - `list`: List active reservations
//! - `reserve_group`: Reserve ports for a group of services
//! - `autoreserve`: Automatically discover and reserve ports
//! - `prune`: Remove reservations for non-existent paths
//! - `expire`: Remove old reservations
//! - `autoclean`: Combined cleanup operations

pub mod autoreserve;
pub mod list;
pub mod release;
pub mod reserve;
pub mod reserve_group;
pub mod prune;
pub mod expire;
pub mod autoclean;

pub use autoreserve::AutoreserveCommand;
pub use list::ListCommand;
pub use release::ReleaseCommand;
pub use reserve::ReserveCommand;
pub use reserve_group::ReserveGroupCommand;
pub use prune::PruneCommand;
pub use expire::ExpireCommand;
pub use autoclean::AutocleanCommand;
```

#### 3.2 Update CLI Enum (`trop-cli/src/cli.rs`)

```rust
use crate::commands::{
    AutoreserveCommand, ListCommand, ReleaseCommand, ReserveCommand,
    ReserveGroupCommand, PruneCommand, ExpireCommand, AutocleanCommand,
};

/// Available CLI commands.
#[derive(Subcommand)]
pub enum Command {
    /// Reserve a port for a directory
    Reserve(ReserveCommand),

    /// Release a port reservation
    Release(ReleaseCommand),

    /// List active reservations
    List(ListCommand),

    /// Reserve ports for a group of services defined in a config file
    ReserveGroup(ReserveGroupCommand),

    /// Automatically discover and reserve ports from project config
    Autoreserve(AutoreserveCommand),

    /// Remove reservations for non-existent directories
    Prune(PruneCommand),

    /// Remove reservations based on age
    Expire(ExpireCommand),

    /// Combined cleanup (prune + expire)
    Autoclean(AutocleanCommand),
}
```

#### 3.3 Update Main Handler (`trop-cli/src/main.rs`)

```rust
// Execute the command
let result = match cli.command {
    cli::Command::Reserve(cmd) => cmd.execute(&global),
    cli::Command::Release(cmd) => cmd.execute(&global),
    cli::Command::List(cmd) => cmd.execute(&global),
    cli::Command::ReserveGroup(cmd) => cmd.execute(&global),
    cli::Command::Autoreserve(cmd) => cmd.execute(&global),
    cli::Command::Prune(cmd) => cmd.execute(&global),
    cli::Command::Expire(cmd) => cmd.execute(&global),
    cli::Command::Autoclean(cmd) => cmd.execute(&global),
};
```

### Part 4: Auto-cleanup During Allocation

#### 4.1 Modified Reserve Operation (`trop/src/operations/reserve.rs`)

```rust
impl<'a> ReservePlan<'a> {
    /// Builds an operation plan for this reserve request.
    pub fn build_plan(&self, db: &mut Database) -> Result<OperationPlan> {
        let mut plan = OperationPlan::new(format!("Reserve port for {}", self.options.key));

        // Step 1: Validate path relationship
        if !self.options.force && !self.options.allow_unrelated_path {
            db.validate_path_relationship(&self.options.key.path, false)?;
        }

        // Step 2: Check for existing reservation
        if let Some(existing) = db.get_reservation(&self.options.key)? {
            self.validate_sticky_fields(&existing)?;
            plan = plan.add_action(PlanAction::UpdateLastUsed(self.options.key.clone()));
            return Ok(plan);
        }

        // Step 3: Initial allocation attempt
        let port = {
            let allocator = allocator_from_config(self.config)?;
            let allocation_options = AllocationOptions {
                preferred: self.options.port.or(self.options.preferred_port),
                ignore_occupied: self.options.ignore_occupied,
                ignore_exclusions: self.options.ignore_exclusions,
            };
            let occupancy_config = self.occupancy_config();

            match allocator.allocate_single(db, &allocation_options, &occupancy_config)? {
                AllocationResult::Allocated(port) => port,

                AllocationResult::PreferredUnavailable { .. } => {
                    // Fall back to scanning
                    let fallback_options = AllocationOptions {
                        preferred: None,
                        ignore_occupied: self.options.ignore_occupied,
                        ignore_exclusions: self.options.ignore_exclusions,
                    };

                    match allocator.allocate_single(db, &fallback_options, &occupancy_config)? {
                        AllocationResult::Allocated(port) => port,
                        AllocationResult::Exhausted { cleanup_suggested, .. } => {
                            // Attempt cleanup if suggested and enabled
                            if cleanup_suggested && self.should_attempt_cleanup() {
                                if let Some(port) = self.attempt_cleanup_and_retry(
                                    db,
                                    &allocator,
                                    &fallback_options,
                                    &occupancy_config
                                )? {
                                    port
                                } else {
                                    return Err(Error::PortExhausted {
                                        range: *allocator.range(),
                                        tried_cleanup: true,
                                    });
                                }
                            } else {
                                return Err(Error::PortExhausted {
                                    range: *allocator.range(),
                                    tried_cleanup: false,
                                });
                            }
                        }
                        AllocationResult::PreferredUnavailable { .. } => unreachable!(),
                    }
                }

                AllocationResult::Exhausted { cleanup_suggested, .. } => {
                    // Attempt cleanup if suggested and enabled
                    if cleanup_suggested && self.should_attempt_cleanup() {
                        if let Some(port) = self.attempt_cleanup_and_retry(
                            db,
                            &allocator,
                            &allocation_options,
                            &occupancy_config
                        )? {
                            port
                        } else {
                            return Err(Error::PortExhausted {
                                range: *allocator.range(),
                                tried_cleanup: true,
                            });
                        }
                    } else {
                        return Err(Error::PortExhausted {
                            range: *allocator.range(),
                            tried_cleanup: false,
                        });
                    }
                }
            }
        };

        // Step 4: Create the new reservation
        let reservation = Reservation::builder(self.options.key.clone(), port)
            .project(self.options.project.clone())
            .task(self.options.task.clone())
            .build()?;

        plan = plan.add_action(PlanAction::CreateReservation(reservation));
        Ok(plan)
    }

    /// Determines if auto-cleanup should be attempted.
    fn should_attempt_cleanup(&self) -> bool {
        // Check if cleanup is disabled via config or CLI flags
        let autoprune_disabled = self.config.disable_autoprune.unwrap_or(false)
            || self.options.disable_autoprune;
        let autoexpire_disabled = self.config.disable_autoexpire.unwrap_or(false)
            || self.options.disable_autoexpire;

        // Cleanup is worthwhile if at least one operation is enabled
        !autoprune_disabled || !autoexpire_disabled
    }

    /// Attempts cleanup and retries allocation.
    fn attempt_cleanup_and_retry(
        &self,
        db: &mut Database,
        allocator: &PortAllocator,
        options: &AllocationOptions,
        occupancy_config: &OccupancyCheckConfig,
    ) -> Result<Option<Port>> {
        use crate::operations::CleanupOperations;

        let mut freed_any = false;

        // Try pruning if enabled
        if !self.config.disable_autoprune.unwrap_or(false)
            && !self.options.disable_autoprune {
            let prune_result = CleanupOperations::prune(db, false)?;
            freed_any |= prune_result.removed_count > 0;

            if prune_result.removed_count > 0 {
                log::info!("Auto-pruned {} reservation(s) for non-existent paths",
                    prune_result.removed_count);
            }
        }

        // Try expiring if enabled and configured
        if !self.config.disable_autoexpire.unwrap_or(false)
            && !self.options.disable_autoexpire {
            if let Some(ref cleanup_config) = self.config.cleanup {
                if cleanup_config.expire_after_days.is_some() {
                    let expire_result = CleanupOperations::expire(db, cleanup_config, false)?;
                    freed_any |= expire_result.removed_count > 0;

                    if expire_result.removed_count > 0 {
                        log::info!("Auto-expired {} old reservation(s)",
                            expire_result.removed_count);
                    }
                }
            }
        }

        // Retry allocation if we freed anything
        if freed_any {
            match allocator.allocate_single(db, options, occupancy_config)? {
                AllocationResult::Allocated(port) => Ok(Some(port)),
                _ => Ok(None),
            }
        } else {
            Ok(None)
        }
    }
}
```

#### 4.2 Update ReserveOptions to Include Cleanup Flags

```rust
// In trop/src/operations/reserve.rs
#[derive(Debug, Clone)]
pub struct ReserveOptions {
    // ... existing fields ...

    /// Disable automatic pruning during allocation.
    pub disable_autoprune: bool,

    /// Disable automatic expiration during allocation.
    pub disable_autoexpire: bool,
}

impl ReserveOptions {
    // ... existing methods ...

    /// Sets the disable_autoprune flag.
    #[must_use]
    pub const fn with_disable_autoprune(mut self, disable: bool) -> Self {
        self.disable_autoprune = disable;
        self
    }

    /// Sets the disable_autoexpire flag.
    #[must_use]
    pub const fn with_disable_autoexpire(mut self, disable: bool) -> Self {
        self.disable_autoexpire = disable;
        self
    }
}
```

#### 4.3 Update Reserve Command to Pass Cleanup Flags

```rust
// In trop-cli/src/commands/reserve.rs
impl ReserveCommand {
    pub fn execute(self, global: &GlobalOptions) -> Result<(), CliError> {
        // ... existing code ...

        // Build library ReserveOptions including cleanup flags
        let options = ReserveOptions::new(key, port)
            .with_project(self.project)
            .with_task(self.task)
            .with_ignore_occupied(self.ignore_occupied || self.skip_occupancy_check)
            .with_ignore_exclusions(self.ignore_exclusions)
            .with_force(self.force)
            .with_allow_unrelated_path(self.allow_unrelated_path)
            .with_allow_project_change(self.allow_project_change || self.allow_change)
            .with_allow_task_change(self.allow_task_change || self.allow_change)
            .with_disable_autoprune(self.disable_autoprune || self.disable_autoclean)
            .with_disable_autoexpire(self.disable_autoexpire || self.disable_autoclean);

        // ... rest of implementation ...
    }
}
```

### Part 5: Configuration Flow

#### 5.1 Configuration Precedence

The configuration precedence for cleanup operations is:

1. **CLI Flags** (highest priority)
   - `--days` for expire/autoclean commands
   - `--disable-autoprune`, `--disable-autoexpire` for reserve command
2. **Environment Variables**
   - `TROP_DISABLE_AUTOPRUNE`
   - `TROP_DISABLE_AUTOEXPIRE`
3. **Configuration File** (lowest priority)
   - `cleanup.expire_after_days`
   - `disable_autoprune`
   - `disable_autoexpire`

#### 5.2 Configuration Validation

```rust
// In configuration loading (already exists)
impl CleanupConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(days) = self.expire_after_days {
            if days == 0 {
                return Err(Error::Validation {
                    field: "expire_after_days".into(),
                    message: "Expiration threshold must be at least 1 day".into(),
                });
            }
            if days > 365 {
                log::warn!("Large expiration threshold: {} days", days);
            }
        }
        Ok(())
    }
}
```

### Part 6: Testing Strategy

#### 6.1 Unit Tests for CLI Commands

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{create_test_global_options, create_temp_database};

    #[test]
    fn test_prune_command_dry_run() {
        let global = create_test_global_options();
        let cmd = PruneCommand { dry_run: true };

        // Should succeed without actually removing anything
        let result = cmd.execute(&global);
        assert!(result.is_ok());
    }

    #[test]
    fn test_expire_command_requires_threshold() {
        let global = create_test_global_options();
        let cmd = ExpireCommand {
            days: None,
            dry_run: false
        };

        // Should fail without a threshold
        let result = cmd.execute(&global);
        assert!(matches!(result, Err(CliError::InvalidArguments(_))));
    }

    #[test]
    fn test_autoclean_command_with_override() {
        let global = create_test_global_options();
        let cmd = AutocleanCommand {
            days: Some(7),
            dry_run: true,
        };

        let result = cmd.execute(&global);
        assert!(result.is_ok());
    }
}
```

#### 6.2 Integration Tests for Auto-cleanup

```rust
#[test]
fn test_auto_cleanup_on_exhaustion() {
    let mut db = create_test_database();
    let config = create_config_with_small_range(); // Only 2 ports

    // Fill the range with stale reservations
    create_stale_reservation(&mut db, 5000, "/nonexistent1");
    create_stale_reservation(&mut db, 5001, "/nonexistent2");

    // Attempt to reserve - should trigger cleanup and succeed
    let options = ReserveOptions::new(key, None);
    let plan = ReservePlan::new(options, &config)
        .build_plan(&mut db)
        .unwrap();

    // Should have created a reservation after cleanup
    assert!(matches!(plan.actions[0], PlanAction::CreateReservation(_)));
}

#[test]
fn test_auto_cleanup_disabled() {
    let mut db = create_test_database();
    let mut config = create_config_with_small_range();
    config.disable_autoprune = Some(true);
    config.disable_autoexpire = Some(true);

    // Fill the range
    create_reservation(&mut db, 5000);
    create_reservation(&mut db, 5001);

    // Attempt to reserve - should fail with exhaustion
    let options = ReserveOptions::new(key, None);
    let result = ReservePlan::new(options, &config).build_plan(&mut db);

    assert!(matches!(result, Err(Error::PortExhausted { tried_cleanup: false, .. })));
}

#[test]
fn test_auto_cleanup_partial_success() {
    let mut db = create_test_database();
    let config = create_config_with_cleanup();

    // Create mix of removable and permanent reservations
    create_stale_reservation(&mut db, 5000, "/nonexistent");
    create_fresh_reservation(&mut db, 5001, "/existing");

    // Both ports occupied, but one can be cleaned
    let options = ReserveOptions::new(key, None);
    let plan = ReservePlan::new(options, &config)
        .build_plan(&mut db)
        .unwrap();

    // Should succeed by cleaning port 5000
    match &plan.actions[0] {
        PlanAction::CreateReservation(res) => {
            assert_eq!(res.port().value(), 5000);
        }
        _ => panic!("Expected CreateReservation"),
    }
}
```

#### 6.3 Error Handling Tests

```rust
#[test]
fn test_cleanup_database_error_handling() {
    // Test that database errors during cleanup are properly propagated
    let mut db = create_failing_database(); // Mock that fails on certain operations

    let result = CleanupOperations::prune(&mut db, false);
    assert!(result.is_err());
}

#[test]
fn test_cleanup_partial_completion_reporting() {
    // Test that partial cleanup success is reported correctly
    let mut db = create_test_database();

    // Create multiple stale reservations
    for i in 0..5 {
        create_stale_reservation(&mut db, 5000 + i, &format!("/nonexistent{}", i));
    }

    // Inject failure after 3 deletions (would need mock/test harness)
    let result = CleanupOperations::prune(&mut db, false);

    // Even if it fails, we should know how many were removed
    // This would require CleanupOperations to track partial success
}
```

### Part 7: Error Handling Strategy

#### 7.1 Partial Cleanup Handling

During cleanup operations, if an error occurs partway through:

1. **Database State**: Individual deletions are atomic, so partial cleanup is valid
2. **Reporting**: Return success with the count of what was removed
3. **Logging**: Log warnings for any errors during cleanup
4. **Auto-cleanup**: Continue with allocation attempt even if cleanup partially failed

#### 7.2 Auto-cleanup Error Handling

```rust
// In attempt_cleanup_and_retry
fn attempt_cleanup_and_retry(...) -> Result<Option<Port>> {
    let mut freed_any = false;

    // Pruning errors are logged but don't stop the process
    match CleanupOperations::prune(db, false) {
        Ok(result) => {
            freed_any |= result.removed_count > 0;
            if result.removed_count > 0 {
                log::info!("Auto-pruned {} reservation(s)", result.removed_count);
            }
        }
        Err(e) => {
            log::warn!("Auto-prune failed: {}", e);
            // Continue to expiration attempt
        }
    }

    // Similar for expiration...

    // Always attempt retry if we freed anything
    if freed_any {
        // Retry allocation
    }
}
```

### Part 8: Output Format Specifications

#### 8.1 Prune Command Output

**Quiet Mode** (`--quiet`):
```
3
```
(Just the count, or nothing if 0)

**Normal Mode**:
```
Removed 3 reservation(s) for non-existent paths
```

**Verbose Mode** (`--verbose`):
```
Removed 3 reservation(s):
  - Port 5000: /old/project/dir (tag: api, project: old-project)
  - Port 5001: /deleted/service (tag: None, project: None)
  - Port 5002: /tmp/test (tag: test, project: experiment)
```

**Dry Run** (prefix all output with `[DRY RUN]`):
```
[DRY RUN] Would remove 3 reservation(s) for non-existent paths
```

#### 8.2 Expire Command Output

**Quiet Mode**:
```
5
```

**Normal Mode**:
```
Expired 5 reservation(s) older than 30 days
```

**Verbose Mode**:
```
Expired 5 reservation(s) older than 30 days:
  - Port 5000: /project/api (45 days old, project: api)
  - Port 5001: /project/web (31 days old, project: web)
  - Port 5002: /test/service (60 days old, project: None)
  - Port 5003: /dev/app (35 days old, project: dev)
  - Port 5004: /staging/api (40 days old, project: staging)
```

#### 8.3 Autoclean Command Output

**Quiet Mode**:
```
8
```

**Normal Mode**:
```
Removed 8 total reservation(s) (pruned: 3, expired: 5)
```

**Verbose Mode**:
```
Removed 8 total reservation(s):
  Pruned: 3 (non-existent paths)
  Expired: 5 (old reservations)

Pruned reservations:
  - Port 5000: /deleted/path1
  - Port 5001: /deleted/path2
  - Port 5002: /deleted/path3

Expired reservations:
  - Port 6000: /old/service1 (45 days old)
  - Port 6001: /old/service2 (60 days old)
  - Port 6002: /old/service3 (31 days old)
  - Port 6003: /old/service4 (40 days old)
  - Port 6004: /old/service5 (35 days old)
```

### Part 9: Dry-Run Behavior

#### 9.1 Command-Level Dry Run

All cleanup commands support `--dry-run` which:
- Performs all scanning and detection
- Reports what WOULD be removed
- Makes no actual changes to the database
- Prefixes all output with `[DRY RUN]`

#### 9.2 Auto-cleanup and Dry Run

Auto-cleanup during allocation NEVER operates in dry-run mode because:
- It's triggered automatically during a real allocation attempt
- The allocation needs actual cleanup to succeed
- It's an implementation detail, not a user-visible operation

If the reserve command itself is in dry-run mode, it doesn't open the database at all, so auto-cleanup never occurs.

## Implementation Order

1. **Enhanced AllocationResult** - Update enum to include cleanup hints
2. **CLI Command Implementations** - Create prune, expire, autoclean commands
3. **CLI Integration** - Wire commands into CLI structure and main
4. **Manual Testing** - Verify commands work correctly
5. **Auto-cleanup Logic** - Implement two-phase allocation with cleanup
6. **Integration Tests** - Comprehensive test coverage
7. **Documentation** - Update help text and README

## Risk Mitigation

### Data Safety
- All cleanup operations are opt-in (explicit commands or enabled config)
- Dry-run mode allows safe preview
- Fail-open policy for path checking (uncertain = preserve)
- Auto-cleanup only triggers on actual exhaustion

### Performance
- Cleanup operations are O(n) but typically fast
- Auto-cleanup adds negligible overhead (only on exhaustion)
- Database operations are atomic per reservation

### Compatibility
- No breaking changes to existing APIs
- New commands don't affect existing functionality
- Configuration defaults preserve current behavior

## Success Criteria

1. **Functional Requirements:**
   - `trop prune` removes reservations for non-existent paths ✓
   - `trop expire --days N` removes old reservations ✓
   - `trop autoclean` combines both operations ✓
   - All commands support `--dry-run` ✓
   - Auto-cleanup triggers on port exhaustion when enabled ✓

2. **Quality Requirements:**
   - Complete test coverage for all scenarios ✓
   - Clear error messages and logging ✓
   - Consistent output formatting ✓
   - Proper configuration precedence ✓

3. **Architecture Requirements:**
   - No breaking changes to allocator API ✓
   - Clean separation of concerns ✓
   - Maintainable and extensible design ✓