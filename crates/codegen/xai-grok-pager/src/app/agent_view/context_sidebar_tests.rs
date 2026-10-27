//! F7: the sidebar shows the shell's policy, asks for changes and never shows an unanswered one.
#[path = "context_scope_tests.rs"]
mod context_scope;
use super::{AgentView, AppRenderParams, BannerSlotParams, test_fixtures};
use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::render::ScratchBuffer;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;
use xai_grok_shell::sampling::{CategoryTokens, ContextCategory, ContextPolicy, ContextSwitch, ContextPolicyMutation, ContextPolicySelection, ContextPolicyVersion, ContextScope};

fn selection(scope: ContextScope, revision: u64) -> Option<ContextPolicySelection> {
    Some(ContextPolicySelection { version: ContextPolicyVersion { revision, incarnation: "actor".to_owned() }, scope })
}

fn draw(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, 160, 45);
    let mut buf = Buffer::empty(area);
    agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut ScratchBuffer::new(),
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    buf
}

fn screen(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    )
}

fn sidebar_row(buf: &Buffer, label: &str) -> String {
    screen(buf)
        .lines()
        .find(|line| line.contains(label))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn f7_shows_the_shells_policy_and_waits_for_its_answer_to_a_change() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = test_fixtures::make_agent();
    agent.session.session_id = Some(agent_client_protocol::SessionId::new("context-session"));

    assert!(matches!(
        press(&mut agent, KeyCode::F(7)),
        InputOutcome::Action(Action::FetchContextPolicy)
    ));
    assert!(screen(&draw(&mut agent)).contains("Loading"));

    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(ContextPolicyReport {
        selection: selection(ContextScope::Session, 1),
        policy,
        usage: vec![CategoryTokens {
            category: ContextCategory::Reasoning,
            tokens: 12_345,
        }],
    }));
    let buf = draw(&mut agent);
    assert!(sidebar_row(&buf, "Reasoning").contains("OFF"));
    assert!(sidebar_row(&buf, "Reasoning").contains("~12K"));
    assert!(sidebar_row(&buf, "Your messages").contains("ON"));

    let InputOutcome::Action(Action::SetContextPolicy(requested)) =
        press(&mut agent, KeyCode::Char(' '))
    else {
        panic!("space asks the shell for the change");
    };
    let first = xai_grok_shell::sampling::all_categories()[0];
    assert_eq!(requested.mutation, ContextPolicyMutation::SetCategories {
        categories: vec![first].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Session,
    }, "only the selected category is changed");
    let mut confirmed = ContextPolicy::default();
    confirmed.set(first, ContextSwitch::Excluded);
    confirmed.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    assert!(
        screen(&draw(&mut agent)).contains("Applying"),
        "the row keeps the shell's last answer until it answers again"
    );

    agent.context_policy_answered(Ok(ContextPolicyReport {
        policy: confirmed,
        usage: Vec::new(),
        selection: selection(ContextScope::Session, 2),
    }));
    assert!(!screen(&draw(&mut agent)).contains("Applying"));
    assert!(matches!(
        press(&mut agent, KeyCode::Esc),
        InputOutcome::Changed
    ));
    assert!(!screen(&draw(&mut agent)).contains("Session overview"));
}

fn focus_row(agent: &mut AgentView, label: &str) {
    let _ = press(agent, KeyCode::Home);
    for _ in 0..150 {
        if matches!(&agent.context_sidebar, ContextSidebar::Open(open) if open.cursor_label() == Some(label)) {
            return;
        }
        let _ = press(agent, KeyCode::Down);
    }
    panic!("row is not reachable: {label}");
}

fn click_row(agent: &mut AgentView, buf: &Buffer, label: &str) -> InputOutcome {
    let y = screen(buf)
        .lines()
        .position(|line| line.contains(label))
        .expect("row is visible") as u16;
    agent.handle_input(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: buf.area.right() - 12,
            row: y,
            modifiers: KeyModifiers::NONE,
        }),
        &ActionRegistry::defaults(),
    )
}

#[test]
fn f7_message_display_flips_a_kind_between_one_line_and_full() {
    use crate::scrollback::block::MessageKind;
    use crate::scrollback::types::DisplayForm;

    let _theme = crate::theme::cache::pin_theme();
    let mut agent = test_fixtures::make_agent();
    let _ = press(&mut agent, KeyCode::F(7));
    focus_row(&mut agent, "File reads");
    let buf = draw(&mut agent);
    assert!(sidebar_row(&buf, "File reads").contains("1 line"));

    assert!(matches!(
        click_row(&mut agent, &buf, "File reads"),
        InputOutcome::Changed
    ));
    assert!(sidebar_row(&draw(&mut agent), "File reads").contains("full"));
    assert_eq!(
        agent.scrollback.display_defaults().get(MessageKind::Read),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        agent
            .scrollback
            .display_defaults()
            .get(MessageKind::Execute),
        None
    );

    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "File reads");
    assert!(sidebar_row(&draw(&mut agent), "File reads").contains("1 line"));
}

