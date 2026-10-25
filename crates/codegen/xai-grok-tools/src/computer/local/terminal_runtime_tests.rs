use super::*;
use crate::computer::types::{BackgroundReason as RunBackgroundReason, TaskKind};
use crate::notification::{AcknowledgedToolNotification, ToolNotification};

fn request(command: &str, directory: &std::path::Path) -> TerminalRunRequest {
    TerminalRunRequest {
        command: command.into(), working_directory: directory.into(), env: HashMap::new(),
        timeout: Duration::from_secs(30), output_byte_limit: 4096,
        output_file: directory.join(format!("{}.log", uuid::Uuid::now_v7())),
        notification_handle: ToolNotificationHandle::noop(), tool_call_id: "reused-call".into(),
        display_command: None, auto_background_on_timeout: false, foreground_block_budget: None,
        kind: TaskKind::Bash, owner_session_id: Some("owner".into()), description: None,
    }
}

async fn enrollment(
    deliveries: &mut mpsc::UnboundedReceiver<AcknowledgedToolNotification>,
) -> (TaskSnapshot, oneshot::Sender<Result<(), String>>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let delivery = deliveries.recv().await.expect("enrollment transport");
            if let ToolNotification::TaskCompleted(snapshot) = delivery.notification {
                return (snapshot, delivery.acknowledgement.expect("publication cannot precede ACK"));
            }
        }
    }).await.expect("bounded enrollment dispatch")
}

#[derive(Clone, Copy)]
enum AckFailure { Held, Negative, Dropped }

#[tokio::test]
async fn actor_kill_ack_failures_are_bounded_live_and_complete_later() {
    for failure in [AckFailure::Held, AckFailure::Negative, AckFailure::Dropped] {
        for source in [KillSource::ModelTool, KillSource::ClientUi, KillSource::Teardown] {
            let dir = tempfile::tempdir().unwrap();
            let backend = LocalTerminalBackend::new_with_completed_task_ttl(Duration::ZERO);
            let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
            let mut run = request("printf ack-marker; sleep 60", dir.path());
            run.notification_handle = notifications;
            let handle = backend.run_background(run).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while backend.get_task(&handle.task_id).await.unwrap().output != "ack-marker" {
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            let waiter = tokio::spawn({
                let backend = backend.clone();
                let id = handle.task_id.clone();
                async move { backend.wait_for_completion(&id, Some(Duration::from_secs(30))).await }
            });
            let kill = tokio::spawn({
                let backend = backend.clone();
                let id = handle.task_id.clone();
                async move { backend.kill_task_with_source(&id, source).await }
            });
            let (frozen, ack) = enrollment(&mut deliveries).await;
            assert_eq!(frozen.task_id, handle.task_id);
            assert_eq!(frozen.command, "printf ack-marker; sleep 60");
            assert_eq!(frozen.output, "ack-marker");
            assert_eq!(frozen.owner_session_id.as_deref(), Some("owner"));
            assert_eq!(frozen.kind, TaskKind::Bash);
            assert_eq!(frozen.signal.as_deref(), Some("killed"));
            assert!(frozen.completed);
            assert!(!frozen.kill_result_delivered);
            assert!(!waiter.is_finished());
            assert!(!kill.is_finished());

            let other = tokio::time::timeout(Duration::from_secs(2), backend.run_background(request("sleep 60", dir.path())))
                .await.expect("actor must accept another run while ACK is held").unwrap();
            assert!(tokio::time::timeout(Duration::from_secs(2), backend.get_task(&other.task_id))
                .await.expect("actor get is live").is_some());
            assert!(matches!(tokio::time::timeout(Duration::from_secs(3), backend.kill_task(&other.task_id))
                .await.expect("another kill must not wait for the held ACK"), KillOutcome::Killed | KillOutcome::AlreadyExited));
            let output = tokio::time::timeout(Duration::from_secs(2), backend.run(request("printf independent-output", dir.path())))
                .await.expect("foreground dispatch is live").unwrap();
            assert_eq!(output.combined_output, "independent-output");
            assert_eq!(output.exit_code, Some(0));
            assert!(!output.timed_out);

            match failure {
                AckFailure::Held => { let _ = tokio::time::timeout(Duration::from_secs(7), async {
                    while !kill.is_finished() { tokio::task::yield_now().await; }
                }).await.expect("absolute ACK budget"); drop(ack); }
                AckFailure::Negative => { ack.send(Err("injected negative ACK".into())).unwrap(); }
                AckFailure::Dropped => drop(ack),
            }
            assert_eq!(tokio::time::timeout(Duration::from_secs(7), kill).await.unwrap().unwrap(), enrollment_uncertain());
            assert!(!waiter.is_finished(), "uncertainty cannot release a terminal waiter");
            assert!(backend.get_task(&handle.task_id).await.is_none(), "unenrolled result cannot be observed or evicted into a successful tombstone");
            let pending = backend.list_tasks_metadata().await.into_iter().find(|task| task.task_id == handle.task_id).unwrap();
            assert!(!pending.completed);
            assert!(!pending.kill_result_delivered);

            let (retry, ack) = enrollment(&mut deliveries).await;
            assert_eq!(retry, frozen, "retry must preserve the frozen terminal fact and identity");
            ack.send(Ok(())).unwrap();
            let completed = tokio::time::timeout(Duration::from_secs(3), waiter).await.unwrap().unwrap().unwrap();
            assert_eq!(completed.task_id, handle.task_id);
            assert!(completed.completed);
            assert!(completed.block_waited);
            assert_eq!(backend.kill_task(&handle.task_id).await, KillOutcome::AlreadyExited);
            assert_eq!(backend.kill_task("genuinely-absent").await, KillOutcome::NotFound);
            let published = tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let delivery = deliveries.recv().await.unwrap();
                    assert!(delivery.acknowledgement.is_none(), "one owner publishes after ACK");
                    if let ToolNotification::TaskCompleted(snapshot) = delivery.notification { break snapshot; }
                }
            }).await.unwrap();
            assert_eq!(published.task_id, handle.task_id);
            assert!(published.completed);
            assert!(backend.get_task(&handle.task_id).await.unwrap().completed);
        }
    }
}

