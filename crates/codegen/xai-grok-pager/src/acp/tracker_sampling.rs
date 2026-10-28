use super::*;
use crate::acp::meta::SamplingMetadata;
use xai_grok_shell::extensions::notification::RetryState;
use xai_grok_shell::extensions::sampling_lifecycle::{
    RequestAttemptId, RetryClockMapping, SamplingLifecycle, SamplingOwner, SamplingPhase,
};

#[derive(Debug, Default)]
pub(super) enum RetryActivity {
    #[default]
    Inactive,
    Legacy(TurnActivity),
    Scheduled { owner: SamplingOwner, activity: TurnActivity, clock: RetryClockMapping },
}

#[derive(Debug)]
pub(super) struct SamplingRequest {
    prompt_id: String,
    attempt: RequestAttemptId,
    phase: RequestPhase,
}

#[derive(Debug, Clone, Copy)]
enum RequestPhase {
    Reserved,
    Dispatched { started_at: xai_grok_shell::extensions::tool_timing::MonotonicPoint, clock: xai_grok_shell::extensions::tool_timing::LocalClockMapping },
    OutputObserved,
    Finished,
}

impl AcpUpdateTracker {
    pub(crate) fn current_sampling_mapping(&self, owner: &SamplingOwner, point: xai_grok_shell::extensions::tool_timing::MonotonicPoint) -> Option<xai_grok_shell::extensions::tool_timing::LocalClockMapping> {
        let request = self.sampling_requests.get(&owner.request_id)?;
        if request.prompt_id != owner.prompt_id || request.attempt != owner.attempt { return None; }
        match request.phase {
            RequestPhase::Dispatched { started_at, clock } if started_at == point => Some(clock),
            RequestPhase::Reserved | RequestPhase::Dispatched { .. } | RequestPhase::OutputObserved | RequestPhase::Finished => None,
        }
    }

    pub(crate) fn current_sampling_started_at(&self) -> Option<std::time::Instant> {
        let owner = match &self.model_activity {
            ModelActivity::Dispatched { owner } => Some(owner),
            ModelActivity::Thinking { owner } | ModelActivity::Responding { owner } => owner.as_ref(),
            ModelActivity::AwaitingOutput => None,
        };
        let request = self.sampling_requests.get(&owner?.request_id)?;
        match request.phase {
            RequestPhase::Dispatched { clock: xai_grok_shell::extensions::tool_timing::LocalClockMapping::Correlated { latest_at, .. }, .. } => Some(latest_at.into_std()),
            RequestPhase::Dispatched { clock: xai_grok_shell::extensions::tool_timing::LocalClockMapping::Uncorrelated, .. }
            | RequestPhase::Reserved | RequestPhase::OutputObserved | RequestPhase::Finished => None,
        }
    }

    pub(crate) fn current_model_activity(&self, current: &SamplingOwner) -> Option<TurnActivity> {
        match &self.model_activity {
            ModelActivity::Thinking { owner } if owner.as_ref() == Some(current) => Some(TurnActivity::Thinking),
            ModelActivity::Responding { owner } if owner.as_ref() == Some(current) => Some(TurnActivity::Responding),
            ModelActivity::Dispatched { .. } | ModelActivity::Thinking { .. } | ModelActivity::Responding { .. } | ModelActivity::AwaitingOutput => None,
        }
    }

    pub(crate) fn current_retry_activity(&self, current: &SamplingOwner) -> Option<TurnActivity> {
        match &self.retry_activity {
            RetryActivity::Scheduled { owner, .. } if owner == current => self.retry_display_activity(),
            RetryActivity::Scheduled { .. } | RetryActivity::Legacy(_) | RetryActivity::Inactive => None,
        }
    }

