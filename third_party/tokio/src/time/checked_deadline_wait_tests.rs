use crate::runtime::time::registration_test_hook;
use crate::runtime::{Builder, Runtime};
use crate::time::{error::Error, sleep_until, CheckedDeadline, Duration, Instant};

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Clone, Copy)]
enum Flavor {
    Traditional,
    #[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
    Alternative,
}

fn flavors() -> impl Iterator<Item = Flavor> {
    [
        Flavor::Traditional,
        #[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
        Flavor::Alternative,
    ]
    .into_iter()
}

fn runtime(flavor: Flavor) -> Runtime {
    match flavor {
        Flavor::Traditional => Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap(),
        #[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
        Flavor::Alternative => Builder::new_multi_thread()
            .worker_threads(1)
            .enable_alt_timer()
            .build()
            .unwrap(),
    }
}

fn deadline(owner: &Runtime, delay: Duration) -> CheckedDeadline {
    let _guard = owner.enter();
    CheckedDeadline::new(Instant::now() + delay).unwrap()
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(futures::task::noop_waker_ref()))
}

fn assert_shutdown(result: Poll<Result<(), Error>>) {
    match result {
        Poll::Ready(Err(error)) => assert!(error.is_shutdown()),
        result => panic!("expected shutdown, got {result:?}"),
    }
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl futures::task::ArcWake for WakeCount {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct WakeSignal(std::sync::Mutex<std::sync::mpsc::Sender<()>>);

impl futures::task::ArcWake for WakeSignal {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let _ = arc_self.0.lock().unwrap().send(());
    }
}

#[test]
fn new_wait_and_clone_after_owner_drop_return_shutdown_without_a_runtime() {
    for flavor in flavors() {
        let owner = runtime(flavor);
        let deadline = deadline(&owner, Duration::from_secs(3600));
        let cloned = deadline.clone();
        let at = deadline.deadline();
        drop(owner);

        let mut wait = Box::pin(deadline.wait());
        assert_shutdown(poll_once(wait.as_mut()));
        let other = runtime(Flavor::Traditional);
        other.block_on(async {
            let mut wait = Box::pin(cloned.wait());
            assert_shutdown(poll_once(wait.as_mut()));
        });
        assert_eq!(cloned.deadline(), at);
    }
}

#[test]
fn registered_moved_cloned_and_reregistered_waits_are_woken_with_shutdown() {
    for flavor in flavors() {
        let owner = runtime(flavor);
        let other = runtime(Flavor::Traditional);
        let deadline = deadline(&owner, Duration::from_secs(3600));
        let cloned = deadline.clone();
        let count = Arc::new(WakeCount::default());
        let waker = futures::task::waker(count.clone());
        let mut moved = Box::pin(deadline.wait());
        owner.block_on(async {
            assert!(moved.as_mut().poll(&mut Context::from_waker(&waker)).is_pending());
        });
        let cancelled_count = Arc::new(WakeCount::default());
        let cancelled_waker = futures::task::waker(cancelled_count.clone());
        let mut cancelled = Box::pin(cloned.wait());
        assert!(cancelled
            .as_mut()
            .poll(&mut Context::from_waker(&cancelled_waker))
            .is_pending());
        drop(cancelled);
        let mut reregistered = Box::pin(cloned.wait());
        other.block_on(async {
            assert!(poll_once(reregistered.as_mut()).is_pending());
        });
        drop(owner);
        assert!(count.0.load(Ordering::SeqCst) > 0);
        assert_eq!(cancelled_count.0.load(Ordering::SeqCst), 0);
        other.block_on(async {
            assert_shutdown(poll_once(moved.as_mut()));
            assert_shutdown(poll_once(reregistered.as_mut()));
        });
        let mut new_wait = Box::pin(cloned.wait());
        assert_shutdown(poll_once(new_wait.as_mut()));
    }
}

#[test]
fn true_elapsed_completion_survives_shutdown_but_a_new_wait_is_rejected() {
    for flavor in flavors() {
        let owner = runtime(flavor);
        let deadline = deadline(&owner, Duration::from_secs(1));
        let cloned = deadline.clone();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let waker = futures::task::waker(Arc::new(WakeSignal(std::sync::Mutex::new(wake_tx))));
        let mut wait = Box::pin(deadline.wait());
        assert!(wait.as_mut().poll(&mut Context::from_waker(&waker)).is_pending());
        owner.block_on(async { sleep_until(deadline.deadline()).await });
        wake_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(owner);
        assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(Ok(()))));
        let mut new_wait = Box::pin(cloned.wait());
        assert_shutdown(poll_once(new_wait.as_mut()));
    }
}

