//! Verified kernel: Unix time to a UTC civil date and time (proleptic
//! Gregorian calendar, astronomical year numbering) and its
//! `YYYY-MM-DD HH:MM:SS` text.
//!
//! - Calendar vocabulary: `is_leap_year` (divisible by 4 and not by 100, or
//!   divisible by 400), `days_in_month`, `days_in_year`, and
//!   `days_from_civil(y, m, d)`, the day number counted from 1970-01-01. It
//!   is defined as the plain sum of the lengths of the years and months
//!   before the date (`days_before_year`, `days_before_month`), not by a
//!   closed formula, so it is the calendar by construction.
//! - `civil_from_instant`: the civil date and time of the second that
//!   contains an instant given as `SystemTime::duration_since(UNIX_EPOCH)`
//!   reports it (a direction and a magnitude). The result is valid (month in
//!   `1..=12`, day in `1..=days_in_month`, hour below 24, minute and second
//!   below 60) and exactly inverts the calendar:
//!   `days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 +
//!   second` is the floor of the instant in seconds. It is the only valid
//!   civil time with that second count (`lemma_civil_unique`), so it is
//!   `utc_civil(floor_unix_seconds(..))`. Every instant a `Duration` can
//!   express is supported; nothing overflows.
//! - `civil_text`: chrono's `%Y-%m-%d %H:%M:%S`. The year is zero-padded to
//!   four digits, `+`-prefixed above 9999 and `-`-prefixed below 0; the other
//!   fields are two digits. For years `0..=9999` the text is 19 ASCII bytes
//!   with digits and separators at fixed positions (`lemma_civil_text_shape`),
//!   and its byte order is exactly the order of the instants
//!   (`lemma_text_order`), so sorting the text sorts by time.
//! - `format_unix_instant` composes the two and returns a `String` whose
//!   UTF-8 bytes are `civil_text`.
//!
//! Production glue: `trop::output::format_utc_timestamp` (used by the CLI's
//! `format_timestamp`) splits a `SystemTime` with `duration_since`.
//! Agreement with chrono's formatting over chrono's whole range is pinned by
//! differential tests, not proved.

use vstd::prelude::*;

use super::text;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

// ---------------------------------------------------------------------------
// The proleptic Gregorian calendar
// ---------------------------------------------------------------------------

/// A leap year: divisible by 4 and not by 100, or divisible by 400. Years
/// are astronomical (year 0 is 1 BC), and `%` is Euclidean, so the rule
/// extends to negative years with period 400.
pub open spec fn is_leap_year(y: int) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// The number of days in year `y`.
pub open spec fn days_in_year(y: int) -> int {
    if is_leap_year(y) {
        366
    } else {
        365
    }
}

/// The number of days in month `m` (`1..=12`) of year `y`.
pub open spec fn days_in_month(y: int, m: int) -> int {
    if m == 2 {
        if is_leap_year(y) {
            29
        } else {
            28
        }
    } else if m == 4 || m == 6 || m == 9 || m == 11 {
        30
    } else {
        31
    }
}

/// The number of days from 1970-01-01 to January 1 of year `y`: the sum of
/// the lengths of the years in between, negative before 1970.
pub open spec fn days_before_year(y: int) -> int
    decreases (if y >= 1970 {
        y - 1970
    } else {
        1970 - y
    }),
{
    if y > 1970 {
        days_before_year(y - 1) + days_in_year(y - 1)
    } else if y < 1970 {
        days_before_year(y + 1) - days_in_year(y)
    } else {
        0
    }
}

/// The number of days from January 1 to the first day of month `m`: the
/// sum of the lengths of the months before it.
pub open spec fn days_before_month(y: int, m: int) -> int
    decreases m,
{
    if m <= 1 {
        0
    } else {
        days_before_month(y, m - 1) + days_in_month(y, m - 1)
    }
}

/// A valid calendar date.
pub open spec fn is_valid_date(y: int, m: int, d: int) -> bool {
    1 <= m <= 12 && 1 <= d <= days_in_month(y, m)
}

/// The day number of a date, counted from 1970-01-01 (day 0).
pub open spec fn days_from_civil(y: int, m: int, d: int) -> int {
    days_before_year(y) + days_before_month(y, m) + d - 1
}

/// The number of leap years in `1..=n` (for `n >= 0`; the count is extended
/// to every `n` by the same floor formula).
spec fn leaps_through(n: int) -> int {
    n / 4 - n / 100 + n / 400
}

/// Each year adds one to the leap count exactly when it is a leap year.
proof fn lemma_leap_step(n: int)
    ensures
        leaps_through(n) - leaps_through(n - 1) == if is_leap_year(n) {
            1int
        } else {
            0int
        },
{
}

/// Every 400 consecutive years hold 97 leap years.
proof fn lemma_leap_period(n: int)
    ensures
        leaps_through(n + 400) == leaps_through(n) + 97,
{
    assert((n + 400) / 4 == n / 4 + 100);
    assert((n + 400) / 100 == n / 100 + 4);
    assert((n + 400) / 400 == n / 400 + 1);
}

/// Consecutive years: `days_before_year(y + 1) == days_before_year(y) +
/// days_in_year(y)` on both sides of 1970.
pub proof fn lemma_year_step(y: int)
    ensures
        days_before_year(y + 1) == days_before_year(y) + days_in_year(y),
{
    if y >= 1970 {
        assert(days_before_year(y + 1) == days_before_year(y) + days_in_year(y));
    } else {
        assert(days_before_year(y) == days_before_year(y + 1) - days_in_year(y));
    }
}

/// The closed form of `days_before_year`.
proof fn lemma_days_before_year_closed(y: int)
    ensures
        days_before_year(y) == 365 * (y - 1970) + leaps_through(y - 1) - leaps_through(1969),
    decreases (if y >= 1970 {
        y - 1970
    } else {
        1970 - y
    }),
{
    if y > 1970 {
        lemma_days_before_year_closed(y - 1);
        lemma_leap_step(y - 1);
    } else if y < 1970 {
        lemma_days_before_year_closed(y + 1);
        lemma_leap_step(y);
    }
}

/// The calendar repeats every 400 years, which hold 146097 days.
pub proof fn lemma_year_period(y: int)
    ensures
        days_before_year(y + 400) == days_before_year(y) + 146097,
{
    lemma_days_before_year_closed(y + 400);
    lemma_days_before_year_closed(y);
    lemma_leap_period(y - 1);
}

