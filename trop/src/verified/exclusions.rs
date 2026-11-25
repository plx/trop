//! Verified kernel: exclusion entries, the canonical excluded-port set, and
//! compaction.
//!
//! Production glue: `crate::port::exclusions` (`ExclusionManager`, list
//! validation and compaction), `crate::config::validator`
//! (`excluded_ports` validation), and `crate::config::schema`
//! (`PortExclusion` text parsing).

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

/// Builds the canonical run list of a valid exclusion list.
fn canonical_runs(es: &[ExclusionEntry]) -> (r: Result<
    Vec<ExclusionEntry>,
    (usize, ExclusionError),
>)
    ensures
        r is Ok <==> all_valid(es@),
        r matches Ok(out) ==> is_compaction(out@, es@),
        r matches Err((i, err)) ==> first_invalid(es@, i as int, err),
{
    let mut runs: Vec<ExclusionEntry> = Vec::new();
    let mut i: usize = 0;
    while i < es.len()
        invariant
            i <= es@.len(),
            canonical(runs@),
            all_valid(es@.take(i as int)),
            same_coverage(runs@, es@.take(i as int)),
        decreases es@.len() - i,
    {
        let e = es[i];
        if let Err(err) = validate_entry(e) {
            return Err((i, err));
        }
        let (lo, hi) = entry_bounds(e);
        let next = insert_run(runs.as_slice(), lo, hi);
        proof {
            assert(es@.take(i as int + 1) =~= es@.take(i as int).push(e));
            assert forall|p: int| covers(next@, p) == covers(es@.take(i as int + 1), p) by {
                lemma_covers_push(es@.take(i as int), e, p);
            }
        }
        runs = next;
        i += 1;
    }
    assert(es@.take(i as int) =~= es@);
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
    // Copies the run list entry by entry (a derived `clone` could not be
    // checked against the type invariant).
    fn clone(&self) -> Self {
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

/// `s` is the decimal text of the `u16` value `v` in the syntax that
/// `u16::from_str` accepts: an optional `+`, then one or more ASCII digits
/// (leading zeros allowed) whose value is at most 65535. No whitespace, no
/// `-`.
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

/// Parses a `u16` with exactly the syntax and range of `u16::from_str`
/// (for any byte input).
pub fn parse_u16(s: &[u8]) -> (r: Option<u16>)
    ensures
        r matches Some(v) ==> is_u16_text(s@, v as int),
        r is None ==> forall|v: int| !is_u16_text(s@, v),
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
    let mut value: u16 = 0;
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
        // On overflow the digits read so far already exceed 65535, and more
        // digits only increase the value.
        let Some(shifted) = value.checked_mul(10) else {
            proof {
                lemma_decimal_value_nonneg(next.drop_last());
                if all_digits(d) {
                    lemma_decimal_value_monotone(d, i - start + 1);
                }
            }
            return None;
        };
        let Some(next_value) = shifted.checked_add(u16::from(digit)) else {
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
    Some(value)
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

} // verus!
