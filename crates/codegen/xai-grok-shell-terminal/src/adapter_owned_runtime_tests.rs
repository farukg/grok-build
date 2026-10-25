use super::*;
use super::resources::{Fixture, Plan};
use xai_grok_tools::notification::{AcknowledgedToolNotification, ToolNotification};

type Deliveries = tokio::sync::mpsc::UnboundedReceiver<AcknowledgedToolNotification>;

async fn enrollment(deliveries: &mut Deliveries) -> (TaskSnapshot, tokio::sync::oneshot::Sender<Result<(), String>>) {
    let delivery = tokio::time::timeout(Duration::from_secs(3), deliveries.recv()).await.unwrap().unwrap();
    let ToolNotification::TaskCompleted(snapshot) = delivery.notification else { panic!("terminal enrollment"); };
    (snapshot, delivery.acknowledgement.expect("no publication before enrollment"))
}

async fn publication(deliveries: &mut Deliveries) -> TaskSnapshot {
    let delivery = tokio::time::timeout(Duration::from_secs(3), deliveries.recv()).await.unwrap().unwrap();
    assert!(delivery.acknowledgement.is_none());
    let ToolNotification::TaskCompleted(snapshot) = delivery.notification else { panic!("terminal publication"); };
    snapshot
}

async fn live_waiters(adapter: &AcpTerminalAdapter, id: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if adapter.tasks.lock().unwrap().get(id).is_some_and(|task| task.live_waiters == count) { break; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}

#[derive(Clone, Copy)]
enum Failure { Held, Negative, Dropped }
#[derive(Clone, Copy)]
enum Observer { Absent, Cancelled, Waiting }

#[tokio::test]
async fn registered_resources_survive_ack_failures_without_false_observation() {
    for failure in [Failure::Held, Failure::Negative, Failure::Dropped] {
        for source in [KillSource::ModelTool, KillSource::ClientUi, KillSource::Teardown] {
            for observer in [Observer::Absent, Observer::Cancelled, Observer::Waiting] {
                let fixture = Fixture::new(Vec::new());
                let adapter = fixture.adapter.clone();
                let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
                let first = adapter.run_background(fixture.request("owner", notifications)).await.unwrap();
                fixture.until(0, |resource| resource.wait_requests == 1).await;
                let waiter = match observer {
                    Observer::Absent => None,
                    Observer::Cancelled | Observer::Waiting => {
                        let waiter = tokio::spawn({
                            let adapter = adapter.clone();
                            let id = first.task_id.clone();
                            async move { adapter.wait_for_completion(&id, Some(Duration::from_secs(30))).await }
                        });
                        live_waiters(&adapter, &first.task_id, 1).await;
                        Some(waiter)
                    }
                };
                let kill = tokio::spawn({
                    let adapter = adapter.clone();
                    let id = first.task_id.clone();
                    async move { adapter.kill_task_with_source(&id, source).await }
                });
                let (frozen, ack) = enrollment(&mut deliveries).await;
                assert_eq!(frozen.task_id, first.task_id);
                assert_eq!(frozen.output, "output:resource-0");
                assert_eq!(frozen.signal.as_deref(), Some("SIGTERM"));
                assert!(frozen.completed);
                assert!(!frozen.block_waited);
                assert!(!frozen.kill_result_delivered);
                assert!(fixture.stats(0).unwrap().exited);
                assert!(fixture.stats(0).unwrap().output_requests >= 1);
                assert_eq!(fixture.stats(0).unwrap().kill_requests, 1);
                assert_eq!(fixture.stats(0).unwrap().releases, 0);
                assert!(!kill.is_finished());
                if let Some(waiter) = &waiter { assert!(!waiter.is_finished()); }
                assert!(deliveries.try_recv().is_err(), "one enrollment attempt per owner");

                let second = tokio::time::timeout(Duration::from_secs(2), adapter.run_background(
                    fixture.request("other", ToolNotificationHandle::noop()),
                )).await.unwrap().unwrap();
                let second_snapshot = tokio::time::timeout(Duration::from_secs(2), adapter.get_task(&second.task_id))
                    .await.unwrap().unwrap();
                assert_eq!(second_snapshot.output, "output:resource-1");
                assert_eq!(tokio::time::timeout(Duration::from_secs(2), adapter.kill_task(&second.task_id)).await.unwrap(), KillOutcome::Killed);
                fixture.until(1, |resource| resource.releases == 1).await;
                assert_eq!(fixture.stats(0).unwrap().releases, 0);
                if matches!(observer, Observer::Cancelled) {
                    waiter.as_ref().unwrap().abort();
                    live_waiters(&adapter, &first.task_id, 0).await;
                }
                match failure {
                    Failure::Held => {
                        tokio::time::timeout(Duration::from_secs(7), async {
                            while !kill.is_finished() { tokio::task::yield_now().await; }
                        }).await.unwrap();
                        drop(ack);
                    }
                    Failure::Negative => { ack.send(Err("negative durable ACK".into())).unwrap(); }
                    Failure::Dropped => drop(ack),
                }
                assert_eq!(tokio::time::timeout(Duration::from_secs(3), kill).await.unwrap().unwrap(), KillOutcome::Indeterminate {
                    stage: KillStage::DurableEnrollment,
                    process_effect: KillProcessEffect::TerminalObserved,
                });
                if let Some(waiter) = waiter {
                    match observer {
                        Observer::Waiting => assert!(waiter.await.unwrap().is_none()),
                        Observer::Cancelled => assert!(waiter.await.unwrap_err().is_cancelled()),
                        Observer::Absent => unreachable!(),
                    }
                }
                let (retry, ack) = enrollment(&mut deliveries).await;
                assert_eq!(retry, frozen, "same frozen fact, including unconsumed flags");
                assert_eq!(fixture.stats(0).unwrap().releases, 0);
                assert!(deliveries.try_recv().is_err(), "no overlapping retry or publication");
                ack.send(Ok(())).unwrap();
                let published = publication(&mut deliveries).await;
                assert_eq!(published.task_id, first.task_id);
                assert_eq!(published.output, "output:resource-0");
                assert_eq!(published.signal.as_deref(), Some("SIGTERM"));
                assert!(!published.block_waited);
                assert!(!published.kill_result_delivered);
                assert!(!published.is_auto_wake_suppressed());
                fixture.until(0, |resource| resource.releases == 1).await;
                assert_eq!(fixture.stats(0).unwrap().wait_requests, 1);
                assert_eq!(adapter.kill_task(&first.task_id).await, KillOutcome::AlreadyExited);
                assert_eq!(adapter.get_task(&first.task_id).await.unwrap().task_id, first.task_id);
                assert!(deliveries.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn registered_wait_delivery_not_live_waiter_count_establishes_consumption() {
    for source in [KillSource::ModelTool, KillSource::ClientUi, KillSource::Teardown] {
        let fixture = Fixture::new(Vec::new());
        let adapter = fixture.adapter.clone();
        let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
        let handle = adapter.run_background(fixture.request("owner", notifications)).await.unwrap();
        let waiter = tokio::spawn({
            let adapter = adapter.clone();
            let id = handle.task_id.clone();
            async move { adapter.wait_for_completion(&id, Some(Duration::from_secs(30))).await }
        });
        live_waiters(&adapter, &handle.task_id, 1).await;
        let kill = tokio::spawn({
            let adapter = adapter.clone();
            let id = handle.task_id.clone();
            async move { adapter.kill_task_with_source(&id, source).await }
        });
        let (frozen, ack) = enrollment(&mut deliveries).await;
        assert!(!frozen.block_waited);
        assert!(!frozen.kill_result_delivered);
        assert!(!waiter.is_finished());
        assert_eq!(fixture.stats(0).unwrap().releases, 0);
        ack.send(Ok(())).unwrap();
        let observed = waiter.await.unwrap().unwrap();
        assert!(observed.completed);
        assert!(observed.block_waited);
        assert!(observed.kill_result_delivered);
        assert_eq!(observed.output, "output:resource-0");
        assert_eq!(observed.signal.as_deref(), Some("SIGTERM"));
        assert_eq!(kill.await.unwrap(), KillOutcome::Killed);
        let published = publication(&mut deliveries).await;
        assert!(published.block_waited);
        assert!(published.kill_result_delivered);
        assert!(published.is_auto_wake_suppressed());
        assert_eq!(published.task_id, frozen.task_id);
        assert_eq!(published.end_time, frozen.end_time);
        fixture.until(0, |resource| resource.releases == 1).await;
    }
}

#[tokio::test]
async fn expired_registered_wait_does_not_freeze_consumption() {
    let fixture = Fixture::new(Vec::new());
    let (notifications, mut deliveries) = ToolNotificationHandle::acknowledged_channel();
    let handle = fixture.adapter.run_background(fixture.request("owner", notifications)).await.unwrap();
    assert!(fixture.adapter.wait_for_completion(&handle.task_id, Some(Duration::from_millis(20))).await.is_none());
    let snapshot = fixture.adapter.get_task(&handle.task_id).await.unwrap();
    assert!(!snapshot.completed);
    assert!(!snapshot.block_waited);
    assert!(!snapshot.kill_result_delivered);
    fixture.exit(0, "natural output", acp::TerminalExitStatus::new().exit_code(Some(7)));
    let (frozen, ack) = enrollment(&mut deliveries).await;
    assert!(!frozen.block_waited);
    assert!(!frozen.kill_result_delivered);
    assert_eq!(frozen.exit_code, Some(7));
    ack.send(Ok(())).unwrap();
    let published = publication(&mut deliveries).await;
    assert!(!published.is_auto_wake_suppressed());
    assert_eq!(published.output, "natural output");
    assert_eq!(published.exit_code, Some(7));
    fixture.until(0, |resource| resource.releases == 1).await;
}

#[tokio::test]
async fn aborting_foreground_final_output_observer_cannot_abandon_its_resource() {
    let fixture = Fixture::new(vec![Plan::HoldOutput]);
    let mut request = fixture.request("owner", ToolNotificationHandle::noop());
    request.auto_background_on_timeout = false;
    let caller = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run(request).await } });
    fixture.until(0, |resource| resource.wait_requests == 1).await;
    fixture.exit(0, "finished bytes", acp::TerminalExitStatus::new().exit_code(Some(0)));
    fixture.until(0, |resource| resource.held_outputs == 1).await;
    assert!(!fixture.adapter.background_foreground_command("same-call").await);
    assert!(!caller.is_finished());
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    fixture.open_output(0);
    fixture.until(0, |resource| resource.releases == 1).await;
    assert!(fixture.adapter.tasks.lock().unwrap().is_empty());
    assert_eq!(fixture.stats(0).unwrap().wait_requests, 1);
    assert_eq!(fixture.stats(0).unwrap().kill_requests, 0);
}

#[tokio::test]
async fn aborting_auto_background_transfer_keeps_one_registered_successor() {
    let fixture = Fixture::new(vec![Plan::HoldOutput]);
    let (notifications, mut events) = ToolNotificationHandle::channel();
    let request = fixture.request("owner", notifications);
    let caller = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run(request).await } });
    fixture.until(0, |resource| resource.held_outputs == 1).await;
    let original = fixture.adapter.list_tasks_metadata().await.pop().unwrap();
    assert!(original.is_backgrounded);
    assert_eq!(fixture.stats(0).unwrap().wait_requests, 1);
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    fixture.open_output(0);
    fixture.until(0, |resource| resource.wait_requests == 2).await;
    fixture.exit(0, "background tail", acp::TerminalExitStatus::new().exit_code(Some(9)));
    let ToolNotification::TaskCompleted(completed) = tokio::time::timeout(Duration::from_secs(3), events.recv()).await.unwrap().unwrap() else { panic!("completion"); };
    assert_eq!(completed.task_id, original.task_id);
    assert_eq!(completed.output, "background tail");
    assert_eq!(completed.exit_code, Some(9));
    fixture.until(0, |resource| resource.releases == 1).await;
    assert_eq!(fixture.stats(0).unwrap().wait_requests, 2);
    assert!(fixture.adapter.get_task(&original.task_id).await.unwrap().completed);
}

