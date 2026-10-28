use super::{assert_shutdown, deadline, flavors, poll_once, runtime, Flavor};
use crate::time::{Duration, Instant};

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

#[derive(Debug, Default)]
enum ConcreteType {
    Sleep,
    #[default]
    Other,
}

impl tracing::field::Visit for ConcreteType {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "concrete_type" {
            *self = match value {
                "Sleep" => Self::Sleep,
                _ => Self::Other,
            };
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

#[derive(Debug, Default)]
enum DurationField {
    #[default]
    Absent,
    Milliseconds(u64),
    Invalid(String),
}

impl tracing::field::Visit for DurationField {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "duration" {
            *self = Self::Milliseconds(value);
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "duration" {
            *self = Self::Invalid(format!("{value:?}"));
        }
    }
}

#[derive(Debug)]
struct SpanInfo {
    metadata: &'static Metadata<'static>,
    parent: Option<u64>,
    concrete_type: ConcreteType,
}

#[derive(Default)]
struct TraceState {
    next_id: u64,
    spans: BTreeMap<u64, SpanInfo>,
    entered: Vec<&'static str>,
    active: HashMap<ThreadId, Vec<u64>>,
    durations: Vec<u64>,
    errors: Vec<String>,
}

impl TraceState {
    fn current(&self) -> Option<u64> {
        self.active.get(&thread::current().id())?.last().copied()
    }

    fn parent(&self, explicit: Option<&Id>, contextual: bool) -> Option<u64> {
        explicit.map(Id::into_u64).or_else(|| {
            if contextual {
                self.current()
            } else {
                None
            }
        })
    }

    fn sleep_resource(&self, mut parent: Option<u64>) -> Option<u64> {
        while let Some(id) = parent {
            let span = self.spans.get(&id)?;
            if span.metadata.name() == "runtime.resource" {
                return match span.concrete_type {
                    ConcreteType::Sleep => Some(id),
                    ConcreteType::Other => None,
                };
            }
            parent = span.parent;
        }
        None
    }
}

#[derive(Default)]
struct Observed(Mutex<TraceState>);

impl Observed {
    fn lock(&self) -> MutexGuard<'_, TraceState> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn durations(&self) -> Vec<u64> {
        let (durations, errors) = {
            let state = self.lock();
            (state.durations.clone(), state.errors.clone())
        };
        assert!(errors.is_empty(), "trace collector: {errors:#?}");
        durations
    }

    fn entered(&self) -> Vec<&'static str> {
        self.lock().entered.clone()
    }
}

struct Capture(Arc<Observed>);

impl Subscriber for Capture {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let mut concrete_type = ConcreteType::default();
        attributes.record(&mut concrete_type);
        let mut state = self.0.lock();
        state.next_id += 1;
        let id = state.next_id;
        let parent = state.parent(attributes.parent(), attributes.is_contextual());
        state.spans.insert(
            id,
            SpanInfo {
                metadata: attributes.metadata(),
                parent,
                concrete_type,
            },
        );
        Id::from_u64(id)
    }

    fn record(&self, id: &Id, values: &Record<'_>) {
        if let Some(span) = self.0.lock().spans.get_mut(&id.into_u64()) {
            values.record(&mut span.concrete_type);
        }
    }

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != "runtime::resource::state_update" {
            return;
        }
        let mut duration = DurationField::default();
        event.record(&mut duration);
        if let DurationField::Absent = duration {
            return;
        }
        let mut state = self.0.lock();
        let parent = state.parent(event.parent(), event.is_contextual());
        let resource = state.sleep_resource(parent);
        let invalid = match duration {
            DurationField::Absent => return,
            DurationField::Milliseconds(value) => {
                if resource.is_some() {
                    state.durations.push(value);
                    return;
                }
                format!("unbound duration={value}")
            }
            DurationField::Invalid(value) => format!("invalid duration={value}"),
        };
        let evidence = format!(
            "{}:{} {invalid}, explicit={:?}, contextual={}, parent={parent:?}, sleep_resource={resource:?}, spans={:?}",
            event.metadata().file().unwrap_or("<unknown>"),
            event.metadata().line().unwrap_or(0),
            event.parent(), event.is_contextual(), state.spans,
        );
        state.errors.push(evidence);
    }

    fn enter(&self, id: &Id) {
        let mut state = self.0.lock();
        match state.spans.get(&id.into_u64()) {
            Some(span) => {
                let name = span.metadata.name();
                state.entered.push(name);
            }
            None => state.errors.push(format!("enter unknown span {id:?}")),
        }
        state.active.entry(thread::current().id()).or_default().push(id.into_u64());
    }

    fn exit(&self, id: &Id) {
        let mut state = self.0.lock();
        let actual = state.active.get_mut(&thread::current().id()).and_then(Vec::pop);
        if actual != Some(id.into_u64()) {
            state.errors.push(format!("exit {id:?}, active={actual:?}"));
        }
    }
}

