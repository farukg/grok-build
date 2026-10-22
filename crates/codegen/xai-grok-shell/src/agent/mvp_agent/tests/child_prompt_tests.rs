//! `session/prompt` addressed to a child session id: a running child runs the turn and settles the
//! parent's receipt, a finished child is woken through the coordinator, a workflow child refuses.

use acp::Agent as _;
use agent_client_protocol as acp;
use serde_json::json;
use xai_grok_tools::implementations::grok_build::task::types::AgentAddress;

use super::{build_minimal_agent_for_tests, make_live_session_handle, run_local_for_bridge_test};
use crate::agent::mvp_agent::{ChildHost, ChildReach, ChildResidence, RunningChild};
use crate::extensions::notification::SubagentDelivery;
use crate::session::SessionCommand;
use crate::session::commands::{PromptCompletionKind, PromptTurnOk};

const PARENT: &str = "child-prompt-parent";
const CHILD: &str = "child-prompt-child";

fn child_host(reach: ChildReach) -> ChildHost {
    ChildHost {
        parent_session_id: acp::SessionId::new(PARENT),
        reach,
    }
}

fn prompt(text: &str) -> acp::PromptRequest {
    acp::PromptRequest::new(
        acp::SessionId::new(CHILD),
        vec![acp::ContentBlock::from(text)],
    )
}

/// Answers the child actor's prompt intake and ends every turn with `total_tokens`.
fn spawn_child_actor(
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    total_tokens: u64,
) {
    tokio::task::spawn_local(async move {
        while let Some(command) = cmd_rx.recv().await {
            match command {
                SessionCommand::GetCurrentPromptMode { responds_to } => {
                    let _ = responds_to.send(Default::default());
                }
                SessionCommand::GetCurrentModel { responds_to } => {
                    let _ = responds_to.send(crate::session::CurrentModel {
                        id: "test-model".to_owned(),
                        reasoning_effort: None,
                    });
                }
                SessionCommand::Prompt { respond_to, .. } => {
                    let _ = respond_to.send(Ok(PromptTurnOk {
                        stop_reason: acp::StopReason::EndTurn,
                        total_tokens,
                        turn_snapshot: None,
                        completion_kind: PromptCompletionKind::Completed,
                        structured_output: None,
                        usage: None,
                        tool_overrides: None,
                    }));
                }
                _ => {}
            }
        }
    });
}

#[test]
fn running_child_prompt_runs_on_child_and_settles_the_parent_receipt() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let child = acp::SessionId::new(CHILD);
        let (handle, _cmd_tx, cmd_rx) = make_live_session_handle(&child, None);
        spawn_child_actor(cmd_rx, 42);
        let (turns, mut receipts) = tokio::sync::mpsc::channel(4);
        let (delivery, delivery_rx) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
        agent.register_child_session(
            &child,
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(1),
                residence: ChildResidence::Running(Box::new(RunningChild {
                    handle,
                    turns,
                    parent_prompt_index: Default::default(),
                    delivery,
                })),
            }),
        );

        let prompting = agent.prompt(
            prompt("look again").meta(
                serde_json::json!({ "promptId": "human-1" })
                    .as_object()
                    .cloned(),
            ),
        );
        tokio::pin!(prompting);
        let receipt = tokio::select! {
            receipt = receipts.recv() => receipt.expect("the prompt registers a turn receipt for the parent"),
            response = &mut prompting => panic!("the prompt ended without a receipt: {response:?}"),
        };
        assert_eq!(receipt.prompt_id, "human-1");
        let settled = tokio::select! {
            biased;
            settled = receipt.result => settled,
            response = &mut prompting => panic!("the prompt ended without settling its receipt: {response:?}"),
        };
        let settled = settled
            .expect("the receipt settles with the child's turn")
            .expect("the child's turn succeeded");
        assert_eq!(settled.total_tokens, 42);
        assert_eq!(
            *delivery_rx.borrow(),
            SubagentDelivery::Held,
            "a human prompt holds the child's answer back from its caller"
        );

        assert_eq!(deliver(&agent).await, json!({ "kind": "delivered" }));
        assert_eq!(*delivery_rx.borrow(), SubagentDelivery::OnTurnEnd);
        assert_eq!(deliver(&agent).await, json!({ "kind": "not_held" }));
    });
}

