use super::tests::{child_views, painted, settle, source_text, view};
use super::*;
use crate::actions::ActionRegistry;
use crate::app::agent::BgTaskStatus;
use crate::app::agent_view::{AppRenderParams, BannerSlotParams};
use crate::app::app_view::{ActiveView, AppView};
use crate::render::draw::EscapeWriter;
use crate::scrollback::render::ScratchBuffer;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{buffer::Buffer, layout::Rect};
use xai_grok_shell::extensions::background_task::{BackgroundTaskRow, BackgroundTaskStatus};
use xai_grok_shell::extensions::notification::{SessionNotification, SessionUpdate};

const SID: &str = "paint-replay";
const STDOUT: &str = "  PAINT_STDOUT 界 with whitespace  \n\nlong unwrapped source words remain canonical across widths\n";

fn replay(app: &mut AppView, update: SessionUpdate) {
    let notification = SessionNotification {
        session_id: agent_client_protocol::SessionId::new(SID),
        update,
        meta: Some(crate::acp::meta::ReplayMetaStamp::replayed()),
    };
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: xai_grok_test_support::acp_fixtures::ext_notification(
                "x.ai/session/update",
                &notification,
            ),
            response_tx,
        }),
        app,
    );
}

fn replay_task(app: &mut AppView, id: &str, output: &str) {
    replay(
        app,
        SessionUpdate::TaskBackgrounded {
            tool_call_id: format!("call-{id}"),
            task_id: id.into(),
            command: "printf fixture".into(),
            cwd: "/tmp".into(),
            output_file: format!("/tmp/paint-{id}.log"),
            monitor_description: None,
            description: None,
            start_time: None,
        },
    );
    replay(
        app,
        SessionUpdate::TaskCompleted {
            task_snapshot: xai_grok_tools::types::TaskSnapshot {
                task_id: id.into(),
                command: "printf fixture".into(),
                display_command: None,
                cwd: "/tmp".into(),
                start_time: std::time::SystemTime::UNIX_EPOCH,
                end_time: Some(std::time::SystemTime::UNIX_EPOCH),
                output: output.into(),
                output_file: format!("/tmp/paint-{id}.log").into(),
                truncated: false,
                exit_code: Some(0),
                signal: None,
                completed: true,
                kind: Default::default(),
                block_waited: false,
                explicitly_killed: false,
                kill_result_delivered: false,
                owner_session_id: None,
                description: None,
                is_backgrounded: true,
                output_total_bytes: output.len(),
            },
            will_wake: false,
        },
    );
}

fn replayed_app() -> AppView {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(
        tx,
        crate::acp::model_state::ModelState::default(),
        Vec::new(),
        EscapeWriter::disconnected(),
    );
    let mut agent = view(SID);
    agent.session.loading_replay = true;
    let mut appearance = agent.scrollback.appearance().clone();
    appearance.show_timestamps = true;
    agent.scrollback.set_appearance(appearance);
    app.agents.insert(AgentId(0), agent);
    app.active_view = ActiveView::Agent(AgentId(0));
    for (id, output) in [("empty", ""), ("stdout", STDOUT)] {
        replay_task(&mut app, id, output);
    }
    replay(
        &mut app,
        SessionUpdate::BackgroundTasks {
            tasks: ["empty", "stdout"]
                .into_iter()
                .map(|id| BackgroundTaskRow {
                    task_id: id.into(),
                    command: "printf fixture".into(),
                    display_command: None,
                    description: None,
                    cwd: "/tmp".into(),
                    kind: Default::default(),
                    status: BackgroundTaskStatus::Completed,
                    started_at: "1970-01-01T00:00:00Z".into(),
                    ended_at: Some("1970-01-01T00:00:00Z".into()),
                    output_file: Some(format!("/tmp/paint-{id}.log")),
                    exit_code: Some(0),
                    signal: None,
                })
                .collect(),
            truncated: false,
        },
    );
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.loading_replay = false;
    assert_eq!(agent.scrollback.len(), 4);
    assert!(
        agent
            .session
            .bg_tasks
            .values()
            .all(|task| task.restored_from_replay && task.status == BgTaskStatus::Done)
    );
    assert!(
        agent
            .scrollback
            .iter_entries()
            .all(|(_, entry)| entry.display_mode == DisplayMode::Collapsed)
    );
    app
}