/// The leap rule has period 400.
proof fn lemma_leap_period_shift(y: int, k: int)
    ensures
        is_leap_year(y + 400 * k) == is_leap_year(y),
{
    assert((y + 400 * k) % 4 == y % 4);
    assert((y + 400 * k) % 100 == y % 100);
    assert((y + 400 * k) % 400 == y % 400);
}

/// The 400-year cycle that starts in year `1970 + 400 * k` starts on day
/// `146097 * k`.
pub proof fn lemma_cycle_start(k: int)
    ensures
        days_before_year(1970 + 400 * k) == 146097 * k,
    decreases (if k >= 0 {
        k
    } else {
        -k
    }),
{
    if k > 0 {
        lemma_cycle_start(k - 1);
        lemma_year_period(1970 + 400 * (k - 1));
    } else if k < 0 {
        lemma_cycle_start(k + 1);
        lemma_year_period(1970 + 400 * k);
    }
}

/// Later years start later: `days_before_year` strictly increases, by at
/// least the length of the earlier year.
pub proof fn lemma_years_increase(y1: int, y2: int)
    requires
        y1 < y2,
    ensures
        days_before_year(y1) + days_in_year(y1) <= days_before_year(y2),
    decreases y2 - y1,
{
    lemma_year_step(y1);
    if y1 + 1 < y2 {
        lemma_years_increase(y1 + 1, y2);
    }
}

/// The first day of month `m` (`1..=13`) in a common year, counted from
/// January 1 (13 stands for the end of the year).
spec fn common_month_start(m: int) -> int {
    if m <= 1 {
        0
    } else if m == 2 {
        31
    } else if m == 3 {
        59
    } else if m == 4 {
        90
    } else if m == 5 {
        120
    } else if m == 6 {
        151
    } else if m == 7 {
        181
    } else if m == 8 {
        212
    } else if m == 9 {
        243
    } else if m == 10 {
        273
    } else if m == 11 {
        304
    } else if m == 12 {
        334
    } else {
        365
    }
}

/// The month table: months after February start one day later in a leap
/// year, and the twelve months fill the year.
proof fn lemma_month_table(y: int, m: int)
    requires
        1 <= m <= 13,
    ensures
        days_before_month(y, m) == common_month_start(m) + if m > 2 && is_leap_year(y) {
            1int
        } else {
            0int
        },
        days_before_month(y, 13) == days_in_year(y),
{
    assert(days_before_month(y, 1) == 0);
    assert(days_before_month(y, 2) == days_before_month(y, 1) + 31);
    assert(days_before_month(y, 3) == days_before_month(y, 2) + days_in_month(y, 2));
    assert(days_before_month(y, 4) == days_before_month(y, 3) + 31);
    assert(days_before_month(y, 5) == days_before_month(y, 4) + 30);
    assert(days_before_month(y, 6) == days_before_month(y, 5) + 31);
    assert(days_before_month(y, 7) == days_before_month(y, 6) + 30);
    assert(days_before_month(y, 8) == days_before_month(y, 7) + 31);
    assert(days_before_month(y, 9) == days_before_month(y, 8) + 31);
    assert(days_before_month(y, 10) == days_before_month(y, 9) + 30);
    assert(days_before_month(y, 11) == days_before_month(y, 10) + 31);
    assert(days_before_month(y, 12) == days_before_month(y, 11) + 30);
    assert(days_before_month(y, 13) == days_before_month(y, 12) + 31);
}

/// Later months start later, by at least the length of the earlier month,
/// and every month starts within its year.
pub proof fn lemma_months_increase(y: int, m1: int, m2: int)
    requires
        1 <= m1 < m2 <= 13,
    ensures
        0 <= days_before_month(y, m1),
        days_before_month(y, m1) + days_in_month(y, m1) <= days_before_month(y, m2),
        days_before_month(y, m2) <= days_in_year(y),
{
    lemma_month_table(y, m1);
    lemma_month_table(y, m1 + 1);
    lemma_month_table(y, m2);
    assert(days_before_month(y, m1 + 1) == days_before_month(y, m1) + days_in_month(y, m1));
}

/// A valid date lies inside its year and its month.
pub proof fn lemma_date_bounds(y: int, m: int, d: int)
    requires
        is_valid_date(y, m, d),
    ensures
        days_before_year(y) + days_before_month(y, m) <= days_from_civil(y, m, d),
        days_from_civil(y, m, d) < days_before_year(y) + days_before_month(y, m) + days_in_month(
            y,
            m,
        ),
        0 <= days_before_month(y, m),
        days_before_month(y, m) + days_in_month(y, m) <= days_in_year(y),
        days_from_civil(y, m, d) < days_before_year(y + 1),
{
    lemma_months_increase(y, m, 13);
    lemma_year_step(y);
}

/// Lexicographic order of dates.
pub open spec fn date_lt(y1: int, m1: int, d1: int, y2: int, m2: int, d2: int) -> bool {
    y1 < y2 || (y1 == y2 && (m1 < m2 || (m1 == m2 && d1 < d2)))
}

/// Day numbers follow the calendar order of valid dates.
pub proof fn lemma_dates_increase(y1: int, m1: int, d1: int, y2: int, m2: int, d2: int)
    requires
        is_valid_date(y1, m1, d1),
        is_valid_date(y2, m2, d2),
        date_lt(y1, m1, d1, y2, m2, d2),
    ensures
        days_from_civil(y1, m1, d1) < days_from_civil(y2, m2, d2),
{
    lemma_date_bounds(y1, m1, d1);
    lemma_date_bounds(y2, m2, d2);
    if y1 < y2 {
        lemma_years_increase(y1, y2);
        lemma_year_step(y1);
    } else if m1 < m2 {
        lemma_months_increase(y1, m1, m2);
    }
}

// ---------------------------------------------------------------------------
// Civil date and time
// ---------------------------------------------------------------------------

/// A UTC civil date and time, to the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDateTime {
    /// The astronomical year (year 0 is 1 BC).
    pub year: i64,
    /// The month, `1..=12`.
    pub month: u8,
    /// The day of the month, `1..=31`.
    pub day: u8,
    /// The hour, `0..=23`.
    pub hour: u8,
    /// The minute, `0..=59`.
    pub minute: u8,
    /// The second, `0..=59` (UTC as Unix time counts it: no leap seconds).
    pub second: u8,
}

