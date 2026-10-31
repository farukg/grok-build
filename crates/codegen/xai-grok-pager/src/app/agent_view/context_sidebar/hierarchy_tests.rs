//! Group behavior through AgentView input and sidebar rendering.

use super::groups::{ContextGroup, GroupId};
use super::outline::CursorTarget;
use super::*;
use crate::actions::ActionRegistry;
use crate::scrollback::block::{MessageGroup, MessageKind, RenderBlock};
use crate::scrollback::blocks::InjectedBlock;
use crate::scrollback::types::{DisplayForm, DisplayMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use xai_grok_shell::sampling::{ContextPolicy, ContextPolicyMutation, ContextPolicySelection, ContextPolicyVersion, ContextScope};

fn key(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)), &ActionRegistry::defaults())
}

fn render(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, 100, 100);
    let mut buf = Buffer::empty(area);
    agent.draw_context_sidebar(area, &mut buf);
    buf
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height).map(|y| {
        (0..buf.area.width).filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol())).collect::<String>()
    }).collect::<Vec<_>>().join("\n")
}

fn open(scope: ContextScope) -> AgentView {
    let mut agent = crate::app::agent_view::test_fixtures::make_agent();
    agent.session.session_id = Some(agent_client_protocol::SessionId::new("hierarchy-a"));
    let _ = key(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ContextPolicyReport {
        policy: ContextPolicy::default(), usage: Vec::new(),
        selection: Some(ContextPolicySelection {
            version: ContextPolicyVersion { revision: 12, incarnation: "actor".to_owned() }, scope,
        }),
    }));
    let _ = render(&mut agent);
    agent
}

fn target(agent: &AgentView) -> Option<CursorTarget> {
    match &agent.context_sidebar {
        ContextSidebar::Open(open) => open.sidebar.cursor.and_then(|line| CursorTarget::of(line, open.disclosure)),
        ContextSidebar::Closed => None,
    }
}

fn select(agent: &mut AgentView, wanted: CursorTarget) {
    let _ = key(agent, KeyCode::Home);
    for _ in 0..150 {
        if target(agent) == Some(wanted) { return; }
        let _ = key(agent, KeyCode::Down);
    }
    panic!("target was not reachable: {wanted:?}");
}

#[test]
fn mixed_display_parent_sets_one_target_including_hidden_children_then_cycles() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let group = CursorTarget::Row(RowTarget::Group(GroupId::Display(MessageGroup::Conversation)));
    select(&mut agent, group);
    assert!(text(&render(&mut agent)).lines().any(|line| line.contains("Conversation") && line.contains("Mixed")));
    let _ = key(&mut agent, KeyCode::Left);
    assert_eq!(target(&agent), Some(group));
    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in [MessageKind::UserPrompt, MessageKind::AgentMessage, MessageKind::Thinking, MessageKind::Btw, MessageKind::SentMessage] {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Collapsed));
    }
    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in [MessageKind::UserPrompt, MessageKind::AgentMessage, MessageKind::Thinking, MessageKind::Btw, MessageKind::SentMessage] {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Expanded));
    }
    let _ = key(&mut agent, KeyCode::Right);
    select(&mut agent, CursorTarget::Row(RowTarget::Display(MessageKind::AgentMessage)));
    let _ = key(&mut agent, KeyCode::Char(' '));
    assert!(text(&render(&mut agent)).lines().any(|line| line.contains("Conversation") && line.contains("Mixed")));
    select(&mut agent, group);
    let before = agent.scrollback.display_defaults().clone();
    let _ = key(&mut agent, KeyCode::Enter);
    assert!(agent.block_viewer.is_some());
    assert_eq!(agent.scrollback.display_defaults(), &before);
}

#[test]
fn global_context_parent_has_one_warning_cancel_and_one_session_target_set() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Global);
    let group = CursorTarget::Row(RowTarget::Group(GroupId::Context(ContextGroup::Conversation)));
    select(&mut agent, group);
    let _ = key(&mut agent, KeyCode::Left);
    assert!(matches!(key(&mut agent, KeyCode::Char(' ')), InputOutcome::Changed));
    assert!(agent.context_confirmation_pending());
    assert_eq!(text(&render(&mut agent)).matches("Confirm Context").count(), 1);
    let _ = key(&mut agent, KeyCode::Enter);
    assert!(!agent.context_confirmation_pending());
    let _ = key(&mut agent, KeyCode::Char(' '));
    let InputOutcome::Action(Action::SetContextPolicy(edit)) = key(&mut agent, KeyCode::Char('s')) else { panic!("one bulk edit"); };
    assert_eq!(edit.expected.revision, 12);
    let ContextPolicyMutation::SetCategories { categories, value, target } = edit.mutation else { panic!("category edit"); };
    assert_eq!(categories.iter().copied().collect::<Vec<_>>(), vec![ContextCategory::UserTurns, ContextCategory::AssistantTurns, ContextCategory::Reasoning, ContextCategory::ToolExchanges]);
    assert_eq!(value, ContextSwitch::Excluded);
    assert_eq!(target, ContextScope::Session);
}

