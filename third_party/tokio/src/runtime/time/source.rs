use super::MAX_SAFE_MILLIS_DURATION;
use crate::time::error::DeadlineError;
use crate::time::{Clock, Duration, Instant};

const DEADLINE_ROUNDING: Duration = Duration::from_nanos(999_999);

/// A structure which handles conversion from Instants to `u64` timestamps.
#[derive(Debug)]
pub(crate) struct TimeSource {
    start_time: Instant,
}

impl TimeSource {
    pub(crate) fn new(clock: &Clock) -> Self {
        Self {
            start_time: clock.now(),
        }
    }

    pub(crate) fn deadline_to_tick(&self, t: Instant) -> u64 {
        // Round up to the end of a ms
        self.instant_to_tick(t + DEADLINE_ROUNDING)
    }

    pub(crate) fn checked_deadline_to_tick(&self, t: Instant) -> Result<u64, DeadlineError> {
        let rounded = t
            .checked_add(DEADLINE_ROUNDING)
            .ok_or(DeadlineError::RoundingOverflow)?;
        self.checked_instant_to_tick(rounded)
    }

    pub(crate) fn instant_to_tick(&self, t: Instant) -> u64 {
        self.checked_instant_to_tick(t)
            .unwrap_or(MAX_SAFE_MILLIS_DURATION)
    }

    fn checked_instant_to_tick(&self, t: Instant) -> Result<u64, DeadlineError> {
        let Some(dur) = t.checked_duration_since(self.start_time) else {
            return Ok(0);
        };
        let ms = dur.as_millis();
        if ms > u128::from(MAX_SAFE_MILLIS_DURATION) {
            return Err(DeadlineError::TickOutOfRange);
        }
        u64::try_from(ms).map_err(|_| DeadlineError::TickOutOfRange)
    }

    pub(crate) fn tick_to_duration(&self, t: u64) -> Duration {
        Duration::from_millis(t)
    }

    pub(crate) fn now(&self, clock: &Clock) -> u64 {
        self.instant_to_tick(clock.now())
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn start_time(&self) -> Instant {
        self.start_time
    }
}
