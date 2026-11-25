//! Verified kernel: group allocation.
//!
//! - request validation and normalization (`validate_group_request`,
//!   producing the invariant-carrying `ValidatedGroupShape`): service tags,
//!   the project, and the task follow the shared identifier rule of
//!   `super::identifier`, which also trims them;
//! - the lowest-base offset-pattern search over an availability oracle
//!   (`pattern_scan_end`, `find_pattern_base`);
//! - fresh allocation planning: pin available preferred ports, place the
//!   remaining services at one base, all ports distinct (`plan_fresh_group`);
//! - matching stored rows to requested services (`match_stored_tags`) and
//!   checking that a stored group still has the requested shape
//!   (`derive_compatible_base`), with the theorem that a fresh plan always
//!   passes that check (`lemma_fresh_plan_is_compatible`);
//! - the reconciliation decision (`decide_group_reconciliation`) and
//!   configuration helpers. (The scan range derived from `reservations.base`
//!   is `super::port_bounds::group_scan_span`, next to the one resolution of
//!   the configured bounds.)
//! - the output selection of `reserve-group` and `autoreserve`
//!   (`select_group_output`: an explicit `--shell` is parsed, and the shell
//!   detected, only for the export format).
//!
//! Exhausted fresh allocations are retried once after automatic cleanup by
//! the shared control `super::reserve::allocate_with_cleanup_retry`, under
//! the cleanup policy `super::reserve::cleanup_policy`.
//!
//! Production glue: `crate::port::group`, `crate::operations::reserve_group`,
//! and `crate::operations::autoreserve`.

use super::identifier::{check_identifier, IdentifierError};
use super::port::PortSpan;
use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Identifiers: the shared rule of `super::identifier`
// ---------------------------------------------------------------------------

/// `t` is a stored identifier value: the shared identifier rule accepts it
/// unchanged (it is valid and already trimmed).
pub open spec fn is_identifier_value(t: Seq<char>) -> bool {
    super::identifier::identifier_verdict(t) == Ok::<Seq<char>, IdentifierError>(t)
}

/// An optional raw identifier (project or task) is absent, or accepted by the
/// shared identifier rule.
pub open spec fn opt_identifier_ok(value: Option<&str>) -> bool {
    match value {
        Some(s) => super::identifier::is_valid_identifier(s@),
        None => true,
    }
}

/// `t` is the normalized form of the optional raw identifier `raw`: absent
/// when `raw` is, and otherwise its trimmed value.
pub open spec fn opt_normalized(raw: Option<&str>, t: Option<&str>) -> bool {
    match raw {
        Some(s) => t matches Some(v) && v@ == super::identifier::trimmed(s@),
        None => t is None,
    }
}

/// An identifier the shared rule accepts is accepted unchanged after
/// trimming: its trimmed value is a stored identifier value.
pub proof fn lemma_trimmed_is_identifier_value(s: Seq<char>)
    requires
        super::identifier::is_valid_identifier(s),
    ensures
        is_identifier_value(super::identifier::trimmed(s)),
{
    super::identifier::lemma_revalidation(s);
}

/// A stored identifier value is its own trimmed form and passes the rule.
pub proof fn lemma_identifier_value_is_trimmed(t: Seq<char>)
    requires
        is_identifier_value(t),
    ensures
        super::identifier::trimmed(t) == t,
        super::identifier::is_valid_identifier(t),
{
}

/// Applies the shared identifier rule to an optional raw identifier.
fn check_optional(value: Option<&str>) -> (r: Result<Option<&str>, IdentifierError>)
    ensures
        r is Ok <==> opt_identifier_ok(value),
        r matches Ok(t) ==> opt_normalized(value, t),
        r matches Err(e) ==> value matches Some(s) && super::identifier::identifier_verdict(s@)
            == Err::<Seq<char>, IdentifierError>(e),
{
    match value {
        Some(s) => match check_identifier(s) {
            Ok(t) => Ok(Some(t)),
            Err(e) => Err(e),
        },
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Group request validation
// ---------------------------------------------------------------------------

/// One service: its tag and its allocation strategy.
///
/// In a request the tag is raw; `validate_group_request` trims it, so the
/// services of a `ValidatedGroupShape` carry trimmed tags.
pub struct RawService {
    /// The service tag.
    pub tag: String,
    /// Offset from the group base (the fallback strategy).
    pub offset: Option<u16>,
    /// Preferred absolute port (tried first).
    pub preferred: Option<u16>,
}

/// Why a group request is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// The group has no services.
    NoServices,
    /// The project breaks the identifier rule.
    Project(IdentifierError),
    /// The task breaks the identifier rule.
    Task(IdentifierError),
    /// A tag breaks the identifier rule.
    Tag {
        /// Service index.
        index: usize,
        /// The rule the tag breaks.
        issue: IdentifierError,
    },
    /// A tag repeats an earlier service's tag (after trimming).
    DuplicateTag {
        /// Service index.
        index: usize,
    },
    /// A service has neither an offset nor a preferred port.
    MissingOffsetAndPreferred {
        /// Service index.
        index: usize,
    },
    /// A preferred port is 0.
    ZeroPreferred {
        /// Service index.
        index: usize,
    },
    /// An offset repeats an earlier service's offset.
    DuplicateOffset {
        /// Service index.
        index: usize,
        /// The repeated offset.
        offset: u16,
    },
    /// A preferred port repeats an earlier service's preferred port.
    DuplicatePreferred {
        /// Service index.
        index: usize,
        /// The repeated port.
        port: u16,
    },
}

/// A validated service is well formed: its tag is a stored identifier value
/// (valid and trimmed), it has some allocation strategy, and its preferred
/// port is not 0.
pub open spec fn service_well_formed(s: RawService) -> bool {
    &&& is_identifier_value(s.tag@)
    &&& (s.offset is Some || s.preferred is Some)
    &&& s.preferred != Some(0u16)
}

/// Two services clash: same tag, same offset, or same preferred port.
pub open spec fn services_clash(a: RawService, b: RawService) -> bool {
    ||| a.tag@ == b.tag@
    ||| (a.offset is Some && a.offset == b.offset)
    ||| (a.preferred is Some && a.preferred == b.preferred)
}

/// A valid (validated) group: non-empty, every service well formed, and
/// tags, offsets, and preferred ports pairwise distinct.
pub open spec fn group_shape_valid(s: Seq<RawService>) -> bool {
    &&& s.len() > 0
    &&& forall|i: int| 0 <= i < s.len() ==> #[trigger] service_well_formed(s[i])
    &&& forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j ==> !#[trigger] services_clash(s[i], s[j])
}

/// The identity a requested service is stored under: its trimmed tag.
pub open spec fn tag_key(s: RawService) -> Seq<char> {
    super::identifier::trimmed(s.tag@)
}

/// A requested service on its own is acceptable: the identifier rule accepts
/// its tag, it has some allocation strategy, and its preferred port is not 0.
pub open spec fn request_service_ok(s: RawService) -> bool {
    &&& super::identifier::is_valid_identifier(s.tag@)
    &&& (s.offset is Some || s.preferred is Some)
    &&& s.preferred != Some(0u16)
}

/// Two requested services clash: same trimmed tag, same offset, or same
/// preferred port.
pub open spec fn requests_clash(a: RawService, b: RawService) -> bool {
    ||| tag_key(a) == tag_key(b)
    ||| (a.offset is Some && a.offset == b.offset)
    ||| (a.preferred is Some && a.preferred == b.preferred)
}

/// A valid group request: non-empty, every service acceptable, and no two
/// services clash.
pub open spec fn group_request_valid(s: Seq<RawService>) -> bool {
    &&& s.len() > 0
    &&& forall|i: int| 0 <= i < s.len() ==> #[trigger] request_service_ok(s[i])
    &&& forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j ==> !#[trigger] requests_clash(s[i], s[j])
}

/// `v` is the normalized form of the requested service `raw`: its trimmed
/// tag, and the same strategy.
pub open spec fn service_normalized(raw: RawService, v: RawService) -> bool {
    &&& v.tag@ == tag_key(raw)
    &&& v.offset == raw.offset
    &&& v.preferred == raw.preferred
}

/// `v` normalizes the requested services `raw` one by one, in order.
pub open spec fn shape_normalized(raw: Seq<RawService>, v: Seq<RawService>) -> bool {
    &&& v.len() == raw.len()
    &&& forall|i: int| 0 <= i < raw.len() ==> #[trigger] service_normalized(raw[i], v[i])
}

/// The first problem with requested service `i` given the services before
/// it, in the production check order: the identifier rule for the tag,
/// duplicate (trimmed) tag, missing strategy, preferred port 0, duplicate
/// offset, duplicate preferred port.
pub open spec fn service_issue(s: Seq<RawService>, i: int) -> Option<ShapeError> {
    let svc = s[i];
    let verdict = super::identifier::identifier_verdict(svc.tag@);
    if verdict is Err {
        Some(ShapeError::Tag { index: i as usize, issue: verdict->Err_0 })
    } else if exists|j: int| 0 <= j < i && #[trigger] tag_key(s[j]) == tag_key(svc) {
        Some(ShapeError::DuplicateTag { index: i as usize })
    } else if svc.offset is None && svc.preferred is None {
        Some(ShapeError::MissingOffsetAndPreferred { index: i as usize })
    } else if svc.preferred == Some(0u16) {
        Some(ShapeError::ZeroPreferred { index: i as usize })
    } else if svc.offset is Some && exists|j: int| 0 <= j < i && #[trigger] s[j].offset == svc.offset {
        Some(ShapeError::DuplicateOffset { index: i as usize, offset: svc.offset->Some_0 })
    } else if svc.preferred is Some && exists|j: int|
        0 <= j < i && #[trigger] s[j].preferred == svc.preferred {
        Some(ShapeError::DuplicatePreferred { index: i as usize, port: svc.preferred->Some_0 })
    } else {
        None
    }
}

/// `e` is the error of service `i`, and every earlier service passed.
pub open spec fn first_service_issue(s: Seq<RawService>, i: int, e: ShapeError) -> bool {
    &&& 0 <= i < s.len()
    &&& forall|k: int| 0 <= k < i ==> #[trigger] service_issue(s, k) is None
    &&& service_issue(s, i) == Some(e)
}

/// The service index a per-service error names (-1 for group-level errors).
pub open spec fn shape_error_index(e: ShapeError) -> int {
    match e {
        ShapeError::Tag { index, .. } => index as int,
        ShapeError::DuplicateTag { index } => index as int,
        ShapeError::MissingOffsetAndPreferred { index } => index as int,
        ShapeError::ZeroPreferred { index } => index as int,
        ShapeError::DuplicateOffset { index, .. } => index as int,
        ShapeError::DuplicatePreferred { index, .. } => index as int,
        _ => -1,
    }
}

/// No service before `n` has an issue, so services `0..n` are acceptable and
/// pairwise clash-free.
proof fn lemma_prefix_valid(s: Seq<RawService>, n: int)
    requires
        0 <= n <= s.len(),
        forall|k: int| 0 <= k < n ==> #[trigger] service_issue(s, k) is None,
    ensures
        forall|i: int| 0 <= i < n ==> #[trigger] request_service_ok(s[i]),
        forall|i: int, j: int|
            0 <= i < n && 0 <= j < n && i != j ==> !#[trigger] requests_clash(s[i], s[j]),
{
    assert forall|i: int| 0 <= i < n implies #[trigger] request_service_ok(s[i]) by {
        assert(service_issue(s, i) is None);
    }
    assert forall|i: int, j: int| 0 <= i < n && 0 <= j < n && i != j implies !#[trigger] requests_clash(
        s[i],
        s[j],
    ) by {
        let lo = if i < j { i } else { j };
        let hi = if i < j { j } else { i };
        assert(service_issue(s, hi) is None);
        if tag_key(s[lo]) == tag_key(s[hi]) {
            assert(exists|k: int| 0 <= k < hi && #[trigger] tag_key(s[k]) == tag_key(s[hi]));
        }
        if s[hi].offset is Some && s[lo].offset == s[hi].offset {
            assert(exists|k: int| 0 <= k < hi && #[trigger] s[k].offset == s[hi].offset);
        }
        if s[hi].preferred is Some && s[lo].preferred == s[hi].preferred {
            assert(exists|k: int| 0 <= k < hi && #[trigger] s[k].preferred == s[hi].preferred);
        }
    }
}

/// A valid group request has no service issue.
proof fn lemma_valid_has_no_issue(s: Seq<RawService>)
    requires
        group_request_valid(s),
    ensures
        forall|k: int| 0 <= k < s.len() ==> #[trigger] service_issue(s, k) is None,
{
    assert forall|k: int| 0 <= k < s.len() implies #[trigger] service_issue(s, k) is None by {
        assert(request_service_ok(s[k]));
        if exists|j: int| 0 <= j < k && #[trigger] tag_key(s[j]) == tag_key(s[k]) {
            let j = choose|j: int| 0 <= j < k && #[trigger] tag_key(s[j]) == tag_key(s[k]);
            assert(requests_clash(s[j], s[k]));
        }
        if s[k].offset is Some && exists|j: int| 0 <= j < k && #[trigger] s[j].offset == s[k].offset {
            let j = choose|j: int| 0 <= j < k && #[trigger] s[j].offset == s[k].offset;
            assert(requests_clash(s[j], s[k]));
        }
        if s[k].preferred is Some && exists|j: int|
            0 <= j < k && #[trigger] s[j].preferred == s[k].preferred {
            let j = choose|j: int| 0 <= j < k && #[trigger] s[j].preferred == s[k].preferred;
            assert(requests_clash(s[j], s[k]));
        }
    }
}

