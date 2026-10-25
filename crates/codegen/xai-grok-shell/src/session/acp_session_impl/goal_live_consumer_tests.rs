use super::*;
use super::super::support;
use crate::session::completion_delivery::{
    CompletionArtifacts, CompletionEventPolicy, CompletionWake, CompletionWakeKind,
    SubagentCompletionEvent,
};
use crate::session::goal_tracker::{GoalClassifierVerdict, GoalStatus};
use xai_grok_tools::implementations::grok_build::task::types::{
    SubagentCancelOutcome, SubagentEvent, SubagentResult, SubagentState,
};

struct EvaluatorHttpRequest {
    body: serde_json::Value,
    response: oneshot::Sender<String>,
}

struct LiveGoalFixture {
    actor: Arc<SessionActor>,
    commands: mpsc::UnboundedSender<SessionCommand>,
    requests: mpsc::UnboundedReceiver<EvaluatorHttpRequest>,
    verifications: Arc<std::sync::atomic::AtomicUsize>,
    _server: tokio_util::task::AbortOnDropHandle<()>,
    _session: tokio_util::task::AbortOnDropHandle<()>,
    _coordinator: tokio_util::task::AbortOnDropHandle<()>,
}

fn completion_stream(text: &str) -> String {
    let content = serde_json::json!({
        "id": "goal-test", "object": "chat.completion.chunk", "created": 1,
        "model": "test", "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]
    });
    let stop = serde_json::json!({
        "id": "goal-test", "object": "chat.completion.chunk", "created": 1,
        "model": "test", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    });
    format!("data: {content}\n\ndata: {stop}\n\ndata: [DONE]\n\n")
}

