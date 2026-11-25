//! Verified kernel: configured port bounds (`min`, then `max` or
//! `max_offset`) -- their validation, resolution, layering, and defaults.
//!
//! A port configuration names its range as `min` plus either an explicit
//! `max` or a `max_offset` (`max = min + max_offset`). This kernel owns every
//! decision about such bounds:
//!
//! - [`resolve_port_bounds`] -- the *resolution* every consumer of a loaded
//!   configuration uses to obtain the range (the allocator, `trop scan`,
//!   `reserve-group`). It is lenient: an explicit `max` wins over
//!   `max_offset`, and a zero offset yields the single-port range `min..=min`.
//! - [`check_port_bounds`] -- the *validation* rules a configuration document
//!   must satisfy (`ConfigValidator::validate_port_config`): `max` and
//!   `max_offset` are mutually exclusive and the offset is strictly positive.
//! - [`overlay_port_bounds`] -- how one configuration layer (a YAML document,
//!   the environment, a programmatic override, or the command line) is laid
//!   over the bounds below it. A layer's `max_offset` is resolved against the
//!   `min` in effect *at that layer* into an absolute `max`, so a higher
//!   layer that changes only `min` never moves a lower layer's upper bound.
//! - [`check_upper_declaration`] and [`check_override_bounds`] -- the checks
//!   the environment and command-line layers apply to themselves.
//! - [`group_scan_span`] and [`group_base_outside_range`] -- the
//!   `reservations.base` rules: the base must lie in the range, and rebasing
//!   only narrows it.
//! - [`DEFAULT_MIN_PORT`], [`DEFAULT_MAX_PORT`], and [`default_port_bounds`]
//!   -- the built-in bounds, proved valid.
//!
//! The lemmas relate the pieces: every validated document that names an upper
//! bound resolves to exactly the validated range; every effective section
//! that passes validation holds exactly one, absolute upper bound (and so does
//! every section built from layers whose declarations resolve);
//! `{min: m, max_offset: o}` and `{min: m, max: m + o}` are interchangeable in
//! every layer stack; and layering is associative.
//!
//! Production glue: `crate::port::allocator::port_range_from_config` (and
//! `port_span_from_config`),
//! `crate::config::validator`, `crate::config::merger`,
//! `crate::config::effective` (`apply_precise`), `crate::config::environment`,
//! `crate::config::schema` (defaults and `PortConfig::check_command_line_bounds`),
//! and `crate::operations::reserve_group`.

use vstd::prelude::*;

