//! Request inspector: open/close, capture-answer correlation, honest states, Enter/Esc ownership.

use super::super::test_fixtures;
use crate::actions::{ActionId, ActionRegistry, When};
use crate::app::actions::{Action, RequestSnapshotQuery, RequestSnapshotTransportError};
use crate::app::app_view::InputOutcome;
use crate::views::request_viewer::{
    PendingSnapshotRefresh, RawBodyState, RequestBodyContent, RequestBodyKey,
    RequestViewData, RequestViewTab, SnapshotQueryState,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use xai_grok_shell::session::request_capture::{
    CaptureFailure, RequestBodyRead, RequestSnapshot, RequestSnapshotId, RequestSnapshotNotification,
};

const SESSION: &str = "11111111-1111-1111-1111-111111111111";
const REQUEST: &str = "22222222-2222-2222-2222-222222222222";

fn press(agent: &mut super::AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    )
}

fn wire_snapshot(turn: u64, prepared_only: bool) -> RequestSnapshot {
    let source_id: serde_json::Value = serde_json::to_value([1u8; 16]).expect("id bytes");
    let digest: serde_json::Value = serde_json::to_value([7u8; 32]).expect("digest bytes");
    let prepared = serde_json::json!({
        "event": "prepared",
        "backend": "responses",
        "membership": [
            { "source": { "state": "admitted", "id": source_id }, "category": { "category": "user_turns" }, "evidence": { "state": "unknown" } },
            { "source": { "state": "unadmitted" }, "category": { "category": "core_instructions" }, "evidence": { "state": "unknown" } },
        ],
        "body": {
            "digest": digest,
            "byte_len": 1234,
            "availability": { "state": "captured", "publication_digest": digest, "representation": { "representation": "exact_local_json" } },
        },
        "source_durability": "unconfirmed_independent_history_channel",
    });
    let mut observations = vec![serde_json::json!({ "attempt": 2, "observation": prepared })];
    if !prepared_only {
        observations.push(serde_json::json!({ "attempt": 2, "observation": { "event": "dispatch_started" } }));
        observations.push(serde_json::json!({ "attempt": 2, "observation": { "event": "response_headers", "status": 200 } }));
        observations.push(serde_json::json!({ "attempt": 2, "observation": { "event": "finished", "outcome": "completed" } }));
    }
    let snapshot = serde_json::json!({
        "origin": {
            "session_id": SESSION,
            "request": REQUEST,
            "turn": turn,
            "generation": 1,
            "prompt_id": "prompt-abc12345",
            "conversation_id": null,
            "logical_request_id": null,
        },
        "observations": observations,
    });
    serde_json::from_value(snapshot).expect("typed snapshot fixture")
}

fn wire_notification(session_id: &str) -> RequestSnapshotNotification {
    let notification = serde_json::json!({
        "sessionId": session_id,
        "request": REQUEST,
        "evidence": { "attempt": 2, "observation": { "event": "dispatch_started" } },
        "receipt": { "persistence": "unconfirmed", "cause": "storage" },
    });
    serde_json::from_value(notification).expect("typed notification fixture")
}

fn wire_request_id() -> RequestSnapshotId {
    RequestSnapshotId(uuid::Uuid::parse_str(REQUEST).expect("uuid"))
}

fn wire_body_available() -> RequestBodyRead {
    let digest: serde_json::Value = serde_json::to_value([7u8; 32]).expect("digest bytes");
    let body = serde_json::json!({
        "state": "available",
        "json": "{\"model\":\"grok\"}",
        "representation": { "representation": "exact_local_json" },
        "protected_local": {
            "snapshot": { "sessionId": SESSION, "request": REQUEST },
            "attempt": 2,
            "digest": digest,
        },
    });
    serde_json::from_value(body).expect("typed body fixture")
}

fn wire_body_expired() -> RequestBodyRead {
    serde_json::from_value(serde_json::json!({
        "state": "unavailable",
        "availability": { "state": "expired" },
    }))
    .expect("typed expired fixture")
}

fn session_agent() -> super::AgentView {
    let mut agent = test_fixtures::make_agent();
    agent.session.session_id = Some(agent_client_protocol::SessionId::new(SESSION));
    agent
}

