use super::*;

fn update_remote(app: &mut AppView, source: Source, id: &str, activity: RosterActivity) {
    let mut roster = app.dashboard_roster().to_vec();
    let row = roster.iter_mut().find(|row| row.session_id == id).unwrap();
    row.activity = activity;
    row.last_change_unix_ms = chrono::Utc::now().timestamp_millis();
    let result = match source {
        Source::Leader => TaskResult::RosterLoaded { sessions: roster },
        Source::Local | Source::Workspace => TaskResult::DashboardSessionsLoaded { sessions: roster },
    };
    dispatch(Action::TaskComplete(result), app);
}

#[test]
fn hidden_dashboard_keeps_ingress_order_and_reentry_without_intermediate_rows() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        app.active_view = ActiveView::Agent(AgentId(90));
        update_remote(&mut app, source, "HistChildC", RosterActivity::Working);
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Idle);
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Working);
        app.active_view = ActiveView::AgentDashboard;
        app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
        assert_eq!(
            painted_names(&paint_mixed(&mut app, 100))[..4],
            ["RemoteRoot", "HistChildC", "QueuedRoot", "WorkingChild"],
            "{source:?}",
        );
    }
}

#[test]
fn acp_roster_transitions_are_observed_between_frames() {
    let mut app = mixed_app(Source::Leader);
    app.active_view = ActiveView::Agent(AgentId(90));
    for (id, activity) in [
        ("HistChildC", "working"),
        ("RemoteRoot", "idle"),
        ("RemoteRoot", "working"),
    ] {
        let notification = acp::ExtNotification::new(
            "x.ai/sessions/changed",
            std::sync::Arc::from(serde_json::value::to_raw_value(&serde_json::json!({
                "upserted": [{
                    "sessionId": id,
                    "cwd": format!("/history/{id}"),
                    "sessionKind": (id == "HistChildC").then_some("subagent"),
                    "activity": activity,
                    "title": id,
                    "lastChangeUnixMs": 1,
                }],
            })).unwrap()),
        );
        let (response_tx, _response_rx) = tokio::sync::oneshot::channel();
        crate::app::acp_handler::handle(
            xai_acp_lib::AcpArgs { request: notification, response_tx }.into(),
            &mut app,
        );
    }
    app.active_view = ActiveView::AgentDashboard;
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
    assert_eq!(
        painted_names(&paint_mixed(&mut app, 100))[..4],
        ["RemoteRoot", "HistChildC", "QueuedRoot", "WorkingChild"],
    );
}

#[test]
fn local_dispatch_exit_and_acp_reentry_survive_a_hidden_dashboard() {
    let mut app = mixed_app(Source::Local);
    app.active_view = ActiveView::Agent(AgentId(90));
    dispatch(Action::TaskComplete(TaskResult::BgTaskKilled {
        session_id: "WorkingChild".into(),
        task_id: "task".into(),
        outcome: Some(xai_tool_types::KillOutcome::NotFound),
    }), &mut app);
    assert_eq!(
        crate::views::dashboard::classify_top_level(app.agents.get(&AgentId(1)).unwrap()),
        crate::views::dashboard::RowState::Idle,
    );
    update_remote(&mut app, Source::Local, "HistChildC", RosterActivity::Working);
    let notification = acp::ExtNotification::new(
        "x.ai/queue/changed",
        std::sync::Arc::from(serde_json::value::to_raw_value(&serde_json::json!({
            "sessionId": "WorkingChild",
            "entries": [],
            "runningPromptId": "new-work",
        })).unwrap()),
    );
    let (response_tx, _response_rx) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(
        xai_acp_lib::AcpArgs { request: notification, response_tx }.into(),
        &mut app,
    );
    assert_eq!(
        crate::views::dashboard::classify_top_level(app.agents.get(&AgentId(1)).unwrap()),
        crate::views::dashboard::RowState::Working,
    );
    app.active_view = ActiveView::AgentDashboard;
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
    assert_eq!(
        painted_names(&paint_mixed(&mut app, 100))[..4],
        ["WorkingChild", "HistChildC", "QueuedRoot", "RemoteRoot"],
    );
}