impl CivilDateTime {
    /// A real date and time of day.
    pub open spec fn is_valid(self) -> bool {
        &&& is_valid_date(self.year as int, self.month as int, self.day as int)
        &&& self.hour < 24
        &&& self.minute < 60
        &&& self.second < 60
    }

    /// The seconds since midnight.
    pub open spec fn time_of_day(self) -> int {
        self.hour * 3600 + self.minute * 60 + self.second
    }

    /// The Unix time (seconds since 1970-01-01 00:00:00 UTC) this names.
    pub open spec fn unix_seconds(self) -> int {
        days_from_civil(self.year as int, self.month as int, self.day as int) * 86400
            + self.time_of_day()
    }
}

/// Lexicographic order of civil date-times (year, month, day, hour,
/// minute, second).
pub open spec fn civil_lt(a: CivilDateTime, b: CivilDateTime) -> bool {
    date_lt(a.year as int, a.month as int, a.day as int, b.year as int, b.month as int, b.day as int)
        || (a.year == b.year && a.month == b.month && a.day == b.day && a.time_of_day()
        < b.time_of_day())
}

/// The valid civil date-time whose Unix time is `s` (unique by
/// `lemma_civil_unique`; it exists for every supported `s`, as
/// `civil_from_instant` constructs it).
pub open spec fn utc_civil(s: int) -> CivilDateTime {
    choose|c: CivilDateTime| c.is_valid() && c.unix_seconds() == s
}

/// Valid civil date-times are ordered like their Unix times.
pub proof fn lemma_civil_order(a: CivilDateTime, b: CivilDateTime)
    requires
        a.is_valid(),
        b.is_valid(),
        civil_lt(a, b),
    ensures
        a.unix_seconds() < b.unix_seconds(),
{
    let da = days_from_civil(a.year as int, a.month as int, a.day as int);
    let db = days_from_civil(b.year as int, b.month as int, b.day as int);
    if date_lt(
        a.year as int,
        a.month as int,
        a.day as int,
        b.year as int,
        b.month as int,
        b.day as int,
    ) {
        lemma_dates_increase(
            a.year as int,
            a.month as int,
            a.day as int,
            b.year as int,
            b.month as int,
            b.day as int,
        );
        assert(da * 86400 + 86400 <= db * 86400);
    }
}

/// A Unix time names at most one valid civil date-time: the conversion is a
/// bijection between Unix times and valid civil date-times (onto, by
/// `civil_from_instant`).
pub proof fn lemma_civil_unique(a: CivilDateTime, b: CivilDateTime)
    requires
        a.is_valid(),
        b.is_valid(),
        a.unix_seconds() == b.unix_seconds(),
    ensures
        a == b,
{
    if civil_lt(a, b) {
        lemma_civil_order(a, b);
    } else if civil_lt(b, a) {
        lemma_civil_order(b, a);
    } else {
        // Same date and time of day; the time of day determines the fields.
        assert(a.time_of_day() == b.time_of_day());
        assert(a.hour == b.hour);
        assert(a.minute == b.minute);
    }
}

/// `utc_civil(s)` is any valid civil date-time with Unix time `s`.
pub proof fn lemma_utc_civil(c: CivilDateTime)
    requires
        c.is_valid(),
    ensures
        utc_civil(c.unix_seconds()) == c,
{
    let s = c.unix_seconds();
    assert(exists|x: CivilDateTime| x.is_valid() && x.unix_seconds() == s) by {
        assert(c.is_valid() && c.unix_seconds() == s);
    }
    lemma_civil_unique(utc_civil(s), c);
}

// ---------------------------------------------------------------------------
// Instants
// ---------------------------------------------------------------------------

/// The instant `SystemTime::duration_since(UNIX_EPOCH)` describes, in
/// nanoseconds since the epoch: `Ok(d)` is `d` after the epoch,
/// `Err(e)` is `e.duration()` before it.
pub open spec fn instant_nanos(before_epoch: bool, secs: u64, nanos: u32) -> int {
    let magnitude = secs * 1_000_000_000 + nanos;
    if before_epoch {
        -magnitude
    } else {
        magnitude
    }
}

/// The Unix second containing an instant: the floor of its time in
/// seconds (`/` on `int` is Euclidean, which is the floor for a positive
/// divisor).
pub open spec fn floor_unix_seconds(before_epoch: bool, secs: u64, nanos: u32) -> int {
    instant_nanos(before_epoch, secs, nanos) / 1_000_000_000
}

/// The Unix times this kernel converts: every floor of a `Duration`-sized
/// instant (at most about `2^64` seconds either side of the epoch).
pub open spec fn supported_seconds(s: int) -> bool {
    -0x2_0000_0000_0000_0000 <= s <= 0x2_0000_0000_0000_0000
}

/// A bound on the day numbers of supported Unix times.
spec fn max_supported_days() -> int {
    0x2_0000_0000_0000_0000int / 86400 + 1
}

/// The floor of an instant in seconds. Total.
fn floor_seconds(before_epoch: bool, secs: u64, nanos: u32) -> (r: i128)
    ensures
        r as int == floor_unix_seconds(before_epoch, secs, nanos),
        supported_seconds(r as int),
{
    let whole = i128::from(nanos / 1_000_000_000);
    let fraction = nanos % 1_000_000_000;
    let magnitude = i128::from(secs) + whole;
    proof {
        let n = secs * 1_000_000_000 + nanos;
        assert(n == (secs + whole) * 1_000_000_000 + fraction);
    }
    if before_epoch {
        if fraction > 0 {
            -magnitude - 1
        } else {
            -magnitude
        }
    } else {
        magnitude
    }
}

/// Splits a Unix time into its day number and the seconds since midnight.
fn split_days(s: i128) -> (r: (i128, i128))
    requires
        supported_seconds(s as int),
    ensures
        s == r.0 * 86400 + r.1,
        0 <= r.1 < 86400,
        -max_supported_days() <= r.0 <= max_supported_days(),
{
    if s >= 0 {
        (s / 86400, s % 86400)
    } else {
        let m = -s;
        let q = m / 86400;
        let rem = m % 86400;
        if rem == 0 {
            (-q, 0)
        } else {
            (-q - 1, 86400 - rem)
        }
    }
}

