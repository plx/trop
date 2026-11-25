//! Verified kernel: exclusion entries, the canonical excluded-port set, and
//! compaction.
//!
//! Production glue:
//!
//! - `crate::port::exclusions`: `ExclusionManager` (the excluded-port set),
//!   `validate_exclusions`, `compact_exclusions`, `is_compact`, and the
//!   list edits `add_exclusion_to_list` (`add_exclusion`),
//!   `add_ports_to_list` (`add_ports`) and `first_reserved_port`.
//! - `crate::config::validator` (`excluded_ports` validation).
//! - `crate::config::schema` (`PortExclusion` text parsing:
//!   `find_range_separator` and `parse_u16`) and
//!   `crate::config::environment` (`TROP_EXCLUDED_PORTS`, through the same
//!   parser; `parse_u16` also decides `TROP_PORT_MIN`, `TROP_PORT_MAX` and
//!   `TROP_PORT_MAX_OFFSET`).
//! - `crate::cli_support` (`unreserved_ports`, the ports
//!   `scan --autoexclude` may exclude).
//!
//! The CLI reaches these through `trop::port::exclusions` and
//! `PortExclusion`'s `FromStr`: `trop-cli/src/commands/exclude.rs` (its
//! argument, `add_exclusion_to_list`, `first_reserved_port`),
//! `trop-cli/src/commands/compact_exclusions.rs`
//! (`compact_exclusions`, `is_compact`), and `trop-cli/src/commands/scan.rs`
//! (`add_ports_to_list`, `compact_exclusions`, `is_compact`).
//!
//! The decimal parsers decide the kernel's own syntax (`is_unsigned_text`).
//! That this is also the syntax of the unsigned `from_str` implementations
//! is not proved (`vstd` does not specify `from_str`); it is trusted and
//! pinned by differential tests in the glue.

use vstd::prelude::*;

use super::port::PortSpan;

verus! {

// ---------------------------------------------------------------------------
// Exclusion entries
// ---------------------------------------------------------------------------
/// One configured exclusion: a single port, or an inclusive range of ports.
///
/// This mirrors the configuration type `PortExclusion` field for field (the
/// kernel cannot refer to configuration types). Values are unchecked: an
/// entry may be invalid (port 0, or a range whose end precedes its start).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionEntry {
    /// A single port.
    Single(u16),
    /// The ports `start..=end`.
    Range {
        /// First port of the range (inclusive).
        start: u16,
        /// Last port of the range (inclusive).
        end: u16,
    },
}

/// Why an [`ExclusionEntry`] is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusionError {
    /// A single port of 0.
    ZeroPort,
    /// A range starting at port 0.
    ZeroStart,
    /// A range (with a non-zero start) ending at port 0.
    ZeroEnd,
    /// A range of non-zero ports whose end precedes its start.
    Reversed {
        /// The range's start.
        start: u16,
        /// The range's end (`< start`).
        end: u16,
    },
}

/// The lowest port an entry names.
pub open spec fn entry_lo(e: ExclusionEntry) -> int {
    match e {
        ExclusionEntry::Single(p) => p as int,
        ExclusionEntry::Range { start, .. } => start as int,
    }
}

/// The highest port an entry names.
pub open spec fn entry_hi(e: ExclusionEntry) -> int {
    match e {
        ExclusionEntry::Single(p) => p as int,
        ExclusionEntry::Range { end, .. } => end as int,
    }
}

/// Whether the entry excludes `p`. Ranges are inclusive on both ends; a
/// reversed range covers nothing.
pub open spec fn entry_covers(e: ExclusionEntry, p: int) -> bool {
    entry_lo(e) <= p <= entry_hi(e)
}

/// A valid entry: a port `>= 1`, or a range with `1 <= start <= end`
/// (`5000..5000` is valid).
pub open spec fn entry_valid(e: ExclusionEntry) -> bool {
    match e {
        ExclusionEntry::Single(p) => p >= 1,
        ExclusionEntry::Range { start, end } => 1 <= start <= end,
    }
}

/// The error the validator reports for an entry, in precedence order: a zero
/// single port; for ranges a zero start, then a zero end, then a reversed
/// range. `None` for a valid entry.
pub open spec fn entry_error(e: ExclusionEntry) -> Option<ExclusionError> {
    match e {
        ExclusionEntry::Single(p) => if p == 0 {
            Some(ExclusionError::ZeroPort)
        } else {
            None
        },
        ExclusionEntry::Range { start, end } => if start == 0 {
            Some(ExclusionError::ZeroStart)
        } else if end == 0 {
            Some(ExclusionError::ZeroEnd)
        } else if end < start {
            Some(ExclusionError::Reversed { start, end })
        } else {
            None
        },
    }
}

