//! Port-bound layering across configuration sources.
//!
//! The verified port-bounds kernel decides how `ports.min`, `ports.max`, and
//! `ports.max_offset` from the built-in defaults, user and project YAML
//! files, the environment, programmatic overrides, and the command line
//! combine. These tests exercise the (trusted) glue in each source against an
//! independently written model of the intended semantics:
//!
//! - a layer's `max_offset` is anchored at the `min` in effect *at that
//!   layer* and fixed there as an absolute upper bound;
//! - a layer that sets no upper bound keeps the one below unchanged, even
//!   when it changes `min`.
//!
//! Environment variables are process-global, so every test here is
//! `#[serial]`.

use serial_test::serial;
use std::env;
use std::fs;
use std::path::Path;
use tempfile::TempDir;
use trop::config::{
    Config, ConfigBuilder, ConfigField, ConfigFileKind, ConfigValidator, ConfigValueSource,
    EffectiveConfig, PortConfig, ReservationGroup, ServiceDefinition, DEFAULT_MAX_PORT,
    DEFAULT_MIN_PORT,
};
use trop::port::allocator::port_range_from_config;
use trop::Error;

const PORT_VARIABLES: [&str; 3] = ["TROP_PORT_MIN", "TROP_PORT_MAX", "TROP_PORT_MAX_OFFSET"];

/// Sets (or removes) the `TROP_PORT_*` variables for one build and restores
/// the previous values afterwards.
struct PortEnv {
    saved: Vec<(&'static str, Option<String>)>,
}

impl PortEnv {
    fn set(min: Option<u16>, max: Option<u16>, max_offset: Option<u16>) -> Self {
        let saved = PORT_VARIABLES
            .iter()
            .map(|&key| (key, env::var(key).ok()))
            .collect();
        for (key, value) in PORT_VARIABLES.into_iter().zip([min, max, max_offset]) {
            match value {
                Some(value) => env::set_var(key, value.to_string()),
                None => env::remove_var(key),
            }
        }
        Self { saved }
    }
}

impl Drop for PortEnv {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }
}

/// One layer's port leaves (`None` = not set by this layer).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Layer {
    min: Option<u16>,
    max: Option<u16>,
    max_offset: Option<u16>,
}

impl Layer {
    const fn yaml(min: u16, max: Option<u16>, max_offset: Option<u16>) -> Self {
        Self {
            min: Some(min),
            max,
            max_offset,
        }
    }
}

/// The intended effective range, computed independently of the kernel:
/// start from the built-in `5000..=7000`; each layer's `min` replaces the
/// current one; its explicit `max`, or else its `max_offset` added to the
/// `min` in effect at that layer, replaces the upper bound. `None` when the
/// result is not a valid range.
fn model(layers: &[Layer]) -> Option<(u16, u16)> {
    let (mut min, mut max) = (u32::from(DEFAULT_MIN_PORT), u32::from(DEFAULT_MAX_PORT));
    for layer in layers {
        if let Some(value) = layer.min {
            min = u32::from(value);
        }
        if let Some(value) = layer.max {
            max = u32::from(value);
        } else if let Some(offset) = layer.max_offset {
            max = min + u32::from(offset);
        }
    }
    (1 <= min && min <= max && max <= 65535)
        .then(|| (u16::try_from(min).unwrap(), u16::try_from(max).unwrap()))
}

fn yaml_ports(layer: Layer) -> String {
    let mut text = format!("ports:\n  min: {}\n", layer.min.unwrap());
    if let Some(max) = layer.max {
        text.push_str(&format!("  max: {max}\n"));
    }
    if let Some(offset) = layer.max_offset {
        text.push_str(&format!("  max_offset: {offset}\n"));
    }
    text
}

