use super::*;
use crate::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn real_awaited_hook_keeps_its_original_point_and_observation_never_answers_or_replays_it() {
    tokio::task::LocalSet::new().run_until(async {
        for cancel in [false, true] {
            let (actor, mut gateway, mut persistence) = test_actor().await;
            install_client_hook(&actor, xai_grok_hooks::event::HookEventName::PostToolUse, &["awaited-fixture"]);
            actor.current_actions.attach_emitter(actor.notifications.gateway.clone(), actor.notifications.gateway_enabled.clone(), actor.session_info.id.clone());
            let envelope = post_tool_use_envelope(&actor);
            let binding = SnapshotBinding { session_id: actor.session_info.id.0.to_string(), binding_epoch: 2, load_generation: 8 };
            let reader = actor.current_actions_reader();
            let mut run = Box::pin(actor.run_post_tool_use_client_hooks(&envelope));
            let mut started = None;
            let response = loop {
                tokio::select! {
                    result = run.as_mut() => panic!("hook completed before its real reverse response: {}", result.results.len()),
                    request = gateway.recv() => match request.unwrap() {
                        xai_acp_lib::AcpClientMessage::ExtMethod(request) => break request.response_tx,
                        xai_acp_lib::AcpClientMessage::ExtNotification(request) if request.request.method.as_ref() == crate::session::current_actions::TRANSITION_METHOD => {
                            let event: crate::session::current_actions::CurrentActionsEvent = serde_json::from_str(request.request.params.get()).unwrap();
                            event.validate().unwrap();
                            let crate::session::current_actions::CurrentActionTransition::HookStarted { hook } = event.transition else { panic!("actual start transition"); };
                            started = Some(hook);
                        }
                        xai_acp_lib::AcpClientMessage::SessionNotification(request) => { request.response_tx.send(Ok(())).unwrap(); }
                        _ => {}
                    }
                }
            };
            let captured = reader.snapshot(binding.clone());
            let CurrentActionsSnapshot::Current(before) = captured.clone() else { panic!("resident source"); };
            assert_eq!(before.hooks.len(), 1);
            let original = before.hooks[0].clone();
            assert_eq!(Some(original.clone()), started, "source event and cold snapshot share the same genuine batch and point");
            tokio::time::advance(std::time::Duration::from_secs(10)).await;
            let actor_lock = actor.state.lock().await;
            let CurrentActionsSnapshot::Current(later) = actor.current_actions_reader().snapshot(binding.clone()) else { panic!("read-only source"); };
            assert_eq!(later.hooks, [original]);
            assert_eq!(later.version, before.version);
            assert!(!response.is_closed(), "query cannot answer or migrate a reverse request");
            assert!(persistence.try_recv().is_err(), "query cannot enqueue an ACK, activity, or work event");
            let encoded = serde_json::to_string(&later).unwrap();
            for private_field in ["toolInput", "toolResult", "hookCallbackId", "systemMessage"] {
                assert!(!encoded.contains(private_field));
            }
            drop(actor_lock);
            if cancel {
                drop(run);
                assert!(response.is_closed());
            } else {
                response.send(Ok(acp::ExtResponse::new(serde_json::value::to_raw_value(
                    &serde_json::json!({}),
                ).unwrap().into()))).unwrap();
                run.await;
            }
            let terminal = loop {
                let xai_acp_lib::AcpClientMessage::ExtNotification(request) = gateway.recv().await.unwrap() else { continue; };
                if request.request.method.as_ref() != crate::session::current_actions::TRANSITION_METHOD { continue; }
                let event: crate::session::current_actions::CurrentActionsEvent = serde_json::from_str(request.request.params.get()).unwrap();
                event.validate().unwrap();
                let crate::session::current_actions::CurrentActionTransition::HookFinished { batch_id } = event.transition else { panic!("actual terminal transition"); };
                assert_eq!(batch_id, before.hooks[0].batch_id);
                break event.version;
            };
            assert!(!captured.is_current_for(&binding, Some(terminal), None));
            let CurrentActionsSnapshot::Current(settled) = reader.snapshot(binding.clone()) else { panic!("resident source"); };
            assert!(settled.hooks.is_empty());
            actor.handle_xai_session_notification(crate::extensions::notification::SessionNotification {
                session_id: actor.session_info.id.clone(),
                update: crate::extensions::notification::SessionUpdate::HookRunStarted {
                    event_name: "post_tool_use".into(), tool_name: None, prompt_id: None, count: 1,
                },
                meta: Some(serde_json::json!({"isReplay": true})),
            }).await;
            let CurrentActionsSnapshot::Current(after_replay) = reader.snapshot(binding) else { panic!("resident source"); };
            assert!(after_replay.hooks.is_empty());
            assert_eq!(after_replay.version, settled.version, "historical markers never register backend work");
        }
    }).await;
}
