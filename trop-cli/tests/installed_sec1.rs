//! SEC-1 acceptance tests for an installed `trop` package artifact.
//!
//! The test in this file never evaluates, sources, or passes generated output
//! to a shell. Hostile strings remain inert configuration data, and all output
//! is inspected as captured bytes.

mod common;

use common::TestEnv;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

const DEFAULT_EXPECTED_VERSION: &str = "0.2.0";
const EXPECTED_NAMES: [&str; 2] = ["API_V2", "WEB_PORT"];
const MAPPED_HOSTILE_TAG: &str = "service\nexport FORGED=1\u{1b}[31m";

type CollisionCase<'a> = (&'a str, &'a [(&'a str, Option<&'a str>)]);

#[derive(Clone, Copy)]
enum GroupCommand {
    ReserveGroup,
    Autoreserve,
}

impl GroupCommand {
    const ALL: [Self; 2] = [Self::ReserveGroup, Self::Autoreserve];

    const fn name(self) -> &'static str {
        match self {
            Self::ReserveGroup => "reserve-group",
            Self::Autoreserve => "autoreserve",
        }
    }
}

#[derive(Clone, Copy)]
enum OutputSyntax {
    Posix,
    Fish,
    PowerShell,
    Dotenv,
}

#[derive(Clone, Copy)]
struct OutputBoundary {
    name: &'static str,
    format: &'static str,
    shell: Option<&'static str>,
    syntax: OutputSyntax,
}

const OUTPUT_BOUNDARIES: [OutputBoundary; 5] = [
    OutputBoundary {
        name: "bash",
        format: "export",
        shell: Some("bash"),
        syntax: OutputSyntax::Posix,
    },
    OutputBoundary {
        name: "zsh",
        format: "export",
        shell: Some("zsh"),
        syntax: OutputSyntax::Posix,
    },
    OutputBoundary {
        name: "fish",
        format: "export",
        shell: Some("fish"),
        syntax: OutputSyntax::Fish,
    },
    OutputBoundary {
        name: "powershell",
        format: "export",
        shell: Some("powershell"),
        syntax: OutputSyntax::PowerShell,
    },
    OutputBoundary {
        name: "dotenv",
        format: "dotenv",
        shell: None,
        syntax: OutputSyntax::Dotenv,
    },
];

fn installed_binary() -> PathBuf {
    let raw_path = env::var_os("TROP_SEC1_BINARY")
        .unwrap_or_else(|| panic!("TROP_SEC1_BINARY must name the installed trop binary"));
    let path = PathBuf::from(raw_path);

    assert!(
        path.is_absolute(),
        "TROP_SEC1_BINARY must be an absolute path, got {}",
        path.display()
    );
    assert!(
        path.is_file(),
        "TROP_SEC1_BINARY must name an existing file, got {}",
        path.display()
    );

    path
}

fn expected_version() -> String {
    env::var("TROP_SEC1_EXPECTED_VERSION").unwrap_or_else(|_| DEFAULT_EXPECTED_VERSION.to_owned())
}

fn escaped_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).escape_debug().to_string()
}

