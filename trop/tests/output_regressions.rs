//! Regression tests for the output boundary (human output, shell exports,
//! and diagnostics that echo user input).
//!
//! Each test exercises the public library API and failed before the
//! verified output kernels (`verified::render`, `verified::shell`,
//! `verified::verbosity`) took over these decisions.

use std::collections::HashMap;

use trop::output::{DotenvFormatter, ExportFormatter, HumanFormatter, OutputFormatter, ShellType};
use trop::{LogLevel, Port};

const SHELLS: [ShellType; 4] = [
    ShellType::Bash,
    ShellType::Zsh,
    ShellType::Fish,
    ShellType::PowerShell,
];

/// A tag containing a newline used to forge a `service: 1: 5001` line in
/// human output.
#[test]
fn human_output_cannot_be_forged_by_a_tag_containing_a_newline() {
    let allocations = HashMap::from([
        (
            "evil\nservice: 1".to_string(),
            Port::try_from(5001).unwrap(),
        ),
        ("web".to_string(), Port::try_from(5000).unwrap()),
    ]);
    let output = HumanFormatter.format(&allocations).unwrap();
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        [
            "Reserved ports:",
            "  \"evil\\nservice: 1\": 5001",
            "  web: 5000"
        ]
    );
}

/// A tag carrying a terminal escape sequence is shown escaped.
#[test]
fn human_output_does_not_pass_terminal_escapes_through() {
    let allocations = HashMap::from([(
        "\u{1b}]0;owned\u{7}\u{9b}2J".to_string(),
        Port::try_from(5001).unwrap(),
    )]);
    let output = HumanFormatter.format(&allocations).unwrap();
    assert!(!output.chars().any(|c| c.is_control() && c != '\n'));
    assert_eq!(
        output,
        "Reserved ports:\n  \"\\x1b]0;owned\\x07\\x9b2J\": 5001"
    );
}

/// A tag carrying a line separator or a bidi override used to pass through:
/// `U+2028` splits the line for readers such as Python's `str.splitlines`,
/// and `U+202E` reorders the rest of the line in bidi-aware terminals.
#[test]
fn human_output_escapes_line_separators_and_bidi_controls() {
    let allocations = HashMap::from([
        (
            "evil\u{2028}service: 1".to_string(),
            Port::try_from(5001).unwrap(),
        ),
        ("web\u{202e}".to_string(), Port::try_from(5000).unwrap()),
        ("\u{2066}x\u{2069}".to_string(), Port::try_from(1).unwrap()),
        ("\u{61c}".to_string(), Port::try_from(65535).unwrap()),
    ]);
    let output = HumanFormatter.format(&allocations).unwrap();
    let is_line_break = |c: char| {
        matches!(
            c,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'
                ..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    };
    let lines: Vec<&str> = output.split(is_line_break).collect();
    assert_eq!(
        lines,
        [
            "Reserved ports:",
            "  \"evil\\u{2028}service: 1\": 5001",
            "  \"web\\u{202e}\": 5000",
            "  \"\\u{061c}\": 65535",
            "  \"\\u{2066}x\\u{2069}\": 1",
        ]
    );
}

/// `format_export` used to interpolate the value unquoted, so a value could
/// run arbitrary commands when the output was evaluated.
#[test]
fn format_export_rejects_values_that_would_inject_shell_code() {
    for shell in SHELLS {
        for value in [
            "1\"; Remove-Item -Recurse ~; \"",
            "1; rm -rf ~",
            "1\nrm -rf ~",
            "$(id)",
            "`id`",
            "1 2",
            "",
        ] {
            assert!(
                shell.format_export("PORT", value).is_err(),
                "{shell:?} accepted {value:?}"
            );
        }
        assert!(shell.format_export("PORT", "5000").is_ok());
    }
    assert_eq!(
        ShellType::PowerShell.format_export("PORT", "5000").unwrap(),
        "$env:PORT=\"5000\""
    );
}

/// Diagnostics that echo rejected input used to pass newlines and terminal
/// escapes through verbatim.
#[test]
fn unknown_shell_and_log_level_diagnostics_escape_control_characters() {
    let hostile = "zsh\nERROR: forged\u{1b}[2J";
    let shell_error = ShellType::from_string(hostile).unwrap_err().to_string();
    let level_error = LogLevel::parse(hostile).unwrap_err();
    for message in [shell_error, level_error] {
        assert!(!message.contains('\n'), "{message:?}");
        assert!(!message.contains('\u{1b}'), "{message:?}");
        assert!(
            message.contains("\"zsh\\nERROR: forged\\x1b[2J\""),
            "{message:?}"
        );
    }
}

/// Export and dotenv blocks have one line per service, in tag order, for
/// boundary ports.
#[test]
fn export_and_dotenv_blocks_have_one_statement_per_line() {
    let allocations = HashMap::from([
        ("b".to_string(), Port::try_from(65535).unwrap()),
        ("a".to_string(), Port::try_from(1).unwrap()),
        ("c-d".to_string(), Port::try_from(10).unwrap()),
    ]);
    assert_eq!(
        ExportFormatter::new(ShellType::Bash, None)
            .format(&allocations)
            .unwrap(),
        "export A=1\nexport B=65535\nexport C_D=10"
    );
    assert_eq!(
        ExportFormatter::new(ShellType::Fish, None)
            .format(&allocations)
            .unwrap(),
        "set -x A 1\nset -x B 65535\nset -x C_D 10"
    );
    assert_eq!(
        ExportFormatter::new(ShellType::PowerShell, None)
            .format(&allocations)
            .unwrap(),
        "$env:A=\"1\"\n$env:B=\"65535\"\n$env:C_D=\"10\""
    );
    assert_eq!(
        DotenvFormatter::new(None).format(&allocations).unwrap(),
        "A=1\nB=65535\nC_D=10"
    );
    assert_eq!(
        DotenvFormatter::new(None).format(&HashMap::new()).unwrap(),
        ""
    );
}
