use super::*;
use crate::implementations::grok_build::task::types::{GoalLoopActive, SubagentEventSender};
use crate::reminders::task_completion::{
    ReportedTaskCompletions, TaskCompletionReminder, TaskCompletionReservations,
};
use crate::types::resources::{Resources, State};
use crate::types::tool::Reminder;

#[tokio::test]
async fn tool_reminders_reservations_and_foreign_acks_cannot_consume_child_occurrences() {
    let mut harness = harness_with_config(false, buffering());
    let spawn = tokio::spawn({
        let backend = harness.backend.clone();
        async move {
            backend
                .spawn(request("owned-child", true), None, None)
                .await
        }
    });
    assert_eq!(harness.started.recv().await.as_deref(), Some("owned-child"));
    harness.finish.send(()).unwrap();
    assert!(spawn.await.unwrap().unwrap().success());
    harness.completions.recv().await.unwrap();
    let completions =
        completions_via(&harness, Some("parent"), SubagentEvent::PeekCompletions).await;
    assert_eq!(completions.len(), 1);
    let occurrence = xai_message_delivery_core::SubagentCompletionId {
        subagent_id: "owned-child".into(),
        attempt_id: completions[0].attempt_id.clone(),
    };
    let mut resources = Resources::new();
    resources.register_state::<ReportedTaskCompletions>();
    resources.insert(SubagentEventSender(harness.backend.sender()));
    let reservations = TaskCompletionReservations::default();
    reservations.reserve("owned-child".into());
    resources.insert(reservations);
    let shared = resources.into_shared();
    for goal in [false, true] {
        shared.lock().await.insert(GoalLoopActive(goal));
        let output = crate::types::output::ToolOutput::Dynamic(serde_json::Value::Null.into());
        for _ in 0..2 {
            assert!(
                TaskCompletionReminder
                    .collect_reminders(shared.clone(), &output)
                    .await
                    .is_empty()
            );
            assert_eq!(
                peeked_completion_ids(&harness, Some("parent")).await,
                ["owned-child"]
            );
        }
    }
    assert!(
        !shared
            .lock()
            .await
            .get::<State<ReportedTaskCompletions>>()
            .unwrap()
            .is_reported("owned-child")
    );
    for (parent_session_id, id) in [
        ("foreign-parent", occurrence.clone()),
        (
            "parent",
            xai_message_delivery_core::SubagentCompletionId {
                subagent_id: "owned-child".into(),
                attempt_id: xai_message_delivery_core::AttemptId::mint(99),
            },
        ),
    ] {
        harness
            .backend
            .sender()
            .send(SubagentEvent::AcknowledgeCompletions {
                parent_session_id: parent_session_id.into(),
                completions: vec![id],
            })
            .unwrap();
        assert_eq!(
            peeked_completion_ids(&harness, Some("parent")).await,
            ["owned-child"]
        );
    }
    harness
        .backend
        .sender()
        .send(SubagentEvent::AcknowledgeCompletions {
            parent_session_id: "parent".into(),
            completions: vec![occurrence],
        })
        .unwrap();
    assert!(
        peeked_completion_ids(&harness, Some("parent"))
            .await
            .is_empty()
    );
    harness.actor.abort();
}

#[tokio::test]
async fn producer_commit_failure_fails_the_caller_and_leaves_no_phantom_completion() {
    let dropped_ack_rejection = "host producer callback dropped before durable acknowledgement";
    for (behavior, expected_reason) in [
        (
            ProducerCommitBehavior::Reject {
                reason: "disk full",
            },
            "disk full",
        ),
        (
            ProducerCommitBehavior::DropAcknowledgement,
            dropped_ack_rejection,
        ),
    ] {
        let mut harness = harness_with_options(
            RunnerBehavior {
                producer_commit: behavior,
                ..Default::default()
            },
            buffering(),
        );
        let spawn = tokio::spawn({
            let backend = harness.backend.clone();
            async move {
                backend
                    .spawn(request("commit-failed", true), None, None)
                    .await
            }
        });
        assert_eq!(
            harness.started.recv().await.as_deref(),
            Some("commit-failed")
        );
        harness.finish.send(()).unwrap();
        let outcome = spawn.await.unwrap().unwrap();
        let error = outcome.error().expect("a rejected commit fails the caller");
        assert!(!outcome.success(), "{error}");
        assert!(
            error.contains(expected_reason),
            "producer rejection must reach the caller: {error}"
        );
        assert!(
            peeked_completion_ids(&harness, Some("parent"))
                .await
                .is_empty(),
            "a rejected commit must not buffer a completable occurrence"
        );
        harness.actor.abort();
    }
}

#[tokio::test]
async fn reused_child_keeps_both_attempts_and_a_late_ack_cannot_consume_the_new_one() {
    const CHILD: &str = "019e0000-0000-7000-8000-0000000000c1";
    let mut harness = harness_with_config(false, buffering());
    let backend = harness.backend.scoped_to_session("parent");
    let spawn = tokio::spawn({
        let backend = backend.clone();
        async move { backend.spawn(request(CHILD, true), None, None).await }
    });
    assert_eq!(harness.started.recv().await.as_deref(), Some(CHILD));
    harness.finish.send(()).unwrap();
    assert!(spawn.await.unwrap().unwrap().success());
    harness.completions.recv().await.unwrap();
    let first = completions_via(&harness, Some("parent"), SubagentEvent::PeekCompletions).await;
    assert_eq!(first.len(), 1);
    let old = xai_message_delivery_core::SubagentCompletionId {
        subagent_id: CHILD.into(),
        attempt_id: first[0].attempt_id.clone(),
    };
    let wake = tokio::spawn({
        let backend = backend.clone();
        async move {
            backend
                .send_active_message(
                    ActiveAgentMessageRequest::try_new(CHILD, "second actual attempt").unwrap(),
                )
                .await
        }
    });
    assert_eq!(harness.started.recv().await.as_deref(), Some(CHILD));
    assert!(matches!(
        wake.await.unwrap(),
        ActiveAgentMessageOutcome::Accepted { .. }
    ));
    harness.finish.send(()).unwrap();
    harness.completions.recv().await.unwrap();
    let both = completions_via(&harness, Some("parent"), SubagentEvent::PeekCompletions).await;
    assert_eq!(both.len(), 2);
    let new = both
        .iter()
        .find(|summary| summary.attempt_id != old.attempt_id)
        .expect("new actual coordinator attempt")
        .attempt_id
        .clone();
    let (respond_to, receive) = oneshot::channel();
    backend
        .sender()
        .send(SubagentEvent::PeekCompletions(SubagentCompletionsRequest {
            parent_session_id: Some("parent".into()),
            suppress_ids: vec![old.clone()],
            respond_to,
        }))
        .unwrap();
    let unsuppressed = receive.await.unwrap();
    assert_eq!(unsuppressed.len(), 1);
    assert_eq!(unsuppressed[0].attempt_id, new);
    for _ in 0..2 {
        backend
            .sender()
            .send(SubagentEvent::AcknowledgeCompletions {
                parent_session_id: "parent".into(),
                completions: vec![old.clone()],
            })
            .unwrap();
        let pending =
            completions_via(&harness, Some("parent"), SubagentEvent::PeekCompletions).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt_id, new);
    }
    harness.actor.abort();
}
