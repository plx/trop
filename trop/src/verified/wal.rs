//! Verified kernel: enabling WAL journal mode with a bounded retry.
//!
//! `enable_wal` drives `PRAGMA journal_mode = WAL` through four oracles: the
//! clock (nanoseconds since the start), `PRAGMA busy_timeout = <ms>`, the
//! journal-mode attempt itself, and a pause. It returns how the attempt
//! ended together with a ghost trace of every effect it performed, in order,
//! with the answer each oracle gave. Proved over that trace:
//!
//! - **The busy timeout is restored on every exit.** The last effect is always
//!   `busy_timeout = timeout_ms`, whatever happened before
//!   (`restored_last`).
//! - **Every attempt runs under the remaining budget.** Each attempt is
//!   immediately preceded by a successful `busy_timeout` of the remaining
//!   time (`timeout - elapsed`, saturating, in whole milliseconds) at the
//!   clock reading just before it (`attempts_guarded`).
//! - **Retry only on contention, only while time remains.** Every pause
//!   directly follows an attempt that reported lock contention and a clock
//!   reading `u < timeout`, and lasts exactly `min(timeout - u, 10 ms)`,
//!   which is at least 1 ns. Every attempt but the last reported contention
//!   (`pauses_justified`, `only_contention_retried`).
//! - **Bounded total wait.** The pauses add up to at most `timeout`
//!   (`total_pause`), and the loop terminates for every clock oracle: the
//!   elapsed time the kernel works with is the clock reading, never less than
//!   the time already paused (`clock_monotone`).
//! - **The outcome is exact** (`outcome_matches`): `Enabled` only when the
//!   final attempt reported an accepted journal mode and the restore
//!   succeeded; `NotWal(m)` when it reported any other mode `m`;
//!   `TimedOut(e)` when it reported contention `e` and the next clock reading
//!   was at or past the timeout; `Failed(e)` for any other attempt error;
//!   `BusyTimeoutFailed(e)` when setting the per-attempt busy timeout failed;
//!   `RestoreFailed(e)` only when everything else succeeded. An earlier
//!   error wins over a failed restore.
//! - `mode_accepted`: the reported mode is `wal` (ASCII case-insensitive), or
//!   `memory` for an in-memory database, which SQLite can never put in WAL
//!   mode and no other connection can share.
//!
//! Trusted: SQLite's busy/locked classification, `Instant::elapsed` and
//! `thread::sleep` in the glue. With a monotonic clock and a sleep that
//! lasts at least as long as requested (both `std` guarantees), the
//! elapsed time the kernel works with equals the clock reading, so the
//! decisions are exactly those of the historical loop.
//!
//! Production glue: `enable_wal_with_retry` in
//! `crate::database::connection`.

use vstd::prelude::*;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The longest single pause between attempts: 10 ms.
pub const PAUSE_CAP_NANOS: u64 = 10_000_000;

/// Nanoseconds per millisecond.
pub const NANOS_PER_MILLI: u128 = 1_000_000;

// ---------------------------------------------------------------------------
// Journal modes
// ---------------------------------------------------------------------------

/// `mode` is `wal`, ignoring ASCII case.
pub open spec fn is_wal_mode(mode: Seq<u8>) -> bool {
    super::text::eq_ignore_ascii_case(mode, seq![b'w', b'a', b'l'])
}

/// `mode` is `memory`, ignoring ASCII case.
pub open spec fn is_memory_mode(mode: Seq<u8>) -> bool {
    super::text::eq_ignore_ascii_case(mode, seq![b'm', b'e', b'm', b'o', b'r', b'y'])
}

/// The journal mode SQLite reported after `PRAGMA journal_mode = WAL` is
/// acceptable: WAL, or (only for an in-memory database) `memory`.
pub open spec fn mode_accepted(mode: Seq<u8>, in_memory: bool) -> bool {
    is_wal_mode(mode) || (in_memory && is_memory_mode(mode))
}

/// Whether the reported journal mode is acceptable (see `mode_accepted`).
pub fn journal_mode_accepted(mode: &[u8], in_memory: bool) -> (accepted: bool)
    ensures
        accepted == mode_accepted(mode@, in_memory),
{
    let upper = |i: usize| -> (u: u8)
        requires
            i < mode.len(),
        ensures
            u == super::text::ascii_upper(mode@[i as int]),
        { super::text::upper_byte(mode[i]) };
    let wal = mode.len() == 3 && upper(0) == b'W' && upper(1) == b'A' && upper(2) == b'L';
    let memory = mode.len() == 6 && upper(0) == b'M' && upper(1) == b'E' && upper(2) == b'M'
        && upper(3) == b'O' && upper(4) == b'R' && upper(5) == b'Y';
    proof {
        let m = mode@;
        if m.len() == 3 {
            assert(is_wal_mode(m) == (super::text::ascii_upper(m[0]) == b'W'
                && super::text::ascii_upper(m[1]) == b'A' && super::text::ascii_upper(m[2])
                == b'L'));
        }
        if m.len() == 6 {
            assert(is_memory_mode(m) == (super::text::ascii_upper(m[0]) == b'M'
                && super::text::ascii_upper(m[1]) == b'E' && super::text::ascii_upper(m[2])
                == b'M' && super::text::ascii_upper(m[3]) == b'O' && super::text::ascii_upper(m[4])
                == b'R' && super::text::ascii_upper(m[5]) == b'Y'));
        }
    }
    wal || (in_memory && memory)
}

// ---------------------------------------------------------------------------
// Time arithmetic
// ---------------------------------------------------------------------------

/// The time left before `timeout`, or zero once it has passed.
pub open spec fn remaining(elapsed: int, timeout: int) -> int {
    if elapsed < timeout {
        timeout - elapsed
    } else {
        0
    }
}