/// Normalizing a valid request yields a valid group: trimmed tags are stored
/// identifier values, and distinct trimmed tags stay distinct.
pub proof fn lemma_normalized_shape_valid(raw: Seq<RawService>, v: Seq<RawService>)
    requires
        group_request_valid(raw),
        shape_normalized(raw, v),
    ensures
        group_shape_valid(v),
{
    assert forall|i: int| 0 <= i < v.len() implies #[trigger] service_well_formed(v[i]) by {
        assert(request_service_ok(raw[i]));
        assert(service_normalized(raw[i], v[i]));
        lemma_trimmed_is_identifier_value(raw[i].tag@);
    }
    assert forall|i: int, j: int|
        0 <= i < v.len() && 0 <= j < v.len() && i != j implies !#[trigger] services_clash(
        v[i],
        v[j],
    ) by {
        assert(!requests_clash(raw[i], raw[j]));
        assert(service_normalized(raw[i], v[i]));
        assert(service_normalized(raw[j], v[j]));
    }
}

/// Validation is idempotent: a validated group is itself a valid request,
/// and normalizes to itself. Validating its services again (with any valid
/// project and task) therefore succeeds with the same services, so
/// normalizing an already normalized request changes nothing.
pub proof fn lemma_validation_idempotent(s: Seq<RawService>)
    requires
        group_shape_valid(s),
    ensures
        group_request_valid(s),
        shape_normalized(s, s),
        forall|k: int| 0 <= k < s.len() ==> #[trigger] service_issue(s, k) is None,
{
    assert forall|i: int| 0 <= i < s.len() implies #[trigger] tag_key(s[i]) == s[i].tag@
        && super::identifier::is_valid_identifier(s[i].tag@) by {
        assert(service_well_formed(s[i]));
        lemma_identifier_value_is_trimmed(s[i].tag@);
    }
    assert forall|i: int| 0 <= i < s.len() implies #[trigger] request_service_ok(s[i]) by {
        assert(service_well_formed(s[i]));
        assert(tag_key(s[i]) == s[i].tag@);
    }
    assert forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j implies !#[trigger] requests_clash(
        s[i],
        s[j],
    ) by {
        assert(!services_clash(s[i], s[j]));
        assert(tag_key(s[i]) == s[i].tag@);
        assert(tag_key(s[j]) == s[j].tag@);
    }
    assert forall|i: int| 0 <= i < s.len() implies #[trigger] service_normalized(s[i], s[i]) by {
        assert(tag_key(s[i]) == s[i].tag@);
    }
    lemma_valid_has_no_issue(s);
}

/// A validated group shape: the normalized services of a request that passed
/// `validate_group_request`, in the order they were given.
///
/// The field is private and `validate_group_request` is the only
/// constructor, so `group_shape_valid` holds for every value -- including
/// values held by unverified code.
pub struct ValidatedGroupShape {
    services: Vec<RawService>,
}

impl ValidatedGroupShape {
    #[verifier::type_invariant]
    spec fn well_formed(self) -> bool {
        group_shape_valid(self.services@)
    }

    /// The validated services, in order.
    pub closed spec fn services_view(self) -> Seq<RawService> {
        self.services@
    }

    /// The validated services (also exposes the invariant to callers).
    pub fn services(&self) -> (s: &[RawService])
        ensures
            s@ == self.services_view(),
            group_shape_valid(s@),
    {
        proof {
            use_type_invariant(self);
        }
        self.services.as_slice()
    }
}

/// A validated group request: the normalized services, project, and task.
pub struct ValidatedGroupRequest<'a> {
    /// The services with trimmed tags, in request order.
    pub shape: ValidatedGroupShape,
    /// The trimmed project.
    pub project: Option<&'a str>,
    /// The trimmed task.
    pub task: Option<&'a str>,
}

/// Whether some (normalized) service of `out` already has tag `key`.
fn tag_taken(out: &[RawService], key: &String) -> (found: bool)
    ensures
        found == exists|j: int| 0 <= j < out@.len() && #[trigger] out@[j].tag@ == key@,
{
    let mut j: usize = 0;
    while j < out.len()
        invariant
            0 <= j <= out@.len(),
            forall|k: int| 0 <= k < j ==> #[trigger] out@[k].tag@ != key@,
        decreases out@.len() - j,
    {
        if out[j].tag == *key {
            assert(out@[j as int].tag@ == key@);
            return true;
        }
        j += 1;
    }
    false
}

/// Whether some service before `i` has offset `Some(offset)`.
fn earlier_offset(s: &[RawService], i: usize, offset: u16) -> (found: bool)
    requires
        i < s@.len(),
    ensures
        found == exists|j: int| 0 <= j < i && #[trigger] s@[j].offset == Some(offset),
{
    let mut j: usize = 0;
    while j < i
        invariant
            0 <= j <= i < s@.len(),
            forall|k: int| 0 <= k < j ==> #[trigger] s@[k].offset != Some(offset),
        decreases i - j,
    {
        if let Some(other) = s[j].offset {
            if other == offset {
                return true;
            }
        }
        j += 1;
    }
    false
}

/// Whether some service before `i` has preferred port `Some(port)`.
fn earlier_preferred(s: &[RawService], i: usize, port: u16) -> (found: bool)
    requires
        i < s@.len(),
    ensures
        found == exists|j: int| 0 <= j < i && #[trigger] s@[j].preferred == Some(port),
{
    let mut j: usize = 0;
    while j < i
        invariant
            0 <= j <= i < s@.len(),
            forall|k: int| 0 <= k < j ==> #[trigger] s@[k].preferred != Some(port),
        decreases i - j,
    {
        if let Some(other) = s[j].preferred {
            if other == port {
                return true;
            }
        }
        j += 1;
    }
    false
}

/// Checks requested service `i` (see `service_issue`), given the normalized
/// services `out` before it; returns its trimmed tag.
fn check_service(s: &[RawService], out: &[RawService], i: usize) -> (r: Result<String, ShapeError>)
    requires
        i < s@.len(),
        out@.len() == i,
        forall|k: int| 0 <= k < i ==> #[trigger] service_normalized(s@[k], out@[k]),
    ensures
        r matches Ok(key) ==> service_issue(s@, i as int) is None && key@ == tag_key(s@[i as int]),
        r matches Err(e) ==> service_issue(s@, i as int) == Some(e),
{
    let svc = &s[i];
    let trimmed = match check_identifier(svc.tag.as_str()) {
        Ok(t) => t,
        Err(issue) => {
            return Err(ShapeError::Tag { index: i, issue });
        },
    };
    let key = trimmed.to_owned();
    if tag_taken(out, &key) {
        proof {
            let j = choose|j: int| 0 <= j < out@.len() && #[trigger] out@[j].tag@ == key@;
            assert(service_normalized(s@[j], out@[j]));
            assert(tag_key(s@[j]) == tag_key(s@[i as int]));
        }
        return Err(ShapeError::DuplicateTag { index: i });
    }
    proof {
        assert forall|j: int| 0 <= j < i implies #[trigger] tag_key(s@[j]) != tag_key(
            s@[i as int],
        ) by {
            assert(service_normalized(s@[j], out@[j]));
            assert(out@[j].tag@ != key@);
        }
    }
    if svc.offset.is_none() && svc.preferred.is_none() {
        return Err(ShapeError::MissingOffsetAndPreferred { index: i });
    }
    if let Some(0) = svc.preferred {
        return Err(ShapeError::ZeroPreferred { index: i });
    }
    if let Some(offset) = svc.offset {
        if earlier_offset(s, i, offset) {
            return Err(ShapeError::DuplicateOffset { index: i, offset });
        }
    }
    if let Some(port) = svc.preferred {
        if earlier_preferred(s, i, port) {
            return Err(ShapeError::DuplicatePreferred { index: i, port });
        }
    }
    Ok(key)
}

/// Validates and normalizes a group request.
///
/// Succeeds exactly when the group is non-empty, the shared identifier rule
/// accepts the project and the task (when given), and the services form a
/// valid request (`group_request_valid`: every tag accepted by the identifier
/// rule, some allocation strategy and no preferred port 0 per service, and
/// trimmed tags, offsets, and preferred ports pairwise distinct). The result
/// holds the services in order with trimmed tags (`shape_normalized`), and
/// the trimmed project and task. Otherwise reports the first error in
/// production order: no services, project, task, then the first service with
/// an issue (`service_issue`).
pub fn validate_group_request<'a>(
    services: &[RawService],
    project: Option<&'a str>,
    task: Option<&'a str>,
) -> (r: Result<ValidatedGroupRequest<'a>, ShapeError>)
    ensures
        r is Ok <==> (opt_identifier_ok(project) && opt_identifier_ok(task) && group_request_valid(
            services@,
        )),
        r matches Ok(v) ==> {
            &&& shape_normalized(services@, v.shape.services_view())
            &&& opt_normalized(project, v.project)
            &&& opt_normalized(task, v.task)
        },
        r matches Err(e) ==> match e {
            ShapeError::NoServices => services@.len() == 0,
            ShapeError::Project(issue) => {
                &&& services@.len() > 0
                &&& project matches Some(p) && super::identifier::identifier_verdict(p@) == Err::<
                    Seq<char>,
                    IdentifierError,
                >(issue)
            },
            ShapeError::Task(issue) => {
                &&& services@.len() > 0
                &&& opt_identifier_ok(project)
                &&& task matches Some(t) && super::identifier::identifier_verdict(t@) == Err::<
                    Seq<char>,
                    IdentifierError,
                >(issue)
            },
            _ => {
                &&& services@.len() > 0
                &&& opt_identifier_ok(project)
                &&& opt_identifier_ok(task)
                &&& first_service_issue(services@, shape_error_index(e), e)
            },
        },
{
    if services.is_empty() {
        return Err(ShapeError::NoServices);
    }
    let checked_project = match check_optional(project) {
        Ok(value) => value,
        Err(issue) => {
            return Err(ShapeError::Project(issue));
        },
    };
    let checked_task = match check_optional(task) {
        Ok(value) => value,
        Err(issue) => {
            return Err(ShapeError::Task(issue));
        },
    };
    let mut out: Vec<RawService> = Vec::new();
    let mut i: usize = 0;
    while i < services.len()
        invariant
            0 <= i <= services@.len(),
            services@.len() > 0,
            opt_identifier_ok(project),
            opt_identifier_ok(task),
            out@.len() == i,
            forall|k: int| 0 <= k < i ==> #[trigger] service_normalized(services@[k], out@[k]),
            forall|k: int| 0 <= k < i ==> #[trigger] service_issue(services@, k) is None,
        decreases services@.len() - i,
    {
        let key = match check_service(services, out.as_slice(), i) {
            Ok(key) => key,
            Err(e) => {
                assert(shape_error_index(e) == i);
                proof {
                    if group_request_valid(services@) {
                        lemma_valid_has_no_issue(services@);
                    }
                }
                return Err(e);
            },
        };
        let svc = &services[i];
        let ghost prev = out@;
        out.push(RawService { tag: key, offset: svc.offset, preferred: svc.preferred });
        proof {
            assert forall|k: int| 0 <= k < i + 1 implies #[trigger] service_normalized(
                services@[k],
                out@[k],
            ) by {
                if k < i {
                    assert(out@[k] == prev[k]);
                    assert(service_normalized(services@[k], prev[k]));
                }
            }
        }
        i += 1;
    }
    proof {
        lemma_prefix_valid(services@, services@.len() as int);
        lemma_normalized_shape_valid(services@, out@);
    }
    Ok(
        ValidatedGroupRequest {
            shape: ValidatedGroupShape { services: out },
            project: checked_project,
            task: checked_task,
        },
    )
}

// ---------------------------------------------------------------------------
// Offset patterns
// ---------------------------------------------------------------------------

/// The largest offset of a pattern (0 for the empty pattern).
pub open spec fn seq_max(s: Seq<u16>) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else {
        let m = seq_max(s.drop_last());
        if s.last() as int > m {
            s.last() as int
        } else {
            m
        }
    }
}