verus! {

use super::port::PortSpan;
// Spec-only items are erased from ordinary builds, so they are referred to by
// path (`super::port::is_valid_port`) rather than imported.

// ---------------------------------------------------------------------------
// Specifications
// ---------------------------------------------------------------------------

/// The upper bound a configuration asks for, in unbounded arithmetic: an
/// explicit `max` wins; otherwise `min + max_offset`; otherwise there is none.
pub open spec fn requested_max(min: u16, max: Option<u16>, max_offset: Option<u16>) -> Option<int> {
    match max {
        Some(m) => Some(m as int),
        None => match max_offset {
            Some(o) => Some(min + o),
            None => None,
        },
    }
}

/// Resolution succeeds: the configuration names an upper bound, and
/// `min..=that bound` is a valid, non-empty port range.
pub open spec fn resolvable(min: u16, max: Option<u16>, max_offset: Option<u16>) -> bool {
    match requested_max(min, max, max_offset) {
        Some(hi) => super::port::is_valid_range(min as int, hi),
        None => false,
    }
}

/// The error the resolution reports: the first failing check, in the order
/// `allocator_from_config` has always applied them (`min`, then the upper
/// bound's presence and overflow, then the upper bound itself, then
/// `min <= max`). `None` when there is no error.
pub open spec fn resolution_error(min: u16, max: Option<u16>, max_offset: Option<u16>) -> Option<
    PortBoundsError,
> {
    if min == 0 {
        Some(PortBoundsError::InvalidMin)
    } else {
        match max {
            Some(m) => if m == 0 {
                Some(PortBoundsError::InvalidMax)
            } else if m < min {
                Some(PortBoundsError::MaxBelowMin { max: m })
            } else {
                None
            },
            None => match max_offset {
                Some(o) => if min + o > 65535 {
                    Some(PortBoundsError::OffsetOverflow { offset: o })
                } else {
                    None
                },
                None => Some(PortBoundsError::MissingMax),
            },
        }
    }
}

/// The configuration satisfies the validation rules: `min` is a valid port;
/// an explicit `max` is a valid port `>= min`; a `max_offset` is strictly
/// positive and lands on a valid port; and `max` and `max_offset` are not both
/// given. (Giving neither is allowed by validation.)
pub open spec fn strictly_valid(min: u16, max: Option<u16>, max_offset: Option<u16>) -> bool {
    &&& super::port::is_valid_port(min as int)
    &&& (max matches Some(m) ==> super::port::is_valid_range(min as int, m as int))
    &&& (max_offset matches Some(o) ==> o >= 1 && super::port::is_valid_range(min as int, min + o))
    &&& !(max is Some && max_offset is Some)
}

/// The violation validation reports: the first failing rule, in the order of
/// `ConfigValidator::validate_port_config` (`min`; `max` valid; `max >= min`;
/// offset positive; offset does not overflow; not both). `None` when valid.
pub open spec fn validation_error(min: u16, max: Option<u16>, max_offset: Option<u16>) -> Option<
    PortBoundsViolation,
> {
    if min == 0 {
        Some(PortBoundsViolation::InvalidMin)
    } else if max == Some(0u16) {
        Some(PortBoundsViolation::InvalidMax)
    } else if (match max {
        Some(m) => m < min,
        None => false,
    }) {
        Some(PortBoundsViolation::MaxBelowMin)
    } else if max_offset == Some(0u16) {
        Some(PortBoundsViolation::ZeroOffset)
    } else if (match max_offset {
        Some(o) => min + o > 65535,
        None => false,
    }) {
        Some(PortBoundsViolation::OffsetOverflow { offset: max_offset->0 })
    } else if max is Some && max_offset is Some {
        Some(PortBoundsViolation::BothMaxAndOffset)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Resolution (used by the allocator and `trop scan`)
// ---------------------------------------------------------------------------

/// Why a configuration's bounds cannot be resolved to a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortBoundsError {
    /// `min` is 0.
    InvalidMin,
    /// The explicit `max` is 0.
    InvalidMax,
    /// The explicit `max` lies below `min` (an offset never does).
    MaxBelowMin {
        /// The explicit `max`.
        max: u16,
    },
    /// `min + max_offset` exceeds 65535 (and no explicit `max` was given).
    OffsetOverflow {
        /// The offending `max_offset`.
        offset: u16,
    },
    /// Neither `max` nor `max_offset` is given.
    MissingMax,
}

/// Resolves `min` and `max` / `max_offset` to a port range.
///
/// Succeeds exactly when the requested bounds form a valid range, and then
/// returns precisely `min..=requested_max`. `min + max_offset` is computed
/// without overflow. Errors follow `resolution_error`'s precedence.
pub fn resolve_port_bounds(min: u16, max: Option<u16>, max_offset: Option<u16>) -> (r: Result<
    PortSpan,
    PortBoundsError,
>)
    ensures
        r is Ok <==> resolvable(min, max, max_offset),
        r matches Ok(span) ==> span.spec_min() == min && requested_max(min, max, max_offset)
            == Some(span.spec_max() as int),
        r matches Err(e) ==> resolution_error(min, max, max_offset) == Some(e),
{
    if min == 0 {
        return Err(PortBoundsError::InvalidMin);
    }
    let hi = match max {
        Some(m) => m,
        None => match max_offset {
            Some(offset) => match min.checked_add(offset) {
                Some(sum) => sum,
                None => return Err(PortBoundsError::OffsetOverflow { offset }),
            },
            None => return Err(PortBoundsError::MissingMax),
        },
    };
    if hi == 0 {
        return Err(PortBoundsError::InvalidMax);
    }
    match PortSpan::new(min, hi) {
        Some(span) => Ok(span),
        None => Err(PortBoundsError::MaxBelowMin { max: hi }),
    }
}

// ---------------------------------------------------------------------------
// Validation rules (for `ConfigValidator::validate_port_config`)
// ---------------------------------------------------------------------------

/// A configuration-file rule the port bounds violate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortBoundsViolation {
    /// `min` is 0.
    InvalidMin,
    /// The explicit `max` is 0.
    InvalidMax,
    /// The explicit `max` lies below `min`.
    MaxBelowMin,
    /// `max_offset` is 0.
    ZeroOffset,
    /// `min + max_offset` exceeds 65535.
    OffsetOverflow {
        /// The offending `max_offset`.
        offset: u16,
    },
    /// Both `max` and `max_offset` are given.
    BothMaxAndOffset,
}

/// Checks the configuration-file rules for port bounds.
///
/// Succeeds exactly for `strictly_valid` bounds, returning the range they
/// name (`None` when neither `max` nor `max_offset` is given, which the rules
/// allow). Violations follow `validation_error`'s precedence, which is the
/// order `ConfigValidator::validate_port_config` has always reported.
pub fn check_port_bounds(min: u16, max: Option<u16>, max_offset: Option<u16>) -> (r: Result<
    Option<PortSpan>,
    PortBoundsViolation,
>)
    ensures
        r is Ok <==> strictly_valid(min, max, max_offset),
        r matches Ok(s) ==> (s is Some <==> requested_max(min, max, max_offset) is Some),
        r matches Ok(Some(span)) ==> span.spec_min() == min && requested_max(min, max, max_offset)
            == Some(span.spec_max() as int),
        r matches Err(e) ==> validation_error(min, max, max_offset) == Some(e),
{
    if min == 0 {
        return Err(PortBoundsViolation::InvalidMin);
    }
    if let Some(m) = max {
        if m == 0 {
            return Err(PortBoundsViolation::InvalidMax);
        }
        if m < min {
            return Err(PortBoundsViolation::MaxBelowMin);
        }
    }
    let mut offset_max: Option<u16> = None;
    if let Some(offset) = max_offset {
        if offset == 0 {
            return Err(PortBoundsViolation::ZeroOffset);
        }
        match min.checked_add(offset) {
            Some(sum) => offset_max = Some(sum),
            None => return Err(PortBoundsViolation::OffsetOverflow { offset }),
        }
    }
    if max.is_some() && max_offset.is_some() {
        return Err(PortBoundsViolation::BothMaxAndOffset);
    }
    match max {
        Some(m) => Ok(PortSpan::new(min, m)),
        None => match offset_max {
            Some(sum) => Ok(PortSpan::new(min, sum)),
            None => Ok(None),
        },
    }
}

// ---------------------------------------------------------------------------
// Built-in defaults
// ---------------------------------------------------------------------------

/// The built-in lower bound of the allocation range.
pub const DEFAULT_MIN_PORT: u16 = 5000;

/// The built-in upper bound of the allocation range.
pub const DEFAULT_MAX_PORT: u16 = 7000;

/// The built-in `ports` section, `{min: 5000, max: 7000}`: canonical (one
/// absolute upper bound) and strictly valid, naming `5000..=7000` (see
/// `lemma_defaults_valid`).
pub const fn default_port_bounds() -> (b: PortBounds)
    ensures
        b == (PortBounds { min: DEFAULT_MIN_PORT, max: Some(DEFAULT_MAX_PORT), max_offset: None }),
        canonical(b),
        strictly_valid(b.min, b.max, b.max_offset),
        upper_of(b) == Some(7000int),
{
    PortBounds { min: DEFAULT_MIN_PORT, max: Some(DEFAULT_MAX_PORT), max_offset: None }
}

// ---------------------------------------------------------------------------
// Layering (YAML documents, environment, programmatic and command-line
// overrides)
// ---------------------------------------------------------------------------

/// A `ports` section: `min`, plus an explicit `max` and/or a `max_offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortBounds {
    /// `ports.min`.
    pub min: u16,
    /// `ports.max`.
    pub max: Option<u16>,
    /// `ports.max_offset` (relative to `min`).
    pub max_offset: Option<u16>,
}

