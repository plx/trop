//! Verified kernel: enabling WAL journal mode with a bounded retry.
//!
//! `enable_wal` drives `PRAGMA journal_mode = WAL` through four oracles: the
//! clock (nanoseconds since the start), `PRAGMA busy_timeout = <ms>`, the
//! journal-mode attempt itself, and a pause. Every oracle call takes a
//! *ticket* and returns the next one: the first call takes the ticket the
//! caller passes in, and `enable_wal` hands back the ticket the last call
//! returned. It also returns how the attempt ended and a ghost log
//! (`WalLog`) of its calls. Proved (`wal_run`):
//!
//! - **The log is the calls, in order, with their own answers**
//!   (`calls_chained`). Call `i` took ticket `tickets[i]`, returned
//!   `tickets[i + 1]`, and gave the answer recorded in `events[i]`. A pause
//!   is recorded with the length the kernel requested. The ticket type is
//!   arbitrary, so the kernel can neither copy a ticket nor make one. Each
//!   ticket therefore goes into exactly one call, and the calls form a single
//!   chain from the caller's ticket to the returned one. So no answer is
//!   replayed, no call is left out of the log, and no call happens out of
//!   order. The glue numbers its calls (ticket `n` in, `n + 1` out), which
//!   ties every recorded answer to one numbered call.
//! - **The effects have exactly this shape** (`settled_run`, `wal_run`):
//!   zero or more *retried rounds*, then one *last round*, then the restore.
//!   - A retried round (`retried_round`): a clock reading `t`; a successful
//!     busy timeout of the time remaining at `t` (whole milliseconds); an
//!     attempt that reported lock contention; a clock reading `u` *before*
//!     the timeout; a pause of `min(timeout - u, 10 ms)`, which is at least
//!     1 ns.
//!   - The last round (`last_round`) starts with the same clock reading and
//!     busy timeout. Then:
//!     - setting the busy timeout failed: `BusyTimeoutFailed(e)`;
//!     - the attempt reported an accepted journal mode: `Enabled`, or
//!       `RestoreFailed(e)` if the restore then failed;
//!     - it reported any other mode `m`: `NotWal(m)`;
//!     - it failed with an error other than contention: `Failed(e)`;
//!     - it reported contention `e` and the next clock reading was at or
//!       past the timeout: `TimedOut(e)`.
//!   - The restore: the busy timeout is set back to the whole timeout, as the
//!     last effect, on every exit.
//!
//!   So an attempt is retried only after contention, only while time
//!   remains, and only after a pause; the restore is always last; and an
//!   earlier error wins over a failed restore.
//! - **Elapsed time** (`clock_at`, `round_floor`): the kernel works with the
//!   clock reading, raised only if needed to the time already accounted for
//!   (the previous elapsed time plus the pause since). Every retry therefore
//!   advances it by at least 1 ns, and the loop terminates for every clock
//!   oracle. The pauses add up to at most `timeout` (`total_pause`).
//! - `mode_accepted`: the reported mode is `wal` (ASCII case-insensitive), or
//!   `memory` for an in-memory database, which SQLite can never put in WAL
//!   mode and no other connection can share.
//!
//! Trusted: SQLite's busy/locked classification, `Instant::elapsed` and
//! `thread::sleep` in the glue. With a monotonic clock and a sleep that
//! lasts at least as long as requested (both `std` guarantees), the
//! elapsed time the kernel works with equals the clock reading, so the
//! decisions are exactly those of the historical loop. The chain argument
//! relies on the kernel being generic in the ticket type: it uses no
//! operation that inspects a type (no `size_of`, `Any` or trait bound on the
//! ticket), so it makes the same calls whatever ticket type the glue uses.
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

/// A pause after contention before the timeout lasts at least 1 ns and at
/// most 10 ms, and ends no later than the timeout.
pub proof fn lemma_pause_bounds(elapsed: int, timeout: int)
    requires
        elapsed < timeout,
    ensures
        1 <= pause_nanos(elapsed, timeout) <= PAUSE_CAP_NANOS,
        elapsed + pause_nanos(elapsed, timeout) <= timeout,
{
}

