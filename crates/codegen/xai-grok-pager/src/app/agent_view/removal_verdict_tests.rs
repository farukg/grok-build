//! Removal verdict per entry kind and per row shape.
use super::*;
use crate::app::agent_view::removal_test_support::{push_numbered_prompt, push_registered_read};
use crate::app::agent_view::test_fixtures::make_agent;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

fn facts(kind: MessageKind) -> EntryFacts {
    EntryFacts {
        kind,
        prompt_index: None,
        is_interjection: false,
        is_bash: false,
        is_running: false,
        tool_id: None,
        is_compaction_event: false,
    }
}

fn tool(kind: MessageKind, id: &str) -> EntryFacts {
    EntryFacts {
        tool_id: Some(ToolCallId::from(id)),
        ..facts(kind)
    }
}

fn exchange(id: &str) -> ContextItemRef {
    ContextItemRef::ToolExchange {
        tool_call_id: ToolCallId::from(id),
    }
}

fn removable(item: ContextItemRef) -> RemovalVerdict {
    RemovalVerdict::Removable {
        refs: vec![item],
        skipped: 0,
    }
}

fn refused(refusal: EntryRefusal) -> RemovalVerdict {
    RemovalVerdict::NotRemovable(NotRemovable::Entry(refusal))
}

fn verdict_of(entry: EntryFacts) -> RemovalVerdict {
    removal_verdict(&RowFacts {
        has_session: true,
        members: Members::One(entry),
    })
}

fn group(first: EntryFacts, rest: Vec<EntryFacts>) -> RemovalVerdict {
    removal_verdict(&RowFacts {
        has_session: true,
        members: Members::Group { first, rest },
    })
}

/// One row per `MessageKind`, with the expected verdict written out by hand.
fn kind_table() -> Vec<(MessageKind, EntryFacts, RemovalVerdict)> {
    vec![
        (
            MessageKind::UserPrompt,
            EntryFacts {
                prompt_index: Some(3),
                ..facts(MessageKind::UserPrompt)
            },
            removable(ContextItemRef::Turn { prompt_index: 3 }),
        ),
        (
            MessageKind::AgentMessage,
            facts(MessageKind::AgentMessage),
            refused(EntryRefusal::ModelReply),
        ),
        (
            MessageKind::Execute,
            tool(MessageKind::Execute, "call-execute"),
            removable(exchange("call-execute")),
        ),
        (
            MessageKind::Read,
            tool(MessageKind::Read, "call-read"),
            removable(exchange("call-read")),
        ),
        (
            MessageKind::Edit,
            tool(MessageKind::Edit, "call-edit"),
            removable(exchange("call-edit")),
        ),
        (
            MessageKind::ListDir,
            tool(MessageKind::ListDir, "call-list"),
            removable(exchange("call-list")),
        ),
        (
            MessageKind::Search,
            tool(MessageKind::Search, "call-search"),
            removable(exchange("call-search")),
        ),
        (
            MessageKind::WebFetch,
            tool(MessageKind::WebFetch, "call-fetch"),
            removable(exchange("call-fetch")),
        ),
        (
            MessageKind::WebSearch,
            tool(MessageKind::WebSearch, "call-web"),
            removable(exchange("call-web")),
        ),
        (
            MessageKind::IntegrationSearch,
            tool(MessageKind::IntegrationSearch, "call-integration"),
            removable(exchange("call-integration")),
        ),
        (
            MessageKind::UseTool,
            tool(MessageKind::UseTool, "call-use"),
            removable(exchange("call-use")),
        ),
        (
            MessageKind::MemorySearch,
            tool(MessageKind::MemorySearch, "call-memory"),
            removable(exchange("call-memory")),
        ),
        (
            MessageKind::SentMessage,
            tool(MessageKind::SentMessage, "call-sent"),
            removable(exchange("call-sent")),
        ),
        (
            MessageKind::Skill,
            tool(MessageKind::Skill, "call-skill"),
            removable(exchange("call-skill")),
        ),
        (
            MessageKind::OtherTool,
            tool(MessageKind::OtherTool, "call-other"),
            removable(exchange("call-other")),
        ),
        (
            MessageKind::Thinking,
            facts(MessageKind::Thinking),
            refused(EntryRefusal::Reasoning),
        ),
        (
            MessageKind::System,
            facts(MessageKind::System),
            refused(EntryRefusal::UiOnly),
        ),
        (
            MessageKind::SessionEvent,
            facts(MessageKind::SessionEvent),
            refused(EntryRefusal::UiOnly),
        ),
        (
            MessageKind::BgTask,
            facts(MessageKind::BgTask),
            refused(EntryRefusal::BackgroundOrSubagent),
        ),
        (
            MessageKind::Subagent,
            facts(MessageKind::Subagent),
            refused(EntryRefusal::BackgroundOrSubagent),
        ),
        (
            MessageKind::Workflow,
            facts(MessageKind::Workflow),
            refused(EntryRefusal::BackgroundOrSubagent),
        ),
        (
            MessageKind::Btw,
            facts(MessageKind::Btw),
            refused(EntryRefusal::UiOnly),
        ),
        (
            MessageKind::ContextInfo,
            facts(MessageKind::ContextInfo),
            refused(EntryRefusal::UiOnly),
        ),
        (
            MessageKind::MemoryCapture,
            facts(MessageKind::MemoryCapture),
            refused(EntryRefusal::UiOnly),
        ),
        (
            MessageKind::Stub,
            facts(MessageKind::Stub),
            refused(EntryRefusal::UiOnly),
        ),
    ]
}