use super::context_sidebar::{ContextSidebar, SectionDisclosure, SectionId};
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use crate::theme::Theme;
use crate::views::row_style::{SidebarRowClass, sidebar_style};
use crate::views::sidebar::SidebarLine;
use ratatui::style::Modifier;

fn ready_report() -> ContextPolicyReport {
    ContextPolicyReport {
        policy: ContextPolicy::default(),
        usage: Vec::new(),
        selection: selection(ContextScope::Session, 1),
    }
}

#[test]
fn context_clicks_never_change_policy_even_with_bulk_modifiers() {
    let _theme = crate::theme::cache::pin_theme();
    for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::CONTROL] {
        let mut agent = open_sidebar();
        let before = disclosure(&agent);
        let buf = draw(&mut agent);
        assert!(matches!(click_row_with(&mut agent, &buf, "Core instructions", modifiers), InputOutcome::Changed));
        assert_eq!(disclosure(&agent), before);
        let after = draw(&mut agent);
        assert!(sidebar_row(&after, "Core instructions").contains("ON"));
        assert!(!screen(&after).contains("Applying"));
        assert!(!screen(&after).contains("Notice: "));
    }
}

/// F7 open, the shell answered, first frame drawn: the cursor sits on the first context switch.
fn open_sidebar() -> AgentView {
    let mut agent = test_fixtures::make_agent();
    agent.session.session_id = Some(agent_client_protocol::SessionId::new("context-session"));
    let _ = press(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    let _ = draw(&mut agent);
    agent
}

fn press_with(agent: &mut AgentView, code: KeyCode, modifiers: KeyModifiers) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, modifiers)),
        &ActionRegistry::defaults(),
    )
}

fn click_row_with(
    agent: &mut AgentView,
    buf: &Buffer,
    label: &str,
    modifiers: KeyModifiers,
) -> InputOutcome {
    let y = screen(buf)
        .lines()
        .position(|line| line.contains(label))
        .expect("row is visible") as u16;
    agent.handle_input(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: buf.area.right() - 12,
            row: y,
            modifiers,
        }),
        &ActionRegistry::defaults(),
    )
}

fn cursor(agent: &AgentView) -> Option<SidebarLine> {
    match &agent.context_sidebar {
        ContextSidebar::Open(open) => open.cursor(),
        ContextSidebar::Closed => None,
    }
}

fn disclosure(agent: &AgentView) -> SectionDisclosure {
    match &agent.context_sidebar {
        ContextSidebar::Open(open) => open.disclosure(),
        ContextSidebar::Closed => SectionDisclosure::default(),
    }
}

fn every_bulk_kind_shows(agent: &AgentView, form: DisplayForm) -> bool {
    MessageKind::bulk_kinds()
        .iter()
        .all(|&kind| agent.scrollback.display_defaults().shown(kind) == form)
}

fn never_bulk_toggled_stay(agent: &AgentView, before: &[(MessageKind, DisplayForm)]) -> bool {
    before
        .iter()
        .all(|&(kind, form)| agent.scrollback.display_defaults().shown(kind) == form)
}

fn never_bulk_toggled_snapshot(agent: &AgentView) -> Vec<(MessageKind, DisplayForm)> {
    [
        MessageKind::UserPrompt,
        MessageKind::AgentMessage,
        MessageKind::MemoryCapture,
    ]
    .into_iter()
    .map(|kind| (kind, agent.scrollback.display_defaults().shown(kind)))
    .collect()
}

fn toast(agent: &AgentView) -> String {
    agent
        .toast
        .as_ref()
        .map(|(message, _)| message.clone())
        .unwrap_or_default()
}

/// Screen row and text of the first line containing `needle`.
fn row_of(buf: &Buffer, needle: &str) -> (u16, String) {
    let (y, line) = screen(buf)
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(needle))
        .map(|(y, line)| (y as u16, line.to_string()))
        .expect("text is visible");
    (y, line)
}

/// Column of the rightmost `needle`: the sidebar is the rightmost column of the screen.
fn column_of(line: &str, needle: &str) -> u16 {
    let byte = line.rfind(needle).expect("text is in the row");
    line[..byte].chars().count() as u16
}

fn cell(buf: &Buffer, x: u16, y: u16) -> &ratatui::buffer::Cell {
    buf.cell((x, y)).expect("cell is on the screen")
}

