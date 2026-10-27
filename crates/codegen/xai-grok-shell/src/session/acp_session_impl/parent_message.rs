//! Admission and safe-point delivery of messages from an owning parent agent.

use super::parent_interject::ParentInterjectSignal;
use super::*;
use crate::session::telemetry::ActiveAgentMessageSafePointTrigger;
use std::sync::Arc;
use xai_grok_tools::implementations::grok_build::task::coordinator::ActiveMessageAdmission;
use xai_grok_tools::implementations::grok_build::task::types::{
    ActiveAgentMessage, ActiveAgentMessageDelivery, ActiveAgentMessageOperation,
    ActiveAgentMessageSource,
};
use xai_message_delivery_core::{
    DeliveryMessage, MessageDeliveryLifecycle, OwnedDelivery, TerminalCause, TerminalTarget,
    TurnBinding,
};

#[derive(Clone)]
pub(super) struct ParentMessageOrigin {
    sender_session_id: String,
    source: ActiveAgentMessageSource,
}

#[derive(Clone)]
pub(super) struct PendingParentAgentMessage {
    prompt_id: String,
    message_id: String,
    text: Arc<str>,
    /// Effective operation; a safe-point slot holds `Steer` or `Interject`, which sets drain order.
    operation: ActiveAgentMessageOperation,
    telemetry: crate::session::telemetry::ActiveAgentMessageAdmissionTelemetry,
}

type ParentMessageCompletion = oneshot::Sender<crate::session::commands::PromptTurnResult>;
pub(super) type ParentDeliveryMessage =
    DeliveryMessage<String, ParentMessageOrigin, PendingParentAgentMessage>;
type ParentTurnBinding = TurnBinding<String, TurnEpoch>;
type ParentMessageLifecycle = MessageDeliveryLifecycle<
    String,
    ParentMessageOrigin,
    PendingParentAgentMessage,
    ParentMessageCompletion,
    String,
    TurnEpoch,
>;
pub(super) type ParentOwnedDelivery = OwnedDelivery<
    String,
    ParentMessageOrigin,
    PendingParentAgentMessage,
    ParentMessageCompletion,
    String,
    TurnEpoch,
>;

/// Named bound on live Steer and Interject slots waiting for the next safe point.
/// Parent text becomes model-visible in one batch; this keeps that batch finite.
const MAX_PARENT_SAFE_POINT_SLOTS: usize = 32;

/// A safe-point delivery commit could not reach a downstream actor;
/// the slots stay projecting for terminal settlement.
enum DrainCommitError {
    PersistenceUnavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ParentTeardownStatus {
    #[default]
    Open,
    Settled,
    Unsettled,
}

#[derive(Clone, Copy)]
pub(super) enum ParentRunningTeardown { Preserve, Stop }

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentTeardownRequest { Ordinary, HardTeardown, ActorDrop }

#[cfg(test)]
#[derive(Default)]
enum ParentProjectionCut {
    #[default]
    Live,
    Park { entered: oneshot::Sender<()>, release: oneshot::Receiver<()> },
}

pub(super) fn parent_teardown_uncertain() -> acp::Error {
    acp::Error::internal_error().data("parent admission outcome uncertain after unacknowledged teardown")
}

/// Every slot mutation goes through this impl so the interject signal is recounted from the
/// lifecycle rather than kept in step by callers.
#[derive(Default)]
pub(crate) struct MessageDeliveryState {
    lifecycle: ParentMessageLifecycle,
    interject_signal: Arc<ParentInterjectSignal>,
    pub(super) teardown: ParentTeardownStatus,
    #[cfg(test)]
    projection_cut: ParentProjectionCut,
}

impl MessageDeliveryState {
    pub(super) fn interject_signal(&self) -> Arc<ParentInterjectSignal> {
        Arc::clone(&self.interject_signal)
    }

    fn has_identity(&self, identity: &String) -> bool {
        self.lifecycle.contains_identity(identity)
    }

