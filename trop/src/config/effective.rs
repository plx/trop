//! Source-aware effective configuration.
//!
//! This module wraps the existing [`Config`] value with provenance metadata.
//! The raw configuration schema and its merge semantics remain unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::config::merger::ConfigMerger;
use crate::config::schema::{
    CleanupConfig, Config, OccupancyConfig, OutputFormat, PortConfig, PortExclusion,
    ReservationGroup, DEFAULT_MAX_PORT, DEFAULT_MIN_PORT,
};
use crate::config::validator::ConfigValidator;
use crate::error::Result;

static BUILT_IN_PORTS: PortConfig = PortConfig {
    min: DEFAULT_MIN_PORT,
    max: Some(DEFAULT_MAX_PORT),
    max_offset: None,
};
static BUILT_IN_CLEANUP: CleanupConfig = CleanupConfig {
    expire_after_days: Some(30),
};
static BUILT_IN_OCCUPANCY: OccupancyConfig = OccupancyConfig {
    skip: Some(false),
    skip_ip4: Some(false),
    skip_ip6: Some(false),
    skip_tcp: Some(false),
    skip_udp: Some(false),
    check_all_interfaces: Some(false),
};

/// A leaf setting in the documented configuration schema.
///
/// Reservation groups are merged atomically, so their complete subtree is
/// represented by [`Self::Reservations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ConfigField {
    /// Default project metadata.
    Project,
    /// Runtime task metadata for single reservations.
    Task,
    /// Automatic database initialization policy.
    DisableAutoinit,
    /// Automatic stale-path pruning policy.
    DisableAutoprune,
    /// Automatic age-based expiration policy.
    DisableAutoexpire,
    /// Default list output format.
    OutputFormat,
    /// Unrelated-path permission.
    AllowUnrelatedPath,
    /// Project-change permission.
    AllowChangeProject,
    /// Task-change permission.
    AllowChangeTask,
    /// Combined project/task-change permission.
    AllowChange,
    /// Database lock wait timeout.
    MaximumLockWaitSeconds,
    /// Skip-all occupancy setting.
    OccupancySkip,
    /// Skip-IPv4 occupancy setting.
    OccupancySkipIp4,
    /// Skip-IPv6 occupancy setting.
    OccupancySkipIp6,
    /// Skip-TCP occupancy setting.
    OccupancySkipTcp,
    /// Skip-UDP occupancy setting.
    OccupancySkipUdp,
    /// All-interfaces occupancy setting.
    OccupancyCheckAllInterfaces,
    /// Minimum allocation port.
    PortsMin,
    /// Maximum allocation port.
    PortsMax,
    /// Maximum allocation offset.
    PortsMaxOffset,
    /// Accumulated excluded ports.
    ExcludedPorts,
    /// Cleanup expiration threshold.
    CleanupExpireAfterDays,
    /// The complete atomic reservation-group subtree.
    Reservations,
}

impl ConfigField {
    const ALL: &[Self] = &[
        Self::Project,
        Self::Task,
        Self::DisableAutoinit,
        Self::DisableAutoprune,
        Self::DisableAutoexpire,
        Self::OutputFormat,
        Self::AllowUnrelatedPath,
        Self::AllowChangeProject,
        Self::AllowChangeTask,
        Self::AllowChange,
        Self::MaximumLockWaitSeconds,
        Self::OccupancySkip,
        Self::OccupancySkipIp4,
        Self::OccupancySkipIp6,
        Self::OccupancySkipTcp,
        Self::OccupancySkipUdp,
        Self::OccupancyCheckAllInterfaces,
        Self::PortsMin,
        Self::PortsMax,
        Self::PortsMaxOffset,
        Self::ExcludedPorts,
        Self::CleanupExpireAfterDays,
        Self::Reservations,
    ];