#[test]
fn f7_arrow_keys_walk_the_tree_like_a_tree_view() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let (context, display) = (SectionId::Context, SectionId::Display);
    assert_eq!(cursor(&agent), Some(context.row_line(1)));
    let _ = press(&mut agent, KeyCode::Left);
    assert_eq!(cursor(&agent), Some(context.row_line(0)), "Left selects the semantic parent");
    let _ = press(&mut agent, KeyCode::Left);
    assert_eq!(cursor(&agent), Some(context.row_line(0)), "Left folds the semantic parent");
    let _ = press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor(&agent),
        Some(context.title_line()),
        "Left from a folded group selects its section"
    );
    assert!(disclosure(&agent).is_expanded(context));
    let _ = press(&mut agent, KeyCode::Left);
    assert!(
        !disclosure(&agent).is_expanded(context),
        "Left on an open title folds it"
    );
    assert_eq!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor(&agent),
        Some(context.title_line()),
        "a folded root title has no parent"
    );
    assert!(!disclosure(&agent).is_expanded(context));

    let _ = press(&mut agent, KeyCode::Down);
    assert_eq!(
        cursor(&agent),
        Some(display.title_line()),
        "Down skips the folded rows"
    );
    let _ = press(&mut agent, KeyCode::Right);
    assert_eq!(
        cursor(&agent),
        Some(display.row_line(0)),
        "Right on an open title enters it"
    );
    let _ = press(&mut agent, KeyCode::Up);
    assert_eq!(cursor(&agent), Some(display.title_line()));
    let _ = press(&mut agent, KeyCode::Up);
    assert_eq!(
        cursor(&agent),
        Some(context.title_line()),
        "Up skips the folded rows"
    );

    let _ = press(&mut agent, KeyCode::Right);
    assert!(
        disclosure(&agent).is_expanded(context),
        "Right on a folded title opens it"
    );
    assert_eq!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::Right);
    assert_eq!(cursor(&agent), Some(context.row_line(0)));
}

#[test]
fn f7_vi_keys_mirror_the_arrows() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let context = SectionId::Context;
    let _ = press(&mut agent, KeyCode::Char('h'));
    assert_eq!(cursor(&agent), Some(context.row_line(0)));
    let _ = press(&mut agent, KeyCode::Char('h'));
    let _ = press(&mut agent, KeyCode::Char('h'));
    assert_eq!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::Char('l'));
    assert_eq!(cursor(&agent), Some(context.row_line(0)));
    let _ = press(&mut agent, KeyCode::Char('l'));
    let _ = press(&mut agent, KeyCode::Char('j'));
    assert_eq!(cursor(&agent), Some(context.row_line(1)));
    let _ = press(&mut agent, KeyCode::Char('k'));
    assert_eq!(cursor(&agent), Some(context.row_line(0)));
}

#[test]
fn f7_home_end_and_page_keys_move_over_visible_lines() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let (context, display) = (SectionId::Context, SectionId::Display);
    let last = MessageKind::settings_kinds().len() + 4;

    let _ = press(&mut agent, KeyCode::End);
    assert_eq!(cursor(&agent), Some(display.row_line(last)));
    let _ = press(&mut agent, KeyCode::Home);
    assert_eq!(cursor(&agent), Some(SectionId::Dock.title_line()));
    let _ = press(&mut agent, KeyCode::PageDown);
    assert_ne!(cursor(&agent), Some(SectionId::Dock.title_line()));
    let _ = press(&mut agent, KeyCode::PageUp);
    assert_eq!(cursor(&agent), Some(SectionId::Dock.title_line()));
    for _ in 0..4 { let _ = press(&mut agent, KeyCode::Down); }
    assert_eq!(cursor(&agent), Some(context.title_line()));

    let _ = press(&mut agent, KeyCode::Char(' '));
    assert!(!disclosure(&agent).is_expanded(context));
    let _ = press(&mut agent, KeyCode::Down);
    assert_eq!(cursor(&agent), Some(display.title_line()));
    let _ = press(&mut agent, KeyCode::Up);
    let _ = press(&mut agent, KeyCode::Char('j'));
    assert_eq!(
        cursor(&agent),
        Some(display.title_line()),
        "j also skips the folded rows"
    );
}

#[test]
fn f7_context_title_click_only_selects_and_keyboard_changes_disclosure() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "Context");
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("Core instructions"));

    assert!(matches!(
        press(&mut agent, KeyCode::Char(' ')),
        InputOutcome::Changed
    ));
    let buf = draw(&mut agent);
    assert!(
        !screen(&buf).contains("Core instructions"),
        "a folded section shows no rows"
    );
    assert!(screen(&buf).contains("\u{25B8} Context"));
    assert!(
        screen(&buf).contains("Message display"),
        "the other section stays"
    );

    assert!(matches!(
        click_row(&mut agent, &buf, "\u{25B8} Context"),
        InputOutcome::Changed
    ));
    assert!(!disclosure(&agent).is_expanded(SectionId::Context));
    assert_eq!(cursor(&agent), Some(SectionId::Context.title_line()));
    assert!(matches!(press(&mut agent, KeyCode::Char(' ')), InputOutcome::Changed));
    let buf = draw(&mut agent);
    assert!(screen(&buf).contains("Core instructions"));
    assert!(screen(&buf).contains("\u{25BE} Context"));

    assert!(matches!(
        press(&mut agent, KeyCode::Char(' ')),
        InputOutcome::Changed
    ));
    assert!(!disclosure(&agent).is_expanded(SectionId::Context));
}

#[test]
fn f7_clicks_hit_the_right_row_after_a_fold() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "Context");
    let _ = press(&mut agent, KeyCode::Left);
    let buf = draw(&mut agent);
    assert!(matches!(
        click_row(&mut agent, &buf, "File reads"),
        InputOutcome::Changed
    ));
    assert_eq!(
        agent.scrollback.display_defaults().get(MessageKind::Read),
        Some(DisplayForm::Expanded),
        "the row moved up with the fold and the click found it"
    );
    assert_eq!(
        agent
            .scrollback
            .display_defaults()
            .get(MessageKind::Execute),
        None
    );
}

