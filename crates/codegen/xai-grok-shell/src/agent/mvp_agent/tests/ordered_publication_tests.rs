use super::*;
use crate::agent::mvp_agent::test_hooks::{AttachPause, with_pause_at};
use crate::extensions::current_actions::{
    CutUnavailable, DeliveryRole, GENERATION_END_METHOD, GenerationEnd, LoadCut,
};
use crate::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
enum Applied {
    Acp(acp::SessionNotification),
    Ext(acp::ExtNotification),
}

enum HistoryDecision {
    Apply,
    Reject,
}

struct HistoryGate {
    reached: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    release: std::sync::Arc<tokio::sync::Semaphore>,
    rejected: std::sync::Arc<tokio::sync::Semaphore>,
    decision: HistoryDecision,
}

struct LocalReceiver {
    applied: Rc<RefCell<Vec<Applied>>>,
    background_reached: RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
    background_release: std::sync::Arc<tokio::sync::Semaphore>,
    history: Option<HistoryGate>,
}

#[async_trait::async_trait(?Send)]
impl acp::Client for LocalReceiver {
    async fn request_permission(
        &self,
        _: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        Err(acp::Error::internal_error())
    }

    async fn session_notification(
        &self,
        notification: acp::SessionNotification,
    ) -> acp::Result<()> {
        self.applied.borrow_mut().push(Applied::Acp(notification));
        Ok(())
    }

    async fn ext_notification(&self, notification: acp::ExtNotification) -> acp::Result<()> {
        if notification.method.as_ref() == "x.ai/test/failed_delivery" {
            return Err(acp::Error::internal_error());
        }
        if notification.method.as_ref() == "x.ai/session/update" {
            let value: serde_json::Value = serde_json::from_str(notification.params.get()).unwrap();
            if value["update"]["task_id"] == "hold-history"
                && let Some(gate) = &self.history
            {
                let reached = gate.reached.borrow_mut().take();
                if let Some(reached) = reached {
                    reached.send(()).unwrap();
                }
                gate.release.acquire().await.unwrap().forget();
                match gate.decision {
                    HistoryDecision::Apply => {}
                    HistoryDecision::Reject => return Err(acp::Error::internal_error()),
                }
            }
            if value["update"]["task_id"] == "reject-history" {
                if let Some(gate) = &self.history {
                    gate.rejected.add_permits(1);
                }
                return Err(acp::Error::internal_error());
            }
        }
        if notification.method.as_ref() == "x.ai/task_backgrounded" {
            let reached = self.background_reached.borrow_mut().take();
            if let Some(reached) = reached {
                reached.send(()).unwrap();
                self.background_release.acquire().await.unwrap().forget();
            }
        }
        self.applied.borrow_mut().push(Applied::Ext(notification));
        Ok(())
    }
}

struct LocalFixture {
    agent: MvpAgent,
    applied: Rc<RefCell<Vec<Applied>>>,
    reached: tokio::sync::oneshot::Receiver<()>,
    release: std::sync::Arc<tokio::sync::Semaphore>,
}

fn local_agent(local: xai_acp_lib::ReceiverSettlement) -> LocalFixture {
    local_agent_with_history(local, None)
}

fn local_agent_with_history(
    local: xai_acp_lib::ReceiverSettlement,
    history: Option<HistoryGate>,
) -> LocalFixture {
    let applied = Rc::new(RefCell::new(Vec::new()));
    let (reached, reached_rx) = tokio::sync::oneshot::channel();
    let release = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let (sender, receiver) = xai_acp_lib::acp_gateway::<acp::AgentSide, _>(LocalReceiver {
        applied: applied.clone(),
        background_reached: RefCell::new(Some(reached)),
        background_release: release.clone(),
        history,
    });
    let sender = match local {
        xai_acp_lib::ReceiverSettlement::TransportOnly => sender,
        xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded => {
            sender.with_local_receiver_settlement().unwrap()
        }
    };
    tokio::task::spawn_local(receiver.run());
    let mut agent = build_minimal_agent_for_tests();
    agent.gateway = sender;
    LocalFixture {
        agent,
        applied,
        reached: reached_rx,
        release,
    }
}

