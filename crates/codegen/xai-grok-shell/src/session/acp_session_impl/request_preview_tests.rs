use super::*;
use crate::session::acp_session::support::create_test_actor_with_chat_persistence;
use crate::session::helpers::context_policy_store::{ContextPolicyBinding, ContextPolicyStore};
use xai_grok_sampling_types::{ContextCategory, ContextPolicyEdit, ContextPolicyMutation, ContextScope, ContextSwitch};

#[tokio::test(flavor = "current_thread")]
async fn loaded_preview_reads_new_global_policy_without_applying_it_or_publishing_draft() {
    tokio::task::LocalSet::new().run_until(async {
        let root = tempfile::tempdir().unwrap();
        let legacy = tempfile::tempdir().unwrap();
        let (gateway, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, mut persisted) = tokio::sync::mpsc::unbounded_channel();
        let (chat_persistence, mut chat_records) = xai_chat_state::MockChatPersistence::new();
        let (mut actor, mut events) = create_test_actor_with_chat_persistence(
            0, 256_000, 85, gateway, persistence, Box::new(chat_persistence),
        ).await;
        actor.context_policy_binding = ContextPolicyBinding::new(std::sync::Arc::new(
            tokio::sync::RwLock::new(ContextPolicyStore::new(root.path().to_owned())),
        ));
        let cfg = actor.chat_state_handle.get_sampling_config().await.unwrap();
        let (sampler_events, mut sampler_rx) = tokio::sync::mpsc::unbounded_channel();
        actor.sampler_handle = xai_grok_sampler::SamplerActor::spawn(xai_grok_sampler::SamplerConfig {
            model: cfg.model, api_backend: cfg.api_backend, ..Default::default()
        }, xai_grok_sampler::RetryPolicy::default(), sampler_events);
        actor.chat_state_handle.push_user_message_and_ack(ConversationItem::user("old ask")).await.unwrap();
        actor.chat_state_handle.push_model_output(ConversationItem::assistant("old answer"));
        let before = actor.chat_state_handle.snapshot().await.unwrap();
        let old_policy = actor.chat_state_handle.get_context_policy().await.unwrap();
        let applied = actor.context_policy_binding.applied_revision.load(std::sync::atomic::Ordering::Acquire);
        {
            let mut store = actor.context_policy_binding.authority.write().await;
            let id = actor.session_info.id.0.as_ref();
            store.ensure_session(id, legacy.path()).unwrap();
            let expected = store.selection(id, &actor.context_policy_binding.incarnation).unwrap().version;
            store.edit(id, &actor.context_policy_binding.incarnation, &ContextPolicyEdit {
                expected, mutation: ContextPolicyMutation::SetCategories {
                    categories: vec![ContextCategory::AssistantTurns].try_into().unwrap(),
                    value: ContextSwitch::Excluded, target: ContextScope::Global,
                },
            }).unwrap();
        }
        let durable = std::fs::read(root.path().join("context_policy.toml")).unwrap();
        while notifications.try_recv().is_ok() {}
        while persisted.try_recv().is_ok() {}
        while events.try_recv().is_ok() {}
        chat_records.drain();
        let request = RequestPreviewRequest {
            session_id: actor.session_info.id.0.to_string(), ui_generation: 7,
            input: PreviewInput::Draft {
                delivery: PreviewDelivery::Prompt,
                blocks: vec![acp::ContentBlock::Text(acp::TextContent::new("unsent canary"))], json_schema: None,
            },
        };
        for _ in 0..2 {
            let result = actor.preview_next_request(request.clone()).await;
            assert!(!format!("{result:?}").contains("unsent canary"));
            let RequestPreview::Partial { snapshot, first, additional } = result else { panic!("read-only loaded candidate"); };
            assert_eq!(snapshot.key.ui_generation, 7);
            assert!(snapshot.key.context_policy_revision > applied);
            assert_eq!(snapshot.key.canonical_history_revision, *blake3::hash(&serde_json::to_vec(&before.conversation).unwrap()).as_bytes());
            assert!(!snapshot.context_policy.includes(ContextCategory::AssistantTurns));
            assert!(snapshot.projected_items.iter().any(|item| item.text_content() == "<user_query>\nunsent canary\n</user_query>"));
            assert!(!snapshot.projected_items.iter().any(|item| item.text_content() == "old answer"));
            assert!(matches!(first, PreviewLateBound::ScheduledInjections));
            assert!(additional.contains(&PreviewLateBound::InterruptEnvelope));
            assert_eq!(actor.context_policy_binding.applied_revision.load(std::sync::atomic::Ordering::Acquire), applied);
            assert_eq!(actor.chat_state_handle.get_context_policy().await.unwrap(), old_policy);
            assert_eq!(serde_json::to_vec(&actor.chat_state_handle.snapshot().await.unwrap()).unwrap(), serde_json::to_vec(&before).unwrap());
            assert_eq!(std::fs::read(root.path().join("context_policy.toml")).unwrap(), durable);
            assert!(actor.state.lock().await.pending_inputs.is_empty());
            assert!(actor.pending_interjections.is_empty());
            assert!(notifications.try_recv().is_err());
            assert!(persisted.try_recv().is_err());
            assert!(events.try_recv().is_err());
            assert!(sampler_rx.try_recv().is_err());
            assert!(chat_records.drain().is_empty());
            assert_eq!(actor.sampler_handle.active_count().await, 0);
        }
        let unresolved_schema = RequestPreviewRequest {
            session_id: request.session_id.clone(), ui_generation: 8,
            input: PreviewInput::Draft { delivery: PreviewDelivery::Prompt,
                blocks: vec![acp::ContentBlock::Text(acp::TextContent::new("schema draft"))],
                json_schema: Some(serde_json::json!({"$ref": "https://preview.invalid/schema"})) },
        };
        assert!(matches!(actor.preview_next_request(unresolved_schema).await,
            RequestPreview::Unavailable { cause: PreviewUnavailable::SchemaNotPrepared, .. }));
        assert!(notifications.try_recv().is_err());
        assert!(persisted.try_recv().is_err());
        assert!(sampler_rx.try_recv().is_err());
        assert!(chat_records.drain().is_empty());
        let attachment = RequestPreviewRequest {
            session_id: request.session_id.clone(), ui_generation: 8,
            input: PreviewInput::Draft { delivery: PreviewDelivery::Prompt,
                blocks: vec![acp::ContentBlock::Image(acp::ImageContent::new("eA==", "image/png"))], json_schema: None },
        };
        let input_before = serde_json::to_vec(match &attachment.input {
            PreviewInput::Draft { blocks, .. } => blocks,
            PreviewInput::Current => panic!("draft"),
        }).unwrap();
        let RequestPreview::Partial { snapshot, additional, .. } = actor.preview_next_request(attachment.clone()).await else { panic!("partial attachment"); };
        assert!(additional.contains(&PreviewLateBound::AttachmentPreparation));
        assert!(snapshot.projected_items.iter().all(|item| item.source() != xai_grok_sampling_types::SourceOccurrence::Unadmitted));
        assert_eq!(serde_json::to_vec(match &attachment.input {
            PreviewInput::Draft { blocks, .. } => blocks,
            PreviewInput::Current => panic!("draft"),
        }).unwrap(), input_before);
        assert_eq!(serde_json::to_vec(&actor.chat_state_handle.snapshot().await.unwrap()).unwrap(), serde_json::to_vec(&before).unwrap());
        let (respond_to, mut response) = tokio::sync::oneshot::channel();
        let queued = super::super::prompt_queue::QueueInputRequest::from_legacy_prompt_id(
            vec![acp::ContentBlock::Text(acp::TextContent::new("queued question"))],
            "preview-queued-user".into(), crate::session::plan_mode::PromptMode::Agent, respond_to,
        );
        assert!(!actor.queue_input(queued).await);
        let queue_before: Vec<_> = actor.state.lock().await.pending_inputs.iter()
            .map(|item| (item.prompt_id.clone(), serde_json::to_vec(&item.prompt_blocks).unwrap())).collect();
        while notifications.try_recv().is_ok() {}
        while persisted.try_recv().is_ok() {}
        let queued_request = RequestPreviewRequest { input: PreviewInput::Current, ..request };
        let RequestPreview::Partial { snapshot, additional, .. } = actor.preview_next_request(queued_request).await else { panic!("queued candidate"); };
        assert!(matches!(snapshot.input_position, PreviewInputPosition::QueuedInput));
        assert!(additional.contains(&PreviewLateBound::QueuePromotion));
        assert!(snapshot.projected_items.iter().any(|item| item.text_content() == "<user_query>\nqueued question\n</user_query>"));
        let queue_after: Vec<_> = actor.state.lock().await.pending_inputs.iter()
            .map(|item| (item.prompt_id.clone(), serde_json::to_vec(&item.prompt_blocks).unwrap())).collect();
        assert_eq!(queue_after, queue_before);
        assert!(matches!(response.try_recv(), Err(tokio::sync::oneshot::error::TryRecvError::Empty)));
        assert!(notifications.try_recv().is_err());
        assert!(persisted.try_recv().is_err());
        assert!(sampler_rx.try_recv().is_err());
        assert!(chat_records.drain().is_empty());
    }).await;
}

#[test]
fn unavailable_authority_and_uninspected_legacy_are_not_loaded_or_migrated_by_preview() {
    let root = tempfile::tempdir().unwrap();
    let legacy = tempfile::tempdir().unwrap();
    let bytes = serde_json::to_vec(&xai_grok_sampling_types::ContextPolicy::default()).unwrap();
    std::fs::write(legacy.path().join("context_policy.json"), &bytes).unwrap();
    let mut store = ContextPolicyStore::new(root.path().to_owned());
    assert!(matches!(store.preview_snapshot("s", "binding"), Err(ContextPreviewUnavailable::AuthorityNotLoaded)));
    assert!(!root.path().join("context_policy.toml").exists());
    store.load().unwrap();
    assert!(matches!(store.preview_snapshot("s", "binding"), Err(ContextPreviewUnavailable::LegacyNotInspected)));
    assert_eq!(std::fs::read(legacy.path().join("context_policy.json")).unwrap(), bytes);
    assert!(!root.path().join("context_policy.toml").exists());
}