/// Splits a day number into its 400-year cycle (from 1970) and the day
/// within that cycle.
fn split_cycles(days: i128) -> (r: (i128, i128))
    requires
        -max_supported_days() <= days <= max_supported_days(),
    ensures
        days == r.0 * 146097 + r.1,
        0 <= r.1 < 146097,
        -3_000_000_000 <= r.0 <= 3_000_000_000,
{
    if days >= 0 {
        (days / 146_097, days % 146_097)
    } else {
        let m = -days;
        let q = m / 146_097;
        let rem = m % 146_097;
        if rem == 0 {
            (-q, 0)
        } else {
            (-q - 1, 146_097 - rem)
        }
    }
}

/// Whether `1970 + offset` is a leap year, for an offset in `0..400`.
fn is_leap_offset(offset: i128) -> (r: bool)
    requires
        0 <= offset < 400,
    ensures
        r == is_leap_year(1970 + offset),
{
    let y = 1970 + offset;
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// `days_in_month(y, month)`, computed from whether `y` is a leap year.
fn month_length(leap: bool, month: u8, Ghost(y): Ghost<int>) -> (r: i128)
    requires
        leap == is_leap_year(y),
    ensures
        r == days_in_month(y, month as int),
{
    if month == 2 {
        if leap {
            29
        } else {
            28
        }
    } else if month == 4 || month == 6 || month == 9 || month == 11 {
        30
    } else {
        31
    }
}

/// The valid civil date-time of a supported Unix time.
fn civil_from_unix_seconds(s: i128) -> (c: CivilDateTime)
    requires
        supported_seconds(s as int),
    ensures
        c.is_valid(),
        c.unix_seconds() == s,
{
    let (days, sod) = split_days(s);
    let (cycle, day_of_cycle) = split_cycles(days);
    let ghost base = 1970 + 400 * cycle;
    proof {
        lemma_cycle_start(cycle as int);
        lemma_cycle_start(cycle + 1);
    }
    // The year: whole years of the cycle (at most 400 steps).
    let mut offset: i128 = 0;
    let mut rem: i128 = day_of_cycle;
    while rem >= (if is_leap_offset(offset) {
        366
    } else {
        365
    })
        invariant
            0 <= offset < 400,
            0 <= rem,
            days == days_before_year(base + offset) + rem,
            days < days_before_year(base + 400),
            base == 1970 + 400 * cycle,
            -3_000_000_000 <= cycle <= 3_000_000_000,
        decreases rem,
    {
        proof {
            lemma_leap_period_shift(1970 + offset, cycle as int);
            assert(base + offset == 1970 + offset + 400 * cycle);
            lemma_year_step(base + offset);
        }
        rem -= if is_leap_offset(offset) {
            366
        } else {
            365
        };
        offset += 1;
        proof {
            if offset == 400 {
                assert(days_before_year(base + offset) <= days);
            }
        };
    }
    let leap = is_leap_offset(offset);
    let ghost year = base + offset;
    proof {
        lemma_leap_period_shift(1970 + offset, cycle as int);
        assert(year == 1970 + offset + 400 * cycle);
        assert(leap == is_leap_year(year));
        assert(rem < days_in_year(year));
    }
    // The month: whole months of the year (at most 12 steps).
    let mut month: u8 = 1;
    let ghost start = rem;
    while rem >= month_length(leap, month, Ghost(year))
        invariant
            1 <= month <= 12,
            0 <= rem,
            start == days_before_month(year, month as int) + rem,
            start < days_in_year(year),
            leap == is_leap_year(year),
        decreases rem,
    {
        proof {
            if month == 12 {
                lemma_month_table(year, 12);
            }
        }
        rem -= month_length(leap, month, Ghost(year));
        month += 1;
    }
    let ghost total = days_before_month(year, month as int) + rem;
    proof {
        assert(days == days_before_year(year) + days_before_month(year, month as int) + rem);
    }
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;
    let y = 1970 + 400 * cycle + offset;
    let c = CivilDateTime {
        year: i64::try_from(y).unwrap_or_default(),
        month,
        day: u8::try_from(rem + 1).unwrap_or_default(),
        hour: u8::try_from(hour).unwrap_or_default(),
        minute: u8::try_from(minute).unwrap_or_default(),
        second: u8::try_from(second).unwrap_or_default(),
    };
    proof {
        assert(c.year as int == year);
        assert(c.time_of_day() == sod);
        assert(c.unix_seconds() == days * 86400 + sod);
    }
    c
}

/// The civil date-time (UTC) of the second containing an instant, given as
/// `SystemTime::duration_since(UNIX_EPOCH)` reports it: `before_epoch` is
/// `true` for `Err(e)` (with `e.duration()`'s `as_secs` and
/// `subsec_nanos`). Total: every `u64` second count and every `u32`
/// nanosecond count is converted exactly.
pub fn civil_from_instant(before_epoch: bool, secs: u64, nanos: u32) -> (c: CivilDateTime)
    ensures
        c.is_valid(),
        c.unix_seconds() == floor_unix_seconds(before_epoch, secs, nanos),
        c == utc_civil(floor_unix_seconds(before_epoch, secs, nanos)),
{
    let s = floor_seconds(before_epoch, secs, nanos);
    let c = civil_from_unix_seconds(s);
    proof {
        lemma_utc_civil(c);
    }
    c
}

// ---------------------------------------------------------------------------
// Text: chrono's `%Y-%m-%d %H:%M:%S`
// ---------------------------------------------------------------------------

/// The ASCII digit of `d` (`0..=9`).
pub open spec fn digit(d: int) -> u8 {
    (48 + d) as u8
}

/// Two decimal digits (`%m`, `%d`, `%H`, `%M`, `%S`): `v` zero-padded to
/// width 2 (meaningful for `v < 100`).
pub open spec fn two_digits(v: int) -> Seq<u8> {
    seq![digit(v / 10), digit(v % 10)]
}

/// Four decimal digits: `v` (`0..=9999`) zero-padded to width 4.
pub open spec fn four_digits(v: int) -> Seq<u8> {
    seq![digit(v / 1000), digit((v / 100) % 10), digit((v / 10) % 10), digit(v % 10)]
}

/// `v` zero-padded to at least four digits (Rust's `{:04}`).
pub open spec fn padded4(v: nat) -> Seq<u8> {
    if v <= 9999 {
        four_digits(v as int)
    } else {
        super::exclusions::decimal_text(v)
    }
}

/// chrono's `%Y`: the year zero-padded to four digits, with a sign when it
/// is outside `0..=9999` (`+10000`, `-0001`).
pub open spec fn year_text(y: int) -> Seq<u8> {
    if y < 0 {
        seq![b'-'] + padded4((-y) as nat)
    } else if y > 9999 {
        seq![b'+'] + padded4(y as nat)
    } else {
        padded4(y as nat)
    }
}

/// `HH:MM:SS`.
pub open spec fn time_text(c: CivilDateTime) -> Seq<u8> {
    two_digits(c.hour as int) + (seq![b':'] + (two_digits(c.minute as int) + (seq![b':']
        + two_digits(c.second as int))))
}

/// `-MM-DD HH:MM:SS`, the part after the year.
pub open spec fn after_year_text(c: CivilDateTime) -> Seq<u8> {
    seq![b'-'] + (two_digits(c.month as int) + (seq![b'-'] + (two_digits(c.day as int) + (seq![
        b' ',
    ] + time_text(c)))))
}

/// The text of a civil date-time: `%Y-%m-%d %H:%M:%S`.
pub open spec fn civil_text(c: CivilDateTime) -> Seq<u8> {
    year_text(c.year as int) + after_year_text(c)
}

/// Appends `two_digits(v)`.
fn push_two_digits(out: &mut Vec<u8>, v: u8)
    ensures
        final(out)@ == old(out)@ + two_digits(v as int),
{
    out.push(b'0' + v / 10);
    out.push(b'0' + v % 10);
}

/// Appends the digit `d` (`0..=9`).
fn push_digit(out: &mut Vec<u8>, d: u64)
    requires
        d < 10,
    ensures
        final(out)@ == old(out)@.push(digit(d as int)),
{
    out.push(b'0' + u8::try_from(d).unwrap_or_default());
}

/// Appends the canonical decimal text of `v`.
fn push_decimal(out: &mut Vec<u8>, v: u64)
    ensures
        final(out)@ == old(out)@ + super::exclusions::decimal_text(v as nat),
    decreases v,
{
    if v >= 10 {
        push_decimal(out, v / 10);
    }
    push_digit(out, v % 10);
    proof {
        if v < 10 {
            assert(super::exclusions::decimal_text(v as nat) =~= seq![digit(v as int)]);
        } else {
            assert(super::exclusions::decimal_text(v as nat) == super::exclusions::decimal_text(
                (v / 10) as nat,
            ).push(digit((v % 10) as int)));
        }
    };
}

/// Appends `year_text(y)`.
fn push_year(out: &mut Vec<u8>, y: i64)
    ensures
        final(out)@ == old(out)@ + year_text(y as int),
{
    let ghost start = out@;
    let wide = i128::from(y);
    let magnitude = u64::try_from(
        if wide < 0 {
            -wide
        } else {
            wide
        },
    ).unwrap_or_default();
    if y < 0 {
        out.push(b'-');
    } else if y > 9999 {
        out.push(b'+');
    }
    let ghost signed = out@;
    if magnitude <= 9999 {
        push_digit(out, magnitude / 1000);
        push_digit(out, (magnitude / 100) % 10);
        push_digit(out, (magnitude / 10) % 10);
        push_digit(out, magnitude % 10);
        proof {
            assert(out@ =~= signed + four_digits(magnitude as int));
        };
    } else {
        push_decimal(out, magnitude);
    }
    proof {
        assert(padded4(magnitude as nat) =~= out@.subrange(signed.len() as int, out@.len() as int));
        if y < 0 {
            assert(signed =~= start + seq![b'-']);
        } else if y > 9999 {
            assert(signed =~= start + seq![b'+']);
        } else {
            assert(signed =~= start);
        }
        assert(out@ =~= start + year_text(y as int));
    };
}

/// The bytes of `civil_text(c)`. Total.
pub fn civil_text_bytes(c: CivilDateTime) -> (out: Vec<u8>)
    ensures
        out@ == civil_text(c),
{
    let mut out = Vec::new();
    push_year(&mut out, c.year);
    out.push(b'-');
    push_two_digits(&mut out, c.month);
    out.push(b'-');
    push_two_digits(&mut out, c.day);
    out.push(b' ');
    push_two_digits(&mut out, c.hour);
    out.push(b':');
    push_two_digits(&mut out, c.minute);
    out.push(b':');
    push_two_digits(&mut out, c.second);
    proof {
        assert(out@ =~= civil_text(c));
    }
    out
}

/// Every byte of a civil text is ASCII.
pub proof fn lemma_civil_text_ascii(c: CivilDateTime)
    ensures
        forall|i: int| 0 <= i < civil_text(c).len() ==> #[trigger] civil_text(c)[i] < 0x80,
{
    let y = c.year as int;
    let m: nat = if y < 0 {
        (-y) as nat
    } else {
        y as nat
    };
    super::exclusions::lemma_decimal_text(m);
    let t = civil_text(c);
    let yt = year_text(y);
    let sign_len: int = if y < 0 || y > 9999 {
        1
    } else {
        0
    };
    assert(yt == if sign_len == 1 {
        seq![if y < 0 {
            b'-'
        } else {
            b'+'
        }] + padded4(m)
    } else {
        padded4(m)
    });
    assert forall|i: int| 0 <= i < t.len() implies #[trigger] t[i] < 0x80 by {
        if i < yt.len() {
            assert(t[i] == yt[i]);
            if i >= sign_len {
                assert(yt[i] == padded4(m)[i - sign_len]);
                if m > 9999 {
                    assert(super::exclusions::is_digit(super::exclusions::decimal_text(m)[i - sign_len]));
                }
            }
        } else {
            assert(t[i] == after_year_text(c)[i - yt.len()]);
        }
    }
}

