use super::super::AutoCompactTriggerInfo;
use super::*;
use crate::extensions::notification::{CompactPhase, SessionNotification, SessionUpdate};
use crate::session::acp_session::support;
use crate::session::acp_types::{AUTOMATIC_COMPACT_META_KEY, AutomaticCompactInvocation};

fn enable_two_pass(actor: &SessionActor) {
    let mut slot = actor.agent.borrow_mut();
    let agent = &*slot;
    let mut policy = agent.compaction_policy().clone();
    policy.two_pass_enabled = true;
    *slot = xai_grok_agent::Agent::new(
        agent.definition().clone(),
        agent.prompt_context().clone(),
        agent.system_prompt().to_string(),
        Arc::clone(agent.tool_bridge()),
        agent.reminder_policy().clone(),
        policy,
        vec![],
        false,
    );
}

fn seed_prefire(actor: &SessionActor) {
    actor.chat_state_handle.replace_conversation(vec![
        ConversationItem::system("sys"),
        ConversationItem::user("repair file"),
        ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
            id: "prefix-edit".into(),
            name: "edit_file".into(),
            arguments: "{\"path\":\"before.rs\"}".into(),
        }]),
        ConversationItem::tool_result("prefix-edit", "edited"),
        ConversationItem::assistant("Completed original work. ".repeat(2_000)),
        ConversationItem::user("preserve the remaining task"),
    ]);
}

#[tokio::test(flavor = "current_thread")]
async fn prefire_allows_concurrent_tail_append_but_rejects_changed_prefix_tool_arguments() {
    tokio::task::LocalSet::new()
        .run_until(async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                for change_prefix in [false, true] {
                    let fixture = fixture().await;
                    let server = route_to_mock(&fixture.actor).await;
                    enable_two_pass(&fixture.actor);
                    seed_prefire(&fixture.actor);
                    server.set_auxiliary_hold();
                    assert!(fixture.actor.compaction.prefire.try_begin());
                    let actor = fixture.actor.clone();
                    let prefire =
                        tokio::task::spawn_local(async move { actor.run_prefire_pass1().await });
                    server.wait_until_parked_replies(1).await;
                    fixture
                        .actor
                        .chat_state_handle
                        .push_user_message(ConversationItem::user("concurrent-tail-marker"));
                    if change_prefix {
                        let mut conversation =
                            fixture.actor.chat_state_handle.get_conversation().await;
                        let ConversationItem::Assistant(assistant) = &mut conversation[2] else {
                            panic!("seeded tool call");
                        };
                        assistant.tool_calls[0].arguments = "{\"path\":\"after.rs\"}".into();
                        fixture
                            .actor
                            .chat_state_handle
                            .replace_conversation(conversation);
                    }
                    server.release_parked_replies();
                    prefire.await.unwrap();
                    assert!(
                        fixture.actor.compaction.prefire.has_cache(),
                        "pass-1 must have actually completed"
                    );
                    let before = server.request_count();
                    let output = fixture.actor.try_two_pass_pass2_apply(None, false).await;
                    if change_prefix {
                        assert!(
                            output.is_none(),
                            "NOTE1 for different tool arguments cannot be applied"
                        );
                        assert_eq!(
                            server.request_count(),
                            before,
                            "stale NOTE1 must be rejected before a pass-2 request"
                        );
                    } else {
                        assert!(
                            output.is_some(),
                            "an append-only tail must not invalidate NOTE1"
                        );
                        assert_eq!(server.request_count(), before + 1);
                        assert!(
                            server
                                .request_bodies()
                                .last()
                                .unwrap()
                                .to_string()
                                .contains("concurrent-tail-marker"),
                            "the actual pass-2 request must contain the concurrent append"
                        );
                    }
                    assert!(
                        fixture
                            .actor
                            .chat_state_handle
                            .get_conversation()
                            .await
                            .iter()
                            .any(|item| item.text_content() == "concurrent-tail-marker")
                    );
                }
            })
            .await
            .expect("isolated prefire fixture must settle");
        })
        .await;
}

#[derive(Clone, Copy)]
enum Outcome {
    Completed,
    Failed,
    Cancelled,
}