/// The busy timeout for an attempt at `elapsed`: the remaining time in whole
/// milliseconds (what `Duration::as_millis` gives).
pub open spec fn busy_millis(elapsed: int, timeout: int) -> int {
    remaining(elapsed, timeout) / NANOS_PER_MILLI as int
}

/// The pause after contention at `elapsed < timeout`: the remaining time,
/// at most 10 ms.
pub open spec fn pause_nanos(elapsed: int, timeout: int) -> int {
    if timeout - elapsed < PAUSE_CAP_NANOS as int {
        timeout - elapsed
    } else {
        PAUSE_CAP_NANOS as int
    }
}

// ---------------------------------------------------------------------------
// Oracle answers, outcomes, and the effect trace
// ---------------------------------------------------------------------------

/// What one `PRAGMA journal_mode = WAL` attempt answered.
pub enum WalAnswer<E> {
    /// The pragma ran and reported this journal mode.
    Mode(Vec<u8>),
    /// SQLite reported lock contention (busy or locked).
    Contention(E),
    /// Any other error.
    Failed(E),
}

/// How enabling WAL ended.
pub enum WalOutcome<E> {
    /// The database is in an accepted journal mode and the busy timeout was
    /// restored.
    Enabled,
    /// SQLite reported this journal mode instead of WAL.
    NotWal(Vec<u8>),
    /// Lock contention lasted until the timeout.
    TimedOut(E),
    /// The attempt failed with an error other than contention.
    Failed(E),
    /// Setting the busy timeout before an attempt failed.
    BusyTimeoutFailed(E),
    /// WAL was enabled, but restoring the busy timeout failed.
    RestoreFailed(E),
}

/// One effect `enable_wal` performed, with the answer it got.
// A ghost type: only specifications use it. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint.
#[allow(dead_code)]
pub ghost enum WalEvent<E> {
    /// The clock answered `reading`; the kernel works with the elapsed time
    /// `elapsed`: the reading, but never less than the time already paused.
    Clock { reading: u128, elapsed: int },
    /// `PRAGMA busy_timeout = millis` answered `result`.
    SetBusy { millis: int, result: Result<(), E> },
    /// `PRAGMA journal_mode = WAL` answered `answer`.
    Attempt { answer: WalAnswer<E> },
    /// A pause of `nanos` nanoseconds.
    Pause { nanos: int },
}

/// The answer recorded in `ev` is one the oracle gave for that call (a
/// clock event also records an elapsed time no less than the reading).
pub open spec fn answered<E, C, B, A>(clock: C, set_busy: B, attempt: A, ev: WalEvent<E>) -> bool where
    C: Fn() -> u128,
    B: Fn(u128) -> Result<(), E>,
    A: Fn() -> WalAnswer<E>,
 {
    match ev {
        WalEvent::Clock { reading, elapsed } => clock.ensures((), reading) && reading <= elapsed,
        WalEvent::SetBusy { millis, result } => 0 <= millis <= u128::MAX && set_busy.ensures(
            (millis as u128,),
            result,
        ),
        WalEvent::Attempt { answer } => attempt.ensures((), answer),
        WalEvent::Pause { .. } => true,
    }
}

/// Every recorded answer is one the oracle gave for that call.
#[verifier::opaque]
pub open spec fn answers_from<E, C, B, A>(clock: C, set_busy: B, attempt: A, trace: Seq<WalEvent<E>>) -> bool where
    C: Fn() -> u128,
    B: Fn(u128) -> Result<(), E>,
    A: Fn() -> WalAnswer<E>,
 {
    forall|i: int| 0 <= i < trace.len() ==> answered(clock, set_busy, attempt, #[trigger] trace[i])
}

/// The last effect restored the busy timeout to the whole timeout.
pub open spec fn restored_last<E>(trace: Seq<WalEvent<E>>, timeout: int) -> bool {
    &&& trace.len() >= 1
    &&& trace.last() is SetBusy
    &&& trace.last()->SetBusy_millis == busy_millis(0, timeout)
}

/// Every attempt directly follows a clock reading and a successful busy
/// timeout of the time remaining at that reading.
#[verifier::opaque]
pub open spec fn attempts_guarded<E>(trace: Seq<WalEvent<E>>, timeout: int) -> bool {
    forall|i: int| 0 <= i < trace.len() && #[trigger] trace[i] is Attempt ==> guarded_at(trace, i, timeout)
}

/// Position `i` directly follows a clock reading and a successful busy
/// timeout of the time remaining at that reading.
pub open spec fn guarded_at<E>(trace: Seq<WalEvent<E>>, i: int, timeout: int) -> bool {
    &&& 2 <= i
    &&& trace[i - 2] is Clock
    &&& trace[i - 1] is SetBusy
    &&& trace[i - 1]->SetBusy_millis == busy_millis(trace[i - 2]->Clock_elapsed, timeout)
    &&& trace[i - 1]->SetBusy_result is Ok
}

/// Every pause directly follows an attempt that reported contention and a
/// clock reading before the timeout, and lasts `min(timeout - elapsed,
/// 10 ms)`.
#[verifier::opaque]
pub open spec fn pauses_justified<E>(trace: Seq<WalEvent<E>>, timeout: int) -> bool {
    forall|i: int|
        0 <= i < trace.len() && #[trigger] trace[i] is Pause ==> {
            &&& 2 <= i
            &&& trace[i - 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) }
            &&& trace[i - 1] is Clock
            &&& trace[i - 1]->Clock_elapsed < timeout
            &&& trace[i]->Pause_nanos == pause_nanos(trace[i - 1]->Clock_elapsed, timeout)
        }
}