    pub(crate) fn present_in(config: &Config) -> BTreeSet<Self> {
        let mut fields = BTreeSet::new();

        macro_rules! scalar {
            ($value:expr, $field:ident) => {
                if $value.is_some() {
                    fields.insert(Self::$field);
                }
            };
        }

        scalar!(config.project, Project);
        scalar!(config.disable_autoinit, DisableAutoinit);
        scalar!(config.disable_autoprune, DisableAutoprune);
        scalar!(config.disable_autoexpire, DisableAutoexpire);
        scalar!(config.output_format, OutputFormat);
        scalar!(config.allow_unrelated_path, AllowUnrelatedPath);
        scalar!(config.allow_change_project, AllowChangeProject);
        scalar!(config.allow_change_task, AllowChangeTask);
        scalar!(config.allow_change, AllowChange);
        scalar!(config.maximum_lock_wait_seconds, MaximumLockWaitSeconds);

        if let Some(ports) = &config.ports {
            fields.insert(Self::PortsMin);
            scalar!(ports.max, PortsMax);
            scalar!(ports.max_offset, PortsMaxOffset);
        }

        scalar!(config.excluded_ports, ExcludedPorts);

        if let Some(cleanup) = &config.cleanup {
            scalar!(cleanup.expire_after_days, CleanupExpireAfterDays);
        }

        if let Some(occupancy) = &config.occupancy_check {
            scalar!(occupancy.skip, OccupancySkip);
            scalar!(occupancy.skip_ip4, OccupancySkipIp4);
            scalar!(occupancy.skip_ip6, OccupancySkipIp6);
            scalar!(occupancy.skip_tcp, OccupancySkipTcp);
            scalar!(occupancy.skip_udp, OccupancySkipUdp);
            scalar!(occupancy.check_all_interfaces, OccupancyCheckAllInterfaces);
        }

        scalar!(config.reservations, Reservations);
        fields
    }
}

/// The role of a loaded YAML configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigFileKind {
    /// User-wide `config.yaml`.
    User,
    /// Project `trop.yaml` or an explicitly nominated project file.
    Project,
    /// Private project `trop.local.yaml`.
    Local,
}

/// The source that supplied an effective configuration value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigValueSource {
    /// A built-in default.
    BuiltIn,
    /// A YAML configuration file.
    File {
        /// The file's role in the precedence hierarchy.
        kind: ConfigFileKind,
        /// The loaded file path.
        path: PathBuf,
    },
    /// An environment variable.
    Environment {
        /// The exact variable name that was read.
        variable: &'static str,
    },
    /// A command-line override.
    CommandLine,
    /// A programmatic [`crate::config::ConfigBuilder::with_config`] override.
    Programmatic,
}

impl ConfigValueSource {
    /// Return the source file path, if this value came from a YAML file.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        match self {
            Self::File { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// Provenance for one effective configuration field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProvenance {
    winner: ConfigValueSource,
    contributors: Vec<ConfigValueSource>,
    winner_order: u64,
}

impl FieldProvenance {
    /// Return the highest-precedence source that affected the field.
    #[must_use]
    pub const fn winner(&self) -> &ConfigValueSource {
        &self.winner
    }

    /// Return all contributing sources in low-to-high precedence order.
    ///
    /// This is especially useful for accumulated fields such as
    /// `excluded_ports`.
    #[must_use]
    pub fn contributors(&self) -> &[ConfigValueSource] {
        &self.contributors
    }
}

/// A YAML file included in effective configuration resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedConfigFile {
    kind: ConfigFileKind,
    path: PathBuf,
}

impl LoadedConfigFile {
    pub(crate) fn new(kind: ConfigFileKind, path: PathBuf) -> Self {
        Self { kind, path }
    }

    /// Return the file's role in the precedence hierarchy.
    #[must_use]
    pub const fn kind(&self) -> ConfigFileKind {
        self.kind
    }

    /// Return the loaded file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A fully merged configuration with field-level provenance.
///
/// The contained [`Config`] intentionally retains its existing public shape so
/// current library consumers remain source compatible.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    config: Config,
    task: Option<String>,
    provenance: BTreeMap<ConfigField, FieldProvenance>,
    loaded_files: Vec<LoadedConfigFile>,
    next_source_order: u64,
}

impl EffectiveConfig {
    pub(crate) fn from_defaults(config: Config) -> Self {
        let mut effective = Self {
            config,
            task: None,
            provenance: BTreeMap::new(),
            loaded_files: Vec::new(),
            next_source_order: 0,
        };

        for &field in ConfigField::ALL {
            effective.record(field, ConfigValueSource::BuiltIn);
        }
        effective
    }

    pub(crate) fn merge_config(&mut self, config: &Config, source: &ConfigValueSource) {
        let mut changed = Vec::new();
        ConfigMerger::merge_into_observed(&mut self.config, config, |field| {
            changed.push(field);
        });
        for field in changed {
            self.record(field, source.clone());
        }
    }

    pub(crate) fn clear_reservations(&mut self, source: &ConfigValueSource) {
        self.config.reservations = None;
        self.record(ConfigField::Reservations, source.clone());
    }

