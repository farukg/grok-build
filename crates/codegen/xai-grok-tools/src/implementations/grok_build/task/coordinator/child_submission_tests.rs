use super::*;
use super::active_message::tests::*;
use super::child_submission::{ChildSubmission, ChildSubmissionAuthority};
use crate::implementations::grok_build::task::backend::SubagentCoordinatorSender;
use crate::implementations::grok_build::task::types::*;

async fn reserve(coordinator: &mut TestCoordinator, parent: &str, child: &str) -> ChildSubmissionAuthority {
    let (respond_to, response) = oneshot::channel();
    coordinator.handle_command(SubagentEvent::ReserveChildSubmission {
        parent_session_id: parent.into(), child_session_id: child.into(), prompt: "new work".into(), respond_to,
    });
    response.await.unwrap().unwrap()
}

fn begin_submission(
    coordinator: &mut TestCoordinator,
    sender: &SubagentCoordinatorSender,
    address: &str,
    parent: &str,
    authority: ChildSubmissionAuthority,
) -> oneshot::Receiver<ActiveAgentMessageOutcome> {
    let (respond_to, response) = oneshot::channel();
    sender.try_send_active_message(SubagentActiveMessageRequest {
        request: ActiveAgentMessageRequest::try_new_from_human(address, "new work", ActiveAgentMessageOperation::Steer)
            .unwrap().with_child_submission(authority),
        sender_context: ActiveMessageSenderContext::HumanRoot { session_id: Arc::from(parent) },
        respond_to,
    }).unwrap();
    let ingress = coordinator.active_message_ingress.as_mut().unwrap().try_recv().unwrap();
    coordinator.handle_send_active_message(ingress);
    response
}

async fn quiesce(coordinator: &mut TestCoordinator) {
    let (respond_to, response) = oneshot::channel();
    coordinator.handle_command(SubagentEvent::QuiesceRestartChildren { parent_session_id: "parent".into(), respond_to });
    assert!(matches!(response.await.unwrap(), SubagentCancelOutcome::Cancelled { .. }));
}

#[tokio::test]
async fn fresh_submission_wakes_only_its_child_while_parent_remains_blocked() {
    let (mut coordinator, sender, admissions, _received) = fixture();
    let address = insert_child(&mut coordinator, admissions.clone(), "child", "parent");
    insert_child(&mut coordinator, admissions, "sibling", "parent");
    finish_child(&mut coordinator, "child");
    finish_child(&mut coordinator, "sibling");
    quiesce(&mut coordinator).await;
    let authority = reserve(&mut coordinator, "parent", "child").await;
    let mut response = begin_submission(&mut coordinator, &sender, &address, "parent", authority);
    assert!(response.try_recv().is_err());
    let pending = coordinator.pending.get("child").expect("fresh submission starts the exact wake");
    assert!(pending.request.runtime_overrides.child_submission.permits("parent", "child"));
    assert!(!pending.request.runtime_overrides.child_submission.permits("parent", "sibling"));
    assert!(coordinator.spawn_blocked_sessions.contains_key("parent"));
    assert!(coordinator.completed.contains_key("sibling"));
    assert!(!coordinator.pending.contains_key("sibling"));
    quiesce(&mut coordinator).await;
    assert!(!coordinator.pending["child"].cancellation.is_cancelled(), "repeated stale attach preserves newly admitted child work");
}

#[derive(Clone, Copy, Debug)]
enum Invalidation { Stop, Teardown, Incarnation, ChildStop, ForeignParent, OtherChild }

#[tokio::test]
async fn stop_teardown_incarnation_revocation_and_foreign_targets_reject_reserved_submission() {
    for invalidation in [Invalidation::Stop, Invalidation::Teardown, Invalidation::Incarnation,
        Invalidation::ChildStop, Invalidation::ForeignParent, Invalidation::OtherChild] {
        let (mut coordinator, sender, admissions, _received) = fixture();
        let address = insert_child(&mut coordinator, admissions.clone(), "child", "parent");
        let sibling = insert_child(&mut coordinator, admissions, "sibling", "parent");
        finish_child(&mut coordinator, "child");
        finish_child(&mut coordinator, "sibling");
        quiesce(&mut coordinator).await;
        let authority = reserve(&mut coordinator, "parent", "child").await;
        let (target, parent) = match invalidation {
            Invalidation::Stop => {
                coordinator.cancel_parent_session(Some("parent"), ChildCancellation::Authorized);
                (address.as_str(), "parent")
            }
            Invalidation::Teardown => {
                coordinator.handle_command(SubagentEvent::TeardownSession { parent_session_id: "parent".into(), respond_to: None });
                (address.as_str(), "parent")
            }
            Invalidation::Incarnation => {
                coordinator.begin_wake_incarnation("child");
                (address.as_str(), "parent")
            }
            Invalidation::ChildStop => {
                coordinator.cancel_one("child", Some("parent"), InterruptionCause::ExplicitStop { actor: SubagentActor::Human });
                (address.as_str(), "parent")
            }
            Invalidation::ForeignParent => (address.as_str(), "foreign"),
            Invalidation::OtherChild => (sibling.as_str(), "parent"),
        };
        let response = begin_submission(&mut coordinator, &sender, target, parent, authority);
        assert!(matches!(response_outcome(response).await,
            ActiveAgentMessageOutcome::NotActiveOrFinalizing | ActiveAgentMessageOutcome::NotFoundOrNotOwned), "{invalidation:?}");
        assert!(coordinator.pending.is_empty(), "{invalidation:?}");
        assert!(coordinator.spawn_blocked_sessions.contains_key("parent"));
    }
}

