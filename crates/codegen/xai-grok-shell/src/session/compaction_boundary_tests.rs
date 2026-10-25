use super::*;
use crate::session::acp_session::support;
use crate::session::completion_delivery::{CompletionArtifacts, CompletionEventPolicy, SubagentCompletionEvent};

async fn completion_fixture() -> support::CompletionWakeReceiverFixture {
    let (gateway, gateway_events) = mpsc::unbounded_channel();
    support::drain_gateway(gateway_events);
    let (mut actor, events, writes) = support::create_observed_completion_test_actor(
        50_000, 200_000, 85, gateway,
    ).await;
    let sampling_events = wire_sampler(&mut actor).await;
    let actor = Arc::new(actor);
    seed_conversation(&actor);
    let (sender, task) = start_fixture_actor(actor.clone(), events, sampling_events);
    support::CompletionWakeReceiverFixture { actor, sender, writes, task }
}

async fn publish_completion(
    actor: &SessionActor,
    sender: &mpsc::UnboundedSender<SessionCommand>,
    kind: crate::session::CompletionWakeKind,
    policy: CompletionEventPolicy,
    id: &str,
) -> (crate::session::CompletionWake, oneshot::Receiver<PromptTurnResult>) {
    let completion = crate::session::CompletionWake {
        id: xai_chat_state::SubagentCompletionId {
            subagent_id: id.into(),
            attempt_id: xai_message_delivery_core::AttemptId::mint(1),
        },
        kind,
    };
    let event = SubagentCompletionEvent {
        id: completion.id.clone(),
        state: xai_grok_tools::implementations::grok_build::task::types::SubagentState::Failed {
            message: format!("{id}: original child result"),
        },
        text: format!("{id}: original child result"),
        output: format!("{id}: original child result"),
        child_session_id: format!("{id}-session"),
        tool_calls: 2,
        turns: 1,
        duration_ms: 100,
        tokens_used: 11,
        completed_at: chrono::Utc::now(),
        policy,
        artifacts: CompletionArtifacts::None,
    };
    let (respond_to, response) = oneshot::channel();
    actor.notifications.persistence_tx.send(PersistenceMsg::PrepareSubagentCompletion {
        event,
        respond_to,
    }).unwrap();
    let committed = response.await.unwrap().unwrap();
    let (respond_to, response) = oneshot::channel();
    sender.send(SessionCommand::PublishSubagentCompletion {
        event: committed.event,
        will_wake: true,
        respond_to,
    }).unwrap();
    let published = response.await.unwrap().unwrap();
    let (respond_to, response) = oneshot::channel();
    sender.send(SessionCommand::SubagentCompletionWake {
        prompt_id: crate::session::completion_delivery::completion_wake_prompt_id(&completion),
        completion: completion.clone(),
        text: published.text,
        respond_to,
    }).unwrap();
    let (respond_to, boundary) = oneshot::channel();
    sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
    boundary.await.unwrap();
    (completion, response)
}

fn seed_conversation(actor: &SessionActor) {
    actor.chat_state_handle.replace_conversation(vec![
        ConversationItem::system("sys"),
        ConversationItem::user("original task"),
        ConversationItem::assistant("Original completed work. ".repeat(40)),
        ConversationItem::user("final query"),
    ]);
}