    pub(crate) fn set_task(&mut self, task: Option<String>, source: ConfigValueSource) {
        self.task = task;
        self.record(ConfigField::Task, source);
    }

    pub(crate) fn set_project(&mut self, project: Option<String>, source: ConfigValueSource) {
        self.config.project = project;
        self.record(ConfigField::Project, source);
    }

    pub(crate) fn record_file(&mut self, kind: ConfigFileKind, path: PathBuf) {
        self.loaded_files.push(LoadedConfigFile::new(kind, path));
    }

    pub(crate) const fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub(crate) fn record(&mut self, field: ConfigField, source: ConfigValueSource) {
        let winner_order = self.next_source_order;
        self.next_source_order += 1;
        match self.provenance.get_mut(&field) {
            Some(provenance) => {
                provenance.winner.clone_from(&source);
                provenance.winner_order = winner_order;
                if !provenance.contributors.contains(&source) {
                    provenance.contributors.push(source);
                }
            }
            None => {
                self.provenance.insert(
                    field,
                    FieldProvenance {
                        winner: source.clone(),
                        contributors: vec![source],
                        winner_order,
                    },
                );
            }
        }
    }

    /// Return the merged configuration value.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Consume this wrapper and return the merged configuration value.
    #[must_use]
    pub fn into_config(self) -> Config {
        self.config
    }

    /// Return provenance for a configuration field.
    #[must_use]
    pub fn provenance(&self, field: ConfigField) -> Option<&FieldProvenance> {
        self.provenance.get(&field)
    }

    /// Return all YAML files included in resolution order.
    #[must_use]
    pub fn loaded_files(&self) -> &[LoadedConfigFile] {
        &self.loaded_files
    }

    /// Return the highest-precedence loaded file of the requested kind.
    #[must_use]
    pub fn loaded_file(&self, kind: ConfigFileKind) -> Option<&Path> {
        self.loaded_files
            .iter()
            .rev()
            .find(|file| file.kind == kind)
            .map(LoadedConfigFile::path)
    }

    /// Return the effective project identifier.
    #[must_use]
    pub fn project(&self) -> Option<&str> {
        self.config.project.as_deref()
    }

    /// Return the effective runtime task identifier.
    ///
    /// Task is intentionally not part of the YAML schema. Its supported
    /// sources are `TROP_TASK` and the single-reservation command line.
    #[must_use]
    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// Return the effective port configuration.
    #[must_use]
    pub fn ports(&self) -> &PortConfig {
        self.config.ports.as_ref().unwrap_or(&BUILT_IN_PORTS)
    }

    /// Return the effective accumulated exclusions.
    #[must_use]
    pub fn excluded_ports(&self) -> &[PortExclusion] {
        self.config.excluded_ports.as_deref().unwrap_or_default()
    }

    /// Return the effective cleanup configuration.
    #[must_use]
    pub fn cleanup(&self) -> &CleanupConfig {
        self.config.cleanup.as_ref().unwrap_or(&BUILT_IN_CLEANUP)
    }

    /// Return the effective occupancy configuration.
    #[must_use]
    pub fn occupancy_check(&self) -> &OccupancyConfig {
        self.config
            .occupancy_check
            .as_ref()
            .unwrap_or(&BUILT_IN_OCCUPANCY)
    }

    /// Return the effective reservation group.
    #[must_use]
    pub fn reservations(&self) -> Option<&ReservationGroup> {
        self.config.reservations.as_ref()
    }

    /// Return whether automatic database initialization is disabled.
    #[must_use]
    pub fn disable_autoinit(&self) -> bool {
        self.config.disable_autoinit.unwrap_or(false)
    }

    /// Return whether automatic pruning is disabled.
    #[must_use]
    pub fn disable_autoprune(&self) -> bool {
        self.config.disable_autoprune.unwrap_or(false)
    }

    /// Return whether automatic expiration is disabled.
    #[must_use]
    pub fn disable_autoexpire(&self) -> bool {
        self.config.disable_autoexpire.unwrap_or(false)
    }

    /// Return whether operations on unrelated paths are allowed.
    #[must_use]
    pub fn allow_unrelated_path(&self) -> bool {
        self.config.allow_unrelated_path.unwrap_or(false)
    }

    /// Return the effective project-change permission.
    #[must_use]
    pub fn allow_project_change(&self) -> bool {
        self.config.allow_change.unwrap_or(false)
            || self.config.allow_change_project.unwrap_or(false)
    }

