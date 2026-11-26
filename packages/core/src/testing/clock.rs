//! A clock that tests control.

use std::sync::Mutex;

use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::ports::clock::Clock;

/// A fixed time that only moves when a test calls [`FixedClock::set`] or
/// [`FixedClock::advance`]. Starts at 2026-01-01T00:00:00Z.
#[derive(Debug)]
pub struct FixedClock(Mutex<DateTime<Utc>>);

impl FixedClock {
    pub fn new(now: DateTime<Utc>) -> Self {
        Self(Mutex::new(now))
    }

    pub fn set(&self, now: DateTime<Utc>) {
        *self.0.lock().expect("clock lock") = now;
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.0.lock().expect("clock lock");
        *now += by;
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::new(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("a valid fixed start time"),
        )
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().expect("clock lock")
    }
}