/// Only the last attempt may report anything but contention.
#[verifier::opaque]
pub open spec fn only_contention_retried<E>(trace: Seq<WalEvent<E>>) -> bool {
    forall|i: int, j: int|
        0 <= i < j < trace.len() && #[trigger] trace[i] is Attempt && #[trigger] trace[j] is Attempt
            ==> trace[i] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) }
}

/// No clock reading in `trace[i + 1..j]`.
pub open spec fn no_clock_between<E>(trace: Seq<WalEvent<E>>, i: int, j: int) -> bool {
    forall|k: int| i < k < j ==> !(#[trigger] trace[k] is Clock)
}

/// The elapsed time the kernel uses at each clock reading: the first is the
/// reading itself; every later one is the reading, raised (only if needed)
/// to the previous elapsed time plus the pauses since. A monotonic clock
/// with sleeps that last at least as long as requested never reads less,
/// so in practice the elapsed time is exactly the reading.
#[verifier::opaque]
pub open spec fn clock_consistent<E>(trace: Seq<WalEvent<E>>) -> bool {
    &&& forall|j: int|
        0 <= j < trace.len() && #[trigger] trace[j] is Clock && no_clock_between(trace, -1, j)
            ==> trace[j]->Clock_elapsed == trace[j]->Clock_reading
    &&& forall|i: int, j: int|
        0 <= i < j < trace.len() && #[trigger] trace[i] is Clock && #[trigger] trace[j] is Clock
            && no_clock_between(trace, i, j) ==> trace[j]->Clock_elapsed == max_int(
            trace[j]->Clock_reading as int,
            trace[i]->Clock_elapsed + pauses_between(trace, i, j),
        )
}

/// The larger of two integers.
pub open spec fn max_int(a: int, b: int) -> int {
    if a < b {
        b
    } else {
        a
    }
}

/// The total length of the pauses in `trace[i..j]`.
pub open spec fn pauses_between<E>(trace: Seq<WalEvent<E>>, i: int, j: int) -> int
    decreases j - i,
{
    if j <= i {
        0
    } else {
        pauses_between(trace, i, j - 1) + match trace[j - 1] {
            WalEvent::Pause { nanos } => nanos,
            _ => 0,
        }
    }
}

/// The total length of all pauses.
pub open spec fn total_pause<E>(trace: Seq<WalEvent<E>>) -> int {
    pauses_between(trace, 0, trace.len() as int)
}

/// The outcome is the one the final effects determine.
pub open spec fn outcome_matches<E>(
    trace: Seq<WalEvent<E>>,
    timeout: int,
    in_memory: bool,
    outcome: WalOutcome<E>,
) -> bool {
    let n = trace.len() as int;
    match outcome {
        WalOutcome::Enabled => {
            &&& n >= 2
            &&& trace[n - 2] matches WalEvent::Attempt { answer: WalAnswer::Mode(m) }
                && mode_accepted(m@, in_memory)
            &&& trace[n - 1] is SetBusy
            &&& trace[n - 1]->SetBusy_result is Ok
        },
        WalOutcome::NotWal(m) => {
            &&& n >= 2
            &&& trace[n - 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Mode(m) }
            &&& !mode_accepted(m@, in_memory)
        },
        WalOutcome::TimedOut(e) => {
            &&& n >= 3
            &&& trace[n - 3] == WalEvent::<E>::Attempt { answer: WalAnswer::Contention(e) }
            &&& trace[n - 2] is Clock
            &&& trace[n - 2]->Clock_elapsed >= timeout
        },
        WalOutcome::Failed(e) => {
            &&& n >= 2
            &&& trace[n - 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Failed(e) }
        },
        WalOutcome::BusyTimeoutFailed(e) => {
            &&& n >= 2
            &&& trace[n - 2] is SetBusy
            &&& trace[n - 2]->SetBusy_result == Err::<(), E>(e)
        },
        WalOutcome::RestoreFailed(e) => {
            &&& n >= 2
            &&& trace[n - 2] matches WalEvent::Attempt { answer: WalAnswer::Mode(m) }
                && mode_accepted(m@, in_memory)
            &&& trace[n - 1] is SetBusy
            &&& trace[n - 1]->SetBusy_result == Err::<(), E>(e)
        },
    }
}

/// The shape of the effects up to the restore: every attempt guarded, every
/// pause justified, only contention retried, consistent clock readings, and
/// at most `timeout` of pauses in total.
pub open spec fn well_paced<E>(trace: Seq<WalEvent<E>>, timeout: int) -> bool {
    &&& attempts_guarded(trace, timeout)
    &&& pauses_justified(trace, timeout)
    &&& only_contention_retried(trace)
    &&& clock_consistent(trace)
    &&& total_pause(trace) <= timeout
}

/// `trace` and `outcome` describe a correct run of `enable_wal` with
/// `timeout` (nanoseconds): every answer came from the oracles, the effects
/// are well paced, the busy timeout was restored last, and the outcome is
/// the one the final effects determine.
pub open spec fn wal_run<E, C, B, A>(
    timeout: int,
    in_memory: bool,
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    outcome: WalOutcome<E>,
) -> bool where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E> {
    &&& answers_from(clock, set_busy, attempt, trace)
    &&& well_paced(trace, timeout)
    &&& restored_last(trace, timeout)
    &&& outcome_matches(trace, timeout, in_memory, outcome)
}

// ---------------------------------------------------------------------------
// Trace lemmas
// ---------------------------------------------------------------------------

/// Extending a trace does not change the pauses within its old part.
pub proof fn lemma_pauses_between_push<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, i: int, j: int)
    requires
        j <= trace.len(),
    ensures
        pauses_between(trace.push(ev), i, j) == pauses_between(trace, i, j),
    decreases j - i,
{
    if i < j {
        lemma_pauses_between_push(trace, ev, i, j - 1);
    }
}