#[tokio::test]
async fn enrolled_tombstone_keeps_a_working_handle_after_live_record_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalTerminalBackend::new_with_completed_task_ttl(Duration::from_millis(100));
    let handle = backend.run_background(request("printf tombstone-marker", dir.path())).await.unwrap();
    let completed = backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(3))).await.unwrap();
    assert!(completed.completed);
    assert_eq!(completed.output, "tombstone-marker");
    assert_eq!(completed.exit_code, Some(0));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let tombstone = backend.get_task(&handle.task_id).await.unwrap();
    assert_eq!(tombstone.task_id, handle.task_id);
    assert!(tombstone.completed);
    assert!(tombstone.output.is_empty(), "TTL replaces the live output buffer with metadata only");
    assert!(tombstone.truncated);
    assert_eq!(tombstone.exit_code, Some(0));
    assert_eq!(backend.kill_task(&handle.task_id).await, KillOutcome::AlreadyExited);
    assert_eq!(backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(1))).await.unwrap().task_id, handle.task_id);
    assert_eq!(backend.kill_task("absent").await, KillOutcome::NotFound);
}

#[tokio::test]
async fn demotion_keeps_the_pending_drain_execution_key() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalTerminalBackend::new();
    let run = request("(sleep 1; printf late-output) &", dir.path());
    let foreground = tokio::spawn({ let backend = backend.clone(); async move { backend.run(run).await } });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let (reply, receiver) = oneshot::channel();
            backend.cmd_tx.send(TerminalCommand::AnyTaskDraining { reply }).await.unwrap();
            if receiver.await.unwrap() { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let original = backend.list_tasks_metadata().await.into_iter().find(|task| !task.is_backgrounded).unwrap().task_id;
    assert!(backend.background_foreground_command("reused-call").await);
    let result = foreground.await.unwrap().unwrap();
    let RunDisposition::Backgrounded { handle, .. } = result.disposition else { panic!("background handle"); };
    assert_eq!(handle.task_id, original);
    let completed = tokio::time::timeout(Duration::from_secs(4), backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(3))))
        .await.unwrap().unwrap();
    assert_eq!(completed.task_id, original);
    assert_eq!(completed.output, "late-output");
    assert_eq!(completed.exit_code, Some(0));
    assert!(completed.completed);
    assert_eq!(backend.kill_task(&handle.task_id).await, KillOutcome::AlreadyExited);
}

#[tokio::test]
async fn registration_identity_survives_ctrl_g_send_now_and_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalTerminalBackend::new();
    for reason in [RunBackgroundReason::UserRedirect, RunBackgroundReason::ForegroundBudget] {
        let mut run = request("sleep 60", dir.path());
        run.auto_background_on_timeout = matches!(reason, RunBackgroundReason::ForegroundBudget);
        run.foreground_block_budget = Some(Duration::from_millis(300));
        let foreground = tokio::spawn({ let backend = backend.clone(); async move { backend.run(run).await } });
        let original = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(snapshot) = backend.list_tasks_metadata().await.into_iter().find(|task| !task.is_backgrounded) { break snapshot.task_id; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        if matches!(reason, RunBackgroundReason::UserRedirect) {
            assert!(backend.background_foreground_command("reused-call").await);
        }
        let result = tokio::time::timeout(Duration::from_secs(3), foreground).await.unwrap().unwrap().unwrap();
        let RunDisposition::Backgrounded { handle, reason: actual } = result.disposition else { panic!("background result"); };
        assert_eq!(actual, reason);
        assert_eq!(handle.task_id, original);
        assert_ne!(handle.task_id, "reused-call");
        assert!(backend.get_task(&handle.task_id).await.unwrap().is_backgrounded);
        assert!(matches!(backend.kill_task(&handle.task_id).await, KillOutcome::Killed | KillOutcome::AlreadyExited));
        assert!(backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(2))).await.unwrap().completed);
    }
    let run = request("sleep 60", dir.path());
    let foreground = tokio::spawn({ let backend = backend.clone(); async move { backend.run(run).await } });
    let original = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(snapshot) = backend.list_tasks_metadata().await.into_iter().find(|task| !task.is_backgrounded) { break snapshot.task_id; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let moved = backend.background_foreground_commands(Some("owner")).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved.first().unwrap().tool_call_id, "reused-call");
    assert_eq!(moved.first().unwrap().task_id, original);
    let result = foreground.await.unwrap().unwrap();
    let RunDisposition::Backgrounded { handle, reason } = result.disposition else { panic!("background result"); };
    assert_eq!(reason, RunBackgroundReason::UserRedirect);
    assert_eq!(handle.task_id, original);
    assert!(matches!(backend.kill_task(&handle.task_id).await, KillOutcome::Killed | KillOutcome::AlreadyExited));
    let replacement = LocalTerminalBackend::new();
    let first = backend.run_background(request("sleep 60", dir.path())).await.unwrap();
    let second = backend.run_background(request("sleep 60", dir.path())).await.unwrap();
    let rebuilt = replacement.run_background(request("sleep 60", dir.path())).await.unwrap();
    assert_ne!(first.task_id, second.task_id);
    assert_ne!(second.task_id, rebuilt.task_id);
    assert_ne!(first.task_id, rebuilt.task_id);
    for (backend, handle) in [(&backend, first), (&backend, second), (&replacement, rebuilt)] {
        assert_eq!(backend.get_task(&handle.task_id).await.unwrap().task_id, handle.task_id);
        assert!(matches!(backend.kill_task(&handle.task_id).await, KillOutcome::Killed | KillOutcome::AlreadyExited));
        assert_eq!(backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(2))).await.unwrap().task_id, handle.task_id);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn actor_dispatch_remains_live_during_sigterm_grace() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalTerminalBackend::new();
    let handle = backend.run_background(request(
        "trap '' TERM; printf ready; while :; do sleep 1; done",
        dir.path(),
    )).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if backend.get_task(&handle.task_id).await.unwrap().output.contains("ready") {
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let kill = tokio::spawn({
        let backend = backend.clone();
        let id = handle.task_id.clone();
        async move { backend.kill_task(&id).await }
    });
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let snapshot = backend.get_task(&handle.task_id).await.unwrap();
            if snapshot.explicitly_killed {
                assert!(!snapshot.completed);
                assert!(!snapshot.kill_result_delivered);
                break;
            }
            tokio::task::yield_now().await;
        }
        let independent = backend.run(request("printf responsive", dir.path())).await.unwrap();
        assert_eq!(independent.combined_output, "responsive");
        assert_eq!(independent.exit_code, Some(0));
    }).await.expect("process-stop grace cannot hold the command dispatcher");
    assert!(!kill.is_finished());
    assert_eq!(tokio::time::timeout(Duration::from_secs(10), kill).await.unwrap().unwrap(), KillOutcome::Killed);
    let completed = backend.wait_for_completion(&handle.task_id, Some(Duration::from_secs(2))).await.unwrap();
    assert!(completed.completed);
    assert!(completed.kill_result_delivered);
    assert_eq!(completed.output, "ready");
}