/// `seq_max` bounds every element and (for a non-empty pattern) is one of them.
pub proof fn lemma_seq_max(s: Seq<u16>)
    ensures
        0 <= seq_max(s) <= 65535,
        forall|i: int| 0 <= i < s.len() ==> s[i] as int <= seq_max(s),
        s.len() > 0 ==> exists|i: int| 0 <= i < s.len() && s[i] as int == seq_max(s),
    decreases s.len(),
{
    if s.len() > 0 {
        lemma_seq_max(s.drop_last());
        let m = seq_max(s.drop_last());
        assert forall|i: int| 0 <= i < s.len() implies s[i] as int <= seq_max(s) by {
            if i < s.len() - 1 {
                assert(s.drop_last()[i] == s[i]);
            }
        }
        if s.last() as int > m {
            assert(s[s.len() - 1] as int == seq_max(s));
        } else if s.len() > 1 {
            let j = choose|j: int| 0 <= j < s.drop_last().len() && s.drop_last()[j] as int == m;
            assert(s[j] as int == seq_max(s));
        } else {
            assert(s[0] as int == seq_max(s));
        }
    }
}

/// Port `b + offset` lies in `range_min..=range_max`.
pub open spec fn port_in_range(range_min: int, range_max: int, b: int, offset: int) -> bool {
    range_min <= b + offset <= range_max
}

/// Some offset of `pattern` puts base `b` past `range_max`.
pub open spec fn overshoots(pattern: Seq<u16>, range_max: int, b: int) -> bool {
    exists|i: int| 0 <= i < pattern.len() && b + pattern[i] > range_max
}

/// The last base worth scanning for `pattern` in `range_min..=range_max`:
/// `range_max - max(pattern)`, or `None` when the pattern is empty or no base
/// at or above `range_min` keeps the whole pattern in range.
///
/// Scanning bases `range_min..=e` therefore never overflows `u16`, keeps
/// every pattern port in range, and stops exactly where the spec's "stop
/// early" rule says: every higher base overshoots `range_max`.
pub fn pattern_scan_end(range_min: u16, range_max: u16, pattern: &[u16]) -> (r: Option<u16>)
    ensures
        pattern@.len() == 0 ==> r is None,
        pattern@.len() > 0 ==> (r is Some <==> range_max - seq_max(pattern@) >= range_min),
        r matches Some(e) ==> e == range_max - seq_max(pattern@) && range_min <= e,
        r matches Some(e) ==> forall|i: int|
            0 <= i < pattern@.len() ==> e + #[trigger] pattern@[i] <= range_max,
        r matches Some(e) ==> forall|b: int, i: int|
            range_min <= b <= e && 0 <= i < pattern@.len() ==> #[trigger] port_in_range(
                range_min as int,
                range_max as int,
                b,
                pattern@[i] as int,
            ),
        r matches Some(e) ==> forall|b: int| b > e ==> overshoots(pattern@, range_max as int, b),
        (pattern@.len() > 0 && r is None) ==> forall|b: int|
            b >= range_min ==> overshoots(pattern@, range_max as int, b),
{
    if pattern.is_empty() {
        return None;
    }
    let mut max_offset: u16 = 0;
    let mut i: usize = 0;
    while i < pattern.len()
        invariant
            0 <= i <= pattern@.len(),
            max_offset as int == seq_max(pattern@.take(i as int)),
        decreases pattern@.len() - i,
    {
        proof {
            assert(pattern@.take(i as int + 1).drop_last() =~= pattern@.take(i as int));
        }
        if pattern[i] > max_offset {
            max_offset = pattern[i];
        }
        i += 1;
    }
    proof {
        assert(pattern@.take(pattern@.len() as int) =~= pattern@);
        lemma_seq_max(pattern@);
        let m = seq_max(pattern@);
        let k = choose|k: int| 0 <= k < pattern@.len() && pattern@[k] as int == m;
        assert forall|b: int| b > range_max - m implies overshoots(pattern@, range_max as int, b) by {
            assert(b + pattern@[k] > range_max);
        }
    }
    match range_max.checked_sub(max_offset) {
        Some(end) if end >= range_min => Some(end),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Lowest-base pattern search over an availability oracle
// ---------------------------------------------------------------------------

/// Whether `port` occurs in `ports`.
pub fn slice_contains(ports: &[u16], port: u16) -> (found: bool)
    ensures
        found == ports@.contains(port),
{
    let mut i: usize = 0;
    while i < ports.len()
        invariant
            0 <= i <= ports@.len(),
            forall|j: int| 0 <= j < i ==> ports@[j] != port,
        decreases ports@.len() - i,
    {
        if ports[i] == port {
            return true;
        }
        i += 1;
    }
    false
}

/// The probe may be called on `q`: an in-range port that is not avoided.
pub open spec fn probe_domain(range_min: int, range_max: int, avoid: Seq<u16>, q: u16) -> bool {
    range_min <= q <= range_max && !avoid.contains(q)
}

/// `port` is usable for a fallback service: in range, not avoided, and the
/// probe reported it available.
pub open spec fn port_usable<E, F: Fn(u16) -> Result<bool, E>>(
    probe: F,
    range_max: int,
    avoid: Seq<u16>,
    port: int,
) -> bool {
    port <= range_max && !avoid.contains(port as u16) && probe.ensures(
        (port as u16,),
        Ok::<bool, E>(true),
    )
}

/// `port` was rejected: past `range_max`, avoided, or reported unavailable.
pub open spec fn port_rejected<E, F: Fn(u16) -> Result<bool, E>>(
    probe: F,
    range_max: int,
    avoid: Seq<u16>,
    port: int,
) -> bool {
    port > range_max || avoid.contains(port as u16) || probe.ensures(
        (port as u16,),
        Ok::<bool, E>(false),
    )
}

/// Every port of the pattern at base `b` is usable.
pub open spec fn pattern_fits<E, F: Fn(u16) -> Result<bool, E>>(
    probe: F,
    range_max: int,
    pattern: Seq<u16>,
    avoid: Seq<u16>,
    b: int,
) -> bool {
    forall|i: int|
        0 <= i < pattern.len() ==> #[trigger] port_usable(probe, range_max, avoid, b + pattern[i])
}

/// Some port of the pattern at base `b` was rejected.
pub open spec fn pattern_rejected<E, F: Fn(u16) -> Result<bool, E>>(
    probe: F,
    range_max: int,
    pattern: Seq<u16>,
    avoid: Seq<u16>,
    b: int,
) -> bool {
    exists|i: int|
        0 <= i < pattern.len() && #[trigger] port_rejected(probe, range_max, avoid, b + pattern[i])
}

/// Checks the whole pattern at base `b` (pattern order, stopping at the
/// first rejected port, skipping the probe for avoided ports).
fn check_base<E, F: Fn(u16) -> Result<bool, E>>(
    Ghost(range_min): Ghost<u16>,
    Ghost(range_max): Ghost<u16>,
    pattern: &[u16],
    avoid: &[u16],
    probe: &F,
    b: u16,
) -> (r: Result<bool, E>)
    requires
        forall|q: u16| #[trigger]
            probe_domain(range_min as int, range_max as int, avoid@, q) ==> probe.requires((q,)),
        forall|i: int|
            0 <= i < pattern@.len() ==> range_min <= b + #[trigger] pattern@[i] <= range_max,
    ensures
        r matches Ok(true) ==> pattern_fits(*probe, range_max as int, pattern@, avoid@, b as int),
        r matches Ok(false) ==> pattern_rejected(
            *probe,
            range_max as int,
            pattern@,
            avoid@,
            b as int,
        ),
        r matches Err(e) ==> exists|q: u16|
            #[trigger] probe_domain(range_min as int, range_max as int, avoid@, q) && probe.ensures(
                (q,),
                Err::<bool, E>(e),
            ),
{
    let mut i: usize = 0;
    while i < pattern.len()
        invariant
            0 <= i <= pattern@.len(),
            forall|q: u16| #[trigger]
                probe_domain(range_min as int, range_max as int, avoid@, q) ==> probe.requires(
                    (q,),
                ),
            forall|k: int|
                0 <= k < pattern@.len() ==> range_min <= b + #[trigger] pattern@[k] <= range_max,
            forall|k: int|
                0 <= k < i ==> #[trigger] port_usable(
                    *probe,
                    range_max as int,
                    avoid@,
                    b + pattern@[k],
                ),
        decreases pattern@.len() - i,
    {
        let port = b + pattern[i];
        if slice_contains(avoid, port) {
            assert(port_rejected(*probe, range_max as int, avoid@, b + pattern@[i as int]));
            return Ok(false);
        }
        assert(probe_domain(range_min as int, range_max as int, avoid@, port));
        match probe(port) {
            Err(e) => {
                assert(probe_domain(range_min as int, range_max as int, avoid@, port)
                    && probe.ensures((port,), Err::<bool, E>(e)));
                return Err(e);
            },
            Ok(false) => {
                assert(port_rejected(*probe, range_max as int, avoid@, b + pattern@[i as int]));
                return Ok(false);
            },
            Ok(true) => {
                assert(port_usable(*probe, range_max as int, avoid@, b + pattern@[i as int]));
            },
        }
        i += 1;
    }
    Ok(true)
}

/// Finds the lowest base `b >= range_min` at which every port `b + offset`
/// of `pattern` lies in `range_min..=range_max`, is not in `avoid`, and is
/// reported available by `probe`.
///
/// Bases are tried in ascending order and offsets in pattern order; the probe
/// is never called for an avoided or out-of-range port (the `requires` clause
/// only grants the probe's precondition on that domain), and a probe error is
/// returned unchanged. Scanning stops at `pattern_scan_end`, past which every
/// base overshoots `range_max`, so `Ok(None)` means no base at all fits.
pub fn find_pattern_base<E, F: Fn(u16) -> Result<bool, E>>(
    range_min: u16,
    range_max: u16,
    pattern: &[u16],
    avoid: &[u16],
    probe: &F,
) -> (r: Result<Option<u16>, E>)
    requires
        forall|q: u16| #[trigger]
            probe_domain(range_min as int, range_max as int, avoid@, q) ==> probe.requires((q,)),
    ensures
        r matches Ok(Some(b)) ==> {
            &&& pattern@.len() > 0
            &&& range_min <= b
            &&& pattern_fits(*probe, range_max as int, pattern@, avoid@, b as int)
            &&& forall|c: int|
                range_min <= c < b ==> pattern_rejected(
                    *probe,
                    range_max as int,
                    pattern@,
                    avoid@,
                    c,
                )
        },
        r matches Ok(None) ==> pattern@.len() == 0 || forall|c: int|
            range_min <= c ==> pattern_rejected(*probe, range_max as int, pattern@, avoid@, c),
        r matches Err(e) ==> exists|q: u16|
            #[trigger] probe_domain(range_min as int, range_max as int, avoid@, q) && probe.ensures(
                (q,),
                Err::<bool, E>(e),
            ),
{
    let Some(end) = pattern_scan_end(range_min, range_max, pattern) else {
        proof {
            if pattern@.len() > 0 {
                assert forall|c: int| range_min <= c implies pattern_rejected(
                    *probe,
                    range_max as int,
                    pattern@,
                    avoid@,
                    c,
                ) by {
                    assert(overshoots(pattern@, range_max as int, c));
                    let i = choose|i: int|
                        0 <= i < pattern@.len() && c + pattern@[i] > range_max;
                    assert(port_rejected(*probe, range_max as int, avoid@, c + pattern@[i]));
                }
            }
        }
        return Ok(None);
    };
    let mut b = range_min;
    loop
        invariant
            range_min <= b <= end,
            end == range_max - seq_max(pattern@),
            pattern@.len() > 0,
            forall|q: u16| #[trigger]
                probe_domain(range_min as int, range_max as int, avoid@, q) ==> probe.requires(
                    (q,),
                ),
            forall|i: int| 0 <= i < pattern@.len() ==> end + #[trigger] pattern@[i] <= range_max,
            forall|c: int| c > end ==> overshoots(pattern@, range_max as int, c),
            forall|c: int|
                range_min <= c < b ==> pattern_rejected(
                    *probe,
                    range_max as int,
                    pattern@,
                    avoid@,
                    c,
                ),
        decreases end - b,
    {
        let fits = check_base(Ghost(range_min), Ghost(range_max), pattern, avoid, probe, b);
        match fits {
            Err(e) => {
                return Err(e);
            },
            Ok(true) => {
                return Ok(Some(b));
            },
            Ok(false) => {},
        }
        if b == end {
            proof {
                assert forall|c: int| range_min <= c implies pattern_rejected(
                    *probe,
                    range_max as int,
                    pattern@,
                    avoid@,
                    c,
                ) by {
                    if c > end {
                        assert(overshoots(pattern@, range_max as int, c));
                        let i = choose|i: int|
                            0 <= i < pattern@.len() && c + pattern@[i] > range_max;
                        assert(port_rejected(*probe, range_max as int, avoid@, c + pattern@[i]));
                    }
                }
            }
            return Ok(None);
        }
        b += 1;
    }
}

// ---------------------------------------------------------------------------
// Fresh group allocation plan
// ---------------------------------------------------------------------------