/// The pauses of a pushed trace: the old ones, plus the new event's.
pub proof fn lemma_total_pause_push<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>)
    ensures
        total_pause(trace.push(ev)) == total_pause(trace) + match ev {
            WalEvent::Pause { nanos } => nanos,
            _ => 0,
        },
{
    lemma_pauses_between_push(trace, ev, 0, trace.len() as int);
}

/// Pushing anything but an attempt keeps every attempt guarded.
pub proof fn lemma_push_keeps_guarded<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, timeout: int)
    requires
        attempts_guarded(trace, timeout),
        !(ev is Attempt),
    ensures
        attempts_guarded(trace.push(ev), timeout),
{
    reveal(attempts_guarded);
    let t2 = trace.push(ev);
    assert forall|i: int| 0 <= i < t2.len() && #[trigger] t2[i] is Attempt implies guarded_at(
        t2,
        i,
        timeout,
    ) by {
        assert(trace[i] is Attempt);
    }
}

/// Pushing anything but a pause keeps every pause justified.
pub proof fn lemma_push_keeps_paused<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, timeout: int)
    requires
        pauses_justified(trace, timeout),
        !(ev is Pause),
    ensures
        pauses_justified(trace.push(ev), timeout),
{
    reveal(pauses_justified);
    let t2 = trace.push(ev);
    assert forall|i: int| 0 <= i < t2.len() && #[trigger] t2[i] is Pause implies {
        &&& 2 <= i
        &&& t2[i - 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) }
        &&& t2[i - 1] is Clock
        &&& t2[i - 1]->Clock_elapsed < timeout
        &&& t2[i]->Pause_nanos == pause_nanos(t2[i - 1]->Clock_elapsed, timeout)
    } by {
        assert(trace[i] is Pause);
    }
}

/// Pushing anything but an attempt keeps only contention retried.
pub proof fn lemma_push_keeps_retries<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>)
    requires
        only_contention_retried(trace),
        !(ev is Attempt),
    ensures
        only_contention_retried(trace.push(ev)),
{
    reveal(only_contention_retried);
    let t2 = trace.push(ev);
    assert forall|i: int, j: int|
        0 <= i < j < t2.len() && #[trigger] t2[i] is Attempt && #[trigger] t2[j] is Attempt implies t2[i] matches WalEvent::Attempt {
        answer: WalAnswer::Contention(_),
    } by {
        assert(trace[i] is Attempt && trace[j] is Attempt);
    }
}

/// Pushing anything but a clock reading keeps the clock readings
/// consistent.
pub proof fn lemma_push_keeps_clock<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>)
    requires
        clock_consistent(trace),
        !(ev is Clock),
    ensures
        clock_consistent(trace.push(ev)),
{
    reveal(clock_consistent);
    let t2 = trace.push(ev);
    assert forall|i: int, j: int|
        0 <= i < j < t2.len() && #[trigger] t2[i] is Clock && #[trigger] t2[j] is Clock
            && no_clock_between(t2, i, j) implies t2[j]->Clock_elapsed == max_int(
        t2[j]->Clock_reading as int,
        t2[i]->Clock_elapsed + pauses_between(t2, i, j),
    ) by {
        lemma_pauses_between_push(trace, ev, i, j);
        assert(trace[i] is Clock && trace[j] is Clock);
        assert forall|k: int| i < k < j implies !(#[trigger] trace[k] is Clock) by {
            assert(t2[k] == trace[k]);
        }
    }
    assert forall|j: int|
        0 <= j < t2.len() && #[trigger] t2[j] is Clock && no_clock_between(t2, -1, j) implies t2[j]->Clock_elapsed
        == t2[j]->Clock_reading by {
        assert(trace[j] is Clock);
        assert forall|k: int| -1 < k < j implies !(#[trigger] trace[k] is Clock) by {
            assert(t2[k] == trace[k]);
        }
    }
}

/// Pushing an answered event keeps every answer accounted for.
pub proof fn lemma_push_answered<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    ev: WalEvent<E>,
) where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E>
    requires
        answers_from(clock, set_busy, attempt, trace),
        answered(clock, set_busy, attempt, ev),
    ensures
        answers_from(clock, set_busy, attempt, trace.push(ev)),
{
    reveal(answers_from);
    let t2 = trace.push(ev);
    assert forall|i: int| 0 <= i < t2.len() implies answered(
        clock,
        set_busy,
        attempt,
        #[trigger] t2[i],
    ) by {
        if i < trace.len() {
            assert(t2[i] == trace[i]);
        }
    }
}

