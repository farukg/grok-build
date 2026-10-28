use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use super::{RetryScheduleCause, SamplingError};
use tokio::time::error::{DeadlineError, Error as TimerError};

fn assert_native_cause(actual: &RetryScheduleCause, expected: &RetryScheduleCause) {
    match expected {
        RetryScheduleCause::InstantRange => {
            assert!(matches!(actual, RetryScheduleCause::InstantRange));
            assert!(actual.source().is_none());
        }
        RetryScheduleCause::Deadline(expected) => {
            let RetryScheduleCause::Deadline(actual_error) = actual else {
                panic!("expected native deadline error, got {actual:?}");
            };
            assert_eq!(actual_error, expected);
            let source = actual.source().unwrap();
            assert_eq!(source.downcast_ref::<DeadlineError>(), Some(expected));
            assert!(source.source().is_none());
        }
        RetryScheduleCause::Wait(expected) => {
            let RetryScheduleCause::Wait(actual_error) = actual else {
                panic!("expected native timer error, got {actual:?}");
            };
            let expected_kind = (
                expected.is_shutdown(),
                expected.is_at_capacity(),
                expected.is_invalid(),
            );
            assert_eq!(
                (
                    actual_error.is_shutdown(),
                    actual_error.is_at_capacity(),
                    actual_error.is_invalid(),
                ),
                expected_kind,
            );
            let source = actual.source().unwrap();
            let source_error = source.downcast_ref::<TimerError>().unwrap();
            assert_eq!(
                (
                    source_error.is_shutdown(),
                    source_error.is_at_capacity(),
                    source_error.is_invalid(),
                ),
                expected_kind,
            );
            assert!(source.source().is_none());
        }
    }
}

fn assert_wire(cause: RetryScheduleCause, wire: &str, display: &str) {
    assert_eq!(serde_json::to_string(&cause).unwrap(), wire);
    let decoded: RetryScheduleCause = serde_json::from_str(wire).unwrap();
    assert_native_cause(&decoded, &cause);
    assert_eq!(decoded.to_string(), display);
    assert_eq!(cause.to_string(), display);
    assert_eq!(serde_json::to_string(&decoded).unwrap(), wire);

    let original = Arc::new(SamplingError::auth_unknown("original rejection"));
    let error = SamplingError::RetrySchedule {
        requested_backoff: Duration::from_secs(17),
        original: Arc::clone(&original),
        cause: decoded,
    };
    assert_eq!(
        error.to_string(),
        format!("retry scheduling failed for 17s: {display}; original error: original rejection"),
    );
    let source = error.source().unwrap();
    let source_cause = source.downcast_ref::<RetryScheduleCause>().unwrap();
    assert_native_cause(source_cause, &cause);
    let SamplingError::RetrySchedule {
        original: retained, ..
    } = error
    else {
        panic!("expected retry schedule failure");
    };
    assert!(Arc::ptr_eq(&retained, &original));
}

#[test]
fn instant_range_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::InstantRange,
        r#"{"cause":"instant_range"}"#,
        "requested delay overflows Instant",
    );
}

#[test]
fn deadline_missing_runtime_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Deadline(DeadlineError::MissingRuntime),
        r#"{"cause":"deadline","detail":"missing_runtime"}"#,
        "no current Tokio runtime is accessible",
    );
}

#[test]
fn deadline_time_disabled_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Deadline(DeadlineError::TimeDisabled),
        r#"{"cause":"deadline","detail":"time_disabled"}"#,
        "the current Tokio runtime has no time driver enabled",
    );
}

#[test]
fn deadline_rounding_overflow_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Deadline(DeadlineError::RoundingOverflow),
        r#"{"cause":"deadline","detail":"rounding_overflow"}"#,
        "rounding the deadline overflows Instant",
    );
}

#[test]
fn deadline_tick_out_of_range_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Deadline(DeadlineError::TickOutOfRange),
        r#"{"cause":"deadline","detail":"tick_out_of_range"}"#,
        "the deadline exceeds the time driver's safe tick range",
    );
}

