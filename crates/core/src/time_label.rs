//! Relative time labels for cards: "now", "5m", "2h", "Yesterday", "Mon",
//! "3 Nov" (and "3 Nov 2025" for another year).
//!
//! Calendar maths is done on local civil dates derived from the clock's UTC
//! offset, so labels are deterministic under a fake clock.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::clock::Clock;

const SECS_PER_DAY: i64 = 86_400;
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Label for a notification received at `at`, as seen at `clock.now()`.
///
/// Timestamps in the future (clock skew) read as "now".
#[must_use]
pub fn time_label(at: SystemTime, clock: &dyn Clock) -> String {
    let now = clock.now();
    let age = now.duration_since(at).unwrap_or(Duration::ZERO).as_secs();
    if age < 60 {
        return "now".to_owned();
    }
    if age < 3600 {
        return format!("{}m", age / 60);
    }
    let today = local_day(now, clock.utc_offset_secs(now));
    let day = local_day(at, clock.utc_offset_secs(at));
    match today - day {
        0 => format!("{}h", age / 3600),
        1 => "Yesterday".to_owned(),
        2..=6 => WEEKDAYS[weekday(day)].to_owned(),
        _ => {
            let (y, m, d) = civil_from_days(day);
            let (this_year, _, _) = civil_from_days(today);
            let month = MONTHS[usize::from(m - 1)];
            if y == this_year {
                format!("{d} {month}")
            } else {
                format!("{d} {month} {y}")
            }
        }
    }
}

/// Days since 1970-01-01 in local time.
fn local_day(t: SystemTime, offset_secs: i32) -> i64 {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX / 2),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX / 2),
    };
    (secs + i64::from(offset_secs)).div_euclid(SECS_PER_DAY)
}

/// Index into `WEEKDAYS` (0 = Sunday). 1970-01-01 was a Thursday.
fn weekday(day: i64) -> usize {
    // rem_euclid(7) is in 0..7, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let idx = (day + 4).rem_euclid(7) as usize;
    idx
}

/// Proleptic Gregorian (year, month 1..=12, day 1..=31) for days since the
/// epoch. Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // d is in 1..=31 and m in 1..=12 by construction.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (y, m as u8, d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;

    // 2026-11-04 (Wednesday) 15:00:00 UTC.
    const NOW: u64 = 1_793_804_400;

    fn label_ago(secs: u64) -> String {
        let clock = FakeClock::at_unix(NOW);
        time_label(UNIX_EPOCH + Duration::from_secs(NOW - secs), &clock)
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(
            civil_from_days(i64::try_from(NOW).unwrap() / SECS_PER_DAY),
            (2026, 11, 4)
        );
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(WEEKDAYS[weekday(0)], "Thu");
        assert_eq!(
            WEEKDAYS[weekday(i64::try_from(NOW).unwrap() / SECS_PER_DAY)],
            "Wed"
        );
    }

    #[test]
    fn minutes_and_hours() {
        assert_eq!(label_ago(0), "now");
        assert_eq!(label_ago(59), "now");
        assert_eq!(label_ago(60), "1m");
        assert_eq!(label_ago(5 * 60 + 30), "5m");
        assert_eq!(label_ago(3599), "59m");
        assert_eq!(label_ago(3600), "1h");
        assert_eq!(label_ago(2 * 3600 + 59 * 60), "2h");
        assert_eq!(label_ago(15 * 3600), "15h");
    }

    #[test]
    fn days_weekdays_and_dates() {
        assert_eq!(label_ago(15 * 3600 + 1), "Yesterday");
        assert_eq!(label_ago(30 * 3600), "Yesterday");
        assert_eq!(label_ago(2 * 86_400), "Mon");
        assert_eq!(label_ago(6 * 86_400), "Thu");
        assert_eq!(label_ago(7 * 86_400), "28 Oct");
        assert_eq!(label_ago(400 * 86_400), "30 Sep 2025");
    }

    #[test]
    fn future_timestamps_read_now() {
        let clock = FakeClock::at_unix(NOW);
        assert_eq!(
            time_label(UNIX_EPOCH + Duration::from_secs(NOW + 500), &clock),
            "now"
        );
    }

    #[test]
    fn utc_offset_moves_the_day_boundary() {
        // 15:00 UTC; 14h ago is 01:00 UTC the same day...
        assert_eq!(label_ago(14 * 3600), "14h");
        // ...but at UTC-2 it was 23:00 the previous local day.
        let clock = FakeClock::at_unix(NOW);
        clock.set_utc_offset_secs(-2 * 3600);
        assert_eq!(
            time_label(UNIX_EPOCH + Duration::from_secs(NOW - 14 * 3600), &clock),
            "Yesterday"
        );
    }

    #[test]
    fn pre_epoch_timestamps_do_not_panic() {
        let clock = FakeClock::new(UNIX_EPOCH + Duration::from_secs(10));
        let label = time_label(UNIX_EPOCH - Duration::from_secs(86_400 * 40), &clock);
        assert_eq!(label, "22 Nov 1969");
    }
}