fn assert_installed_version(binary: &Path, expected_version: &str) {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .expect("Failed to query the installed trop binary");

    assert!(
        output.status.success(),
        "installed binary --version failed, stdout: {}, stderr: {}",
        escaped_bytes(&output.stdout),
        escaped_bytes(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .expect("installed binary version must be UTF-8")
            .trim(),
        format!("trop {expected_version}")
    );
    assert!(
        output.stderr.is_empty(),
        "installed binary --version wrote stderr: {}",
        escaped_bytes(&output.stderr)
    );
}

fn create_identifier_config(path: &Path, services: &[(&str, Option<&str>)]) {
    let services = services
        .iter()
        .enumerate()
        .map(|(offset, (tag, env_name))| {
            let mut definition = serde_json::Map::new();
            definition.insert("offset".to_owned(), serde_json::json!(offset));
            if let Some(env_name) = env_name {
                definition.insert("env".to_owned(), serde_json::json!(env_name));
            }
            ((*tag).to_owned(), serde_json::Value::Object(definition))
        })
        .collect::<serde_json::Map<_, _>>();

    let config = serde_json::json!({
        "project": "installed-sec1",
        "ports": {
            "min": 5000,
            "max": 9000
        },
        "reservations": {
            "base": 8200,
            "services": services
        }
    });

    fs::write(
        path,
        serde_json::to_vec_pretty(&config).expect("Failed to serialize test config"),
    )
    .expect("Failed to write test config");
}

fn run_group_command(
    binary: &Path,
    test_env: &TestEnv,
    project_dir: &Path,
    config_path: &Path,
    command_kind: GroupCommand,
    boundary: OutputBoundary,
) -> Output {
    let mut command = test_env.command_for_binary(binary);

    match command_kind {
        GroupCommand::ReserveGroup => {
            command.arg("reserve-group").arg(config_path);
        }
        GroupCommand::Autoreserve => {
            command.arg("autoreserve").current_dir(project_dir);
        }
    }

    command.arg("--format").arg(boundary.format);
    if let Some(shell) = boundary.shell {
        command.arg("--shell").arg(shell);
    }
    command
        .arg("--allow-unrelated-path")
        .output()
        .expect("Failed to run installed trop binary")
}

fn looks_like_dotenv_assignment(line: &str) -> bool {
    let Some((name, _)) = line.split_once('=') else {
        return false;
    };
    is_portable_identifier(name)
}

fn is_portable_identifier(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }

    let bytes = name.as_bytes();
    (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn assert_safe_stderr(stderr: &[u8], forbidden_reflections: &[&str], case_name: &str) {
    let stderr =
        std::str::from_utf8(stderr).unwrap_or_else(|_| panic!("{case_name} stderr must be UTF-8"));

    for character in stderr.chars() {
        assert!(
            character == '\n' || !character.is_control(),
            "{case_name} stderr contains control character {character:?}: {stderr:?}"
        );
    }
    for forbidden in forbidden_reflections {
        assert!(
            !stderr.contains(forbidden),
            "{case_name} stderr reflected untrusted input {forbidden:?}: {stderr:?}"
        );
        let escaped = forbidden.escape_debug().to_string();
        assert!(
            escaped == *forbidden || !stderr.contains(&escaped),
            "{case_name} stderr reflected escaped untrusted input {escaped:?}: {stderr:?}"
        );
    }
    for line in stderr.lines() {
        let line = line.trim_start();
        assert!(
            !line.starts_with("export ")
                && !line.starts_with("set -x ")
                && !line.starts_with("$env:")
                && !looks_like_dotenv_assignment(line),
            "{case_name} stderr contains an executable-looking line: {line:?}"
        );
    }
}

fn assert_closed_failure(
    test_env: &TestEnv,
    output: &Output,
    forbidden_reflections: &[&str],
    case_name: &str,
) {
    assert!(
        !output.status.success(),
        "{case_name} should fail, stdout: {}, stderr: {}",
        escaped_bytes(&output.stdout),
        escaped_bytes(&output.stderr)
    );
    assert_eq!(
        output.stdout, b"",
        "{case_name} must not emit generated output"
    );
    assert!(
        output.stderr.starts_with(b"Error: validation error"),
        "{case_name} should report a typed validation error: {}",
        escaped_bytes(&output.stderr)
    );
    assert_safe_stderr(&output.stderr, forbidden_reflections, case_name);
    assert_eq!(
        test_env.reservation_count(),
        0,
        "{case_name} must not persist a reservation"
    );
}

fn run_failure_case(
    binary: &Path,
    command_kind: GroupCommand,
    boundary: OutputBoundary,
    services: &[(&str, Option<&str>)],
    forbidden_reflections: &[&str],
    case_name: &str,
) {
    let test_env = TestEnv::new();
    let project_dir = test_env.create_dir("project");
    let config_path = project_dir.join("trop.yaml");
    create_identifier_config(&config_path, services);

    let output = run_group_command(
        binary,
        &test_env,
        &project_dir,
        &config_path,
        command_kind,
        boundary,
    );
    assert_closed_failure(
        &test_env,
        &output,
        forbidden_reflections,
        &format!(
            "{case_name} via {} at {} boundary",
            command_kind.name(),
            boundary.name
        ),
    );
}

fn parse_numeric_value(value: &str, case_name: &str) -> u16 {
    assert!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "{case_name} value must contain only decimal digits: {value:?}"
    );
    value
        .parse()
        .unwrap_or_else(|_| panic!("{case_name} value must be a valid port: {value:?}"))
}

fn parse_assignment(line: &str, boundary: OutputBoundary, case_name: &str) -> (String, u16) {
    let (name, value) = match boundary.syntax {
        OutputSyntax::Posix => line
            .strip_prefix("export ")
            .and_then(|assignment| assignment.split_once('='))
            .unwrap_or_else(|| {
                panic!(
                    "{case_name} must use exact {} export syntax: {line:?}",
                    boundary.name
                )
            }),
        OutputSyntax::Fish => {
            let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
            assert_eq!(
                fields.len(),
                4,
                "{case_name} must use exact fish export syntax: {line:?}"
            );
            assert_eq!(fields[0], "set", "{case_name} fish command");
            assert_eq!(fields[1], "-x", "{case_name} fish export flag");
            (fields[2], fields[3])
        }
        OutputSyntax::PowerShell => {
            let assignment = line.strip_prefix("$env:").unwrap_or_else(|| {
                panic!("{case_name} must use exact PowerShell syntax: {line:?}")
            });
            let (name, quoted_value) = assignment
                .split_once("=\"")
                .unwrap_or_else(|| panic!("{case_name} must quote its PowerShell value: {line:?}"));
            let value = quoted_value
                .strip_suffix('"')
                .unwrap_or_else(|| panic!("{case_name} must close its PowerShell value: {line:?}"));
            (name, value)
        }
        OutputSyntax::Dotenv => line.split_once('=').unwrap_or_else(|| {
            panic!("{case_name} must use exact dotenv assignment syntax: {line:?}")
        }),
    };

    assert!(
        is_portable_identifier(name),
        "{case_name} emitted a nonportable identifier: {name:?}"
    );
    (name.to_owned(), parse_numeric_value(value, case_name))
}

fn assert_success_output(
    test_env: &TestEnv,
    output: &Output,
    boundary: OutputBoundary,
    case_name: &str,
) {
    assert!(
        output.status.success(),
        "{case_name} should succeed, stdout: {}, stderr: {}",
        escaped_bytes(&output.stdout),
        escaped_bytes(&output.stderr)
    );
    let stdout =
        std::str::from_utf8(&output.stdout).expect("successful generated output must be UTF-8");
    assert!(
        !stdout.contains('\r') && !stdout.contains('\t') && !stdout.contains('\u{1b}'),
        "{case_name} stdout contains an unsafe control character: {stdout:?}"
    );

    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        2,
        "{case_name} must emit exactly two assignment lines: {stdout:?}"
    );
    let assignments = lines
        .into_iter()
        .map(|line| parse_assignment(line, boundary, case_name))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        assignments.len(),
        2,
        "{case_name} must emit exactly two unique assignments: {stdout:?}"
    );
    assert_eq!(
        assignments
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        EXPECTED_NAMES.into_iter().collect(),
        "{case_name} emitted unexpected identifiers: {stdout:?}"
    );
    assert!(
        assignments.values().all(|port| *port != 0),
        "{case_name} emitted an invalid zero port: {stdout:?}"
    );

    assert_safe_stderr(
        &output.stderr,
        &[MAPPED_HOSTILE_TAG, "FORGED", "\u{1b}"],
        case_name,
    );
    assert_eq!(
        test_env.reservation_count(),
        2,
        "{case_name} must persist exactly two reservations"
    );
}