#[test]
fn context_parent_clicks_only_focus_and_missing_authority_never_mutates() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::CONTROL] {
        let buf = render(&mut agent);
        let row = text(&buf).lines().position(|line| line.contains("Instructions / capabilities")).unwrap() as u16;
        let before = match &agent.context_sidebar { ContextSidebar::Open(open) => open.disclosure, ContextSidebar::Closed => panic!("open") };
        let outcome = agent.route_context_sidebar_input(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left), column: 15, row, modifiers,
        }));
        assert!(matches!(outcome, Some(InputOutcome::Changed)));
        assert_eq!(target(&agent), Some(CursorTarget::Row(RowTarget::Group(GroupId::Context(ContextGroup::Instructions)))));
        assert!(matches!(&agent.context_sidebar, ContextSidebar::Open(open) if open.disclosure == before && matches!(open.policy, PolicyState::Ready(_))));
    }
    agent.context_policy_answered(Err(crate::app::actions::ContextPolicyTransportError::Rpc(agent_client_protocol::Error::internal_error())));
    assert!(matches!(key(&mut agent, KeyCode::Char(' ')), InputOutcome::Changed));
    assert!(!agent.context_confirmation_pending());
    let _ = key(&mut agent, KeyCode::Enter);
    assert!(agent.block_viewer.is_some());
}

#[test]
fn display_title_sets_every_leaf_kind_including_folded_groups_then_cycles() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    agent.scrollback.choose_kind_default(MessageKind::AgentMessage, DisplayForm::Collapsed);
    select(&mut agent, CursorTarget::Row(RowTarget::Group(GroupId::Display(MessageGroup::Files))));
    let _ = key(&mut agent, KeyCode::Left);
    let files_folded = match &agent.context_sidebar {
        ContextSidebar::Open(open) => !open.disclosure.group_expanded(GroupId::Display(MessageGroup::Files)),
        ContextSidebar::Closed => panic!("open"),
    };
    assert!(files_folded);
    assert!(!text(&render(&mut agent)).contains("File reads"));

    select(&mut agent, CursorTarget::Title(SectionId::Display));
    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in super::groups::all_display_leaf_kinds() {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Collapsed), "{kind:?}");
    }
    assert_eq!(
        agent.scrollback.display_defaults().get(MessageKind::Injected),
        None,
        "the legacy injected default is not a leaf and keeps its own value"
    );
    match &agent.context_sidebar {
        ContextSidebar::Open(open) => {
            assert!(!open.disclosure.group_expanded(GroupId::Display(MessageGroup::Files)), "the fold stays remembered");
            assert!(open.disclosure.is_expanded(SectionId::Display), "the section itself stays open");
        }
        ContextSidebar::Closed => panic!("open"),
    }

    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in super::groups::all_display_leaf_kinds() {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Expanded), "{kind:?}");
    }
    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in super::groups::all_display_leaf_kinds() {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Hidden), "{kind:?}");
    }
    let _ = key(&mut agent, KeyCode::Char(' '));
    for kind in super::groups::all_display_leaf_kinds() {
        assert_eq!(agent.scrollback.display_defaults().get(kind), Some(DisplayForm::Collapsed), "{kind:?}");
    }
}

#[test]
fn display_title_click_and_space_set_the_same_leaves_without_folding() {
    let _theme = crate::theme::cache::pin_theme();
    let mut keyboard = open(ContextScope::Session);
    select(&mut keyboard, CursorTarget::Title(SectionId::Display));
    let _ = key(&mut keyboard, KeyCode::Char(' '));

    let mut mouse = open(ContextScope::Session);
    let buf = render(&mut mouse);
    let row = text(&buf).lines().position(|line| line.contains("Message display")).unwrap() as u16;
    let outcome = mouse.route_context_sidebar_input(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left), column: 15, row, modifiers: KeyModifiers::NONE,
    }));
    assert!(matches!(outcome, Some(InputOutcome::Changed)));
    assert_eq!(
        keyboard.scrollback.display_defaults(),
        mouse.scrollback.display_defaults(),
        "a plain click on the title does what Space does"
    );
    assert!(
        text(&render(&mut mouse)).contains("File reads"),
        "the title gesture changes values, not the section fold"
    );
}

