use super::*;
use crate::extensions::interject::{
    QUEUE_INTERJECT_METHOD, QueueAction, QueueCaller, QueueOperation, QueueRejection, QueueRequest,
    QueueResponse, QueueResult,
};
use crate::session::current_actions::{
    CurrentActionsSnapshot, CurrentSamplingPhase, CurrentTurnPhase, FrozenOwner, SnapshotBinding,
};
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};

#[test]
fn resident_handle_reads_same_source_during_real_work_without_ack_or_activity() {
    run_local_for_bridge_test(|| async {
        let server = xai_grok_test_support::MockInferenceServer::start()
            .await
            .unwrap();
        server.set_response("finished");
        let agent = build_minimal_agent_for_tests();
        let mut entry =
            crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
        entry.info.base_url = server.url();
        entry.info.api_backend = crate::sampling::ApiBackend::Responses;
        entry.api_key = Some("test".into());
        agent.models_manager.insert_test_entry("test-model", entry);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let handle = agent.resident_handle(&sid).unwrap();
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 7,
            load_generation: 11,
        };
        let CurrentActionsSnapshot::Current(idle) = handle.current_actions(binding.clone()) else {
            panic!("spawn exposes actual source before its command loop");
        };
        assert!(idle.turn.is_none());
        let mut blocked = server.expect_response_blocked(
            "read-only query while working",
            InferenceRequestMatcher::foreground_containing(
                InferenceEndpoint::Responses,
                "actual-boundary-input",
            ),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(
                "finished",
                "test-model",
            )),
        );
        let prompt = acp::Agent::prompt(
            &agent,
            acp::PromptRequest::new(
                sid.clone(),
                vec![acp::ContentBlock::from("actual-boundary-input")],
            ),
        );
        tokio::pin!(prompt);
        tokio::select! {
            result = &mut prompt => panic!("real inference must still be working: {result:?}"),
            reached = tokio::time::timeout(std::time::Duration::from_secs(30), blocked.wait_blocked()) => {
                reached.unwrap();
            }
        }
        drain_persistence(&agent, &sid).await;
        let summary_path =
            crate::session::persistence::session_dir(&handle.info).join("summary.json");
        let persisted_before = std::fs::read(&summary_path).unwrap();
        let prompt_before = handle.current_prompt_id.lock().unwrap().clone();
        let work_before = handle
            .active_work
            .load(std::sync::atomic::Ordering::Acquire);
        assert!(work_before > 0);
        assert_eq!(
            agent.session_live_state_for(&sid),
            Some(SessionLiveState::Working)
        );
        let reply = acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new(
                crate::session::current_actions::CAPABILITY,
                serde_json::value::to_raw_value(&binding).unwrap().into(),
            ),
        )
        .await
        .unwrap();
        let snapshot: CurrentActionsSnapshot = serde_json::from_str(reply.0.get()).unwrap();
        let CurrentActionsSnapshot::Current(current) = &snapshot else {
            panic!("working actor source is readable synchronously");
        };
        assert_eq!(current.binding, binding);
        assert_eq!(current.version.incarnation, idle.version.incarnation);
        let turn = current.turn.as_ref().unwrap();
        assert!(
            matches!(&turn.owner, FrozenOwner::Root { prompt_id, .. } if Some(prompt_id) == prompt_before.as_ref())
        );
        assert!(matches!(turn.phase, CurrentTurnPhase::Running { .. }));
        assert!(current.sampling.iter().any(|request| matches!(
            request.phase,
            CurrentSamplingPhase::Reserved | CurrentSamplingPhase::Submitted { .. }
        )));
        let CurrentActionsSnapshot::Current(again) = handle.current_actions(binding.clone()) else {
            panic!("same reader");
        };
        assert_eq!(again.version, current.version);
        assert_eq!(again.turn, current.turn);
        for request in &current.sampling {
            assert!(
                again
                    .sampling
                    .iter()
                    .any(|later| later.owner == request.owner)
            );
        }
        assert_eq!(
            handle.current_prompt_id.lock().unwrap().clone(),
            prompt_before
        );
        assert_eq!(
            handle
                .active_work
                .load(std::sync::atomic::Ordering::Acquire),
            work_before
        );
        assert_eq!(
            agent.session_live_state_for(&sid),
            Some(SessionLiveState::Working)
        );
        assert_eq!(std::fs::read(&summary_path).unwrap(), persisted_before);
        assert!(
            matches!(futures::poll!(&mut prompt), std::task::Poll::Pending),
            "observation cannot acknowledge the original RPC"
        );
        let mut foreign = binding.clone();
        foreign.session_id.push_str("-other");
        assert_eq!(
            handle.current_actions(foreign.clone()),
            CurrentActionsSnapshot::Unavailable
        );
        assert_eq!(
            agent.current_actions_snapshot(foreign),
            CurrentActionsSnapshot::Unavailable
        );
        let mut rebound = binding.clone();
        rebound.binding_epoch += 1;
        rebound.load_generation += 1;
        let CurrentActionsSnapshot::Current(retagged) =
            agent.current_actions_snapshot(rebound.clone())
        else {
            panic!("binding tags are not a new actor");
        };
        assert_eq!(retagged.binding, rebound);
        assert_eq!(retagged.version.incarnation, current.version.incarnation);
        assert_eq!(retagged.turn, current.turn);
        assert!(!snapshot.is_current_for(&rebound, Some(current.version), None));
        let mut unavailable = handle.clone();
        unavailable.current_actions = None;
        assert_eq!(
            unavailable.current_actions(binding.clone()),
            CurrentActionsSnapshot::Unavailable
        );
        let (closed, receiver) = tokio::sync::mpsc::unbounded_channel();
        drop(receiver);
        let mut dead = handle.clone();
        dead.cmd_tx = closed;
        assert_eq!(
            dead.current_actions(binding.clone()),
            CurrentActionsSnapshot::Unavailable
        );
        let mut load_meta = acp::Meta::new();
        crate::extensions::current_actions::stamp_load_binding(&mut load_meta, &binding).unwrap();
        let loaded = acp::Agent::load_session(
            &agent,
            acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf())
                .meta(Some(load_meta)),
        )
        .await
        .unwrap();
        let loaded_snapshot =
            crate::extensions::current_actions::load_snapshot(loaded.meta.as_ref()).unwrap();
        let CurrentActionsSnapshot::Current(loaded_current) = &loaded_snapshot else {
            panic!("load observes the resident actor");
        };
        assert_eq!(loaded_current.binding, binding);
        assert_eq!(loaded_current.version, current.version);
        assert_eq!(loaded_current.turn, current.turn);
        assert_eq!(
            handle.current_prompt_id.lock().unwrap().clone(),
            prompt_before
        );
        assert_eq!(
            handle
                .active_work
                .load(std::sync::atomic::Ordering::Acquire),
            work_before
        );
        assert!(matches!(
            futures::poll!(&mut prompt),
            std::task::Poll::Pending
        ));
        blocked.release();
        tokio::time::timeout(std::time::Duration::from_secs(30), &mut prompt)
            .await
            .unwrap()
            .unwrap();
        blocked.assert_satisfied();
        agent.remove_session(&sid);
        assert_eq!(
            agent.current_actions_snapshot(binding),
            CurrentActionsSnapshot::Unavailable
        );
    });
}