async fn deliver(agent: &crate::agent::mvp_agent::MvpAgent) -> serde_json::Value {
    let params = json!({ "sessionId": CHILD });
    let response = agent
        .ext_method(acp::ExtRequest::new(
            "x.ai/subagent/deliver",
            std::sync::Arc::from(
                serde_json::value::to_raw_value(&params).expect("deliver params serialize"),
            ),
        ))
        .await
        .expect("deliver answers");
    let body: serde_json::Value =
        serde_json::from_str(response.0.get()).expect("deliver response is JSON");
    body["result"].clone()
}

#[test]
fn deliver_on_a_finished_child_reports_not_running() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        agent.register_child_session(
            &acp::SessionId::new(CHILD),
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(3),
                residence: ChildResidence::Finished,
            }),
        );

        assert_eq!(deliver(&agent).await, json!({ "kind": "not_running" }));
    });
}

#[test]
fn by_id_handlers_reach_a_running_child_but_root_enumerations_do_not() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let child = acp::SessionId::new(CHILD);
        let (handle, _cmd_tx, _cmd_rx) = make_live_session_handle(&child, None);
        let (turns, _receipts) = tokio::sync::mpsc::channel(4);
        let (delivery, _delivery_rx) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
        agent.register_child_session(
            &child,
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(2),
                residence: ChildResidence::Running(Box::new(RunningChild {
                    handle,
                    turns,
                    parent_prompt_index: Default::default(),
                    delivery,
                })),
            }),
        );

        assert!(
            agent.resident_handle(&child).is_some(),
            "set_model, compact, mode … address a running child by its id"
        );
        assert!(!agent.is_resident(&child), "a child is not a root");
        assert!(agent.session_registry.resident_ids().is_empty());

        agent.finish_child_session(&child);
        assert!(agent.resident_handle(&child).is_none());
    });
}

#[test]
fn compact_unknown_or_finished_without_saved_state_answers_instead_of_hanging() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        for finished in [false, true] {
            if finished {
                agent.register_child_session(
                    &acp::SessionId::new(CHILD),
                    child_host(ChildReach::Addressed {
                        address: AgentAddress::mint(9),
                        residence: ChildResidence::Finished,
                    }),
                );
            }
            let request = acp::ExtRequest::new(
                "x.ai/compact_conversation",
                std::sync::Arc::from(
                    serde_json::value::to_raw_value(&json!({ "sessionId": CHILD })).unwrap(),
                ),
            );
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(1), agent.ext_method(request))
                    .await
                    .expect("compact must resolve before allocating a reply channel");
            assert!(
                result.is_err(),
                "an unknown or unsaved child cannot compact"
            );
        }
    });
}