/// Every entry of `es` is valid.
pub open spec fn all_valid(es: Seq<ExclusionEntry>) -> bool {
    forall|i: int| 0 <= i < es.len() ==> entry_valid(#[trigger] es[i])
}

/// Some entry of `es` excludes `p`.
pub open spec fn covers(es: Seq<ExclusionEntry>, p: int) -> bool {
    exists|i: int| 0 <= i < es.len() && #[trigger] entry_covers(es[i], p)
}

/// `a` and `b` exclude exactly the same ports.
pub open spec fn same_coverage(a: Seq<ExclusionEntry>, b: Seq<ExclusionEntry>) -> bool {
    forall|p: int| covers(a, p) == covers(b, p)
}

/// The valid ports (`1..=65535`) that `es` excludes.
pub open spec fn port_set(es: Seq<ExclusionEntry>) -> Set<int> {
    Set::<int>::range(1, 65536).filter(|p: int| covers(es, p))
}

/// The ports `lo..=hi` as a set.
pub open spec fn span_set(lo: int, hi: int) -> Set<int> {
    Set::<int>::range(lo, hi + 1)
}

/// A canonical entry: valid, and a `Single` exactly when it names one port.
pub open spec fn entry_canonical(e: ExclusionEntry) -> bool {
    entry_valid(e) && (e is Single <==> entry_lo(e) == entry_hi(e))
}

/// The canonical (compacted) form of an exclusion list: canonical entries,
/// sorted strictly ascending, and every two entries separated by at least
/// one port that neither excludes (so no overlap and no adjacency).
pub open spec fn canonical(es: Seq<ExclusionEntry>) -> bool {
    &&& forall|i: int| 0 <= i < es.len() ==> entry_canonical(#[trigger] es[i])
    &&& forall|i: int, j: int|
        0 <= i < j < es.len() ==> entry_hi(#[trigger] es[i]) + 1 < entry_lo(#[trigger] es[j])
}

/// `out` is a compaction of `es`: canonical, and excluding exactly the same
/// ports.
pub open spec fn is_compaction(out: Seq<ExclusionEntry>, es: Seq<ExclusionEntry>) -> bool {
    canonical(out) && same_coverage(out, es)
}

// ---------------------------------------------------------------------------
// Entry validation
// ---------------------------------------------------------------------------
/// Validity means "names a non-empty set of ports, all of them valid".
pub proof fn lemma_entry_valid_meaning(e: ExclusionEntry)
    ensures
        entry_valid(e) <==> ((exists|p: int| entry_covers(e, p)) && (forall|p: int|
            #[trigger] entry_covers(e, p) ==> super::port::is_valid_port(p))),
        entry_valid(e) ==> span_set(entry_lo(e), entry_hi(e)).len() >= 1,
        entry_valid(e) <==> entry_error(e) is None,
{
    if entry_valid(e) {
        assert(entry_covers(e, entry_lo(e)));
        assert(span_set(entry_lo(e), entry_hi(e)).contains(entry_lo(e)));
    } else {
        if exists|p: int| entry_covers(e, p) {
            let p = choose|p: int| entry_covers(e, p);
            // An invalid entry that covers anything covers port 0.
            assert(entry_covers(e, 0));
            assert(!super::port::is_valid_port(0));
        }
    }
}

/// A one-port range `p..p` (for example `5000..5000`) is valid, and its
/// canonical form is the single port `p`: by `compact_exclusions`'
/// uniqueness guarantee, that is what compaction returns for it.
pub proof fn lemma_one_port_range(p: u16)
    requires
        p >= 1,
    ensures
        entry_valid(ExclusionEntry::Range { start: p, end: p }),
        is_compaction(
            seq![ExclusionEntry::Single(p)],
            seq![ExclusionEntry::Range { start: p, end: p }],
        ),
{
    let single = seq![ExclusionEntry::Single(p)];
    let range = seq![ExclusionEntry::Range { start: p, end: p }];
    assert forall|q: int| covers(single, q) == covers(range, q) by {
        if covers(single, q) {
            assert(entry_covers(range[0], q));
        }
        if covers(range, q) {
            assert(entry_covers(single[0], q));
        }
    }
}

/// Validates one entry.
///
/// Errors follow the configuration validator's precedence: for ranges a zero
/// start is reported before a zero end, which is reported before a reversed
/// range.
pub fn validate_entry(e: ExclusionEntry) -> (r: Result<(), ExclusionError>)
    ensures
        r is Ok <==> entry_valid(e),
        r is Ok <==> entry_error(e) is None,
        r matches Err(err) ==> entry_error(e) == Some(err),
{
    match e {
        ExclusionEntry::Single(p) => {
            if p == 0 {
                Err(ExclusionError::ZeroPort)
            } else {
                Ok(())
            }
        },
        ExclusionEntry::Range { start, end } => {
            if start == 0 {
                Err(ExclusionError::ZeroStart)
            } else if end == 0 {
                Err(ExclusionError::ZeroEnd)
            } else if end < start {
                Err(ExclusionError::Reversed { start, end })
            } else {
                Ok(())
            }
        },
    }
}

/// Validates a list, reporting the first invalid entry.
pub fn validate_entries(es: &[ExclusionEntry]) -> (r: Result<(), (usize, ExclusionError)>)
    ensures
        r is Ok <==> all_valid(es@),
        r matches Err((i, err)) ==> first_invalid(es@, i as int, err),
{
    let mut i: usize = 0;
    while i < es.len()
        invariant
            i <= es@.len(),
            all_valid(es@.take(i as int)),
        decreases es@.len() - i,
    {
        if let Err(err) = validate_entry(es[i]) {
            return Err((i, err));
        }
        assert(es@.take(i as int + 1) =~= es@.take(i as int).push(es@[i as int]));
        i += 1;
    }
    assert(es@.take(i as int) =~= es@);
    Ok(())
}

// ---------------------------------------------------------------------------
// Coverage lemmas
// ---------------------------------------------------------------------------
/// Coverage of a concatenation.
pub proof fn lemma_covers_add(a: Seq<ExclusionEntry>, b: Seq<ExclusionEntry>, p: int)
    ensures
        covers(a + b, p) <==> covers(a, p) || covers(b, p),
{
    if covers(a + b, p) {
        let i = choose|i: int| 0 <= i < (a + b).len() && #[trigger] entry_covers((a + b)[i], p);
        if i < a.len() {
            assert(entry_covers(a[i], p));
        } else {
            assert(entry_covers(b[i - a.len()], p));
        }
    }
    if covers(a, p) {
        let i = choose|i: int| 0 <= i < a.len() && #[trigger] entry_covers(a[i], p);
        assert(entry_covers((a + b)[i], p));
    }
    if covers(b, p) {
        let i = choose|i: int| 0 <= i < b.len() && #[trigger] entry_covers(b[i], p);
        assert(entry_covers((a + b)[a.len() + i], p));
    }
}

/// Coverage after appending one entry.
pub proof fn lemma_covers_push(a: Seq<ExclusionEntry>, e: ExclusionEntry, p: int)
    ensures
        covers(a.push(e), p) <==> covers(a, p) || entry_covers(e, p),
{
    lemma_covers_add(a, seq![e], p);
    assert(a.push(e) =~= a + seq![e]);
    if entry_covers(e, p) {
        assert(entry_covers(seq![e][0], p));
    }
}

/// A valid list excludes only valid ports.
pub proof fn lemma_valid_covers_valid_ports(es: Seq<ExclusionEntry>, p: int)
    requires
        all_valid(es),
        covers(es, p),
    ensures
        super::port::is_valid_port(p),
{
    let i = choose|i: int| 0 <= i < es.len() && #[trigger] entry_covers(es[i], p);
    assert(entry_valid(es[i]));
}

/// For a valid list, `port_set` is exactly the covered ports.
pub proof fn lemma_port_set_valid(es: Seq<ExclusionEntry>)
    requires
        all_valid(es),
    ensures
        forall|p: int| #[trigger] port_set(es).contains(p) <==> covers(es, p),
{
    assert forall|p: int| #[trigger] port_set(es).contains(p) <==> covers(es, p) by {
        if covers(es, p) {
            lemma_valid_covers_valid_ports(es, p);
        }
    }
}

/// Canonical lists are valid.
pub proof fn lemma_canonical_valid(es: Seq<ExclusionEntry>)
    requires
        canonical(es),
    ensures
        all_valid(es),
{
    assert forall|i: int| 0 <= i < es.len() implies entry_valid(#[trigger] es[i]) by {
        assert(entry_canonical(es[i]));
    }
}

/// Prefixes, suffixes, and slices of canonical lists are canonical.
pub proof fn lemma_canonical_subrange(es: Seq<ExclusionEntry>, lo: int, hi: int)
    requires
        canonical(es),
        0 <= lo <= hi <= es.len(),
    ensures
        canonical(es.subrange(lo, hi)),
{
    let s = es.subrange(lo, hi);
    assert forall|i: int| 0 <= i < s.len() implies entry_canonical(#[trigger] s[i]) by {
        assert(s[i] == es[lo + i]);
    }
    assert forall|i: int, j: int| 0 <= i < j < s.len() implies entry_hi(#[trigger] s[i]) + 1
        < entry_lo(#[trigger] s[j]) by {
        assert(s[i] == es[lo + i]);
        assert(s[j] == es[lo + j]);
    }
}

/// Gluing canonical lists around a separating entry keeps them canonical.
pub proof fn lemma_canonical_join(a: Seq<ExclusionEntry>, e: ExclusionEntry, b: Seq<ExclusionEntry>)
    requires
        canonical(a),
        canonical(b),
        entry_canonical(e),
        a.len() > 0 ==> entry_hi(a.last()) + 1 < entry_lo(e),
        b.len() > 0 ==> entry_hi(e) + 1 < entry_lo(b[0]),
    ensures
        canonical(a.push(e) + b),
{
    let s = a.push(e) + b;
    assert forall|i: int| 0 <= i < s.len() implies entry_canonical(#[trigger] s[i]) by {
        if i < a.len() {
            assert(s[i] == a[i]);
        } else if i > a.len() {
            assert(s[i] == b[i - a.len() - 1]);
        }
    }
    assert forall|i: int, j: int| 0 <= i < j < s.len() implies entry_hi(#[trigger] s[i]) + 1
        < entry_lo(#[trigger] s[j]) by {
        // Every entry of `a` ends before `e` starts; every entry of `b`
        // starts after `e` ends.
        if i < a.len() {
            assert(s[i] == a[i]);
            if i < a.len() - 1 {
                assert(entry_hi(a[i]) + 1 < entry_lo(a[a.len() - 1]));
            }
            assert(entry_hi(a[i]) + 1 < entry_lo(e));
        }
        if j > a.len() {
            assert(s[j] == b[j - a.len() - 1]);
            if j - a.len() - 1 > 0 {
                assert(entry_hi(b[0]) + 1 < entry_lo(b[j - a.len() - 1]));
            }
            assert(entry_hi(e) + 1 < entry_lo(b[j - a.len() - 1]));
        }
        if i < a.len() && j < a.len() {
            assert(s[j] == a[j]);
        }
        if i > a.len() && j > a.len() {
            assert(s[i] == b[i - a.len() - 1]);
        }
    }
}

// ---------------------------------------------------------------------------
// Exec helpers
// ---------------------------------------------------------------------------
/// The bounds `(lo, hi)` of an entry.
const fn entry_bounds(e: ExclusionEntry) -> (r: (u16, u16))
    ensures
        r.0 == entry_lo(e),
        r.1 == entry_hi(e),
{
    match e {
        ExclusionEntry::Single(p) => (p, p),
        ExclusionEntry::Range { start, end } => (start, end),
    }
}

/// The canonical entry for the ports `lo..=hi`.
const fn run_entry(lo: u16, hi: u16) -> (e: ExclusionEntry)
    requires
        1 <= lo <= hi,
    ensures
        entry_canonical(e),
        entry_lo(e) == lo,
        entry_hi(e) == hi,
{
    if lo == hi {
        ExclusionEntry::Single(lo)
    } else {
        ExclusionEntry::Range { start: lo, end: hi }
    }
}

/// Copies `src[from..to]` onto the end of `out`.
fn extend_from(out: &mut Vec<ExclusionEntry>, src: &[ExclusionEntry], from: usize, to: usize)
    requires
        from <= to <= src@.len(),
    ensures
        final(out)@ == old(out)@ + src@.subrange(from as int, to as int),
{
    let mut k = from;
    while k < to
        invariant
            from <= k <= to <= src@.len(),
            out@ == old(out)@ + src@.subrange(from as int, k as int),
        decreases to - k,
    {
        out.push(src[k]);
        k += 1;
        assert(out@ =~= old(out)@ + src@.subrange(from as int, k as int));
    }
}

/// Inserts the ports `lo..=hi` into the canonical list `runs`, merging every
/// run that overlaps or touches them.
fn insert_run(runs: &[ExclusionEntry], lo: u16, hi: u16) -> (out: Vec<ExclusionEntry>)
    requires
        canonical(runs@),
        1 <= lo <= hi,
    ensures
        canonical(out@),
        forall|p: int| covers(out@, p) <==> (covers(runs@, p) || lo <= p <= hi),
{
    let n = runs.len();
    // Phase 1: skip the runs that end before `lo - 1` (left of the new
    // ports, and not adjacent to them).
    let mut k: usize = 0;
    while k < n && u32::from(entry_bounds(runs[k]).1) + 1 < u32::from(lo)
        invariant
            k <= n == runs@.len(),
            canonical(runs@),
            forall|j: int| 0 <= j < k ==> entry_hi(#[trigger] runs@[j]) + 1 < lo,
        decreases n - k,
    {
        k += 1;
    }
    let left = k;
    // Phase 2: absorb the runs that overlap or touch `cur_lo..=cur_hi`.
    let mut cur_lo = lo;
    let mut cur_hi = hi;
    while k < n && u32::from(entry_bounds(runs[k]).0) <= u32::from(cur_hi) + 1
        invariant
            left <= k <= n == runs@.len(),
            canonical(runs@),
            1 <= cur_lo <= lo <= hi <= cur_hi,
            forall|j: int| 0 <= j < left ==> entry_hi(#[trigger] runs@[j]) + 1 < cur_lo,
            left < n ==> entry_hi(runs@[left as int]) + 1 >= lo,
            forall|p: int|
                cur_lo <= p <= cur_hi <==> (lo <= p <= hi || #[trigger] covers(
                    runs@.subrange(left as int, k as int),
                    p,
                )),
        decreases n - k,
    {
        let (a, b) = entry_bounds(runs[k]);
        proof {
            // The run starts no later than one past the merged block and,
            // like every run from `left` on, ends no earlier than one before
            // `lo`: the union is again a block.
            if k > left {
                assert(entry_hi(runs@[left as int]) + 1 < entry_lo(runs@[k as int]));
            }
            assert(b as int + 1 >= cur_lo);
            let new_lo: int = if a < cur_lo { a as int } else { cur_lo as int };
            let new_hi: int = if b > cur_hi { b as int } else { cur_hi as int };
            assert forall|p: int|
                new_lo <= p <= new_hi <==> (lo <= p <= hi || #[trigger] covers(
                    runs@.subrange(left as int, k + 1),
                    p,
                )) by {
                assert(runs@.subrange(left as int, k + 1) =~= runs@.subrange(
                    left as int,
                    k as int,
                ).push(runs@[k as int]));
                lemma_covers_push(runs@.subrange(left as int, k as int), runs@[k as int], p);
            }
            assert forall|j: int| 0 <= j < left implies entry_hi(#[trigger] runs@[j]) + 1
                < new_lo by {
                assert(entry_hi(runs@[j]) + 1 < entry_lo(runs@[k as int]));
            }
        }
        if a < cur_lo {
            cur_lo = a;
        }
        if b > cur_hi {
            cur_hi = b;
        }
        k += 1;
    }
    let right = k;
    let merged = run_entry(cur_lo, cur_hi);
    let mut out: Vec<ExclusionEntry> = Vec::new();
    extend_from(&mut out, runs, 0, left);
    out.push(merged);
    extend_from(&mut out, runs, right, n);
    proof {
        let a = runs@.subrange(0, left as int);
        let m = runs@.subrange(left as int, right as int);
        let b = runs@.subrange(right as int, n as int);
        assert(out@ =~= a.push(merged) + b);
        lemma_canonical_subrange(runs@, 0, left as int);
        lemma_canonical_subrange(runs@, right as int, n as int);
        if left > 0 {
            assert(a.last() == runs@[left - 1]);
        }
        if right < n {
            assert(b[0] == runs@[right as int]);
            // The run after the merged block starts more than one past it.
            if right > left {
                assert(entry_hi(runs@[right - 1]) + 1 < entry_lo(runs@[right as int]));
            }
        }
        lemma_canonical_join(a, merged, b);
        assert(runs@ =~= a + m + b);
        assert forall|p: int| covers(out@, p) <==> (covers(runs@, p) || lo <= p <= hi) by {
            lemma_covers_add(a.push(merged), b, p);
            lemma_covers_push(a, merged, p);
            lemma_covers_add(a + m, b, p);
            lemma_covers_add(a, m, p);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Canonical form: minimality and uniqueness
// ---------------------------------------------------------------------------
/// A port strictly between two consecutive runs of a canonical list is not
/// excluded.
pub proof fn lemma_canonical_gap(c: Seq<ExclusionEntry>, i: int, p: int)
    requires
        canonical(c),
        0 <= i,
        i + 1 < c.len(),
        entry_hi(c[i]) < p < entry_lo(c[i + 1]),
    ensures
        !covers(c, p),
{
    if covers(c, p) {
        let j = choose|j: int| 0 <= j < c.len() && #[trigger] entry_covers(c[j], p);
        if j < i {
            assert(entry_hi(c[j]) + 1 < entry_lo(c[i]));
            assert(entry_canonical(c[i]));
        } else if j > i + 1 {
            assert(entry_hi(c[i + 1]) + 1 < entry_lo(c[j]));
            assert(entry_canonical(c[i + 1]));
        }
    }
}

/// The first and last ports of every run of a canonical list are excluded.
pub proof fn lemma_canonical_run_start(c: Seq<ExclusionEntry>, i: int)
    requires
        canonical(c),
        0 <= i < c.len(),
    ensures
        entry_covers(c[i], entry_lo(c[i])),
        covers(c, entry_lo(c[i])),
        entry_covers(c[i], entry_hi(c[i])),
        covers(c, entry_hi(c[i])),
{
    assert(entry_canonical(c[i]));
    assert(entry_covers(c[i], entry_lo(c[i])));
    assert(entry_covers(c[i], entry_hi(c[i])));
}

/// Minimality: every exclusion list (valid or not) that excludes the same
/// ports as a canonical list has at least as many entries.
///
/// Each run's first port is excluded by some entry of `l`; no entry can do
/// this for two runs, because an entry is an interval and would then also
/// exclude the non-excluded gap port between the runs.
pub proof fn lemma_canonical_minimal(c: Seq<ExclusionEntry>, l: Seq<ExclusionEntry>)
    requires
        canonical(c),
        same_coverage(c, l),
    ensures
        c.len() <= l.len(),
{
    let f = |i: int|
        choose|j: int| 0 <= j < l.len() && #[trigger] entry_covers(l[j], entry_lo(c[i]));
    let x = Set::<int>::range(0, c.len() as int);
    let y = x.map(f);
    let target = Set::<int>::range(0, l.len() as int);
    assert forall|i: int| 0 <= i < c.len() implies 0 <= #[trigger] f(i) < l.len() && entry_covers(
        l[f(i)],
        entry_lo(c[i]),
    ) by {
        lemma_canonical_run_start(c, i);
        assert(covers(l, entry_lo(c[i])));
    }
    assert(x.injective_on(f)) by {
        assert forall|i: int, j: int|
            x.contains(i) && x.contains(j) && #[trigger] f(i) == #[trigger] f(j) implies i == j by {
            if i != j {
                let (a, b) = if i < j {
                    (i, j)
                } else {
                    (j, i)
                };
                // `l[f(a)]` excludes the first ports of runs `a` and `b`, so
                // it also excludes the gap port just after run `a`.
                let g = entry_hi(c[a]) + 1;
                lemma_canonical_run_start(c, a);
                assert(entry_hi(c[a]) + 1 < entry_lo(c[a + 1]));
                if a + 1 < b {
                    assert(entry_hi(c[a + 1]) + 1 < entry_lo(c[b]));
                    lemma_canonical_run_start(c, a + 1);
                }
                assert(entry_covers(l[f(a)], g));
                assert(covers(l, g));
                lemma_canonical_gap(c, a, g);
            }
        }
    }
    vstd::set_lib::lemma_map_size(x, y, f);
    assert(y.subset_of(target));
    vstd::set_lib::lemma_len_subset(y, target);
    vstd::set_lib::lemma_int_range(0, c.len() as int);
    vstd::set_lib::lemma_int_range(0, l.len() as int);
}

/// Uniqueness, one position at a time: two canonical lists with the same
/// coverage that agree before position `k` agree at `k`.
proof fn lemma_canonical_unique_at(a: Seq<ExclusionEntry>, b: Seq<ExclusionEntry>, k: int)
    requires
        canonical(a),
        canonical(b),
        same_coverage(a, b),
        0 <= k < a.len(),
        k < b.len(),
        forall|i: int| 0 <= i < k ==> a[i] == b[i],
    ensures
        a[k] == b[k],
{
    lemma_canonical_run_start(a, k);
    lemma_canonical_run_start(b, k);
    // Equal first ports: the smaller one is excluded by both lists, but by
    // no run of the other list.
    if entry_lo(a[k]) < entry_lo(b[k]) {
        let p = entry_lo(a[k]);
        assert(covers(b, p));
        let j = choose|j: int| 0 <= j < b.len() && #[trigger] entry_covers(b[j], p);
        if j < k {
            assert(a[j] == b[j]);
            assert(entry_hi(a[j]) + 1 < entry_lo(a[k]));
        } else if j > k {
            assert(entry_hi(b[k]) + 1 < entry_lo(b[j]));
        }
    }
    if entry_lo(b[k]) < entry_lo(a[k]) {
        let p = entry_lo(b[k]);
        assert(covers(a, p));
        let j = choose|j: int| 0 <= j < a.len() && #[trigger] entry_covers(a[j], p);
        if j < k {
            assert(a[j] == b[j]);
            assert(entry_hi(b[j]) + 1 < entry_lo(b[k]));
        } else if j > k {
            assert(entry_hi(a[k]) + 1 < entry_lo(a[j]));
        }
    }
    // Equal last ports: the port after the shorter run is excluded by the
    // longer run, but by no run of the other list.
    if entry_hi(a[k]) < entry_hi(b[k]) {
        let p = entry_hi(a[k]) + 1;
        assert(entry_covers(b[k], p));
        assert(covers(a, p));
        let j = choose|j: int| 0 <= j < a.len() && #[trigger] entry_covers(a[j], p);
        if j < k {
            assert(entry_hi(a[j]) + 1 < entry_lo(a[k]));
        } else if j > k {
            assert(entry_hi(a[k]) + 1 < entry_lo(a[j]));
        }
    }
    if entry_hi(b[k]) < entry_hi(a[k]) {
        let p = entry_hi(b[k]) + 1;
        assert(entry_covers(a[k], p));
        assert(covers(b, p));
        let j = choose|j: int| 0 <= j < b.len() && #[trigger] entry_covers(b[j], p);
        if j < k {
            assert(entry_hi(b[j]) + 1 < entry_lo(b[k]));
        } else if j > k {
            assert(entry_hi(b[k]) + 1 < entry_lo(b[j]));
        }
    }
    assert(entry_canonical(a[k]));
    assert(entry_canonical(b[k]));
}

/// Two canonical lists with the same coverage agree on every position below
/// `k` (for `k` no longer than either list).
proof fn lemma_canonical_unique_prefix(a: Seq<ExclusionEntry>, b: Seq<ExclusionEntry>, k: int)
    requires
        canonical(a),
        canonical(b),
        same_coverage(a, b),
        0 <= k <= a.len(),
        k <= b.len(),
    ensures
        forall|i: int| 0 <= i < k ==> a[i] == b[i],
    decreases k,
{
    if k > 0 {
        lemma_canonical_unique_prefix(a, b, k - 1);
        lemma_canonical_unique_at(a, b, k - 1);
    }
}

/// Uniqueness: two canonical lists that exclude the same ports are equal.
pub proof fn lemma_canonical_unique(a: Seq<ExclusionEntry>, b: Seq<ExclusionEntry>)
    requires
        canonical(a),
        canonical(b),
        same_coverage(a, b),
    ensures
        a == b,
{
    let n = if a.len() <= b.len() {
        a.len() as int
    } else {
        b.len() as int
    };
    lemma_canonical_unique_prefix(a, b, n);
    // Neither list has a run beyond the common prefix: its first port would
    // be excluded by an earlier (shared) run of the other list.
    if a.len() < b.len() {
        lemma_canonical_run_start(b, n);
        assert(covers(a, entry_lo(b[n])));
        let j = choose|j: int| 0 <= j < a.len() && #[trigger] entry_covers(a[j], entry_lo(b[n]));
        assert(a[j] == b[j]);
        assert(entry_hi(b[j]) + 1 < entry_lo(b[n]));
    }
    if b.len() < a.len() {
        lemma_canonical_run_start(a, n);
        assert(covers(b, entry_lo(a[n])));
        let j = choose|j: int| 0 <= j < b.len() && #[trigger] entry_covers(b[j], entry_lo(a[n]));
        assert(a[j] == b[j]);
        assert(entry_hi(a[j]) + 1 < entry_lo(a[n]));
    }
    assert(a =~= b);
}

// ---------------------------------------------------------------------------
// Cardinality
// ---------------------------------------------------------------------------
/// The total length of the entries of `es` (overlaps counted repeatedly).
pub open spec fn runs_size(es: Seq<ExclusionEntry>) -> int
    decreases es.len(),
{
    if es.len() == 0 {
        0
    } else {
        runs_size(es.drop_last()) + entry_hi(es.last()) - entry_lo(es.last()) + 1
    }
}

/// For a canonical list, the number of excluded ports is the sum of the run
/// lengths, which never exceeds the last run's end.
pub proof fn lemma_canonical_size(es: Seq<ExclusionEntry>)
    requires
        canonical(es),
    ensures
        port_set(es).len() == runs_size(es),
        es.len() > 0 ==> runs_size(es) <= entry_hi(es.last()),
    decreases es.len(),
{
    if es.len() == 0 {
        assert(port_set(es) =~= Set::<int>::empty());
    } else {
        let init = es.drop_last();
        let last = es.last();
        lemma_canonical_subrange(es, 0, es.len() - 1);
        assert(init =~= es.subrange(0, es.len() - 1));
        lemma_canonical_size(init);
        assert(entry_canonical(es[es.len() - 1]));
        let run = span_set(entry_lo(last), entry_hi(last));
        assert(es =~= init.push(last));
        assert forall|p: int|
            #[trigger] port_set(es).contains(p) <==> (port_set(init) + run).contains(p) by {
            lemma_covers_push(init, last, p);
        }
        assert(port_set(es) =~= port_set(init) + run);
        assert forall|p: int| #[trigger] port_set(init).contains(p) implies !run.contains(p) by {
            let i = choose|i: int| 0 <= i < init.len() && #[trigger] entry_covers(init[i], p);
            assert(es[i] == init[i]);
            assert(entry_hi(es[i]) + 1 < entry_lo(es[es.len() - 1]));
        }
        assert(port_set(init).disjoint(run));
        vstd::set_lib::lemma_set_disjoint_lens(port_set(init), run);
        vstd::set_lib::lemma_int_range(entry_lo(last), entry_hi(last) + 1);
        if init.len() > 0 {
            assert(init.last() == es[es.len() - 2]);
            assert(entry_hi(es[es.len() - 2]) + 1 < entry_lo(es[es.len() - 1]));
        }
    }
}

// ---------------------------------------------------------------------------
// Compaction
// ---------------------------------------------------------------------------
/// `i` is the first invalid entry of `es`, and `err` is the validator's error
/// for it.
pub open spec fn first_invalid(es: Seq<ExclusionEntry>, i: int, err: ExclusionError) -> bool {
    0 <= i < es.len() && entry_error(es[i]) == Some(err) && all_valid(es.take(i))
}

/// The lowest port of each entry, as sort keys.
fn low_ports(es: &[ExclusionEntry]) -> (keys: Vec<u64>)
    ensures
        keys@.len() == es@.len(),
        forall|j: int| 0 <= j < es@.len() ==> keys@[j] as int == entry_lo(#[trigger] es@[j]),
{
    let mut keys: Vec<u64> = Vec::new();
    let mut index: usize = 0;
    while index < es.len()
        invariant
            index <= es@.len(),
            keys@.len() == index,
            forall|j: int| 0 <= j < index ==> keys@[j] as int == entry_lo(#[trigger] es@[j]),
        decreases es@.len() - index,
    {
        keys.push(u64::from(entry_bounds(es[index]).0));
        index += 1;
    }
    keys
}

/// Adds the ports `lo..=hi` to the canonical run list `runs`, whose last run
/// starts no later than `lo`: they extend the last run when they start no
/// later than one past its end, and form a new last run otherwise.
fn extend_runs(runs: &mut Vec<ExclusionEntry>, lo: u16, hi: u16)
    requires
        canonical(old(runs)@),
        1 <= lo <= hi,
        old(runs)@.len() > 0 ==> entry_lo(old(runs)@.last()) <= lo,
    ensures
        canonical(final(runs)@),
        final(runs)@.len() > 0,
        entry_lo(final(runs)@.last()) <= lo,
        forall|p: int| #[trigger]
            covers(final(runs)@, p) <==> (covers(old(runs)@, p) || lo <= p <= hi),
{
    let len = runs.len();
    if len == 0 || u32::from(entry_bounds(runs[len - 1]).1) + 1 < u32::from(lo) {
        // A gap separates the ports from every run: they form a new run.
        let run = run_entry(lo, hi);
        proof {
            lemma_canonical_join(runs@, run, Seq::empty());
            assert(runs@.push(run) + Seq::<ExclusionEntry>::empty() =~= runs@.push(run));
            assert forall|p: int| #[trigger]
                covers(runs@.push(run), p) <==> (covers(runs@, p) || lo <= p <= hi) by {
                lemma_covers_push(runs@, run, p);
            }
        }
        runs.push(run);
    } else {
        // The ports start at or after the last run's start and no later
        // than one past its end: the union is one run.
        let ghost before = runs@;
        let (last_lo, last_hi) = entry_bounds(runs[len - 1]);
        let top = if hi > last_hi {
            hi
        } else {
            last_hi
        };
        proof {
            lemma_canonical_valid(before);
            assert(entry_valid(before[len - 1]));
        }
        let run = run_entry(last_lo, top);
        proof {
            let init = before.subrange(0, len - 1);
            lemma_canonical_subrange(before, 0, len - 1);
            if len > 1 {
                assert(entry_hi(before[len - 2]) + 1 < entry_lo(before[len - 1]));
                assert(init.last() == before[len - 2]);
            }
            assert(before =~= init.push(before[len - 1]));
            assert(before.update(len - 1, run) =~= init.push(run));
            lemma_canonical_join(init, run, Seq::empty());
            assert(init.push(run) + Seq::<ExclusionEntry>::empty() =~= init.push(run));
            assert forall|p: int| #[trigger]
                covers(init.push(run), p) <==> (covers(before, p) || lo <= p <= hi) by {
                lemma_covers_push(init, run, p);
                lemma_covers_push(init, before[len - 1], p);
            }
        }
        runs[len - 1] = run;
    }
}

/// The entries of `es` at the positions `order` lists, in that order.
pub open spec fn in_order(es: Seq<ExclusionEntry>, order: Seq<usize>) -> Seq<ExclusionEntry> {
    Seq::new(order.len(), |b: int| es[order[b] as int])
}

/// A list in stable sorted order excludes the same ports as the list.
proof fn lemma_in_order_coverage(es: Seq<ExclusionEntry>, keys: Seq<u64>, order: Seq<usize>)
    requires
        keys.len() == es.len(),
        super::sort::is_stable_order(keys, order),
    ensures
        same_coverage(in_order(es, order), es),
{
    let s = in_order(es, order);
    assert forall|p: int| covers(s, p) == covers(es, p) by {
        if covers(s, p) {
            let b = choose|b: int| 0 <= b < s.len() && #[trigger] entry_covers(s[b], p);
            let k = order[b] as int;
            assert(0 <= k < es.len());
            assert(entry_covers(es[k], p));
        }
        if covers(es, p) {
            let k = choose|k: int| 0 <= k < es.len() && #[trigger] entry_covers(es[k], p);
            assert(super::render::occurs(order, k));
            let b = choose|b: int| 0 <= b < order.len() && order[b] as int == k;
            assert(s[b] == es[k]);
            assert(entry_covers(s[b], p));
        }
    }
}

/// Builds the canonical run list of a valid exclusion list.
///
/// The entries are validated, put in ascending order of their lowest port
/// by the verified stable merge sort (`super::sort::stable_order`), and
/// merged in one pass (`extend_runs`): each entry either extends the last
/// run or starts a new one. That is `O(n log n)` for `n` entries, instead of
/// one copy of the run list per entry.
fn canonical_runs(es: &[ExclusionEntry]) -> (r: Result<
    Vec<ExclusionEntry>,
    (usize, ExclusionError),
>)
    ensures
        r is Ok <==> all_valid(es@),
        r matches Ok(out) ==> is_compaction(out@, es@),
        r matches Err((i, err)) ==> first_invalid(es@, i as int, err),
{
    validate_entries(es)?;
    let keys = low_ports(es);
    let order = super::sort::stable_order(keys.as_slice());
    let mut runs: Vec<ExclusionEntry> = Vec::new();
    let mut pos: usize = 0;
    proof {
        assert(order@.take(0) =~= Seq::<usize>::empty());
        assert(in_order(es@, order@.take(0)) =~= Seq::<ExclusionEntry>::empty());
    }
    while pos < order.len()
        invariant
            all_valid(es@),
            keys@.len() == es@.len(),
            forall|j: int| 0 <= j < es@.len() ==> keys@[j] as int == entry_lo(#[trigger] es@[j]),
            super::sort::is_stable_order(keys@, order@),
            pos <= order@.len() == es@.len(),
            canonical(runs@),
            same_coverage(runs@, in_order(es@, order@.take(pos as int))),
            (runs@.len() == 0) == (pos == 0),
            pos > 0 ==> entry_lo(runs@.last()) <= keys@[order@[pos - 1] as int],
        decreases es@.len() - pos,
    {
        let at = order[pos];
        proof {
            assert(0 <= order@[pos as int] < es@.len());
        }
        let entry = es[at];
        let (lo, hi) = entry_bounds(entry);
        let ghost before = runs@;
        let ghost prefix = in_order(es@, order@.take(pos as int));
        proof {
            assert(entry_valid(es@[at as int]));
            assert(keys@[at as int] as int == entry_lo(entry));
            if pos > 0 {
                // Sorted: the previous entry's lowest port is at most this
                // one's.
                assert(super::sort::key_before(
                    keys@,
                    order@[pos - 1] as int,
                    order@[pos as int] as int,
                ));
            }
            assert(order@.take(pos + 1) =~= order@.take(pos as int).push(at));
            assert(in_order(es@, order@.take(pos + 1)) =~= prefix.push(entry));
        }
        extend_runs(&mut runs, lo, hi);
        proof {
            assert forall|p: int| #[trigger]
                covers(runs@, p) == covers(prefix.push(entry), p) by {
                lemma_covers_push(prefix, entry, p);
            }
        }
        pos += 1;
    }
    proof {
        assert(order@.take(es@.len() as int) =~= order@);
        lemma_in_order_coverage(es@, keys@, order@);
    }
    Ok(runs)
}

/// Compacts an exclusion list: the unique canonical list that excludes
/// exactly the same ports, or the first invalid entry.
///
/// Invalid entries are never dropped silently: the list is rejected.
pub fn compact_exclusions(es: &[ExclusionEntry]) -> (r: Result<
    Vec<ExclusionEntry>,
    (usize, ExclusionError),
>)
    ensures
        r is Ok <==> all_valid(es@),
        r matches Err((i, err)) ==> first_invalid(es@, i as int, err),
        // The excluded ports are unchanged, and the result is canonical.
        r matches Ok(out) ==> is_compaction(out@, es@),
        r matches Ok(out) ==> port_set(out@) == port_set(es@),
        // Minimal: no exclusion list with the same coverage is shorter.
        r matches Ok(out) ==> forall|l: Seq<ExclusionEntry>| #[trigger]
            same_coverage(l, es@) ==> out@.len() <= l.len(),
        r matches Ok(out) ==> out@.len() <= es@.len(),
        // Unique: it is the only canonical list with this coverage, so it
        // does not depend on entry order, duplicates, or overlaps.
        r matches Ok(out) ==> forall|c: Seq<ExclusionEntry>| #[trigger]
            is_compaction(c, es@) ==> c == out@,
        // Idempotent: a canonical list is its own compaction.
        r matches Ok(out) ==> (canonical(es@) ==> out@ == es@),
{
    let r = canonical_runs(es);
    proof {
        if r is Ok {
            let out = r->Ok_0;
            assert(port_set(out@) =~= port_set(es@));
            assert forall|l: Seq<ExclusionEntry>| #[trigger]
                same_coverage(l, es@) implies out@.len() <= l.len() by {
                lemma_canonical_minimal(out@, l);
            }
            lemma_canonical_minimal(out@, es@);
            assert forall|c: Seq<ExclusionEntry>| #[trigger]
                is_compaction(c, es@) implies c == out@ by {
                lemma_canonical_unique(c, out@);
            }
            if canonical(es@) {
                lemma_canonical_unique(es@, out@);
            }
        }
    }
    r
}

// ---------------------------------------------------------------------------
// The excluded-port set
// ---------------------------------------------------------------------------
/// Appends the ports `from..=to` to `v`.
fn push_span(v: &mut Vec<u16>, from: u16, to: u16)
    requires
        from <= to,
    ensures
        final(v)@ == old(v)@ + super::port::ports_between(from as int, to as int),
{
    let mut p = from;
    let mut done = false;
    while !done
        invariant
            from <= p <= to,
            done ==> v@ == old(v)@ + super::port::ports_between(from as int, to as int),
            !done ==> v@ == old(v)@ + super::port::ports_between(from as int, p - 1),
        decreases to - p + if done {
            0int
        } else {
            1int
        },
    {
        v.push(p);
        proof {
            assert(super::port::ports_between(from as int, p as int)
                =~= super::port::ports_between(from as int, p - 1).push(p));
        }
        if p == to {
            done = true;
        } else {
            p += 1;
        }
    }
}

/// A set of excluded ports, stored as its canonical run list.
///
/// The field is private and every constructor and mutator is verified, so
/// the type invariant (the run list is canonical) holds for every value —
/// including values held by unverified code. Port 0 is never a member.
#[derive(Debug)]
pub struct ExclusionSet {
    runs: Vec<ExclusionEntry>,
}

impl Clone for ExclusionSet {
    /// A copy with the same run list, and so the same excluded ports.
    // Copies the run list entry by entry (a derived `clone` could not be
    // checked against the type invariant).
    fn clone(&self) -> (res: Self)
        ensures
            res.spec_runs() == self.spec_runs(),
            res@ == self@,
    {
        proof {
            use_type_invariant(self);
        }
        let mut runs: Vec<ExclusionEntry> = Vec::new();
        extend_from(&mut runs, self.runs.as_slice(), 0, self.runs.len());
        assert(runs@ =~= self.runs@);
        ExclusionSet { runs }
    }
}

impl View for ExclusionSet {
    type V = Set<int>;

    /// The excluded ports.
    open spec fn view(&self) -> Set<int> {
        port_set(self.spec_runs())
    }
}

impl ExclusionSet {
    #[verifier::type_invariant]
    spec fn well_formed(self) -> bool {
        canonical(self.runs@)
    }

    /// The canonical run list.
    pub closed spec fn spec_runs(self) -> Seq<ExclusionEntry> {
        self.runs@
    }

    /// The empty set.
    pub fn empty() -> (s: ExclusionSet)
        ensures
            s@ == Set::<int>::empty(),
    {
        let s = ExclusionSet { runs: Vec::new() };
        assert(s@ =~= Set::<int>::empty());
        s
    }

    /// The set of ports excluded by `es`, or the first invalid entry.
    pub fn from_entries(es: &[ExclusionEntry]) -> (r: Result<ExclusionSet, (usize, ExclusionError)>)
        ensures
            r is Ok <==> all_valid(es@),
            r matches Ok(s) ==> s@ == port_set(es@),
            r matches Ok(s) ==> forall|p: int|
                #![trigger s@.contains(p)]
                #![trigger covers(es@, p)]
                s@.contains(p) <==> covers(es@, p),
            r matches Err((i, err)) ==> first_invalid(es@, i as int, err),
    {
        match canonical_runs(es) {
            Ok(runs) => {
                let s = ExclusionSet { runs };
                proof {
                    assert(s@ =~= port_set(es@));
                    lemma_port_set_valid(es@);
                }
                Ok(s)
            },
            Err(e) => Err(e),
        }
    }

    /// Whether `port` is in the set (binary search over the runs).
    pub fn contains(&self, port: u16) -> (b: bool)
        ensures
            b == self@.contains(port as int),
    {
        proof {
            use_type_invariant(self);
            lemma_canonical_valid(self.runs@);
            lemma_port_set_valid(self.runs@);
        }
        let mut lo: usize = 0;
        let mut hi: usize = self.runs.len();
        while lo < hi
            invariant
                lo <= hi <= self.runs@.len(),
                canonical(self.runs@),
                forall|j: int| 0 <= j < lo ==> entry_hi(#[trigger] self.runs@[j]) < port,
                forall|j: int|
                    hi <= j < self.runs@.len() ==> entry_lo(#[trigger] self.runs@[j]) > port,
            decreases hi - lo,
        {
            let mid = lo + (hi - lo) / 2;
            let (a, b) = entry_bounds(self.runs[mid]);
            if b < port {
                proof {
                    assert forall|j: int| 0 <= j <= mid implies entry_hi(
                        #[trigger] self.runs@[j],
                    ) < port by {
                        if j < mid {
                            assert(entry_hi(self.runs@[j]) + 1 < entry_lo(self.runs@[mid as int]));
                            assert(entry_canonical(self.runs@[mid as int]));
                        }
                    }
                }
                lo = mid + 1;
            } else if port < a {
                proof {
                    assert forall|j: int| mid <= j < self.runs@.len() implies entry_lo(
                        #[trigger] self.runs@[j],
                    ) > port by {
                        if j > mid {
                            assert(entry_hi(self.runs@[mid as int]) + 1 < entry_lo(self.runs@[j]));
                            assert(entry_canonical(self.runs@[mid as int]));
                        }
                    }
                }
                hi = mid;
            } else {
                assert(entry_covers(self.runs@[mid as int], port as int));
                return true;
            }
        }
        false
    }

    /// Inserts the ports `lo..=hi`.
    fn insert_bounds(&mut self, lo: u16, hi: u16)
        requires
            1 <= lo <= hi,
        ensures
            final(self)@ == old(self)@ + span_set(lo as int, hi as int),
    {
        proof {
            use_type_invariant(&*self);
        }
        let runs = insert_run(self.runs.as_slice(), lo, hi);
        self.runs = runs;
        assert(final(self)@ =~= old(self)@ + span_set(lo as int, hi as int));
    }

    /// Inserts one port. Port 0 is not a valid port: it is rejected and the
    /// set is unchanged.
    pub fn insert_port(&mut self, port: u16) -> (inserted: bool)
        ensures
            inserted == super::port::is_valid_port(port as int),
            final(self)@ == if inserted {
                old(self)@.insert(port as int)
            } else {
                old(self)@
            },
    {
        if port == 0 {
            false
        } else {
            self.insert_bounds(port, port);
            assert(final(self)@ =~= old(self)@.insert(port as int));
            true
        }
    }

    /// Inserts every port of `span`.
    pub fn insert_span(&mut self, span: PortSpan)
        ensures
            final(self)@ == old(self)@ + span_set(span.spec_min() as int, span.spec_max() as int),
    {
        let lo = span.min();
        let hi = span.max();
        self.insert_bounds(lo, hi);
    }

    /// Inserts the ports an entry names. An invalid entry is rejected with
    /// the validator's error, and the set is unchanged (atomicity).
    pub fn insert_entry(&mut self, e: ExclusionEntry) -> (r: Result<(), ExclusionError>)
        ensures
            r is Ok <==> entry_valid(e),
            r matches Err(err) ==> entry_error(e) == Some(err) && final(self)@ == old(self)@,
            r is Ok ==> final(self)@ == old(self)@ + span_set(entry_lo(e), entry_hi(e)),
    {
        validate_entry(e)?;
        let (lo, hi) = entry_bounds(e);
        self.insert_bounds(lo, hi);
        Ok(())
    }

    /// The canonical run list of the set: its compacted exclusion list.
    pub fn runs(&self) -> (out: Vec<ExclusionEntry>)
        ensures
            out@ == self.spec_runs(),
            canonical(out@),
            port_set(out@) == self@,
            forall|p: int|
                #![trigger covers(out@, p)]
                #![trigger self@.contains(p)]
                covers(out@, p) <==> self@.contains(p),
            // The only canonical list for this set of ports.
            forall|c: Seq<ExclusionEntry>| #[trigger]
                canonical(c) && port_set(c) == self@ ==> c == out@,
    {
        proof {
            use_type_invariant(self);
            lemma_canonical_valid(self.runs@);
            lemma_port_set_valid(self.runs@);
            assert forall|c: Seq<ExclusionEntry>| #[trigger]
                canonical(c) && port_set(c) == self@ implies c == self.runs@ by {
                lemma_canonical_valid(c);
                lemma_port_set_valid(c);
                assert forall|p: int| #[trigger] covers(c, p) == covers(self.runs@, p) by {
                    assert(port_set(c).contains(p) == port_set(self.runs@).contains(p));
                }
                lemma_canonical_unique(c, self.runs@);
            }
        }
        let mut out: Vec<ExclusionEntry> = Vec::new();
        extend_from(&mut out, self.runs.as_slice(), 0, self.runs.len());
        assert(out@ =~= self.runs@);
        out
    }

    /// The number of ports in the set.
    pub fn len(&self) -> (n: usize)
        ensures
            n == self@.len(),
            n <= 65535,
    {
        proof {
            use_type_invariant(self);
        }
        let mut total: usize = 0;
        let mut i: usize = 0;
        while i < self.runs.len()
            invariant
                i <= self.runs@.len(),
                canonical(self.runs@),
                total == runs_size(self.runs@.take(i as int)),
                i > 0 ==> total <= entry_hi(self.runs@[i - 1]),
            decreases self.runs@.len() - i,
        {
            let (a, b) = entry_bounds(self.runs[i]);
            proof {
                let next = self.runs@.take(i as int + 1);
                assert(next =~= self.runs@.subrange(0, i as int + 1));
                lemma_canonical_subrange(self.runs@, 0, i as int + 1);
                lemma_canonical_size(next);
                assert(next.drop_last() =~= self.runs@.take(i as int));
                assert(entry_canonical(self.runs@[i as int]));
            }
            total = total + usize::from(b - a) + 1;
            i += 1;
        }
        proof {
            assert(self.runs@.take(i as int) =~= self.runs@);
            lemma_canonical_size(self.runs@);
        }
        total
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> (empty: bool)
        ensures
            empty == (self@.len() == 0),
            empty == (self@ == Set::<int>::empty()),
    {
        proof {
            use_type_invariant(self);
            lemma_canonical_valid(self.runs@);
            lemma_port_set_valid(self.runs@);
            if self.runs@.len() == 0 {
                assert(self@ =~= Set::<int>::empty());
            } else {
                lemma_canonical_run_start(self.runs@, 0);
                assert(self@.contains(entry_lo(self.runs@[0])));
            }
        }
        self.runs.is_empty()
    }

    /// The members of the set within `lo..=hi`, in strictly ascending order.
    pub fn members_between(&self, lo: u16, hi: u16) -> (members: Vec<u16>)
        ensures
            forall|i: int, j: int| 0 <= i < j < members@.len() ==> members@[i] < members@[j],
            forall|p: u16|
                #![trigger members@.contains(p)]
                #![trigger self@.contains(p as int)]
                members@.contains(p) <==> (lo <= p <= hi && self@.contains(p as int)),
            forall|i: int|
                0 <= i < members@.len() ==> super::port::is_valid_port(
                    #[trigger] members@[i] as int,
                ),
    {
        proof {
            use_type_invariant(self);
            lemma_canonical_valid(self.runs@);
            lemma_port_set_valid(self.runs@);
        }
        let n = self.runs.len();
        let mut members: Vec<u16> = Vec::new();
        let mut i: usize = 0;
        while i < n
            invariant
                i <= n == self.runs@.len(),
                canonical(self.runs@),
                forall|x: int, y: int| 0 <= x < y < members@.len() ==> members@[x] < members@[y],
                forall|x: int|
                    0 <= x < members@.len() ==> lo <= #[trigger] members@[x] <= hi && covers(
                        self.runs@.take(i as int),
                        members@[x] as int,
                    ),
                forall|p: u16|
                    lo <= p <= hi && covers(self.runs@.take(i as int), p as int)
                        ==> #[trigger] members@.contains(p),
                i < n ==> forall|x: int|
                    0 <= x < members@.len() ==> #[trigger] members@[x] < entry_lo(
                        self.runs@[i as int],
                    ),
            decreases n - i,
        {
            let (run_lo, run_hi) = entry_bounds(self.runs[i]);
            let from = if run_lo < lo {
                lo
            } else {
                run_lo
            };
            let to = if run_hi > hi {
                hi
            } else {
                run_hi
            };
            let ghost old_members = members@;
            let ghost prefix = self.runs@.take(i as int);
            let ghost next = self.runs@.take(i as int + 1);
            proof {
                assert(next =~= prefix.push(self.runs@[i as int]));
                assert(entry_canonical(self.runs@[i as int]));
            }
            if from <= to {
                push_span(&mut members, from, to);
            }
            proof {
                let added = if from <= to {
                    super::port::ports_between(from as int, to as int)
                } else {
                    Seq::<u16>::empty()
                };
                assert(members@ =~= old_members + added);
                assert forall|x: int| 0 <= x < members@.len() implies lo <= #[trigger] members@[x]
                    <= hi && covers(next, members@[x] as int) by {
                    lemma_covers_push(prefix, self.runs@[i as int], members@[x] as int);
                    if x >= old_members.len() {
                        assert(members@[x] == added[x - old_members.len()]);
                    }
                }
                assert forall|p: u16|
                    lo <= p <= hi && covers(next, p as int) implies #[trigger] members@.contains(
                        p,
                    ) by {
                    lemma_covers_push(prefix, self.runs@[i as int], p as int);
                    if covers(prefix, p as int) {
                        assert(old_members.contains(p));
                        let x = choose|x: int| 0 <= x < old_members.len() && old_members[x] == p;
                        assert(members@[x] == p);
                    } else {
                        let x = old_members.len() + (p - from);
                        assert(members@[x] == p);
                    }
                }
                if i + 1 < n {
                    assert(entry_hi(self.runs@[i as int]) + 1 < entry_lo(self.runs@[i + 1]));
                    assert forall|x: int| 0 <= x < members@.len() implies #[trigger] members@[x]
                        < entry_lo(self.runs@[i + 1]) by {
                        if x >= old_members.len() {
                            assert(members@[x] == added[x - old_members.len()]);
                        }
                    }
                }
            }
            i += 1;
        }
        proof {
            assert(self.runs@.take(i as int) =~= self.runs@);
            assert forall|x: int| 0 <= x < members@.len() implies super::port::is_valid_port(
                #[trigger] members@[x] as int,
            ) by {
                lemma_valid_covers_valid_ports(self.runs@, members@[x] as int);
            }
        }
        members
    }
}

// ---------------------------------------------------------------------------
// Exclusion text: `"5001"` and `"5000..5010"`
// ---------------------------------------------------------------------------
/// An ASCII decimal digit (`b'0'..=b'9'`).
pub open spec fn is_digit(b: u8) -> bool {
    48 <= b <= 57
}

/// Every byte is an ASCII decimal digit.
pub open spec fn all_digits(s: Seq<u8>) -> bool {
    forall|i: int| 0 <= i < s.len() ==> is_digit(#[trigger] s[i])
}

/// The value of a string of decimal digits, most significant first.
pub open spec fn decimal_value(s: Seq<u8>) -> int
    decreases s.len(),
{
    if s.len() == 0 {
        0
    } else {
        decimal_value(s.drop_last()) * 10 + (s.last() - 48)
    }
}

/// The text after an optional leading `+`.
pub open spec fn unsigned_digits(s: Seq<u8>) -> Seq<u8> {
    if s.len() > 0 && s[0] == 43 {
        s.drop_first()
    } else {
        s
    }
}

/// `s` is the decimal text of the `u16` value `v`: an optional `+`, then
/// one or more ASCII digits (leading zeros allowed) whose value is at most
/// 65535. No whitespace, no `-`.
///
/// This is meant to be the syntax `u16::from_str` accepts. That
/// correspondence is not proved (`vstd` does not specify `from_str`): it is
/// trusted, and pinned by differential tests in the glue
/// (`test_parse_u16_kernel_matches_from_str`,
/// `prop_decimal_parsers_match_std`).
pub open spec fn is_u16_text(s: Seq<u8>, v: int) -> bool {
    let d = unsigned_digits(s);
    d.len() > 0 && all_digits(d) && decimal_value(d) == v && v <= 65535
}

/// The decimal value of digits is never negative.
proof fn lemma_decimal_value_nonneg(s: Seq<u8>)
    requires
        all_digits(s),
    ensures
        decimal_value(s) >= 0,
    decreases s.len(),
{
    if s.len() > 0 {
        assert(all_digits(s.drop_last())) by {
            assert forall|i: int| 0 <= i < s.drop_last().len() implies is_digit(
                #[trigger] s.drop_last()[i],
            ) by {
                assert(s.drop_last()[i] == s[i]);
            }
        }
        lemma_decimal_value_nonneg(s.drop_last());
        assert(is_digit(s[s.len() - 1]));
    }
}

/// Appending digits never decreases the value.
proof fn lemma_decimal_value_monotone(s: Seq<u8>, k: int)
    requires
        all_digits(s),
        0 <= k <= s.len(),
    ensures
        decimal_value(s.take(k)) <= decimal_value(s),
    decreases s.len() - k,
{
    if k < s.len() {
        let t = s.take(k + 1);
        let p = s.take(k);
        assert(all_digits(p)) by {
            assert forall|i: int| 0 <= i < p.len() implies is_digit(#[trigger] p[i]) by {
                assert(p[i] == s[i]);
            }
        }
        assert(t.drop_last() =~= p);
        assert(t.last() == s[k]);
        assert(is_digit(s[k]));
        lemma_decimal_value_nonneg(p);
        assert(decimal_value(t) == decimal_value(p) * 10 + (s[k] - 48));
        lemma_decimal_value_monotone(s, k + 1);
    } else {
        assert(s.take(k) =~= s);
    }
}

/// `s` is the decimal text of `v` (an optional `+`, then one or more ASCII
/// digits, leading zeros allowed; no whitespace, no `-`), and `v` is at most
/// `max`. `is_u16_text(s, v)` is `is_unsigned_text(s, v, 65535)`.
///
/// This is meant to be the syntax of the unsigned integer `from_str`
/// implementations. That correspondence is trusted and pinned by
/// differential tests, not proved.
pub open spec fn is_unsigned_text(s: Seq<u8>, v: int, max: int) -> bool {
    let d = unsigned_digits(s);
    d.len() > 0 && all_digits(d) && decimal_value(d) == v && v <= max
}

/// Parses an unsigned decimal number of at most `max` (for any byte input),
/// without overflow: the result is the value of the text exactly when the
/// text is well formed (`is_unsigned_text`) and its value is at most `max`.
///
/// With `max` equal to `u16::MAX`, `u32::MAX` or `u64::MAX` this is meant to
/// agree with `u16::from_str`, `u32::from_str` or `u64::from_str`. Only the
/// agreement with `is_unsigned_text` is proved; the agreement with `from_str`
/// is trusted and pinned by differential tests
/// (`test_parse_u16_kernel_matches_from_str`,
/// `prop_decimal_parsers_match_std`).
pub fn parse_unsigned(s: &[u8], max: u64) -> (r: Option<u64>)
    ensures
        r matches Some(v) ==> is_unsigned_text(s@, v as int, max as int),
        r is None ==> forall|v: int| !is_unsigned_text(s@, v, max as int),
{
    let n = s.len();
    let mut start: usize = 0;
    if n > 0 && s[0] == b'+' {
        start = 1;
    }
    let ghost d = unsigned_digits(s@);
    proof {
        assert(d =~= s@.subrange(start as int, n as int));
    }
    if start == n {
        return None;
    }
    let mut value: u64 = 0;
    let mut i = start;
    while i < n
        invariant
            start <= i <= n == s@.len(),
            d == unsigned_digits(s@),
            d == s@.subrange(start as int, n as int),
            all_digits(d.take(i - start)),
            value as int == decimal_value(d.take(i - start)),
        decreases n - i,
    {
        let b = s[i];
        proof {
            assert(d[i - start] == b);
        }
        // Bytes below `b'0'` wrap around to values above 9.
        let digit = b.wrapping_sub(b'0');
        if digit > 9 {
            proof {
                if all_digits(d) {
                    assert(is_digit(d[i - start]));
                }
            }
            return None;
        }
        let ghost next = d.take(i - start + 1);
        proof {
            assert(next.drop_last() =~= d.take(i - start));
            assert(next.last() == b);
            assert(decimal_value(next) == decimal_value(d.take(i - start)) * 10 + (b - 48));
            assert(all_digits(next)) by {
                assert forall|j: int| 0 <= j < next.len() implies is_digit(#[trigger] next[j]) by {
                    if j < i - start {
                        assert(next[j] == d.take(i - start)[j]);
                    }
                }
            }
        }
        // On overflow the digits read so far already exceed `u64::MAX` (so
        // `max`), and more digits only increase the value.
        let Some(shifted) = value.checked_mul(10) else {
            proof {
                lemma_decimal_value_nonneg(next.drop_last());
                if all_digits(d) {
                    lemma_decimal_value_monotone(d, i - start + 1);
                }
            }
            return None;
        };
        let Some(next_value) = shifted.checked_add(u64::from(digit)) else {
            proof {
                if all_digits(d) {
                    lemma_decimal_value_monotone(d, i - start + 1);
                }
            }
            return None;
        };
        value = next_value;
        i += 1;
    }
    proof {
        assert(d.take(n - start) =~= d);
    }
    if value <= max {
        Some(value)
    } else {
        None
    }
}

/// Parses a `u16`: accepts exactly `is_u16_text` (for any byte input).
///
/// That this is `u16::from_str`'s syntax and range is trusted and pinned by
/// differential tests, not proved.
pub fn parse_u16(s: &[u8]) -> (r: Option<u16>)
    ensures
        r matches Some(v) ==> is_u16_text(s@, v as int),
        r is None ==> forall|v: int| !is_u16_text(s@, v),
{
    let parsed = parse_unsigned(s, 0xFFFF);
    proof {
        assert forall|v: int| #[trigger] is_u16_text(s@, v) implies is_unsigned_text(
            s@,
            v,
            65535,
        ) by {}
    }
    match parsed {
        Some(v) => u16::try_from(v).ok(),
        None => None,
    }
}

/// `s[i..i + 2]` is the range separator `..`.
pub open spec fn separator_at(s: Seq<u8>, i: int) -> bool {
    0 <= i && i + 1 < s.len() && s[i] == 46 && s[i + 1] == 46
}

/// The position of the first `..` in `s` (where `str::split_once("..")`
/// splits), if any.
pub fn find_range_separator(s: &[u8]) -> (r: Option<usize>)
    ensures
        r matches Some(i) ==> separator_at(s@, i as int) && forall|j: int|
            0 <= j < i ==> !separator_at(s@, j),
        r is None ==> forall|j: int| !separator_at(s@, j),
{
    let n = s.len();
    let mut i: usize = 0;
    while i < n
        invariant
            i <= n == s@.len(),
            forall|j: int| 0 <= j < i ==> !separator_at(s@, j),
        decreases n - i,
    {
        if s[i] == b'.' && i + 1 < n && s[i + 1] == b'.' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The canonical decimal text of `v`: no sign, no leading zeros.
pub open spec fn decimal_text(v: nat) -> Seq<u8>
    decreases v,
{
    if v < 10 {
        seq![(v + 48) as u8]
    } else {
        decimal_text(v / 10).push((v % 10 + 48) as u8)
    }
}

/// The canonical text of a port number parses back to it.
pub proof fn lemma_decimal_text(v: nat)
    ensures
        decimal_text(v).len() > 0,
        all_digits(decimal_text(v)),
        decimal_value(decimal_text(v)) == v,
        forall|j: int| !separator_at(decimal_text(v), j),
        v <= 65535 ==> is_u16_text(decimal_text(v), v as int),
    decreases v,
{
    let t = decimal_text(v);
    if v < 10 {
        assert(t.drop_last() =~= Seq::<u8>::empty());
        assert(decimal_value(t) == decimal_value(t.drop_last()) * 10 + (t.last() - 48));
    } else {
        lemma_decimal_text(v / 10);
        let init = decimal_text(v / 10);
        assert(t.drop_last() =~= init);
        assert(all_digits(t)) by {
            assert forall|i: int| 0 <= i < t.len() implies is_digit(#[trigger] t[i]) by {
                if i < init.len() {
                    assert(t[i] == init[i]);
                }
            }
        }
        assert(v == 10 * (v / 10) + v % 10) by {
            vstd::arithmetic::div_mod::lemma_fundamental_div_mod(v as int, 10);
        }
        assert(t.last() == (v % 10 + 48) as u8);
        assert(decimal_value(t) == decimal_value(init) * 10 + (t.last() - 48));
    }
    assert forall|j: int| !separator_at(t, j) by {
        if 0 <= j < t.len() {
            assert(is_digit(t[j]));
        }
    }
    assert(unsigned_digits(t) == t) by {
        assert(is_digit(t[0]));
    }
}

/// Round trip: the text `"start..end"` built from canonical decimal texts
/// splits at the end of the start text, into exactly the two texts.
pub proof fn lemma_range_text_roundtrip(start: u16, end: u16)
    ensures
        ({
            let a = decimal_text(start as nat);
            let b = decimal_text(end as nat);
            let t = a + seq![46u8, 46u8] + b;
            &&& separator_at(t, a.len() as int)
            &&& forall|j: int| 0 <= j < a.len() ==> !separator_at(t, j)
            &&& t.subrange(0, a.len() as int) == a
            &&& t.subrange(a.len() as int + 2, t.len() as int) == b
            &&& is_u16_text(a, start as int)
            &&& is_u16_text(b, end as int)
        }),
{
    let a = decimal_text(start as nat);
    let b = decimal_text(end as nat);
    let t = a + seq![46u8, 46u8] + b;
    lemma_decimal_text(start as nat);
    lemma_decimal_text(end as nat);
    assert forall|j: int| 0 <= j < a.len() implies !separator_at(t, j) by {
        assert(t[j] == a[j]);
        assert(is_digit(a[j]));
    }
    assert(t[a.len() as int] == 46u8);
    assert(t[a.len() as int + 1] == 46u8);
    assert(t.subrange(0, a.len() as int) =~= a);
    assert(t.subrange(a.len() as int + 2, t.len() as int) =~= b);
}

// ---------------------------------------------------------------------------
// Canonical lists: `trop compact-exclusions` and `trop scan --autocompact`
// ---------------------------------------------------------------------------
/// Whether `es` is already in canonical (compacted) form.
///
/// A list is canonical exactly when it is its own compaction: then
/// `compact_exclusions(es)` returns `es` unchanged (by its uniqueness
/// guarantee), and otherwise compaction changes the list — even when it does
/// not change the number of entries (`[9000, 8000]`, or `[5000..5000]`).
/// Callers rewrite a list exactly when this returns `false`.
pub fn is_compact(es: &[ExclusionEntry]) -> (b: bool)
    ensures
        b == canonical(es@),
        b == is_compaction(es@, es@),
        // A compact list is the only compaction of itself ...
        b ==> forall|c: Seq<ExclusionEntry>| #[trigger] is_compaction(c, es@) ==> c == es@,
        // ... and every compaction of a list that is not compact differs from it.
        !b ==> forall|c: Seq<ExclusionEntry>| #[trigger] is_compaction(c, es@) ==> c != es@,
{
    let n = es.len();
    let mut i: usize = 0;
    while i < n
        invariant
            i <= n == es@.len(),
            canonical(es@.take(i as int)),
        decreases n - i,
    {
        let (lo, hi) = entry_bounds(es[i]);
        let single = match es[i] {
            ExclusionEntry::Single(_) => true,
            ExclusionEntry::Range { .. } => false,
        };
        // The entry is valid and a `Single` exactly when it names one port.
        let entry_ok = lo >= 1 && lo <= hi && single == (lo == hi);
        // It starts more than one port after the previous entry ends.
        let gap_ok = i == 0 || u32::from(entry_bounds(es[i - 1]).1) + 1 < u32::from(lo);
        proof {
            assert(entry_ok == entry_canonical(es@[i as int]));
        }
        if !(entry_ok && gap_ok) {
            proof {
                if !entry_ok {
                    assert(!entry_canonical(es@[i as int]));
                } else {
                    assert(entry_hi(es@[i - 1]) + 1 >= entry_lo(es@[i as int]));
                }
                assert(!canonical(es@));
            }
            return false;
        }
        proof {
            let prefix = es@.take(i as int);
            if i > 0 {
                assert(prefix.last() == es@[i - 1]);
            }
            lemma_canonical_join(prefix, es@[i as int], Seq::<ExclusionEntry>::empty());
            assert(prefix.push(es@[i as int]) + Seq::<ExclusionEntry>::empty() =~= es@.take(
                i as int + 1,
            ));
        }
        i += 1;
    }
    proof {
        assert(es@.take(i as int) =~= es@);
        assert(same_coverage(es@, es@));
        assert forall|c: Seq<ExclusionEntry>| #[trigger] is_compaction(c, es@) implies c == es@ by {
            lemma_canonical_unique(c, es@);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Adding to an exclusion list: `trop exclude` and `trop scan --autoexclude`
// ---------------------------------------------------------------------------
/// `e` and the block of ports `lo..=hi` share at least one port
/// (`lemma_overlaps_block_meaning`). Adjacent entries do not overlap.
pub open spec fn overlaps_block(e: ExclusionEntry, lo: int, hi: int) -> bool {
    &&& entry_lo(e) <= entry_hi(e)
    &&& lo <= hi
    &&& entry_lo(e) <= hi
    &&& lo <= entry_hi(e)
}

/// The bounds of `e` lie within `lo..=hi`.
pub open spec fn entry_within(e: ExclusionEntry, lo: int, hi: int) -> bool {
    lo <= entry_lo(e) && entry_hi(e) <= hi
}

/// Every entry of `es` that overlaps the block `lo..=hi` lies within it.
pub open spec fn closed_block(es: Seq<ExclusionEntry>, lo: int, hi: int) -> bool {
    forall|i: int|
        0 <= i < es.len() && #[trigger] overlaps_block(es[i], lo, hi) ==> entry_within(
            es[i],
            lo,
            hi,
        )
}

/// `lo..=hi` is the merge block of `new` in `es`: the smallest block that
/// contains `new` and within which lies every entry of `es` it overlaps.
/// (Overlap is transitive through the block: an entry that overlaps an entry
/// merged into the block is merged too.)
pub open spec fn merge_block(es: Seq<ExclusionEntry>, new: ExclusionEntry, lo: int, hi: int) -> bool {
    &&& entry_within(new, lo, hi)
    &&& closed_block(es, lo, hi)
    &&& forall|a: int, b: int|
        entry_within(new, a, b) && #[trigger] closed_block(es, a, b) ==> a <= lo && hi <= b
}

/// A single entry of `es` already excludes every port of `new` (in
/// particular, an entry identical to `new`).
pub open spec fn already_excluded(es: Seq<ExclusionEntry>, new: ExclusionEntry) -> bool {
    exists|i: int|
        0 <= i < es.len() && entry_within(new, entry_lo(#[trigger] es[i]), entry_hi(es[i]))
}

/// The entries that do not overlap the block `lo..=hi`.
pub open spec fn outside_block(lo: int, hi: int) -> spec_fn(ExclusionEntry) -> bool {
    |e: ExclusionEntry| !overlaps_block(e, lo, hi)
}

/// `out` is `es` with `new` added through its merge block `lo..=hi`:
/// - if no entry of `es` overlaps the block (`f == es.len()`), `new` is
///   appended unchanged;
/// - otherwise every entry that overlaps the block is replaced by one range
///   naming exactly the block, placed at the position `f` of the first such
///   entry, and every other entry keeps its content and relative order.
///
/// Afterwards no entry other than the one at `f` overlaps the block.
pub open spec fn added_through(
    es: Seq<ExclusionEntry>,
    new: ExclusionEntry,
    out: Seq<ExclusionEntry>,
    lo: int,
    hi: int,
    f: int,
) -> bool {
    &&& merge_block(es, new, lo, hi)
    &&& 0 <= f <= es.len()
    &&& forall|j: int| 0 <= j < f ==> !overlaps_block(#[trigger] es[j], lo, hi)
    &&& if f == es.len() {
        out == es.push(new)
    } else {
        &&& overlaps_block(es[f], lo, hi)
        &&& f < out.len()
        &&& out[f] is Range
        &&& entry_lo(out[f]) == lo
        &&& entry_hi(out[f]) == hi
        &&& out == es.take(f).push(out[f]) + es.skip(f + 1).filter(outside_block(lo, hi))
    }
    &&& forall|j: int| 0 <= j < out.len() && j != f ==> !overlaps_block(#[trigger] out[j], lo, hi)
}

/// `out` is `es` with `new` added through its merge block (`added_through`,
/// for some block and position).
pub open spec fn merged_in(es: Seq<ExclusionEntry>, new: ExclusionEntry, out: Seq<ExclusionEntry>) -> bool {
    exists|lo: int, hi: int, f: int| #[trigger] added_through(es, new, out, lo, hi, f)
}

/// Overlap means sharing a port.
pub proof fn lemma_overlaps_block_meaning(e: ExclusionEntry, lo: int, hi: int)
    ensures
        overlaps_block(e, lo, hi) <==> exists|p: int| lo <= p <= hi && #[trigger] entry_covers(e, p),
{
    if overlaps_block(e, lo, hi) {
        let p = if entry_lo(e) > lo {
            entry_lo(e)
        } else {
            lo
        };
        assert(lo <= p <= hi && entry_covers(e, p));
    }
}

/// An identical entry already excludes every port of `new`.
pub proof fn lemma_identical_is_already_excluded(es: Seq<ExclusionEntry>, new: ExclusionEntry)
    ensures
        (exists|i: int| 0 <= i < es.len() && es[i] == new) ==> already_excluded(es, new),
{
    if exists|i: int| 0 <= i < es.len() && es[i] == new {
        let i = choose|i: int| 0 <= i < es.len() && es[i] == new;
        assert(entry_within(new, entry_lo(es[i]), entry_hi(es[i])));
    }
}

/// `p` is excluded by an entry of `es` that lies within `lo..=hi`.
pub open spec fn covered_within(es: Seq<ExclusionEntry>, lo: int, hi: int, p: int) -> bool {
    exists|i: int|
        0 <= i < es.len() && entry_within(es[i], lo, hi) && #[trigger] entry_covers(es[i], p)
}

/// When no entry overlaps the merge block, appending `new` adds it through
/// that block.
proof fn lemma_appended_through(es: Seq<ExclusionEntry>, new: ExclusionEntry, lo: int, hi: int)
    requires
        merge_block(es, new, lo, hi),
        forall|j: int| 0 <= j < es.len() ==> !overlaps_block(#[trigger] es[j], lo, hi),
    ensures
        merged_in(es, new, es.push(new)),
{
    let out = es.push(new);
    assert forall|j: int| 0 <= j < out.len() && j != es.len() implies !overlaps_block(
        #[trigger] out[j],
        lo,
        hi,
    ) by {
        assert(out[j] == es[j]);
    }
    assert(added_through(es, new, out, lo, hi, es.len() as int));
}

/// Whether `e` overlaps the block `lo..=hi`.
const fn overlaps_bounds(e: ExclusionEntry, lo: u16, hi: u16) -> (r: bool)
    ensures
        r == overlaps_block(e, lo as int, hi as int),
{
    let (a, b) = entry_bounds(e);
    a <= b && lo <= hi && a <= hi && lo <= b
}

/// One growth step of a merge block: `cur_lo..=cur_hi` (which contains
/// `lo..=hi`) grows to its hull with an entry `e` of `es` that overlaps it.
/// The grown block still lies within every closed block around `lo..=hi`,
/// and each of its ports outside `lo..=hi` is excluded by an entry lying
/// within it.
proof fn lemma_grow_block(
    es: Seq<ExclusionEntry>,
    lo: int,
    hi: int,
    cur_lo: int,
    cur_hi: int,
    i: int,
    new_lo: int,
    new_hi: int,
)
    requires
        all_valid(es),
        0 <= i < es.len(),
        lo <= hi,
        cur_lo <= lo,
        hi <= cur_hi,
        entry_lo(es[i]) <= cur_hi,
        cur_lo <= entry_hi(es[i]),
        new_lo == if entry_lo(es[i]) < cur_lo {
            entry_lo(es[i])
        } else {
            cur_lo
        },
        new_hi == if entry_hi(es[i]) > cur_hi {
            entry_hi(es[i])
        } else {
            cur_hi
        },
        forall|a: int, b: int|
            a <= lo && hi <= b && #[trigger] closed_block(es, a, b) ==> a <= cur_lo && cur_hi <= b,
        forall|p: int|
            cur_lo <= p <= cur_hi && !(lo <= p <= hi) ==> #[trigger] covered_within(
                es,
                cur_lo,
                cur_hi,
                p,
            ),
    ensures
        entry_within(es[i], new_lo, new_hi),
        forall|a: int, b: int|
            a <= lo && hi <= b && #[trigger] closed_block(es, a, b) ==> a <= new_lo && new_hi <= b,
        forall|p: int|
            new_lo <= p <= new_hi && !(lo <= p <= hi) ==> #[trigger] covered_within(
                es,
                new_lo,
                new_hi,
                p,
            ),
{
    assert(entry_valid(es[i]));
    // Every closed block around `lo..=hi` contains the current block, so it
    // overlaps this entry and contains it too.
    assert forall|a: int, b: int|
        a <= lo && hi <= b && #[trigger] closed_block(es, a, b) implies a <= new_lo && new_hi
        <= b by {
        assert(overlaps_block(es[i], a, b));
    }
    // The grown block is the union of the current block and this entry.
    assert forall|p: int|
        new_lo <= p <= new_hi && !(lo <= p <= hi) implies #[trigger] covered_within(
        es,
        new_lo,
        new_hi,
        p,
    ) by {
        if cur_lo <= p <= cur_hi {
            assert(covered_within(es, cur_lo, cur_hi, p));
            let w = choose|w: int|
                0 <= w < es.len() && entry_within(es[w], cur_lo, cur_hi) && #[trigger] entry_covers(
                    es[w],
                    p,
                );
            assert(entry_within(es[w], new_lo, new_hi));
        } else {
            assert(entry_covers(es[i], p));
        }
    }
}

/// The merge block of the ports `lo..=hi` in the valid list `es`: grows the
/// block by every overlapping entry until no entry overlapping it sticks
/// out. Every port of the block is in `lo..=hi` or excluded by an entry
/// lying within the block.
fn merge_block_of(es: &[ExclusionEntry], lo: u16, hi: u16) -> (r: (u16, u16))
    requires
        1 <= lo <= hi,
        all_valid(es@),
    ensures
        1 <= r.0 <= lo,
        hi <= r.1,
        closed_block(es@, r.0 as int, r.1 as int),
        forall|a: int, b: int|
            a <= lo && hi <= b && #[trigger] closed_block(es@, a, b) ==> a <= r.0 && r.1 <= b,
        forall|p: int|
            r.0 <= p <= r.1 && !(lo <= p <= hi) ==> #[trigger] covered_within(
                es@,
                r.0 as int,
                r.1 as int,
                p,
            ),
{
    let n = es.len();
    let mut cur_lo = lo;
    let mut cur_hi = hi;
    loop
        invariant
            n == es@.len(),
            all_valid(es@),
            1 <= cur_lo <= lo <= hi <= cur_hi,
            forall|a: int, b: int|
                a <= lo && hi <= b && #[trigger] closed_block(es@, a, b) ==> a <= cur_lo && cur_hi
                    <= b,
            forall|p: int|
                cur_lo <= p <= cur_hi && !(lo <= p <= hi) ==> #[trigger] covered_within(
                    es@,
                    cur_lo as int,
                    cur_hi as int,
                    p,
                ),
        decreases cur_lo as int + (65535 - cur_hi as int),
    {
        let ghost start_lo = cur_lo;
        let ghost start_hi = cur_hi;
        let mut grew = false;
        let mut i: usize = 0;
        while i < n
            invariant
                i <= n == es@.len(),
                all_valid(es@),
                1 <= cur_lo <= start_lo <= lo <= hi <= start_hi <= cur_hi,
                forall|a: int, b: int|
                    a <= lo && hi <= b && #[trigger] closed_block(es@, a, b) ==> a <= cur_lo
                        && cur_hi <= b,
                forall|p: int|
                    cur_lo <= p <= cur_hi && !(lo <= p <= hi) ==> #[trigger] covered_within(
                        es@,
                        cur_lo as int,
                        cur_hi as int,
                        p,
                    ),
                grew == (cur_lo < start_lo || start_hi < cur_hi),
                !grew ==> forall|j: int|
                    0 <= j < i && #[trigger] overlaps_block(es@[j], cur_lo as int, cur_hi as int)
                        ==> entry_within(es@[j], cur_lo as int, cur_hi as int),
            decreases n - i,
        {
            let (a, b) = entry_bounds(es[i]);
            if a <= cur_hi && cur_lo <= b {
                let new_lo = if a < cur_lo {
                    a
                } else {
                    cur_lo
                };
                let new_hi = if b > cur_hi {
                    b
                } else {
                    cur_hi
                };
                proof {
                    lemma_grow_block(
                        es@,
                        lo as int,
                        hi as int,
                        cur_lo as int,
                        cur_hi as int,
                        i as int,
                        new_lo as int,
                        new_hi as int,
                    );
                }
                if new_lo < cur_lo || new_hi > cur_hi {
                    grew = true;
                }
                cur_lo = new_lo;
                cur_hi = new_hi;
            }
            i += 1;
        }
        if !grew {
            return (cur_lo, cur_hi);
        }
    }
}

/// Copies `es` into a new vector.
fn copy_entries(es: &[ExclusionEntry]) -> (out: Vec<ExclusionEntry>)
    ensures
        out@ == es@,
{
    let mut out: Vec<ExclusionEntry> = Vec::new();
    extend_from(&mut out, es, 0, es.len());
    assert(out@ =~= es@);
    out
}

/// `es` with its entries overlapping the block `lo..=hi` replaced by one
/// range for the block at position `f` (the first of them).
fn replace_block(es: &[ExclusionEntry], f: usize, lo: u16, hi: u16) -> (out: Vec<
    ExclusionEntry,
>)
    requires
        f < es@.len(),
        all_valid(es@),
        1 <= lo <= hi,
        closed_block(es@, lo as int, hi as int),
    ensures
        out@ == es@.take(f as int).push(ExclusionEntry::Range { start: lo, end: hi }) + es@.skip(
            f + 1,
        ).filter(outside_block(lo as int, hi as int)),
        all_valid(out@),
        forall|j: int| f < j < out@.len() ==> !overlaps_block(#[trigger] out@[j], lo as int, hi as int),
        forall|p: int|
            #[trigger] covers(out@, p) <==> (covers(es@.take(f as int), p) || (lo <= p <= hi)
                || covers(es@.skip(f + 1), p)),
{
    let n = es.len();
    let merged = ExclusionEntry::Range { start: lo, end: hi };
    let ghost keep = outside_block(lo as int, hi as int);
    let mut out: Vec<ExclusionEntry> = Vec::new();
    extend_from(&mut out, es, 0, f);
    out.push(merged);
    proof {
        reveal(Seq::filter);
        assert(es@.subrange(0, f as int) =~= es@.take(f as int));
        assert(es@.subrange(f + 1, f + 1).filter(keep) =~= Seq::<ExclusionEntry>::empty());
        assert(out@ =~= es@.take(f as int).push(merged) + es@.subrange(f + 1, f + 1).filter(keep));
        assert forall|p: int|
            #[trigger] covers(out@, p) <==> (covers(es@.take(f as int), p) || (lo <= p <= hi)
                || covers(es@.subrange(f + 1, f + 1), p)) by {
            lemma_covers_push(es@.take(f as int), merged, p);
        }
        assert forall|i: int| 0 <= i < out@.len() implies entry_valid(#[trigger] out@[i]) by {
            if i < f {
                assert(out@[i] == es@[i]);
            }
        }
    }
    let mut k: usize = f + 1;
    while k < n
        invariant
            f < k <= n == es@.len(),
            all_valid(es@),
            1 <= lo <= hi,
            merged == (ExclusionEntry::Range { start: lo, end: hi }),
            keep == outside_block(lo as int, hi as int),
            closed_block(es@, lo as int, hi as int),
            out@ == es@.take(f as int).push(merged) + es@.subrange(f + 1, k as int).filter(keep),
            all_valid(out@),
            forall|j: int|
                f < j < out@.len() ==> !overlaps_block(#[trigger] out@[j], lo as int, hi as int),
            forall|p: int|
                #[trigger] covers(out@, p) <==> (covers(es@.take(f as int), p) || (lo <= p <= hi)
                    || covers(es@.subrange(f + 1, k as int), p)),
        decreases n - k,
    {
        let e = es[k];
        let kept = !overlaps_bounds(e, lo, hi);
        let ghost before = out@;
        if kept {
            out.push(e);
        }
        proof {
            reveal(Seq::filter);
            let s = es@.subrange(f + 1, k + 1);
            assert(s.drop_last() =~= es@.subrange(f + 1, k as int));
            assert(s.last() == e);
            assert(kept == keep(e));
            assert(entry_valid(es@[k as int]));
            assert(out@ =~= es@.take(f as int).push(merged) + s.filter(keep));
            if !kept {
                // A dropped entry overlaps the block, so it lies within it.
                assert(entry_within(es@[k as int], lo as int, hi as int));
            }
            assert forall|p: int|
                #[trigger] covers(out@, p) <==> (covers(es@.take(f as int), p) || (lo <= p <= hi)
                    || covers(s, p)) by {
                lemma_covers_push(es@.subrange(f + 1, k as int), e, p);
                assert(s =~= es@.subrange(f + 1, k as int).push(e));
                if kept {
                    lemma_covers_push(before, e, p);
                }
            }
        }
        k += 1;
    }
    proof {
        assert(es@.subrange(f + 1, n as int) =~= es@.skip(f + 1));
    }
    out
}

/// Coverage after `replace_block` through the merge block of `new`: the
/// block is `new` plus entries lying within it, and the replaced entries lie
/// within the block.
proof fn lemma_replace_block_coverage(
    es: Seq<ExclusionEntry>,
    new: ExclusionEntry,
    out: Seq<ExclusionEntry>,
    f: int,
    lo: int,
    hi: int,
)
    requires
        0 <= f < es.len(),
        entry_within(new, lo, hi),
        entry_within(es[f], lo, hi),
        forall|p: int|
            lo <= p <= hi && !entry_covers(new, p) ==> #[trigger] covered_within(es, lo, hi, p),
        forall|p: int|
            #[trigger] covers(out, p) <==> (covers(es.take(f), p) || (lo <= p <= hi) || covers(
                es.skip(f + 1),
                p,
            )),
    ensures
        forall|p: int| #[trigger] covers(out, p) <==> covers(es, p) || entry_covers(new, p),
{
    let a = es.take(f);
    let tail = es.skip(f + 1);
    assert(es =~= a.push(es[f]) + tail);
    assert forall|p: int| #[trigger] covers(out, p) <==> covers(es, p) || entry_covers(new, p) by {
        lemma_covers_add(a.push(es[f]), tail, p);
        lemma_covers_push(a, es[f], p);
        if lo <= p <= hi && !entry_covers(new, p) {
            assert(covered_within(es, lo, hi, p));
            let w = choose|w: int|
                0 <= w < es.len() && entry_within(es[w], lo, hi) && #[trigger] entry_covers(
                    es[w],
                    p,
                );
            assert(covers(es, p));
        }
    }
}

/// Adds `new` to the valid list `es` when no single entry already excludes
/// all of it: through its merge block (`added_through`). The list changes.
fn add_uncovered(es: &[ExclusionEntry], new: ExclusionEntry) -> (out: Vec<ExclusionEntry>)
    requires
        all_valid(es@),
        entry_valid(new),
        !already_excluded(es@, new),
    ensures
        all_valid(out@),
        forall|p: int| #[trigger] covers(out@, p) <==> covers(es@, p) || entry_covers(new, p),
        out@ != es@,
        merged_in(es@, new, out@),
{
    let n = es.len();
    let (nlo, nhi) = entry_bounds(new);
    let (lo, hi) = merge_block_of(es, nlo, nhi);
    proof {
        assert(merge_block(es@, new, lo as int, hi as int));
    }
    let mut f: usize = 0;
    while f < n && !overlaps_bounds(es[f], lo, hi)
        invariant
            f <= n == es@.len(),
            all_valid(es@),
            entry_valid(new),
            1 <= lo <= hi,
            !already_excluded(es@, new),
            merge_block(es@, new, lo as int, hi as int),
            forall|p: int|
                lo <= p <= hi && !entry_covers(new, p) ==> #[trigger] covered_within(
                    es@,
                    lo as int,
                    hi as int,
                    p,
                ),
            forall|j: int| 0 <= j < f ==> !overlaps_block(#[trigger] es@[j], lo as int, hi as int),
        decreases n - f,
    {
        f += 1;
    }
    if f == n {
        // Nothing overlaps the block: append `new`.
        let mut out = copy_entries(es);
        out.push(new);
        proof {
            assert forall|p: int| #[trigger]
                covers(out@, p) <==> covers(es@, p) || entry_covers(new, p) by {
                lemma_covers_push(es@, new, p);
            }
            lemma_appended_through(es@, new, lo as int, hi as int);
            assert(out@.len() != es@.len());
        }
        return out;
    }
    // Replace the overlapping entries by one range for the block, at the
    // position of the first of them.
    let out = replace_block(es, f, lo, hi);
    let ghost merged = ExclusionEntry::Range { start: lo, end: hi };
    proof {
        let a = es@.take(f as int);
        let tail = es@.skip(f + 1).filter(outside_block(lo as int, hi as int));
        assert(entry_within(es@[f as int], lo as int, hi as int));
        assert(out@[f as int] == merged) by {
            assert(out@[f as int] == (a.push(merged) + tail)[f as int]);
        }
        assert forall|j: int| 0 <= j < out@.len() && j != f implies !overlaps_block(
            #[trigger] out@[j],
            lo as int,
            hi as int,
        ) by {
            if j < f {
                assert(out@[j] == es@[j]);
            }
        }
        assert(added_through(es@, new, out@, lo as int, hi as int, f as int));
        lemma_replace_block_coverage(es@, new, out@, f as int, lo as int, hi as int);
        // The list changed: otherwise the entry at `f` would be the block,
        // which contains `new`.
        if out@ == es@ {
            assert(es@[f as int] == merged);
            assert(entry_within(new, entry_lo(es@[f as int]), entry_hi(es@[f as int])));
        }
    }
    out
}

/// Adds the exclusion `new` to the list `es` (`trop exclude`), reporting
/// whether the list changed.
///
/// - The result excludes exactly the ports `es` excluded plus the ports of
///   `new`: nothing is lost and nothing extra is added.
/// - If one entry already excludes every port of `new` (in particular, an
///   identical entry), the list is returned unchanged.
/// - Otherwise `new` is merged through its merge block (`added_through`):
///   entries that overlap it (directly, or through other merged entries)
///   become one range at the position of the first of them, and every other
///   entry keeps its content and order. Adjacency is not overlap: `5010`
///   next to `5000..5009` is appended.
/// - The flag is `true` exactly when the returned list differs from `es`.
///
/// An invalid entry (in `es`, or `new` at index `es.len()`) is reported as
/// the first invalid entry of `es` followed by `new`; nothing is added.
pub fn add_exclusion(es: &[ExclusionEntry], new: ExclusionEntry) -> (r: Result<
    (Vec<ExclusionEntry>, bool),
    (usize, ExclusionError),
>)
    ensures
        r is Ok <==> all_valid(es@.push(new)),
        r matches Err((i, err)) ==> first_invalid(es@.push(new), i as int, err),
        r matches Ok((out, changed)) ==> all_valid(out@),
        r matches Ok((out, changed)) ==> forall|p: int| #[trigger]
            covers(out@, p) <==> covers(es@, p) || entry_covers(new, p),
        r matches Ok((out, changed)) ==> changed == (out@ != es@),
        // The list changes exactly when no single entry already excludes
        // every port of `new`.
        r matches Ok((out, changed)) ==> changed == !already_excluded(es@, new),
        r matches Ok((out, changed)) ==> (already_excluded(es@, new) ==> out@ == es@),
        r matches Ok((out, changed)) ==> ((exists|i: int| 0 <= i < es@.len() && es@[i] == new)
            ==> out@ == es@ && !changed),
        r matches Ok((out, changed)) ==> (!already_excluded(es@, new) ==> merged_in(
            es@,
            new,
            out@,
        )),
{
    let n = es.len();
    if let Err((i, err)) = validate_entries(es) {
        proof {
            assert(es@.push(new).take(i as int) =~= es@.take(i as int));
            assert(es@.push(new)[i as int] == es@[i as int]);
        }
        return Err((i, err));
    }
    if let Err(err) = validate_entry(new) {
        proof {
            assert(es@.push(new).take(n as int) =~= es@);
            assert(es@.push(new)[n as int] == new);
        }
        return Err((n, err));
    }
    proof {
        assert forall|i: int| 0 <= i < es@.push(new).len() implies entry_valid(
            #[trigger] es@.push(new)[i],
        ) by {
            if i < n {
                assert(es@.push(new)[i] == es@[i]);
            }
        }
        lemma_identical_is_already_excluded(es@, new);
    }
    let (nlo, nhi) = entry_bounds(new);
    // One entry already excludes every port of `new`: unchanged.
    let mut k: usize = 0;
    while k < n
        invariant
            k <= n == es@.len(),
            all_valid(es@),
            all_valid(es@.push(new)),
            entry_valid(new),
            nlo == entry_lo(new),
            nhi == entry_hi(new),
            forall|j: int|
                0 <= j < k ==> !entry_within(new, entry_lo(#[trigger] es@[j]), entry_hi(es@[j])),
        decreases n - k,
    {
        let (a, b) = entry_bounds(es[k]);
        if a <= nlo && nhi <= b {
            let out = copy_entries(es);
            proof {
                assert(already_excluded(es@, new));
                assert forall|p: int| #[trigger]
                    covers(out@, p) <==> covers(es@, p) || entry_covers(new, p) by {
                    if entry_covers(new, p) {
                        assert(entry_covers(es@[k as int], p));
                    }
                }
            }
            return Ok((out, false));
        }
        k += 1;
    }
    proof {
        if already_excluded(es@, new) {
            let i = choose|i: int|
                0 <= i < es@.len() && entry_within(
                    new,
                    entry_lo(#[trigger] es@[i]),
                    entry_hi(es@[i]),
                );
            assert(!entry_within(new, entry_lo(es@[i]), entry_hi(es@[i])));
        }
    }
    let out = add_uncovered(es, new);
    Ok((out, true))
}

/// The single-port entries for `ports`, in order.
pub open spec fn singles(ports: Seq<u16>) -> Seq<ExclusionEntry> {
    ports.map_values(|q: u16| ExclusionEntry::Single(q))
}

/// The valid ports of `ports` that `es` does not exclude.
pub open spec fn newly_excluded(es: Seq<ExclusionEntry>, ports: Seq<u16>) -> Set<u16> {
    ports.to_set().filter(|q: u16| q != 0 && !covers(es, q as int))
}

/// `fresh` lists the ports newly excluded by adding `ports` to `es`, each
/// once.
pub open spec fn fresh_ports(es: Seq<ExclusionEntry>, ports: Seq<u16>, fresh: Seq<u16>) -> bool {
    &&& fresh.no_duplicates()
    &&& forall|q: u16| #[trigger] fresh.contains(q) <==> newly_excluded(es, ports).contains(q)
}

/// The ports that adding `ports` to `es` newly excludes, in the order of
/// their first occurrence in `ports`: walking `ports` from the front, a port
/// is taken when it is valid (not 0), `es` does not exclude it, and it was
/// not taken before.
pub open spec fn fresh_in_order(es: Seq<ExclusionEntry>, ports: Seq<u16>) -> Seq<u16>
    decreases ports.len(),
{
    if ports.len() == 0 {
        Seq::empty()
    } else {
        let earlier = fresh_in_order(es, ports.drop_last());
        let q = ports.last();
        if q != 0 && !covers(es, q as int) && !earlier.contains(q) {
            earlier.push(q)
        } else {
            earlier
        }
    }
}

/// Whether some entry of `es` (any list, not necessarily canonical) excludes
/// `p`.
fn list_covers(es: &[ExclusionEntry], p: u16) -> (b: bool)
    ensures
        b == covers(es@, p as int),
{
    let mut i: usize = 0;
    while i < es.len()
        invariant
            i <= es@.len(),
            forall|j: int| 0 <= j < i ==> !entry_covers(#[trigger] es@[j], p as int),
        decreases es@.len() - i,
    {
        let (lo, hi) = entry_bounds(es[i]);
        if lo <= p && p <= hi {
            assert(entry_covers(es@[i as int], p as int));
            return true;
        }
        i += 1;
    }
    false
}

/// Membership after a push.
proof fn lemma_push_contains<A>(s: Seq<A>, x: A)
    ensures
        forall|q: A| #[trigger] s.push(x).contains(q) <==> (s.contains(q) || q == x),
{
    assert forall|q: A| #[trigger] s.push(x).contains(q) <==> (s.contains(q) || q == x) by {
        if s.push(x).contains(q) {
            let j = choose|j: int| 0 <= j < s.push(x).len() && s.push(x)[j] == q;
            if j < s.len() {
                assert(s[j] == q);
            }
        }
        if s.contains(q) {
            let j = choose|j: int| 0 <= j < s.len() && s[j] == q;
            assert(s.push(x)[j] == q);
        }
        if q == x {
            assert(s.push(x)[s.len() as int] == q);
        }
    }
}

/// Appending a port not yet in `fresh` keeps it free of duplicates.
proof fn lemma_push_fresh(fresh: Seq<u16>, p: u16)
    requires
        fresh.no_duplicates(),
        !fresh.contains(p),
    ensures
        fresh.push(p).no_duplicates(),
        singles(fresh.push(p)) == singles(fresh).push(ExclusionEntry::Single(p)),
{
    let s = fresh.push(p);
    assert forall|x: int, y: int| 0 <= x < s.len() && 0 <= y < s.len() && x != y implies s[x]
        != s[y] by {
        if x == fresh.len() {
            assert(fresh.contains(s[y]));
        } else if y == fresh.len() {
            assert(fresh.contains(s[x]));
        }
    }
    assert(singles(fresh.push(p)) =~= singles(fresh).push(ExclusionEntry::Single(p)));
}

/// Adds single-port exclusions for `ports` (`trop scan --autoexclude`),
/// returning the new list and the number of ports newly excluded.
///
/// The result is `es` followed by one `Single` for each valid port of
/// `ports` that `es` does not already exclude, each once, in the order of
/// their first occurrence in `ports` (`fresh_in_order`): it excludes exactly
/// the ports `es` excluded plus every valid port of `ports`. The count is the
/// number of entries appended, which is the number of distinct ports newly
/// excluded. Port 0 (never a valid port) is ignored.
pub fn add_ports(es: &[ExclusionEntry], ports: &[u16]) -> (r: (Vec<ExclusionEntry>, usize))
    ensures
        r.0@ == es@ + singles(fresh_in_order(es@, ports@)),
        fresh_ports(es@, ports@, fresh_in_order(es@, ports@)),
        r.1 == r.0@.len() - es@.len(),
        r.1 == newly_excluded(es@, ports@).len(),
        forall|p: int| #[trigger]
            covers(r.0@, p) <==> covers(es@, p) || (1 <= p <= 65535 && ports@.contains(p as u16)),
        all_valid(es@) ==> all_valid(r.0@),
{
    let mut out = copy_entries(es);
    let mut added: usize = 0;
    let ghost mut fresh: Seq<u16> = Seq::empty();
    let mut i: usize = 0;
    while i < ports.len()
        invariant
            i <= ports@.len(),
            added == fresh.len(),
            added <= i,
            out@ == es@ + singles(fresh),
            fresh == fresh_in_order(es@, ports@.take(i as int)),
            fresh.no_duplicates(),
            forall|q: u16|
                #[trigger] fresh.contains(q) <==> (q != 0 && ports@.take(i as int).contains(q)
                    && !covers(es@, q as int)),
            forall|p: int|
                #[trigger] covers(out@, p) <==> covers(es@, p) || (1 <= p <= 65535 && ports@.take(
                    i as int,
                ).contains(p as u16)),
            all_valid(es@) ==> all_valid(out@),
        decreases ports@.len() - i,
    {
        let p = ports[i];
        let ghost before = out@;
        let is_new = p != 0 && !list_covers(out.as_slice(), p);
        if is_new {
            out.push(ExclusionEntry::Single(p));
            added += 1;
        }
        proof {
            let seen = ports@.take(i as int);
            assert(ports@.take(i as int + 1) =~= seen.push(p));
            lemma_push_contains(seen, p);
            // One step of `fresh_in_order`: `p` is taken exactly when it is
            // new.
            assert(ports@.take(i as int + 1).drop_last() =~= seen);
            assert(ports@.take(i as int + 1).last() == p);
            assert(is_new == (p != 0 && !covers(es@, p as int) && !fresh.contains(p))) by {
                if p != 0 {
                    lemma_covers_add(es@, singles(fresh), p as int);
                    if covers(before, p as int) && !covers(es@, p as int) {
                        assert(seen.contains(p));
                        assert(fresh.contains(p));
                    }
                    if fresh.contains(p) {
                        assert(covers(before, p as int));
                    }
                }
            }
            if is_new {
                assert(!fresh.contains(p)) by {
                    if fresh.contains(p) {
                        assert(covers(before, p as int));
                    }
                }
                assert(!covers(es@, p as int)) by {
                    lemma_covers_add(es@, singles(fresh), p as int);
                }
                lemma_push_fresh(fresh, p);
                lemma_push_contains(fresh, p);
                assert(out@ =~= es@ + singles(fresh.push(p)));
                assert forall|x: int| #[trigger] covers(out@, x) <==> covers(before, x) || x == p by {
                    lemma_covers_push(before, ExclusionEntry::Single(p), x);
                }
                fresh = fresh.push(p);
            } else if p != 0 {
                // `p` was already excluded, by `es` or by an earlier port.
                assert(covers(before, p as int));
            }
        }
        i += 1;
    }
    proof {
        assert(ports@.take(i as int) =~= ports@);
        fresh.to_set_ensures();
        ports@.to_set_ensures();
        // The appended ports are exactly the newly excluded ports.
        assert(fresh.to_set() =~= newly_excluded(es@, ports@));
        fresh.unique_seq_to_set();
        assert(fresh_ports(es@, ports@, fresh));
    }
    (out, added)
}

// ---------------------------------------------------------------------------
// The reserved-port guard of `trop exclude`
// ---------------------------------------------------------------------------
/// The oracle of `first_reserved_port` may be consulted at the valid ports
/// `e` names, and the requirement grants nothing more.
pub open spec fn callable_on_entry<E, F: Fn(u16) -> Result<bool, E>>(
    is_reserved: F,
    e: ExclusionEntry,
) -> bool {
    forall|q: u16| 1 <= q && entry_covers(e, q as int) ==> #[trigger] is_reserved.requires((q,))
}

/// Checks whether a valid port of `e` is reserved, and returns the lowest
/// one.
///
/// The precondition grants the oracle's own precondition only at the valid
/// ports (`1..=65535`) that `e` names, and a generic oracle can be called
/// only where its precondition is known to hold, so the oracle is consulted
/// at no other port (a call at any other port fails verification). Errors
/// are returned, never treated as "not reserved". The cursor never
/// overflows, including for entries that end at 65535.
///
/// The body scans from the lowest such port upward, consulting each port
/// once and stopping at the first port reported reserved or at the first
/// failure. That order is not part of the specification (closure calls
/// leave no trace in it); it is pinned by the glue's unit tests.
pub fn first_reserved_port<E, F: Fn(u16) -> Result<bool, E>>(
    e: ExclusionEntry,
    is_reserved: &F,
) -> (r: Result<Option<u16>, super::allocation::ScanFailure<E>>)
    requires
        callable_on_entry(*is_reserved, e),
    ensures
        r matches Ok(None) ==> forall|q: u16|
            1 <= q && entry_covers(e, q as int) ==> #[trigger] super::allocation::answered(
                *is_reserved,
                q,
                false,
            ),
        r matches Ok(Some(p)) ==> {
            &&& 1 <= p && entry_covers(e, p as int)
            &&& super::allocation::answered(*is_reserved, p, true)
            &&& forall|q: u16|
                1 <= q < p && entry_covers(e, q as int) ==> #[trigger] super::allocation::answered(
                    *is_reserved,
                    q,
                    false,
                )
        },
        r matches Err(failure) ==> {
            &&& 1 <= failure.port && entry_covers(e, failure.port as int)
            &&& super::allocation::failed(*is_reserved, failure.port, failure.error)
            &&& forall|q: u16|
                1 <= q < failure.port && entry_covers(e, q as int)
                    ==> #[trigger] super::allocation::answered(*is_reserved, q, false)
        },
{
    let (lo, hi) = entry_bounds(e);
    let start = if lo == 0 {
        1
    } else {
        lo
    };
    if start > hi {
        return Ok(None);
    }
    let mut p = start;
    loop
        invariant
            1 <= start <= p <= hi,
            start as int == if entry_lo(e) == 0 {
                1
            } else {
                entry_lo(e)
            },
            hi == entry_hi(e),
            callable_on_entry(*is_reserved, e),
            forall|q: u16|
                start <= q < p ==> #[trigger] super::allocation::answered(*is_reserved, q, false),
        decreases hi - p,
    {
        match is_reserved(p) {
            Err(error) => {
                return Err(super::allocation::ScanFailure { port: p, error });
            },
            Ok(true) => {
                return Ok(Some(p));
            },
            Ok(false) => {},
        }
        if p == hi {
            return Ok(None);
        }
        p += 1;
    }
}

