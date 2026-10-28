use super::*;
use crate::app::agent_view::queue::QueueTransport;
use crate::app::app_view::InputOutcome;
use crate::app::prompt_queue::QueueEntryWire;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{buffer::Buffer, layout::Rect};
use xai_grok_shell::extensions::interject::{QueueAction, QueueOperation, QueueRejection, QueueRequest, QueueResponse, QueueResult};

fn row(version: u64) -> QueueEntryWire {
    QueueEntryWire {
        id: "queued-input".into(), version, owner: Some("grok-tui".into()),
        last_editor: None, kind: "prompt".into(), text: "held exact payload".into(),
        position: 0, combined_texts: None,
    }
}

fn broadcast(app: &mut AppView, entries: Vec<QueueEntryWire>, running: Option<&str>) {
    let params = serde_json::json!({
        "sessionId": "test-session", "entries": entries, "runningPromptId": running,
        "runningText": running.map(|id| if id == "queued-input" { "held exact payload" } else { "running input" }), "runningKind": "prompt",
    });
    let (response_tx, _) = tokio::sync::oneshot::channel();
    crate::app::acp_handler::handle(xai_acp_lib::AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
        request: acp::ExtNotification::new("x.ai/queue/changed", serde_json::value::to_raw_value(&params).unwrap().into()),
        response_tx,
    }), app);
}

fn app_with_row(wake: bool) -> AppView {
    let mut app = test_app_with_agent();
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.session.state = AgentState::TurnRunning;
    agent.session.current_prompt_id = Some("running-input".into());
    agent.front_message_committed = true;
    if wake {
        agent.session.state = AgentState::Idle;
        agent.session.current_prompt_id = None;
        agent.running_wake_turn = Some(crate::app::agent_view::RunningWakeTurn {
            prompt_id: "task-completed-running".into(), cancelling: false,
        });
    }
    broadcast(&mut app, vec![row(3)], Some(if wake { "task-completed-running" } else { "running-input" }));
    app.agents.get_mut(&AgentId(0)).unwrap().active_pane = ActivePane::Prompt;
    app
}

fn enter(app: &mut AppView) -> Vec<Effect> {
    match app.handle_input(&Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))) {
        InputOutcome::Action(action) => dispatch(action, app),
        InputOutcome::Changed | InputOutcome::Unchanged => Vec::new(),
        outcome @ (InputOutcome::ActionThenForward(_) | InputOutcome::ActionPair(_, _) | InputOutcome::ArmPending { .. }) => {
            panic!("unexpected input outcome: {outcome:?}")
        }
    }
}

fn request(effects: &[Effect]) -> (QueueRequest, u64) {
    match effects {
        [Effect::QueueInterject { request, binding_epoch }] => (request.clone(), *binding_epoch),
        other => panic!("expected correlated admission: {other:?}"),
    }
}

fn complete(app: &mut AppView, operation: QueueOperation, epoch: u64, result: Result<QueueResponse, QueueTransport>) {
    dispatch(Action::TaskComplete(TaskResult::QueueInterjectComplete { operation, binding_epoch: epoch, result }), app);
}

fn answer(app: &mut AppView, request: &QueueRequest, epoch: u64, result: QueueResult) {
    complete(app, request.operation.clone(), epoch, Ok(QueueResponse { operation: request.operation.clone(), result }));
}

fn rendered_queue(app: &mut AppView) -> String {
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.sync_queue_pane();
    let area = Rect::new(0, 0, 160, 4);
    let mut buffer = Buffer::empty(area);
    agent.queue.render(area, &mut buffer, false, &crate::appearance::LayoutConfig::default(), None, true);
    (0..area.height).flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .map(|(x, y)| buffer[(x, y)].symbol().to_owned()).collect()
}

fn assert_unsent(app: &AppView) {
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert!(agent.expect_send_now_cancel.is_none());
    assert!(!agent.send_now_painted_blocks.contains_key("queued-input"));
    assert!(!(0..agent.scrollback.len()).filter_map(|i| agent.scrollback.entry(i)).any(|entry| {
        matches!(&entry.block, RenderBlock::UserPrompt(block) if block.text == "held exact payload")
    }));
}

#[test]
fn empty_enter_keeps_pending_row_visible_for_committed_and_wake_turns() {
    for wake in [false, true] {
        let mut app = app_with_row(wake);
        let (sent, _) = request(&enter(&mut app));
        assert_eq!(sent.operation.id, "queued-input");
        assert_eq!(sent.operation.expected_version, 3);
        assert_unsent(&app);
        let rendered = rendered_queue(&mut app);
        assert!(rendered.contains("send now: pending"), "{rendered}");
        assert!(rendered.contains("held exact payload"), "{rendered}");
        assert!(enter(&mut app).is_empty(), "pending Enter must not resend");
        broadcast(&mut app, Vec::new(), Some("running-input"));
        assert!(rendered_queue(&mut app).contains("held exact payload"));
        assert_unsent(&app);
    }
}