#[tokio::test]
async fn ui_kill_without_waiter_retains_unconsumed_completion_after_ack_failure() {
    for failure in [AckFailure::Held, AckFailure::Negative, AckFailure::Dropped] {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalTerminalBackend::new_with_completed_task_ttl(Duration::ZERO);
        let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
        let mut run = request("sleep 60", dir.path());
        run.notification_handle = notifications;
        let handle = backend.run_background(run).await.unwrap();
        let kill = tokio::spawn({
            let backend = backend.clone();
            let id = handle.task_id.clone();
            async move { backend.kill_task_with_source(&id, KillSource::ClientUi).await }
        });
        let (frozen, ack) = enrollment(&mut deliveries).await;
        match failure {
            AckFailure::Held => {
                tokio::time::timeout(Duration::from_secs(7), async {
                    while !kill.is_finished() { tokio::task::yield_now().await; }
                }).await.unwrap();
                drop(ack);
            }
            AckFailure::Negative => { ack.send(Err("negative ACK".into())).unwrap(); }
            AckFailure::Dropped => drop(ack),
        }
        assert_eq!(kill.await.unwrap(), enrollment_uncertain());
        assert!(!frozen.block_waited);
        assert!(!frozen.kill_result_delivered);
        let (retry, ack) = enrollment(&mut deliveries).await;
        assert_eq!(retry, frozen);
        ack.send(Ok(())).unwrap();
        let published = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let delivery = deliveries.recv().await.unwrap();
                if let ToolNotification::TaskCompleted(snapshot) = delivery.notification {
                    assert!(delivery.acknowledgement.is_none());
                    break snapshot;
                }
            }
        }).await.unwrap();
        assert_eq!(published.task_id, handle.task_id);
        assert!(published.completed);
        assert!(published.explicitly_killed);
        assert!(!published.block_waited);
        assert!(!published.kill_result_delivered);
        assert!(!published.is_auto_wake_suppressed());
    }
}

#[tokio::test]
async fn kill_transport_distinguishes_nondispatch_from_lost_reply() {
    let (cmd_tx, cmd_rx) = mpsc::channel(1);
    drop(cmd_rx);
    let unavailable = LocalTerminalBackend { cmd_tx, cancel_token: CancellationToken::new() };
    assert_eq!(unavailable.kill_task("execution").await, KillOutcome::Failed {
        stage: KillStage::ProcessTermination,
        reason: KillFailureReason::DispatchUnavailable,
    });

    let (cmd_tx, mut cmd_rx) = mpsc::channel(1);
    let disconnected = LocalTerminalBackend { cmd_tx, cancel_token: CancellationToken::new() };
    let kill = tokio::spawn(async move { disconnected.kill_task("execution").await });
    let command = cmd_rx.recv().await.unwrap();
    let TerminalCommand::Kill { task_id, reply, .. } = command else { panic!("kill dispatch"); };
    assert_eq!(task_id, "execution");
    drop(reply);
    assert_eq!(kill.await.unwrap(), KillOutcome::Indeterminate {
        stage: KillStage::ProcessTermination,
        process_effect: KillProcessEffect::Unknown,
    });
}

#[cfg(unix)]
fn owned_actor(directory: &std::path::Path) -> (LocalTerminalActor, mpsc::Sender<TerminalCommand>, crate::util::ProcessScope) {
    let (tx, rx) = mpsc::channel(COMMAND_CHANNEL_SIZE);
    let scope = crate::util::ProcessScope::new();
    let mut actor = LocalTerminalActor::new(
        rx, tx.downgrade(), CancellationToken::new(), CgroupGuard::noop(),
        MemoryMonitor::noop(), false, false, SearchShadowConfig::default(),
        ActorSettings::default(), scope.clone(), None, None, None,
    );
    actor.login_env = Some(HashMap::new());
    #[cfg(target_os = "linux")]
    { actor.orphan_root = Some(directory.join("registry")); }
    #[cfg(not(target_os = "linux"))]
    let _ = directory;
    (actor, tx, scope)
}