#[tokio::test]
async fn aborted_explicit_background_registration_still_transfers_owned_resource() {
    let fixture = Fixture::new(vec![Plan::HoldCreate]);
    let (notifications, mut events) = ToolNotificationHandle::channel();
    let request = fixture.request("owner", notifications);
    let caller = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run_background(request).await } });
    fixture.until(0, |resource| resource.held_creates == 1).await;
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    fixture.open_create(0);
    fixture.until(0, |resource| resource.wait_requests == 1).await;
    let registered = fixture.adapter.list_tasks_metadata().await.pop().unwrap();
    assert_ne!(registered.task_id, "same-call");
    assert_ne!(registered.task_id, "resource-0");
    fixture.exit(0, "registered after observer abort", acp::TerminalExitStatus::new().exit_code(Some(3)));
    let ToolNotification::TaskCompleted(completed) = tokio::time::timeout(Duration::from_secs(3), events.recv()).await.unwrap().unwrap() else { panic!("completion"); };
    assert_eq!(completed.task_id, registered.task_id);
    assert_eq!(completed.output, "registered after observer abort");
    assert_eq!(completed.exit_code, Some(3));
    fixture.until(0, |resource| resource.releases == 1).await;
    assert_eq!(fixture.stats(0).unwrap().wait_requests, 1);
    assert!(fixture.adapter.get_task(&registered.task_id).await.unwrap().completed);
}

