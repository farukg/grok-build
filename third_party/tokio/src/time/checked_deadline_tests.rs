use crate::runtime::{Builder, Runtime};
use crate::time::{advance, error::DeadlineError, sleep_until, CheckedDeadline, Duration, Instant};

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap()
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(futures::task::noop_waker_ref()))
}

#[test]
fn missing_runtime_and_disabled_time_are_typed() {
    assert_eq!(
        CheckedDeadline::new(Instant::now()).unwrap_err(),
        DeadlineError::MissingRuntime
    );
    let runtime = Builder::new_current_thread().build().unwrap();
    let _guard = runtime.enter();
    assert_eq!(
        CheckedDeadline::new(Instant::now()).unwrap_err(),
        DeadlineError::TimeDisabled
    );
}

#[test]
fn original_anchor_survives_delayed_wait_and_cancellation() {
    runtime().block_on(async {
        let start = Instant::now();
        let at = start + Duration::from_secs(1200);
        let deadline = CheckedDeadline::new(at).unwrap();
        advance(Duration::from_secs(90)).await;

        let mut cancelled = Box::pin(deadline.wait());
        assert!(poll_once(cancelled.as_mut()).is_pending());
        drop(cancelled);

        let mut wait = Box::pin(deadline.wait());
        assert!(poll_once(wait.as_mut()).is_pending());
        advance(Duration::from_millis(1_109_999)).await;
        assert!(poll_once(wait.as_mut()).is_pending());
        wait.await.unwrap();
        assert_eq!(Instant::now(), at);
        assert_eq!(deadline.deadline(), at);
        deadline.wait().await.unwrap();
        assert_eq!(Instant::now(), at);
    });
}

#[test]
fn cloned_deadline_and_moved_wait_keep_the_validated_driver() {
    let owner = runtime();
    let other = runtime();
    let deadline = owner.block_on(async {
        CheckedDeadline::new(Instant::now() + Duration::from_secs(1200)).unwrap()
    });
    let at = deadline.deadline();
    let cloned = deadline.clone();
    owner.block_on(advance(Duration::from_secs(90)));
    other.block_on(advance(Duration::from_secs(3600)));

    let mut moved = Box::pin(deadline.wait());
    owner.block_on(async {
        assert!(poll_once(moved.as_mut()).is_pending());
    });
    let mut first_polled_elsewhere = Box::pin(cloned.wait());
    other.block_on(async {
        assert!(poll_once(moved.as_mut()).is_pending());
        assert!(poll_once(first_polled_elsewhere.as_mut()).is_pending());
    });
    owner.block_on(advance(Duration::from_secs(1109)));
    assert!(poll_once(moved.as_mut()).is_pending());
    other.block_on(async {
        assert!(poll_once(first_polled_elsewhere.as_mut()).is_pending());
    });
    owner.block_on(async {
        sleep_until(at).await;
        assert_eq!(Instant::now(), at);
    });
    other.block_on(async {
        assert!(matches!(poll_once(moved.as_mut()), Poll::Ready(Ok(()))));
        assert!(matches!(
            poll_once(first_polled_elsewhere.as_mut()),
            Poll::Ready(Ok(()))
        ));
    });
}

#[cfg(target_os = "linux")]
#[test]
fn old_runtime_checks_its_real_epoch_and_rounding_at_the_tick_limit() {
    let short_delay = Duration::from_millis(50) + Duration::from_nanos(1);
    runtime().block_on(async {
        let start = Instant::now();
        let deadline = CheckedDeadline::new(start + short_delay).unwrap();
        deadline.wait().await.unwrap();
        assert_eq!(Instant::now(), start + Duration::from_millis(51));
    });

    runtime().block_on(async {
        let epoch = Instant::now();
        let limit = epoch + Duration::from_millis(u64::MAX - 2);
        advance(Duration::from_millis(u64::MAX - 52)).await;
        let now = Instant::now();
        assert_eq!(now + Duration::from_millis(50), limit);
        assert_eq!(
            CheckedDeadline::new(now + short_delay).unwrap_err(),
            DeadlineError::TickOutOfRange
        );
        assert_eq!(
            CheckedDeadline::new(limit + Duration::from_millis(1)).unwrap_err(),
            DeadlineError::TickOutOfRange
        );

        let deadline = CheckedDeadline::new(limit).unwrap();
        let mut wait = Box::pin(deadline.wait());
        assert!(poll_once(wait.as_mut()).is_pending());
        advance(Duration::from_millis(49)).await;
        assert!(poll_once(wait.as_mut()).is_pending());
        wait.await.unwrap();
        assert_eq!(Instant::now(), limit);
        assert_eq!(deadline.deadline(), limit);
    });
}