#[test]
fn compact_finished_saved_child_reloads_same_identity_and_is_cancellable_during_prepare() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let child = super::new_root_session(&agent, cwd.path()).await;
        let handle = agent.resident_handle(&child).unwrap();
        let conversation = vec![crate::sampling::ConversationItem::user(
            "saved child context",
        )];
        handle
            .chat_state_handle
            .replace_conversation(conversation.clone());
        assert_eq!(
            serde_json::to_value(handle.chat_state_handle.get_conversation().await).unwrap(),
            serde_json::to_value(&conversation).unwrap()
        );
        super::drain_persistence(&agent, &child).await;
        let info = handle.info.clone();
        let mut policy = xai_grok_sampling_types::ContextPolicy::default();
        policy.set(
            xai_grok_sampling_types::ContextCategory::Reasoning,
            xai_grok_sampling_types::ContextSwitch::Excluded,
        );
        crate::session::helpers::context_policy_store::save(
            &crate::session::persistence::session_dir(&info),
            &policy,
        );
        use crate::session::storage::StorageAdapter as _;
        let storage = crate::session::storage::JsonlStorageAdapter::with_root(
            crate::util::grok_home::grok_home(),
        );
        let saved_identity = storage.load_summary(&info).await.unwrap();
        let saved_model = handle.model_id.clone();
        handle
            .cmd_tx
            .send(SessionCommand::Shutdown(
                crate::session::ShutdownKind::Graceful,
            ))
            .unwrap();
        agent.remove_session(&child);
        drop(handle);
        agent.drain_old_session_thread(&child).await;
        let parent = acp::SessionId::new(PARENT);
        let (mut parent_handle, _tx, _rx) = make_live_session_handle(&parent, None);
        parent_handle.info.cwd = cwd.path().to_string_lossy().into_owned();
        agent.insert_resident(&parent, parent_handle);
        agent.set_session_host(&parent, crate::agent::mvp_agent::SessionHost::Root);
        agent.register_child_session(
            &child,
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(11),
                residence: ChildResidence::Finished,
            }),
        );
        let meta_dir = crate::session::persistence::session_dir(&crate::session::info::Info {
            id: parent.clone(),
            cwd: info.cwd.clone(),
        })
        .join("subagents")
        .join(child.0.as_ref());
        std::fs::create_dir_all(&meta_dir).unwrap();
        std::fs::write(
            meta_dir.join("meta.json"),
            serde_json::to_vec(&json!({
                "subagent_id": child.0, "child_session_id": child.0,
                "parent_session_id": PARENT, "subagent_type": "grok-build",
                "description": "saved child", "prompt": "original task", "status": "completed",
                "started_at": chrono::Utc::now(), "effective_context_source": "new",
                "child_cwd": info.cwd,
            }))
            .unwrap(),
        )
        .unwrap();
        let saved_info = agent
            .ext_method(acp::ExtRequest::new(
                "x.ai/session/info",
                std::sync::Arc::from(
                    serde_json::value::to_raw_value(&json!({ "sessionId": child.0 })).unwrap(),
                ),
            ))
            .await
            .unwrap();
        let saved_info: serde_json::Value = serde_json::from_str(saved_info.0.get()).unwrap();
        assert_eq!(saved_info["result"]["source"], "restored");
        assert_eq!(saved_info["result"]["runtime"], "management");
        let request = acp::ExtRequest::new(
            "x.ai/compact_conversation",
            std::sync::Arc::from(
                serde_json::value::to_raw_value(&json!({ "sessionId": child.0 })).unwrap(),
            ),
        );
        let (compacting, reached, _release) =
            super::with_pause_at(super::AttachPause::AfterInstall, agent.ext_method(request));
        let cancellation = async {
            reached
                .await
                .expect("compact reloads the actual saved actor");
            let restored = agent.resident_handle(&child).unwrap();
            assert_eq!(restored.info.id, child);
            assert_eq!(
                serde_json::to_value(restored.chat_state_handle.get_conversation().await).unwrap(),
                serde_json::to_value(&conversation).unwrap()
            );
            assert_eq!(
                restored.chat_state_handle.get_context_policy().await,
                policy
            );
            assert_eq!(restored.model_id, saved_model);
            assert!(agent.session_registry.root_for_session(&child).is_none());
            agent
                .cancel(acp::CancelNotification::new(child.clone()))
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(compacting, cancellation);
        assert_eq!(
            crate::session::helpers::session_compact::compact_error_kind(&result.unwrap_err()),
            Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled),
        );
        assert!(agent.resident_handle(&child).is_none());
        assert!(agent.resident_handle(&parent).is_some());
        assert!(matches!(
            agent.session_host(&child),
            Some(crate::agent::mvp_agent::SessionHost::Child(_))
        ));
        let reloaded = agent
            .management_session(&child)
            .await
            .expect("a cancelled preparation remains restorable");
        assert_eq!(reloaded.handle.info.id, child);
        assert_eq!(
            reloaded.handle.chat_state_handle.get_context_policy().await,
            policy
        );
        assert_eq!(
            serde_json::to_value(reloaded.handle.chat_state_handle.get_conversation().await)
                .unwrap(),
            serde_json::to_value(&conversation).unwrap()
        );
        assert!(!agent.resident_ids().contains(&child));
        drop(reloaded);
        let after = storage.load_summary(&info).await.unwrap();
        assert_eq!(after.agent_id, saved_identity.agent_id);
        assert_eq!(after.attempt_id, saved_identity.attempt_id);
        assert_eq!(after.next_trace_turn, saved_identity.next_trace_turn);
        assert!(agent.resident_handle(&child).is_none());
    });
}