#[test]
fn f7_clicks_hit_the_right_row_after_end_scrolls() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let buf = draw(&mut agent);
    assert!(
        !screen(&buf).contains("Memory captures"),
        "the last row starts below the viewport"
    );

    let _ = press(&mut agent, KeyCode::End);
    let buf = draw(&mut agent);
    assert!(
        screen(&buf).contains("Memory captures"),
        "End scrolls the last row into view"
    );
    assert!(matches!(
        click_row(&mut agent, &buf, "Memory captures"),
        InputOutcome::Changed
    ));
    assert_eq!(
        agent
            .scrollback
            .display_defaults()
            .get(MessageKind::MemoryCapture),
        Some(DisplayForm::Collapsed),
        "memory captures start full, the click made them one line"
    );
}

#[test]
fn f7_shift_enter_on_a_message_row_sets_the_other_bulk_kinds_and_reverts() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "Memory captures");
    // The cursor sits on MemoryCapture: the gesture leaves it,
    // and the user and assistant rows, exactly as they were.
    let before = never_bulk_toggled_snapshot(&agent);

    assert!(matches!(
        press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT),
        InputOutcome::Changed
    ));
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::MemoryCapture {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Expanded,
            "{kind:?}"
        );
    }
    assert!(never_bulk_toggled_stay(&agent, &before));
    assert!(disclosure(&agent).is_expanded(SectionId::Context));
    assert!(disclosure(&agent).is_expanded(SectionId::Display));

    assert!(matches!(
        press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT),
        InputOutcome::Changed
    ));
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::MemoryCapture {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Collapsed,
            "{kind:?}"
        );
    }
    assert!(never_bulk_toggled_stay(&agent, &before));
    let buf = draw(&mut agent);
    assert!(sidebar_row(&buf, "File reads").contains("1 line"));
}

#[test]
fn f7_shift_click_on_a_message_row_sets_the_other_bulk_kinds() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "File reads");
    let buf = draw(&mut agent);
    let before = [
        MessageKind::UserPrompt,
        MessageKind::AgentMessage,
        MessageKind::Read,
    ]
    .into_iter()
    .map(|kind| (kind, agent.scrollback.display_defaults().shown(kind)))
    .collect::<Vec<_>>();
    assert!(matches!(
        click_row_with(&mut agent, &buf, "File reads", KeyModifiers::SHIFT),
        InputOutcome::Changed
    ));
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::Read {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Expanded,
            "{kind:?}"
        );
    }
    assert!(
        never_bulk_toggled_stay(&agent, &before),
        "user, assistant and the clicked kind stay untouched"
    );
    let buf = draw(&mut agent);
    let _ = click_row_with(&mut agent, &buf, "File reads", KeyModifiers::SHIFT);
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::Read {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Collapsed,
            "{kind:?}"
        );
    }
    assert!(never_bulk_toggled_stay(&agent, &before));
}

#[test]
fn f7_plus_and_minus_open_and_close_all_bulk_kinds_and_say_when_nothing_changes() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let _ = press(&mut agent, KeyCode::End);
    let before = never_bulk_toggled_snapshot(&agent);

    let _ = press(&mut agent, KeyCode::Char('+'));
    assert!(every_bulk_kind_shows(&agent, DisplayForm::Expanded));
    assert!(
        never_bulk_toggled_stay(&agent, &before),
        "user and assistant rows are never bulk-toggled"
    );
    let _ = press(&mut agent, KeyCode::Char('+'));
    assert_eq!(toast(&agent), "All message kinds already show full");

    let _ = press(&mut agent, KeyCode::Char('-'));
    assert!(every_bulk_kind_shows(&agent, DisplayForm::Collapsed));
    let _ = press(&mut agent, KeyCode::Char('-'));
    assert_eq!(toast(&agent), "All message kinds already show one line");
}

#[test]
fn f7_shift_enter_on_a_title_folds_the_other_sections_and_reverts() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "Context");
    let kinds_before: Vec<_> = MessageKind::settings_kinds()
        .iter()
        .map(|&kind| agent.scrollback.display_defaults().shown(kind))
        .collect();

    let _ = press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    // The cursor is on the Context title: it stays open, the other sections fold.
    assert!(disclosure(&agent).is_expanded(SectionId::Context));
    assert!(!disclosure(&agent).is_expanded(SectionId::Display));
    let buf = draw(&mut agent);
    assert!(!screen(&buf).contains("File reads"));

    let _ = press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert!(disclosure(&agent).is_expanded(SectionId::Context));
    assert!(disclosure(&agent).is_expanded(SectionId::Display));
    let kinds_after: Vec<_> = MessageKind::settings_kinds()
        .iter()
        .map(|&kind| agent.scrollback.display_defaults().shown(kind))
        .collect();
    assert_eq!(
        kinds_before, kinds_after,
        "a title gesture leaves the display forms alone"
    );
}