/// Pushing a clock reading keeps the readings consistent when its elapsed
/// time is the reading, raised to the previous elapsed time plus the pauses
/// since (`last` is the previous reading's index, or -1 when there is none).
pub proof fn lemma_push_clock<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, last: int)
    requires
        clock_consistent(trace),
        ev is Clock,
        -1 <= last < trace.len(),
        no_clock_between(trace, last, trace.len() as int),
        last == -1 ==> ev->Clock_elapsed == ev->Clock_reading,
        last >= 0 ==> trace[last] is Clock && ev->Clock_elapsed == max_int(
            ev->Clock_reading as int,
            trace[last]->Clock_elapsed + pauses_between(trace, last, trace.len() as int),
        ),
    ensures
        clock_consistent(trace.push(ev)),
{
    reveal(clock_consistent);
    let t2 = trace.push(ev);
    let n = trace.len() as int;
    assert forall|i: int, j: int|
        0 <= i < j < t2.len() && #[trigger] t2[i] is Clock && #[trigger] t2[j] is Clock
            && no_clock_between(t2, i, j) implies t2[j]->Clock_elapsed == max_int(
        t2[j]->Clock_reading as int,
        t2[i]->Clock_elapsed + pauses_between(t2, i, j),
    ) by {
        lemma_pauses_between_push(trace, ev, i, j);
        if j < n {
            assert(trace[i] is Clock && trace[j] is Clock);
            assert forall|k: int| i < k < j implies !(#[trigger] trace[k] is Clock) by {
                assert(t2[k] == trace[k]);
            }
        } else {
            // The previous reading is the last one before .
            if last == -1 {
                assert(!(trace[i] is Clock));
            } else if i < last {
                assert(t2[last] == trace[last]);
                assert(!(t2[last] is Clock));
            } else if last < i {
                assert(!(trace[i] is Clock));
            }
        }
    }
    assert forall|j: int|
        0 <= j < t2.len() && #[trigger] t2[j] is Clock && no_clock_between(t2, -1, j) implies t2[j]->Clock_elapsed
        == t2[j]->Clock_reading by {
        if j < n {
            assert(trace[j] is Clock);
            assert forall|k: int| -1 < k < j implies !(#[trigger] trace[k] is Clock) by {
                assert(t2[k] == trace[k]);
            }
        } else if last >= 0 {
            assert(t2[last] == trace[last]);
            assert(!(t2[last] is Clock));
        }
    }
}

/// Pushing an attempt right after a clock reading and a successful busy
/// timeout of the remaining time keeps every attempt guarded.
pub proof fn lemma_push_attempt_guarded<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, timeout: int)
    requires
        attempts_guarded(trace, timeout),
        ev is Attempt,
        trace.len() >= 2,
        guarded_at(trace.push(ev), trace.len() as int, timeout),
    ensures
        attempts_guarded(trace.push(ev), timeout),
{
    reveal(attempts_guarded);
    let t2 = trace.push(ev);
    let n = trace.len() as int;
    assert forall|i: int| 0 <= i < t2.len() && #[trigger] t2[i] is Attempt implies guarded_at(
        t2,
        i,
        timeout,
    ) by {
        if i < n {
            assert(t2[i] == trace[i]);
            assert(trace[i] is Attempt);
            assert(t2[i - 2] == trace[i - 2]);
            assert(t2[i - 1] == trace[i - 1]);
        } else {
            assert(t2[i - 2] == trace[n - 2]);
            assert(t2[i - 1] == trace[n - 1]);
        }
    }
}

/// Every attempt in `trace` reported contention.
#[verifier::opaque]
pub open spec fn all_contended<E>(trace: Seq<WalEvent<E>>) -> bool {
    forall|i: int|
        0 <= i < trace.len() && #[trigger] trace[i] is Attempt ==> trace[i] matches WalEvent::Attempt {
            answer: WalAnswer::Contention(_),
        }
}

/// Pushing an attempt when every earlier attempt reported contention keeps
/// only contention retried.
pub proof fn lemma_push_attempt_retries<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>)
    requires
        all_contended(trace),
        ev is Attempt,
    ensures
        only_contention_retried(trace.push(ev)),
{
    reveal(only_contention_retried);
    reveal(all_contended);
    let t2 = trace.push(ev);
    assert forall|i: int, j: int|
        0 <= i < j < t2.len() && #[trigger] t2[i] is Attempt && #[trigger] t2[j] is Attempt implies t2[i] matches WalEvent::Attempt {
        answer: WalAnswer::Contention(_),
    } by {
        assert(t2[i] == trace[i]);
        assert(trace[i] is Attempt);
    }
}

/// Pushing a contended attempt keeps every attempt contended.
pub proof fn lemma_push_contended<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>)
    requires
        all_contended(trace),
        !(ev is Attempt) || ev matches WalEvent::Attempt { answer: WalAnswer::Contention(_) },
    ensures
        all_contended(trace.push(ev)),
{
    reveal(all_contended);
    let t2 = trace.push(ev);
    assert forall|i: int| 0 <= i < t2.len() && #[trigger] t2[i] is Attempt implies t2[i] matches WalEvent::Attempt {
        answer: WalAnswer::Contention(_),
    } by {
        if i < trace.len() {
            assert(t2[i] == trace[i]);
            assert(trace[i] is Attempt);
        }
    }
}

/// Pushing a pause right after a contended attempt and a clock reading
/// before the timeout, lasting `pause_nanos`, keeps pauses justified.
pub proof fn lemma_push_pause<E>(trace: Seq<WalEvent<E>>, ev: WalEvent<E>, timeout: int)
    requires
        pauses_justified(trace, timeout),
        ev is Pause,
        trace.len() >= 2,
        trace[trace.len() - 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) },
        trace[trace.len() - 1] is Clock,
        trace[trace.len() - 1]->Clock_elapsed < timeout,
        ev->Pause_nanos == pause_nanos(trace[trace.len() - 1]->Clock_elapsed, timeout),
    ensures
        pauses_justified(trace.push(ev), timeout),
{
    reveal(pauses_justified);
    let t2 = trace.push(ev);
    let n = trace.len() as int;
    assert forall|i: int| 0 <= i < t2.len() && #[trigger] t2[i] is Pause implies {
        &&& 2 <= i
        &&& t2[i - 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) }
        &&& t2[i - 1] is Clock
        &&& t2[i - 1]->Clock_elapsed < timeout
        &&& t2[i]->Pause_nanos == pause_nanos(t2[i - 1]->Clock_elapsed, timeout)
    } by {
        if i < n {
            assert(trace[i] is Pause);
        } else {
            assert(t2[i - 2] == trace[n - 2]);
            assert(t2[i - 1] == trace[n - 1]);
        }
    }
}

