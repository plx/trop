//! Port exclusion management for filtering out unavailable ports.
//!
//! This module provides efficient checking and management of excluded port
//! lists, supporting both individual ports and ranges.
//!
//! The decisions are made by the verified `verified::exclusions` kernel:
//! entry validation (with the configuration validator's error precedence),
//! the excluded-port set (stored as its canonical run list), membership,
//! insertion, range queries, counting, compaction, the canonical-form check,
//! adding exclusions to a list (`trop exclude`, `trop scan --autoexclude`),
//! and the reserved-port guard of `trop exclude`. The verified
//! `verified::config_rewrite` kernel plans configuration-file rewrites that
//! change only `excluded_ports`, and chooses the file an exclusion edit
//! writes to. The code here only converts between [`PortExclusion`] /
//! [`Port`] / YAML values and the kernels' primitive values.

use super::{Port, PortRange};
use crate::config::{Config, PortExclusion};
use crate::error::Error;
use crate::verified::config_rewrite::{plan_key_rewrite, RewriteStep};
use crate::verified::exclusions::{
    add_exclusion as add_entry, add_ports as add_port_entries,
    compact_exclusions as compact_entries, first_reserved_port as first_reserved_entry_port,
    is_compact as entries_are_compact, validate_entries, ExclusionEntry, ExclusionError,
    ExclusionSet,
};
use serde::Deserialize;
use serde_yaml::value::TaggedValue;
use serde_yaml::{Mapping, Value};

pub use crate::verified::config_rewrite::{select_write_target, WriteScope, WriteTarget};

/// Converts a configuration entry to the kernel's field-for-field mirror.
const fn to_entry(exclusion: &PortExclusion) -> ExclusionEntry {
    match *exclusion {
        PortExclusion::Single(port) => ExclusionEntry::Single(port),
        PortExclusion::Range { start, end } => ExclusionEntry::Range { start, end },
    }
}

/// Converts a kernel entry back to a configuration entry.
const fn from_entry(entry: ExclusionEntry) -> PortExclusion {
    match entry {
        ExclusionEntry::Single(port) => PortExclusion::Single(port),
        ExclusionEntry::Range { start, end } => PortExclusion::Range { start, end },
    }
}

fn to_entries(exclusions: &[PortExclusion]) -> Vec<ExclusionEntry> {
    exclusions.iter().map(to_entry).collect()
}

/// The configuration validator's message for an invalid entry.
fn validation_message(error: ExclusionError) -> String {
    match error {
        ExclusionError::ZeroPort => "Invalid port: 0".into(),
        ExclusionError::ZeroStart => "Invalid start port: 0".into(),
        ExclusionError::ZeroEnd => "Invalid end port: 0".into(),
        ExclusionError::Reversed { start, end } => {
            format!("Invalid range: {start}..{end} (end < start)")
        }
    }
}

/// The configuration validator's error for the entry at `index`.
fn validation_error(index: usize, error: ExclusionError) -> Error {
    Error::Validation {
        field: format!("excluded_ports[{index}]"),
        message: validation_message(error),
    }
}

/// The error [`ExclusionManager`] reports for an invalid entry: the same
/// errors `Port::try_from` (port 0) and `PortRange::new` (reversed range)
/// report.
fn manager_error(error: ExclusionError) -> Error {
    match error {
        ExclusionError::ZeroPort | ExclusionError::ZeroStart | ExclusionError::ZeroEnd => {
            Error::InvalidPort {
                value: 0,
                reason: "port 0 is invalid".into(),
            }
        }
        ExclusionError::Reversed { start, end } => Error::InvalidPortRange {
            min: start,
            max: end,
            reason: "max must be greater than or equal to min".into(),
        },
    }
}

/// Validates an `excluded_ports` list.
///
/// Every entry must be a valid port (`1..=65535`) or a valid inclusive range
/// (`1 <= start <= end`; `5000..5000` is valid). Reports the first invalid
/// entry; for a range a zero start is reported before a zero end, which is
/// reported before a reversed range.
///
/// # Errors
///
/// Returns [`Error::Validation`] for field `excluded_ports[i]`, where `i` is
/// the index of the first invalid entry.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::validate_exclusions;
///
/// assert!(validate_exclusions(&[PortExclusion::Range { start: 5000, end: 5000 }]).is_ok());
///
/// let err = validate_exclusions(&[
///     PortExclusion::Single(5001),
///     PortExclusion::Range { start: 5010, end: 5000 },
/// ])
/// .unwrap_err();
/// assert!(err.to_string().contains("excluded_ports[1]"));
/// ```
pub fn validate_exclusions(exclusions: &[PortExclusion]) -> crate::Result<()> {
    validate_entries(&to_entries(exclusions))
        .map_err(|(index, error)| validation_error(index, error))
}

/// Compacts an exclusion list to its minimal representation.
///
/// The result excludes exactly the same ports as `exclusions`. It is sorted,
/// merges overlapping and adjacent entries into maximal ranges, and uses
/// `Single` exactly for one-port runs. No exclusion list for the same ports
/// has fewer entries, and the result is the only list with these properties,
/// so it depends only on the excluded ports (not on entry order, duplicates,
/// or overlaps), and compacting it again returns it unchanged. These
/// properties are verified by the `verified::exclusions` kernel.
///
/// # Errors
///
/// Returns the same [`Error::Validation`] as [`validate_exclusions`] if any
/// entry is invalid: invalid entries are never dropped or changed.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::compact_exclusions;
///
/// let compacted = compact_exclusions(&[
///     PortExclusion::Single(5003),
///     PortExclusion::Range { start: 5000, end: 5002 },
///     PortExclusion::Single(6000),
/// ])
/// .unwrap();
/// assert_eq!(
///     compacted,
///     vec![
///         PortExclusion::Range { start: 5000, end: 5003 },
///         PortExclusion::Single(6000),
///     ]
/// );
///
/// assert!(compact_exclusions(&[PortExclusion::Range { start: 5010, end: 5000 }]).is_err());
/// ```
pub fn compact_exclusions(exclusions: &[PortExclusion]) -> crate::Result<Vec<PortExclusion>> {
    compact_entries(&to_entries(exclusions))
        .map(|runs| runs.into_iter().map(from_entry).collect())
        .map_err(|(index, error)| validation_error(index, error))
}