/// The text of a civil date-time as a `String` (its UTF-8 bytes are
/// `civil_text(c)`). Total.
pub fn civil_text_string(c: CivilDateTime) -> (text: String)
    ensures
        vstd::utf8::encode_utf8(text@) == civil_text(c),
        text@ == text::chars_of_bytes(civil_text(c)),
{
    let bytes = civil_text_bytes(c);
    proof {
        lemma_civil_text_ascii(c);
        text::lemma_ascii_bytes_round_trip(bytes@);
    }
    text::string_from_ascii(bytes.as_slice())
}

/// The UTC `%Y-%m-%d %H:%M:%S` text of the second containing an instant,
/// given as `SystemTime::duration_since(UNIX_EPOCH)` reports it (see
/// `civil_from_instant`). Total.
pub fn format_unix_instant(before_epoch: bool, secs: u64, nanos: u32) -> (text: String)
    ensures
        vstd::utf8::encode_utf8(text@) == civil_text(
            utc_civil(floor_unix_seconds(before_epoch, secs, nanos)),
        ),
        text@ == text::chars_of_bytes(
            civil_text(utc_civil(floor_unix_seconds(before_epoch, secs, nanos))),
        ),
{
    civil_text_string(civil_from_instant(before_epoch, secs, nanos))
}