/// The port leaves one configuration layer sets; `None` leaves the value
/// below in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortLayer {
    /// The layer's `min`, if it sets one.
    pub min: Option<u16>,
    /// The layer's explicit `max`, if it sets one.
    pub max: Option<u16>,
    /// The layer's `max_offset`, if it sets one.
    pub max_offset: Option<u16>,
}

/// Which leaves a layer set (provenance records these against the layer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerLeaves {
    /// The layer set `min`.
    pub min: bool,
    /// The layer set an upper bound (`max` or `max_offset`); either one
    /// replaces both upper leaves below.
    pub upper: bool,
}

/// `offset` lands on a valid port above `anchor`: it is strictly positive
/// and `anchor + offset <= 65535`.
pub open spec fn offset_resolves(anchor: u16, offset: u16) -> bool {
    offset >= 1 && anchor + offset <= 65535
}

/// A lone `max_offset` that cannot be resolved to a port at the section's own
/// `min` (it is 0, or `min + max_offset` exceeds 65535).
pub open spec fn unresolvable_offset(b: PortBounds) -> bool {
    b.max is None && b.max_offset is Some && !offset_resolves(b.min, b.max_offset->0)
}

/// The section with a lone, resolvable `max_offset` replaced by the absolute
/// `max` it names (`min + max_offset`). Every other shape is kept as is, so
/// validation still reports it.
pub open spec fn normalized(b: PortBounds) -> PortBounds {
    if b.max is None && b.max_offset is Some && offset_resolves(b.min, b.max_offset->0) {
        PortBounds { min: b.min, max: Some((b.min + b.max_offset->0) as u16), max_offset: None }
    } else {
        b
    }
}

/// Canonical bounds: exactly one upper bound, held as an absolute `max`.
pub open spec fn canonical(b: PortBounds) -> bool {
    b.max is Some && b.max_offset is None
}

/// The upper bound a section names, in unbounded arithmetic (an explicit
/// `max` wins; otherwise `min + max_offset`).
pub open spec fn upper_of(b: PortBounds) -> Option<int> {
    requested_max(b.min, b.max, b.max_offset)
}

/// The `min` in effect at a layer: its own, otherwise the one below.
pub open spec fn layer_anchor(lower: PortBounds, layer: PortLayer) -> u16 {
    match layer.min {
        Some(m) => m,
        None => lower.min,
    }
}

/// The layer sets an upper bound.
pub open spec fn declares_upper(layer: PortLayer) -> bool {
    layer.max is Some || layer.max_offset is Some
}

/// An upper-bound declaration that resolves at `anchor`: not both forms, and a
/// `max_offset` lands on a valid port above `anchor`.
pub open spec fn upper_declaration_ok(anchor: u16, max: Option<u16>, max_offset: Option<u16>) -> bool {
    &&& !(max is Some && max_offset is Some)
    &&& (max_offset matches Some(o) ==> offset_resolves(anchor, o))
}

/// The first upper-bound rule a declaration at `anchor` violates, in the
/// order of `validation_error` (offset positive; offset does not overflow;
/// not both). `None` when the declaration resolves.
pub open spec fn upper_declaration_error(
    anchor: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
) -> Option<PortBoundsViolation> {
    if max_offset == Some(0u16) {
        Some(PortBoundsViolation::ZeroOffset)
    } else if (match max_offset {
        Some(o) => anchor + o > 65535,
        None => false,
    }) {
        Some(PortBoundsViolation::OffsetOverflow { offset: max_offset->0 })
    } else if max is Some && max_offset is Some {
        Some(PortBoundsViolation::BothMaxAndOffset)
    } else {
        None
    }
}

/// Laying `layer` over the section `lower`.
///
/// - `min` is the layer's, otherwise the one below (the *anchor*).
/// - A layer that sets an upper bound replaces both upper leaves below; its
///   `max_offset` is anchored at the `min` in effect at the layer.
/// - A layer that sets no upper bound keeps the one below *where it is*: a
///   resolvable `max_offset` below is first fixed to the absolute `max` it
///   named at its own `min`, so changing `min` never moves it.
/// - The result is normalized: a resolvable lone `max_offset` is stored as the
///   absolute `max`. Shapes validation rejects (both forms, a zero or
///   overflowing offset) are kept as they are, so validation still reports
///   them.
pub open spec fn overlay(lower: PortBounds, layer: PortLayer) -> PortBounds {
    let anchor = layer_anchor(lower, layer);
    if declares_upper(layer) {
        normalized(PortBounds { min: anchor, max: layer.max, max_offset: layer.max_offset })
    } else {
        let inherited = normalized(lower);
        normalized(PortBounds { min: anchor, max: inherited.max, max_offset: inherited.max_offset })
    }
}

/// Laying `layers` over `start`, lowest precedence first.
pub open spec fn overlay_all(start: PortBounds, layers: Seq<PortLayer>) -> PortBounds
    decreases layers.len(),
{
    if layers.len() == 0 {
        start
    } else {
        overlay(overlay_all(start, layers.drop_last()), layers.last())
    }
}

/// Every layer's upper-bound declaration resolves at the `min` in effect at
/// that layer.
pub open spec fn declarations_ok(start: PortBounds, layers: Seq<PortLayer>) -> bool
    decreases layers.len(),
{
    if layers.len() == 0 {
        true
    } else {
        let below = overlay_all(start, layers.drop_last());
        let top = layers.last();
        declarations_ok(start, layers.drop_last()) && upper_declaration_ok(
            layer_anchor(below, top),
            top.max,
            top.max_offset,
        )
    }
}

