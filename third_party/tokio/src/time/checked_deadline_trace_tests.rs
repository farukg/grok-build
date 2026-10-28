use super::{assert_shutdown, deadline, flavors, poll_once, runtime, Flavor};
use crate::time::{Duration, Instant};

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

#[derive(Default)]
struct Observed {
    next_id: AtomicU64,
    spans: Mutex<BTreeMap<u64, &'static str>>,
    entered: Mutex<Vec<&'static str>>,
    active: Mutex<Vec<u64>>,
    durations: Mutex<Vec<u64>>,
}

struct Capture(Arc<Observed>);

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let id = self.0.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.spans.lock().unwrap().insert(id, attributes.metadata().name());
        Id::from_u64(id)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != "runtime::resource::state_update" {
            return;
        }
        let active = self.0.active.lock().unwrap();
        let resource = event
            .parent()
            .map(Id::into_u64)
            .or_else(|| active.last().copied());
        assert_eq!(
            self.0.spans.lock().unwrap()[&resource.unwrap()],
            "runtime.resource",
        );
        struct Durations<'a>(&'a Mutex<Vec<u64>>);
        impl tracing::field::Visit for Durations<'_> {
            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                if field.name() == "duration" {
                    self.0.lock().unwrap().push(value);
                }
            }

            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }
        event.record(&mut Durations(&self.0.durations));
    }

    fn enter(&self, id: &Id) {
        let name = self.0.spans.lock().unwrap()[&id.into_u64()];
        self.0.entered.lock().unwrap().push(name);
        self.0.active.lock().unwrap().push(id.into_u64());
    }

    fn exit(&self, id: &Id) {
        assert_eq!(self.0.active.lock().unwrap().pop(), Some(id.into_u64()));
    }
}

#[test]
fn checked_wait_trace_duration_rounds_at_the_driver_epoch() {
    for (phase, expected) in [
        (Duration::ZERO, 60_000),
        (Duration::from_nanos(1), 60_001),
    ] {
        let owner = runtime(Flavor::Traditional);
        owner.block_on(crate::time::advance(phase));
        let deadline = deadline(&owner, Duration::from_secs(60));
        let observed = Arc::new(Observed::default());
        let _subscriber = tracing::subscriber::set_default(Capture(observed.clone()));
        let mut wait = Box::pin(deadline.wait());
        assert!(poll_once(wait.as_mut()).is_pending());
        assert_eq!(*observed.durations.lock().unwrap(), vec![expected]);
        drop(owner);
        assert_shutdown(poll_once(wait.as_mut()));
    }
}

#[test]
fn checked_wait_uses_sleep_spans_and_the_bound_driver_clock_on_both_flavors() {
    for flavor in flavors() {
        let owner = runtime(flavor);
        let other = runtime(Flavor::Traditional);
        let deadline = deadline(&owner, Duration::from_secs(60));
        other.block_on(crate::time::advance(Duration::from_secs(3600)));
        let observed = Arc::new(Observed::default());
        let _subscriber = tracing::subscriber::set_default(Capture(observed.clone()));
        let mut wait = Box::pin(deadline.wait());
        let time_source = owner.handle().inner.driver().time().time_source();
        let deadline_tick = time_source
            .checked_deadline_to_tick(deadline.deadline())
            .unwrap();
        let before = {
            let _guard = owner.enter();
            time_source.instant_to_tick(Instant::now())
        };
        other.block_on(async {
            assert!(poll_once(wait.as_mut()).is_pending());
        });
        let after = {
            let _guard = owner.enter();
            time_source.instant_to_tick(Instant::now())
        };
        let durations = observed.durations.lock().unwrap().clone();
        assert_eq!(durations.len(), 1);
        assert!(
            durations[0] > 0,
            "durations={durations:?}, ticks={before}..={after}",
        );
        // Deadline ticks round up; clock ticks round down at the driver's epoch.
        assert!(
            (deadline_tick.saturating_sub(after)..=deadline_tick.saturating_sub(before))
                .contains(&durations[0]),
            "durations={durations:?}, deadline_tick={deadline_tick}, ticks={before}..={after}",
        );
        drop(owner);
        assert_shutdown(poll_once(wait.as_mut()));
        let entered = observed.entered.lock().unwrap();
        for name in [
            "runtime.resource",
            "runtime.resource.async_op",
            "runtime.resource.async_op.poll",
        ] {
            assert!(entered.contains(&name));
        }
    }
}