/// The preferred-port probe's verdict (reservations, exclusions, and OS
/// occupancy across the whole port domain; no scan-range check).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferredVerdict {
    /// The port can be pinned.
    Available,
    /// Reserved by another reservation.
    Reserved,
    /// Excluded by configuration.
    Excluded,
    /// Occupied on the system.
    Occupied,
}

/// Why a fresh group cannot be planned.
pub enum FreshGroupError<E> {
    /// A probe failed; its error is returned unchanged.
    Probe(E),
    /// A service without an offset fallback has an unavailable preferred port.
    PreferredOnlyUnavailable {
        /// The preferred port.
        port: u16,
        /// The probe's verdict (never `Available`).
        verdict: PreferredVerdict,
    },
    /// No scanned base fits the complete fallback pattern.
    PatternExhausted {
        /// Number of services that fell back to their offsets.
        fallback_count: usize,
    },
}

/// A fresh allocation plan: one port per service, in service order.
pub struct FreshGroupPlan {
    /// The planned port of each service.
    pub ports: Vec<u16>,
    /// Whether each service was pinned to its preferred port (otherwise it
    /// is placed at `base + offset`).
    pub pinned: Vec<bool>,
    /// The base of the fallback services (`None` when every service is
    /// pinned).
    pub base: Option<u16>,
}

/// Service `svc` may be pinned: the probe reported its preferred port
/// available.
pub open spec fn pinned_ok<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    svc: RawService,
) -> bool {
    &&& svc.preferred is Some
    &&& preferred_probe.ensures(
        (svc.preferred->Some_0,),
        Ok::<PreferredVerdict, E>(PreferredVerdict::Available),
    )
}

/// Service `svc` may fall back to its offset: it has one, and it has no
/// preferred port or the probe reported its preferred port unavailable.
pub open spec fn fallback_ok<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    svc: RawService,
) -> bool {
    &&& svc.offset is Some
    &&& (svc.preferred is Some ==> exists|v: PreferredVerdict|
        v != PreferredVerdict::Available && #[trigger] preferred_probe.ensures(
            (svc.preferred->Some_0,),
            Ok::<PreferredVerdict, E>(v),
        ))
}

/// `pinned` classifies every service consistently with the probe's answers.
pub open spec fn classified<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    pinned: Seq<bool>,
) -> bool {
    &&& pinned.len() == s.len()
    &&& forall|i: int|
        0 <= i < s.len() ==> if #[trigger] pinned[i] {
            pinned_ok(preferred_probe, s[i])
        } else {
            fallback_ok(preferred_probe, s[i])
        }
}

/// `x` is the preferred port of a pinned service.
pub open spec fn is_pinned_port(s: Seq<RawService>, pinned: Seq<bool>, x: u16) -> bool {
    exists|j: int|
        0 <= j < s.len() && 0 <= j < pinned.len() && #[trigger] pinned[j] && s[j].preferred == Some(
            x,
        )
}

/// The preferred probe failed with `e` on the preferred port of `svc`.
pub open spec fn preferred_failed<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    svc: RawService,
    e: E,
) -> bool {
    svc.preferred is Some && preferred_probe.ensures(
        (svc.preferred->Some_0,),
        Err::<PreferredVerdict, E>(e),
    )
}

/// Service `index` is the first that cannot be placed: it has no offset
/// fallback and its preferred port `port` is unavailable, while every earlier
/// service was pinned or fell back consistently with the probe's answers.
pub open spec fn preferred_only_failed_at<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    index: int,
    port: u16,
) -> bool {
    &&& 0 <= index < s.len()
    &&& s[index].offset is None
    &&& s[index].preferred == Some(port)
    &&& forall|j: int|
        0 <= j < index ==> pinned_ok(preferred_probe, #[trigger] s[j]) || fallback_ok(
            preferred_probe,
            s[j],
        )
}

/// The preferred probe failed with `e` on some service's preferred port.
pub open spec fn some_preferred_failed<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    e: E,
) -> bool {
    exists|j: int| 0 <= j < s.len() && #[trigger] preferred_failed(preferred_probe, s[j], e)
}

/// A probe error `e` came from the preferred probe on some service's
/// preferred port, or from the scan probe on an in-range port.
pub open spec fn probe_error_source<E, G, F>(
    preferred_probe: G,
    scan_probe: F,
    s: Seq<RawService>,
    range_min: int,
    range_max: int,
    e: E,
) -> bool where G: Fn(u16) -> Result<PreferredVerdict, E>, F: Fn(u16) -> Result<bool, E> {
    ||| some_preferred_failed(preferred_probe, s, e)
    ||| exists|q: u16|
        range_min <= q <= range_max && #[trigger] scan_probe.ensures((q,), Err::<bool, E>(e))
}

/// The number of services that fall back to their offsets.
pub open spec fn count_unpinned(pinned: Seq<bool>) -> nat
    decreases pinned.len(),
{
    if pinned.len() == 0 {
        0
    } else {
        count_unpinned(pinned.drop_last()) + if pinned.last() {
            0nat
        } else {
            1nat
        }
    }
}

/// At base `b`, the fallback port of service `i` was rejected: past the
/// range, taken by a pinned preference, or reported unavailable.
pub open spec fn fallback_port_rejected<E, F: Fn(u16) -> Result<bool, E>>(
    scan_probe: F,
    range_max: int,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    i: int,
    b: int,
) -> bool {
    let port = b + s[i].offset->Some_0;
    ||| port > range_max
    ||| is_pinned_port(s, pinned, port as u16)
    ||| scan_probe.ensures((port as u16,), Ok::<bool, E>(false))
}

/// Base `b` fails: some fallback service's port was rejected.
pub open spec fn base_rejected<E, F: Fn(u16) -> Result<bool, E>>(
    scan_probe: F,
    range_max: int,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    b: int,
) -> bool {
    exists|i: int|
        0 <= i < s.len() && !pinned[i] && #[trigger] fallback_port_rejected(
            scan_probe,
            range_max,
            s,
            pinned,
            i,
            b,
        )
}

/// The oracle-independent shape of a fresh plan: pinned services get their
/// preferred port (anywhere in the port domain), the others get
/// `base + offset` inside the scan range, and all ports are valid and
/// pairwise distinct.
pub open spec fn fresh_plan_placement(
    s: Seq<RawService>,
    ports: Seq<u16>,
    pinned: Seq<bool>,
    base: Option<u16>,
    range_min: int,
    range_max: int,
) -> bool {
    &&& ports.len() == s.len()
    &&& pinned.len() == s.len()
    &&& forall|i: int| 0 <= i < s.len() && #[trigger] pinned[i] ==> s[i].preferred == Some(ports[i])
    &&& forall|i: int|
        0 <= i < s.len() && !#[trigger] pinned[i] ==> {
            &&& s[i].offset is Some
            &&& base is Some
            &&& ports[i] == base->Some_0 + s[i].offset->Some_0
            &&& range_min <= ports[i] <= range_max
        }
    &&& (base is None <==> forall|i: int| 0 <= i < s.len() ==> #[trigger] pinned[i])
    &&& (base is Some ==> range_min <= base->Some_0)
    &&& forall|i: int, j: int|
        0 <= i < s.len() && 0 <= j < s.len() && i != j ==> #[trigger] ports[i] != #[trigger] ports[j]
    &&& forall|i: int| 0 <= i < s.len() ==> 1 <= #[trigger] ports[i]
}

/// A rejection of the search pattern at base `c` is a rejection of some
/// fallback service at `c`.
proof fn lemma_pattern_rejection_maps<E, F: Fn(u16) -> Result<bool, E>>(
    scan_probe: F,
    range_max: int,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    pattern: Seq<u16>,
    avoid: Seq<u16>,
    pat_idx: Seq<int>,
    c: int,
)
    requires
        pinned.len() == s.len(),
        pattern.len() == pat_idx.len(),
        forall|k: int|
            0 <= k < pat_idx.len() ==> 0 <= #[trigger] pat_idx[k] < s.len() && !pinned[pat_idx[k]]
                && s[pat_idx[k]].offset == Some(pattern[k]),
        forall|x: u16| #[trigger] avoid.contains(x) ==> is_pinned_port(s, pinned, x),
        pattern_rejected(scan_probe, range_max, pattern, avoid, c),
    ensures
        base_rejected(scan_probe, range_max, s, pinned, c),
{
    let k = choose|k: int|
        0 <= k < pattern.len() && #[trigger] port_rejected(
            scan_probe,
            range_max,
            avoid,
            c + pattern[k],
        );
    let i = pat_idx[k];
    assert(0 <= i < s.len() && !pinned[i] && s[i].offset == Some(pattern[k]));
    assert(fallback_port_rejected(scan_probe, range_max, s, pinned, i, c));
}

/// `pat_idx[k]` is the fallback service whose offset is `pattern[k]`.
pub open spec fn pattern_index_map(
    s: Seq<RawService>,
    pinned: Seq<bool>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
) -> bool {
    &&& pattern.len() == pat_idx.len()
    &&& pinned.len() <= s.len()
    &&& forall|k: int|
        0 <= k < pat_idx.len() ==> 0 <= #[trigger] pat_idx[k] < pinned.len() && !pinned[pat_idx[k]]
            && s[pat_idx[k]].offset == Some(pattern[k])
}

/// The classification of services `0..n`: each is pinned or falls back
/// consistently with the probe, `avoid` holds exactly the pinned ports, and
/// `pattern` lists the fallback offsets in service order.
pub open spec fn classification_state<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    n: int,
    pinned: Seq<bool>,
    avoid: Seq<u16>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
) -> bool {
    &&& 0 <= n <= s.len()
    &&& pinned.len() == n
    &&& forall|k: int|
        0 <= k < n ==> if #[trigger] pinned[k] {
            pinned_ok(preferred_probe, s[k])
        } else {
            fallback_ok(preferred_probe, s[k])
        }
    &&& forall|x: u16| #[trigger] avoid.contains(x) <==> is_pinned_port(s, pinned, x)
    &&& pattern.len() == count_unpinned(pinned)
    &&& pattern_index_map(s, pinned, pattern, pat_idx)
    &&& forall|j: int|
        0 <= j < n && !#[trigger] pinned[j] ==> exists|k: int|
            0 <= k < pat_idx.len() && #[trigger] pat_idx[k] == j
}

/// Classifying service `i` (pinned to its preferred port, or falling back to
/// its offset) extends the classification state to `0..i + 1`.
proof fn lemma_classify_step<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    i: int,
    pinned: Seq<bool>,
    avoid: Seq<u16>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
    pin: bool,
)
    requires
        0 <= i < s.len(),
        classification_state(preferred_probe, s, i, pinned, avoid, pattern, pat_idx),
        pin ==> pinned_ok(preferred_probe, s[i]),
        !pin ==> fallback_ok(preferred_probe, s[i]),
    ensures
        pin ==> classification_state(
            preferred_probe,
            s,
            i + 1,
            pinned.push(true),
            avoid.push(s[i].preferred->Some_0),
            pattern,
            pat_idx,
        ),
        !pin ==> classification_state(
            preferred_probe,
            s,
            i + 1,
            pinned.push(false),
            avoid,
            pattern.push(s[i].offset->Some_0),
            pat_idx.push(i),
        ),
{
    let np = pinned.push(pin);
    assert(np.drop_last() =~= pinned);
    assert forall|x: u16| is_pinned_port(s, pinned, x) implies is_pinned_port(s, np, x) by {
        let j = choose|j: int|
            0 <= j < s.len() && 0 <= j < pinned.len() && #[trigger] pinned[j] && s[j].preferred == Some(
                x,
            );
        assert(np[j]);
    }
    if pin {
        let na = avoid.push(s[i].preferred->Some_0);
        assert forall|x: u16| #[trigger] na.contains(x) <==> is_pinned_port(s, np, x) by {
            if na.contains(x) {
                let m = choose|m: int| 0 <= m < na.len() && na[m] == x;
                if m < avoid.len() {
                    assert(avoid[m] == x);
                    assert(avoid.contains(x));
                } else {
                    assert(np[i]);
                }
            }
            if is_pinned_port(s, np, x) {
                let j = choose|j: int|
                    0 <= j < s.len() && 0 <= j < np.len() && #[trigger] np[j] && s[j].preferred
                        == Some(x);
                if j < i {
                    assert(pinned[j]);
                    assert(is_pinned_port(s, pinned, x));
                    assert(avoid.contains(x));
                    let m = choose|m: int| 0 <= m < avoid.len() && avoid[m] == x;
                    assert(na[m] == x);
                } else {
                    assert(na[na.len() - 1] == x);
                }
            }
        }
        assert forall|j: int| 0 <= j < i + 1 && !#[trigger] np[j] implies exists|k: int|
            0 <= k < pat_idx.len() && #[trigger] pat_idx[k] == j by {
            assert(pinned[j] == np[j]);
        }
    } else {
        let npat = pattern.push(s[i].offset->Some_0);
        let nidx = pat_idx.push(i);
        assert forall|x: u16| #[trigger] avoid.contains(x) <==> is_pinned_port(s, np, x) by {
            if is_pinned_port(s, np, x) {
                let j = choose|j: int|
                    0 <= j < s.len() && 0 <= j < np.len() && #[trigger] np[j] && s[j].preferred
                        == Some(x);
                assert(pinned[j]);
                assert(is_pinned_port(s, pinned, x));
            }
        }
        assert forall|k: int| 0 <= k < nidx.len() implies 0 <= #[trigger] nidx[k] < np.len()
            && !np[nidx[k]] && s[nidx[k]].offset == Some(npat[k]) by {
            if k < pat_idx.len() {
                assert(nidx[k] == pat_idx[k]);
            }
        }
        assert forall|j: int| 0 <= j < i + 1 && !#[trigger] np[j] implies exists|k: int|
            0 <= k < nidx.len() && #[trigger] nidx[k] == j by {
            if j < i {
                assert(pinned[j] == np[j]);
                let k = choose|k: int| 0 <= k < pat_idx.len() && #[trigger] pat_idx[k] == j;
                assert(nidx[k] == j);
            } else {
                assert(nidx[nidx.len() - 1] == j);
            }
        }
    }
}

