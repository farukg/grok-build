use super::*;
use crate::commands::ChatStateCommand;
use crate::{ChatStateHandle, ContextPolicyAdmissionError};
use xai_grok_sampling_types::{ContextCategory, ContextPolicy, ContextSwitch, ToolSpec};

#[tokio::test]
async fn closed_policy_mailbox_is_not_an_included_policy_or_successful_set() {
    let handle = ChatStateHandle::noop();
    assert!(matches!(handle.get_context_policy().await,
        Err(ContextPolicyAdmissionError::MailboxClosed(crate::ChatStateMailboxClosed))));
    assert!(matches!(handle.set_context_policy(ContextPolicy::default()).await,
        Err(ContextPolicyAdmissionError::MailboxClosed(crate::ChatStateMailboxClosed))));
}

#[tokio::test]
async fn admitted_policy_commands_report_reply_loss_separately() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let handle = ChatStateHandle::new(sender);
    let read = handle.get_context_policy();
    let lose_read = async {
        let ChatStateCommand::GetContextPolicy { reply } = receiver.recv().await.unwrap() else {
            panic!("expected policy read");
        };
        drop(reply);
    };
    let (read, ()) = tokio::join!(read, lose_read);
    assert!(matches!(read, Err(ContextPolicyAdmissionError::ReplyLost(_))));

    let write = handle.set_context_policy(ContextPolicy::default());
    let lose_write = async {
        let ChatStateCommand::SetContextPolicy { reply, .. } = receiver.recv().await.unwrap() else {
            panic!("expected policy mutation");
        };
        drop(reply);
    };
    let (write, ()) = tokio::join!(write, lose_write);
    assert!(matches!(write, Err(ContextPolicyAdmissionError::ReplyLost(_))));
}

#[tokio::test]
async fn policy_acknowledges_applied_state_and_built_requests_keep_their_snapshot() {
    let h = TestHarness::with_conversation(vec![
        ConversationItem::system("core"),
        ConversationItem::project_instructions("project rules"),
        ConversationItem::user("current prompt"),
    ]);
    let mut restricted = ContextPolicy::default();
    restricted.set(ContextCategory::ProjectInstructions, ContextSwitch::Excluded);
    restricted.set(ContextCategory::Memory, ContextSwitch::Excluded);
    restricted.set(ContextCategory::ToolDefinitions, ContextSwitch::Excluded);
    let applied = h.handle.set_context_policy(restricted.clone()).await.unwrap();
    assert_eq!(applied, restricted);
    assert_eq!(h.handle.get_context_policy().await.unwrap(), applied);
    let tool = ToolSpec {
        name: "read_file".into(),
        description: None,
        parameters: serde_json::json!({"type": "object"}),
    };
    let built = h.handle.build_request(vec![tool.clone()], Some(
        "<memory-context>private memory</memory-context>".into(),
    ), false, None, "c".into(), "restricted".into()).await.unwrap();
    let applied = h.handle.set_context_policy(ContextPolicy::default()).await.unwrap();
    assert!(applied.is_unrestricted());
    assert_eq!(built.context_policy, restricted);
    assert!(built.tools.is_empty());
    assert_eq!(built.items.iter().map(ConversationItem::text_content).collect::<Vec<_>>(),
        ["core", "current prompt"]);
    let restored = h.handle.build_request(vec![tool], None, false, None,
        "c".into(), "restored".into()).await.unwrap();
    assert_eq!(restored.context_policy, applied);
    assert_eq!(restored.tools.len(), 1);
    assert_eq!(restored.items.iter().map(ConversationItem::text_content).collect::<Vec<_>>(),
        ["core", "project rules", "current prompt"]);
    assert_eq!(h.handle.get_conversation().await.len(), 3);
}