/// `normalized`, executably.
fn normalize(b: PortBounds) -> (r: PortBounds)
    ensures
        r == normalized(b),
{
    match b.max_offset {
        Some(offset) if b.max.is_none() && offset >= 1 => match b.min.checked_add(offset) {
            Some(sum) => PortBounds { min: b.min, max: Some(sum), max_offset: None },
            None => b,
        },
        _ => b,
    }
}

/// Lays one configuration layer over the port section below it, and reports
/// which leaves the layer set.
///
/// The result is exactly `overlay(lower, layer)`. In particular:
/// - its `min` is the layer's `min`, otherwise the one below;
/// - a layer that sets an upper bound determines it, anchored at that `min`;
/// - a layer that sets no upper bound keeps the value of the one below (it
///   is never re-anchored to a new `min`), unless that one is a lone
///   `max_offset` that never resolved;
/// - a canonical section stays canonical under every layer whose declaration
///   resolves, and a declaration that does not resolve always fails
///   validation of the result;
/// - no `max_offset` that resolves survives: it is stored as the absolute
///   `max`.
pub fn overlay_port_bounds(lower: PortBounds, layer: PortLayer) -> (r: (PortBounds, LayerLeaves))
    ensures
        r.0 == overlay(lower, layer),
        r.1.min == (layer.min is Some),
        r.1.upper == declares_upper(layer),
        r.0.min == layer_anchor(lower, layer),
        declares_upper(layer) ==> upper_of(r.0) == requested_max(
            layer_anchor(lower, layer),
            layer.max,
            layer.max_offset,
        ),
        !declares_upper(layer) && !unresolvable_offset(lower) ==> upper_of(r.0) == upper_of(lower),
        !declares_upper(layer) && canonical(lower) ==> r.0 == (PortBounds {
            min: layer_anchor(lower, layer),
            max: lower.max,
            max_offset: None,
        }),
        canonical(lower) && upper_declaration_ok(
            layer_anchor(lower, layer),
            layer.max,
            layer.max_offset,
        ) ==> canonical(r.0),
        !upper_declaration_ok(layer_anchor(lower, layer), layer.max, layer.max_offset)
            ==> !strictly_valid(r.0.min, r.0.max, r.0.max_offset),
        normalized(r.0) == r.0,
{
    proof {
        lemma_overlay_properties(lower, layer);
    }
    let anchor = match layer.min {
        Some(m) => m,
        None => lower.min,
    };
    let upper = layer.max.is_some() || layer.max_offset.is_some();
    let bounds = if upper {
        normalize(PortBounds { min: anchor, max: layer.max, max_offset: layer.max_offset })
    } else {
        let inherited = normalize(lower);
        normalize(PortBounds { min: anchor, max: inherited.max, max_offset: inherited.max_offset })
    };
    (bounds, LayerLeaves { min: layer.min.is_some(), upper })
}

/// Checks that a layer's upper-bound declaration resolves at `anchor` (the
/// `min` in effect at that layer). The environment layer applies this to
/// itself, as validation does to every YAML document.
///
/// Succeeds exactly when `upper_declaration_ok`; otherwise reports
/// `upper_declaration_error`, which is the error `ConfigValidator` reports
/// for the same declaration in a document with that `min`
/// (`lemma_upper_declaration_agrees_with_validation`).
pub fn check_upper_declaration(anchor: u16, max: Option<u16>, max_offset: Option<u16>) -> (r: Result<
    (),
    PortBoundsViolation,
>)
    ensures
        r is Ok <==> upper_declaration_ok(anchor, max, max_offset),
        r is Ok <==> upper_declaration_error(anchor, max, max_offset) is None,
        r matches Err(e) ==> upper_declaration_error(anchor, max, max_offset) == Some(e),
{
    if let Some(offset) = max_offset {
        if offset == 0 {
            return Err(PortBoundsViolation::ZeroOffset);
        }
        if anchor.checked_add(offset).is_none() {
            return Err(PortBoundsViolation::OffsetOverflow { offset });
        }
        if max.is_some() {
            return Err(PortBoundsViolation::BothMaxAndOffset);
        }
    }
    Ok(())
}

/// Command-line `--min` / `--max` overrides are acceptable: every given value
/// is a valid port and, when both are given, `min <= max`.
pub open spec fn override_ok(min: Option<u16>, max: Option<u16>) -> bool {
    &&& (min matches Some(a) ==> super::port::is_valid_port(a as int))
    &&& (max matches Some(b) ==> super::port::is_valid_port(b as int))
    &&& (min is Some && max is Some ==> min->0 <= max->0)
}

/// The first rule command-line overrides violate: `--min` is 0, `--max` is 0,
/// then `--max < --min`. `None` when acceptable.
pub open spec fn override_error(min: Option<u16>, max: Option<u16>) -> Option<PortBoundsViolation> {
    if min == Some(0u16) {
        Some(PortBoundsViolation::InvalidMin)
    } else if max == Some(0u16) {
        Some(PortBoundsViolation::InvalidMax)
    } else if min is Some && max is Some && max->0 < min->0 {
        Some(PortBoundsViolation::MaxBelowMin)
    } else {
        None
    }
}

