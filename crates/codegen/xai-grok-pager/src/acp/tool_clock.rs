use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};
use uuid::Uuid;
use xai_grok_shell::extensions::tool_timing::{
    LocalClockMapping, MonotonicPoint, ToolExecutionId, ToolExecutionTermination,
    ToolExecutionTiming,
};

#[derive(Clone, Copy)]
pub(crate) enum ToolClockAdmission {
    Current,
    Unconfirmed,
}

#[derive(Debug, Clone)]
enum ExecutionStartClock {
    Unconfirmed,
    Mapped(LocalClockMapping),
}

#[derive(Debug, Clone)]
enum ExecutionSpan {
    Started {
        point: MonotonicPoint,
        clock: ExecutionStartClock,
    },
    Finished {
        point: MonotonicPoint,
        duration_ms: u64,
        termination: ToolExecutionTermination,
    },
}

#[derive(Debug, Clone, Copy)]
enum CallPhase {
    Open,
    Unconfirmed,
    Terminal,
}

#[derive(Debug)]
struct ExecutionSpans {
    attempts: HashMap<ToolExecutionId, ExecutionSpan>,
    missing_calls: HashSet<Uuid>,
    phase: CallPhase,
}

#[derive(Debug)]
enum ClockGroup {
    Root(ExecutionSpans),
    Joined(Rc<RefCell<ClockGroup>>),
}

#[derive(Debug, Clone, Default)]
enum ToolClockState {
    #[default]
    Unknown,
    Observed {
        invocation: Uuid,
        group: Rc<RefCell<ClockGroup>>,
    },
}

/// Shared by the invocation, pending projection and every refined render block.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToolExecutionClock {
    state: ToolClockState,
}

impl ToolExecutionClock {
    pub(crate) fn new(invocation: Uuid) -> Self {
        Self {
            state: ToolClockState::Observed {
                invocation,
                group: Rc::new(RefCell::new(ClockGroup::Root(ExecutionSpans {
                    attempts: HashMap::new(),
                    missing_calls: HashSet::from([invocation]),
                    phase: CallPhase::Open,
                }))),
            },
        }
    }

