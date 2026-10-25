use super::*;
use crate::app::actions::{Action, TaskResult};
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentPane, VisibleFocus};
use crate::views::dock::{self, DockPlacement};

#[derive(Clone, Copy, Debug)]
enum DockContents {
    Empty,
    Active,
}

impl DockContents {
    fn count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Active => 1,
        }
    }
}

fn populate(agent: &mut AgentView, name: &str, contents: DockContents) {
    match contents {
        DockContents::Empty => {}
        DockContents::Active => {
            agent.session.bg_tasks.insert(format!("task-{name}"), crate::app::agent_test_fixtures::running_bg_task(&format!("task-{name}"), false));
            agent.subagent_sessions.insert(format!("child-{name}"), crate::app::agent_view::test_fixtures::running_subagent_info(&format!("child-{name}")));
        }
    }
    agent.todo.update_todos(vec![xai_grok_shell::tools::TodoItem {
        content: format!("todo-{name}"), priority: Default::default(),
        status: xai_grok_shell::tools::TodoStatus::Pending, meta: None,
    }]);
    agent.generated_session_title = Some(name.into());
}

#[test]
fn global_work_panes_and_focus_survive_remote_attachment_and_switches_in_every_placement() {
    let _theme = crate::theme::cache::pin_theme();
    dock::set_enabled_for_test(false);
    for contents in [DockContents::Empty, DockContents::Active] {
        check_session_focus_matrix(contents);
    }
}

fn check_session_focus_matrix(contents: DockContents) {
    for placement in [DockPlacement::Off, DockPlacement::Top, DockPlacement::Bottom] {
        let mut focuses = vec![VisibleFocus::Pane(AgentPane::Prompt), VisibleFocus::Pane(AgentPane::Todo),
            VisibleFocus::Timeline, VisibleFocus::Context];
        if matches!(contents, DockContents::Active) {
            let task_focus = if placement.is_visible() { AgentPane::Dock } else { AgentPane::Tasks };
            focuses.push(VisibleFocus::Pane(task_focus));
            focuses.push(VisibleFocus::SidebarDock);
        }
        for focus in focuses {
            let settings = tempfile::tempdir().unwrap();
            let mut app = test_app_with_agent();
            app.screen_mode = ScreenMode::Fullscreen;
            app.next_agent_id = 1;
            app.restore_code = Some(false);
            let root = AgentId(0);
            {
                let agent = app.agents.get_mut(&root).unwrap();
                agent.session.session_id = Some("display-root".into());
                agent.open_panel_settings_in(settings.path());
                populate(agent, "root", contents);
                assert!(agent.set_dock_placement(placement));
            }
            for key in ['g', 't'] {
                let _ = app.handle_input(&Event::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL)));
            }
            let agent = app.agents.get_mut(&root).unwrap();
            assert!(agent.tasks.overlay.visible && agent.todo.overlay.visible);
            agent.adopt_global_sidebars(true, true, false, false);
            agent.restore_visible_focus(focus);
            let (mut terminal, _frames) = crate::test_util::test_terminal_in(ratatui::layout::Rect::new(0, 0, 220, 50));
            app.draw(&mut terminal);
            assert_eq!(app.agents.get(&root).unwrap().visible_focus(), focus);
            app.capture_sidebar_chrome();
            assert!(app.agents.find_by_session_id("display-remote").is_none());
            let effects = crate::app::dispatch::dispatch(Action::LoadSession("display-remote".into(), None, false), &mut app);
            let remote = effects.iter().find_map(|effect| match effect {
                crate::app::actions::Effect::LoadSession { agent_id, .. } => Some(*agent_id),
                _ => None,
            }).expect("an actual remote load must be scheduled");
            assert_ne!(remote, root);
            {
                let agent = app.agents.get_mut(&remote).unwrap();
                agent.open_panel_settings_in(settings.path());
                populate(agent, "remote", contents);
            }
            let _ = crate::app::dispatch::dispatch(Action::TaskComplete(TaskResult::SessionLoaded {
                agent_id: remote, session_id: "display-remote".into(), models: None, modes: None,
                code_restored: false, restore_summary: None, restore_degree: None, running_prompt_id: None,
            }), &mut app);
            for (session_id, expected_id) in [("display-remote", remote), ("display-root", root), ("display-remote", remote)] {
                app.capture_sidebar_chrome();
                let _ = crate::app::dispatch::dispatch(Action::OpenSession(session_id.into()), &mut app);
                app.draw(&mut terminal);
                assert_eq!(app.active_view, ActiveView::Agent(expected_id));
                let agent = app.agents.get(&expected_id).unwrap();
                assert_eq!(agent.visible_focus(), focus, "placement {placement:?}, session {session_id}");
                assert!(agent.tasks.overlay.visible && agent.todo.overlay.visible);
                assert!(agent.pane_areas.todo.height > 0);
                assert!(agent.f7_body.area() > 0);
                assert_eq!(agent.todo.todos().len(), 1);
                assert_eq!(agent.session.bg_tasks.len(), contents.count());
                assert_eq!(agent.subagent_sessions.len(), contents.count());
                match (contents, placement) {
                    (DockContents::Empty, _) => {
                        assert_eq!(agent.pane_areas.dock.height, 0);
                        assert!(agent.context_dock_geometry().is_none());
                    }
                    (DockContents::Active, DockPlacement::Off) => {
                        assert!(agent.pane_areas.tasks.height > 0);
                        let (hosted, _) = agent.context_dock_geometry().expect("Off retains independent F7 work");
                        assert_eq!(agent.pane_areas.dock, hosted);
                        assert!(!hosted.intersects(agent.pane_areas.tasks));
                    }
                    (DockContents::Active, DockPlacement::Top) => assert!(agent.pane_areas.dock.bottom() <= agent.pane_areas.scrollback.y),
                    (DockContents::Active, DockPlacement::Bottom) => assert!(agent.pane_areas.dock.y >= agent.pane_areas.scrollback.bottom()),
                }
            }
            let mut restarted = crate::app::agent_view::test_fixtures::make_agent();
            restarted.open_panel_settings_in(settings.path());
            assert!(restarted.tasks.overlay.visible && restarted.todo.overlay.visible);
            assert_eq!(dock::placement(), placement);
        }
    }
}
