use super::*;
use crate::session::source_binding::{SourceBindingObservation, StrictSourceOrigin, SOURCE_BINDING_NOTIFICATION};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter, SessionUpdate};
use xai_chat_state::{CompletionProjection, SubagentCompletionId};

#[path = "request_source_binding_loss_tests.rs"]
mod loss;
#[path = "request_source_binding_projection_tests.rs"]
mod projections;

async fn flush(tx: &mpsc::UnboundedSender<PersistenceMsg>) {
    let (respond_to, response) = oneshot::channel();
    tx.send(PersistenceMsg::FlushAndAck { respond_to }).unwrap();
    response.await.unwrap().unwrap();
}

async fn observed(rx: &mut mpsc::UnboundedReceiver<xai_acp_lib::AcpClientMessage>) -> SourceBindingObservation {
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
        if let xai_acp_lib::AcpClientMessage::ExtNotification(args) = message
            && args.request.method.as_ref() == SOURCE_BINDING_NOTIFICATION
        {
            return serde_json::from_str(args.request.params.get()).unwrap();
        }
    }
}

fn source(binding: &InputSourceBinding) -> xai_grok_sampling_types::SourceOccurrence {
    let InputSourceBinding::Applied { meta } = binding else { panic!("actual admission source required"); };
    assert!(matches!(meta.publication.durability, SourceDurability::UnconfirmedIndependentHistoryChannel));
    meta.publication.source
}