#[cfg(unix)]
async fn actor_foreground(
    actor: &mut LocalTerminalActor,
    run: TerminalRunRequest,
) -> (String, oneshot::Receiver<Result<TerminalRunResult, ComputerError>>) {
    let (reply, receiver) = oneshot::channel();
    actor.handle_command(TerminalCommand::Run { request: run, reply }).await;
    let id = actor.processes.keys().next().unwrap().clone();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            actor.poll_process(&id).await;
            if actor.processes[&id].ring_output() == "owned-ready" { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    (id, receiver)
}

#[cfg(unix)]
async fn collect_actor(actor: &mut LocalTerminalActor) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !actor.processes.is_empty() {
            actor.poll_all_processes().await;
            while let Ok(command) = actor.owner_rx.try_recv() {
                actor.handle_command(command).await;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("actor must actually collect before retirement");
}

#[cfg(target_os = "linux")]
fn isolated_registration(actor: &mut LocalTerminalActor, id: &str, root: &std::path::Path) -> PathBuf {
    assert!(actor.processes[id].orphan_registration.is_some());
    assert_eq!(actor.orphan_root.as_deref(), Some(root));
    let rows = std::fs::read_dir(root).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(rows.len(), 1);
    rows[0].path()
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn foreground_timeout_keeps_scope_and_registration_through_escalation() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _tx, scope) = owned_actor(dir.path());
    let command = "trap '' TERM; printf owned-ready; while :; do sleep 1; done";
    let (id, result) = actor_foreground(&mut actor, request(command, dir.path())).await;
    let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
    actor.processes.get_mut(&id).unwrap().timeout = Duration::ZERO;
    actor.poll_all_processes().await;
    let result = result.await.unwrap().unwrap();
    assert!(result.timed_out);
    assert_eq!(result.combined_output, "owned-ready");
    assert_eq!(result.signal.as_deref(), Some("timeout"));
    assert_eq!(actor.processes[&id].owner_session_id.as_deref(), Some("owner"));
    assert_eq!(actor.processes[&id].command, command);
    assert!(actor.processes[&id].child.id().is_some());
    assert!(!actor.processes[&id].is_complete());
    assert_eq!(scope.live_count(), 1);
    assert!(row.exists());
    let pid = actor.processes[&id].child.id().unwrap();
    actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Unavailable;
    tokio::time::sleep(SIGTERM_GRACE + Duration::from_millis(20)).await;
    actor.poll_all_processes().await;
    actor.evict_if_foreground(&id);
    assert!(actor.processes.contains_key(&id), "failed collection is not retirement proof");
    assert_eq!(scope.live_count(), 1);
    assert!(row.exists());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            let fields: Vec<_> = stat.rsplit_once(") ").unwrap().1.split_whitespace().collect();
            if fields[0] == "Z" {
                assert_eq!(fields[49], "9", "kernel exit status must prove SIGKILL, not the timeout string");
                break;
            }
            actor.poll_all_processes().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("overdue capability-based SIGKILL must run while collection is unavailable");
    assert!(matches!(actor.processes[&id].collection_access, CollectionAccess::Unavailable));
    assert_eq!(actor.processes[&id].child.id(), Some(pid));
    assert!(!actor.processes[&id].draining, "failed collection cannot start a drain-as-exit loop");
    assert_eq!(scope.live_count(), 1);
    assert!(row.exists());
    actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Kernel;
    collect_actor(&mut actor).await;
    assert_eq!(scope.live_count(), 0);
    assert!(!row.exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn running_actor_timeout_escalates_before_retiring_foreground_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _tx, scope) = owned_actor(dir.path());
    let (id, result) = actor_foreground(&mut actor, request(
        "trap '' TERM; printf owned-ready; while :; do sleep 1; done", dir.path(),
    )).await;
    let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
    actor.processes.get_mut(&id).unwrap().timeout = Duration::from_millis(200);
    actor.tick_interval = Duration::from_secs(60);
    let cancel = actor.cancel_token.clone();
    let task = tokio::spawn(actor.run());
    let result = tokio::time::timeout(Duration::from_secs(1), result).await.unwrap().unwrap().unwrap();
    assert_eq!(result.combined_output, "owned-ready");
    assert!(result.timed_out);
    assert_eq!(scope.live_count(), 1);
    assert!(row.exists());
    tokio::time::timeout(Duration::from_secs(3), async {
        while scope.live_count() != 0 || row.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("grace escalation and collection must not depend on the 60-second tick");
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn running_actor_shutdown_collects_before_releasing_scope_and_registration() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _tx, scope) = owned_actor(dir.path());
    let (id, result) = actor_foreground(&mut actor, request(
        "trap '' TERM; printf owned-ready; while :; do sleep 1; done", dir.path(),
    )).await;
    let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
    let cancel = actor.cancel_token.clone();
    let task = tokio::spawn(actor.run());
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert_eq!(scope.live_count(), 0);
    assert!(!row.exists());
    assert_eq!(result.await.unwrap().unwrap().combined_output, "owned-ready");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn foreground_cancel_and_shutdown_retain_uncollected_ownership() {
    for by_owner in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, _tx, scope) = owned_actor(dir.path());
        let (id, result) = actor_foreground(&mut actor, request(
            "trap '' TERM; printf owned-ready; while :; do sleep 1; done", dir.path(),
        )).await;
        let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
        actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Unavailable;
        let command = if by_owner {
            TerminalCommand::KillForegroundCommandsByOwner { owner_session_id: "owner".into() }
        } else {
            TerminalCommand::KillForegroundCommands
        };
        tokio::time::timeout(Duration::from_millis(500), actor.handle_command(command)).await.unwrap();
        let cancelled = result.await.unwrap().unwrap();
        assert_eq!(cancelled.combined_output, "owned-ready");
        assert_eq!(cancelled.signal.as_deref(), Some("cancelled"));
        actor.evict_exited_processes().await;
        assert!(actor.processes.contains_key(&id));
        assert_eq!(scope.live_count(), 1);
        assert!(row.exists());
        actor.shutdown_all().await;
        actor.collect_shutdown_children().await;
        assert!(actor.processes.contains_key(&id));
        assert_eq!(scope.live_count(), 1);
        assert!(row.exists());
        actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Kernel;
        tokio::time::timeout(Duration::from_secs(3), async {
            while !actor.processes.is_empty() {
                actor.collect_shutdown_children().await;
                while let Ok(command) = actor.owner_rx.try_recv() {
                    actor.handle_command(command).await;
                }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(scope.live_count(), 0);
        assert!(!row.exists());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn actor_destruction_hands_off_the_existing_row_without_claiming_collection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _tx, scope) = owned_actor(dir.path());
    let (id, _result) = actor_foreground(&mut actor, request(
        "printf owned-ready; sleep 1", dir.path(),
    )).await;
    let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
    assert!(actor.processes[&id].child.id().is_some());
    drop(actor);
    assert!(row.exists(), "handoff must not unlink the only durable ownership row");
    assert_eq!(scope.live_count(), 0);
    let lease = std::fs::OpenOptions::new().read(true).write(true).open(&row).unwrap();
    fs2::FileExt::try_lock_exclusive(&lease).unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropped_spawn_registration_preserves_the_preinstallation_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let scope = crate::util::ProcessScope::new();
    let mut command = tokio::process::Command::new("sleep");
    command.arg("60").kill_on_drop(true);
    let (mut child, group) = scope.spawn(command).unwrap();
    let registration = orphan_registry::register_child_at(dir.path(), child.id().unwrap()).unwrap().unwrap();
    let row = std::fs::read_dir(dir.path()).unwrap().next().unwrap().unwrap().path();
    drop(registration);
    assert!(row.exists(), "a dropped spawn result did not collect its child");
    assert!(child.id().is_some());
    assert_eq!(scope.live_count(), 1);
    let lease = std::fs::OpenOptions::new().read(true).write(true).open(&row).unwrap();
    fs2::FileExt::try_lock_exclusive(&lease).unwrap();
    group.kill().unwrap();
    tokio::time::timeout(Duration::from_secs(3), child.wait()).await.unwrap().unwrap();
    assert!(child.id().is_none());
    drop(group);
    assert_eq!(scope.live_count(), 0);
    assert!(row.exists(), "collection after handoff does not fabricate retirement of the released record");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn expired_foreground_kill_does_not_retire_an_uncollected_child() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _tx, scope) = owned_actor(dir.path());
    let (id, _result) = actor_foreground(&mut actor, request(
        "trap '' TERM; printf owned-ready; while :; do sleep 1; done", dir.path(),
    )).await;
    let row = isolated_registration(&mut actor, &id, &dir.path().join("registry"));
    let (reply, receiver) = oneshot::channel();
    actor.handle_command(TerminalCommand::Kill {
        task_id: id.clone(), source: KillSource::ModelTool,
        deadline: Instant::now() + Duration::from_millis(50), reply,
    }).await;
    actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Unavailable;
    tokio::time::sleep(Duration::from_millis(60)).await;
    actor.settle_pending_kills().await;
    assert_eq!(receiver.await.unwrap(), KillOutcome::Indeterminate {
        stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::StopRequested,
    });
    actor.evict_exited_processes().await;
    assert!(actor.processes.contains_key(&id));
    assert_eq!(scope.live_count(), 1);
    assert!(row.exists());
    assert!(!actor.processes[&id].kill_result_delivered);
    actor.processes.get_mut(&id).unwrap().collection_access = CollectionAccess::Kernel;
    collect_actor(&mut actor).await;
    assert_eq!(scope.live_count(), 0);
    assert!(!row.exists());
}

fn saturate_commands(backend: &LocalTerminalBackend) -> (CancellationToken, Vec<tokio::task::JoinHandle<()>>) {
    let stop = CancellationToken::new();
    let workers = (0..64).map(|_| {
        let tx = backend.cmd_tx.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            loop {
                let (reply, _receiver) = oneshot::channel();
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    sent = tx.send(TerminalCommand::GetShellCwd { reply }) => {
                        if sent.is_err() { break; }
                    }
                }
            }
        })
    }).collect();
    (stop, workers)
}