    fn is_at_slot_cap(&self) -> bool {
        self.lifecycle.len() >= MAX_PARENT_SAFE_POINT_SLOTS
    }

    fn sync_interject_signal(&self, running: Option<&ParentTurnBinding>) {
        let has_pending = running.is_some_and(|binding| {
            self.lifecycle
                .pending_messages(binding)
                .iter()
                .any(|message| message.content().is_interject())
        });
        self.interject_signal.set_pending(has_pending);
    }

    fn admit_pending(
        &mut self,
        binding: &ParentTurnBinding,
        message: ParentDeliveryMessage,
        completion: ParentMessageCompletion,
    ) -> Result<(), ParentDeliveryMessage> {
        let admitted = self
            .lifecycle
            .admit_pending(binding.clone(), message, completion);
        self.sync_interject_signal(Some(binding));
        admitted
    }

    /// Consumes the wait-abort mark in the same step, so an abort noted after this call belongs
    /// to a later drain and a failed barrier cannot leave it set; only a mark noted for `running`
    /// counts.
    fn begin_delivery(
        &mut self,
        running: &AgentTask,
    ) -> (
        Vec<ParentDeliveryMessage>,
        ActiveAgentMessageSafePointTrigger,
    ) {
        let binding = turn_binding(running);
        let messages = self.lifecycle.begin_delivery(&binding);
        self.sync_interject_signal(Some(&binding));
        let trigger = if self.interject_signal.take_wait_aborted(running.epoch) {
            ActiveAgentMessageSafePointTrigger::WaitAbort
        } else {
            ActiveAgentMessageSafePointTrigger::Natural
        };
        (messages, trigger)
    }

    fn finish_delivery<Error>(
        &mut self,
        binding: &ParentTurnBinding,
        commit: impl FnOnce(&[ParentDeliveryMessage]) -> Result<(), Error>,
    ) -> Result<Vec<ParentDeliveryMessage>, Error> {
        self.lifecycle.finish_delivery(binding, commit)
    }

    /// `running` is the turn still live after the transition, if any; a stale completion for an
    /// earlier turn must not silence its pending slots or drop its wait-abort mark.
    fn transition(
        &mut self,
        target: TerminalTarget<'_, String, TurnEpoch>,
        cause: TerminalCause,
        running: Option<&AgentTask>,
    ) -> xai_message_delivery_core::TerminalTransition<
        String,
        ParentMessageOrigin,
        PendingParentAgentMessage,
        ParentMessageCompletion,
        String,
        TurnEpoch,
    > {
        let ends_running_turn = match &target {
            TerminalTarget::All => true,
            TerminalTarget::Turn(binding) => {
                running.is_none_or(|task| **binding == turn_binding(task))
            }
        };
        let transition = self.lifecycle.transition(target, cause);
        // The ended turn's mark must not be attributed to the next turn's first drain.
        if ends_running_turn {
            self.interject_signal.clear_wait_aborted();
        }
        self.sync_interject_signal(running.map(turn_binding).as_ref());
        transition
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.lifecycle.is_empty()
    }
}

impl Drop for MessageDeliveryState {
    fn drop(&mut self) {
        let transition = self.transition(TerminalTarget::All, TerminalCause::ActorDrop, None);
        let result = Err(parent_teardown_uncertain());
        transition
            .completions
            .into_iter()
            .chain(transition.fallbacks)
            .for_each(|owned| {
                let (_, _, completion) = owned.into_parts();
                let _ = completion.send(result.clone());
            });
    }
}

impl PendingParentAgentMessage {
    pub(super) fn is_interject(&self) -> bool {
        self.operation == ActiveAgentMessageOperation::Interject
    }