/// Checks command-line `--min` / `--max` overrides (`trop reserve`,
/// `trop scan`) before they are laid over the configuration as the layer
/// `{min, max}` (which never sets `max_offset`).
///
/// Succeeds exactly for `override_ok`; otherwise reports `override_error`.
/// It applies the document rules to whatever the user typed
/// (`lemma_override_matches_document_rules`), and an accepted pair always
/// becomes exactly the effective range `min..=max`
/// (`lemma_override_pair_is_effective_range`).
pub fn check_override_bounds(min: Option<u16>, max: Option<u16>) -> (r: Result<(), PortBoundsViolation>)
    ensures
        r is Ok <==> override_ok(min, max),
        r is Ok <==> override_error(min, max) is None,
        r matches Err(e) ==> override_error(min, max) == Some(e),
{
    if let Some(a) = min {
        if a == 0 {
            return Err(PortBoundsViolation::InvalidMin);
        }
    }
    if let Some(b) = max {
        if b == 0 {
            return Err(PortBoundsViolation::InvalidMax);
        }
        if let Some(a) = min {
            if b < a {
                return Err(PortBoundsViolation::MaxBelowMin);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `reservations.base`
// ---------------------------------------------------------------------------

/// The scan range of a reservation group whose configured range is `span`.
///
/// Without a base the configured range is used. With base `b` the scan runs
/// over `b..=span.max` -- exactly the ports of `span` that are `>= b`, in
/// order -- and it exists exactly when `b` lies in `span`. The upper bound is
/// always the configured one (never `b + max_offset`), so rebasing only
/// narrows the range.
pub fn group_scan_span(base: Option<u16>, span: PortSpan) -> (r: Option<PortSpan>)
    ensures
        base is None ==> r == Some(span),
        base matches Some(b) ==> (r is Some <==> span.spec_min() <= b <= span.spec_max()),
        base is Some && r is Some ==> r->0.spec_min() == base->0,
        r matches Some(s) ==> s.spec_max() == span.spec_max() && span.spec_min() <= s.spec_min(),
        r matches Some(s) ==> s.ports() == span.ports().subrange(
            s.spec_min() - span.spec_min(),
            span.ports().len() as int,
        ),
{
    let Some(b) = base else {
        return Some(span);
    };
    let lo = span.min();
    let hi = span.max();
    if b < lo || b > hi {
        return None;
    }
    let r = PortSpan::new(b, hi);
    proof {
        lemma_ports_suffix(lo as int, hi as int, b as int);
    }
    r
}

/// A `reservations.base` outside the range its own document names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseOutsideRange {
    /// The base.
    pub base: u16,
    /// The range the document's `ports` section names.
    pub range: PortSpan,
}

/// The document-level `reservations.base` rule: when the same document's
/// `ports` section is strictly valid and names a range, the base must lie in
/// that range. (A section that is invalid, or that names no upper bound, is
/// left to its own checks and to validation of the effective configuration,
/// where every section is canonical.)
///
/// Reports a violation exactly when the base lies outside that range, and
/// then the range is exactly the one `check_port_bounds` validates.
pub fn group_base_outside_range(
    base: Option<u16>,
    min: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
) -> (r: Option<BaseOutsideRange>)
    ensures
        r is Some <==> base is Some && strictly_valid(min, max, max_offset) && (requested_max(
            min,
            max,
            max_offset,
        ) matches Some(hi) && !(min <= base->0 <= hi)),
        r matches Some(v) ==> base == Some(v.base) && v.range.spec_min() == min && requested_max(
            min,
            max,
            max_offset,
        ) == Some(v.range.spec_max() as int),
{
    let b = base?;
    match check_port_bounds(min, max, max_offset) {
        Ok(Some(range)) => match group_scan_span(Some(b), range) {
            Some(_) => None,
            None => Some(BaseOutsideRange { base: b, range }),
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Lemmas
// ---------------------------------------------------------------------------

/// The resolution's error table is consistent with its semantics: it reports
/// no error exactly when the bounds are resolvable.
pub proof fn lemma_resolution_error_iff_unresolvable(
    min: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
)
    ensures
        resolution_error(min, max, max_offset) is None <==> resolvable(min, max, max_offset),
{
}

/// The validation error table is consistent with its semantics: it reports
/// no violation exactly for strictly valid bounds.
pub proof fn lemma_validation_error_iff_invalid(min: u16, max: Option<u16>, max_offset: Option<u16>)
    ensures
        validation_error(min, max, max_offset) is None <==> strictly_valid(min, max, max_offset),
{
}

/// `ConfigValidator::validate_port_config` used to check, after a successful
/// `min.checked_add(offset)`, that the sum is a valid port ("Offset would
/// create invalid max port"). Whenever that check was reached — `min` already
/// validated, the offset non-zero, and no overflow — the sum *is* a valid
/// port, so that branch was dead code; it was removed when the validator
/// adopted `check_port_bounds`.
pub proof fn lemma_validated_offset_max_is_valid_port(min: u16, offset: u16)
    requires
        super::port::is_valid_port(min as int),
        offset != 0,
        min + offset <= 65535,
    ensures
        super::port::is_valid_port(min + offset),
        super::port::is_valid_range(min as int, min + offset),
{
}

/// Every validated configuration that names an upper bound resolves, to the
/// same range validation describes; one that names none fails resolution with
/// `MissingMax` (the only validated shape the allocator rejects).
pub proof fn lemma_valid_bounds_resolve(min: u16, max: Option<u16>, max_offset: Option<u16>)
    requires
        strictly_valid(min, max, max_offset),
    ensures
        requested_max(min, max, max_offset) is Some ==> resolvable(min, max, max_offset)
            && resolution_error(min, max, max_offset) is None,
        requested_max(min, max, max_offset) is None ==> resolution_error(min, max, max_offset)
            == Some(PortBoundsError::MissingMax),
{
}

/// The resolution is more permissive than validation in exactly two ways:
/// when both `max` and `max_offset` are given (the explicit `max` wins), and
/// when `max_offset` is 0 (the range `min..=min`). A resolvable configuration
/// is strictly valid exactly when it has neither shape, and both shapes are
/// resolved whenever the rest of the configuration is valid.
pub proof fn lemma_resolution_leniency(min: u16, max: Option<u16>, max_offset: Option<u16>)
    ensures
        resolvable(min, max, max_offset) ==> (strictly_valid(min, max, max_offset) <==> !((max is Some
            && max_offset is Some) || max_offset == Some(0u16))),
        max is Some && max_offset is Some ==> (resolvable(min, max, max_offset)
            <==> super::port::is_valid_range(min as int, max->0 as int)),
        max is None && max_offset == Some(0u16) ==> (resolvable(min, max, max_offset)
            <==> super::port::is_valid_port(min as int)),
{
}

/// The built-in `ports` section is canonical and passes every rule: it
/// validates, resolves, and names exactly `5000..=7000`.
pub proof fn lemma_defaults_valid()
    ensures
        canonical(
            PortBounds { min: DEFAULT_MIN_PORT, max: Some(DEFAULT_MAX_PORT), max_offset: None },
        ),
        strictly_valid(DEFAULT_MIN_PORT, Some(DEFAULT_MAX_PORT), None),
        validation_error(DEFAULT_MIN_PORT, Some(DEFAULT_MAX_PORT), None) is None,
        resolvable(DEFAULT_MIN_PORT, Some(DEFAULT_MAX_PORT), None),
        resolution_error(DEFAULT_MIN_PORT, Some(DEFAULT_MAX_PORT), None) is None,
        DEFAULT_MIN_PORT == 5000 && DEFAULT_MAX_PORT == 7000,
        requested_max(DEFAULT_MIN_PORT, Some(DEFAULT_MAX_PORT), None) == Some(7000int),
{
}

/// Facts about `normalized`: it keeps `min` and the named upper bound, is
/// idempotent, and changes only a lone resolvable `max_offset` (into a
/// canonical section).
pub proof fn lemma_normalized(b: PortBounds)
    ensures
        normalized(b).min == b.min,
        upper_of(normalized(b)) == upper_of(b),
        normalized(normalized(b)) == normalized(b),
        normalized(b) != b ==> canonical(normalized(b)),
        canonical(b) ==> normalized(b) == b,
        unresolvable_offset(b) ==> normalized(b) == b,
        strictly_valid(b.min, b.max, b.max_offset) ==> strictly_valid(
            normalized(b).min,
            normalized(b).max,
            normalized(b).max_offset,
        ),
{
}

/// The properties `overlay_port_bounds` guarantees, stated on the
/// specification `overlay`.
pub proof fn lemma_overlay_properties(lower: PortBounds, layer: PortLayer)
    ensures
        overlay(lower, layer).min == layer_anchor(lower, layer),
        declares_upper(layer) ==> upper_of(overlay(lower, layer)) == requested_max(
            layer_anchor(lower, layer),
            layer.max,
            layer.max_offset,
        ),
        !declares_upper(layer) && !unresolvable_offset(lower) ==> upper_of(overlay(lower, layer))
            == upper_of(lower),
        !declares_upper(layer) && canonical(lower) ==> overlay(lower, layer) == (PortBounds {
            min: layer_anchor(lower, layer),
            max: lower.max,
            max_offset: None,
        }),
        canonical(lower) && upper_declaration_ok(
            layer_anchor(lower, layer),
            layer.max,
            layer.max_offset,
        ) ==> canonical(overlay(lower, layer)),
        !upper_declaration_ok(layer_anchor(lower, layer), layer.max, layer.max_offset)
            ==> !strictly_valid(
            overlay(lower, layer).min,
            overlay(lower, layer).max,
            overlay(lower, layer).max_offset,
        ),
        normalized(overlay(lower, layer)) == overlay(lower, layer),
{
    let anchor = layer_anchor(lower, layer);
    lemma_normalized(lower);
    lemma_normalized(PortBounds { min: anchor, max: layer.max, max_offset: layer.max_offset });
    let inherited = normalized(lower);
    lemma_normalized(
        PortBounds { min: anchor, max: inherited.max, max_offset: inherited.max_offset },
    );
}

/// Representation independence for one layer: a `max_offset` that resolves
/// at the layer's `min` in effect and the absolute `max` it names are
/// interchangeable. Laying either over any section gives the same section,
/// and the layer sets the same leaves.
pub proof fn lemma_offset_form_interchangeable_in_layer(lower: PortBounds, min: Option<u16>, offset: u16)
    requires
        offset_resolves(layer_anchor(lower, PortLayer { min, max: None, max_offset: Some(offset) }), offset),
    ensures
        ({
            let anchor = layer_anchor(lower, PortLayer { min, max: None, max_offset: Some(offset) });
            let offset_form = PortLayer { min, max: None, max_offset: Some(offset) };
            let max_form = PortLayer { min, max: Some((anchor + offset) as u16), max_offset: None };
            &&& overlay(lower, offset_form) == overlay(lower, max_form)
            &&& declares_upper(offset_form) && declares_upper(max_form)
        }),
{
}

/// A layer that sets nothing changes nothing in a normalized section (every
/// overlay result is one) and is recorded against no leaf.
pub proof fn lemma_empty_layer_is_identity(lower: PortBounds)
    requires
        normalized(lower) == lower,
    ensures
        overlay(lower, PortLayer { min: None, max: None, max_offset: None }) == lower,
        !declares_upper(PortLayer { min: None, max: None, max_offset: None }),
{
    lemma_normalized(lower);
}

/// Document validation predicts the effect of a complete document: a
/// strictly valid `ports` section that names an upper bound, laid over any
/// section at all, yields exactly its own range in canonical form.
pub proof fn lemma_valid_document_layer(
    lower: PortBounds,
    min: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
)
    requires
        strictly_valid(min, max, max_offset),
        requested_max(min, max, max_offset) is Some,
    ensures
        ({
            let r = overlay(lower, PortLayer { min: Some(min), max, max_offset });
            &&& r.min == min
            &&& canonical(r)
            &&& strictly_valid(r.min, r.max, r.max_offset)
            &&& upper_of(r) == requested_max(min, max, max_offset)
        }),
{
    lemma_overlay_properties(lower, PortLayer { min: Some(min), max, max_offset });
}

/// Two layers combined into one: `b`'s `min` if it sets one, otherwise
/// `a`'s; `b`'s upper bound if it sets one, otherwise `a`'s, with a
/// resolvable `max_offset` fixed at `a`'s own `min`.
pub open spec fn compose(a: PortLayer, b: PortLayer) -> PortLayer {
    let min = if b.min is Some {
        b.min
    } else {
        a.min
    };
    if declares_upper(b) {
        PortLayer { min, max: b.max, max_offset: b.max_offset }
    } else if a.min is Some {
        let fixed = normalized(PortBounds { min: a.min->0, max: a.max, max_offset: a.max_offset });
        PortLayer { min, max: fixed.max, max_offset: fixed.max_offset }
    } else {
        PortLayer { min, max: a.max, max_offset: a.max_offset }
    }
}

/// Layering is associative: laying `a` then `b` equals laying their
/// composition, so how sources are grouped (for example
/// `ConfigMerger::merge` over pre-merged documents) does not matter. This
/// needs `a` to anchor any `max_offset` it sets at its own `min` (every YAML
/// document sets `min`), and the section below to hold no unresolvable
/// offset (every section produced from valid layers is canonical).
pub proof fn lemma_overlay_compose(s: PortBounds, a: PortLayer, b: PortLayer)
    requires
        !unresolvable_offset(s),
        a.min is Some || a.max_offset is None,
    ensures
        overlay(overlay(s, a), b) == overlay(s, compose(a, b)),
        declares_upper(compose(a, b)) == (declares_upper(a) || declares_upper(b)),
        (compose(a, b).min is Some) == (a.min is Some || b.min is Some),
{
    lemma_normalized(s);
    lemma_overlay_properties(s, a);
    lemma_overlay_properties(overlay(s, a), b);
    lemma_overlay_properties(s, compose(a, b));
    if a.min is Some {
        lemma_normalized(PortBounds { min: a.min->0, max: a.max, max_offset: a.max_offset });
    }
}

/// Laying `p + q` is laying `p`, then laying `q` over the result.
pub proof fn lemma_overlay_all_append(start: PortBounds, p: Seq<PortLayer>, q: Seq<PortLayer>)
    ensures
        overlay_all(start, p + q) == overlay_all(overlay_all(start, p), q),
    decreases q.len(),
{
    if q.len() == 0 {
        assert(p + q =~= p);
    } else {
        assert((p + q).drop_last() =~= p + q.drop_last());
        assert((p + q).last() == q.last());
        lemma_overlay_all_append(start, p, q.drop_last());
    }
}

/// Representation independence for every layer stack: replacing one layer's
/// `{min: m, max_offset: o}` by `{min: m, max: m + o}` (for any `o` that
/// resolves at `m`) leaves the effective section unchanged, whatever lies
/// below and above it.
pub proof fn lemma_offset_form_interchangeable(
    start: PortBounds,
    below: Seq<PortLayer>,
    above: Seq<PortLayer>,
    m: u16,
    o: u16,
)
    requires
        offset_resolves(m, o),
    ensures
        overlay_all(
            start,
            below + seq![PortLayer { min: Some(m), max: None, max_offset: Some(o) }] + above,
        ) == overlay_all(
            start,
            below + seq![PortLayer { min: Some(m), max: Some((m + o) as u16), max_offset: None }]
                + above,
        ),
{
    let offset_form = PortLayer { min: Some(m), max: None, max_offset: Some(o) };
    let max_form = PortLayer { min: Some(m), max: Some((m + o) as u16), max_offset: None };
    let x = overlay_all(start, below);
    lemma_offset_form_interchangeable_in_layer(x, Some(m), o);
    let with_offset = below + seq![offset_form];
    let with_max = below + seq![max_form];
    assert(with_offset.drop_last() =~= below);
    assert(with_max.drop_last() =~= below);
    assert(with_offset.last() == offset_form);
    assert(with_max.last() == max_form);
    assert(overlay_all(start, with_offset) == overlay(x, offset_form));
    assert(overlay_all(start, with_max) == overlay(x, max_form));
    lemma_overlay_all_append(start, with_offset, above);
    lemma_overlay_all_append(start, with_max, above);
}

/// The section names an upper bound (in either form).
pub open spec fn has_upper(b: PortBounds) -> bool {
    b.max is Some || b.max_offset is Some
}

/// Every effective section that passes validation holds exactly one,
/// absolute upper bound, whatever layers produced it (checked or not): the
/// built-in section has one, no layer removes it, and every overlay result is
/// normalized, so the only strictly valid shape left is canonical. Hence
/// "Either max or max_offset must be specified" can never be reported for an
/// effective configuration.
pub proof fn lemma_validated_overlay_is_canonical(start: PortBounds, layers: Seq<PortLayer>)
    requires
        canonical(start),
        strictly_valid(
            overlay_all(start, layers).min,
            overlay_all(start, layers).max,
            overlay_all(start, layers).max_offset,
        ),
    ensures
        canonical(overlay_all(start, layers)),
{
    lemma_overlay_all_has_upper(start, layers);
}

/// Layering never removes the upper bound, and its results are normalized.
pub proof fn lemma_overlay_all_has_upper(start: PortBounds, layers: Seq<PortLayer>)
    requires
        canonical(start),
    ensures
        has_upper(overlay_all(start, layers)),
        normalized(overlay_all(start, layers)) == overlay_all(start, layers),
    decreases layers.len(),
{
    if layers.len() > 0 {
        let below = overlay_all(start, layers.drop_last());
        lemma_overlay_all_has_upper(start, layers.drop_last());
        lemma_overlay_properties(below, layers.last());
        lemma_normalized(below);
        let anchor = layer_anchor(below, layers.last());
        lemma_normalized(
            PortBounds { min: anchor, max: layers.last().max, max_offset: layers.last().max_offset },
        );
        let inherited = normalized(below);
        lemma_normalized(
            PortBounds { min: anchor, max: inherited.max, max_offset: inherited.max_offset },
        );
    }
}

/// Exactly one, absolute upper bound before validation too: laying layers
/// whose upper-bound declarations resolve over a canonical section (such as
/// the built-in defaults) yields a canonical section.
pub proof fn lemma_overlay_all_canonical(start: PortBounds, layers: Seq<PortLayer>)
    requires
        canonical(start),
        declarations_ok(start, layers),
    ensures
        canonical(overlay_all(start, layers)),
    decreases layers.len(),
{
    if layers.len() > 0 {
        lemma_overlay_all_canonical(start, layers.drop_last());
        lemma_overlay_properties(overlay_all(start, layers.drop_last()), layers.last());
    }
}

/// A layer whose upper-bound declaration resolves wherever it is laid: it
/// sets `min` and its declaration resolves there, or it sets no
/// `max_offset`.
pub open spec fn self_anchored_ok(layer: PortLayer) -> bool {
    match layer.min {
        Some(m) => upper_declaration_ok(m, layer.max, layer.max_offset),
        None => layer.max_offset is None,
    }
}

/// Every validated document is a self-anchored layer (it always sets `min`),
/// and so is every command-line layer (it never sets `max_offset`).
pub proof fn lemma_self_anchored_layers(
    min: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
    cli_min: Option<u16>,
    cli_max: Option<u16>,
)
    ensures
        strictly_valid(min, max, max_offset) ==> self_anchored_ok(
            PortLayer { min: Some(min), max, max_offset },
        ),
        self_anchored_ok(PortLayer { min: cli_min, max: cli_max, max_offset: None }),
{
}

/// A stack of self-anchored layers has resolvable declarations throughout.
pub proof fn lemma_self_anchored_stack_ok(start: PortBounds, layers: Seq<PortLayer>)
    requires
        forall|i: int| 0 <= i < layers.len() ==> self_anchored_ok(#[trigger] layers[i]),
    ensures
        declarations_ok(start, layers),
    decreases layers.len(),
{
    if layers.len() > 0 {
        let rest = layers.drop_last();
        assert forall|i: int| 0 <= i < rest.len() implies self_anchored_ok(#[trigger] rest[i]) by {
            assert(rest[i] == layers[i]);
        }
        lemma_self_anchored_stack_ok(start, rest);
        assert(self_anchored_ok(layers[layers.len() - 1]));
    }
}

/// On a canonical section, validation and resolution agree, and only the
/// `min`/`max` rules can fail: "Either max or max_offset must be specified",
/// the offset errors, and "Cannot specify both max and max_offset" are all
/// unreachable.
pub proof fn lemma_canonical_bounds_checks(b: PortBounds)
    requires
        canonical(b),
    ensures
        strictly_valid(b.min, b.max, b.max_offset) <==> resolvable(b.min, b.max, b.max_offset),
        resolvable(b.min, b.max, b.max_offset) <==> super::port::is_valid_range(
            b.min as int,
            b.max->0 as int,
        ),
        upper_of(b) == Some(b.max->0 as int),
        validation_error(b.min, b.max, b.max_offset) matches Some(e) ==> (e is InvalidMin
            || e is InvalidMax || e is MaxBelowMin),
        resolution_error(b.min, b.max, b.max_offset) matches Some(e) ==> (e is InvalidMin
            || e is InvalidMax || e is MaxBelowMin),
{
}

/// The environment layer is held to the document rules: every validated
/// document's declaration resolves at its own `min`, validation reports
/// exactly the declaration's error whenever that is the first rule a document
/// breaks, and validation rejects every document whose declaration does not
/// resolve.
pub proof fn lemma_upper_declaration_agrees_with_validation(
    min: u16,
    max: Option<u16>,
    max_offset: Option<u16>,
)
    ensures
        strictly_valid(min, max, max_offset) ==> upper_declaration_ok(min, max, max_offset),
        upper_declaration_ok(min, max, max_offset) <==> upper_declaration_error(
            min,
            max,
            max_offset,
        ) is None,
        ({
            let e = validation_error(min, max, max_offset);
            e is Some && (e->0 is ZeroOffset || e->0 is OffsetOverflow || e->0 is BothMaxAndOffset)
                ==> upper_declaration_error(min, max, max_offset) == e
        }),
        upper_declaration_error(min, max, max_offset) is Some ==> validation_error(
            min,
            max,
            max_offset,
        ) is Some,
{
}

/// Command-line overrides follow the document rules: given both, the check
/// is exactly the validation of `{min, max}` (same rules, same order); given
/// only `--min`, it is the validation of `{min}`; given only `--max`, the
/// value must be a valid port.
pub proof fn lemma_override_matches_document_rules(min: Option<u16>, max: Option<u16>)
    ensures
        min is Some && max is Some ==> override_error(min, max) == validation_error(
            min->0,
            max,
            None,
        ),
        min is Some && max is None ==> override_error(min, max) == validation_error(
            min->0,
            None,
            None,
        ),
        min is None && max is Some ==> (override_ok(min, max) <==> super::port::is_valid_port(
            max->0 as int,
        )),
{
}

/// An accepted `--min a --max b` pair becomes exactly the effective range
/// `a..=b`, whatever lies below; `--min a` alone keeps the configured
/// (absolute) upper bound.
pub proof fn lemma_override_pair_is_effective_range(lower: PortBounds, a: u16, b: u16)
    ensures
        override_ok(Some(a), Some(b)) ==> {
            let r = overlay(lower, PortLayer { min: Some(a), max: Some(b), max_offset: None });
            &&& r == (PortBounds { min: a, max: Some(b), max_offset: None })
            &&& canonical(r)
            &&& strictly_valid(r.min, r.max, r.max_offset)
            &&& upper_of(r) == Some(b as int)
        },
        canonical(lower) ==> overlay(lower, PortLayer { min: Some(a), max: None, max_offset: None })
            == (PortBounds { min: a, max: lower.max, max_offset: None }),
{
}

/// The ports of `lo..=hi` that are `>= b`, in order, are the ports of
/// `b..=hi`.
pub proof fn lemma_ports_suffix(lo: int, hi: int, b: int)
    requires
        1 <= lo <= b <= hi <= 65535,
    ensures
        super::port::ports_between(b, hi) == super::port::ports_between(lo, hi).subrange(
            b - lo,
            hi - lo + 1,
        ),
{
    assert(super::port::ports_between(b, hi) =~= super::port::ports_between(lo, hi).subrange(
        b - lo,
        hi - lo + 1,
    ));
}

} // verus!
