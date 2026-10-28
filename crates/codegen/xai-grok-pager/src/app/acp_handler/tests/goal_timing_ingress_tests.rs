use super::super::*;
use std::time::Duration;

#[test]
fn delayed_goal_terminal_uses_backend_elapsed_in_projection_and_history() {
    for status in ["user_paused", "complete"] {
        let mut app = make_app_with_agent("sess-A");
        send_goal_update(&mut app, "goal", "active", 5_000);
        app.agents.get_mut(&AgentId(0)).unwrap().goal_state.as_mut().unwrap().received_at =
            Instant::now().checked_sub(Duration::from_secs(20)).unwrap();
        send_goal_update(&mut app, "goal", status, 6_000);
        let agent = test_agent(&app, AgentId(0));
        let goal = agent.goal_state.as_ref().unwrap();
        assert_eq!(goal.elapsed_ms, 6_000);
        assert_eq!(goal.live_elapsed_ms(), 6_000);
        assert_eq!(goal.elapsed_floor_ms, 6_000);
        let markers: Vec<_> = (0..agent.scrollback.len())
            .filter_map(|index| agent.scrollback.get(index))
            .filter_map(|entry| {
                let RenderBlock::SessionEvent(block) = &entry.block else { return None; };
                let SessionEvent::GoalCompleted { elapsed } = &block.event else { return None; };
                Some(*elapsed)
            }).collect();
        let history_len = agent.scrollback.len();
        match GoalDisplayStatus::parse(status) {
            GoalDisplayStatus::Complete => {
                assert_eq!(markers, vec![Duration::from_secs(6)]);
                send_goal_update(&mut app, "goal", "complete", 6_000);
                assert_eq!(test_agent(&app, AgentId(0)).scrollback.len(), history_len);
            }
            GoalDisplayStatus::UserPaused => {
                assert!(markers.is_empty());
                send_goal_update(&mut app, "goal", "active", 6_000);
                let goal = test_agent(&app, AgentId(0)).goal_state.as_ref().unwrap();
                assert_eq!(goal.elapsed_ms, 6_000);
                assert_eq!(goal.elapsed_floor_ms, 6_000);
            }
            GoalDisplayStatus::Active | GoalDisplayStatus::BackOffPaused
            | GoalDisplayStatus::NoProgressPaused | GoalDisplayStatus::InfraPaused
            | GoalDisplayStatus::Failed | GoalDisplayStatus::Interrupted
            | GoalDisplayStatus::Blocked | GoalDisplayStatus::BudgetLimited => panic!("unexpected fixture status"),
        }
    }
}

#[test]
fn goal_id_change_and_wire_replay_do_not_inherit_the_live_floor() {
    let mut app = make_app_with_agent("sess-A");
    send_goal_update(&mut app, "first", "active", 5_000);
    app.agents.get_mut(&AgentId(0)).unwrap().goal_state.as_mut().unwrap().received_at =
        Instant::now().checked_sub(Duration::from_secs(20)).unwrap();
    send_goal_update(&mut app, "second", "active", 1_000);
    assert_eq!(test_agent(&app, AgentId(0)).goal_state.as_ref().unwrap().elapsed_floor_ms, 1_000);

    let update = serde_json::from_value(goal_update_value("second", "active", 700)).unwrap();
    let notification = SessionNotification {
        session_id: acp::SessionId::new("sess-A"), update,
        meta: Some(crate::acp::meta::ReplayMetaStamp::replayed()),
    };
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: acp_fixtures::ext_notification("x.ai/session_notification", &notification),
        response_tx,
    }), &mut app);
    let goal = test_agent(&app, AgentId(0)).goal_state.as_ref().unwrap();
    assert_eq!(goal.goal_id, "second");
    assert_eq!(goal.elapsed_ms, 700);
    assert_eq!(goal.elapsed_floor_ms, 700);
}