async fn live_goal_fixture() -> LiveGoalFixture {
    let (request_tx, requests) = mpsc::unbounded_channel::<EvaluatorHttpRequest>();
    let router = axum::Router::new().fallback(move |axum::Json(body): axum::Json<serde_json::Value>| {
        let request_tx = request_tx.clone();
        async move {
            let (response, receive) = oneshot::channel();
            request_tx.send(EvaluatorHttpRequest { body, response }).unwrap();
            let text = receive.await.unwrap_or_default();
            ([(axum::http::header::CONTENT_TYPE, "text/event-stream")], completion_stream(&text))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    support::drain_gateway(gateway_rx);
    let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
        0, 256_000, 85, gateway_tx,
    ).await;
    actor.goal_enabled = true;
    actor.background_workflows_enabled = true;
    actor.goal_classifier_enabled = true;
    actor.goal_use_current_model_only = true;
    actor.goal_verifier_skeptic_count = 1;
    actor.goal_planner_enabled = false;
    actor.goal_summary_enabled = false;
    actor.goal_plan_reconciled.store(true, std::sync::atomic::Ordering::Relaxed);
    actor.goal_harness_enabled.store(true, std::sync::atomic::Ordering::Relaxed);
    let agent = support::test_agent_with_goal_tool().await;
    *actor.agent.borrow_mut() = agent;
    let mut config = actor.chat_state_handle.get_sampling_config().await.unwrap();
    config.base_url = format!("http://{address}/v1");
    config.api_backend = xai_grok_sampling_types::ApiBackend::ChatCompletions;
    config.max_retries = Some(0);
    actor.chat_state_handle.update_sampling_config(config);
    actor.goal_tracker.lock().create_goal(
        "goal-A".to_string(), "objective-A".to_string(), None, 0,
        chrono::Utc::now().to_rfc3339(), None,
    );
    let verifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = verifications.clone();
    let (subagent_tx, mut subagents) = mpsc::unbounded_channel();
    actor.tool_context.subagent_event_tx = Some(subagent_tx);
    let coordinator = tokio::task::spawn_local(async move {
        while let Some(event) = subagents.recv().await {
            match event {
                SubagentEvent::Spawn(request) => {
                    observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    request.result_tx.send(SubagentResult {
                        state: SubagentState::Completed,
                        output: "Not Refuted".into(),
                        ..Default::default()
                    }).unwrap();
                }
                SubagentEvent::PeekCompletions(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::ListActive(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::ListRunning(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::Cancel(request) => { let _ = request.respond_to.send(SubagentCancelOutcome::NotFound); }
                SubagentEvent::Query(request) => { let _ = request.respond_to.send(None); }
                SubagentEvent::ListOwned(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::Outstanding(request) => { let _ = request.respond_to.send(Default::default()); }
                SubagentEvent::WaitPromptDrained(request) => { let _ = request.respond_to.send(Default::default()); }
                SubagentEvent::MarkUsageNotApplied(request) => { let _ = request.respond_to.send(()); }
                SubagentEvent::RegistryCounts(request) => { let _ = request.respond_to.send(Default::default()); }
                SubagentEvent::Inspect(request) => { let _ = request.respond_to.send(None); }
                SubagentEvent::HandOffForeground(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::SpawnedRefs(request) => { let _ = request.respond_to.send(Vec::new()); }
                SubagentEvent::TeardownSession { respond_to, .. } => {
                    if let Some(respond_to) = respond_to { let _ = respond_to.send(()); }
                }
                SubagentEvent::AcknowledgeCompletions { .. }
                | SubagentEvent::OpenSpawnAdmission { .. }
                | SubagentEvent::ClearUsageNotApplied(_) => {}
                SubagentEvent::ValidateType(_)
                | SubagentEvent::DescribeType(_) | SubagentEvent::LoopUnitActive(_)
                | SubagentEvent::ResolveResume(_) => panic!("unexpected coordinator operation in live goal fixture"),
            }
        }
    });
    let actor = Arc::new(actor);
    let (commands, receive) = mpsc::unbounded_channel();
    let running = actor.clone();
    let session = tokio::task::spawn_local(async move {
        let (_chat_sender, chat_events) = mpsc::unbounded_channel();
        super::super::run_session(
            running, receive, chat_events, events, None,
            Arc::new(parking_lot::Mutex::new(xai_grok_workspace::file_system::CodebaseIndexManager::new())),
            std::path::PathBuf::from("/tmp"), crate::session::fs_watch::FsWatchCapabilities::none(),
        ).await;
    });
    LiveGoalFixture {
        actor, commands, requests, verifications,
        _server: tokio_util::task::AbortOnDropHandle::new(server),
        _session: tokio_util::task::AbortOnDropHandle::new(session),
        _coordinator: tokio_util::task::AbortOnDropHandle::new(coordinator),
    }
}

impl LiveGoalFixture {
    async fn fifo(&self) {
        let (respond_to, receive) = oneshot::channel();
        self.commands.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
        receive.await.unwrap();
    }

    fn prompt(&self, id: &str, text: &str) -> oneshot::Receiver<PromptTurnResult> {
        let (respond_to, receive) = oneshot::channel();
        self.commands.send(SessionCommand::Prompt {
            prompt_id: id.to_string(), prompt_blocks: vec![acp::ContentBlock::from(text.to_string())],
            prompt_mode: PromptMode::Agent, artifact_upload_ctx: None, client_identifier: None,
            screen_mode: None, verbatim: false, traceparent: None, json_schema: None,
            send_now: false, admission: None, tool_overrides_update: None, respond_to,
            prompt_admitted: None, persist_ack: None, parsed_prompt_tx: None,
        }).unwrap();
        receive
    }

    async fn child(&self, id: &str) -> (CompletionWake, oneshot::Receiver<PromptTurnResult>) {
        let completion = CompletionWake {
            id: xai_chat_state::SubagentCompletionId {
                subagent_id: id.to_string(), attempt_id: xai_message_delivery_core::AttemptId::mint(1),
            },
            kind: CompletionWakeKind::Fresh,
        };
        let event = SubagentCompletionEvent {
            id: completion.id.clone(), state: SubagentState::Completed,
            text: format!("{id}: delivered child evidence"), output: "delivered child evidence".to_string(),
            child_session_id: id.to_string(), tool_calls: 0, turns: 1, duration_ms: 1,
            tokens_used: 1, completed_at: chrono::Utc::now(), policy: CompletionEventPolicy::Automatic,
            artifacts: CompletionArtifacts::None,
        };
        let (respond_to, receive) = oneshot::channel();
        self.actor.notifications.persistence_tx.send(PersistenceMsg::PrepareSubagentCompletion { event, respond_to }).unwrap();
        let prepared = receive.await.unwrap().unwrap();
        let (respond_to, receive) = oneshot::channel();
        self.commands.send(SessionCommand::PublishSubagentCompletion {
            event: prepared.event, will_wake: true, respond_to,
        }).unwrap();
        let published = receive.await.unwrap().unwrap();
        let (respond_to, receive) = oneshot::channel();
        self.commands.send(SessionCommand::SubagentCompletionWake {
            prompt_id: format!("subagent-completed-{id}/{}", completion.id.attempt_id),
            completion: completion.clone(), text: published.text, respond_to,
        }).unwrap();
        (completion, receive)
    }

    async fn evaluated_child(&mut self, id: &str) -> (CompletionWake, oneshot::Receiver<PromptTurnResult>, EvaluatorHttpRequest) {
        let (completion, receive) = self.child(id).await;
        let worker = self.requests.recv().await.unwrap();
        assert!(!worker.body.to_string().contains("hidden completion evaluator"));
        assert!(worker.body.to_string().contains(id), "the worker request must contain the durably projected child result");
        worker.response.send("Child evidence is complete.".to_string()).unwrap();
        let evaluator = self.requests.recv().await.unwrap();
        assert!(evaluator.body.to_string().contains("hidden completion evaluator"));
        assert!(evaluator.body.to_string().contains("objective-A"));
        assert!(self.actor.persisted_completion_state().await.unwrap().receipts.iter().any(|receipt| receipt.completion == completion.id));
        (completion, receive, evaluator)
    }
}

fn candidate_complete() -> String {
    serde_json::json!({
        "decision": "candidate_complete", "evidence": "A-evaluator-evidence",
        "next_step": "Verify objective A", "blocker_key": ""
    }).to_string()
}

#[tokio::test(flavor = "current_thread")]
async fn live_child_consumer_completes_the_unchanged_active_goal() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut fixture = live_goal_fixture().await;
        let (_completion, receive, evaluator) = fixture.evaluated_child("positive-child").await;
        evaluator.response.send(candidate_complete()).unwrap();
        receive.await.unwrap().unwrap();
        fixture.fifo().await;
        let tracker = fixture.actor.goal_tracker.lock();
        let goal = tracker.snapshot().unwrap();
        assert_eq!(goal.goal_id, "goal-A");
        assert_eq!(goal.status, GoalStatus::Complete);
        assert_eq!(goal.total_worker_rounds, 1);
        assert_eq!(goal.last_classifier_verdict, Some(GoalClassifierVerdict::Achieved));
        assert_eq!(fixture.verifications.load(std::sync::atomic::Ordering::SeqCst), 1);
    })).await.expect("unchanged active goal must consume evaluator and verifier results");
}

#[tokio::test(flavor = "current_thread")]
async fn actual_goal_replacement_queues_behind_the_owning_evaluation() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut fixture = live_goal_fixture().await;
        let (_completion, receive, evaluator) = fixture.evaluated_child("replacement-child").await;
        let _replacement = fixture.prompt("replace-goal", "/goal objective-B");
        fixture.fifo().await;
        {
            let tracker = fixture.actor.goal_tracker.lock();
            let goal = tracker.snapshot().unwrap();
            assert_eq!(goal.goal_id, "goal-A");
            assert_eq!(goal.status, GoalStatus::Active);
            assert_eq!(goal.total_worker_rounds, 0);
            assert_eq!(goal.classifier_runs_attempted, 0);
        }
        assert!(fixture.actor.state.lock().await.pending_inputs.iter().any(|input| input.prompt_id == "replace-goal"));
        evaluator.response.send(candidate_complete()).unwrap();
        receive.await.unwrap().unwrap();
        let replacement_worker = fixture.requests.recv().await.unwrap();
        assert!(!replacement_worker.body.to_string().contains("hidden completion evaluator"));
        let tracker = fixture.actor.goal_tracker.lock();
        let goal = tracker.snapshot().unwrap();
        assert_ne!(goal.goal_id, "goal-A");
        assert_eq!(goal.objective, "objective-B");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.total_worker_rounds, 0);
        assert_eq!(goal.total_verify_rounds, 0);
        assert_eq!(goal.first_final_response, None);
        assert_eq!(goal.last_classifier_gaps, None);
        assert_eq!(goal.classifier_runs_attempted, 0);
        assert_eq!(goal.last_classifier_verdict, None);
        assert_eq!(goal.evaluator_blocker_key, None);
        assert_eq!(goal.evaluator_blocked_streak, 0);
        assert!(!goal.verifying_in_flight);
        assert!(!format!("{:?}", goal.history).contains("A-evaluator-evidence"));
        assert_eq!(fixture.verifications.load(std::sync::atomic::Ordering::SeqCst), 1);
    })).await.expect("actor replacement must not overlap the previous goal's evaluator ownership");
}

