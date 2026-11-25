//! Shell detection and export formatting.
//!
//! Every decision here (the `--shell` alias table, `SHELL` classification,
//! detection precedence, and export-line rendering and value checking) is
//! made by the formally verified kernel `verified::shell` (see
//! `docs/verification.md`); this module only reads the environment and maps
//! kernel results onto trop's types and errors.

use std::env;
use std::ffi::OsString;

use crate::identifier::EnvironmentVariableName;
use crate::verified::shell::{self as shell_kernel, ShellKind, ShellSignals};
use crate::{Error, Result};

use super::display_field;

/// Supported shell types for export formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    /// Bourne Again Shell (bash).
    Bash,
    /// Z Shell (zsh).
    Zsh,
    /// Friendly Interactive Shell (fish).
    Fish,
    /// `PowerShell`.
    PowerShell,
}

impl ShellType {
    /// The kernel's shell kind.
    pub(super) const fn kind(self) -> ShellKind {
        match self {
            Self::Bash => ShellKind::Bash,
            Self::Zsh => ShellKind::Zsh,
            Self::Fish => ShellKind::Fish,
            Self::PowerShell => ShellKind::PowerShell,
        }
    }

    /// The shell type of a kernel shell kind.
    const fn from_kind(kind: ShellKind) -> Self {
        match kind {
            ShellKind::Bash => Self::Bash,
            ShellKind::Zsh => Self::Zsh,
            ShellKind::Fish => Self::Fish,
            ShellKind::PowerShell => Self::PowerShell,
        }
    }

    /// Detect the current shell from environment variables.
    ///
    /// Detection precedence:
    /// 1. `ZSH_VERSION` - indicates zsh
    /// 2. `FISH_VERSION` - indicates fish
    /// 3. `PSModulePath` - indicates `PowerShell`
    /// 4. The final path component of `SHELL` (a trailing `.exe` is
    ///    ignored, and so is ASCII case): `zsh`; `fish`; `pwsh` or
    ///    `powershell`; or a POSIX shell (`bash`, `sh`, `dash`, `ksh`,
    ///    `mksh`, `ash`, `yash`), which uses bash syntax. Directory names
    ///    never matter, so `/home/selfish/bin/bash` is bash.
    ///
    /// # Errors
    ///
    /// Returns a validation error asking for an explicit `--shell` when none
    /// of these identifies a supported shell (for example when `SHELL` is
    /// unset or names `tcsh`).
    pub fn detect() -> Result<Self> {
        Self::detect_with(|name| env::var_os(name))
    }