/// The port the plan gives service `k`.
pub open spec fn planned_port(s: Seq<RawService>, pinned: Seq<bool>, base: Option<u16>, k: int) -> int {
    if pinned[k] {
        s[k].preferred->Some_0 as int
    } else {
        base->Some_0 + s[k].offset->Some_0
    }
}

/// The base found by the pattern search, or none for an empty pattern.
pub open spec fn base_search_result<E, F: Fn(u16) -> Result<bool, E>>(
    scan_probe: F,
    range_min: int,
    range_max: int,
    pattern: Seq<u16>,
    avoid: Seq<u16>,
    base: Option<u16>,
) -> bool {
    &&& (base is None <==> pattern.len() == 0)
    &&& base is Some ==> {
        &&& range_min <= base->Some_0
        &&& pattern_fits(scan_probe, range_max, pattern, avoid, base->Some_0 as int)
        &&& forall|c: int|
            range_min <= c < base->Some_0 ==> pattern_rejected(
                scan_probe,
                range_max,
                pattern,
                avoid,
                c,
            )
    }
}

/// A fallback service's port at the found base is usable.
proof fn lemma_fallback_port_usable<E, G, F>(
    preferred_probe: G,
    scan_probe: F,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    avoid: Seq<u16>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
    base: Option<u16>,
    range_min: int,
    range_max: int,
    j: int,
) where G: Fn(u16) -> Result<PreferredVerdict, E>, F: Fn(u16) -> Result<bool, E>
    requires
        classification_state(preferred_probe, s, s.len() as int, pinned, avoid, pattern, pat_idx),
        base_search_result(scan_probe, range_min, range_max, pattern, avoid, base),
        0 <= j < s.len(),
        !pinned[j],
    ensures
        base is Some,
        s[j].offset is Some,
        port_usable(scan_probe, range_max, avoid, base->Some_0 + s[j].offset->Some_0),
{
    let k = choose|k: int| 0 <= k < pat_idx.len() && #[trigger] pat_idx[k] == j;
    assert(s[j].offset == Some(pattern[k]));
    assert(port_usable(scan_probe, range_max, avoid, base->Some_0 as int + pattern[k] as int));
}

/// When the pattern search finds no base, every base at or above the scan
/// start is rejected for some fallback service.
proof fn lemma_exhaustion_rejects_every_base<E, G, F>(
    preferred_probe: G,
    scan_probe: F,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    avoid: Seq<u16>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
    range_min: int,
    range_max: int,
) where G: Fn(u16) -> Result<PreferredVerdict, E>, F: Fn(u16) -> Result<bool, E>
    requires
        classification_state(preferred_probe, s, s.len() as int, pinned, avoid, pattern, pat_idx),
        forall|c: int|
            range_min <= c ==> pattern_rejected(scan_probe, range_max, pattern, avoid, c),
    ensures
        classified(preferred_probe, s, pinned),
        forall|c: int| range_min <= c ==> base_rejected(scan_probe, range_max, s, pinned, c),
{
    assert forall|c: int| range_min <= c implies base_rejected(
        scan_probe,
        range_max,
        s,
        pinned,
        c,
    ) by {
        lemma_pattern_rejection_maps(scan_probe, range_max, s, pinned, pattern, avoid, pat_idx, c);
    }
}

/// A correct fresh plan: correctly placed (`fresh_plan_placement`), every
/// pinned port reported available by the preferred probe and every other
/// service legitimately falling back (`classified`), every fallback port
/// reported available by the scan probe, and the base the lowest that works:
/// every lower base in the scan range is rejected for some fallback service.
pub open spec fn fresh_plan_correct<E, G, F>(
    preferred_probe: G,
    scan_probe: F,
    s: Seq<RawService>,
    ports: Seq<u16>,
    pinned: Seq<bool>,
    base: Option<u16>,
    range_min: int,
    range_max: int,
) -> bool where G: Fn(u16) -> Result<PreferredVerdict, E>, F: Fn(u16) -> Result<bool, E> {
    &&& fresh_plan_placement(s, ports, pinned, base, range_min, range_max)
    &&& classified(preferred_probe, s, pinned)
    &&& forall|i: int|
        0 <= i < ports.len() && !#[trigger] pinned[i] ==> scan_probe.ensures(
            (ports[i],),
            Ok::<bool, E>(true),
        )
    &&& base is Some ==> forall|c: int|
        range_min <= c < base->Some_0 ==> base_rejected(scan_probe, range_max, s, pinned, c)
}

/// The assembled plan satisfies the contract of `plan_fresh_group`.
proof fn lemma_fresh_plan_valid<E, G, F>(
    preferred_probe: G,
    scan_probe: F,
    s: Seq<RawService>,
    pinned: Seq<bool>,
    avoid: Seq<u16>,
    pattern: Seq<u16>,
    pat_idx: Seq<int>,
    base: Option<u16>,
    ports: Seq<u16>,
    range_min: int,
    range_max: int,
) where G: Fn(u16) -> Result<PreferredVerdict, E>, F: Fn(u16) -> Result<bool, E>
    requires
        group_shape_valid(s),
        1 <= range_min,
        classification_state(preferred_probe, s, s.len() as int, pinned, avoid, pattern, pat_idx),
        base_search_result(scan_probe, range_min, range_max, pattern, avoid, base),
        ports.len() == s.len(),
        forall|k: int|
            0 <= k < s.len() ==> #[trigger] ports[k] as int == planned_port(s, pinned, base, k),
    ensures
        fresh_plan_correct(preferred_probe, scan_probe, s, ports, pinned, base, range_min, range_max),
{
    assert forall|i: int| 0 <= i < s.len() && !#[trigger] pinned[i] implies {
        &&& s[i].offset is Some
        &&& base is Some
        &&& ports[i] == base->Some_0 + s[i].offset->Some_0
        &&& range_min <= ports[i] <= range_max
        &&& port_usable(scan_probe, range_max, avoid, ports[i] as int)
    } by {
        lemma_fallback_port_usable(
            preferred_probe,
            scan_probe,
            s,
            pinned,
            avoid,
            pattern,
            pat_idx,
            base,
            range_min,
            range_max,
            i,
        );
        assert(ports[i] as int == planned_port(s, pinned, base, i));
    }
    assert forall|i: int| 0 <= i < s.len() && #[trigger] pinned[i] implies s[i].preferred == Some(
        ports[i],
    ) by {
        assert(ports[i] as int == planned_port(s, pinned, base, i));
        assert(pinned_ok(preferred_probe, s[i]));
    }
    if pattern.len() == 0 {
        assert forall|j: int| 0 <= j < s.len() implies #[trigger] pinned[j] by {
            if !pinned[j] {
                let k = choose|k: int| 0 <= k < pat_idx.len() && #[trigger] pat_idx[k] == j;
            }
        }
    } else {
        assert(!pinned[pat_idx[0]]);
    }
    assert forall|x: int, y: int|
        0 <= x < s.len() && 0 <= y < s.len() && x != y implies #[trigger] ports[x]
        != #[trigger] ports[y] by {
        assert(!services_clash(s[x], s[y]));
        if pinned[x] && !pinned[y] {
            assert(is_pinned_port(s, pinned, ports[x]));
            assert(avoid.contains(ports[x]));
        } else if !pinned[x] && pinned[y] {
            assert(is_pinned_port(s, pinned, ports[y]));
            assert(avoid.contains(ports[y]));
        }
    }
    assert forall|x: int| 0 <= x < s.len() implies 1 <= #[trigger] ports[x] by {
        assert(service_well_formed(s[x]));
        if pinned[x] {
            assert(s[x].preferred == Some(ports[x]));
        }
    }
    if base is Some {
        assert forall|c: int| range_min <= c < base->Some_0 implies base_rejected(
            scan_probe,
            range_max,
            s,
            pinned,
            c,
        ) by {
            lemma_pattern_rejection_maps(
                scan_probe,
                range_max,
                s,
                pinned,
                pattern,
                avoid,
                pat_idx,
                c,
            );
        }
    }
}

/// The pinned flags, pinned ports, fallback offset pattern, and (ghost) the
/// service index of each pattern entry; or the classification error.
type ClassifyResult<E> = Result<(Vec<bool>, Vec<u16>, Vec<u16>, Ghost<Seq<int>>), FreshGroupError<E>>;

/// Classifies the services in order: a service whose preferred port the
/// probe reports available is pinned to it; any other service falls back
/// to its offset, or fails when it has none. Returns the pinned flags, the
/// pinned ports (to avoid), and the fallback offset pattern.
fn classify_services<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    shape: &ValidatedGroupShape,
    preferred_probe: &G,
) -> (r: ClassifyResult<E>)
    requires
        forall|q: u16|
            super::port::is_valid_port(q as int) ==> #[trigger] preferred_probe.requires((q,)),
    ensures
        r matches Ok((pinned, avoid, pattern, pat_idx)) ==> classification_state(
            *preferred_probe,
            shape.services_view(),
            shape.services_view().len() as int,
            pinned@,
            avoid@,
            pattern@,
            pat_idx@,
        ),
        r matches Err(FreshGroupError::PreferredOnlyUnavailable { port, verdict }) ==> {
            &&& verdict != PreferredVerdict::Available
            &&& preferred_probe.ensures((port,), Ok::<PreferredVerdict, E>(verdict))
            &&& exists|index: int| #[trigger]
                preferred_only_failed_at(*preferred_probe, shape.services_view(), index, port)
        },
        r matches Err(FreshGroupError::Probe(e)) ==> some_preferred_failed(
            *preferred_probe,
            shape.services_view(),
            e,
        ),
        !(r matches Err(FreshGroupError::PatternExhausted { .. })),
{
    let ss = shape.services();
    let ghost s = shape.services_view();
    let mut pinned: Vec<bool> = Vec::new();
    let mut avoid: Vec<u16> = Vec::new();
    let mut pattern: Vec<u16> = Vec::new();
    let ghost mut pat_idx: Seq<int> = Seq::empty();
    let mut i: usize = 0;
    while i < ss.len()
        invariant
            ss@ == s,
            s == shape.services_view(),
            group_shape_valid(s),
            forall|q: u16|
                super::port::is_valid_port(q as int) ==> #[trigger] preferred_probe.requires((q,)),
            classification_state(*preferred_probe, s, i as int, pinned@, avoid@, pattern@, pat_idx),
        decreases s.len() - i,
    {
        assert(service_well_formed(s[i as int]));
        let offset = ss[i].offset;
        let (pin, port) = if let Some(p) = ss[i].preferred {
            let verdict = match preferred_probe(p) {
                Ok(verdict) => verdict,
                Err(e) => {
                    assert(preferred_failed(*preferred_probe, s[i as int], e));
                    assert(some_preferred_failed(*preferred_probe, s, e));
                    return Err(FreshGroupError::Probe(e));
                },
            };
            if let PreferredVerdict::Available = verdict {
                (true, p)
            } else if let Some(o) = offset {
                assert(fallback_ok(*preferred_probe, s[i as int]));
                (false, o)
            } else {
                proof {
                    assert forall|k: int| 0 <= k < i implies pinned_ok(
                        *preferred_probe,
                        #[trigger] s[k],
                    ) || fallback_ok(*preferred_probe, s[k]) by {
                        if pinned@[k] {
                        }
                    }
                    assert(preferred_only_failed_at(*preferred_probe, s, i as int, p));
                }
                return Err(FreshGroupError::PreferredOnlyUnavailable { port: p, verdict });
            }
        } else {
            (false, offset.unwrap())
        };
        proof {
            lemma_classify_step(*preferred_probe, s, i as int, pinned@, avoid@, pattern@, pat_idx, pin);
        }
        pinned.push(pin);
        if pin {
            avoid.push(port);
        } else {
            proof {
                pat_idx = pat_idx.push(i as int);
            }
            pattern.push(port);
        }
        i += 1;
    }
    Ok((pinned, avoid, pattern, Ghost(pat_idx)))
}

