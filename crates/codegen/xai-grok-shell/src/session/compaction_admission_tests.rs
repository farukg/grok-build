use super::super::*;
use super::super::support::{create_test_actor_ex, running_task_stub};
use crate::session::{CancelOptions, CancelTrigger, SessionCommand};
use crate::extensions::notification::SessionUpdate as XaiSessionUpdate;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::{mpsc, oneshot};

#[path = "compaction_teardown_tests.rs"]
mod teardown_tests;
#[path = "compaction_correlation_tests.rs"]
mod correlation_tests;
#[path = "compaction_boundary_tests.rs"]
mod boundary_tests;
#[path = "injected_sampler_loop_tests.rs"]
mod injected_sampler_loop_tests;

struct Fixture {
    actor: Arc<SessionActor>,
    sender: mpsc::UnboundedSender<SessionCommand>,
    persistence: mpsc::UnboundedReceiver<PersistenceMsg>,
    updates: Rc<RefCell<Vec<XaiSessionUpdate>>>,
    updates_changed: Rc<tokio::sync::Notify>,
    _task: tokio_util::task::AbortOnDropHandle<()>,
}

async fn fixture() -> Fixture {
    let (gateway_tx, mut gateway_rx) = mpsc::unbounded_channel();
    let updates = Rc::new(RefCell::new(Vec::new()));
    let captured = updates.clone();
    let updates_changed = Rc::new(tokio::sync::Notify::new());
    let changed = updates_changed.clone();
    tokio::task::spawn_local(async move {
        while let Some(message) = gateway_rx.recv().await {
            match message {
                xai_acp_lib::AcpClientMessage::SessionNotification(args) => {
                    let _ = args.response_tx.send(Ok(()));
                }
                xai_acp_lib::AcpClientMessage::ExtNotification(args) => {
                    if args.request.method.as_ref() == "x.ai/session_notification" {
                        let params: serde_json::Value =
                            serde_json::from_str(args.request.params.get()).unwrap();
                        captured.borrow_mut().push(serde_json::from_value(params["update"].clone()).unwrap());
                        changed.notify_one();
                    }
                }
                _ => {}
            }
        }
    });
    let (persistence_tx, persistence) = mpsc::unbounded_channel();
    let (mut actor, events) =
        create_test_actor_ex(50_000, 200_000, 85, gateway_tx, persistence_tx).await;
    let sampling_events = wire_sampler(&mut actor).await;
    let actor = Arc::new(actor);
    actor.chat_state_handle.replace_conversation(vec![
        ConversationItem::system("sys"),
        ConversationItem::user("original task"),
        ConversationItem::assistant("Completed original work. ".repeat(40)),
        ConversationItem::user("final query"),
    ]);
    let (sender, task) = start_fixture_actor(actor.clone(), events, sampling_events);
    Fixture {
        actor,
        sender,
        persistence,
        updates,
        updates_changed,
        _task: task,
    }
}

async fn wire_sampler(actor: &mut SessionActor) -> mpsc::UnboundedReceiver<xai_grok_sampler::SamplingEvent> {
    let (sender, events) = mpsc::unbounded_channel();
    actor.sampler_handle = xai_grok_sampler::SamplerActor::spawn(
        actor.reconstruct_full_config().await,
        xai_grok_sampler::RetryPolicy { max_retries: 0, ..Default::default() },
        sender,
    );
    events
}

fn start_fixture_actor(
    actor: Arc<SessionActor>,
    events: mpsc::UnboundedReceiver<SessionEvent>,
    mut sampling_events: mpsc::UnboundedReceiver<xai_grok_sampler::SamplingEvent>,
) -> (mpsc::UnboundedSender<SessionCommand>, tokio_util::task::AbortOnDropHandle<()>) {
    let sampling_actor = Arc::downgrade(&actor);
    tokio::task::spawn_local(async move {
        while let Some(event) = sampling_events.recv().await {
            let Some(actor) = sampling_actor.upgrade() else { break };
            actor.handle_sampling_event(event).await;
        }
    });
    let (sender, commands) = mpsc::unbounded_channel();
    let (chat_sender, chat_events) = mpsc::unbounded_channel();
    let task = tokio::task::spawn_local(async move {
        let _chat_sender = chat_sender;
        run_session(
            actor, commands, chat_events, events, None,
            Arc::new(parking_lot::Mutex::new(xai_grok_workspace::file_system::CodebaseIndexManager::new())),
            std::path::PathBuf::from("/tmp"),
            crate::session::fs_watch::FsWatchCapabilities::none(),
        ).await;
    });
    (sender, tokio_util::task::AbortOnDropHandle::new(task))
}