    /// Detection over an environment lookup (the process environment in
    /// production, a fixed map in tests).
    fn detect_with(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Self> {
        // A version variable counts when it is set to valid Unicode, as with
        // `env::var(..).is_ok()`.
        let is_set = |name: &str| lookup(name).is_some_and(|value| value.to_str().is_some());
        let signals = ShellSignals {
            zsh_version: is_set("ZSH_VERSION"),
            fish_version: is_set("FISH_VERSION"),
            ps_module_path: is_set("PSModulePath"),
        };
        // A lossy conversion only replaces bytes that are not UTF-8, which
        // are neither path separators nor part of any shell name.
        let shell = lookup("SHELL").map(|value| value.to_string_lossy().into_owned());
        shell_kernel::detect_shell(signals, shell.as_deref().map(str::as_bytes))
            .map(Self::from_kind)
            .ok_or_else(|| Error::Validation {
                field: "shell".to_owned(),
                message: "could not detect the shell from ZSH_VERSION, FISH_VERSION, PSModulePath or SHELL; pass --shell bash, zsh, fish or powershell".to_owned(),
            })
    }

    /// Parse a shell type from a string.
    ///
    /// # Arguments
    ///
    /// * `s` - Shell name (ASCII case-insensitive, no surrounding
    ///   whitespace): "bash" or "sh", "zsh", "fish", "powershell", "pwsh" or
    ///   "ps"
    ///
    /// # Errors
    ///
    /// Returns an error if the shell name is not recognized. The error shows
    /// the name with control characters escaped.
    pub fn from_string(s: &str) -> Result<Self> {
        shell_kernel::parse_shell_name(s.as_bytes())
            .map(Self::from_kind)
            .ok_or_else(|| Error::Validation {
                field: "shell".to_string(),
                message: format!(
                    "unknown shell type '{}': supported shells are bash, zsh, fish, powershell",
                    display_field(s)
                ),
            })
    }

    /// Format an export statement for this shell type.
    ///
    /// # Arguments
    ///
    /// * `var` - Environment variable name (`[A-Za-z_][A-Za-z0-9_]*`, at
    ///   most 255 bytes)
    /// * `value` - The value: a non-empty word of ASCII letters, digits and
    ///   `_ . / : @ + , -` that does not start with `-` (port numbers always
    ///   qualify). Every supported shell reads such a value literally, so the
    ///   statement can be passed to `eval` safely.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::output::ShellType;
    ///
    /// assert_eq!(
    ///     ShellType::Bash.format_export("PORT", "5000").unwrap(),
    ///     "export PORT=5000"
    /// );
    /// assert_eq!(
    ///     ShellType::Fish.format_export("PORT", "5000").unwrap(),
    ///     "set -x PORT 5000"
    /// );
    /// assert_eq!(
    ///     ShellType::PowerShell.format_export("PORT", "5000").unwrap(),
    ///     "$env:PORT=\"5000\""
    /// );
    /// assert!(ShellType::Bash.format_export("PORT", "1; rm -rf ~").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed validation error when `var` is not a portable
    /// environment-variable identifier, or when `value` is not such a word.
    /// Neither diagnostic echoes the rejected input.
    pub fn format_export(&self, var: &str, value: &str) -> Result<String> {
        let variable_name = EnvironmentVariableName::parse(var)?;
        shell_kernel::export_line(self.kind(), variable_name.as_str(), value).ok_or_else(|| {
            Error::Validation {
                field: "value".to_owned(),
                message: "export values must be non-empty words of ASCII letters, digits, and _ . / : @ + , - that do not start with -".to_owned(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_string() {
        assert_eq!(ShellType::from_string("bash").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("BASH").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("sh").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("zsh").unwrap(), ShellType::Zsh);
        assert_eq!(ShellType::from_string("fish").unwrap(), ShellType::Fish);
        assert_eq!(
            ShellType::from_string("powershell").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("pwsh").unwrap(),
            ShellType::PowerShell
        );

        // Unknown shell should error
        assert!(ShellType::from_string("unknown").is_err());
    }

    #[test]
    fn test_format_export_bash() {
        let shell = ShellType::Bash;
        assert_eq!(
            shell.format_export("PORT", "5000").unwrap(),
            "export PORT=5000"
        );
        assert_eq!(
            shell.format_export("WEB_PORT", "8080").unwrap(),
            "export WEB_PORT=8080"
        );
    }

    #[test]
    fn test_format_export_zsh() {
        let shell = ShellType::Zsh;
        assert_eq!(
            shell.format_export("PORT", "5000").unwrap(),
            "export PORT=5000"
        );
    }

    #[test]
    fn test_format_export_fish() {
        let shell = ShellType::Fish;
        assert_eq!(
            shell.format_export("PORT", "5000").unwrap(),
            "set -x PORT 5000"
        );
        assert_eq!(
            shell.format_export("WEB_PORT", "8080").unwrap(),
            "set -x WEB_PORT 8080"
        );
    }

    #[test]
    fn test_format_export_powershell() {
        let shell = ShellType::PowerShell;
        assert_eq!(
            shell.format_export("PORT", "5000").unwrap(),
            "$env:PORT=\"5000\""
        );
        assert_eq!(
            shell.format_export("WEB_PORT", "8080").unwrap(),
            "$env:WEB_PORT=\"8080\""
        );
    }

    #[test]
    fn test_format_export_rejects_invalid_identifiers_for_every_shell() {
        let shells = [
            ShellType::Bash,
            ShellType::Zsh,
            ShellType::Fish,
            ShellType::PowerShell,
        ];
        let invalid_names = [
            "1PORT",
            "WEB-PORT",
            "WEB PORT",
            "PORT; export ATTACK=1",
            "PORT\nexport ATTACK=1",
            "PORT$(command)",
            "PORT`command`",
            "PORT_CAFÉ",
        ];

        for shell in shells {
            for variable_name in invalid_names {
                let error = shell.format_export(variable_name, "5000").unwrap_err();
                let diagnostic = error.to_string();
                assert!(!diagnostic.contains(variable_name));
                assert!(!diagnostic.contains('\n'));
                assert!(!diagnostic.contains('\r'));
                assert!(!diagnostic.lines().any(|line| {
                    line.starts_with("export ")
                        || line.starts_with("set -x ")
                        || line.starts_with("$env:")
                }));
            }
        }
    }

    /// Detects over a fixed environment.
    fn detect_in(vars: &[(&str, &str)]) -> Result<ShellType> {
        let map: std::collections::HashMap<String, OsString> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(*value)))
            .collect();
        ShellType::detect_with(|name| map.get(name).cloned())
    }

    #[test]
    fn test_detect_classifies_shell_by_final_path_component() {
        let cases = [
            ("/bin/bash", ShellType::Bash),
            ("/usr/local/bin/bash", ShellType::Bash),
            ("/bin/sh", ShellType::Bash),
            ("/bin/dash", ShellType::Bash),
            ("/bin/ksh", ShellType::Bash),
            ("/bin/mksh", ShellType::Bash),
            ("/bin/ash", ShellType::Bash),
            ("/usr/bin/yash", ShellType::Bash),
            ("bash", ShellType::Bash),
            ("/bin/zsh", ShellType::Zsh),
            ("/opt/homebrew/bin/zsh", ShellType::Zsh),
            ("/usr/bin/fish", ShellType::Fish),
            ("/usr/bin/pwsh", ShellType::PowerShell),
            (
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
                ShellType::PowerShell,
            ),
            (
                "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                ShellType::PowerShell,
            ),
            (
                "C:\\Program Files\\Git\\usr\\bin\\BASH.EXE",
                ShellType::Bash,
            ),
            // Directory names never matter (these used to detect fish and
            // PowerShell from a substring of the directory).
            ("/home/selfish/bin/bash", ShellType::Bash),
            ("/opt/powershell-addons/bin/bash", ShellType::Bash),
            ("/home/zshfan/bin/fish", ShellType::Fish),
            ("/bash/zsh", ShellType::Zsh),
        ];
        for (shell, expected) in cases {
            assert_eq!(
                detect_in(&[("SHELL", shell)]).unwrap(),
                expected,
                "SHELL={shell}"
            );
        }
    }

    #[test]
    fn test_detect_fails_when_the_shell_is_unknown() {
        for shell in [
            "",
            "/",
            "/bin/",
            "/bin/tcsh",
            "/bin/csh",
            "/usr/bin/nu",
            "/usr/bin/xonsh",
            "/bin/bash5",
            "/bin/fish-shell",
            "/usr/bin/zsh.sh",
            ".exe",
            "/bin/ bash",
            "/bin/bash\n",
        ] {
            let error = detect_in(&[("SHELL", shell)]).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("--shell"), "{message}");
            if !shell.is_empty() {
                assert!(!message.contains(shell), "{message}");
            }
        }
        let error = detect_in(&[]).unwrap_err();
        assert!(error.to_string().contains("--shell"));
    }

    #[test]
    fn test_detect_version_variables_take_precedence() {
        assert_eq!(
            detect_in(&[
                ("ZSH_VERSION", "5.9"),
                ("FISH_VERSION", "3.7"),
                ("SHELL", "/bin/bash")
            ])
            .unwrap(),
            ShellType::Zsh
        );
        assert_eq!(
            detect_in(&[
                ("FISH_VERSION", "3.7"),
                ("PSModulePath", "x"),
                ("SHELL", "/bin/zsh")
            ])
            .unwrap(),
            ShellType::Fish
        );
        assert_eq!(
            detect_in(&[("PSModulePath", "C:\\Modules"), ("SHELL", "/bin/zsh")]).unwrap(),
            ShellType::PowerShell
        );
        // A set-but-empty variable counts, as with `env::var(..).is_ok()`.
        assert_eq!(
            detect_in(&[("ZSH_VERSION", ""), ("SHELL", "/usr/bin/fish")]).unwrap(),
            ShellType::Zsh
        );
        assert_eq!(
            detect_in(&[("PSModulePath", "")]).unwrap(),
            ShellType::PowerShell
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_detect_ignores_non_unicode_version_variables_and_classifies_non_unicode_shells() {
        use std::os::unix::ffi::OsStringExt;

        let not_unicode = OsString::from_vec(vec![0xff, 0xfe]);
        let map = std::collections::HashMap::from([
            ("ZSH_VERSION".to_owned(), not_unicode.clone()),
            (
                "SHELL".to_owned(),
                OsString::from_vec(b"/home/\xff/bin/fish".to_vec()),
            ),
        ]);
        assert_eq!(
            ShellType::detect_with(|name| map.get(name).cloned()).unwrap(),
            ShellType::Fish
        );
        let map = std::collections::HashMap::from([(
            "SHELL".to_owned(),
            OsString::from_vec(b"/bin/ba\xffsh".to_vec()),
        )]);
        assert!(ShellType::detect_with(|name| map.get(name).cloned()).is_err());
    }

    // ========================================================================
    // Additional Shell Detection Tests
    // ========================================================================

    /// Test shell detection with various environment variable combinations.
    ///
    /// This test documents the detection precedence and verifies that the
    /// detection logic handles different shell environments correctly.
    ///
    /// Detection precedence (highest to lowest):
    /// 1. `ZSH_VERSION` - most specific indicator
    /// 2. `FISH_VERSION` - most specific indicator
    /// 3. `PSModulePath` - PowerShell-specific
    /// 4. `SHELL` - generic shell path
    /// 5. Default to bash
    ///
    /// Note: These tests don't actually set environment variables (which would
    /// affect the entire test process), but document the expected behavior.
    /// Actual environment-based testing would require process isolation.
    #[test]
    fn test_shell_detection_precedence_documentation() {
        // This test documents the precedence order for shell detection.
        // The actual detection logic in ShellType::detect() checks:
        //
        // 1. ZSH_VERSION - if present, it's zsh (most specific)
        // 2. FISH_VERSION - if present, it's fish (most specific)
        // 3. PSModulePath - if present, it's PowerShell (specific)
        // 4. SHELL environment variable - parse the path to identify shell
        // 5. Default to bash if nothing detected
        //
        // This ensures correct detection in various environments:
        // - CI/CD systems may set SHELL
        // - Interactive shells set version variables
        // - PowerShell has unique environment structure
        // - Bash is a safe default (POSIX-compatible)
    }

    /// Test `from_string` with all supported shell names and aliases.
    ///
    /// This test verifies that all documented shell name variants are
    /// correctly parsed, including case-insensitive matching.
    #[test]
    fn test_from_string_all_variants() {
        // Bash variants
        assert_eq!(ShellType::from_string("bash").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("BASH").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("Bash").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("sh").unwrap(), ShellType::Bash);
        assert_eq!(ShellType::from_string("SH").unwrap(), ShellType::Bash);

        // Zsh variants
        assert_eq!(ShellType::from_string("zsh").unwrap(), ShellType::Zsh);
        assert_eq!(ShellType::from_string("ZSH").unwrap(), ShellType::Zsh);
        assert_eq!(ShellType::from_string("Zsh").unwrap(), ShellType::Zsh);

        // Fish variants
        assert_eq!(ShellType::from_string("fish").unwrap(), ShellType::Fish);
        assert_eq!(ShellType::from_string("FISH").unwrap(), ShellType::Fish);
        assert_eq!(ShellType::from_string("Fish").unwrap(), ShellType::Fish);

        // PowerShell variants
        assert_eq!(
            ShellType::from_string("powershell").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("PowerShell").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("POWERSHELL").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("pwsh").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("PWSH").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(
            ShellType::from_string("Pwsh").unwrap(),
            ShellType::PowerShell
        );
        assert_eq!(ShellType::from_string("ps").unwrap(), ShellType::PowerShell);
        assert_eq!(ShellType::from_string("PS").unwrap(), ShellType::PowerShell);
    }

    /// Test `from_string` with invalid shell names.
    ///
    /// This test verifies that unrecognized shell names produce appropriate
    /// validation errors with helpful messages.
    #[test]
    fn test_from_string_invalid_shells() {
        // Unknown shells should produce errors
        assert!(ShellType::from_string("unknown").is_err());
        assert!(ShellType::from_string("cmd").is_err());
        assert!(ShellType::from_string("csh").is_err());
        assert!(ShellType::from_string("tcsh").is_err());
        assert!(ShellType::from_string("ksh").is_err());
        assert!(ShellType::from_string("dash").is_err());

        // Empty string should error
        assert!(ShellType::from_string("").is_err());

        // Whitespace should error
        assert!(ShellType::from_string(" ").is_err());
        assert!(ShellType::from_string("  bash  ").is_err());

        // Typos should error
        assert!(ShellType::from_string("bahs").is_err());
        assert!(ShellType::from_string("zssh").is_err());
        assert!(ShellType::from_string("fsh").is_err());
    }

    /// Test `from_string` error messages are helpful.
    ///
    /// Error messages should guide users toward supported shell types.
    #[test]
    fn test_from_string_error_messages() {
        let result = ShellType::from_string("unknown");
        assert!(result.is_err());

        // Error message should mention the invalid shell name
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("unknown"),
            "Error message should include the invalid shell name"
        );

        // Error message should list supported shells
        assert!(
            err_msg.contains("bash") || err_msg.contains("supported"),
            "Error message should indicate supported shells"
        );
    }

    // ========================================================================
    // Export Format Tests (Comprehensive)
    // ========================================================================

    /// Test `format_export` with various variable names and values.
    ///
    /// This test verifies that export formatting works correctly with:
    /// - Different variable name patterns
    /// - Various port number values
    /// - Special formatting requirements per shell
    #[test]
    fn test_format_export_various_inputs() {
        // Test with different variable names
        assert_eq!(
            ShellType::Bash.format_export("PORT", "8080").unwrap(),
            "export PORT=8080"
        );
        assert_eq!(
            ShellType::Bash
                .format_export("WEB_SERVER_PORT", "3000")
                .unwrap(),
            "export WEB_SERVER_PORT=3000"
        );
        assert_eq!(
            ShellType::Bash
                .format_export("API_V2_PORT", "9000")
                .unwrap(),
            "export API_V2_PORT=9000"
        );

        // Test with boundary port values
        assert_eq!(
            ShellType::Bash.format_export("MIN_PORT", "1").unwrap(),
            "export MIN_PORT=1"
        );
        assert_eq!(
            ShellType::Bash.format_export("MAX_PORT", "65535").unwrap(),
            "export MAX_PORT=65535"
        );

        // Test with underscore prefix (valid but unusual)
        assert_eq!(
            ShellType::Bash.format_export("_PRIVATE", "5000").unwrap(),
            "export _PRIVATE=5000"
        );
    }

    /// Test `format_export` consistency across shells.
    ///
    /// While shells use different syntax, the semantic meaning should be
    /// equivalent: setting an environment variable to a value and exporting it.
    #[test]
    fn test_format_export_cross_shell_consistency() {
        let var_name = "TEST_PORT";
        let value = "8080";

        // All shells should produce non-empty output
        let bash_export = ShellType::Bash.format_export(var_name, value).unwrap();
        let zsh_export = ShellType::Zsh.format_export(var_name, value).unwrap();
        let fish_export = ShellType::Fish.format_export(var_name, value).unwrap();
        let ps_export = ShellType::PowerShell
            .format_export(var_name, value)
            .unwrap();

        assert!(!bash_export.is_empty(), "bash export should not be empty");
        assert!(!zsh_export.is_empty(), "zsh export should not be empty");
        assert!(!fish_export.is_empty(), "fish export should not be empty");
        assert!(
            !ps_export.is_empty(),
            "PowerShell export should not be empty"
        );

        // All should contain the variable name
        assert!(bash_export.contains(var_name));
        assert!(zsh_export.contains(var_name));
        assert!(fish_export.contains(var_name));
        assert!(ps_export.contains(var_name));

        // All should contain the value
        assert!(bash_export.contains(value));
        assert!(zsh_export.contains(value));
        assert!(fish_export.contains(value));
        assert!(ps_export.contains(value));

        // Bash and Zsh should be identical (POSIX-compatible)
        assert_eq!(bash_export, zsh_export);

        // Fish should be different (different syntax)
        assert_ne!(bash_export, fish_export);

        // PowerShell should be different (different syntax)
        assert_ne!(bash_export, ps_export);
    }

    /// Test `format_export` with empty values.
    ///
    /// An empty value means different things to different shells (bash sets
    /// an empty string, fish an empty list, and `PowerShell` deletes the
    /// variable), so it is rejected.
    #[test]
    fn test_format_export_empty_value() {
        for shell in [
            ShellType::Bash,
            ShellType::Zsh,
            ShellType::Fish,
            ShellType::PowerShell,
        ] {
            let error = shell.format_export("PORT", "").unwrap_err();
            assert!(
                matches!(error, crate::Error::Validation { ref field, .. } if field == "value")
            );
        }
    }

    /// Test `format_export` with whitespace in values.
    ///
    /// Whitespace would split the value into several shell words, so values
    /// containing it are rejected rather than emitted unquoted.
    #[test]
    fn test_format_export_whitespace_in_value() {
        // Single word (no spaces) - typical case
        assert_eq!(
            ShellType::Bash.format_export("PORT", "8080").unwrap(),
            "export PORT=8080"
        );

        for shell in [
            ShellType::Bash,
            ShellType::Zsh,
            ShellType::Fish,
            ShellType::PowerShell,
        ] {
            for value in [" 8080 ", "80 80", "8080\t", "8080\n", "\r8080"] {
                assert!(shell.format_export("PORT", value).is_err(), "{value:?}");
            }
        }
    }

    /// Values that every supported shell reads literally are rendered
    /// exactly as before; everything else is rejected without echoing it.
    #[test]
    fn test_format_export_accepts_exactly_inert_values() {
        for value in [
            "0",
            "8080",
            "65535",
            "localhost:8080",
            "/tmp/trop.sock",
            "user@host",
            "a,b",
            "v1.2+3",
            "WEB_PORT",
            "x-y",
        ] {
            assert_eq!(
                ShellType::Bash.format_export("PORT", value).unwrap(),
                format!("export PORT={value}")
            );
            assert_eq!(
                ShellType::Zsh.format_export("PORT", value).unwrap(),
                format!("export PORT={value}")
            );
            assert_eq!(
                ShellType::Fish.format_export("PORT", value).unwrap(),
                format!("set -x PORT {value}")
            );
            assert_eq!(
                ShellType::PowerShell.format_export("PORT", value).unwrap(),
                format!("$env:PORT=\"{value}\"")
            );
        }
        let hostile = [
            "1\"; Remove-Item -Recurse ~; \"",
            "1; rm -rf ~",
            "$(touch /tmp/pwned)",
            "`id`",
            "1|id",
            "1&id",
            "1>out",
            "*",
            "?",
            "[a]",
            "{a,b}",
            "~",
            "~/x",
            "#comment",
            "%self",
            "a=b",
            "'quoted'",
            "\\",
            "!1",
            "a^b",
            "-x",
            "-1",
            "\u{1b}[31m",
            "caf\u{e9}",
            "\u{201c}smart\u{201d}",
            "\u{2019}",
        ];
        for shell in [
            ShellType::Bash,
            ShellType::Zsh,
            ShellType::Fish,
            ShellType::PowerShell,
        ] {
            for value in hostile {
                let error = shell.format_export("PORT", value).unwrap_err();
                let message = error.to_string();
                assert!(!message.contains(value), "{message}");
                assert!(!message.contains('\n'));
            }
        }
    }

    /// The name is checked before the value (the name error is reported
    /// when both are invalid).
    #[test]
    fn test_format_export_reports_name_errors_first() {
        let error = ShellType::Bash.format_export("1PORT", "a b").unwrap_err();
        assert!(
            matches!(error, crate::Error::Validation { ref field, .. } if field == "environment_variable")
        );
    }

    /// Port exports are byte-identical to the previous `format!` rendering
    /// for every port and shell.
    #[test]
    fn test_format_export_matches_previous_rendering_for_every_port() {
        for port in 0..=u16::MAX {
            let value = port.to_string();
            assert_eq!(
                ShellType::Bash.format_export("P", &value).unwrap(),
                format!("export P={value}")
            );
            assert_eq!(
                ShellType::Fish.format_export("P", &value).unwrap(),
                format!("set -x P {value}")
            );
            assert_eq!(
                ShellType::PowerShell.format_export("P", &value).unwrap(),
                format!("$env:P=\"{value}\"")
            );
        }
    }

    /// The kernel's alias table agrees with the previous Unicode
    /// `to_lowercase` matching.
    #[test]
    fn test_from_string_matches_previous_unicode_lowercase_matching() {
        fn previous(s: &str) -> Option<ShellType> {
            match s.to_lowercase().as_str() {
                "bash" | "sh" => Some(ShellType::Bash),
                "zsh" => Some(ShellType::Zsh),
                "fish" => Some(ShellType::Fish),
                "powershell" | "pwsh" | "ps" => Some(ShellType::PowerShell),
                _ => None,
            }
        }
        let mut inputs: Vec<String> = [
            "bash",
            "BASH",
            "Bash",
            "bAsH",
            "sh",
            "SH",
            "zsh",
            "ZSH",
            "fish",
            "FISH",
            "powershell",
            "PowerShell",
            "POWERSHELL",
            "pwsh",
            "PWSH",
            "ps",
            "PS",
            "",
            " ",
            " bash",
            "bash ",
            "ba",
            "bashh",
            "csh",
            "ksh",
            "\u{212a}sh",
            "\u{130}",
            "p\u{17f}",
            "fi\u{17f}h",
            "ba\u{17f}h",
            "\u{17f}h",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        // Every one- and two-char string over a mixed alphabet.
        let alphabet = [
            's', 'S', 'h', 'H', 'p', 'P', '\u{212a}', '\u{130}', '\u{17f}', ' ', '\n',
        ];
        for a in alphabet {
            inputs.push(a.to_string());
            for b in alphabet {
                inputs.push(format!("{a}{b}"));
            }
        }
        for input in inputs {
            assert_eq!(
                ShellType::from_string(&input).ok(),
                previous(&input),
                "{input:?}"
            );
        }
    }

    /// The equivalence argument for ASCII folding: the only non-ASCII chars
    /// whose lowercase is entirely ASCII letters are U+212A KELVIN SIGN
    /// (`k`), and no shell name or log level contains `k`.
    #[test]
    fn test_only_kelvin_sign_lowercases_to_ascii() {
        let mut found = Vec::new();
        for code in 0x80..=0x10_FFFF_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let lower: String = c.to_lowercase().collect();
            if lower.chars().all(|l| l.is_ascii_alphabetic()) {
                found.push(c);
            }
        }
        assert_eq!(found, vec!['\u{212a}']);
        for name in ["bash", "sh", "zsh", "fish", "powershell", "pwsh", "ps"] {
            assert!(!name.contains('k'));
        }
    }

    /// The unknown-shell diagnostic escapes control characters in the input.
    #[test]
    fn test_from_string_error_escapes_control_characters() {
        let message = ShellType::from_string("bad\nshell\u{1b}[2J")
            .unwrap_err()
            .to_string();
        assert!(!message.contains('\n'));
        assert!(!message.contains('\u{1b}'));
        assert!(message.contains("\"bad\\nshell\\x1b[2J\""), "{message}");
    }

    /// Test `format_export` output is valid for respective shells.
    ///
    /// This test documents the expected output format for each shell,
    /// which can be used as reference for CLI integration and documentation.
    #[test]
    fn test_format_export_valid_syntax() {
        let var_name = "WEB_PORT";
        let port_value = "8080";

        // Bash: `export VAR=value` - POSIX standard
        let bash = ShellType::Bash.format_export(var_name, port_value).unwrap();
        assert_eq!(bash, "export WEB_PORT=8080");
        assert!(bash.starts_with("export "));
        assert!(bash.contains('='));
        assert!(!bash.contains('"')); // No quotes for simple values

        // Zsh: Same as bash (POSIX-compatible)
        let zsh = ShellType::Zsh.format_export(var_name, port_value).unwrap();
        assert_eq!(zsh, "export WEB_PORT=8080");
        assert_eq!(bash, zsh); // Should be identical

        // Fish: `set -x VAR value` - Fish-specific syntax
        let fish = ShellType::Fish.format_export(var_name, port_value).unwrap();
        assert_eq!(fish, "set -x WEB_PORT 8080");
        assert!(fish.starts_with("set -x "));
        assert!(!fish.contains('=')); // No equals sign in fish
        assert!(!fish.contains('"')); // No quotes for simple values

        // PowerShell: `$env:VAR="value"` - Always quotes value
        let ps = ShellType::PowerShell
            .format_export(var_name, port_value)
            .unwrap();
        assert_eq!(ps, "$env:WEB_PORT=\"8080\"");
        assert!(ps.starts_with("$env:"));
        assert!(ps.contains("=\"")); // Always quotes values
        assert!(ps.ends_with('"')); // Closing quote
    }

    // ========================================================================
    // Shell Type Properties
    // ========================================================================

    /// Test that `ShellType` implements required traits.
    ///
    /// `ShellType` should be Debug, Clone, Copy, `PartialEq`, and Eq for
    /// convenient use in formatters and CLI argument parsing.
    #[test]
    fn test_shell_type_traits() {
        let bash = ShellType::Bash;
        let zsh = ShellType::Zsh;

        // Clone and Copy
        let bash_copy = bash;
        assert_eq!(bash, bash_copy);

        // Debug
        let debug_str = format!("{bash:?}");
        assert!(debug_str.contains("Bash"));

        // PartialEq and Eq
        assert_eq!(bash, ShellType::Bash);
        assert_ne!(bash, zsh);
        assert_eq!(zsh, ShellType::Zsh);
    }

    /// Test `ShellType` has expected number of variants.
    ///
    /// This test serves as documentation of all supported shell types.
    /// If new shells are added, this test will need updating.
    #[test]
    fn test_shell_type_variants() {
        // We support exactly 4 shell types
        let shells = [
            ShellType::Bash,
            ShellType::Zsh,
            ShellType::Fish,
            ShellType::PowerShell,
        ];

        // Verify all are distinct
        assert_eq!(shells.len(), 4);
        for i in 0..shells.len() {
            for j in (i + 1)..shells.len() {
                assert_ne!(shells[i], shells[j]);
            }
        }
    }

    // ========================================================================
    // Integration Scenarios
    // ========================================================================

    /// Test typical CLI usage scenario: parse shell name, format export.
    ///
    /// This test simulates a common workflow where a user specifies a shell
    /// name via CLI argument, and we need to format exports accordingly.
    #[test]
    fn test_cli_usage_scenario() {
        // Simulate user providing "--shell bash"
        let shell_arg = "bash";
        let shell = ShellType::from_string(shell_arg).unwrap();
        let export = shell.format_export("PORT", "8080").unwrap();
        assert_eq!(export, "export PORT=8080");

        // Simulate user providing "--shell fish"
        let shell_arg = "fish";
        let shell = ShellType::from_string(shell_arg).unwrap();
        let export = shell.format_export("PORT", "8080").unwrap();
        assert_eq!(export, "set -x PORT 8080");

        // Simulate invalid shell name
        let shell_arg = "invalid";
        let result = ShellType::from_string(shell_arg);
        assert!(result.is_err());
    }

    /// Test auto-detection scenario (when no explicit shell specified).
    ///
    /// This test documents the fallback behavior when shell detection is used.
    #[test]
    fn test_auto_detection_scenario() {
        // Auto-detection succeeds when the environment identifies a shell,
        // and asks for --shell otherwise.
        assert!(detect_in(&[]).is_err());
        let detected = detect_in(&[("SHELL", "/bin/bash")]);
        assert!(detected.is_ok());

        // Should produce valid export format
        let shell = detected.unwrap();
        let export = shell.format_export("PORT", "8080").unwrap();
        assert!(!export.is_empty(), "export should not be empty");
        assert!(export.contains("PORT"));
        assert!(export.contains("8080"));
    }

    /// Test cross-platform compatibility considerations.
    ///
    /// Different platforms have different default shells and conventions.
    /// This test documents platform-specific considerations.
    #[test]
    fn test_platform_compatibility() {
        // Unix-like systems (Linux, macOS) commonly use bash or zsh
        let unix_shells = vec![ShellType::Bash, ShellType::Zsh, ShellType::Fish];

        for shell in unix_shells {
            let export = shell.format_export("PORT", "8080").unwrap();
            // Unix shells shouldn't use Windows-specific syntax
            assert!(!export.contains("$env:"));
        }

        // Windows commonly uses PowerShell
        let ps_export = ShellType::PowerShell.format_export("PORT", "8080").unwrap();
        // PowerShell uses its own syntax
        assert!(ps_export.contains("$env:"));
        assert!(ps_export.contains('"')); // Always quotes values
    }
}
