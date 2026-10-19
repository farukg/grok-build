use super::*;
use crate::{
    AssistantItem, ContextSwitch, RuntimeNotice, ToolCall, all_categories,
    conversation::ConversationItem as Item,
};
use async_openai::types::responses as rs;

fn reasoning(id: &str) -> Item {
    Item::Reasoning(rs::ReasoningItem {
        id: id.to_string(),
        summary: vec![rs::SummaryPart::SummaryText(rs::SummaryTextContent {
            text: format!("thinking {id}"),
        })],
        content: None,
        encrypted_content: Some(format!("enc_{id}")),
        status: None,
    })
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read_file".to_string(),
        arguments: "{}".into(),
    }
}

fn answer(text: &str, calls: Vec<ToolCall>) -> Item {
    Item::Assistant(AssistantItem {
        content: text.into(),
        tool_calls: calls,
        model_id: None,
        model_fingerprint: None,
        reasoning_effort: None,
    })
}

/// One item per category the policy can switch, then the current turn.
fn history() -> Vec<Item> {
    vec![
        Item::system("core"),
        Item::session_prefix("environment"),
        Item::project_instructions("rules"),
        Item::user_meta("summary"),
        Item::user("old prompt"),
        reasoning("r1"),
        answer("old answer", vec![call("c1"), call("c2")]),
        Item::tool_result("c1", "result one"),
        Item::tool_result("c2", "result two"),
        reasoning("r2"),
        answer("done", vec![]),
        Item::system_reminder("reminder"),
        Item::interjection("steer"),
        Item::user("current prompt"),
        reasoning("r3"),
        answer("", vec![call("c3")]),
        Item::tool_result("c3", "current result"),
    ]
}

fn texts(items: &[Item]) -> Vec<String> {
    items.iter().map(Item::text_content).collect()
}

fn without(category: ContextCategory) -> ContextPolicy {
    let mut policy = ContextPolicy::default();
    policy.set(category, ContextSwitch::Excluded);
    policy
}

/// Every call has exactly one result and every result a call, so a provider accepts the history.
fn assert_tool_pairs_hold(items: &[Item]) {
    let mut open: Vec<String> = Vec::new();
    for item in items {
        match item {
            Item::Assistant(assistant) => {
                assert!(open.is_empty(), "calls without results before {item:?}");
                open = assistant.tool_calls.iter().map(|c| c.id.to_string()).collect();
            }
            Item::ToolResult(result) => {
                let position = open.iter().position(|id| *id == result.tool_call_id);
                assert!(position.is_some(), "result without a call: {result:?}");
                open.remove(position.unwrap_or_default());
            }
            _ => {}
        }
    }
    assert!(open.is_empty(), "calls without results at the end: {open:?}");
}

/// A reasoning item is followed by an output item of its response.
fn assert_reasoning_is_answered(items: &[Item]) {
    for (index, item) in items.iter().enumerate() {
        if matches!(item, Item::Reasoning(_)) {
            let next = items[index + 1..]
                .iter()
                .find(|next| !matches!(next, Item::Reasoning(_)));
            assert!(
                matches!(next, Some(Item::Assistant(_) | Item::BackendToolCall(_))),
                "reasoning at {index} has no output: {items:?}"
            );
        }
    }
}

#[test]
fn an_unrestricted_policy_sends_the_history_as_it_is() {
    let items = history();
    let before = texts(&items);
    assert_eq!(texts(&ContextPolicy::default().project(items)), before);
}

#[test]
fn every_category_off_removes_only_its_own_items_and_leaves_valid_history() {
    let baseline = history();
    let expected_gone: &[(ContextCategory, &[&str])] = &[
        (ContextCategory::CoreInstructions, &["core"]),
        (ContextCategory::Environment, &["environment"]),
        (ContextCategory::ProjectInstructions, &["rules"]),
        (ContextCategory::CompactionSummary, &["summary"]),
        (ContextCategory::UserTurns, &["old prompt"]),
        (
            ContextCategory::AssistantTurns,
            &["old answer", "done", "thinking r2"],
        ),
        (
            ContextCategory::ToolExchanges,
            &["result one", "result two"],
        ),
        (
            ContextCategory::RuntimeNotices(RuntimeNotice::SystemReminder),
            &["reminder"],
        ),
        (
            ContextCategory::RuntimeNotices(RuntimeNotice::Interjection),
            &["steer"],
        ),
    ];
    for (category, gone) in expected_gone {
        let projected = without(*category).project(baseline.clone());
        let kept = texts(&projected);
        for text in *gone {
            assert!(!kept.iter().any(|t| t.contains(text)), "{category:?} keeps {text}");
        }
        for text in ["current prompt", "current result"] {
            assert!(kept.iter().any(|t| t.contains(text)), "{category:?} drops {text}");
        }
        let untouched = texts(&baseline)
            .into_iter()
            .filter(|text| !gone.iter().any(|g| text.contains(g)))
            .filter(|text| !text.is_empty())
            .all(|text| kept.contains(&text));
        assert!(untouched, "{category:?} removed more than its own items: {kept:?}");
        assert_tool_pairs_hold(&projected);
        assert_reasoning_is_answered(&projected);
    }
}

#[test]
fn reasoning_off_removes_reasoning_before_the_current_turn_only() {
    let projected = without(ContextCategory::Reasoning).project(history());
    let reasoning_ids: Vec<_> = projected
        .iter()
        .filter_map(|item| match item {
            Item::Reasoning(r) => Some(r.id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning_ids, ["r3"]);
}

#[test]
fn a_response_that_loses_all_output_loses_its_reasoning() {
    let mut policy = without(ContextCategory::AssistantTurns);
    policy.set(ContextCategory::ToolExchanges, ContextSwitch::Excluded);
    let projected = policy.project(history());
    assert_reasoning_is_answered(&projected);
    assert!(!projected.iter().any(|item| matches!(item, Item::Reasoning(r) if r.id == "r1" || r.id == "r2")));
}

#[test]
fn tool_definitions_off_takes_the_exchanges_that_need_them() {
    let projected = without(ContextCategory::ToolDefinitions).project(history());
    let kept = texts(&projected);
    assert!(!kept.iter().any(|t| t.contains("result one")));
    assert_tool_pairs_hold(&projected);
}

#[test]
fn every_category_off_still_sends_the_current_turn() {
    let mut policy = ContextPolicy::default();
    for category in all_categories() {
        policy.set(category, ContextSwitch::Excluded);
    }
    let projected = policy.project(history());
    assert_eq!(
        texts(&projected),
        ["current prompt", "thinking r3", "", "current result"],
        "{projected:?}"
    );
}