async fn route_to_mock(actor: &SessionActor) -> xai_grok_test_support::MockInferenceServer {
    let server = xai_grok_test_support::MockInferenceServer::start().await.unwrap();
    server.set_response("Summary of completed original work; preserve the task and findings. ".repeat(30));
    let mut config = actor.chat_state_handle.get_sampling_config().await.unwrap();
    config.base_url = server.url();
    actor.chat_state_handle.update_sampling_config(config);
    server
}

fn submit_compact(fixture: &Fixture) -> oneshot::Receiver<acp::Result<()>> {
    let (respond_to, response) = oneshot::channel();
    fixture.sender.send(SessionCommand::CompactSession {
        invocation_id: None,
        cancellation: tokio_util::sync::CancellationToken::new(),
        respond_to,
    }).unwrap();
    response
}

fn submit_input(fixture: &Fixture) -> oneshot::Receiver<PromptTurnResult> {
    submit_named_input(fixture, "input-during-compact", true)
}

fn submit_named_input(fixture: &Fixture, id: &str, send_now: bool) -> oneshot::Receiver<PromptTurnResult> {
    send_named_input(&fixture.sender, id, send_now)
}

fn send_named_input(sender: &mpsc::UnboundedSender<SessionCommand>, id: &str, send_now: bool) -> oneshot::Receiver<PromptTurnResult> {
    let (respond_to, response) = oneshot::channel();
    sender.send(SessionCommand::Prompt {
        prompt_id: id.into(),
        prompt_blocks: vec![acp::ContentBlock::Text(acp::TextContent::new("new input"))],
        prompt_mode: PromptMode::Agent,
        artifact_upload_ctx: None,
        client_identifier: None,
        screen_mode: None,
        verbatim: false,
        traceparent: None,
        json_schema: None,
        send_now,
        admission: None,
        tool_overrides_update: None,
        respond_to,
        prompt_admitted: None,
        persist_ack: None,
        parsed_prompt_tx: None,
    }).unwrap();
    response
}

async fn command_boundary(fixture: &Fixture) {
    let (respond_to, response) = oneshot::channel();
    fixture.sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
    response.await.unwrap();
}

async fn compact_barrier(fixture: &mut Fixture) -> oneshot::Sender<std::io::Result<()>> {
    let mut checkpoint_seen = false;
    loop {
        match fixture.persistence.recv().await.unwrap() {
            PersistenceMsg::CompactionCheckpoint(_) => checkpoint_seen = true,
            PersistenceMsg::FlushAndAck { respond_to } => {
                assert!(checkpoint_seen, "the compact checkpoint precedes its durability barrier");
                return respond_to;
            }
            _ => {}
        }
    }
}

async fn foreground_parked(fixture: &mut Fixture, server: &xai_grok_test_support::MockInferenceServer) {
    loop {
        tokio::select! {
            () = server.wait_until_parked_replies(1) => return,
            message = fixture.persistence.recv() => match message.unwrap() {
                PersistenceMsg::FlushAndAck { respond_to } | PersistenceMsg::ProbeWritable { respond_to } => {
                    respond_to.send(Ok(())).unwrap();
                }
                PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } => {
                    respond_to.send(Ok(())).unwrap();
                }
                _ => {}
            },
        }
    }
}