fn verify_hostile_tags(binary: &Path) {
    let overlong = "a".repeat(256);
    let hostile_tags = [
        ("whitespace", "web port", None),
        ("tab", "web\tport", Some("\\t")),
        ("newline", "web\nexport FORGED=1", Some("FORGED")),
        ("CRLF", "web\r\nset -x FORGED 1", Some("FORGED")),
        ("shell metacharacters", "web;|&<>port", None),
        ("quotes and backticks", "web'\"`port", Some("`")),
        ("shell substitutions", "web$(NO_OP)${NO_OP}", Some("NO_OP")),
        ("leading digit", "9web", None),
        ("Unicode", "wéb端口", Some("端口")),
        ("overlong", overlong.as_str(), None),
    ];

    for (fixture_name, tag, marker) in hostile_tags {
        let mut forbidden = vec![tag];
        if let Some(marker) = marker {
            forbidden.push(marker);
        }
        for command_kind in GroupCommand::ALL {
            for boundary in OUTPUT_BOUNDARIES {
                run_failure_case(
                    binary,
                    command_kind,
                    boundary,
                    &[(tag, None)],
                    &forbidden,
                    fixture_name,
                );
            }
        }
    }
}

fn verify_invalid_mapping(binary: &Path) {
    for command_kind in GroupCommand::ALL {
        for boundary in OUTPUT_BOUNDARIES {
            run_failure_case(
                binary,
                command_kind,
                boundary,
                &[("web", Some("9WEB PORT"))],
                &["9WEB PORT"],
                "invalid explicit mapping",
            );
        }
    }
}