#[test]
fn finished_coordinator_child_compacts_persisted_checkpoint_without_parent_delivery() {
    run_local_for_bridge_test(|| async {
        use crate::session::storage::StorageAdapter as _;
        use xai_grok_tools::implementations::grok_build::task::backend::{
            ChannelBackend, SubagentBackend,
        };
        use xai_grok_tools::implementations::grok_build::task::types::{
            SubagentOwner, SubagentRequest,
        };
        let server = xai_grok_test_support::MockInferenceServer::start()
            .await
            .unwrap();
        server.set_response("Completed original child work. ".repeat(40));
        let agent = build_minimal_agent_for_tests();
        let mut entry =
            crate::agent::config::ModelEntry::fallback("test-model", &Default::default());
        entry.info.base_url = server.url();
        entry.info.api_backend = crate::sampling::ApiBackend::Responses;
        entry.api_key = Some("mock-key".into());
        agent.models_manager.insert_test_entry("test-model", entry);
        let cwd = tempfile::tempdir().unwrap();
        let parent = super::new_root_session(&agent, cwd.path()).await;
        agent.start_subagent_coordinator();
        let id = uuid::Uuid::now_v7().to_string();
        let backend = ChannelBackend::for_coordinator_session(
            agent.subagent_event_tx.clone(),
            parent.0.to_string(),
        );
        let result = backend
            .spawn(
                SubagentRequest {
                    id: id.clone(),
                    prompt: "Perform the original task and report the result.".into(),
                    description: "administrative compact fixture".into(),
                    subagent_type: "general-purpose".into(),
                    parent_session_id: parent.0.to_string(),
                    parent_prompt_id: None,
                    resume_from: None,
                    cwd: None,
                    runtime_overrides: Default::default(),
                    run_in_background: false,
                    surface_completion: true,
                    await_to_completion: true,
                    fork_context: false,
                    owner: SubagentOwner::Task,
                    cancel_token: Default::default(),
                    spawn_root: Default::default(),
                    tool_call_id: None,
                },
                None,
            )
            .await
            .unwrap();
        assert!(
            result.success(),
            "coordinator child must finish: {result:?}"
        );
        assert!(result.output.contains("Completed original child work"));
        let child = acp::SessionId::new(id);
        assert!(agent.resident_handle(&child).is_none());
        let source = crate::agent::subagent::durable_resume_source(
            child.0.as_ref(),
            parent.0.as_ref(),
            cwd.path(),
        )
        .unwrap();
        let info = crate::session::info::Info {
            id: child.clone(),
            cwd: source.child_cwd,
        };
        let storage = crate::session::storage::JsonlStorageAdapter::with_root(
            crate::util::grok_home::grok_home(),
        );
        let prior = storage.load_summary(&info).await.unwrap();
        let parent_handle = agent.resident_handle(&parent).unwrap();
        let parent_before = parent_handle.chat_state_handle.get_conversation().await;
        server.set_response("Summary of completed original work; preserve the original task and verified findings. ".repeat(30));
        let response = agent
            .ext_method(acp::ExtRequest::new(
                "x.ai/compact_conversation",
                std::sync::Arc::from(
                    serde_json::value::to_raw_value(&json!({ "sessionId": child.0 })).unwrap(),
                ),
            ))
            .await;
        response.expect("finished child compacts through the real handler");
        let updates = crate::session::persistence::session_dir(&info).join("updates.jsonl");
        assert!(
            crate::session::helpers::replay::find_latest_compaction_checkpoint(&updates)
                .unwrap()
                .is_some(),
            "successful handler durably flushes its checkpoint"
        );
        let after = storage.load_summary(&info).await.unwrap();
        assert_eq!(after.agent_id, prior.agent_id);
        assert_eq!(after.attempt_id, prior.attempt_id);
        assert_eq!(after.next_trace_turn, prior.next_trace_turn);
        assert_eq!(
            serde_json::to_value(parent_handle.chat_state_handle.get_conversation().await).unwrap(),
            serde_json::to_value(&parent_before).unwrap(),
            "administrative compact must not deliver a second child answer to the parent"
        );
        assert!(agent.resident_handle(&child).is_none());
        agent
            .cfg
            .borrow_mut()
            .feature_values
            .insert(crate::agent::config::Feature::ActiveAgentMessages, true);
        let compact_request = acp::ExtRequest::new(
            "x.ai/compact_conversation",
            std::sync::Arc::from(
                serde_json::value::to_raw_value(&json!({ "sessionId": child.0 })).unwrap(),
            ),
        );
        let (compacting, reached, release) = super::with_pause_at(
            super::AttachPause::AfterInstall,
            agent.ext_method(compact_request),
        );
        let waking = async {
            reached.await.unwrap();
            let prompt = agent.prompt(acp::PromptRequest::new(
                child.clone(),
                vec![acp::ContentBlock::from(
                    "Continue after the administrative operation.",
                )],
            ));
            tokio::pin!(prompt);
            assert!(
                futures_util::poll!(&mut prompt).is_pending(),
                "concurrent wake waits for compact ownership"
            );
            release.send(()).unwrap();
            prompt.await
        };
        let (compacted, woken) = tokio::join!(compacting, waking);
        compacted.expect("compact wins its lease and commits before wake");
        let woken =
            woken.expect("concurrent human wake is accepted after compact releases persistence");
        assert!(
            woken
                .meta
                .as_ref()
                .is_some_and(|meta| meta.contains_key("childWake"))
        );
        parent_handle
            .cmd_tx
            .send(SessionCommand::Shutdown(
                crate::session::ShutdownKind::Graceful,
            ))
            .unwrap();
    });
}