#[tokio::test(flavor = "current_thread")]
async fn boundary_only_completion_does_not_cycle_with_compact_and_later_human_input() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let support::CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } =
                completion_fixture().await;
            let server = route_to_mock(&actor).await;
            server.set_foreground_hold();
            let (completion, _wake_response) = publish_completion(&actor, &sender,
                crate::session::CompletionWakeKind::Boundary,
                CompletionEventPolicy::BoundaryOnly, "compact-boundary-child").await;
            let identity: xai_chat_state::completion::CompletionIdentity = completion.id.clone().into();
            assert!(actor.persisted_completion_state().await.unwrap().obligations.iter().any(|obligation| obligation.id() == identity));
            assert!(actor.state.lock().await.running_task.is_none(), "boundary-only completion cannot own a caller turn alone");

            let (respond_to, compact) = oneshot::channel();
            sender.send(SessionCommand::CompactSession {
                invocation_id: None,
                cancellation: tokio_util::sync::CancellationToken::new(),
                respond_to,
            }).unwrap();
            let later = send_named_input(&sender, "boundary-followup", false);
            compact.await.unwrap().expect("the original compact must not require boundary-only completion promotion");
            server.wait_until_parked_replies(1).await;
            assert_eq!(actor.state.lock().await.running_task.as_ref().unwrap().prompt_id, "boundary-followup");
            let delivery = actor.persisted_completion_state().await.unwrap();
            assert_eq!(delivery.receipts.iter().filter(|receipt| receipt.completion == identity).count(), 1);
            assert!(delivery.obligations.iter().any(|obligation| obligation.id() == identity),
                "the projected obligation remains owned until the later caller settles");
            assert_eq!(actor.chat_state_handle.get_conversation().await.iter().filter(|item| matches!(item,
                ConversationItem::User(user) if user.synthetic_reason == xai_grok_sampling_types::SyntheticReason::SubagentCompleted
                    && item.text_content().contains("compact-boundary-child"))).count(), 1,
                "maintenance cannot drop or duplicate the completion's history projection");
            server.release_parked_replies();
            later.await.unwrap().expect("the real later prompt must progress after the original compact");
            let delivery = actor.persisted_completion_state().await.unwrap();
            assert_eq!(delivery.receipts.iter().filter(|receipt| receipt.completion == identity).count(), 1);
        }).await.expect("boundary-only completion, original compact, and later prompt must make progress without a cycle");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn compact_skips_stop_parked_runtime_predecessor_without_reopening_its_wake_barrier() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let support::CompletionWakeReceiverFixture { actor, sender, task: _task, writes: _writes } =
                completion_fixture().await;
            let _server = route_to_mock(&actor).await;
            sender.send(SessionCommand::Cancel(CancelOptions {
                trigger: Some(CancelTrigger::CtrlC),
                user_initiated: true,
                ..Default::default()
            })).unwrap();
            let (respond_to, stopped) = oneshot::channel();
            sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
            stopped.await.unwrap();
            assert!(actor.state.lock().await.notifications_suppressed);
            let (completion, mut wake_response) = publish_completion(&actor, &sender,
                crate::session::CompletionWakeKind::Fresh,
                CompletionEventPolicy::Automatic, "compact-stopped-child").await;
            let identity: xai_chat_state::completion::CompletionIdentity = completion.id.clone().into();
            let (respond_to, compact) = oneshot::channel();
            sender.send(SessionCommand::CompactSession {
                invocation_id: None,
                cancellation: tokio_util::sync::CancellationToken::new(),
                respond_to,
            }).unwrap();
            compact.await.unwrap().expect("a stop-parked runtime obligation is not a maintenance prerequisite");
            let state = actor.state.lock().await;
            assert!(state.notifications_suppressed);
            assert!(state.running_task.is_none());
            assert!(state.pending_inputs.iter().any(|input| matches!(input.input_origin.as_prompt_origin(),
                crate::session::PromptOrigin::SubagentCompleted { completion: queued } if queued.id == completion.id)));
            drop(state);
            let delivery = actor.persisted_completion_state().await.unwrap();
            assert_eq!(delivery.wake_policy, crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
            assert!(delivery.obligations.iter().any(|obligation| obligation.id() == identity));
            assert!(!delivery.receipts.iter().any(|receipt| receipt.completion == identity));
            assert!(matches!(wake_response.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
        }).await.expect("maintenance must finish without clearing a stopped session's barrier");
    }).await;
}

#[derive(Clone, Copy)]
enum ManualStopPhase {
    Reserved,
    Generating,
    OwnAck,
}