fn verify_collisions(binary: &Path) {
    let collision_cases: [CollisionCase<'_>; 3] = [
        (
            "derived/derived collision",
            &[("api-server", None), ("api_server", None)],
        ),
        (
            "explicit/derived collision",
            &[("api-server", None), ("mapped", Some("API_SERVER"))],
        ),
        (
            "ASCII-case-insensitive explicit collision",
            &[
                ("uppercase", Some("WEB_PORT")),
                ("lowercase", Some("web_port")),
            ],
        ),
    ];

    for (case_name, services) in collision_cases {
        let forbidden = services
            .iter()
            .flat_map(|(tag, env_name)| [Some(*tag), *env_name])
            .flatten()
            .collect::<Vec<_>>();
        for command_kind in GroupCommand::ALL {
            for boundary in OUTPUT_BOUNDARIES {
                run_failure_case(
                    binary,
                    command_kind,
                    boundary,
                    services,
                    &forbidden,
                    case_name,
                );
            }
        }
    }
}

fn verify_mixed_invalid_atomicity(binary: &Path) {
    const INVALID_TAG: &str = "zzz;printf-not-run";
    for command_kind in GroupCommand::ALL {
        for boundary in OUTPUT_BOUNDARIES {
            run_failure_case(
                binary,
                command_kind,
                boundary,
                &[("aaa-valid", None), (INVALID_TAG, None)],
                &[INVALID_TAG],
                "valid-first invalid-last group",
            );
        }
    }
}

fn verify_successes(binary: &Path) {
    for command_kind in GroupCommand::ALL {
        for boundary in OUTPUT_BOUNDARIES {
            let test_env = TestEnv::new();
            let project_dir = test_env.create_dir("project");
            let config_path = project_dir.join("trop.yaml");
            create_identifier_config(
                &config_path,
                &[("api-v2", None), (MAPPED_HOSTILE_TAG, Some("WEB_PORT"))],
            );

            let output = run_group_command(
                binary,
                &test_env,
                &project_dir,
                &config_path,
                command_kind,
                boundary,
            );
            assert_success_output(
                &test_env,
                &output,
                boundary,
                &format!(
                    "mapped and derived names via {} at {} boundary",
                    command_kind.name(),
                    boundary.name
                ),
            );
        }
    }
}

#[test]
#[ignore = "requires an installed binary selected by absolute TROP_SEC1_BINARY (expected version defaults to 0.2.0)"]
fn installed_binary_sec1_contract() {
    let binary = installed_binary();
    assert_installed_version(&binary, &expected_version());

    verify_hostile_tags(&binary);
    verify_invalid_mapping(&binary);
    verify_collisions(&binary);
    verify_mixed_invalid_atomicity(&binary);
    verify_successes(&binary);
}