#[tokio::test]
async fn scoped_stop_addresses_registered_resources_without_stopping_survivors() {
    let fixture = Fixture::new(Vec::new());
    let mut a = fixture.request("a", ToolNotificationHandle::noop());
    a.foreground_block_budget = Some(Duration::MAX);
    let foreground_a = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run(a).await } });
    fixture.until(0, |resource| resource.wait_requests == 1).await;
    let mut b = fixture.request("b", ToolNotificationHandle::noop());
    b.foreground_block_budget = Some(Duration::MAX);
    let foreground_b = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run(b).await } });
    fixture.until(1, |resource| resource.wait_requests == 1).await;
    let background_a = fixture.adapter.run_background(fixture.request("a", ToolNotificationHandle::noop())).await.unwrap();
    let background_b = fixture.adapter.run_background(fixture.request("b", ToolNotificationHandle::noop())).await.unwrap();
    fixture.adapter.kill_foreground_commands_by_owner("a").await;
    fixture.until(0, |resource| resource.releases == 1).await;
    assert_eq!(fixture.stats(0).unwrap().kill_requests, 1);
    for index in [1, 2, 3] {
        assert_eq!(fixture.stats(index).unwrap().kill_requests, 0);
        assert_eq!(fixture.stats(index).unwrap().releases, 0);
    }
    let a = foreground_a.await.unwrap().unwrap();
    let RunDisposition::Backgrounded { handle, .. } = a.disposition else { panic!("stop transfers foreground ownership"); };
    assert!(fixture.adapter.get_task(&handle.task_id).await.unwrap().completed);
    fixture.adapter.kill_foreground_commands().await;
    fixture.until(1, |resource| resource.releases == 1).await;
    assert!(matches!(foreground_b.await.unwrap().unwrap().disposition, RunDisposition::Backgrounded { .. }));
    assert_eq!(fixture.stats(2).unwrap().kill_requests, 0);
    assert_eq!(fixture.stats(3).unwrap().kill_requests, 0);
    fixture.adapter.kill_all_background_tasks_by_owner("a").await;
    fixture.until(2, |resource| resource.releases == 1).await;
    assert_eq!(fixture.stats(3).unwrap().kill_requests, 0);
    assert!(!fixture.adapter.get_task(&background_b.task_id).await.unwrap().completed);
    assert!(fixture.adapter.get_task(&background_a.task_id).await.unwrap().completed);
    fixture.adapter.kill_all_background_tasks().await;
    fixture.until(3, |resource| resource.releases == 1).await;
    for index in 0..4 { assert_eq!(fixture.stats(index).unwrap().kill_requests, 1); }
    assert!(fixture.calls.lock().unwrap().iter().all(|(_, resource)| resource.starts_with("resource-")));
}