fn send_backgrounded(handle: &crate::session::SessionHandle, task_id: &str, cwd: &std::path::Path) {
    handle
        .tools_notification_handle
        .as_ref()
        .unwrap()
        .send_backgrounded(xai_grok_tools::notification::BashExecutionBackgrounded {
            base: xai_grok_tools::notification::BashNotificationBase {
                start_time: None,
                tool_call_id: "actual-background-provider".into(),
                command: "fixture".into(),
                output: Vec::new(),
                total_bytes: 0,
                truncated: false,
                cwd: cwd.to_path_buf(),
            },
            task_id: task_id.into(),
            output_file: format!("/tmp/{task_id}.log").into(),
            monitor_description: None,
            description: None,
        });
}

fn request(
    sid: &acp::SessionId,
    cwd: &std::path::Path,
    binding: &SnapshotBinding,
    replay: bool,
) -> acp::LoadSessionRequest {
    let mut meta = acp::Meta::new();
    meta.insert("noReplay".into(), (!replay).into());
    crate::extensions::current_actions::stamp_load_binding(&mut meta, binding).unwrap();
    acp::LoadSessionRequest::new(sid.clone(), cwd.to_path_buf()).meta(Some(meta))
}

fn ends(applied: &Rc<RefCell<Vec<Applied>>>) -> Vec<GenerationEnd> {
    applied
        .borrow()
        .iter()
        .filter_map(|record| match record {
            Applied::Ext(notification) if notification.method.as_ref() == GENERATION_END_METHOD => {
                Some(serde_json::from_str(notification.params.get()).unwrap())
            }
            Applied::Acp(_) | Applied::Ext(_) => None,
        })
        .collect()
}

