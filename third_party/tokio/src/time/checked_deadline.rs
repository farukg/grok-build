use crate::runtime::scheduler;
use crate::time::error::{DeadlineError, Error};
use crate::time::{Instant, Sleep};
use crate::util::trace;

/// An immutable deadline validated against, and bound to, one time driver.
///
/// Clones and waits retain the original instant and driver, even when moved or
/// polled in another runtime. Validation includes millisecond rounding and the
/// driver's elapsed lifetime, not just the duration from the current instant.
#[derive(Debug, Clone)]
pub struct CheckedDeadline {
    handle: scheduler::Handle,
    deadline: Instant,
}

impl CheckedDeadline {
    /// Validates `deadline` against the current runtime's actual time driver.
    ///
    /// Returns a typed error if the runtime or time driver is unavailable, if
    /// rounding overflows `Instant`, or if the rounded tick is out of range.
    /// Past deadlines are accepted, just as with [`sleep_until`](super::sleep_until).
    pub fn new(deadline: Instant) -> Result<Self, DeadlineError> {
        #[cfg(feature = "rt")]
        {
            let handle = crate::runtime::Handle::try_current()
                .map_err(|_| DeadlineError::MissingRuntime)?
                .inner;
            let time = handle.driver().try_time().ok_or(DeadlineError::TimeDisabled)?;
            time.time_source().checked_deadline_to_tick(deadline)?;
            Ok(Self { handle, deadline })
        }
        #[cfg(not(feature = "rt"))]
        {
            let _ = deadline;
            Err(DeadlineError::MissingRuntime)
        }
    }

    /// Returns the original, unrounded instant supplied at construction.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Waits for the original deadline using the driver captured at construction.
    ///
    /// The bound runtime must continue driving its timers, including when this
    /// future is polled elsewhere. Waiting does not extend the runtime's lifetime.
    /// Owner shutdown and timer registration errors return the original
    /// [`Error`]. A timer that actually elapsed before shutdown still succeeds.
    /// A living runtime that is not driving its timers may leave this pending.
    ///
    /// Dropping this future cancels only this wait. The deadline and its clones
    /// remain usable; each wait registers its own timer on the same driver.
    pub async fn wait(&self) -> Result<(), Error> {
        let sleep = Sleep::new_timeout_with_handle(
            self.handle.clone(),
            self.deadline,
            trace::caller_location(),
        );
        crate::pin!(sleep);
        std::future::poll_fn(|cx| sleep.as_mut().poll_checked(cx)).await
    }
}

#[cfg(all(test, feature = "test-util", not(loom)))]
#[path = "checked_deadline_tests.rs"]
mod tests;

#[cfg(all(test, feature = "test-util", not(loom)))]
#[path = "checked_deadline_wait_tests.rs"]
mod wait_tests;
