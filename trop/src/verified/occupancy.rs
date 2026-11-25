//! Verified occupancy-check policy: the effective probe flags, the probe
//! matrix those flags select, the fail-closed fold of probe outcomes, and the
//! completeness status of owner metadata.
//!
//! Production glue (`crate::port::occupancy`):
//!
//! - `OccupancyCheckConfig::from(&OccupancyConfig)` calls
//!   [`effective_occupancy_flags`] and copies the resulting [`OccFlags`].
//! - `SystemOccupancyChecker` runs [`run_probe_matrix`]; its oracle closure
//!   maps each [`Probe`] to a socket address, performs the bind, and
//!   classifies the `io::Result` as a [`ProbeOutcome`]. The report is built
//!   from the returned [`MatrixEvidence`] (or the failing probe and its error).
//! - `MockOccupancyChecker` uses [`occupancy_disabled`] to decide when
//!   checking is disabled, so it agrees with the system checker's matrix.
//! - `OccupancyOwner::refresh_status` uses [`owner_status`].
//!
//! The socket binds, the `AddrInUse` classification of an `io::Error`, and
//! the owner lookups stay outside the kernel.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ---------------------------------------------------------------------------
// Effective flags
// ---------------------------------------------------------------------------

/// The `occupancy_check` settings as written in configuration. Every field is
/// optional; `None` means "not set".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccSettings {
    /// Skip every occupancy probe.
    pub skip: Option<bool>,
    /// Skip TCP probes.
    pub skip_tcp: Option<bool>,
    /// Skip UDP probes.
    pub skip_udp: Option<bool>,
    /// Skip IPv4 probes.
    pub skip_ipv4: Option<bool>,
    /// Skip IPv6 probes.
    pub skip_ipv6: Option<bool>,
    /// Also probe the wildcard (all-interfaces) addresses.
    pub check_all_interfaces: Option<bool>,
}

/// The effective occupancy policy: which probe dimensions are skipped and
/// whether wildcard addresses are probed in addition to localhost.
// One flag per independently configurable probe dimension, mirroring the
// public `OccupancyCheckConfig` field for field so the glue is a plain copy.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccFlags {
    /// Skip TCP probes.
    pub skip_tcp: bool,
    /// Skip UDP probes.
    pub skip_udp: bool,
    /// Skip IPv4 probes.
    pub skip_ipv4: bool,
    /// Skip IPv6 probes.
    pub skip_ipv6: bool,
    /// Also probe the wildcard (all-interfaces) addresses.
    pub check_all_interfaces: bool,
}

/// A configured switch is on exactly when it is explicitly `Some(true)`;
/// `None` and `Some(false)` both leave it off.
pub open spec fn switched_on(setting: Option<bool>) -> bool {
    setting == Some(true)
}

/// The effective flags for `settings`: each skip flag is on when the global
/// `skip` switch or its own switch is on, and `check_all_interfaces` is on
/// only when its switch is on.
pub open spec fn spec_effective_flags(settings: OccSettings) -> OccFlags {
    OccFlags {
        skip_tcp: switched_on(settings.skip) || switched_on(settings.skip_tcp),
        skip_udp: switched_on(settings.skip) || switched_on(settings.skip_udp),
        skip_ipv4: switched_on(settings.skip) || switched_on(settings.skip_ipv4),
        skip_ipv6: switched_on(settings.skip) || switched_on(settings.skip_ipv6),
        check_all_interfaces: switched_on(settings.check_all_interfaces),
    }
}

/// The policy with nothing skipped and only localhost probed.
pub open spec fn default_flags() -> OccFlags {
    OccFlags {
        skip_tcp: false,
        skip_udp: false,
        skip_ipv4: false,
        skip_ipv6: false,
        check_all_interfaces: false,
    }
}

/// The configured settings ask for `probe` to run: neither the global `skip`
/// switch nor the skip switch of the probe's protocol or address family is
/// on, and the probe is a localhost probe or `check_all_interfaces` is on.
pub open spec fn settings_request_probe(settings: OccSettings, probe: Probe) -> bool {
    &&& !switched_on(settings.skip)
    &&& !switched_on(
        match probe.protocol {
            Protocol::Tcp => settings.skip_tcp,
            Protocol::Udp => settings.skip_udp,
        },
    )
    &&& !switched_on(
        match probe.family {
            Family::Ipv4 => settings.skip_ipv4,
            Family::Ipv6 => settings.skip_ipv6,
        },
    )
    &&& (probe.scope == Scope::Localhost || switched_on(settings.check_all_interfaces))
}