    fn into_input(
        self,
        origin: ParentMessageOrigin,
        respond_to: ParentMessageCompletion,
    ) -> InputItem {
        let prompt_blocks = vec![acp::ContentBlock::Text(acp::TextContent::new(
            self.text.to_string(),
        ))];
        let queue_meta = crate::session::prompt_queue::QueueEntryMeta {
            id: self.prompt_id.clone(),
            version: 0,
            owner: None,
            last_editor: None,
            kind: "parent_agent_message".to_owned(),
            text: SessionActor::queue_text_from_blocks(&prompt_blocks),
            combined_texts: None,
        };
        InputItem {
            prompt_id: self.prompt_id,
            prompt_blocks,
            prompt_mode: PromptMode::Agent,
            trace_gcs_config: None,
            artifact_tracker: None,
            client_identifier: None,
            screen_mode: None,
            verbatim: matches!(origin.source, ActiveAgentMessageSource::Human),
            json_schema: None,
            input_origin: InputOrigin::new(
                if matches!(origin.source, ActiveAgentMessageSource::Human) {
                    super::PromptOrigin::ParentHumanMessage {
                        message_id: self.message_id,
                        sender_session_id: origin.sender_session_id,
                    }
                } else {
                    super::PromptOrigin::ParentAgentMessage {
                        message_id: self.message_id,
                        sender_session_id: origin.sender_session_id,
                    }
                },
            ),
            task_wake_fallback: None,
            tool_overrides_update: None,
            respond_to,
            persist_ack: None,
            parsed_prompt_tx: None,
            initial_child_prompt_ready: None,
            queue_meta: Some(queue_meta),
            queue_mutation_policy: QueueMutationPolicy::new(true, false),
            send_now: false,
            traceparent: None,
        }
    }
}

fn turn_binding(task: &AgentTask) -> TurnBinding<String, TurnEpoch> {
    TurnBinding::new(task.prompt_id.clone(), task.epoch)
}

fn contains_queued_identity(state: &State, identity: &str) -> bool {
    state.pending_inputs.iter().any(|item| {
        matches!(
            item.input_origin.as_prompt_origin(),
            PromptOrigin::ParentAgentMessage { message_id, .. }
            | PromptOrigin::ParentHumanMessage { message_id, .. }
                if message_id == identity
        )
    })
}

impl SessionActor {
    pub(super) async fn admit_parent_agent_message(
        self: &Arc<Self>,
        delivery: ActiveAgentMessageDelivery,
        receipt_sink: mpsc::Sender<crate::agent::subagent::PromptTurnReceipt>,
        parent_telemetry_ctx: xai_grok_telemetry::TelemetryCtx,
        respond_to: oneshot::Sender<ActiveMessageAdmission>,
        completion_tx: mpsc::UnboundedSender<super::turn_task::TurnCompletionMsg>,
    ) {
        let message = delivery.message().clone();
        let requested = delivery.operation();
        let source = delivery.source();
        self.admit_parent_agent_message_inner(
            Some(delivery),
            source,
            message,
            requested,
            receipt_sink,
            parent_telemetry_ctx,
            respond_to,
            completion_tx,
        )
        .await;
    }

    async fn admit_parent_agent_message_inner(
        self: &Arc<Self>,
        delivery: Option<ActiveAgentMessageDelivery>,
        message_source: ActiveAgentMessageSource,
        message: ActiveAgentMessage,
        requested: ActiveAgentMessageOperation,
        receipt_sink: mpsc::Sender<crate::agent::subagent::PromptTurnReceipt>,
        parent_telemetry_ctx: xai_grok_telemetry::TelemetryCtx,
        respond_to: oneshot::Sender<ActiveMessageAdmission>,
        completion_tx: mpsc::UnboundedSender<super::turn_task::TurnCompletionMsg>,
    ) {
        let receipt_permit = match receipt_sink.reserve_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                let _ = respond_to.send(ActiveMessageAdmission::ChannelClosed);
                return;
            }
        };
        self.ensure_prefix_ready().await;