#[test]
fn shutdown_between_poll_entry_and_native_registration_is_returned() {
    for flavor in flavors() {
        let owner = runtime(flavor);
        let deadline = deadline(&owner, Duration::from_secs(3600));
        let called = Arc::new(AtomicUsize::new(0));
        let observed = called.clone();
        let _hook = registration_test_hook::install(move || {
            drop(owner);
            observed.fetch_add(1, Ordering::SeqCst);
            None
        });
        let mut wait = Box::pin(deadline.wait());
        assert_shutdown(poll_once(wait.as_mut()));
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn native_registration_errors_keep_their_cause_and_do_not_poison_the_deadline() {
    for flavor in flavors() {
        for expected in [Error::shutdown(), Error::at_capacity(), Error::invalid()] {
            let owner = runtime(flavor);
            let deadline = deadline(&owner, Duration::from_millis(1));
            let _hook = registration_test_hook::install(move || Some(expected));
            let mut wait = Box::pin(deadline.wait());
            let error = match poll_once(wait.as_mut()) {
                Poll::Ready(Err(error)) => error,
                result => panic!("expected registration failure, got {result:?}"),
            };
            assert_eq!(error.is_shutdown(), expected.is_shutdown());
            assert_eq!(error.is_at_capacity(), expected.is_at_capacity());
            assert_eq!(error.is_invalid(), expected.is_invalid());
            drop(wait);
            owner.block_on(deadline.wait()).unwrap();
            owner.block_on(deadline.clone().wait()).unwrap();
        }
    }
}

#[test]
fn an_unpolled_living_owner_does_not_use_the_polling_runtime_clock() {
    let owner = runtime(Flavor::Traditional);
    let other = runtime(Flavor::Traditional);
    let deadline = deadline(&owner, Duration::from_secs(1));
    let mut wait = Box::pin(deadline.wait());
    other.block_on(async {
        crate::time::advance(Duration::from_secs(3600)).await;
        assert!(poll_once(wait.as_mut()).is_pending());
    });
    owner.block_on(async {
        crate::time::advance(Duration::from_secs(1)).await;
    });
    assert!(matches!(poll_once(wait.as_mut()), Poll::Ready(Ok(()))));
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn alternative_unpolled_owner_remains_pending_and_cancelled_wait_can_reregister() {
    let owner = runtime(Flavor::Alternative);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = owner.spawn(async move {
        started_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let deadline = {
        let _guard = owner.enter();
        CheckedDeadline::new(Instant::now() - Duration::from_secs(1)).unwrap()
    };
    let cloned = deadline.clone();
    let other = runtime(Flavor::Traditional);
    let mut cancelled = Box::pin(deadline.wait());
    other.block_on(async {
        crate::time::advance(Duration::from_secs(3600)).await;
        assert!(poll_once(cancelled.as_mut()).is_pending());
    });
    drop(cancelled);
    let mut moved = Box::pin(cloned.wait());
    assert!(poll_once(moved.as_mut()).is_pending());
    other.block_on(async {
        assert!(poll_once(moved.as_mut()).is_pending());
    });
    release_tx.send(()).unwrap();
    owner.block_on(blocker).unwrap();
    other.block_on(moved).unwrap();
    owner.block_on(deadline.wait()).unwrap();
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn alternative_local_registration_can_move_cancel_and_reregister_before_shutdown() {
    let owner = runtime(Flavor::Alternative);
    let deadline = deadline(&owner, Duration::from_secs(3600));
    let locally_polled = |deadline: CheckedDeadline| async move {
        let mut wait = Box::pin(async move { deadline.wait().await });
        assert!(poll_once(wait.as_mut()).is_pending());
        wait
    };
    let cancelled = owner
        .block_on(owner.spawn(locally_polled(deadline.clone())))
        .unwrap();
    drop(cancelled);
    let mut moved = owner
        .block_on(owner.spawn(locally_polled(deadline.clone())))
        .unwrap();
    let other = runtime(Flavor::Traditional);
    other.block_on(async {
        assert!(poll_once(moved.as_mut()).is_pending());
    });
    drop(owner);
    assert_shutdown(poll_once(moved.as_mut()));
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn alternative_local_registration_serializes_with_actual_owner_close() {
    let owner = runtime(Flavor::Alternative);
    let deadline = deadline(&owner, Duration::from_secs(3600));
    let spawner = owner.handle().clone();
    let task = spawner.spawn(async move {
        let _hook = registration_test_hook::install(move || {
            owner.shutdown_background();
            None
        });
        let mut wait = Box::pin(deadline.wait());
        poll_once(wait.as_mut())
    });
    let other = runtime(Flavor::Traditional);
    assert_shutdown(other.block_on(task).unwrap());
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn alternative_local_registration_forwards_native_errors() {
    for expected in [Error::shutdown(), Error::at_capacity(), Error::invalid()] {
        let owner = runtime(Flavor::Alternative);
        let deadline = deadline(&owner, Duration::from_secs(3600));
        let task = owner.spawn(async move {
            let _hook = registration_test_hook::install(move || Some(expected));
            let mut wait = Box::pin(deadline.wait());
            poll_once(wait.as_mut())
        });
        let error = match owner.block_on(task).unwrap() {
            Poll::Ready(Err(error)) => error,
            result => panic!("expected local registration failure, got {result:?}"),
        };
        assert_eq!(error.is_shutdown(), expected.is_shutdown());
        assert_eq!(error.is_at_capacity(), expected.is_at_capacity());
        assert_eq!(error.is_invalid(), expected.is_invalid());
    }
}

#[test]
#[should_panic(expected = "it is being shutdown")]
fn legacy_traditional_registered_shutdown_still_panics() {
    let owner = runtime(Flavor::Traditional);
    let mut sleep = {
        let _guard = owner.enter();
        Box::pin(sleep_until(Instant::now() + Duration::from_secs(3600)))
    };
    assert!(poll_once(sleep.as_mut()).is_pending());
    drop(owner);
    let _ = poll_once(sleep.as_mut());
}

#[test]
#[should_panic(expected = "it is being shutdown")]
fn legacy_traditional_first_poll_after_shutdown_still_panics() {
    let owner = runtime(Flavor::Traditional);
    let mut sleep = {
        let _guard = owner.enter();
        Box::pin(sleep_until(Instant::now() + Duration::from_secs(3600)))
    };
    drop(owner);
    let _ = poll_once(sleep.as_mut());
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
fn legacy_alternative_registered_shutdown_still_completes_successfully() {
    let owner = runtime(Flavor::Alternative);
    let mut sleep = {
        let _guard = owner.enter();
        Box::pin(sleep_until(Instant::now() + Duration::from_secs(3600)))
    };
    assert!(poll_once(sleep.as_mut()).is_pending());
    drop(owner);
    assert!(sleep.is_elapsed());
    assert!(matches!(poll_once(sleep.as_mut()), Poll::Ready(())));
}

#[cfg(all(tokio_unstable, feature = "rt-multi-thread"))]
#[test]
#[should_panic(expected = "it is being shutdown")]
fn legacy_alternative_first_poll_after_shutdown_still_panics() {
    let owner = runtime(Flavor::Alternative);
    let mut sleep = {
        let _guard = owner.enter();
        Box::pin(sleep_until(Instant::now() + Duration::from_secs(3600)))
    };
    drop(owner);
    let _ = poll_once(sleep.as_mut());
}

#[cfg(all(tokio_unstable, feature = "tracing"))]
#[path = "checked_deadline_trace_tests.rs"]
mod trace_tests;