#[test]
fn actual_local_cut_waits_for_background_application_despite_overtaking_transport() {
    run_local_for_bridge_test(|| async {
        let LocalFixture {
            agent,
            applied,
            reached,
            release: release_background,
        } = local_agent(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let handle = agent.resident_handle(&sid).unwrap();
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 3,
            load_generation: 7,
        };
        let reader = handle.current_actions.as_ref().unwrap();
        reader.bind_publication(binding.clone()).unwrap();
        handle
            .cmd_tx
            .send(SessionCommand::AdvertiseCommands {
                trigger: crate::session::AdvertiseTrigger::SessionLoad,
            })
            .unwrap();
        let (respond_to, advertised) = tokio::sync::oneshot::channel();
        handle
            .cmd_tx
            .send(SessionCommand::GetToolOverrides { respond_to })
            .unwrap();
        let _ = advertised.await.unwrap();
        let work = handle
            .active_work
            .load(std::sync::atomic::Ordering::Acquire);
        send_backgrounded(&handle, "actual-background-task", cwd.path());
        reached.await.unwrap();
        let (load, reached_cut, release_cut) = with_pause_at(
            AttachPause::PublicationDrain,
            acp::Agent::load_session(&agent, request(&sid, cwd.path(), &binding, false)),
        );
        tokio::pin!(load);
        tokio::select! {
            result = &mut load => panic!("load passed the held receiver: {result:?}"),
            reached = reached_cut => reached.unwrap(),
        }
        assert!(ends(&applied).is_empty());
        let overtake = agent
            .gateway
            .forward_with_completion(acp::ExtNotification::new(
                "x.ai/test/overtake",
                serde_json::value::to_raw_value(&serde_json::json!({}))
                    .unwrap()
                    .into(),
            ));
        overtake.await.unwrap().unwrap();
        assert!(applied.borrow().iter().any(|record| matches!(record,
            Applied::Ext(notification) if notification.method.as_ref() == "x.ai/test/overtake"
        )));
        release_cut.send(()).unwrap();
        assert!(
            futures::poll!(&mut load).is_pending(),
            "a later handler completion cannot settle the older record"
        );
        assert!(ends(&applied).is_empty());
        release_background.add_permits(1);
        let loaded = load.await.unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
            LoadCut::Settled
        ));
        let end = ends(&applied)
            .into_iter()
            .next()
            .expect("real local recipient and producer prefix must settle");
        assert_eq!(end.snapshot.binding, binding);
        assert_eq!(
            crate::extensions::current_actions::load_snapshot(loaded.meta.as_ref()).unwrap(),
            CurrentActionsSnapshot::Current(end.snapshot.clone())
        );
        assert_eq!(ends(&applied).len(), 1);
        let records = applied.borrow();
        for record in records.iter() {
            match record {
                Applied::Acp(notification) => assert_eq!(notification.session_id, sid),
                Applied::Ext(_) => {}
            }
        }
        let background = records.iter().position(|record| matches!(record,
            Applied::Ext(notification) if notification.method.as_ref() == "x.ai/task_backgrounded"
        )).unwrap();
        let terminal = records.iter().position(|record| matches!(record,
            Applied::Ext(notification) if notification.method.as_ref() == GENERATION_END_METHOD
        )).unwrap();
        assert!(background < terminal);
        let native_prefix = records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| match record {
                Applied::Acp(notification) => notification
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("eventId"))
                    .and_then(|id| id.as_str())
                    .and_then(|id| id.rsplit('-').next())
                    .and_then(|sequence| sequence.parse::<u64>().ok())
                    .map(|sequence| (index, sequence)),
                Applied::Ext(_) => None,
            })
            .filter(|(_, sequence)| *sequence < end.snapshot.event_frontier)
            .collect::<Vec<_>>();
        assert!(
            !native_prefix.is_empty(),
            "actual actor command must enter the native SessionEvent rail"
        );
        assert!(native_prefix.iter().all(|(index, _)| *index < terminal));
        let Applied::Ext(background) = &records[background] else {
            panic!("background publication");
        };
        let background: serde_json::Value = serde_json::from_str(background.params.get()).unwrap();
        let sequence: u64 = background["_meta"]["eventId"]
            .as_str()
            .unwrap()
            .rsplit('-')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(sequence < end.snapshot.event_frontier);
        drop(records);
        assert_eq!(
            handle
                .active_work
                .load(std::sync::atomic::Ordering::Acquire),
            work
        );
        assert!(handle.current_prompt_id.lock().unwrap().is_none());
        let mut replay_binding = binding;
        replay_binding.load_generation += 1;
        let replayed =
            acp::Agent::load_session(&agent, request(&sid, cwd.path(), &replay_binding, true))
                .await
                .unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_cut(replayed.meta.as_ref()).unwrap(),
            LoadCut::Settled
        ));
        let replayed_background = applied
            .borrow()
            .iter()
            .find_map(|record| match record {
                Applied::Ext(notification)
                    if notification.method.as_ref() == "x.ai/session/update" =>
                {
                    let value: serde_json::Value =
                        serde_json::from_str(notification.params.get()).unwrap();
                    (value["update"]["sessionUpdate"] == "task_backgrounded").then_some(value)
                }
                Applied::Acp(_) | Applied::Ext(_) => None,
            })
            .unwrap();
        let meta = replayed_background["_meta"].as_object().unwrap();
        assert_eq!(
            crate::extensions::current_actions::delivery_role(Some(meta)).unwrap(),
            DeliveryRole::Historical {
                binding: replay_binding
            }
        );
        assert_eq!(
            replayed_background["_meta"]["eventId"],
            background["_meta"]["eventId"]
        );
        assert_eq!(replayed_background["update"], background["update"]);
        agent.remove_session(&sid);
    });
}