#[cfg(unix)]
#[tokio::test]
async fn saturated_commands_service_grace_reply_and_ack_retry_deadlines() {
    let dir = tempfile::tempdir().unwrap();
    let backend = LocalTerminalBackend::new_with_tick_interval(Duration::from_secs(60));
    let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
    let command = "trap '' TERM; printf saturated-marker; while :; do sleep 1; done";
    let mut run = request(command, dir.path());
    run.kind = TaskKind::Monitor;
    run.notification_handle = notifications;
    let handle = backend.run_background(run).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while backend.get_task(&handle.task_id).await.unwrap().output != "saturated-marker" {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let (stop, workers) = saturate_commands(&backend);
    let (reply, receiver) = oneshot::channel();
    let started = Instant::now();
    backend.cmd_tx.send(TerminalCommand::Kill {
        task_id: handle.task_id.clone(), source: KillSource::ClientUi,
        deadline: started + Duration::from_millis(300), reply,
    }).await.unwrap();
    let outcome = tokio::time::timeout(Duration::from_millis(900), receiver).await.unwrap().unwrap();
    assert_eq!(outcome, KillOutcome::Indeterminate {
        stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::StopRequested,
    });
    assert!(started.elapsed() < SIGTERM_GRACE);
    let (frozen, ack) = tokio::time::timeout(Duration::from_secs(4), enrollment(&mut deliveries)).await.unwrap();
    assert!(started.elapsed() >= SIGTERM_GRACE, "TERM-ignoring child needs escalation");
    assert_eq!(frozen.task_id, handle.task_id);
    assert_eq!(frozen.kind, TaskKind::Monitor);
    assert_eq!(frozen.owner_session_id.as_deref(), Some("owner"));
    assert_eq!(frozen.command, command);
    assert_eq!(frozen.output, "saturated-marker");
    assert_eq!(frozen.signal.as_deref(), Some("killed"));
    assert!(frozen.completed, "enrollment requires collection");
    assert!(!frozen.kill_result_delivered);
    ack.send(Err("retry under command saturation".into())).unwrap();
    let retry_started = Instant::now();
    let (retry, ack) = tokio::time::timeout(Duration::from_secs(3), enrollment(&mut deliveries)).await.unwrap();
    assert!(retry_started.elapsed() >= Duration::from_secs(1));
    assert_eq!(retry, frozen);
    ack.send(Ok(())).unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(3), backend.wait_for_completion(
        &handle.task_id, Some(Duration::from_secs(2)),
    )).await.unwrap().unwrap();
    assert_eq!(completed.output, "saturated-marker");
    assert_eq!(completed.owner_session_id.as_deref(), Some("owner"));
    assert_eq!(completed.kind, TaskKind::Monitor);
    assert!(!completed.kill_result_delivered);
    stop.cancel();
    for worker in workers { worker.await.unwrap(); }
}