/// The larger of two integers.
pub open spec fn max_int(a: int, b: int) -> int {
    if a < b {
        b
    } else {
        a
    }
}

// ---------------------------------------------------------------------------
// Oracle answers, outcomes, and the log
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

/// One call `enable_wal` made, with the answer it got.
// A ghost type: only specifications use it. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint.
#[allow(dead_code)]
pub ghost enum WalEvent<E> {
    /// The clock answered `reading`; the kernel works with the elapsed time
    /// `elapsed` (see `clock_at`).
    Clock { reading: u128, elapsed: int },
    /// `PRAGMA busy_timeout = millis` answered `result`.
    SetBusy { millis: int, result: Result<(), E> },
    /// `PRAGMA journal_mode = WAL` answered `answer`.
    Attempt { answer: WalAnswer<E> },
    /// A pause of `nanos` nanoseconds.
    Pause { nanos: int },
}

/// The ghost log of a run: call `i` made the effect `events[i]`, took the
/// ticket `tickets[i]`, and returned the ticket `tickets[i + 1]`.
// A ghost type, kept by ordinary builds like `WalEvent`.
#[allow(dead_code)]
pub ghost struct WalLog<E, K> {
    /// The effects, in the order the calls were made.
    pub events: Seq<WalEvent<E>>,
    /// The tickets: one more than there are events.
    pub tickets: Seq<K>,
}

/// The call that took ticket `tin` and returned ticket `tout` made the
/// effect `ev` and gave the answer recorded there (a pause: of the length
/// recorded).
pub open spec fn answered<E, K, C, B, A, P>(
    clock: C,
    set_busy: B,
    attempt: A,
    pause: P,
    tin: K,
    ev: WalEvent<E>,
    tout: K,
) -> bool where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,
 {
    match ev {
        WalEvent::Clock { reading, .. } => clock.ensures((tin,), (reading, tout)),
        WalEvent::SetBusy { millis, result } => 0 <= millis <= u128::MAX && set_busy.ensures(
            (tin, millis as u128),
            (result, tout),
        ),
        WalEvent::Attempt { answer } => attempt.ensures((tin,), (answer, tout)),
        WalEvent::Pause { nanos } => 0 <= nanos <= u64::MAX && pause.ensures(
            (tin, nanos as u64),
            tout,
        ),
    }
}

/// The log's calls form one chain from ticket `start` to ticket `end`, and
/// every event is the effect and answer of its call.
pub open spec fn calls_chained<E, K, C, B, A, P>(
    clock: C,
    set_busy: B,
    attempt: A,
    pause: P,
    start: K,
    log: WalLog<E, K>,
    end: K,
) -> bool where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,
 {
    &&& log.tickets.len() == log.events.len() + 1
    &&& log.tickets[0] == start
    &&& log.tickets.last() == end
    &&& forall|i: int|
        0 <= i < log.events.len() ==> answered(
            clock,
            set_busy,
            attempt,
            pause,
            log.tickets[i],
            #[trigger] log.events[i],
            log.tickets[i + 1],
        )
}

// ---------------------------------------------------------------------------
// The shape of a run
// ---------------------------------------------------------------------------

/// `trace[s]` is a clock reading, and the elapsed time the kernel works with
/// is the reading, raised (only if needed) to `floor`, the time already
/// accounted for.
pub open spec fn clock_at<E>(trace: Seq<WalEvent<E>>, s: int, floor: int) -> bool {
    &&& 0 <= s < trace.len()
    &&& trace[s] is Clock
    &&& trace[s]->Clock_elapsed == max_int(trace[s]->Clock_reading as int, floor)
}

/// The time already accounted for when the round starting at `s` reads the
/// clock: none for the first round; otherwise the previous round's last
/// elapsed time plus its pause.
pub open spec fn round_floor<E>(trace: Seq<WalEvent<E>>, s: int) -> int {
    if s == 0 {
        0
    } else {
        trace[s - 2]->Clock_elapsed + trace[s - 1]->Pause_nanos
    }
}