#[test]
fn workspace_only_members_without_live_status_are_not_running() {
    let mut app = mixed_app(Source::Workspace);
    app.agents = crate::app::session_views::SessionViews::new();
    app.dashboard_local_sessions.clear();
    let member = crate::app::workspace_test_fixtures::member("workspace-only", "WorkspaceOnly");
    app.workspace_membership.set_snapshot_for_test(
        crate::app::workspace_test_fixtures::snapshot(vec![member]),
    );
    let painted = paint_mixed(&mut app, 100);
    assert!(painted.contains("WorkspaceOnly"));
    let rows = crate::app::dispatch::dashboard::dashboard_focusables(&app);
    assert!(rows.contains(&crate::views::dashboard::Focusable::Row(
        DashboardRowId::Workspace { session_id: "workspace-only".into() },
    )));
    assert!(!rows.contains(&crate::views::dashboard::Focusable::Section(
        crate::views::dashboard::SectionKey::State(crate::views::dashboard::RowState::Working),
    )));
}

#[test]
fn continuous_running_membership_ignores_live_activity_in_every_source_and_lane() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
        let initial = paint_mixed(&mut app, 100);
        assert_eq!(painted_names(&initial), NAMES);
        let selected = DashboardRowId::TopLevel(AgentId(1));
        app.dashboard.as_mut().unwrap().focus_row(selected.clone());
        for step in 0..8 {
            let child = app.agents.get_mut(&AgentId(1)).unwrap();
            child.last_active_at = Some(Instant::now() + Duration::from_secs(step));
            child.display_name = Some(format!("WorkingChild live-{step}"));
            update_remote(&mut app, source, "RemoteRoot", RosterActivity::Working);
            for (lane, expected) in [
                (SessionLane::Main, vec!["QueuedRoot", "RemoteRoot", "HistRootA", "HistRootD", "TieRootZ", "UnknownRootZ"]),
                (SessionLane::Subagents, vec!["WorkingChild", "HistChildB", "HistChildC", "TieChildA", "UnknownChildA"]),
                (SessionLane::All, NAMES.to_vec()),
            ] {
                app.dashboard.as_mut().unwrap().set_session_lane(lane);
                let painted = paint_mixed(&mut app, 100);
                assert_eq!(painted_names(&painted), expected, "{source:?} {lane:?}");
                if lane != SessionLane::Main {
                    assert!(painted.contains(&format!("live-{step}")));
                }
            }
        }
        app.dashboard.as_mut().unwrap().focus_row(selected.clone());
        paint_mixed(&mut app, 100);
        assert_eq!(app.dashboard.as_ref().unwrap().selected, Some(selected.clone()));
        let hits = app.dashboard.as_ref().unwrap().row_rects.clone();
        let hit_ids = hits.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        let keyboard_ids = crate::app::dispatch::dashboard::dashboard_focusables(&app)
            .into_iter()
            .filter_map(|focus| match focus {
                crate::views::dashboard::Focusable::Row(id) => Some(id),
                crate::views::dashboard::Focusable::Section(_) | crate::views::dashboard::Focusable::IdleOverflow => None,
            }).collect::<Vec<_>>();
        assert_eq!(hit_ids, keyboard_ids);
        press_page_key(&mut app, crossterm::event::KeyCode::Home);
        assert_eq!(app.dashboard.as_ref().unwrap().selected, Some(DashboardRowId::TopLevel(AgentId(90))));
        assert_eq!(crate::views::dashboard::overlay_cycle_order(app.dashboard.as_ref().unwrap(), &app.agents)[0], AgentId(90));
    }
}

#[test]
fn newest_entry_wins_even_with_old_activity_and_reentry_gets_a_new_place() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
        assert_eq!(painted_names(&paint_mixed(&mut app, 100)), NAMES);
        update_remote(&mut app, source, "HistChildC", RosterActivity::Working);
        // Entry order must not be a disguised activity sort.
        match source {
            Source::Leader => app.leader_roster.iter_mut(),
            Source::Local | Source::Workspace => app.dashboard_local_sessions.iter_mut(),
        }.find(|row| row.session_id == "HistChildC").unwrap().last_change_unix_ms = 1;
        let entered = paint_mixed(&mut app, 100);
        assert_eq!(painted_names(&entered)[..4], ["HistChildC", "QueuedRoot", "RemoteRoot", "WorkingChild"]);
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Idle);
        let exited = paint_mixed(&mut app, 100);
        assert_eq!(painted_names(&exited)[..3], ["HistChildC", "QueuedRoot", "WorkingChild"]);
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Working);
        let reentered = paint_mixed(&mut app, 100);
        assert_eq!(painted_names(&reentered)[..4], ["RemoteRoot", "HistChildC", "QueuedRoot", "WorkingChild"]);
        update_remote(&mut app, source, "HistChildC", RosterActivity::NeedsInput);
        let needs_input = paint_mixed(&mut app, 100);
        assert_eq!(painted_names(&needs_input)[..4], ["RemoteRoot", "QueuedRoot", "WorkingChild", "HistChildC"]);
    }
}