#[test]
fn every_message_kind_has_the_expected_verdict() {
    for (kind, entry, expected) in kind_table() {
        assert_eq!(verdict_of(entry), expected, "{kind:?}");
    }
}

#[test]
fn table_covers_every_message_kind() {
    let tabled: HashSet<MessageKind> = kind_table().into_iter().map(|(kind, ..)| kind).collect();
    let all: HashSet<MessageKind> = MessageKind::settings_kinds()
        .iter()
        .copied()
        .chain([MessageKind::Stub])
        .collect();
    assert_eq!(tabled, all);
}

#[test]
fn prompt_variants_name_their_own_reason() {
    let prompt = |index, interjection, bash| EntryFacts {
        prompt_index: index,
        is_interjection: interjection,
        is_bash: bash,
        ..facts(MessageKind::UserPrompt)
    };
    assert_eq!(
        verdict_of(prompt(None, false, false)),
        refused(EntryRefusal::PromptNotNumbered)
    );
    assert_eq!(
        verdict_of(prompt(None, true, false)),
        refused(EntryRefusal::Interjection)
    );
    assert_eq!(
        verdict_of(prompt(Some(2), true, false)),
        refused(EntryRefusal::Interjection)
    );
    assert_eq!(
        verdict_of(prompt(None, false, true)),
        refused(EntryRefusal::BashCommand)
    );
}

#[test]
fn tool_variants_name_their_own_reason() {
    let running = EntryFacts {
        is_running: true,
        ..tool(MessageKind::Execute, "call-1")
    };
    let untracked = facts(MessageKind::Read);
    let bash = EntryFacts {
        is_bash: true,
        ..tool(MessageKind::Execute, "bash-mode-1")
    };
    assert_eq!(verdict_of(running), refused(EntryRefusal::ToolStillRunning));
    assert_eq!(verdict_of(untracked), refused(EntryRefusal::ToolNotTracked));
    assert_eq!(verdict_of(bash), refused(EntryRefusal::BashCommand));
}

#[test]
fn finished_compaction_event_points_at_the_context_switch() {
    let event = EntryFacts {
        is_compaction_event: true,
        ..facts(MessageKind::SessionEvent)
    };
    assert_eq!(verdict_of(event), refused(EntryRefusal::CompactionSummary));
}