#[test]
fn f7_ctrl_enter_leaves_the_cursor_row_alone_and_star_sweeps_everything() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "Memory captures");
    let before = never_bulk_toggled_snapshot(&agent);

    let _ = press_with(&mut agent, KeyCode::Enter, KeyModifiers::CONTROL);
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::MemoryCapture {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Expanded,
            "{kind:?}"
        );
    }
    assert!(
        never_bulk_toggled_stay(&agent, &before),
        "the cursor kind and user/assistant rows keep their form"
    );
    assert!(
        disclosure(&agent).is_expanded(SectionId::Context),
        "a collapsed kind makes the combined scope expand"
    );
    assert!(
        disclosure(&agent).is_expanded(SectionId::Display),
        "the cursor row's own section stays open"
    );

    let _ = press_with(&mut agent, KeyCode::Enter, KeyModifiers::CONTROL);
    for &kind in MessageKind::bulk_kinds() {
        if kind == MessageKind::MemoryCapture {
            continue;
        }
        assert_eq!(
            agent.scrollback.display_defaults().shown(kind),
            DisplayForm::Collapsed,
            "{kind:?}"
        );
    }
    assert!(never_bulk_toggled_stay(&agent, &before));
    assert!(!disclosure(&agent).is_expanded(SectionId::Context));
    assert!(disclosure(&agent).is_expanded(SectionId::Display));
    assert!(matches!(&agent.context_sidebar, ContextSidebar::Open(open) if open.cursor_label() == Some("Memory captures")), "the cursor row stayed visible");

    let _ = press(&mut agent, KeyCode::Char('*'));
    assert!(
        every_bulk_kind_shows(&agent, DisplayForm::Expanded),
        "* sweeps the whole scope, cursor kind included"
    );
    assert!(disclosure(&agent).is_expanded(SectionId::Context));
    assert!(disclosure(&agent).is_expanded(SectionId::Display));
    assert!(
        never_bulk_toggled_stay(&agent, &before),
        "user and assistant rows stay out of every bulk gesture"
    );
}

#[test]
fn f7_bulk_gestures_on_a_context_row_never_ask_the_shell_for_a_change() {
    let _theme = crate::theme::cache::pin_theme();
    let gestures: [(KeyCode, KeyModifiers); 5] = [
        (KeyCode::Enter, KeyModifiers::SHIFT),
        (KeyCode::Enter, KeyModifiers::CONTROL),
        (KeyCode::Char('+'), KeyModifiers::NONE),
        (KeyCode::Char('-'), KeyModifiers::NONE),
        (KeyCode::Char('*'), KeyModifiers::NONE),
    ];
    for (code, modifiers) in gestures {
        let mut agent = open_sidebar();
        assert_eq!(cursor(&agent), Some(SectionId::Context.row_line(1)));
        let outcome = press_with(&mut agent, code, modifiers);
        assert!(
            matches!(outcome, InputOutcome::Changed),
            "{code:?}+{modifiers:?} must not return an action"
        );
        assert!(
            !screen(&draw(&mut agent)).contains("Applying"),
            "{code:?}+{modifiers:?}"
        );
    }
    let mut agent = open_sidebar();
    let buf = draw(&mut agent);
    let outcome = click_row_with(&mut agent, &buf, "Core instructions", KeyModifiers::SHIFT);
    assert!(
        matches!(outcome, InputOutcome::Changed),
        "Shift+click on a switch"
    );
    assert!(
        disclosure(&agent).is_expanded(SectionId::Context),
        "the cursor row's section stays open"
    );
    assert!(
        disclosure(&agent).is_expanded(SectionId::Display),
        "selection-only Context clicks do not fold other sections"
    );
    let mut agent = open_sidebar();
    let buf = draw(&mut agent);
    let outcome = click_row_with(&mut agent, &buf, "Project rules", KeyModifiers::CONTROL);
    assert!(
        matches!(outcome, InputOutcome::Changed),
        "Ctrl+click on a switch"
    );
}

#[test]
fn f7_a_switch_that_cannot_be_sent_says_why() {
    let _theme = crate::theme::cache::pin_theme();

    let mut loading = test_fixtures::make_agent();
    let _ = press(&mut loading, KeyCode::F(7));
    let _ = draw(&mut loading);
    assert!(matches!(
        press(&mut loading, KeyCode::Char(' ')),
        InputOutcome::Changed
    ));
    let while_loading = toast(&loading);
    assert!(while_loading.contains("still loading"), "{while_loading}");

    let mut applying = open_sidebar();
    assert!(matches!(
        press(&mut applying, KeyCode::Char(' ')),
        InputOutcome::Action(Action::SetContextPolicy(_))
    ));
    assert!(matches!(
        press(&mut applying, KeyCode::Char(' ')),
        InputOutcome::Changed
    ));
    let while_applying = toast(&applying);
    assert!(
        while_applying.contains("still being applied"),
        "{while_applying}"
    );
    assert!(
        screen(&draw(&mut applying)).contains("Applying"),
        "the footer keeps the state"
    );

    let mut unavailable = test_fixtures::make_agent();
    let _ = press(&mut unavailable, KeyCode::F(7));
    unavailable.context_policy_answered(Err(crate::app::actions::ContextPolicyTransportError::Rpc(agent_client_protocol::Error::internal_error())));
    let _ = draw(&mut unavailable);
    assert!(matches!(
        press(&mut unavailable, KeyCode::Char(' ')),
        InputOutcome::Changed
    ));
    let when_unavailable = toast(&unavailable);
    assert!(
        when_unavailable.contains("could not be read"),
        "{when_unavailable}"
    );
    assert!(screen(&draw(&mut unavailable)).contains("Unavailable"));

    assert_ne!(while_loading, while_applying);
    assert_ne!(while_applying, when_unavailable);
}