#[test]
fn actual_working_resident_cut_keeps_original_task_and_prompt_rpc() {
    // LocalRef's pointee must outlive the helper's LocalSet, including unwind.
    let mut coordinator_owner = None;
    run_local_for_bridge_test(|| async {
        use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, ScriptedResponse};
        let LocalFixture {
            agent,
            applied,
            reached: _reached,
            release: _release,
        } = local_agent(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
        let agent = Rc::new(agent);
        coordinator_owner = Some(agent.clone());
        let server = xai_grok_test_support::MockInferenceServer::start()
            .await
            .unwrap();
        server.set_response("finished");
        server.set_keep_requests(true);
        let mut entry =
            crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
        entry.info.base_url = server.url();
        entry.info.api_backend = crate::sampling::ApiBackend::Responses;
        entry.api_key = Some("test".into());
        agent.models_manager.insert_test_entry("test-model", entry);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        agent.start_subagent_coordinator();
        let handle = agent.resident_handle(&sid).unwrap();
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 4,
            load_generation: 10,
        };
        let mut blocked = server.expect_response_blocked(
            "ordered cut during original model dispatch",
            InferenceRequestMatcher::foreground_containing(
                InferenceEndpoint::Responses,
                "actual-ordered-cut-input",
            ),
            ScriptedResponse::sse(xai_grok_test_support::sse::responses_api_script_exact(
                "finished",
                "test-model",
            )),
        );
        let prompt = acp::Agent::prompt(
            agent.as_ref(),
            acp::PromptRequest::new(
                sid.clone(),
                vec![acp::ContentBlock::from("actual-ordered-cut-input")],
            ),
        );
        tokio::pin!(prompt);
        tokio::select! {
            result = &mut prompt => panic!("original inference must remain held: {result:?}"),
            reached = tokio::time::timeout(std::time::Duration::from_secs(30), blocked.wait_blocked()) => {
                reached.unwrap_or_else(|error| panic!(
                    "original dispatch did not reach its barrier: {error}; {}; routes={:?}; active_work={}; snapshot={:?}",
                    blocked.diagnostic(),
                    server.requests().into_iter()
                        .map(|entry| (entry.method, entry.path, entry.endpoint, entry.conversation))
                        .collect::<Vec<_>>(),
                    handle.active_work.load(std::sync::atomic::Ordering::Acquire),
                    agent.current_actions_snapshot(binding.clone()),
                ));
            }
        }
        let before = agent.current_actions_snapshot(binding.clone());
        let CurrentActionsSnapshot::Current(before) = before else {
            panic!("actual working reader");
        };
        let work = handle
            .active_work
            .load(std::sync::atomic::Ordering::Acquire);
        let prompt_id = handle.current_prompt_id.lock().unwrap().clone();
        assert!(work > 0);
        assert!(before.turn.is_some());
        assert!(!before.sampling.is_empty());
        let loaded = acp::Agent::load_session(agent.as_ref(), request(&sid, cwd.path(), &binding, false))
            .await
            .unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
            LoadCut::Settled
        ));
        let snapshot =
            crate::extensions::current_actions::load_snapshot(loaded.meta.as_ref()).unwrap();
        let end = ends(&applied)
            .into_iter()
            .next()
            .expect("real receiver consumed the cut marker");
        assert_eq!(
            snapshot,
            CurrentActionsSnapshot::Current(end.snapshot.clone())
        );
        assert_eq!(end.snapshot.version, before.version);
        assert_eq!(end.snapshot.turn, before.turn);
        assert_eq!(end.snapshot.sampling, before.sampling);
        assert_eq!(end.snapshot.tools, before.tools);
        assert_eq!(
            handle
                .active_work
                .load(std::sync::atomic::Ordering::Acquire),
            work
        );
        assert_eq!(handle.current_prompt_id.lock().unwrap().clone(), prompt_id);
        assert!(
            futures::poll!(&mut prompt).is_pending(),
            "cut cannot acknowledge original work"
        );
        blocked.release();
        tokio::time::timeout(std::time::Duration::from_secs(30), &mut prompt)
            .await
            .unwrap()
            .unwrap();
        blocked.assert_satisfied();
        agent.remove_session(&sid);
    });
    drop(coordinator_owner);
}

enum Replacement {
    Generation,
    Actor,
}

