//! Shared identifier rules.
//!
//! - Tags, projects, and tasks: Unicode-whitespace trimming, the identifier
//!   rule (non-blank, no NUL, at most 255 UTF-8 bytes after trimming), and
//!   the canonical-form check stored rows must satisfy.
//! - Generated environment-variable names: the portable grammar, derivation
//!   from unmapped service tags, explicit-mapping resolution, and
//!   ASCII-case-insensitive collision keys.
//!
//! Every decision here is made by the formally verified kernels
//! `verified::identifier` and `verified::env_name` (see
//! `docs/verification.md`); this module only maps their results onto trop's
//! error values.

use crate::verified::env_name::{self, EnvNameError, ResolveError};
use crate::verified::identifier as identifier_kernel;
use crate::{Error, Result};

pub(crate) use crate::verified::identifier::IdentifierError;

/// Maximum supported length, in bytes, for an environment-variable identifier.
#[cfg(test)]
pub(crate) const MAX_ENVIRONMENT_VARIABLE_NAME_LEN: usize = env_name::MAX_ENV_NAME_BYTES;

/// Removes leading and trailing Unicode whitespace; identical to `str::trim`.
///
/// The one whitespace trim of the crate's verified text paths: identifiers,
/// and exclusion text (`PortExclusion`'s `FromStr` and the items of
/// `TROP_EXCLUDED_PORTS`).
pub(crate) fn trim_identifier(value: &str) -> &str {
    identifier_kernel::trim(value)
}

/// Applies the shared rule for tags, projects, and tasks.
///
/// Returns the trimmed identifier, or the first violated condition in the
/// order blank (empty or only whitespace), contains NUL, longer than 255
/// UTF-8 bytes.
pub(crate) fn check_identifier(value: &str) -> std::result::Result<&str, IdentifierError> {
    identifier_kernel::check_identifier(value)
}

/// Whether `value` is a canonical stored identifier: non-empty and equal to
/// its own trimmed form.
pub(crate) fn is_canonical_identifier(value: &str) -> bool {
    identifier_kernel::is_canonical_identifier(value)
}

/// Constant diagnostics for the identifier rule; they never echo the input.
///
/// Shared by configuration validation (projects, tasks, service tags) and by
/// [`crate::ReservationKey::new_validated`], so the same rule is reported in
/// the same words everywhere.
pub(crate) const fn identifier_error_message(error: IdentifierError) -> &'static str {
    match error {
        IdentifierError::Blank => "Cannot be empty or only whitespace",
        IdentifierError::ContainsNul => "Cannot contain null bytes",
        IdentifierError::TooLong => "Cannot exceed 255 bytes",
    }
}

/// A portable, validated environment-variable identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct EnvironmentVariableName(String);

impl EnvironmentVariableName {
    /// Validates an explicit environment-variable name.
    pub(crate) fn parse(name: &str) -> Result<Self> {
        env_name::check_env_name(name)
            .map(|()| Self(name.to_owned()))
            .map_err(invalid_environment_variable_name)
    }

    /// Derives a portable name from an unmapped service tag.
    ///
    /// Derivation uppercases ASCII letters, replaces hyphens with
    /// underscores, and then validates the final identifier; tags containing
    /// non-ASCII text never derive a name.
    #[cfg(test)]
    pub(crate) fn derive_from_tag(tag: &str) -> Result<Self> {
        env_name::derive_env_name(tag)
            .map(Self)
            .ok_or_else(non_convertible_service_tag)
    }

    /// Resolves an explicit mapping or derives a name from an unmapped tag.
    ///
    /// An explicit mapping is validated on its own and never falls back to
    /// the tag.
    pub(crate) fn resolve(tag: &str, explicit: Option<&str>) -> Result<Self> {
        env_name::resolve_env_name(tag, explicit)
            .map(Self)
            .map_err(|error| match error {
                ResolveError::InvalidExplicit(error) => invalid_environment_variable_name(error),
                ResolveError::NonConvertibleTag => non_convertible_service_tag(),
            })
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
    ///
    /// Production collision checks run inside the verified kernels
    /// (`verified::config::check_config_group`,
    /// `verified::render::render_group`); the reference models in tests use
    /// this key.
    #[cfg(test)]
    pub(crate) fn collision_key(&self) -> String {
        env_name::env_collision_key(&self.0)
    }
}

/// The diagnostic for an environment-variable name that breaks the portable
/// grammar. Constant: it never echoes the rejected input.
///
/// The one message table for these errors: `EnvironmentVariableName` and the
/// configuration validator's group checks both report it.
pub(crate) const fn environment_variable_name_message(error: EnvNameError) -> &'static str {
    match error {
        EnvNameError::Empty => "must not be empty and must match [A-Za-z_][A-Za-z0-9_]*",
        EnvNameError::TooLong => "must not exceed 255 bytes",
        EnvNameError::BadFirst => "must start with an ASCII letter or underscore",
        EnvNameError::BadRest => "must contain only ASCII letters, digits, and underscores",
    }
}