fn draw(views: &mut SessionViews, area: Rect) -> Buffer {
    let mut buffer = Buffer::empty(area);
    views.get_mut(&AgentId(0)).unwrap().draw(
        area,
        &mut buffer,
        &ActionRegistry::defaults(),
        &mut ScratchBuffer::new(),
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    let root = views.get(&AgentId(0)).unwrap();
    let mut inline_rows = 0;
    for geometry in &root.last_scrollback_selection_model.visible_blocks {
        let entry = root.scrollback.entry(geometry.entry_idx).unwrap();
        if !entry.block.is_bg_task() && !entry.block.is_subagent() {
            continue;
        }
        let reserved = if entry.display_mode == DisplayMode::Collapsed {
            10
        } else {
            0
        };
        assert_eq!(
            geometry.content_width,
            geometry.content_area.width.saturating_sub(reserved),
            "paint and selection must use the same timestamp gutter for entry {:?}",
            entry.id
        );
        assert_eq!(entry.inline_request_width(), Some(geometry.content_width));
        inline_rows += 1;
    }
    assert!(inline_rows > 0);
    buffer
}

fn forms(views: &mut SessionViews, empty: DisplayMode, stdout: DisplayMode) {
    let root = views.get_mut(&AgentId(0)).unwrap();
    let entries: Vec<_> = root.scrollback.iter_entries().map(|(id, _)| id).collect();
    for id in entries {
        let entry = root.scrollback.get_by_id_mut(id).unwrap();
        let RenderBlock::BgTask(task) = &entry.block else {
            panic!("expected replayed task row")
        };
        entry.set_display_mode(if task.task_id == "empty" {
            empty
        } else {
            stdout
        });
        root.scrollback.mark_height_dirty(id);
    }
}

#[test]
fn acp_replayed_terminal_stdout_uses_the_modal_emulator_in_chat() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = replayed_app();
    let raw = "\x1b[31mREPLAY_RED\x1b[0m\nobsolete\r\x1b[2KREPLAY_DONE";
    app.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = true;
    replay_task(&mut app, "ansi", raw);
    app.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = false;
    forms(&mut app.agents, DisplayMode::Collapsed, DisplayMode::Expanded);
    let buffer = settle(&mut app.agents, AgentId(0), 100);
    assert!(painted(&buffer).contains("REPLAY_RED"));
    assert!(painted(&buffer).contains("REPLAY_DONE"));
    assert!(!painted(&buffer).contains("obsolete"));
    assert!(!painted(&buffer).contains('\x1b'));
    let id = app.agents.get(&AgentId(0)).unwrap().scrollback.iter_entries()
        .find_map(|(id, entry)| {
            if let RenderBlock::BgTask(task) = &entry.block
                && task.task_id == "ansi"
            {
                return Some(id);
            }
            None
        }).unwrap();
    assert_eq!(source_text(&mut app.agents, id).as_deref(), Some(raw));
}

#[test]
fn replayed_completed_task_rows_have_one_width_for_measurement_paint_and_selection() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = replayed_app();
    for width in [120, 60, 121] {
        for (empty, stdout) in [
            (DisplayMode::Collapsed, DisplayMode::Collapsed),
            (DisplayMode::Expanded, DisplayMode::Expanded),
            (DisplayMode::Collapsed, DisplayMode::Expanded),
        ] {
            forms(&mut app.agents, empty, stdout);
            let area = Rect::new(0, 0, width, 60);
            app.agents.prepare_inline_output(AgentId(0));
            draw(&mut app.agents, area);
            app.agents.prepare_inline_output(AgentId(0));
            let buffer = draw(&mut app.agents, area);
            assert!(
                !app.agents.prepare_inline_output(AgentId(0)),
                "a settled draw must not reproject"
            );
            let root = app.agents.get(&AgentId(0)).unwrap();
            assert_eq!(root.last_scrollback_selection_model.visible_blocks.len(), 4);
            for (id, entry) in root.scrollback.iter_entries() {
                assert!(
                    !entry.inline_width_pending(),
                    "settled source width for {id:?}"
                );
            }
            if stdout == DisplayMode::Expanded {
                assert!(painted(&buffer).contains("PAINT_STDOUT"));
                if empty == DisplayMode::Expanded {
                    assert!(painted(&buffer).contains("No output"));
                }
                let ranges = &root.last_scrollback_selection_model.ranges;
                let output: Vec<_> = ranges
                    .iter()
                    .filter(|range| {
                        range
                            .lines
                            .iter()
                            .any(|line| line.text.contains("PAINT_STDOUT"))
                    })
                    .collect();
                assert_eq!(output.len(), 2);
                assert_ne!(output[0].range_id, output[1].range_id);
                for range in output {
                    let mut selected = String::new();
                    for (index, line) in range.lines.iter().enumerate() {
                        if index > 0 {
                            selected.push_str(line.joiner_to_previous.as_deref().unwrap_or("\n"));
                        }
                        selected.push_str(&line.text);
                    }
                    assert_eq!(selected, STDOUT);
                }
            }
            let ids: Vec<_> = root
                .scrollback
                .iter_entries()
                .filter_map(|(id, entry)| {
                    matches!(&entry.block, RenderBlock::BgTask(task) if task.task_id == "stdout")
                        .then_some(id)
                })
                .collect();
            for id in ids {
                assert_eq!(source_text(&mut app.agents, id).as_deref(), Some(STDOUT));
            }
            assert!(matches!(app.active_view, ActiveView::Agent(AgentId(0))));
        }
    }
}

