//! Output formatter implementations.

use std::collections::HashMap;

use crate::{Error, Port, Result};

use super::{OutputFormatter, ShellType};

/// Validates that a string is a valid environment variable name.
///
/// Valid names must:
/// - Start with a letter or underscore
/// - Contain only letters, digits, and underscores
fn is_valid_env_var_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap();

    // First character must be letter or underscore
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    // Remaining characters must be alphanumeric or underscore
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Converts a service tag to an environment variable name.
///
/// - Converts to uppercase
/// - Replaces hyphens with underscores
/// - Validates the result
fn tag_to_env_var(tag: &str) -> Result<String> {
    let var_name = tag.to_uppercase().replace('-', "_");

    if !is_valid_env_var_name(&var_name) {
        return Err(Error::Validation {
            field: "environment_variable".to_string(),
            message: format!(
                "invalid environment variable name '{var_name}': must contain only alphanumeric characters and underscores, and start with a letter or underscore"
            ),
        });
    }

    Ok(var_name)
}

/// Resolve environment variable name for a service tag using optional mappings.
fn resolve_env_var_name(
    tag: &str,
    env_mappings: Option<&HashMap<String, String>>,
) -> Result<String> {
    if let Some(mappings) = env_mappings {
        Ok(mappings
            .get(tag)
            .cloned()
            .unwrap_or_else(|| tag_to_env_var(tag).unwrap_or_else(|_| tag.to_uppercase())))
    } else {
        tag_to_env_var(tag)
    }
}

/// Formatter for shell-specific export statements.
pub struct ExportFormatter {
    shell: ShellType,
    env_mappings: Option<HashMap<String, String>>,
}

impl ExportFormatter {
    /// Create a new export formatter.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell type to format exports for
    /// * `env_mappings` - Optional mapping from service tags to environment variable names.
    ///   If None, tags are converted to uppercase.
    #[must_use]
    pub fn new(shell: ShellType, env_mappings: Option<HashMap<String, String>>) -> Self {
        Self {
            shell,
            env_mappings,
        }
    }
}

impl OutputFormatter for ExportFormatter {
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String> {
        let mut exports = Vec::new();

        // Sort by tag for consistent output
        let mut tags: Vec<_> = allocations.keys().collect();
        tags.sort();

        for tag in tags {
            let port = allocations[tag];

            // Resolve environment variable name
            let var_name = resolve_env_var_name(tag, self.env_mappings.as_ref())?;

            exports.push(
                self.shell
                    .format_export(&var_name, &port.value().to_string()),
            );
        }

        Ok(exports.join("\n"))
    }
}

/// Formatter for JSON output.
pub struct JsonFormatter;

impl OutputFormatter for JsonFormatter {
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String> {
        // Convert Port values to u16 for JSON
        let json_map: HashMap<String, u16> = allocations
            .iter()
            .map(|(k, v)| (k.clone(), v.value()))
            .collect();

        serde_json::to_string_pretty(&json_map).map_err(|e| Error::Validation {
            field: "json_output".to_string(),
            message: format!("failed to serialize to JSON: {e}"),
        })
    }
}

/// Formatter for dotenv (.env file) format.
pub struct DotenvFormatter {
    env_mappings: Option<HashMap<String, String>>,
}

impl DotenvFormatter {
    /// Create a new dotenv formatter.
    ///
    /// # Arguments
    ///
    /// * `env_mappings` - Optional mapping from service tags to environment variable names.
    ///   If None, tags are converted to uppercase.
    #[must_use]
    pub fn new(env_mappings: Option<HashMap<String, String>>) -> Self {
        Self { env_mappings }
    }
}

impl OutputFormatter for DotenvFormatter {
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String> {
        let mut lines = Vec::new();

        // Sort by tag for consistent output
        let mut tags: Vec<_> = allocations.keys().collect();
        tags.sort();

        for tag in tags {
            let port = allocations[tag];

            // Resolve environment variable name
            let var_name = resolve_env_var_name(tag, self.env_mappings.as_ref())?;

            lines.push(format!("{}={}", var_name, port.value()));
        }

        Ok(lines.join("\n"))
    }
}

/// Formatter for human-readable output.
pub struct HumanFormatter;