/// The command-line layer exactly as `trop reserve`/`trop scan` build it:
/// absent bounds are filled with defaults, and only typed leaves are
/// recorded.
fn cli_layer(min: Option<u16>, max: Option<u16>) -> (Config, Vec<ConfigField>) {
    let config = Config {
        ports: Some(PortConfig {
            min: min.unwrap_or(DEFAULT_MIN_PORT),
            max: max.or(Some(DEFAULT_MAX_PORT)),
            max_offset: None,
        }),
        ..Default::default()
    };
    let mut fields = Vec::new();
    if min.is_some() {
        fields.push(ConfigField::PortsMin);
    }
    if max.is_some() {
        fields.push(ConfigField::PortsMax);
        fields.push(ConfigField::PortsMaxOffset);
    }
    (config, fields)
}

struct Stack {
    project: Option<Layer>,
    local: Option<Layer>,
    env: Layer,
    cli: (Option<u16>, Option<u16>),
}

impl Stack {
    fn layers(&self) -> Vec<Layer> {
        let cli = Layer {
            min: self.cli.0,
            max: self.cli.1,
            max_offset: None,
        };
        self.project
            .into_iter()
            .chain(self.local)
            .chain([self.env, cli])
            .collect()
    }

    fn build(&self, dir: &Path) -> trop::Result<EffectiveConfig> {
        let data_dir = dir.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        for (name, layer) in [("trop.yaml", self.project), ("trop.local.yaml", self.local)] {
            let path = dir.join(name);
            match layer {
                Some(layer) => fs::write(&path, yaml_ports(layer)).unwrap(),
                None => {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        let _env = PortEnv::set(self.env.min, self.env.max, self.env.max_offset);
        let (cli, fields) = cli_layer(self.cli.0, self.cli.1);
        ConfigBuilder::new()
            .with_working_dir(dir)
            .with_data_dir(&data_dir)
            .with_cli_config_fields(cli, fields)
            .build_effective()
    }
}

fn resolved(effective: &EffectiveConfig) -> (u16, u16) {
    let range = port_range_from_config(effective.ports()).unwrap();
    (range.min().value(), range.max().value())
}

/// Every source, every combination: the effective range is exactly the
/// model's, it is stored in canonical form (one absolute `max`, no
/// `max_offset`), and a combination the model rejects fails validation with
/// the "max must be >= min" rule.
///
/// Regression: a higher layer that changed only `min` used to re-anchor a
/// lower layer's `max_offset` (`--min 6000` over `{min: 5000, max_offset: 5}`
/// allocated from `6000..=6005`).
#[test]
#[serial]
fn effective_ports_follow_the_layer_model() {
    let temp = TempDir::new().unwrap();
    let projects = [
        None,
        Some(Layer::yaml(5000, None, Some(5))),
        Some(Layer::yaml(5000, Some(5005), None)),
        Some(Layer::yaml(6000, None, Some(100))),
    ];
    let locals = [
        None,
        Some(Layer::yaml(5003, None, None)),
        Some(Layer::yaml(5000, Some(5100), None)),
    ];
    let envs = [
        Layer::default(),
        Layer {
            min: Some(5002),
            ..Layer::default()
        },
        Layer {
            min: Some(6010),
            ..Layer::default()
        },
        Layer {
            max_offset: Some(10),
            ..Layer::default()
        },
        Layer {
            min: Some(5001),
            max_offset: Some(3),
            ..Layer::default()
        },
        Layer {
            max: Some(5050),
            ..Layer::default()
        },
    ];
    let clis = [
        (None, None),
        (Some(5004), None),
        (None, Some(5200)),
        (Some(6000), None),
    ];

    let mut checked = 0;
    for project in projects {
        for local in locals {
            for env in envs {
                for cli in clis {
                    let stack = Stack {
                        project,
                        local,
                        env,
                        cli,
                    };
                    let expected = model(&stack.layers());
                    match (stack.build(temp.path()), expected) {
                        (Ok(effective), Some(range)) => {
                            let ports = effective.ports();
                            assert!(
                                ports.max.is_some() && ports.max_offset.is_none(),
                                "not canonical: {ports:?} for {:?}",
                                stack.layers()
                            );
                            assert_eq!(resolved(&effective), range, "{:?}", stack.layers());
                        }
                        (Err(Error::Validation { message, .. }), None) => {
                            assert_eq!(message, "max must be >= min", "{:?}", stack.layers());
                        }
                        (result, expected) => panic!(
                            "{:?}: expected {expected:?}, got {:?}",
                            stack.layers(),
                            result.map(|effective| effective.ports().clone())
                        ),
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 4 * 3 * 6 * 4);
}

/// Representation independence through the real sources: a project file's
/// `{min: m, max_offset: o}` and `{min: m, max: m + o}` give the same
/// effective section, provenance winners, and errors under every override.
#[test]
#[serial]
fn offset_and_max_forms_are_interchangeable() {
    let temp = TempDir::new().unwrap();
    let envs = [
        Layer::default(),
        Layer {
            min: Some(5003),
            ..Layer::default()
        },
        Layer {
            min: Some(6010),
            ..Layer::default()
        },
        Layer {
            max: Some(5004),
            ..Layer::default()
        },
    ];
    let clis = [(None, None), (Some(5001), None), (Some(6000), None)];
    for env in envs {
        for cli in clis {
            for local in [None, Some(Layer::yaml(4990, None, None))] {
                let build = |project| {
                    Stack {
                        project: Some(project),
                        local,
                        env,
                        cli,
                    }
                    .build(temp.path())
                };
                let offset_form = build(Layer::yaml(5000, None, Some(5)));
                let max_form = build(Layer::yaml(5000, Some(5005), None));
                match (offset_form, max_form) {
                    (Ok(offset_form), Ok(max_form)) => {
                        assert_eq!(offset_form.ports(), max_form.ports());
                        for field in [
                            ConfigField::PortsMin,
                            ConfigField::PortsMax,
                            ConfigField::PortsMaxOffset,
                        ] {
                            assert_eq!(
                                offset_form.provenance(field).map(|p| p.winner()),
                                max_form.provenance(field).map(|p| p.winner()),
                                "{field:?}"
                            );
                        }
                    }
                    (Err(offset_form), Err(max_form)) => {
                        assert_eq!(offset_form.to_string(), max_form.to_string());
                    }
                    (offset_form, max_form) => panic!(
                        "forms differ under env {env:?}, cli {cli:?}, local {local:?}: \
                         {:?} vs {:?}",
                        offset_form.map(|e| e.ports().clone()),
                        max_form.map(|e| e.ports().clone())
                    ),
                }
            }
        }
    }
}

/// `TROP_PORT_MIN` over a project `max_offset` keeps the project's absolute
/// upper bound, and provenance credits each leaf to the source that set it.
#[test]
#[serial]
fn min_override_keeps_lower_offset_bound_and_provenance() {
    let temp = TempDir::new().unwrap();
    let effective = Stack {
        project: Some(Layer::yaml(5000, None, Some(5))),
        local: None,
        env: Layer {
            min: Some(5002),
            ..Layer::default()
        },
        cli: (None, None),
    }
    .build(temp.path())
    .unwrap();

    assert_eq!(
        effective.ports(),
        &PortConfig {
            min: 5002,
            max: Some(5005),
            max_offset: None,
        }
    );
    let project = ConfigValueSource::File {
        kind: ConfigFileKind::Project,
        path: temp.path().canonicalize().unwrap().join("trop.yaml"),
    };
    let winner = |field| effective.provenance(field).unwrap().winner().clone();
    assert_eq!(
        winner(ConfigField::PortsMin),
        ConfigValueSource::Environment {
            variable: "TROP_PORT_MIN"
        }
    );
    assert_eq!(winner(ConfigField::PortsMax), project);
    assert_eq!(winner(ConfigField::PortsMaxOffset), project);
}

/// Regression: with both `TROP_PORT_MAX` and `TROP_PORT_MAX_OFFSET` set, the
/// offset used to win silently. The environment layer is now held to the
/// document rule and rejected with the same message as YAML.
#[test]
#[serial]
fn environment_rejects_both_upper_bounds() {
    let temp = TempDir::new().unwrap();
    let result = Stack {
        project: None,
        local: None,
        env: Layer {
            min: None,
            max: Some(6000),
            max_offset: Some(10),
        },
        cli: (None, None),
    }
    .build(temp.path());
    match result {
        Err(Error::Validation { field, message }) => {
            assert_eq!(field, "ports (TROP_PORT_MAX, TROP_PORT_MAX_OFFSET)");
            assert_eq!(message, "Cannot specify both max and max_offset");
        }
        other => panic!("expected a validation error, got {:?}", other.map(|_| ())),
    }

    // The same message a YAML document gets.
    let document = Config {
        ports: Some(PortConfig {
            min: 5000,
            max: Some(6000),
            max_offset: Some(10),
        }),
        ..Default::default()
    };
    match ConfigValidator::validate(&document, true) {
        Err(Error::Validation { message, .. }) => {
            assert_eq!(message, "Cannot specify both max and max_offset");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// The environment's own `max_offset` must land on a valid port above the
/// `min` in effect, with the messages validation has always used.
#[test]
#[serial]
fn environment_offset_is_checked_at_its_layer() {
    let temp = TempDir::new().unwrap();
    let build = |project, env| {
        Stack {
            project,
            local: None,
            env,
            cli: (None, None),
        }
        .build(temp.path())
    };

    let error = build(
        None,
        Layer {
            max_offset: Some(0),
            ..Layer::default()
        },
    )
    .err()
    .unwrap();
    assert_eq!(
        error.to_string(),
        "validation error for 'ports.max_offset (TROP_PORT_MAX_OFFSET)': max_offset must be > 0"
    );

    let error = build(
        Some(Layer::yaml(65000, Some(65100), None)),
        Layer {
            max_offset: Some(1000),
            ..Layer::default()
        },
    )
    .err()
    .unwrap();
    assert_eq!(
        error.to_string(),
        "validation error for 'ports.max_offset (TROP_PORT_MAX_OFFSET)': \
         Offset 1000 would overflow when added to min port 65000"
    );

    // The anchor is the environment's own min when it sets one.
    let effective = build(
        Some(Layer::yaml(65000, Some(65100), None)),
        Layer {
            min: Some(1000),
            max_offset: Some(1000),
            ..Layer::default()
        },
    )
    .unwrap();
    assert_eq!(resolved(&effective), (1000, 2000));

    // Largest offset that still fits, at the very top of the port domain.
    let effective = build(
        None,
        Layer {
            min: Some(65000),
            max_offset: Some(535),
            ..Layer::default()
        },
    )
    .unwrap();
    assert_eq!(resolved(&effective), (65000, 65535));
}

/// The command-line layer built from exactly the present fields and the
/// programmatic (merged) layer agree for every port shape.
#[test]
#[serial]
fn command_line_and_programmatic_layers_agree() {
    let _env = PortEnv::set(None, None, None);
    let shapes = [
        (6000, Some(6100), None),
        (6000, None, Some(25)),
        (4000, None, None),
        (6000, Some(5000), None),
        (6000, Some(6100), Some(5)),
        (6000, None, Some(0)),
        (65000, None, Some(1000)),
    ];
    for (min, max, max_offset) in shapes {
        let layer = Config {
            ports: Some(PortConfig {
                min,
                max,
                max_offset,
            }),
            ..Default::default()
        };
        let base = || ConfigBuilder::new().skip_files().skip_env();
        let as_cli = base().with_cli_config(layer.clone()).build_effective();
        let as_programmatic = base().with_config(layer.clone()).build_effective();
        match (as_cli, as_programmatic) {
            (Ok(cli), Ok(programmatic)) => {
                assert_eq!(cli.ports(), programmatic.ports(), "{layer:?}");
            }
            (Err(cli), Err(programmatic)) => {
                // Same rule and message; only the source annotation differs.
                let strip = |error: Error| match error {
                    Error::Validation { field, message } => {
                        (field.split(' ').next().unwrap().to_string(), message)
                    }
                    other => panic!("unexpected error {other:?}"),
                };
                assert_eq!(strip(cli), strip(programmatic), "{layer:?}");
            }
            (cli, programmatic) => panic!(
                "{layer:?}: {:?} vs {:?}",
                cli.map(|e| e.ports().clone()),
                programmatic.map(|e| e.ports().clone())
            ),
        }
    }
}

fn group_with_base(base: u16) -> ReservationGroup {
    let mut services = std::collections::HashMap::new();
    services.insert(
        "web".to_string(),
        ServiceDefinition {
            offset: Some(0),
            preferred: None,
            env: None,
        },
    );
    ReservationGroup {
        base: Some(base),
        services,
    }
}

/// Regression: `reservations.base` used to be checked against the range
/// only when `reserve-group` built its plan, so validation accepted base
/// 9000 with ports `5000..=5010`.
#[test]
#[serial]
fn validation_rejects_group_base_outside_the_range() {
    let document = |ports: Option<PortConfig>, base| Config {
        ports,
        reservations: Some(group_with_base(base)),
        ..Default::default()
    };
    let range = |min, max, max_offset| {
        Some(PortConfig {
            min,
            max,
            max_offset,
        })
    };

    for (ports, base, expected) in [
        (
            range(5000, Some(5010), None),
            9000,
            Some("Base port 9000 must be within port range 5000..5010"),
        ),
        (
            range(5000, None, Some(10)),
            4999,
            Some("Base port 4999 must be within port range 5000..5010"),
        ),
        (
            range(5000, None, Some(10)),
            5011,
            Some("Base port 5011 must be within port range 5000..5010"),
        ),
        (range(5000, None, Some(10)), 5000, None),
        (range(5000, None, Some(10)), 5010, None),
        (range(1, Some(65535), None), 65535, None),
        (range(1, Some(65535), None), 1, None),
        (range(65535, None, None), 65535, None),
        // A section that names only `min` admits no base below it; its upper
        // bound comes from another layer.
        (range(6000, None, None), 9000, None),
        (range(6000, None, None), 6000, None),
        (
            range(6000, None, None),
            5999,
            Some("Base port 5999 must not be below the minimum port 6000"),
        ),
        (
            range(65535, None, None),
            1,
            Some("Base port 1 must not be below the minimum port 65535"),
        ),
        // Without a `ports` section the range comes from other layers.
        (None, 9000, None),
    ] {
        let result = ConfigValidator::validate(&document(ports.clone(), base), true);
        match expected {
            Some(message) => match result {
                Err(Error::Validation {
                    field,
                    message: actual,
                }) => {
                    assert_eq!(field, "reservations.base");
                    assert_eq!(actual, message);
                }
                other => panic!("{ports:?} base {base}: expected an error, got {other:?}"),
            },
            None => assert!(result.is_ok(), "{ports:?} base {base}: {result:?}"),
        }
    }

    // An invalid section is reported by its own rule, not as a base error.
    for invalid in [
        range(6000, Some(5000), None),
        range(6000, None, Some(0)),
        range(0, None, None),
    ] {
        let error = ConfigValidator::validate(&document(invalid.clone(), 1), true).unwrap_err();
        assert!(
            matches!(&error, Error::Validation { field, .. } if !field.starts_with("reservations")),
            "{invalid:?}: {error:?}"
        );
    }

    // Layered over the configuration files, every section names a complete
    // range (here the built-in one), so a project file without `ports` is
    // checked too.
    let temp = TempDir::new().unwrap();
    let _env = PortEnv::set(None, None, None);
    fs::write(
        temp.path().join("trop.yaml"),
        "reservations:\n  base: 9000\n  services:\n    web:\n      offset: 0\n",
    )
    .unwrap();
    let error = ConfigBuilder::new()
        .with_working_dir(temp.path())
        .with_data_dir(temp.path().join("data"))
        .build_effective()
        .err()
        .unwrap();
    assert!(
        matches!(&error, Error::Validation { field, message }
            if field.starts_with("reservations.base")
                && message == "Base port 9000 must be within port range 5000..7000"),
        "{error:?}"
    );
}

/// `ConfigValidator::validate_port_config` before it delegated to the
/// verified kernel (reference model, including its unreachable
/// "Offset would create invalid max port" branch).
fn historical_port_validation(config: &PortConfig) -> Result<(), (String, String)> {
    let error = |field: &str, message: String| Err((field.to_string(), message));
    if config.min == 0 {
        return error("ports.min", format!("Invalid port number: {}", config.min));
    }
    if let Some(max) = config.max {
        if max == 0 {
            return error("ports.max", format!("Invalid port number: {max}"));
        }
        if max < config.min {
            return error("ports", "max must be >= min".into());
        }
    }
    if let Some(offset) = config.max_offset {
        if offset == 0 {
            return error("ports.max_offset", "max_offset must be > 0".into());
        }
        let Some(computed_max) = config.min.checked_add(offset) else {
            return error(
                "ports.max_offset",
                format!(
                    "Offset {} would overflow when added to min port {}",
                    offset, config.min
                ),
            );
        };
        if computed_max == 0 {
            return error(
                "ports.max_offset",
                format!("Offset would create invalid max port: {computed_max}"),
            );
        }
    }
    if config.max.is_some() && config.max_offset.is_some() {
        return error("ports", "Cannot specify both max and max_offset".into());
    }
    Ok(())
}

/// Differential test: the kernel-backed validator reports exactly the
/// historical (field, message) for every boundary triple.
#[test]
fn port_validation_matches_historical_validator() {
    const EDGES: [u16; 12] = [
        0, 1, 2, 534, 535, 536, 999, 5000, 64999, 65000, 65534, 65535,
    ];
    let options = || std::iter::once(None).chain(EDGES.into_iter().map(Some));
    let mut checked = 0;
    for min in EDGES {
        for max in options() {
            for max_offset in options() {
                let ports = PortConfig {
                    min,
                    max,
                    max_offset,
                };
                let config = Config {
                    ports: Some(ports.clone()),
                    ..Default::default()
                };
                let actual =
                    ConfigValidator::validate(&config, true).map_err(|error| match error {
                        Error::Validation { field, message } => (field, message),
                        other => panic!("{ports:?}: unexpected error {other:?}"),
                    });
                assert_eq!(actual, historical_port_validation(&ports), "{ports:?}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 12 * 13 * 13);
}

/// The built-in range is defined once and is valid.
#[test]
fn built_in_range_is_single_sourced_and_valid() {
    assert_eq!((DEFAULT_MIN_PORT, DEFAULT_MAX_PORT), (5000, 7000));
    let ports = PortConfig::default();
    assert_eq!(
        ports,
        PortConfig {
            min: DEFAULT_MIN_PORT,
            max: Some(DEFAULT_MAX_PORT),
            max_offset: None,
        }
    );
    let config = Config {
        ports: Some(ports.clone()),
        ..Default::default()
    };
    ConfigValidator::validate(&config, true).unwrap();
    let range = port_range_from_config(&ports).unwrap();
    assert_eq!((range.min().value(), range.max().value()), (5000, 7000));
}

/// Command-line bounds: each must be a valid port and `min <= max`, with the
/// messages the CLI prints.
#[test]
fn command_line_bounds_check() {
    let check = |min, max| {
        PortConfig::check_command_line_bounds(min, max).map_err(|error| match error {
            Error::Validation { message, .. } => message,
            other => panic!("unexpected error {other:?}"),
        })
    };
    for (min, max) in [
        (None, None),
        (Some(1), None),
        (None, Some(65535)),
        (Some(1), Some(1)),
        (Some(65535), Some(65535)),
        (Some(5000), Some(7000)),
    ] {
        assert_eq!(check(min, max), Ok(()), "{min:?} {max:?}");
    }
    let zero = "Invalid port number: port 0 is reserved and cannot be used";
    assert_eq!(
        check(Some(0), Some(0)),
        Err(format!("invalid --min value: {zero}"))
    );
    assert_eq!(
        check(Some(5), Some(0)),
        Err(format!("invalid --max value: {zero}"))
    );
    assert_eq!(
        check(None, Some(0)),
        Err(format!("invalid --max value: {zero}"))
    );
    assert_eq!(
        check(Some(6000), Some(5999)),
        Err("Invalid port range: min (6000) must be less than or equal to max (5999)".into())
    );
}

/// Writes `contents` to `name` in `dir`.
fn write(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).unwrap();
}

const GROUP_AT_6000: &str = "reservations:\n  base: 6000\n  services:\n    web:\n      offset: 0\n";

/// `reservations.base` is checked against the range the configuration files
/// name, and a violation names the file that last set the group or a port
/// bound. Environment and command-line port overrides apply to one
/// invocation and do not invalidate the group; `reserve-group` still checks
/// the base against the range it scans.
///
/// Regression (against this package's first version, which checked the base
/// after every override): a single `--min 6100 --max 6200`, or
/// `TROP_PORT_MIN=6500`, failed in any project whose group base was 6000,
/// and the error blamed `trop.yaml`.
#[test]
#[serial]
fn group_base_is_checked_against_the_configured_range() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path();
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let builder = || {
        ConfigBuilder::new()
            .with_working_dir(dir)
            .with_data_dir(&data_dir)
    };
    let base_error = |result: trop::Result<EffectiveConfig>| match result {
        Err(Error::Validation { field, message }) => (field, message),
        other => panic!("expected a validation error, got {:?}", other.map(|_| ())),
    };

    write(
        dir,
        "trop.yaml",
        &format!("ports:\n  min: 5000\n  max: 7000\n{GROUP_AT_6000}"),
    );
    let _env = PortEnv::set(Some(6500), None, None);
    let effective = builder().build_effective().unwrap();
    assert_eq!(resolved(&effective), (6500, 7000));
    drop(_env);

    let _env = PortEnv::set(None, None, None);
    let (cli, fields) = cli_layer(Some(6100), Some(6200));
    let effective = builder()
        .with_cli_config_fields(cli, fields)
        .build_effective()
        .unwrap();
    assert_eq!(resolved(&effective), (6100, 6200));

    // `reserve-group` scans the overridden range, which excludes the base.
    let options = trop::operations::ReserveGroupOptions::new(dir.join("trop.yaml"));
    let error = trop::operations::ReserveGroupPlan::from_effective(options, &effective)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "validation error for 'reservations.base': \
         Base port 6000 must be within port range 6100..6200"
    );

    // A higher file that moves the range is named, although the base is
    // declared by trop.yaml (which is valid on its own).
    write(dir, "trop.yaml", GROUP_AT_6000);
    write(dir, "trop.local.yaml", "ports:\n  min: 6500\n  max: 7000\n");
    let local = dir.join("trop.local.yaml");
    assert_eq!(
        base_error(builder().build_effective()),
        (
            format!("reservations.base ({})", local.display()),
            "Base port 6000 must be within port range 6500..7000".to_string()
        )
    );

    // The base inside the range the files name is accepted.
    write(dir, "trop.local.yaml", "ports:\n  min: 6000\n  max: 7000\n");
    builder().build_effective().unwrap();

    // A lower file that sets the range: the newer trop.yaml is named.
    fs::remove_file(&local).unwrap();
    write(
        &data_dir,
        "config.yaml",
        "ports:\n  min: 6500\n  max: 7000\n",
    );
    assert_eq!(
        base_error(builder().build_effective()),
        (
            format!("reservations.base ({})", dir.join("trop.yaml").display()),
            "Base port 6000 must be within port range 6500..7000".to_string()
        )
    );
}

/// A zero or overflowing `TROP_PORT_MAX_OFFSET` is rejected at the
/// environment layer, even when a command-line `--max` replaces the upper
/// bound. (Before, the command line silently rescued it.)
#[test]
#[serial]
fn environment_offset_is_not_rescued_by_the_command_line() {
    let temp = TempDir::new().unwrap();
    for (env, cli, expected) in [
        (
            Layer {
                max_offset: Some(0),
                ..Layer::default()
            },
            (None, Some(6500)),
            "validation error for 'ports.max_offset (TROP_PORT_MAX_OFFSET)': \
             max_offset must be > 0",
        ),
        (
            Layer {
                min: Some(65000),
                max_offset: Some(1000),
                ..Layer::default()
            },
            (Some(6000), Some(6000)),
            "validation error for 'ports.max_offset (TROP_PORT_MAX_OFFSET)': \
             Offset 1000 would overflow when added to min port 65000",
        ),
    ] {
        let error = Stack {
            project: None,
            local: None,
            env,
            cli,
        }
        .build(temp.path())
        .err()
        .unwrap();
        assert_eq!(error.to_string(), expected);
    }
}

/// The programmatic layer (`ConfigBuilder::with_config`) is validated on its
/// own, like a configuration file: a later layer can neither rescue an
/// invalid upper bound in it nor re-anchor an offset that never resolved.
#[test]
#[serial]
fn programmatic_layer_is_validated_on_its_own() {
    let _env = PortEnv::set(None, None, None);
    let build = |ports: PortConfig, cli: (Option<u16>, Option<u16>)| {
        let (cli, fields) = cli_layer(cli.0, cli.1);
        ConfigBuilder::new()
            .skip_files()
            .skip_env()
            .with_config(Config {
                ports: Some(ports),
                ..Default::default()
            })
            .with_cli_config_fields(cli, fields)
            .build_effective()
    };
    for (ports, cli, expected) in [
        (
            PortConfig {
                min: 5000,
                max: Some(5005),
                max_offset: Some(5),
            },
            (None, Some(6500)),
            "validation error for 'ports (<programmatic override>)': \
             Cannot specify both max and max_offset",
        ),
        (
            PortConfig {
                min: 65000,
                max: None,
                max_offset: Some(1000),
            },
            (Some(100), None),
            "validation error for 'ports.max_offset (<programmatic override>)': \
             Offset 1000 would overflow when added to min port 65000",
        ),
    ] {
        let error = build(ports, cli).err().unwrap();
        assert_eq!(error.to_string(), expected);
    }

    // A valid programmatic offset is fixed where it is declared.
    let effective = build(
        PortConfig {
            min: 5000,
            max: None,
            max_offset: Some(10),
        },
        (Some(5004), None),
    )
    .unwrap();
    assert_eq!(resolved(&effective), (5004, 5010));
}

/// `EffectiveConfig::apply_cli_config` commits only a valid result: after a
/// rejected layer the configuration (bounds and provenance) is unchanged.
#[test]
#[serial]
fn rejected_command_line_layer_leaves_the_configuration_unchanged() {
    let _env = PortEnv::set(None, None, None);
    let mut effective = ConfigBuilder::new()
        .skip_files()
        .skip_env()
        .with_config(Config {
            ports: Some(PortConfig {
                min: 5000,
                max: None,
                max_offset: Some(50),
            }),
            ..Default::default()
        })
        .build_effective()
        .unwrap();
    let before = effective.clone();
    assert_eq!(
        effective.ports(),
        &PortConfig {
            min: 5000,
            max: Some(5050),
            max_offset: None,
        }
    );

    let bad = Config {
        ports: Some(PortConfig {
            min: 6000,
            max: None,
            max_offset: Some(0),
        }),
        ..Default::default()
    };
    let error = effective
        .apply_cli_config(&bad, [ConfigField::PortsMaxOffset])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "validation error for 'ports.max_offset (<command line>)': max_offset must be > 0"
    );
    assert_eq!(effective, before);

    // A later valid layer applies over the unchanged configuration.
    let min_only = Config {
        ports: Some(PortConfig {
            min: 5040,
            max: None,
            max_offset: None,
        }),
        ..Default::default()
    };
    effective
        .apply_cli_config(&min_only, [ConfigField::PortsMin])
        .unwrap();
    assert_eq!(resolved(&effective), (5040, 5050));
}
