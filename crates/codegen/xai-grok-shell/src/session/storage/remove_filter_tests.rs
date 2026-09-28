use super::remove_filter::{ConversationRemoval, remove_from_conversation};
use crate::sampling::{ConversationItem, ToolCall};
use crate::session::ContextItemRef;

#[test]
fn removing_one_parallel_tool_call_preserves_sibling_pair_and_assistant_text() {
    let mut assistant = ConversationItem::assistant_tool_calls(vec![
        ToolCall { id: "call_a".into(), name: "read_file".into(), arguments: "{}".into() },
        ToolCall { id: "call_b".into(), name: "read_file".into(), arguments: "{}".into() },
    ]);
    if let ConversationItem::Assistant(message) = &mut assistant { message.content = "kept".into(); }
    let mut items = vec![
        ConversationItem::system("system"),
        ConversationItem::user("prompt"),
        assistant,
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::tool_result("call_b", "b"),
        ConversationItem::assistant("answer"),
    ];
    let removed = remove_from_conversation(&mut items, &[ContextItemRef::ToolExchange { tool_call_id: "call_a".into() }]);
    assert_eq!(removed, ConversationRemoval::Removed(1));
    let ConversationItem::Assistant(assistant) = &items[2] else { panic!("assistant response survives") };
    assert_eq!(assistant.content.as_ref(), "kept");
    assert_eq!(assistant.tool_calls.iter().map(|call| call.id.as_ref()).collect::<Vec<_>>(), ["call_b"]);
    assert!(items.iter().any(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == "call_b")));
    assert!(!items.iter().any(|item| matches!(item, ConversationItem::ToolResult(result) if result.tool_call_id == "call_a")));
}

#[test]
fn removing_tool_call_without_result_is_rejected_without_mutation() {
    let mut items = vec![
        ConversationItem::user("prompt"),
        ConversationItem::assistant_tool_calls(vec![ToolCall { id: "call_a".into(), name: "read_file".into(), arguments: "{}".into() }]),
    ];
    let original = items.clone();
    assert_eq!(remove_from_conversation(&mut items, &[ContextItemRef::ToolExchange { tool_call_id: "call_a".into() }]), ConversationRemoval::WouldOrphan);
    assert_eq!(items.len(), original.len());
}
