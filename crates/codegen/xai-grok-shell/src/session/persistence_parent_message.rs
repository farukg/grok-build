use super::SessionPersistence;
use crate::session::{delivery_receiver, receiver_execution};
use std::io;
use xai_grok_tools::implementations::grok_build::task::types::{ActiveAgentMessageDelivery, ActiveAgentMessageSource};

#[derive(Debug)]
pub struct ParentMessageTurnReservation {
    pub(crate) delivery: ActiveAgentMessageDelivery,
    pub(crate) prompt_id: String,
    pub(crate) payload_digest: String,
    pub(crate) turn_id: String,
    pub(crate) admission: xai_chat_state::InputAdmission,
}

pub struct ReservedParentMessageTurn {
    session_id: String,
    delivery: ActiveAgentMessageDelivery,
    binding: xai_chat_state::DeliveryBinding,
    cancellation: tokio_util::sync::CancellationToken,
    publication: receiver_execution::ParentMessagePublicationContract,
    _receiver_lease: receiver_execution::ReceiverExecutionLease,
}

impl std::fmt::Debug for ReservedParentMessageTurn {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ReservedParentMessageTurn").finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum ParentMessagePublicationState {
    Pending {
        release: tokio::sync::oneshot::Sender<()>,
        reservation: std::sync::Arc<ReservedParentMessageTurn>,
    },
    Released,
}

#[derive(Debug)]
pub struct ParentMessagePublicationGate {
    state: ParentMessagePublicationState,
    persistence_tx: tokio::sync::mpsc::UnboundedSender<super::PersistenceMsg>,
}

impl ParentMessagePublicationGate {
    pub(crate) fn publish(mut self) -> Result<(), ()> {
        match std::mem::replace(&mut self.state, ParentMessagePublicationState::Released) {
            ParentMessagePublicationState::Pending { release, reservation } => {
                if reservation.cancellation.is_cancelled() || reservation.publication.publish(release).is_err() {
                    reservation.finish_unpublished(&self.persistence_tx);
                    Err(())
                } else {
                    Ok(())
                }
            }
            ParentMessagePublicationState::Released => Err(()),
        }
    }
}

impl Drop for ParentMessagePublicationGate {
    fn drop(&mut self) {
        match &self.state {
            ParentMessagePublicationState::Pending { reservation, .. } => reservation.finish_unpublished(&self.persistence_tx),
            ParentMessagePublicationState::Released => {}
        }
    }
}

impl ReservedParentMessageTurn {
    pub(crate) fn publication_contract(&self) -> receiver_execution::ParentMessagePublicationContract {
        self.publication.clone()
    }

    pub(crate) fn authorize_cancel(&self) -> xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome {
        use xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome;
        match &self.publication {
            receiver_execution::ParentMessagePublicationContract::Admission => ChildCancelOutcome::Accepted,
            receiver_execution::ParentMessagePublicationContract::Adoption(adoption) => adoption.authorize_cancel(),
        }
    }

    pub(crate) fn authorize_turn_cancel(&self, turn_id: &str, parent_task: &xai_chat_state::ParentTask) -> xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome {
        use xai_grok_tools::implementations::grok_build::task::coordinator::ChildCancelOutcome;
        if self.binding.turn_id != turn_id || &self.binding.parent_task != parent_task {
            return ChildCancelOutcome::AttemptEnded;
        }
        self.authorize_cancel()
    }

    pub(crate) fn publication(
        self,
        release: tokio::sync::oneshot::Sender<()>,
        persistence_tx: tokio::sync::mpsc::UnboundedSender<super::PersistenceMsg>,
    ) -> crate::session::message_delivery::InitialParentMessageReady {
        let reservation = std::sync::Arc::new(self);
        let gate = ParentMessagePublicationGate {
            state: ParentMessagePublicationState::Pending { release, reservation: reservation.clone() },
            persistence_tx,
        };
        crate::session::message_delivery::InitialParentMessageReady { reservation, release: gate }
    }

    fn finish_unpublished(&self, persistence_tx: &tokio::sync::mpsc::UnboundedSender<super::PersistenceMsg>) {
        match &self.publication {
            receiver_execution::ParentMessagePublicationContract::Adoption(adoption) => {
                adoption.abandon();
                self.cancellation.cancel();
                return;
            }
            receiver_execution::ParentMessagePublicationContract::Admission => {}
        }
        self.cancellation.cancel();
        let (respond_to, receive) = tokio::sync::oneshot::channel();
        drop(receive);
        if persistence_tx.send(super::PersistenceMsg::FinishDeliveryTurn {
            turn_id: self.binding.turn_id.clone(), parent_task: self.binding.parent_task.clone(),
            origin: delivery_receiver::ReceiverTurnOrigin::ParentMessage { message_id: self.delivery.message().message_id.clone() },
            progress: delivery_receiver::DeliveryProgress::Cancelled { reason: "initial parent-message publication was not released".into() },
            respond_to,
        }).is_err() {
            tracing::warn!("initial parent-message publication teardown owner closed");
        }
    }

    pub(crate) fn matches_turn(&self, turn_id: &str, admission: &xai_chat_state::InputAdmission) -> bool {
        self.binding.turn_id == turn_id
            && self.binding.parent_task == admission.parent_task
            && !self.cancellation.is_cancelled()
            && !admission.cancellation.is_cancelled()
    }

    pub(crate) fn matches_delivery(
        &self,
        session_id: &str,
        delivery: &ActiveAgentMessageDelivery,
    ) -> bool {
        self.session_id == session_id
            && self.delivery.message() == delivery.message()
            && self.delivery.source() == delivery.source()
            && self.delivery.operation() == delivery.operation()
            && self.delivery.route() == delivery.route()
            && self.delivery.receiver() == delivery.receiver()
            && self.delivery.admission_is_committed()
            && !self.cancellation.is_cancelled()
    }
}