#[test]
fn nothing_is_removable_without_a_session() {
    for (kind, entry, _) in kind_table() {
        let verdict = removal_verdict(&RowFacts {
            has_session: false,
            members: Members::One(entry),
        });
        assert_eq!(
            verdict,
            RemovalVerdict::NotRemovable(NotRemovable::NoSession),
            "{kind:?}"
        );
    }
}

#[test]
fn group_returns_removable_members_and_counts_the_rest() {
    let verdict = group(
        tool(MessageKind::Read, "call-a"),
        vec![
            facts(MessageKind::Thinking),
            tool(MessageKind::Read, "call-b"),
        ],
    );
    assert_eq!(
        verdict,
        RemovalVerdict::Removable {
            refs: vec![exchange("call-a"), exchange("call-b")],
            skipped: 1,
        }
    );
}

#[test]
fn group_with_a_refused_head_still_removes_the_others() {
    let verdict = group(
        facts(MessageKind::Thinking),
        vec![tool(MessageKind::Read, "call-a")],
    );
    assert_eq!(
        verdict,
        RemovalVerdict::Removable {
            refs: vec![exchange("call-a")],
            skipped: 1,
        }
    );
}

#[test]
fn group_without_removable_members_names_the_first_reason() {
    let verdict = group(
        facts(MessageKind::Thinking),
        vec![facts(MessageKind::AgentMessage), facts(MessageKind::Read)],
    );
    assert_eq!(
        verdict,
        RemovalVerdict::NotRemovable(NotRemovable::Group {
            members: 3,
            first: EntryRefusal::Reasoning,
        })
    );
    assert!(
        NotRemovable::Group {
            members: 3,
            first: EntryRefusal::Reasoning
        }
        .text()
        .starts_with("None of the 3 entries in this group can be removed: "),
    );
}

#[test]
fn confirm_label_names_the_target() {
    assert_eq!(
        confirm_label(&[ContextItemRef::Turn { prompt_index: 0 }]),
        "remove this turn from context"
    );
    assert_eq!(
        confirm_label(&[exchange("c")]),
        "remove this tool call from context"
    );
    assert_eq!(
        confirm_label(&[exchange("a"), exchange("b")]),
        "remove these items from context"
    );
}

#[test]
fn items_phrase_agrees_with_the_count() {
    assert_eq!(items_phrase(1), "1 item");
    assert_eq!(items_phrase(0), "0 items");
    assert_eq!(items_phrase(3), "3 items");
}

#[test]
fn counted_uses_the_singular_only_for_one() {
    assert_eq!(counted(1, "entry", "entries"), "1 entry");
    assert_eq!(counted(0, "entry", "entries"), "0 entries");
    assert_eq!(counted(2, "entry", "entries"), "2 entries");
}

#[test]
fn single_member_group_reason_is_singular() {
    let text = NotRemovable::Group {
        members: 1,
        first: EntryRefusal::Reasoning,
    }
    .text();
    assert_eq!(
        text,
        format!(
            "The entry in this group cannot be removed: {}",
            EntryRefusal::Reasoning.text()
        )
    );
    assert!(!text.contains("1 entries"));
}

#[test]
fn arming_a_group_names_the_left_out_entries_in_the_right_number() {
    let arm = |extra_unregistered: usize| {
        crate::appearance::cache::set_group_tool_verbs(true);
        let mut agent = make_agent();
        agent.session.session_id = Some("sess".to_owned().into());
        push_numbered_prompt(&mut agent, "Q1", 0);
        push_registered_read(&mut agent, "call-a", "a.rs");
        push_registered_read(&mut agent, "call-c", "c.rs");
        for n in 0..extra_unregistered {
            agent
                .scrollback
                .push_block(RenderBlock::read(format!("u{n}.rs"), None));
        }
        agent
            .scrollback
            .push_block(RenderBlock::agent_message("done"));
        agent.scrollback.prepare_layout(80, 20);
        let group = agent
            .scrollback
            .timeline_outline()
            .into_iter()
            .find(|row| matches!(row.kind, TimelineRowKind::Group { .. }))
            .expect("consecutive reads fold into one group row");
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        agent.run_timeline_item_remove(group, &key);
        agent.toast.clone().map(|(text, _)| text)
    };
    assert_eq!(
        arm(1).as_deref(),
        Some(
            "Press again to remove 2 items from the model context (1 entry not removable, left out)"
        )
    );
    assert_eq!(
        arm(2).as_deref(),
        Some(
            "Press again to remove 2 items from the model context (2 entries not removable, left out)"
        )
    );
}

