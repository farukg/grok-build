use super::super::tests::*;
use super::super::*;
use crate::implementations::grok_build::task::backend::{ChannelBackend, SubagentBackend};
use crate::implementations::grok_build::task::types::SubagentAccessError;

#[tokio::test]
async fn revocation_captures_committing_attempt_without_claiming_completion() {
    let mut harness = harness_with_options(
        RunnerBehavior {
            producer_commit: ProducerCommitBehavior::HoldAcknowledgement,
            ..Default::default()
        },
        CoordinatorConfig::default(),
    );
    let backend = parent_backend(&harness);
    let spawn = tokio::spawn({
        let backend = backend.clone();
        async move { backend.spawn(request("committing", false), None, None).await }
    });
    harness.requests.recv().await.expect("runner request");
    harness.started.recv().await.expect("started");
    let (subagent_id, attempt_id, ..) = harness.wake_runs.recv().await.expect("original runner attempt");
    let _ = harness.finish.send(());
    let commit = harness.terminal_publications.recv().await.expect("held producer commit");
    let revocation = backend.cancel_parent_session().await.expect("revocation ACK");
    assert_eq!(revocation.parent_session_id, "parent");
    assert_eq!(revocation.completions, vec![xai_message_delivery_core::SubagentCompletionId {
        subagent_id, attempt_id,
    }]);
    assert!(!spawn.is_finished(), "revocation is not producer commit");
    assert!(harness.completions.try_recv().is_err());
    commit();
    spawn.await.expect("spawn join").expect("real producer completion");
    harness.actor.abort();
}

#[tokio::test]
async fn lost_revocation_ack_is_not_empty_success() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let backend = ChannelBackend::for_session(sender, "parent");
    let waiting = tokio::spawn(async move { backend.cancel_parent_session().await });
    let command = receiver.recv().await.expect("revocation command");
    assert!(matches!(&command, crate::implementations::grok_build::task::types::SubagentEvent::RevokeParentSession { parent_session_id, .. } if parent_session_id == "parent"));
    drop(command);
    assert!(matches!(waiting.await.expect("request join"),
        Err(SubagentAccessError::CoordinatorUnavailable)));
}

#[tokio::test]
async fn revocation_transport_loss_is_not_empty_success() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    drop(receiver);
    let backend = ChannelBackend::for_session(sender, "parent");
    assert!(matches!(backend.cancel_parent_session().await,
        Err(SubagentAccessError::CoordinatorUnavailable)));
}
