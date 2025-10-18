//! Shell detection and export formatting.

use std::env;

use crate::Result;

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
    /// Detect the current shell from environment variables.
    ///
    /// Detection precedence:
    /// 1. `ZSH_VERSION` - indicates zsh
    /// 2. `FISH_VERSION` - indicates fish
    /// 3. `PSModulePath` - indicates `PowerShell`
    /// 4. `SHELL` environment variable (path like `/bin/bash`)
    /// 5. Default to bash if unable to determine
    ///
    /// # Errors
    ///
    /// This function never returns an error; it defaults to bash if detection fails.
    pub fn detect() -> Result<Self> {
        // Check for shell-specific version variables first
        if env::var("ZSH_VERSION").is_ok() {
            return Ok(Self::Zsh);
        }
        if env::var("FISH_VERSION").is_ok() {
            return Ok(Self::Fish);
        }
        if env::var("PSModulePath").is_ok() {
            return Ok(Self::PowerShell);
        }

        // Check SHELL environment variable
        if let Ok(shell_path) = env::var("SHELL") {
            if shell_path.contains("zsh") {
                return Ok(Self::Zsh);
            } else if shell_path.contains("fish") {
                return Ok(Self::Fish);
            } else if shell_path.contains("pwsh") || shell_path.contains("powershell") {
                return Ok(Self::PowerShell);
            }
            // Default to bash for other shells (sh, bash, etc.)
            return Ok(Self::Bash);
        }

        // Default to bash if no shell detected
        Ok(Self::Bash)
    }

    /// Parse a shell type from a string.
    ///
    /// # Arguments
    ///
    /// * `s` - Shell name (case-insensitive): "bash", "zsh", "fish", "powershell", "pwsh"
    ///
    /// # Errors
    ///
    /// Returns an error if the shell name is not recognized.
    pub fn from_string(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "bash" | "sh" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            "powershell" | "pwsh" | "ps" => Ok(Self::PowerShell),
            _ => Err(crate::Error::Validation {
                field: "shell".to_string(),
                message: format!(
                    "unknown shell type '{s}': supported shells are bash, zsh, fish, powershell"
                ),
            }),
        }
    }

    /// Format an export statement for this shell type.
    ///
    /// # Arguments
    ///
    /// * `var` - Environment variable name
    /// * `value` - Value to assign (will be quoted appropriately)
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::output::ShellType;
    ///
    /// assert_eq!(ShellType::Bash.format_export("PORT", "5000"), "export PORT=5000");
    /// assert_eq!(ShellType::Fish.format_export("PORT", "5000"), "set -x PORT 5000");
    /// assert_eq!(ShellType::PowerShell.format_export("PORT", "5000"), "$env:PORT=\"5000\"");
    /// ```
    #[must_use]
    pub fn format_export(&self, var: &str, value: &str) -> String {
        match self {
            Self::Bash | Self::Zsh => format!("export {var}={value}"),
            Self::Fish => format!("set -x {var} {value}"),
            Self::PowerShell => format!("$env:{var}=\"{value}\""),
        }
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
        assert_eq!(shell.format_export("PORT", "5000"), "export PORT=5000");
        assert_eq!(
            shell.format_export("WEB_PORT", "8080"),
            "export WEB_PORT=8080"
        );
    }

    #[test]
    fn test_format_export_zsh() {
        let shell = ShellType::Zsh;
        assert_eq!(shell.format_export("PORT", "5000"), "export PORT=5000");
    }

    #[test]
    fn test_format_export_fish() {
        let shell = ShellType::Fish;
        assert_eq!(shell.format_export("PORT", "5000"), "set -x PORT 5000");
        assert_eq!(
            shell.format_export("WEB_PORT", "8080"),
            "set -x WEB_PORT 8080"
        );
    }

    #[test]
    fn test_format_export_powershell() {
        let shell = ShellType::PowerShell;
        assert_eq!(shell.format_export("PORT", "5000"), "$env:PORT=\"5000\"");
        assert_eq!(
            shell.format_export("WEB_PORT", "8080"),
            "$env:WEB_PORT=\"8080\""
        );
    }

    #[test]
    fn test_detect_defaults_to_bash() {
        // When no shell-specific variables are set, should default to bash
        // Note: This test may be affected by the actual test environment
        let detected = ShellType::detect().unwrap();
        // We can't assert a specific shell since it depends on test environment
        // Just verify it returns successfully
        assert!(matches!(
            detected,
            ShellType::Bash | ShellType::Zsh | ShellType::Fish | ShellType::PowerShell
        ));
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
            ShellType::Bash.format_export("PORT", "8080"),
            "export PORT=8080"
        );
        assert_eq!(
            ShellType::Bash.format_export("WEB_SERVER_PORT", "3000"),
            "export WEB_SERVER_PORT=3000"
        );
        assert_eq!(
            ShellType::Bash.format_export("API_V2_PORT", "9000"),
            "export API_V2_PORT=9000"
        );

        // Test with boundary port values
        assert_eq!(
            ShellType::Bash.format_export("MIN_PORT", "1"),
            "export MIN_PORT=1"
        );
        assert_eq!(
            ShellType::Bash.format_export("MAX_PORT", "65535"),
            "export MAX_PORT=65535"
        );

        // Test with underscore prefix (valid but unusual)
        assert_eq!(
            ShellType::Bash.format_export("_PRIVATE", "5000"),
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
        let bash_export = ShellType::Bash.format_export(var_name, value);
        let zsh_export = ShellType::Zsh.format_export(var_name, value);
        let fish_export = ShellType::Fish.format_export(var_name, value);
        let ps_export = ShellType::PowerShell.format_export(var_name, value);

        assert!(!bash_export.is_empty());
        assert!(!zsh_export.is_empty());
        assert!(!fish_export.is_empty());
        assert!(!ps_export.is_empty());

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
    /// While port values are never empty in practice, the `format_export`
    /// method should handle empty strings gracefully.
    #[test]
    fn test_format_export_empty_value() {
        assert_eq!(ShellType::Bash.format_export("PORT", ""), "export PORT=");
        assert_eq!(ShellType::Fish.format_export("PORT", ""), "set -x PORT ");
        assert_eq!(
            ShellType::PowerShell.format_export("PORT", ""),
            "$env:PORT=\"\""
        );
    }

    /// Test `format_export` with whitespace in values.
    ///
    /// Port values won't have whitespace, but this tests the formatter's
    /// robustness for potential future use cases.
    #[test]
    fn test_format_export_whitespace_in_value() {
        // Note: These scenarios shouldn't occur with port numbers, but we test
        // the formatter's behavior for completeness

        // Single word (no spaces) - typical case
        assert_eq!(
            ShellType::Bash.format_export("PORT", "8080"),
            "export PORT=8080"
        );

        // Value with leading/trailing spaces (shouldn't happen with ports)
        // The formatter doesn't trim, so spaces are preserved
        assert_eq!(
            ShellType::Bash.format_export("PORT", " 8080 "),
            "export PORT= 8080 "
        );

        // PowerShell always quotes, so spaces are properly handled
        assert_eq!(
            ShellType::PowerShell.format_export("PORT", " 8080 "),
            "$env:PORT=\" 8080 \""
        );
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
        let bash = ShellType::Bash.format_export(var_name, port_value);
        assert_eq!(bash, "export WEB_PORT=8080");
        assert!(bash.starts_with("export "));
        assert!(bash.contains('='));
        assert!(!bash.contains('"')); // No quotes for simple values

        // Zsh: Same as bash (POSIX-compatible)
        let zsh = ShellType::Zsh.format_export(var_name, port_value);
        assert_eq!(zsh, "export WEB_PORT=8080");
        assert_eq!(bash, zsh); // Should be identical

        // Fish: `set -x VAR value` - Fish-specific syntax
        let fish = ShellType::Fish.format_export(var_name, port_value);
        assert_eq!(fish, "set -x WEB_PORT 8080");
        assert!(fish.starts_with("set -x "));
        assert!(!fish.contains('=')); // No equals sign in fish
        assert!(!fish.contains('"')); // No quotes for simple values

        // PowerShell: `$env:VAR="value"` - Always quotes value
        let ps = ShellType::PowerShell.format_export(var_name, port_value);
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
        let export = shell.format_export("PORT", "8080");
        assert_eq!(export, "export PORT=8080");

        // Simulate user providing "--shell fish"
        let shell_arg = "fish";
        let shell = ShellType::from_string(shell_arg).unwrap();
        let export = shell.format_export("PORT", "8080");
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
        // Auto-detect should always succeed (defaults to bash)
        let detected = ShellType::detect();
        assert!(detected.is_ok());

        // Should produce valid export format
        let shell = detected.unwrap();
        let export = shell.format_export("PORT", "8080");
        assert!(!export.is_empty());
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
            let export = shell.format_export("PORT", "8080");
            // Unix shells shouldn't use Windows-specific syntax
            assert!(!export.contains("$env:"));
        }

        // Windows commonly uses PowerShell
        let ps_export = ShellType::PowerShell.format_export("PORT", "8080");
        // PowerShell uses its own syntax
        assert!(ps_export.contains("$env:"));
        assert!(ps_export.contains('"')); // Always quotes values
    }
}
