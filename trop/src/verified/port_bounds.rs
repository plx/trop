//! Verified kernel: resolution of configured port bounds (`min`, then `max` or
//! `max_offset`).
//!
//! A port configuration names its range as `min` plus either an explicit
//! `max` or a `max_offset` (`max = min + max_offset`). Two rule sets exist:
//!
//! - [`resolve_port_bounds`] — the *resolution* every consumer of a loaded
//!   configuration uses to obtain the range (the allocator, `trop scan`). It
//!   is lenient: an explicit `max` wins over `max_offset`, and a zero offset
//!   yields the single-port range `min..=min`.
//! - [`check_port_bounds`] — the *validation* rules a configuration file must
//!   satisfy (`ConfigValidator::validate_port_config`): `max` and
//!   `max_offset` are mutually exclusive and the offset is strictly positive.
//!
//! The lemmas at the end relate the two: on every validated configuration that
//! names an upper bound, resolution succeeds with exactly the validated range,
//! and resolution accepts precisely two shapes that validation rejects.
//!
//! Production glue: `crate::port::allocator::port_range_from_config` (used by
//! `allocator_from_config` and by the CLI's `trop scan`) calls
//! [`resolve_port_bounds`] and maps [`PortBoundsError`] to the historical
//! error values.

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
/// order of `ConfigValidator::validate_port_config`.
// Not called by production code yet: `ConfigValidator::validate_port_config`
// (owned by the configuration work package) adopts it next. Remove this
// `allow` then.
#[allow(dead_code)]
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

/// `ConfigValidator::validate_port_config` checks, after a successful
/// `min.checked_add(offset)`, that the sum is a valid port ("Offset would
/// create invalid max port"). Whenever that check is reached — `min` already
/// validated, the offset non-zero, and no overflow — the sum *is* a valid
/// port, so that branch is dead code.
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
/// when `max_offset` is 0 (the range `min..=min`). Every other resolvable
/// configuration is strictly valid.
pub proof fn lemma_resolution_leniency(min: u16, max: Option<u16>, max_offset: Option<u16>)
    requires
        resolvable(min, max, max_offset),
        !strictly_valid(min, max, max_offset),
    ensures
        (max is Some && max_offset is Some) || max_offset == Some(0u16),
{
}

} // verus!
