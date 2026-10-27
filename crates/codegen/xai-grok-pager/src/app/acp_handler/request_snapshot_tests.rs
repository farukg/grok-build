//! `x.ai/session/request_snapshot/observed` reaches only the session that owns the capture.

use super::super::tests::make_app_two_agents;
use crate::app::actions::{Effect, RequestSnapshotQuery};
use crate::app::agent::AgentId;
use crate::views::request_viewer::{RequestViewData, SnapshotQueryState};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use xai_grok_shell::session::request_capture::REQUEST_SNAPSHOT_NOTIFICATION;

#[test]
fn an_observation_refreshes_only_the_inspector_of_its_own_session() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = make_app_two_agents();
    for (id, agent) in app.agents.all_mut() {
        agent.session.session_id = Some(agent_client_protocol::SessionId::new(if id == AgentId(0) {
            "11111111-1111-1111-1111-111111111111"
        } else {
            "33333333-3333-3333-3333-333333333333"
        }));
        let _ = agent.handle_input(
            &Event::Key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            &crate::actions::ActionRegistry::defaults(),
        );
        let (session_id, _, generation) = agent.open_request_viewer().expect("session id");
        assert!(agent.request_snapshot_loaded(&session_id, generation, Ok(None)).is_none());
    }
    app.pending_effects.clear();

    let digest: serde_json::Value = serde_json::to_value([7u8; 32]).expect("digest bytes");
    let payload = serde_json::json!({
        "sessionId": "11111111-1111-1111-1111-111111111111",
        "request": "22222222-2222-2222-2222-222222222222",
        "evidence": { "attempt": 1, "observation": { "event": "prepared", "backend": "responses", "membership": [], "body": { "digest": digest, "byte_len": 10, "availability": { "state": "unknown" } }, "source_durability": "unconfirmed_independent_history_channel" } },
        "receipt": { "persistence": "unconfirmed", "cause": "storage" },
    });
    let notification = xai_grok_test_support::acp_fixtures::ext_notification(
        REQUEST_SNAPSHOT_NOTIFICATION,
        &payload,
    );
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    assert!(crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: notification,
            response_tx,
        }),
        &mut app,
    ));

    assert!(
        app.agents
            .get(&AgentId(0))
            .unwrap()
            .request_viewer
            .as_ref()
            .is_some_and(|viewer| matches!(viewer.snapshot_query, SnapshotQueryState::InFlight { .. })),
        "the owning session's viewer is marked stale"
    );
    assert!(
        !app.agents
            .get(&AgentId(1))
            .unwrap()
            .request_viewer
            .as_ref()
            .is_some_and(|viewer| matches!(viewer.snapshot_query, SnapshotQueryState::InFlight { .. })),
        "the other session's viewer is untouched"
    );
    let effects = std::mem::take(&mut app.pending_effects);
    assert_eq!(effects.len(), 1, "exactly the owning session refreshes");
    assert!(matches!(
        &effects[0],
        Effect::RequestSnapshotQuery { agent_id, query, .. }
            if *agent_id == AgentId(0) && matches!(query, RequestSnapshotQuery::Latest)
    ));

    for _ in 0..16 {
        let notification = xai_grok_test_support::acp_fixtures::ext_notification(
            REQUEST_SNAPSHOT_NOTIFICATION, &payload,
        );
        let (response_tx, _response) = tokio::sync::oneshot::channel();
        assert!(crate::app::acp_handler::handle(
            xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: notification, response_tx,
            }), &mut app,
        ));
    }
    assert!(app.pending_effects.is_empty(), "observations coalesce while the read is active");

    let foreign = serde_json::json!({
        "sessionId": "99999999-9999-9999-9999-999999999999",
        "request": "22222222-2222-2222-2222-222222222222",
        "evidence": { "attempt": 1, "observation": { "event": "dispatch_started" } },
        "receipt": { "persistence": "confirmed", "origin": { "session_id": "99999999-9999-9999-9999-999999999999", "request": "22222222-2222-2222-2222-222222222222", "turn": 1, "generation": 1, "prompt_id": null, "conversation_id": null, "logical_request_id": null } },
    });
    let notification = xai_grok_test_support::acp_fixtures::ext_notification(
        REQUEST_SNAPSHOT_NOTIFICATION,
        &foreign,
    );
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    assert!(!crate::app::acp_handler::handle(
        xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: notification,
            response_tx,
        }),
        &mut app,
    ));
    assert!(app.pending_effects.is_empty());
    assert!(matches!(
        app.agents
            .get(&AgentId(0))
            .unwrap()
            .request_viewer
            .as_ref()
            .unwrap()
            .data,
        RequestViewData::Absent
    ));
}