/// Whether an exclusion list is already in its compacted (canonical) form.
///
/// This holds exactly when [`compact_exclusions`] would return the list
/// unchanged: every entry is valid, the entries are sorted, every two
/// entries are separated by at least one port that is not excluded, and
/// `Single` is used exactly for one-port entries. Lists of minimal length can
/// still be non-canonical (`[9000, 8000]`, or `[5000..5000]`), so callers
/// that rewrite a list after compaction decide by this, not by comparing
/// entry counts. Verified by the `verified::exclusions` kernel.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::is_compact;
///
/// assert!(is_compact(&[PortExclusion::Single(7000), PortExclusion::Single(9000)]));
/// assert!(!is_compact(&[PortExclusion::Single(9000), PortExclusion::Single(7000)]));
/// assert!(!is_compact(&[PortExclusion::Range { start: 7000, end: 7000 }]));
/// ```
#[must_use]
pub fn is_compact(exclusions: &[PortExclusion]) -> bool {
    entries_are_compact(&to_entries(exclusions))
}

/// Adds an exclusion to a list, the way `trop exclude` does, and reports
/// whether the list changed.
///
/// - The result excludes exactly the ports of `exclusions` plus the ports of
///   `exclusion`.
/// - If one entry already excludes every port of `exclusion` (in particular
///   an identical entry), the list is returned unchanged and the flag is
///   `false`.
/// - Otherwise the entries that overlap the new one (directly, or through
///   other merged entries) are merged with it into one range at the position
///   of the first of them; every other entry keeps its place. Adjacent
///   entries do not overlap, so `5010` next to `5000..5009` is appended.
///
/// The flag is `true` exactly when the returned list differs from the input.
/// These properties are verified by the `verified::exclusions` kernel.
///
/// # Errors
///
/// Returns [`Error::Validation`] for the first invalid entry: field
/// `excluded_ports[i]` for an entry of `exclusions`, or field `exclusion`
/// for the added entry. Nothing is added then.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::add_exclusion_to_list;
///
/// let list = vec![PortExclusion::Range { start: 8080, end: 8090 }];
///
/// // Already excluded: unchanged.
/// let (same, changed) = add_exclusion_to_list(&list, &PortExclusion::Single(8085)).unwrap();
/// assert!(!changed);
/// assert_eq!(same, list);
///
/// // Overlapping: merged.
/// let (merged, changed) =
///     add_exclusion_to_list(&list, &PortExclusion::Range { start: 8085, end: 8095 }).unwrap();
/// assert!(changed);
/// assert_eq!(merged, vec![PortExclusion::Range { start: 8080, end: 8095 }]);
/// ```
pub fn add_exclusion_to_list(
    exclusions: &[PortExclusion],
    exclusion: &PortExclusion,
) -> crate::Result<(Vec<PortExclusion>, bool)> {
    match add_entry(&to_entries(exclusions), to_entry(exclusion)) {
        Ok((entries, changed)) => Ok((entries.into_iter().map(from_entry).collect(), changed)),
        Err((index, error)) if index < exclusions.len() => Err(validation_error(index, error)),
        Err((_, error)) => Err(Error::Validation {
            field: "exclusion".into(),
            message: validation_message(error),
        }),
    }
}

/// Adds a single-port exclusion for each port, the way
/// `trop scan --autoexclude` does, and returns the new list with the number
/// of ports newly excluded.
///
/// Ports the list already excludes (and repeated ports) are skipped; every
/// other port is appended as a `Single`, in the order of its first
/// occurrence in `ports`. The count is the number of entries appended, which
/// is the number of distinct ports newly excluded. Verified by the
/// `verified::exclusions` kernel.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::add_ports_to_list;
/// use trop::Port;
///
/// let list = vec![PortExclusion::Range { start: 8080, end: 8090 }];
/// let ports = [8085, 9000, 9000].map(|p| Port::try_from(p).unwrap());
/// let (updated, added) = add_ports_to_list(&list, &ports);
/// assert_eq!(added, 1);
/// assert_eq!(updated[1], PortExclusion::Single(9000));
/// ```
#[must_use]
pub fn add_ports_to_list(
    exclusions: &[PortExclusion],
    ports: &[Port],
) -> (Vec<PortExclusion>, usize) {
    let ports: Vec<u16> = ports.iter().map(|port| port.value()).collect();
    let (entries, added) = add_port_entries(&to_entries(exclusions), &ports);
    (entries.into_iter().map(from_entry).collect(), added)
}

/// The lowest port of `exclusion` for which `is_reserved` answers `true`.
///
/// The `verified::exclusions` kernel proves that `is_reserved` is consulted
/// only at valid ports the exclusion names (never port 0), that a returned
/// port answered `true` while every lower port of the exclusion answered
/// `false`, and that an error is returned, never treated as "not reserved".
/// The ports are checked one at a time in ascending order, stopping at the
/// first reserved port or the first error; that order is how the kernel is
/// written (and is tested), not part of its proved specification.
///
/// # Errors
///
/// Returns the first error `is_reserved` reports.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::first_reserved_port;
///
/// let range = PortExclusion::Range { start: 8081, end: 8090 };
/// let found = first_reserved_port(&range, |port| Ok::<_, ()>(port.value() % 4 == 0));
/// assert_eq!(found.unwrap().map(|port| port.value()), Some(8084));
/// ```
pub fn first_reserved_port<E, F>(
    exclusion: &PortExclusion,
    is_reserved: F,
) -> Result<Option<Port>, E>
where
    F: Fn(Port) -> Result<bool, E>,
{
    // The kernel consults the oracle only at valid ports (never 0) and
    // returns one of them, so `Port` is built directly.
    first_reserved_entry_port(to_entry(exclusion), &|port: u16| is_reserved(Port(port)))
        .map(|found| found.map(Port))
        .map_err(|failure| failure.error)
}

/// The top-level key of the exclusion list.
const EXCLUDED_PORTS_KEY: &str = "excluded_ports";

/// `value` without its YAML tags.
fn untagged(mut value: &Value) -> &Value {
    while let Value::Tagged(tagged) = value {
        value = &tagged.value;
    }
    value
}

/// Whether the configuration reader reads `key` as the `excluded_ports`
/// field. It identifies a field by the text of its key and ignores YAML tags
/// on the key, so `!tag excluded_ports` names the field as well.
fn is_excluded_ports_key(key: &Value) -> bool {
    untagged(key).as_str() == Some(EXCLUDED_PORTS_KEY)
}

