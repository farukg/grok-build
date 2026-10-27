use super::*;
use crate::session::delivery_receiver::{
    DeliveryExecution, DeliveryProgress, DeliveryReceipt, DeliveryRequest, DeliveryResult,
};

impl SessionActor {
    pub(super) fn durable_delivery_payload(
        blocks: &[acp::ContentBlock],
    ) -> Result<String, acp::Error> {
        match blocks {
            [acp::ContentBlock::Text(text)] => Ok(text.text.clone()),
            _ => Err(acp::Error::internal_error()
                .data("receiver payload must remain one exact text block")),
        }
    }

    pub(super) fn delivery_changed(&self, receipt: &DeliveryReceipt) {
        if let Ok(params) = serde_json::value::to_raw_value(receipt) {
            self.notifications
                .gateway
                .forward_fire_and_forget(acp::ExtNotification::new(
                    "x.ai/session/delivery_changed",
                    params.into(),
                ));
        }
    }

    pub(super) async fn receive_durable_delivery(
        &self,
        request: DeliveryRequest,
    ) -> DeliveryResult {
        let (respond_to, receive) = oneshot::channel();
        if self
            .notifications
            .persistence_tx
            .send(PersistenceMsg::ReceiveDelivery {
                request: request.clone(),
                respond_to,
            })
            .is_err()
        {
            return DeliveryResult::Indeterminate {
                reason: "receiver persistence owner closed".into(),
            };
        }
        let result = match receive.await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                return DeliveryResult::Indeterminate {
                    reason: error.to_string(),
                };
            }
            Err(_) => return DeliveryResult::Indeterminate {
                reason:
                    "receiver admission acknowledgement lost; observe the same delivery identity"
                        .into(),
            },
        };
        if let DeliveryResult::Admitted { receipt } = &result {
            if matches!(
                request.action,
                crate::session::delivery_receiver::DeliveryAction::Admit
            ) && receipt.progress == DeliveryProgress::Queued
            {
                self.resume_durable_deliveries(
                    crate::session::delivery_receiver::ReceiverResume::CurrentQueue,
                )
                .await;
            }
            if matches!(
                request.action,
                crate::session::delivery_receiver::DeliveryAction::Admit
            ) {
                self.delivery_changed(receipt);
            }
        }
        result
    }

    pub(super) async fn resume_durable_deliveries(
        &self,
        mode: crate::session::delivery_receiver::ReceiverResume,
    ) {
        let (respond_to, receive) = oneshot::channel();
        let command = match &mode {
            crate::session::delivery_receiver::ReceiverResume::Recovery { .. } => {
                PersistenceMsg::RecoverDeliveries { respond_to }
            }
            crate::session::delivery_receiver::ReceiverResume::CurrentQueue => {
                PersistenceMsg::GetDeliveryQueue { respond_to }
            }
        };
        if self.notifications.persistence_tx.send(command).is_err() {
            return;
        }
        let summary = match receive.await {
            Ok(Ok(summary)) => summary,
            Ok(Err(error)) => {
                tracing::error!(%error, "durable receiver recovery failed");
                return;
            }
            Err(_) => {
                tracing::error!("durable receiver recovery acknowledgement lost");
                return;
            }
        };
        match mode {
            crate::session::delivery_receiver::ReceiverResume::Recovery { deliveries } => {
                for id in deliveries {
                    if let Some(record) = summary.completion_delivery.receiver.records.get(&id) {
                        self.delivery_changed(&record.receipt(
                            &self.session_id_string(),
                            &id,
                            crate::session::receiver_execution::ReceiverPresence::Live,
                        ));
                    }
                }
            }
            crate::session::delivery_receiver::ReceiverResume::CurrentQueue => {}
        }
        for (delivery_id, record) in &summary.completion_delivery.receiver.records {
            let DeliveryExecution::Queued { payload, prompt_id } = &record.execution else {
                continue;
            };
            let origin = match &record.provenance {
                crate::session::delivery_receiver::DeliveryProvenance::External => PromptOrigin::Delivery {
                    delivery_id: delivery_id.clone(), payload_digest: record.digest.clone(),
                },
                crate::session::delivery_receiver::DeliveryProvenance::ParentMessage { admission } => match admission.source {
                    crate::session::delivery_receiver::ParentMessageSource::Agent => PromptOrigin::ParentAgentMessage { message_id: admission.message_id.clone(), sender_session_id: admission.sender_session_id.clone() },
                    crate::session::delivery_receiver::ParentMessageSource::Human => PromptOrigin::ParentHumanMessage { message_id: admission.message_id.clone(), sender_session_id: admission.sender_session_id.clone() },
                },
            };
            let exists = self.state.lock().await.pending_inputs.iter().any(|item| item.prompt_id == *prompt_id);
            if exists {
                continue;
            }
            if matches!(&origin, PromptOrigin::ParentAgentMessage { .. } | PromptOrigin::ParentHumanMessage { .. }) {
                self.state.lock().await.message_delivery.teardown = super::parent_message::ParentTeardownStatus::Open;
            }
            let (respond_to, receive) = oneshot::channel();
            drop(receive);
            let mut request = QueueInputRequest::from_input_origin(
                vec![acp::ContentBlock::Text(acp::TextContent::new(
                    payload.clone(),
                ))],
                prompt_id.clone(),
                InputOrigin::new(origin),
                PromptMode::Agent,
                respond_to,
            );
            request.verbatim = true;
            let _ = self.queue_input(request).await;
        }
    }

    pub(super) async fn project_durable_delivery(
        &self,
        delivery_id: &str,
        digest: &str,
        turn_id: &str,
        payload: String,
    ) -> Result<bool, acp::Error> {
        let admission = self
            .current_turn_admission(turn_id)
            .await
            .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
        let projection = xai_chat_state::DeliveryProjection {
            item: ConversationItem::user(payload),
            binding: xai_chat_state::DeliveryBinding {
                delivery_id: delivery_id.to_owned(),
                payload_digest: digest.to_owned(),
                turn_id: turn_id.to_owned(),
                parent_task: admission.parent_task.clone(),
            },
        };
        let projection = xai_chat_state::InputProjection::Delivery(projection);
        let identity = crate::session::source_binding::StrictSourceOrigin::from_projection(self.session_id_string(), &projection);
        let result = self.chat_state_handle.append_input_with_source_and_ack(projection, admission).await;
        self.observe_strict_source(identity, &result);
        match result.map(|ack| ack.acknowledgement).map_err(|error| error.map_acknowledgement(|ack| ack.acknowledgement)) {
            Ok(xai_chat_state::CompletionProjectionAck::Appended) => {
                self.delivery_changed(&DeliveryReceipt {
                    session_id: self.session_id_string(),
                    delivery_id: delivery_id.to_owned(),
                    payload_digest: digest.to_owned(),
                    revision: 2,
                    progress: DeliveryProgress::Processing {
                        turn_id: turn_id.to_owned(),
                    },
                });
                Ok(true)
            }
            Ok(
                xai_chat_state::CompletionProjectionAck::AlreadyPresent { .. }
                | xai_chat_state::CompletionProjectionAck::TurnClosed,
            ) => Ok(false),
            Ok(xai_chat_state::CompletionProjectionAck::Continue { .. }) => {
                Err(acp::Error::internal_error()
                    .data("receiver execution cannot resume an already projected input"))
            }
            Err(error) => {
                Err(acp::Error::internal_error()
                    .data(format!("receiver input persistence: {error}")))
            }
        }
    }

    pub(super) async fn drain_durable_deliveries_at_safe_point(&self) -> bool {
        if self
            .tool_context
            .task_wake_suppressed
            .as_ref()
            .is_some_and(|gate| gate.get())
        {
            return false;
        }
        let mut changed = false;
        loop {
            let candidate = {
                let state = self.state.lock().await;
                let Some(running_id) = state.running_prompt_id() else {
                    return changed;
                };
                state.pending_inputs.iter().find_map(|item| {
                    match item.input_origin.as_prompt_origin() {
                        PromptOrigin::Delivery {
                            delivery_id,
                            payload_digest,
                        } if item.prompt_id != running_id => Some((
                            running_id.to_owned(),
                            item.prompt_id.clone(),
                            delivery_id.clone(),
                            payload_digest.clone(),
                            Self::durable_delivery_payload(&item.prompt_blocks),
                        )),
                        _ => None,
                    }
                })
            };
            let Some((turn_id, queued_id, delivery_id, digest, payload)) = candidate else {
                return changed;
            };
            let payload = match payload {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::error!(%error, "receiver queue payload invalid");
                    return changed;
                }
            };
            match self
                .project_durable_delivery(&delivery_id, &digest, &turn_id, payload)
                .await
            {
                Ok(projected) => changed |= projected,
                Err(error) => {
                    tracing::error!(%error, "receiver safe-point projection failed");
                    return changed;
                }
            }
            let mut state = self.state.lock().await;
            if state.running_prompt_id() != Some(turn_id.as_str()) {
                return changed;
            }
            let removed = state.sweep_pending_inputs(|item| item.prompt_id == queued_id);
            drop(state);
            for item in removed {
                let _ = item.respond_to.send(ok_end_turn(0, None));
            }
        }
    }

    pub(super) async fn settle_durable_delivery_turn(
        &self,
        turn_id: &str,
        parent_task: xai_chat_state::ParentTask,
        origin: crate::session::delivery_receiver::ReceiverTurnOrigin,
        progress: DeliveryProgress,
    ) -> Result<(), acp::Error> {
        let (respond_to, receive) = oneshot::channel();
        self
            .notifications
            .persistence_tx
            .send(PersistenceMsg::FinishDeliveryTurn {
                turn_id: turn_id.to_owned(),
                parent_task,
                origin,
                progress,
                respond_to,
            }).map_err(|_| acp::Error::internal_error().data("terminal settlement owner closed"))?;
        receive.await.map_err(|_| acp::Error::internal_error().data("terminal settlement acknowledgement lost"))?
            .map_err(|error| crate::session::persistence::io_error_to_acp(&error))?;
        Ok(())
    }
}