/// The round starting at `s` reads the clock and then sets the busy timeout
/// to the time remaining at that reading, in whole milliseconds.
pub open spec fn round_opens<E>(trace: Seq<WalEvent<E>>, s: int, timeout: int) -> bool {
    &&& clock_at(trace, s, round_floor(trace, s))
    &&& s + 1 < trace.len()
    &&& trace[s + 1] is SetBusy
    &&& trace[s + 1]->SetBusy_millis == busy_millis(trace[s]->Clock_elapsed, timeout)
}

/// Round `j` (the effects `5j .. 5j + 5`) was retried: after it opened (see
/// `round_opens`), the busy timeout was set, the attempt reported lock
/// contention, the next clock reading was before the timeout, and the kernel
/// paused for `min(timeout - elapsed, 10 ms)`.
pub open spec fn retried_round<E>(trace: Seq<WalEvent<E>>, j: int, timeout: int) -> bool {
    let s = 5 * j;
    &&& 0 <= j
    &&& s + 4 < trace.len()
    &&& round_opens(trace, s, timeout)
    &&& trace[s + 1]->SetBusy_result is Ok
    &&& trace[s + 2] matches WalEvent::Attempt { answer: WalAnswer::Contention(_) }
    &&& clock_at(trace, s + 3, trace[s]->Clock_elapsed)
    &&& trace[s + 3]->Clock_elapsed < timeout
    &&& trace[s + 4] == WalEvent::<E>::Pause {
        nanos: pause_nanos(trace[s + 3]->Clock_elapsed, timeout),
    }
}

/// The last round, the effects `s .. end`, ended the attempts with `outcome`
/// (before the restore): it opened (see `round_opens`), and then exactly one
/// of these happened.
pub open spec fn last_round<E>(
    trace: Seq<WalEvent<E>>,
    s: int,
    end: int,
    timeout: int,
    in_memory: bool,
    outcome: WalOutcome<E>,
) -> bool {
    let set_ok = trace[s + 1]->SetBusy_result is Ok;
    &&& 0 <= s
    &&& end <= trace.len()
    &&& s + 2 <= end
    &&& round_opens(trace, s, timeout)
    &&& match outcome {
        // Setting the busy timeout failed.
        WalOutcome::BusyTimeoutFailed(e) => {
            &&& end == s + 2
            &&& trace[s + 1]->SetBusy_result == Err::<(), E>(e)
        },
        // The attempt reported an accepted journal mode.
        WalOutcome::Enabled => {
            &&& end == s + 3
            &&& set_ok
            &&& (trace[s + 2] matches WalEvent::Attempt { answer: WalAnswer::Mode(m) }
                && mode_accepted(m@, in_memory))
        },
        // The attempt reported another journal mode.
        WalOutcome::NotWal(m) => {
            &&& end == s + 3
            &&& set_ok
            &&& trace[s + 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Mode(m) }
            &&& !mode_accepted(m@, in_memory)
        },
        // The attempt failed with an error other than contention.
        WalOutcome::Failed(e) => {
            &&& end == s + 3
            &&& set_ok
            &&& trace[s + 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Failed(e) }
        },
        // The attempt reported contention, and the next clock reading was at
        // or past the timeout.
        WalOutcome::TimedOut(e) => {
            &&& end == s + 4
            &&& set_ok
            &&& trace[s + 2] == WalEvent::<E>::Attempt { answer: WalAnswer::Contention(e) }
            &&& clock_at(trace, s + 3, trace[s]->Clock_elapsed)
            &&& trace[s + 3]->Clock_elapsed >= timeout
        },
        // Decided only by the restore.
        WalOutcome::RestoreFailed(_) => false,
    }
}