#[tokio::test(flavor = "current_thread")]
async fn actual_ctrl_c_during_manual_maintenance_parks_automatic_completion_until_human_input() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            for phase in [ManualStopPhase::Reserved, ManualStopPhase::Generating, ManualStopPhase::OwnAck] {
                let (gateway, gateway_events) = mpsc::unbounded_channel();
                support::drain_gateway(gateway_events);
                let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                    50_000, 200_000, 85, gateway,
                ).await;
                let sampling_events = wire_sampler(&mut actor).await;
                let persistence = actor.notifications.persistence_tx.clone();
                let (relay, mut relayed) = mpsc::unbounded_channel();
                let (held_ack, mut acks) = mpsc::unbounded_channel();
                let disk = persistence.clone();
                let relay_task = tokio::task::spawn_local(async move {
                    while let Some(message) = relayed.recv().await {
                        match message {
                            message @ PersistenceMsg::FlushAndAck { .. } => held_ack.send(message).unwrap(),
                            message => disk.send(message).unwrap(),
                        }
                    }
                });
                actor.notifications.persistence_tx = relay;
                let actor = Arc::new(actor);
                seed_conversation(&actor);
                actor.goal_tracker.lock().create_goal(
                    "maintenance-goal".into(), "retained objective".into(), None, 0,
                    chrono::Utc::now().to_rfc3339(), None,
                );
                let (sender, _task) = start_fixture_actor(actor.clone(), events, sampling_events);
                let server = route_to_mock(&actor).await;
                let mut reservation_release = None;
                let mut reserved = None;
                match phase {
                    ManualStopPhase::Reserved => {
                        let (reached, reached_rx) = oneshot::channel();
                        let (release, released) = oneshot::channel();
                        actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, released)));
                        reservation_release = Some(release);
                        reserved = Some(reached_rx);
                    }
                    ManualStopPhase::Generating => server.set_auxiliary_hold(),
                    ManualStopPhase::OwnAck => {}
                }
                let (respond_to, mut compact) = oneshot::channel();
                sender.send(SessionCommand::CompactSession {
                    invocation_id: None,
                    cancellation: tokio_util::sync::CancellationToken::new(), respond_to,
                }).unwrap();
                let own_ack = match phase {
                    ManualStopPhase::Reserved => { reserved.unwrap().await.unwrap(); None }
                    ManualStopPhase::Generating => { server.wait_until_parked_replies(1).await; None }
                    ManualStopPhase::OwnAck => Some(acks.recv().await.unwrap()),
                };
                let (completion, mut wake) = publish_completion(&actor, &sender,
                    crate::session::CompletionWakeKind::Fresh, CompletionEventPolicy::Automatic,
                    "maintenance-stop-child").await;
                sender.send(SessionCommand::Cancel(CancelOptions {
                    trigger: Some(CancelTrigger::CtrlC), user_initiated: true, ..Default::default()
                })).unwrap();
                let (respond_to, stopped) = oneshot::channel();
                sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
                stopped.await.unwrap();
                assert!(actor.state.lock().await.notifications_suppressed);
                assert_eq!(actor.goal_tracker.lock().status(),
                    Some(crate::session::goal_tracker::GoalStatus::UserPaused));
                match phase {
                    ManualStopPhase::OwnAck => {
                        assert!(actor.state.lock().await.finalization_gate.is_active());
                        assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                        persistence.send(own_ack.unwrap()).unwrap();
                        compact.await.unwrap().expect("committed compact keeps its own durable success");
                    }
                    ManualStopPhase::Reserved | ManualStopPhase::Generating => {
                        if let Some(release) = reservation_release { release.send(()).unwrap(); }
                        let error = compact.await.unwrap().unwrap_err();
                        assert_eq!(crate::session::helpers::session_compact::compact_error_kind(&error),
                            Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled));
                    }
                }
                let (respond_to, finished) = oneshot::channel();
                sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
                finished.await.unwrap();
                let identity: xai_chat_state::completion::CompletionIdentity = completion.id.clone().into();
                let delivery = actor.persisted_completion_state().await.unwrap();
                assert_eq!(delivery.wake_policy, crate::session::completion_delivery::CompletionWakePolicy::HumanStopped);
                assert!(delivery.obligations.iter().any(|row| row.id() == identity));
                assert!(!delivery.receipts.iter().any(|row| row.completion == identity));
                assert!(actor.state.lock().await.notifications_suppressed);
                assert!(actor.state.lock().await.running_task.is_none());
                assert!(matches!(wake.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0,
                    "maintenance completion must not admit automatic foreground inference after Ctrl+C");
                server.release_parked_replies();
                let human = send_named_input(&sender, "real-human-reengagement", false);
                tokio::pin!(human);
                loop {
                    tokio::select! {
                        result = &mut human => { result.unwrap().unwrap(); break; }
                        message = acks.recv() => persistence.send(message.unwrap()).unwrap(),
                    }
                }
                assert!(actor.persisted_completion_state().await.unwrap().receipts.iter()
                    .any(|row| row.completion == identity));
                relay_task.abort();
            }
        }).await.expect("actual stop must retain the wake barrier through all manual maintenance phases");
    }).await;
}