fn completion(id: &str) -> crate::session::SubagentCompletionEvent {
    crate::session::SubagentCompletionEvent {
        id: SubagentCompletionId { subagent_id: id.into(), attempt_id: xai_message_delivery_core::AttemptId::mint(1) },
        text: "same output".into(),
        state: xai_grok_tools::implementations::grok_build::task::types::SubagentState::Completed,
        output: "same output".into(), child_session_id: id.into(), tool_calls: 0, turns: 1,
        duration_ms: 0, tokens_used: 0, completed_at: chrono::Utc::now(),
        policy: crate::session::completion_delivery::CompletionEventPolicy::Automatic,
        artifacts: crate::session::completion_delivery::CompletionArtifacts::None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn strict_sources_follow_exact_persisted_receipts_and_replay_without_echo_or_readmission() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage.clone());
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, mut events) = mpsc::unbounded_channel();
        let (actor, _) = super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
        actor.state.lock().await.running_task = Some(super::super::support::running_task_stub("same-prompt"));
        let mut originals = Vec::new();
        let mut observations = Vec::new();
        for id in ["first-child", "second-child"] {
            let event = completion(id);
            storage.prepare_subagent_completion(&info, event.clone()).await.unwrap();
            assert!(actor.commit_subagent_projection(CompletionProjection::new(event.id.clone(), "same-prompt".into(), event.text.clone()), "same-prompt").await.unwrap());
            let observation = observed(&mut events).await;
            let SourceBindingObservation::Strict { identity: StrictSourceOrigin::Completion { session_id, completions }, binding } = &observation else { panic!("completion identity required"); };
            assert_eq!(session_id, "test-actor");
            assert_eq!(completions, &vec![event.id.clone().into()]);
            originals.push((event, source(binding)));
            observations.push(observation);
        }
        assert_ne!(originals[0].1, originals[1].1);
        let mut combined = CompletionProjection::new(originals[0].0.id.clone(), "same-prompt".into(), "retry is not another projection".into());
        combined.additional_receipts.push(xai_chat_state::CompletionReceipt {
            completion: originals[1].0.id.clone().into(), prompt_id: "same-prompt".into(), parent_task: xai_chat_state::ParentTask::Unknown,
        });
        assert!(!actor.commit_subagent_projection(combined, "same-prompt").await.unwrap());
        for (event, expected_source) in &originals {
            let SourceBindingObservation::Strict { identity: StrictSourceOrigin::Completion { completions, .. }, binding } = observed(&mut events).await else { panic!("each retained projection keeps its own receipt origin"); };
            assert_eq!(completions, vec![event.id.clone().into()]);
            assert_eq!(source(&binding), *expected_source);
        }
        let before = actor.chat_state_handle.get_conversation().await;
        assert_eq!(before.iter().map(ConversationItem::source).collect::<Vec<_>>(), originals.iter().map(|(_, source)| *source).collect::<Vec<_>>());
        let (event, admitted) = &originals[0];
        assert!(!actor.commit_subagent_projection(CompletionProjection::new(event.id.clone(), "same-prompt".into(), "unrelated retry body".into()), "same-prompt").await.unwrap());
        let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("retained receipt"); };
        assert_eq!(source(&binding), *admitted);
        actor.state.lock().await.running_task = Some(super::super::support::running_task_stub("same-prompt"));
        assert!(actor.commit_subagent_projection(CompletionProjection::new(event.id.clone(), "same-prompt".into(), "continuation body is not evidence".into()), "same-prompt").await.unwrap());
        let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("continued receipt"); };
        assert_eq!(source(&binding), *admitted);
        flush(&persistence.handle.tx).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        let restored = reopened.load_session(&info).await.unwrap();
        assert_eq!(restored.chat_history.len(), 2);
        assert_eq!(restored.chat_history.iter().map(ConversationItem::source).collect::<Vec<_>>(), originals.iter().map(|(_, source)| *source).collect::<Vec<_>>());
        let replayed: Vec<_> = restored.updates.iter().filter_map(|update| match update {
            SessionUpdate::Xai(notification) => match &notification.update {
                XaiSessionUpdate::RequestSourceObserved { observation } => Some(observation.clone()),
                _ => None,
            },
            SessionUpdate::Acp(_) => None,
        }).collect();
        assert_eq!(&replayed[..2], observations.as_slice());
        assert!(restored.updates.iter().all(|update| !matches!(update, SessionUpdate::Acp(notification) if matches!(notification.update, acp::SessionUpdate::UserMessageChunk(_)))));
        assert!(crate::session::storage::request_capture::published_sources(directory.path()).unwrap().is_empty(), "a binding observation is not a publication claim or proof of an echo");
        storage.replace_chat_history(&info, &[ConversationItem::assistant("compacted")]).await.unwrap();
        assert!(!actor.commit_subagent_projection(CompletionProjection::new(event.id.clone(), "same-prompt".into(), event.text.clone()), "same-prompt").await.unwrap());
        let SourceBindingObservation::Strict { binding, .. } = observed(&mut events).await else { panic!("compacted receipt"); };
        assert!(matches!(binding, InputSourceBinding::Unknown { cause: InputSourceUnknown::StrictReceiptSourceUnavailable }));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn output_receipts_keep_distinct_item_ids_and_honest_multi_source_channel_aggregates() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let info = crate::session::SessionInfo { id: acp::SessionId::new("test-actor"), cwd: "/tmp".into() };
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage);
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        let (gateway, mut events) = mpsc::unbounded_channel();
        let (actor, _) = super::super::support::create_test_actor_with_chat_persistence(0, 256_000, 85,
            gateway, persistence.handle.tx.clone(), Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence.handle.tx.clone()))).await;
        let request = xai_grok_sampler::RequestId::from("actual-output-request");
        let reasoning = |id: &str| ConversationItem::Reasoning(xai_grok_sampling_types::rs::ReasoningItem {
            id: id.into(), summary: vec![], content: None, encrypted_content: Some("same private body".into()), status: None,
        }.into());
        use xai_grok_sampling_types::rs;
        let message = |id: &str| rs::OutputItem::Message(rs::OutputMessage {
            id: id.into(), role: rs::AssistantRole::Assistant, status: rs::OutputStatus::Completed,
            content: vec![rs::OutputMessageContent::OutputText(rs::OutputTextContent { text: "same output".into(), annotations: vec![], logprobs: None })],
        });
        let mut items = vec![reasoning("reasoning-a"), reasoning("reasoning-b")];
        items.extend(xai_grok_sampling_types::response_to_conversation_items(rs::Response {
            background: None, billing: None, conversation: None, created_at: 0, completed_at: None,
            error: None, id: "provider-response".into(), incomplete_details: None, instructions: None,
            max_output_tokens: None, metadata: None, model: "test".into(), object: "response".into(),
            output: vec![message("message-a"), message("message-b"), rs::OutputItem::FunctionCall(rs::FunctionToolCall {
                arguments: "{}".into(), call_id: "actual-call".into(), name: "read_file".into(), id: None, status: None,
            })],
            parallel_tool_calls: None, previous_response_id: None, prompt: None, prompt_cache_key: None,
            prompt_cache_retention: None, reasoning: None, safety_identifier: None, service_tier: None,
            status: rs::Status::Completed, temperature: None, text: None, tool_choice: None, tools: None,
            top_logprobs: None, top_p: None, truncation: None, usage: None,
        }));
        actor.record_response_items(&request, None, items, true).await;
        let canonical = actor.chat_state_handle.get_conversation().await;
        assert_eq!(canonical.len(), 3);
        let mut origins = Vec::new();
        for _ in 0..5 {
            let observation = observed(&mut events).await;
            let SourceBindingObservation::Output { identity, bindings } = observation else { panic!("output receipt"); };
            assert_eq!(identity.request_id, request.as_str());
            assert_eq!(identity.session_id, "test-actor");
            origins.push((identity.reference, bindings));
        }
        assert_eq!(source(&origins[0].1[0]), canonical[0].source());
        assert_eq!(source(&origins[1].1[0]), canonical[1].source());
        assert_eq!(source(&origins[2].1[0]), canonical[2].source());
        assert_ne!(canonical[0].source(), canonical[1].source());
        let xai_grok_sampling_types::OutputOrigin::Aggregate { references } = &origins[2].0 else { panic!("multiple provider outputs cannot be a single item origin"); };
        assert_eq!(references.len(), 3);
        let xai_grok_sampling_types::OutputOrigin::ChannelAggregate { channel: xai_grok_sampling_types::OutputChannel::Reasoning } = origins[4].0 else { panic!("reasoning channel aggregate"); };
        assert_eq!(origins[4].1.iter().map(source).collect::<Vec<_>>(), vec![canonical[0].source(), canonical[1].source()]);
        assert_eq!(actor.chat_state_handle.get_conversation().await.len(), 3, "observing is not another admission or a model call");
        flush(&persistence.handle.tx).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        let restored = reopened.load_session(&info).await.unwrap();
        assert_eq!(restored.chat_history.iter().map(ConversationItem::source).collect::<Vec<_>>(), canonical.iter().map(ConversationItem::source).collect::<Vec<_>>());
        let replayed: Vec<_> = restored.updates.into_iter().filter_map(|update| match update {
            SessionUpdate::Xai(notification) => match notification.update {
                XaiSessionUpdate::RequestSourceObserved { observation: SourceBindingObservation::Output { identity, bindings } } => Some((identity.reference, bindings)),
                _ => None,
            },
            SessionUpdate::Acp(_) => None,
        }).collect();
        assert_eq!(replayed, origins);
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn input_binding_journal_retains_original_coordinates_without_inventing_echoes() {
    tokio::task::LocalSet::new().run_until(async {
        let directory = tempfile::tempdir().unwrap();
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
        let (gateway, _events) = mpsc::unbounded_channel();
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, tx).await;
        let info = actor.session_info.clone();
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage);
        let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
        actor.notifications.persistence_tx = persistence.handle.tx.clone();
        actor.current_turn_number.set(4);
        let input = actor.prompt_input_ref("original-input");
        let applied = actor.chat_state_handle.push_user_message_and_ack(ConversationItem::user("never echoed")).await.unwrap();
        actor.current_turn_number.set(99);
        actor.observe_input_source(input.clone(), Ok(applied));
        actor.observe_input_source(actor.prompt_input_ref("missing-reply"), Err(InputSourceUnknown::ReplyLost));
        flush(&persistence.handle.tx).await;
        let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
        let restored = reopened.load_session(&info).await.unwrap();
        let observations: Vec<_> = restored.updates.iter().filter_map(|update| match update {
            SessionUpdate::Xai(notification) => match &notification.update {
                XaiSessionUpdate::RequestSourceObserved { observation: SourceBindingObservation::Input { notification } } => Some(notification),
                _ => None,
            },
            SessionUpdate::Acp(_) => None,
        }).collect();
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].input, input);
        assert_eq!(source(&observations[0].binding), applied.source);
        assert!(matches!(observations[1].binding, InputSourceBinding::Unknown { cause: InputSourceUnknown::ReplyLost }));
        assert!(restored.chat_history.is_empty(), "an observation cannot turn an independent in-memory append into durable model history");
        assert!(restored.updates.iter().all(|update| matches!(update, SessionUpdate::Xai(_))));
        assert!(crate::session::storage::request_capture::published_sources(directory.path()).unwrap().is_empty());
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn provider_message_identity_remains_an_aggregate_for_multiple_unidentified_reasoning_items() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, mut events) = mpsc::unbounded_channel();
        let (tx, _rx) = mpsc::unbounded_channel();
        let actor = super::super::support::create_test_actor(0, 256_000, 85, gateway, tx).await;
        let request = xai_grok_sampler::RequestId::from("message-request");
        actor.record_response_items(&request, Some("provider-message"), vec![
            ConversationItem::Reasoning(xai_grok_sampling_types::synthesized_reasoning_item("first").into()),
            ConversationItem::Reasoning(xai_grok_sampling_types::synthesized_reasoning_item("second").into()),
            ConversationItem::assistant("answer"),
        ], true).await;
        let canonical = actor.chat_state_handle.get_conversation().await;
        assert_eq!(canonical.len(), 3);
        for channel in [xai_grok_sampling_types::OutputChannel::Text, xai_grok_sampling_types::OutputChannel::Reasoning] {
            let SourceBindingObservation::Output { identity, bindings } = observed(&mut events).await else { panic!("provider message aggregate"); };
            assert_eq!(identity.reference, xai_grok_sampling_types::OutputOrigin::MessageChannelAggregate { message_id: "provider-message".into(), channel });
            match channel {
                xai_grok_sampling_types::OutputChannel::Text => assert_eq!(bindings.iter().map(source).collect::<Vec<_>>(), vec![canonical[2].source()]),
                xai_grok_sampling_types::OutputChannel::Reasoning => assert_eq!(bindings.iter().map(source).collect::<Vec<_>>(), vec![canonical[0].source(), canonical[1].source()]),
            }
        }
    }).await;
}