fn invalid_environment_variable_name(error: EnvNameError) -> Error {
    Error::Validation {
        field: "environment_variable".to_owned(),
        message: environment_variable_name_message(error).to_owned(),
    }
}

/// The diagnostic for an unmapped service tag that derives no portable
/// environment-variable name. Shared with the configuration validator's
/// group checks.
pub(crate) const NON_CONVERTIBLE_SERVICE_TAG_MESSAGE: &str = "service tag cannot be converted to a portable environment-variable name; provide an explicit valid env mapping";

fn non_convertible_service_tag() -> Error {
    Error::Validation {
        field: "reservations.services.tag".to_owned(),
        message: NON_CONVERTIBLE_SERVICE_TAG_MESSAGE.to_owned(),
    }
}

/// Pre-kernel implementations, kept as independent differential oracles for
/// the kernels (they do not call any kernel).
#[cfg(test)]
mod reference {
    use crate::verified::env_name::EnvNameError;

    /// The pre-kernel environment-variable grammar.
    pub(super) fn reference_parse(name: &str) -> Result<(), EnvNameError> {
        if name.is_empty() {
            return Err(EnvNameError::Empty);
        }
        if name.len() > 255 {
            return Err(EnvNameError::TooLong);
        }
        let bytes = name.as_bytes();
        if !bytes[0].is_ascii_alphabetic() && bytes[0] != b'_' {
            return Err(EnvNameError::BadFirst);
        }
        if !bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            return Err(EnvNameError::BadRest);
        }
        Ok(())
    }

    /// The pre-kernel tag derivation.
    pub(super) fn reference_derive(tag: &str) -> Option<String> {
        if !tag.is_ascii() {
            return None;
        }
        let derived = tag.to_ascii_uppercase().replace('-', "_");
        reference_parse(&derived).ok().map(|()| derived)
    }
}

#[cfg(test)]
mod tests {
    use super::reference::{reference_derive, reference_parse};
    use super::*;

    /// The 25 code points of Unicode's `White_Space` property, exactly as the
    /// verified kernel's `is_ws` specification lists them.
    fn kernel_whitespace_table(c: char) -> bool {
        matches!(
            u32::from(c),
            0x09..=0x0D
                | 0x20
                | 0x85
                | 0xA0
                | 0x1680
                | 0x2000..=0x200A
                | 0x2028
                | 0x2029
                | 0x202F
                | 0x205F
                | 0x3000
        )
    }

    /// The UTF-8 width of a scalar value, by code-point range, as `vstd`'s
    /// `encode_scalar` model computes it.
    fn kernel_utf8_width(c: char) -> usize {
        match u32::from(c) {
            0..=0x7F => 1,
            0x80..=0x7FF => 2,
            0x800..=0xFFFF => 3,
            _ => 4,
        }
    }

    fn all_scalar_values() -> impl Iterator<Item = char> {
        (0..=0x0010_FFFF_u32).filter_map(char::from_u32)
    }

    /// Evidence for the trusted modeling assumptions behind the trim kernel:
    /// for every Unicode scalar value, `char::is_whitespace` agrees with the
    /// 25-code-point table the kernel is proved against, `char::len_utf8`
    /// agrees with the range-based UTF-8 width model, and the kernel trim
    /// agrees with `str::trim` on padded single-character inputs.
    #[test]
    fn kernel_whitespace_model_matches_std_for_every_scalar_value() {
        let mut checked = 0_u32;
        let mut whitespace = 0_u32;
        for c in all_scalar_values() {
            assert_eq!(
                c.is_whitespace(),
                kernel_whitespace_table(c),
                "U+{:04X}",
                u32::from(c)
            );
            assert_eq!(c.len_utf8(), kernel_utf8_width(c), "U+{:04X}", u32::from(c));
            let padded = format!("{c}x{c}");
            assert_eq!(
                trim_identifier(&padded),
                padded.trim(),
                "U+{:04X}",
                u32::from(c)
            );
            checked += 1;
            whitespace += u32::from(c.is_whitespace());
        }
        assert_eq!(
            checked,
            0x0011_0000 - 0x800,
            "every non-surrogate code point"
        );
        assert_eq!(whitespace, 25);
    }