fn open(agent: &mut super::AgentView) -> u64 {
    assert!(matches!(
        press(agent, KeyCode::F(3)),
        InputOutcome::Action(Action::OpenRequestInspector)
    ));
    let fetch = agent.open_request_viewer().expect("session id present");
    assert!(matches!(fetch.1, RequestSnapshotQuery::Latest));
    fetch.2
}

fn body_fetch(agent: &mut super::AgentView) -> (RequestBodyKey, u64) {
    let (session_id, query, generation) = agent.request_body_query_started().expect("body query armed");
    assert_eq!(Some(session_id), agent.session.session_id);
    let RequestSnapshotQuery::Body { key } = query else { panic!("body query"); };
    (key, generation)
}

fn apply_body(
    agent: &mut super::AgentView,
    key: &RequestBodyKey,
    generation: u64,
    body: RequestBodyRead,
) {
    let session_id = agent.session.session_id.clone().unwrap();
    let content = RequestBodyContent::prepare(key, body);
    agent.request_body_loaded(&session_id, generation, key, content);
}

fn refresh(agent: &mut super::AgentView) -> u64 {
    agent.request_snapshot_observed(&wire_notification(SESSION)).expect("refresh starts").2
}

fn rendered_text(agent: &mut super::AgentView) -> String {
    let area = ratatui::layout::Rect::new(0, 0, 180, 40);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    agent.request_viewer.as_mut().unwrap().render(area, &mut buffer);
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

#[test]
fn f3_opens_esc_closes_and_enter_never_touches_the_composer() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    agent.prompt.set_text("draft stays");
    open(&mut agent);
    assert!(agent.request_viewer.is_some());

    let outcome = agent.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    );
    assert!(matches!(outcome, InputOutcome::Changed), "Enter is consumed by the inspector");
    assert_eq!(agent.prompt.text(), "draft stays", "Enter never injects or edits composer content");

    assert!(matches!(press(&mut agent, KeyCode::Esc), InputOutcome::Changed));
    assert!(agent.request_viewer.is_none(), "Esc closes the inspector");
    assert_eq!(agent.prompt.text(), "draft stays");
}

#[test]
fn readable_tab_shows_capture_facts_without_claiming_model_consumption() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    assert!(
        agent
            .request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, false))))
            .is_none(),
        "the readable tab arms no body query"
    );
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    let text = viewer.full_copy_text().expect("readable text");
    assert!(text.contains("attempt 2"));
    assert!(text.contains("prepared"));
    assert!(text.contains("dispatch started"));
    assert!(text.contains("response HTTP 200"));
    assert!(text.contains("finished: completed"));
    assert!(text.contains("Your messages 1"));
    assert!(text.contains("Core instructions 1"));
    assert!(text.contains("sources: 1 with local id  ·  1 without"));
    assert!(text.contains("dispatch and response facts do not prove what the model consumed"));
}

#[test]
fn raw_tab_fetches_the_prepared_body_lazily() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    assert!(agent.request_body_query_started().is_none(), "nothing loaded yet");
    assert!(
        agent
            .request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))))
            .is_none()
    );

    assert!(matches!(
        press(&mut agent, KeyCode::Char('r')),
        InputOutcome::Action(Action::FetchRequestBody)
    ));
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    assert_eq!(viewer.tab, RequestViewTab::Raw);
    assert!(matches!(viewer.raw, RawBodyState::NotRequested), "the body is not prefetched");

    let (key, generation) = body_fetch(&mut agent);
    assert_eq!(key.0.snapshot.request, wire_request_id());
    assert_eq!(key.0.attempt.0, 2);
    assert_eq!(key.0.digest, [7; 32]);
    assert!(matches!(
        agent.request_viewer.as_ref().expect("viewer open").raw,
        RawBodyState::Loading { .. }
    ));

    apply_body(&mut agent, &key, generation, wire_body_available());
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    assert!(matches!(viewer.raw, RawBodyState::Loaded { .. }));
    assert_eq!(viewer.full_copy_text().as_deref(), Some("{\"model\":\"grok\"}"));
    assert!(rendered_text(&mut agent).contains("\"model\": \"grok\""));
}