// ---------------------------------------------------------------------------
// Properties of the text
// ---------------------------------------------------------------------------

/// The fields after the year are below 100, so each is exactly two digits.
pub open spec fn small_fields(c: CivilDateTime) -> bool {
    c.month < 100 && c.day < 100 && c.hour < 100 && c.minute < 100 && c.second < 100
}

/// Equal prefixes of equal length compare by what follows; a smaller prefix
/// of equal length decides the order.
proof fn lemma_lex_concat(a1: Seq<u8>, a2: Seq<u8>, b1: Seq<u8>, b2: Seq<u8>)
    requires
        a1.len() == a2.len(),
    ensures
        a1 == a2 ==> (super::render::lex_lt(a1 + b1, a2 + b2) == super::render::lex_lt(b1, b2)),
        super::render::lex_lt(a1, a2) ==> super::render::lex_lt(a1 + b1, a2 + b2),
    decreases a1.len(),
{
    if a1.len() == 0 {
        assert(a1 + b1 =~= b1);
        assert(a2 + b2 =~= b2);
    } else {
        assert((a1 + b1).drop_first() =~= a1.drop_first() + b1);
        assert((a2 + b2).drop_first() =~= a2.drop_first() + b2);
        assert((a1 + b1)[0] == a1[0]);
        assert((a2 + b2)[0] == a2[0]);
        lemma_lex_concat(a1.drop_first(), a2.drop_first(), b1, b2);
        if a1 == a2 {
            assert(a1.drop_first() == a2.drop_first());
        }
    }
}

/// One byte against one byte.
proof fn lemma_lex_single(x: u8, y: u8)
    ensures
        super::render::lex_lt(seq![x], seq![y]) == (x < y),
{
    assert(seq![x].drop_first() =~= Seq::<u8>::empty());
    assert(seq![y].drop_first() =~= Seq::<u8>::empty());
    assert(!super::render::lex_lt(Seq::<u8>::empty(), Seq::<u8>::empty()));
}

/// Two-digit fields compare like their values.
proof fn lemma_two_digits_lt(u: int, v: int)
    requires
        0 <= u < v < 100,
    ensures
        super::render::lex_lt(two_digits(u), two_digits(v)),
{
    assert(two_digits(u) =~= seq![digit(u / 10)] + seq![digit(u % 10)]);
    assert(two_digits(v) =~= seq![digit(v / 10)] + seq![digit(v % 10)]);
    lemma_lex_single(digit(u / 10), digit(v / 10));
    lemma_lex_single(digit(u % 10), digit(v % 10));
    lemma_lex_concat(seq![digit(u / 10)], seq![digit(v / 10)], seq![digit(u % 10)], seq![digit(v % 10)]);
}

/// Four digits are the two digits of the hundreds, then of the rest.
#[verifier::spinoff_prover]
proof fn lemma_four_digits_split(v: int)
    requires
        0 <= v <= 9999,
    ensures
        four_digits(v) == two_digits(v / 100) + two_digits(v % 100),
{
    assert((v / 100) / 10 == v / 1000);
    assert((v % 100) / 10 == (v / 10) % 10);
    assert((v % 100) % 10 == v % 10);
    assert(four_digits(v) =~= two_digits(v / 100) + two_digits(v % 100));
}

/// Four-digit years compare like their values.
proof fn lemma_four_digits_lt(u: int, v: int)
    requires
        0 <= u < v <= 9999,
    ensures
        super::render::lex_lt(four_digits(u), four_digits(v)),
{
    lemma_four_digits_split(u);
    lemma_four_digits_split(v);
    lemma_lex_concat(two_digits(u / 100), two_digits(v / 100), two_digits(u % 100), two_digits(v % 100));
    if u / 100 < v / 100 {
        lemma_two_digits_lt(u / 100, v / 100);
    } else {
        assert(u / 100 == v / 100);
        lemma_two_digits_lt(u % 100, v % 100);
    }
}

/// For years `0..=9999` the text follows the lexicographic order of the
/// fields.
proof fn lemma_text_lt(a: CivilDateTime, b: CivilDateTime)
    requires
        small_fields(a),
        small_fields(b),
        a.second < 60,
        b.second < 60,
        a.minute < 60,
        b.minute < 60,
        0 <= a.year <= 9999,
        0 <= b.year <= 9999,
        civil_lt(a, b),
    ensures
        super::render::lex_lt(civil_text(a), civil_text(b)),
{
    let ya = year_text(a.year as int);
    let yb = year_text(b.year as int);
    assert(ya == four_digits(a.year as int));
    assert(yb == four_digits(b.year as int));
    if a.year < b.year {
        lemma_four_digits_lt(a.year as int, b.year as int);
        lemma_lex_concat(ya, yb, after_year_text(a), after_year_text(b));
        return;
    }
    lemma_lex_concat(ya, yb, after_year_text(a), after_year_text(b));
    // Same year: compare after the `-`.
    let dash = seq![b'-'];
    let ma = two_digits(a.month as int);
    let mb = two_digits(b.month as int);
    let ra = seq![b'-'] + (two_digits(a.day as int) + (seq![b' '] + time_text(a)));
    let rb = seq![b'-'] + (two_digits(b.day as int) + (seq![b' '] + time_text(b)));
    lemma_lex_concat(dash, dash, ma + ra, mb + rb);
    lemma_lex_concat(ma, mb, ra, rb);
    if a.month < b.month {
        lemma_two_digits_lt(a.month as int, b.month as int);
        return;
    }
    let da = two_digits(a.day as int);
    let db = two_digits(b.day as int);
    let sa = seq![b' '] + time_text(a);
    let sb = seq![b' '] + time_text(b);
    lemma_lex_concat(dash, dash, da + sa, db + sb);
    lemma_lex_concat(da, db, sa, sb);
    if a.day < b.day {
        lemma_two_digits_lt(a.day as int, b.day as int);
        return;
    }
    let space = seq![b' '];
    lemma_lex_concat(space, space, time_text(a), time_text(b));
    let colon = seq![b':'];
    let ha = two_digits(a.hour as int);
    let hb = two_digits(b.hour as int);
    let ta = seq![b':'] + (two_digits(a.minute as int) + (seq![b':'] + two_digits(a.second as int)));
    let tb = seq![b':'] + (two_digits(b.minute as int) + (seq![b':'] + two_digits(b.second as int)));
    lemma_lex_concat(ha, hb, ta, tb);
    if a.hour < b.hour {
        lemma_two_digits_lt(a.hour as int, b.hour as int);
        return;
    }
    let mia = two_digits(a.minute as int);
    let mib = two_digits(b.minute as int);
    let ua = seq![b':'] + two_digits(a.second as int);
    let ub = seq![b':'] + two_digits(b.second as int);
    lemma_lex_concat(colon, colon, mia + ua, mib + ub);
    lemma_lex_concat(mia, mib, ua, ub);
    if a.minute < b.minute {
        lemma_two_digits_lt(a.minute as int, b.minute as int);
        return;
    }
    lemma_lex_concat(colon, colon, two_digits(a.second as int), two_digits(b.second as int));
    lemma_two_digits_lt(a.second as int, b.second as int);
}

