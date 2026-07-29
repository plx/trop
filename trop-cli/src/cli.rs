//! CLI structure and command definitions.
//!
//! This module defines the main CLI structure using clap's derive macros,
//! including global options and subcommands.

use crate::commands::{
    AssertDataDirCommand, AssertPortCommand, AssertReservationCommand, AutocleanCommand,
    AutoreserveCommand, CompactExclusionsCommand, CompletionsCommand, ExcludeCommand,
    ExpireCommand, InitCommand, ListCommand, ListProjectsCommand, MigrateCommand, PortInfoCommand,
    PruneCommand, ReleaseCommand, ReserveCommand, ReserveGroupCommand, ScanCommand,
    ShowDataDirCommand, ShowPathCommand, ValidateCommand,
};
use crate::error::CliError;
use crate::invocation::{
    CommandLineConfig, ConfigRequest, ConfigScope, InvocationContext, ReservationMetadataRequest,
};
use crate::utils::GlobalOptions;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use trop::config::{
    CleanupConfig, ConfigField, OccupancyConfig, OutputFormat as ConfigOutputFormat, PortConfig,
    DEFAULT_MAX_PORT, DEFAULT_MIN_PORT,
};

/// Command-line tool for managing ephemeral port reservations.
#[derive(Parser)]
#[command(name = "trop")]
#[command(version, about = "Manage ephemeral port reservations", long_about = None)]
pub struct Cli {
    /// Enable verbose output
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Suppress non-essential output
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Override the data directory location
    #[arg(long, value_name = "PATH", global = true, env = "TROP_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Database lock wait in seconds; 0 disables waiting (max 2147483)
    #[arg(long, value_name = "SECONDS", global = true)]
    pub busy_timeout: Option<u32>,

    /// Disable automatic database initialization
    #[arg(long, global = true)]
    pub disable_autoinit: bool,

    #[command(subcommand)]
    pub command: Command,
}

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

    /// Assert that a reservation exists for a path/tag
    AssertReservation(AssertReservationCommand),

    /// Assert that a specific port is reserved
    AssertPort(AssertPortCommand),

    /// Assert that the data directory exists and is valid
    AssertDataDir(AssertDataDirCommand),

    /// Display information about a specific port
    #[command(name = "port-info")]
    PortInfo(PortInfoCommand),

    /// Show the resolved data directory path
    ShowDataDir(ShowDataDirCommand),

    /// Show the resolved path for a reservation
    ShowPath(ShowPathCommand),

    /// Scan port range for occupied ports
    Scan(ScanCommand),

    /// Validate a configuration file
    Validate(ValidateCommand),

    /// Add port or range to exclusion list
    Exclude(ExcludeCommand),

    /// Compact exclusion list to minimal representation
    CompactExclusions(CompactExclusionsCommand),

    /// Initialize trop data directory and database
    Init(InitCommand),

    /// List all unique project identifiers
    ListProjects(ListProjectsCommand),

    /// Migrate reservations between paths
    Migrate(MigrateCommand),

    /// Generate shell completion scripts
    Completions(CompletionsCommand),
}