#[test]
fn current_actions_wire_rejects_bad_load_tags_without_attaching_another_source() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 4,
            load_generation: 9,
        };
        let before = agent.current_actions_snapshot(binding.clone());
        let bad = acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new(
                crate::session::current_actions::CAPABILITY,
                serde_json::value::to_raw_value(&serde_json::json!({ "sessionId": sid.0 }))
                    .unwrap()
                    .into(),
            ),
        )
        .await;
        assert!(bad.is_err());
        let mut foreign = binding.clone();
        foreign.session_id.push_str("-unknown");
        let reply = acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new(
                crate::session::current_actions::CAPABILITY,
                serde_json::value::to_raw_value(&foreign).unwrap().into(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<CurrentActionsSnapshot>(reply.0.get()).unwrap(),
            CurrentActionsSnapshot::Unavailable
        );
        let mut meta = acp::Meta::new();
        crate::extensions::current_actions::stamp_load_binding(&mut meta, &foreign).unwrap();
        assert!(
            acp::Agent::load_session(
                &agent,
                acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf(),)
                    .meta(Some(meta))
            )
            .await
            .is_err()
        );
        let after = agent.current_actions_snapshot(binding.clone());
        let (CurrentActionsSnapshot::Current(before), CurrentActionsSnapshot::Current(after)) =
            (before, after)
        else {
            panic!("actual resident source");
        };
        assert_eq!(before.version, after.version);
        assert_eq!(before.turn, after.turn);
        assert_eq!(before.binding, after.binding);
        let legacy = acp::Agent::load_session(
            &agent,
            acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf()),
        )
        .await
        .unwrap();
        assert_eq!(
            crate::extensions::current_actions::load_snapshot(legacy.meta.as_ref()).unwrap(),
            CurrentActionsSnapshot::Unavailable
        );
        agent.remove_session(&sid);
    });
}

