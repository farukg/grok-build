use super::*;
use crate::session::acp_session::support::{create_test_actor, input_with_origin_rx, running_task_stub};
use crate::session::current_actions::SnapshotBinding;
use crate::session::storage::{SessionUpdate, StorageAdapter};
use crate::session::{CancelOptions, CancelTarget, WakeCancelRequest};

fn bind(actor: &SessionActor) -> PublicationContext {
    actor.current_actions_reader().bind_publication(SnapshotBinding {
        session_id: actor.session_info.id.0.to_string(),
        binding_epoch: 9,
        load_generation: 2,
    }).unwrap()
}

fn options(context: PublicationContext, prompt_id: &str) -> (
    CancelOptions,
    tokio::sync::mpsc::UnboundedReceiver<WakeCancelResponse>,
) {
    let (respond_to, response) = tokio::sync::mpsc::unbounded_channel();
    let request = WakeCancelRequest {
        context,
        prompt_id: prompt_id.into(),
        cancel_subagents: false,
        trigger: Some("ctrl_c".into()),
    };
    (CancelOptions {
        trigger: Some(crate::session::CancelTrigger::CtrlC),
        user_initiated: true,
        target_prompt_id: Some(CancelTarget::Wake { request, respond_to }),
        ..Default::default()
    }, response)
}

#[tokio::test(flavor = "current_thread")]
async fn queued_receipt_replays_without_touching_running_or_next_user() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, _received) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, _persisted) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
        let context = bind(&actor);
        let (user, mut user_response) = input_with_origin_rx("user", crate::session::PromptOrigin::User);
        let (wake, mut wake_response) = input_with_origin_rx(
            "task-completed-queued",
            crate::session::PromptOrigin::TaskCompleted { task_id: "queued".into() },
        );
        let (next, mut next_response) = input_with_origin_rx("next", crate::session::PromptOrigin::User);
        {
            let mut state = actor.state.lock().await;
            state.running_task = Some(running_task_stub("user"));
            state.pending_inputs.extend([user, wake, next]);
        }
        for _ in 0..2 {
            let (options, mut response) = options(context.clone(), "task-completed-queued");
            let outcome = actor.cancel_running_task(options).await;
            assert!(!outcome.turn_stopped);
            assert_eq!(outcome.barrier, WakeBarrier::Clear);
            assert!(matches!(response.recv().await.unwrap().settlement,
                WakeCancelSettlement::RemovedFromQueue));
            assert_eq!(actor.state.lock().await.running_task.as_ref().unwrap().prompt_id, "user");
            assert!(matches!(user_response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
            assert!(matches!(next_response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
        }
        assert!(matches!(wake_response.try_recv().unwrap().unwrap().completion_kind,
            crate::session::PromptCompletionKind::RemovedFromQueue));
        assert_eq!(actor.state.lock().await.pending_inputs.len(), 2);
        assert!(!actor.state.lock().await.notifications_suppressed);
        assert_eq!(actor.current_actions.version().unwrap().incarnation, context.incarnation);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn terminal_read_ignores_unrelated_history_but_preserves_rewinds_and_decode_errors() {
    tokio::task::LocalSet::new().run_until(async {
        let root = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::session::storage::jsonl::JsonlStorageAdapter::with_root(
            root.path().to_path_buf(),
        ));
        let (gateway, _received) = tokio::sync::mpsc::unbounded_channel();
        let (unused, _messages) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, unused).await;
        storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
        let updates_path = storage.updates_file_path(&actor.session_info).unwrap();
        let rewind_path = storage.rewind_points_file_path(&actor.session_info).unwrap();
        std::fs::write(rewind_path, "not valid rewind snapshots\n").unwrap();
        let notification = crate::extensions::notification::SessionNotification {
            session_id: actor.session_info.id.clone(),
            update: XaiSessionUpdate::TurnCompleted {
                prompt_id: "task-completed-original".into(),
                stop_reason: "error".into(),
                agent_result: Some("original failure".into()),
                error_kind: None,
                usage: None,
                elapsed_ms: Some(17),
            },
            meta: None,
        };
        let encode = |notification| serde_json::to_string(
            &crate::session::storage::SessionUpdateEnvelope::from_update(
                &SessionUpdate::Xai(Box::new(notification)),
            ).unwrap(),
        ).unwrap();
        let user_line = serde_json::to_string(
            &crate::session::storage::SessionUpdateEnvelope::from_update(
                &SessionUpdate::Acp(Box::new(acp::SessionNotification::new(
                    actor.session_info.id.clone(),
                    acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new("wake input")),
                    )),
                ))),
            ).unwrap(),
        ).unwrap();
        let terminal_line = encode(notification.clone());
        let rewind_line = encode(crate::extensions::notification::SessionNotification {
            session_id: actor.session_info.id.clone(),
            update: XaiSessionUpdate::RewindMarker {
                target_prompt_index: 0,
                created_at: "2026-10-30".into(),
            },
            meta: None,
        });
        let persistence = crate::session::persistence::durable_update_tests::test_actor(
            actor.session_info.clone(), storage,
        );
        let _persistent = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        enum Expected { Original, Rewound, Malformed }
        for (contents, expected) in [
            (format!("{terminal_line}\n"), Expected::Original),
            (format!("{user_line}\n{terminal_line}\n{rewind_line}\n"), Expected::Rewound),
            (format!("{terminal_line}\nmalformed update\n"), Expected::Malformed),
        ] {
            std::fs::write(&updates_path, contents).unwrap();
            let (respond_to, response) = tokio::sync::oneshot::channel();
            persistence.handle.tx.send(PersistenceMsg::ReadTurnTerminal {
                prompt_id: "task-completed-original".into(),
                respond_to,
            }).unwrap();
            let result = response.await.unwrap();
            match expected {
                Expected::Original => assert_eq!(serde_json::to_value(result.unwrap().unwrap()).unwrap(),
                    serde_json::to_value(&notification).unwrap()),
                Expected::Rewound => assert!(result.unwrap().is_none(), "rewound terminal is not settlement"),
                Expected::Malformed => assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidData),
            }
        }
    }).await;
}

