//! Time source port.

use chrono::{DateTime, SubsecRound, Utc};

/// The current time. Services and repositories read time only through this port, so tests can
/// fix it (`testing::FixedClock`).
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// The system clock, truncated to microseconds (the precision of Postgres `timestamptz`), so a
/// value read back from the database equals the value that was written.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(6)
    }
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    #[test]
    fn truncates_to_microseconds() {
        let now = SystemClock.now();
        assert_eq!(now.nanosecond() % 1_000, 0);
    }
}
