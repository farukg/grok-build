use super::*;
use crate::extensions::sampling_lifecycle::{
    SAMPLING_LIFECYCLE_META_KEY, SAMPLING_OWNER_META_KEY, SamplingLifecycle, SamplingOutcome,
    SamplingOwner, SamplingPhase,
};
use xai_grok_sampler::RequestId;
use xai_grok_sampler::request_capture::RequestAttemptId;
use xai_grok_sampler::retry_deadline::RetryDeadline;

use crate::session::current_actions::{SamplingProgress, SamplingRequest};

impl SessionActor {
    pub(super) fn register_sampling_request(&self, request_id: &RequestId) {
        let Some(prompt_id) = self.current_prompt_id.lock().ok().and_then(|id| id.clone()) else {
            return;
        };
        let mut state = self.current_actions.lock();
        if state.requests.contains_key(request_id) {
            return;
        }
        let turn = state.sampling_turn_owner(&prompt_id);
        state.requests.insert(
            request_id.clone(),
            SamplingRequest {
                prompt_id,
                turn,
                progress: SamplingProgress::Reserved(RequestAttemptId(1)),
            },
        );
        state.changed();
    }

    pub(super) fn begin_sampling_turn(&self) {
        let mut state = self.current_actions.lock();
        if !state.requests.is_empty() {
            state.requests.clear();
            state.changed();
        }
    }

    pub(super) fn sampling_request_discarded(&self, request_id: &RequestId) -> bool {
        self.current_actions
            .lock()
            .requests
            .get(request_id)
            .is_some_and(|request| matches!(request.progress, SamplingProgress::Discarded(_)))
    }

    pub(super) fn sampling_owner(&self, request_id: &RequestId) -> Option<SamplingOwner> {
        let requests = self.current_actions.lock();
        let request = requests.requests.get(request_id)?;
        Some(SamplingOwner {
            prompt_id: request.prompt_id.clone(),
            request_id: request_id.clone(),
            attempt: request.progress.attempt(),
        })
    }

    pub(super) fn advance_sampling_attempt(
        &self,
        request_id: &RequestId,
        next_attempt: RequestAttemptId,
    ) -> bool {
        let mut requests = self.current_actions.lock();
        let Some(request) = requests.requests.get_mut(request_id) else {
            return false;
        };
        if matches!(
            request.progress,
            SamplingProgress::Discarded(_) | SamplingProgress::Completed(_)
        ) || next_attempt.0 <= request.progress.attempt().0
        {
            return false;
        }
        request.progress = SamplingProgress::Reserved(next_attempt);
        requests.changed();
        true
    }

    pub(super) fn dispatch_sampling_attempt(
        &self,
        request_id: &RequestId,
        attempt: RequestAttemptId,
        started_at: xai_monotonic_clock::MonotonicPoint,
    ) {
        let _publication = self.notifications.gateway.begin_publication();
        let owner = {
            let mut state = self.current_actions.lock();
            let Some(request) = state.requests.get_mut(request_id) else {
                return;
            };
            match &request.progress {
                SamplingProgress::Reserved(expected)
                | SamplingProgress::RetryScheduled(expected, _)
                    if *expected == attempt => {}
                SamplingProgress::Reserved(_)
                | SamplingProgress::RetryScheduled(_, _)
                | SamplingProgress::Submitted(_, _)
                | SamplingProgress::Completed(_)
                | SamplingProgress::Discarded(_) => return,
            }
            request.progress = SamplingProgress::Submitted(attempt, started_at);
            let owner = SamplingOwner {
                prompt_id: request.prompt_id.clone(),
                request_id: request_id.clone(),
                attempt,
            };
            state.changed();
            owner
        };
        self.enqueue_sampling_phase(owner, SamplingPhase::DispatchStarted { started_at });
    }

    pub(super) fn stamp_sampling_owner(
        meta: &mut serde_json::Map<String, serde_json::Value>,
        owner: &SamplingOwner,
    ) {
        meta.insert("promptId".into(), json!(owner.prompt_id));
        meta.insert("requestId".into(), json!(owner.request_id));
        meta.insert("streamEpoch".into(), json!(owner.attempt));
        if let Ok(value) = serde_json::to_value(owner) {
            meta.insert(SAMPLING_OWNER_META_KEY.into(), value);
        }
    }

    pub(super) fn enqueue_sampling_lifecycle(
        &self,
        lifecycle: SamplingLifecycle,
        update: XaiSessionUpdate,
    ) {
        let mut publication = self.notifications.gateway.begin_publication();
        publication.require_delivery();
        let mut meta = self.build_notification_meta();
        let Some(object) = meta.as_object_mut() else {
            return;
        };
        Self::stamp_sampling_owner(object, &lifecycle.owner);
        self.stamp_current_actions_version(object);
        let Ok(value) = serde_json::to_value(lifecycle) else {
            return;
        };
        object.insert(SAMPLING_LIFECYCLE_META_KEY.into(), value);
        let notification = XaiSessionNotification {
            session_id: self.session_info.id.clone(),
            update,
            meta: Some(meta),
        };
        if self
            .event_tx
            .send(SessionEvent::Notification(notification.into()))
            .is_ok()
        {
            publication.enqueued();
        }
    }

    pub(super) fn enqueue_sampling_phase(&self, owner: SamplingOwner, phase: SamplingPhase) {
        let lifecycle = SamplingLifecycle { owner, phase };
        self.enqueue_sampling_lifecycle(
            lifecycle.clone(),
            XaiSessionUpdate::SamplingLifecycle { lifecycle },
        );
    }