impl Command {
    /// Stable command name used in database error context.
    pub(crate) const fn name(&self) -> &'static str {
        match self {
            Self::Reserve(_) => "reserve",
            Self::Release(_) => "release",
            Self::List(_) => "list",
            Self::ReserveGroup(_) => "reserve-group",
            Self::Autoreserve(_) => "autoreserve",
            Self::Prune(_) => "prune",
            Self::Expire(_) => "expire",
            Self::Autoclean(_) => "autoclean",
            Self::AssertReservation(_) => "assert-reservation",
            Self::AssertPort(_) => "assert-port",
            Self::AssertDataDir(_) => "assert-data-dir",
            Self::PortInfo(_) => "port-info",
            Self::ShowDataDir(_) => "show-data-dir",
            Self::ShowPath(_) => "show-path",
            Self::Scan(_) => "scan",
            Self::Validate(_) => "validate",
            Self::Exclude(_) => "exclude",
            Self::CompactExclusions(_) => "compact-exclusions",
            Self::Init(_) => "init",
            Self::ListProjects(_) => "list-projects",
            Self::Migrate(_) => "migrate",
            Self::Completions(_) => "completions",
        }
    }

    /// Translate all configuration-shaped CLI arguments exactly once.
    pub(crate) fn config_request(&self, global: &GlobalOptions) -> Result<ConfigRequest, CliError> {
        let mut command_line = CommandLineConfig::default();
        let mut data_dir_override = None;

        if let Some(seconds) = global.busy_timeout {
            command_line.config.maximum_lock_wait_seconds = Some(u64::from(seconds));
            command_line.record(ConfigField::MaximumLockWaitSeconds);
        }
        if global.disable_autoinit {
            command_line.config.disable_autoinit = Some(true);
            command_line.record(ConfigField::DisableAutoinit);
        }

        let scope = match self {
            Self::Reserve(command) => {
                if let Some(project) = &command.project {
                    command_line.config.project = Some(project.clone());
                    command_line.record(ConfigField::Project);
                }
                command_line.reservation_metadata = Some(ReservationMetadataRequest {
                    clear_project: command.clear_project,
                    task: command.task.clone(),
                    clear_task: command.clear_task,
                });

                let min = command
                    .min
                    .as_deref()
                    .map(crate::commands::reserve::parse_port_string)
                    .transpose()
                    .map_err(|error| {
                        CliError::InvalidArguments(format!("invalid --min value: {error}"))
                    })?;
                let max = command
                    .max
                    .as_deref()
                    .map(crate::commands::reserve::parse_port_string)
                    .transpose()
                    .map_err(|error| {
                        CliError::InvalidArguments(format!("invalid --max value: {error}"))
                    })?;
                if let (Some(min), Some(max)) = (min, max) {
                    if min > max {
                        return Err(CliError::InvalidArguments(format!(
                            "Invalid port range: min ({min}) must be less than or equal to max ({max})"
                        )));
                    }
                }
                set_port_overrides(&mut command_line, min, max);

                set_true(
                    &mut command_line,
                    command.allow_unrelated_path,
                    ConfigField::AllowUnrelatedPath,
                );
                set_true(
                    &mut command_line,
                    command.allow_project_change,
                    ConfigField::AllowChangeProject,
                );
                set_true(
                    &mut command_line,
                    command.allow_task_change,
                    ConfigField::AllowChangeTask,
                );
                set_true(
                    &mut command_line,
                    command.allow_change,
                    ConfigField::AllowChange,
                );
                set_true(
                    &mut command_line,
                    command.disable_autoprune || command.disable_autoclean,
                    ConfigField::DisableAutoprune,
                );
                set_true(
                    &mut command_line,
                    command.disable_autoexpire || command.disable_autoclean,
                    ConfigField::DisableAutoexpire,
                );
                set_occupancy_overrides(
                    &mut command_line,
                    command.skip_occupancy_check,
                    command.skip_ipv4,
                    command.skip_ipv6,
                    command.skip_tcp,
                    command.skip_udp,
                    command.check_all_interfaces,
                );
                ConfigScope::Discover
            }
            Self::ReserveGroup(command) => {
                if !command.config_path.exists() {
                    return Err(CliError::InvalidArguments(format!(
                        "Configuration file not found: {}",
                        command.config_path.display()
                    )));
                }
                if !command.config_path.is_file() {
                    return Err(CliError::InvalidArguments(format!(
                        "Path is not a file: {}",
                        command.config_path.display()
                    )));
                }
                set_permissions(
                    &mut command_line,
                    command.allow_unrelated_path,
                    command.allow_project_change,
                    command.allow_task_change,
                    command.allow_change,
                );
                set_occupancy_overrides(
                    &mut command_line,
                    command.skip_occupancy_check,
                    command.skip_ipv4,
                    command.skip_ipv6,
                    command.skip_tcp,
                    command.skip_udp,
                    command.check_all_interfaces,
                );
                ConfigScope::ExplicitProject(command.config_path.clone())
            }
            Self::Autoreserve(command) => {
                set_permissions(
                    &mut command_line,
                    command.allow_unrelated_path,
                    command.allow_project_change,
                    command.allow_task_change,
                    command.allow_change,
                );
                set_occupancy_overrides(
                    &mut command_line,
                    command.skip_occupancy_check,
                    command.skip_ipv4,
                    command.skip_ipv6,
                    command.skip_tcp,
                    command.skip_udp,
                    command.check_all_interfaces,
                );
                ConfigScope::Discover
            }
            Self::List(command) => {
                if let Some(format) = command.format {
                    command_line.config.output_format = Some(match format {
                        crate::commands::list::OutputFormat::Table => ConfigOutputFormat::Table,
                        crate::commands::list::OutputFormat::Json => ConfigOutputFormat::Json,
                        crate::commands::list::OutputFormat::Csv => ConfigOutputFormat::Csv,
                        crate::commands::list::OutputFormat::Tsv => ConfigOutputFormat::Tsv,
                    });
                    command_line.record(ConfigField::OutputFormat);
                }
                ConfigScope::Discover
            }
            Self::Expire(command) => {
                set_cleanup_days(&mut command_line, command.days);
                ConfigScope::Discover
            }
            Self::Autoclean(command) => {
                set_cleanup_days(&mut command_line, command.days);
                ConfigScope::Discover
            }
            Self::Scan(command) => {
                set_port_overrides(&mut command_line, command.min, command.max);
                set_occupancy_overrides(
                    &mut command_line,
                    false,
                    command.skip_ipv4,
                    command.skip_ipv6,
                    command.skip_tcp,
                    command.skip_udp,
                    command.check_all_interfaces,
                );
                ConfigScope::Discover
            }
            Self::PortInfo(command) => {
                set_occupancy_overrides(
                    &mut command_line,
                    command.skip_occupancy_check,
                    command.skip_ipv4,
                    command.skip_ipv6,
                    command.skip_tcp,
                    command.skip_udp,
                    command.check_all_interfaces,
                );
                ConfigScope::Discover
            }
            Self::Release(command) => {
                set_true(
                    &mut command_line,
                    command.allow_unrelated_path,
                    ConfigField::AllowUnrelatedPath,
                );
                ConfigScope::Discover
            }
            Self::Prune(_)
            | Self::AssertReservation(_)
            | Self::AssertPort(_)
            | Self::Exclude(_)
            | Self::ListProjects(_)
            | Self::Migrate(_) => ConfigScope::Discover,
            Self::AssertDataDir(command) => {
                data_dir_override.clone_from(&command.data_dir);
                if command.validate {
                    ConfigScope::Discover
                } else {
                    ConfigScope::None
                }
            }
            Self::Init(command) => {
                data_dir_override.clone_from(&command.data_dir);
                ConfigScope::DefaultsEnvironmentCli
            }
            Self::ShowDataDir(_)
            | Self::ShowPath(_)
            | Self::Validate(_)
            | Self::CompactExclusions(_)
            | Self::Completions(_) => ConfigScope::None,
        };

        Ok(ConfigRequest::new(scope, command_line).with_data_dir_override(data_dir_override))
    }

    /// Dispatch one command with the already-resolved invocation context.
    pub(crate) fn execute(self, context: &InvocationContext) -> Result<(), CliError> {
        match self {
            Self::Reserve(command) => command.execute(context),
            Self::Release(command) => command.execute(context),
            Self::List(command) => command.execute(context),
            Self::ReserveGroup(command) => command.execute(context),
            Self::Autoreserve(command) => command.execute(context),
            Self::Prune(command) => command.execute(context),
            Self::Expire(command) => command.execute(context),
            Self::Autoclean(command) => command.execute(context),
            Self::AssertReservation(command) => command.execute(context),
            Self::AssertPort(command) => command.execute(context),
            Self::AssertDataDir(command) => command.execute(context),
            Self::PortInfo(command) => command.execute(context),
            Self::ShowDataDir(command) => command.execute(context.global()),
            Self::ShowPath(command) => command.execute(context),
            Self::Scan(command) => command.execute(context),
            Self::Validate(command) => command.execute(context.global()),
            Self::Exclude(command) => command.execute(context),
            Self::CompactExclusions(command) => command.execute(context.global()),
            Self::Init(command) => command.execute(context),
            Self::ListProjects(command) => command.execute(context),
            Self::Migrate(command) => command.execute(context),
            Self::Completions(command) => command.execute(context.global()),
        }
    }
}