async fn refuses_superseded_end(replacement: Replacement) {
    let LocalFixture {
        agent,
        applied,
        reached: _reached,
        release: _release,
    } = local_agent(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
    let cwd = tempfile::tempdir().unwrap();
    let sid = new_root_session(&agent, cwd.path()).await;
    let binding = SnapshotBinding {
        session_id: sid.0.to_string(),
        binding_epoch: 9,
        load_generation: 2,
    };
    let CurrentActionsSnapshot::Current(original) = agent.current_actions_snapshot(binding.clone())
    else {
        panic!("actual resident actor");
    };
    let (old, reached, release) = with_pause_at(
        AttachPause::PublicationEnd,
        acp::Agent::load_session(&agent, request(&sid, cwd.path(), &binding, false)),
    );
    tokio::pin!(old);
    tokio::select! {
        result = &mut old => panic!("cut did not pause: {result:?}"),
        reached = reached => reached.unwrap(),
    }
    let mut replacement_binding = binding.clone();
    match replacement {
        Replacement::Generation => replacement_binding.load_generation += 1,
        Replacement::Actor => agent.remove_session(&sid),
    }
    let newer = acp::Agent::load_session(
        &agent,
        request(&sid, cwd.path(), &replacement_binding, false),
    )
    .await
    .unwrap();
    assert!(matches!(
        crate::extensions::current_actions::load_cut(newer.meta.as_ref()).unwrap(),
        LoadCut::Settled
    ));
    let CurrentActionsSnapshot::Current(newer_snapshot) =
        crate::extensions::current_actions::load_snapshot(newer.meta.as_ref()).unwrap()
    else {
        panic!("actual replacement attach settles its own cut");
    };
    match replacement {
        Replacement::Generation => assert_eq!(
            newer_snapshot.version.incarnation,
            original.version.incarnation
        ),
        Replacement::Actor => assert_ne!(
            newer_snapshot.version.incarnation,
            original.version.incarnation
        ),
    }
    let count = ends(&applied).len();
    release.send(()).unwrap();
    let old = old.await.unwrap();
    assert!(matches!(
        crate::extensions::current_actions::load_cut(old.meta.as_ref()).unwrap(),
        LoadCut::Unavailable {
            reason: CutUnavailable::Superseded
        }
    ));
    assert_eq!(
        ends(&applied).len(),
        count,
        "old prepared cut cannot publish an end into a replacement attach"
    );
    agent.remove_session(&sid);
}

#[test]
fn old_generation_cannot_finish_a_newer_load() {
    run_local_for_bridge_test(|| async { refuses_superseded_end(Replacement::Generation).await });
}

#[test]
fn replaced_resident_actor_cannot_publish_its_old_prepared_end() {
    run_local_for_bridge_test(|| async { refuses_superseded_end(Replacement::Actor).await });
}

#[test]
fn rejected_actual_history_cannot_be_hidden_by_a_successful_live_cut() {
    run_local_for_bridge_test(|| async {
        let LocalFixture {
            agent,
            applied,
            reached,
            release,
        } = local_agent(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let handle = agent.resident_handle(&sid).unwrap();
        release.add_permits(1);
        send_backgrounded(&handle, "reject-history", cwd.path());
        reached.await.unwrap();
        agent.gateway.producer_prefix().settle().await.unwrap();
        agent.gateway.delivery_prefix().settle().await.unwrap();
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 5,
            load_generation: 1,
        };
        let loaded = acp::Agent::load_session(&agent, request(&sid, cwd.path(), &binding, true))
            .await
            .unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
            LoadCut::Unavailable {
                reason: CutUnavailable::PublicationIncomplete
            }
        ));
        assert!(ends(&applied).is_empty());
        agent.remove_session(&sid);
    });
}

#[test]
fn failed_receiver_completion_is_not_a_successful_generation_cut() {
    run_local_for_bridge_test(|| async {
        let LocalFixture {
            agent,
            applied,
            reached: _reached,
            release: _release,
        } = local_agent(xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        assert!(
            agent
                .gateway
                .forward_fire_and_forget(acp::ExtNotification::new(
                    "x.ai/test/failed_delivery",
                    serde_json::value::to_raw_value(&serde_json::json!({}))
                        .unwrap()
                        .into(),
                ))
        );
        assert_eq!(
            agent.gateway.delivery_prefix().settle().await,
            Err(xai_acp_lib::PublicationFailure::ReceiverFailed)
        );
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 5,
            load_generation: 1,
        };
        let loaded = acp::Agent::load_session(&agent, request(&sid, cwd.path(), &binding, false))
            .await
            .unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
            LoadCut::Unavailable {
                reason: CutUnavailable::ReceiverFailed
            }
        ));
        assert!(ends(&applied).is_empty());
        agent.remove_session(&sid);
    });
}

enum PriorHistory {
    HeldOnly,
    Rejected,
}