    pub(crate) fn adopt_current_sampling(&mut self, current: &xai_grok_shell::session::current_actions::CurrentSampling, scrollback: &mut ScrollbackState) {
        use xai_grok_shell::session::current_actions::CurrentSamplingPhase;
        let owner = &current.owner;
        if let Some(known) = self.sampling_requests.get(&owner.request_id) {
            if known.prompt_id != owner.prompt_id || known.attempt.0 > owner.attempt.0
                || (known.attempt == owner.attempt && matches!(known.phase, RequestPhase::Finished)) { return; }
            if known.attempt == owner.attempt {
                match (&known.phase, &current.phase) {
                    (RequestPhase::Dispatched { .. } | RequestPhase::OutputObserved, CurrentSamplingPhase::Reserved | CurrentSamplingPhase::RetryScheduled { .. }) => return,
                    (RequestPhase::Dispatched { .. }, CurrentSamplingPhase::Submitted { .. }) => return,
                    (RequestPhase::Reserved | RequestPhase::OutputObserved | RequestPhase::Finished, CurrentSamplingPhase::Submitted { .. })
                    | (RequestPhase::Reserved | RequestPhase::Finished, CurrentSamplingPhase::Reserved | CurrentSamplingPhase::RetryScheduled { .. }) => {}
                }
            }
        }
        self.retire_previous_sampling_output(owner, scrollback);
        match &current.phase {
            CurrentSamplingPhase::Reserved => {
                self.sampling_requests.insert(owner.request_id.clone(), SamplingRequest { prompt_id: owner.prompt_id.clone(), attempt: owner.attempt, phase: RequestPhase::Reserved });
            }
            CurrentSamplingPhase::RetryScheduled { deadline } => {
                if !matches!(&self.retry_activity, RetryActivity::Scheduled { owner: known, .. } if known == owner) {
                    self.apply_sampling_lifecycle(&SamplingLifecycle { owner: owner.clone(), phase: SamplingPhase::RetryScheduled { deadline: deadline.clone() } }, None, scrollback);
                }
            }
            CurrentSamplingPhase::Submitted { started_at } => {
                self.apply_sampling_lifecycle(&SamplingLifecycle { owner: owner.clone(), phase: SamplingPhase::DispatchStarted { started_at: *started_at } }, None, scrollback);
            }
        }
    }

    pub(super) fn tool_source_is_retired(&self, owner: &SamplingOwner) -> bool {
        self.sampling_requests.get(&owner.request_id).is_some_and(|request| {
            request.prompt_id != owner.prompt_id || request.attempt.0 > owner.attempt.0
        })
    }

    pub(super) fn retry_display_activity(&self) -> Option<TurnActivity> {
        match &self.retry_activity {
            RetryActivity::Inactive => None,
            RetryActivity::Legacy(activity) => Some(activity.clone()),
            RetryActivity::Scheduled { activity, clock, .. } => {
                let mut activity = activity.clone();
                if let TurnActivity::Retrying { due_at, .. } | TurnActivity::Waiting(WaitingReason::RetryBackoff { due_at }) = &mut activity {
                    *due_at = match clock {
                        RetryClockMapping::Correlated { latest_due_at, .. } => Some(latest_due_at.into_std()),
                        RetryClockMapping::Uncorrelated => None,
                    };
                }
                Some(activity)
            }
        }
    }

    pub(crate) fn has_live_sampling_request(&self) -> bool {
        self.sampling_requests.values().any(|request| match request.phase {
            RequestPhase::Reserved | RequestPhase::Dispatched { .. } | RequestPhase::OutputObserved => true,
            RequestPhase::Finished => false,
        })
    }

    pub(crate) fn accepts_sampling_owner(&self, owner: &SamplingOwner) -> bool {
        self.sampling_requests.get(&owner.request_id).is_none_or(|request| {
            request.prompt_id == owner.prompt_id && request.attempt == owner.attempt
                && matches!(request.phase, RequestPhase::Dispatched { .. } | RequestPhase::OutputObserved)
        })
    }

    pub(crate) fn accepts_sampling_acp(&self, update: &acp::SessionUpdate, meta: &NotificationMeta) -> bool {
        match &meta.sampling {
            SamplingMetadata::Malformed | SamplingMetadata::Lifecycle(_) => false,
            SamplingMetadata::Legacy => true,
            SamplingMetadata::Owner(owner) => {
                meta.is_replay
                    || !matches!(update, acp::SessionUpdate::AgentMessageChunk(_) | acp::SessionUpdate::AgentThoughtChunk(_))
                    || self.accepts_sampling_owner(owner)
            }
        }
    }

    pub(crate) fn accepts_sampling_lifecycle(&self, lifecycle: &SamplingLifecycle) -> bool {
        let owner = &lifecycle.owner;
        let Some(request) = self.sampling_requests.get(&owner.request_id) else { return true; };
        if request.prompt_id != owner.prompt_id { return false; }
        match &lifecycle.phase {
            SamplingPhase::RetryScheduled { .. } => {
                owner.attempt.0 > request.attempt.0
            }
            SamplingPhase::DispatchStarted { .. } => {
                owner.attempt == request.attempt && matches!(request.phase, RequestPhase::Reserved | RequestPhase::OutputObserved)
            }
            SamplingPhase::Finished { .. } => {
                owner.attempt == request.attempt && !matches!(request.phase, RequestPhase::Finished)
            }
        }
    }