#[test]
fn timer_shutdown_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Wait(TimerError::shutdown()),
        r#"{"cause":"wait","detail":"shutdown"}"#,
        "the timer is shutdown, must be called from the context of Tokio runtime",
    );
}

#[test]
fn timer_at_capacity_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Wait(TimerError::at_capacity()),
        r#"{"cause":"wait","detail":"at_capacity"}"#,
        "timer is at capacity and cannot create a new entry",
    );
}

#[test]
fn timer_invalid_wire_and_readback() {
    assert_wire(
        RetryScheduleCause::Wait(TimerError::invalid()),
        r#"{"cause":"wait","detail":"invalid"}"#,
        "timer duration exceeds maximum duration",
    );
}

#[test]
fn missing_detail_is_rejected_without_a_runtime_cause() {
    for wire in [r#"{"cause":"deadline"}"#, r#"{"cause":"wait"}"#] {
        let error = serde_json::from_str::<RetryScheduleCause>(wire).unwrap_err();
        assert!(
            error.to_string().contains("missing field `detail`"),
            "{wire}: {error}",
        );
    }
}

#[test]
fn wrong_detail_is_rejected_without_a_runtime_cause() {
    for wire in [
        r#"{"cause":"deadline","detail":null}"#,
        r#"{"cause":"deadline","detail":false}"#,
        r#"{"cause":"deadline","detail":42}"#,
        r#"{"cause":"deadline","detail":[]}"#,
        r#"{"cause":"deadline","detail":{}}"#,
        r#"{"cause":"deadline","detail":"shutdown"}"#,
        r#"{"cause":"deadline","detail":{"missing_runtime":"invented"}}"#,
        r#"{"detail":null,"cause":"deadline"}"#,
        r#"{"cause":"wait","detail":null}"#,
        r#"{"cause":"wait","detail":false}"#,
        r#"{"cause":"wait","detail":42}"#,
        r#"{"cause":"wait","detail":[]}"#,
        r#"{"cause":"wait","detail":{}}"#,
        r#"{"cause":"wait","detail":"missing_runtime"}"#,
        r#"{"cause":"wait","detail":{"shutdown":"invented"}}"#,
        r#"{"detail":null,"cause":"wait"}"#,
        r#"{"cause":"instant_range","detail":"shutdown"}"#,
        r#"{"cause":"instant_range","detail":{}}"#,
    ] {
        assert!(
            serde_json::from_str::<RetryScheduleCause>(wire).is_err(),
            "{wire}",
        );
    }
}

#[test]
fn unknown_variants_are_rejected_without_a_runtime_cause() {
    for wire in [
        r#"{"cause":"future_cause"}"#,
        r#"{"cause":"future_cause","detail":"shutdown"}"#,
        r#"{"cause":"deadline","detail":"future_deadline"}"#,
        r#"{"cause":"wait","detail":"future_timer"}"#,
        r#"{"detail":"shutdown"}"#,
    ] {
        assert!(
            serde_json::from_str::<RetryScheduleCause>(wire).is_err(),
            "{wire}",
        );
    }
}

#[test]
fn existing_content_first_and_unit_map_forms_remain_readable() {
    for (wire, expected) in [
        (
            r#"{"detail":"tick_out_of_range","cause":"deadline"}"#,
            RetryScheduleCause::Deadline(DeadlineError::TickOutOfRange),
        ),
        (
            r#"{"detail":"at_capacity","cause":"wait"}"#,
            RetryScheduleCause::Wait(TimerError::at_capacity()),
        ),
        (
            r#"{"cause":"deadline","detail":{"time_disabled":null}}"#,
            RetryScheduleCause::Deadline(DeadlineError::TimeDisabled),
        ),
        (
            r#"{"cause":"wait","detail":{"invalid":null}}"#,
            RetryScheduleCause::Wait(TimerError::invalid()),
        ),
        (
            r#"{"cause":"instant_range","detail":null}"#,
            RetryScheduleCause::InstantRange,
        ),
    ] {
        let decoded: RetryScheduleCause = serde_json::from_str(wire).unwrap();
        assert_native_cause(&decoded, &expected);
    }
}