#[test]
fn rejection_and_transport_are_visible_without_a_sent_bubble() {
    for reason in [QueueRejection::Stale, QueueRejection::NotOwner, QueueRejection::Protected, QueueRejection::Missing, QueueRejection::Running, QueueRejection::Unauthorized, QueueRejection::Capacity, QueueRejection::PayloadTooLarge] {
        let mut app = app_with_row(false);
        let (sent, epoch) = request(&enter(&mut app));
        answer(&mut app, &sent, epoch, QueueResult::Rejected { reason: reason.clone(), row: Some(row(3)) });
        let rendered = rendered_queue(&mut app);
        assert!(rendered.contains(reason.notice()), "{rendered}");
        assert!(rendered.contains("held exact payload"));
        assert_unsent(&app);
    }
    for failure in [QueueTransport::Disconnected, QueueTransport::InvalidReply, QueueTransport::Encode] {
        let mut app = app_with_row(false);
        let (sent, epoch) = request(&enter(&mut app));
        complete(&mut app, sent.operation.clone(), epoch, Err(failure));
        assert!(rendered_queue(&mut app).contains("unconfirmed"));
        assert_unsent(&app);
        let (observe, _) = request(&enter(&mut app));
        assert_eq!(observe.operation, sent.operation);
        assert_eq!(observe.action, QueueAction::Observe);
        assert!(enter(&mut app).is_empty());
    }
}

#[test]
fn queued_acceptance_stays_visible_and_observes_the_same_operation() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::Queued { row: row(3) });
    assert!(rendered_queue(&mut app).contains("queued, awaiting start"));
    assert_unsent(&app);
    answer(&mut app, &sent, epoch, QueueResult::Rejected { reason: QueueRejection::Missing, row: None });
    assert!(rendered_queue(&mut app).contains("queued, awaiting start"), "late duplicate cannot regress acceptance");
    let (observe, _) = request(&enter(&mut app));
    assert_eq!(observe.operation, sent.operation);
    assert_eq!(observe.action, QueueAction::Observe);
}

#[test]
fn late_results_cannot_replace_new_versions_operations_or_bindings() {
    let mut app = app_with_row(false);
    let (old, epoch) = request(&enter(&mut app));
    let mut newer = row(4);
    newer.text = "new authoritative text".into();
    broadcast(&mut app, vec![newer.clone()], Some("running-input"));
    answer(&mut app, &old, epoch, QueueResult::Queued { row: row(3) });
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue[0], newer);
    let (new, _) = request(&enter(&mut app));
    assert_ne!(new.operation.operation_id, old.operation.operation_id);
    answer(&mut app, &old, epoch, QueueResult::GoalInterjection { running_prompt_id: "running-input".into() });
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue[0], newer);
    app.agents.get_mut(&AgentId(0)).unwrap().session_binding_epoch += 1;
    answer(&mut app, &new, epoch, QueueResult::Started { prompt_id: "queued-input".into() });
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue[0], newer);
    app.agents.get_mut(&AgentId(0)).unwrap().session.session_id = Some(acp::SessionId::new("another-session"));
    answer(&mut app, &new, epoch + 1, QueueResult::Started { prompt_id: "queued-input".into() });
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue[0], newer);
}

#[test]
fn edited_pending_text_is_visible_before_and_after_rejection() {
    let mut app = app_with_row(false);
    let effects = dispatch(Action::QueueInterjectShared {
        id: "queued-input".into(), expected_version: 3, new_text: Some("edited exact payload".into()),
    }, &mut app);
    let (sent, epoch) = request(&effects);
    assert!(rendered_queue(&mut app).contains("edited exact payload"));
    answer(&mut app, &sent, epoch, QueueResult::Rejected { reason: QueueRejection::Stale, row: Some(row(3)) });
    assert!(rendered_queue(&mut app).contains("edited exact payload"));
    assert_unsent(&app);
}