        let prompt_id = format!("parent-message-{}", message.message_id);
        let (turn_result_tx, turn_result_rx) = oneshot::channel();
        let admitted_at = std::time::Instant::now();
        let mut state = self.state.lock().await;
        if state.message_delivery.has_identity(&message.message_id)
            || contains_queued_identity(&state, &message.message_id)
        {
            let _ = respond_to.send(ActiveMessageAdmission::Rejected);
            return;
        }
        let effective = match (requested, state.running_task.as_ref()) {
            (ActiveAgentMessageOperation::Steer, Some(_)) => ActiveAgentMessageOperation::Steer,
            (ActiveAgentMessageOperation::Interject, Some(_)) => {
                ActiveAgentMessageOperation::Interject
            }
            (
                ActiveAgentMessageOperation::Queue
                | ActiveAgentMessageOperation::Steer
                | ActiveAgentMessageOperation::Interject,
                None,
            )
            | (ActiveAgentMessageOperation::Queue, Some(_)) => ActiveAgentMessageOperation::Queue,
        };
        if effective != ActiveAgentMessageOperation::Queue
            && state.message_delivery.is_at_slot_cap()
        {
            let _ = respond_to.send(ActiveMessageAdmission::Rejected);
            return;
        }
        let telemetry = crate::session::telemetry::ActiveAgentMessageAdmissionTelemetry::new(
            admitted_at,
            parent_telemetry_ctx,
            requested,
            effective,
            (requested != ActiveAgentMessageOperation::Queue
                && effective == ActiveAgentMessageOperation::Queue)
                .then_some(crate::session::telemetry::ActiveAgentMessageFallbackReason::Idle),
        );
        use crate::session::delivery_receiver::{ParentMessageAdmission, ParentMessageOperation, ParentMessageSource};
        let durable_operation = match effective {
            ActiveAgentMessageOperation::Queue => ParentMessageOperation::Queue,
            ActiveAgentMessageOperation::Steer | ActiveAgentMessageOperation::Interject => {
                let Some(task) = state.running_task.as_ref() else {
                    let _ = respond_to.send(ActiveMessageAdmission::Rejected);
                    return;
                };
                let prompt_id = task.prompt_id.clone();
                let parent_task = xai_chat_state::ParentTask::Instance { id: task.identity.as_ref().clone() };
                match effective {
                    ActiveAgentMessageOperation::Steer => ParentMessageOperation::Steer { prompt_id, parent_task },
                    ActiveAgentMessageOperation::Interject => ParentMessageOperation::Interject { prompt_id, parent_task },
                    ActiveAgentMessageOperation::Queue => ParentMessageOperation::Queue,
                }
            }
        };
        let admission = ParentMessageAdmission {
            message_id: message.message_id.clone(),
            sender_session_id: message.sender_session_id.clone(),
            source: match message_source { ActiveAgentMessageSource::Agent => ParentMessageSource::Agent, ActiveAgentMessageSource::Human => ParentMessageSource::Human },
            effective: durable_operation,
            prompt_id: prompt_id.clone(),
            text: message.text.to_string(),
        };
        use crate::session::delivery_receiver::{ParentAdmissionAck, ParentAdmissionAuthority};
        let expected_delivery = delivery.clone();
        let authority = match delivery {
            Some(delivery) => match delivery.claim_admission() {
                Some(claim) => ParentAdmissionAuthority::Coordinator(claim),
                None => {
                    let _ = respond_to.send(ActiveMessageAdmission::Rejected);
                    return;
                }
            },
            None => ParentAdmissionAuthority::DirectLocal,
        };
        // Keep the exact live binding reserved until the receiver owner acknowledges admission.
        let (persisted_tx, persisted_rx) = oneshot::channel();
        if self.notifications.persistence_tx.send(PersistenceMsg::AdmitParentMessage { admission, authority, respond_to: persisted_tx }).is_err() {
            let _ = respond_to.send(ActiveMessageAdmission::ChannelClosed);
            return;
        }
        let proof = match persisted_rx.await {
            Ok(Ok(proof)) => proof,
            Ok(Err(_)) | Err(_) => {
                let _ = respond_to.send(ActiveMessageAdmission::ChannelClosed);
                return;
            }
        };
        match (&expected_delivery, &proof) {
            (Some(expected), ParentAdmissionAck::Coordinator(proof))
                if proof.delivery().message() == expected.message()
                    && proof.delivery().operation() == expected.operation()
                    && proof.delivery().source() == expected.source()
                    && proof.delivery().route() == expected.route() => {}
            (None, ParentAdmissionAck::DirectLocal) => {}
            (Some(_), ParentAdmissionAck::Coordinator(_) | ParentAdmissionAck::DirectLocal)
            | (None, ParentAdmissionAck::Coordinator(_)) => {
                let _ = respond_to.send(ActiveMessageAdmission::ChannelClosed);
                return;
            }
        }
        let content = PendingParentAgentMessage {
            prompt_id: prompt_id.clone(),
            message_id: message.message_id.clone(),
            text: message.text,
            operation: effective,
            telemetry: telemetry.clone(),
        };
        let origin = ParentMessageOrigin {
            sender_session_id: message.sender_session_id,
            source: message_source,
        };
        let commit = || {
            state.message_delivery.teardown = ParentTeardownStatus::Open;
            match effective {
                ActiveAgentMessageOperation::Queue => {
                    let item = content.into_input(origin, turn_result_tx);
                    self.commit_queued_delivery(
                        &mut state,
                        super::prompt_queue::PreparedDelivery(item),
                    );
                }
                ActiveAgentMessageOperation::Steer | ActiveAgentMessageOperation::Interject => {
                    let binding = state.running_task.as_ref().map(turn_binding).ok_or(())?;
                    state.message_delivery.admit_pending(
                        &binding,
                        DeliveryMessage::new(message.message_id, origin, content),
                        turn_result_tx,
                    ).map_err(|_| ())?;
                }
            }
            let receipt_sender = receipt_permit.send(crate::agent::subagent::PromptTurnReceipt {
                prompt_id,
                result: turn_result_rx,
                telemetry,
            });
            if receipt_sender.is_closed() {
                return Err(());
            }
            Ok::<(), ()>(())
        };
        let committed = match proof {
            ParentAdmissionAck::Coordinator(proof) => proof.try_handoff(commit),
            ParentAdmissionAck::DirectLocal => commit(),
        };
        if committed.is_err() {
            let _ = respond_to.send(ActiveMessageAdmission::ChannelClosed);
            return;
        }
        drop(state);