async fn queued_compact_barrier(fixture: &mut Fixture) -> oneshot::Sender<std::io::Result<()>> {
    let mut checkpoint_seen = false;
    loop {
        match fixture.persistence.recv().await.unwrap() {
            PersistenceMsg::CompactionCheckpoint(_) => checkpoint_seen = true,
            PersistenceMsg::FlushAndAck { respond_to } if checkpoint_seen => return respond_to,
            PersistenceMsg::FlushAndAck { respond_to } | PersistenceMsg::ProbeWritable { respond_to } => {
                respond_to.send(Ok(())).unwrap();
            }
            PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } => {
                respond_to.send(Ok(())).unwrap();
            }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn actor_retains_busy_compact_and_executes_original_request_in_input_order() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut fixture = fixture().await;
            let server = route_to_mock(&fixture.actor).await;
            server.set_foreground_hold();
            let foreground = submit_named_input(&fixture, "foreground", false);
            foreground_parked(&mut fixture, &server).await;
            let earlier = submit_named_input(&fixture, "before-compact", false);
            let mut compact = submit_compact(&fixture);
            let _later = submit_named_input(&fixture, "after-compact", false);
            command_boundary(&fixture).await;
            assert_eq!(fixture.actor.state.lock().await.running_task.as_ref().unwrap().prompt_id, "foreground");
            assert!(fixture.actor.has_pending_compact());
            assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            while !fixture.updates.borrow().iter().any(|update| matches!(update,
                XaiSessionUpdate::AutoCompactStarted {
                    phase: crate::extensions::notification::CompactPhase::Queued, ..
                }))) {
                fixture.updates_changed.notified().await;
            }

            server.release_parked_replies();
            let persisted = queued_compact_barrier(&mut fixture).await;
            foreground.await.unwrap().expect("foreground turn completes before maintenance");
            earlier.await.unwrap().expect("earlier input completes before maintenance");
            {
                let state = fixture.actor.state.lock().await;
                assert!(state.running_task.is_none(), "compaction owns an exclusive conversation boundary");
                assert!(state.pending_inputs.iter().any(|input| input.prompt_id == "after-compact"));
            }
            assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
                "the original retained reply cannot precede its own durability boundary");
            persisted.send(Ok(())).unwrap();
            compact.await.unwrap().expect("the original command runs automatically without resubmission");
            assert!(!fixture.actor.has_pending_compact());
        }).await.expect("queued compact must advance automatically at the preceding turn boundary");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn pending_compact_abort_and_shutdown_settle_the_original_responder() {
    enum Stop {
        Manual,
        Management,
        Shutdown,
    }
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for stop in [Stop::Manual, Stop::Management, Stop::Shutdown] {
                let fixture = fixture().await;
                fixture.actor.state.lock().await.running_task = Some(running_task_stub("foreground-child-wait"));
                let cancellation = tokio_util::sync::CancellationToken::new();
                let (respond_to, response) = oneshot::channel();
                fixture.sender.send(SessionCommand::CompactSession {
                    invocation_id: None,
                    cancellation: cancellation.clone(),
                    respond_to,
                }).unwrap();
                command_boundary(&fixture).await;
                assert!(fixture.actor.has_pending_compact());
                match stop {
                    Stop::Manual => fixture.sender.send(SessionCommand::Cancel(CancelOptions {
                        trigger: Some(CancelTrigger::CtrlC),
                        user_initiated: true,
                        ..Default::default()
                    })).unwrap(),
                    Stop::Management => cancellation.cancel(),
                    Stop::Shutdown => fixture.sender.send(SessionCommand::Shutdown(crate::session::ShutdownKind::Graceful)).unwrap(),
                }
                let error = response.await.unwrap().unwrap_err();
                assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                    Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
                assert!(!fixture.actor.has_pending_compact(), "a stopped queued operation cannot leave a ghost queue");
            }
        }).await.expect("pending compact responders must settle without waiting for the busy turn");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actor_new_input_preserves_compact_until_its_own_durable_ack() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut fixture = fixture().await;
            let server = route_to_mock(&fixture.actor).await;
            server.set_auxiliary_hold();
            let mut response = submit_compact(&fixture);
            server.wait_until_parked_replies(1).await;

            let _new_input = submit_input(&fixture);
            command_boundary(&fixture).await;
            assert!(fixture.actor.state.lock().await.pending_inputs.iter().any(|input| input.prompt_id == "input-during-compact"));
            assert!(!fixture.actor.compaction.cancel.is_cancelled());
            assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));

            server.release_parked_replies();
            let persisted = compact_barrier(&mut fixture).await;
            assert!(fixture.actor.state.lock().await.running_task.is_none());
            assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
                "success cannot precede the compact's durable acknowledgement");
            persisted.send(Ok(())).unwrap();
            response.await.unwrap().expect("incidental cancellation must not abort compact");
            let state = fixture.actor.state.lock().await;
            assert!(state.running_task.as_ref().is_some_and(|task| task.prompt_id == "input-during-compact")
                || state.pending_inputs.iter().any(|input| input.prompt_id == "input-during-compact"),
                "new input must retain its ordinary turn owner after the compact reply");
        }).await.expect("compact must resolve without the unrelated turn's 30-second flush barrier");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actor_manual_stop_aborts_compact_once_and_a_later_compact_can_complete() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut fixture = fixture().await;
            let server = route_to_mock(&fixture.actor).await;
            server.set_auxiliary_hold();
            let response = submit_compact(&fixture);
            server.wait_until_parked_replies(1).await;
            for _ in 0..2 {
                fixture.sender.send(SessionCommand::Cancel(CancelOptions {
                    trigger: Some(CancelTrigger::CtrlC),
                    user_initiated: true,
                    ..Default::default()
                })).unwrap();
            }
            let error = response.await.unwrap().unwrap_err();
            assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
            command_boundary(&fixture).await;
            server.release_parked_replies();
            let response = submit_compact(&fixture);
            compact_barrier(&mut fixture).await.send(Ok(())).unwrap();
            response.await.unwrap().expect("an old manual abort must not poison the next operation");
            while !fixture.updates.borrow().iter().any(|update| matches!(update, XaiSessionUpdate::AutoCompactCancelled { .. })) {
                fixture.updates_changed.notified().await;
            }
            assert_eq!(fixture.updates.borrow().iter().filter(|update| matches!(update,
                XaiSessionUpdate::AutoCompactCancelled { .. })).count(), 1,
                "repeated manual stop must publish one cancellation for the owned compact");
        }).await.expect("manual stop must settle exactly one operation and release its cancellation scope");
    }).await;
}