/// Plans a fresh group allocation (services in the order given).
///
/// Each service with a preferred port is probed in order. An available
/// preferred port is pinned (preferred ports use the whole port domain); an
/// unavailable one falls back to the service's offset, or fails when the
/// service has none. The fallback offsets, in service order, then form the
/// pattern for `find_pattern_base` over the scan range, avoiding the pinned
/// ports. The plan gives every service a valid port, and all ports are
/// pairwise distinct.
pub fn plan_fresh_group<E, G, F>(
    shape: &ValidatedGroupShape,
    span: PortSpan,
    preferred_probe: &G,
    scan_probe: &F,
) -> (r: Result<FreshGroupPlan, FreshGroupError<E>>) where
    G: Fn(u16) -> Result<PreferredVerdict, E>,
    F: Fn(u16) -> Result<bool, E>,

    requires
        forall|q: u16|
            super::port::is_valid_port(q as int) ==> #[trigger] preferred_probe.requires((q,)),
        forall|q: u16|
            span.spec_min() <= q <= span.spec_max() ==> #[trigger] scan_probe.requires((q,)),
    ensures
        r matches Ok(plan) ==> fresh_plan_correct(
            *preferred_probe,
            *scan_probe,
            shape.services_view(),
            plan.ports@,
            plan.pinned@,
            plan.base,
            span.spec_min() as int,
            span.spec_max() as int,
        ),
        r matches Err(FreshGroupError::PreferredOnlyUnavailable { port, verdict }) ==> {
            &&& verdict != PreferredVerdict::Available
            &&& preferred_probe.ensures((port,), Ok::<PreferredVerdict, E>(verdict))
            &&& exists|index: int| #[trigger]
                preferred_only_failed_at(*preferred_probe, shape.services_view(), index, port)
        },
        r matches Err(FreshGroupError::PatternExhausted { fallback_count }) ==> exists|
            pinned: Seq<bool>,
        |
            {
                &&& #[trigger] classified(*preferred_probe, shape.services_view(), pinned)
                &&& fallback_count == count_unpinned(pinned)
                &&& fallback_count >= 1
                &&& forall|c: int|
                    span.spec_min() <= c ==> base_rejected(
                        *scan_probe,
                        span.spec_max() as int,
                        shape.services_view(),
                        pinned,
                        c,
                    )
            },
        r matches Err(FreshGroupError::Probe(e)) ==> probe_error_source(
            *preferred_probe,
            *scan_probe,
            shape.services_view(),
            span.spec_min() as int,
            span.spec_max() as int,
            e,
        ),
{
    let ss = shape.services();
    let ghost s = shape.services_view();
    let range_min = span.min();
    let range_max = span.max();
    let (pinned, avoid, pattern, Ghost(pat_idx)) = match classify_services(shape, preferred_probe) {
        Ok(classification) => classification,
        Err(error) => {
            return Err(error);
        },
    };
    // Find the lowest base for the fallback pattern, avoiding pinned ports.
    let base = if pattern.is_empty() {
        None
    } else {
        let found = find_pattern_base(
            range_min,
            range_max,
            pattern.as_slice(),
            avoid.as_slice(),
            scan_probe,
        );
        match found {
            Ok(Some(b)) => Some(b),
            Ok(None) => {
                proof {
                    lemma_exhaustion_rejects_every_base(*preferred_probe, *scan_probe, s, pinned@,
                        avoid@, pattern@, pat_idx, range_min as int, range_max as int);
                }
                return Err(FreshGroupError::PatternExhausted { fallback_count: pattern.len() });
            },
            Err(e) => {
                proof {
                    let q = choose|q: u16|
                        #[trigger] probe_domain(range_min as int, range_max as int, avoid@, q)
                            && scan_probe.ensures((q,), Err::<bool, E>(e));
                    assert(range_min <= q <= range_max);
                }
                return Err(FreshGroupError::Probe(e));
            },
        }
    };
    // Place every service.
    let mut ports: Vec<u16> = Vec::new();
    let mut j: usize = 0;
    while j < ss.len()
        invariant
            ss@ == s,
            s == shape.services_view(),
            classification_state(*preferred_probe, s, s.len() as int, pinned@, avoid@, pattern@,
                pat_idx),
            base_search_result(*scan_probe, range_min as int, range_max as int, pattern@, avoid@,
                base),
            0 <= j <= s.len(),
            ports@.len() == j,
            forall|k: int|
                0 <= k < j ==> #[trigger] ports@[k] as int == planned_port(s, pinned@, base, k),
        decreases s.len() - j,
    {
        if pinned[j] {
            assert(pinned_ok(*preferred_probe, s[j as int]));
            ports.push(ss[j].preferred.unwrap());
        } else {
            proof {
                lemma_fallback_port_usable(*preferred_probe, *scan_probe, s, pinned@, avoid@,
                    pattern@, pat_idx, base, range_min as int, range_max as int, j as int);
            }
            ports.push(base.unwrap() + ss[j].offset.unwrap());
        }
        j += 1;
    }
    proof {
        lemma_fresh_plan_valid(*preferred_probe, *scan_probe, s, pinned@, avoid@, pattern@,
            pat_idx, base, ports@, range_min as int, range_max as int);
    }
    Ok(FreshGroupPlan { ports, pinned, base })
}

// ---------------------------------------------------------------------------
// Matching stored rows to requested services
// ---------------------------------------------------------------------------

/// Why the stored rows of a group do not match the requested services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagMatchError {
    /// Stored row `index` has no tag.
    Untagged {
        /// Stored row index.
        index: usize,
    },
    /// Stored row `index` repeats the tag of an earlier row.
    DuplicateStored {
        /// Stored row index.
        index: usize,
    },
    /// The stored tags and the requested tags differ as sets.
    SetMismatch,
}

/// The tag of stored row `k` (meaningful when the row is tagged).
pub open spec fn stored_tag(stored: Seq<Option<String>>, k: int) -> Seq<char> {
    stored[k]->Some_0@
}

/// Stored rows `0..n` are all tagged, with pairwise distinct tags.
pub open spec fn stored_prefix_ok(stored: Seq<Option<String>>, n: int) -> bool {
    &&& forall|k: int| 0 <= k < n ==> (#[trigger] stored[k]) is Some
    &&& forall|j: int, k: int|
        0 <= j < n && 0 <= k < n && j != k ==> #[trigger] stored_tag(stored, j)
            != #[trigger] stored_tag(
            stored,
            k,
        )
}

/// Some stored row is tagged `t`.
pub open spec fn tag_stored(stored: Seq<Option<String>>, t: Seq<char>) -> bool {
    exists|k: int| 0 <= k < stored.len() && stored[k] is Some && #[trigger] stored_tag(stored, k) == t
}

/// Some requested service is tagged `t`.
pub open spec fn tag_requested(req: Seq<RawService>, t: Seq<char>) -> bool {
    exists|i: int| 0 <= i < req.len() && #[trigger] req[i].tag@ == t
}

