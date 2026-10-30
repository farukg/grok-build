use super::active_message::{ParkedSpawnReadyMessage, SpawnReadyInput};
use super::queue::StartOrigin;
use super::*;
use crate::implementations::grok_build::task::active_message::ActiveMessageAdmissionLease;
use crate::implementations::grok_build::task::admission::AdmissionDecision;
use crate::implementations::grok_build::task::native_recovery::{
    MessageRecoveryAction, MessageRecoveryState, NativeWakeMode, PreparedMessageRecovery,
    SubagentMessageRecoveryRequest,
};
use crate::implementations::grok_build::task::types::ActiveAgentMessageDelivery;

impl<R: ChildRunner> SubagentCoordinator<R> {
    pub(super) fn recover_message(&mut self, command: SubagentMessageRecoveryRequest) {
        if let Err(outcome) = self.verify_message_sender(&command.sender_context) {
            let _ = command.respond_to.send(outcome);
            return;
        }
        use crate::implementations::grok_build::task::{
            native_recovery::MessageRecoveryAudience, types::ActiveMessageSenderContext,
        };
        let audience = match &command.sender_context {
            ActiveMessageSenderContext::RootSession { session_id }
            | ActiveMessageSenderContext::HumanRoot { session_id } => {
                MessageRecoveryAudience::parent(session_id)
            }
            ActiveMessageSenderContext::GrantedChild { holder } => {
                let Some(parent) = self.graph.root_session(holder.agent_id().as_str()) else {
                    let _ = command
                        .respond_to
                        .send(ActiveAgentMessageOutcome::NotFoundOrNotOwned);
                    return;
                };
                MessageRecoveryAudience::sender(parent, holder.session_id())
            }
        };
        let prepared = self.runner.prepare_message_recovery(&command, &audience);
        let (custody, message, source, publication) = match prepared {
            PreparedMessageRecovery::Refused => {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::NotFoundOrNotOwned);
                return;
            }
            PreparedMessageRecovery::Observed { reference, state } => {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::RecoveryObserved {
                        recovery_ref: reference,
                        state,
                    });
                return;
            }
            PreparedMessageRecovery::Unavailable { reference } => {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::AdmissionUncertain {
                        recovery_ref: reference,
                    });
                return;
            }
            PreparedMessageRecovery::Initial {
                custody,
                message,
                source,
                publication,
            } => (custody, message, source, publication),
        };
        let id = &command.reference.receiver.session_id;
        let busy = self.pending.contains_key(id)
            || self.active.contains_key(id)
            || self.queued.contains_id(id)
            || self.terminal_outputs.contains_key(id);
        match command.action {
            MessageRecoveryAction::Observe => {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::RecoveryObserved {
                        recovery_ref: command.reference,
                        state: MessageRecoveryState::Queued,
                    });
                return;
            }
            MessageRecoveryAction::Adopt
                if busy
                    || self
                        .spawn_blocked_sessions
                        .contains_key(&custody.launch.parent_session_id) =>
            {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::AdmissionUncertain {
                        recovery_ref: command.reference,
                    });
                return;
            }
            MessageRecoveryAction::Adopt => {}
        }
        let Some(attempt_id) =
            xai_message_delivery_core::AttemptId::parse(&custody.launch.attempt_id)
        else {
            let _ = command
                .respond_to
                .send(ActiveAgentMessageOutcome::NotFoundOrNotOwned);
            return;
        };
        let request = custody
            .launch
            .request(&command.reference, message.text.to_string());
        let running = self.session_running_count(&request.parent_session_id);
        match self.admission.admit(&request, running) {
            AdmissionDecision::Start => {}
            AdmissionDecision::Enqueue | AdmissionDecision::Reject(_) => {
                let _ = command
                    .respond_to
                    .send(ActiveAgentMessageOutcome::AdmissionUncertain {
                        recovery_ref: command.reference,
                    });
                return;
            }
        }
        let delivery = ActiveAgentMessageDelivery::new(
            message,
            custody.operation,
            source,
            custody.route,
            ActiveMessageAdmissionLease::for_recovery(publication.kind()),
        );
        delivery.bind_receiver(id, command.reference.receiver.generation);
        let Some(address) = AgentAddress::parse(&custody.launch.agent_address) else {
            let _ = command
                .respond_to
                .send(ActiveAgentMessageOutcome::NotFoundOrNotOwned);
            return;
        };
        let spawner = custody.launch.spawner_session_id.clone();
        let displaced = match &publication {
            super::super::native_recovery::RecoveryPublication::Published => {
                self.completed.remove(id);
                None
            }
            super::super::native_recovery::RecoveryPublication::Unpublished { previous_prompt } => {
                let previous = match self.completed.remove(id) {
                    Some(previous) => previous,
                    None => {
                        let age = self.next_completion_age;
                        let Some(next_age) = age.checked_add(1) else {
                            let _ = command.respond_to.send(
                                ActiveAgentMessageOutcome::AdmissionUncertain {
                                    recovery_ref: command.reference,
                                },
                            );
                            return;
                        };
                        let Some(previous) = custody.launch.previous_completion.restore(
                            id,
                            previous_prompt.clone(),
                            age,
                        ) else {
                            let _ = command.respond_to.send(
                                ActiveAgentMessageOutcome::AdmissionUncertain {
                                    recovery_ref: command.reference,
                                },
                            );
                            return;
                        };
                        self.next_completion_age = next_age;
                        previous
                    }
                };
                Some(DisplacedCompletedChild {
                    completed: Box::new(previous),
                })
            }
        };
        if !self.graph.contains(id) {
            self.graph.insert_root_child(id, &request.parent_session_id);
        }
        self.completed_order.retain(|entry| entry != id);
        self.spawn_ready.push(ParkedSpawnReadyMessage {
            subagent_id: id.clone(),
            sender_context: command.sender_context,
            input: SpawnReadyInput::Initial(delivery.clone()),
            respond_to: Some(command.respond_to),
            quota_admission: None,
            permit: command.permit,
        });
        self.start_child(
            request,
            None,
            None,
            StartOrigin::Direct,
            Some(address),
            spawner,
            Some(WakeOrigin {
                delivery,
                previous_completion: custody.launch.previous_completion.clone(),
                mode: NativeWakeMode::Adopt {
                    custody,
                    attempt_id,
                    publication,
                },
            }),
            displaced,
        );
    }
}
