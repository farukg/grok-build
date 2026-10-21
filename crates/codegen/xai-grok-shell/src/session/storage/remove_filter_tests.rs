use super::remove_filter::remove_from_conversation;
use crate::sampling::{ConversationItem, ToolCall};
use crate::session::{ContextItemRef, RemoveContextItemsOutcome, SkipReason, SkippedItem};
use xai_grok_sampling_types::{BackendToolCallItem, BackendToolKind, rs};

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read_file".into(),
        arguments: "{}".into(),
    }
}

fn user_at(text: &str, prompt_index: usize) -> ConversationItem {
    let mut item = ConversationItem::user(text);
    item.set_prompt_index(prompt_index);
    item
}

fn reasoning(id: &str) -> ConversationItem {
    ConversationItem::Reasoning(rs::ReasoningItem {
        id: id.to_string(),
        summary: vec![],
        content: None,
        encrypted_content: Some(format!("enc_{id}")),
        status: None,
    })
}

fn backend_search(id: &str) -> ConversationItem {
    ConversationItem::BackendToolCall(BackendToolCallItem {
        kind: BackendToolKind::WebSearch(rs::WebSearchToolCall {
            id: id.to_string(),
            status: rs::WebSearchToolCallStatus::Completed,
            action: rs::WebSearchToolCallAction::Search(rs::WebSearchActionSearch {
                query: "q".to_string(),
                sources: Some(vec![]),
            }),
        }),
    })
}

fn exchange(id: &str) -> ContextItemRef {
    ContextItemRef::ToolExchange {
        tool_call_id: id.into(),
    }
}

fn turn(prompt_index: usize) -> ContextItemRef {
    ContextItemRef::Turn { prompt_index }
}

fn skipped(item: ContextItemRef, reason: SkipReason) -> SkippedItem {
    SkippedItem { item, reason }
}

fn shape(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            ConversationItem::System(_) => "system".to_string(),
            ConversationItem::User(_) => format!("user:{}", item.text_content()),
            ConversationItem::Assistant(assistant) => format!(
                "assistant:{}:{}",
                assistant.content,
                assistant
                    .tool_calls
                    .iter()
                    .map(|call| call.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            ConversationItem::ToolResult(result) => format!("result:{}", result.tool_call_id),
            ConversationItem::BackendToolCall(call) => format!("backend:{}", call.id()),
            ConversationItem::Reasoning(reasoning) => format!("reasoning:{}", reasoning.id),
        })
        .collect()
}

/// Same adjacency rule as `ContextPolicy::project`: reasoning stays only while an output item of its response follows.
fn assert_reasoning_is_answered(items: &[ConversationItem]) {
    for (index, item) in items.iter().enumerate() {
        if matches!(item, ConversationItem::Reasoning(_)) {
            let next = items
                .iter()
                .skip(index + 1)
                .find(|next| !matches!(next, ConversationItem::Reasoning(_)));
            assert!(
                matches!(
                    next,
                    Some(ConversationItem::Assistant(_) | ConversationItem::BackendToolCall(_))
                ),
                "reasoning at {index} has no output item: {:?}",
                shape(items)
            );
        }
    }
}

#[test]
fn removing_one_parallel_tool_call_preserves_sibling_pair_and_assistant_text() {
    let mut assistant =
        ConversationItem::assistant_tool_calls(vec![call("call_a"), call("call_b")]);
    if let ConversationItem::Assistant(message) = &mut assistant {
        message.content = "kept".into();
    }
    let mut items = vec![
        ConversationItem::system("system"),
        ConversationItem::user("prompt"),
        assistant,
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::tool_result("call_b", "b"),
        ConversationItem::assistant("answer"),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("call_a")]);
    assert_eq!(removal.removed, [exchange("call_a")]);
    assert!(removal.skipped.is_empty());
    assert_eq!(
        shape(&items),
        [
            "system",
            "user:prompt",
            "assistant:kept:call_b",
            "result:call_b",
            "assistant:answer:"
        ]
    );
}