#[test]
fn collector_observes_contextual_ancestry_and_explicit_parents() {
    let observed = Arc::new(Observed::default());
    let _subscriber = tracing::subscriber::set_default(Capture(observed.clone()));
    tracing::trace!(target: "runtime::resource::state_update", rx_dropped = true);
    tracing::trace!(target: "unrelated", duration = 999_u64);
    let resource = tracing::trace_span!(
        parent: None, "runtime.resource", concrete_type = "Sleep",
    );
    {
        let _resource = resource.enter();
        tracing::trace!(target: "runtime::resource::state_update", duration = 11_u64);
        let operation = tracing::trace_span!("runtime.resource.async_op");
        let _operation = operation.enter();
        tracing::trace!(target: "runtime::resource::state_update", duration = 12_u64);
        tracing::trace!(target: "runtime::resource::state_update", parent: None, rx_dropped = true);
    }
    let operation = tracing::trace_span!(parent: &resource, "runtime.resource.async_op");
    operation.in_scope(|| {
        tracing::trace!(target: "runtime::resource::state_update", duration = 14_u64);
    });
    let unrelated = tracing::trace_span!(parent: None, "unrelated");
    let _unrelated = unrelated.enter();
    tracing::trace!(target: "runtime::resource::state_update", parent: &resource, duration = 13_u64);
    assert_eq!(observed.durations(), vec![11, 12, 14, 13]);
}

#[test]
fn collector_rejects_unbound_durations_without_panicking_in_callbacks() {
    let observed = Arc::new(Observed::default());
    let _subscriber = tracing::subscriber::set_default(Capture(observed.clone()));
    tracing::trace!(target: "runtime::resource::state_update", duration = 21_u64);
    let foreign = tracing::trace_span!(
        parent: None, "runtime.resource", concrete_type = "oneshot",
    );
    let resource = tracing::trace_span!(
        parent: None, "runtime.resource", concrete_type = "Sleep",
    );
    {
        let _resource = resource.enter();
        tracing::trace!(target: "runtime::resource::state_update", parent: None, duration = 22_u64);
        tracing::trace!(target: "runtime::resource::state_update", parent: &foreign, duration = 23_u64);
        tracing::trace!(target: "runtime::resource::state_update", duration = "invalid");
        let detached = tracing::trace_span!(parent: None, "detached");
        detached.in_scope(|| {
            tracing::trace!(target: "runtime::resource::state_update", duration = 24_u64);
        });
    }
    let (durations, errors) = {
        let state = observed.lock();
        (state.durations.clone(), state.errors.clone())
    };
    assert!(durations.is_empty());
    assert_eq!(errors.len(), 5, "{errors:#?}");
    for (error, expected) in errors.iter().zip([
        "unbound duration=21,",
        "unbound duration=22,",
        "unbound duration=23,",
        "invalid duration=\"invalid\",",
        "unbound duration=24,",
    ]) {
        assert!(error.contains(expected), "{error}");
    }
    let failure = std::panic::catch_unwind(|| {
        let resource = tracing::trace_span!(
            parent: None, "runtime.resource", concrete_type = "Sleep",
        );
        let _resource = resource.enter();
        observed.durations();
    });
    assert!(failure.is_err());
    assert!(!observed.0.is_poisoned());
    assert!(observed.lock().current().is_none());
}

#[test]
fn collector_context_is_thread_local() {
    let observed = Arc::new(Observed::default());
    let _subscriber = tracing::subscriber::set_default(Capture(observed.clone()));
    let resource = tracing::trace_span!(
        parent: None, "runtime.resource", concrete_type = "Sleep",
    );
    let _resource = resource.enter();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    thread::spawn(move || tracing::dispatcher::with_default(&dispatch, || {
        tracing::trace!(target: "runtime::resource::state_update", duration = 25_u64);
    })).join().unwrap();
    tracing::trace!(target: "runtime::resource::state_update", duration = 15_u64);
    let (durations, errors) = {
        let state = observed.lock();
        (state.durations.clone(), state.errors.clone())
    };
    assert_eq!(durations, vec![15]);
    assert_eq!(errors.len(), 1, "{errors:#?}");
    assert!(errors[0].contains("unbound duration=25,"), "{errors:#?}");
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
        assert_eq!(observed.durations(), vec![expected]);
        drop(owner);
        assert_shutdown(poll_once(wait.as_mut()));
        assert_eq!(observed.durations(), vec![expected]);
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
        let durations = observed.durations();
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
        assert_eq!(observed.durations(), durations);
        let entered = observed.entered();
        for name in [
            "runtime.resource",
            "runtime.resource.async_op",
            "runtime.resource.async_op.poll",
        ] {
            assert!(entered.contains(&name));
        }
    }
}
