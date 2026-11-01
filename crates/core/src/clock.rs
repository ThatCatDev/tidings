//! Injected time source.
//!
//! Expiry deadlines use a monotonic clock so they survive wall-clock jumps
//! (the target machine's RTC can boot at 1970 until NTP syncs); time labels
//! use wall-clock time plus the local UTC offset.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// Source of wall-clock time, monotonic time and the local UTC offset.
pub trait Clock: Send + Sync + Debug {
    /// Current wall-clock time.
    fn now(&self) -> SystemTime;

    /// Monotonic time since an arbitrary fixed origin.
    fn monotonic(&self) -> Duration;

    /// Local UTC offset in seconds (east positive) in effect at `at`.
    fn utc_offset_secs(&self, at: SystemTime) -> i32;
}

/// A manually driven clock for tests: time only moves when told to.
///
/// Clones share the same underlying time, so a test can keep one handle and
/// give another to the code under test.
#[derive(Debug, Clone)]
pub struct FakeClock {
    inner: Arc<Mutex<FakeTime>>,
}

#[derive(Debug, Clone, Copy)]
struct FakeTime {
    wall: SystemTime,
    mono: Duration,
    offset_secs: i32,
}

impl FakeClock {
    /// A clock reading `wall`, with monotonic time 0 and a UTC offset of 0.
    #[must_use]
    pub fn new(wall: SystemTime) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeTime {
                wall,
                mono: Duration::ZERO,
                offset_secs: 0,
            })),
        }
    }

    /// A clock reading `secs` seconds after the Unix epoch.
    #[must_use]
    pub fn at_unix(secs: u64) -> Self {
        Self::new(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Moves both wall-clock and monotonic time forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut t = self.lock();
        t.wall += by;
        t.mono += by;
    }

    /// Sets the wall-clock time without touching monotonic time (a clock jump).
    pub fn set_wall(&self, wall: SystemTime) {
        self.lock().wall = wall;
    }

    /// Sets the local UTC offset reported for every instant.
    pub fn set_utc_offset_secs(&self, offset_secs: i32) {
        self.lock().offset_secs = offset_secs;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeTime> {
        // The guarded data is plain `Copy` values, so a poisoned lock is still consistent.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        self.lock().wall
    }

    fn monotonic(&self) -> Duration {
        self.lock().mono
    }

    fn utc_offset_secs(&self, _at: SystemTime) -> i32 {
        self.lock().offset_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_moves_both_clocks_and_clones_share_time() {
        let c = FakeClock::at_unix(100);
        let other = c.clone();
        c.advance(Duration::from_secs(5));
        assert_eq!(
            other.now(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(105)
        );
        assert_eq!(other.monotonic(), Duration::from_secs(5));
    }

    #[test]
    fn wall_jump_leaves_monotonic_alone() {
        let c = FakeClock::at_unix(0);
        c.set_wall(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000));
        assert_eq!(c.monotonic(), Duration::ZERO);
        c.set_utc_offset_secs(3600);
        assert_eq!(c.utc_offset_secs(SystemTime::UNIX_EPOCH), 3600);
    }
}