#[test]
fn authoritative_start_adopts_once_and_late_admission_cannot_resurrect_the_row() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::Queued { row: row(3) });
    assert_unsent(&app);
    broadcast(&mut app, Vec::new(), Some("queued-input"));
    dispatch(Action::TaskComplete(TaskResult::PromptResponse {
        agent_id: AgentId(0), result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        http_status: None, prompt_id: Some("running-input".into()),
    }), &mut app);
    broadcast(&mut app, Vec::new(), Some("queued-input"));
    answer(&mut app, &sent, epoch, QueueResult::Queued { row: row(3) });
    let agent = app.agents.get(&AgentId(0)).unwrap();
    assert!(agent.shared_queue.is_empty());
    assert!(!agent.queue_operations.contains_key("queued-input"));
    assert_eq!(agent.session.current_prompt_id.as_deref(), Some("queued-input"));
    assert_eq!((0..agent.scrollback.len()).filter_map(|i| agent.scrollback.entry(i)).filter(|entry| {
        matches!(&entry.block, RenderBlock::UserPrompt(block) if block.text == "held exact payload")
    }).count(), 1);
}

#[tokio::test]
async fn closed_rpc_transport_returns_a_visible_unconfirmed_result() {
    let mut app = app_with_row(false);
    let effects = enter(&mut app);
    let (sent, epoch) = request(&effects);
    let effect = effects.into_iter().next().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    drop(rx);
    let (progress, _) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = tokio::task::JoinSet::new();
    crate::app::effects::execute(
        effect, &mut tasks, &tx, std::path::Path::new("/tmp"),
        &crate::app::effects::SessionFlags::default(), &progress, None,
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), tasks.join_next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&result, TaskResult::QueueInterjectComplete {
        operation, binding_epoch, result: Err(QueueTransport::Disconnected),
    } if operation == &sent.operation && *binding_epoch == epoch));
    dispatch(Action::TaskComplete(result), &mut app);
    let rendered = rendered_queue(&mut app);
    assert!(rendered.contains("transport failed"));
    assert!(rendered.contains("held exact payload"));
    assert_unsent(&app);
}

#[tokio::test]
async fn queue_effect_uses_correlated_rpc_and_decodes_owner_rejection() {
    let mut app = app_with_row(false);
    let effects = enter(&mut app);
    let (sent, _) = request(&effects);
    let effect = effects.into_iter().next().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (progress, _) = tokio::sync::mpsc::unbounded_channel();
    let mut tasks = tokio::task::JoinSet::new();
    crate::app::effects::execute(
        effect, &mut tasks, &tx, std::path::Path::new("/tmp"),
        &crate::app::effects::SessionFlags::default(), &progress, None,
    );
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    let xai_acp_lib::AcpAgentMessage::ExtMethod(args) = message else {
        panic!("queue admission requires a reply, not a notification");
    };
    assert_eq!(args.request.method.as_ref(), xai_grok_shell::extensions::interject::QUEUE_INTERJECT_METHOD);
    assert_eq!(serde_json::from_str::<QueueRequest>(args.request.params.get()).unwrap(), sent);
    let response = QueueResponse {
        operation: sent.operation.clone(),
        result: QueueResult::Rejected { reason: QueueRejection::NotOwner, row: Some(row(3)) },
    };
    assert!(args.response_tx.send(Ok(acp::ExtResponse::new(serde_json::value::to_raw_value(&response).unwrap().into()))).is_ok());
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), tasks.join_next()).await.unwrap().unwrap().unwrap();
    dispatch(Action::TaskComplete(result), &mut app);
    let rendered = rendered_queue(&mut app);
    assert!(rendered.contains("send now rejected: row belongs to another owner"));
    assert!(rendered.contains("held exact payload"));
    assert_unsent(&app);
}

#[test]
fn mismatched_started_identity_preserves_the_pending_payload() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::Started { prompt_id: "unrelated-input".into() });
    let rendered = rendered_queue(&mut app);
    assert!(rendered.contains("invalid reply"), "{rendered}");
    assert!(rendered.contains("held exact payload"));
    assert_unsent(&app);
    let (observe, _) = request(&enter(&mut app));
    assert_eq!(observe.operation, sent.operation);
    assert_eq!(observe.action, QueueAction::Observe);
}

#[test]
fn unconfirmed_reply_preserves_text_and_observes_without_resending() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::Unconfirmed);
    let rendered = rendered_queue(&mut app);
    assert!(rendered.contains("send now: unconfirmed"), "{rendered}");
    assert!(rendered.contains("held exact payload"));
    assert_unsent(&app);
    let (observe, _) = request(&enter(&mut app));
    assert_eq!(observe.operation, sent.operation);
    assert_eq!(observe.action, QueueAction::Observe);
}

