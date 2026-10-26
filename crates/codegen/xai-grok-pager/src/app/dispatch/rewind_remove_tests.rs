//! Context removal through the real input, router and completion paths.
use super::*;
use crate::actions::ActionRegistry;
use crate::app::actions::{Action, Effect, TaskResult};
use crate::app::agent::AgentId;
use crate::app::agent_view::AgentView;
use crate::app::agent_view::removal_test_support::{
    feed_hidden_wake_prompt, push_numbered_prompt, push_registered_read,
};
use crate::app::agent_view::test_fixtures::key;
use crate::app::app_view::InputOutcome;
use crate::app::dispatch::dispatch;
use crate::app::dispatch::tests::{test_agent, test_agent_mut, test_app_with_agent};
use agent_client_protocol as acp;
use crossterm::event::KeyCode;
use pretty_assertions::assert_eq;

const ID: AgentId = AgentId(0);

fn session() -> acp::SessionId {
    acp::SessionId::new("test-session")
}

fn turn(prompt_index: usize) -> ContextItemRef {
    ContextItemRef::Turn { prompt_index }
}

fn exchange(id: &str) -> ContextItemRef {
    ContextItemRef::ToolExchange {
        tool_call_id: ToolCallId::from(id),
    }
}

fn toast(app: &AppView) -> Option<String> {
    test_agent(app, ID)
        .toast
        .as_ref()
        .map(|(text, _)| text.clone())
}

fn entry_ids(app: &AppView) -> Vec<EntryId> {
    test_agent(app, ID)
        .scrollback
        .iter_entries()
        .map(|(id, _)| id)
        .collect()
}

fn complete(app: &mut AppView, outcome: Result<RemoveContextItemsOutcome, String>) {
    test_agent_mut(app, ID)
        .pending_removed_context_items
        .push(session());
    handle_remove_context_items_complete(app, &session(), outcome);
}

fn removed(removed: Vec<ContextItemRef>) -> Result<RemoveContextItemsOutcome, String> {
    Ok(RemoveContextItemsOutcome::Removed {
        removed,
        skipped: Vec::new(),
    })
}

/// Turn 7: prompt, reply, interjection, read; turn 8: prompt, reply.
struct TwoTurns {
    prompt_7: EntryId,
    reply_7: EntryId,
    interjection_7: EntryId,
    read_7: EntryId,
    prompt_8: EntryId,
    reply_8: EntryId,
}

fn app_with_two_turns() -> (AppView, TwoTurns) {
    let mut app = test_app_with_agent();
    let agent = test_agent_mut(&mut app, ID);
    let prompt_7 = push_numbered_prompt(agent, "Q7", 7);
    let reply_7 = agent
        .scrollback
        .push_block(RenderBlock::agent_message("A7"));
    let interjection_7 = agent
        .scrollback
        .push_block(RenderBlock::interjection_prompt("wait"));
    let read_7 = push_registered_read(agent, "call-read-7", "a.rs");
    let prompt_8 = push_numbered_prompt(agent, "Q8", 8);
    let reply_8 = agent
        .scrollback
        .push_block(RenderBlock::agent_message("A8"));
    let turns = TwoTurns {
        prompt_7,
        reply_7,
        interjection_7,
        read_7,
        prompt_8,
        reply_8,
    };
    (app, turns)
}

fn app_with_two_reads() -> (AppView, [EntryId; 4]) {
    let mut app = test_app_with_agent();
    let agent = test_agent_mut(&mut app, ID);
    let prompt = push_numbered_prompt(agent, "Q", 0);
    let read_a = push_registered_read(agent, "call-a", "a.rs");
    let read_b = push_registered_read(agent, "call-b", "b.rs");
    let reply = agent.scrollback.push_block(RenderBlock::agent_message("A"));
    (app, [prompt, read_a, read_b, reply])
}

fn removal_action(app: &mut AppView, prompt_index: usize) -> Action {
    let agent = test_agent_mut(app, ID);
    let idx = agent.scrollback.iter_entries().position(|(_, entry)| {
        matches!(&entry.block, RenderBlock::UserPrompt(prompt) if prompt.prompt_index == Some(prompt_index))
    }).unwrap();
    agent.open_timeline_panel_at(Some(idx));
    Action::RemoveContextItems(
        crate::app::agent_view::context_removal::ContextRemovalTarget {
            agent_id: ID,
            session_id: session(),
            row: *agent.timeline_mode.panel().unwrap().cursor_row().unwrap(),
            items: vec![turn(prompt_index)],
        },
    )
}