#[test]
fn wire_request_routes_by_bound_session_and_rejects_conflicting_session() {
    let request = WakeCancelRequest {
        context: PublicationContext {
            binding: SnapshotBinding {
                session_id: "bound-session".into(),
                binding_epoch: 4,
                load_generation: 3,
            },
            incarnation: uuid::Uuid::new_v4(),
        },
        prompt_id: "task-completed-wire".into(),
        cancel_subagents: false,
        trigger: Some("ctrl_c".into()),
    };
    let mut wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["sessionId"], "bound-session");
    let decoded: WakeCancelRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded.context, request.context);
    assert_eq!(decoded.prompt_id, request.prompt_id);
    wire["sessionId"] = serde_json::json!("foreign-session");
    let rpc = acp::ExtRequest::new(crate::session::CANCEL_SETTLEMENT_METHOD,
        serde_json::value::to_raw_value(&wire).unwrap().into());
    let error = crate::extensions::parse_params::<WakeCancelRequest>(&rpc).unwrap_err();
    assert_eq!(error.code, acp::Error::invalid_params().code);
}

#[tokio::test(flavor = "current_thread")]
async fn queue_receipt_retention_is_bounded_and_eviction_is_not_settlement() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, _received) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, mut persisted) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
        let context = bind(&actor);
        for index in 0..=crate::session::current_actions::REMOVED_WAKE_RECEIPT_LIMIT {
            let prompt_id = format!("task-completed-queued-{index}");
            let (wake, mut producer) = input_with_origin_rx(&prompt_id,
                crate::session::PromptOrigin::TaskCompleted { task_id: index.to_string() });
            actor.state.lock().await.pending_inputs.push_back(wake);
            let (cancel, mut receipt) = options(context.clone(), &prompt_id);
            assert!(!actor.cancel_running_task(cancel).await.turn_stopped);
            assert!(matches!(receipt.recv().await.unwrap().settlement,
                WakeCancelSettlement::RemovedFromQueue));
            assert!(matches!(producer.try_recv().unwrap().unwrap().completion_kind,
                crate::session::PromptCompletionKind::RemovedFromQueue));
        }
        let (cancel, mut receipt) = options(context, "task-completed-queued-0");
        let (_, ()) = tokio::join!(actor.cancel_running_task(cancel), async {
            loop {
                if let PersistenceMsg::ReadTurnTerminal { respond_to, .. } = persisted.recv().await.unwrap() {
                    respond_to.send(Ok(None)).unwrap();
                    break;
                }
            }
        });
        assert!(matches!(receipt.recv().await.unwrap().settlement,
            WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::UnknownTarget }));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn running_cancel_waits_for_durable_terminal_then_finished_retry_returns_same_terminal() {
    tokio::task::LocalSet::new().run_until(async {
        let root = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::session::storage::jsonl::JsonlStorageAdapter::with_root(
            root.path().to_path_buf(),
        ));
        let (gateway, _received) = tokio::sync::mpsc::unbounded_channel();
        let (proxy, mut messages) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, proxy).await;
        storage.init_session(&actor.session_info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(
            actor.session_info.clone(), storage.clone(),
        );
        let _persistent = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let real = persistence.handle.tx;
        let (held, pending) = tokio::sync::oneshot::channel();
        let forward = real.clone();
        let _proxy = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
            let mut held = Some(held);
            while let Some(message) = messages.recv().await {
                if matches!(&message, PersistenceMsg::AppendUpdateDurablyAndAck { update, .. }
                    if matches!(update, SessionUpdate::Xai(n)
                        if matches!(&n.update, XaiSessionUpdate::TurnCompleted { .. })))
                    && let Some(held) = held.take()
                {
                    held.send(message).unwrap();
                    continue;
                }
                forward.send(message).unwrap();
            }
        }));
        let context = bind(&actor);
        let prompt_id = "task-completed-running";
        let (wake, mut wake_response) = input_with_origin_rx(prompt_id,
            crate::session::PromptOrigin::TaskCompleted { task_id: "running".into() });
        let (next, mut next_response) = input_with_origin_rx("next-user", crate::session::PromptOrigin::User);
        {
            let mut state = actor.state.lock().await;
            state.running_task = Some(running_task_stub(prompt_id));
            state.pending_inputs.extend([wake, next]);
        }
        *actor.current_prompt_id.lock().unwrap() = Some(prompt_id.into());
        let (cancel, mut receipt) = options(context.clone(), prompt_id);
        let (outcome, ()) = tokio::join!(actor.cancel_running_task(cancel), async {
            let message = pending.await.unwrap();
            assert!(matches!(receipt.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)),
                "sending cancel or aborting the task is not a durable terminal receipt");
            assert!(matches!(wake_response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
            real.send(message).unwrap();
        });
        assert!(outcome.turn_stopped && outcome.settled);
        let first = receipt.recv().await.unwrap();
        let WakeCancelSettlement::Terminal { notification: original } = first.settlement else {
            panic!("real running cancellation must return its committed terminal");
        };
        assert!(matches!(&original.update, XaiSessionUpdate::TurnCompleted { prompt_id: id, stop_reason, .. }
            if id == prompt_id && stop_reason == "cancelled"));
        assert!(wake_response.try_recv().unwrap().is_ok());
        {
            let mut state = actor.state.lock().await;
            state.running_task = Some(running_task_stub("next-user"));
        }
        let (cancel, mut receipt) = options(context.clone(), prompt_id);
        let repeated = actor.cancel_running_task(cancel).await;
        assert!(!repeated.turn_stopped);
        let WakeCancelSettlement::Terminal { notification: replayed } = receipt.recv().await.unwrap().settlement else {
            panic!("finished retry must retrieve the original durable terminal");
        };
        assert_eq!(serde_json::to_value(original).unwrap(), serde_json::to_value(replayed).unwrap());
        assert!(matches!(next_response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
        assert_eq!(actor.state.lock().await.running_task.as_ref().unwrap().prompt_id, "next-user");
        let persisted = storage.load_session(&actor.session_info).await.unwrap();
        assert_eq!(persisted.updates.iter().filter(|update| matches!(update, SessionUpdate::Xai(n)
            if matches!(&n.update, XaiSessionUpdate::TurnCompleted { prompt_id: id, .. } if id == prompt_id))).count(), 1);
        for stale in [
            PublicationContext { incarnation: uuid::Uuid::new_v4(), ..context.clone() },
            PublicationContext { binding: SnapshotBinding { session_id: "foreign".into(), ..context.binding.clone() }, ..context.clone() },
        ] {
            let (cancel, mut receipt) = options(stale, "next-user");
            assert!(!actor.cancel_running_task(cancel).await.turn_stopped);
            assert!(matches!(receipt.recv().await.unwrap().settlement,
                WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::SourceChanged }));
        }
        let (cancel, mut receipt) = options(context, "task-completed-unknown");
        assert!(!actor.cancel_running_task(cancel).await.turn_stopped);
        assert!(matches!(receipt.recv().await.unwrap().settlement,
            WakeCancelSettlement::Unresolved { reason: WakeCancelFailure::UnknownTarget }));
        assert!(matches!(next_response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
    }).await;
}