#[tokio::test(flavor = "current_thread")]
async fn actual_stop_aborts_pending_evaluation_before_pausing_the_goal() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut fixture = live_goal_fixture().await;
        let (completion, receive, evaluator) = fixture.evaluated_child("stopped-child").await;
        fixture.commands.send(SessionCommand::Cancel(crate::session::CancelOptions {
            trigger: Some(crate::session::CancelTrigger::CtrlC), user_initiated: true,
            ..Default::default()
        })).unwrap();
        fixture.fifo().await;
        assert_eq!(receive.await.unwrap().unwrap().stop_reason, acp::StopReason::Cancelled);
        assert!(fixture.actor.state.lock().await.running_task.is_none());
        let _ = evaluator.response.send(candidate_complete());
        fixture.fifo().await;
        let tracker = fixture.actor.goal_tracker.lock();
        let goal = tracker.snapshot().unwrap();
        assert_eq!(goal.goal_id, "goal-A");
        assert_eq!(goal.status, GoalStatus::UserPaused);
        assert_eq!(goal.total_worker_rounds, 0);
        assert_eq!(goal.classifier_runs_attempted, 0);
        assert_eq!(goal.last_classifier_verdict, None);
        assert_eq!(goal.evaluator_blocker_key, None);
        assert_eq!(goal.evaluator_blocked_streak, 0);
        assert!(!goal.verifying_in_flight);
        drop(tracker);
        assert_eq!(fixture.verifications.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(fixture.actor.persisted_completion_state().await.unwrap().receipts.iter().any(|receipt| receipt.completion == completion.id));
    })).await.expect("stop must abort the owning task rather than apply its late verdict");
}