    pub(super) fn schedule_sampling_retry(
        &self,
        request_id: &RequestId,
        deadline: &RetryDeadline,
        state: crate::extensions::notification::RetryState,
    ) {
        let _publication = self.notifications.gateway.begin_publication();
        self.register_sampling_request(request_id);
        let projection = deadline.projection();
        let owner = {
            let mut state = self.current_actions.lock();
            let Some(request) = state.requests.get_mut(request_id) else {
                return;
            };
            if !matches!(request.progress, SamplingProgress::Reserved(_)) {
                return;
            }
            let attempt = request.progress.attempt();
            request.progress = SamplingProgress::RetryScheduled(attempt, projection.clone());
            let owner = SamplingOwner {
                prompt_id: request.prompt_id.clone(),
                request_id: request_id.clone(),
                attempt,
            };
            state.changed();
            owner
        };
        self.enqueue_sampling_lifecycle(
            SamplingLifecycle {
                owner,
                phase: SamplingPhase::RetryScheduled {
                    deadline: projection,
                },
            },
            XaiSessionUpdate::RetryState(state),
        );
    }

    pub(super) fn retry_schedule_error(
        &self,
        reserved: Option<&RequestId>,
        original: &xai_grok_sampler::SamplingErrorInfo,
        requested_backoff: std::time::Duration,
        cause: xai_grok_sampler::retry_deadline::RetryScheduleCause,
    ) -> acp::Error {
        if let Some(request_id) = reserved {
            self.discard_failed_sampling_request(request_id);
        }
        let info = xai_grok_sampler::SamplingErrorInfo::retry_schedule_failure(
            original.clone(),
            requested_backoff,
            cause,
        );
        crate::sampling::error::map_retry_schedule_info_to_acp(info)
    }

    pub(super) fn cancel_sampling_requests(&self) {
        let active: Vec<_> = self
            .current_actions
            .lock()
            .requests
            .iter()
            .filter_map(|(id, request)| request.progress.active().then(|| id.clone()))
            .collect();
        for request_id in active {
            self.finish_sampling_request(&request_id, SamplingOutcome::Cancelled);
        }
    }

    pub(super) fn discard_failed_sampling_request(&self, request_id: &RequestId) {
        self.finish_sampling_request(request_id, SamplingOutcome::Failed);
    }

    pub(super) fn finish_sampling_request(&self, request_id: &RequestId, outcome: SamplingOutcome) {
        let _publication = self.notifications.gateway.begin_publication();
        let owner = {
            let mut requests = self.current_actions.lock();
            let Some(request) = requests.requests.get_mut(request_id) else {
                return;
            };
            if matches!(
                request.progress,
                SamplingProgress::Completed(_) | SamplingProgress::Discarded(_)
            ) {
                return;
            }
            let attempt = request.progress.attempt();
            request.progress = match outcome {
                SamplingOutcome::Completed => SamplingProgress::Completed(attempt),
                SamplingOutcome::Failed | SamplingOutcome::Cancelled => {
                    SamplingProgress::Discarded(attempt)
                }
            };
            let owner = SamplingOwner {
                prompt_id: request.prompt_id.clone(),
                request_id: request_id.clone(),
                attempt,
            };
            requests.changed();
            owner
        };
        self.enqueue_sampling_phase(owner, SamplingPhase::Finished { outcome });
    }

    pub(super) fn settle_sampling_delivery(&self, lifecycle: &SamplingLifecycle) {
        if let SamplingPhase::Finished {
            outcome: SamplingOutcome::Completed,
        } = lifecycle.phase
        {
            let mut requests = self.current_actions.lock();
            if let Some(request) = requests.requests.get_mut(&lifecycle.owner.request_id)
                && request.prompt_id == lifecycle.owner.prompt_id
                && matches!(&request.progress, SamplingProgress::Completed(attempt) if *attempt == lifecycle.owner.attempt)
            {
                request.progress = SamplingProgress::Discarded(lifecycle.owner.attempt);
            }
        }
    }

    pub(super) fn accepts_sampling_owner(&self, owner: &SamplingOwner) -> bool {
        let requests = self.current_actions.lock();
        let Some(request) = requests.requests.get(&owner.request_id) else {
            return false;
        };
        if request.prompt_id != owner.prompt_id {
            return false;
        }
        match &request.progress {
            SamplingProgress::Reserved(attempt)
            | SamplingProgress::RetryScheduled(attempt, _)
            | SamplingProgress::Submitted(attempt, _)
            | SamplingProgress::Completed(attempt) => *attempt == owner.attempt,
            SamplingProgress::Discarded(_) => false,
        }
    }

    pub(super) fn accepts_sampling_notification(&self, notification: &SessionNotification) -> bool {
        let meta = match notification {
            SessionNotification::Acp(notification) => notification.meta.as_ref(),
            SessionNotification::Xai(notification) => notification
                .meta
                .as_ref()
                .and_then(serde_json::Value::as_object),
        };
        let Some(value) = meta.and_then(|meta| meta.get(SAMPLING_OWNER_META_KEY)) else {
            return true;
        };
        let Ok(owner) = serde_json::from_value::<SamplingOwner>(value.clone()) else {
            return false;
        };
        // Transitions use the FIFO rail even when their producer has already advanced.
        if meta.is_some_and(|meta| meta.contains_key(SAMPLING_LIFECYCLE_META_KEY)) {
            return true;
        }
        self.accepts_sampling_owner(&owner)
    }
}

#[cfg(test)]
#[path = "sampling_progress_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sampling_dispatch_tests.rs"]
mod dispatch_tests;
