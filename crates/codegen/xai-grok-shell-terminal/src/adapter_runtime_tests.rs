use super::*;
use xai_acp_lib::AcpClientMessage;
use xai_grok_tools::notification::{AcknowledgedToolNotification, ToolNotification, ToolNotificationHandle};

type ResourceCalls = Arc<Mutex<Vec<(&'static str, String)>>>;

fn resource_gateway(calls: ResourceCalls) -> GatewaySender {
    super::resources::Fixture::with_calls(Vec::new(), calls).adapter.gateway.clone()
}

fn request(path: PathBuf, handle: ToolNotificationHandle) -> TerminalRunRequest {
    TerminalRunRequest {
        command: "command".into(), working_directory: PathBuf::from("/tmp"), env: HashMap::new(),
        timeout: Duration::from_secs(30), output_byte_limit: 4096, output_file: path,
        notification_handle: handle, tool_call_id: "same-call".into(), display_command: None,
        auto_background_on_timeout: true, foreground_block_budget: Some(Duration::from_millis(10)),
        kind: TaskKind::Bash, owner_session_id: Some("owner".into()), description: Some("label".into()),
    }
}

#[tokio::test(start_paused = true)]
async fn reused_call_and_reconstructed_backend_return_unique_working_execution_handles() {
    let dir = tempfile::tempdir().unwrap();
    let calls: ResourceCalls = Arc::default();
    let gateway = resource_gateway(calls.clone());
    let first_backend = AcpTerminalAdapter::new(gateway.clone(), acp::SessionId::new("owner"));
    let reconstructed = AcpTerminalAdapter::new(gateway, acp::SessionId::new("owner"));
    let (notifications, mut events) = ToolNotificationHandle::channel();
    let result = first_backend.run(request(dir.path().join("first.log"), notifications.clone())).await.unwrap();
    let RunDisposition::Backgrounded { handle: first, reason } = result.disposition else { panic!("background handle"); };
    assert_eq!(reason, BackgroundReason::ForegroundBudget);
    assert_eq!(result.combined_output, "output:resource-0");
    assert_eq!(result.exit_code, None);
    assert!(!result.timed_out);
    let mut monitor = request(dir.path().join("second.log"), notifications.clone());
    monitor.kind = TaskKind::Monitor;
    let second = first_backend.run_background(monitor).await.unwrap();
    let third = reconstructed.run_background(request(dir.path().join("third.log"), notifications)).await.unwrap();
    assert_ne!(first.task_id, second.task_id);
    assert_ne!(second.task_id, third.task_id);
    assert_ne!(first.task_id, third.task_id);
    let mut completed_ids = Vec::new();
    for (backend, handle, resource, kind) in [
        (&first_backend, &first, "resource-0", TaskKind::Bash),
        (&first_backend, &second, "resource-1", TaskKind::Monitor),
        (&reconstructed, &third, "resource-2", TaskKind::Bash),
    ] {
        assert_ne!(handle.task_id, "same-call");
        assert!(uuid::Uuid::parse_str(&handle.task_id).is_ok());
        let snapshot = backend.get_task(&handle.task_id).await.unwrap();
        assert_eq!(snapshot.task_id, handle.task_id);
        assert_eq!(snapshot.output, format!("output:{resource}"));
        assert_eq!(snapshot.kind, kind);
        assert_eq!(snapshot.owner_session_id.as_deref(), Some("owner"));
        assert_eq!(backend.kill_task(&handle.task_id).await, KillOutcome::Killed);
        let snapshot = backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(1))).await.unwrap();
        assert_eq!(snapshot.task_id, handle.task_id);
        assert_eq!(snapshot.signal.as_deref(), Some("SIGTERM"));
        assert!(snapshot.completed);
        assert_eq!(backend.kill_task(&handle.task_id).await, KillOutcome::AlreadyExited);
        completed_ids.push(handle.task_id.clone());
    }
    for _ in 0..3 {
        let notification = tokio::time::timeout(Duration::from_secs(1), events.recv()).await.unwrap().unwrap();
        let ToolNotification::TaskCompleted(snapshot) = notification else { panic!("terminal publication"); };
        assert!(completed_ids.contains(&snapshot.task_id));
        completed_ids.retain(|id| id != &snapshot.task_id);
    }
    assert!(completed_ids.is_empty());
    let calls = calls.lock().unwrap();
    for (_, id) in calls.iter() { assert!(id.starts_with("resource-"), "task/call IDs cannot address ACP resources"); }
    for id in ["resource-0", "resource-1", "resource-2"] {
        for method in ["get", "wait", "kill", "release"] {
            assert!(calls.iter().any(|entry| *entry == (method, id.to_string())));
        }
    }
}