/// Combines configured settings into the effective policy.
///
/// The global `skip` switch disables every probe; an unset or `false`
/// setting never re-enables a probe that another switch disabled.
pub fn effective_occupancy_flags(settings: OccSettings) -> (flags: OccFlags)
    ensures
        flags == spec_effective_flags(settings),
        forall|probe: Probe|
            #[trigger] probe_enabled(flags, probe) <==> settings_request_probe(settings, probe),
        switched_on(settings.skip) ==> no_probe_enabled(flags),
        settings.skip_tcp is None && settings.skip_udp is None && settings.skip_ipv4 is None
            && settings.skip_ipv6 is None && settings.skip is None
            && settings.check_all_interfaces is None ==> flags == default_flags(),
{
    let skip_all = settings.skip.unwrap_or(false);
    OccFlags {
        skip_tcp: skip_all || settings.skip_tcp.unwrap_or(false),
        skip_udp: skip_all || settings.skip_udp.unwrap_or(false),
        skip_ipv4: skip_all || settings.skip_ipv4.unwrap_or(false),
        skip_ipv6: skip_all || settings.skip_ipv6.unwrap_or(false),
        check_all_interfaces: settings.check_all_interfaces.unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// The probe matrix
// ---------------------------------------------------------------------------

/// Transport protocol of a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// Address family of a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// IPv4.
    Ipv4,
    /// IPv6.
    Ipv6,
}

/// Bind-address scope of a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The family's loopback address (`127.0.0.1` or `::1`).
    Localhost,
    /// The family's unspecified address (`0.0.0.0` or `::`).
    Wildcard,
}

/// One bind probe: a protocol, an address family, and an address scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    /// Transport protocol.
    pub protocol: Protocol,
    /// Address family.
    pub family: Family,
    /// Address scope.
    pub scope: Scope,
}

/// Whether the policy probes `protocol`.
pub open spec fn protocol_enabled(flags: OccFlags, protocol: Protocol) -> bool {
    match protocol {
        Protocol::Tcp => !flags.skip_tcp,
        Protocol::Udp => !flags.skip_udp,
    }
}

/// Whether the policy probes `family`.
pub open spec fn family_enabled(flags: OccFlags, family: Family) -> bool {
    match family {
        Family::Ipv4 => !flags.skip_ipv4,
        Family::Ipv6 => !flags.skip_ipv6,
    }
}

/// Whether the policy probes `scope`: localhost always, wildcard only when
/// `check_all_interfaces` is on.
pub open spec fn scope_enabled(flags: OccFlags, scope: Scope) -> bool {
    match scope {
        Scope::Localhost => true,
        Scope::Wildcard => flags.check_all_interfaces,
    }
}

/// Whether the policy runs `probe`: its protocol, family, and scope are all
/// enabled.
pub open spec fn probe_enabled(flags: OccFlags, probe: Probe) -> bool {
    protocol_enabled(flags, probe.protocol) && family_enabled(flags, probe.family)
        && scope_enabled(flags, probe.scope)
}

/// The policy runs no probe at all.
pub open spec fn no_probe_enabled(flags: OccFlags) -> bool {
    forall|probe: Probe| !#[trigger] probe_enabled(flags, probe)
}

/// Position of `probe` in the documented matrix order: address family first
/// (IPv4, IPv6), then scope (localhost, wildcard), then protocol (TCP, UDP).
pub open spec fn matrix_rank(probe: Probe) -> int {
    (match probe.family {
        Family::Ipv4 => 0int,
        Family::Ipv6 => 4int,
    }) + (match probe.scope {
        Scope::Localhost => 0int,
        Scope::Wildcard => 2int,
    }) + (match probe.protocol {
        Protocol::Tcp => 0int,
        Protocol::Udp => 1int,
    })
}

/// `probes` is strictly increasing in the documented matrix order (so it has
/// no duplicates).
pub open spec fn in_matrix_order(probes: Seq<Probe>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < probes.len() ==> #[trigger] matrix_rank(probes[i]) < #[trigger] matrix_rank(
            probes[j],
        )
}

/// `probes` is the probe matrix of `flags`: exactly the enabled probes, each
/// once, in documented matrix order. (`lemma_probe_matrix_unique` shows this
/// determines the sequence.)
pub open spec fn is_probe_matrix(flags: OccFlags, probes: Seq<Probe>) -> bool {
    &&& forall|probe: Probe| #[trigger] probes.contains(probe) <==> probe_enabled(flags, probe)
    &&& in_matrix_order(probes)
}

/// Number of enabled protocols (0, 1, or 2).
pub open spec fn enabled_protocol_count(flags: OccFlags) -> int {
    (if flags.skip_tcp {
        0int
    } else {
        1int
    }) + (if flags.skip_udp {
        0int
    } else {
        1int
    })
}

/// Number of enabled address families (0, 1, or 2).
pub open spec fn enabled_family_count(flags: OccFlags) -> int {
    (if flags.skip_ipv4 {
        0int
    } else {
        1int
    }) + (if flags.skip_ipv6 {
        0int
    } else {
        1int
    })
}

/// Number of enabled scopes (1, or 2 with `check_all_interfaces`).
pub open spec fn enabled_scope_count(flags: OccFlags) -> int {
    if flags.check_all_interfaces {
        2int
    } else {
        1int
    }
}

/// Number of probes the policy runs: |protocols| x |families| x |scopes|.
pub open spec fn enabled_probe_count(flags: OccFlags) -> int {
    enabled_protocol_count(flags) * enabled_family_count(flags) * enabled_scope_count(flags)
}

/// The probe at matrix position `rank` (meaningful for `0 <= rank < 8`).
pub open spec fn probe_of_rank(rank: int) -> Probe {
    Probe {
        protocol: if rank % 2 == 0 {
            Protocol::Tcp
        } else {
            Protocol::Udp
        },
        family: if rank < 4 {
            Family::Ipv4
        } else {
            Family::Ipv6
        },
        scope: if rank % 4 < 2 {
            Scope::Localhost
        } else {
            Scope::Wildcard
        },
    }
}