/// Valid civil date-times are equal, or one precedes the other.
proof fn lemma_civil_trichotomy(a: CivilDateTime, b: CivilDateTime)
    requires
        a.is_valid(),
        b.is_valid(),
    ensures
        a == b || civil_lt(a, b) || civil_lt(b, a),
{
    if !civil_lt(a, b) && !civil_lt(b, a) {
        assert(a.time_of_day() == b.time_of_day());
        assert(a.hour == b.hour);
        assert(a.minute == b.minute);
    }
}

/// For years `0..=9999`, the byte order of the texts is exactly the order of
/// the Unix times: sorting the text sorts by time, and distinct times give
/// distinct texts.
pub proof fn lemma_text_order(a: CivilDateTime, b: CivilDateTime)
    requires
        a.is_valid(),
        b.is_valid(),
        0 <= a.year <= 9999,
        0 <= b.year <= 9999,
    ensures
        a.unix_seconds() < b.unix_seconds() <==> super::render::lex_lt(civil_text(a), civil_text(b)),
{
    lemma_civil_trichotomy(a, b);
    super::render::lemma_lex_strict_total(civil_text(a), civil_text(b));
    if civil_lt(a, b) {
        lemma_civil_order(a, b);
        lemma_text_lt(a, b);
    } else if civil_lt(b, a) {
        lemma_civil_order(b, a);
        lemma_text_lt(b, a);
    }
}

/// The value of two digits.
proof fn lemma_two_digits_value(v: int)
    requires
        0 <= v < 100,
    ensures
        super::exclusions::all_digits(two_digits(v)),
        super::exclusions::decimal_value(two_digits(v)) == v,
{
    let s = two_digits(v);
    let first = seq![digit(v / 10)];
    assert(s.drop_last() =~= first);
    assert(first.drop_last() =~= Seq::<u8>::empty());
    assert(super::exclusions::decimal_value(Seq::<u8>::empty()) == 0);
    assert(first.last() as int == 48 + v / 10);
    assert(super::exclusions::decimal_value(first) == v / 10);
}

/// The value of four digits.
proof fn lemma_four_digits_value(v: int)
    requires
        0 <= v <= 9999,
    ensures
        super::exclusions::all_digits(four_digits(v)),
        super::exclusions::decimal_value(four_digits(v)) == v,
{
    let s = four_digits(v);
    let s3 = seq![digit(v / 1000), digit((v / 100) % 10), digit((v / 10) % 10)];
    let s2 = seq![digit(v / 1000), digit((v / 100) % 10)];
    lemma_two_digits_value(v / 100);
    assert((v / 100) / 10 == v / 1000);
    assert(s2 =~= two_digits(v / 100));
    assert(s.drop_last() =~= s3);
    assert(s3.drop_last() =~= s2);
    assert(super::exclusions::decimal_value(s3) == (v / 100) * 10 + (v / 10) % 10);
}

/// The layout of the text after the year: 15 bytes, `-MM-DD HH:MM:SS`.
proof fn lemma_after_year_layout(c: CivilDateTime)
    ensures
        after_year_text(c).len() == 15,
        after_year_text(c)[0] == b'-' && after_year_text(c)[3] == b'-',
        after_year_text(c)[6] == b' ',
        after_year_text(c)[9] == b':' && after_year_text(c)[12] == b':',
        after_year_text(c).subrange(1, 3) == two_digits(c.month as int),
        after_year_text(c).subrange(4, 6) == two_digits(c.day as int),
        after_year_text(c).subrange(7, 9) == two_digits(c.hour as int),
        after_year_text(c).subrange(10, 12) == two_digits(c.minute as int),
        after_year_text(c).subrange(13, 15) == two_digits(c.second as int),
{
    let r = after_year_text(c);
    assert(r =~= seq![b'-'] + two_digits(c.month as int) + seq![b'-'] + two_digits(c.day as int)
        + seq![b' '] + two_digits(c.hour as int) + seq![b':'] + two_digits(c.minute as int)
        + seq![b':'] + two_digits(c.second as int));
    assert(r.subrange(1, 3) =~= two_digits(c.month as int));
    assert(r.subrange(4, 6) =~= two_digits(c.day as int));
    assert(r.subrange(7, 9) =~= two_digits(c.hour as int));
    assert(r.subrange(10, 12) =~= two_digits(c.minute as int));
    assert(r.subrange(13, 15) =~= two_digits(c.second as int));
}

