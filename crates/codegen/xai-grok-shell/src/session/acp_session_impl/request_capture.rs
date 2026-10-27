use super::*;
use crate::session::request_capture::*;

#[cfg(test)]
#[path = "request_capture_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "request_source_binding_tests.rs"]
mod binding_tests;

impl SessionActor {
    pub(super) fn prompt_input_ref(&self, prompt_id: &str) -> PromptInputRef {
        PromptInputRef {
            session_id: self.session_info.id.to_string(), prompt_id: prompt_id.to_owned(),
            turn: self.current_turn_number.get(), generation: self.turn_phases.current_generation(),
        }
    }

    pub(super) fn strict_input_origin(&self, input: &InputOrigin) -> Option<crate::session::source_binding::StrictInputRef> {
        use crate::session::source_binding::StrictInputRef;
        let session_id = self.session_id_string();
        if let super::notification_drain::NotificationBatchPayload::Rows { id, .. } = input.notification_batch() {
            return Some(StrictInputRef::Notification { session_id, batch_id: id.clone() });
        }
        if !input.completions().is_empty() {
            return Some(StrictInputRef::CompletionAggregate { session_id, completions: input.completions().to_vec() });
        }
        match input.as_prompt_origin() {
            PromptOrigin::Delivery { delivery_id, .. } => Some(StrictInputRef::Delivery { session_id, delivery_id: delivery_id.clone() }),
            PromptOrigin::ParentAgentMessage { message_id, .. } | PromptOrigin::ParentHumanMessage { message_id, .. } => Some(StrictInputRef::Delivery {
                session_id, delivery_id: crate::session::delivery_receiver::parent_message_delivery_id(message_id),
            }),
            PromptOrigin::SubagentCompleted { completion } => Some(StrictInputRef::CompletionAggregate { session_id, completions: vec![completion.id.clone().into()] }),
            PromptOrigin::User | PromptOrigin::TaskCompleted { .. } | PromptOrigin::WorkflowCompleted { .. } | PromptOrigin::NotificationDrain
            | PromptOrigin::GoalSummary | PromptOrigin::GoalClassifierNudge | PromptOrigin::SchedulerFired | PromptOrigin::PlanResume => None,
        }
    }

    pub(super) fn observe_input_source(
        &self,
        input: PromptInputRef,
        applied: Result<xai_chat_state::AppliedAppend, InputSourceUnknown>,
    ) {
        let binding = match applied {
            Ok(applied) => Self::applied_source_binding(applied),
            Err(cause) => InputSourceBinding::Unknown { cause },
        };
        let notification = InputSourceNotification { input, binding };
        self.persist_source_observation(crate::session::source_binding::SourceBindingObservation::Input { notification: notification.clone() });
        self.forward_request_diagnostic(INPUT_SOURCE_NOTIFICATION, &notification);
    }

    pub(super) fn persist_source_observation(&self, observation: crate::session::source_binding::SourceBindingObservation) {
        let update = crate::session::storage::SessionUpdate::Xai(Box::new(XaiSessionNotification {
            session_id: self.session_info.id.clone(),
            update: XaiSessionUpdate::RequestSourceObserved { observation },
            meta: None,
        }));
        let (respond_to, _) = oneshot::channel();
        let _ = self.notifications.persistence_tx.send(PersistenceMsg::AppendUpdatesAndAck { updates: vec![update], respond_to });
    }