/// Number of enabled probes at matrix positions below `rank`.
spec fn enabled_below(flags: OccFlags, rank: int) -> int
    decreases rank,
{
    if rank <= 0 {
        0
    } else {
        enabled_below(flags, rank - 1) + if probe_enabled(flags, probe_of_rank(rank - 1)) {
            1int
        } else {
            0int
        }
    }
}

/// Matrix ranks are `0..8`, and each rank names exactly one probe.
proof fn lemma_matrix_rank(probe: Probe)
    ensures
        0 <= matrix_rank(probe) < 8,
        probe_of_rank(matrix_rank(probe)) == probe,
{
}

/// Distinct probes have distinct ranks.
proof fn lemma_matrix_rank_injective(a: Probe, b: Probe)
    ensures
        matrix_rank(a) == matrix_rank(b) ==> a == b,
{
    lemma_matrix_rank(a);
    lemma_matrix_rank(b);
}

/// Counting the enabled positions of the whole matrix gives the product of
/// the per-dimension counts.
proof fn lemma_enabled_below_all(flags: OccFlags)
    ensures
        enabled_below(flags, 8) == enabled_probe_count(flags),
{
    reveal_with_fuel(enabled_below, 9);
    assert(enabled_protocol_count(flags) * enabled_family_count(flags) * enabled_scope_count(
        flags,
    ) == enabled_below(flags, 8)) by (nonlinear_arith)
        requires
            enabled_below(flags, 8) == (if probe_enabled(flags, probe_of_rank(0)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(1)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(2)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(3)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(4)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(5)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(6)) {
                1int
            } else {
                0int
            }) + (if probe_enabled(flags, probe_of_rank(7)) {
                1int
            } else {
                0int
            }),
    ;
}

/// The probe at matrix position `rank`.
fn probe_at_rank(rank: u8) -> (probe: Probe)
    requires
        rank < 8,
    ensures
        probe == probe_of_rank(rank as int),
        matrix_rank(probe) == rank,
{
    Probe {
        protocol: if rank % 2 == 0 {
            Protocol::Tcp
        } else {
            Protocol::Udp
        },
        family: if rank < 4 {
            Family::Ipv4
        } else {
            Family::Ipv6
        },
        scope: if rank % 4 < 2 {
            Scope::Localhost
        } else {
            Scope::Wildcard
        },
    }
}

/// Whether the policy runs `probe`.
fn is_probe_enabled(flags: OccFlags, probe: Probe) -> (enabled: bool)
    ensures
        enabled == probe_enabled(flags, probe),
{
    let protocol = match probe.protocol {
        Protocol::Tcp => !flags.skip_tcp,
        Protocol::Udp => !flags.skip_udp,
    };
    let family = match probe.family {
        Family::Ipv4 => !flags.skip_ipv4,
        Family::Ipv6 => !flags.skip_ipv6,
    };
    let scope = match probe.scope {
        Scope::Localhost => true,
        Scope::Wildcard => flags.check_all_interfaces,
    };
    protocol && family && scope
}

/// The probes the policy runs, in documented matrix order: every enabled
/// probe exactly once, nothing else, sorted by (family, scope, protocol).
pub fn probe_matrix(flags: OccFlags) -> (probes: Vec<Probe>)
    ensures
        is_probe_matrix(flags, probes@),
        probes.len() == enabled_probe_count(flags),
        probes.len() <= 8,
{
    let mut probes: Vec<Probe> = Vec::with_capacity(8);
    let mut rank: u8 = 0;
    while rank < 8
        invariant
            rank <= 8,
            forall|i: int|
                0 <= i < probes.len() ==> probe_enabled(flags, #[trigger] probes@[i])
                    && matrix_rank(probes@[i]) < rank,
            in_matrix_order(probes@),
            forall|probe: Probe|
                #[trigger] probe_enabled(flags, probe) && matrix_rank(probe) < rank
                    ==> probes@.contains(probe),
            probes.len() == enabled_below(flags, rank as int),
        decreases 8 - rank,
    {
        let probe = probe_at_rank(rank);
        if is_probe_enabled(flags, probe) {
            probes.push(probe);
        }
        proof {
            assert forall|other: Probe|
                #[trigger] probe_enabled(flags, other) && matrix_rank(other) < rank + 1
                    implies probes@.contains(other) by {
                if matrix_rank(other) == rank {
                    lemma_matrix_rank_injective(other, probe);
                    assert(probes@[probes.len() - 1] == other);
                }
            }
        }
        rank += 1;
    }
    proof {
        lemma_enabled_below_all(flags);
        assert forall|probe: Probe|
            #[trigger] probes@.contains(probe) <==> probe_enabled(flags, probe) by {
            lemma_matrix_rank(probe);
        }
        lemma_count_at_most_eight(flags);
    }
    probes
}

/// At most eight probes are ever enabled.
proof fn lemma_count_at_most_eight(flags: OccFlags)
    ensures
        0 <= enabled_probe_count(flags) <= 8,
{
    let p = enabled_protocol_count(flags);
    let f = enabled_family_count(flags);
    let s = enabled_scope_count(flags);
    assert(0 <= p * f * s <= 8) by (nonlinear_arith)
        requires
            0 <= p <= 2,
            0 <= f <= 2,
            1 <= s <= 2,
    ;
}