/// The effects `0 .. end` are some retried rounds followed by a last round
/// that ended the attempts with `outcome`. Each retried round has 5 effects
/// and the last round 2, 3 or 4, so the number of retried rounds is
/// `(end - 2) / 5`.
pub open spec fn settled_run<E>(
    trace: Seq<WalEvent<E>>,
    end: int,
    timeout: int,
    in_memory: bool,
    outcome: WalOutcome<E>,
) -> bool {
    let k = (end - 2) / 5;
    &&& 2 <= end <= trace.len()
    &&& forall|j: int| 0 <= j < k ==> #[trigger] retried_round(trace, j, timeout)
    &&& last_round(trace, 5 * k, end, timeout, in_memory, outcome)
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

/// `log`, `end` and `outcome` describe a correct run of `enable_wal` from
/// ticket `start` with `timeout` (nanoseconds): the log is the chain of
/// calls with their answers; the effects are retried rounds and a last round
/// (see `settled_run`), followed by the restore of the whole timeout as the
/// last effect; and the pauses add up to at most `timeout`. The outcome is
/// the last round's, except that a successful last round becomes
/// `RestoreFailed(e)` when the restore failed with `e`.
pub open spec fn wal_run<E, K, C, B, A, P>(
    timeout: int,
    in_memory: bool,
    clock: C,
    set_busy: B,
    attempt: A,
    pause: P,
    start: K,
    end: K,
    log: WalLog<E, K>,
    outcome: WalOutcome<E>,
) -> bool where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,
 {
    let trace = log.events;
    let n = trace.len() as int;
    &&& calls_chained(clock, set_busy, attempt, pause, start, log, end)
    &&& n >= 3
    &&& trace[n - 1] is SetBusy
    &&& trace[n - 1]->SetBusy_millis == busy_millis(0, timeout)
    &&& match outcome {
        WalOutcome::Enabled => {
            &&& settled_run(trace, n - 1, timeout, in_memory, WalOutcome::Enabled)
            &&& trace[n - 1]->SetBusy_result is Ok
        },
        WalOutcome::RestoreFailed(e) => {
            &&& settled_run(trace, n - 1, timeout, in_memory, WalOutcome::Enabled)
            &&& trace[n - 1]->SetBusy_result == Err::<(), E>(e)
        },
        _ => settled_run(trace, n - 1, timeout, in_memory, outcome),
    }
    &&& total_pause(trace) <= timeout
}

// ---------------------------------------------------------------------------
// Log lemmas
// ---------------------------------------------------------------------------

/// `log` with one more call: the effect `ev`, which returned the ticket
/// `next`.
pub open spec fn record<E, K>(log: WalLog<E, K>, ev: WalEvent<E>, next: K) -> WalLog<E, K> {
    WalLog { events: log.events.push(ev), tickets: log.tickets.push(next) }
}

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

/// Recording the answered call that took the chain's last ticket keeps the
/// chain, and adds the call's pause (if any) to the total.
pub proof fn lemma_record<E, K, C, B, A, P>(
    clock: C,
    set_busy: B,
    attempt: A,
    pause: P,
    start: K,
    log: WalLog<E, K>,
    end: K,
    ev: WalEvent<E>,
    next: K,
) where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,

    requires
        calls_chained(clock, set_busy, attempt, pause, start, log, end),
        answered(clock, set_busy, attempt, pause, end, ev, next),
    ensures
        calls_chained(clock, set_busy, attempt, pause, start, record(log, ev, next), next),
        total_pause(record(log, ev, next).events) == total_pause(log.events) + match ev {
            WalEvent::Pause { nanos } => nanos,
            _ => 0,
        },
{
    let l2 = record(log, ev, next);
    let n = log.events.len() as int;
    assert forall|i: int| 0 <= i < l2.events.len() implies answered(
        clock,
        set_busy,
        attempt,
        pause,
        l2.tickets[i],
        #[trigger] l2.events[i],
        l2.tickets[i + 1],
    ) by {
        if i < n {
            assert(l2.events[i] == log.events[i]);
            assert(l2.tickets[i] == log.tickets[i]);
            assert(l2.tickets[i + 1] == log.tickets[i + 1]);
        } else {
            assert(l2.tickets[i] == log.tickets.last());
        }
    }
    lemma_total_pause_push(log.events, ev);
}

/// Extending a trace keeps the shape of the retried rounds within its old
/// part.
pub proof fn lemma_rounds_kept<E>(
    old: Seq<WalEvent<E>>,
    new: Seq<WalEvent<E>>,
    k: int,
    timeout: int,
)
    requires
        0 <= k,
        5 * k <= old.len() <= new.len(),
        forall|i: int| 0 <= i < old.len() ==> new[i] == old[i],
        forall|j: int| 0 <= j < k ==> #[trigger] retried_round(old, j, timeout),
    ensures
        forall|j: int| 0 <= j < k ==> #[trigger] retried_round(new, j, timeout),
{
    assert forall|j: int| 0 <= j < k implies #[trigger] retried_round(new, j, timeout) by {
        assert(retried_round(old, j, timeout));
        let s = 5 * j;
        assert(new[s] == old[s] && new[s + 1] == old[s + 1] && new[s + 2] == old[s + 2]);
        assert(new[s + 3] == old[s + 3] && new[s + 4] == old[s + 4]);
        if j > 0 {
            assert(new[s - 2] == old[s - 2] && new[s - 1] == old[s - 1]);
        }
    }
}