    pub(crate) fn apply_sampling_lifecycle(
        &mut self,
        lifecycle: &SamplingLifecycle,
        retry: Option<&RetryState>,
        scrollback: &mut ScrollbackState,
    ) {
        let owner = &lifecycle.owner;
        let phase = match &lifecycle.phase {
            SamplingPhase::RetryScheduled { deadline } => {
                let clock = deadline.map_local();
                let activity = match retry {
                    Some(RetryState::Retrying { attempt, max_retries, reason, error_type, .. }) => TurnActivity::Retrying {
                        attempt: *attempt, max_retries: *max_retries, reason: reason.clone(),
                        error_type: error_type.clone(), due_at: None,
                    },
                    Some(RetryState::Exhausted { .. } | RetryState::Failed { .. }) | None => {
                        TurnActivity::Waiting(WaitingReason::RetryBackoff { due_at: None })
                    }
                };
                self.retry_activity = RetryActivity::Scheduled { owner: owner.clone(), activity, clock };
                self.retire_previous_sampling_output(owner, scrollback);
                RequestPhase::Reserved
            }
            SamplingPhase::DispatchStarted { started_at } => {
                self.settle_sampling_retry(owner);
                if matches!(self.model_activity, ModelActivity::AwaitingOutput)
                    || matches!(&self.model_activity, ModelActivity::Dispatched { owner: current } if current.request_id == owner.request_id) {
                    self.model_activity = ModelActivity::Dispatched { owner: owner.clone() };
                }
                RequestPhase::Dispatched { started_at: *started_at, clock: started_at.map_local() }
            }
            SamplingPhase::Finished { .. } => {
                self.settle_sampling_retry(owner);
                let model_owner = match &self.model_activity {
                    ModelActivity::Dispatched { owner } => Some(owner),
                    ModelActivity::Thinking { owner } | ModelActivity::Responding { owner } => owner.as_ref(),
                    ModelActivity::AwaitingOutput => None,
                };
                if model_owner == Some(owner) {
                    self.close_sampling_output(scrollback);
                }
                if matches!(&self.writing_stream, WritingDeltaStream::Owned(writing) if writing == owner) {
                    self.writing_tool_call = None;
                    self.writing_tool_names.clear();
                }
                RequestPhase::Finished
            }
        };
        self.sampling_requests.insert(owner.request_id.clone(), SamplingRequest {
            prompt_id: owner.prompt_id.clone(), attempt: owner.attempt, phase,
        });
        if matches!(self.model_activity, ModelActivity::AwaitingOutput)
            && let Some((request_id, request)) = self.sampling_requests.iter().find(|(_, request)| matches!(request.phase, RequestPhase::Dispatched { .. })) {
            self.model_activity = ModelActivity::Dispatched { owner: SamplingOwner {
                prompt_id: request.prompt_id.clone(), request_id: request_id.clone(), attempt: request.attempt,
            } };
        }
    }

    fn retire_previous_sampling_output(&mut self, owner: &SamplingOwner, scrollback: &mut ScrollbackState) {
        let model_is_previous = match &self.model_activity {
            ModelActivity::Dispatched { owner: previous } | ModelActivity::Thinking { owner: Some(previous) }
            | ModelActivity::Responding { owner: Some(previous) } => previous.request_id == owner.request_id && previous.attempt.0 < owner.attempt.0,
            ModelActivity::Thinking { owner: None } | ModelActivity::Responding { owner: None } | ModelActivity::AwaitingOutput => false,
        };
        if model_is_previous { self.close_sampling_output(scrollback); }
        if matches!(&self.writing_stream, WritingDeltaStream::Owned(previous) if previous.request_id == owner.request_id && previous.attempt.0 < owner.attempt.0) {
            self.writing_tool_call = None;
            self.writing_tool_names.clear();
        }
        if matches!(&self.retry_activity, RetryActivity::Scheduled { owner: previous, .. } if previous.request_id == owner.request_id && previous.attempt.0 < owner.attempt.0) {
            self.retry_activity = RetryActivity::Inactive;
        }
    }

    fn settle_sampling_retry(&mut self, owner: &SamplingOwner) {
        if matches!(&self.retry_activity, RetryActivity::Scheduled { owner: reserved, .. } if reserved == owner) {
            self.retry_activity = RetryActivity::Inactive;
        }
    }

    fn close_sampling_output(&mut self, scrollback: &mut ScrollbackState) {
        self.model_activity = ModelActivity::AwaitingOutput;
        self.finish_thinking(scrollback);
        if let Some(id) = self.current_agent_msg.take() {
            scrollback.finish_running(id);
        }
    }

    pub(super) fn note_sampling_output(&mut self, owner: &SamplingOwner) {
        if !self.sampling_requests.contains_key(&owner.request_id) {
            self.sampling_requests.insert(owner.request_id.clone(), SamplingRequest {
                prompt_id: owner.prompt_id.clone(), attempt: owner.attempt, phase: RequestPhase::OutputObserved,
            });
        }
    }

    pub(super) fn finish_sampling_turn(&mut self) {
        self.retry_activity = RetryActivity::Inactive;
        for request in self.sampling_requests.values_mut() {
            request.phase = RequestPhase::Finished;
        }
    }
}