/// Whether the policy disables occupancy checking entirely (its matrix is
/// empty). This is the predicate the mock checker uses, so the mock and the
/// system checker agree on when checking is off.
pub fn occupancy_disabled(flags: OccFlags) -> (disabled: bool)
    ensures
        disabled == no_probe_enabled(flags),
        disabled == (enabled_probe_count(flags) == 0),
{
    let disabled = (flags.skip_tcp && flags.skip_udp) || (flags.skip_ipv4 && flags.skip_ipv6);
    proof {
        if !disabled {
            let witness = Probe {
                protocol: if flags.skip_tcp {
                    Protocol::Udp
                } else {
                    Protocol::Tcp
                },
                family: if flags.skip_ipv4 {
                    Family::Ipv6
                } else {
                    Family::Ipv4
                },
                scope: Scope::Localhost,
            };
            assert(probe_enabled(flags, witness));
        }
        let p = enabled_protocol_count(flags);
        let f = enabled_family_count(flags);
        let s = enabled_scope_count(flags);
        assert((p * f * s == 0) == (p == 0 || f == 0)) by (nonlinear_arith)
            requires
                0 <= p <= 2,
                0 <= f <= 2,
                1 <= s <= 2,
        ;
    }
    disabled
}

// ---------------------------------------------------------------------------
// Properties of the matrix specification
// ---------------------------------------------------------------------------

/// In a sequence in matrix order, an element is either the head or lies in
/// the tail and ranks strictly above the head.
proof fn lemma_head_or_tail(s: Seq<Probe>, probe: Probe)
    requires
        in_matrix_order(s),
        s.contains(probe),
    ensures
        probe == s[0] || (s.drop_first().contains(probe) && matrix_rank(s[0]) < matrix_rank(
            probe,
        )),
{
    let i = choose|i: int| 0 <= i < s.len() && s[i] == probe;
    if i > 0 {
        assert(s.drop_first()[i - 1] == probe);
        assert(matrix_rank(s[0]) < matrix_rank(s[i]));
    }
}

/// Every element of the tail of a sequence in matrix order is an element of
/// the sequence ranked strictly above its head.
proof fn lemma_tail_element(s: Seq<Probe>, probe: Probe)
    requires
        in_matrix_order(s),
        s.len() > 0,
        s.drop_first().contains(probe),
    ensures
        s.contains(probe),
        matrix_rank(s[0]) < matrix_rank(probe),
{
    let i = choose|i: int| 0 <= i < s.len() - 1 && s.drop_first()[i] == probe;
    assert(s[i + 1] == probe);
    assert(matrix_rank(s[0]) < matrix_rank(s[i + 1]));
}

/// Two sequences in matrix order with the same elements are equal.
proof fn lemma_sorted_unique(a: Seq<Probe>, b: Seq<Probe>)
    requires
        in_matrix_order(a),
        in_matrix_order(b),
        forall|probe: Probe| #[trigger] a.contains(probe) <==> b.contains(probe),
    ensures
        a == b,
    decreases a.len(),
{
    if a.len() == 0 || b.len() == 0 {
        if a.len() > 0 {
            assert(a.contains(a[0]));
            assert(b.contains(a[0]));
        }
        if b.len() > 0 {
            assert(b.contains(b[0]));
            assert(a.contains(b[0]));
        }
        assert(a =~= b);
    } else {
        // The heads agree: each head is in the other sequence, so neither can
        // rank strictly above the other.
        assert(a.contains(a[0]));
        assert(b.contains(b[0]));
        lemma_head_or_tail(b, a[0]);
        lemma_head_or_tail(a, b[0]);
        assert(a[0] == b[0]);
        // The tails are in matrix order and have the same elements.
        let ta = a.drop_first();
        let tb = b.drop_first();
        assert forall|probe: Probe| #[trigger] ta.contains(probe) implies tb.contains(probe) by {
            lemma_tail_element(a, probe);
            lemma_head_or_tail(b, probe);
        }
        assert forall|probe: Probe| #[trigger] tb.contains(probe) implies ta.contains(probe) by {
            lemma_tail_element(b, probe);
            lemma_head_or_tail(a, probe);
        }
        assert forall|i: int, j: int| 0 <= i < j < ta.len() implies #[trigger] matrix_rank(ta[i])
            < #[trigger] matrix_rank(ta[j]) by {
            assert(matrix_rank(a[i + 1]) < matrix_rank(a[j + 1]));
        }
        assert forall|i: int, j: int| 0 <= i < j < tb.len() implies #[trigger] matrix_rank(tb[i])
            < #[trigger] matrix_rank(tb[j]) by {
            assert(matrix_rank(b[i + 1]) < matrix_rank(b[j + 1]));
        }
        lemma_sorted_unique(ta, tb);
        assert forall|k: int| 0 <= k < a.len() implies a[k] == b[k] by {
            if k > 0 {
                assert(a[k] == ta[k - 1]);
                assert(b[k] == tb[k - 1]);
            }
        }
        assert(a =~= b);
    }
}