// ---------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------

/// How the attempts ended, before the restore: the outcome the last effects
/// determine (`RestoreFailed` is decided only by the restore).
pub open spec fn settled<E>(
    trace: Seq<WalEvent<E>>,
    timeout: int,
    in_memory: bool,
    outcome: WalOutcome<E>,
) -> bool {
    let n = trace.len() as int;
    match outcome {
        WalOutcome::Enabled => {
            &&& n >= 1
            &&& trace[n - 1] matches WalEvent::Attempt { answer: WalAnswer::Mode(m) }
                && mode_accepted(m@, in_memory)
        },
        WalOutcome::NotWal(m) => {
            &&& n >= 1
            &&& trace[n - 1] == WalEvent::<E>::Attempt { answer: WalAnswer::Mode(m) }
            &&& !mode_accepted(m@, in_memory)
        },
        WalOutcome::TimedOut(e) => {
            &&& n >= 2
            &&& trace[n - 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Contention(e) }
            &&& trace[n - 1] is Clock
            &&& trace[n - 1]->Clock_elapsed >= timeout
        },
        WalOutcome::Failed(e) => {
            &&& n >= 1
            &&& trace[n - 1] == WalEvent::<E>::Attempt { answer: WalAnswer::Failed(e) }
        },
        WalOutcome::BusyTimeoutFailed(e) => {
            &&& n >= 1
            &&& trace[n - 1] is SetBusy
            &&& trace[n - 1]->SetBusy_result == Err::<(), E>(e)
        },
        WalOutcome::RestoreFailed(_) => false,
    }
}

/// The empty trace has every property.
proof fn lemma_empty_trace<E, C, B, A>(clock: C, set_busy: B, attempt: A, timeout: int) where
    C: Fn() -> u128,
    B: Fn(u128) -> Result<(), E>,
    A: Fn() -> WalAnswer<E>,

    ensures
        answers_from(clock, set_busy, attempt, Seq::<WalEvent<E>>::empty()),
        attempts_guarded(Seq::<WalEvent<E>>::empty(), timeout),
        pauses_justified(Seq::<WalEvent<E>>::empty(), timeout),
        only_contention_retried(Seq::<WalEvent<E>>::empty()),
        clock_consistent(Seq::<WalEvent<E>>::empty()),
        all_contended(Seq::<WalEvent<E>>::empty()),
        total_pause(Seq::<WalEvent<E>>::empty()) == 0,
{
    reveal(answers_from);
    reveal(attempts_guarded);
    reveal(pauses_justified);
    reveal(only_contention_retried);
    reveal(clock_consistent);
    reveal(all_contended);
}

/// The trace properties the retry loop maintains: every answer from the
/// oracles, every attempt guarded, every pause justified, only contention
/// retried, and consistent clock readings.
pub open spec fn loop_props<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    timeout: int,
) -> bool where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E> {
    &&& answers_from(clock, set_busy, attempt, trace)
    &&& attempts_guarded(trace, timeout)
    &&& pauses_justified(trace, timeout)
    &&& only_contention_retried(trace)
    &&& clock_consistent(trace)
}

/// Pushing an answered busy-timeout effect keeps the loop properties and the
/// total pause.
proof fn lemma_push_busy_event<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    ev: WalEvent<E>,
    timeout: int,
) where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E>
    requires
        loop_props(clock, set_busy, attempt, trace, timeout),
        answered(clock, set_busy, attempt, ev),
        ev is SetBusy,
    ensures
        loop_props(clock, set_busy, attempt, trace.push(ev), timeout),
        total_pause(trace.push(ev)) == total_pause(trace),
        all_contended(trace) ==> all_contended(trace.push(ev)),
{
    lemma_push_answered(clock, set_busy, attempt, trace, ev);
    lemma_push_keeps_guarded(trace, ev, timeout);
    lemma_push_keeps_paused(trace, ev, timeout);
    lemma_push_keeps_retries(trace, ev);
    lemma_push_keeps_clock(trace, ev);
    lemma_total_pause_push(trace, ev);
    if all_contended(trace) {
        lemma_push_contended(trace, ev);
    }
}

/// Pushing an answered clock reading whose elapsed time is the reading,
/// raised to the previous elapsed time plus the pauses since, keeps the
/// loop properties.
proof fn lemma_push_clock_event<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    ev: WalEvent<E>,
    last: int,
    timeout: int,
) where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E>
    requires
        loop_props(clock, set_busy, attempt, trace, timeout),
        all_contended(trace),
        answered(clock, set_busy, attempt, ev),
        ev is Clock,
        -1 <= last < trace.len(),
        no_clock_between(trace, last, trace.len() as int),
        last == -1 ==> ev->Clock_elapsed == ev->Clock_reading,
        last >= 0 ==> trace[last] is Clock && ev->Clock_elapsed == max_int(
            ev->Clock_reading as int,
            trace[last]->Clock_elapsed + pauses_between(trace, last, trace.len() as int),
        ),
    ensures
        loop_props(clock, set_busy, attempt, trace.push(ev), timeout),
        all_contended(trace.push(ev)),
        total_pause(trace.push(ev)) == total_pause(trace),
{
    lemma_push_clock(trace, ev, last);
    lemma_push_answered(clock, set_busy, attempt, trace, ev);
    lemma_push_keeps_guarded(trace, ev, timeout);
    lemma_push_keeps_paused(trace, ev, timeout);
    lemma_push_keeps_retries(trace, ev);
    lemma_push_contended(trace, ev);
    lemma_total_pause_push(trace, ev);
}