impl SessionPersistence {
    pub(super) async fn reconfirm_parent_message_publication(
        &mut self,
        reservation: &ReservedParentMessageTurn,
        launch: &xai_grok_tools::implementations::grok_build::task::native_recovery::NativeWakeLaunch,
    ) -> io::Result<()> {
        use delivery_receiver::{DeliveryExecution, DeliveryProvenance, ParentMessageCustody, ParentMessagePublication};
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "published native start no longer matches its reserved custody");
        if !reservation.matches_delivery(self.info.id.0.as_ref(), &reservation.delivery) {
            return Err(invalid());
        }
        match self.receiver_owner.access() {
            receiver_execution::ReceiverAccess::Owner(lease) if lease.session_id == reservation.session_id => {}
            receiver_execution::ReceiverAccess::Owner(_) | receiver_execution::ReceiverAccess::Observe => return Err(invalid()),
        }
        self.receiver_owner.bind_parent_message(&reservation.delivery.message().message_id, &reservation.binding.turn_id, &reservation.binding.parent_task)?;
        self.flush_and_sync().await?;
        let summary = self.storage.load_summary(&self.info).await?;
        let record = summary.completion_delivery.receiver.records.get(&reservation.binding.delivery_id).ok_or_else(invalid)?;
        let DeliveryProvenance::ParentMessage { admission } = &record.provenance else { return Err(invalid()); };
        let ParentMessageCustody::Initial { custody, .. } = &admission.custody else { return Err(invalid()); };
        if &custody.launch != launch
            || reservation.delivery.receiver() != Some(&custody.receiver)
            || record.digest != reservation.binding.payload_digest
            || !matches!(&record.execution, DeliveryExecution::Queued { payload, prompt_id } if payload == reservation.delivery.message().text.as_ref() && prompt_id == &reservation.binding.turn_id)
            || !ParentMessagePublication::matches_started(&summary, launch)
            || reservation.cancellation.is_cancelled()
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) async fn reserve_parent_message_turn(
        &mut self,
        reservation: ParentMessageTurnReservation,
    ) -> io::Result<ReservedParentMessageTurn> {
        use delivery_receiver::{DeliveryExecution, DeliveryProvenance, ParentMessageOperation, ParentMessageSource};
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "initial parent-message reservation does not match durable admission");
        let ParentMessageTurnReservation { delivery, prompt_id, payload_digest, turn_id, admission } = reservation;
        let message = delivery.message();
        let session_id = self.info.id.0.as_ref();
        if admission.cancellation.is_cancelled()
            || !matches!(&admission.parent_task, xai_chat_state::ParentTask::Instance { id } if !id.is_empty())
            || !delivery.admission_is_committed()
            || delivery.receiver().is_none_or(|receiver| receiver.session_id != session_id)
            || prompt_id != delivery_receiver::parent_message_prompt_id(&message.message_id)
            || turn_id != prompt_id
            || payload_digest != delivery_receiver::payload_digest(&message.text)
        {
            return Err(invalid());
        }
        match self.receiver_owner.access() {
            receiver_execution::ReceiverAccess::Owner(lease) if lease.session_id == session_id => {}
            receiver_execution::ReceiverAccess::Owner(_)
            | receiver_execution::ReceiverAccess::Observe => return Err(invalid()),
        }
        self.flush_and_sync().await?;
        let summary = self.storage.load_summary(&self.info).await?;
        if summary.hidden == Some(true) {
            return Err(invalid());
        }
        let record = summary.completion_delivery.receiver.records
            .get(&delivery_receiver::parent_message_delivery_id(&message.message_id))
            .ok_or_else(invalid)?;
        let DeliveryProvenance::ParentMessage { admission: durable } = &record.provenance else {
            return Err(invalid());
        };
        let source = match delivery.source() {
            ActiveAgentMessageSource::Agent => ParentMessageSource::Agent,
            ActiveAgentMessageSource::Human => ParentMessageSource::Human,
        };
        if durable.message_id != message.message_id
            || durable.sender_session_id != message.sender_session_id
            || durable.source != source
            || durable.effective != ParentMessageOperation::Queue
            || durable.prompt_id != prompt_id
            || durable.text != message.text.as_ref()
            || record.digest != payload_digest
            || !matches!(&record.execution, DeliveryExecution::Queued { payload, prompt_id: queued } if payload == message.text.as_ref() && queued == &prompt_id)
            || admission.cancellation.is_cancelled()
        {
            return Err(invalid());
        }
        self.receiver_owner.bind_parent_message(&message.message_id, &turn_id, &admission.parent_task)?;
        let receiver_lease = match self.receiver_owner.access() {
            receiver_execution::ReceiverAccess::Owner(lease) => lease.retain(),
            receiver_execution::ReceiverAccess::Observe => return Err(invalid()),
        };
        let binding = xai_chat_state::DeliveryBinding {
            delivery_id: delivery_receiver::parent_message_delivery_id(&message.message_id),
            payload_digest, turn_id, parent_task: admission.parent_task,
        };
        Ok(ReservedParentMessageTurn { session_id: self.info.id.0.to_string(), delivery, binding, cancellation: admission.cancellation, publication: self.receiver_owner.publication_contract(), _receiver_lease: receiver_lease })
    }
}

#[cfg(test)]
pub(super) mod tests_support {
    pub(crate) fn cancellation(reservation: &super::ReservedParentMessageTurn) -> tokio_util::sync::CancellationToken {
        reservation.cancellation.clone()
    }
}