#[test]
fn replayed_task_width_stays_settled_across_sidebar_resize_and_scroll_edges() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = replayed_app();
    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .session
        .loading_replay = true;
    for index in 0..6 {
        replay_task(&mut app, &format!("edge-{index}"), "");
    }
    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .session
        .loading_replay = false;
    for key in [
        None,
        Some(KeyCode::F(7)),
        Some(KeyCode::F(6)),
        Some(KeyCode::F(7)),
    ] {
        if let Some(code) = key {
            app.agents.get_mut(&AgentId(0)).unwrap().handle_input(
                &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                &ActionRegistry::defaults(),
            );
        }
        for (width, height) in [(160, 40), (80, 16), (81, 16), (160, 40)] {
            for at_bottom in [false, true] {
                let root = app.agents.get_mut(&AgentId(0)).unwrap();
                if at_bottom {
                    root.scrollback.goto_bottom();
                } else {
                    root.scrollback.goto_top();
                }
                let area = Rect::new(0, 0, width, height);
                app.agents.prepare_inline_output(AgentId(0));
                draw(&mut app.agents, area);
                app.agents.prepare_inline_output(AgentId(0));
                draw(&mut app.agents, area);
                assert!(!app.agents.prepare_inline_output(AgentId(0)));
                let (_, viewport, total) = app
                    .agents
                    .get(&AgentId(0))
                    .unwrap()
                    .scrollback
                    .scroll_info();
                assert_eq!(total > usize::from(viewport), height == 16);
                draw(&mut app.agents, area);
                assert!(!app.agents.prepare_inline_output(AgentId(0)));
            }
        }
    }
}

#[test]
fn deferred_child_windows_keep_collapsed_task_width_and_do_not_load_inactive_children() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = replayed_app();
    let mut child = app.agents.remove_tree(AgentId(0)).pop().unwrap();
    for index in 0..160 {
        child
            .scrollback
            .push_block(RenderBlock::agent_message(format!(
                "DEFERRED_CHILD_{index:03}"
            )));
    }
    let (mut views, root_id) = child_views(SID, child);
    let inactive_id = {
        let root = views.get_mut(&AgentId(0)).unwrap();
        let mut appearance = root.scrollback.appearance().clone();
        appearance.show_timestamps = true;
        root.scrollback.set_appearance(appearance);
        let id = root.scrollback.push_block(RenderBlock::Subagent(
            crate::scrollback::blocks::SubagentBlock::completed(
                "inactive",
                "paint-inactive",
                std::time::Duration::ZERO,
            ),
        ));
        root.scrollback
            .get_by_id_mut(id)
            .unwrap()
            .set_display_mode(DisplayMode::Collapsed);
        id
    };
    let mut inactive = view("paint-inactive");
    inactive
        .scrollback
        .push_block(RenderBlock::agent_message("INACTIVE_BODY"));
    crate::app::session_views::test_support::link_child(
        &mut views,
        AgentId(0),
        AgentId(2),
        inactive,
        std::time::Instant::now(),
    );
    let reads = crate::app::subagent::test_support::transcript_reads();
    for width in [120, 80, 81, 120] {
        views.get_mut(&AgentId(0)).unwrap().scrollback.goto_top();
        settle(&mut views, AgentId(0), width);
        assert!(!views.prepare_inline_output(AgentId(0)));
        let child = views.get(&AgentId(1)).unwrap();
        assert!(
            child
                .scrollback
                .iter_entries()
                .filter(|(_, entry)| entry.has_cached_output())
                .count()
                < 80
        );
        let root = views.get(&AgentId(0)).unwrap();
        let tree = root.scrollback.get_by_id(root_id).unwrap().cached_tree();
        let mut task_parts = 0;
        tree.visit_native(0..tree.len(), 0, &mut |_, part| {
            if part.entry.block.is_bg_task() {
                assert!(!part.entry.inline_width_pending());
                task_parts += 1;
            }
        });
        assert_eq!(task_parts, 4);
        views.get_mut(&AgentId(0)).unwrap().scrollback.goto_bottom();
        let buffer = settle(&mut views, AgentId(0), width);
        assert!(painted(&buffer).contains("DEFERRED_CHILD_159"));
        assert!(!views.prepare_inline_output(AgentId(0)));
        assert_eq!(
            views
                .get(&AgentId(0))
                .unwrap()
                .scrollback
                .get_by_id(inactive_id)
                .unwrap()
                .display_mode,
            DisplayMode::Collapsed
        );
    }
    assert_eq!(
        crate::app::subagent::test_support::transcript_reads(),
        reads
    );
    let inactive = views.get(&AgentId(2)).unwrap();
    assert!(matches!(
        inactive.inline_sources.replay,
        InlineReplay::Unrequested
    ));
    assert!(
        inactive
            .scrollback
            .iter_entries()
            .all(|(_, entry)| !entry.has_cached_output())
    );
}