#[cfg(unix)]
#[tokio::test]
async fn detached_drain_keeps_stderr_when_stdout_is_inherited() {
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg("(sleep 4) 2>/dev/null & printf stdout-head; printf stderr-tail >&2; exit 7")
        .stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    crate::util::detach_command(&mut command);
    #[allow(clippy::disallowed_methods)]
    let mut child = command.spawn().unwrap();
    let scope = crate::util::ProcessScope::new();
    let group = scope.enroll(&child).unwrap();
    let status = tokio::time::timeout(Duration::from_secs(1), child.wait()).await.unwrap().unwrap();
    drop(group);
    assert_eq!(scope.live_count(), 0);
    assert_eq!(status.code(), Some(7));
    let (tx, mut rx) = mpsc::channel(1);
    let started = Instant::now();
    spawn_detached_drain(tx.clone(), "literal-drain-id".into(), child.stdout.take(), child.stderr.take(), extract_exit_status(status));
    let drained = tokio::time::timeout(DRAIN_TIMEOUT + Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
    let TerminalCommand::DrainedOutput { task_id, output, status } = drained else { panic!("drain result"); };
    assert_eq!(task_id, "literal-drain-id");
    assert_eq!(output, b"stdout-headstderr-tail");
    assert_eq!(status.exit_code, Some(7));
    assert!(started.elapsed() >= DRAIN_TIMEOUT);
    assert!(started.elapsed() < DRAIN_TIMEOUT + Duration::from_secs(1));
}

#[cfg(unix)]
#[tokio::test]
async fn late_processed_ack_after_public_deadline_does_not_consume_the_result() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, tx, _scope) = owned_actor(dir.path());
    let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
    let command = "printf deadline-marker; sleep 60";
    let mut run = request(command, dir.path());
    run.notification_handle = notifications;
    let (reply, receiver) = oneshot::channel();
    actor.handle_command(TerminalCommand::RunBackground { request: run, reply }).await;
    let handle = receiver.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            actor.poll_process(&handle.task_id).await;
            if actor.processes[&handle.task_id].ring_output() == "deadline-marker" { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    let backend = LocalTerminalBackend { cmd_tx: tx, cancel_token: CancellationToken::new() };
    let started = Instant::now();
    let kill = tokio::spawn({
        let backend = backend.clone();
        let id = handle.task_id.clone();
        async move { backend.kill_task(&id).await }
    });
    let command = actor.cmd_rx.recv().await.unwrap();
    let TerminalCommand::Kill { deadline, .. } = &command else { panic!("public kill command"); };
    let public_deadline = *deadline;
    assert!(public_deadline.duration_since(started) >= Duration::from_secs(13));
    actor.handle_command(command).await;
    let (frozen, ack) = tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            actor.poll_all_processes().await;
            while let Ok(command) = actor.owner_rx.try_recv() {
                actor.handle_command(command).await;
            }
            tokio::select! {
                result = enrollment(&mut deliveries) => break result,
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    }).await.unwrap();
    assert_eq!(frozen.command, "printf deadline-marker; sleep 60");
    assert_eq!(frozen.output, "deadline-marker");
    assert_eq!(frozen.owner_session_id.as_deref(), Some("owner"));
    assert!(!frozen.kill_result_delivered);
    ack.send(Ok(())).unwrap();
    let completion = tokio::time::timeout(Duration::from_secs(1), actor.owner_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(&completion, TerminalCommand::EnrollmentFinished { result: Ok(()), .. }));
    let outcome = tokio::time::timeout(Duration::from_secs(15), kill).await.unwrap().unwrap();
    assert_eq!(outcome, KillOutcome::Indeterminate {
        stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::Unknown,
    });
    assert!(Instant::now() >= public_deadline);
    actor.handle_command(completion).await;
    assert!(matches!(actor.processes[&handle.task_id].enrollment, TerminalEnrollment::Enrolled));
    assert!(!actor.processes[&handle.task_id].kill_result_delivered);
    assert!(!actor.processes[&handle.task_id].block_waited);
    let published = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let delivery = deliveries.recv().await.unwrap();
            if let ToolNotification::TaskCompleted(snapshot) = delivery.notification {
                assert!(delivery.acknowledgement.is_none());
                break snapshot;
            }
        }
    }).await.unwrap();
    assert_eq!(published.task_id, handle.task_id);
    assert_eq!(published.output, "deadline-marker");
    assert!(!published.is_auto_wake_suppressed());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_kill_and_waiter_do_not_consume_frozen_retry_completion() {
    for source in [KillSource::ModelTool, KillSource::ClientUi, KillSource::Teardown] {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalTerminalBackend::new();
        let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
        let command = "printf cancelled-observer-marker; sleep 60";
        let mut run = request(command, dir.path());
        run.notification_handle = notifications;
        run.kind = TaskKind::Monitor;
        let handle = backend.run_background(run).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while backend.get_task(&handle.task_id).await.unwrap().output != "cancelled-observer-marker" {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let waiter = tokio::spawn({
            let backend = backend.clone();
            let id = handle.task_id.clone();
            async move { backend.wait_for_completion(&id, Some(Duration::from_secs(30))).await }
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !backend.get_task(&handle.task_id).await.unwrap().block_waited {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let kill = tokio::spawn({
            let backend = backend.clone();
            let id = handle.task_id.clone();
            async move { backend.kill_task_with_source(&id, source).await }
        });
        let (frozen, ack) = enrollment(&mut deliveries).await;
        assert_eq!(frozen.command, command);
        assert_eq!(frozen.output, "cancelled-observer-marker");
        assert_eq!(frozen.owner_session_id.as_deref(), Some("owner"));
        assert_eq!(frozen.kind, TaskKind::Monitor);
        assert!(frozen.block_waited);
        waiter.abort();
        kill.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(kill.await.unwrap_err().is_cancelled());
        ack.send(Err("retry after observer cancellation".into())).unwrap();
        let (retry, ack) = enrollment(&mut deliveries).await;
        assert_eq!(retry, frozen);
        ack.send(Ok(())).unwrap();
        let published = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let delivery = deliveries.recv().await.unwrap();
                if let ToolNotification::TaskCompleted(snapshot) = delivery.notification {
                    assert!(delivery.acknowledgement.is_none());
                    break snapshot;
                }
            }
        }).await.unwrap();
        assert_eq!(published.task_id, handle.task_id);
        assert_eq!(published.output, "cancelled-observer-marker");
        assert_eq!(published.owner_session_id.as_deref(), Some("owner"));
        assert_eq!(published.kind, TaskKind::Monitor);
        assert!(!published.block_waited);
        assert!(!published.kill_result_delivered);
        assert!(!published.is_auto_wake_suppressed());
    }
}

#[cfg(unix)]
async fn wait_for_collected_drain(actor: &mut LocalTerminalActor, id: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            actor.poll_process(id).await;
            if actor.processes[id].draining {
                assert!(actor.processes[id].child.id().is_none());
                break;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("actual child collection must precede the shutdown cut");
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_retains_collected_foreground_until_inherited_tail_is_drained() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, tx, scope) = owned_actor(dir.path());
    let mut run = request(
        "printf shutdown-head; (i=0; while [ ! -f release ] && [ \"$i\" -lt 500 ]; do i=$((i+1)); sleep 0.01; done; printf '%s' '-tail') & exit 7",
        dir.path(),
    );
    run.kind = TaskKind::Monitor;
    let log = run.output_file.clone();
    let (reply, mut result) = oneshot::channel();
    actor.handle_command(TerminalCommand::Run { request: run, reply }).await;
    let id = actor.processes.keys().next().unwrap().clone();
    wait_for_collected_drain(&mut actor, &id).await;
    assert_eq!(scope.live_count(), 0);
    assert_eq!(actor.processes[&id].owner_session_id.as_deref(), Some("owner"));
    assert_eq!(actor.processes[&id].kind, TaskKind::Monitor);
    assert!(actor.owner_rx.is_empty(), "the gated pipe must still own its delayed tail");
    let cancel = actor.cancel_token.clone();
    cancel.cancel();
    let task = tokio::spawn(actor.run());
    tokio::time::timeout(Duration::from_millis(500), tx.closed()).await.unwrap();
    assert!(!task.is_finished(), "collection alone cannot finish shutdown");
    assert!(matches!(result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    tokio::fs::write(dir.path().join("release"), b"release").await.unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), result).await.unwrap().unwrap().unwrap();
    assert_eq!(output.combined_output, "shutdown-head-tail");
    assert_eq!(output.total_bytes, 18);
    assert_eq!(output.exit_code, Some(7));
    assert_eq!(output.signal, None);
    assert!(!output.timed_out);
    assert_eq!(tokio::fs::read(&log).await.unwrap(), b"shutdown-head-tail");
    tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ShutdownIntakeCut {
    Cancellation,
    ClientsGone,
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_dispatches_a_queued_real_drained_output_before_foreground_reply() {
    for cut in [ShutdownIntakeCut::Cancellation, ShutdownIntakeCut::ClientsGone] {
        let dir = tempfile::tempdir().unwrap();
        let (mut actor, tx, scope) = owned_actor(dir.path());
        let run = request("printf queued-head; (sleep 0.05; printf '%s' '-tail') & exit 19", dir.path());
        let log = run.output_file.clone();
        let (reply, result) = oneshot::channel();
        actor.handle_command(TerminalCommand::Run { request: run, reply }).await;
        let id = actor.processes.keys().next().unwrap().clone();
        wait_for_collected_drain(&mut actor, &id).await;
        let drained = tokio::time::timeout(Duration::from_secs(3), actor.owner_rx.recv()).await.unwrap().unwrap();
        assert!(matches!(&drained, TerminalCommand::DrainedOutput { task_id, status, .. }
            if task_id == &id && status.exit_code == Some(19)));
        actor.owner_tx.send(drained).await.unwrap();
        assert_eq!(scope.live_count(), 0);
        match cut {
            ShutdownIntakeCut::Cancellation => actor.cancel_token.cancel(),
            ShutdownIntakeCut::ClientsGone => drop(tx),
        }
        let task = tokio::spawn(actor.run());
        let output = tokio::time::timeout(Duration::from_secs(3), result).await.unwrap().unwrap().unwrap();
        assert_eq!(output.combined_output, "queued-head-tail");
        assert_eq!(output.total_bytes, 16);
        assert_eq!(output.exit_code, Some(19));
        assert_eq!(output.signal, None);
        assert_eq!(tokio::fs::read(log).await.unwrap(), b"queued-head-tail");
        tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ShutdownAckCut {
    Held,
    Negative,
    QueuedSuccess,
}

#[cfg(unix)]
#[tokio::test]
async fn shutdown_retains_enrollment_waiters_and_kill_replies_through_ack_cuts() {
    for cut in [ShutdownAckCut::Held, ShutdownAckCut::Negative, ShutdownAckCut::QueuedSuccess] {
        for intake in [ShutdownIntakeCut::Cancellation, ShutdownIntakeCut::ClientsGone] {
            shutdown_enrollment_cut(cut, intake).await;
        }
    }
}

#[cfg(unix)]
async fn shutdown_enrollment_cut(cut: ShutdownAckCut, intake: ShutdownIntakeCut) {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, tx, scope) = owned_actor(dir.path());
    let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
    let command = "printf shutdown-completion; exit 23";
    let mut run = request(command, dir.path());
    run.notification_handle = notifications;
    run.kind = TaskKind::Monitor;
    let log = run.output_file.clone();
    let (reply, receiver) = oneshot::channel();
    actor.handle_command(TerminalCommand::RunBackground { request: run, reply }).await;
    let handle = receiver.await.unwrap().unwrap();
    wait_for_collected_drain(&mut actor, &handle.task_id).await;
    assert_eq!(scope.live_count(), 0);
    let (reply, mut waiter) = oneshot::channel();
    actor.handle_command(TerminalCommand::WaitForCompletion {
        task_id: handle.task_id.clone(), timeout: Some(Duration::from_secs(30)), reply,
    }).await;
    let (reply, mut kill) = oneshot::channel();
    let source = match cut {
        ShutdownAckCut::Held => KillSource::ModelTool,
        ShutdownAckCut::Negative => KillSource::ClientUi,
        ShutdownAckCut::QueuedSuccess => KillSource::Teardown,
    };
    actor.handle_command(TerminalCommand::Kill {
        task_id: handle.task_id.clone(), source,
        deadline: Instant::now() + Duration::from_secs(13), reply,
    }).await;
    let drained = tokio::time::timeout(Duration::from_secs(3), actor.owner_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(&drained, TerminalCommand::DrainedOutput { .. }));
    actor.handle_command(drained).await;
    let (frozen, ack) = enrollment(&mut deliveries).await;
    assert_eq!(frozen.task_id, handle.task_id);
    assert_eq!(frozen.command, command);
    assert_eq!(frozen.output, "shutdown-completion");
    assert_eq!(frozen.output_total_bytes, 19);
    assert_eq!(frozen.exit_code, Some(23));
    assert_eq!(frozen.signal, None);
    assert_eq!(frozen.owner_session_id.as_deref(), Some("owner"));
    assert_eq!(frozen.kind, TaskKind::Monitor);
    assert!(frozen.completed);
    assert!(!frozen.kill_result_delivered);
    assert!(frozen.block_waited);
    assert!(matches!(waiter.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    assert!(matches!(kill.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    let ack = match cut {
        ShutdownAckCut::QueuedSuccess => {
            ack.send(Ok(())).unwrap();
            let finished = tokio::time::timeout(Duration::from_secs(1), actor.owner_rx.recv()).await.unwrap().unwrap();
            assert!(matches!(&finished, TerminalCommand::EnrollmentFinished { result: Ok(()), .. }));
            actor.owner_tx.send(finished).await.unwrap();
            None
        }
        ShutdownAckCut::Held | ShutdownAckCut::Negative => Some(ack),
    };
    let rejected = request("printf must-not-run", dir.path());
    let rejected_log = rejected.output_file.clone();
    let (reply, rejected_result) = oneshot::channel();
    tx.send(TerminalCommand::Run { request: rejected, reply }).await.unwrap();
    let tx = match intake {
        ShutdownIntakeCut::Cancellation => {
            actor.cancel_token.cancel();
            Some(tx)
        }
        ShutdownIntakeCut::ClientsGone => {
            drop(tx);
            None
        }
    };
    let task = tokio::spawn(actor.run());
    if let Some(tx) = &tx {
        tokio::time::timeout(Duration::from_millis(500), tx.closed()).await.unwrap();
    }
    assert!(tokio::time::timeout(Duration::from_secs(1), rejected_result).await.unwrap().unwrap().is_err());
    assert!(!rejected_log.exists(), "queued external work must not spawn during shutdown");
    let expected_consumed = match cut {
        ShutdownAckCut::Held => {
            assert!(!task.is_finished());
            assert!(matches!(waiter.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert!(matches!(kill.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            ack.unwrap().send(Ok(())).unwrap();
            assert_eq!(tokio::time::timeout(Duration::from_secs(2), kill).await.unwrap().unwrap(), KillOutcome::AlreadyExited);
            true
        }
        ShutdownAckCut::Negative => {
            assert!(!task.is_finished());
            ack.unwrap().send(Err("shutdown enrollment retained".into())).unwrap();
            assert_eq!(tokio::time::timeout(Duration::from_secs(2), kill).await.unwrap().unwrap(), enrollment_uncertain());
            assert!(!task.is_finished(), "negative ACK cannot discharge shutdown ownership");
            assert!(matches!(waiter.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            let (retry, retry_ack) = tokio::time::timeout(Duration::from_secs(3), enrollment(&mut deliveries)).await.unwrap();
            assert_eq!(retry, frozen);
            assert!(!retry.kill_result_delivered);
            assert!(!task.is_finished());
            retry_ack.send(Ok(())).unwrap();
            false
        }
        ShutdownAckCut::QueuedSuccess => {
            assert_eq!(tokio::time::timeout(Duration::from_secs(2), kill).await.unwrap().unwrap(), KillOutcome::AlreadyExited);
            true
        }
    };
    let completed = tokio::time::timeout(Duration::from_secs(3), waiter).await.unwrap().unwrap().unwrap();
    assert_eq!(completed.task_id, handle.task_id);
    assert_eq!(completed.output, "shutdown-completion");
    assert_eq!(completed.output_total_bytes, 19);
    assert_eq!(completed.exit_code, Some(23));
    assert_eq!(completed.signal, None);
    assert_eq!(completed.owner_session_id.as_deref(), Some("owner"));
    assert_eq!(completed.kind, TaskKind::Monitor);
    assert!(completed.completed);
    assert!(completed.block_waited);
    assert_eq!(completed.kill_result_delivered, expected_consumed);
    tokio::time::timeout(Duration::from_secs(3), task).await.unwrap().unwrap();
    assert_eq!(tokio::fs::read(log).await.unwrap(), b"shutdown-completion");
    let mut publications = 0;
    while let Ok(delivery) = deliveries.try_recv() {
        if let ToolNotification::TaskCompleted(snapshot) = delivery.notification {
            assert!(delivery.acknowledgement.is_none(), "no unserviced enrollment attempt may remain");
            publications += 1;
            assert_eq!(snapshot.task_id, handle.task_id);
            assert_eq!(snapshot.command, command);
            assert_eq!(snapshot.output, "shutdown-completion");
            assert_eq!(snapshot.output_total_bytes, 19);
            assert_eq!(snapshot.exit_code, Some(23));
            assert_eq!(snapshot.signal, None);
            assert_eq!(snapshot.owner_session_id.as_deref(), Some("owner"));
            assert_eq!(snapshot.kind, TaskKind::Monitor);
            assert_eq!(snapshot.kill_result_delivered, expected_consumed);
            assert!(snapshot.block_waited);
        }
    }
    assert_eq!(publications, 1, "one successful ACK must publish exactly one owned completion");
}