#[test]
fn expired_and_failed_bodies_surface_as_typed_unavailability() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    assert!(
        agent
            .request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))))
            .is_none()
    );
    press(&mut agent, KeyCode::Char('r'));
    let (key, generation) = body_fetch(&mut agent);

    apply_body(&mut agent, &key, generation, wire_body_expired());
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    assert!(matches!(viewer.raw, RawBodyState::Unavailable { .. }));
    assert!(viewer.full_copy_text().expect("expired text").contains("expired"));

    let refresh_generation = refresh(&mut agent);
    let mut snapshot = wire_snapshot(4, true);
    snapshot.observations[0].attempt.0 = 3;
    let (_, query, generation) = agent.request_snapshot_loaded(
        &session_id, refresh_generation, Ok(Some(snapshot)),
    ).expect("new attempt fetches even after unavailability");
    let RequestSnapshotQuery::Body { key } = query else { panic!("body query"); };
    agent.request_body_loaded(
        &session_id,
        generation,
        &key,
        Err(RequestSnapshotTransportError::Rpc(
            agent_client_protocol::Error::internal_error(),
        )),
    );
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    assert!(matches!(viewer.raw, RawBodyState::Failed { .. }));
    assert!(viewer.full_copy_text().expect("failed text").contains("capture RPC failed"));
}

#[test]
fn stale_and_foreign_answers_never_replace_the_shown_session() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let old_generation = open(&mut agent);
    assert!(matches!(press(&mut agent, KeyCode::Esc), InputOutcome::Changed));
    let generation = open(&mut agent);
    assert_ne!(old_generation, generation);
    let session_id = agent.session.session_id.clone().unwrap();
    let other = agent_client_protocol::SessionId::new("33333333-3333-3333-3333-333333333333");

    assert!(
        agent
            .request_snapshot_loaded(&other, generation, Ok(Some(wire_snapshot(9, true))))
            .is_none(),
        "a foreign session's answer is dropped"
    );
    assert!(
        matches!(
            agent.request_viewer.as_ref().expect("viewer open").data,
            RequestViewData::Loading
        )
    );

    assert!(agent.request_snapshot_loaded(
        &session_id, old_generation, Ok(Some(wire_snapshot(5, true))),
    ).is_none());
    assert!(agent.request_snapshot_loaded(
        &session_id, old_generation, Err(RequestSnapshotTransportError::UnexpectedShape),
    ).is_none());
    assert!(matches!(agent.request_viewer.as_ref().unwrap().data, RequestViewData::Loading));
    assert!(
        agent
            .request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))))
            .is_none()
    );
    assert!(
        agent
            .request_viewer
            .as_ref()
            .expect("viewer open")
            .full_copy_text()
            .expect("text")
            .contains("turn 4"),
        "a stale generation does not overwrite the current content"
    );

    // The session switched under the viewer: it closes instead of showing another session's data.
    agent.session.session_id = Some(other);
    agent.close_foreign_request_viewer();
    assert!(agent.request_viewer.is_none());
}

#[test]
fn real_observations_refresh_only_the_owning_session() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    assert!(
        agent
            .request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))))
            .is_none()
    );

    assert!(
        agent
            .request_snapshot_observed(&wire_notification("99999999-9999-9999-9999-999999999999"))
            .is_none()
    );
    assert!(
        matches!(agent.request_viewer.as_ref().expect("viewer open").snapshot_query, SnapshotQueryState::Idle),
        "foreign observation changes nothing"
    );

    let fetch = agent
        .request_snapshot_observed(&wire_notification(SESSION))
        .expect("own observation refreshes");
    assert_eq!(fetch.0, session_id);
    assert!(matches!(fetch.1, RequestSnapshotQuery::Latest));
    assert!(matches!(agent.request_viewer.as_ref().expect("viewer open").snapshot_query,
        SnapshotQueryState::InFlight { generation, refresh: PendingSnapshotRefresh::Clear } if generation == fetch.2));
}

#[test]
fn capture_unavailability_is_shown_as_its_typed_cause() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    assert!(
        agent
            .request_snapshot_loaded(
                &session_id,
                generation,
                Err(RequestSnapshotTransportError::Unavailable(CaptureFailure::Storage)),
            )
            .is_none()
    );
    let viewer = agent.request_viewer.as_ref().expect("viewer open");
    assert!(matches!(viewer.data, RequestViewData::Unavailable { .. }));
    assert!(
        viewer
            .full_copy_text()
            .expect("text")
            .contains("capture storage failed")
    );
}