fn open_panel_press_remove(agent: &mut AgentView) -> InputOutcome {
    let registry = ActionRegistry::defaults();
    agent.handle_input(&key(KeyCode::F(6)), &registry);
    agent.handle_input(&key(KeyCode::Char('d')), &registry)
}

#[test]
fn first_press_arms_removal_of_the_prompts_turn() {
    let (mut app, turns) = app_with_two_turns();
    let agent = test_agent_mut(&mut app, ID);
    match open_panel_press_remove(agent) {
        InputOutcome::ArmPending {
            action: Action::RemoveContextItems(refs),
            label,
            ..
        } => {
            assert_eq!(refs.items, vec![turn(7)]);
            assert_eq!(label, Some("remove this turn from context"));
        }
        other => panic!("expected an armed removal, got {other:?}"),
    }
    assert_eq!(
        test_agent(&app, ID)
            .timeline_mode
            .panel()
            .and_then(|panel| panel.cursor_row())
            .map(|row| row.entry_id()),
        Some(turns.prompt_7)
    );
    assert_eq!(
        toast(&app).as_deref(),
        Some("Press again to remove turn 1 from the model context")
    );
}

#[test]
fn second_press_through_the_app_yields_the_action() {
    let (mut app, _) = app_with_two_turns();
    app.handle_input(&key(KeyCode::F(6)));
    assert!(matches!(
        app.handle_input(&key(KeyCode::Char('d'))),
        InputOutcome::Changed
    ));
    let armed = app
        .pending_action
        .as_ref()
        .expect("the first press arms a pending action");
    assert_eq!(armed.label, Some("remove this turn from context"));
    assert!(
        matches!(&armed.action, Action::RemoveContextItems(refs) if refs.items == vec![turn(7)])
    );

    let InputOutcome::Action(Action::RemoveContextItems(refs)) =
        app.handle_input(&key(KeyCode::Char('d')))
    else {
        panic!("the second press must yield the removal action");
    };
    assert_eq!(refs.items, vec![turn(7)]);
    assert!(app.pending_action.is_none());
}

#[test]
fn unremovable_row_toasts_its_reason_and_arms_nothing() {
    let mut app = test_app_with_agent();
    let agent = test_agent_mut(&mut app, ID);
    agent
        .scrollback
        .push_block(RenderBlock::user_prompt("unnumbered"));
    let outcome = open_panel_press_remove(agent);
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    assert_eq!(
        toast(&app).as_deref(),
        Some(
            "This prompt has no context id yet (sent with send-now, or the session predates numbering)"
        )
    );
}

#[test]
fn a_second_request_while_one_is_pending_is_refused_at_arming() {
    let (mut app, _) = app_with_two_turns();
    let agent = test_agent_mut(&mut app, ID);
    agent.pending_removed_context_items.push(session());
    let outcome = open_panel_press_remove(agent);
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    assert_eq!(
        toast(&app).as_deref(),
        Some("A removal is already in progress")
    );
}

#[test]
fn dispatching_the_action_returns_the_effect_and_records_the_request() {
    let (mut app, _) = app_with_two_turns();
    let effects = dispatch(removal_action(&mut app, 7), &mut app);
    let [Effect::RemoveContextItems { session_id, items }] = effects.as_slice() else {
        panic!("expected exactly the remove effect, got {effects:?}");
    };
    assert_eq!(session_id, &session());
    assert_eq!(items, &vec![turn(7)]);
    let agent = test_agent(&app, ID);
    assert_eq!(agent.pending_removed_context_items, vec![session()]);
    assert!(
        agent.pending_effects.is_empty(),
        "the effect travels with the dispatch result"
    );
}

#[test]
fn dispatching_a_second_request_while_pending_is_refused() {
    let (mut app, _) = app_with_two_turns();
    dispatch(removal_action(&mut app, 7), &mut app);
    let effects = dispatch(removal_action(&mut app, 8), &mut app);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        test_agent(&app, ID).pending_removed_context_items,
        vec![session()]
    );
    assert_eq!(
        toast(&app).as_deref(),
        Some("A removal is already in progress")
    );
}