#[test]
fn actual_session_control_caller_crossing_another_actor_is_refused_for_send_and_observe() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let first = new_root_session(&agent, cwd.path()).await;
        let second = agent
            .new_session_inner(
                acp::NewSessionRequest::new(cwd.path().to_path_buf()).meta(
                    serde_json::json!({ "modelId": "test-model" })
                        .as_object()
                        .cloned(),
                ),
            )
            .await
            .unwrap()
            .session_id;
        let first_handle = agent.resident_handle(&first).unwrap();
        let second_handle = agent.resident_handle(&second).unwrap();
        let request = QueueRequest {
            operation: QueueOperation {
                session_id: second.0.to_string(),
                id: "missing-row".into(),
                expected_version: 0,
                operation_id: uuid::Uuid::new_v4(),
            },
            action: QueueAction::Prioritize { new_text: None },
            owner: Some("copied-owner".into()),
        };
        let response = acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new(
                QUEUE_INTERJECT_METHOD,
                serde_json::value::to_raw_value(&request).unwrap().into(),
            ),
        )
        .await
        .unwrap();
        let accepted: QueueResponse = serde_json::from_str(response.0.get()).unwrap();
        assert!(matches!(
            accepted.result,
            QueueResult::Rejected {
                reason: QueueRejection::Missing,
                ..
            }
        ));
        for action in [
            QueueAction::Prioritize {
                new_text: Some("must not be admitted".into()),
            },
            QueueAction::Observe,
        ] {
            let mut copied = request.clone();
            copied.action = action;
            let (respond_to, received) = tokio::sync::oneshot::channel();
            second_handle
                .cmd_tx
                .send(SessionCommand::QueueSendNow {
                    caller: QueueCaller::from_session_control(&first_handle),
                    request: copied,
                    respond_to,
                })
                .unwrap();
            let refused = tokio::time::timeout(std::time::Duration::from_secs(5), received)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(refused.operation, request.operation);
            assert_eq!(
                refused.result,
                QueueResult::Rejected {
                    reason: QueueRejection::Unauthorized,
                    row: None
                }
            );
        }
        let mut observe = request;
        observe.action = QueueAction::Observe;
        let response = acp::Agent::ext_method(
            &agent,
            acp::ExtRequest::new(
                QUEUE_INTERJECT_METHOD,
                serde_json::value::to_raw_value(&observe).unwrap().into(),
            ),
        )
        .await
        .unwrap();
        let observed: QueueResponse = serde_json::from_str(response.0.get()).unwrap();
        assert_eq!(
            observed, accepted,
            "foreign caller cannot read or replace the genuine receipt"
        );
    });
}
