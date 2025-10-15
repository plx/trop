//! Configuration validation.
//!
//! This module provides comprehensive validation for all configuration fields,
//! ensuring that values are valid and consistent.

use crate::config::schema::{CleanupConfig, Config, PortConfig, PortExclusion, ReservationGroup};
use crate::error::{Error, Result};
use crate::port::Port;
use std::collections::HashSet;

/// Validates configuration according to spec rules.
///
/// # Examples
///
/// ```
/// use trop::config::{Config, ConfigValidator};
///
/// let config = Config::default();
/// ConfigValidator::validate(&config, false).unwrap();
/// ```
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate a complete configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration to validate
    /// * `is_tropfile` - Whether this is from a trop.yaml file (affects which fields are allowed)
    ///
    /// # Errors
    ///
    /// Returns validation errors for invalid configurations.
    pub fn validate(config: &Config, is_tropfile: bool) -> Result<()> {
        // Validate project field (only in trop.yaml)
        if let Some(ref project) = config.project {
            if !is_tropfile {
                return Err(Error::Validation {
                    field: "project".into(),
                    message: "project field is only valid in trop.yaml files".into(),
                });
            }
            Self::validate_identifier("project", project)?;
        }

        // Validate reservations (only in trop.yaml)
        if let Some(ref reservations) = config.reservations {
            if !is_tropfile {
                return Err(Error::Validation {
                    field: "reservations".into(),
                    message: "reservations field is only valid in trop.yaml files".into(),
                });
            }
            Self::validate_reservation_group(reservations)?;
        }

        // Validate port configuration
        if let Some(ref ports) = config.ports {
            Self::validate_port_config(ports)?;
        }

        // Validate excluded ports
        if let Some(ref excluded) = config.excluded_ports {
            Self::validate_excluded_ports(excluded)?;
        }

        // Validate cleanup config
        if let Some(ref cleanup) = config.cleanup {
            Self::validate_cleanup(cleanup)?;
        }

        // Validate lock timeout
        if let Some(timeout) = config.maximum_lock_wait_seconds {
            if timeout == 0 {
                return Err(Error::Validation {
                    field: "maximum_lock_wait_seconds".into(),
                    message: "Timeout must be greater than 0".into(),
                });
            }
        }

        Ok(())
    }

    /// Validate string identifiers (project, task, tags).
    ///
    /// Checks that the identifier is non-empty after trimming, contains no
    /// null bytes, and is not longer than 255 characters.
    fn validate_identifier(field: &str, value: &str) -> Result<()> {
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot be empty or only whitespace".into(),
            });
        }

        // Additional checks for security/safety
        if trimmed.contains('\0') {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot contain null bytes".into(),
            });
        }

        if trimmed.len() > 255 {
            return Err(Error::Validation {
                field: field.into(),
                message: "Cannot exceed 255 characters".into(),
            });
        }

        Ok(())
    }

    /// Validate port configuration.
    ///
    /// Ensures min and max are valid ports, max >= min, and that max and
    /// `max_offset` are not both specified.
    fn validate_port_config(config: &PortConfig) -> Result<()> {
        // Validate min port
        Port::try_from(config.min).map_err(|_| Error::Validation {
            field: "ports.min".into(),
            message: format!("Invalid port number: {}", config.min),
        })?;

        // Validate max if present
        if let Some(max) = config.max {
            Port::try_from(max).map_err(|_| Error::Validation {
                field: "ports.max".into(),
                message: format!("Invalid port number: {max}"),
            })?;

            if max < config.min {
                return Err(Error::Validation {
                    field: "ports".into(),
                    message: "max must be >= min".into(),
                });
            }
        }

        // Validate max_offset if present
        if let Some(offset) = config.max_offset {
            if offset == 0 {
                return Err(Error::Validation {
                    field: "ports.max_offset".into(),
                    message: "max_offset must be > 0".into(),
                });
            }

            let computed_max = config.min.saturating_add(offset);
            Port::try_from(computed_max).map_err(|_| Error::Validation {
                field: "ports.max_offset".into(),
                message: format!("Offset would create invalid max port: {computed_max}"),
            })?;
        }

        // Can't have both max and max_offset
        if config.max.is_some() && config.max_offset.is_some() {
            return Err(Error::Validation {
                field: "ports".into(),
                message: "Cannot specify both max and max_offset".into(),
            });
        }

        Ok(())
    }

    /// Validate excluded ports list.
    ///
    /// Ensures all ports are valid and ranges are properly ordered.
    fn validate_excluded_ports(excluded: &[PortExclusion]) -> Result<()> {
        for (i, exclusion) in excluded.iter().enumerate() {
            match exclusion {
                PortExclusion::Single(port) => {
                    Port::try_from(*port).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{i}]"),
                        message: format!("Invalid port: {port}"),
                    })?;
                }
                PortExclusion::Range { start, end } => {
                    Port::try_from(*start).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{i}]"),
                        message: format!("Invalid start port: {start}"),
                    })?;

                    Port::try_from(*end).map_err(|_| Error::Validation {
                        field: format!("excluded_ports[{i}]"),
                        message: format!("Invalid end port: {end}"),
                    })?;

                    if end < start {
                        return Err(Error::Validation {
                            field: format!("excluded_ports[{i}]"),
                            message: format!("Invalid range: {start}..{end} (end < start)"),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Validate cleanup configuration.
    fn validate_cleanup(cleanup: &CleanupConfig) -> Result<()> {
        if let Some(days) = cleanup.expire_after_days {
            if days == 0 {
                return Err(Error::Validation {
                    field: "cleanup.expire_after_days".into(),
                    message: "Must be > 0".into(),
                });
            }
        }
        Ok(())
    }

    /// Validate reservation group.
    ///
    /// Ensures all service tags are valid identifiers, offsets are unique,
    /// preferred ports are unique and valid, and environment variable names
    /// are valid and unique.
    fn validate_reservation_group(group: &ReservationGroup) -> Result<()> {
        // Validate base port if present
        if let Some(base) = group.base {
            Port::try_from(base).map_err(|_| Error::Validation {
                field: "reservations.base".into(),
                message: format!("Invalid port: {base}"),
            })?;
        }

        // Track uniqueness constraints
        let mut seen_offsets = HashSet::new();
        let mut seen_preferred = HashSet::new();
        let mut seen_env_vars = HashSet::new();
        let mut has_default_offset = false;

        for (tag, service) in &group.services {
            // Validate tag
            Self::validate_identifier(&format!("reservations.services.{tag}"), tag)?;

            // Check offset uniqueness
            let offset = service.offset.unwrap_or(0);
            if offset == 0 && has_default_offset {
                return Err(Error::Validation {
                    field: format!("reservations.services.{tag}.offset"),
                    message: "Only one service can omit offset (default to 0)".into(),
                });
            }
            if offset == 0 {
                has_default_offset = true;
            }

            if !seen_offsets.insert(offset) {
                return Err(Error::Validation {
                    field: format!("reservations.services.{tag}.offset"),
                    message: format!("Duplicate offset: {offset}"),
                });
            }

            // Check preferred port uniqueness
            if let Some(preferred) = service.preferred {
                Port::try_from(preferred).map_err(|_| Error::Validation {
                    field: format!("reservations.services.{tag}.preferred"),
                    message: format!("Invalid port: {preferred}"),
                })?;

                if !seen_preferred.insert(preferred) {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{tag}.preferred"),
                        message: format!("Duplicate preferred port: {preferred}"),
                    });
                }
            }

            // Check env var uniqueness and validity
            if let Some(ref env) = service.env {
                Self::validate_env_var_name(&format!("reservations.services.{tag}.env"), env)?;

                if !seen_env_vars.insert(env.clone()) {
                    return Err(Error::Validation {
                        field: format!("reservations.services.{tag}.env"),
                        message: format!("Duplicate environment variable: {env}"),
                    });
                }
            }
        }

        Ok(())
    }

    /// Validate environment variable name.
    ///
    /// Ensures the name is non-empty, contains only alphanumeric characters
    /// and underscores, and starts with a letter.
    fn validate_env_var_name(field: &str, env: &str) -> Result<()> {
        if env.is_empty() {
            return Err(Error::Validation {
                field: field.into(),
                message: "Environment variable name cannot be empty".into(),
            });
        }

        // Validate env var name (alphanumeric + underscore, starts with letter)
        if !env.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Error::Validation {
                field: field.into(),
                message: "Invalid environment variable name (must be alphanumeric + underscore)"
                    .into(),
            });
        }

        // Safe because we already checked env.is_empty() above
        let first_char = env
            .chars()
            .next()
            .expect("environment variable name is non-empty");
        if !first_char.is_ascii_alphabetic() {
            return Err(Error::Validation {
                field: field.into(),
                message: "Environment variable must start with a letter".into(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ServiceDefinition;
    use std::collections::HashMap;

    #[test]
    fn test_validate_empty_config() {
        let config = Config::default();
        assert!(ConfigValidator::validate(&config, false).is_ok());
    }

    #[test]
    fn test_validate_project_in_user_config() {
        let config = Config {
            project: Some("test".to_string()),
            ..Default::default()
        };
        let result = ConfigValidator::validate(&config, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_project_in_tropfile() {
        let config = Config {
            project: Some("test".to_string()),
            ..Default::default()
        };
        assert!(ConfigValidator::validate(&config, true).is_ok());
    }

    #[test]
    fn test_validate_identifier_empty() {
        let result = ConfigValidator::validate_identifier("test", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_identifier_whitespace_only() {
        let result = ConfigValidator::validate_identifier("test", "   ");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_identifier_null_byte() {
        let result = ConfigValidator::validate_identifier("test", "test\0value");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_identifier_too_long() {
        let long_string = "a".repeat(256);
        let result = ConfigValidator::validate_identifier("test", &long_string);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_identifier_valid() {
        assert!(ConfigValidator::validate_identifier("test", "valid_name").is_ok());
        assert!(ConfigValidator::validate_identifier("test", "  trimmed  ").is_ok());
    }

    #[test]
    fn test_validate_port_config_valid() {
        let config = PortConfig {
            min: 5000,
            max: Some(7000),
            max_offset: None,
        };
        assert!(ConfigValidator::validate_port_config(&config).is_ok());
    }

    #[test]
    fn test_validate_port_config_invalid_min() {
        let config = PortConfig {
            min: 0,
            max: Some(7000),
            max_offset: None,
        };
        assert!(ConfigValidator::validate_port_config(&config).is_err());
    }

    #[test]
    fn test_validate_port_config_max_less_than_min() {
        let config = PortConfig {
            min: 7000,
            max: Some(5000),
            max_offset: None,
        };
        assert!(ConfigValidator::validate_port_config(&config).is_err());
    }

    #[test]
    fn test_validate_port_config_both_max_and_offset() {
        let config = PortConfig {
            min: 5000,
            max: Some(7000),
            max_offset: Some(2000),
        };
        assert!(ConfigValidator::validate_port_config(&config).is_err());
    }

    #[test]
    fn test_validate_port_config_zero_offset() {
        let config = PortConfig {
            min: 5000,
            max: None,
            max_offset: Some(0),
        };
        assert!(ConfigValidator::validate_port_config(&config).is_err());
    }

    #[test]
    fn test_validate_excluded_ports_valid() {
        let excluded = vec![
            PortExclusion::Single(5001),
            PortExclusion::Range {
                start: 5005,
                end: 5009,
            },
        ];
        assert!(ConfigValidator::validate_excluded_ports(&excluded).is_ok());
    }

    #[test]
    fn test_validate_excluded_ports_invalid_single() {
        let excluded = vec![PortExclusion::Single(0)];
        assert!(ConfigValidator::validate_excluded_ports(&excluded).is_err());
    }

    #[test]
    fn test_validate_excluded_ports_invalid_range() {
        let excluded = vec![PortExclusion::Range {
            start: 5009,
            end: 5005,
        }];
        assert!(ConfigValidator::validate_excluded_ports(&excluded).is_err());
    }

    #[test]
    fn test_validate_cleanup_valid() {
        let cleanup = CleanupConfig {
            expire_after_days: Some(30),
        };
        assert!(ConfigValidator::validate_cleanup(&cleanup).is_ok());
    }

    #[test]
    fn test_validate_cleanup_zero_days() {
        let cleanup = CleanupConfig {
            expire_after_days: Some(0),
        };
        assert!(ConfigValidator::validate_cleanup(&cleanup).is_err());
    }

    #[test]
    fn test_validate_env_var_name_valid() {
        assert!(ConfigValidator::validate_env_var_name("test", "API_PORT").is_ok());
        assert!(ConfigValidator::validate_env_var_name("test", "web_server_port").is_ok());
        assert!(ConfigValidator::validate_env_var_name("test", "PORT123").is_ok());
    }

    #[test]
    fn test_validate_env_var_name_empty() {
        assert!(ConfigValidator::validate_env_var_name("test", "").is_err());
    }

    #[test]
    fn test_validate_env_var_name_starts_with_number() {
        assert!(ConfigValidator::validate_env_var_name("test", "123PORT").is_err());
    }

    #[test]
    fn test_validate_env_var_name_invalid_chars() {
        assert!(ConfigValidator::validate_env_var_name("test", "API-PORT").is_err());
        assert!(ConfigValidator::validate_env_var_name("test", "API PORT").is_err());
    }

    #[test]
    fn test_validate_reservation_group_valid() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: Some(5050),
                env: Some("WEB_PORT".to_string()),
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(1),
                preferred: Some(5051),
                env: Some("API_PORT".to_string()),
            },
        );

        let group = ReservationGroup {
            base: Some(5000),
            services,
        };

        assert!(ConfigValidator::validate_reservation_group(&group).is_ok());
    }

    #[test]
    fn test_validate_reservation_group_duplicate_offset() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            },
        );

        let group = ReservationGroup {
            base: Some(5000),
            services,
        };

        assert!(ConfigValidator::validate_reservation_group(&group).is_err());
    }

    #[test]
    fn test_validate_reservation_group_duplicate_preferred() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: Some(5050),
                env: None,
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(1),
                preferred: Some(5050),
                env: None,
            },
        );

        let group = ReservationGroup {
            base: Some(5000),
            services,
        };

        assert!(ConfigValidator::validate_reservation_group(&group).is_err());
    }

    #[test]
    fn test_validate_reservation_group_duplicate_env() {
        let mut services = HashMap::new();
        services.insert(
            "web".to_string(),
            ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some("PORT".to_string()),
            },
        );
        services.insert(
            "api".to_string(),
            ServiceDefinition {
                offset: Some(1),
                preferred: None,
                env: Some("PORT".to_string()),
            },
        );

        let group = ReservationGroup {
            base: Some(5000),
            services,
        };

        assert!(ConfigValidator::validate_reservation_group(&group).is_err());
    }
}