#[test]
fn dispatching_without_a_session_toasts_and_sends_nothing() {
    let (mut app, _) = app_with_two_turns();
    test_agent_mut(&mut app, ID).session.session_id = None;
    let effects = dispatch(removal_action(&mut app, 7), &mut app);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(
        test_agent(&app, ID)
            .pending_removed_context_items
            .is_empty()
    );
    assert_eq!(toast(&app).as_deref(), Some("Session is not connected"));
}

#[test]
fn removed_turn_takes_its_whole_span_and_nothing_else() {
    let (mut app, turns) = app_with_two_turns();
    complete(&mut app, removed(vec![turn(7)]));
    assert_eq!(entry_ids(&app), vec![turns.prompt_8, turns.reply_8]);
    let tracker = &test_agent(&app, ID).session.tracker;
    assert_eq!(tracker.context_tool_call_id_for_entry(turns.read_7), None);
    assert_eq!(
        toast(&app).as_deref(),
        Some("Removed 1 item from the model context")
    );
    assert!(
        test_agent(&app, ID)
            .pending_removed_context_items
            .is_empty()
    );
}

#[test]
fn removed_last_turn_runs_to_the_end() {
    let (mut app, turns) = app_with_two_turns();
    complete(&mut app, removed(vec![turn(8)]));
    assert_eq!(
        entry_ids(&app),
        vec![
            turns.prompt_7,
            turns.reply_7,
            turns.interjection_7,
            turns.read_7
        ]
    );
}

#[test]
fn removed_tool_exchange_takes_only_its_entry() {
    let (mut app, [prompt, read_a, read_b, reply]) = app_with_two_reads();
    complete(&mut app, removed(vec![exchange("call-a")]));
    assert_eq!(entry_ids(&app), vec![prompt, read_b, reply]);
    let tracker = &test_agent(&app, ID).session.tracker;
    assert_eq!(tracker.context_tool_call_id_for_entry(read_a), None);
    assert_eq!(
        tracker.context_tool_call_id_for_entry(read_b),
        Some(ToolCallId::from("call-b"))
    );
}

#[test]
fn only_what_the_shell_reports_is_removed() {
    let (mut app, [prompt, read_a, _, reply]) = app_with_two_reads();
    let skipped = vec![SkippedItem {
        item: exchange("call-a"),
        reason: SkipReason::CallWithoutResult,
    }];
    complete(
        &mut app,
        Ok(RemoveContextItemsOutcome::Removed {
            removed: vec![exchange("call-b")],
            skipped,
        }),
    );
    assert_eq!(entry_ids(&app), vec![prompt, read_a, reply]);
    assert_eq!(
        toast(&app).as_deref(),
        Some("Removed 1 item from the model context; 1 skipped: tool call has no result yet")
    );
}

#[test]
fn skip_reasons_are_worded() {
    for (reason, words) in [
        (SkipReason::NotInContext, "not in the model context"),
        (SkipReason::CallWithoutResult, "tool call has no result yet"),
        (SkipReason::AmbiguousId, "ambiguous id"),
    ] {
        let (mut app, _) = app_with_two_reads();
        let skipped = vec![SkippedItem {
            item: exchange("call-a"),
            reason,
        }];
        complete(
            &mut app,
            Ok(RemoveContextItemsOutcome::Removed {
                removed: vec![turn(0), exchange("call-b")],
                skipped,
            }),
        );
        assert_eq!(
            toast(&app).as_deref(),
            Some(format!("Removed 2 items from the model context; 1 skipped: {words}").as_str())
        );
    }
}

#[test]
fn refusals_and_errors_leave_the_scrollback_and_say_why() {
    let cases = [
        (
            Ok(RemoveContextItemsOutcome::TurnRunning),
            "Wait until the running turn finishes, then try again",
        ),
        (
            Ok(RemoveContextItemsOutcome::NotFound),
            "The model context no longer contains this item",
        ),
        (
            Ok(RemoveContextItemsOutcome::BeforeCompaction),
            "This turn was compacted into the summary and cannot be removed",
        ),
        (
            Ok(RemoveContextItemsOutcome::WouldOrphan),
            "Removing this item would orphan a tool result",
        ),
        (
            Err("boom".to_owned()),
            "Could not remove from the model context: boom",
        ),
    ];
    for (outcome, text) in cases {
        let (mut app, _) = app_with_two_turns();
        let before = entry_ids(&app);
        complete(&mut app, outcome);
        assert_eq!(entry_ids(&app), before, "{text}");
        assert_eq!(toast(&app).as_deref(), Some(text));
        assert!(
            test_agent(&app, ID)
                .pending_removed_context_items
                .is_empty(),
            "{text}"
        );
    }
}

