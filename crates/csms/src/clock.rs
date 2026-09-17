//! The CSMS's notion of "now", as unix time.

use std::time::{SystemTime, UNIX_EPOCH};

use ocpp::DateTime;
use tokio::time::Instant;

/// Unix time that moves with tokio's clock: the wall time when the clock was made, plus the
/// tokio time elapsed since.
///
/// It never jumps back when the system clock is adjusted, and under `tokio::time::pause` it
/// advances with the paused clock, so tests can run hours of planning in no time.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    start_unix_ms: i64,
    start: Instant,
}

impl Clock {
    /// A clock that starts at the current wall time.
    pub fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after 1970");
        Self::starting_at_ms(i64::try_from(now.as_millis()).expect("a sane date"))
    }

    /// A clock that starts at `unix_ms`.
    pub fn starting_at_ms(unix_ms: i64) -> Self {
        Self {
            start_unix_ms: unix_ms,
            start: Instant::now(),
        }
    }

    /// The current unix time in milliseconds.
    pub fn now_ms(&self) -> i64 {
        let elapsed = i64::try_from(self.start.elapsed().as_millis()).unwrap_or(i64::MAX);
        self.start_unix_ms.saturating_add(elapsed)
    }

    /// The current unix time in whole seconds.
    pub fn now_secs(&self) -> i64 {
        self.now_ms().div_euclid(1000)
    }

    /// The current time as an OCPP timestamp.
    pub fn now(&self) -> DateTime {
        chrono::DateTime::from_timestamp_millis(self.now_ms())
            .expect("a representable date")
            .into()
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn follows_tokio_time() {
        let clock = Clock::starting_at_ms(1_790_000_000_500);
        assert_eq!(clock.now_secs(), 1_790_000_000);
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(clock.now_secs(), 1_790_003_600);
        assert_eq!(clock.now().to_string(), "2026-09-21T15:13:20.500Z");
    }
}