/// A configuration document as the configuration loader reads it.
#[derive(Debug, PartialEq)]
struct LoadedDocument {
    /// The document parsed as a [`Config`].
    config: Config,
    /// The state of the `reservations` key: absent (`None`), `null`
    /// (`Some(false)`), or set (`Some(true)`). The loader reads an absent
    /// key as "inherit" and `null` as "clear".
    reservations: Option<bool>,
}

/// The `reservations` key as the loader's reservations-presence probe reads
/// it (`ReservationPresence` in `config/loader.rs`).
#[derive(Deserialize)]
struct ReservationsKey {
    #[serde(default, deserialize_with = "reservations_state")]
    reservations: Option<bool>,
}

fn reservations_state<'de, D>(deserializer: D) -> std::result::Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<serde::de::IgnoredAny>::deserialize(deserializer).map(|value| Some(value.is_some()))
}

/// Reads a configuration document exactly the way `ConfigLoader` reads a
/// file: `serde_yaml::from_str` into [`Config`] and into the reservations
/// probe, with no special cases.
///
/// A document with no content (empty, only comments, or only a `---`
/// marker) reads as the default configuration, because `serde_yaml` reads it
/// as an empty mapping. A document whose content is an explicit null
/// (`null`, `~`, or a tagged null) is not a mapping, and is rejected here as
/// it is by the loader.
///
/// The error message is `serde_yaml`'s description of the problem, under the
/// field `configuration`.
fn read_configuration(document: &str) -> crate::Result<LoadedDocument> {
    let unreadable = |error: serde_yaml::Error| Error::Validation {
        field: "configuration".into(),
        message: error.to_string(),
    };
    Ok(LoadedDocument {
        config: serde_yaml::from_str(document).map_err(unreadable)?,
        reservations: serde_yaml::from_str::<ReservationsKey>(document)
            .map_err(unreadable)?
            .reservations,
    })
}

/// Whether two documents are equal with their top-level mapping keys in the
/// same order. (`Mapping` equality ignores the order of keys.)
fn same_in_order(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Tagged(left), Value::Tagged(right)) => {
            left.tag == right.tag && same_in_order(&left.value, &right.value)
        }
        (Value::Mapping(left), Value::Mapping(right)) => {
            left.len() == right.len() && left.iter().eq(right.iter())
        }
        _ => left == right,
    }
}

/// The `excluded_ports` list of a YAML configuration document, or `None`
/// when the document has no list (an empty document has none).
///
/// The document is read the way the configuration loader reads a file, so
/// unknown keys and malformed values are rejected; the entries themselves
/// are not validated (see [`validate_exclusions`]).
///
/// # Errors
///
/// Returns [`Error::Validation`] (field `configuration`, with `serde_yaml`'s
/// description of the problem) if the document is not valid YAML or not a
/// valid configuration.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::excluded_ports_in;
///
/// assert_eq!(excluded_ports_in("").unwrap(), None);
/// assert_eq!(
///     excluded_ports_in("excluded_ports:\n  - 8080\n").unwrap(),
///     Some(vec![PortExclusion::Single(8080)])
/// );
/// ```
pub fn excluded_ports_in(document: &str) -> crate::Result<Option<Vec<PortExclusion>>> {
    Ok(read_configuration(document)?.config.excluded_ports)
}