#[test]
fn tool_call_without_result_is_skipped_and_refused_without_mutation() {
    let mut items = vec![
        ConversationItem::user("prompt"),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
    ];
    let before = shape(&items);
    let removal = remove_from_conversation(&mut items, &[exchange("call_a")]);
    assert!(removal.removed.is_empty());
    assert_eq!(
        removal.skipped,
        [skipped(exchange("call_a"), SkipReason::CallWithoutResult)]
    );
    assert_eq!(removal.refusal(), RemoveContextItemsOutcome::WouldOrphan);
    assert_eq!(shape(&items), before);
}

#[test]
fn mixed_request_removes_the_answered_call_and_reports_the_unanswered_one() {
    let mut items = vec![
        user_at("prompt", 0),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::assistant_tool_calls(vec![call("call_b")]),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("call_a"), exchange("call_b")]);
    assert_eq!(removal.removed, [exchange("call_a")]);
    assert_eq!(
        removal.skipped,
        [skipped(exchange("call_b"), SkipReason::CallWithoutResult)]
    );
    assert_eq!(shape(&items), ["user:prompt", "assistant::call_b"]);
}

#[test]
fn id_with_two_results_or_two_calls_is_ambiguous_and_untouched() {
    let mut two_results = vec![
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "first"),
        ConversationItem::tool_result("call_a", "second"),
    ];
    let before = shape(&two_results);
    let removal = remove_from_conversation(&mut two_results, &[exchange("call_a")]);
    assert_eq!(
        removal.skipped,
        [skipped(exchange("call_a"), SkipReason::AmbiguousId)]
    );
    assert_eq!(shape(&two_results), before);

    let mut two_calls = vec![
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "only"),
    ];
    let removal = remove_from_conversation(&mut two_calls, &[exchange("call_a")]);
    assert_eq!(
        removal.skipped,
        [skipped(exchange("call_a"), SkipReason::AmbiguousId)]
    );
    assert_eq!(two_calls.len(), 3);
}

#[test]
fn refs_matching_nothing_are_not_found_and_leave_the_conversation_alone() {
    let mut items = vec![user_at("prompt", 0), ConversationItem::assistant("answer")];
    let before = shape(&items);
    let removal = remove_from_conversation(&mut items, &[turn(7), exchange("missing")]);
    assert!(removal.removed.is_empty());
    assert_eq!(
        removal.skipped,
        [
            skipped(turn(7), SkipReason::NotInContext),
            skipped(exchange("missing"), SkipReason::NotInContext)
        ]
    );
    assert_eq!(removal.refusal(), RemoveContextItemsOutcome::NotFound);
    assert_eq!(shape(&items), before);
}

#[test]
fn repeated_ref_is_removed_once() {
    let mut items = vec![
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::assistant("tail"),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("call_a"), exchange("call_a")]);
    assert_eq!(removal.removed, [exchange("call_a")]);
    assert!(removal.skipped.is_empty());
    assert_eq!(shape(&items), ["assistant:tail:"]);
}