/// The requested and stored tags are the same set (and, both being
/// duplicate-free, the same multiset).
pub open spec fn same_tag_sets(req: Seq<RawService>, stored: Seq<Option<String>>) -> bool {
    &&& req.len() == stored.len()
    &&& forall|i: int| 0 <= i < req.len() ==> tag_stored(stored, #[trigger] req[i].tag@)
    &&& forall|k: int| 0 <= k < stored.len() ==> tag_requested(req, #[trigger] stored_tag(stored, k))
}

/// Stored row `k` is the image of some requested service under `m`.
pub open spec fn row_matched(m: Seq<usize>, k: int) -> bool {
    exists|i: int| 0 <= i < m.len() && m[i] == k
}

/// The first stored row tagged `tag`.
fn find_stored(stored: &[Option<String>], tag: &String) -> (r: Option<usize>)
    ensures
        r matches Some(k) ==> k < stored@.len() && stored@[k as int] is Some && stored_tag(
            stored@,
            k as int,
        ) == tag@,
        r is None ==> !tag_stored(stored@, tag@),
{
    let mut k: usize = 0;
    while k < stored.len()
        invariant
            0 <= k <= stored@.len(),
            forall|j: int|
                0 <= j < k && stored@[j] is Some ==> #[trigger] stored_tag(stored@, j) != tag@,
        decreases stored@.len() - k,
    {
        if let Some(t) = &stored[k] {
            if *t == *tag {
                return Some(k);
            }
        }
        k += 1;
    }
    None
}

/// Whether some requested service is tagged `tag`.
fn requested_has(req: &[RawService], tag: &String) -> (found: bool)
    ensures
        found == tag_requested(req@, tag@),
{
    let mut i: usize = 0;
    while i < req.len()
        invariant
            0 <= i <= req@.len(),
            forall|j: int| 0 <= j < i ==> #[trigger] req@[j].tag@ != tag@,
        decreases req@.len() - i,
    {
        if req[i].tag == *tag {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether a stored row before `k` carries `tag`.
fn earlier_stored_tag(stored: &[Option<String>], k: usize, tag: &String) -> (found: bool)
    requires
        k <= stored@.len(),
        forall|j: int| 0 <= j < k ==> (#[trigger] stored@[j]) is Some,
    ensures
        found == exists|j: int| 0 <= j < k && #[trigger] stored_tag(stored@, j) == tag@,
{
    let mut j: usize = 0;
    while j < k
        invariant
            0 <= j <= k <= stored@.len(),
            forall|i: int| 0 <= i < k ==> (#[trigger] stored@[i]) is Some,
            forall|i: int| 0 <= i < j ==> #[trigger] stored_tag(stored@, i) != tag@,
        decreases k - j,
    {
        if let Some(other) = &stored[j] {
            if *other == *tag {
                assert(stored_tag(stored@, j as int) == tag@);
                return true;
            }
        }
        j += 1;
    }
    false
}

/// Checks that every stored row is tagged and no tag repeats, reporting the
/// first offending row.
fn check_stored_rows(stored: &[Option<String>]) -> (r: Result<(), TagMatchError>)
    ensures
        r is Ok <==> stored_prefix_ok(stored@, stored@.len() as int),
        r matches Err(TagMatchError::Untagged { index }) ==> {
            &&& index < stored@.len()
            &&& stored@[index as int] is None
            &&& stored_prefix_ok(stored@, index as int)
        },
        r matches Err(TagMatchError::DuplicateStored { index }) ==> {
            &&& index < stored@.len()
            &&& stored@[index as int] is Some
            &&& stored_prefix_ok(stored@, index as int)
            &&& exists|j: int|
                0 <= j < index && #[trigger] stored_tag(stored@, j) == stored_tag(
                    stored@,
                    index as int,
                )
        },
        !(r matches Err(TagMatchError::SetMismatch)),
{
    let mut k: usize = 0;
    while k < stored.len()
        invariant
            0 <= k <= stored@.len(),
            stored_prefix_ok(stored@, k as int),
        decreases stored@.len() - k,
    {
        let Some(tag) = &stored[k] else {
            return Err(TagMatchError::Untagged { index: k });
        };
        if earlier_stored_tag(stored, k, tag) {
            proof {
                let j = choose|j: int| 0 <= j < k && #[trigger] stored_tag(stored@, j) == tag@;
                assert(stored_tag(stored@, j) == stored_tag(stored@, k as int));
            }
            return Err(TagMatchError::DuplicateStored { index: k });
        }
        proof {
            assert forall|a: int, b: int|
                0 <= a < k + 1 && 0 <= b < k + 1 && a != b implies #[trigger] stored_tag(
                stored@,
                a,
            ) != #[trigger] stored_tag(stored@, b) by {
                if a == k {
                    assert(stored_tag(stored@, b) != tag@);
                } else if b == k {
                    assert(stored_tag(stored@, a) != tag@);
                }
            }
        }
        k += 1;
    }
    Ok(())
}

/// Matches the stored rows of a group (in database order) to the requested
/// services (in the order of `requested`).
///
/// Untagged or duplicate stored rows are reported first (the first such row
/// in order), then a mismatch between the two tag sets. On success
/// `m[i]` is the stored row of requested service `i`, and `m` is a
/// bijection between requested services and stored rows.
pub fn match_stored_tags(requested: &ValidatedGroupShape, stored: &[Option<String>]) -> (r: Result<
    Vec<usize>,
    TagMatchError,
>)
    ensures
        r is Ok <==> stored_prefix_ok(stored@, stored@.len() as int) && same_tag_sets(
            requested.services_view(),
            stored@,
        ),
        r matches Err(TagMatchError::Untagged { index }) ==> {
            &&& index < stored@.len()
            &&& stored@[index as int] is None
            &&& stored_prefix_ok(stored@, index as int)
        },
        r matches Err(TagMatchError::DuplicateStored { index }) ==> {
            &&& index < stored@.len()
            &&& stored@[index as int] is Some
            &&& stored_prefix_ok(stored@, index as int)
            &&& exists|j: int|
                0 <= j < index && #[trigger] stored_tag(stored@, j) == stored_tag(
                    stored@,
                    index as int,
                )
        },
        r matches Err(TagMatchError::SetMismatch) ==> {
            &&& stored_prefix_ok(stored@, stored@.len() as int)
            &&& !same_tag_sets(requested.services_view(), stored@)
        },
        r matches Ok(m) ==> {
            let req = requested.services_view();
            &&& m@.len() == req.len()
            &&& forall|i: int|
                0 <= i < req.len() ==> (#[trigger] m@[i] as int) < stored@.len()
                    && stored@[m@[i] as int]
                    is Some && stored_tag(stored@, m@[i] as int) == req[i].tag@
            &&& forall|i: int, j: int|
                0 <= i < req.len() && 0 <= j < req.len() && i != j ==> #[trigger] m@[i]
                    != #[trigger] m@[j]
            &&& forall|k: int| 0 <= k < stored@.len() ==> #[trigger] row_matched(m@, k)
        },
{
    let req = requested.services();
    check_stored_rows(stored)?;
    let ghost s = requested.services_view();
    if req.len() != stored.len() {
        return Err(TagMatchError::SetMismatch);
    }
    // Every requested tag is stored.
    let mut m: Vec<usize> = Vec::new();
    let mut i: usize = 0;
    while i < req.len()
        invariant
            req@ == s,
            s == requested.services_view(),
            group_shape_valid(s),
            req@.len() == stored@.len(),
            stored_prefix_ok(stored@, stored@.len() as int),
            0 <= i <= req@.len(),
            m@.len() == i,
            forall|x: int|
                0 <= x < i ==> (#[trigger] m@[x] as int) < stored@.len()
                    && stored@[m@[x] as int] is Some
                    && stored_tag(stored@, m@[x] as int) == s[x].tag@,
        decreases req@.len() - i,
    {
        match find_stored(stored, &req[i].tag) {
            None => {
                assert(!tag_stored(stored@, s[i as int].tag@));
                return Err(TagMatchError::SetMismatch);
            },
            Some(found) => {
                m.push(found);
            },
        }
        i += 1;
    }
    // Every stored tag is requested.
    let mut k: usize = 0;
    while k < stored.len()
        invariant
            req@ == s,
            s == requested.services_view(),
            group_shape_valid(s),
            req@.len() == stored@.len(),
            stored_prefix_ok(stored@, stored@.len() as int),
            0 <= k <= stored@.len(),
            forall|j: int| 0 <= j < k ==> tag_requested(s, #[trigger] stored_tag(stored@, j)),
        decreases stored@.len() - k,
    {
        let requested_tag = match &stored[k] {
            Some(tag) => requested_has(req, tag),
            None => false,
        };
        if !requested_tag {
            assert(!tag_requested(s, stored_tag(stored@, k as int)));
            return Err(TagMatchError::SetMismatch);
        }
        k += 1;
    }
    proof {
        assert forall|x: int| 0 <= x < s.len() implies tag_stored(stored@, #[trigger] s[x].tag@) by {
            assert(stored_tag(stored@, m@[x] as int) == s[x].tag@);
        }
        // Injective: distinct requested tags.
        assert forall|x: int, y: int|
            0 <= x < s.len() && 0 <= y < s.len() && x != y implies #[trigger] m@[x]
                != #[trigger] m@[y] by {
            assert(!services_clash(s[x], s[y]));
        }
        // Surjective: every stored row carries some requested tag.
        assert forall|kk: int| 0 <= kk < stored@.len() implies #[trigger] row_matched(m@, kk) by {
            assert(tag_requested(s, stored_tag(stored@, kk)));
            let x = choose|x: int| 0 <= x < s.len() && #[trigger] s[x].tag@ == stored_tag(stored@, kk);
            assert(stored_tag(stored@, m@[x] as int) == stored_tag(stored@, kk));
            assert(m@[x] == kk);
        }
    }
    Ok(m)
}

// ---------------------------------------------------------------------------
// Shape compatibility of a stored group
// ---------------------------------------------------------------------------

/// A stored service row with the requested strategy of its service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoredService {
    /// The stored port.
    pub stored: u16,
    /// The requested preferred port.
    pub preferred: Option<u16>,
    /// The requested offset.
    pub offset: Option<u16>,
}

/// Why a stored group no longer has the requested shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatError {
    /// The stored port is not the preferred port, and the service has no
    /// offset fallback.
    NoOffsetFallback {
        /// Service index.
        index: usize,
    },
    /// The stored port is below the offset.
    OffsetUnderflow {
        /// Service index.
        index: usize,
    },
    /// The stored port implies base 0.
    ZeroBase {
        /// Service index.
        index: usize,
    },
    /// The implied base or the stored port is outside the scan range.
    OutOfScanRange {
        /// Service index.
        index: usize,
    },
    /// The implied base differs from the base of earlier services.
    BaseMismatch {
        /// Service index.
        index: usize,
        /// The base this service implies.
        candidate: u16,
        /// The base earlier services imply.
        base: u16,
    },
}

/// The service sits on its preferred port (which then imposes nothing).
pub open spec fn matches_preferred(s: StoredService) -> bool {
    s.preferred == Some(s.stored)
}

/// The service fits base `b`: it sits on its preferred port, or it sits at
/// `b + offset` with both `b` and the port in the scan range and `b` a valid
/// port.
pub open spec fn fits_base(s: StoredService, range_min: int, range_max: int, b: int) -> bool {
    ||| matches_preferred(s)
    ||| {
        &&& s.offset is Some
        &&& s.stored == b + s.offset->Some_0
        &&& 1 <= b
        &&& range_min <= b <= range_max
        &&& range_min <= s.stored <= range_max
    }
}

/// Every service fits base `b`.
pub open spec fn all_fit(services: Seq<StoredService>, range_min: int, range_max: int, b: int) -> bool {
    forall|i: int|
        0 <= i < services.len() ==> #[trigger] fits_base(services[i], range_min, range_max, b)
}

/// The base a service not on its preferred port implies.
pub open spec fn implied_base(s: StoredService) -> int {
    s.stored - s.offset->Some_0
}

/// The per-service check, in production order: no offset fallback, offset
/// underflow, base 0, out of scan range.
pub open spec fn local_compat_error(s: StoredService, i: int, range_min: int, range_max: int) -> Option<
    CompatError,
> {
    if matches_preferred(s) {
        None
    } else if s.offset is None {
        Some(CompatError::NoOffsetFallback { index: i as usize })
    } else if s.stored < s.offset->Some_0 {
        Some(CompatError::OffsetUnderflow { index: i as usize })
    } else if s.stored == s.offset->Some_0 {
        Some(CompatError::ZeroBase { index: i as usize })
    } else if !(range_min <= implied_base(s) <= range_max && range_min <= s.stored <= range_max) {
        Some(CompatError::OutOfScanRange { index: i as usize })
    } else {
        None
    }
}

/// The service index a compatibility error names.
pub open spec fn compat_error_index(e: CompatError) -> int {
    match e {
        CompatError::NoOffsetFallback { index } => index as int,
        CompatError::OffsetUnderflow { index } => index as int,
        CompatError::ZeroBase { index } => index as int,
        CompatError::OutOfScanRange { index } => index as int,
        CompatError::BaseMismatch { index, .. } => index as int,
    }
}

/// The full contract of `derive_compatible_base`.
pub open spec fn compat_result_spec(
    services: Seq<StoredService>,
    range_min: int,
    range_max: int,
    r: Result<Option<u16>, CompatError>,
) -> bool {
    // Soundness.
    &&& (r matches Ok(Some(b)) ==> all_fit(services, range_min, range_max, b as int) && exists|i: int|
        0 <= i < services.len() && !#[trigger] matches_preferred(services[i]))
    &&& (r matches Ok(None) ==> forall|i: int|
        0 <= i < services.len() ==> #[trigger] matches_preferred(services[i]))
    // Completeness: whenever some base fits every service, the group is accepted.
    &&& ((exists|b: int| all_fit(services, range_min, range_max, b)) ==> r is Ok)
    &&& (r is Err ==> forall|b: int| !all_fit(services, range_min, range_max, b))
    &&& ((forall|i: int| 0 <= i < services.len() ==> #[trigger] matches_preferred(services[i])) ==> r
        == Ok::<Option<u16>, CompatError>(None))
    // Uniqueness: the reported base is the only one that fits.
    &&& (r matches Ok(Some(b)) ==> forall|c: int| all_fit(services, range_min, range_max, c) ==> c == b)
    // Error precedence: the first failing service in order.
    &&& (r matches Err(e) ==> {
        let i = compat_error_index(e);
        &&& 0 <= i < services.len()
        &&& forall|k: int|
            0 <= k < i ==> #[trigger] local_compat_error(services[k], k, range_min, range_max)
                is None
        &&& forall|k: int, l: int|
            0 <= k < i && 0 <= l < i && !matches_preferred(services[k])
                && !matches_preferred(services[l])
                ==> #[trigger] implied_base(services[k]) == #[trigger] implied_base(services[l])
        &&& match e {
            CompatError::BaseMismatch { index, candidate, base } => {
                &&& local_compat_error(services[i], i, range_min, range_max) is None
                &&& !matches_preferred(services[i])
                &&& candidate as int == implied_base(services[i])
                &&& candidate != base
                &&& exists|k: int|
                    0 <= k < i && !matches_preferred(services[k]) && #[trigger] implied_base(
                        services[k],
                    ) == base as int
            },
            _ => local_compat_error(services[i], i, range_min, range_max) == Some(e),
        }
    })
}

/// Checks that a stored group (services in tag order, each with its stored
/// port) still has the requested shape under the current scan range, and
/// derives its base.
///
/// A service on its preferred port imposes nothing. Every other service
/// must sit at `base + offset` for one common base, with base and port in
/// the scan range. The result is `None` when every service is on its
/// preferred port. See `compat_result_spec` for the full contract:
/// soundness, completeness, uniqueness of the base, and error precedence.
pub fn derive_compatible_base(services: &[StoredService], range_min: u16, range_max: u16) -> (r:
    Result<Option<u16>, CompatError>)
    ensures
        compat_result_spec(services@, range_min as int, range_max as int, r),
{
    let ghost s = services@;
    let ghost lo = range_min as int;
    let ghost hi = range_max as int;
    let mut base: Option<u16> = None;
    let mut i: usize = 0;
    while i < services.len()
        invariant
            s == services@,
            lo == range_min as int,
            hi == range_max as int,
            0 <= i <= s.len(),
            forall|k: int| 0 <= k < i ==> #[trigger] local_compat_error(s[k], k, lo, hi) is None,
            base is None <==> forall|k: int| 0 <= k < i ==> #[trigger] matches_preferred(s[k]),
            base is Some ==> {
                &&& forall|k: int|
                    0 <= k < i && !matches_preferred(s[k]) ==> #[trigger] implied_base(s[k])
                        == base->Some_0 as int
                &&& exists|k: int| 0 <= k < i && !#[trigger] matches_preferred(s[k])
            },
        decreases s.len() - i,
    {
        let svc = services[i];
        let on_preferred = match svc.preferred {
            Some(p) => p == svc.stored,
            None => false,
        };
        if !on_preferred {
            let Some(offset) = svc.offset else {
                proof {
                    lemma_compat_error_excludes_fit(s, i as int, lo, hi);
                }
                return Err(CompatError::NoOffsetFallback { index: i });
            };
            if svc.stored < offset {
                proof { lemma_compat_error_excludes_fit(s, i as int, lo, hi); }
                return Err(CompatError::OffsetUnderflow { index: i });
            }
            let candidate = svc.stored - offset;
            if candidate == 0 {
                proof { lemma_compat_error_excludes_fit(s, i as int, lo, hi); }
                return Err(CompatError::ZeroBase { index: i });
            }
            if !(range_min <= candidate && candidate <= range_max && range_min <= svc.stored
                && svc.stored <= range_max) {
                proof { lemma_compat_error_excludes_fit(s, i as int, lo, hi); }
                return Err(CompatError::OutOfScanRange { index: i });
            }
            match base {
                Some(b) => {
                    if b != candidate {
                        proof {
                            let k = choose|k: int| 0 <= k < i && !#[trigger] matches_preferred(s[k]);
                            assert(implied_base(s[k]) == b as int);
                            assert forall|c: int| !all_fit(s, lo, hi, c) by {
                                if all_fit(s, lo, hi, c) {
                                    assert(fits_base(s[k], lo, hi, c));
                                    assert(fits_base(s[i as int], lo, hi, c));
                                }
                            }
                        }
                        return Err(CompatError::BaseMismatch { index: i, candidate, base: b });
                    }
                },
                None => {
                    base = Some(candidate);
                },
            }
        }
        proof {
            assert(local_compat_error(s[i as int], i as int, lo, hi) is None);
        }
        i += 1;
    }
    proof {
        if base is Some {
            let b = base->Some_0 as int;
            assert forall|k: int| 0 <= k < s.len() implies #[trigger] fits_base(s[k], lo, hi, b) by {
                assert(local_compat_error(s[k], k, lo, hi) is None);
            }
            assert forall|c: int| all_fit(s, lo, hi, c) implies c == b by {
                let k = choose|k: int| 0 <= k < s.len() && !#[trigger] matches_preferred(s[k]);
                assert(implied_base(s[k]) == b);
                assert(fits_base(s[k], lo, hi, c));
            }
        }
    }
    Ok(base)
}

/// A service with a local error fits no base, so neither does the group.
proof fn lemma_compat_error_excludes_fit(s: Seq<StoredService>, i: int, lo: int, hi: int)
    requires
        0 <= i < s.len(),
        local_compat_error(s[i], i, lo, hi) is Some,
    ensures
        forall|c: int| !all_fit(s, lo, hi, c),
{
    assert forall|c: int| !all_fit(s, lo, hi, c) by {
        if all_fit(s, lo, hi, c) {
            assert(fits_base(s[i], lo, hi, c));
        }
    }
}

// ---------------------------------------------------------------------------
// Idempotence: a fresh plan is always compatible
// ---------------------------------------------------------------------------

/// The stored view of a freshly planned group: each service with the port
/// the plan gave it.
pub open spec fn stored_view_of_plan(s: Seq<RawService>, ports: Seq<u16>) -> Seq<StoredService> {
    Seq::new(
        s.len(),
        |i: int| StoredService { stored: ports[i], preferred: s[i].preferred, offset: s[i].offset },
    )
}

/// Repeating a group request right after a fresh allocation refreshes it:
/// for every fresh plan, `derive_compatible_base` on the stored group (same
/// services, same scan range) accepts it, so the request takes the refresh
/// path -- never a conflict or a forced replacement -- and keeps exactly the
/// planned ports. A reported base equals the plan's base. It can only be
/// `None` while the plan has a base when every service ended up on its own
/// preferred port (a fallback port can coincide with the service's own
/// unavailable preference if the probes' answers change between calls).
pub proof fn lemma_fresh_plan_is_compatible(
    s: Seq<RawService>,
    ports: Seq<u16>,
    pinned: Seq<bool>,
    base: Option<u16>,
    range_min: int,
    range_max: int,
    r: Result<Option<u16>, CompatError>,
)
    requires
        fresh_plan_placement(s, ports, pinned, base, range_min, range_max),
        1 <= range_min,
        compat_result_spec(stored_view_of_plan(s, ports), range_min, range_max, r),
    ensures
        r is Ok,
        r matches Ok(Some(b)) ==> base == Some(b),
        r matches Ok(None) ==> base is None || forall|i: int|
            0 <= i < s.len() ==> #[trigger] s[i].preferred == Some(ports[i]),
        base is None ==> r == Ok::<Option<u16>, CompatError>(None),
{
    let v = stored_view_of_plan(s, ports);
    if base is Some {
        let b = base->Some_0 as int;
        assert forall|i: int| 0 <= i < v.len() implies #[trigger] fits_base(
            v[i],
            range_min,
            range_max,
            b,
        ) by {
            if pinned[i] {
                assert(matches_preferred(v[i]));
            }
        }
        assert(all_fit(v, range_min, range_max, b));
        if r matches Ok(None) {
            assert forall|i: int| 0 <= i < s.len() implies #[trigger] s[i].preferred == Some(
                ports[i],
            ) by {
                assert(matches_preferred(v[i]));
            }
        }
    } else {
        assert forall|i: int| 0 <= i < v.len() implies #[trigger] matches_preferred(v[i]) by {
            assert(pinned[i]);
        }
    }
}