/// Rewrites the `excluded_ports` list of a YAML configuration document,
/// leaving every other top-level key exactly as it was.
///
/// The document is read as a YAML mapping (an empty document is an empty
/// mapping; a tag on the document is kept). The verified
/// `verified::config_rewrite` kernel plans the result over the entries'
/// "is `excluded_ports`" marks: every other entry is copied unchanged and in
/// order, and `excluded_ports` is replaced in place, or appended when
/// absent. An entry is marked when the configuration reader reads its key as
/// `excluded_ports` (it ignores YAML tags on keys). A key that was absent
/// stays absent and an explicit `null` stays `null`, so the rewrite does not
/// change how the file overlays the files below it (for example, an omitted
/// `reservations` key keeps inheriting the group of `trop.yaml`).
///
/// Before the text is returned it is checked three ways:
///
/// - it parses back to exactly the planned document, with the top-level
///   keys in the planned order;
/// - against the original mapping: every unmarked entry reads back with its
///   exact value, exactly one key reads as `excluded_ports` and it holds the
///   new list, and no other key was added;
/// - through the configuration loader's own reading: the text reads as the
///   original configuration with only `excluded_ports` replaced, and the
///   `reservations` key keeps its absent, `null`, or set state.
///
/// YAML comments and the original formatting are not preserved. Scalars are
/// re-emitted in canonical form, so a document whose string-typed value is
/// written as an unquoted scalar that YAML reads as a non-string in a
/// non-canonical spelling (a project named `0x10`, say) fails the last check
/// and is refused rather than changed.
///
/// # Errors
///
/// Returns [`Error::Validation`] if the document is not valid YAML, not a
/// mapping, or not a valid configuration, or if the rewritten document cannot
/// be emitted or fails one of the checks above.
///
/// # Examples
///
/// ```
/// use trop::config::PortExclusion;
/// use trop::port::exclusions::rewrite_excluded_ports;
///
/// let text = rewrite_excluded_ports("project: demo\n", &[PortExclusion::Single(8080)]).unwrap();
/// assert_eq!(text, "project: demo\nexcluded_ports:\n- 8080\n");
/// ```
pub fn rewrite_excluded_ports(
    document: &str,
    exclusions: &[PortExclusion],
) -> crate::Result<String> {
    let invalid = |message: String| Error::Validation {
        field: EXCLUDED_PORTS_KEY.into(),
        message,
    };
    // How the loader reads the document now (this also rejects documents
    // that are not configurations, and documents with a repeated key).
    let before = read_configuration(document)?;
    let parsed: Value = serde_yaml::from_str(document)
        .map_err(|error| invalid(format!("Invalid YAML: {error}")))?;
    // A tagged document reads like an untagged one; its tag is kept.
    let (tag, root) = match parsed {
        Value::Tagged(tagged) => {
            let TaggedValue { tag, value } = *tagged;
            (Some(tag), value)
        }
        root => (None, root),
    };
    let original = match root {
        Value::Null => Mapping::new(),
        Value::Mapping(mapping) => mapping,
        _ => return Err(invalid("the configuration is not a YAML mapping".into())),
    };
    let key = Value::from(EXCLUDED_PORTS_KEY);
    let value = serde_yaml::to_value(exclusions)
        .map_err(|error| invalid(format!("Failed to serialize exclusions: {error}")))?;
    let entries: Vec<(&Value, &Value)> = original.iter().collect();
    let is_target: Vec<bool> = entries
        .iter()
        .map(|(entry_key, _)| is_excluded_ports_key(entry_key))
        .collect();

    // Execute the kernel's plan. The kept keys are distinct (they come from
    // one mapping) and differ from `excluded_ports` (they are unmarked), so
    // every `insert` appends.
    let mut planned = Mapping::new();
    for step in plan_key_rewrite(&is_target) {
        let (entry_key, entry_value) = match step {
            RewriteStep::Keep(index) => (entries[index].0.clone(), entries[index].1.clone()),
            RewriteStep::Set => (key.clone(), value.clone()),
        };
        planned.insert(entry_key, entry_value);
    }
    let planned = match tag {
        Some(tag) => Value::Tagged(Box::new(TaggedValue {
            tag,
            value: Value::Mapping(planned),
        })),
        None => Value::Mapping(planned),
    };

    let text = serde_yaml::to_string(&planned)
        .map_err(|error| invalid(format!("Failed to serialize configuration: {error}")))?;
    // Check the trusted YAML emitter against the plan: the text must read
    // back as exactly the planned document, keys in the planned order.
    let reread: Value = serde_yaml::from_str(&text).map_err(|error| {
        invalid(format!(
            "Rewritten configuration is not valid YAML: {error}"
        ))
    })?;
    if !same_in_order(&reread, &planned) {
        return Err(invalid(
            "the rewritten configuration does not read back unchanged".into(),
        ));
    }
    // Check the result against the original as well (independently of how
    // the plan was executed): every unmarked entry reads back with its exact
    // value, exactly one key reads as `excluded_ports` and holds the new
    // list, and no other key was added.
    let preserved = untagged(&reread).as_mapping().is_some_and(|rewritten| {
        let kept = entries
            .iter()
            .filter(|(entry_key, _)| !is_excluded_ports_key(entry_key));
        rewritten.len() == kept.clone().count() + 1
            && rewritten
                .keys()
                .filter(|entry_key| is_excluded_ports_key(entry_key))
                .count()
                == 1
            && rewritten.get(&key) == Some(&value)
            && kept
                .into_iter()
                .all(|(entry_key, entry_value)| rewritten.get(*entry_key) == Some(*entry_value))
    });
    if !preserved {
        return Err(invalid(
            "the rewrite would change keys other than excluded_ports".into(),
        ));
    }
    // Check the result through the loader's own reading of the file: the
    // effective configuration changes only in `excluded_ports`, and the
    // overlay state of `reservations` is unchanged.
    let expected = LoadedDocument {
        config: Config {
            excluded_ports: Some(exclusions.to_vec()),
            ..before.config
        },
        reservations: before.reservations,
    };
    if read_configuration(&text).ok() != Some(expected) {
        return Err(invalid(
            "the rewritten configuration would not read as the original with only \
             excluded_ports replaced"
                .into(),
        ));
    }
    Ok(text)
}

/// Manages excluded port lists with efficient checking.
///
/// The `ExclusionManager` stores ports and port ranges that should not be
/// allocated. It provides efficient checking using sorted data structures
/// and supports compaction to merge overlapping ranges.
///
/// # Examples
///
/// ```
/// use trop::port::exclusions::ExclusionManager;
/// use trop::config::PortExclusion;
/// use trop::Port;
///
/// let exclusions = vec![
///     PortExclusion::Single(5001),
///     PortExclusion::Range { start: 5005, end: 5009 },
/// ];
///
/// let manager = ExclusionManager::from_config(&exclusions).unwrap();
///
/// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
/// assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
/// assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
/// ```
#[derive(Debug, Clone)]
pub struct ExclusionManager {
    /// The excluded ports, stored as their canonical (sorted, disjoint,
    /// non-adjacent) run list by the verified `ExclusionSet` kernel type.
    excluded: ExclusionSet,
}

impl ExclusionManager {
    /// Create a new `ExclusionManager` from a configuration.
    ///
    /// The excluded ports are stored as sorted, merged ranges, so memory use
    /// is proportional to the number of distinct ranges, not ports.
    ///
    /// # Errors
    ///
    /// Returns an error for the first invalid entry: a port of 0
    /// ([`Error::InvalidPort`]) or a range whose end precedes its start
    /// ([`Error::InvalidPortRange`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Range { start: 5005, end: 5009 },
    /// ];
    ///
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    /// ```
    pub fn from_config(exclusions: &[PortExclusion]) -> crate::Result<Self> {
        ExclusionSet::from_entries(&to_entries(exclusions))
            .map(|excluded| Self { excluded })
            .map_err(|(_, error)| manager_error(error))
    }

    /// Create an empty exclusion manager.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let manager = ExclusionManager::empty();
    /// assert!(!manager.is_excluded(Port::try_from(8080).unwrap()));
    /// ```
    #[must_use]
    pub fn empty() -> Self {
        Self {
            excluded: ExclusionSet::empty(),
        }
    }

    /// Check if a specific port is excluded.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    /// use trop::Port;
    ///
    /// let exclusions = vec![PortExclusion::Single(5001)];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
    /// assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
    /// ```
    #[must_use]
    pub fn is_excluded(&self, port: Port) -> bool {
        self.excluded.contains(port.value())
    }

    /// Get all excluded ports within a given range.
    ///
    /// Returns a vector of excluded ports that fall within the specified
    /// range (inclusive on both ends), in ascending order.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    /// use trop::{Port, PortRange};
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Single(5005),
    ///     PortExclusion::Single(5020),
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(5010).unwrap(),
    /// ).unwrap();
    ///
    /// let excluded = manager.excluded_in_range(&range);
    /// assert_eq!(excluded.len(), 2); // 5001 and 5005, not 5020
    /// ```
    #[must_use]
    pub fn excluded_in_range(&self, range: &PortRange) -> Vec<Port> {
        // The kernel guarantees every member is a valid port (never 0).
        self.excluded
            .members_between(range.min().value(), range.max().value())
            .into_iter()
            .map(Port)
            .collect()
    }