/// The matrix specification pins down a unique sequence: two sequences that
/// both satisfy `is_probe_matrix` for the same flags are equal.
pub proof fn lemma_probe_matrix_unique(flags: OccFlags, a: Seq<Probe>, b: Seq<Probe>)
    requires
        is_probe_matrix(flags, a),
        is_probe_matrix(flags, b),
    ensures
        a == b,
{
    assert forall|probe: Probe| #[trigger] a.contains(probe) <==> b.contains(probe) by {
        assert(a.contains(probe) <==> probe_enabled(flags, probe));
        assert(b.contains(probe) <==> probe_enabled(flags, probe));
    }
    lemma_sorted_unique(a, b);
}

/// With nothing configured, the policy probes exactly TCP and UDP on IPv4 and
/// IPv6 localhost, in this order: the specification's conservative default.
pub proof fn lemma_default_matrix(probes: Seq<Probe>)
    requires
        is_probe_matrix(default_flags(), probes),
    ensures
        probes == seq![
            Probe { protocol: Protocol::Tcp, family: Family::Ipv4, scope: Scope::Localhost },
            Probe { protocol: Protocol::Udp, family: Family::Ipv4, scope: Scope::Localhost },
            Probe { protocol: Protocol::Tcp, family: Family::Ipv6, scope: Scope::Localhost },
            Probe { protocol: Protocol::Udp, family: Family::Ipv6, scope: Scope::Localhost },
        ],
{
    let expected = seq![
        Probe { protocol: Protocol::Tcp, family: Family::Ipv4, scope: Scope::Localhost },
        Probe { protocol: Protocol::Udp, family: Family::Ipv4, scope: Scope::Localhost },
        Probe { protocol: Protocol::Tcp, family: Family::Ipv6, scope: Scope::Localhost },
        Probe { protocol: Protocol::Udp, family: Family::Ipv6, scope: Scope::Localhost },
    ];
    assert forall|probe: Probe|
        #[trigger] expected.contains(probe) <==> probe_enabled(default_flags(), probe) by {
        if probe_enabled(default_flags(), probe) {
            if probe.family == Family::Ipv4 && probe.protocol == Protocol::Tcp {
                assert(expected[0] == probe);
            } else if probe.family == Family::Ipv4 {
                assert(expected[1] == probe);
            } else if probe.protocol == Protocol::Tcp {
                assert(expected[2] == probe);
            } else {
                assert(expected[3] == probe);
            }
        }
    }
    lemma_probe_matrix_unique(default_flags(), probes, expected);
}

/// Enabling `check_all_interfaces` only adds the wildcard probes: every probe
/// run without it is still run, and the only new probes are wildcard probes.
pub proof fn lemma_all_interfaces_only_adds_wildcards(flags: OccFlags, probe: Probe)
    ensures
        probe_enabled(flags, probe) ==> probe_enabled(
            OccFlags { check_all_interfaces: true, ..flags },
            probe,
        ),
        probe_enabled(OccFlags { check_all_interfaces: true, ..flags }, probe) && !probe_enabled(
            flags,
            probe,
        ) ==> probe.scope == Scope::Wildcard,
{
}

// ---------------------------------------------------------------------------
// Folding probe outcomes into a report
// ---------------------------------------------------------------------------

/// What one bind probe observed, as classified by the caller.
#[derive(Debug)]
pub enum ProbeOutcome<E> {
    /// The address was free (the bind succeeded).
    Free,
    /// The address was in use (the bind failed with `AddrInUse`).
    InUse,
    /// The probe itself failed with `E`, so occupancy is unknown.
    Failed(E),
}

/// The result of running every probe of the matrix without a failure.
#[derive(Debug)]
pub struct MatrixEvidence {
    /// Number of probes the policy enabled (the matrix length).
    pub enabled: usize,
    /// The probes that found the address in use, in matrix order.
    pub occupied: Vec<Probe>,
}

/// Every enabled probe ranked below `bound` was run and completed: the oracle
/// returned `seen(probe)` for it, and that outcome is not a failure.
pub open spec fn completed_below<E, F: Fn(Probe) -> ProbeOutcome<E>>(
    flags: OccFlags,
    run: F,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    bound: int,
) -> bool {
    forall|probe: Probe|
        #[trigger] probe_enabled(flags, probe) && matrix_rank(probe) < bound ==> run.ensures(
            (probe,),
            seen(probe),
        ) && !(seen(probe) is Failed)
}

/// `occupied` is exactly the enabled probes whose outcome in `seen` is
/// `InUse`, in matrix order.
pub open spec fn is_occupied_evidence<E>(
    flags: OccFlags,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    occupied: Seq<Probe>,
) -> bool {
    &&& forall|probe: Probe| #[trigger]
        occupied.contains(probe) <==> probe_enabled(flags, probe) && seen(probe) is InUse
    &&& in_matrix_order(occupied)
}

/// `seen` with the outcome of `probe` replaced by `outcome`.
pub open spec fn seen_with<E>(
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    probe: Probe,
    outcome: ProbeOutcome<E>,
) -> spec_fn(Probe) -> ProbeOutcome<E> {
    |other: Probe|
        if other == probe {
            outcome
        } else {
            seen(other)
        }
}

