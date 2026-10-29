use super::*;
use crate::session::current_actions::{
    CompactOwner, CompactStage, CurrentActions, CurrentActionsSnapshot, CurrentSamplingPhase,
    CurrentTurnPhase, FrozenOwner, SnapshotBinding,
};
use std::time::Duration;

fn tag(actor: &SessionActor) -> SnapshotBinding {
    SnapshotBinding {
        session_id: actor.session_info.id.0.to_string(),
        binding_epoch: 11,
        load_generation: 4,
    }
}

fn capture(actor: &SessionActor) -> CurrentActions {
    let CurrentActionsSnapshot::Current(mut snapshot) =
        actor.current_actions_reader().snapshot(tag(actor))
    else {
        panic!("resident source");
    };
    // The native cursor is process-wide; unrelated sessions may allocate between reads.
    snapshot.event_frontier = 0;
    snapshot
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn readonly_snapshot_during_delayed_real_inference_neither_waits_for_actor_nor_acknowledges_work()
 {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = fixture().await;
        let server = route_to_mock(&fixture.actor).await;
        fixture.actor.notifications.gateway_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
        server.set_foreground_hold();
        let mut prompt = submit_named_input(&fixture, "held-real-model", false);
        foreground_parked(&mut fixture, &server).await;
        while fixture.persistence.try_recv().is_ok() {}
        let reader = fixture.actor.current_actions_reader();
        let binding = tag(&fixture.actor);
        let first = reader.snapshot(binding.clone());
        let observed = capture(&fixture.actor);
        assert!(matches!(&observed.turn.as_ref().unwrap().owner, FrozenOwner::Root { prompt_id, .. } if prompt_id == "held-real-model"));
        assert!(matches!(observed.turn.as_ref().unwrap().phase, CurrentTurnPhase::Running { .. }));
        assert!(observed.sampling.iter().any(|request| matches!(request.phase,
            CurrentSamplingPhase::Reserved | CurrentSamplingPhase::Submitted { .. }
        )));
        assert!(!serde_json::to_string(&observed).unwrap().contains("new input"), "actual model prompt text is not continuation metadata");
        let prompt_id = fixture.actor.current_prompt_id.lock().unwrap().clone();
        let active_work = fixture.actor.active_work.load(std::sync::atomic::Ordering::Acquire);
        let actor_lock = fixture.actor.state.try_lock().ok();
        assert_eq!(capture(&fixture.actor), observed, "reader cannot queue behind the working actor or take its async state lock");
        assert_eq!(fixture.actor.current_prompt_id.lock().unwrap().clone(), prompt_id);
        assert_eq!(fixture.actor.active_work.load(std::sync::atomic::Ordering::Acquire), active_work);
        assert!(matches!(prompt.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        assert!(fixture.persistence.try_recv().is_err(), "query emits no persistence, ACK, activity, or work record");
        drop(actor_lock);
        tokio::time::advance(Duration::from_secs(30)).await;
        let cold = capture(&fixture.actor);
        assert_eq!(cold.turn, observed.turn, "cold viewer attach keeps the original task and phase anchors");
        assert_eq!(cold.sampling.iter().map(|request| &request.owner).collect::<Vec<_>>(), observed.sampling.iter().map(|request| &request.owner).collect::<Vec<_>>());
        let mut foreign = binding.clone();
        foreign.session_id.push_str("-foreign");
        assert_eq!(reader.snapshot(foreign), CurrentActionsSnapshot::Unavailable);
        server.release_parked_replies();
        let terminal = loop {
            tokio::select! {
                result = &mut prompt => break result.unwrap(),
                record = fixture.persistence.recv() => match record.unwrap() {
                    PersistenceMsg::FlushAndAck { respond_to } | PersistenceMsg::ProbeWritable { respond_to } => { respond_to.send(Ok(())).unwrap(); }
                    PersistenceMsg::AppendUpdateDurablyAndAck { respond_to, .. } => { respond_to.send(Ok(())).unwrap(); }
                    _ => {}
                }
            }
        };
        assert!(terminal.is_ok());
        let settled = capture(&fixture.actor);
        assert!(settled.turn.is_none());
        assert!(settled.sampling.is_empty());
        assert!(settled.tools.is_empty());
        assert!(!first.is_current_for(&binding, Some(settled.version), None));
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actual_compaction_registration_execution_and_original_rpc_have_distinct_source_stages() {
    tokio::task::LocalSet::new().run_until(async {
        let mut fixture = fixture().await;
        let server = route_to_mock(&fixture.actor).await;
        fixture.actor.notifications.gateway_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
        let (reached, reservation) = oneshot::channel();
        let (release, proceed) = oneshot::channel();
        fixture.actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, proceed)));
        server.set_auxiliary_hold();
        let invocation_id = crate::session::acp_types::CompactInvocationId(uuid::Uuid::new_v4());
        let (respond_to, mut response) = oneshot::channel();
        fixture.sender.send(SessionCommand::CompactSession {
            invocation_id: Some(invocation_id), cancellation: tokio_util::sync::CancellationToken::new(), respond_to,
        }).unwrap();
        reservation.await.unwrap();
        let queued = capture(&fixture.actor);
        let [queued_compact] = queued.compactions.as_slice() else { panic!("one original request"); };
        assert_eq!(queued_compact.stage, CompactStage::Queued);
        assert!(matches!(queued_compact.owner, CompactOwner::Manual { invocation_id: Some(id), .. } if id == invocation_id));
        assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        let admitted_at = queued_compact.admitted_at;
        let original_owner = queued_compact.owner.clone();
        release.send(()).unwrap();
        server.wait_until_parked_replies(1).await;
        let executing = capture(&fixture.actor);
        assert_eq!(executing.compactions[0].stage, CompactStage::Executing);
        assert_eq!(executing.compactions[0].owner, original_owner);
        assert_eq!(executing.compactions[0].admitted_at, admitted_at);
        let while_awaiting_model = fixture.actor.current_actions_reader().snapshot(tag(&fixture.actor));
        let executing_state = capture(&fixture.actor);
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(capture(&fixture.actor), executing_state);
        server.release_parked_replies();
        let persisted = compact_barrier(&mut fixture).await;
        let awaiting_original_rpc = capture(&fixture.actor);
        assert_eq!(awaiting_original_rpc.compactions[0].stage, CompactStage::AwaitingPersistence);
        assert_eq!(awaiting_original_rpc.compactions[0].admitted_at, admitted_at);
        assert_eq!(awaiting_original_rpc.compactions[0].owner, original_owner);
        assert!(matches!(response.try_recv(), Err(oneshot::error::TryRecvError::Empty)), "a snapshot cannot settle the original RPC");
        assert!(!while_awaiting_model.is_current_for(&tag(&fixture.actor), Some(awaiting_original_rpc.version), None));
        persisted.send(Ok(())).unwrap();
        response.await.unwrap().unwrap();
        assert!(capture(&fixture.actor).compactions.is_empty(), "before-load RPC terminal cannot be resurrected");
    }).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn genuine_root_child_and_wake_task_registrations_retain_anchors_and_cancel_before_start_without_resurrection()
 {
    tokio::task::LocalSet::new().run_until(async {
        for (child, origin) in [
            (false, PromptOrigin::User),
            (true, PromptOrigin::User),
            (false, PromptOrigin::TaskCompleted { task_id: "completed-fixture-task".into() }),
        ] {
            let (gateway, _messages) = mpsc::unbounded_channel();
            let (persistence, _records) = mpsc::unbounded_channel();
            let (mut actor, _events) = create_test_actor_ex(0, 200_000, 85, gateway, persistence).await;
            actor.startup_hints.is_subagent = child;
            let actor = Arc::new(actor);
            let (gate, start_gate) = oneshot::channel();
            let (completed, _completion) = mpsc::unbounded_channel();
            let task = AgentTask::new_prompt(actor.clone(), TurnInputRequest {
                prompt_id: "registered-runtime-task".into(), input_origin: InputOrigin::new(origin.clone()),
                prompt_blocks: vec![], prompt_mode: PromptMode::Agent, trace_gcs_config: None,
                artifact_tracker: None, client_identifier: None, screen_mode: None, verbatim: false,
                send_now: false, json_schema: None, persist_ack: None, parsed_prompt_tx: None,
                traceparent: None, start_gate: Some(start_gate),
            }, TurnEpoch::default(), completed);
            tokio::task::yield_now().await;
            let first = capture(&actor);
            let turn = first.turn.as_ref().unwrap();
            assert_eq!(turn.phase, CurrentTurnPhase::Admitted);
            let correct = match (&turn.owner, child, &origin) {
                (FrozenOwner::Root { task_id, .. }, false, PromptOrigin::User)
                | (FrozenOwner::Child { task_id, .. }, true, PromptOrigin::User)
                | (FrozenOwner::Wake { task_id, .. }, false, PromptOrigin::TaskCompleted { .. }) => task_id == task.identity.as_str(),
                _ => false,
            };
            assert!(correct, "frozen provenance comes from the actual task registration, not prompt text or a replay marker");
            tokio::time::advance(Duration::from_secs(30)).await;
            assert_eq!(capture(&actor), first, "there is no focused-tab or rendering dependency");
            task.abort();
            let settled = capture(&actor);
            assert!(settled.turn.is_none());
            assert!(settled.sampling.is_empty());
            drop(gate);
            tokio::task::yield_now().await;
            assert_eq!(capture(&actor), settled);
        }
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn actual_queued_compact_cancellation_is_terminal_before_any_load_or_dispatch() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let fixture = fixture().await;
            fixture.actor.state.lock().await.running_task =
                Some(running_task_stub("preceding-task"));
            let cancellation = tokio_util::sync::CancellationToken::new();
            let (respond_to, response) = oneshot::channel();
            fixture
                .sender
                .send(SessionCommand::CompactSession {
                    invocation_id: None,
                    cancellation: cancellation.clone(),
                    respond_to,
                })
                .unwrap();
            command_boundary(&fixture).await;
            let queued = capture(&fixture.actor);
            assert_eq!(queued.compactions[0].stage, CompactStage::Queued);
            assert!(
                matches!(
                    queued.compactions[0].owner,
                    CompactOwner::Manual {
                        invocation_id: None,
                        ..
                    }
                ),
                "legacy callers keep the real internal admission identity, not a forged invocation"
            );
            cancellation.cancel();
            response.await.unwrap().unwrap_err();
            assert!(capture(&fixture.actor).compactions.is_empty());
        })
        .await;
}