#[derive(Clone, Copy)]
enum NamedRewindTarget {
    Stale,
    Queued,
}

#[tokio::test(flavor = "current_thread")]
async fn named_rewind_during_manual_maintenance_preserves_original_request_and_own_ack() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            for phase in [ManualStopPhase::Reserved, ManualStopPhase::Generating] {
                for target in [NamedRewindTarget::Stale, NamedRewindTarget::Queued] {
                    let (gateway, gateway_events) = mpsc::unbounded_channel();
                    support::drain_gateway(gateway_events);
                    let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                        50_000, 200_000, 85, gateway,
                    ).await;
                    let sampling_events = wire_sampler(&mut actor).await;
                    let persistence = actor.notifications.persistence_tx.clone();
                    let (relay, mut relayed) = mpsc::unbounded_channel();
                    let (held_ack, mut acks) = mpsc::unbounded_channel();
                    let disk = persistence.clone();
                    let relay_task = tokio::task::spawn_local(async move {
                        while let Some(message) = relayed.recv().await {
                            match message {
                                message @ PersistenceMsg::FlushAndAck { .. } => held_ack.send(message).unwrap(),
                                message => disk.send(message).unwrap(),
                            }
                        }
                    });
                    actor.notifications.persistence_tx = relay;
                    let actor = Arc::new(actor);
                    seed_conversation(&actor);
                    actor.goal_tracker.lock().create_goal(
                        "named-rewind-goal".into(), "retained objective".into(), None, 0,
                        chrono::Utc::now().to_rfc3339(), None,
                    );
                    let goal_status = actor.goal_tracker.lock().status();
                    let (sender, _task) = start_fixture_actor(actor.clone(), events, sampling_events);
                    let server = route_to_mock(&actor).await;
                    let reservation = match phase {
                        ManualStopPhase::Reserved => {
                            let (reached, reached_rx) = oneshot::channel();
                            let (release, released) = oneshot::channel();
                            actor.compaction.cancel.manual_reservation_pause.replace(Some((reached, released)));
                            Some((reached_rx, release))
                        }
                        ManualStopPhase::Generating => { server.set_auxiliary_hold(); None }
                        ManualStopPhase::OwnAck => unreachable!(),
                    };
                    let cancellation = tokio_util::sync::CancellationToken::new();
                    let (respond_to, mut compact) = oneshot::channel();
                    sender.send(SessionCommand::CompactSession {
                        invocation_id: None,
                        cancellation: cancellation.clone(), respond_to,
                    }).unwrap();
                    let reservation_release = if let Some((reached, release)) = reservation {
                        reached.await.unwrap();
                        Some(release)
                    } else {
                        server.wait_until_parked_replies(1).await;
                        None
                    };
                    let pending_id = actor.compaction.cancel.pending_manual.borrow().front().map(|row| row.id);
                    match phase {
                        ManualStopPhase::Reserved => assert!(pending_id.is_some()),
                        ManualStopPhase::Generating => {
                            assert!(pending_id.is_none());
                            assert!(actor.compaction.cancel.is_active());
                        }
                        ManualStopPhase::OwnAck => unreachable!(),
                    }
                    let queued = match target {
                        NamedRewindTarget::Stale => None,
                        NamedRewindTarget::Queued => Some(send_named_input(&sender, "named-rewind-target", false)),
                    };
                    let (respond_to, queued_boundary) = oneshot::channel();
                    sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
                    queued_boundary.await.unwrap();
                    if matches!(target, NamedRewindTarget::Queued) {
                        assert!(actor.state.lock().await.pending_inputs.iter()
                            .any(|row| row.prompt_id == "named-rewind-target"));
                    }
                    sender.send(SessionCommand::Cancel(CancelOptions {
                        trigger: Some(CancelTrigger::CtrlC),
                        user_initiated: true,
                        history: CancelHistoryDisposition::RewindIfNoOutput {
                            prompt_id: Some("named-rewind-target".into()),
                        },
                        ..Default::default()
                    })).unwrap();
                    let (respond_to, cancelled_boundary) = oneshot::channel();
                    sender.send(SessionCommand::GetToolOverrides { respond_to }).unwrap();
                    cancelled_boundary.await.unwrap();
                    assert_eq!(actor.compaction.cancel.pending_manual.borrow().front().map(|row| row.id), pending_id);
                    assert!(matches!(&*actor.compaction.cancel.manual_worker.borrow(),
                        crate::session::compaction_config::ManualCompactWorker::Running(_)));
                    assert!(!actor.compaction.cancel.is_cancelled());
                    assert!(!cancellation.is_cancelled());
                    assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                    let state = actor.state.lock().await;
                    assert!(state.finalization_gate.is_active());
                    assert!(!state.notifications_suppressed);
                    assert!(!state.pending_inputs.iter().any(|row| row.prompt_id == "named-rewind-target"));
                    drop(state);
                    if let Some(queued) = queued {
                        assert_eq!(queued.await.unwrap().unwrap().stop_reason, acp::StopReason::Cancelled);
                    }
                    assert_eq!(actor.goal_tracker.lock().status(), goal_status);
                    assert_eq!(actor.persisted_completion_state().await.unwrap().wake_policy,
                        crate::session::completion_delivery::CompletionWakePolicy::Automatic);
                    if let Some(release) = reservation_release { release.send(()).unwrap(); }
                    server.release_parked_replies();
                    let own_ack = acks.recv().await.unwrap();
                    assert!(actor.compaction.cancel.is_active());
                    assert!(actor.compaction.cancel.is_committing());
                    assert!(!actor.compaction.cancel.is_cancelled());
                    assert!(actor.state.lock().await.finalization_gate.is_active());
                    assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
                    persistence.send(own_ack).unwrap();
                    compact.await.unwrap().expect("named rewind must preserve the original compact's durable success");
                    assert_eq!(actor.goal_tracker.lock().status(), goal_status);
                    assert!(!actor.state.lock().await.notifications_suppressed);
                    assert_eq!(actor.persisted_completion_state().await.unwrap().wake_policy,
                        crate::session::completion_delivery::CompletionWakePolicy::Automatic);
                    assert_eq!(server.requests().iter().filter(|request| request.conversation.is_some()).count(), 0);
                    relay_task.abort();
                }
            }
        }).await.expect("named rewinds must not cancel unrelated reserved or generating maintenance");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn compact_preserves_an_earlier_human_edit_hold_until_explicit_release() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut fixture = fixture().await;
            let server = route_to_mock(&fixture.actor).await;
            server.set_foreground_hold();
            let mut foreground = submit_named_input(&fixture, "held-foreground", false);
            foreground_parked(&mut fixture, &server).await;
            let earlier = submit_named_input(&fixture, "held-human", false);
            fixture.sender.send(SessionCommand::HoldEdit { id: "held-human".into() }).unwrap();
            let mut compact = submit_compact(&fixture);
            command_boundary(&fixture).await;
            server.release_parked_replies();
            loop {
                tokio::select! {
                    result = &mut foreground => { result.unwrap().unwrap(); break; }
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
            command_boundary(&fixture).await;
            assert!(fixture.actor.has_pending_compact());
            assert!(matches!(compact.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
            assert!(fixture.actor.state.lock().await.edit_holds.contains_key("held-human"));
            fixture.sender.send(SessionCommand::ReleaseEdit { id: "held-human".into() }).unwrap();
            let persisted = queued_compact_barrier(&mut fixture).await;
            earlier.await.unwrap().expect("the earlier human input must run after its edit release");
            persisted.send(Ok(())).unwrap();
            compact.await.unwrap().expect("the same compact must run after the held human input");
        }).await.expect("a human edit hold must remain an explicit maintenance ordering boundary");
    }).await;
}