#[test]
fn f7_footer_hint_follows_the_line_under_the_cursor() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let on_switch = screen(&draw(&mut agent));
    assert!(on_switch.contains("Space ON/OFF"));
    let _ = press(&mut agent, KeyCode::Home);
    assert!(screen(&draw(&mut agent)).contains("Space folds"));
    let _ = press(&mut agent, KeyCode::End);
    assert!(screen(&draw(&mut agent)).contains("Space flips"));
}

#[test]
fn f7_rows_are_coloured_by_their_meaning() {
    let _theme = crate::theme::cache::pin_theme();
    let theme = Theme::current();
    let mut agent = test_fixtures::make_agent();
    let _ = press(&mut agent, KeyCode::F(7));
    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(ContextPolicyReport {
        selection: selection(ContextScope::Session, 1),
        policy,
        usage: Vec::new(),
    }));
    let buf = draw(&mut agent);

    let on = sidebar_style(SidebarRowClass::SwitchOn, &theme);
    let off = sidebar_style(SidebarRowClass::SwitchOff, &theme);
    assert_ne!(
        on.glyph.fg, off.glyph.fg,
        "the palette must tell ON from OFF"
    );

    let (on_y, on_line) = row_of(&buf, "Core instructions");
    let on_x = column_of(&on_line, "Core instructions");
    assert_eq!(
        cell(&buf, on_x - 2, on_y).fg,
        on.glyph.fg.unwrap_or_default()
    );
    assert_eq!(
        cell(&buf, column_of(&on_line, "ON"), on_y).fg,
        on.value.fg.unwrap_or_default()
    );

    let (off_y, off_line) = row_of(&buf, "Reasoning");
    let off_x = column_of(&off_line, "Reasoning");
    assert_eq!(
        cell(&buf, off_x - 2, off_y).fg,
        off.glyph.fg.unwrap_or_default()
    );
    assert_eq!(
        cell(&buf, column_of(&off_line, "OFF"), off_y).fg,
        off.value.fg.unwrap_or_default()
    );
    assert_ne!(
        cell(&buf, on_x - 2, on_y).fg,
        cell(&buf, off_x - 2, off_y).fg,
        "an ON row and an OFF row render in different colours"
    );
    assert_ne!(
        cell(&buf, column_of(&on_line, "ON"), on_y).fg,
        cell(&buf, column_of(&off_line, "OFF"), off_y).fg
    );

    let title = sidebar_style(SidebarRowClass::SectionTitle, &theme);
    let (title_y, title_line) = row_of(&buf, "\u{25BE} Context");
    let title_x = column_of(&title_line, "\u{25BE} Context");
    assert_eq!(
        cell(&buf, title_x, title_y).fg,
        title.glyph.fg.unwrap_or_default()
    );
    assert!(
        cell(&buf, title_x + 2, title_y)
            .modifier
            .contains(Modifier::BOLD)
    );
    assert!(
        !cell(&buf, on_x, on_y).modifier.contains(Modifier::BOLD),
        "a row label is not bold like a title"
    );

    focus_row(&mut agent, "File reads");
    let buf = draw(&mut agent);
    let one_line = sidebar_style(SidebarRowClass::FormOneLine, &theme);
    let (form_y, form_line) = row_of(&buf, "File reads");
    assert_eq!(
        cell(&buf, column_of(&form_line, "1 line"), form_y).fg,
        one_line.value.fg.unwrap_or_default()
    );
}

#[test]
fn f7_a_flipped_display_form_changes_its_colour() {
    let _theme = crate::theme::cache::pin_theme();
    let theme = Theme::current();
    let mut agent = open_sidebar();
    focus_row(&mut agent, "File reads");
    let buf = draw(&mut agent);
    let (y, line) = row_of(&buf, "File reads");
    let one_line = cell(&buf, column_of(&line, "1 line"), y).fg;
    let _ = click_row(&mut agent, &buf, "File reads");
    let buf = draw(&mut agent);
    let (y, line) = row_of(&buf, "File reads");
    let full = cell(&buf, column_of(&line, "full"), y).fg;
    assert_eq!(
        full,
        sidebar_style(SidebarRowClass::FormExpanded, &theme)
            .value
            .fg
            .unwrap_or_default()
    );
    assert_ne!(one_line, full);
}