    /// The pre-kernel identifier rule, kept as a differential oracle.
    fn reference_check_identifier(value: &str) -> std::result::Result<&str, IdentifierError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(IdentifierError::Blank)
        } else if trimmed.contains('\0') {
            Err(IdentifierError::ContainsNul)
        } else if trimmed.len() > 255 {
            Err(IdentifierError::TooLong)
        } else {
            Ok(trimmed)
        }
    }

    #[test]
    fn trim_matches_str_trim_at_boundaries() {
        let cases = [
            "",
            " ",
            "\u{3000}",
            "a",
            " a ",
            "\t\n\u{b}\u{c}\r a\u{85}b \u{a0}",
            "\u{1680}\u{2000}x\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}",
            "\u{200b}zero-width-space-is-not-whitespace\u{200b}",
            "\u{feff}bom\u{feff}",
            "é",
            " 端口 ",
            "\u{10ffff} \u{10ffff}",
            "\0",
            " \0 ",
        ];
        for value in cases {
            assert_eq!(trim_identifier(value), value.trim(), "{value:?}");
            assert_eq!(
                check_identifier(value),
                reference_check_identifier(value),
                "{value:?}"
            );
            assert_eq!(
                is_canonical_identifier(value),
                !value.is_empty() && value.trim() == value,
                "{value:?}"
            );
        }
    }

    #[test]
    fn identifier_rule_error_precedence_and_byte_limit() {
        assert_eq!(check_identifier(""), Err(IdentifierError::Blank));
        assert_eq!(check_identifier(" \u{3000}\t"), Err(IdentifierError::Blank));
        // NUL is reported before length, and NUL is never trimmed away.
        let nul_and_long = format!("\0{}", "a".repeat(300));
        assert_eq!(
            check_identifier(&nul_and_long),
            Err(IdentifierError::ContainsNul)
        );
        assert_eq!(check_identifier(" \0 "), Err(IdentifierError::ContainsNul));
        // The limit is 255 UTF-8 bytes of the trimmed value.
        let max = "a".repeat(255);
        assert_eq!(check_identifier(&max), Ok(max.as_str()));
        assert_eq!(
            check_identifier(&format!("  {max}\u{3000}")),
            Ok(max.as_str())
        );
        assert_eq!(
            check_identifier(&"a".repeat(256)),
            Err(IdentifierError::TooLong)
        );
        // 127 two-byte chars = 254 bytes; 128 = 256 bytes.
        assert!(check_identifier(&"é".repeat(127)).is_ok());
        assert_eq!(
            check_identifier(&"é".repeat(128)),
            Err(IdentifierError::TooLong)
        );
        // 85 three-byte chars = 255 bytes; one more ASCII byte exceeds.
        assert!(check_identifier(&"端".repeat(85)).is_ok());
        assert_eq!(
            check_identifier(&format!("{}a", "端".repeat(85))),
            Err(IdentifierError::TooLong)
        );
        // Interior whitespace and control characters other than NUL are kept.
        assert_eq!(check_identifier(" a b\nc "), Ok("a b\nc"));
    }

    #[test]
    fn environment_variable_kernel_matches_reference_on_every_short_name() {
        // Every string of length 0..=2 over a byte alphabet that exercises
        // every grammar class and the derivation's special cases.
        let alphabet = [
            "", "a", "z", "A", "Z", "_", "-", "0", "9", "$", " ", "\n", "=", ".", "é", "ß", "ſ",
            "端",
        ];
        for first in alphabet {
            for second in alphabet {
                let name = format!("{first}{second}");
                let parsed = env_name::check_env_name(&name);
                assert_eq!(parsed, reference_parse(&name), "{name:?}");
                assert_eq!(
                    env_name::derive_env_name(&name),
                    reference_derive(&name),
                    "{name:?}"
                );
                if parsed.is_ok() {
                    assert_eq!(
                        EnvironmentVariableName::parse(&name)
                            .unwrap()
                            .collision_key(),
                        name.to_ascii_uppercase()
                    );
                }
            }
        }
    }

    #[test]
    fn environment_variable_grammar_boundaries() {
        assert_eq!(env_name::check_env_name(""), Err(EnvNameError::Empty));
        // Length is checked before the grammar, so an overlong invalid name
        // reports TooLong.
        assert_eq!(
            env_name::check_env_name(&"1".repeat(256)),
            Err(EnvNameError::TooLong)
        );
        assert_eq!(
            env_name::check_env_name(&"é".repeat(128)),
            Err(EnvNameError::TooLong)
        );
        assert_eq!(
            env_name::check_env_name(&"A".repeat(255)),
            Ok(()),
            "255 bytes is the maximum"
        );
        assert_eq!(env_name::check_env_name("1A"), Err(EnvNameError::BadFirst));
        assert_eq!(env_name::check_env_name("é"), Err(EnvNameError::BadFirst));
        assert_eq!(env_name::check_env_name("Aé"), Err(EnvNameError::BadRest));
        assert_eq!(env_name::check_env_name("A\0"), Err(EnvNameError::BadRest));
        // A 255-byte tag derives; 256 bytes never does.
        assert!(env_name::derive_env_name(&"a".repeat(255)).is_some());
        assert!(env_name::derive_env_name(&"a".repeat(256)).is_none());
        assert_eq!(
            env_name::derive_env_name("web-Server_2").as_deref(),
            Some("WEB_SERVER_2")
        );
    }

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
    fn explicit_mapping_never_falls_back_to_the_tag() {
        for explicit in ["", "1BAD", "BAD-NAME", "BAD NAME"] {
            assert!(EnvironmentVariableName::resolve("web", Some(explicit)).is_err());
        }
        assert_eq!(
            EnvironmentVariableName::resolve("web", None)
                .unwrap()
                .as_str(),
            "WEB"
        );
    }

    #[test]
    fn collision_keys_ignore_ascii_case() {
        let upper = EnvironmentVariableName::parse("WEB_PORT").unwrap();
        let lower = EnvironmentVariableName::parse("web_port").unwrap();
        assert_eq!(upper.collision_key(), lower.collision_key());
        assert_eq!(upper.collision_key(), "WEB_PORT");
    }
}