#[test]
fn registered_toggle_closes_without_releasing_other_agent_shortcuts() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    open(&mut agent);
    assert!(matches!(press(&mut agent, KeyCode::F(6)), InputOutcome::Unchanged));
    assert!(agent.request_viewer.is_some());
    assert!(matches!(press(&mut agent, KeyCode::F(3)), InputOutcome::Changed));
    assert!(agent.request_viewer.is_none());

    let mut definition = ActionRegistry::defaults().find(ActionId::ToggleRequestInspector).unwrap().clone();
    definition.default_key = crate::key!(F(4));
    definition.context = When::AgentScreen;
    let registry = ActionRegistry::new(vec![definition]);
    agent.open_request_viewer().unwrap();
    assert!(matches!(agent.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)), &registry,
    ), InputOutcome::Unchanged));
    assert!(agent.request_viewer.is_some(), "no hard-coded F3 bypass");
    assert!(matches!(agent.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)), &registry,
    ), InputOutcome::Changed));
    assert!(agent.request_viewer.is_none());
}

#[test]
fn search_enter_and_escape_remain_inside_the_inspector() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    agent.prompt.set_text("untouched editor");
    open(&mut agent);
    assert!(matches!(press(&mut agent, KeyCode::Char('/')), InputOutcome::Changed));
    assert!(matches!(press(&mut agent, KeyCode::Esc), InputOutcome::Changed));
    assert!(agent.request_viewer.is_some(), "Esc cancels active search before closing the modal");
    assert!(matches!(press(&mut agent, KeyCode::Char('/')), InputOutcome::Changed));
    assert!(matches!(press(&mut agent, KeyCode::Char('x')), InputOutcome::Changed));
    assert!(matches!(press(&mut agent, KeyCode::Enter), InputOutcome::Changed));
    assert!(agent.request_viewer.is_some(), "Enter accepts search without sending the prompt");
    assert_eq!(agent.prompt.text(), "untouched editor");
    assert!(matches!(press(&mut agent, KeyCode::Esc), InputOutcome::Changed));
    assert!(agent.request_viewer.is_none());
    assert_eq!(agent.prompt.text(), "untouched editor");
}

#[test]
fn observations_coalesce_into_one_active_read_and_one_event_triggered_successor() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    for _ in 0..128 {
        assert!(agent.request_snapshot_observed(&wire_notification(SESSION)).is_none());
    }
    assert!(matches!(agent.request_viewer.as_ref().unwrap().snapshot_query,
        SnapshotQueryState::InFlight { generation: active, refresh: PendingSnapshotRefresh::Pending }
            if active == generation));
    let (_, query, successor) = agent.request_snapshot_loaded(
        &session_id, generation, Err(RequestSnapshotTransportError::Unavailable(CaptureFailure::Storage)),
    ).expect("events request one successor even if the older read failed");
    assert!(matches!(query, RequestSnapshotQuery::Latest));
    assert_ne!(generation, successor);
    assert!(matches!(agent.request_viewer.as_ref().unwrap().data, RequestViewData::Loading));
    assert!(agent.request_snapshot_loaded(&session_id, generation, Ok(None)).is_none());
    assert!(agent.request_snapshot_loaded(&session_id, generation,
        Err(RequestSnapshotTransportError::UnexpectedShape)).is_none());
    assert!(matches!(agent.request_viewer.as_ref().unwrap().data, RequestViewData::Loading));
    assert!(agent.request_snapshot_loaded(&session_id, successor, Ok(Some(wire_snapshot(4, true)))).is_none());
    assert!(agent.request_snapshot_loaded(&session_id, generation, Ok(None)).is_none());
    assert!(agent.request_snapshot_loaded(&session_id, generation,
        Err(RequestSnapshotTransportError::UnexpectedShape)).is_none());
    let viewer = agent.request_viewer.as_ref().unwrap();
    assert!(matches!(viewer.snapshot_query, SnapshotQueryState::Idle));
    assert!(viewer.full_copy_text().unwrap().contains("turn 4"));
}