    /// Return the effective task-change permission.
    #[must_use]
    pub fn allow_task_change(&self) -> bool {
        self.config.allow_change.unwrap_or(false) || self.config.allow_change_task.unwrap_or(false)
    }

    /// Return the effective database lock wait in seconds.
    #[must_use]
    pub fn maximum_lock_wait_seconds(&self) -> u64 {
        self.config.maximum_lock_wait_seconds.unwrap_or(5)
    }

    /// Return the effective list output format.
    #[must_use]
    pub fn output_format(&self) -> OutputFormat {
        self.config.output_format.unwrap_or(OutputFormat::Table)
    }

    /// Apply precise command-line fields to this effective configuration.
    ///
    /// Only fields named by `explicit_fields` are considered. This allows a
    /// command to override `ports.max`, for example, without also replacing
    /// `ports.min` merely because [`PortConfig`] stores `min` as a concrete
    /// value.
    ///
    /// # Errors
    ///
    /// Returns a validation error if the resulting configuration is invalid.
    pub fn apply_cli_config<I>(&mut self, config: &Config, explicit_fields: I) -> Result<()>
    where
        I: IntoIterator<Item = ConfigField>,
    {
        self.apply_precise(
            config,
            explicit_fields.into_iter().collect(),
            &ConfigValueSource::CommandLine,
        );
        self.validate(true)
    }

    pub(crate) fn validate(&self, is_tropfile: bool) -> Result<()> {
        ConfigValidator::validate(&self.config, is_tropfile)
            .and_then(|()| {
                self.task.as_deref().map_or(Ok(()), |task| {
                    ConfigValidator::validate_runtime_identifier("task", task)
                })
            })
            .map_err(|error| {
                let source = match &error {
                    crate::error::Error::Validation { field, .. } => self.validation_source(field),
                    _ => None,
                };
                match source {
                    Some(source) => ConfigValidator::annotate(error, source),
                    None => error,
                }
            })
    }