#[tokio::test(flavor = "current_thread")]
async fn paused_goal_still_projects_child_without_autonomous_evaluation() {
    tokio::task::LocalSet::new().run_until(tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut fixture = live_goal_fixture().await;
        fixture.prompt("pause-goal", "/goal pause").await.unwrap().unwrap();
        assert_eq!(fixture.actor.goal_tracker.lock().status(), Some(GoalStatus::UserPaused));
        let (completion, receive) = fixture.child("paused-child").await;
        let worker = fixture.requests.recv().await.unwrap();
        assert!(!worker.body.to_string().contains("hidden completion evaluator"));
        assert!(worker.body.to_string().contains("paused-child"));
        worker.response.send("Delivered the child result while the goal remains paused.".to_string()).unwrap();
        receive.await.unwrap().unwrap();
        fixture.fifo().await;
        assert!(matches!(fixture.requests.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
        let tracker = fixture.actor.goal_tracker.lock();
        let goal = tracker.snapshot().unwrap();
        assert_eq!(goal.status, GoalStatus::UserPaused);
        assert_eq!(goal.total_worker_rounds, 0);
        assert_eq!(goal.classifier_runs_attempted, 0);
        drop(tracker);
        assert_eq!(fixture.verifications.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(fixture.actor.persisted_completion_state().await.unwrap().receipts.iter().any(|receipt| receipt.completion == completion.id));
    })).await.expect("paused goal must not suppress projection or start autonomous evaluation");
}