    pub(super) fn observe_strict_source(
        &self,
        identity: crate::session::source_binding::StrictSourceOrigin,
        result: &Result<xai_chat_state::MutationAck<xai_chat_state::CompletionProjectionAck>, xai_chat_state::StrictAppendError<xai_chat_state::MutationAck<xai_chat_state::CompletionProjectionAck>>>,
    ) {
        use xai_chat_state::{AppendBinding, CompletionProjectionAck, StrictAppendError};
        let acknowledgement = match result {
            Ok(ack) | Err(StrictAppendError::Committed { acknowledgement: ack, .. }) => Some(ack),
            Err(StrictAppendError::NotCommitted(_) | StrictAppendError::Indeterminate(_)) => None,
        };
        if let Some(ack) = acknowledgement {
            match &ack.acknowledgement {
                CompletionProjectionAck::Continue { sources, .. } | CompletionProjectionAck::AlreadyPresent { sources, .. } => {
                    for retained in &sources.retained {
                        self.publish_source_observation(crate::session::source_binding::SourceBindingObservation::Strict {
                            identity: crate::session::source_binding::StrictSourceOrigin::from_receipt(self.session_id_string(), retained.identity.clone()),
                            binding: Self::retained_source_binding(retained.source),
                        });
                    }
                    for missing in &sources.missing {
                        self.publish_source_observation(crate::session::source_binding::SourceBindingObservation::Strict {
                            identity: crate::session::source_binding::StrictSourceOrigin::from_receipt(self.session_id_string(), missing.clone()),
                            binding: InputSourceBinding::Unknown { cause: InputSourceUnknown::StrictReceiptSourceUnavailable },
                        });
                    }
                    if sources.retained.is_empty() && sources.missing.is_empty() {
                        self.publish_source_observation(crate::session::source_binding::SourceBindingObservation::Strict {
                            identity, binding: InputSourceBinding::Unknown { cause: InputSourceUnknown::StrictReceiptSourceUnavailable },
                        });
                    }
                    return;
                }
                CompletionProjectionAck::Appended | CompletionProjectionAck::TurnClosed => {}
            }
        }
        let binding = match result {
            Ok(ack) | Err(StrictAppendError::Committed { acknowledgement: ack, .. }) => match ack.binding {
                AppendBinding::Applied(applied) => Self::applied_source_binding(applied),
                AppendBinding::ReceiptSourceUnavailable => InputSourceBinding::Unknown { cause: InputSourceUnknown::StrictReceiptSourceUnavailable },
                AppendBinding::NotApplied => InputSourceBinding::Unknown { cause: InputSourceUnknown::NotApplied },
            },
            Err(StrictAppendError::NotCommitted(error)) => InputSourceBinding::Unknown { cause: InputSourceUnknown::NotCommitted { cause: CaptureFailure::io(error) } },
            Err(StrictAppendError::Indeterminate(error)) => {
                let cause = match error.get_ref().and_then(|source| source.downcast_ref::<xai_chat_state::MutationAdmissionError>()) {
                    Some(xai_chat_state::MutationAdmissionError::MailboxClosed(_)) => InputSourceUnknown::MailboxClosed,
                    Some(xai_chat_state::MutationAdmissionError::ReplyLost(_)) => InputSourceUnknown::ReplyLost,
                    None => InputSourceUnknown::Indeterminate { cause: CaptureFailure::io(error) },
                };
                InputSourceBinding::Unknown { cause }
            },
        };
        self.publish_source_observation(crate::session::source_binding::SourceBindingObservation::Strict { identity, binding });
    }

    pub(super) fn publish_source_observation(&self, observation: crate::session::source_binding::SourceBindingObservation) {
        self.persist_source_observation(observation.clone());
        self.forward_request_diagnostic(crate::session::source_binding::SOURCE_BINDING_NOTIFICATION, &observation);
    }

    pub(super) fn mutation_source_binding(result: &Result<xai_chat_state::AppliedAppend, xai_chat_state::MutationAdmissionError>) -> InputSourceBinding {
        match result {
            Ok(applied) => Self::applied_source_binding(*applied),
            Err(xai_chat_state::MutationAdmissionError::MailboxClosed(_)) => InputSourceBinding::Unknown { cause: InputSourceUnknown::MailboxClosed },
            Err(xai_chat_state::MutationAdmissionError::ReplyLost(_)) => InputSourceBinding::Unknown { cause: InputSourceUnknown::ReplyLost },
        }
    }

    pub(super) fn applied_source_binding(applied: xai_chat_state::AppliedAppend) -> InputSourceBinding {
        Self::retained_source_binding(applied.source)
    }