#[cfg(all(test, feature = "property-tests"))]
mod proptests {
    use super::reference::{reference_derive, reference_parse};
    use super::*;
    use proptest::prelude::*;

    /// Strings mixing ASCII, every Unicode whitespace class, NUL, and
    /// multi-byte text, so trimming and the byte limit are both exercised.
    fn identifier_text() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just(' '),
                Just('\t'),
                Just('\n'),
                Just('\u{85}'),
                Just('\u{a0}'),
                Just('\u{2009}'),
                Just('\u{3000}'),
                Just('\u{200b}'),
                Just('\0'),
                Just('a'),
                Just('é'),
                Just('端'),
                Just('\u{1f600}'),
                any::<char>(),
            ],
            0..300,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        #[test]
        fn trim_agrees_with_str_trim(value in identifier_text()) {
            prop_assert_eq!(trim_identifier(&value), value.trim());
            prop_assert_eq!(
                is_canonical_identifier(&value),
                !value.is_empty() && value.trim() == value
            );
        }

        #[test]
        fn identifier_rule_agrees_with_reference(value in identifier_text()) {
            let trimmed = value.trim();
            let expected = if trimmed.is_empty() {
                Err(IdentifierError::Blank)
            } else if trimmed.contains('\0') {
                Err(IdentifierError::ContainsNul)
            } else if trimmed.len() > 255 {
                Err(IdentifierError::TooLong)
            } else {
                Ok(trimmed)
            };
            prop_assert_eq!(check_identifier(&value), expected);
        }

        #[test]
        fn env_name_derivation_agrees_with_reference(tag in "[a-zA-Z0-9_\\- $.é]{0,260}") {
            prop_assert_eq!(env_name::derive_env_name(&tag), reference_derive(&tag));
        }

        #[test]
        fn env_name_grammar_agrees_with_reference(
            name in prop_oneof![
                "[a-zA-Z0-9_]{0,260}",
                "[a-zA-Z_][a-zA-Z0-9_]{250,258}",
                "[a-zA-Z0-9_\\- $=.;`'\"\n\r\t\u{0}\u{1b}é端]{0,260}",
                any::<String>(),
            ]
        ) {
            let verdict = env_name::check_env_name(&name);
            prop_assert_eq!(verdict, reference_parse(&name));
            prop_assert_eq!(
                EnvironmentVariableName::parse(&name).is_ok(),
                verdict.is_ok()
            );
            let resolved = EnvironmentVariableName::resolve("ignored tag", Some(&name));
            prop_assert_eq!(resolved.ok().map(EnvironmentVariableName::into_string), verdict.ok().map(|()| name.clone()));
        }
    }
}
