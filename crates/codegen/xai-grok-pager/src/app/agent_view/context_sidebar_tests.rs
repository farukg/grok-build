//! F7: the sidebar shows the shell's policy, asks for changes and never shows an unanswered one.
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
use xai_grok_shell::sampling::{CategoryTokens, ContextCategory, ContextPolicy, ContextSwitch};

fn draw(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, 160, 40);
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

    assert!(matches!(
        press(&mut agent, KeyCode::F(7)),
        InputOutcome::Action(Action::FetchContextPolicy)
    ));
    assert!(screen(&draw(&mut agent)).contains("Loading"));

    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::Reasoning, ContextSwitch::Excluded);
    agent.context_policy_answered(Ok(ContextPolicyReport {
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
    assert!(
        !requested.includes(first),
        "the cursor starts on the first row, which was ON"
    );
    assert!(
        !requested.includes(ContextCategory::Reasoning),
        "the rest of the policy is unchanged"
    );
    assert!(
        screen(&draw(&mut agent)).contains("Applying"),
        "the row keeps the shell's last answer until it answers again"
    );

    agent.context_policy_answered(Ok(ContextPolicyReport {
        policy: requested,
        usage: Vec::new(),
    }));
    assert!(!screen(&draw(&mut agent)).contains("Applying"));
    assert!(matches!(
        press(&mut agent, KeyCode::Esc),
        InputOutcome::Changed
    ));
    assert!(!screen(&draw(&mut agent)).contains("Context sent to the model"));
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
    }
}

/// F7 open, the shell answered, first frame drawn: the cursor sits on the first context switch.
fn open_sidebar() -> AgentView {
    let mut agent = test_fixtures::make_agent();
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
    assert_eq!(cursor(&agent), Some(context.row_line(0)));

    let _ = press(&mut agent, KeyCode::Left);
    assert_eq!(
        cursor(&agent),
        Some(context.title_line()),
        "Left from a row selects its title"
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
    assert_eq!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::Char('l'));
    assert_eq!(cursor(&agent), Some(context.row_line(0)));
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
    let last = MessageKind::settings_kinds().len() - 1;

    let _ = press(&mut agent, KeyCode::End);
    assert_eq!(cursor(&agent), Some(display.row_line(last)));
    let _ = press(&mut agent, KeyCode::Home);
    assert_eq!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::PageDown);
    assert_ne!(cursor(&agent), Some(context.title_line()));
    let _ = press(&mut agent, KeyCode::PageUp);
    assert_eq!(cursor(&agent), Some(context.title_line()));

    let _ = press(&mut agent, KeyCode::Enter);
    assert!(!disclosure(&agent).is_expanded(context));
    let _ = press(&mut agent, KeyCode::Down);
    assert_eq!(cursor(&agent), Some(display.title_line()));
    let _ = press(&mut agent, KeyCode::Home);
    let _ = press(&mut agent, KeyCode::Char('j'));
    assert_eq!(
        cursor(&agent),
        Some(display.title_line()),
        "j also skips the folded rows"
    );
}

#[test]
fn f7_enter_and_space_fold_a_title_and_a_click_unfolds_it() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    let _ = press(&mut agent, KeyCode::Home);
    assert!(sidebar_row(&draw(&mut agent), "Core instructions").contains("Core instructions"));

    assert!(matches!(
        press(&mut agent, KeyCode::Enter),
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
    let buf = draw(&mut agent);
    assert!(
        screen(&buf).contains("Core instructions"),
        "a click on a folded title unfolds it"
    );
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
    let _ = press(&mut agent, KeyCode::Left);
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
    let _ = press(&mut agent, KeyCode::End);
    // The cursor sits on the last display row (MemoryCapture): the gesture leaves it,
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
    let buf = draw(&mut agent);
    let before = never_bulk_toggled_snapshot(&agent);
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
    let _ = press(&mut agent, KeyCode::Home);
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
    let _ = press(&mut agent, KeyCode::End);
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
        !disclosure(&agent).is_expanded(SectionId::Context),
        "the other section folded"
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
    assert!(disclosure(&agent).is_expanded(SectionId::Context));
    assert!(disclosure(&agent).is_expanded(SectionId::Display));
    assert_eq!(
        cursor(&agent),
        Some(SectionId::Display.row_line(MessageKind::settings_kinds().len() - 1)),
        "the cursor row stayed visible"
    );

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
        assert_eq!(cursor(&agent), Some(SectionId::Context.row_line(0)));
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
        !disclosure(&agent).is_expanded(SectionId::Context),
        "it only folded the sections"
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
    unavailable.context_policy_answered(Err("no policy".to_string()));
    let _ = draw(&mut unavailable);
    assert!(matches!(
        press(&mut unavailable, KeyCode::Enter),
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
    let kinds = MessageKind::settings_kinds().len();
    let buf = draw(&mut agent);
    assert!(
        sidebar_row(&buf, "\u{25BE} Context").contains(&format!("{categories}/{categories} ON"))
    );

    let _ = press(&mut agent, KeyCode::Home);
    let _ = press(&mut agent, KeyCode::Enter);
    let buf = draw(&mut agent);
    assert!(
        sidebar_row(&buf, "\u{25B8} Context").contains(&format!("{categories}/{categories} ON"))
    );

    let full = MessageKind::settings_kinds()
        .iter()
        .filter(|&&kind| agent.scrollback.display_defaults().shown(kind) == DisplayForm::Expanded)
        .count();
    assert!(sidebar_row(&buf, "Message display").contains(&format!("{full}/{kinds} full")));
}

#[path = "context_sidebar_persistence_tests.rs"]
mod persistence;