    /// Add a single port to the exclusion set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_port(Port::try_from(8080).unwrap());
    ///
    /// assert!(manager.is_excluded(Port::try_from(8080).unwrap()));
    /// ```
    pub fn add_port(&mut self, port: Port) {
        // The kernel rejects only port 0, which no `Port` holds.
        let inserted = self.excluded.insert_port(port.value());
        debug_assert!(inserted, "a Port is never 0");
    }

    /// Add a range of ports to the exclusion set.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::{Port, PortRange};
    ///
    /// let mut manager = ExclusionManager::empty();
    /// let range = PortRange::new(
    ///     Port::try_from(5000).unwrap(),
    ///     Port::try_from(5002).unwrap(),
    /// ).unwrap();
    ///
    /// manager.add_range(&range);
    ///
    /// assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
    /// assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
    /// assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
    /// assert!(!manager.is_excluded(Port::try_from(5003).unwrap()));
    /// ```
    pub fn add_range(&mut self, range: &PortRange) {
        self.excluded.insert_span(range.span);
    }

    /// Add an exclusion from configuration.
    ///
    /// On error the exclusion set is unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is invalid: a port of 0
    /// ([`Error::InvalidPort`]) or a range whose end precedes its start
    /// ([`Error::InvalidPortRange`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_exclusion(&PortExclusion::Single(8080)).unwrap();
    /// ```
    pub fn add_exclusion(&mut self, exclusion: &PortExclusion) -> crate::Result<()> {
        self.excluded
            .insert_entry(to_entry(exclusion))
            .map_err(manager_error)
    }

    /// Get a compacted representation of exclusions.
    ///
    /// This returns the minimal list of `PortExclusion` entries for the
    /// excluded ports: sorted, with adjacent and overlapping ports merged
    /// into ranges, and `Single` exactly for one-port runs (see
    /// [`compact_exclusions`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::Port;
    ///
    /// let mut manager = ExclusionManager::empty();
    /// manager.add_port(Port::try_from(5000).unwrap());
    /// manager.add_port(Port::try_from(5001).unwrap());
    /// manager.add_port(Port::try_from(5002).unwrap());
    /// manager.add_port(Port::try_from(5010).unwrap());
    ///
    /// let compacted = manager.compact();
    /// // Should produce: Range(5000..5002) and Single(5010)
    /// assert_eq!(compacted.len(), 2);
    /// ```
    #[must_use]
    pub fn compact(&self) -> Vec<PortExclusion> {
        self.excluded.runs().into_iter().map(from_entry).collect()
    }