fn set_port_overrides(command_line: &mut CommandLineConfig, min: Option<u16>, max: Option<u16>) {
    if min.is_none() && max.is_none() {
        return;
    }

    command_line.config.ports = Some(PortConfig {
        min: min.unwrap_or(DEFAULT_MIN_PORT),
        max: max.or(Some(DEFAULT_MAX_PORT)),
        max_offset: None,
    });
    if min.is_some() {
        command_line.record(ConfigField::PortsMin);
    }
    if max.is_some() {
        command_line.record(ConfigField::PortsMax);
        command_line.record(ConfigField::PortsMaxOffset);
    }
}

fn set_permissions(
    command_line: &mut CommandLineConfig,
    allow_unrelated_path: bool,
    allow_project_change: bool,
    allow_task_change: bool,
    allow_change: bool,
) {
    set_true(
        command_line,
        allow_unrelated_path,
        ConfigField::AllowUnrelatedPath,
    );
    set_true(
        command_line,
        allow_project_change,
        ConfigField::AllowChangeProject,
    );
    set_true(
        command_line,
        allow_task_change,
        ConfigField::AllowChangeTask,
    );
    set_true(command_line, allow_change, ConfigField::AllowChange);
}

#[allow(clippy::too_many_arguments)]
fn set_occupancy_overrides(
    command_line: &mut CommandLineConfig,
    skip: bool,
    skip_ip4: bool,
    skip_ip6: bool,
    skip_tcp: bool,
    skip_udp: bool,
    check_all_interfaces: bool,
) {
    if !(skip || skip_ip4 || skip_ip6 || skip_tcp || skip_udp || check_all_interfaces) {
        return;
    }

    command_line.config.occupancy_check = Some(OccupancyConfig {
        skip: skip.then_some(true),
        skip_ip4: skip_ip4.then_some(true),
        skip_ip6: skip_ip6.then_some(true),
        skip_tcp: skip_tcp.then_some(true),
        skip_udp: skip_udp.then_some(true),
        check_all_interfaces: check_all_interfaces.then_some(true),
    });

    for (enabled, field) in [
        (skip, ConfigField::OccupancySkip),
        (skip_ip4, ConfigField::OccupancySkipIp4),
        (skip_ip6, ConfigField::OccupancySkipIp6),
        (skip_tcp, ConfigField::OccupancySkipTcp),
        (skip_udp, ConfigField::OccupancySkipUdp),
        (
            check_all_interfaces,
            ConfigField::OccupancyCheckAllInterfaces,
        ),
    ] {
        if enabled {
            command_line.record(field);
        }
    }
}