#[test]
fn goal_acceptance_fences_delayed_snapshots_without_hiding_new_versions() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::GoalInterjection { running_prompt_id: "running-input".into() });
    assert!(app.agents.get(&AgentId(0)).unwrap().shared_queue.is_empty());
    broadcast(&mut app, vec![row(3)], Some("running-input"));
    assert!(app.agents.get(&AgentId(0)).unwrap().shared_queue.is_empty());
    let mut newer = row(4);
    newer.text = "new authoritative text".into();
    broadcast(&mut app, vec![newer.clone()], Some("running-input"));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue, vec![newer.clone()]);
    assert!(rendered_queue(&mut app).contains("new authoritative text"));
    broadcast(&mut app, vec![row(3)], Some("running-input"));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue, vec![newer.clone()]);
    assert_eq!(app.shared_prompt_queue("test-session"), Some(&vec![newer]));
    assert!(rendered_queue(&mut app).contains("new authoritative text"));
    assert_unsent(&app);
}

#[test]
fn queued_row_disappearance_is_unconfirmed_not_started_or_lost() {
    let mut app = app_with_row(false);
    let (sent, epoch) = request(&enter(&mut app));
    answer(&mut app, &sent, epoch, QueueResult::Queued { row: row(3) });
    broadcast(&mut app, Vec::new(), Some("running-input"));
    let rendered = rendered_queue(&mut app);
    assert!(rendered.contains("send now: unconfirmed"), "{rendered}");
    assert!(rendered.contains("held exact payload"));
    assert_unsent(&app);
    let (observe, _) = request(&enter(&mut app));
    assert_eq!(observe.operation, sent.operation);
    assert_eq!(observe.action, QueueAction::Observe);
}


#[test]
fn pending_and_queued_late_responses_cannot_regress_a_newer_snapshot() {
    for queued in [false, true] {
        let mut app = app_with_row(false);
        let (sent, epoch) = request(&enter(&mut app));
        if queued {
            answer(&mut app, &sent, epoch, QueueResult::Queued { row: row(3) });
            let (observe, _) = request(&enter(&mut app));
            assert_eq!(observe.action, QueueAction::Observe);
        }
        let mut newer = row(4);
        newer.text = "version four exact payload: α\nsecond line".into();
        broadcast(&mut app, vec![newer.clone()], Some("running-input"));
        broadcast(&mut app, vec![row(3)], Some("running-input"));
        answer(&mut app, &sent, epoch, QueueResult::GoalInterjection { running_prompt_id: "running-input".into() });
        assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue, vec![newer.clone()]);
        assert_eq!(app.shared_prompt_queue("test-session"), Some(&vec![newer]));
        assert!(rendered_queue(&mut app).contains("version four exact payload: α"));
        assert_unsent(&app);
    }
}

#[test]
fn snapshot_version_fence_does_not_prevent_removal_start_or_rebind() {
    let mut app = app_with_row(false);
    let mut newer = row(4);
    newer.text = "version four".into();
    broadcast(&mut app, vec![newer.clone()], Some("running-input"));
    broadcast(&mut app, Vec::new(), Some("running-input"));
    assert!(app.shared_prompt_queue("test-session").is_none());
    assert!(app.agents.get(&AgentId(0)).unwrap().shared_queue.is_empty());
    broadcast(&mut app, vec![newer.clone()], Some("running-input"));
    broadcast(&mut app, Vec::new(), Some("queued-input"));
    assert!(app.shared_prompt_queue("test-session").is_none());
    assert!(app.agents.get(&AgentId(0)).unwrap().shared_queue.is_empty());
    broadcast(&mut app, vec![newer], Some("running-input"));
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    agent.bind_session_id(acp::SessionId::new("another-session"));
    agent.bind_session_id(acp::SessionId::new("test-session"));
    broadcast(&mut app, vec![row(1)], Some("running-input"));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue, vec![row(1)]);
    assert_eq!(app.shared_prompt_queue("test-session"), Some(&vec![row(1)]));
}


#[test]
fn accepted_atomic_edit_advances_the_existing_snapshot_mirror() {
    let mut app = app_with_row(false);
    let effects = dispatch(Action::QueueInterjectShared {
        id: "queued-input".into(), expected_version: 3, new_text: Some("accepted exact edit".into()),
    }, &mut app);
    let (sent, epoch) = request(&effects);
    let mut edited = row(4);
    edited.text = "accepted exact edit".into();
    answer(&mut app, &sent, epoch, QueueResult::Queued { row: edited.clone() });
    assert_eq!(app.shared_prompt_queue("test-session"), Some(&vec![edited.clone()]));
    broadcast(&mut app, vec![row(3)], Some("running-input"));
    assert_eq!(app.agents.get(&AgentId(0)).unwrap().shared_queue, vec![edited.clone()]);
    assert_eq!(app.shared_prompt_queue("test-session"), Some(&vec![edited]));
    assert!(rendered_queue(&mut app).contains("accepted exact edit"));
    assert_unsent(&app);
}