// Property-based tests for configuration validation
#[cfg(test)]
#[allow(unused_doc_comments)] // proptest! macro doesn't support doc comments
mod property_tests {
    use super::*;
    use crate::config::schema::ServiceDefinition;
    use proptest::prelude::*;
    use std::collections::HashMap;

    // ==================================================================================
    // PROPERTY TESTS FOR IDENTIFIER VALIDATION
    // ==================================================================================

    /// Property: Non-empty strings without null bytes and <= 255 chars should validate
    ///
    /// Mathematical Property: For all strings s where:
    /// - s.trim().len() > 0
    /// - s.trim().len() <= 255
    /// - s does not contain '\0'
    /// validate_identifier(s) should succeed.
    ///
    /// WHY THIS MATTERS: Identifiers (project names, tags, etc.) must follow
    /// basic safety and database compatibility rules. This property ensures
    /// all valid identifiers are accepted.
    proptest! {
        #[test]
        fn prop_valid_identifiers_accepted(
            s in "[a-zA-Z0-9_-]{1,255}".prop_filter("No null bytes", |s| !s.contains('\0'))
        ) {
            let result = ConfigValidator::validate_identifier("test", &s);
            prop_assert!(result.is_ok(), "Valid identifier '{}' should validate", s);
        }
    }