#[test]
fn changed_request_attempt_or_digest_invalidates_every_raw_state_and_rejects_old_completions() {
    #[derive(Clone, Copy)]
    enum Previous { Loading, Loaded, Failed }
    let _theme = crate::theme::cache::pin_theme();
    for previous in [Previous::Loading, Previous::Loaded, Previous::Failed] {
        for (request, attempt, digest) in [
            ("33333333-3333-3333-3333-333333333333", 2, [7; 32]),
            (REQUEST, 3, [7; 32]),
            (REQUEST, 2, [8; 32]),
        ] {
            let mut agent = session_agent();
            let generation = open(&mut agent);
            let session_id = agent.session.session_id.clone().unwrap();
            agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
            press(&mut agent, KeyCode::Char('r'));
            let (old_key, old_generation) = body_fetch(&mut agent);
            match previous {
                Previous::Loading => {}
                Previous::Loaded => apply_body(&mut agent, &old_key, old_generation, wire_body_available()),
                Previous::Failed => agent.request_body_loaded(&session_id, old_generation, &old_key,
                    Err(RequestSnapshotTransportError::UnexpectedShape)),
            }
            let refresh_generation = refresh(&mut agent);
            let mut snapshot = wire_snapshot(5, true);
            snapshot.origin.request = RequestSnapshotId(uuid::Uuid::parse_str(request).unwrap());
            snapshot.observations[0].attempt.0 = attempt;
            if let xai_grok_shell::session::request_capture::RequestObservation::Prepared { body, .. } =
                &mut snapshot.observations[0].observation {
                body.digest = digest;
            }
            let (_, query, new_generation) = agent.request_snapshot_loaded(
                &session_id, refresh_generation, Ok(Some(snapshot)),
            ).expect("the open raw tab fetches the successor without another keypress");
            let RequestSnapshotQuery::Body { key } = query else { panic!("body query"); };
            assert_ne!(old_key, key);
            assert_ne!(old_generation, new_generation);
            assert_eq!(key.0.snapshot.request.0.to_string(), request);
            assert_eq!(key.0.attempt.0, attempt);
            assert_eq!(key.0.digest, digest);
            apply_body(&mut agent, &old_key, old_generation, wire_body_available());
            agent.request_body_loaded(&session_id, old_generation, &old_key,
                Err(RequestSnapshotTransportError::Unavailable(CaptureFailure::Storage)));
            assert!(matches!(&agent.request_viewer.as_ref().unwrap().raw,
                RawBodyState::Loading { key: active, generation } if active == &key && *generation == new_generation));
            let mut body = wire_body_available();
            if let RequestBodyRead::Available { protected_local, json, .. } = &mut body {
                *protected_local = key.0.clone();
                *json = "{\"successor\":true}".to_owned();
            }
            apply_body(&mut agent, &key, new_generation, body);
            agent.request_body_loaded(&session_id, old_generation, &old_key,
                Err(RequestSnapshotTransportError::UnexpectedShape));
            assert_eq!(agent.request_viewer.as_ref().unwrap().full_copy_text().as_deref(),
                Some("{\"successor\":true}"));
        }
    }
}

#[test]
fn reopened_raw_rejects_old_successes_and_errors_even_when_the_evidence_key_is_identical() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
    press(&mut agent, KeyCode::Char('r'));
    let (old_key, old_generation) = body_fetch(&mut agent);
    press(&mut agent, KeyCode::Esc);
    let generation = open(&mut agent);
    agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
    press(&mut agent, KeyCode::Char('r'));
    let (key, generation) = body_fetch(&mut agent);
    assert_eq!(old_key, key);
    assert_ne!(old_generation, generation);
    apply_body(&mut agent, &old_key, old_generation, wire_body_available());
    agent.request_body_loaded(&session_id, old_generation, &old_key,
        Err(RequestSnapshotTransportError::UnexpectedShape));
    assert!(matches!(&agent.request_viewer.as_ref().unwrap().raw,
        RawBodyState::Loading { generation: active, .. } if *active == generation));
    apply_body(&mut agent, &key, generation, wire_body_available());
    assert_eq!(agent.request_viewer.as_ref().unwrap().full_copy_text().as_deref(), Some("{\"model\":\"grok\"}"));
}