#[test]
fn backend_tool_call_is_removed_with_its_sealed_blob_after_it() {
    let mut items = vec![
        user_at("prompt", 0),
        reasoning("rs_main"),
        backend_search("ws_r1_call-1"),
        reasoning("tco_r1_call-1"),
        backend_search("ws_r1_call-2"),
        reasoning("tco_r1_call-2"),
        ConversationItem::assistant("answer"),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("ws_r1_call-1")]);
    assert_eq!(removal.removed, [exchange("ws_r1_call-1")]);
    assert!(removal.skipped.is_empty());
    assert_eq!(
        shape(&items),
        [
            "user:prompt",
            "reasoning:rs_main",
            "backend:ws_r1_call-2",
            "reasoning:tco_r1_call-2",
            "assistant:answer:"
        ]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn sealed_blobs_are_matched_by_id_before_or_after_the_call_and_unrelated_blobs_stay() {
    let mut items = vec![
        user_at("prompt", 0),
        reasoning("tco_r1_call-1"),
        backend_search("ws_r1_call-1"),
        backend_search("ws_r1_call-2"),
        reasoning("tco_r1_call-2"),
        backend_search("ws_r1_call-3"),
        reasoning("tco_r1_call-3"),
        reasoning("tco_r2_call-1"),
        reasoning("rs_main"),
        ConversationItem::assistant("answer"),
    ];
    let removal = remove_from_conversation(
        &mut items,
        &[exchange("ws_r1_call-1"), exchange("ws_r1_call-2")],
    );
    assert_eq!(
        removal.removed,
        [exchange("ws_r1_call-1"), exchange("ws_r1_call-2")]
    );
    assert_eq!(
        shape(&items),
        [
            "user:prompt",
            "backend:ws_r1_call-3",
            "reasoning:tco_r1_call-3",
            "reasoning:tco_r2_call-1",
            "reasoning:rs_main",
            "assistant:answer:",
        ]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn removing_the_last_backend_call_keeps_the_other_calls_blob() {
    let mut items = vec![
        user_at("prompt", 0),
        reasoning("rs_main"),
        backend_search("ws_r1_call-1"),
        reasoning("tco_r1_call-1"),
        backend_search("ws_r1_call-2"),
        reasoning("tco_r1_call-2"),
        ConversationItem::assistant("answer"),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("ws_r1_call-2")]);
    assert_eq!(removal.removed, [exchange("ws_r1_call-2")]);
    assert_eq!(
        shape(&items),
        [
            "user:prompt",
            "reasoning:rs_main",
            "backend:ws_r1_call-1",
            "reasoning:tco_r1_call-1",
            "assistant:answer:"
        ]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn turn_removal_takes_its_exchanges_backend_calls_and_reasoning_but_not_the_next_turn() {
    let mut items = vec![
        ConversationItem::system("system"),
        user_at("first", 0),
        reasoning("r1"),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "a"),
        reasoning("rs_main"),
        backend_search("ws_r1_call-1"),
        reasoning("tco_r1_call-1"),
        ConversationItem::assistant("done"),
        user_at("second", 1),
        reasoning("r2"),
        ConversationItem::assistant("answer"),
    ];
    let removal = remove_from_conversation(&mut items, &[turn(0)]);
    assert_eq!(removal.removed, [turn(0)]);
    assert!(removal.skipped.is_empty());
    assert_eq!(
        shape(&items),
        ["system", "user:second", "reasoning:r2", "assistant:answer:"]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn dropping_an_emptied_assistant_drops_the_reasoning_of_its_response() {
    let mut items = vec![
        user_at("prompt", 0),
        reasoning("r1"),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "a"),
        reasoning("r2"),
        ConversationItem::assistant("final"),
    ];
    let removal = remove_from_conversation(&mut items, &[exchange("call_a")]);
    assert_eq!(removal.removed, [exchange("call_a")]);
    assert_eq!(
        shape(&items),
        ["user:prompt", "reasoning:r2", "assistant:final:"]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn assistant_that_keeps_text_keeps_its_reasoning() {
    let mut speaking = ConversationItem::assistant_tool_calls(vec![call("call_a")]);
    if let ConversationItem::Assistant(message) = &mut speaking {
        message.content = "let me look".into();
    }
    let mut items = vec![
        user_at("prompt", 0),
        reasoning("r1"),
        speaking,
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::assistant("final"),
    ];
    remove_from_conversation(&mut items, &[exchange("call_a")]);
    assert_eq!(
        shape(&items),
        [
            "user:prompt",
            "reasoning:r1",
            "assistant:let me look:",
            "assistant:final:"
        ]
    );
    assert_reasoning_is_answered(&items);
}

#[test]
fn unmatched_refs_are_reported_beside_the_removed_turn() {
    let mut items = vec![
        user_at("first", 0),
        ConversationItem::assistant("a0"),
        user_at("second", 1),
    ];
    let removal = remove_from_conversation(&mut items, &[turn(0), turn(9), exchange("nope")]);
    assert_eq!(removal.removed, [turn(0)]);
    assert_eq!(
        removal.skipped,
        [
            skipped(turn(9), SkipReason::NotInContext),
            skipped(exchange("nope"), SkipReason::NotInContext)
        ]
    );
    assert_eq!(shape(&items), ["user:second"]);
}