/// Extending a trace keeps a settled run within its old part.
pub proof fn lemma_settled_kept<E>(
    old: Seq<WalEvent<E>>,
    new: Seq<WalEvent<E>>,
    end: int,
    timeout: int,
    in_memory: bool,
    outcome: WalOutcome<E>,
)
    requires
        old.len() <= new.len(),
        forall|i: int| 0 <= i < old.len() ==> new[i] == old[i],
        settled_run(old, end, timeout, in_memory, outcome),
    ensures
        settled_run(new, end, timeout, in_memory, outcome),
{
    let k = (end - 2) / 5;
    lemma_rounds_kept(old, new, k, timeout);
    let s = 5 * k;
    assert(new[s] == old[s] && new[s + 1] == old[s + 1]);
    if s + 2 < end {
        assert(new[s + 2] == old[s + 2]);
    }
    if s + 3 < end {
        assert(new[s + 3] == old[s + 3]);
    }
    if k > 0 {
        assert(new[s - 2] == old[s - 2] && new[s - 1] == old[s - 1]);
    }
}

/// Recording one more call during round `k` keeps the chain, every earlier
/// effect (so the retried rounds, and the current round's opening and
/// floor), and adds the call's pause (if any) to the total.
proof fn lemma_record_in_round<E, K, C, B, A, P>(
    oracles: (C, B, A, P),
    start: K,
    log: WalLog<E, K>,
    end: K,
    ev: WalEvent<E>,
    next: K,
    k: int,
    timeout: int,
) where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,

    requires
        calls_chained(oracles.0, oracles.1, oracles.2, oracles.3, start, log, end),
        answered(oracles.0, oracles.1, oracles.2, oracles.3, end, ev, next),
        0 <= k,
        5 * k <= log.events.len(),
        forall|j: int| 0 <= j < k ==> #[trigger] retried_round(log.events, j, timeout),
    ensures
        calls_chained(
            oracles.0,
            oracles.1,
            oracles.2,
            oracles.3,
            start,
            record(log, ev, next),
            next,
        ),
        total_pause(record(log, ev, next).events) == total_pause(log.events) + match ev {
            WalEvent::Pause { nanos } => nanos,
            _ => 0,
        },
        record(log, ev, next).events.len() == log.events.len() + 1,
        record(log, ev, next).events[log.events.len() as int] == ev,
        forall|i: int|
            0 <= i < log.events.len() ==> #[trigger] record(log, ev, next).events[i]
                == log.events[i],
        forall|j: int|
            0 <= j < k ==> #[trigger] retried_round(record(log, ev, next).events, j, timeout),
        round_floor(record(log, ev, next).events, 5 * k) == round_floor(log.events, 5 * k),
        round_opens(log.events, 5 * k, timeout) ==> round_opens(
            record(log, ev, next).events,
            5 * k,
            timeout,
        ),
{
    let (clock, set_busy, attempt, pause) = oracles;
    let new = record(log, ev, next).events;
    lemma_record(clock, set_busy, attempt, pause, start, log, end, ev, next);
    lemma_rounds_kept(log.events, new, k, timeout);
    if k > 0 {
        assert(new[5 * k - 2] == log.events[5 * k - 2]);
        assert(new[5 * k - 1] == log.events[5 * k - 1]);
    }
    if round_opens(log.events, 5 * k, timeout) {
        assert(new[5 * k] == log.events[5 * k]);
        assert(new[5 * k + 1] == log.events[5 * k + 1]);
    }
}

