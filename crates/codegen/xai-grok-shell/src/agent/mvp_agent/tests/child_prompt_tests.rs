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