#[test]
fn hidden_lanes_and_filters_observe_entries_without_resetting_continuous_peers() {
    for source in [Source::Local, Source::Leader, Source::Workspace] {
        let mut app = mixed_app(source);
        paint_mixed(&mut app, 100);
        app.dashboard.as_mut().unwrap().filter = Filter::Substring("no match".into());
        update_remote(&mut app, source, "HistChildC", RosterActivity::Working);
        assert!(painted_names(&paint_mixed(&mut app, 100)).is_empty());
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Idle);
        paint_mixed(&mut app, 100);
        update_remote(&mut app, source, "RemoteRoot", RosterActivity::Working);
        paint_mixed(&mut app, 100);
        let state = app.dashboard.as_mut().unwrap();
        state.filter = Filter::None;
        state.set_session_lane(SessionLane::All);
        assert_eq!(painted_names(&paint_mixed(&mut app, 100))[..4], ["RemoteRoot", "HistChildC", "QueuedRoot", "WorkingChild"]);
    }
}

#[test]
fn local_exit_and_reentry_use_membership_not_turn_or_activity_age() {
    let mut app = mixed_app(Source::Local);
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
    paint_mixed(&mut app, 100);
    app.agents.get_mut(&AgentId(1)).unwrap().session.bg_tasks.clear();
    let idle = paint_mixed(&mut app, 100);
    assert_eq!(painted_names(&idle)[..2], ["QueuedRoot", "RemoteRoot"]);
    let child = app.agents.get_mut(&AgentId(1)).unwrap();
    child.session.bg_tasks.insert("again".into(), crate::app::agent_test_fixtures::running_bg_task("again", false));
    child.last_active_at = Some(Instant::now() - Duration::from_secs(1_000));
    assert_eq!(painted_names(&paint_mixed(&mut app, 100))[..3], ["WorkingChild", "QueuedRoot", "RemoteRoot"]);
}

#[test]
fn equal_initial_entries_keep_their_tie_break_when_session_ids_bind() {
    let mut app = mixed_app(Source::Local);
    for (id, title) in [(98, "UnboundFirst"), (99, "UnboundSecond")] {
        let mut agent = crate::app::agent_view::test_fixtures::make_agent();
        agent.display_name = Some(title.into());
        agent.session.enqueue_prompt("queued".into());
        agent.last_active_at = None;
        app.agents.insert(AgentId(id), agent);
    }
    let before = paint_mixed(&mut app, 100);
    assert!(before.find("UnboundFirst").unwrap() < before.find("UnboundSecond").unwrap());
    app.agents.get_mut(&AgentId(98)).unwrap().session.session_id = Some(acp::SessionId::new("z-bound"));
    app.agents.get_mut(&AgentId(99)).unwrap().session.session_id = Some(acp::SessionId::new("a-bound"));
    let after = paint_mixed(&mut app, 100);
    assert!(after.find("UnboundFirst").unwrap() < after.find("UnboundSecond").unwrap());
}

#[test]
fn provisional_binding_and_roster_loading_preserve_running_membership() {
    let mut app = mixed_app(Source::Local);
    app.dashboard.as_mut().unwrap().set_session_lane(SessionLane::All);
    paint_mixed(&mut app, 100);
    let mut provisional = crate::app::agent_view::test_fixtures::make_agent();
    provisional.display_name = Some("NewArrival".into());
    provisional.session.enqueue_prompt("new work".into());
    app.agents.insert(AgentId(99), provisional);
    let painted = paint_mixed(&mut app, 100);
    assert!(painted.find("NewArrival").unwrap() < painted.find("QueuedRoot").unwrap());
    app.agents.get_mut(&AgentId(99)).unwrap().session.session_id = Some(acp::SessionId::new("new-arrival"));
    let mut remote = crate::app::agent_view::test_fixtures::make_agent();
    remote.display_name = Some("RemoteRoot".into());
    remote.session.session_id = Some(acp::SessionId::new("RemoteRoot"));
    remote.session.enqueue_prompt("loaded work".into());
    remote.last_active_at = Some(Instant::now() + Duration::from_secs(100));
    app.agents.insert(AgentId(100), remote);
    let rebound = paint_mixed(&mut app, 100);
    assert!(rebound.find("NewArrival").unwrap() < rebound.find("QueuedRoot").unwrap());
    assert_eq!(painted_names(&rebound)[..3], ["QueuedRoot", "RemoteRoot", "WorkingChild"]);
    assert!(crate::app::dispatch::dashboard::dashboard_focusables(&app).contains(
        &crate::views::dashboard::Focusable::Row(DashboardRowId::TopLevel(AgentId(100)))
    ));
}