#[tokio::test]
async fn foreground_registration_survives_ctrl_g_and_aborted_send_now_caller() {
    let dir = tempfile::tempdir().unwrap();
    let calls: ResourceCalls = Arc::default();
    let adapter = Arc::new(AcpTerminalAdapter::new(resource_gateway(calls), acp::SessionId::new("owner")));
    for send_now in [false, true] {
        let (notifications, mut events) = ToolNotificationHandle::channel();
        let mut run = request(dir.path().join(format!("manual-{send_now}.log")), notifications);
        run.foreground_block_budget = Some(Duration::MAX);
        let foreground = tokio::spawn({ let adapter = adapter.clone(); async move { adapter.run(run).await } });
        let original = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(task) = adapter.list_tasks_metadata().await.into_iter().find(|task| !task.is_backgrounded) { break task.task_id; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_ne!(original, "same-call");
        if send_now {
            let moved = adapter.background_foreground_commands(Some("owner")).await;
            assert_eq!(moved.len(), 1);
            assert_eq!(moved.first().unwrap().tool_call_id, "same-call");
            assert_eq!(moved.first().unwrap().task_id, original);
            foreground.abort();
            let notification = events.recv().await.unwrap();
            let ToolNotification::BashExecutionBackgrounded(backgrounded) = notification else { panic!("send-now announcement"); };
            assert_eq!(backgrounded.task_id, original);
            assert_eq!(backgrounded.base.tool_call_id, "same-call");
        } else {
            assert!(adapter.background_foreground_command("same-call").await);
            let result = foreground.await.unwrap().unwrap();
            let RunDisposition::Backgrounded { handle, reason } = result.disposition else { panic!("background handle"); };
            assert_eq!(handle.task_id, original);
            assert_eq!(reason, BackgroundReason::UserRedirect);
        }
        assert!(adapter.get_task(&original).await.unwrap().is_backgrounded);
        assert_eq!(adapter.kill_task(&original).await, KillOutcome::Killed);
        let completed = adapter.wait_for_completion(&original, Some(Duration::from_secs(2))).await.unwrap();
        assert_eq!(completed.task_id, original);
        assert_eq!(completed.signal.as_deref(), Some("SIGTERM"));
        assert!(completed.completed);
    }
}

async fn enrollment(deliveries: &mut tokio::sync::mpsc::UnboundedReceiver<AcknowledgedToolNotification>) -> AcknowledgedToolNotification {
    tokio::time::timeout(Duration::from_secs(2), deliveries.recv()).await.unwrap().unwrap()
}

#[derive(Clone, Copy)]
enum AckFailure { Held, Negative, Dropped }

#[tokio::test(start_paused = true)]
async fn owned_exit_ack_failure_never_becomes_kill_success_and_later_enrolls_same_fact() {
    for failure in [AckFailure::Held, AckFailure::Negative, AckFailure::Dropped] {
        for source in [KillSource::ModelTool, KillSource::ClientUi, KillSource::Teardown] {
            let calls: ResourceCalls = Arc::default();
            let adapter = Arc::new(AcpTerminalAdapter::new(resource_gateway(calls.clone()), acp::SessionId::new("owner")));
            let (handle, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
            adapter.tasks.lock().unwrap().insert("execution".into(), TrackedTask {
                notification_handle: handle.clone(), terminal_id: acp::TerminalId::new("owned-resource"),
                owner_session_id: Some("owner".into()), ..Default::default()
            });
            let producer = tokio::spawn({
                let adapter = adapter.clone();
                async move {
                    crate::exit_watcher::complete_and_release(
                        &adapter.gateway, &adapter.session_id, &acp::TerminalId::new("owned-resource"),
                        &adapter.tasks, &handle, "execution", SnapshotOutput {
                            output: "final bytes".into(), truncated: false, exit_code: Some(7), signal: None,
                        },
                    ).await;
                }
            });
            let delivery = enrollment(&mut deliveries).await;
            let frozen = delivery.notification;
            let ack = delivery.acknowledgement.unwrap();
            let kill = tokio::spawn({ let adapter = adapter.clone(); async move { adapter.kill_task_with_source("execution", source).await } });
            tokio::task::yield_now().await;
            assert!(!kill.is_finished());
            assert!(!adapter.tasks.lock().unwrap()["execution"].completed);
            match failure {
                AckFailure::Held => { tokio::time::advance(xai_grok_tools::notification::handle::TERMINAL_ACK_BUDGET).await; drop(ack); }
                AckFailure::Negative => { ack.send(Err("negative durable ACK".into())).unwrap(); }
                AckFailure::Dropped => drop(ack),
            }
            assert_eq!(kill.await.unwrap(), KillOutcome::Indeterminate {
                stage: KillStage::DurableEnrollment, process_effect: KillProcessEffect::TerminalObserved,
            });
            assert!(!adapter.tasks.lock().unwrap()["execution"].kill_result_delivered);
            assert!(!adapter.tasks.lock().unwrap()["execution"].completed);
            assert!(calls.lock().unwrap().is_empty(), "pending owned outcome must not be killed again or released");
            let retry = enrollment(&mut deliveries).await;
            match (&frozen, &retry.notification) {
                (ToolNotification::TaskCompleted(first), ToolNotification::TaskCompleted(next)) => assert_eq!(first, next),
                _ => panic!("same frozen terminal obligation"),
            }
            retry.acknowledgement.unwrap().send(Ok(())).unwrap();
            producer.await.unwrap();
            let publication = enrollment(&mut deliveries).await;
            assert!(publication.acknowledgement.is_none());
            let ToolNotification::TaskCompleted(snapshot) = publication.notification else { panic!("publication"); };
            assert_eq!(snapshot.task_id, "execution");
            assert_eq!(snapshot.output, "final bytes");
            assert_eq!(snapshot.exit_code, Some(7));
            assert!(!snapshot.kill_result_delivered, "a later owner retry cannot turn an uncertain caller into terminal observation");
            assert_eq!(adapter.kill_task("execution").await, KillOutcome::AlreadyExited);
        }
    }
}

#[tokio::test]
async fn acp_negative_response_is_distinct_from_absent_and_resource_probe_is_not_owned() {
    for absent in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                match message {
                    AcpClientMessage::TerminalOutput(args) => {
                        let error = if absent { acp::Error::resource_not_found(None::<String>) } else { acp::Error::internal_error() };
                        let _ = args.response_tx.send(Err(error));
                    }
                    AcpClientMessage::KillTerminalCommand(args) => { let _ = args.response_tx.send(Err(acp::Error::internal_error())); }
                    _ => {}
                }
            }
        });
        let adapter = AcpTerminalAdapter::new(GatewaySender::new(tx), acp::SessionId::new("owner"));
        let outcome = adapter.kill_task("resource-only").await;
        if absent { assert_eq!(outcome, KillOutcome::NotFound); }
        else { assert_eq!(outcome, KillOutcome::Indeterminate { stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::Unknown }); }
        assert!(adapter.tasks.lock().unwrap().is_empty());
    }
    let snapshot = TrackedTask::resource_snapshot("unowned", SnapshotOutput {
        output: "observed bytes".into(), truncated: false, exit_code: Some(0), signal: None,
    });
    assert!(snapshot.completed);
    assert!(!snapshot.is_backgrounded);
    assert_eq!(snapshot.owner_session_id, None);
}