#[cfg(target_os = "linux")]
#[test]
fn actual_instant_rounding_overflow_is_typed() {
    fn duration(nanos: u128) -> Duration {
        Duration::new(
            (nanos / 1_000_000_000).try_into().unwrap(),
            (nanos % 1_000_000_000).try_into().unwrap(),
        )
    }

    runtime().block_on(async {
        let start = Instant::now();
        let mut low = 0;
        let mut high = Duration::MAX.as_nanos();
        assert!(start.checked_add(Duration::MAX).is_none());
        while low + 1 < high {
            let middle = low + (high - low) / 2;
            match start.checked_add(duration(middle)) {
                Some(_) => low = middle,
                None => high = middle,
            }
        }
        let last = start.checked_add(duration(low)).unwrap();
        assert!(last.checked_add(Duration::from_nanos(1)).is_none());
        assert_eq!(
            CheckedDeadline::new(last).unwrap_err(),
            DeadlineError::RoundingOverflow
        );
    });
}

#[test]
fn checked_and_legacy_timers_keep_rounding_past_deadlines_and_long_reinsertion() {
    for (delay, elapsed) in [
        (Duration::ZERO, Duration::ZERO),
        (Duration::from_nanos(1), Duration::from_millis(1)),
        (Duration::from_millis(1200), Duration::from_millis(1200)),
        (
            Duration::from_secs(5 * 365 * 24 * 60 * 60),
            Duration::from_secs(5 * 365 * 24 * 60 * 60),
        ),
    ] {
        runtime().block_on(async {
            let start = Instant::now();
            let at = start + delay;
            let deadline = CheckedDeadline::new(at).unwrap();
            let mut checked = Box::pin(deadline.wait());
            let mut legacy = Box::pin(sleep_until(at));
            if !delay.is_zero() {
                assert!(poll_once(checked.as_mut()).is_pending());
                assert!(poll_once(legacy.as_mut()).is_pending());
            }
            checked.await.unwrap();
            assert_eq!(Instant::now(), start + elapsed);
            legacy.await;
            assert_eq!(Instant::now(), start + elapsed);
            assert_eq!(deadline.deadline(), at);

            let past = CheckedDeadline::new(start - Duration::from_secs(1)).unwrap();
            past.wait().await.unwrap();
            assert_eq!(Instant::now(), start + elapsed);
        });
    }
}

#[cfg(target_os = "linux")]
#[test]
fn legacy_out_of_range_deadline_is_still_constructible_and_saturates() {
    runtime().block_on(async {
        let at = Instant::now() + Duration::from_millis(u64::MAX - 1);
        assert_eq!(
            CheckedDeadline::new(at).unwrap_err(),
            DeadlineError::TickOutOfRange
        );
        let handle = crate::runtime::Handle::current();
        let source = handle.inner.driver().time().time_source();
        assert_eq!(source.deadline_to_tick(at), u64::MAX - 2);
        let mut legacy = Box::pin(sleep_until(at));
        assert_eq!(legacy.deadline(), at);
        assert!(poll_once(legacy.as_mut()).is_pending());
        drop(legacy);
    });
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn alternative_timer_wait_keeps_its_owner_when_polled_elsewhere() {
    let owner = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_alt_timer()
        .build()
        .unwrap();
    let deadline = {
        let _guard = owner.enter();
        CheckedDeadline::new(Instant::now() + Duration::from_millis(20)).unwrap()
    };
    let at = deadline.deadline().into_std();
    runtime().block_on(async {
        advance(Duration::from_secs(3600)).await;
        deadline.wait().await.unwrap();
        assert!(std::time::Instant::now() >= at);
    });
}
