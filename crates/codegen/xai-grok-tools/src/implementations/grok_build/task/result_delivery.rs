use std::task::{Context, Poll};
use tokio::sync::oneshot;
use xai_message_delivery_core::AttemptId;
use super::types::SubagentResult;
use crate::types::completion_observation::SubagentCompletionObservations;

pub struct SubagentResultSender {
    sender: oneshot::Sender<SubagentResult>,
    observations: Option<SubagentCompletionObservations>,
}

impl SubagentResultSender {
    pub fn new(sender: oneshot::Sender<SubagentResult>, observations: Option<SubagentCompletionObservations>) -> Self {
        Self { sender, observations }
    }
    pub fn is_closed(&self) -> bool { self.sender.is_closed() }
    pub fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<()> { self.sender.poll_closed(cx) }
    pub async fn closed(&mut self) { self.sender.closed().await; }
    pub fn send(self, result: SubagentResult) -> Result<(), SubagentResult> { self.sender.send(result) }
    pub fn send_committed(self, result: SubagentResult, attempt: &AttemptId) -> Result<(), SubagentResult> {
        if !self.sender.is_closed() && let Some(observer) = &self.observations {
            observer.record(&result, attempt);
        }
        self.sender.send(result)
    }
}
