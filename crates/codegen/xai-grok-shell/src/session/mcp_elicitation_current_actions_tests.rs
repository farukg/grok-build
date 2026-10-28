use super::*;
use crate::session::current_actions::{
    CurrentActionTransition, CurrentActionsEvent, CurrentActionsSnapshot, FrozenOwner,
    InteractionSource, SnapshotBinding, Source, TRANSITION_METHOD,
};
use xai_acp_lib::AcpClientMessage;
use xai_grok_mcp::elicitation::WireElicitFields;
use xai_grok_tools::mcp_elicitation::McpElicitModeFields;

enum Finish { Answer, ServerCancel, CoordinatorDrop }

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn actual_elicitation_keeps_original_rpc_point_without_inventing_a_turn_and_retires_on_every_exit() {
    tokio::task::LocalSet::new().run_until(async {
        for finish in [Finish::Answer, Finish::ServerCancel, Finish::CoordinatorDrop] {
            let (tx, mut messages) = tokio::sync::mpsc::unbounded_channel();
            let gateway = GatewaySender::new(tx);
            let source = Source::default();
            let session_id = acp::SessionId::new("elicitation-source-session");
            source.attach_emitter(gateway.clone(), Arc::new(std::sync::atomic::AtomicBool::new(true)), session_id.clone());
            let pending = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let reader = source.reader(pending.clone(), session_id.0.to_string());
            let binding = SnapshotBinding { session_id: session_id.0.to_string(), binding_epoch: 4, load_generation: 9 };
            let inbox = ElicitationInbox::new();
            let coordinator = spawn_elicitation_coordinator(
                inbox.clone(), gateway, session_id.clone(), pending.clone(), source,
                Rc::new(Cell::new(false)),
            );
            let (response_tx, mut result) = tokio::sync::oneshot::channel();
            inbox.push(ElicitationJob {
                server_name: "fixture-server".into(),
                fields: WireElicitFields {
                    message: "private elicitation payload".into(),
                    mode: McpElicitModeFields::Form { requested_schema: None },
                },
                response_tx,
            }).unwrap();
            let mut started = None;
            let request = loop {
                match messages.recv().await.unwrap() {
                    AcpClientMessage::ExtMethod(request) => break request,
                    AcpClientMessage::ExtNotification(request) if request.request.method.as_ref() == TRANSITION_METHOD => {
                        let event: CurrentActionsEvent = serde_json::from_str(request.request.params.get()).unwrap();
                        event.validate().unwrap();
                        let CurrentActionTransition::InteractionStarted { interaction } = event.transition else { panic!("real interaction start"); };
                        started = Some(interaction);
                    }
                    _ => {}
                }
            };
            assert_eq!(request.request.method.as_ref(), MCP_ELICIT);
            let wire: McpElicitExtRequest = serde_json::from_str(request.request.params.get()).unwrap();
            assert_eq!(wire.session_id, session_id.0.as_ref());
            let captured = reader.snapshot(binding.clone());
            let CurrentActionsSnapshot::Current(before) = &captured else { panic!("same resident source"); };
            let [interaction] = before.interactions.as_slice() else { panic!("one real reverse request"); };
            assert_eq!(Some(interaction), started.as_ref());
            assert_eq!(interaction.tool_call_id, wire.tool_call_id);
            assert_eq!(interaction.kind, PendingKind::McpElicitation);
            assert!(matches!(interaction.source, InteractionSource::Registered { turn: FrozenOwner::Unavailable, .. }));
            assert!(before.turn.is_none());
            tokio::time::advance(std::time::Duration::from_secs(30)).await;
            let CurrentActionsSnapshot::Current(later) = reader.snapshot(binding.clone()) else { panic!("same reader"); };
            assert_eq!(later.interactions, before.interactions);
            assert_eq!(later.version, before.version);
            assert!(!request.response_tx.is_closed());
            assert!(matches!(result.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
            assert!(!serde_json::to_string(&later).unwrap().contains("private elicitation payload"));
            match finish {
                Finish::Answer => {
                    request.response_tx.send(Ok(acp::ExtResponse::new(
                        serde_json::value::to_raw_value(&McpElicitExtResponse::Decline).unwrap().into(),
                    ))).unwrap();
                    assert_eq!(result.await.unwrap().action, xai_grok_mcp::rmcp::model::ElicitationAction::Decline);
                    drop(coordinator);
                }
                Finish::ServerCancel => {
                    drop(result);
                    let terminal = loop {
                        let AcpClientMessage::ExtNotification(message) = messages.recv().await.unwrap() else { continue; };
                        if message.request.method.as_ref() != TRANSITION_METHOD { continue; }
                        let event: CurrentActionsEvent = serde_json::from_str(message.request.params.get()).unwrap();
                        if matches!(event.transition, CurrentActionTransition::InteractionFinished { .. }) { break event; }
                    };
                    terminal.validate().unwrap();
                    assert!(!captured.is_current_for(&binding, Some(terminal.version), None));
                    assert!(request.response_tx.is_closed());
                    drop(coordinator);
                }
                Finish::CoordinatorDrop => {
                    drop(coordinator);
                    assert!(result.await.is_err());
                    assert!(request.response_tx.is_closed());
                }
            }
            let CurrentActionsSnapshot::Current(settled) = reader.snapshot(binding.clone()) else { panic!("same reader after terminal"); };
            assert!(settled.interactions.is_empty());
            assert!(pending.lock().unwrap().is_empty());
            assert!(!captured.is_current_for(&binding, Some(settled.version), None));
        }
    }).await;
}
