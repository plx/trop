//! Integration tests for the `completions` command.
//!
//! These tests verify that completion scripts are generated for every
//! supported shell and include shell-specific content.

use assert_cmd::Command;

struct CompletionCase {
    shell: &'static str,
    stdout_fragment: &'static str,
}

fn run_completions(case: &CompletionCase) {
    let output = Command::cargo_bin("trop")
        .expect("Failed to find trop binary")
        .arg("completions")
        .arg(case.shell)
        .output()
        .expect("Failed to run completions command");

    assert!(
        output.status.success(),
        "completions {} failed: {}",
        case.shell,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("Invalid UTF-8 in stdout");
    assert!(
        !stdout.trim().is_empty(),
        "completions {} should output a script",
        case.shell
    );
    assert!(
        stdout.contains(case.stdout_fragment),
        "completions {} output missing shell fragment {:?}",
        case.shell,
        case.stdout_fragment
    );
    assert!(
        stdout.contains("reserve"),
        "completions {} output should include command names",
        case.shell
    );

    let stderr = String::from_utf8(output.stderr).expect("Invalid UTF-8 in stderr");
    assert!(
        stderr.contains(&format!("# Generating {} completion script", case.shell)),
        "completions {} stderr missing generation banner: {stderr}",
        case.shell
    );
    assert!(
        stderr.contains("Run the following command to enable completions"),
        "completions {} stderr should include usage guidance: {stderr}",
        case.shell
    );
}

#[test]
fn test_completions_supported_shells() {
    let cases = [
        CompletionCase {
            shell: "bash",
            stdout_fragment: "complete -F",
        },
        CompletionCase {
            shell: "zsh",
            stdout_fragment: "#compdef",
        },
        CompletionCase {
            shell: "fish",
            stdout_fragment: "complete -c",
        },
        CompletionCase {
            shell: "powershell",
            stdout_fragment: "Register-ArgumentCompleter -Native",
        },
        CompletionCase {
            shell: "elvish",
            stdout_fragment: "edit:completion:arg-completer",
        },
    ];

    for case in cases {
        run_completions(&case);
    }
}
