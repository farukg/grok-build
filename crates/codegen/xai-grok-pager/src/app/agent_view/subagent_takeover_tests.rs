use crate::actions::ActionRegistry;
use crate::app::actions::Action;
use crate::app::agent_view::test_fixtures::{
    add_running_execute, ctrl, key, make_agent, parent_with_child,
};
use crate::app::agent_view::{AgentPane, AgentView, InputMode, ViewSurface};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::SubagentBlock;
use crate::scrollback::blocks::tool::{
    SentMessageDelivery, SentMessageInput, SentMessagePresentation, SentMessageTarget,
    SentMessageToolCallBlock, ToolCallBlock,
};
use crate::scrollback::render::ScratchBuffer;
use crate::scrollback::types::DisplayMode;
use crate::views::shortcuts_bar::PendingHint;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::sync::Arc;
use std::time::{Duration, Instant};
const CHILD_SID: &str = "child-sid";
fn subagent_row() -> RenderBlock {
    RenderBlock::Subagent(SubagentBlock::started(
        "child task",
        CHILD_SID,
        "general-purpose",
        None,
        None,
        None,
        false,
    ))
}
fn message_row(target: SentMessageTarget) -> RenderBlock {
    RenderBlock::ToolCall(ToolCallBlock::SentMessage(SentMessageToolCallBlock::new(
        SentMessagePresentation::Sent,
        Some(SentMessageInput {
            target,
            delivery: Some(SentMessageDelivery::Steer),
            text: "follow up".to_owned(),
        }),
    )))
}
fn named_child() -> SentMessageTarget {
    SentMessageTarget::Named {
        label: Arc::from("Subagent \u{201c}sleeper\u{201d}"),
        child_session_id: Arc::from(CHILD_SID),
    }
}
/// A parent with `block` as its only, selected scrollback entry; the child view exists only when `owns_child`.
fn parent_selecting(block: RenderBlock, owns_child: bool) -> AgentView {
    let mut parent = if owns_child {
        parent_with_child(CHILD_SID)
    } else {
        make_agent()
    };
    parent.scrollback.push_block(block);
    parent.scrollback.prepare_layout(80, 40);
    parent.scrollback.set_selected(Some(0));
    if parent.scrollback.toggle_group_expansion() {
        parent.scrollback.prepare_layout(80, 40);
    }
    parent
}
#[test]
fn viewer_keys_open_the_linked_child_from_subagent_and_message_rows() {
    let registry = ActionRegistry::defaults();
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let ctrl_f = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
    let unresolved = SentMessageTarget::Unresolved {
        subagent_id: CHILD_SID.to_owned(),
    };
    for (case, block, owns_child, opens) in [
        ("subagent row", subagent_row(), true, true),
        ("named message row", message_row(named_child()), true, true),
        (
            "named row, child view gone",
            message_row(named_child()),
            false,
            false,
        ),
        (
            "parent alias",
            message_row(SentMessageTarget::Parent),
            true,
            false,
        ),
        ("unresolved id", message_row(unresolved), true, false),
    ] {
        for chord in [&enter, &ctrl_f] {
            let mut parent = parent_selecting(block.clone(), owns_child);
            let outcome = parent.handle_scrollback_key(chord, &registry);
            assert_eq!(
                opens.then_some(CHILD_SID),
                parent.active_subagent.as_deref(),
                "{case}: {chord:?}"
            );
            assert!(
                matches!(
                    (opens, &outcome),
                    (true, InputOutcome::Changed)
                        | (false, InputOutcome::Action(Action::OpenBlockViewer))
                ),
                "{case}: {chord:?} gave {outcome:?}"
            );
        }
    }
}
#[test]
fn double_click_opens_the_linked_child_or_folds_the_message_row() {
    let now = Instant::now();
    let again = now + Duration::from_millis(10);
    for (case, block, owns_child, expected_child, expected_mode) in [
        (
            "message row",
            message_row(named_child()),
            true,
            Some(CHILD_SID),
            DisplayMode::Collapsed,
        ),
        (
            "message row, child view gone",
            message_row(named_child()),
            false,
            None,
            DisplayMode::Expanded,
        ),
        (
            "subagent row",
            subagent_row(),
            true,
            Some(CHILD_SID),
            DisplayMode::Collapsed,
        ),
        (
            "subagent row, child view gone",
            subagent_row(),
            false,
            None,
            DisplayMode::Collapsed,
        ),
    ] {
        let mut parent = parent_selecting(block, owns_child);
        (parent.last_click, _) = parent.handle_scrollback_click(now, 0, false);
        let _ = parent.handle_scrollback_click(again, 0, false);
        let mode = parent.scrollback.entry(0).map(|entry| entry.display_mode);
        assert_eq!(
            (expected_child, Some(expected_mode)),
            (parent.active_subagent.as_deref(), mode),
            "{case}"
        );
    }
}
#[test]
fn fullscreen_child_ctrl_b_never_demotes_child_or_parent() {
    let registry = ActionRegistry::defaults();
    let child_sid = "child-sid".to_string();
    let mut parent = make_agent();
    add_running_execute(&mut parent);
    assert!(
        parent
            .session
            .tracker
            .running_execute_tool_call_id()
            .is_some()
    );
    let mut child = make_agent();
    add_running_execute(&mut child);
    assert!(
        child
            .session
            .tracker
            .running_execute_tool_call_id()
            .is_some()
    );
    child.set_active_pane(AgentPane::Scrollback, true);
    parent.insert_test_child(child_sid.clone(), Box::new(child));
    assert_eq!(
        ViewSurface::ChildTakeover,
        parent
            .subagent_views
            .get(&child_sid)
            .unwrap_or_else(|| panic!("missing map entry"))
            .surface()
    );
    parent.open_subagent_fullscreen(child_sid.clone());
    let outcome = parent.handle_input(&ctrl('b'), &registry);
    assert!(matches!(outcome, InputOutcome::Changed));
    assert!(!matches!(
        outcome,
        InputOutcome::Action(Action::DemoteToBackground)
    ));
    assert_eq!(parent.active_subagent.as_deref(), Some(child_sid.as_str()));
    assert!(
        parent
            .session
            .tracker
            .running_execute_tool_call_id()
            .is_some()
    );
    assert!(
        parent
            .subagent_views
            .get(&child_sid)
            .unwrap_or_else(|| panic!("missing map entry"))
            .session
            .tracker
            .running_execute_tool_call_id()
            .is_some()
    );
    let child = &parent
        .subagent_views
        .get(&child_sid)
        .unwrap_or_else(|| panic!("missing map entry"));
    assert_eq!(ViewSurface::ChildTakeover, child.surface());
    assert!(
        !child
            .current_shortcut_hints(&registry)
            .iter()
            .any(|hint| hint.label == "send to bg")
    );
    assert!(child.hit_bg_button.rect.is_none());
}
/// Every glyph in `area`, row by row.
fn buffer_text(buf: &Buffer, area: Rect) -> String {
    (area.y..area.y + area.height)
        .map(|y| {
            (area.x..area.x + area.width)
                .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_owned()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
/// The parent paints none of its own chrome under a takeover, so a pending hint can only reach the buffer by being
/// forwarded into the child's shortcuts bar; the child's hidden composer yields no cursor to forward.
#[test]
fn takeover_draw_forwards_pending_hint_and_child_cursor() {
    let registry = ActionRegistry::defaults();
    let mut parent = parent_with_child("child");
    parent.open_subagent_fullscreen("child".to_owned());
    let area = Rect::new(0, 0, 80, 30);
    let mut buf = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    let (cursor, _) = parent.draw(
        area,
        &mut buf,
        &registry,
        &mut scratch,
        Some(PendingHint {
            shortcut: crate::key!(Esc),
            label: "clear pending hint",
        }),
        false,
        crate::app::agent_view::BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        crate::app::agent_view::AppRenderParams::default(),
    );
    assert_eq!(None, cursor);
    let text = buffer_text(&buf, area);
    assert!(text.contains("press again to clear pending hint"), "{text}");
    assert_eq!(
        0,
        parent
            .subagent_view("child")
            .expect("child view")
            .pane_areas
            .prompt
            .height
    );
}
/// A child view paints no `[Dashboard]` button, even when its own registry could dispatch `/dashboard`.
/// Inside the dashboard overlay the button stays as the way back.
#[test]
fn takeover_shows_dashboard_button_only_inside_the_overlay() {
    let registry = ActionRegistry::defaults();
    for (in_overlay, expect_button) in [(false, false), (true, true)] {
        let mut parent = parent_with_child("child");
        parent
            .subagent_view_mut("child")
            .expect("child view")
            .set_dashboard_visible(true);
        parent.open_subagent_fullscreen("child".to_owned());
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        let mut scratch = ScratchBuffer::new();
        let _ = parent.draw(
            area,
            &mut buf,
            &registry,
            &mut scratch,
            None,
            false,
            crate::app::agent_view::BannerSlotParams::none(),
            in_overlay,
            &mut Vec::new(),
            crate::app::agent_view::AppRenderParams::default(),
        );
        let text = buffer_text(&buf, area);
        assert_eq!(
            expect_button,
            text.contains("[Dashboard]"),
            "in_overlay={in_overlay}: {text}"
        );
        let child = parent.subagent_view("child").expect("child view");
        assert_eq!(
            expect_button,
            child.hit_dashboard.rect.is_some(),
            "in_overlay={in_overlay}"
        );
    }
}
/// Title row of the takeover frame for `child` after one draw.
fn takeover_title_row(parent: &mut AgentView, child: &str) -> String {
    let registry = ActionRegistry::defaults();
    parent.open_subagent_fullscreen(child.to_owned());
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    let mut scratch = ScratchBuffer::new();
    let _ = parent.draw(
        area,
        &mut buf,
        &registry,
        &mut scratch,
        None,
        false,
        crate::app::agent_view::BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        crate::app::agent_view::AppRenderParams::default(),
    );
    buffer_text(&buf, area)
        .lines()
        .find(|line| line.contains("gpt-5.6-sol"))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("takeover title row not rendered"))
}
#[test]
fn takeover_header_meta_includes_tokens() {
    for (tokens, expected) in [
        (Some(14_200), Some("gpt-5.6-sol \u{b7} 14K/200K")),
        (None, None),
    ] {
        let mut parent = parent_with_child("child");
        let mut info = crate::app::agent_view::test_fixtures::running_subagent_info("child");
        info.attempt.model = Some("gpt-5.6-sol".into());
        info.attempt.tokens_used = tokens;
        info.attempt.context_window_tokens = Some(200_000);
        parent.subagent_sessions.insert("child".to_owned(), info);
        let title = takeover_title_row(&mut parent, "child");
        match expected {
            Some(meta) => assert!(title.contains(meta), "{title}"),
            None => assert!(!title.contains("200K"), "{title}"),
        }
    }
}
/// A parent whose child, with a transcript, sits on bare scrollback under an open takeover. `vim_mode` is pinned on
/// both views: `AgentView::new` reads it from the user's config, which CI does not have.
fn open_takeover(child_sid: &str, vim_mode: bool) -> AgentView {
    let mut parent = parent_with_child(child_sid);
    let child = parent.subagent_view_mut(child_sid).expect("child view");
    add_running_execute(child);
    child.set_input_mode(InputMode::Vim);
    parent.set_vim_mode_recursive(vim_mode);
    parent.open_subagent_fullscreen(child_sid.to_owned());
    parent
}
/// Root-only chords never open a modal on the child under either binding set; the takeover stays up and `q` still
/// closes it. `?` is the palette's alt key and dies in the funnel on both; with vim bindings it is query text once the
/// child's search is open. `Shift+/` matches no chord, so without vim bindings it falls to type-to-focus, whose
/// `FocusPrompt` the composer guard refuses.
#[test]
fn child_root_only_chords_are_swallowed() {
    let registry = ActionRegistry::defaults();
    for vim_mode in [true, false] {
        let mut parent = open_takeover("child", vim_mode);
        for chord in [
            ctrl('p'),
            ctrl('m'),
            ctrl('r'),
            ctrl('o'),
            ctrl('l'),
            key(KeyCode::F(2)),
        ] {
            let outcome = parent.handle_input(&chord, &registry);
            assert!(
                matches!(outcome, InputOutcome::Changed | InputOutcome::Unchanged),
                "vim={vim_mode} {chord:?}: {outcome:?}"
            );
            let child = parent.subagent_view("child").expect("child view");
            assert!(
                child.active_modal.is_none(),
                "vim={vim_mode} {chord:?} opened a modal"
            );
            assert!(
                child.is_bare_scrollback(),
                "vim={vim_mode} {chord:?} left bare scrollback"
            );
        }
        let question = key(KeyCode::Char('?'));
        let shift_slash = Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::SHIFT));
        for printable in [&question, &shift_slash] {
            let outcome = parent.handle_input(printable, &registry);
            let expected = if vim_mode || printable == &question {
                matches!(outcome, InputOutcome::Changed | InputOutcome::Unchanged)
            } else {
                matches!(
                    outcome,
                    InputOutcome::ActionThenForward(Action::FocusPrompt)
                )
            };
            assert!(expected, "vim={vim_mode} {printable:?}: {outcome:?}");
            let child = parent.subagent_view("child").expect("child view");
            assert!(
                child.active_modal.is_none(),
                "vim={vim_mode} {printable:?} opened a modal"
            );
            assert!(
                child.is_bare_scrollback(),
                "vim={vim_mode} {printable:?} left bare scrollback"
            );
            assert_eq!(AgentPane::Scrollback, child.active_pane);
        }
        assert_eq!(Some("child"), parent.active_subagent.as_deref());
        if vim_mode {
            parent.handle_input(&key(KeyCode::Char('/')), &registry);
            parent.handle_input(&key(KeyCode::Char('?')), &registry);
            let child = parent.subagent_view("child").expect("child view");
            assert!(child.active_modal.is_none());
            assert_eq!(
                Some("?"),
                child.scrollback_search.as_ref().map(|s| s.query())
            );
            parent.handle_input(&key(KeyCode::Esc), &registry);
            assert!(
                parent
                    .subagent_view("child")
                    .expect("child view")
                    .scrollback_search
                    .is_none()
            );
            assert_eq!(Some("child"), parent.active_subagent.as_deref());
        }
        parent.handle_input(&key(KeyCode::Char('q')), &registry);
        assert_eq!(None, parent.active_subagent, "vim={vim_mode}");
    }
}
/// The child resolves a key pane-first exactly as a root does: with the mouse-capture toggle enabled, Ctrl+R on
/// scrollback is `ToggleMouseCapture` (allowed) rather than the `OpenSessions` chord it maps to under `AgentScreen`.
#[test]
fn child_ctrl_r_keeps_scrollback_precedence() {
    let mut parent = open_takeover("child", true);
    let outcome = parent.handle_input(&ctrl('r'), &ActionRegistry::defaults_with_config(true));
    assert!(
        matches!(outcome, InputOutcome::Action(Action::ToggleMouseCapture)),
        "{outcome:?}"
    );
    let outcome = parent.handle_input(&ctrl('r'), &ActionRegistry::defaults());
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    let child = parent.subagent_view("child").expect("child view");
    assert!(child.active_modal.is_none());
    assert!(child.is_bare_scrollback());
}
fn ctrl_alt(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(
        code,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ))
}
/// Registers `child_sid` under `parent_sid` (the root's own session is `"parent"`), started `age` ago.
fn add_child(root: &mut AgentView, parent_sid: &str, child_sid: &str, age: Duration) {
    let mut info = crate::app::agent_view::test_fixtures::running_subagent_info(child_sid);
    info.attempt.started_at = Instant::now() - age;
    root.subagent_sessions.insert(child_sid.to_owned(), info);
    root.insert_subagent_view(
        child_sid.to_owned(),
        Box::new(make_agent()),
        crate::app::agent_view::ChildLink::unaddressable(agent_client_protocol::SessionId::new(
            parent_sid,
        )),
    );
}
fn toast(view: &AgentView) -> Option<&str> {
    view.toast.as_ref().map(|(msg, _)| msg.as_str())
}
#[test]
fn ctrl_alt_down_from_root_opens_latest_started_child() {
    let registry = ActionRegistry::defaults();
    let mut root = make_agent();
    add_child(&mut root, "parent", "old", Duration::from_secs(30));
    add_child(&mut root, "parent", "newest", Duration::from_secs(1));
    add_child(&mut root, "parent", "middle", Duration::from_secs(10));
    add_child(&mut root, "newest", "grandchild-new", Duration::ZERO);
    add_child(
        &mut root,
        "newest",
        "grandchild-old",
        Duration::from_millis(500),
    );
    let outcome = root.handle_input(&ctrl_alt(KeyCode::Down), &registry);
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    assert_eq!(Some("newest"), root.active_subagent.as_deref());
    root.handle_input(&ctrl_alt(KeyCode::Down), &registry);
    assert_eq!(Some("grandchild-new"), root.active_subagent.as_deref());
    root.handle_input(&ctrl_alt(KeyCode::Up), &registry);
    assert_eq!(Some("newest"), root.active_subagent.as_deref());
    root.handle_input(&ctrl_alt(KeyCode::Up), &registry);
    assert_eq!(None, root.active_subagent, "Up from a root child leaves");
}

