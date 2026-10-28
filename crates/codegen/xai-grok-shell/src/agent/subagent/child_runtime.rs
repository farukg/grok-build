use tokio::sync::mpsc;
use xai_grok_tools::implementations::grok_build::task::coordinator::{
    ActiveMessageAdmission, ChildCancelOutcome, ChildCancellation, ChildControl, LocalBoxFuture,
    SendBoxFuture, SubagentProgress,
};
use xai_grok_tools::implementations::grok_build::task::types::ActiveAgentMessageDelivery;
use xai_message_delivery_core::DeliveryEnvelope;

use super::prompt_turn_receipt::{PromptTurnReceipt, cancel_shell_child_turn};
use crate::session::{SessionCommand, SessionThread};

/// Shell runtime handle retained while a child is active.
pub(crate) struct ShellChildRuntime {
    pub(crate) initial_parent_reservation:
        Option<std::sync::Arc<crate::session::persistence::ReservedParentMessageTurn>>,
    pub(crate) child_cmd_tx: mpsc::UnboundedSender<SessionCommand>,
    pub(crate) message_delivery: crate::session::message_delivery::MessageDeliveryHandle,
    pub(crate) active_message_target_session_id: String,
    pub(crate) active_message_target_agent_id: xai_message_delivery_core::AgentId,
    pub(crate) active_message_target_generation:
        xai_grok_tools::implementations::grok_build::task::root_control::AgentMessageGeneration,
    pub(crate) child_signals: crate::session::signals::SessionSignalsHandle,
    /// Held by the worker until promotion succeeds.
    /// `None` means the caller still owns the join handle, so a cancel during promotion can wait for the actor to exit.
    pub(crate) _child_thread: Option<SessionThread>,
    pub(crate) receipt_sink: mpsc::Sender<PromptTurnReceipt>,
    pub(crate) caller: tokio::sync::watch::Sender<
        xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCaller,
    >,
    #[cfg(test)]
    pub(crate) force_queue_envelope: bool,
    pub(crate) active_message_parent_session_id: String,
    pub(crate) active_message_parent_prompt_index: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl ChildControl for ShellChildRuntime {
    type ProgressFuture = LocalBoxFuture<SubagentProgress>;

    fn has_initial_message_reservation(&self, delivery: &ActiveAgentMessageDelivery) -> bool {
        self.initial_parent_reservation
            .as_ref()
            .is_some_and(|reservation| {
                reservation.matches_delivery(&self.active_message_target_session_id, delivery)
                    && delivery.receiver().is_some_and(|receiver| {
                        receiver.generation == self.active_message_target_generation
                    })
            })
    }

    fn initial_message_terminal_ownership(&self) -> xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership{
        use xai_grok_tools::implementations::grok_build::task::coordinator::InitialMessageTerminalOwnership;
        match &self.initial_parent_reservation {
            Some(reservation) => reservation.publication_contract().terminal_ownership(),
            None => InitialMessageTerminalOwnership::Message,
        }
    }

    fn progress(&self) -> Self::ProgressFuture {
        let signals = self.child_signals.clone();
        Box::pin(async move {
            signals
                .snapshot()
                .await
                .map(|snapshot| SubagentProgress {
                    turn_count: snapshot.turn_count,
                    tool_call_count: snapshot.tool_call_count,
                    tokens_used: snapshot.context_tokens_used,
                    context_window_tokens: snapshot.context_window_tokens,
                    context_usage_pct: snapshot.context_window_usage,
                    tools_used: snapshot.tools_used,
                    error_count: snapshot.error_count,
                })
                .unwrap_or_default()
        })
    }

    fn observe_caller(
        &self,
        caller: xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCaller,
    ) {
        self.caller.send_if_modified(|current| {
            if *current == caller {
                return false;
            }
            *current = caller;
            true
        });
    }

    fn send_active_message(
        &self,
        delivery: ActiveAgentMessageDelivery,
    ) -> SendBoxFuture<ActiveMessageAdmission> {
        let message_delivery = self.message_delivery.clone();
        let receipt_sink = self.receipt_sink.clone();
        let message = delivery.message();
        let envelope = DeliveryEnvelope::from_agent(
            {
                #[cfg(test)]
                if self.force_queue_envelope {
                    xai_message_delivery_core::Operation::Queue
                } else {
                    xai_message_delivery_core::Operation::from(delivery.operation())
                }
                #[cfg(not(test))]
                xai_message_delivery_core::Operation::from(delivery.operation())
            },
            message.text.clone(),
            crate::session::message_delivery::agent_delivery_identity(message.message_id.clone()),
            crate::session::message_delivery::CoordinatorAgentDeliveryGrant::new(
                self.active_message_target_session_id.clone(),
                self.active_message_target_agent_id.clone(),
                self.active_message_target_generation,
                delivery,
            ),
        );
        let parent_telemetry_ctx = parent_telemetry_ctx(
            &self.active_message_parent_session_id,
            &self.active_message_parent_prompt_index,
        );
        Box::pin(async move {
            message_delivery
                .send(envelope, receipt_sink, parent_telemetry_ctx)
                .await
        })
    }

    fn cancel(&self, intent: ChildCancellation) -> ChildCancelOutcome {
        match intent {
            ChildCancellation::Authorized => {
                if let Some(reservation) = &self.initial_parent_reservation {
                    match reservation.authorize_cancel() {
                        ChildCancelOutcome::Accepted => {}
                        ChildCancelOutcome::AttemptEnded => {
                            return ChildCancelOutcome::AttemptEnded;
                        }
                    }
                }
            }
            ChildCancellation::Shutdown => {}
        }
        cancel_shell_child_turn(&self.child_cmd_tx, intent);
        ChildCancelOutcome::Accepted
    }
}

/// Telemetry context of the parent turn a message into the child is attributed to.
pub(crate) fn parent_telemetry_ctx(
    parent_session_id: &str,
    parent_prompt_index: &std::sync::atomic::AtomicUsize,
) -> xai_grok_telemetry::TelemetryCtx {
    xai_grok_telemetry::TelemetryCtx::new(
        parent_session_id.to_owned(),
        std::sync::Arc::new(tokio::sync::Mutex::new(
            parent_prompt_index.load(std::sync::atomic::Ordering::Acquire),
        )),
    )
}

const SESSION_THREAD_EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// How long to wait for an unpromoted child actor to exit after cancel and shutdown.
pub(crate) const UNPROMOTED_SESSION_THREAD_EXIT_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

/// Polls `SessionThread::is_finished` until the actor thread exits or `timeout` elapses.
/// A `timeout` of `Duration::ZERO` checks once and never sleeps.
pub(crate) async fn await_session_thread_exit(
    thread: &SessionThread,
    timeout: std::time::Duration,
) -> bool {
    if thread.is_finished() {
        return true;
    }
    if timeout.is_zero() {
        return false;
    }
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return thread.is_finished();
        }
        tokio::time::sleep(remaining.min(SESSION_THREAD_EXIT_POLL)).await;
        if thread.is_finished() {
            return true;
        }
    }
}

/// Whether an unpromoted child may drop its worktree and workspace binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnpromotedResourceFate {
    Release,
    Preserve,
}

impl UnpromotedResourceFate {
    pub(crate) fn from_thread_exit(exited: bool) -> Self {
        if exited {
            Self::Release
        } else {
            Self::Preserve
        }
    }

    pub(crate) fn should_release(self) -> bool {
        matches!(self, Self::Release)
    }
}

#[cfg(test)]
#[path = "child_runtime_tests.rs"]
mod tests;
