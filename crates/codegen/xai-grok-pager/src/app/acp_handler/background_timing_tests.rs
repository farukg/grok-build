use super::super::tests::{
    make_app_with_agent, make_replayed_task_backgrounded_notif, make_task_backgrounded_notif,
    make_task_completed_notif, test_agent,
};
use super::*;
use crate::scrollback::block::BlockContent;
use crate::scrollback::blocks::BgTaskKind;
use crate::scrollback::types::{BlockContext, DisplayMode, line_plain_text};
use std::time::{Duration, SystemTime};
use xai_grok_test_support::acp_fixtures;

fn completed_snapshot(exit_code: i32) -> xai_grok_tools::types::TaskSnapshot {
    let notification = make_task_completed_notif("timing", "task", "sleep 2", Some(exit_code));
    let notification: SessionNotification =
        serde_json::from_str(notification.params.get()).unwrap();
    let XaiSessionUpdate::TaskCompleted {
        mut task_snapshot, ..
    } = notification.update
    else {
        panic!("expected completion fixture");
    };
    let delivery = SystemTime::now();
    task_snapshot.start_time = delivery - Duration::from_secs(62);
    task_snapshot.end_time = Some(delivery - Duration::from_secs(60));
    task_snapshot
}

fn deliver(app: &mut AppView, update: XaiSessionUpdate, replay: bool) {
    let notification = SessionNotification {
        session_id: acp::SessionId::new("timing"),
        update,
        meta: replay.then(crate::acp::meta::ReplayMetaStamp::replayed),
    };
    handle_ext_notification(
        &acp_fixtures::ext_notification("x.ai/session/update", &notification),
        app,
    );
}

fn task(app: &AppView) -> &BgTaskState {
    test_agent(app, AgentId(0))
        .session
        .bg_tasks
        .get("task")
        .expect("task")
}

fn completion_elapsed(app: &AppView) -> Option<Duration> {
    let agent = test_agent(app, AgentId(0));
    (0..agent.scrollback.len())
        .rev()
        .filter_map(|index| agent.scrollback.get(index))
        .find_map(|entry| {
            let RenderBlock::BgTask(block) = &entry.block else {
                return None;
            };
            match block.kind {
                BgTaskKind::Completed { elapsed } | BgTaskKind::Failed { elapsed, .. } => {
                    Some(elapsed)
                }
                BgTaskKind::Started => None,
            }
        })
        .expect("completion block")
}