    fn validation_source(&self, field: &str) -> Option<&ConfigValueSource> {
        const PORT_FIELDS: &[ConfigField] = &[
            ConfigField::PortsMin,
            ConfigField::PortsMax,
            ConfigField::PortsMaxOffset,
        ];

        let fields: &[ConfigField] = if field == "ports" {
            PORT_FIELDS
        } else if field == "ports.min" {
            &[ConfigField::PortsMin]
        } else if field == "ports.max" {
            &[ConfigField::PortsMax]
        } else if field == "ports.max_offset" {
            &[ConfigField::PortsMaxOffset]
        } else if field.starts_with("excluded_ports") {
            &[ConfigField::ExcludedPorts]
        } else if field == "cleanup.expire_after_days" {
            &[ConfigField::CleanupExpireAfterDays]
        } else if field.starts_with("reservations") {
            &[ConfigField::Reservations]
        } else if field == "project" {
            &[ConfigField::Project]
        } else if field == "task" {
            &[ConfigField::Task]
        } else if field == "maximum_lock_wait_seconds" {
            &[ConfigField::MaximumLockWaitSeconds]
        } else {
            &[]
        };

        fields
            .iter()
            .filter_map(|field| self.provenance.get(field))
            .max_by_key(|provenance| provenance.winner_order)
            .map(FieldProvenance::winner)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn apply_precise(
        &mut self,
        source_config: &Config,
        fields: BTreeSet<ConfigField>,
        source: &ConfigValueSource,
    ) {
        let both_port_bounds = fields.contains(&ConfigField::PortsMax)
            && fields.contains(&ConfigField::PortsMaxOffset)
            && source_config
                .ports
                .as_ref()
                .is_some_and(|ports| ports.max.is_some() && ports.max_offset.is_some());

        for field in fields {
            let mut changed = Vec::new();

            match field {
                ConfigField::Project => {
                    if source_config.project.is_some() {
                        self.config.project.clone_from(&source_config.project);
                        changed.push(field);
                    }
                }
                ConfigField::Task => {}
                ConfigField::DisableAutoinit => {
                    if let Some(value) = source_config.disable_autoinit {
                        self.config.disable_autoinit = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::DisableAutoprune => {
                    if let Some(value) = source_config.disable_autoprune {
                        self.config.disable_autoprune = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::DisableAutoexpire => {
                    if let Some(value) = source_config.disable_autoexpire {
                        self.config.disable_autoexpire = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OutputFormat => {
                    if let Some(value) = source_config.output_format {
                        self.config.output_format = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::AllowUnrelatedPath => {
                    if let Some(value) = source_config.allow_unrelated_path {
                        self.config.allow_unrelated_path = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::AllowChangeProject => {
                    if let Some(value) = source_config.allow_change_project {
                        self.config.allow_change_project = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::AllowChangeTask => {
                    if let Some(value) = source_config.allow_change_task {
                        self.config.allow_change_task = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::AllowChange => {
                    if let Some(value) = source_config.allow_change {
                        self.config.allow_change = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::MaximumLockWaitSeconds => {
                    if let Some(value) = source_config.maximum_lock_wait_seconds {
                        self.config.maximum_lock_wait_seconds = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::PortsMin => {
                    if let Some(source_ports) = &source_config.ports {
                        self.config
                            .ports
                            .get_or_insert_with(PortConfig::default)
                            .min = source_ports.min;
                        changed.push(field);
                    }
                }
                ConfigField::PortsMax => {
                    if let Some(value) = source_config.ports.as_ref().and_then(|ports| ports.max) {
                        let ports = self.config.ports.get_or_insert_with(PortConfig::default);
                        ports.max = Some(value);
                        changed.push(ConfigField::PortsMax);
                        if !both_port_bounds {
                            ports.max_offset = None;
                            changed.push(ConfigField::PortsMaxOffset);
                        }
                    }
                }
                ConfigField::PortsMaxOffset => {
                    if let Some(value) = source_config
                        .ports
                        .as_ref()
                        .and_then(|ports| ports.max_offset)
                    {
                        let ports = self.config.ports.get_or_insert_with(PortConfig::default);
                        ports.max_offset = Some(value);
                        changed.push(ConfigField::PortsMaxOffset);
                        if !both_port_bounds {
                            ports.max = None;
                            changed.push(ConfigField::PortsMax);
                        }
                    }
                }
                ConfigField::ExcludedPorts => {
                    if let Some(exclusions) = &source_config.excluded_ports {
                        self.config
                            .excluded_ports
                            .get_or_insert_with(Vec::new)
                            .extend(exclusions.clone());
                        changed.push(field);
                    }
                }
                ConfigField::CleanupExpireAfterDays => {
                    if let Some(value) = source_config
                        .cleanup
                        .as_ref()
                        .and_then(|cleanup| cleanup.expire_after_days)
                    {
                        self.config
                            .cleanup
                            .get_or_insert_with(CleanupConfig::default)
                            .expire_after_days = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancySkip => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.skip)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .skip = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancySkipIp4 => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.skip_ip4)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .skip_ip4 = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancySkipIp6 => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.skip_ip6)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .skip_ip6 = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancySkipTcp => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.skip_tcp)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .skip_tcp = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancySkipUdp => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.skip_udp)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .skip_udp = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::OccupancyCheckAllInterfaces => {
                    if let Some(value) = source_config
                        .occupancy_check
                        .as_ref()
                        .and_then(|occupancy| occupancy.check_all_interfaces)
                    {
                        self.config
                            .occupancy_check
                            .get_or_insert_with(OccupancyConfig::default)
                            .check_all_interfaces = Some(value);
                        changed.push(field);
                    }
                }
                ConfigField::Reservations => {
                    if source_config.reservations.is_some() {
                        self.config
                            .reservations
                            .clone_from(&source_config.reservations);
                        changed.push(field);
                    }
                }
            }

            for changed_field in changed {
                self.record(changed_field, source.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_command_line_project_clear_has_command_line_provenance() {
        let mut effective = EffectiveConfig::from_defaults(Config {
            project: Some("lower-project".to_string()),
            ..Default::default()
        });
        effective.set_project(None, ConfigValueSource::CommandLine);

        assert_eq!(effective.project(), None);
        assert_eq!(
            effective
                .provenance(ConfigField::Project)
                .map(FieldProvenance::winner),
            Some(&ConfigValueSource::CommandLine)
        );
    }

    #[test]
    fn command_line_task_clear_overrides_environment_with_provenance() {
        let mut effective = EffectiveConfig::from_defaults(Config::default());
        effective.set_task(
            Some("environment-task".to_string()),
            ConfigValueSource::Environment {
                variable: "TROP_TASK",
            },
        );
        effective.set_task(None, ConfigValueSource::CommandLine);

        assert_eq!(effective.task(), None);
        assert_eq!(
            effective
                .provenance(ConfigField::Task)
                .map(FieldProvenance::winner),
            Some(&ConfigValueSource::CommandLine)
        );
    }
}