// ---------------------------------------------------------------------------
// Reconciliation decision
// ---------------------------------------------------------------------------

/// How the stored group compares with the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatClass {
    /// The stored group has the requested shape.
    Compatible,
    /// The stored group conflicts with the request (partial or incompatible).
    Conflict,
    /// Any other error (for example a validation error).
    OtherError,
}

/// What to do with a group request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupDecision {
    /// No stored rows: allocate a fresh group.
    AllocateFresh,
    /// Refresh the compatible stored group.
    Refresh,
    /// Delete the stored group and allocate a fresh one (forced).
    Replace,
    /// Report the compatibility error.
    Fail,
}

/// Decides how to reconcile a group request with its stored rows.
///
/// Only `--force` can replace a stored group, only when it conflicts, and
/// never a compatible group; validation-class errors are never overridden.
pub fn decide_group_reconciliation(stored_count: usize, compat: CompatClass, force: bool) -> (d:
    GroupDecision)
    ensures
        (d == GroupDecision::AllocateFresh) <==> stored_count == 0,
        (d == GroupDecision::Refresh) <==> (stored_count > 0 && compat == CompatClass::Compatible),
        (d == GroupDecision::Replace) <==> (stored_count > 0 && compat == CompatClass::Conflict
            && force),
        (d == GroupDecision::Fail) <==> (stored_count > 0 && (compat == CompatClass::OtherError || (
        compat == CompatClass::Conflict && !force))),
        !force ==> d != GroupDecision::Replace,
{
    if stored_count == 0 {
        return GroupDecision::AllocateFresh;
    }
    match compat {
        CompatClass::Compatible => GroupDecision::Refresh,
        CompatClass::Conflict => if force {
            GroupDecision::Replace
        } else {
            GroupDecision::Fail
        },
        CompatClass::OtherError => GroupDecision::Fail,
    }
}

// ---------------------------------------------------------------------------
// Configuration helpers
// ---------------------------------------------------------------------------

/// The offset of a configured service: omitted offsets default to 0, so
/// every configured service has an offset fallback (see
/// `lemma_offsets_exclude_missing_fallbacks`).
pub const fn config_service_offset(offset: Option<u16>) -> (r: u16)
    ensures
        offset matches Some(o) ==> r == o,
        offset is None ==> r == 0,
{
    match offset {
        Some(o) => o,
        None => 0,
    }
}

/// With an offset on every service -- as for every group built from
/// configuration (`config_service_offset`) -- a fresh plan never fails for
/// a preferred-only service, and a stored group is never rejected for a
/// missing offset fallback.
pub proof fn lemma_offsets_exclude_missing_fallbacks<E, G: Fn(u16) -> Result<PreferredVerdict, E>>(
    preferred_probe: G,
    s: Seq<RawService>,
    stored: Seq<StoredService>,
    range_min: int,
    range_max: int,
)
    requires
        forall|i: int| 0 <= i < s.len() ==> (#[trigger] s[i]).offset is Some,
        forall|i: int| 0 <= i < stored.len() ==> (#[trigger] stored[i]).offset is Some,
    ensures
        forall|index: int, port: u16|
            !#[trigger] preferred_only_failed_at(preferred_probe, s, index, port),
        forall|i: int|
            0 <= i < stored.len() ==> #[trigger] local_compat_error(stored[i], i, range_min, range_max)
                != Some(CompatError::NoOffsetFallback { index: i as usize }),
{
    assert forall|index: int, port: u16|
        !#[trigger] preferred_only_failed_at(preferred_probe, s, index, port) by {
        if 0 <= index < s.len() {
            assert(s[index].offset is Some);
        }
    }
    assert forall|i: int| 0 <= i < stored.len() implies #[trigger] local_compat_error(
        stored[i],
        i,
        range_min,
        range_max,
    ) != Some(CompatError::NoOffsetFallback { index: i as usize }) by {
        assert(stored[i].offset is Some);
    }
}

/// The index of the highest precedence, preferring the last one on ties
/// (as `Iterator::max_by_key` does).
pub fn select_highest_precedence(precedences: &[u8]) -> (r: Option<usize>)
    ensures
        r is None <==> precedences@.len() == 0,
        r matches Some(i) ==> {
            &&& i < precedences@.len()
            &&& forall|j: int|
                0 <= j < precedences@.len() ==> #[trigger] precedences@[j] <= precedences@[i as int]
            &&& forall|j: int|
                i < j < precedences@.len() ==> #[trigger] precedences@[j] < precedences@[i as int]
        },
{
    if precedences.is_empty() {
        return None;
    }
    let mut best: usize = 0;
    let mut i: usize = 1;
    while i < precedences.len()
        invariant
            1 <= i <= precedences@.len(),
            best < i,
            forall|j: int| 0 <= j < i ==> #[trigger] precedences@[j] <= precedences@[best as int],
            forall|j: int| best < j < i ==> #[trigger] precedences@[j] < precedences@[best as int],
        decreases precedences@.len() - i,
    {
        if precedences[i] >= precedences[best] {
            best = i;
        }
        i += 1;
    }
    Some(best)
}

/// The index of the first present candidate.
pub fn first_present(present: &[bool]) -> (r: Option<usize>)
    ensures
        r matches Some(i) ==> i < present@.len() && present@[i as int] && forall|j: int|
            0 <= j < i ==> !#[trigger] present@[j],
        r is None ==> forall|j: int| 0 <= j < present@.len() ==> !#[trigger] present@[j],
{
    let mut i: usize = 0;
    while i < present.len()
        invariant
            0 <= i <= present@.len(),
            forall|j: int| 0 <= j < i ==> !#[trigger] present@[j],
        decreases present@.len() - i,
    {
        if present[i] {
            return Some(i);
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Output selection of `reserve-group` and `autoreserve`
// ---------------------------------------------------------------------------

/// The requested `--format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatArg {
    /// Shell export statements (the one format that needs a shell).
    Export,
    /// JSON.
    Json,
    /// Dotenv lines.
    Dotenv,
    /// Human-readable text.
    Human,
}

/// The selected output: export statements for shell `S`, or a
/// shell-independent format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupOutput<S> {
    /// Export statements for this shell.
    Export(S),
    /// JSON.
    Json,
    /// Dotenv lines.
    Dotenv,
    /// Human-readable text.
    Human,
}

/// The call discipline of `select_group_output`, stated as the oracles'
/// preconditions (a body that calls an oracle out of turn does not verify
/// against oracles whose preconditions are exactly these):
///
/// - the explicit `--shell` (`parse`) is parsed only for the export format;
/// - the shell is detected (`detect`) only for the export format, and only
///   when no explicit shell was given.
///
/// Each oracle is an `FnOnce`, so each runs at most once. Ordinary Rust
/// closures have no preconditions, so production glue meets these.
pub open spec fn output_discipline<S, E, P: FnOnce() -> Result<S, E>, D: FnOnce() -> Result<S, E>>(
    format: FormatArg,
    parse: Option<P>,
    detect: D,
) -> bool {
    format == FormatArg::Export ==> match parse {
        Some(p) => p.requires(()),
        None => detect.requires(()),
    }
}

/// `r` is the output `format` selects:
///
/// - JSON, dotenv, and human output need no shell: they are selected
///   unconditionally, whatever `--shell` says;
/// - export output uses the explicit shell when one was given (an invalid one
///   is the parser's error, never replaced by detection), and otherwise the
///   detected shell (or the detector's error).
pub open spec fn output_selected<S, E, P: FnOnce() -> Result<S, E>, D: FnOnce() -> Result<S, E>>(
    format: FormatArg,
    parse: Option<P>,
    detect: D,
    r: Result<GroupOutput<S>, E>,
) -> bool {
    match format {
        FormatArg::Json => r == Ok::<GroupOutput<S>, E>(GroupOutput::Json),
        FormatArg::Dotenv => r == Ok::<GroupOutput<S>, E>(GroupOutput::Dotenv),
        FormatArg::Human => r == Ok::<GroupOutput<S>, E>(GroupOutput::Human),
        FormatArg::Export => match parse {
            Some(p) => match r {
                Ok(GroupOutput::Export(shell)) => p.ensures((), Ok::<S, E>(shell)),
                Err(e) => p.ensures((), Err::<S, E>(e)),
                _ => false,
            },
            None => match r {
                Ok(GroupOutput::Export(shell)) => detect.ensures((), Ok::<S, E>(shell)),
                Err(e) => detect.ensures((), Err::<S, E>(e)),
                _ => false,
            },
        },
    }
}

/// Selects the output of `reserve-group` / `autoreserve` (see
/// `output_selected`), consulting the shell parser and the detector only as
/// `output_discipline` allows.
pub fn select_group_output<S, E, P, D>(format: FormatArg, parse: Option<P>, detect: D) -> (r: Result<
    GroupOutput<S>,
    E,
>) where P: FnOnce() -> Result<S, E>, D: FnOnce() -> Result<S, E>
    requires
        output_discipline(format, parse, detect),
    ensures
        output_selected(format, parse, detect, r),
        format != FormatArg::Export ==> r is Ok,
        r matches Ok(GroupOutput::Export(_)) ==> format == FormatArg::Export,
{
    match format {
        FormatArg::Export => {
            let shell = match parse {
                Some(p) => p()?,
                None => detect()?,
            };
            Ok(GroupOutput::Export(shell))
        },
        FormatArg::Json => Ok(GroupOutput::Json),
        FormatArg::Dotenv => Ok(GroupOutput::Dotenv),
        FormatArg::Human => Ok(GroupOutput::Human),
    }
}

} // verus!