#[test]
fn an_answer_without_a_pending_request_changes_nothing() {
    let (mut app, _) = app_with_two_turns();
    let before = entry_ids(&app);
    handle_remove_context_items_complete(&mut app, &session(), removed(vec![turn(7)]));
    assert_eq!(entry_ids(&app), before);
    assert_eq!(toast(&app), None);
}

#[test]
fn an_answer_for_an_unknown_session_changes_nothing() {
    let (mut app, _) = app_with_two_turns();
    test_agent_mut(&mut app, ID)
        .pending_removed_context_items
        .push(session());
    let before = entry_ids(&app);
    handle_remove_context_items_complete(
        &mut app,
        &acp::SessionId::new("other"),
        removed(vec![turn(7)]),
    );
    assert_eq!(entry_ids(&app), before);
    assert_eq!(
        test_agent(&app, ID).pending_removed_context_items,
        vec![session()]
    );
}

#[test]
fn persist_failure_still_drops_the_rows_the_shell_removed() {
    let (mut app, turns) = app_with_two_turns();
    complete(
        &mut app,
        Ok(RemoveContextItemsOutcome::PersistFailed {
            removed: vec![turn(7)],
        }),
    );
    assert_eq!(entry_ids(&app), vec![turns.prompt_8, turns.reply_8]);
    assert_eq!(
        toast(&app).as_deref(),
        Some("Removed in memory but could not be saved; it may return after a restart")
    );
    assert!(
        test_agent(&app, ID)
            .pending_removed_context_items
            .is_empty()
    );
}

#[test]
fn skipped_count_is_shown_as_a_number() {
    let (mut app, _) = app_with_two_reads();
    let skipped = vec![
        SkippedItem {
            item: exchange("call-a"),
            reason: SkipReason::NotInContext,
        },
        SkippedItem {
            item: exchange("call-x"),
            reason: SkipReason::AmbiguousId,
        },
    ];
    complete(
        &mut app,
        Ok(RemoveContextItemsOutcome::Removed {
            removed: vec![exchange("call-b")],
            skipped,
        }),
    );
    assert_eq!(
        toast(&app).as_deref(),
        Some("Removed 1 item from the model context; 2 skipped: not in the model context")
    );
}

/// Turn 7 is followed by an auto-wake turn 8 the scrollback has no prompt block for; its reply rows stay.
#[test]
fn removed_turn_stops_at_a_following_auto_wake_turn() {
    let mut app = test_app_with_agent();
    let agent = test_agent_mut(&mut app, ID);
    push_numbered_prompt(agent, "Q7", 7);
    agent
        .scrollback
        .push_block(RenderBlock::agent_message("A7"));
    feed_hidden_wake_prompt(agent, 8);
    let wake_reply = agent
        .scrollback
        .push_block(RenderBlock::agent_message("wake reply"));
    let prompt_9 = push_numbered_prompt(agent, "Q9", 9);
    complete(&mut app, removed(vec![turn(7)]));
    assert_eq!(entry_ids(&app), vec![wake_reply, prompt_9]);
}

#[test]
fn boundary_survives_when_the_entry_before_a_wake_turn_is_removed_with_its_turn() {
    let mut app = test_app_with_agent();
    let agent = test_agent_mut(&mut app, ID);
    push_numbered_prompt(agent, "Q6", 6);
    push_numbered_prompt(agent, "Q7", 7);
    agent
        .scrollback
        .push_block(RenderBlock::agent_message("A7"));
    feed_hidden_wake_prompt(agent, 8);
    let wake_reply = agent
        .scrollback
        .push_block(RenderBlock::agent_message("wake reply"));
    complete(&mut app, removed(vec![turn(7)]));
    let agent = test_agent(&app, ID);
    let q6 = agent.scrollback.iter_entries().next().map(|(id, _)| id);
    assert_eq!(entry_ids(&app).len(), 2);
    assert_eq!(entry_ids(&app).last().copied(), Some(wake_reply));
    assert!(q6.is_some_and(|id| agent.session.tracker.turn_ledger.turn_ends_after(id)));
}

