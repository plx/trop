//! Verified kernel: reservation expiry arithmetic and the SQL cutoff.
//!
//! - `expired(now, last_used, max_age)` is the expiry rule in whole seconds:
//!   strictly more than `max_age` seconds have passed since the last use. A
//!   last use in the future is never expired, an age of exactly `max_age` is
//!   not expired, and a larger `max_age` expires a subset (lemmas below).
//! - `expiry_cutoff` turns the rule into the SQL predicate
//!   `last_used_at < cutoff` and is exact for every `i64` timestamp: it
//!   selects precisely the expired timestamps, including for maximum ages
//!   beyond `i64::MAX` seconds (the old `as i64` cast wrapped those to
//!   negative ages and selected every reservation).
//! - `duration_exceeds` is the `Duration` comparison `age > max_age` on
//!   `(seconds, nanoseconds)` pairs. With a whole-second age (a caller that
//!   captures `now` in whole seconds) it agrees with `expired`
//!   (`lemma_whole_second_age_agrees`), so such a caller and the SQL query
//!   select the same reservations. `Reservation::is_expired` reads the clock
//!   with sub-second precision while `find_expired_reservations` truncates
//!   `now` to whole seconds, so the two can disagree for up to one second
//!   around the boundary; the lemma does not cover that pair.
//!
//! Production glue: `Database::find_expired_reservations` binds
//! `expiry_cutoff(now, max_age.as_secs())`; `Reservation::is_expired` decides
//! with `duration_exceeds`. Reading the clock and `SystemTime::duration_since`
//! stay in the glue.

use vstd::prelude::*;

verus! {

/// Nanoseconds per second.
pub open spec fn nanos_per_sec() -> int {
    1_000_000_000
}

/// The expiry rule in whole seconds: more than `max_age` seconds have passed
/// between `last_used` and `now`.
pub open spec fn expired(now: int, last_used: int, max_age: int) -> bool {
    now - last_used > max_age
}

/// The SQL query `last_used_at < cutoff` selects the stored timestamp `t`.
pub open spec fn selected_by(t: int, cutoff: int) -> bool {
    t < cutoff
}

/// A last use in the future (or now) is never expired, for any non-negative
/// maximum age.
pub proof fn lemma_future_never_expired(now: int, last_used: int, max_age: int)
    requires
        last_used >= now,
        max_age >= 0,
    ensures
        !expired(now, last_used, max_age),
{
}

/// The boundary is strict: an age of exactly `max_age` is not expired, one
/// second more is.
pub proof fn lemma_expiry_boundary(now: int, max_age: int)
    ensures
        !expired(now, now - max_age, max_age),
        expired(now, now - max_age - 1, max_age),
{
}

/// A larger maximum age expires a subset of the reservations.
pub proof fn lemma_expiry_monotone(now: int, last_used: int, small: int, large: int)
    requires
        small <= large,
        expired(now, last_used, large),
    ensures
        expired(now, last_used, small),
{
}

/// The cutoff for `last_used_at < cutoff`: exactly the expired stored
/// timestamps are selected, for every `i64` timestamp. Total: `now_secs - max_age_secs`
/// is computed without overflow, and when it lies below `i64::MIN` (no `i64`
/// timestamp is that old) the cutoff is `i64::MIN`, which selects nothing.
pub fn expiry_cutoff(now_secs: i64, max_age_secs: u64) -> (cutoff: i64)
    ensures
        forall|t: i64| #[trigger]
            selected_by(t as int, cutoff as int) == expired(
                now_secs as int,
                t as int,
                max_age_secs as int,
            ),
        now_secs as int - max_age_secs as int >= i64::MIN as int ==> cutoff as int == now_secs as int
            - max_age_secs as int,
{
    let bound = i128::from(now_secs) - i128::from(max_age_secs);
    // `bound <= now_secs <= i64::MAX`, so the conversion fails only below
    // `i64::MIN`.
    i64::try_from(bound).unwrap_or(i64::MIN)
}

/// A maximum age beyond the representable range selects nothing: no stored
/// `i64` timestamp is older than `now - max_age`.
pub proof fn lemma_huge_max_age_selects_nothing(now: i64, max_age: u64, t: i64)
    requires
        max_age as int > now as int - i64::MIN as int,
    ensures
        !expired(now as int, t as int, max_age as int),
{
}

/// `Duration`'s ordering `age > max_age` on `(seconds, nanoseconds)` pairs:
/// lexicographic, seconds first.
pub open spec fn duration_gt(age_secs: int, age_nanos: int, max_secs: int, max_nanos: int) -> bool {
    age_secs > max_secs || (age_secs == max_secs && age_nanos > max_nanos)
}

/// Whether a `Duration` of `age_secs` seconds and `age_nanos` nanoseconds is
/// longer than one of `max_secs` seconds and `max_nanos` nanoseconds. Total.
pub fn duration_exceeds(age_secs: u64, age_nanos: u32, max_secs: u64, max_nanos: u32) -> (exceeds:
    bool)
    ensures
        exceeds == duration_gt(age_secs as int, age_nanos as int, max_secs as int, max_nanos as int),
        age_nanos < nanos_per_sec() && max_nanos < nanos_per_sec() ==> exceeds == (age_secs
            * nanos_per_sec() + age_nanos > max_secs * nanos_per_sec() + max_nanos),
{
    proof {
        if age_nanos < nanos_per_sec() && max_nanos < nanos_per_sec() {
            lemma_duration_gt_is_total_nanos_gt(
                age_secs as int,
                age_nanos as int,
                max_secs as int,
                max_nanos as int,
            );
        }
    }
    age_secs > max_secs || (age_secs == max_secs && age_nanos > max_nanos)
}

/// For well-formed durations (nanoseconds below one second), the
/// lexicographic order is the order of the total number of nanoseconds.
pub proof fn lemma_duration_gt_is_total_nanos_gt(
    age_secs: int,
    age_nanos: int,
    max_secs: int,
    max_nanos: int,
)
    requires
        0 <= age_nanos < nanos_per_sec(),
        0 <= max_nanos < nanos_per_sec(),
    ensures
        duration_gt(age_secs, age_nanos, max_secs, max_nanos) == (age_secs * nanos_per_sec()
            + age_nanos > max_secs * nanos_per_sec() + max_nanos),
{
    if age_secs > max_secs {
        assert(age_secs * nanos_per_sec() >= (max_secs + 1) * nanos_per_sec()) by (nonlinear_arith)
            requires
                age_secs >= max_secs + 1,
        ;
    } else if age_secs < max_secs {
        assert(max_secs * nanos_per_sec() >= (age_secs + 1) * nanos_per_sec()) by (nonlinear_arith)
            requires
                max_secs >= age_secs + 1,
        ;
    }
}

/// With a whole-second age (as when `now` is captured in whole seconds and
/// `last_used` comes from storage), the `Duration` comparison is the
/// whole-second rule `expired`, whatever the maximum age's sub-second part.
/// So a check with such an age and `last_used_at < expiry_cutoff(now,
/// max_secs)` agree. (An age measured with sub-second precision, as in
/// `Reservation::is_expired`, is outside this lemma.)
pub proof fn lemma_whole_second_age_agrees(now: int, last_used: int, max_secs: int, max_nanos: int)
    requires
        now >= last_used,
        max_nanos >= 0,
    ensures
        duration_gt(now - last_used, 0, max_secs, max_nanos) == expired(now, last_used, max_secs),
{
}

} // verus!