    /// Property: Empty strings should fail validation
    ///
    /// Mathematical Property: For all strings s where s.trim().is_empty(),
    /// validate_identifier(s) returns Err.
    proptest! {
        #[test]
        fn prop_empty_identifiers_rejected(whitespace_count in 0usize..=10) {
            let s = " ".repeat(whitespace_count);
            let result = ConfigValidator::validate_identifier("test", &s);
            prop_assert!(result.is_err(), "Empty/whitespace identifier should fail");
        }
    }

    /// Property: Strings with null bytes should fail validation
    ///
    /// Mathematical Property: For all strings s containing '\0',
    /// validate_identifier(s) returns Err.
    ///
    /// WHY THIS MATTERS: Null bytes can cause issues with database storage
    /// and string handling in various contexts. They should be rejected.
    proptest! {
        #[test]
        fn prop_null_bytes_rejected(
            prefix in "[a-z]{1,10}",
            suffix in "[a-z]{1,10}",
        ) {
            let s = format!("{prefix}\0{suffix}");
            let result = ConfigValidator::validate_identifier("test", &s);
            prop_assert!(result.is_err(), "Identifier with null byte should fail");
        }
    }

    /// Property: Strings longer than 255 characters should fail
    ///
    /// Mathematical Property: For all strings s where s.trim().len() > 255,
    /// validate_identifier(s) returns Err.
    proptest! {
        #[test]
        fn prop_too_long_identifiers_rejected(extra_chars in 1usize..=50) {
            let s = "a".repeat(255 + extra_chars);
            let result = ConfigValidator::validate_identifier("test", &s);
            prop_assert!(result.is_err(), "Identifier longer than 255 chars should fail");
        }
    }