#[test]
fn removal_rejects_a_changed_cursor_session_or_missing_row() {
    for change in 0..3 {
        let (mut app, turns) = app_with_two_turns();
        let action = removal_action(&mut app, 7);
        let before = entry_ids(&app);
        let agent = test_agent_mut(&mut app, ID);
        match change {
            0 => {
                agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.prompt_8));
            }
            1 => agent.session.session_id = Some(acp::SessionId::new("another-session")),
            2 => {
                agent.scrollback.remove_entry(turns.prompt_7);
            }
            _ => unreachable!(),
        }
        let after_change = entry_ids(&app);
        assert!(dispatch(action, &mut app).is_empty());
        assert_eq!(entry_ids(&app), after_change);
        assert!(
            test_agent(&app, ID)
                .pending_removed_context_items
                .is_empty()
        );
        if change != 2 {
            assert_eq!(entry_ids(&app), before);
        }
    }
}

#[test]
fn f6_rewind_requires_an_explicit_turn_and_confirmation_even_when_disabled() {
    let (mut app, turns) = app_with_two_turns();
    app.current_ui.confirm_before_rewind = Some(false);
    let agent = test_agent_mut(&mut app, ID);
    agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.read_7));
    let row = *agent.timeline_mode.panel().unwrap().cursor_row().unwrap();
    assert!(dispatch(Action::RewindTimeline(row), &mut app).is_empty());
    assert!(test_agent(&app, ID).rewind_state.is_none());

    let agent = test_agent_mut(&mut app, ID);
    agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.prompt_7));
    let row = *agent.timeline_mode.panel().unwrap().cursor_row().unwrap();
    assert!(dispatch(Action::RewindTimeline(row), &mut app).is_empty());
    assert!(matches!(
        test_agent(&app, ID).rewind_state.as_ref().unwrap().phase,
        crate::views::rewind::RewindPhase::Confirm {
            target_prompt_index: 7,
            active_idx: 2,
            ..
        }
    ));
    let effects = dispatch(Action::RewindConfirm(7), &mut app);
    assert!(
        matches!(effects.as_slice(), [Effect::RewindExecute { session_id, target_prompt_index: 7, .. }] if session_id == &session())
    );
}

#[test]
fn f6_rewind_rejects_changed_selection_before_confirmation() {
    let (mut app, turns) = app_with_two_turns();
    let agent = test_agent_mut(&mut app, ID);
    agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.prompt_7));
    let row = *agent.timeline_mode.panel().unwrap().cursor_row().unwrap();
    dispatch(Action::RewindTimeline(row), &mut app);
    let agent = test_agent_mut(&mut app, ID);
    agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.prompt_8));
    assert!(dispatch(Action::RewindConfirm(7), &mut app).is_empty());
    assert!(dispatch(Action::RewindConfirm(8), &mut app).is_empty());
}

#[test]
fn missing_rewind_point_never_falls_back_to_another_turn() {
    let (mut app, turns) = app_with_two_turns();
    app.current_ui.confirm_before_rewind = Some(false);
    let agent = test_agent_mut(&mut app, ID);
    agent
        .scrollback
        .set_selected(agent.scrollback.index_of_id(turns.prompt_7));
    dispatch(Action::Rewind, &mut app);
    let effects = dispatch(
        Action::TaskComplete(TaskResult::RewindPointsLoaded {
            agent_id: ID,
            points: vec![crate::views::rewind::RewindPointInfo {
                prompt_index: 8,
                created_at: String::new(),
                num_file_snapshots: 0,
                prompt_preview: Some("Q8".to_owned()),
                has_file_changes: false,
            }],
        }),
        &mut app,
    );
    assert!(effects.is_empty());
    assert!(test_agent(&app, ID).rewind_state.is_none());
    assert_eq!(
        toast(&app).as_deref(),
        Some("The selected rewind point no longer exists")
    );
}

#[test]
fn removing_the_selected_row_leaves_no_implicit_replacement_target() {
    let (mut app, turns) = app_with_two_turns();
    let agent = test_agent_mut(&mut app, ID);
    agent.open_timeline_panel_at(agent.scrollback.index_of_id(turns.prompt_7));
    agent.scrollback.remove_entry(turns.prompt_7);
    let registry = ActionRegistry::defaults();
    let outcome = agent.handle_input(&key(KeyCode::Char('d')), &registry);
    assert!(matches!(outcome, InputOutcome::Changed));
    assert!(agent.timeline_mode.panel().unwrap().cursor_row().is_none());
}