        let _ = respond_to.send(ActiveMessageAdmission::Admitted);
        Self::maybe_start_running_task(self.clone(), completion_tx).await;
    }

    pub(super) async fn project_parent_message(
        &self,
        message_id: &str,
        source: ActiveAgentMessageSource,
        turn_id: &str,
        text: String,
    ) -> Result<bool, acp::Error> {
        let admission = self.current_turn_admission(turn_id).await
            .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
        let projected = self.commit_parent_message_projection(message_id, source, turn_id, text, admission).await?;
        #[cfg(test)]
        {
            let cut = std::mem::take(&mut self.state.lock().await.message_delivery.projection_cut);
            match cut {
                ParentProjectionCut::Live => {}
                ParentProjectionCut::Park { entered, release } => {
                    let _ = entered.send(());
                    let _ = release.await;
                }
            }
        }
        Ok(projected)
    }

    async fn commit_parent_message_projection(
        &self,
        message_id: &str,
        source: ActiveAgentMessageSource,
        turn_id: &str,
        text: String,
        admission: xai_chat_state::InputAdmission,
    ) -> Result<bool, acp::Error> {
        let item = match source {
            ActiveAgentMessageSource::Agent => ConversationItem::agent_message(text.clone()),
            ActiveAgentMessageSource::Human => ConversationItem::parent_human_message(text.clone()),
        };
        let projection = xai_chat_state::DeliveryProjection {
            item,
            binding: xai_chat_state::DeliveryBinding {
                delivery_id: crate::session::delivery_receiver::parent_message_delivery_id(message_id),
                payload_digest: crate::session::delivery_receiver::payload_digest(&text),
                turn_id: turn_id.to_owned(),
                parent_task: admission.parent_task.clone(),
            },
        };
        let projection = xai_chat_state::InputProjection::Delivery(projection);
        let identity = crate::session::source_binding::StrictSourceOrigin::from_projection(self.session_id_string(), &projection);
        let result = self.chat_state_handle.append_input_with_source_and_ack(projection, admission).await;
        self.observe_strict_source(identity, &result);
        match result.map(|ack| ack.acknowledgement).map_err(|error| error.map_acknowledgement(|ack| ack.acknowledgement)) {
            Ok(xai_chat_state::CompletionProjectionAck::Appended) => Ok(true),
            Ok(xai_chat_state::CompletionProjectionAck::AlreadyPresent { .. }) => Ok(false),
            Ok(xai_chat_state::CompletionProjectionAck::TurnClosed) => Err(acp::Error::internal_error().data("parent-message turn closed before projection")),
            Ok(xai_chat_state::CompletionProjectionAck::Continue { .. }) => Err(acp::Error::internal_error().data("parent-message execution cannot replay a projected input")),
            Err(error) => Err(acp::Error::internal_error().data(format!("parent-message projection: {error}"))),
        }
    }

    pub(super) async fn drain_parent_messages_at_safe_point(&self) -> bool {
        let model_id = self.current_model_id().await;
        let user_chunk_meta = serde_json::json!({ "modelId": model_id })
            .as_object()
            .cloned();
        let (binding, prompt_id, admission, trigger, messages) = {
            let mut state = self.state.lock().await;
            // Reborrow once so `running_task` and `message_delivery` can be borrowed disjointly.
            let state = &mut *state;
            let Some(task) = state.running_task.as_ref() else {
                return false;
            };
            let (messages, trigger) = state.message_delivery.begin_delivery(task);
            if messages.is_empty() {
                return false;
            }
            let admission = xai_chat_state::InputAdmission {
                parent_task: xai_chat_state::ParentTask::Instance { id: task.identity.as_ref().clone() },
                cancellation: task.admission.clone(),
            };
            (turn_binding(task), task.prompt_id.clone(), admission, trigger, messages)
        };
        let (persisted_tx, persisted_rx) = oneshot::channel();
        if self.notifications.persistence_tx.send(PersistenceMsg::FlushAndAck { respond_to: persisted_tx }).is_err()
            || !matches!(persisted_rx.await, Ok(Ok(())))
        {
            return false;
        }
        let mut state = self.state.lock().await;
        if !state.running_task.as_ref().is_some_and(|task| {
            task.prompt_id == prompt_id
                && matches!(&admission.parent_task, xai_chat_state::ParentTask::Instance { id } if id == task.identity.as_ref())
        }) {
            return false;
        }
        for message in super::parent_interject::order_for_delivery(&messages) {
            if !state.message_delivery.has_identity(message.identity()) { continue; }
            if let Err(error) = self.commit_parent_message_projection(
                message.identity(), message.source().source, &prompt_id,
                message.content().text.to_string(), admission.clone(),
            ).await {
                tracing::error!(%error, "parent-message retained after durable projection failure");
                return false;
            }
        }
        let prompt_id = state.running_prompt_id().map(str::to_owned);
        let committed = state
            .message_delivery
            .finish_delivery(&binding, |messages| {
                let ordered: Vec<&_> =
                    super::parent_interject::order_for_delivery(messages).collect();
                let mut notifications = Vec::with_capacity(ordered.len());
                for message in &ordered {
                    let mut source_meta = user_chunk_meta.clone().unwrap_or_default();
                    source_meta.insert(crate::session::source_binding::STRICT_ORIGIN_META.to_owned(), serde_json::json!(crate::session::source_binding::StrictInputRef::Delivery {
                        session_id: self.session_id_string(), delivery_id: crate::session::delivery_receiver::parent_message_delivery_id(message.identity()),
                    }));
                    let update = acp::SessionUpdate::UserMessageChunk(
                        acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
                            message.content().text.to_string(),
                        )))
                        .meta(Some(source_meta)),
                    );
                    let mut meta = self.build_notification_meta();
                    if let Some(prompt_id) = &prompt_id {
                        meta["promptId"] = serde_json::json!(prompt_id);
                    }
                    let notification =
                        acp::SessionNotification::new(self.session_info.id.clone(), update)
                            .meta(meta.as_object().cloned());
                    self.notifications
                        .persistence_tx
                        .send(PersistenceMsg::Update(SessionUpdate::Acp(Box::new(
                            notification.clone(),
                        ))))
                        .map_err(|_| DrainCommitError::PersistenceUnavailable)?;
                    notifications.push(notification);
                }
                // The sender has no local echo in the child's view.
                if self
                    .notifications
                    .gateway_enabled
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    for notification in notifications {
                        self.notifications
                            .gateway
                            .forward_fire_and_forget(notification);
                    }
                }
                let delivered_at = std::time::Instant::now();
                for message in messages {
                    message
                        .content()
                        .telemetry
                        .record_safe_point_delivery(delivered_at, trigger);
                }
                Ok(())
            });
        match committed {
            Ok(delivered) => !delivered.is_empty(),
            Err(DrainCommitError::PersistenceUnavailable) => {
                tracing::error!(
                    session_id = %self.session_info.id.0,
                    "parent-message drain skipped: persistence actor unavailable"
                );
                false
            }
        }
    }

    pub(super) fn transition_parent_messages(
        &self,
        state: &mut State,
        target: TerminalTarget<'_, String, TurnEpoch>,
        cause: TerminalCause,
    ) -> (Vec<ParentOwnedDelivery>, bool) {
        let running = state.running_task.as_ref();
        let transition = state.message_delivery.transition(target, cause, running);
        let has_fallbacks = !transition.fallbacks.is_empty();
        for owned in transition.fallbacks {
            let fallback_reason = match cause {
                TerminalCause::Completion => {
                    crate::session::telemetry::ActiveAgentMessageFallbackReason::Completion
                }
                TerminalCause::SoftCancel => {
                    crate::session::telemetry::ActiveAgentMessageFallbackReason::SoftCancel
                }
                TerminalCause::Rewind => {
                    crate::session::telemetry::ActiveAgentMessageFallbackReason::Rewind
                }
                TerminalCause::HardTeardown | TerminalCause::ActorDrop => {
                    unreachable!("terminal cause cannot produce a fallback")
                }
            };
            let (_, message, completion) = owned.into_parts();
            let (_, origin, content) = message.into_parts();
            content.telemetry.record_fallback(fallback_reason);
            state
                .pending_inputs
                .push_back(content.into_input(origin, completion));
        }
        (transition.completions, has_fallbacks)
    }

    pub(super) async fn stop_parent_messages_on_actor_exit(&self) {
        let outcome = self.cancel_running_task_with_parent_teardown(crate::session::CancelOptions {
            cancel_subagents: false,
            kill_background_tasks: false,
            history: crate::session::CancelHistoryDisposition::Keep,
            trigger: Some(crate::session::CancelTrigger::Shutdown),
            user_initiated: false,
        }, ParentTeardownRequest::ActorDrop).await;
        if !outcome.settled {
            tracing::error!("actor exit has unresolved durable terminal settlement");
        }
    }

    pub(super) async fn settle_all_parent_messages(&self, cause: TerminalCause) -> Result<(), acp::Error> {
        self.settle_parent_messages_for_exit(cause, ParentRunningTeardown::Stop).await
    }

    pub(super) async fn settle_parent_messages_for_exit(&self, cause: TerminalCause, running: ParentRunningTeardown) -> Result<(), acp::Error> {
        debug_assert!(matches!(
            cause,
            TerminalCause::HardTeardown | TerminalCause::ActorDrop
        ));
        let mut state = self.state.lock().await;
        if state.message_delivery.teardown == ParentTeardownStatus::Unsettled {
            return Err(parent_teardown_uncertain());
        }
        let running_front = state.running_task.as_ref().and_then(|task| state.pending_inputs.front()
            .filter(|item| item.prompt_id == task.prompt_id).map(|item| item.prompt_id.clone()));
        let mut message_ids: Vec<String> = state.message_delivery.lifecycle.identities().cloned().collect();
        for (index, item) in state.pending_inputs.iter().enumerate() {
            if index == 0 && running_front.is_some() && matches!(running, ParentRunningTeardown::Preserve) {
                continue;
            }
            if let PromptOrigin::ParentAgentMessage { message_id, .. }
                | PromptOrigin::ParentHumanMessage { message_id, .. } = item.input_origin.as_prompt_origin()
            {
                message_ids.push(message_id.clone());
            }
        }
        if !message_ids.is_empty() {
            state.message_delivery.teardown = ParentTeardownStatus::Unsettled;
            let (respond_to, receive) = oneshot::channel();
            // Retain local ownership when durable settlement is uncertain.
            self.notifications.persistence_tx.send(PersistenceMsg::CancelParentMessages {
                message_ids: message_ids.clone(), respond_to,
            }).map_err(|_| acp::Error::internal_error().data("parent teardown settlement owner closed"))?;
            receive.await
                .map_err(|_| acp::Error::internal_error().data("parent teardown settlement acknowledgement lost"))?
                .map_err(|error| acp::Error::internal_error().data(format!("parent teardown settlement: {error}")))?;
        }
        state.message_delivery.teardown = match running {
            ParentRunningTeardown::Stop => ParentTeardownStatus::Settled,
            ParentRunningTeardown::Preserve => ParentTeardownStatus::Open,
        };
        let (completions, _) =
            self.transition_parent_messages(&mut state, TerminalTarget::All, cause);
        let result = Err(acp::Error::internal_error()
            .data("active-message owner ended before delivery completed"));
        let mut retained = std::collections::VecDeque::new();
        while let Some(item) = state.pending_inputs.pop_front() {
            if retained.is_empty() && running_front.as_ref() == Some(&item.prompt_id) {
                // Only the exact task-finalization owner removes the running front.
                retained.push_back(item);
                continue;
            }
            if matches!(item.input_origin.as_prompt_origin(),
                PromptOrigin::ParentAgentMessage { message_id, .. }
                | PromptOrigin::ParentHumanMessage { message_id, .. } if message_ids.contains(message_id)) {
                let _ = item.respond_to.send(result.clone());
            } else {
                retained.push_back(item);
            }
        }
        state.pending_inputs = retained;
        Self::settle_parent_message_completions(completions, &result);
        Ok(())
    }

    pub(super) fn settle_parent_message_completions(
        completions: Vec<ParentOwnedDelivery>,
        result: &crate::session::commands::PromptTurnResult,
    ) {
        completions.into_iter().for_each(|owned| {
            let (_, _, completion) = owned.into_parts();
            let _ = completion.send(result.clone());
        });
    }

    #[cfg(test)]
    async fn admit_parent_agent_message_for_test(
        self: &Arc<Self>,
        message: ActiveAgentMessage,
        source: ActiveAgentMessageSource,
        operation: ActiveAgentMessageOperation,
        receipt_sink: mpsc::Sender<crate::agent::subagent::PromptTurnReceipt>,
        respond_to: oneshot::Sender<ActiveMessageAdmission>,
        completion_tx: mpsc::UnboundedSender<super::turn_task::TurnCompletionMsg>,
    ) {
        self.admit_parent_agent_message_inner(
            None,
            source,
            message,
            operation,
            receipt_sink,
            xai_grok_telemetry::TelemetryCtx::new(
                "test-parent".to_owned(),
                std::sync::Arc::new(tokio::sync::Mutex::new(0)),
            ),
            respond_to,
            completion_tx,
        )
        .await;
    }
}

#[cfg(test)]
#[path = "parent_message_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "parent_message_interject_tests.rs"]
mod interject_tests;

#[cfg(test)]
#[path = "parent_message_durability_tests.rs"]
mod durability_tests;