fn history_text(app: &AppView) -> String {
    let context = BlockContext {
        mode: DisplayMode::Collapsed,
        width: 100,
        is_running: false,
        raw: false,
        max_lines: None,
        appearance: Default::default(),
        is_selected: false,
        cwd: None,
    };
    let agent = test_agent(app, AgentId(0));
    (0..agent.scrollback.len())
        .filter_map(|index| agent.scrollback.get(index))
        .flat_map(|entry| entry.block.output(&context).lines)
        .map(|line| line_plain_text(&line.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn pane_text(app: &AppView) -> String {
    let agent = test_agent(app, AgentId(0));
    let mut pane = crate::views::tasks_pane::TasksPane::new();
    pane.set_show_done(true);
    pane.overlay.show();
    pane.sync(
        &agent.session.bg_tasks,
        &Default::default(),
        &Default::default(),
        &[],
    );
    let area = ratatui::layout::Rect::new(0, 0, 100, 8);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    pane.render(
        area,
        &mut buffer,
        false,
        &Default::default(),
        &agent.session.bg_tasks,
        &Default::default(),
        &Default::default(),
    );
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .filter_map(|x| buffer.cell((x, y)).map(|cell| cell.symbol()))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn delivered_sixty_seconds_late_known_and_unknown_completions_use_the_backend_interval() {
    for known in [false, true] {
        for replay in [false, true] {
            for exit_code in [0, 1] {
                let mut app = make_app_with_agent("timing");
                if known {
                    let backgrounded =
                        make_task_backgrounded_notif("timing", "call", "task", "sleep 2");
                    assert!(handle_ext_notification(&backgrounded, &mut app));
                    let task = task(&app);
                    assert_eq!(task.elapsed(), None);
                    assert!(pane_text(&app).contains("unknown"));
                }
                let snapshot = completed_snapshot(exit_code);
                let start = snapshot.start_time;
                let end = snapshot.end_time;
                deliver(
                    &mut app,
                    XaiSessionUpdate::TaskCompleted {
                        task_snapshot: snapshot,
                        will_wake: false,
                    },
                    replay,
                );
                let task = task(&app);
                assert_eq!(task.start_time, Some(start));
                assert_eq!(task.end_time, end);
                assert_eq!(task.elapsed(), Some(Duration::from_secs(2)));
                assert_eq!(completion_elapsed(&app), Some(Duration::from_secs(2)));
                assert!(pane_text(&app).contains("2.0s"));
                assert!(history_text(&app).contains("in 2.0s:"));
                assert!(!history_text(&app).contains("62s"));
            }
        }
    }
}

fn running_row() -> BackgroundTaskRow {
    BackgroundTaskRow {
        task_id: "task".into(),
        command: "sleep 2".into(),
        display_command: None,
        description: None,
        cwd: "/tmp".into(),
        kind: Default::default(),
        status: WireBackgroundTaskStatus::Running,
        started_at: "2026-01-01T00:00:00Z".into(),
        ended_at: None,
        output_file: None,
        exit_code: None,
        signal: None,
    }
}

#[test]
fn running_snapshot_start_survives_late_backgrounding_metadata() {
    for replay in [false, true] {
        let mut app = make_app_with_agent("timing");
        deliver(
            &mut app,
            XaiSessionUpdate::BackgroundTasks {
                tasks: vec![running_row()],
                truncated: false,
            },
            replay,
        );
        let backgrounded = make_task_backgrounded_notif("timing", "call", "task", "sleep 2");
        assert!(handle_ext_notification(&backgrounded, &mut app));
        let running = task(&app);
        assert_eq!(
            running.start_time,
            snapshot_system_time("2026-01-01T00:00:00Z")
        );
        assert_eq!(running.status, BgTaskStatus::Running);
        assert_eq!(running.tool_call_id, "call");
        deliver(
            &mut app,
            XaiSessionUpdate::TaskCompleted {
                task_snapshot: completed_snapshot(0),
                will_wake: false,
            },
            replay,
        );
        assert_eq!(task(&app).elapsed(), Some(Duration::from_secs(2)));
        assert_eq!(completion_elapsed(&app), Some(Duration::from_secs(2)));
        assert!(pane_text(&app).contains("2.0s"));
        assert!(history_text(&app).contains("in 2.0s:"));
    }
}

#[test]
fn terminal_timing_survives_late_live_replay_and_running_snapshot_delivery() {
    for replay in [false, true] {
        let mut app = make_app_with_agent("timing");
        let snapshot = completed_snapshot(0);
        let start = snapshot.start_time;
        let end = snapshot.end_time;
        deliver(
            &mut app,
            XaiSessionUpdate::TaskCompleted {
                task_snapshot: snapshot,
                will_wake: false,
            },
            replay,
        );
        let before = test_agent(&app, AgentId(0)).scrollback.len();
        let late = if replay {
            make_replayed_task_backgrounded_notif("timing", "call", "task", "sleep 2")
        } else {
            make_task_backgrounded_notif("timing", "call", "task", "sleep 2")
        };
        assert!(handle_ext_notification(&late, &mut app));
        deliver(
            &mut app,
            XaiSessionUpdate::BackgroundTasks {
                tasks: vec![running_row()],
                truncated: false,
            },
            replay,
        );
        let agent = test_agent(&app, AgentId(0));
        let task = agent.session.bg_tasks.get("task").expect("task");
        assert_eq!(task.status, BgTaskStatus::Done);
        assert_eq!((task.start_time, task.end_time), (Some(start), end));
        assert_eq!(task.elapsed(), Some(Duration::from_secs(2)));
        assert_eq!(agent.scrollback.len(), before);
        assert!(!agent.scrollback.needs_animation());
    }
}

#[test]
fn terminal_without_backend_end_is_unknown_in_pane_and_history() {
    for known in [false, true] {
        let mut app = make_app_with_agent("timing");
        if known {
            assert!(handle_ext_notification(
                &make_task_backgrounded_notif("timing", "call", "task", "sleep 2"),
                &mut app
            ));
        }
        let mut snapshot = completed_snapshot(0);
        snapshot.end_time = None;
        deliver(
            &mut app,
            XaiSessionUpdate::TaskCompleted {
                task_snapshot: snapshot,
                will_wake: false,
            },
            false,
        );
        let task = task(&app);
        assert_eq!(task.status, BgTaskStatus::Done);
        assert_eq!(task.elapsed(), None);
        assert_eq!(completion_elapsed(&app), None);
        assert!(pane_text(&app).contains("unknown"));
        assert!(history_text(&app).contains("in unknown:"));
        assert!(!history_text(&app).contains("in 0.0s:"));
    }
}

#[test]
fn reversed_backend_interval_is_unknown_instead_of_zero() {
    let mut app = make_app_with_agent("timing");
    let mut snapshot = completed_snapshot(0);
    snapshot.end_time = Some(snapshot.start_time - Duration::from_secs(1));
    deliver(
        &mut app,
        XaiSessionUpdate::TaskCompleted {
            task_snapshot: snapshot,
            will_wake: false,
        },
        false,
    );
    assert_eq!(task(&app).elapsed(), None);
    assert_eq!(completion_elapsed(&app), None);
    assert!(pane_text(&app).contains("unknown"));
    assert!(history_text(&app).contains("in unknown:"));
}

#[test]
fn cold_reconciliation_observation_stamps_stay_quiet_unknown_and_terminal() {
    let mut app = make_app_with_agent("timing");
    assert!(handle_ext_notification(
        &make_replayed_task_backgrounded_notif("timing", "call", "task", "sleep 2"),
        &mut app
    ));
    let mut snapshot = completed_snapshot(1);
    snapshot.start_time = SystemTime::now();
    snapshot.end_time = Some(SystemTime::now());
    snapshot.signal = Some(SESSION_RESTART_SIGNAL.into());
    deliver(
        &mut app,
        XaiSessionUpdate::TaskCompleted {
            task_snapshot: snapshot,
            will_wake: false,
        },
        false,
    );
    assert!(handle_ext_notification(
        &make_task_backgrounded_notif("timing", "call", "task", "sleep 2"),
        &mut app
    ));
    let agent = test_agent(&app, AgentId(0));
    assert_eq!(task(&app).status, BgTaskStatus::Failed);
    assert_eq!(task(&app).elapsed(), None);
    assert_eq!(agent.scrollback.len(), 1);
    assert!(!agent.scrollback.needs_animation());
    assert!(pane_text(&app).contains("unknown"));
}

#[test]
fn malformed_registry_start_is_unknown_until_an_actual_snapshot_arrives() {
    let mut app = make_app_with_agent("timing");
    let mut row = running_row();
    row.started_at = "legacy-unknown".into();
    deliver(
        &mut app,
        XaiSessionUpdate::BackgroundTasks {
            tasks: vec![row],
            truncated: false,
        },
        false,
    );
    assert_eq!(task(&app).elapsed(), None);
    assert!(pane_text(&app).contains("unknown"));
    deliver(
        &mut app,
        XaiSessionUpdate::TaskCompleted {
            task_snapshot: completed_snapshot(0),
            will_wake: false,
        },
        false,
    );
    assert_eq!(task(&app).elapsed(), Some(Duration::from_secs(2)));
    assert!(history_text(&app).contains("in 2.0s:"));
}