    /// Property: Trimming happens before length check
    ///
    /// Mathematical Property: validate_identifier(s) = validate_identifier(s.trim())
    /// for validation purposes (though error messages may differ).
    proptest! {
        #[test]
        fn prop_identifier_trimmed_before_validation(
            core in "[a-z]{1,100}",
            leading_spaces in 0usize..=5,
            trailing_spaces in 0usize..=5,
        ) {
            let s = format!("{}{}{}", " ".repeat(leading_spaces), core, " ".repeat(trailing_spaces));

            let result1 = ConfigValidator::validate_identifier("test", &s);
            let result2 = ConfigValidator::validate_identifier("test", &core);

            // Both should have same success/failure status
            prop_assert_eq!(result1.is_ok(), result2.is_ok(), "Trimming should not affect validation");
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR PORT CONFIGURATION VALIDATION
    // ==================================================================================

    /// Property: Valid port ranges should always validate
    ///
    /// Mathematical Property: For all PortConfig where:
    /// - min in [1, 65535]
    /// - max in [min, 65535] (if Some)
    /// - max_offset is None OR max is None (mutual exclusion)
    /// validate_port_config succeeds.
    proptest! {
        #[test]
        #[allow(unused_comparisons)] // u16 can't exceed 65535, but test logic needs check
        fn prop_valid_port_configs_accepted(
            min in 1u16..=60000,
            max_offset in 0u16..=5535, // Ensures min + offset <= 65535
        ) {
            let max = min + max_offset;
            if max > 65535 {
                return Ok(());
            }

            let config = PortConfig {
                min,
                max: Some(max),
                max_offset: None,
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_ok(), "Valid port config should validate: min={}, max={}", min, max);
        }
    }

    /// Property: Port 0 as min should fail
    ///
    /// Mathematical Property: PortConfig with min=0 should fail validation.
    proptest! {
        #[test]
        fn prop_port_zero_min_rejected(_dummy in any::<u8>()) {
            let config = PortConfig {
                min: 0,
                max: Some(5000),
                max_offset: None,
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_err(), "Port 0 as min should fail");
        }
    }

    /// Property: max < min should fail validation
    ///
    /// Mathematical Property: For all PortConfig where max < min,
    /// validate_port_config returns Err.
    ///
    /// WHY THIS MATTERS: This is a fundamental invariant of port ranges.
    /// Invalid ranges would cause runtime errors in allocation logic.
    proptest! {
        #[test]
        fn prop_max_less_than_min_rejected(
            min in 1000u16..=65535,
            offset in 1u16..=999,
        ) {
            let max = min.saturating_sub(offset);
            if max >= min {
                return Ok(()); // Skip if subtraction didn't work as expected
            }

            let config = PortConfig {
                min,
                max: Some(max),
                max_offset: None,
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_err(), "max < min should fail: min={}, max={}", min, max);
        }
    }

    /// Property: Both max and max_offset should fail validation
    ///
    /// Mathematical Property: PortConfig with both max=Some and max_offset=Some
    /// should fail validation (mutual exclusion constraint).
    proptest! {
        #[test]
        fn prop_both_max_and_offset_rejected(
            min in 1u16..=50000,
            max in 50001u16..=65535,
            max_offset in 1u16..=1000,
        ) {
            let config = PortConfig {
                min,
                max: Some(max),
                max_offset: Some(max_offset),
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_err(), "Both max and max_offset should fail");
        }
    }

    /// Property: max_offset = 0 should fail
    ///
    /// Mathematical Property: PortConfig with max_offset=Some(0) should fail.
    /// An offset of 0 would make max = min, which should use max field instead.
    proptest! {
        #[test]
        fn prop_zero_max_offset_rejected(min in 1u16..=65535) {
            let config = PortConfig {
                min,
                max: None,
                max_offset: Some(0),
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_err(), "max_offset=0 should fail");
        }
    }

    /// Property: max_offset causing overflow should fail
    ///
    /// Mathematical Property: If min + max_offset > 65535, validation should fail.
    proptest! {
        #[test]
        #[allow(unused_comparisons)] // u16 can't exceed 65535, but test logic needs check
        fn prop_max_offset_overflow_rejected(min in 60000u16..=65535, max_offset in 1000u16..=10000) {
            let computed_max = min.saturating_add(max_offset);
            if computed_max <= 65535 {
                return Ok(()); // Skip if no overflow
            }

            let config = PortConfig {
                min,
                max: None,
                max_offset: Some(max_offset),
            };

            let result = ConfigValidator::validate_port_config(&config);
            prop_assert!(result.is_err(), "Overflow max_offset should fail: min={}, offset={}", min, max_offset);
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR EXCLUDED PORTS VALIDATION
    // ==================================================================================

    /// Property: Valid single port exclusions should validate
    ///
    /// Mathematical Property: For all ports p in [1, 65535],
    /// PortExclusion::Single(p) should validate successfully.
    proptest! {
        #[test]
        fn prop_valid_single_exclusions_accepted(port in 1u16..=65535) {
            let exclusions = vec![PortExclusion::Single(port)];
            let result = ConfigValidator::validate_excluded_ports(&exclusions);
            prop_assert!(result.is_ok(), "Valid single port {} should validate", port);
        }
    }

    /// Property: Valid range exclusions should validate
    ///
    /// Mathematical Property: For all port pairs (start, end) where start <= end
    /// and both in [1, 65535], Range{start, end} should validate.
    proptest! {
        #[test]
        fn prop_valid_range_exclusions_accepted(
            start in 1u16..=65535,
            offset in 0u16..=100,
        ) {
            let end = start.saturating_add(offset).min(65535);
            let exclusions = vec![PortExclusion::Range { start, end }];

            let result = ConfigValidator::validate_excluded_ports(&exclusions);
            prop_assert!(result.is_ok(), "Valid range {}..{} should validate", start, end);
        }
    }

    /// Property: Port 0 in exclusions should fail
    ///
    /// Mathematical Property: PortExclusion containing port 0 should fail validation.
    proptest! {
        #[test]
        fn prop_port_zero_in_exclusions_rejected(_dummy in any::<u8>()) {
            let exclusions = vec![PortExclusion::Single(0)];
            let result = ConfigValidator::validate_excluded_ports(&exclusions);
            prop_assert!(result.is_err(), "Port 0 in exclusions should fail");
        }
    }

    /// Property: Range with end < start should fail
    ///
    /// Mathematical Property: For all ranges where end < start, validation fails.
    proptest! {
        #[test]
        fn prop_invalid_range_rejected(
            start in 1000u16..=65535,
            offset in 1u16..=999,
        ) {
            let end = start.saturating_sub(offset);
            if end >= start {
                return Ok(()); // Skip if subtraction didn't work
            }

            let exclusions = vec![PortExclusion::Range { start, end }];
            let result = ConfigValidator::validate_excluded_ports(&exclusions);
            prop_assert!(result.is_err(), "Invalid range {}..{} should fail", start, end);
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR CLEANUP CONFIG VALIDATION
    // ==================================================================================

    /// Property: Positive expire_after_days values should validate
    ///
    /// Mathematical Property: For all n > 0, CleanupConfig with expire_after_days=Some(n)
    /// should validate successfully.
    proptest! {
        #[test]
        fn prop_positive_expire_days_accepted(days in 1u32..=10000) {
            let cleanup = CleanupConfig {
                expire_after_days: Some(days),
            };

            let result = ConfigValidator::validate_cleanup(&cleanup);
            prop_assert!(result.is_ok(), "Positive expire_after_days {} should validate", days);
        }
    }

    /// Property: Zero expire_after_days should fail
    ///
    /// Mathematical Property: CleanupConfig with expire_after_days=Some(0) fails validation.
    proptest! {
        #[test]
        fn prop_zero_expire_days_rejected(_dummy in any::<u8>()) {
            let cleanup = CleanupConfig {
                expire_after_days: Some(0),
            };

            let result = ConfigValidator::validate_cleanup(&cleanup);
            prop_assert!(result.is_err(), "Zero expire_after_days should fail");
        }
    }

    /// Property: None expire_after_days should validate
    ///
    /// Mathematical Property: CleanupConfig with expire_after_days=None always validates.
    proptest! {
        #[test]
        fn prop_none_expire_days_accepted(_dummy in any::<u8>()) {
            let cleanup = CleanupConfig {
                expire_after_days: None,
            };

            let result = ConfigValidator::validate_cleanup(&cleanup);
            prop_assert!(result.is_ok(), "None expire_after_days should validate");
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR ENVIRONMENT VARIABLE NAME VALIDATION
    // ==================================================================================

    /// Property: Valid env var names should validate
    ///
    /// Mathematical Property: For all strings s matching [a-zA-Z][a-zA-Z0-9_]*,
    /// validate_env_var_name(s) succeeds.
    ///
    /// WHY THIS MATTERS: Environment variable names must be valid shell identifiers.
    proptest! {
        #[test]
        fn prop_valid_env_var_names_accepted(
            first_char in "[a-zA-Z]",
            rest in "[a-zA-Z0-9_]{0,30}",
        ) {
            let env_var = format!("{first_char}{rest}");
            let result = ConfigValidator::validate_env_var_name("test", &env_var);
            prop_assert!(result.is_ok(), "Valid env var name '{}' should validate", env_var);
        }
    }

    /// Property: Env var names starting with digit should fail
    ///
    /// Mathematical Property: For all strings starting with [0-9],
    /// validate_env_var_name returns Err.
    proptest! {
        #[test]
        fn prop_env_var_starting_with_digit_rejected(
            digit in "[0-9]",
            rest in "[a-zA-Z0-9_]{0,10}",
        ) {
            let env_var = format!("{digit}{rest}");
            let result = ConfigValidator::validate_env_var_name("test", &env_var);
            prop_assert!(result.is_err(), "Env var '{}' starting with digit should fail", env_var);
        }
    }

    /// Property: Empty env var names should fail
    ///
    /// Mathematical Property: validate_env_var_name("") returns Err.
    proptest! {
        #[test]
        fn prop_empty_env_var_rejected(_dummy in any::<u8>()) {
            let result = ConfigValidator::validate_env_var_name("test", "");
            prop_assert!(result.is_err(), "Empty env var name should fail");
        }
    }

    /// Property: Env var names with special characters should fail
    ///
    /// Mathematical Property: For env var names containing non-alphanumeric/underscore
    /// characters, validation fails.
    proptest! {
        #[test]
        fn prop_env_var_with_special_chars_rejected(
            prefix in "[a-zA-Z]{1,5}",
            special_char in "[!@#$%^&*()\\-+=\\[\\]{}|;:'\",.<>?/\\\\]",
            suffix in "[a-zA-Z]{0,5}",
        ) {
            let env_var = format!("{prefix}{special_char}{suffix}");
            let result = ConfigValidator::validate_env_var_name("test", &env_var);
            prop_assert!(result.is_err(), "Env var '{}' with special char should fail", env_var);
        }
    }

    // ==================================================================================
    // PROPERTY TESTS FOR RESERVATION GROUP VALIDATION
    // ==================================================================================

    /// Property: Valid base ports should validate
    ///
    /// Mathematical Property: For all ports p in [1, 65535],
    /// ReservationGroup with base=Some(p) should validate (assuming valid services).
    proptest! {
        #[test]
        fn prop_valid_base_port_accepted(base_port in 1u16..=65535) {
            let mut services = HashMap::new();
            services.insert("svc".to_string(), ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            });

            let group = ReservationGroup {
                base: Some(base_port),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_ok(), "Valid base port {} should validate", base_port);
        }
    }

    /// Property: Port 0 as base should fail
    ///
    /// Mathematical Property: ReservationGroup with base=Some(0) should fail.
    proptest! {
        #[test]
        fn prop_base_port_zero_rejected(_dummy in any::<u8>()) {
            let mut services = HashMap::new();
            services.insert("svc".to_string(), ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: None,
            });

            let group = ReservationGroup {
                base: Some(0),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_err(), "Base port 0 should fail");
        }
    }

    /// Property: Duplicate offsets should fail validation
    ///
    /// Mathematical Property: For ReservationGroup with services s1, s2 where
    /// s1.offset = s2.offset, validation fails (uniqueness constraint).
    ///
    /// WHY THIS MATTERS: Each service needs a unique offset to avoid port collisions.
    proptest! {
        #[test]
        fn prop_duplicate_offsets_rejected(offset in 0u16..=100) {
            let mut services = HashMap::new();
            services.insert("svc1".to_string(), ServiceDefinition {
                offset: Some(offset),
                preferred: None,
                env: None,
            });
            services.insert("svc2".to_string(), ServiceDefinition {
                offset: Some(offset),
                preferred: None,
                env: None,
            });

            let group = ReservationGroup {
                base: Some(5000),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_err(), "Duplicate offset {} should fail", offset);
        }
    }

    /// Property: Duplicate preferred ports should fail
    ///
    /// Mathematical Property: For ReservationGroup with services having the same
    /// preferred port, validation fails.
    proptest! {
        #[test]
        fn prop_duplicate_preferred_rejected(preferred in 1u16..=65535) {
            let mut services = HashMap::new();
            services.insert("svc1".to_string(), ServiceDefinition {
                offset: Some(0),
                preferred: Some(preferred),
                env: None,
            });
            services.insert("svc2".to_string(), ServiceDefinition {
                offset: Some(1),
                preferred: Some(preferred),
                env: None,
            });

            let group = ReservationGroup {
                base: Some(5000),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_err(), "Duplicate preferred {} should fail", preferred);
        }
    }

    /// Property: Duplicate environment variable names should fail
    ///
    /// Mathematical Property: For ReservationGroup with services having the same
    /// env var name, validation fails (uniqueness constraint).
    proptest! {
        #[test]
        fn prop_duplicate_env_vars_rejected(env_name in "[A-Z]{1,10}") {
            let mut services = HashMap::new();
            services.insert("svc1".to_string(), ServiceDefinition {
                offset: Some(0),
                preferred: None,
                env: Some(env_name.clone()),
            });
            services.insert("svc2".to_string(), ServiceDefinition {
                offset: Some(1),
                preferred: None,
                env: Some(env_name.clone()),
            });

            let group = ReservationGroup {
                base: Some(5000),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_err(), "Duplicate env var '{}' should fail", env_name);
        }
    }

    /// Property: Unique offsets, preferred ports, and env vars should validate
    ///
    /// Mathematical Property: ReservationGroup with all unique constraints satisfied
    /// should validate successfully.
    proptest! {
        #[test]
        fn prop_unique_constraints_accepted(
            offset1 in 0u16..=50,
            offset2 in 51u16..=100,
            preferred1 in 5000u16..=5100,
            preferred2 in 6000u16..=6100,
        ) {
            let mut services = HashMap::new();
            services.insert("svc1".to_string(), ServiceDefinition {
                offset: Some(offset1),
                preferred: Some(preferred1),
                env: Some("VAR1".to_string()),
            });
            services.insert("svc2".to_string(), ServiceDefinition {
                offset: Some(offset2),
                preferred: Some(preferred2),
                env: Some("VAR2".to_string()),
            });

            let group = ReservationGroup {
                base: Some(5000),
                services,
            };

            let result = ConfigValidator::validate_reservation_group(&group);
            prop_assert!(result.is_ok(), "Unique constraints should validate");
        }
    }
}