// ---------------------------------------------------------------------------
// The driver
// ---------------------------------------------------------------------------

/// Runs rounds until one is the last (see `settled_run`), from ticket
/// `start`; returns the outcome, the last ticket, and the log.
fn attempt_until_settled<E, K, C, B, A, P>(
    start: K,
    timeout: u128,
    in_memory: bool,
    clock: &C,
    set_busy: &B,
    attempt: &A,
    pause: &P,
) -> (r: (WalOutcome<E>, K, Ghost<WalLog<E, K>>)) where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,

    requires
        forall|k: K| clock.requires((k,)),
        forall|k: K, ms: u128| set_busy.requires((k, ms)),
        forall|k: K| attempt.requires((k,)),
        forall|k: K, ns: u64| pause.requires((k, ns)),
    ensures
        calls_chained(*clock, *set_busy, *attempt, *pause, start, r.2@, r.1),
        settled_run(r.2@.events, r.2@.events.len() as int, timeout as int, in_memory, r.0),
        total_pause(r.2@.events) <= timeout,
{
    let ghost tt = timeout as int;
    let ghost oracles = (*clock, *set_busy, *attempt, *pause);
    let ghost mut log = WalLog::<E, K> { events: Seq::empty(), tickets: seq![start] };
    // The number of retried rounds so far.
    let ghost mut k: int = 0;
    let mut ticket = start;
    // The time accounted for so far: the last elapsed time plus the pause.
    let mut floor: u128 = 0;
    loop
        invariant
            forall|k: K| clock.requires((k,)),
            forall|k: K, ms: u128| set_busy.requires((k, ms)),
            forall|k: K| attempt.requires((k,)),
            forall|k: K, ns: u64| pause.requires((k, ns)),
            tt == timeout as int,
            oracles == (*clock, *set_busy, *attempt, *pause),
            calls_chained(*clock, *set_busy, *attempt, *pause, start, log, ticket),
            0 <= k,
            log.events.len() == 5 * k,
            forall|j: int| 0 <= j < k ==> #[trigger] retried_round(log.events, j, tt),
            floor as int == round_floor(log.events, 5 * k),
            floor <= timeout,
            total_pause(log.events) <= floor,
        decreases timeout - floor,
    {
        // The elapsed time is the clock reading, never less than the time
        // already accounted for.
        let (reading, next) = clock(ticket);
        let t: u128 = if reading > floor { reading } else { floor };
        proof {
            let ev = WalEvent::<E>::Clock { reading, elapsed: t as int };
            lemma_record_in_round(oracles, start, log, ticket, ev, next, k, tt);
            log = record(log, ev, next);
        }
        ticket = next;
        // Limit SQLite's own busy handler to the remaining time.
        let millis = timeout.saturating_sub(t) / NANOS_PER_MILLI;
        let (set, next) = set_busy(ticket, millis);
        proof {
            let ev = WalEvent::<E>::SetBusy { millis: millis as int, result: set };
            lemma_record_in_round(oracles, start, log, ticket, ev, next, k, tt);
            log = record(log, ev, next);
            assert(round_opens(log.events, 5 * k, tt));
            assert((log.events.len() - 2) / 5 == k);
        }
        ticket = next;
        if let Err(e) = set {
            return (WalOutcome::BusyTimeoutFailed(e), ticket, Ghost(log));
        }
        let (answer, next) = attempt(ticket);
        proof {
            let ev = WalEvent::<E>::Attempt { answer };
            lemma_record_in_round(oracles, start, log, ticket, ev, next, k, tt);
            log = record(log, ev, next);
            assert((log.events.len() - 2) / 5 == k);
        }
        ticket = next;
        let error = match answer {
            WalAnswer::Mode(mode) => {
                if journal_mode_accepted(mode.as_slice(), in_memory) {
                    return (WalOutcome::Enabled, ticket, Ghost(log));
                }
                return (WalOutcome::NotWal(mode), ticket, Ghost(log));
            },
            WalAnswer::Failed(e) => {
                return (WalOutcome::Failed(e), ticket, Ghost(log));
            },
            WalAnswer::Contention(e) => e,
        };
        // Contention: give up once the timeout has passed.
        let (reading_after, next) = clock(ticket);
        let u: u128 = if reading_after > t { reading_after } else { t };
        proof {
            let ev = WalEvent::<E>::Clock { reading: reading_after, elapsed: u as int };
            lemma_record_in_round(oracles, start, log, ticket, ev, next, k, tt);
            log = record(log, ev, next);
            assert(clock_at(log.events, 5 * k + 3, t as int));
            assert((log.events.len() - 2) / 5 == k);
        }
        ticket = next;
        if u >= timeout {
            return (WalOutcome::TimedOut(error), ticket, Ghost(log));
        }
        // Otherwise pause for the remaining time, at most 10 ms, and retry.
        let nanos: u64 = match u64::try_from(timeout - u) {
            Ok(left) if left < PAUSE_CAP_NANOS => left,
            _ => PAUSE_CAP_NANOS,
        };
        let next = pause(ticket, nanos);
        proof {
            lemma_pause_bounds(u as int, tt);
            let ev = WalEvent::<E>::Pause { nanos: nanos as int };
            lemma_record_in_round(oracles, start, log, ticket, ev, next, k, tt);
            log = record(log, ev, next);
            assert(retried_round(log.events, k, tt));
            k = k + 1;
        }
        ticket = next;
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
/// Each takes a ticket and returns the next one; the first call takes
/// `ticket`, and the ticket the last call returned is returned.
pub fn enable_wal<E, K, C, B, A, P>(
    ticket: K,
    timeout: u128,
    in_memory: bool,
    clock: &C,
    set_busy: &B,
    attempt: &A,
    pause: &P,
) -> (r: (WalOutcome<E>, K, Ghost<WalLog<E, K>>)) where
    C: Fn(K) -> (u128, K),
    B: Fn(K, u128) -> (Result<(), E>, K),
    A: Fn(K) -> (WalAnswer<E>, K),
    P: Fn(K, u64) -> K,

    requires
        forall|k: K| clock.requires((k,)),
        forall|k: K, ms: u128| set_busy.requires((k, ms)),
        forall|k: K| attempt.requires((k,)),
        forall|k: K, ns: u64| pause.requires((k, ns)),
    ensures
        wal_run(
            timeout as int,
            in_memory,
            *clock,
            *set_busy,
            *attempt,
            *pause,
            ticket,
            r.1,
            r.2@,
            r.0,
        ),
{
    let ghost start = ticket;
    // Ghost only: ordinary builds erase every use.
    let (outcome, ticket, _settled) = attempt_until_settled(
        ticket,
        timeout,
        in_memory,
        clock,
        set_busy,
        attempt,
        pause,
    );
    let millis = timeout / NANOS_PER_MILLI;
    let (restored, next) = set_busy(ticket, millis);
    let ghost ev = WalEvent::<E>::SetBusy { millis: millis as int, result: restored };
    let ghost log = record(_settled@, ev, next);
    proof {
        lemma_record(*clock, *set_busy, *attempt, *pause, start, _settled@, ticket, ev, next);
        let n = _settled@.events.len() as int;
        lemma_settled_kept(_settled@.events, log.events, n, timeout as int, in_memory, outcome);
    }
    let outcome = match outcome {
        WalOutcome::Enabled => match restored {
            Ok(()) => WalOutcome::Enabled,
            Err(e) => WalOutcome::RestoreFailed(e),
        },
        other => other,
    };
    (outcome, next, Ghost(log))
}

} // verus!
