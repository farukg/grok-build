use super::recovery::{task, terminal};
use super::*;
use crate::session::delivery_receiver::ReceiverTurnOrigin;

#[tokio::test]
async fn multi_result_commit_loss_recovers_one_input_and_one_continuation_for_the_whole_parent() {
    let (_directory, storage, info) = fixture().await;
    let first = event(
        "first",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Completed,
    );
    let second = event(
        "second",
        xai_message_delivery_core::AttemptId::mint(1),
        SubagentState::Failed {
            message: "second actual failure".into(),
        },
    );
    for event in [&first, &second] {
        storage
            .prepare_subagent_completion(&info, event.clone())
            .await
            .unwrap();
        storage
            .publish_subagent_completion(&info, event, true)
            .await
            .unwrap();
    }
    let old = task("original-caller");
    let text = format!("{}\n{}", first.text, second.text);
    let mut projection =
        CompletionProjection::new(first.id.clone(), "original-prompt".into(), text.clone());
    projection.item = ConversationItem::tool_result("actual-multi-result-call", text.clone());
    projection.receipt.parent_task = old.parent_task.clone();
    projection
        .additional_receipts
        .push(xai_chat_state::CompletionReceipt {
            completion: second.id.clone(),
            prompt_id: "original-prompt".into(),
            parent_task: old.parent_task.clone(),
        });
    assert!(matches!(
        storage.append_completion_sync(&info, &projection, &old, |_| Err(io::Error::other(
            "lost multi-result commit ack"
        ))),
        Err(StrictAppendError::Committed { .. })
    ));
    assert!(matches!(
        storage
            .append_completion_commit_aware(&info, &projection, &old)
            .await
            .unwrap(),
        CompletionProjectionAck::AlreadyPresent { .. }
    ));
    let state = storage.read_completion_state(&info).await.unwrap();
    assert_eq!(state.receipts.len(), 2);
    assert_eq!(state.obligations.len(), 2);
    let new = task("restarted-caller");
    let mut resume = CompletionProjection::new(
        first.id.clone(),
        "resumed-prompt".into(),
        "not another model input".into(),
    );
    resume.receipt.parent_task = new.parent_task.clone();
    let CompletionProjectionAck::Continue { history } = storage
        .append_completion_commit_aware(&info, &resume, &new)
        .await
        .unwrap()
    else {
        panic!("one continuation for retained tool inputs")
    };
    assert_eq!(
        history
            .iter()
            .filter(|item| item.text_content() == text)
            .count(),
        1
    );
    let state = storage.read_completion_state(&info).await.unwrap();
    assert!(state.obligations.iter().all(|obligation| {
        obligation.receipt().is_some_and(|receipt| {
            receipt.parent_task == new.parent_task && receipt.prompt_id == "resumed-prompt"
        })
    }));
    storage
        .append_update_durable_commit_aware(
            &info,
            &terminal(&info, "original-prompt", &old, ReceiverTurnOrigin::Other),
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .read_completion_state(&info)
            .await
            .unwrap()
            .obligations
            .len(),
        2
    );
    storage
        .append_update_durable_commit_aware(
            &info,
            &terminal(&info, "resumed-prompt", &new, ReceiverTurnOrigin::Other),
        )
        .await
        .unwrap();
    let settled = storage.recover_subagent_completions(&info).await.unwrap();
    assert!(settled.completion_delivery.obligations.is_empty());
    assert!(
        settled
            .completion_delivery
            .receipts
            .iter()
            .all(|receipt| receipt.prompt_id == "original-prompt"
                && receipt.parent_task == old.parent_task)
    );
    let mut sibling_retry = CompletionProjection::new(
        second.id.clone(),
        "sibling-retry".into(),
        "must not append".into(),
    );
    sibling_retry.receipt.parent_task = new.parent_task.clone();
    assert!(matches!(
        storage
            .append_completion_commit_aware(&info, &sibling_retry, &new)
            .await
            .unwrap(),
        CompletionProjectionAck::AlreadyPresent { .. }
    ));
}
