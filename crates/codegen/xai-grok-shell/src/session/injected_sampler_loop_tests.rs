use super::*;
use crate::session::storage::{SessionUpdate, SessionUpdateEnvelope, StorageAdapter};
use xai_grok_test_support::sse::messages_api_script;
use xai_grok_test_support::{InferenceEndpoint, InferenceRequestMatcher, MockInferenceServer, ScriptedResponse};

fn injection_records(updates: impl IntoIterator<Item = SessionUpdate>, reason: crate::sampling::SyntheticReason) -> Vec<serde_json::Value> {
    updates.into_iter().filter_map(|update| match update {
        SessionUpdate::Xai(notification) if matches!(&notification.update,
            XaiSessionUpdate::InjectedContextRecord { kind, .. } if kind == reason.wire_name()) => {
            Some(serde_json::to_value(&*notification).unwrap())
        }
        SessionUpdate::Acp(_) | SessionUpdate::Xai(_) => None,
    }).collect()
}

#[tokio::test(flavor = "current_thread")]
async fn sampler_loops_publish_the_actual_length_injection_once_after_a_changed_request_view() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let server = MockInferenceServer::start().await.unwrap();
            server.set_keep_requests(true);
            let mut first = server.expect_response_blocked(
                "initial request without length injection",
                InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Messages, "new input"),
                ScriptedResponse::sse(messages_api_script("display segment one", "test", "max_tokens")),
            );
            let mut second = server.expect_response_blocked(
                "changed request containing actual length injection",
                InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Messages,
                    "Your previous response exceeded the output token"),
                ScriptedResponse::sse(messages_api_script("display segment two", "test", "max_tokens")),
            );
            let mut third = server.expect_response_blocked(
                "later request retaining the identical injection",
                InferenceRequestMatcher::foreground_containing(InferenceEndpoint::Messages, "display segment two"),
                ScriptedResponse::sse(messages_api_script("display segment final", "test", "end_turn")),
            );
            let (gateway_tx, mut gateway_rx) = mpsc::unbounded_channel();
            let live = Rc::new(RefCell::new(Vec::new()));
            let captured = live.clone();
            let equal_body = "display equal-body occurrence";
            let equal_live = Rc::new(RefCell::new(Vec::new()));
            let equal_captured = equal_live.clone();
            let changed = Rc::new(tokio::sync::Notify::new());
            let notified = changed.clone();
            let gateway_task = tokio_util::task::AbortOnDropHandle::new(tokio::task::spawn_local(async move {
                while let Some(message) = gateway_rx.recv().await {
                    match message {
                        xai_acp_lib::AcpClientMessage::SessionNotification(args) => {
                            let _ = args.response_tx.send(Ok(()));
                        }
                        xai_acp_lib::AcpClientMessage::ExtNotification(args) => {
                            if args.request.method.as_ref() == "x.ai/session_notification" {
                                let notification: XaiSessionNotification = serde_json::from_str(args.request.params.get()).unwrap();
                                if matches!(&notification.update, XaiSessionUpdate::InjectedContextRecord { kind, text }
                                    if kind == "system_reminder" && text.contains(equal_body))
                                {
                                    equal_captured.borrow_mut().push(serde_json::to_value(&notification).unwrap());
                                    notified.notify_one();
                                }
                                if matches!(&notification.update, XaiSessionUpdate::InjectedContextRecord { kind, .. } if kind == "length_continue") {
                                    captured.borrow_mut().push(serde_json::to_value(notification).unwrap());
                                    notified.notify_one();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }));
            let (capture, _unused_persistence) = mpsc::unbounded_channel();
            let (mut actor, events) = create_test_actor_ex(0, 256_000, 85, gateway_tx, capture).await;
            let directory = tempfile::tempdir().unwrap();
            actor.session_info.cwd = directory.path().to_string_lossy().into_owned();
            actor.length_salvage_remote_budget = Some(2);
            assert_eq!(actor.length_salvage_budget(), Some(2));
            let mut config = actor.chat_state_handle.get_sampling_config().await.unwrap();
            config.base_url = server.url();
            config.api_backend = crate::sampling::ApiBackend::Messages;
            actor.chat_state_handle.update_sampling_config(config);
            let persistence = crate::session::persistence::new_with_explicit_dir(
                &actor.session_info, directory.path().to_path_buf(), acp::ModelId::new("test"),
                crate::session::acp_session::updates_tests::test_persistence_sampling_client(), "test".into(),
                crate::session::persistence::ExplicitSessionOpen::New {
                    identity: None, next_trace_turn: None, parent_session_id: None,
                },
            ).await.unwrap();
            actor.notifications.persistence_tx = persistence.tx.clone();
            let sampling_events = wire_sampler(&mut actor).await;
            let actor = Arc::new(actor);
            actor.chat_state_handle.replace_conversation(vec![ConversationItem::system("sys")]);
            actor.push_system_reminder(equal_body);
            let (sender, _session_task) = start_fixture_actor(actor.clone(), events, sampling_events);
            let response = send_named_input(&sender, "display-sampler-loops", false);
            first.wait_blocked().await;
            assert!(live.borrow().is_empty(), "the first request has no length injection");
            let reminder = crate::session::acp_session::length_salvage::LENGTH_CONTINUE_REMINDER_BODY;
            assert!(server.request_bodies().iter().all(|body| !body.to_string().contains(reminder)));
            while equal_live.borrow().is_empty() { changed.notified().await; }
            assert_eq!(equal_live.borrow().len(), 1);
            actor.push_system_reminder(equal_body);
            first.release();
            second.wait_blocked().await;
            while live.borrow().is_empty() { changed.notified().await; }
            assert_eq!(live.borrow().len(), 1, "the actual changed second view must publish its injection");
            let tag = actor.reminder_wrapper_tag();
            let expected_text = format!("<{tag}>{reminder}</{tag}>");
            assert_eq!(live.borrow()[0]["update"]["text"], expected_text);
            let reopened = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
            let path = reopened.updates_file_path(&actor.session_info).unwrap();
            let read_injections = |reason| {
                injection_records(std::fs::read_to_string(&path).unwrap().lines()
                    .map(|line| SessionUpdateEnvelope::from_str(line).unwrap()).collect::<Vec<_>>(), reason)
            };
            let read_records = || read_injections(crate::sampling::SyntheticReason::LengthContinue);
            let read_equal_records = || read_injections(crate::sampling::SyntheticReason::SystemReminder).into_iter()
                .filter(|record| record["update"]["text"].as_str().is_some_and(|text| text.contains(equal_body))).collect::<Vec<_>>();
            while equal_live.borrow().len() < 2 { changed.notified().await; }
            assert_eq!(equal_live.borrow().len(), 2);
            actor.flush_to_disk().await.unwrap();
            assert_eq!(read_records(), *live.borrow(), "the second sampler request follows durable publication with unchanged stamps");
            assert_eq!(read_equal_records(), *equal_live.borrow());
            assert_ne!(equal_live.borrow()[0]["_meta"]["eventId"], equal_live.borrow()[1]["_meta"]["eventId"]);
            second.release();
            third.wait_blocked().await;
            actor.flush_to_disk().await.unwrap();
            assert_eq!(read_records().len(), 1, "an identical injected view on the third sampler loop must not append again");
            assert_eq!(live.borrow().len(), 1);
            assert_eq!(read_equal_records().len(), 2, "the unchanged third view must not duplicate either equal occurrence");
            assert_eq!(equal_live.borrow().len(), 2);
            let requests: Vec<_> = server.request_bodies().into_iter()
                .filter(|body| body.get("messages").is_some()).collect();
            assert_eq!(requests.len(), 3, "three real sampler submissions are required");
            for (index, body) in requests.iter().enumerate() {
                let wire = body.to_string();
                assert_eq!(wire.matches(reminder).count(), usize::from(index > 0));
                assert_eq!(wire.matches(equal_body).count(), (index + 1).min(2));
                assert!(!wire.contains("injected_context_record"), "UI metadata cannot add model conversation items");
            }
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(conversation.iter().filter(|item| matches!(item,
                ConversationItem::User(user) if user.synthetic_reason == crate::sampling::SyntheticReason::LengthContinue)).count(), 1);
            assert_eq!(conversation.iter().filter(|item| matches!(item,
                ConversationItem::User(user) if user.synthetic_reason == crate::sampling::SyntheticReason::SystemReminder)
                && item.text_content().contains(equal_body)).count(), 2);
            third.release();
            response.await.unwrap().expect("the original prompt completes after both actual continuation loops");
            for expectation in [&first, &second, &third] { expectation.assert_satisfied(); }
            assert_eq!(read_records(), *live.borrow());
            assert_eq!(read_records().len(), 1);
            assert_eq!(read_equal_records(), *equal_live.borrow());
            assert_eq!(read_equal_records().len(), 2);
            let _gateway_task = gateway_task;
        }).await.expect("scripted sampler loops and acknowledged request publication must complete");
    }).await;
}
