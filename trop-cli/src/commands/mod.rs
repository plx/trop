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
//! - `assert_reservation`: Assert reservation exists for path/tag
//! - `assert_port`: Assert specific port is reserved
//! - `assert_data_dir`: Assert data directory exists and is valid
//! - `port_info`: Display information about a specific port
//! - `show_data_dir`: Show resolved data directory path
//! - `show_path`: Show resolved path for a reservation
//! - `scan`: Scan port range for occupied ports
//! - `validate`: Validate configuration file
//! - `exclude`: Add port or range to exclusion list
//! - `compact_exclusions`: Compact exclusion list to minimal representation

pub mod assert_data_dir;
pub mod assert_port;
pub mod assert_reservation;
pub mod autoclean;
pub mod autoreserve;
pub mod compact_exclusions;
pub mod exclude;
pub mod expire;
pub mod init;
pub mod list;
pub mod list_projects;
pub mod migrate;
pub mod port_info;
pub mod prune;
pub mod release;
pub mod reserve;
pub mod reserve_group;
pub mod scan;
pub mod show_data_dir;
pub mod show_path;
pub mod validate;

pub use assert_data_dir::AssertDataDirCommand;
pub use assert_port::AssertPortCommand;
pub use assert_reservation::AssertReservationCommand;
pub use autoclean::AutocleanCommand;
pub use autoreserve::AutoreserveCommand;
pub use compact_exclusions::CompactExclusionsCommand;
pub use exclude::ExcludeCommand;
pub use expire::ExpireCommand;
pub use init::InitCommand;
pub use list::ListCommand;
pub use list_projects::ListProjectsCommand;
pub use migrate::MigrateCommand;
pub use port_info::PortInfoCommand;
pub use prune::PruneCommand;
pub use release::ReleaseCommand;
pub use reserve::ReserveCommand;
pub use reserve_group::ReserveGroupCommand;
pub use scan::ScanCommand;
pub use show_data_dir::ShowDataDirCommand;
pub use show_path::ShowPathCommand;
pub use validate::ValidateCommand;