/// For years `0..=9999` the text is 19 ASCII bytes, `YYYY-MM-DD HH:MM:SS`:
/// separators at fixed positions, and each field's digits are its value.
pub proof fn lemma_civil_text_shape(c: CivilDateTime)
    requires
        small_fields(c),
        0 <= c.year <= 9999,
    ensures
        civil_text(c).len() == 19,
        civil_text(c)[4] == b'-' && civil_text(c)[7] == b'-',
        civil_text(c)[10] == b' ',
        civil_text(c)[13] == b':' && civil_text(c)[16] == b':',
        super::exclusions::all_digits(civil_text(c).subrange(0, 4)),
        super::exclusions::decimal_value(civil_text(c).subrange(0, 4)) == c.year,
        super::exclusions::all_digits(civil_text(c).subrange(5, 7)),
        super::exclusions::decimal_value(civil_text(c).subrange(5, 7)) == c.month,
        super::exclusions::all_digits(civil_text(c).subrange(8, 10)),
        super::exclusions::decimal_value(civil_text(c).subrange(8, 10)) == c.day,
        super::exclusions::all_digits(civil_text(c).subrange(11, 13)),
        super::exclusions::decimal_value(civil_text(c).subrange(11, 13)) == c.hour,
        super::exclusions::all_digits(civil_text(c).subrange(14, 16)),
        super::exclusions::decimal_value(civil_text(c).subrange(14, 16)) == c.minute,
        super::exclusions::all_digits(civil_text(c).subrange(17, 19)),
        super::exclusions::decimal_value(civil_text(c).subrange(17, 19)) == c.second,
{
    let t = civil_text(c);
    let y = four_digits(c.year as int);
    let r = after_year_text(c);
    assert(year_text(c.year as int) == y);
    lemma_after_year_layout(c);
    lemma_four_digits_value(c.year as int);
    lemma_two_digits_value(c.month as int);
    lemma_two_digits_value(c.day as int);
    lemma_two_digits_value(c.hour as int);
    lemma_two_digits_value(c.minute as int);
    lemma_two_digits_value(c.second as int);
    assert(t.subrange(0, 4) =~= y);
    assert(t.subrange(5, 7) =~= r.subrange(1, 3));
    assert(t.subrange(8, 10) =~= r.subrange(4, 6));
    assert(t.subrange(11, 13) =~= r.subrange(7, 9));
    assert(t.subrange(14, 16) =~= r.subrange(10, 12));
    assert(t.subrange(17, 19) =~= r.subrange(13, 15));
    assert(t[4] == r[0] && t[7] == r[3] && t[10] == r[6] && t[13] == r[9] && t[16] == r[12]);
}

/// Canonical decimal text of a value of at least 10000 has at least five
/// digits.
proof fn lemma_decimal_long(v: nat)
    requires
        v > 9999,
    ensures
        super::exclusions::decimal_text(v).len() >= 5,
{
    assert(super::exclusions::decimal_text(v).len() == super::exclusions::decimal_text(v / 10).len() + 1);
    assert(super::exclusions::decimal_text(v / 10).len() == super::exclusions::decimal_text(v / 100).len() + 1);
    assert(super::exclusions::decimal_text(v / 100).len() == super::exclusions::decimal_text(v / 1000).len() + 1);
    assert(super::exclusions::decimal_text(v / 1000).len() == super::exclusions::decimal_text(v / 10000).len()
        + 1);
    super::exclusions::lemma_decimal_text(v / 10000);
}

/// `padded4` is injective.
proof fn lemma_padded4_injective(u: nat, v: nat)
    requires
        padded4(u) == padded4(v),
    ensures
        u == v,
{
    if u <= 9999 && v <= 9999 {
        lemma_four_digits_value(u as int);
        lemma_four_digits_value(v as int);
    } else if u > 9999 && v > 9999 {
        super::exclusions::lemma_decimal_text(u);
        super::exclusions::lemma_decimal_text(v);
    } else if u > 9999 {
        lemma_decimal_long(u);
    } else {
        lemma_decimal_long(v);
    }
}

/// `%Y` is injective: the sign (or first digit) tells the case apart.
proof fn lemma_year_text_injective(y1: int, y2: int)
    requires
        year_text(y1) == year_text(y2),
    ensures
        y1 == y2,
{
    let t = year_text(y1);
    let m1: nat = if y1 < 0 {
        (-y1) as nat
    } else {
        y1 as nat
    };
    let m2: nat = if y2 < 0 {
        (-y2) as nat
    } else {
        y2 as nat
    };
    if y1 < 0 || y1 > 9999 {
        assert(t.drop_first() =~= padded4(m1));
        assert(t[0] == if y1 < 0 {
            b'-'
        } else {
            b'+'
        });
    }
    if y2 < 0 || y2 > 9999 {
        assert(t.drop_first() =~= padded4(m2));
        assert(t[0] == if y2 < 0 {
            b'-'
        } else {
            b'+'
        });
    }
    // The first byte: `-` (45), `+` (43), or a digit.
    if 0 <= y1 <= 9999 {
        assert(t[0] == digit(y1 / 1000));
    }
    if 0 <= y2 <= 9999 {
        assert(t[0] == digit(y2 / 1000));
    }
    if (y1 < 0) == (y2 < 0) && (y1 > 9999) == (y2 > 9999) {
        lemma_padded4_injective(m1, m2);
    }
}

/// The text names the civil date-time: distinct date-times (with fields
/// below 100, as every valid one has) have distinct texts, in every year.
/// With `lemma_civil_unique`, distinct Unix seconds give distinct texts.
pub proof fn lemma_civil_text_injective(a: CivilDateTime, b: CivilDateTime)
    requires
        small_fields(a),
        small_fields(b),
        civil_text(a) == civil_text(b),
    ensures
        a == b,
{
    let ya = year_text(a.year as int);
    let yb = year_text(b.year as int);
    let ra = after_year_text(a);
    let rb = after_year_text(b);
    assert(ra.len() == 15 && rb.len() == 15);
    let t = civil_text(a);
    assert(ya =~= t.subrange(0, ya.len() as int));
    assert(yb =~= t.subrange(0, yb.len() as int));
    lemma_year_text_injective(a.year as int, b.year as int);
    assert(ra =~= rb);
    lemma_after_year_layout(a);
    lemma_after_year_layout(b);
    lemma_two_digits_value(a.month as int);
    lemma_two_digits_value(b.month as int);
    lemma_two_digits_value(a.day as int);
    lemma_two_digits_value(b.day as int);
    lemma_two_digits_value(a.hour as int);
    lemma_two_digits_value(b.hour as int);
    lemma_two_digits_value(a.minute as int);
    lemma_two_digits_value(b.minute as int);
    lemma_two_digits_value(a.second as int);
    lemma_two_digits_value(b.second as int);
}

} // verus!