#[test]
fn global_fold_repair_keeps_identity_or_its_visible_parent_and_end_uses_the_tree() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let wanted = CursorTarget::Row(RowTarget::Display(MessageKind::Read));
    select(&mut agent, wanted);
    let ContextSidebar::Open(open) = &mut agent.context_sidebar else { panic!("open"); };
    let mut disclosure = open.disclosure;
    disclosure.set_group(GroupId::Display(MessageGroup::Conversation), false);
    open.apply_disclosure(disclosure);
    assert_eq!(target(&agent), Some(wanted));
    let ContextSidebar::Open(open) = &mut agent.context_sidebar else { panic!("open"); };
    disclosure.set_group(GroupId::Display(MessageGroup::Files), false);
    open.apply_disclosure(disclosure);
    assert_eq!(target(&agent), Some(CursorTarget::Row(RowTarget::Group(GroupId::Display(MessageGroup::Files)))));
    assert!(!agent.sidebar_end_is_stay());
    let _ = key(&mut agent, KeyCode::End);
    assert!(agent.sidebar_end_is_stay());
    let _ = key(&mut agent, KeyCode::Left);
    let _ = key(&mut agent, KeyCode::Left);
    let _ = key(&mut agent, KeyCode::End);
    assert!(agent.sidebar_end_is_stay());
}

#[test]
fn typed_injected_defaults_apply_to_future_entries_with_legacy_fallback() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    agent.scrollback.choose_kind_default(MessageKind::Injected, DisplayForm::Expanded);
    let first = agent.scrollback.push_block(RenderBlock::Injected(InjectedBlock::from_reason("task_completed", "done")));
    assert_eq!(agent.scrollback.get_by_id(first).unwrap().display_mode, DisplayMode::Expanded);
    agent.scrollback.choose_kind_default(MessageKind::InjectedNotification, DisplayForm::Collapsed);
    agent.scrollback.choose_kind_default(MessageKind::Injected, DisplayForm::Expanded);
    let replay = agent.scrollback.push_block(RenderBlock::Injected(InjectedBlock::from_reason("task_completed", "replayed")));
    assert_eq!(agent.scrollback.get_by_id(first).unwrap().display_mode, DisplayMode::Collapsed);
    assert_eq!(agent.scrollback.get_by_id(replay).unwrap().display_mode, DisplayMode::Collapsed);
    let unknown = InjectedBlock::from_reason("future_external_reason", "body");
    assert_eq!(MessageKind::from(&RenderBlock::Injected(unknown)), MessageKind::InjectedOther);
}

#[test]
fn mixed_context_parent_sends_one_uniform_edit_without_optimistic_values() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(ContextPolicyReport {
        policy, usage: Vec::new(),
        selection: Some(ContextPolicySelection { version: ContextPolicyVersion { revision: 13, incarnation: "actor".to_owned() }, scope: ContextScope::Session }),
    }));
    select(&mut agent, CursorTarget::Row(RowTarget::Group(GroupId::Context(ContextGroup::Conversation))));
    assert!(text(&render(&mut agent)).lines().any(|line| line.contains("Conversation") && line.contains("Mixed")));
    let InputOutcome::Action(Action::SetContextPolicy(edit)) = key(&mut agent, KeyCode::Char(' ')) else { panic!("one edit"); };
    assert_eq!(edit.expected.revision, 13);
    let ContextPolicyMutation::SetCategories { categories, value, target } = edit.mutation else { panic!("category edit"); };
    assert_eq!(categories.count(), 4);
    assert_eq!(value, ContextSwitch::Excluded);
    assert_eq!(target, ContextScope::Session);
    assert!(matches!(&agent.context_sidebar, ContextSidebar::Open(open) if open.policy.shown().unwrap().policy.includes(ContextCategory::UserTurns)));
}

#[test]
fn group_fold_restores_in_loaded_and_reopened_views_without_changing_value_scopes() {
    let _theme = crate::theme::cache::pin_theme();
    let root = tempfile::tempdir().unwrap();
    let mut a = open(ContextScope::Session);
    a.open_panel_settings_in(root.path());
    select(&mut a, CursorTarget::Row(RowTarget::Group(GroupId::Display(MessageGroup::Files))));
    let _ = key(&mut a, KeyCode::Left);
    let mut b = open(ContextScope::Global);
    b.session.session_id = Some(agent_client_protocol::SessionId::new("hierarchy-b"));
    b.open_panel_settings_in(root.path());
    let b_scope = b.panel_persistence.display_layer();
    assert!(!text(&render(&mut b)).contains("File reads"));
    select(&mut a, CursorTarget::Row(RowTarget::Group(GroupId::Display(MessageGroup::Files))));
    let _ = key(&mut a, KeyCode::Right);
    b.reload_global_panel_settings();
    assert!(text(&render(&mut b)).contains("File reads"));
    assert_eq!(b.panel_persistence.display_layer(), b_scope);
    let _ = key(&mut a, KeyCode::Left);
    let _ = key(&mut b, KeyCode::F(7));
    b.reload_global_panel_settings();
    let _ = key(&mut b, KeyCode::F(7));
    assert!(!text(&render(&mut b)).contains("File reads"));
}