// ===========================================================================
// `scan --autoexclude`: occupied ports that are not reserved
// ===========================================================================

/// The occupied ports `scan --autoexclude` may exclude: those not reserved.
pub open spec fn not_reserved(reserved: Seq<u16>) -> spec_fn(u16) -> bool {
    |p: u16| !reserved.contains(p)
}

/// The occupied ports that are not reserved, in their original order
/// (duplicates kept). No reserved port is ever returned, so
/// `scan --autoexclude` never excludes a reserved port, and every occupied
/// unreserved port is returned.
pub fn unreserved_ports(occupied: &[u16], reserved: &[u16]) -> (r: Vec<u16>)
    ensures
        r@ == occupied@.filter(not_reserved(reserved@)),
        forall|i: int| 0 <= i < r@.len() ==> !reserved@.contains(#[trigger] r@[i]),
        forall|i: int|
            0 <= i < occupied@.len() && !reserved@.contains(occupied@[i]) ==> #[trigger] r@.contains(
                occupied@[i],
            ),
{
    let ghost keep = not_reserved(reserved@);
    let mut r: Vec<u16> = Vec::new();
    let mut i: usize = 0;
    proof {
        reveal(Seq::filter);
        assert(occupied@.subrange(0, 0) =~= Seq::<u16>::empty());
    }
    while i < occupied.len()
        invariant
            i <= occupied@.len(),
            keep == not_reserved(reserved@),
            r@ == occupied@.subrange(0, i as int).filter(keep),
        decreases occupied@.len() - i,
    {
        let p = occupied[i];
        let mut j: usize = 0;
        let mut found = false;
        while j < reserved.len()
            invariant
                j <= reserved@.len(),
                found == exists|k: int| 0 <= k < j && reserved@[k] == p,
            decreases reserved@.len() - j,
        {
            if reserved[j] == p {
                found = true;
            }
            j += 1;
        }
        proof {
            reveal(Seq::filter);
            let prefix = occupied@.subrange(0, i as int + 1);
            assert(prefix.drop_last() =~= occupied@.subrange(0, i as int));
            assert(prefix.last() == p);
            assert(found == reserved@.contains(p));
        }
        if !found {
            r.push(p);
        }
        i += 1;
    }
    proof {
        assert(occupied@.subrange(0, occupied@.len() as int) =~= occupied@);
        assert forall|k: int| 0 <= k < r@.len() implies !reserved@.contains(#[trigger] r@[k]) by {
            occupied@.lemma_filter_pred(keep, k);
        }
        assert forall|k: int|
            0 <= k < occupied@.len() && !reserved@.contains(occupied@[k]) implies #[trigger] r@.contains(
            occupied@[k],
        ) by {
            occupied@.lemma_filter_contains(keep, k);
        }
    }
    r
}

} // verus!