#[test]
fn f7_loading_rows_render_as_unavailable() {
    let _theme = crate::theme::cache::pin_theme();
    let theme = Theme::current();
    let mut agent = test_fixtures::make_agent();
    let _ = press(&mut agent, KeyCode::F(7));
    let buf = draw(&mut agent);
    let (y, line) = row_of(&buf, "Core instructions");
    let x = column_of(&line, "Core instructions");
    let unavailable = sidebar_style(SidebarRowClass::Unavailable, &theme);
    assert_eq!(
        cell(&buf, x, y).fg,
        unavailable.label.fg.unwrap_or_default()
    );
    assert!(cell(&buf, x, y).modifier.contains(Modifier::DIM));
}

#[test]
fn f7_disclosure_can_be_read_and_restored() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let mut stored = SectionDisclosure::default();
    stored.set_expanded(SectionId::Display, false);
    match &mut agent.context_sidebar {
        ContextSidebar::Open(open) => {
            open.apply_disclosure(stored);
            assert_eq!(open.disclosure(), stored);
        }
        ContextSidebar::Closed => panic!("F7 is open"),
    }
    let buf = draw(&mut agent);
    assert!(screen(&buf).contains("\u{25B8} Message display"));
    assert!(!screen(&buf).contains("File reads"));
    assert!(screen(&buf).contains("Core instructions"));

    let _ = press(&mut agent, KeyCode::End);
    assert_eq!(
        cursor(&agent),
        Some(SectionId::Display.title_line()),
        "the last visible line"
    );
}

#[test]
fn f7_a_restored_fold_moves_a_hidden_cursor_to_its_title() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let _ = press(&mut agent, KeyCode::End);
    let mut stored = SectionDisclosure::default();
    stored.set_expanded(SectionId::Display, false);
    if let ContextSidebar::Open(open) = &mut agent.context_sidebar {
        open.apply_disclosure(stored);
    }
    assert_eq!(cursor(&agent), Some(SectionId::Display.title_line()));
    let buf = draw(&mut agent);
    assert!(screen(&buf).contains("\u{25B8} Message display"));
}

#[test]
fn f7_titles_summarise_their_section_while_folded() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let categories = xai_grok_shell::sampling::all_categories().len();
    let kinds = MessageKind::settings_kinds().len() - 1;
    let buf = draw(&mut agent);
    assert!(
        sidebar_row(&buf, "\u{25BE} Context").contains(&format!("{categories}/{categories} ON"))
    );

    focus_row(&mut agent, "Context");
    let _ = press(&mut agent, KeyCode::Char(' '));
    let buf = draw(&mut agent);
    assert!(
        sidebar_row(&buf, "\u{25B8} Context").contains(&format!("{categories}/{categories} ON"))
    );

    let full = MessageKind::settings_kinds()
        .iter()
        .filter(|&&kind| kind != MessageKind::Injected && agent.scrollback.display_defaults().shown(kind) == DisplayForm::Expanded)
        .count();
    assert!(sidebar_row(&buf, "Message display").contains(&format!("{full}/{kinds} full")));
}

#[test]
fn f7_todos_are_permanent_independent_and_use_the_existing_pane_owner() {
    use xai_grok_shell::tools::{TodoItem, TodoStatus};
    let _theme = crate::theme::cache::pin_theme();
    let root = tempfile::tempdir().unwrap();
    let mut agent = test_fixtures::make_agent();
    agent.open_panel_settings_in(root.path());
    agent.todo.update_todos(vec![TodoItem {
        content: "Shared todo body".into(), status: TodoStatus::Pending,
        priority: Default::default(), meta: None,
    }]);
    let _ = press(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    let buf = draw(&mut agent);
    let rendered = screen(&buf);
    assert!(rendered.find("Dock").unwrap() < rendered.find("Todos").unwrap());
    assert!(rendered.find("Todos").unwrap() < rendered.find("Context").unwrap());
    assert!(rendered.contains("□ Shared todo body"));
    let _ = agent.handle_input(&Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)), &ActionRegistry::defaults());
    assert!(screen(&draw(&mut agent)).contains("\u{25BE} Todos"));
    let _ = agent.handle_input(&Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)), &ActionRegistry::defaults());
    assert!(screen(&draw(&mut agent)).contains("Shared todo body"));
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25BE} Todos");
    assert!(!screen(&draw(&mut agent)).contains("Shared todo body"));
    assert!(screen(&draw(&mut agent)).contains("\u{25B8} Todos"));
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25B8} Todos");
    assert!(screen(&draw(&mut agent)).contains("□ Shared todo body"));
    agent.todo.update_todos(vec![]);
    let rendered = screen(&draw(&mut agent));
    assert!(rendered.contains("Todos"));
    assert!(rendered.contains("No todo items."));
}