impl OutputFormatter for HumanFormatter {
    fn format(&self, allocations: &HashMap<String, Port>) -> Result<String> {
        if allocations.is_empty() {
            return Ok("No ports reserved.".to_string());
        }

        let mut lines = vec!["Reserved ports:".to_string()];

        // Sort by tag for consistent output
        let mut tags: Vec<_> = allocations.keys().collect();
        tags.sort();

        for tag in tags {
            let port = allocations[tag];
            lines.push(format!("  {}: {}", tag, port.value()));
        }

        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_allocations() -> HashMap<String, Port> {
        let mut map = HashMap::new();
        map.insert("web".to_string(), Port::try_from(5000).unwrap());
        map.insert("api".to_string(), Port::try_from(5001).unwrap());
        map
    }

    // ========================================================================
    // Environment Variable Name Validation Tests
    // ========================================================================

    #[test]
    fn test_is_valid_env_var_name() {
        // Valid names
        assert!(is_valid_env_var_name("PORT"));
        assert!(is_valid_env_var_name("WEB_PORT"));
        assert!(is_valid_env_var_name("_PRIVATE"));
        assert!(is_valid_env_var_name("PORT123"));
        assert!(is_valid_env_var_name("API_V2_PORT"));

        // Invalid names
        assert!(!is_valid_env_var_name(""));
        assert!(!is_valid_env_var_name("123PORT"));
        assert!(!is_valid_env_var_name("WEB-PORT"));
        assert!(!is_valid_env_var_name("WEB.PORT"));
        assert!(!is_valid_env_var_name("WEB PORT"));
    }

    /// Test validation of environment variable names with various edge cases.
    ///
    /// This test verifies the core invariant that environment variable names must:
    /// - Start with a letter or underscore (not a digit)
    /// - Contain only alphanumeric characters and underscores
    /// - Not be empty
    ///
    /// These rules are critical for ensuring generated shell exports are valid
    /// across different shell types (bash, zsh, fish, `PowerShell`).
    #[test]
    fn test_is_valid_env_var_name_edge_cases() {
        // Valid: single character names
        assert!(is_valid_env_var_name("A"));
        assert!(is_valid_env_var_name("_"));
        assert!(is_valid_env_var_name("z"));

        // Valid: names with numbers (not at start)
        assert!(is_valid_env_var_name("PORT1"));
        assert!(is_valid_env_var_name("API_V2"));
        assert!(is_valid_env_var_name("WEB_SERVER_123"));

        // Valid: all underscores
        assert!(is_valid_env_var_name("___"));
        assert!(is_valid_env_var_name("_A_B_C_"));

        // Invalid: empty string
        assert!(!is_valid_env_var_name(""));

        // Invalid: starts with digit
        assert!(!is_valid_env_var_name("1PORT"));
        assert!(!is_valid_env_var_name("9_API"));

        // Invalid: contains special characters
        assert!(!is_valid_env_var_name("WEB-PORT"));
        assert!(!is_valid_env_var_name("API.PORT"));
        assert!(!is_valid_env_var_name("PORT!"));
        assert!(!is_valid_env_var_name("PORT@HOME"));
        assert!(!is_valid_env_var_name("PORT#1"));
        assert!(!is_valid_env_var_name("PORT$"));
        assert!(!is_valid_env_var_name("PORT%"));
        assert!(!is_valid_env_var_name("PORT^"));
        assert!(!is_valid_env_var_name("PORT&"));
        assert!(!is_valid_env_var_name("PORT*"));

        // Invalid: contains whitespace
        assert!(!is_valid_env_var_name("WEB PORT"));
        assert!(!is_valid_env_var_name("PORT\t"));
        assert!(!is_valid_env_var_name("PORT\n"));
        assert!(!is_valid_env_var_name(" PORT"));
        assert!(!is_valid_env_var_name("PORT "));

        // Invalid: Unicode characters (non-ASCII)
        assert!(!is_valid_env_var_name("PORT_CAFÉ"));
        assert!(!is_valid_env_var_name("端口"));
        assert!(!is_valid_env_var_name("PÖRТ"));
    }

    #[test]
    fn test_tag_to_env_var() {
        assert_eq!(tag_to_env_var("web").unwrap(), "WEB");
        assert_eq!(tag_to_env_var("api-server").unwrap(), "API_SERVER");
        assert_eq!(tag_to_env_var("my-web-app").unwrap(), "MY_WEB_APP");
    }

    /// Test conversion of service tags to environment variable names.
    ///
    /// This test verifies the transformation rules:
    /// - Lowercase to uppercase conversion
    /// - Hyphen to underscore replacement
    /// - Validation of the resulting name
    ///
    /// This is important because service tags may use hyphens for readability
    /// (e.g., "web-server"), but environment variables must use underscores.
    #[test]
    fn test_tag_to_env_var_transformations() {
        // Simple cases
        assert_eq!(tag_to_env_var("web").unwrap(), "WEB");
        assert_eq!(tag_to_env_var("api").unwrap(), "API");
        assert_eq!(tag_to_env_var("db").unwrap(), "DB");

        // Hyphen replacement
        assert_eq!(tag_to_env_var("web-server").unwrap(), "WEB_SERVER");
        assert_eq!(tag_to_env_var("api-gateway").unwrap(), "API_GATEWAY");
        assert_eq!(tag_to_env_var("my-web-app").unwrap(), "MY_WEB_APP");

        // Multiple consecutive hyphens
        assert_eq!(tag_to_env_var("web--server").unwrap(), "WEB__SERVER");
        assert_eq!(tag_to_env_var("a---b").unwrap(), "A___B");

        // Tags with numbers
        assert_eq!(tag_to_env_var("web1").unwrap(), "WEB1");
        assert_eq!(tag_to_env_var("api-v2").unwrap(), "API_V2");
        assert_eq!(tag_to_env_var("web2-server").unwrap(), "WEB2_SERVER");

        // Leading/trailing hyphens become underscores
        assert_eq!(tag_to_env_var("-web").unwrap(), "_WEB");
        assert_eq!(tag_to_env_var("web-").unwrap(), "WEB_");
        assert_eq!(tag_to_env_var("-web-").unwrap(), "_WEB_");

        // Case preservation (lowercase to uppercase)
        assert_eq!(tag_to_env_var("WebServer").unwrap(), "WEBSERVER");
        assert_eq!(tag_to_env_var("APIGateway").unwrap(), "APIGATEWAY");
    }

    /// Test that invalid tag patterns result in validation errors.
    ///
    /// Even after transformation, some tags may produce invalid environment
    /// variable names. This test verifies that such cases are properly rejected.
    #[test]
    fn test_tag_to_env_var_invalid_tags() {
        // Tag that becomes empty after transformation (shouldn't happen in practice)
        // Note: our current implementation doesn't have this case, but it's good to document

        // Tag with invalid characters that survive transformation
        // Current implementation only replaces hyphens, so these should fail validation
        // if they contain other special characters

        // Tag that would start with a digit after transformation
        let result = tag_to_env_var("123-api");
        assert!(result.is_err(), "Tag starting with digit should be invalid");

        // Empty tag
        let result = tag_to_env_var("");
        assert!(result.is_err(), "Empty tag should be invalid");
    }

    // ========================================================================
    // Export Formatter Tests (Shell-Specific)
    // ========================================================================

    #[test]
    fn test_export_formatter_bash() {
        let allocations = create_test_allocations();
        let formatter = ExportFormatter::new(ShellType::Bash, None);
        let output = formatter.format(&allocations).unwrap();

        // Output should be sorted by tag
        assert!(output.contains("export API=5001"));
        assert!(output.contains("export WEB=5000"));
        assert!(output.starts_with("export API=5001\nexport WEB=5000"));
    }

    /// Test bash export formatting with comprehensive scenarios.
    ///
    /// Bash export format: `export VAR=value`
    /// Important properties:
    /// - No quotes around the value (for simple numeric values)
    /// - One export per line
    /// - Sorted alphabetically by variable name for deterministic output
    #[test]
    fn test_export_formatter_bash_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());
        allocations.insert("cache".to_string(), Port::try_from(6379).unwrap());