/// Pushing an answered attempt right after a clock reading and a successful
/// busy timeout of the remaining time keeps the loop properties.
proof fn lemma_push_attempt_event<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    ev: WalEvent<E>,
    timeout: int,
) where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E>
    requires
        loop_props(clock, set_busy, attempt, trace, timeout),
        all_contended(trace),
        answered(clock, set_busy, attempt, ev),
        ev is Attempt,
        trace.len() >= 2,
        guarded_at(trace.push(ev), trace.len() as int, timeout),
    ensures
        loop_props(clock, set_busy, attempt, trace.push(ev), timeout),
        total_pause(trace.push(ev)) == total_pause(trace),
        ev matches WalEvent::Attempt { answer: WalAnswer::Contention(_) } ==> all_contended(
            trace.push(ev),
        ),
{
    lemma_push_attempt_guarded(trace, ev, timeout);
    lemma_push_attempt_retries(trace, ev);
    lemma_push_keeps_paused(trace, ev, timeout);
    lemma_push_keeps_clock(trace, ev);
    lemma_push_answered(clock, set_busy, attempt, trace, ev);
    lemma_total_pause_push(trace, ev);
    if ev matches WalEvent::Attempt { answer: WalAnswer::Contention(_) } {
        lemma_push_contended(trace, ev);
    }
}

/// Pushing a pause right after a contended attempt and a clock reading
/// before the timeout, lasting `pause_nanos`, keeps the loop properties.
proof fn lemma_push_pause_event<E, C, B, A>(
    clock: C,
    set_busy: B,
    attempt: A,
    trace: Seq<WalEvent<E>>,
    ev: WalEvent<E>,
    timeout: int,
) where C: Fn() -> u128, B: Fn(u128) -> Result<(), E>, A: Fn() -> WalAnswer<E>
    requires
        loop_props(clock, set_busy, attempt, trace, timeout),
        all_contended(trace),
        ev is Pause,
        trace.len() >= 2,
        trace[trace.len() - 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) },
        trace[trace.len() - 1] is Clock,
        trace[trace.len() - 1]->Clock_elapsed < timeout,
        ev->Pause_nanos == pause_nanos(trace[trace.len() - 1]->Clock_elapsed, timeout),
    ensures
        loop_props(clock, set_busy, attempt, trace.push(ev), timeout),
        all_contended(trace.push(ev)),
        total_pause(trace.push(ev)) == total_pause(trace) + ev->Pause_nanos,
{
    lemma_push_pause(trace, ev, timeout);
    lemma_push_answered(clock, set_busy, attempt, trace, ev);
    lemma_push_keeps_guarded(trace, ev, timeout);
    lemma_push_keeps_retries(trace, ev);
    lemma_push_keeps_clock(trace, ev);
    lemma_push_contended(trace, ev);
    lemma_total_pause_push(trace, ev);
}

/// The pauses over a clock reading, a busy timeout and an attempt: none.
proof fn lemma_round_has_no_pause<E>(trace: Seq<WalEvent<E>>)
    requires
        trace.len() >= 3,
        trace[trace.len() - 3] is Clock,
        trace[trace.len() - 2] is SetBusy,
        trace[trace.len() - 1] is Attempt,
    ensures
        pauses_between(trace, trace.len() - 3, trace.len() as int) == 0,
        no_clock_between(trace, trace.len() - 3, trace.len() as int),
{
    let n = trace.len() as int;
    assert(pauses_between(trace, n - 3, n - 3) == 0);
    assert(pauses_between(trace, n - 3, n - 2) == 0);
    assert(pauses_between(trace, n - 3, n - 1) == 0);
}

/// The pauses over a clock reading and a pause: the pause.
proof fn lemma_round_pause<E>(trace: Seq<WalEvent<E>>)
    requires
        trace.len() >= 2,
        trace[trace.len() - 2] is Clock,
        trace[trace.len() - 1] is Pause,
    ensures
        pauses_between(trace, trace.len() - 2, trace.len() as int) == trace[trace.len()
            - 1]->Pause_nanos,
        no_clock_between(trace, trace.len() - 2, trace.len() as int),
{
    let n = trace.len() as int;
    assert(pauses_between(trace, n - 2, n - 2) == 0);
    assert(pauses_between(trace, n - 2, n - 1) == 0);
}

/// The index of the previous clock reading at the start of a round: none
/// in an empty trace, otherwise the reading just before the last pause.
pub open spec fn previous_reading<E>(trace: Seq<WalEvent<E>>) -> int {
    if trace.len() == 0 {
        -1
    } else {
        trace.len() - 2
    }
}

