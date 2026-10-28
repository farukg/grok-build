//! Time error types.

use std::error;
use std::fmt;

/// Errors encountered by the timer implementation.
///
/// Currently, there are two different errors that can occur:
///
/// * `shutdown` occurs when a timer operation is attempted, but the timer
///   instance has been dropped. In this case, the operation will never be able
///   to complete and the `shutdown` error is returned. This is a permanent
///   error, i.e., once this error is observed, timer operations will never
///   succeed in the future.
///
/// * `at_capacity` occurs when a timer operation is attempted, but the timer
///   instance is currently handling its maximum number of outstanding sleep instances.
///   In this case, the operation is not able to be performed at the current
///   moment, and `at_capacity` is returned. This is a transient error, i.e., at
///   some point in the future, if the operation is attempted again, it might
///   succeed. Callers that observe this error should attempt to [shed load]. One
///   way to do this would be dropping the future that issued the timer operation.
///
/// [shed load]: https://en.wikipedia.org/wiki/Load_Shedding
#[derive(Debug, Copy, Clone)]
pub struct Error(Kind);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Kind {
    Shutdown = 1,
    AtCapacity = 2,
    Invalid = 3,
}

impl From<Kind> for Error {
    fn from(k: Kind) -> Self {
        Error(k)
    }
}

/// Errors returned by `Timeout`.
///
/// This error is returned when a timeout expires before the function was able
/// to finish.
#[derive(Debug, PartialEq, Eq)]
pub struct Elapsed(());

#[derive(Debug)]
pub(crate) enum InsertError {
    Elapsed,
}

// ===== impl Error =====

impl Error {
    /// Creates an error representing a shutdown timer.
    pub fn shutdown() -> Error {
        Error(Kind::Shutdown)
    }

    /// Returns `true` if the error was caused by the timer being shutdown.
    pub fn is_shutdown(&self) -> bool {
        matches!(self.0, Kind::Shutdown)
    }

    /// Creates an error representing a timer at capacity.
    pub fn at_capacity() -> Error {
        Error(Kind::AtCapacity)
    }

    /// Returns `true` if the error was caused by the timer being at capacity.
    pub fn is_at_capacity(&self) -> bool {
        matches!(self.0, Kind::AtCapacity)
    }

    /// Creates an error representing a misconfigured timer.
    pub fn invalid() -> Error {
        Error(Kind::Invalid)
    }

    /// Returns `true` if the error was caused by the timer being misconfigured.
    pub fn is_invalid(&self) -> bool {
        matches!(self.0, Kind::Invalid)
    }
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let descr = match self.0 {
            Kind::Shutdown => {
                "the timer is shutdown, must be called from the context of Tokio runtime"
            }
            Kind::AtCapacity => "timer is at capacity and cannot create a new entry",
            Kind::Invalid => "timer duration exceeds maximum duration",
        };
        write!(fmt, "{descr}")
    }
}

// ===== impl Elapsed =====

impl Elapsed {
    pub(crate) fn new() -> Self {
        Elapsed(())
    }
}

impl fmt::Display for Elapsed {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        "deadline has elapsed".fmt(fmt)
    }
}

impl std::error::Error for Elapsed {}

impl From<Elapsed> for std::io::Error {
    fn from(_err: Elapsed) -> std::io::Error {
        std::io::ErrorKind::TimedOut.into()
    }
}

/// Failure to construct a [`CheckedDeadline`](crate::time::CheckedDeadline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineError {
    /// No current Tokio runtime is accessible.
    MissingRuntime,
    /// The current runtime has no time driver enabled.
    TimeDisabled,
    /// Rounding the requested instant up to timer resolution overflows `Instant`.
    RoundingOverflow,
    /// The rounded deadline exceeds the actual driver's safe tick range.
    TickOutOfRange,
}

impl fmt::Display for DeadlineError {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::MissingRuntime => "no current Tokio runtime is accessible",
            Self::TimeDisabled => "the current Tokio runtime has no time driver enabled",
            Self::RoundingOverflow => "rounding the deadline overflows Instant",
            Self::TickOutOfRange => "the deadline exceeds the time driver's safe tick range",
        };
        fmt.write_str(description)
    }
}

impl error::Error for DeadlineError {}