#[tokio::test]
async fn stopping_foreground_finishing_uses_its_bound_resource_and_completion_owner() {
    let fixture = Fixture::new(vec![Plan::HoldOutput]);
    let mut request = fixture.request("owner", ToolNotificationHandle::noop());
    request.auto_background_on_timeout = false;
    let caller = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.run(request).await } });
    fixture.until(0, |resource| resource.wait_requests == 1).await;
    fixture.exit(0, "real terminal fact", acp::TerminalExitStatus::new().exit_code(Some(4)));
    fixture.until(0, |resource| resource.held_outputs == 1).await;
    let stop = tokio::spawn({ let adapter = fixture.adapter.clone(); async move { adapter.kill_foreground_commands_by_owner("owner").await } });
    fixture.until(0, |resource| resource.kill_requests == 1).await;
    assert!(!stop.is_finished());
    assert_eq!(fixture.stats(0).unwrap().releases, 0);
    fixture.open_output(0);
    stop.await.unwrap();
    let result = caller.await.unwrap().unwrap();
    assert!(matches!(result.disposition, RunDisposition::Foreground));
    assert_eq!(result.combined_output, "real terminal fact");
    assert_eq!(result.exit_code, Some(4));
    fixture.until(0, |resource| resource.releases == 1).await;
    let completed = fixture.adapter.list_tasks_metadata().await.pop().unwrap();
    assert!(completed.completed);
    assert!(completed.kill_result_delivered);
}