/// Runs attempts until one settles (see `settled`).
fn attempt_until_settled<E, C, B, A, P>(
    timeout: u128,
    in_memory: bool,
    clock: &C,
    set_busy: &B,
    attempt: &A,
    pause: &P,
) -> (r: (WalOutcome<E>, Ghost<Seq<WalEvent<E>>>)) where
    C: Fn() -> u128,
    B: Fn(u128) -> Result<(), E>,
    A: Fn() -> WalAnswer<E>,
    P: Fn(u64),

    requires
        clock.requires(()),
        forall|ms: u128| set_busy.requires((ms,)),
        attempt.requires(()),
        forall|ns: u64| pause.requires((ns,)),
    ensures
        answers_from(*clock, *set_busy, *attempt, r.1@),
        well_paced(r.1@, timeout as int),
        settled(r.1@, timeout as int, in_memory, r.0),
{
    let ghost tt = timeout as int;
    let ghost mut trace: Seq<WalEvent<E>> = Seq::empty();
    let mut floor: u128 = 0;
    proof {
        lemma_empty_trace(*clock, *set_busy, *attempt, tt);
    }
    loop
        invariant
            clock.requires(()),
            forall|ms: u128| set_busy.requires((ms,)),
            attempt.requires(()),
            forall|ns: u64| pause.requires((ns,)),
            tt == timeout as int,
            floor <= timeout,
            loop_props(*clock, *set_busy, *attempt, trace, tt),
            all_contended(trace),
            total_pause(trace) <= floor,
            trace.len() == 0 ==> floor == 0,
            trace.len() > 0 ==> {
                &&& trace.len() >= 2
                &&& trace[trace.len() - 1] is Pause
                &&& trace[trace.len() - 2] is Clock
                &&& trace[trace.len() - 2]->Clock_elapsed + trace[trace.len() - 1]->Pause_nanos
                    == floor
            },
        decreases timeout - floor,
    {
        // The elapsed time is the clock reading, never less than the time
        // already accounted for.
        let reading = clock();
        let t: u128 = if reading > floor { reading } else { floor };
        let ghost ev_clock = WalEvent::<E>::Clock { reading, elapsed: t as int };
        proof {
            if trace.len() > 0 {
                lemma_round_pause(trace);
            }
            let last = previous_reading(trace);
            lemma_push_clock_event(*clock, *set_busy, *attempt, trace, ev_clock, last, tt);
            trace = trace.push(ev_clock);
        }
        // Limit SQLite's own busy handler to the remaining time.
        let millis = timeout.saturating_sub(t) / NANOS_PER_MILLI;
        let set = set_busy(millis);
        let ghost ev_set = WalEvent::<E>::SetBusy { millis: millis as int, result: set };
        proof {
            lemma_push_busy_event(*clock, *set_busy, *attempt, trace, ev_set, tt);
            trace = trace.push(ev_set);
        }
        if let Err(e) = set {
            return (WalOutcome::BusyTimeoutFailed(e), Ghost(trace));
        }
        let answer = attempt();
        let ghost ev_attempt = WalEvent::<E>::Attempt { answer };
        proof {
            lemma_push_attempt_event(*clock, *set_busy, *attempt, trace, ev_attempt, tt);
            trace = trace.push(ev_attempt);
        }
        let error = match answer {
            WalAnswer::Mode(mode) => {
                if journal_mode_accepted(mode.as_slice(), in_memory) {
                    return (WalOutcome::Enabled, Ghost(trace));
                }
                return (WalOutcome::NotWal(mode), Ghost(trace));
            },
            WalAnswer::Failed(e) => {
                return (WalOutcome::Failed(e), Ghost(trace));
            },
            WalAnswer::Contention(e) => e,
        };
        // Contention: give up once the timeout has passed.
        let reading_after = clock();
        let u: u128 = if reading_after > t { reading_after } else { t };
        let ghost ev_after = WalEvent::<E>::Clock { reading: reading_after, elapsed: u as int };
        proof {
            lemma_round_has_no_pause(trace);
            let last = trace.len() - 3;
            lemma_push_clock_event(*clock, *set_busy, *attempt, trace, ev_after, last, tt);
            trace = trace.push(ev_after);
        }
        if u >= timeout {
            return (WalOutcome::TimedOut(error), Ghost(trace));
        }
        // Otherwise pause for the remaining time, at most 10 ms, and retry.
        let nanos: u64 = match u64::try_from(timeout - u) {
            Ok(left) if left < PAUSE_CAP_NANOS => left,
            _ => PAUSE_CAP_NANOS,
        };
        pause(nanos);
        let ghost ev_pause = WalEvent::<E>::Pause { nanos: nanos as int };
        proof {
            lemma_push_pause_event(*clock, *set_busy, *attempt, trace, ev_pause, tt);
            trace = trace.push(ev_pause);
        }
        floor = u + u128::from(nanos);
    }
}

/// Enables WAL journal mode, retrying on lock contention within `timeout`
/// nanoseconds, and restores the busy timeout to the whole timeout last (see
/// `wal_run` and the module documentation). Terminates for every oracle.
///
/// The oracles: `clock` reads the nanoseconds elapsed since the start,
/// `set_busy` runs `PRAGMA busy_timeout = <ms>`, `attempt` runs
/// `PRAGMA journal_mode = WAL`, and `pause` sleeps the given nanoseconds.
pub fn enable_wal<E, C, B, A, P>(
    timeout: u128,
    in_memory: bool,
    clock: &C,
    set_busy: &B,
    attempt: &A,
    pause: &P,
) -> (r: (WalOutcome<E>, Ghost<Seq<WalEvent<E>>>)) where
    C: Fn() -> u128,
    B: Fn(u128) -> Result<(), E>,
    A: Fn() -> WalAnswer<E>,
    P: Fn(u64),

    requires
        clock.requires(()),
        forall|ms: u128| set_busy.requires((ms,)),
        attempt.requires(()),
        forall|ns: u64| pause.requires((ns,)),
    ensures
        wal_run(timeout as int, in_memory, *clock, *set_busy, *attempt, r.1@, r.0),
{
    let (outcome, _attempts) = attempt_until_settled(
        timeout,
        in_memory,
        clock,
        set_busy,
        attempt,
        pause,
    );
    let millis = timeout / NANOS_PER_MILLI;
    let restored = set_busy(millis);
    let ghost ev = WalEvent::<E>::SetBusy { millis: millis as int, result: restored };
    let ghost trace = _attempts@.push(ev);
    proof {
        lemma_push_busy_event(*clock, *set_busy, *attempt, _attempts@, ev, timeout as int);
        assert(trace[trace.len() - 2] == _attempts@[_attempts@.len() - 1]);
    }
    let outcome = match outcome {
        WalOutcome::Enabled => match restored {
            Ok(()) => WalOutcome::Enabled,
            Err(e) => WalOutcome::RestoreFailed(e),
        },
        other => other,
    };
    (outcome, Ghost(trace))
}

} // verus!