/// Loop state of [`run_probe_matrix`] after running `matrix[0..done]`: those
/// probes completed with the outcomes in `seen`, and `occupied` holds the
/// ones that were in use, in matrix order, ranked below every probe not yet
/// run.
spec fn fold_state<E, F: Fn(Probe) -> ProbeOutcome<E>>(
    run: F,
    matrix: Seq<Probe>,
    done: int,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    occupied: Seq<Probe>,
) -> bool {
    &&& 0 <= done <= matrix.len()
    &&& forall|k: int|
        0 <= k < done ==> run.ensures((#[trigger] matrix[k],), seen(matrix[k])) && !(seen(
            matrix[k],
        ) is Failed)
    &&& forall|probe: Probe| #[trigger]
        occupied.contains(probe) <==> (exists|k: int| 0 <= k < done && matrix[k] == probe) && seen(
            probe,
        ) is InUse
    &&& in_matrix_order(occupied)
    &&& forall|a: int, k: int|
        0 <= a < occupied.len() && done <= k < matrix.len() ==> matrix_rank(#[trigger] occupied[a])
            < matrix_rank(#[trigger] matrix[k])
}

/// Running `matrix[done]` with a non-failure outcome extends the loop state.
proof fn lemma_fold_step<E, F: Fn(Probe) -> ProbeOutcome<E>>(
    flags: OccFlags,
    run: F,
    matrix: Seq<Probe>,
    done: int,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    occupied: Seq<Probe>,
    outcome: ProbeOutcome<E>,
)
    requires
        is_probe_matrix(flags, matrix),
        fold_state(run, matrix, done, seen, occupied),
        done < matrix.len(),
        run.ensures((matrix[done],), outcome),
        !(outcome is Failed),
    ensures
        fold_state(
            run,
            matrix,
            done + 1,
            seen_with(seen, matrix[done], outcome),
            if outcome is InUse {
                occupied.push(matrix[done])
            } else {
                occupied
            },
        ),
{
    let probe = matrix[done];
    let next_seen = seen_with(seen, probe, outcome);
    let next_occupied = if outcome is InUse {
        occupied.push(probe)
    } else {
        occupied
    };
    // Earlier probes rank strictly below `probe`, so they are distinct from it.
    assert forall|k: int| 0 <= k < done implies #[trigger] matrix[k] != probe by {
        assert(matrix_rank(matrix[k]) < matrix_rank(matrix[done]));
    }
    assert forall|k: int| 0 <= k < done + 1 implies run.ensures(
        (#[trigger] matrix[k],),
        next_seen(matrix[k]),
    ) && !(next_seen(matrix[k]) is Failed) by {}
    assert forall|other: Probe| #[trigger]
        next_occupied.contains(other) <==> (exists|k: int| 0 <= k < done + 1 && matrix[k] == other)
            && next_seen(other) is InUse by {
        if other == probe {
            if occupied.contains(probe) {
                let k = choose|k: int| 0 <= k < done && matrix[k] == probe;
                assert(matrix[k] != probe);
            }
            assert(matrix[done] == other);
        } else if exists|k: int| 0 <= k < done + 1 && matrix[k] == other {
            let k = choose|k: int| 0 <= k < done + 1 && matrix[k] == other;
            assert(k < done);
        }
    }
}

/// When `matrix[done]` fails, every enabled probe ranked before it has
/// already completed.
proof fn lemma_fold_failure<E, F: Fn(Probe) -> ProbeOutcome<E>>(
    flags: OccFlags,
    run: F,
    matrix: Seq<Probe>,
    done: int,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    occupied: Seq<Probe>,
)
    requires
        is_probe_matrix(flags, matrix),
        fold_state(run, matrix, done, seen, occupied),
        done < matrix.len(),
    ensures
        probe_enabled(flags, matrix[done]),
        completed_below(flags, run, seen, matrix_rank(matrix[done])),
{
    assert(matrix.contains(matrix[done]));
    assert forall|other: Probe|
        #[trigger] probe_enabled(flags, other) && matrix_rank(other) < matrix_rank(
            matrix[done],
        ) implies run.ensures((other,), seen(other)) && !(seen(other) is Failed) by {
        assert(matrix.contains(other));
        let k = choose|k: int| 0 <= k < matrix.len() && matrix[k] == other;
        if k > done {
            assert(matrix_rank(matrix[done]) < matrix_rank(matrix[k]));
        }
    }
}

/// Once every probe of the matrix has run, every enabled probe completed and
/// `occupied` is exactly the in-use evidence.
proof fn lemma_fold_done<E, F: Fn(Probe) -> ProbeOutcome<E>>(
    flags: OccFlags,
    run: F,
    matrix: Seq<Probe>,
    seen: spec_fn(Probe) -> ProbeOutcome<E>,
    occupied: Seq<Probe>,
)
    requires
        is_probe_matrix(flags, matrix),
        fold_state(run, matrix, matrix.len() as int, seen, occupied),
    ensures
        completed_below(flags, run, seen, 8),
        is_occupied_evidence(flags, seen, occupied),
{
    assert forall|probe: Probe|
        #[trigger] probe_enabled(flags, probe) && matrix_rank(probe) < 8 implies run.ensures(
        (probe,),
        seen(probe),
    ) && !(seen(probe) is Failed) by {
        assert(matrix.contains(probe));
    }
    assert forall|probe: Probe| #[trigger]
        occupied.contains(probe) <==> probe_enabled(flags, probe) && seen(probe) is InUse by {
        if probe_enabled(flags, probe) {
            assert(matrix.contains(probe));
        }
        if exists|k: int| 0 <= k < matrix.len() && matrix[k] == probe {
            assert(matrix.contains(probe));
        }
    }
}