fn first_entry_row(agent: &AgentView) -> TimelineRow {
    agent.scrollback.timeline_outline().remove(0)
}

#[test]
fn verdict_for_group_row_removes_registered_members_and_counts_the_rest() {
    crate::appearance::cache::set_group_tool_verbs(true);
    let mut agent = make_agent();
    agent.session.session_id = Some("sess".to_owned().into());
    push_numbered_prompt(&mut agent, "Q1", 0);
    push_registered_read(&mut agent, "call-a", "a.rs");
    agent.scrollback.push_block(RenderBlock::read("b.rs", None));
    push_registered_read(&mut agent, "call-c", "c.rs");
    agent
        .scrollback
        .push_block(RenderBlock::agent_message("done"));
    agent.scrollback.prepare_layout(80, 20);
    let group = agent
        .scrollback
        .timeline_outline()
        .into_iter()
        .find(|row| matches!(row.kind, TimelineRowKind::Group { .. }))
        .expect("three consecutive reads fold into one group row");
    assert_eq!(
        agent.verdict_for_row(group),
        RemovalVerdict::Removable {
            refs: vec![exchange("call-a"), exchange("call-c")],
            skipped: 1,
        }
    );
}

#[test]
fn verdict_for_row_reads_scrollback_and_tracker_facts() {
    crate::appearance::cache::set_group_tool_verbs(false);
    let mut agent = make_agent();
    agent.session.session_id = Some("sess".to_owned().into());
    push_numbered_prompt(&mut agent, "Q1", 4);
    push_registered_read(&mut agent, "call-read-1", "a.rs");
    agent.scrollback.prepare_layout(80, 20);
    let rows = agent.scrollback.timeline_outline();
    let [turn, read] = rows.as_slice() else {
        panic!("a turn row and a tool row: {rows:?}");
    };
    assert_eq!(
        agent.verdict_for_row(*turn),
        removable(ContextItemRef::Turn { prompt_index: 4 })
    );
    assert_eq!(
        agent.verdict_for_row(*read),
        removable(exchange("call-read-1"))
    );
}

#[test]
fn verdict_for_row_refuses_an_unnumbered_prompt_and_a_gone_row() {
    let mut agent = make_agent();
    agent.session.session_id = Some("sess".to_owned().into());
    let id = agent.scrollback.push_block(RenderBlock::user_prompt("Q1"));
    agent.scrollback.prepare_layout(80, 20);
    let row = first_entry_row(&agent);
    assert_eq!(
        agent.verdict_for_row(row),
        refused(EntryRefusal::PromptNotNumbered)
    );
    agent.scrollback.remove_entry(id);
    assert_eq!(
        agent.verdict_for_row(row),
        RemovalVerdict::NotRemovable(NotRemovable::RowGone)
    );
}

#[test]
fn verdict_for_row_marks_a_finished_compaction_event() {
    let mut agent = make_agent();
    agent.session.session_id = Some("sess".to_owned().into());
    agent.scrollback.push_block(RenderBlock::session_event(
        SessionEvent::CompactionCompleted {
            tokens_before: Some(1000),
            tokens_after: 200,
            elapsed_ms: None,
        },
    ));
    agent.scrollback.prepare_layout(80, 20);
    let row = first_entry_row(&agent);
    assert_eq!(
        agent.verdict_for_row(row),
        refused(EntryRefusal::CompactionSummary)
    );
}
