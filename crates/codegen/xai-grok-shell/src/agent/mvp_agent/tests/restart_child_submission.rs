use super::*;
use acp::Agent as _;
use crate::session::completion_delivery::CompletionDeliveryState;
use crate::session::persistence::PersistenceMsg;
use xai_grok_tools::types::restart_recovery::{LeaderRestartRecovery, RestartAdmission};

struct Fixture {
    agent: crate::agent::mvp_agent::MvpAgent,
    persistence: tokio::sync::mpsc::UnboundedReceiver<PersistenceMsg>,
    child: xai_grok_tools::types::restart_recovery::SessionRestartRecovery,
    parent: xai_grok_tools::types::restart_recovery::SessionRestartRecovery,
}

fn fixture() -> Fixture {
    let agent = build_minimal_agent_for_tests();
    let startup = LeaderRestartRecovery::stale(chrono::Utc::now());
    let created = chrono::Utc::now() - chrono::Duration::hours(2);
    let child_recovery = startup.register(CHILD, created, RestartAdmission::Unaffected);
    let parent_recovery = startup.register(PARENT, created, RestartAdmission::Unaffected);
    agent.cfg.borrow_mut().leader_restart_recovery = startup;
    agent.cfg.borrow_mut().feature_values.insert(
        crate::agent::config::Feature::ActiveAgentMessages, true,
    );
    let (mut parent, _, _parent_commands) = make_live_session_handle(&acp::SessionId::new(PARENT), None);
    parent.tool_context.restart_recovery = parent_recovery.clone();
    agent.insert_resident(&acp::SessionId::new(PARENT), parent);
    let (mut handle, _, _commands) = make_live_session_handle(&acp::SessionId::new(CHILD), None);
    let (persistence, receiver) = tokio::sync::mpsc::unbounded_channel();
    handle.persistence_tx = persistence;
    handle.tool_context.restart_recovery = child_recovery.clone();
    let (turns, _receipts) = tokio::sync::mpsc::channel(4);
    let (delivery, _) = tokio::sync::watch::channel(SubagentDelivery::OnTurnEnd);
    agent.register_child_session(&acp::SessionId::new(CHILD), child_host(ChildReach::Addressed {
        address: AgentAddress::mint(7),
        residence: ChildResidence::Running(Box::new(RunningChild {
            handle, turns, parent_prompt_index: Default::default(), delivery,
        })),
    }));
    agent.start_subagent_coordinator();
    Fixture { agent, persistence: receiver, child: child_recovery, parent: parent_recovery }
}

async fn activation_reply(
    persistence: &mut tokio::sync::mpsc::UnboundedReceiver<PersistenceMsg>,
    admission: RestartAdmission,
) -> tokio::sync::oneshot::Sender<std::io::Result<()>> {
    for _ in 0..2 {
        let Some(PersistenceMsg::GetCompletionState { respond_to }) = persistence.recv().await else {
            panic!("fresh child submission must read settlement before activation");
        };
        respond_to.send(Ok(CompletionDeliveryState {
            restart_admission: admission, ..Default::default()
        })).unwrap();
    }
    let Some(PersistenceMsg::SetRestartAdmission { admission: next, respond_to }) = persistence.recv().await else {
        panic!("fresh child submission must await durable activation");
    };
    assert_eq!(next, admission.after_user_message());
    respond_to
}

#[test]
fn acp_native_child_submission_waits_for_activation_ack_without_opening_parent() {
    run_local_for_bridge_test(|| async {
        let mut fixture = fixture();
        let prompting = fixture.agent.prompt(prompt("new child work"));
        tokio::pin!(prompting);
        let response = tokio::select! {
            response = activation_reply(&mut fixture.persistence, fixture.child.admission()) => response,
            result = &mut prompting => panic!("activation was bypassed: {result:?}"),
        };
        assert!(futures::poll!(&mut prompting).is_pending());
        assert!(!fixture.child.admission().permits_recovery());
        assert!(!fixture.parent.admission().permits_recovery());
        response.send(Ok(())).unwrap();
        let error = prompting.await.unwrap_err();
        assert_eq!(error.data, Some(json!({ "childWake": { "kind": "rejected" } })));
        assert!(fixture.child.admission().permits_recovery());
        assert!(!fixture.parent.admission().permits_recovery());
    });
}

#[test]
fn acp_child_activation_failure_propagates_without_native_wake() {
    run_local_for_bridge_test(|| async {
        let mut fixture = fixture();
        let prompting = fixture.agent.prompt(prompt("new child work"));
        tokio::pin!(prompting);
        let response = tokio::select! {
            response = activation_reply(&mut fixture.persistence, fixture.child.admission()) => response,
            result = &mut prompting => panic!("activation was bypassed: {result:?}"),
        };
        response.send(Err(std::io::Error::other("injected durable admission failure"))).unwrap();
        assert_eq!(prompting.await.unwrap_err().code, acp::ErrorCode::InternalError);
        assert!(!fixture.child.admission().permits_recovery());
        assert!(!fixture.parent.admission().permits_recovery());
    });
}

#[test]
fn lost_child_activation_ack_keeps_submission_closed() {
    run_local_for_bridge_test(|| async {
        let mut fixture = fixture();
        let prompting = fixture.agent.prompt(prompt("new child work"));
        tokio::pin!(prompting);
        let response = tokio::select! {
            response = activation_reply(&mut fixture.persistence, fixture.child.admission()) => response,
            result = &mut prompting => panic!("activation was bypassed: {result:?}"),
        };
        drop(response);
        assert_eq!(prompting.await.unwrap_err().code, acp::ErrorCode::InternalError);
        assert!(!fixture.child.admission().permits_recovery());
        assert!(!fixture.parent.admission().permits_recovery());
    });
}

#[test]
fn child_route_replacement_during_ack_does_not_publish_to_successor() {
    run_local_for_bridge_test(|| async {
        let mut fixture = fixture();
        let prompting = fixture.agent.prompt(prompt("new child work"));
        tokio::pin!(prompting);
        let response = tokio::select! {
            response = activation_reply(&mut fixture.persistence, fixture.child.admission()) => response,
            result = &mut prompting => panic!("activation was bypassed: {result:?}"),
        };
        fixture.agent.register_child_session(&acp::SessionId::new(CHILD), child_host(ChildReach::Addressed {
            address: AgentAddress::mint(8), residence: ChildResidence::Finished,
        }));
        response.send(Ok(())).unwrap();
        let error = prompting.await.unwrap_err();
        assert_eq!(error.code, acp::ErrorCode::InvalidRequest);
        assert_eq!(error.data, Some(json!("child submission target changed during admission")));
        assert!(!fixture.parent.admission().permits_recovery());
    });
}

#[test]
fn nonmessage_and_generic_resume_do_not_request_child_activation() {
    run_local_for_bridge_test(|| async {
        let mut fixture = fixture();
        let image: Vec<acp::ContentBlock> = serde_json::from_value(json!([
            { "type": "image", "data": "abc", "mimeType": "image/png" }
        ])).unwrap();
        let error = fixture.agent.wake_child_with_prompt(
            &acp::SessionId::new(PARENT), &acp::SessionId::new(CHILD),
            AgentAddress::mint(7), image,
        ).await.unwrap_err();
        assert_eq!(error.code, acp::ErrorCode::InvalidRequest);
        assert!(fixture.agent.resume_subagent(PARENT, CHILD, "resume old work".into()).await.is_err());
        assert!(fixture.persistence.try_recv().is_err());
        assert!(!fixture.child.admission().permits_recovery());
        assert!(!fixture.parent.admission().permits_recovery());
    });
}