#[test]
fn dock_placement_render_input_matrix_keeps_f7_sections_disjoint() {
    use crate::views::dock::{self, DockPlacement};
    let _theme = crate::theme::cache::pin_theme();
    dock::set_enabled_for_test(false);
    let root = tempfile::tempdir().unwrap();
    let mut agent = test_fixtures::make_agent();
    agent.open_panel_settings_in(root.path());
    agent.tasks.overlay.visible = true;
    let _ = press(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    for placement in [DockPlacement::Off, DockPlacement::Top, DockPlacement::Bottom] {
        agent.set_dock_placement(placement);
        let buf = draw(&mut agent);
        let rendered = screen(&buf);
        assert!(rendered.contains("Dock") && rendered.contains("Todos"));
        assert!(sidebar_row(&buf, "Placement").contains(placement.label()));
        match placement {
            DockPlacement::Off => {
                assert!(agent.context_dock_geometry().is_none());
                assert_eq!(agent.pane_areas.dock.height, 0);
            }
            DockPlacement::Top => assert!(agent.pane_areas.dock.bottom() <= agent.pane_areas.scrollback.y),
            DockPlacement::Bottom => assert!(agent.pane_areas.dock.y >= agent.pane_areas.scrollback.bottom()),
        }
        if let Some((rect, offset)) = agent.context_dock_geometry() {
            assert_eq!(offset, 0);
            assert!(rect.y > row_of(&buf, "Placement").0);
            assert!(rect.bottom() <= row_of(&buf, "Todos").0);
        }
        agent.restore_visible_focus(super::VisibleFocus::Pane(super::AgentPane::Scrollback));
        let _ = press(&mut agent, KeyCode::Tab);
        assert_eq!(agent.visible_focus(), super::VisibleFocus::Context);
        let _ = press(&mut agent, KeyCode::Home);
        let ContextSidebar::Open(open) = &agent.context_sidebar else { panic!("F7 stays open"); };
        assert_eq!(open.cursor(), Some(SectionId::Dock.title_line()));
        assert!(sidebar_row(&draw(&mut agent), "Dock").contains("Dock"));
        let _ = press(&mut agent, KeyCode::Down);
        let ContextSidebar::Open(open) = &agent.context_sidebar else { panic!("F7 stays open"); };
        assert_eq!(open.cursor(), Some(SectionId::Dock.row_line(0)));
        let _ = click_row(&mut agent, &buf, "Placement");
        assert_eq!(dock::placement(), placement.next());
    }
}

#[test]
fn focused_off_context_row_preserves_its_neighbor_rows() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "Core instructions");
    let mut report = ready_report();
    report.policy.set(ContextCategory::CoreInstructions, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(report));
    let rendered = screen(&draw(&mut agent));
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("OFF"));
    assert!(rendered.contains("Project rules"));
    assert!(rendered.contains("Environment"));
    assert!(!rendered.contains("The agent loses its base behavior"));
}

#[path = "context_sidebar_persistence_tests.rs"]
mod persistence;

#[path = "todo_preparation_tests.rs"]
mod todo_preparation;

#[test]
fn external_completed_filter_edit_preserves_scrolled_f7_and_pending_local_reveal() {
    use crate::app::panel_settings::{CompletedItemsPane, Layer, PanelSettingsStore, Setting};
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = open_sidebar();
    agent.open_panel_settings_in(dir.path());
    let first = draw(&mut agent);
    let selected = cursor(&agent);
    for _ in 0..12 {
        let _ = agent.handle_input(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: first.area.right() - 10,
            row: 15,
            modifiers: KeyModifiers::NONE,
        }), &ActionRegistry::defaults());
    }
    let before = draw(&mut agent);
    assert!(!screen(&before).contains("Core instructions"), "the selected row is genuinely offscreen");
    assert_ne!(screen(&first), screen(&before), "the wheel moved the viewport without moving selection");
    assert_eq!(cursor(&agent), selected);
    let mut writer = PanelSettingsStore::open(dir.path(), None).store;
    writer.set(Layer::Global, Setting::ShowCompleted { pane: CompletedItemsPane::Tasks, show: true }).unwrap();
    assert!(agent.reload_global_panel_settings());
    let after = draw(&mut agent);
    assert_eq!(screen(&after), screen(&before), "an external h edit must not reveal the offscreen cursor");
    assert_eq!(cursor(&agent), selected);

    let _ = press(&mut agent, KeyCode::End);
    writer.set(Layer::Global, Setting::ShowCompleted { pane: CompletedItemsPane::Tasks, show: false }).unwrap();
    assert!(agent.reload_global_panel_settings());
    assert!(screen(&draw(&mut agent)).contains("Memory captures"), "a legitimate pending End reveal survives reload");
}

#[test]
fn external_group_fold_repairs_selected_leaf_to_its_semantic_parent() {
    use crate::app::panel_settings::{DisclosureState, Layer, PanelSettingsStore, Setting, SidebarSection};
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().unwrap();
    let mut agent = open_sidebar();
    agent.open_panel_settings_in(dir.path());
    focus_row(&mut agent, "File reads");
    let mut writer = PanelSettingsStore::open(dir.path(), None).store;
    writer.set(Layer::Global, Setting::Section { section: SidebarSection::DisplayFiles, state: DisclosureState::Collapsed }).unwrap();
    assert!(agent.reload_global_panel_settings());
    let selected_label = match &agent.context_sidebar {
        ContextSidebar::Open(open) => open.cursor_label(),
        ContextSidebar::Closed => None,
    };
    assert_eq!(selected_label, Some(crate::scrollback::block::MessageGroup::Files.label()));
    let _ = draw(&mut agent);
}