#[test]
fn ctrl_alt_down_on_leaf_shows_toast() {
    let registry = ActionRegistry::defaults();
    let mut root = make_agent();
    let outcome = root.handle_input(&ctrl_alt(KeyCode::Down), &registry);
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    assert_eq!(None, root.active_subagent);
    assert!(toast(&root).is_some(), "a root without children says so");
    add_child(&mut root, "parent", "leaf", Duration::ZERO);
    root.open_subagent_fullscreen("leaf".to_owned());
    root.handle_input(&ctrl_alt(KeyCode::Down), &registry);
    assert_eq!(Some("leaf"), root.active_subagent.as_deref());
    assert!(
        toast(root.subagent_view("leaf").expect("leaf view")).is_some(),
        "the hint shows on the view being looked at"
    );
}
/// A root whose laid-out scrollback shows `row` for its finished child `child_sid`; returns the row's screen line.
fn root_with_subagent_row(child_sid: &str, row: SubagentBlock) -> (AgentView, u16) {
    let mut root = make_agent();
    add_child(&mut root, "parent", child_sid, Duration::ZERO);
    root.subagent_sessions
        .get_mut(child_sid)
        .expect("info")
        .set_finished_for_test(true);
    root.scrollback.push_block(RenderBlock::user_prompt("go"));
    root.scrollback.push_block(RenderBlock::Subagent(row));
    let area = Rect::new(0, 0, 80, 40);
    root.scrollback.prepare_layout(area.width, area.height);
    root.pane_areas.scrollback = area;
    let row = root
        .scrollback
        .entry_screen_area(1, area)
        .map(|(rect, _, _)| rect.y)
        .expect("subagent row on screen");
    (root, row)
}
fn ctrl_alt_press(row: u16) -> Event {
    Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 10,
        row,
        modifiers: KeyModifiers::CONTROL | KeyModifiers::ALT,
    })
}
#[test]
fn ctrl_alt_click_opens_completed_child() {
    let registry = ActionRegistry::defaults();
    let (mut root, row) = root_with_subagent_row(
        "done",
        SubagentBlock::completed("child task", "done", Duration::from_secs(3)),
    );
    let info = root.subagent_sessions.get_mut("done").expect("info");
    info.attempt.status = Some("completed".into());
    info.transcript = crate::app::subagent::ChildTranscript::DiskBacked;
    root.subagent_view_mut("done")
        .expect("child view")
        .scrollback
        .push_block(RenderBlock::user_prompt("retained transcript"));
    let outcome = root.handle_input(&ctrl_alt_press(row), &registry);
    assert!(matches!(outcome, InputOutcome::Changed), "{outcome:?}");
    assert_eq!(Some("done"), root.active_subagent.as_deref());
}
/// A failed child whose view was evicted opens on the transcript replayed from disk.
#[test]
fn ctrl_alt_click_opens_failed_evicted_child_from_disk() {
    let registry = ActionRegistry::defaults();
    let child_sid = "failed-evicted";
    let home = tempfile::tempdir().expect("tempdir");
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).expect("session dir");
    std::fs::write(session_dir.join("summary.json"), "{}").expect("summary");
    let echo = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"from disk"}}}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), echo + "\n").expect("updates");
    crate::app::subagent::set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let (mut root, row) = root_with_subagent_row(
        child_sid,
        SubagentBlock::failed(
            "child task",
            child_sid,
            Duration::from_secs(3),
            Some("boom".to_owned()),
        ),
    );
    root.subagent_sessions
        .get_mut(child_sid)
        .expect("info")
        .attempt
        .status = Some("failed".into());
    assert!(
        root.subagent_view(child_sid)
            .expect("view")
            .scrollback
            .is_empty()
    );
    root.handle_input(&ctrl_alt_press(row), &registry);
    crate::app::subagent::set_replay_grok_home_for_tests(None);
    assert_eq!(Some(child_sid), root.active_subagent.as_deref());
    let child = root.subagent_view(child_sid).expect("child view");
    assert!(
        (0..child.scrollback.len()).any(|idx| child
            .scrollback
            .entry(idx)
            .is_some_and(|entry| matches!(entry.block, RenderBlock::UserPrompt(_)))),
        "the evicted transcript is replayed on open"
    );
}