fn set_cleanup_days(command_line: &mut CommandLineConfig, days: Option<u32>) {
    if let Some(days) = days {
        command_line.config.cleanup = Some(CleanupConfig {
            expire_after_days: Some(days),
        });
        command_line.record(ConfigField::CleanupExpireAfterDays);
    }
}

fn set_true(command_line: &mut CommandLineConfig, enabled: bool, field: ConfigField) {
    if !enabled {
        return;
    }

    match field {
        ConfigField::DisableAutoprune => command_line.config.disable_autoprune = Some(true),
        ConfigField::DisableAutoexpire => command_line.config.disable_autoexpire = Some(true),
        ConfigField::AllowUnrelatedPath => command_line.config.allow_unrelated_path = Some(true),
        ConfigField::AllowChangeProject => command_line.config.allow_change_project = Some(true),
        ConfigField::AllowChangeTask => command_line.config.allow_change_task = Some(true),
        ConfigField::AllowChange => command_line.config.allow_change = Some(true),
        _ => unreachable!("set_true called for a non-boolean command override"),
    }
    command_line.record(field);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals() -> GlobalOptions {
        GlobalOptions {
            verbose: false,
            quiet: false,
            data_dir: None,
            busy_timeout: None,
            disable_autoinit: false,
        }
    }

    fn occupancy_request(args: &[&str]) -> OccupancyConfig {
        let cli = Cli::try_parse_from(args).expect("occupancy arguments should parse");
        cli.command
            .config_request(&globals())
            .expect("occupancy config request should build")
            .command_line
            .config
            .occupancy_check
            .expect("command should create an occupancy CLI layer")
    }

    fn all_cli_overrides() -> OccupancyConfig {
        OccupancyConfig {
            skip: Some(true),
            skip_ip4: Some(true),
            skip_ip6: Some(true),
            skip_tcp: Some(true),
            skip_udp: Some(true),
            check_all_interfaces: Some(true),
        }
    }

    #[test]
    fn occupancy_cli_layer_is_identical_for_all_allocation_commands() {
        let project = tempfile::NamedTempFile::new().unwrap();
        let project_path = project.path().to_string_lossy().into_owned();
        let flags = [
            "--skip-occupancy-check",
            "--skip-ipv4",
            "--skip-ipv6",
            "--skip-tcp",
            "--skip-udp",
            "--check-all-interfaces",
        ];

        let mut reserve = vec!["trop", "reserve"];
        reserve.extend(flags.iter().copied());
        let mut reserve_group = vec!["trop", "reserve-group", &project_path];
        reserve_group.extend(flags.iter().copied());
        let mut autoreserve = vec!["trop", "autoreserve"];
        autoreserve.extend(flags.iter().copied());

        let expected = all_cli_overrides();
        assert_eq!(occupancy_request(&reserve), expected);
        assert_eq!(occupancy_request(&reserve_group), expected);
        assert_eq!(occupancy_request(&autoreserve), expected);
    }

    #[test]
    fn port_info_uses_the_same_complete_occupancy_cli_layer() {
        let args = [
            "trop",
            "port-info",
            "5050",
            "--include-occupancy",
            "--skip-occupancy-check",
            "--skip-ipv4",
            "--skip-ipv6",
            "--skip-tcp",
            "--skip-udp",
            "--check-all-interfaces",
        ];

        assert_eq!(occupancy_request(&args), all_cli_overrides());
    }

    #[test]
    fn scan_uses_the_same_leaf_overrides_except_for_intentional_skip_all_omission() {
        let args = [
            "trop",
            "scan",
            "--skip-ipv4",
            "--skip-ipv6",
            "--skip-tcp",
            "--skip-udp",
            "--check-all-interfaces",
        ];
        let mut expected = all_cli_overrides();
        expected.skip = None;

        assert_eq!(occupancy_request(&args), expected);
    }

    #[test]
    fn port_info_rejects_occupancy_overrides_without_requested_occupancy_output() {
        let Err(error) = Cli::try_parse_from(["trop", "port-info", "5050", "--skip-tcp"]) else {
            panic!("an ignored occupancy override must be rejected");
        };
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn a_partial_cli_override_records_only_its_named_leaf() {
        let request = occupancy_request(&["trop", "scan", "--skip-tcp"]);
        assert_eq!(
            request,
            OccupancyConfig {
                skip_tcp: Some(true),
                ..Default::default()
            }
        );
    }
}