#[test]
fn absent_or_unavailable_after_a_loaded_body_clears_raw_and_ignores_old_errors() {
    let _theme = crate::theme::cache::pin_theme();
    for result in [Ok(None), Err(RequestSnapshotTransportError::Unavailable(CaptureFailure::Storage))] {
        let mut agent = session_agent();
        let generation = open(&mut agent);
        let session_id = agent.session.session_id.clone().unwrap();
        agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
        press(&mut agent, KeyCode::Char('r'));
        let (key, body_generation) = body_fetch(&mut agent);
        apply_body(&mut agent, &key, body_generation, wire_body_available());
        let generation = refresh(&mut agent);
        assert!(agent.request_snapshot_loaded(&session_id, generation, result).is_none());
        agent.request_body_loaded(&session_id, body_generation, &key,
            Err(RequestSnapshotTransportError::UnexpectedShape));
        apply_body(&mut agent, &key, body_generation, wire_body_available());
        let viewer = agent.request_viewer.as_ref().unwrap();
        assert!(matches!(viewer.raw, RawBodyState::NotRequested));
        assert!(!viewer.full_copy_text().unwrap().contains("grok"));
        assert!(agent.request_body_query_started().is_none());
    }
}

#[test]
fn unchanged_evidence_keeps_the_same_presentation_across_refresh_and_tab_changes() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
    press(&mut agent, KeyCode::Char('r'));
    let (key, generation) = body_fetch(&mut agent);
    apply_body(&mut agent, &key, generation, wire_body_available());
    let RawBodyState::Loaded { presentation, .. } = &agent.request_viewer.as_ref().unwrap().raw
        else { panic!("loaded body"); };
    let original = std::sync::Arc::clone(&presentation.items);
    for _ in 0..3 {
        let generation = refresh(&mut agent);
        assert!(agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, false)))).is_none());
        press(&mut agent, KeyCode::Char('r'));
        press(&mut agent, KeyCode::Char('r'));
        assert!(agent.request_body_query_started().is_none());
        let viewer = agent.request_viewer.as_ref().unwrap();
        let RawBodyState::Loaded { presentation, .. } = &viewer.raw else { panic!("loaded body"); };
        assert!(std::sync::Arc::ptr_eq(&original, &presentation.items));
        assert_eq!(viewer.full_copy_text().as_deref(), Some("{\"model\":\"grok\"}"));
    }
}

#[test]
fn rpc_failure_output_is_categorical_and_never_contains_foreign_message_or_data() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = session_agent();
    let generation = open(&mut agent);
    let session_id = agent.session.session_id.clone().unwrap();
    let error = serde_json::from_value(serde_json::json!({
        "code": -32603,
        "message": "untrusted-foreign-message".repeat(4096),
        "data": { "detail": "untrusted-foreign-data".repeat(4096) },
    })).unwrap();
    agent.request_snapshot_loaded(&session_id, generation, Err(RequestSnapshotTransportError::Rpc(error)));
    let text = agent.request_viewer.as_ref().unwrap().full_copy_text().unwrap();
    assert!(text.contains("Capture unavailable: capture RPC failed"));
    assert!(!text.contains("untrusted-foreign"));
    assert!(text.len() < 512);
    assert!(!rendered_text(&mut agent).contains("untrusted-foreign"));
}

#[test]
fn publication_representation_is_truthful_in_render_and_copy_retains_publication_bytes() {
    let _theme = crate::theme::cache::pin_theme();
    for (representation, expected) in [
        (serde_json::json!({"representation": "exact_local_json"}), "Unchanged local JSON publication"),
        (serde_json::json!({"representation": "redacted", "original_digest": vec![7u8; 32]}), "Redacted publication, not the protected original serialization"),
    ] {
        let mut agent = session_agent();
        let generation = open(&mut agent);
        let session_id = agent.session.session_id.clone().unwrap();
        agent.request_snapshot_loaded(&session_id, generation, Ok(Some(wire_snapshot(4, true))));
        press(&mut agent, KeyCode::Char('r'));
        let (key, generation) = body_fetch(&mut agent);
        let mut body = wire_body_available();
        if let RequestBodyRead::Available { representation: actual, .. } = &mut body {
            *actual = serde_json::from_value(representation).unwrap();
        }
        apply_body(&mut agent, &key, generation, body);
        assert!(rendered_text(&mut agent).contains(expected));
        assert_eq!(agent.request_viewer.as_ref().unwrap().full_copy_text().as_deref(), Some("{\"model\":\"grok\"}"));
    }
}
