//! Shared validation and derivation for generated environment-variable names.

use crate::{Error, Result};

/// Maximum supported length, in bytes, for an environment-variable identifier.
pub(crate) const MAX_ENVIRONMENT_VARIABLE_NAME_LEN: usize = 255;

/// A portable, validated environment-variable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct EnvironmentVariableName(String);

impl EnvironmentVariableName {
    /// Validates an explicit environment-variable name.
    pub(crate) fn parse(name: &str) -> Result<Self> {
        if name.is_empty() {
            return Err(invalid_environment_variable_name(
                "must not be empty and must match [A-Za-z_][A-Za-z0-9_]*",
            ));
        }

        if name.len() > MAX_ENVIRONMENT_VARIABLE_NAME_LEN {
            return Err(invalid_environment_variable_name(
                "must not exceed 255 bytes",
            ));
        }

        let bytes = name.as_bytes();
        let first = bytes[0];
        if !first.is_ascii_alphabetic() && first != b'_' {
            return Err(invalid_environment_variable_name(
                "must start with an ASCII letter or underscore",
            ));
        }

        if !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return Err(invalid_environment_variable_name(
                "must contain only ASCII letters, digits, and underscores",
            ));
        }

        Ok(Self(name.to_owned()))
    }

    /// Derives a portable name from an unmapped service tag.
    ///
    /// Derivation accepts ASCII input only, uppercases ASCII letters, replaces
    /// hyphens with underscores, and then validates the final identifier.
    pub(crate) fn derive_from_tag(tag: &str) -> Result<Self> {
        if !tag.is_ascii() {
            return Err(non_convertible_service_tag());
        }

        let derived = tag.to_ascii_uppercase().replace('-', "_");
        Self::parse(&derived).map_err(|_| non_convertible_service_tag())
    }

    /// Resolves an explicit mapping or derives a name from an unmapped tag.
    pub(crate) fn resolve(tag: &str, explicit: Option<&str>) -> Result<Self> {
        explicit.map_or_else(|| Self::derive_from_tag(tag), Self::parse)
    }

    /// Returns the validated identifier.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes this value and returns the validated identifier.
    pub(crate) fn into_string(self) -> String {
        self.0
    }

    /// Returns the portable, ASCII-case-insensitive collision key.
    pub(crate) fn collision_key(&self) -> String {
        self.0.to_ascii_uppercase()
    }
}

fn invalid_environment_variable_name(message: &str) -> Error {
    Error::Validation {
        field: "environment_variable".to_owned(),
        message: message.to_owned(),
    }
}

fn non_convertible_service_tag() -> Error {
    Error::Validation {
        field: "reservations.services.tag".to_owned(),
        message: "service tag cannot be converted to a portable environment-variable name; provide an explicit valid env mapping".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_environment_variable_names() {
        for name in [
            "A",
            "_",
            "PORT",
            "WEB_PORT",
            "_PRIVATE_PORT",
            "web_server_port",
            "PORT123",
        ] {
            assert_eq!(EnvironmentVariableName::parse(name).unwrap().as_str(), name);
        }

        let maximum = format!("A{}", "_".repeat(MAX_ENVIRONMENT_VARIABLE_NAME_LEN - 1));
        assert!(EnvironmentVariableName::parse(&maximum).is_ok());
    }

    #[test]
    fn rejects_nonportable_environment_variable_names_without_echoing_input() {
        let invalid = [
            "",
            "1PORT",
            "WEB-PORT",
            "WEB PORT",
            "PORT\nexport ATTACK=1",
            "PORT$(command)",
            "PORT\u{1b}[31m",
            "PORT_CAFÉ",
            "端口",
        ];

        for name in invalid {
            let error = EnvironmentVariableName::parse(name).unwrap_err();
            let diagnostic = error.to_string();
            if !name.is_empty() {
                assert!(!diagnostic.contains(name));
            }
            assert!(!diagnostic.contains('\n'));
            assert!(!diagnostic.contains('\r'));
            assert!(!diagnostic.contains('\u{1b}'));
        }

        let overlong = format!("A{}", "_".repeat(MAX_ENVIRONMENT_VARIABLE_NAME_LEN));
        assert!(EnvironmentVariableName::parse(&overlong).is_err());
    }

    #[test]
    fn derives_ascii_tags_only() {
        assert_eq!(
            EnvironmentVariableName::derive_from_tag("web")
                .unwrap()
                .as_str(),
            "WEB"
        );
        assert_eq!(
            EnvironmentVariableName::derive_from_tag("api-v2")
                .unwrap()
                .as_str(),
            "API_V2"
        );
        assert_eq!(
            EnvironmentVariableName::derive_from_tag("_private")
                .unwrap()
                .as_str(),
            "_PRIVATE"
        );

        for tag in ["123-api", "api.server", "api server", "ß", "ſ", "端口"] {
            assert!(EnvironmentVariableName::derive_from_tag(tag).is_err());
        }
    }

    #[test]
    fn explicit_mapping_allows_a_nonconvertible_tag() {
        let resolved =
            EnvironmentVariableName::resolve("service $(command)", Some("SERVICE_PORT")).unwrap();
        assert_eq!(resolved.as_str(), "SERVICE_PORT");
    }

    #[test]
    fn collision_keys_ignore_ascii_case() {
        let upper = EnvironmentVariableName::parse("WEB_PORT").unwrap();
        let lower = EnvironmentVariableName::parse("web_port").unwrap();
        assert_eq!(upper.collision_key(), lower.collision_key());
    }
}