/// Runs the policy's probes in matrix order, failing closed.
///
/// On success, every enabled probe completed and the evidence lists exactly
/// the probes that found the address in use, in matrix order. On the first
/// probe failure the fold stops and returns that probe with its error; every
/// enabled probe before it in matrix order completed. `Free` and `InUse`
/// outcomes never produce an error.
pub fn run_probe_matrix<E, F: Fn(Probe) -> ProbeOutcome<E>>(flags: OccFlags, run: &F) -> (result:
    Result<MatrixEvidence, (Probe, E)>)
    requires
        forall|probe: Probe| run.requires((probe,)),
    ensures
        result matches Ok(evidence) ==> {
            &&& evidence.enabled == enabled_probe_count(flags)
            &&& exists|seen: spec_fn(Probe) -> ProbeOutcome<E>|
                #[trigger] completed_below(flags, *run, seen, 8) && is_occupied_evidence(
                    flags,
                    seen,
                    evidence.occupied@,
                )
        },
        result matches Err((failed, error)) ==> {
            &&& probe_enabled(flags, failed)
            &&& run.ensures((failed,), ProbeOutcome::Failed(error))
            &&& exists|seen: spec_fn(Probe) -> ProbeOutcome<E>|
                #[trigger] completed_below(flags, *run, seen, matrix_rank(failed))
        },
{
    let matrix = probe_matrix(flags);
    let mut occupied: Vec<Probe> = Vec::new();
    let ghost mut seen: spec_fn(Probe) -> ProbeOutcome<E> = |probe: Probe| ProbeOutcome::Free;
    let mut i: usize = 0;
    while i < matrix.len()
        invariant
            is_probe_matrix(flags, matrix@),
            matrix.len() == enabled_probe_count(flags),
            forall|probe: Probe| run.requires((probe,)),
            fold_state(*run, matrix@, i as int, seen, occupied@),
        decreases matrix.len() - i,
    {
        let probe = matrix[i];
        let outcome = run(probe);
        let ghost observed = outcome;
        proof {
            if !(observed is Failed) {
                lemma_fold_step(flags, *run, matrix@, i as int, seen, occupied@, observed);
            } else {
                lemma_fold_failure(flags, *run, matrix@, i as int, seen, occupied@);
            }
        }
        match outcome {
            ProbeOutcome::Failed(error) => return Err((probe, error)),
            ProbeOutcome::InUse => occupied.push(probe),
            ProbeOutcome::Free => {},
        }
        proof {
            seen = seen_with(seen, probe, observed);
        }
        i += 1;
    }
    proof {
        lemma_fold_done(flags, *run, matrix@, seen, occupied@);
    }
    Ok(MatrixEvidence { enabled: matrix.len(), occupied })
}

// ---------------------------------------------------------------------------
// Scanning a port range
// ---------------------------------------------------------------------------

/// The values that `seen` keeps for ports `lo..end`, in ascending port order
/// (ports whose result is `None` contribute nothing).
pub open spec fn kept_values<T>(seen: spec_fn(u16) -> Option<T>, lo: int, end: int) -> Seq<T>
    decreases end - lo,
{
    if end <= lo {
        Seq::empty()
    } else {
        let earlier = kept_values(seen, lo, end - 1);
        match seen((end - 1) as u16) {
            Some(value) => earlier.push(value),
            None => earlier,
        }
    }
}

/// Every port in `lo..end` was inspected without error, with result
/// `seen(port)`.
pub open spec fn inspected_without_error<T, E, F: Fn(u16) -> Result<Option<T>, E>>(
    inspect: F,
    seen: spec_fn(u16) -> Option<T>,
    lo: int,
    end: int,
) -> bool {
    forall|port: u16|
        lo <= port < end ==> #[trigger] inspect.ensures((port,), Ok::<Option<T>, E>(seen(port)))
}

/// The scan of `lo..=hi` stopped at `failed` with `error`: the inspection of
/// `failed` returned `error`, and every earlier port was inspected without
/// error.
pub open spec fn scan_failed_at<T, E, F: Fn(u16) -> Result<Option<T>, E>>(
    inspect: F,
    lo: int,
    hi: int,
    failed: u16,
    error: E,
) -> bool {
    &&& lo <= failed <= hi
    &&& inspect.ensures((failed,), Err::<Option<T>, E>(error))
    &&& exists|seen: spec_fn(u16) -> Option<T>|
        #[trigger] inspected_without_error(inspect, seen, lo, failed as int)
}

/// The scan of `lo..=hi` stopped with `error` at some port.
pub open spec fn scan_stopped<T, E, F: Fn(u16) -> Result<Option<T>, E>>(
    inspect: F,
    lo: int,
    hi: int,
    error: E,
) -> bool {
    exists|failed: u16| #[trigger] scan_failed_at::<T, E, F>(inspect, lo, hi, failed, error)
}