#[tokio::test(flavor = "current_thread")]
async fn automatic_compaction_publishes_one_correlated_terminal_for_each_started_operation() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            for outcome in [Outcome::Completed, Outcome::Failed, Outcome::Cancelled] {
                let (gateway, mut messages) = mpsc::unbounded_channel();
                let (notices, mut received) = mpsc::unbounded_channel();
                tokio::task::spawn_local(async move {
                    while let Some(message) = messages.recv().await {
                        match message {
                            xai_acp_lib::AcpClientMessage::SessionNotification(args) => { let _ = args.response_tx.send(Ok(())); }
                            xai_acp_lib::AcpClientMessage::ExtNotification(args) => {
                                if args.request.method.as_ref() == "x.ai/session_notification" {
                                    let notification: SessionNotification = serde_json::from_str(args.request.params.get()).unwrap();
                                    let _ = notices.send(notification);
                                }
                            }
                            _ => {}
                        }
                    }
                });
                let (mut actor, events, _writes) = support::create_observed_completion_test_actor(
                    50_000, 200_000, 85, gateway,
                ).await;
                let sampling = wire_sampler(&mut actor).await;
                let actor = Arc::new(actor);
                seed_prefire(&actor);
                *actor.current_prompt_id.lock().unwrap() = Some("automatic-fixture-prompt".into());
                let server = route_to_mock(&actor).await;
                match outcome {
                    Outcome::Completed => {}
                    Outcome::Failed => server.set_conversations(vec![xai_grok_test_support::Conversation::nth(1)
                        .refuse(xai_grok_test_support::StatusFailure::new(400).with_body("invalid request"))]),
                    Outcome::Cancelled => server.set_auxiliary_hold(),
                }
                let (_sender, _task) = start_fixture_actor(actor.clone(), events, sampling);
                let worker_actor = actor.clone();
                let worker = tokio::task::spawn_local(async move {
                    worker_actor.run_compact_only(AutoCompactTriggerInfo {
                        tokens_used: 50_000, context_window: 200_000, percentage: 25, reason_override: None,
                    }, false).await
                });
                if matches!(outcome, Outcome::Cancelled) {
                    server.wait_until_parked_replies(1).await;
                    actor.compaction.cancel.request_cancel();
                    server.release_parked_replies();
                }
                let result = worker.await.unwrap();
                assert_eq!(result.is_ok(), matches!(outcome, Outcome::Completed));
                actor.send_xai_notification(SessionUpdate::MemoryFlushCompleted { result: "fixture-barrier".into(), path: None }).await;
                let mut lifecycle = Vec::new();
                loop {
                    let notification = received.recv().await.unwrap();
                    if matches!(&notification.update, SessionUpdate::MemoryFlushCompleted { result, path: None } if result == "fixture-barrier") {
                        break;
                    }
                    if matches!(notification.update, SessionUpdate::AutoCompactStarted { .. }
                        | SessionUpdate::AutoCompactCompleted { .. } | SessionUpdate::AutoCompactFailed { .. }
                        | SessionUpdate::AutoCompactCancelled { .. }) {
                        lifecycle.push(notification);
                    }
                }
                assert_eq!(lifecycle.len(), 2, "one start and exactly one terminal outcome");
                assert!(matches!(lifecycle[0].update, SessionUpdate::AutoCompactStarted { phase: CompactPhase::Executing, .. }));
                let invocation: AutomaticCompactInvocation = serde_json::from_value(lifecycle[0].meta.as_ref().unwrap()
                    .get(AUTOMATIC_COMPACT_META_KEY).unwrap().clone()).unwrap();
                assert_eq!(invocation.prompt_id.as_deref(), Some("automatic-fixture-prompt"));
                assert_eq!(lifecycle[1].meta.as_ref().unwrap().get(AUTOMATIC_COMPACT_META_KEY), Some(&serde_json::to_value(&invocation).unwrap()));
                match outcome {
                    Outcome::Completed => assert!(matches!(lifecycle[1].update, SessionUpdate::AutoCompactCompleted { .. })),
                    Outcome::Failed => assert!(matches!(lifecycle[1].update, SessionUpdate::AutoCompactFailed { .. })),
                    Outcome::Cancelled => assert!(matches!(lifecycle[1].update, SessionUpdate::AutoCompactCancelled { .. })),
                }
                assert!(actor.compaction.cancel.activity_invocation().is_none());
            }
        }).await.expect("isolated compaction fixture must settle");
    }).await;
}