    /// Get the total number of excluded ports.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Range { start: 5005, end: 5009 },
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// assert_eq!(manager.len(), 6); // 5001 plus 5005-5009 (5 ports)
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.excluded.len()
    }

    /// Check if there are no exclusions.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    ///
    /// let manager = ExclusionManager::empty();
    /// assert!(manager.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.excluded.is_empty()
    }

    /// Get an iterator over all excluded ports in sorted order.
    ///
    /// # Examples
    ///
    /// ```
    /// use trop::port::exclusions::ExclusionManager;
    /// use trop::config::PortExclusion;
    ///
    /// let exclusions = vec![
    ///     PortExclusion::Single(5001),
    ///     PortExclusion::Single(5003),
    ///     PortExclusion::Single(5002),
    /// ];
    /// let manager = ExclusionManager::from_config(&exclusions).unwrap();
    ///
    /// let mut iter = manager.iter();
    /// assert_eq!(iter.next().unwrap().value(), 5001);
    /// assert_eq!(iter.next().unwrap().value(), 5002);
    /// assert_eq!(iter.next().unwrap().value(), 5003);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = Port> + '_ {
        // The kernel guarantees every member is a valid port (never 0).
        self.excluded
            .members_between(Port::MIN, Port::MAX)
            .into_iter()
            .map(Port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_manager() {
        let manager = ExclusionManager::empty();
        assert!(manager.is_empty());
        assert_eq!(manager.len(), 0);
        assert!(!manager.is_excluded(Port::try_from(8080).unwrap()));
    }

    #[test]
    fn test_from_config_single_ports() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5009),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5009).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_from_config_range() {
        let exclusions = vec![PortExclusion::Range {
            start: 5000,
            end: 5002,
        }];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5003).unwrap()));
    }

    #[test]
    fn test_from_config_mixed() {
        let exclusions = vec![
            PortExclusion::Single(5000),
            PortExclusion::Range {
                start: 5005,
                end: 5009,
            },
            PortExclusion::Single(5020),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 7); // 1 + 5 + 1
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5005).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5009).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5020).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(5010).unwrap()));
    }

    #[test]
    fn test_excluded_in_range() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5020),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 2);
        assert!(excluded.contains(&Port::try_from(5001).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
        assert!(!excluded.contains(&Port::try_from(5020).unwrap()));
    }

    #[test]
    fn test_add_port() {
        let mut manager = ExclusionManager::empty();
        let port = Port::try_from(8080).unwrap();

        assert!(!manager.is_excluded(port));
        manager.add_port(port);
        assert!(manager.is_excluded(port));
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn test_add_range() {
        let mut manager = ExclusionManager::empty();
        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5002).unwrap()).unwrap();

        manager.add_range(&range);

        assert_eq!(manager.len(), 3);
        assert!(manager.is_excluded(Port::try_from(5000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5001).unwrap()));
        assert!(manager.is_excluded(Port::try_from(5002).unwrap()));
    }

    #[test]
    fn test_compact_single_ports() {
        let exclusions = vec![
            PortExclusion::Single(5001),
            PortExclusion::Single(5005),
            PortExclusion::Single(5009),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);
    }

    #[test]
    fn test_compact_adjacent_ports() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5001).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 1);
        assert_eq!(
            compacted[0],
            PortExclusion::Range {
                start: 5000,
                end: 5002
            }
        );
    }

    #[test]
    fn test_compact_range_ending_at_max_port() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(65534).unwrap());
        manager.add_port(Port::try_from(65535).unwrap());

        let compacted = manager.compact();
        assert_eq!(
            compacted,
            vec![PortExclusion::Range {
                start: 65534,
                end: 65535,
            }]
        );
    }

    #[test]
    fn test_compact_mixed() {
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5001).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());
        manager.add_port(Port::try_from(5010).unwrap());
        manager.add_port(Port::try_from(5020).unwrap());
        manager.add_port(Port::try_from(5021).unwrap());

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);

        // Should have: Range(5000..5002), Single(5010), Range(5020..5021)
        assert!(compacted.contains(&PortExclusion::Range {
            start: 5000,
            end: 5002
        }));
        assert!(compacted.contains(&PortExclusion::Single(5010)));
        assert!(compacted.contains(&PortExclusion::Range {
            start: 5020,
            end: 5021
        }));
    }

    #[test]
    fn test_iter() {
        let exclusions = vec![
            PortExclusion::Single(5003),
            PortExclusion::Single(5001),
            PortExclusion::Single(5002),
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let ports: Vec<u16> = manager.iter().map(Port::value).collect();
        assert_eq!(ports, vec![5001, 5002, 5003]); // Should be sorted
    }

    #[test]
    fn test_overlapping_exclusions() {
        // Test that overlapping ranges are correctly merged into the exclusion set
        // This verifies BTreeSet properly handles duplicates
        let exclusions = vec![
            PortExclusion::Range {
                start: 5000,
                end: 5005,
            },
            PortExclusion::Range {
                start: 5003,
                end: 5008,
            },
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        // Should have ports 5000-5008 (9 ports total)
        assert_eq!(manager.len(), 9);
        for port in 5000..=5008 {
            assert!(manager.is_excluded(Port::try_from(port).unwrap()));
        }
    }

    #[test]
    fn test_compaction_single_port_at_boundary() {
        // Test compaction when a single port exists at range boundaries
        // Ensures proper handling of single-element ranges that shouldn't be compacted
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5005).unwrap()); // Gap, shouldn't merge
        manager.add_port(Port::try_from(5010).unwrap()); // Gap, shouldn't merge

        let compacted = manager.compact();
        assert_eq!(compacted.len(), 3);
        assert!(compacted.contains(&PortExclusion::Single(5000)));
        assert!(compacted.contains(&PortExclusion::Single(5005)));
        assert!(compacted.contains(&PortExclusion::Single(5010)));
    }

    #[test]
    fn test_compaction_large_continuous_range() {
        // Test compaction on a large continuous range to verify efficiency
        // This ensures the compaction algorithm scales properly
        let mut manager = ExclusionManager::empty();
        for port in 5000..=5100 {
            manager.add_port(Port::try_from(port).unwrap());
        }

        let compacted = manager.compact();
        // Should compact to a single range
        assert_eq!(compacted.len(), 1);
        assert_eq!(
            compacted[0],
            PortExclusion::Range {
                start: 5000,
                end: 5100
            }
        );
    }

    #[test]
    fn test_compaction_alternating_pattern() {
        // Test compaction with alternating excluded/available ports
        // Verifies correct handling of non-consecutive patterns
        let mut manager = ExclusionManager::empty();
        manager.add_port(Port::try_from(5000).unwrap());
        manager.add_port(Port::try_from(5002).unwrap());
        manager.add_port(Port::try_from(5004).unwrap());
        manager.add_port(Port::try_from(5006).unwrap());

        let compacted = manager.compact();
        // All should be single ports (no consecutive pairs)
        assert_eq!(compacted.len(), 4);
        for exclusion in compacted {
            assert!(matches!(exclusion, PortExclusion::Single(_)));
        }
    }

    #[test]
    fn test_excluded_in_range_partial_overlap() {
        // Test range query when exclusions partially overlap the query range
        // Verifies correct boundary handling in range queries
        let exclusions = vec![
            PortExclusion::Single(4995), // Before range
            PortExclusion::Single(5001), // In range
            PortExclusion::Single(5005), // In range
            PortExclusion::Single(6000), // After range
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 2);
        assert!(excluded.contains(&Port::try_from(5001).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
    }

    #[test]
    fn test_excluded_in_range_exact_boundaries() {
        // Test range query with exclusions exactly at range boundaries
        // Ensures inclusive boundary semantics work correctly
        let exclusions = vec![
            PortExclusion::Single(5000), // Exactly at min
            PortExclusion::Single(5005), // Middle
            PortExclusion::Single(5010), // Exactly at max
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        let range =
            PortRange::new(Port::try_from(5000).unwrap(), Port::try_from(5010).unwrap()).unwrap();

        let excluded = manager.excluded_in_range(&range);
        assert_eq!(excluded.len(), 3);
        assert!(excluded.contains(&Port::try_from(5000).unwrap()));
        assert!(excluded.contains(&Port::try_from(5005).unwrap()));
        assert!(excluded.contains(&Port::try_from(5010).unwrap()));
    }

    #[test]
    fn test_add_exclusion_error_handling() {
        // Test that add_exclusion properly handles invalid port values
        // Verifies error propagation from port validation
        let mut manager = ExclusionManager::empty();

        // Port 0 is invalid and should cause an error
        let invalid_exclusion = PortExclusion::Single(0);
        let result = manager.add_exclusion(&invalid_exclusion);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_config_invalid_range() {
        // Test that from_config properly validates range endpoints
        // Ensures invalid ranges are rejected during construction
        let exclusions = vec![PortExclusion::Range {
            start: 5010,
            end: 5000, // Invalid: max < min
        }];

        let result = ExclusionManager::from_config(&exclusions);
        assert!(result.is_err());
    }

    #[test]
    fn test_exclusion_manager_boundary_ports() {
        // Test exclusion manager with port number boundaries
        // Verifies correct handling of extreme valid port values
        let exclusions = vec![
            PortExclusion::Single(1),     // Minimum valid port
            PortExclusion::Single(65535), // Maximum valid port
        ];
        let manager = ExclusionManager::from_config(&exclusions).unwrap();

        assert_eq!(manager.len(), 2);
        assert!(manager.is_excluded(Port::try_from(1).unwrap()));
        assert!(manager.is_excluded(Port::try_from(65535).unwrap()));
    }

    #[test]
    fn test_compaction_preserves_coverage() {
        // Test that compaction doesn't change which ports are excluded
        // This is an invariant test: compaction should only change representation
        let mut manager = ExclusionManager::empty();
        for port in 5000..=5002 {
            manager.add_port(Port::try_from(port).unwrap());
        }
        manager.add_port(Port::try_from(5005).unwrap());
        for port in 5010..=5012 {
            manager.add_port(Port::try_from(port).unwrap());
        }

        // Save pre-compaction state
        let original_exclusions: Vec<Port> = manager.iter().collect();

        // Compact and reconstruct
        let compacted = manager.compact();
        let new_manager = ExclusionManager::from_config(&compacted).unwrap();

        // Verify same ports are excluded
        let new_exclusions: Vec<Port> = new_manager.iter().collect();
        assert_eq!(original_exclusions, new_exclusions);
    }

    #[test]
    fn test_add_range_large_range() {
        // Test adding a large range to verify performance characteristics
        // Ensures the implementation can handle large exclusion lists
        let mut manager = ExclusionManager::empty();
        let range = PortRange::new(
            Port::try_from(10000).unwrap(),
            Port::try_from(20000).unwrap(),
        )
        .unwrap();

        manager.add_range(&range);

        // Should have 10,001 ports (10000-20000 inclusive)
        assert_eq!(manager.len(), 10001);
        assert!(manager.is_excluded(Port::try_from(10000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(15000).unwrap()));
        assert!(manager.is_excluded(Port::try_from(20000).unwrap()));
        assert!(!manager.is_excluded(Port::try_from(20001).unwrap()));
    }

    // ------------------------------------------------------------------
    // Glue tests for the verified kernel (boundaries, error mapping, and
    // agreement with the previous `BTreeSet` implementation).
    // ------------------------------------------------------------------

    fn single(port: u16) -> PortExclusion {
        PortExclusion::Single(port)
    }

    fn range(start: u16, end: u16) -> PortExclusion {
        PortExclusion::Range { start, end }
    }

    /// The previous implementation: materialize every port, then sweep the
    /// sorted set into maximal runs. Valid input only.
    fn reference_ports(exclusions: &[PortExclusion]) -> std::collections::BTreeSet<u16> {
        let mut ports = std::collections::BTreeSet::new();
        for exclusion in exclusions {
            match *exclusion {
                PortExclusion::Single(port) => {
                    ports.insert(port);
                }
                PortExclusion::Range { start, end } => ports.extend(start..=end),
            }
        }
        ports
    }

    fn reference_compact(exclusions: &[PortExclusion]) -> Vec<PortExclusion> {
        let mut runs: Vec<(u16, u16)> = Vec::new();
        for port in reference_ports(exclusions) {
            match runs.last_mut() {
                Some((_, end)) if u32::from(*end) + 1 == u32::from(port) => *end = port,
                _ => runs.push((port, port)),
            }
        }
        runs.into_iter()
            .map(|(start, end)| {
                if start == end {
                    single(start)
                } else {
                    range(start, end)
                }
            })
            .collect()
    }

    /// Deterministic pseudo-random valid exclusion lists, biased towards
    /// overlaps, adjacency, duplicates, and the ends of the port space.
    fn sample_lists() -> Vec<Vec<PortExclusion>> {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        let mut lists = Vec::new();
        for case in 0..400 {
            let base: u16 = match case % 4 {
                0 => 1,
                1 => 65_500,
                2 => 5000,
                _ => u16::try_from(1 + next(65_000)).unwrap(),
            };
            let len = next(8);
            let mut list = Vec::new();
            for _ in 0..len {
                let start = base.saturating_add(u16::try_from(next(40)).unwrap()).max(1);
                let width = u16::try_from(next(6)).unwrap();
                let end = start.saturating_add(width);
                if next(3) == 0 {
                    list.push(single(start));
                } else {
                    list.push(range(start, end));
                }
            }
            lists.push(list);
        }
        lists
    }

    #[test]
    fn test_kernel_agrees_with_previous_implementation() {
        for list in sample_lists() {
            let manager = ExclusionManager::from_config(&list).unwrap();
            let expected_ports = reference_ports(&list);
            let actual_ports: Vec<u16> = manager.iter().map(Port::value).collect();
            assert_eq!(
                actual_ports,
                expected_ports.iter().copied().collect::<Vec<_>>(),
                "{list:?}"
            );
            assert_eq!(manager.len(), expected_ports.len(), "{list:?}");
            assert_eq!(manager.is_empty(), expected_ports.is_empty(), "{list:?}");
            assert_eq!(manager.compact(), reference_compact(&list), "{list:?}");
            assert_eq!(
                compact_exclusions(&list).unwrap(),
                reference_compact(&list),
                "{list:?}"
            );
            for port in [1, 2, 4999, 5000, 5001, 5039, 5045, 65_500, 65_534, 65_535] {
                let port = Port::try_from(port).unwrap();
                assert_eq!(
                    manager.is_excluded(port),
                    expected_ports.contains(&port.value()),
                    "{list:?} {port}"
                );
            }
            let query = PortRange::new(
                Port::try_from(5010).unwrap(),
                Port::try_from(65_510).unwrap(),
            )
            .unwrap();
            let in_range: Vec<u16> = manager
                .excluded_in_range(&query)
                .into_iter()
                .map(Port::value)
                .collect();
            let expected: Vec<u16> = expected_ports.range(5010..=65_510).copied().collect();
            assert_eq!(in_range, expected, "{list:?}");
        }
    }

    #[test]
    fn test_incremental_inserts_agree_with_from_config() {
        for list in sample_lists() {
            let mut manager = ExclusionManager::empty();
            for exclusion in &list {
                manager.add_exclusion(exclusion).unwrap();
            }
            assert_eq!(manager.compact(), reference_compact(&list), "{list:?}");
        }
    }

    #[test]
    fn test_full_port_space_boundaries() {
        let manager = ExclusionManager::from_config(&[range(1, 65_535)]).unwrap();
        assert_eq!(manager.len(), 65_535);
        assert_eq!(manager.iter().next().map(Port::value), Some(1));
        assert_eq!(manager.iter().last().map(Port::value), Some(65_535));
        assert_eq!(manager.iter().count(), 65_535);
        assert_eq!(manager.compact(), vec![range(1, 65_535)]);

        let edges = ExclusionManager::from_config(&[single(65_535), single(1)]).unwrap();
        assert_eq!(edges.compact(), vec![single(1), single(65_535)]);
        let all =
            PortRange::new(Port::try_from(1).unwrap(), Port::try_from(65_535).unwrap()).unwrap();
        let members: Vec<u16> = edges
            .excluded_in_range(&all)
            .into_iter()
            .map(Port::value)
            .collect();
        assert_eq!(members, vec![1, 65_535]);
        let single_port = PortRange::new(
            Port::try_from(65_535).unwrap(),
            Port::try_from(65_535).unwrap(),
        )
        .unwrap();
        assert_eq!(
            edges.excluded_in_range(&single_port),
            vec![Port::try_from(65_535).unwrap()]
        );

        // A one-port range compacts to a single port.
        assert_eq!(
            compact_exclusions(&[range(5000, 5000)]).unwrap(),
            [single(5000)]
        );
        assert_eq!(
            compact_exclusions(&[]).unwrap(),
            Vec::<PortExclusion>::new()
        );
    }

    #[test]
    fn test_from_config_errors_match_port_and_range_constructors() {
        // The kernel's error is mapped back to exactly the errors the
        // previous `Port::try_from` / `PortRange::new` calls produced.
        let zero = Error::from(Port::try_from(0).unwrap_err()).to_string();
        for invalid in [single(0), range(0, 5000), range(5000, 0), range(0, 0)] {
            let err = ExclusionManager::from_config(&[single(80), invalid.clone()]).unwrap_err();
            assert!(
                matches!(err, Error::InvalidPort { value: 0, .. }),
                "{invalid:?}: {err:?}"
            );
            assert_eq!(err.to_string(), zero, "{invalid:?}");
        }

        let reversed = Error::from(
            PortRange::new(Port::try_from(5010).unwrap(), Port::try_from(5000).unwrap())
                .unwrap_err(),
        )
        .to_string();
        let err = ExclusionManager::from_config(&[range(5010, 5000)]).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidPortRange {
                min: 5010,
                max: 5000,
                ..
            }
        ));
        assert_eq!(err.to_string(), reversed);
    }

    #[test]
    fn test_add_exclusion_is_atomic_on_error() {
        let mut manager = ExclusionManager::from_config(&[range(5000, 5002)]).unwrap();
        for invalid in [single(0), range(5010, 5000), range(0, 5005), range(5003, 0)] {
            assert!(manager.add_exclusion(&invalid).is_err(), "{invalid:?}");
            assert_eq!(manager.compact(), vec![range(5000, 5002)], "{invalid:?}");
            assert_eq!(manager.len(), 3);
        }
        // Valid insertions merge with adjacent runs.
        manager.add_exclusion(&single(5003)).unwrap();
        manager.add_exclusion(&range(4998, 4999)).unwrap();
        assert_eq!(manager.compact(), vec![range(4998, 5003)]);
    }

    #[test]
    fn test_validate_exclusions_reports_first_invalid_entry_with_precedence() {
        let message = |exclusions: &[PortExclusion]| match validate_exclusions(exclusions) {
            Err(Error::Validation { field, message }) => format!("{field}: {message}"),
            other => panic!("expected a validation error, got {other:?}"),
        };
        assert!(validate_exclusions(&[]).is_ok());
        assert!(validate_exclusions(&[single(1), single(65_535), range(1, 65_535)]).is_ok());
        assert!(validate_exclusions(&[range(5000, 5000)]).is_ok());
        assert_eq!(message(&[single(0)]), "excluded_ports[0]: Invalid port: 0");
        assert_eq!(
            message(&[single(1), range(0, 0)]),
            "excluded_ports[1]: Invalid start port: 0"
        );
        assert_eq!(
            message(&[range(0, 5)]),
            "excluded_ports[0]: Invalid start port: 0"
        );
        assert_eq!(
            message(&[range(5, 0)]),
            "excluded_ports[0]: Invalid end port: 0"
        );
        assert_eq!(
            message(&[single(5), range(5010, 5000), single(0)]),
            "excluded_ports[1]: Invalid range: 5010..5000 (end < start)"
        );
    }

    /// Regression: compaction used to drop reversed ranges (changing the
    /// excluded set) and keep port 0. It now rejects such lists.
    #[test]
    fn test_compact_exclusions_rejects_invalid_entries() {
        let err = compact_exclusions(&[range(5010, 5000), single(6000)]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "validation error for 'excluded_ports[0]': Invalid range: 5010..5000 (end < start)"
        );
        assert!(compact_exclusions(&[single(0), single(1)]).is_err());
        assert!(compact_exclusions(&[single(6000), range(0, 3)]).is_err());
    }

    /// Compaction sorts the entries and merges them in one pass, so long
    /// lists in any order compact quickly (the previous construction copied
    /// the run list once per entry). Every valid port as a single, in a
    /// scrambled order, compacts to the full range; tens of thousands of
    /// disjoint singles in descending order compact to the ascending list.
    #[test]
    fn test_compaction_of_long_lists_in_any_order() {
        // 7919 is prime, so `k * 7919 mod 65535` visits every residue once.
        let scrambled: Vec<PortExclusion> = (0..65_535_u32)
            .map(|k| single(u16::try_from(k * 7919 % 65_535 + 1).unwrap()))
            .collect();
        assert_eq!(
            compact_exclusions(&scrambled).unwrap(),
            vec![range(1, 65_535)]
        );
        assert_eq!(
            ExclusionManager::from_config(&scrambled).unwrap().len(),
            65_535
        );

        let descending: Vec<PortExclusion> =
            (1..=32_767_u16).rev().map(|k| single(2 * k)).collect();
        let mut ascending = descending.clone();
        ascending.reverse();
        assert_eq!(compact_exclusions(&descending).unwrap(), ascending);
        assert!(is_compact(&ascending));
        assert!(!is_compact(&descending));
    }

    #[test]
    fn test_compaction_is_order_independent_and_idempotent() {
        let list = vec![
            single(5005),
            range(5000, 5003),
            single(5004),
            range(5001, 5002),
            single(7000),
            single(7000),
        ];
        let mut reversed = list.clone();
        reversed.reverse();
        let compacted = compact_exclusions(&list).unwrap();
        assert_eq!(compacted, vec![range(5000, 5005), single(7000)]);
        assert_eq!(compact_exclusions(&reversed).unwrap(), compacted);
        assert_eq!(compact_exclusions(&compacted).unwrap(), compacted);
    }
}