#[test]
fn compact_running_child_cancel_finishes_while_actor_reply_is_pending() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let child = acp::SessionId::new(CHILD);
        let (handle, _cmd_tx, mut commands) = make_live_session_handle(&child, None);
        let (turns, _receipts) = tokio::sync::mpsc::channel(4);
        let (delivery, _) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
        agent.register_child_session(
            &child,
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(12),
                residence: ChildResidence::Running(Box::new(RunningChild {
                    handle,
                    turns,
                    parent_prompt_index: Default::default(),
                    delivery,
                })),
            }),
        );
        let request = acp::ExtRequest::new(
            "x.ai/compact_conversation",
            std::sync::Arc::from(
                serde_json::value::to_raw_value(&json!({ "sessionId": CHILD })).unwrap(),
            ),
        );
        let actor = async {
            let SessionCommand::CompactSession {
                cancellation,
                mut respond_to,
            } = commands.recv().await.unwrap()
            else {
                panic!("compact must reach the child's actor");
            };
            let overlap = agent
                .ext_method(acp::ExtRequest::new(
                    "x.ai/compact_conversation",
                    std::sync::Arc::from(
                        serde_json::value::to_raw_value(&json!({ "sessionId": CHILD })).unwrap(),
                    ),
                ))
                .await
                .unwrap_err();
            assert_eq!(
                overlap
                    .data
                    .as_ref()
                    .and_then(|data| data.get("kind"))
                    .and_then(|kind| kind.as_str()),
                Some("compact_busy")
            );
            assert!(!cancellation.is_cancelled());
            agent
                .cancel(acp::CancelNotification::new(child.clone()))
                .await
                .unwrap();
            cancellation.cancelled().await;
            assert!(cancellation.is_cancelled());
            respond_to
                .send(Err(
                    crate::session::helpers::session_compact::CompactFailure::cancelled_error(),
                ))
                .unwrap();
        };
        let (result, ()) = tokio::join!(agent.ext_method(request), actor);
        assert_eq!(
            crate::session::helpers::session_compact::compact_error_kind(&result.unwrap_err()),
            Some(crate::session::helpers::session_compact::CompactErrorKind::Cancelled)
        );
        assert!(agent.resident_handle(&child).is_some());
    });
}