    fn root(&self) -> Option<Rc<RefCell<ClockGroup>>> {
        let ToolClockState::Observed { group, .. } = &self.state else {
            return None;
        };
        let mut group = Rc::clone(group);
        loop {
            let next = match &*group.borrow() {
                ClockGroup::Root(_) => None,
                ClockGroup::Joined(next) => Some(Rc::clone(next)),
            };
            match next {
                Some(next) => group = next,
                None => return Some(group),
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn termination(&self, id: ToolExecutionId) -> Option<ToolExecutionTermination> {
        let group = self.root()?;
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return None;
        };
        match spans.attempts.get(&id) {
            Some(ExecutionSpan::Finished { termination, .. }) => Some(*termination),
            Some(ExecutionSpan::Started { .. }) | None => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn mapping(&self, id: ToolExecutionId) -> Option<LocalClockMapping> {
        let group = self.root()?;
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return None;
        };
        match spans.attempts.get(&id) {
            Some(ExecutionSpan::Started {
                clock: ExecutionStartClock::Mapped(clock),
                ..
            }) => Some(*clock),
            Some(ExecutionSpan::Started {
                clock: ExecutionStartClock::Unconfirmed,
                ..
            }) => Some(LocalClockMapping::Uncorrelated),
            Some(ExecutionSpan::Finished { .. }) | None => None,
        }
    }

    pub(crate) fn accepts(&self, timing: ToolExecutionTiming) -> bool {
        let (id, point) = match timing {
            ToolExecutionTiming::Unknown => return true,
            ToolExecutionTiming::Started {
                execution_id,
                started_at,
            }
            | ToolExecutionTiming::Finished {
                execution_id,
                started_at,
                ..
            } => (execution_id, started_at),
        };
        let Some(group) = self.root() else {
            return false;
        };
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return false;
        };
        match spans.attempts.get(&id) {
            None => true,
            Some(ExecutionSpan::Started {
                point: original, ..
            }) => *original == point,
            Some(ExecutionSpan::Finished {
                point: original,
                duration_ms,
                termination,
            }) => {
                *original == point
                    && match timing {
                        ToolExecutionTiming::Started { .. } => true,
                        ToolExecutionTiming::Finished {
                            duration_ms: incoming,
                            termination: end,
                            ..
                        } => *duration_ms == incoming && *termination == end,
                        ToolExecutionTiming::Unknown => true,
                    }
            }
        }
    }

    pub(crate) fn apply(&self, timing: ToolExecutionTiming, admission: ToolClockAdmission) -> bool {
        if !self.accepts(timing) {
            return false;
        }
        let ToolClockState::Observed { invocation, .. } = &self.state else {
            return false;
        };
        let Some(group) = self.root() else {
            return false;
        };
        let mut group = group.borrow_mut();
        let ClockGroup::Root(spans) = &mut *group else {
            return false;
        };
        match timing {
            ToolExecutionTiming::Unknown => false,
            ToolExecutionTiming::Started {
                execution_id,
                started_at,
            } => {
                if spans.attempts.contains_key(&execution_id) {
                    return false;
                }
                let clock = if matches!(admission, ToolClockAdmission::Current)
                    && matches!(spans.phase, CallPhase::Open)
                {
                    ExecutionStartClock::Mapped(started_at.map_local())
                } else {
                    ExecutionStartClock::Unconfirmed
                };
                spans.missing_calls.remove(invocation);
                spans.attempts.insert(
                    execution_id,
                    ExecutionSpan::Started {
                        point: started_at,
                        clock,
                    },
                );
                true
            }
            ToolExecutionTiming::Finished {
                execution_id,
                started_at,
                duration_ms,
                termination,
            } => {
                if matches!(
                    spans.attempts.get(&execution_id),
                    Some(ExecutionSpan::Finished { .. })
                ) {
                    return false;
                }
                spans.missing_calls.remove(invocation);
                spans.attempts.insert(
                    execution_id,
                    ExecutionSpan::Finished {
                        point: started_at,
                        duration_ms,
                        termination,
                    },
                );
                true
            }
        }
    }

    pub(crate) fn confirm_current(&self, execution_id: ToolExecutionId) -> bool {
        let Some(group) = self.root() else {
            return false;
        };
        let mut group = group.borrow_mut();
        let ClockGroup::Root(spans) = &mut *group else {
            return false;
        };
        if matches!(spans.phase, CallPhase::Terminal) {
            return false;
        }
        let reopening = matches!(spans.phase, CallPhase::Unconfirmed);
        let changed = match spans.attempts.get_mut(&execution_id) {
            Some(ExecutionSpan::Started {
                point,
                clock: clock @ ExecutionStartClock::Unconfirmed,
            }) => {
                *clock = ExecutionStartClock::Mapped(point.map_local());
                true
            }
            Some(ExecutionSpan::Started {
                clock: ExecutionStartClock::Mapped(_),
                ..
            }) => reopening,
            Some(ExecutionSpan::Finished { .. }) | None => false,
        };
        if changed {
            spans.phase = CallPhase::Open;
        }
        changed
    }

    pub(crate) fn confirm_pending(&self) -> bool {
        let Some(group) = self.root() else {
            return false;
        };
        let mut group = group.borrow_mut();
        let ClockGroup::Root(spans) = &mut *group else {
            return false;
        };
        if matches!(spans.phase, CallPhase::Terminal) {
            return false;
        }
        spans.phase = CallPhase::Open;
        true
    }

    pub(crate) fn timing(&self, execution: ToolExecutionId) -> Option<ToolExecutionTiming> {
        let group = self.root()?;
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return None;
        };
        Some(match spans.attempts.get(&execution)? {
            ExecutionSpan::Started { point, .. } => ToolExecutionTiming::Started {
                execution_id: execution,
                started_at: *point,
            },
            ExecutionSpan::Finished {
                point,
                duration_ms,
                termination,
            } => ToolExecutionTiming::Finished {
                execution_id: execution,
                started_at: *point,
                duration_ms: *duration_ms,
                termination: *termination,
            },
        })
    }

    pub(crate) fn execution_mapping(
        &self,
        execution: ToolExecutionId,
        point: MonotonicPoint,
    ) -> Option<LocalClockMapping> {
        let group = self.root()?;
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return None;
        };
        match spans.attempts.get(&execution)? {
            ExecutionSpan::Started {
                point: known,
                clock: ExecutionStartClock::Mapped(clock),
            } if *known == point => Some(*clock),
            ExecutionSpan::Started { .. } | ExecutionSpan::Finished { .. } => None,
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        self.phase()
            .is_some_and(|phase| matches!(phase, CallPhase::Terminal))
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.phase()
            .is_some_and(|phase| !matches!(phase, CallPhase::Open))
    }

    fn phase(&self) -> Option<CallPhase> {
        let group = self.root()?;
        let group = group.borrow();
        match &*group {
            ClockGroup::Root(spans) => Some(spans.phase),
            ClockGroup::Joined(_) => None,
        }
    }

    pub(crate) fn close(&self) {
        let Some(group) = self.root() else {
            return;
        };
        let mut group = group.borrow_mut();
        if let ClockGroup::Root(spans) = &mut *group {
            match spans.phase {
                CallPhase::Open => spans.phase = CallPhase::Unconfirmed,
                CallPhase::Unconfirmed | CallPhase::Terminal => {}
            }
        }
    }

    pub(crate) fn settle(&self) {
        let Some(group) = self.root() else {
            return;
        };
        let mut group = group.borrow_mut();
        if let ClockGroup::Root(spans) = &mut *group {
            spans.phase = CallPhase::Terminal;
        }
    }

    pub(crate) fn merge_completed(&self, other: &Self) -> Self {
        let (Some(own), Some(other)) = (self.root(), other.root()) else {
            return Self::default();
        };
        if Rc::ptr_eq(&own, &other) {
            return self.clone();
        }
        let mut own_state = own.borrow_mut();
        let mut other_state = other.borrow_mut();
        let (ClockGroup::Root(own_spans), ClockGroup::Root(other_spans)) =
            (&mut *own_state, &mut *other_state)
        else {
            return Self::default();
        };
        if !matches!(own_spans.phase, CallPhase::Terminal)
            || !matches!(other_spans.phase, CallPhase::Terminal)
        {
            return Self::default();
        }
        own_spans.attempts.extend(other_spans.attempts.drain());
        own_spans
            .missing_calls
            .extend(other_spans.missing_calls.drain());
        *other_state = ClockGroup::Joined(Rc::clone(&own));
        self.clone()
    }

    /// Sum actual attempts, excluding permission, recovery and postflight waits.
    pub(crate) fn elapsed_ms(&self) -> Option<i64> {
        self.elapsed_with(Instant::now)
    }

    #[cfg(test)]
    pub(crate) fn elapsed_at(&self, now: Instant) -> Option<i64> {
        self.elapsed_with(|| now)
    }

    fn elapsed_with(&self, mut now: impl FnMut() -> Instant) -> Option<i64> {
        let group = self.root()?;
        let group = group.borrow();
        let ClockGroup::Root(spans) = &*group else {
            return None;
        };
        if spans.attempts.is_empty() || !spans.missing_calls.is_empty() {
            return None;
        }
        let mut elapsed = Duration::ZERO;
        let mut observed_now = None;
        for span in spans.attempts.values() {
            let duration = match span {
                ExecutionSpan::Finished { duration_ms, .. } => Duration::from_millis(*duration_ms),
                ExecutionSpan::Started { clock, .. } => match (spans.phase, clock) {
                    (
                        CallPhase::Open,
                        ExecutionStartClock::Mapped(LocalClockMapping::Correlated {
                            latest_at,
                            ..
                        }),
                    ) => {
                        let at = *observed_now.get_or_insert_with(&mut now);
                        at.checked_duration_since(latest_at.into_std())
                            .unwrap_or_default()
                    }
                    (
                        CallPhase::Unconfirmed | CallPhase::Terminal,
                        ExecutionStartClock::Mapped(LocalClockMapping::Correlated { .. }),
                    )
                    | (
                        CallPhase::Open | CallPhase::Unconfirmed | CallPhase::Terminal,
                        ExecutionStartClock::Mapped(LocalClockMapping::Uncorrelated)
                        | ExecutionStartClock::Unconfirmed,
                    ) => return None,
                },
            };
            elapsed = elapsed.checked_add(duration)?;
        }
        i64::try_from(elapsed.as_millis()).ok()
    }
}