#[tokio::test]
async fn owned_kill_rpc_refusal_and_lost_response_preserve_execution_ownership() {
    for unsupported in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if let AcpClientMessage::KillTerminalCommand(args) = message {
                    assert_eq!(args.request.terminal_id.0.to_string(), "client-resource");
                    if unsupported {
                        let _ = args.response_tx.send(Err(acp::Error::method_not_found()));
                    } else {
                        drop(args.response_tx);
                    }
                }
            }
        });
        let adapter = AcpTerminalAdapter::new(GatewaySender::new(tx), acp::SessionId::new("owner"));
        let (notifications, mut events) = ToolNotificationHandle::acknowledged_channel();
        adapter.tasks.lock().unwrap().insert("execution".into(), TrackedTask {
            notification_handle: notifications,
            terminal_id: acp::TerminalId::new("client-resource"),
            owner_session_id: Some("owner".into()),
            ..Default::default()
        });
        let outcome = adapter.kill_task("execution").await;
        let expected = if unsupported {
            KillOutcome::Failed {
                stage: KillStage::ProcessTermination,
                reason: KillFailureReason::Unsupported,
            }
        } else {
            KillOutcome::Indeterminate {
                stage: KillStage::ProcessTermination,
                process_effect: KillProcessEffect::Unknown,
            }
        };
        assert_eq!(outcome, expected);
        let tasks = adapter.tasks.lock().unwrap();
        let task = tasks.get("execution").unwrap();
        assert!(!task.completed);
        assert!(!task.kill_result_delivered);
        assert_eq!(task.owner_session_id.as_deref(), Some("owner"));
        assert!(matches!(task.enrollment, TerminalEnrollment::Running));
        assert!(events.try_recv().is_err());
    }
}