    fn retained_source_binding(source: xai_grok_sampling_types::SourceOccurrence) -> InputSourceBinding {
        match source {
            source @ xai_grok_sampling_types::SourceOccurrence::Admitted { .. } => InputSourceBinding::Applied { meta: AppliedSourceMeta { publication: SourcePublication {
                source, evidence: Default::default(), durability: SourceDurability::UnconfirmedIndependentHistoryChannel,
            } } },
            xai_grok_sampling_types::SourceOccurrence::Unadmitted | xai_grok_sampling_types::SourceOccurrence::LegacyNormalized { .. } => InputSourceBinding::Unknown { cause: InputSourceUnknown::StrictReceiptSourceUnavailable },
        }
    }

    pub(super) async fn restore_request_source_publications(&self) -> Result<(), CaptureFailure> {
        if self.notifications.source_publications.lock().needs_restore() {
            let (respond_to, response) = tokio::sync::oneshot::channel();
            let result = if self.notifications.persistence_tx.send(PersistenceMsg::ReadPublishedRequestSources { respond_to }).is_err() {
                Err(CaptureFailure::MailboxClosed)
            } else {
                match response.await {
                    Ok(Ok(sources)) => Ok(sources),
                    Ok(Err(error)) => Err(CaptureFailure::io(&error)),
                    Err(_) => Err(CaptureFailure::ReplyLost),
                }
            };
            self.notifications.source_publications.lock().restore(result);
        }
        match self.notifications.source_publications.lock().failure() {
            Some(cause) => Err(cause),
            None => Ok(()),
        }
    }

    pub(super) fn begin_request_capture(
        &self,
        request: RequestSnapshotId,
        body: &ConversationRequest,
    ) {
        let origin = RequestOrigin {
            session_id: self.session_info.id.to_string(), request,
            turn: self.current_turn_number.get(),
            generation: self.turn_phases.current_generation(),
            prompt_id: self.current_prompt_id.lock().ok().and_then(|id| id.clone()),
            conversation_id: body.x_grok_conv_id.clone(),
            logical_request_id: body.x_grok_req_id.clone(),
        };
        let _ = self.notifications.persistence_tx.send(PersistenceMsg::BeginRequestCapture { origin });
    }

    pub(super) async fn observe_local_request(
        &self,
        request_id: xai_grok_sampler::RequestId,
        attempt: xai_grok_sampler::request_capture::RequestAttemptId,
        evidence: xai_grok_sampler::request_capture::LocalRequestEvidence,
    ) {
        let Ok(id) = uuid::Uuid::try_parse(request_id.as_str()) else { return; };
        let request = RequestSnapshotId(id);
        let mut fallback = RequestSnapshotNotification {
            snapshot: RequestSnapshotRef { session_id: self.session_info.id.to_string(), request },
            evidence: AttemptObservation {
                attempt, observation: crate::session::storage::request_capture::observation(&evidence),
            },
            receipt: CaptureReceipt::Unconfirmed { cause: CaptureFailure::MailboxClosed },
        };
        let (respond_to, response) = tokio::sync::oneshot::channel();
        let notification = if self.notifications.persistence_tx.send(PersistenceMsg::RecordRequestCapture {
            request, attempt, evidence, respond_to,
        }).is_ok() {
            match response.await {
                Ok(notification) => notification,
                Err(_) => {
                    fallback.receipt = CaptureReceipt::Unconfirmed { cause: CaptureFailure::ReplyLost };
                    fallback
                }
            }
        } else { fallback };
        self.forward_request_diagnostic(REQUEST_SNAPSHOT_NOTIFICATION, &notification);
    }

    pub(super) fn forward_request_diagnostic(&self, method: &str, notification: &impl serde::Serialize) {
        if !self.notifications.gateway_enabled.load(std::sync::atomic::Ordering::Relaxed) { return; }
        if let Ok(params) = serde_json::value::to_raw_value(notification) {
            self.notifications.gateway.forward_fire_and_forget(acp::ExtNotification::new(method.to_owned(), params.into()));
        }
    }
}