/// The scan of `lo..=hi` completed: every port was inspected without error
/// and `kept` is exactly the kept values, in ascending port order.
pub open spec fn scan_completed<T, E, F: Fn(u16) -> Result<Option<T>, E>>(
    inspect: F,
    lo: int,
    hi: int,
    kept: Seq<T>,
) -> bool {
    exists|seen: spec_fn(u16) -> Option<T>|
        #[trigger] inspected_without_error(inspect, seen, lo, hi + 1) && kept == kept_values(
            seen,
            lo,
            hi + 1,
        )
}

/// `seen` with the result for `port` replaced by `value`.
pub open spec fn seen_port_with<T>(
    seen: spec_fn(u16) -> Option<T>,
    port: u16,
    value: Option<T>,
) -> spec_fn(u16) -> Option<T> {
    |other: u16|
        if other == port {
            value
        } else {
            seen(other)
        }
}

/// `kept_values` only depends on the results for ports in `lo..end`.
proof fn lemma_kept_values_local<T>(
    a: spec_fn(u16) -> Option<T>,
    b: spec_fn(u16) -> Option<T>,
    lo: int,
    end: int,
)
    requires
        forall|port: u16| lo <= port < end ==> #[trigger] a(port) == b(port),
        0 <= lo,
        end <= 65536,
    ensures
        kept_values(a, lo, end) == kept_values(b, lo, end),
    decreases end - lo,
{
    if end > lo {
        lemma_kept_values_local(a, b, lo, end - 1);
        assert(a((end - 1) as u16) == b((end - 1) as u16));
    }
}

/// Inspects the ports `lo..=hi` in ascending order and collects the values
/// the inspection keeps, failing fast.
///
/// On success, every port of the range was inspected without error and the
/// result is exactly the kept values in ascending port order. On the first
/// error the scan stops and returns it; every port before the failing one was
/// inspected without error. An empty range (`hi < lo`) inspects nothing.
pub fn scan_ports<T, E, F: Fn(u16) -> Result<Option<T>, E>>(lo: u16, hi: u16, inspect: &F) -> (result:
    Result<Vec<T>, E>)
    requires
        forall|port: u16| lo <= port <= hi ==> inspect.requires((port,)),
    ensures
        result matches Ok(kept) ==> scan_completed(*inspect, lo as int, hi as int, kept@),
        result matches Err(error) ==> scan_stopped::<T, E, F>(*inspect, lo as int, hi as int, error),
{
    let mut kept: Vec<T> = Vec::new();
    let ghost mut seen: spec_fn(u16) -> Option<T> = |port: u16| None;
    if hi < lo {
        assert(inspected_without_error(*inspect, seen, lo as int, hi + 1));
        assert(scan_completed(*inspect, lo as int, hi as int, kept@));
        return Ok(kept);
    }
    let mut port = lo;
    loop
        invariant
            lo <= port <= hi,
            forall|p: u16| lo <= p <= hi ==> inspect.requires((p,)),
            inspected_without_error(*inspect, seen, lo as int, port as int),
            kept@ == kept_values(seen, lo as int, port as int),
        decreases hi - port,
    {
        let inspection = inspect(port);
        let ghost observed = inspection;
        let ghost previous = seen;
        match inspection {
            Err(error) => {
                assert(inspected_without_error(*inspect, seen, lo as int, port as int));
                assert(scan_failed_at::<T, E, F>(*inspect, lo as int, hi as int, port, error));
                assert(scan_stopped::<T, E, F>(*inspect, lo as int, hi as int, error));
                return Err(error);
            },
            Ok(Some(item)) => kept.push(item),
            Ok(None) => {},
        }
        proof {
            let value = match observed {
                Ok(value) => value,
                Err(_) => None,
            };
            seen = seen_port_with(previous, port, value);
            lemma_kept_values_local(previous, seen, lo as int, port as int);
            assert(seen(port) == value);
            assert(kept@ == kept_values(seen, lo as int, port + 1));
            assert forall|p: u16| lo <= p < port + 1 implies #[trigger] inspect.ensures(
                (p,),
                Ok::<Option<T>, E>(seen(p)),
            ) by {
                if p < port {
                    assert(inspect.ensures((p,), Ok::<Option<T>, E>(previous(p))));
                }
            }
        }
        if port == hi {
            assert(inspected_without_error(*inspect, seen, lo as int, hi + 1));
            assert(scan_completed(*inspect, lo as int, hi as int, kept@));
            return Ok(kept);
        }
        port += 1;
    }
}

// ---------------------------------------------------------------------------
// Owner metadata
// ---------------------------------------------------------------------------

/// Completeness of the best-effort owner metadata of an occupied probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerStatus {
    /// Process ID, process name, and user are all known.
    Available,
    /// Some, but not all, of the owner fields are known.
    Partial,
    /// None of the owner fields are known.
    Unavailable,
}

/// Classifies which owner fields are known.
pub fn owner_status(has_process_id: bool, has_process_name: bool, has_user: bool) -> (status:
    OwnerStatus)
    ensures
        status is Available <==> (has_process_id && has_process_name && has_user),
        status is Unavailable <==> (!has_process_id && !has_process_name && !has_user),
        status is Partial <==> ((has_process_id || has_process_name || has_user) && !(
        has_process_id && has_process_name && has_user)),
{
    if has_process_id && has_process_name && has_user {
        OwnerStatus::Available
    } else if has_process_id || has_process_name || has_user {
        OwnerStatus::Partial
    } else {
        OwnerStatus::Unavailable
    }
}

} // verus!