#[tokio::test]
async fn foreground_exit_claim_prevents_a_second_background_owner_during_output_fetch() {
    let directory = tempfile::tempdir().unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (outputs, mut held_outputs) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                AcpClientMessage::CreateTerminal(args) => {
                    let _ = args.response_tx.send(Ok(acp::CreateTerminalResponse::new("resource")));
                }
                AcpClientMessage::WaitForTerminalExit(args) => {
                    let _ = args.response_tx.send(Ok(acp::WaitForTerminalExitResponse::new(
                        acp::TerminalExitStatus::new().exit_code(Some(0)),
                    )));
                }
                AcpClientMessage::TerminalOutput(args) => {
                    let _ = outputs.send(args.response_tx);
                }
                AcpClientMessage::ReleaseTerminal(args) => {
                    let _ = args.response_tx.send(Ok(acp::ReleaseTerminalResponse::new()));
                }
                _ => {}
            }
        }
    });
    let adapter = Arc::new(AcpTerminalAdapter::new(GatewaySender::new(tx), acp::SessionId::new("owner")));
    let mut run = request(directory.path().join("foreground.log"), ToolNotificationHandle::noop());
    run.auto_background_on_timeout = false;
    let foreground = tokio::spawn({
        let adapter = adapter.clone();
        async move { adapter.run(run).await }
    });
    let output_reply = tokio::time::timeout(Duration::from_secs(2), held_outputs.recv()).await.unwrap().unwrap();
    assert!(!foreground.is_finished());
    assert!(!adapter.background_foreground_command("same-call").await);
    assert!(adapter.background_foreground_commands(Some("owner")).await.is_empty());
    output_reply.send(Ok(acp::TerminalOutputResponse::new("finished bytes", false)
        .exit_status(Some(acp::TerminalExitStatus::new().exit_code(Some(0)))))).unwrap();
    let result = foreground.await.unwrap().unwrap();
    assert!(matches!(result.disposition, RunDisposition::Foreground));
    assert_eq!(result.combined_output, "finished bytes");
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.timed_out);
    assert!(adapter.tasks.lock().unwrap().is_empty());
}
