use super::*;
use crate::{PreviewFailure, PreviewHistory};
use xai_grok_sampling_types::{ContextCategory, ContextPolicy, ContextSwitch, SourceOccurrence, ToolCall};

#[tokio::test]
async fn preview_uses_supplied_policy_and_draft_without_admission_or_state_changes() {
    let mut h = TestHarness::new();
    h.handle.push_user_message(ConversationItem::user("old question"));
    h.handle.push_model_output(ConversationItem::assistant("old answer"));
    let before = h.handle.snapshot().await.unwrap();
    let policy_before = h.handle.get_context_policy().await.unwrap();
    h.drain_events();
    h.drain_persistence();
    let mut policy = policy_before.clone();
    policy.set(ContextCategory::AssistantTurns, ContextSwitch::Excluded);
    for _ in 0..2 {
        let preview = h.handle.preview_request_with_context(
            PreviewHistory::Stable, policy.clone(), vec![ConversationItem::user("unsent draft")],
            Vec::new(), Some("<memory-context>preview memory</memory-context>".into()),
        ).await.unwrap();
        assert_eq!(preview.canonical_history, before.conversation);
        assert_eq!(preview.built.context_policy, policy);
        assert!(preview.built.items.iter().any(|item| item.text_content() == "unsent draft"));
        assert!(!preview.built.items.iter().any(|item| item.text_content() == "old answer"));
        assert_eq!(preview.built.items.last().unwrap().source(), SourceOccurrence::Unadmitted);
        assert!(!format!("{preview:?}").contains("unsent draft"));
        assert_eq!(h.handle.get_context_policy().await.unwrap(), policy_before);
        assert_eq!(serde_json::to_vec(&h.handle.snapshot().await.unwrap()).unwrap(), serde_json::to_vec(&before).unwrap());
        assert!(h.drain_events().is_empty());
        assert!(h.drain_persistence().is_empty());
    }
}

#[tokio::test]
async fn active_preview_never_fabricates_a_future_tool_result() {
    let mut h = TestHarness::new();
    h.handle.push_user_message(ConversationItem::user("question"));
    h.handle.push_model_output(ConversationItem::assistant_tool_calls(vec![ToolCall {
        id: "pending-call".into(), name: "read_file".into(), arguments: "{}".into(),
    }]));
    let before = h.handle.get_conversation().await;
    h.drain_persistence();
    h.drain_events();
    let preview = h.handle.preview_request_with_context(
        PreviewHistory::InFlight, ContextPolicy::default(), Vec::new(), Vec::new(), None,
    ).await.unwrap();
    assert!(preview.built.items.iter().all(|item| !matches!(item, ConversationItem::ToolResult(_))));
    assert_eq!(h.handle.get_conversation().await, before);
    assert!(h.drain_events().is_empty());
    assert!(h.drain_persistence().is_empty());
}

#[tokio::test]
async fn preview_transport_preserves_distinct_failure_causes() {
    assert!(matches!(crate::ChatStateHandle::noop().preview_request_with_context(
        PreviewHistory::Stable, ContextPolicy::default(), Vec::new(), Vec::new(), None,
    ).await, Err(PreviewFailure::MailboxClosed)));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = crate::ChatStateHandle::new(tx);
    let task = tokio::spawn(async move { handle.preview_request_with_context(
        PreviewHistory::Stable, ContextPolicy::default(), Vec::new(), Vec::new(), None,
    ).await });
    let crate::commands::ChatStateCommand::PreviewConversation { reply, .. } = rx.recv().await.unwrap() else {
        panic!("preview command");
    };
    drop(reply);
    assert!(matches!(task.await.unwrap(), Err(PreviewFailure::ReplyLost)));
}