async fn cancelled_history_keeps_custody(prior: PriorHistory) {
    let (history_reached, reached_history) = tokio::sync::oneshot::channel();
    let history_release = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let rejected = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
    let LocalFixture {
        agent,
        applied,
        reached,
        release,
    } = local_agent_with_history(
        xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded,
        Some(HistoryGate {
            reached: RefCell::new(Some(history_reached)),
            release: history_release.clone(),
            rejected: rejected.clone(),
            decision: match prior {
                PriorHistory::HeldOnly => HistoryDecision::Reject,
                PriorHistory::Rejected => HistoryDecision::Apply,
            },
        }),
    );
    let cwd = tempfile::tempdir().unwrap();
    let sid = new_root_session(&agent, cwd.path()).await;
    let handle = agent.resident_handle(&sid).unwrap();
    release.add_permits(1);
    match prior {
        PriorHistory::HeldOnly => {}
        PriorHistory::Rejected => send_backgrounded(&handle, "reject-history", cwd.path()),
    }
    send_backgrounded(&handle, "hold-history", cwd.path());
    reached.await.unwrap();
    agent.gateway.producer_prefix().settle().await.unwrap();
    agent.gateway.delivery_prefix().settle().await.unwrap();
    let binding = SnapshotBinding {
        session_id: sid.0.to_string(),
        binding_epoch: 12,
        load_generation: 1,
    };
    let mut load = Box::pin(acp::Agent::load_session(
        &agent,
        request(&sid, cwd.path(), &binding, true),
    ));
    tokio::select! {
        result = &mut load => panic!("historical handler must hold load: {result:?}"),
        reached = reached_history => reached.unwrap(),
    }
    match prior {
        PriorHistory::HeldOnly => {}
        PriorHistory::Rejected => {
            tokio::select! {
                result = &mut load => panic!("held later receipt must keep load open: {result:?}"),
                permit = rejected.acquire() => permit.unwrap().forget(),
            }
            assert!(futures::poll!(&mut load).is_pending());
        }
    }
    drop(load);
    let mut next_binding = binding;
    next_binding.load_generation += 1;
    let mut next = Box::pin(acp::Agent::load_session(
        &agent,
        request(&sid, cwd.path(), &next_binding, false),
    ));
    assert!(
        futures::poll!(&mut next).is_pending(),
        "NoReplay must retain the historical handler debt"
    );
    assert!(ends(&applied).is_empty());
    history_release.add_permits(1);
    let loaded = next.await.unwrap();
    assert!(matches!(
        crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
        LoadCut::Unavailable {
            reason: CutUnavailable::ReceiverFailed
        }
    ));
    assert_eq!(
        agent.gateway.delivery_prefix().settle().await,
        Err(xai_acp_lib::PublicationFailure::ReceiverFailed)
    );
    assert!(ends(&applied).is_empty());
    agent.remove_session(&sid);
}

#[test]
fn cancelled_load_retains_held_historical_receipt_for_actual_no_replay_load() {
    run_local_for_bridge_test(|| async {
        cancelled_history_keeps_custody(PriorHistory::HeldOnly).await
    });
}

#[test]
fn cancelled_load_retains_already_failed_historical_receipt() {
    run_local_for_bridge_test(|| async {
        cancelled_history_keeps_custody(PriorHistory::Rejected).await
    });
}

#[test]
fn transport_only_load_keeps_observation_but_never_claims_receiver_settlement() {
    run_local_for_bridge_test(|| async {
        let LocalFixture {
            agent,
            applied,
            reached: _reached,
            release: _release,
        } = local_agent(xai_acp_lib::ReceiverSettlement::TransportOnly);
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let binding = SnapshotBinding {
            session_id: sid.0.to_string(),
            binding_epoch: 5,
            load_generation: 1,
        };
        let loaded = acp::Agent::load_session(&agent, request(&sid, cwd.path(), &binding, false))
            .await
            .unwrap();
        assert!(matches!(
            crate::extensions::current_actions::load_snapshot(loaded.meta.as_ref()).unwrap(),
            CurrentActionsSnapshot::Current(_)
        ));
        assert!(matches!(
            crate::extensions::current_actions::load_cut(loaded.meta.as_ref()).unwrap(),
            LoadCut::Unavailable {
                reason: CutUnavailable::UnsupportedTransport
            }
        ));
        assert!(ends(&applied).is_empty());
        agent.remove_session(&sid);
    });
}
