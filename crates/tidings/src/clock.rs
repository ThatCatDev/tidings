//! The real clock: system time, an `Instant`-based monotonic origin and the
//! local time zone (via chrono, which reads the system tz database).

use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, Offset, TimeZone, Utc};
use tidings_core::clock::Clock;

/// Wall clock + monotonic clock + local UTC offset.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn utc_offset_secs(&self, at: SystemTime) -> i32 {
        let utc: DateTime<Utc> = at.into();
        Local
            .offset_from_utc_datetime(&utc.naive_utc())
            .fix()
            .local_minus_utc()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_advances_and_offset_is_sane() {
        let c = SystemClock::default();
        let a = c.monotonic();
        assert!(c.monotonic() >= a);
        assert!(c.utc_offset_secs(c.now()).abs() <= 14 * 3600);
    }
}
