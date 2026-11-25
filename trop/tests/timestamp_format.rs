//! Differential tests for the verified timestamp text
//! (`verified::civil_time`, behind `trop::output::format_utc_timestamp` and
//! the CLI's `format_timestamp`).
//!
//! The kernel proves the calendar conversion and the text layout; byte
//! equality with chrono's `%Y-%m-%d %H:%M:%S` (which the CLI used before)
//! is pinned here over chrono's whole range, and an independent
//! closed-form algorithm covers the instants chrono cannot represent.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use proptest::prelude::*;
use trop::output::format_utc_timestamp;

/// What the CLI printed before it used the kernel.
fn chrono_text(time: SystemTime) -> String {
    let dt: DateTime<Utc> = time.into();
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// The instant `secs` seconds (floor) plus `nanos` after the epoch, if the
/// platform's `SystemTime` can hold it.
fn instant(secs: i128, nanos: u32) -> Option<SystemTime> {
    let base = if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(secs).ok()?))?
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(u64::try_from(-secs).ok()?))?
    };
    base.checked_add(Duration::from_nanos(u64::from(nanos)))
}

/// An independent reference: Howard Hinnant's closed-form
/// `civil_from_days`, with chrono's sign rule for `%Y`.
fn reference_text(secs: i128) -> String {
    let days = secs.div_euclid(86_400);
    let second_of_day = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i128::from(month <= 2);
    let year_text = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else {
        format!("{year:+05}")
    };
    format!(
        "{year_text}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

fn chrono_min_secs() -> i128 {
    i128::from(DateTime::<Utc>::MIN_UTC.timestamp())
}

fn chrono_max_secs() -> i128 {
    i128::from(DateTime::<Utc>::MAX_UTC.timestamp())
}

fn assert_matches_chrono(secs: i128, nanos: u32) {
    let time = instant(secs, nanos).expect("instant within SystemTime");
    let ours = format_utc_timestamp(time);
    assert_eq!(ours, chrono_text(time), "secs {secs} nanos {nanos}");
    assert_eq!(ours, reference_text(secs), "secs {secs} nanos {nanos}");
}

#[test]
fn known_instants() {
    let cases: &[(i128, &str)] = &[
        (0, "1970-01-01 00:00:00"),
        (-1, "1969-12-31 23:59:59"),
        (1_705_323_045, "2024-01-15 12:50:45"),
        (951_782_400, "2000-02-29 00:00:00"),
        (951_868_799, "2000-02-29 23:59:59"),
        (951_868_800, "2000-03-01 00:00:00"),
        (-2_203_891_200, "1900-03-01 00:00:00"),
        (4_107_456_000, "2100-02-28 00:00:00"),
        (4_107_542_400, "2100-03-01 00:00:00"),
        (2_147_483_647, "2038-01-19 03:14:07"),
        (2_147_483_648, "2038-01-19 03:14:08"),
        (4_294_967_295, "2106-02-07 06:28:15"),
        (253_402_300_799, "9999-12-31 23:59:59"),
        (253_402_300_800, "+10000-01-01 00:00:00"),
        (-62_167_219_200, "0000-01-01 00:00:00"),
        (-62_167_219_201, "-0001-12-31 23:59:59"),
        (-62_135_596_800, "0001-01-01 00:00:00"),
        (8_210_266_876_799, "+262142-12-31 23:59:59"),
    ];
    for &(secs, text) in cases {
        let time = instant(secs, 0).unwrap();
        assert_eq!(format_utc_timestamp(time), text, "secs {secs}");
        assert_matches_chrono(secs, 0);
    }
    assert_eq!(
        format_utc_timestamp(instant(chrono_min_secs(), 0).unwrap()),
        "-262143-01-01 00:00:00"
    );
    assert_matches_chrono(chrono_min_secs(), 0);
    assert_matches_chrono(chrono_max_secs(), 999_999_999);
}

/// Sub-second instants round down to the containing second, before the
/// epoch too.
#[test]
fn fractions_round_down() {
    for &(secs, nanos) in &[
        (0, 1),
        (0, 999_999_999),
        (-1, 1),
        (-1, 999_999_999),
        (-86_401, 500_000_000),
        (1_705_323_045, 999_999_999),
    ] {
        assert_matches_chrono(secs, nanos);
    }
    assert_eq!(
        format_utc_timestamp(UNIX_EPOCH - Duration::from_nanos(1)),
        "1969-12-31 23:59:59"
    );
}

/// Every day of a full 400-year cycle on both sides of the epoch, at its
/// first and last instant.
#[test]
fn every_day_of_two_cycles() {
    for day in -146_097i128..=146_097 {
        assert_matches_chrono(day * 86_400, 0);
        assert_matches_chrono(day * 86_400 + 86_399, 999_999_999);
    }
}

/// Fixed-width text sorts like time for years `0..=9999`.
#[test]
fn text_order_is_time_order() {
    let mut previous = format_utc_timestamp(instant(-62_167_219_200, 0).unwrap());
    let mut secs = -62_167_219_200i128;
    while secs < 253_402_300_799 {
        secs += 7_777_777;
        let current = format_utc_timestamp(instant(secs.min(253_402_300_799), 0).unwrap());
        assert_eq!(current.len(), 19);
        assert!(previous < current, "{previous} !< {current}");
        previous = current;
    }
}

/// Beyond chrono's range the old code panicked; the kernel continues the
/// same calendar and sign rule.
#[test]
fn beyond_chrono_range() {
    let max = chrono_max_secs();
    let min = chrono_min_secs();
    let after = instant(max + 1, 0).unwrap();
    assert!(std::panic::catch_unwind(|| chrono_text(after)).is_err());
    assert_eq!(format_utc_timestamp(after), "+262143-01-01 00:00:00");
    let before = instant(min - 1, 0).unwrap();
    assert!(std::panic::catch_unwind(|| chrono_text(before)).is_err());
    assert_eq!(format_utc_timestamp(before), "-262144-12-31 23:59:59");
    for secs in [
        max + 1,
        min - 1,
        i128::from(i64::MAX),
        i128::from(i64::MIN),
        i128::from(i64::MIN) + 1,
        i128::from(i32::MAX) * 86_400 * 365,
    ] {
        for nanos in [0, 1, 999_999_999] {
            if let Some(time) = instant(secs, nanos) {
                assert_eq!(
                    format_utc_timestamp(time),
                    reference_text(secs),
                    "secs {secs}"
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    #[test]
    fn random_instants_match_chrono(
        secs in chrono_min_secs()..=chrono_max_secs(),
        nanos in 0u32..1_000_000_000,
    ) {
        let time = instant(secs, nanos).unwrap();
        prop_assert_eq!(format_utc_timestamp(time), chrono_text(time));
    }

    #[test]
    fn random_instants_near_now_match_chrono(
        secs in -4_000_000_000i128..=8_000_000_000,
        nanos in 0u32..1_000_000_000,
    ) {
        let time = instant(secs, nanos).unwrap();
        prop_assert_eq!(format_utc_timestamp(time), chrono_text(time));
    }

    #[test]
    fn huge_instants_match_reference(secs in any::<i64>(), nanos in 0u32..1_000_000_000) {
        let secs = i128::from(secs);
        if let Some(time) = instant(secs, nanos) {
            prop_assert_eq!(format_utc_timestamp(time), reference_text(secs));
        }
    }
}