#[tokio::test]
async fn registered_rpc_refusal_or_loss_cannot_discard_its_execution_owner() {
    for plan in [Plan::UnsupportedKill, Plan::DropKill] {
        let fixture = Fixture::new(vec![plan]);
        let handle = fixture.adapter.run_background(fixture.request("owner", ToolNotificationHandle::noop())).await.unwrap();
        fixture.until(0, |resource| resource.wait_requests == 1).await;
        let expected = match plan {
            Plan::UnsupportedKill => KillOutcome::Failed { stage: KillStage::ProcessTermination, reason: KillFailureReason::Unsupported },
            Plan::DropKill => KillOutcome::Indeterminate { stage: KillStage::ProcessTermination, process_effect: KillProcessEffect::Unknown },
            Plan::Ready | Plan::HoldCreate | Plan::HoldOutput | Plan::HoldKill => unreachable!(),
        };
        assert_eq!(fixture.adapter.kill_task(&handle.task_id).await, expected);
        let pending = fixture.adapter.get_task(&handle.task_id).await.unwrap();
        assert!(!pending.completed);
        assert!(!pending.block_waited);
        assert!(!pending.kill_result_delivered);
        assert_eq!(fixture.stats(0).unwrap().releases, 0);
        fixture.exit(0, "later independent exit", acp::TerminalExitStatus::new().exit_code(Some(8)));
        fixture.until(0, |resource| resource.releases == 1).await;
        let completed = fixture.adapter.get_task(&handle.task_id).await.unwrap();
        assert!(completed.completed);
        assert_eq!(completed.exit_code, Some(8));
        assert_eq!(completed.output, "later independent exit");
        assert!(!completed.kill_result_delivered);
    }
}

#[test]
fn explicit_background_caller_abort_during_real_recorder_initialization_keeps_owner() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().max_blocking_threads(1).build().unwrap();
    runtime.block_on(async {
        let (release_worker, hold_worker) = std::sync::mpsc::channel::<()>();
        let (started, ready) = tokio::sync::oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            let _ = hold_worker.recv();
        });
        ready.await.unwrap();
        let fixture = Fixture::new(Vec::new());
        let request = fixture.request("owner", ToolNotificationHandle::noop());
        let caller = tokio::spawn({
            let adapter = fixture.adapter.clone();
            async move { adapter.run_background(request).await }
        });
        let registered = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(snapshot) = fixture.adapter.list_tasks_metadata().await.pop() { break snapshot; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(fixture.stats(0).unwrap().wait_requests, 0, "real recorder initialization is queued behind the held filesystem worker");
        assert!(!caller.is_finished());
        caller.abort();
        assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
        assert_eq!(fixture.stats(0).unwrap().releases, 0);
        assert!(fixture.adapter.tasks.lock().unwrap().contains_key(&registered.task_id));
        release_worker.send(()).unwrap();
        worker.await.unwrap();
        fixture.until(0, |resource| resource.wait_requests == 1).await;
        fixture.exit(0, "continued after initialization", acp::TerminalExitStatus::new().exit_code(Some(6)));
        fixture.until(0, |resource| resource.releases == 1).await;
        let completed = fixture.adapter.get_task(&registered.task_id).await.unwrap();
        assert!(completed.completed);
        assert_eq!(completed.output, "continued after initialization");
        assert_eq!(completed.exit_code, Some(6));
        assert_eq!(fixture.stats(0).unwrap().wait_requests, 1);
    });
}