        let formatter = ExportFormatter::new(ShellType::Bash, None);
        let output = formatter.format(&allocations).unwrap();

        // Verify each export statement
        assert!(output.contains("export API=8081"));
        assert!(output.contains("export CACHE=6379"));
        assert!(output.contains("export DB=5432"));
        assert!(output.contains("export WEB=8080"));

        // Verify sorted order (alphabetical by variable name)
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "export API=8081");
        assert_eq!(lines[1], "export CACHE=6379");
        assert_eq!(lines[2], "export DB=5432");
        assert_eq!(lines[3], "export WEB=8080");
    }

    /// Test bash export with empty allocations.
    ///
    /// Empty allocations should produce empty output (no export statements).
    /// This is important for scripting scenarios where the output is eval'd.
    #[test]
    fn test_export_formatter_bash_empty() {
        let allocations = HashMap::new();
        let formatter = ExportFormatter::new(ShellType::Bash, None);
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "", "Empty allocations should produce empty output");
    }

    /// Test bash export with single allocation.
    ///
    /// Single allocation should produce a single export statement without
    /// trailing newline.
    #[test]
    fn test_export_formatter_bash_single() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let formatter = ExportFormatter::new(ShellType::Bash, None);
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "export WEB=8080");
    }

    #[test]
    fn test_export_formatter_fish() {
        let allocations = create_test_allocations();
        let formatter = ExportFormatter::new(ShellType::Fish, None);
        let output = formatter.format(&allocations).unwrap();

        assert!(output.contains("set -x API 5001"));
        assert!(output.contains("set -x WEB 5000"));
    }

    /// Test fish shell export formatting.
    ///
    /// Fish export format: `set -x VAR value`
    /// Fish uses a different syntax than POSIX shells:
    /// - `set -x` instead of `export`
    /// - Space between variable name and value (no `=`)
    /// - No quotes needed for numeric values
    #[test]
    fn test_export_formatter_fish_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());

        let formatter = ExportFormatter::new(ShellType::Fish, None);
        let output = formatter.format(&allocations).unwrap();

        // Verify fish-specific syntax
        assert!(output.contains("set -x API 8081"));
        assert!(output.contains("set -x DB 5432"));
        assert!(output.contains("set -x WEB 8080"));

        // Verify sorted order
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "set -x API 8081");
        assert_eq!(lines[1], "set -x DB 5432");
        assert_eq!(lines[2], "set -x WEB 8080");
    }

    #[test]
    fn test_export_formatter_powershell() {
        let allocations = create_test_allocations();
        let formatter = ExportFormatter::new(ShellType::PowerShell, None);
        let output = formatter.format(&allocations).unwrap();

        assert!(output.contains("$env:API=\"5001\""));
        assert!(output.contains("$env:WEB=\"5000\""));
    }

    /// Test `PowerShell` export formatting.
    ///
    /// `PowerShell` export format: `$env:VAR="value"`
    /// `PowerShell` uses a different syntax:
    /// - `$env:` prefix for environment variables
    /// - Always quotes the value (even for numbers)
    /// - Uses double quotes
    #[test]
    fn test_export_formatter_powershell_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());

        let formatter = ExportFormatter::new(ShellType::PowerShell, None);
        let output = formatter.format(&allocations).unwrap();

        // Verify PowerShell-specific syntax with quotes
        assert!(output.contains("$env:API=\"8081\""));
        assert!(output.contains("$env:DB=\"5432\""));
        assert!(output.contains("$env:WEB=\"8080\""));

        // Verify sorted order
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "$env:API=\"8081\"");
        assert_eq!(lines[1], "$env:DB=\"5432\"");
        assert_eq!(lines[2], "$env:WEB=\"8080\"");
    }

    /// Test zsh export formatting.
    ///
    /// Zsh uses the same syntax as bash for exports, but we test it separately
    /// to ensure the implementation correctly handles the Zsh variant.
    #[test]
    fn test_export_formatter_zsh() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());

        let formatter = ExportFormatter::new(ShellType::Zsh, None);
        let output = formatter.format(&allocations).unwrap();

        // Zsh uses same syntax as bash
        assert!(output.contains("export API=8081"));
        assert!(output.contains("export WEB=8080"));

        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_export_formatter_with_custom_mappings() {
        let allocations = create_test_allocations();
        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "WEB_SERVER_PORT".to_string());
        mappings.insert("api".to_string(), "API_PORT".to_string());

        let formatter = ExportFormatter::new(ShellType::Bash, Some(mappings));
        let output = formatter.format(&allocations).unwrap();

        assert!(output.contains("export API_PORT=5001"));
        assert!(output.contains("export WEB_SERVER_PORT=5000"));
    }

    /// Test custom environment variable mappings with various scenarios.
    ///
    /// Custom mappings allow users to specify explicit environment variable
    /// names for services, overriding the default tag-to-uppercase conversion.
    /// This is useful for:
    /// - Matching existing environment variable conventions
    /// - Avoiding naming conflicts
    /// - Using more descriptive names
    #[test]
    fn test_export_formatter_custom_mappings_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());

        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "FRONTEND_PORT".to_string());
        mappings.insert("api".to_string(), "BACKEND_API_PORT".to_string());
        // Note: 'db' is not mapped, should use default conversion

        let formatter = ExportFormatter::new(ShellType::Bash, Some(mappings));
        let output = formatter.format(&allocations).unwrap();

        // Verify custom mappings are used
        assert!(output.contains("export FRONTEND_PORT=8080"));
        assert!(output.contains("export BACKEND_API_PORT=8081"));

        // Verify unmapped service uses default conversion
        assert!(output.contains("export DB=5432"));

        // Verify sorted order (by resulting variable name, not tag)
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "export BACKEND_API_PORT=8081");
        assert_eq!(lines[1], "export DB=5432");
        assert_eq!(lines[2], "export FRONTEND_PORT=8080");
    }

    /// Test custom mappings with all shell types.
    ///
    /// Verify that custom mappings work correctly across different shell
    /// syntaxes (bash, fish, `PowerShell`).
    #[test]
    fn test_export_formatter_custom_mappings_all_shells() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "CUSTOM_WEB_PORT".to_string());

        // Test bash
        let bash_formatter = ExportFormatter::new(ShellType::Bash, Some(mappings.clone()));
        let bash_output = bash_formatter.format(&allocations).unwrap();
        assert_eq!(bash_output, "export CUSTOM_WEB_PORT=8080");

        // Test fish
        let fish_formatter = ExportFormatter::new(ShellType::Fish, Some(mappings.clone()));
        let fish_output = fish_formatter.format(&allocations).unwrap();
        assert_eq!(fish_output, "set -x CUSTOM_WEB_PORT 8080");

        // Test PowerShell
        let ps_formatter = ExportFormatter::new(ShellType::PowerShell, Some(mappings));
        let ps_output = ps_formatter.format(&allocations).unwrap();
        assert_eq!(ps_output, "$env:CUSTOM_WEB_PORT=\"8080\"");
    }

    // ========================================================================
    // JSON Formatter Tests
    // ========================================================================

    #[test]
    fn test_json_formatter() {
        let allocations = create_test_allocations();
        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        // Parse the JSON to verify it's valid
        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.get("web"), Some(&5000));
        assert_eq!(parsed.get("api"), Some(&5001));
    }

    /// Test JSON formatter with various allocation scenarios.
    ///
    /// JSON format provides machine-readable output that:
    /// - Uses service tags as keys (not transformed to env var names)
    /// - Uses numeric port values (not strings)
    /// - Is pretty-printed for readability
    /// - Can be parsed by any JSON-aware tool
    #[test]
    fn test_json_formatter_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());
        allocations.insert("cache".to_string(), Port::try_from(6379).unwrap());

        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        // Verify valid JSON
        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed.get("web"), Some(&8080));
        assert_eq!(parsed.get("api"), Some(&8081));
        assert_eq!(parsed.get("db"), Some(&5432));
        assert_eq!(parsed.get("cache"), Some(&6379));

        // Verify pretty-printed (contains newlines and indentation)
        assert!(output.contains('\n'), "JSON should be pretty-printed");
        assert!(output.contains("  "), "JSON should have indentation");
    }

    /// Test JSON formatter with empty allocations.
    ///
    /// Empty allocations should produce an empty JSON object: `{}`
    #[test]
    fn test_json_formatter_empty() {
        let allocations = HashMap::new();
        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.len(), 0);
        assert!(output.contains("{}"));
    }

    /// Test JSON formatter with single allocation.
    #[test]
    fn test_json_formatter_single() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("web"), Some(&8080));
    }

    /// Test JSON formatter preserves service tag format.
    ///
    /// Unlike export formats which transform tags to env var names,
    /// JSON should preserve the original service tag format, including
    /// hyphens and case.
    #[test]
    fn test_json_formatter_preserves_tag_format() {
        let mut allocations = HashMap::new();
        allocations.insert("web-server".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api-gateway".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("DB_primary".to_string(), Port::try_from(5432).unwrap());

        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.get("web-server"), Some(&8080));
        assert_eq!(parsed.get("api-gateway"), Some(&8081));
        assert_eq!(parsed.get("DB_primary"), Some(&5432));
    }

    /// Test JSON formatter with boundary port values.
    ///
    /// Verify that minimum and maximum valid port numbers are correctly
    /// serialized to JSON.
    #[test]
    fn test_json_formatter_boundary_ports() {
        let mut allocations = HashMap::new();
        allocations.insert("min".to_string(), Port::try_from(1).unwrap());
        allocations.insert("max".to_string(), Port::try_from(65535).unwrap());

        let formatter = JsonFormatter;
        let output = formatter.format(&allocations).unwrap();

        let parsed: HashMap<String, u16> = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed.get("min"), Some(&1));
        assert_eq!(parsed.get("max"), Some(&65535));
    }

    // ========================================================================
    // Dotenv Formatter Tests
    // ========================================================================

    #[test]
    fn test_dotenv_formatter() {
        let allocations = create_test_allocations();
        let formatter = DotenvFormatter::new(None);
        let output = formatter.format(&allocations).unwrap();

        // Output should be sorted by tag
        assert!(output.contains("API=5001"));
        assert!(output.contains("WEB=5000"));
        assert_eq!(output, "API=5001\nWEB=5000");
    }

    /// Test dotenv formatter with comprehensive scenarios.
    ///
    /// Dotenv format (.env file format):
    /// - Format: `VAR=value`
    /// - No `export` keyword (unlike bash)
    /// - No quotes around values (for simple numeric values)
    /// - One variable per line
    /// - Sorted alphabetically for deterministic output
    /// - Compatible with dotenv libraries in various languages
    #[test]
    fn test_dotenv_formatter_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());
        allocations.insert("cache".to_string(), Port::try_from(6379).unwrap());

        let formatter = DotenvFormatter::new(None);
        let output = formatter.format(&allocations).unwrap();

        // Verify each line
        assert!(output.contains("API=8081"));
        assert!(output.contains("CACHE=6379"));
        assert!(output.contains("DB=5432"));
        assert!(output.contains("WEB=8080"));

        // Verify sorted order and exact format
        assert_eq!(output, "API=8081\nCACHE=6379\nDB=5432\nWEB=8080");
    }

    /// Test dotenv formatter with empty allocations.
    ///
    /// Empty allocations should produce empty output (empty .env file).
    #[test]
    fn test_dotenv_formatter_empty() {
        let allocations = HashMap::new();
        let formatter = DotenvFormatter::new(None);
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "", "Empty allocations should produce empty output");
    }

    /// Test dotenv formatter with single allocation.
    #[test]
    fn test_dotenv_formatter_single() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let formatter = DotenvFormatter::new(None);
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "WEB=8080");
    }

    #[test]
    fn test_dotenv_formatter_with_custom_mappings() {
        let allocations = create_test_allocations();
        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "WEB_PORT".to_string());
        mappings.insert("api".to_string(), "API_PORT".to_string());

        let formatter = DotenvFormatter::new(Some(mappings));
        let output = formatter.format(&allocations).unwrap();

        assert!(output.contains("API_PORT=5001"));
        assert!(output.contains("WEB_PORT=5000"));
    }

    /// Test dotenv formatter with custom mappings.
    ///
    /// Custom mappings should work the same as with export formatter,
    /// allowing users to specify custom environment variable names.
    #[test]
    fn test_dotenv_formatter_custom_mappings_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());

        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "FRONTEND_PORT".to_string());
        mappings.insert("api".to_string(), "BACKEND_PORT".to_string());
        // 'db' not mapped, should use default

        let formatter = DotenvFormatter::new(Some(mappings));
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "BACKEND_PORT=8081\nDB=5432\nFRONTEND_PORT=8080");
    }

    // ========================================================================
    // Human Formatter Tests
    // ========================================================================

    #[test]
    fn test_human_formatter() {
        let allocations = create_test_allocations();
        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        assert!(output.contains("Reserved ports:"));
        assert!(output.contains("api: 5001"));
        assert!(output.contains("web: 5000"));
    }

    /// Test human-readable formatter with comprehensive scenarios.
    ///
    /// Human format is designed for direct display to users:
    /// - Header line: "Reserved ports:"
    /// - Each port on its own line with 2-space indentation
    /// - Format: "  tag: port"
    /// - Sorted alphabetically by tag
    /// - Uses original service tags (not transformed to env var names)
    #[test]
    fn test_human_formatter_comprehensive() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db".to_string(), Port::try_from(5432).unwrap());
        allocations.insert("cache".to_string(), Port::try_from(6379).unwrap());

        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        // Verify header
        assert!(output.starts_with("Reserved ports:"));

        // Verify each line has correct format
        assert!(output.contains("  api: 8081"));
        assert!(output.contains("  cache: 6379"));
        assert!(output.contains("  db: 5432"));
        assert!(output.contains("  web: 8080"));

        // Verify complete output with sorted order
        let expected = "Reserved ports:\n  api: 8081\n  cache: 6379\n  db: 5432\n  web: 8080";
        assert_eq!(output, expected);
    }

    #[test]
    fn test_human_formatter_empty() {
        let allocations = HashMap::new();
        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "No ports reserved.");
    }

    /// Test human formatter with single allocation.
    #[test]
    fn test_human_formatter_single() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        assert_eq!(output, "Reserved ports:\n  web: 8080");
    }

    /// Test human formatter preserves tag format (doesn't uppercase).
    ///
    /// Unlike export formats, human format should preserve the original
    /// service tag format for better readability.
    #[test]
    fn test_human_formatter_preserves_tag_format() {
        let mut allocations = HashMap::new();
        allocations.insert("web-server".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api-gateway".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("DB_primary".to_string(), Port::try_from(5432).unwrap());

        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        // Tags should be preserved as-is, not transformed
        assert!(output.contains("  web-server: 8080"));
        assert!(output.contains("  api-gateway: 8081"));
        assert!(output.contains("  DB_primary: 5432"));

        // Should be sorted alphabetically by original tag
        let expected =
            "Reserved ports:\n  DB_primary: 5432\n  api-gateway: 8081\n  web-server: 8080";
        assert_eq!(output, expected);
    }

    /// Test human formatter ignores custom mappings.
    ///
    /// Human format should always use the original service tags,
    /// regardless of custom environment variable mappings.
    #[test]
    fn test_human_formatter_ignores_mappings() {
        let mut allocations = HashMap::new();
        allocations.insert("web".to_string(), Port::try_from(8080).unwrap());

        let formatter = HumanFormatter;
        let output = formatter.format(&allocations).unwrap();

        // Should use original tag, not any custom mapping
        assert_eq!(output, "Reserved ports:\n  web: 8080");
    }

    // ========================================================================
    // Edge Case Tests
    // ========================================================================

    #[test]
    fn test_formatter_with_hyphenated_tags() {
        let mut allocations = HashMap::new();
        allocations.insert("web-server".to_string(), Port::try_from(8080).unwrap());

        let formatter = ExportFormatter::new(ShellType::Bash, None);
        let output = formatter.format(&allocations).unwrap();

        // Hyphens should be converted to underscores
        assert!(output.contains("export WEB_SERVER=8080"));
    }

    /// Test formatters with special characters in service tags.
    ///
    /// Service tags may contain hyphens, underscores, and mixed case.
    /// This test verifies that formatters handle these correctly:
    /// - Export formats: transform to valid env var names
    /// - JSON/Human formats: preserve original format
    #[test]
    fn test_formatters_special_characters_in_tags() {
        let mut allocations = HashMap::new();
        allocations.insert("web-server".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("api_gateway".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("db-primary".to_string(), Port::try_from(5432).unwrap());
        allocations.insert("Cache_01".to_string(), Port::try_from(6379).unwrap());

        // Test export formatter (bash)
        let export_formatter = ExportFormatter::new(ShellType::Bash, None);
        let export_output = export_formatter.format(&allocations).unwrap();
        assert!(export_output.contains("export WEB_SERVER=8080"));
        assert!(export_output.contains("export API_GATEWAY=8081"));
        assert!(export_output.contains("export DB_PRIMARY=5432"));
        assert!(export_output.contains("export CACHE_01=6379"));

        // Test dotenv formatter
        let dotenv_formatter = DotenvFormatter::new(None);
        let dotenv_output = dotenv_formatter.format(&allocations).unwrap();
        assert!(dotenv_output.contains("WEB_SERVER=8080"));
        assert!(dotenv_output.contains("API_GATEWAY=8081"));
        assert!(dotenv_output.contains("DB_PRIMARY=5432"));
        assert!(dotenv_output.contains("CACHE_01=6379"));

        // Test JSON formatter (preserves original format)
        let json_formatter = JsonFormatter;
        let json_output = json_formatter.format(&allocations).unwrap();
        let parsed: HashMap<String, u16> = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed.get("web-server"), Some(&8080));
        assert_eq!(parsed.get("api_gateway"), Some(&8081));
        assert_eq!(parsed.get("db-primary"), Some(&5432));
        assert_eq!(parsed.get("Cache_01"), Some(&6379));

        // Test human formatter (preserves original format)
        let human_formatter = HumanFormatter;
        let human_output = human_formatter.format(&allocations).unwrap();
        assert!(human_output.contains("  web-server: 8080"));
        assert!(human_output.contains("  api_gateway: 8081"));
        assert!(human_output.contains("  db-primary: 5432"));
        assert!(human_output.contains("  Cache_01: 6379"));
    }

    /// Test formatters with large number of allocations.
    ///
    /// Verify that formatters handle many allocations efficiently and
    /// maintain correct sorting and formatting.
    #[test]
    fn test_formatters_large_allocations() {
        let mut allocations = HashMap::new();

        // Create 50 allocations
        for i in 0..50 {
            let tag = format!("service_{i:02}");
            let port = 5000 + i;
            allocations.insert(tag, Port::try_from(port).unwrap());
        }

        // Test export formatter
        let export_formatter = ExportFormatter::new(ShellType::Bash, None);
        let export_output = export_formatter.format(&allocations).unwrap();
        let export_lines: Vec<&str> = export_output.lines().collect();
        assert_eq!(export_lines.len(), 50);

        // Verify first and last entries (sorted)
        assert!(export_lines[0].contains("SERVICE_00=5000"));
        assert!(export_lines[49].contains("SERVICE_49=5049"));

        // Verify sorted order
        for i in 0..49 {
            assert!(
                export_lines[i] < export_lines[i + 1],
                "Output should be sorted"
            );
        }

        // Test JSON formatter
        let json_formatter = JsonFormatter;
        let json_output = json_formatter.format(&allocations).unwrap();
        let parsed: HashMap<String, u16> = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed.len(), 50);

        // Test human formatter
        let human_formatter = HumanFormatter;
        let human_output = human_formatter.format(&allocations).unwrap();
        let human_lines: Vec<&str> = human_output.lines().collect();
        assert_eq!(human_lines.len(), 51); // 50 services + 1 header line
        assert_eq!(human_lines[0], "Reserved ports:");
    }

    /// Test formatters with boundary port values.
    ///
    /// Verify that minimum (1) and maximum (65535) valid port numbers
    /// are formatted correctly.
    #[test]
    fn test_formatters_boundary_ports() {
        let mut allocations = HashMap::new();
        allocations.insert("min_port".to_string(), Port::try_from(1).unwrap());
        allocations.insert("max_port".to_string(), Port::try_from(65535).unwrap());
        allocations.insert("common_port".to_string(), Port::try_from(8080).unwrap());

        // Test export formatter
        let export_formatter = ExportFormatter::new(ShellType::Bash, None);
        let export_output = export_formatter.format(&allocations).unwrap();
        assert!(export_output.contains("export MIN_PORT=1"));
        assert!(export_output.contains("export MAX_PORT=65535"));
        assert!(export_output.contains("export COMMON_PORT=8080"));

        // Test JSON formatter
        let json_formatter = JsonFormatter;
        let json_output = json_formatter.format(&allocations).unwrap();
        let parsed: HashMap<String, u16> = serde_json::from_str(&json_output).unwrap();
        assert_eq!(parsed.get("min_port"), Some(&1));
        assert_eq!(parsed.get("max_port"), Some(&65535));
        assert_eq!(parsed.get("common_port"), Some(&8080));

        // Test human formatter
        let human_formatter = HumanFormatter;
        let human_output = human_formatter.format(&allocations).unwrap();
        assert!(human_output.contains("  min_port: 1"));
        assert!(human_output.contains("  max_port: 65535"));
        assert!(human_output.contains("  common_port: 8080"));
    }

    /// Test deterministic ordering across multiple invocations.
    ///
    /// Verify that formatters produce consistent output when called
    /// multiple times with the same input (important for scripting and testing).
    #[test]
    fn test_formatters_deterministic_ordering() {
        let mut allocations = HashMap::new();
        allocations.insert("zebra".to_string(), Port::try_from(8080).unwrap());
        allocations.insert("apple".to_string(), Port::try_from(8081).unwrap());
        allocations.insert("mango".to_string(), Port::try_from(8082).unwrap());
        allocations.insert("banana".to_string(), Port::try_from(8083).unwrap());

        let formatter = ExportFormatter::new(ShellType::Bash, None);

        // Format multiple times
        let output1 = formatter.format(&allocations).unwrap();
        let output2 = formatter.format(&allocations).unwrap();
        let output3 = formatter.format(&allocations).unwrap();

        // All outputs should be identical
        assert_eq!(output1, output2);
        assert_eq!(output2, output3);

        // Verify alphabetical order
        let expected =
            "export APPLE=8081\nexport BANANA=8083\nexport MANGO=8082\nexport ZEBRA=8080";
        assert_eq!(output1, expected);
    }

    /// Test that `resolve_env_var_name` handles fallback correctly.
    ///
    /// When a custom mapping exists but a service is not mapped,
    /// the function should fall back to the default tag-to-env-var conversion.
    #[test]
    fn test_resolve_env_var_name_fallback() {
        let mut mappings = HashMap::new();
        mappings.insert("web".to_string(), "WEB_PORT".to_string());

        // Service with mapping
        let result = resolve_env_var_name("web", Some(&mappings)).unwrap();
        assert_eq!(result, "WEB_PORT");

        // Service without mapping (fallback to default)
        let result = resolve_env_var_name("api", Some(&mappings)).unwrap();
        assert_eq!(result, "API");

        // No mappings provided (use default)
        let result = resolve_env_var_name("db", None).unwrap();
        assert_eq!(result, "DB");
    }
}