#[tokio::test]
async fn generic_human_source_does_not_bypass_restart_or_stop() {
    let (mut coordinator, sender, admissions, _received) = fixture();
    let address = insert_child(&mut coordinator, admissions, "child", "parent");
    finish_child(&mut coordinator, "child");
    quiesce(&mut coordinator).await;
    let response = begin_human_send(&mut coordinator, &sender, &address, "parent");
    assert_eq!(response_outcome(response).await, ActiveAgentMessageOutcome::NotActiveOrFinalizing);
    coordinator.cancel_parent_session(Some("parent"), ChildCancellation::Authorized);
    let (respond_to, response) = oneshot::channel();
    coordinator.handle_command(SubagentEvent::ReserveChildSubmission {
        parent_session_id: "parent".into(), child_session_id: "child".into(), prompt: "new work".into(), respond_to,
    });
    assert!(matches!(response.await.unwrap(), Err(ActiveAgentMessageOutcome::NotActiveOrFinalizing)));
}

#[tokio::test]
async fn continuation_consumes_exact_submission_without_reopening_parent() {
    let (mut coordinator, _sender, admissions, _received) = fixture();
    insert_child(&mut coordinator, admissions, "child", "parent");
    finish_child(&mut coordinator, "child");
    quiesce(&mut coordinator).await;
    let authority = reserve(&mut coordinator, "parent", "child").await;
    let mut request = super::super::resume::human_resume_request("child".into(), "new work".into(), "parent".into());
    let continuation = request.id.clone();
    request.runtime_overrides.child_submission = ChildSubmission::New(authority.clone());
    let (result, _result) = oneshot::channel();
    let (registered_tx, registered) = oneshot::channel();
    coordinator.handle_command(SubagentEvent::Spawn(SubagentSpawnRequest {
        request: Box::new(request), result_tx: super::super::result_delivery::SubagentResultSender::new(result, None),
        registered_tx: Some(registered_tx),
    }));
    registered.await.unwrap();
    assert!(coordinator.pending.contains_key(&continuation));
    assert!(coordinator.spawn_blocked_sessions.contains_key("parent"));
    let mut replay = super::super::resume::human_resume_request("child".into(), "new work".into(), "parent".into());
    replay.runtime_overrides.child_submission = ChildSubmission::New(authority);
    let replay_id = replay.id.clone();
    let (result, rejected) = oneshot::channel();
    coordinator.handle_command(SubagentEvent::Spawn(SubagentSpawnRequest {
        request: Box::new(replay), result_tx: super::super::result_delivery::SubagentResultSender::new(result, None), registered_tx: None,
    }));
    assert!(!rejected.await.unwrap().success());
    assert!(!coordinator.pending.contains_key(&replay_id));
}

#[tokio::test]
async fn a_reserved_submission_cannot_authorize_replaced_text() {
    let (mut coordinator, sender, admissions, _received) = fixture();
    let address = insert_child(&mut coordinator, admissions, "child", "parent");
    finish_child(&mut coordinator, "child");
    quiesce(&mut coordinator).await;
    let authority = reserve(&mut coordinator, "parent", "child").await;
    let (respond_to, response) = oneshot::channel();
    sender.try_send_active_message(SubagentActiveMessageRequest {
        request: ActiveAgentMessageRequest::try_new_from_human(&address, "replacement work", ActiveAgentMessageOperation::Steer)
            .unwrap().with_child_submission(authority),
        sender_context: ActiveMessageSenderContext::HumanRoot { session_id: Arc::from("parent") }, respond_to,
    }).unwrap();
    let ingress = coordinator.active_message_ingress.as_mut().unwrap().try_recv().unwrap();
    coordinator.handle_send_active_message(ingress);
    assert_eq!(response_outcome(response).await, ActiveAgentMessageOutcome::NotActiveOrFinalizing);
    assert!(coordinator.pending.is_empty());
}

#[tokio::test]
async fn fresh_submission_survives_its_original_completion_publication_barrier() {
    let (mut coordinator, sender, admissions, _received) = fixture();
    let address = insert_child(&mut coordinator, admissions, "child", "parent");
    finish_child_before_visibility(&mut coordinator, "child");
    quiesce(&mut coordinator).await;
    let authority = reserve(&mut coordinator, "parent", "child").await;
    let mut response = begin_submission(&mut coordinator, &sender, &address, "parent", authority);
    assert_eq!(coordinator.pending_wakes.get("child").map(Vec::len), Some(1));
    assert!(response.try_recv().is_err());
    let publication = coordinator.terminal_published_rx.try_recv().expect("original completion visibility acknowledgement");
    coordinator.handle_terminal_publication(publication);
    assert!(coordinator.pending.contains_key("child"));
    assert!(coordinator.spawn_blocked_sessions.contains_key("parent"));
    assert!(response.try_recv().is_err());
}