#[test]
fn compact_closed_child_channel_is_terminal() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let child = acp::SessionId::new(CHILD);
        let (handle, _cmd_tx, cmd_rx) = make_live_session_handle(&child, None);
        drop(cmd_rx);
        let (turns, _receipts) = tokio::sync::mpsc::channel(4);
        let (delivery, _delivery_rx) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
        agent.register_child_session(
            &child,
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(10),
                residence: ChildResidence::Running(Box::new(RunningChild {
                    handle,
                    turns,
                    parent_prompt_index: Default::default(),
                    delivery,
                })),
            }),
        );
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            agent.ext_method(acp::ExtRequest::new(
                "x.ai/compact_conversation",
                std::sync::Arc::from(
                    serde_json::value::to_raw_value(&json!({ "sessionId": CHILD })).unwrap(),
                ),
            )),
        )
        .await
        .expect("a closed command channel is terminal");
        assert!(result.is_err());
    });
}

#[test]
fn workflow_child_refuses_prompts() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        agent.register_child_session(
            &acp::SessionId::new(CHILD),
            child_host(ChildReach::Unaddressed),
        );

        let error = agent
            .prompt(prompt("hello"))
            .await
            .expect_err("a workflow child is not promptable");

        assert_eq!(error.code, acp::ErrorCode::InvalidRequest);
    });
}

#[test]
fn finished_child_prompt_is_routed_to_the_coordinator_wake() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        agent
            .cfg
            .borrow_mut()
            .feature_values
            .insert(crate::agent::config::Feature::ActiveAgentMessages, true);
        agent.start_subagent_coordinator();
        let parent = acp::SessionId::new(PARENT);
        let (parent_handle, _parent_tx, _parent_rx) = make_live_session_handle(&parent, None);
        agent.insert_resident(&parent, parent_handle);
        agent.register_child_session(
            &acp::SessionId::new(CHILD),
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(7),
                residence: ChildResidence::Finished,
            }),
        );

        let error = agent
            .prompt(prompt("continue"))
            .await
            .expect_err("the coordinator owns no child under this address");

        assert_eq!(
            error.data,
            Some(serde_json::json!({ "childWake": { "kind": "rejected" } }))
        );
    });
}

#[test]
fn finished_child_without_active_agent_messages_continues_like_a_resume() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        agent
            .cfg
            .borrow_mut()
            .feature_values
            .insert(crate::agent::config::Feature::ActiveAgentMessages, false);
        agent.register_child_session(
            &acp::SessionId::new(CHILD),
            child_host(ChildReach::Addressed {
                address: AgentAddress::mint(7),
                residence: ChildResidence::Finished,
            }),
        );

        let error = agent
            .prompt(prompt("continue"))
            .await
            .expect_err("the parent session of this child is not resident");

        assert_eq!(error.code, acp::ErrorCode::InvalidRequest);
        assert!(
            error
                .data
                .is_some_and(|data| data.get("childResume").is_some()),
            "the refusal comes from the resume route, not from a missing feature"
        );
    });
}

#[test]
fn unknown_child_id_names_its_parent_to_continue_like_a_resume() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let unnamed = agent
            .prompt(prompt("continue"))
            .await
            .expect_err("an unknown id without a parent stays unknown");
        assert_eq!(unnamed.code, acp::ErrorCode::InvalidParams);

        let named = agent
            .prompt(prompt("continue").meta(json!({ "childOf": PARENT }).as_object().cloned()))
            .await
            .expect_err("the named parent is not resident here");
        assert_eq!(named.code, acp::ErrorCode::InvalidRequest);
        assert!(
            named
                .data
                .is_some_and(|data| data.get("childResume").is_some())
        );
    });
}

#[test]
fn finished_child_is_released_with_its_parent() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let parent = acp::SessionId::new(PARENT);
        let (parent_handle, _parent_tx, _parent_rx) = make_live_session_handle(&parent, None);
        agent.insert_resident(&parent, parent_handle);
        agent.set_session_host(&parent, crate::agent::mvp_agent::SessionHost::Root);
        agent.register_child_session(
            &acp::SessionId::new(CHILD),
            child_host(ChildReach::Unaddressed),
        );
        agent.finish_child_session(&acp::SessionId::new(CHILD));
        assert!(
            agent
                .prompt(prompt("hello"))
                .await
                .is_err_and(|error| { error.code == acp::ErrorCode::InvalidRequest })
        );

        agent.session_registry.release(&parent);

        let error = agent
            .prompt(prompt("hello"))
            .await
            .expect_err("the child left with its parent");
        assert_eq!(error.code, acp::ErrorCode::InvalidParams);
    });
}