#[test]
fn every_display_leaf_cycles_forward_and_backward_without_context_traffic() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.session.acp_tx = tx;
    for kind in super::groups::all_display_leaf_kinds() {
        agent.scrollback.choose_kind_default(kind, DisplayForm::Collapsed);
        select(&mut agent, CursorTarget::Row(RowTarget::Display(kind)));
        for expected in [DisplayForm::Expanded, DisplayForm::Hidden, DisplayForm::Collapsed] {
            let _ = key(&mut agent, KeyCode::Char(' '));
            assert_eq!(agent.scrollback.display_defaults().get(kind), Some(expected), "{kind:?}");
        }
        for expected in [DisplayForm::Hidden, DisplayForm::Expanded, DisplayForm::Collapsed] {
            let _ = key(&mut agent, KeyCode::Backspace);
            assert_eq!(agent.scrollback.display_defaults().get(kind), Some(expected), "{kind:?}");
        }
    }
    assert!(rx.try_recv().is_err(), "display settings never send ACP request mutations");
    assert!(agent.pending_effects.is_empty());
    assert!(!agent.context_confirmation_pending());
}

#[test]
fn instructions_have_one_display_owner_before_conversation() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let targets = super::outline::section_targets(SectionDisclosure::default(), SectionId::Display);
    assert_eq!(targets.first(), Some(&RowTarget::Group(GroupId::Display(MessageGroup::Instructions))));
    assert_eq!(targets.iter().filter(|&&target| target == RowTarget::Display(MessageKind::InjectedInstructions)).count(), 1);
    assert!(!MessageGroup::Injected.kinds().any(|kind| kind == MessageKind::InjectedInstructions));
    let display = text(&render(&mut agent));
    let display = display.split("Message display").nth(1).unwrap();
    assert!(display.find("Instructions / capabilities").unwrap() < display.find("Conversation").unwrap());
    for reason in ["primary", "project_instructions", "skill_catalog", "mcp_catalog"] {
        assert_eq!(InjectedBlock::from_reason(reason, "captured").message_kind(), MessageKind::InjectedInstructions);
    }
}

#[test]
fn settings_hide_and_restore_captured_instruction_data_without_request_changes() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open(ContextScope::Session);
    let raw = "full captured instructions\nsecond line";
    let id = agent.scrollback.push_block(RenderBlock::Injected(InjectedBlock::from_reason("primary", raw)));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.session.acp_tx = tx;
    select(&mut agent, CursorTarget::Row(RowTarget::Display(MessageKind::InjectedInstructions)));
    let _ = key(&mut agent, KeyCode::Char(' '));
    let _ = key(&mut agent, KeyCode::Char(' '));
    agent.scrollback.prepare_layout(80, 20);
    assert_eq!(agent.scrollback.get_cached_entry_height(0), Some(0));
    assert_eq!(agent.scrollback.len(), 1);
    let RenderBlock::Injected(block) = &agent.scrollback.get_by_id(id).unwrap().block else { panic!("captured block"); };
    assert_eq!(block.text, raw);
    let _ = key(&mut agent, KeyCode::Backspace);
    agent.scrollback.prepare_layout(80, 20);
    assert!(agent.scrollback.get_cached_entry_height(0).unwrap() > 0);
    let RenderBlock::Injected(block) = &agent.scrollback.get_by_id(id).unwrap().block else { panic!("captured block"); };
    assert_eq!(block.text, raw);
    assert!(rx.try_recv().is_err());
    assert!(agent.pending_effects.is_empty());
}


#[test]
fn keyboard_hidden_choice_reopens_and_reverse_choice_is_saved() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = open(ContextScope::Session);
    agent.open_panel_settings_in(dir.path());
    select(&mut agent, CursorTarget::Row(RowTarget::Display(MessageKind::InjectedInstructions)));
    let _ = key(&mut agent, KeyCode::Char(' '));
    let _ = key(&mut agent, KeyCode::Char(' '));
    let mut reopened = open(ContextScope::Session);
    reopened.open_panel_settings_in(dir.path());
    assert_eq!(reopened.scrollback.display_defaults().get(MessageKind::InjectedInstructions), Some(DisplayForm::Hidden));
    select(&mut reopened, CursorTarget::Row(RowTarget::Display(MessageKind::InjectedInstructions)));
    let _ = key(&mut reopened, KeyCode::Backspace);
    let mut again = open(ContextScope::Session);
    again.open_panel_settings_in(dir.path());
    assert_eq!(again.scrollback.display_defaults().get(MessageKind::InjectedInstructions), Some(DisplayForm::Expanded));
}
