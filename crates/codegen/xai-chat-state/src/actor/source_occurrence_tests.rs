use super::*;
use xai_grok_sampling_types::{ContextCategory, ContextPolicy, ContextSwitch, SourceOccurrence};

#[tokio::test]
async fn append_ack_returns_each_actual_admission_without_observational_appends() {
    let mut h = TestHarness::new();
    let first = h.handle.push_user_message_and_ack(ConversationItem::user("same")).await.unwrap();
    let second = h.handle.push_user_message_and_ack(ConversationItem::user("same")).await.unwrap();
    assert_ne!(first.source, second.source);
    let history = h.handle.get_conversation().await;
    assert_eq!(history.iter().map(ConversationItem::source).collect::<Vec<_>>(), [first.source, second.source]);
    let persisted: Vec<_> = h.drain_persistence().into_iter().filter_map(|record| match record {
        PersistenceRecord::Message(item) => Some(item.source()),
        _ => None,
    }).collect();
    assert_eq!(persisted, [first.source, second.source]);
    assert_eq!(h.handle.get_conversation().await.len(), 2);
    assert!(h.drain_persistence().is_empty());
}

#[tokio::test]
async fn batch_ack_keeps_generated_and_human_sources_separate() {
    let h = TestHarness::new();
    let applied = h.handle.push_user_messages_batch_and_ack(vec![
        ConversationItem::working_directory_switch("cwd", 1),
        ConversationItem::user("same"),
    ]).await.unwrap();
    assert_eq!(applied.len(), 2);
    assert_ne!(applied.first().unwrap().source, applied.get(1).unwrap().source);
    assert_eq!(applied.iter().map(|ack| ack.source).collect::<Vec<_>>(),
        h.handle.get_conversation().await.iter().map(ConversationItem::source).collect::<Vec<_>>());
}

#[tokio::test]
async fn append_ack_reports_admission_owner_not_incoming_source() {
    let h = TestHarness::new();
    let first = h.handle.push_user_message_and_ack(ConversationItem::user("original")).await.unwrap();
    let mut reused = ConversationItem::user("reused");
    *reused.source_mut() = first.source;
    let applied = h.handle.push_user_message_and_ack(reused).await.unwrap();
    assert_ne!(applied.source, first.source);
    assert_eq!(Some(applied.source), h.handle.get_conversation().await.get(1).map(ConversationItem::source));
}

#[tokio::test]
async fn append_transport_distinguishes_closed_mailbox_from_reply_loss() {
    assert!(matches!(crate::ChatStateHandle::noop().push_user_message_and_ack(
        ConversationItem::user("not admitted")
    ).await, Err(crate::MutationAdmissionError::MailboxClosed(_))));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = crate::ChatStateHandle::new(tx);
    let request = tokio::spawn(async move {
        handle.push_user_message_and_ack(ConversationItem::user("may apply")).await
    });
    let command = rx.recv().await.unwrap();
    let crate::commands::ChatStateCommand::PushUserMessageAndAck { reply, .. } = command else {
        panic!("expected append command");
    };
    drop(reply);
    assert!(matches!(request.await.unwrap(), Err(crate::MutationAdmissionError::ReplyLost(_))));
}

#[tokio::test]
async fn strict_switch_retry_returns_the_authoritative_source() {
    let h = TestHarness::new();
    let generation = std::num::NonZeroU64::new(1).unwrap();
    let first = h.handle.append_working_directory_switch_with_source_and_ack("first".into(), generation).await.unwrap();
    let retry = h.handle.append_working_directory_switch_with_source_and_ack("different retry".into(), generation).await.unwrap();
    assert_eq!(first.binding, retry.binding);
    assert!(matches!(first.binding, crate::AppendBinding::Applied(_)));
    assert!(matches!(retry.acknowledgement, crate::StrictAppendAck::AlreadyPresent(_)));
    assert_eq!(h.handle.get_conversation().await.len(), 1);
}

#[tokio::test]
async fn legacy_strict_switch_receipt_does_not_publish_the_discarded_candidate() {
    let mut h = TestHarness::with_manual_persistence_ack(vec![]);
    let handle = h.handle.clone();
    let request = tokio::spawn(async move {
        handle.append_working_directory_switch_with_source_and_ack(
            "candidate".into(), std::num::NonZeroU64::new(1).unwrap(),
        ).await
    });
    let reply = h.persistence_rx.next_persistence_ack().await.unwrap();
    reply.send(Ok(crate::StrictAppendAck::AlreadyPresent(
        ConversationItem::working_directory_switch("legacy", 1),
    ))).unwrap();
    let ack = request.await.unwrap().unwrap();
    assert_eq!(ack.binding, crate::AppendBinding::ReceiptSourceUnavailable);
    assert_eq!(h.handle.get_conversation().await.first().unwrap().text_content(), "legacy");
}

#[tokio::test]
async fn strict_append_binding_comes_from_the_persisted_projection() {
    let mut h = TestHarness::new();
    let projection = crate::InputProjection::Delivery(crate::DeliveryProjection {
        item: ConversationItem::user("strict input"),
        binding: crate::DeliveryBinding {
            delivery_id: "delivery".into(), payload_digest: "digest".into(),
            turn_id: "prompt".into(), parent_task: crate::ParentTask::Unknown,
        },
    });
    let ack = h.handle.append_input_with_source_and_ack(projection, crate::InputAdmission {
        parent_task: crate::ParentTask::Instance { id: "owner".into() },
        cancellation: tokio_util::sync::CancellationToken::new(),
    }).await.unwrap();
    assert!(matches!(ack.acknowledgement, crate::CompletionProjectionAck::Appended));
    let crate::AppendBinding::Applied(applied) = ack.binding else { panic!("applied append"); };
    let records = h.drain_persistence();
    let persisted = records.iter().find_map(|record| match record {
        PersistenceRecord::Input(projection) => Some(projection.item()),
        _ => None,
    }).unwrap();
    assert_eq!(applied.source, persisted.source());
    assert_eq!(h.handle.get_conversation().await.first().unwrap().source(), applied.source);
}

#[tokio::test]
async fn cancelled_strict_append_has_no_applied_binding_or_history() {
    let h = TestHarness::new();
    let cancellation = tokio_util::sync::CancellationToken::new();
    cancellation.cancel();
    let result = h.handle.append_input_with_source_and_ack(
        crate::InputProjection::Delivery(crate::DeliveryProjection {
            item: ConversationItem::user("cancelled"),
            binding: crate::DeliveryBinding {
                delivery_id: "cancelled".into(), payload_digest: "digest".into(),
                turn_id: "prompt".into(), parent_task: crate::ParentTask::Unknown,
            },
        }),
        crate::InputAdmission { parent_task: crate::ParentTask::Unknown, cancellation },
    ).await;
    assert!(matches!(result, Err(crate::StrictAppendError::NotCommitted(_))));
    assert!(h.handle.get_conversation().await.is_empty());
}

#[tokio::test]
async fn identical_occurrences_survive_persistence_restore_compaction_and_retry_builds() {
    let mut h = TestHarness::new();
    h.handle.push_user_message(ConversationItem::user("question"));
    h.handle.push_model_output(ConversationItem::assistant("same response"));
    h.handle.push_model_output(ConversationItem::assistant("same response"));
    h.handle.push_model_output(ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
        id: "tail-call".into(), name: "read_file".into(), arguments: "{}".into(),
    }]));
    h.handle.push_tool_result(ConversationItem::tool_result("tail-call", "original result"));
    let original = h.handle.get_conversation().await;
    let first = original.get(1).unwrap().source();
    let second = original.get(2).unwrap().source();
    let tool_result = original.get(4).unwrap().source();
    assert_ne!(first, second);
    assert!(matches!(first, SourceOccurrence::Admitted { .. }));
    let persisted: Vec<_> = h.drain_persistence().into_iter().filter_map(|record| match record {
        PersistenceRecord::Message(item) => Some(item),
        _ => None,
    }).collect();
    assert_eq!(persisted.iter().map(ConversationItem::source).collect::<Vec<_>>(),
        original.iter().map(ConversationItem::source).collect::<Vec<_>>());
    let restored: Vec<ConversationItem> = serde_json::from_slice(&serde_json::to_vec(&persisted).unwrap()).unwrap();
    let h = TestHarness::with_conversation(restored);
    let snapshot = h.handle.snapshot().await.unwrap();
    let state_context = crate::compaction_utils::CompactionStateContext::build(
        &snapshot.conversation, crate::compaction_utils::CompactionInputs::default(),
    ).await;
    let compacted = crate::compaction_utils::build_compacted_history(crate::compaction_utils::CompactedHistoryInput {
        system_message: ConversationItem::system("system"),
        user_message_prefix: "prefix".into(),
        agents_md_reminder: None,
        state_context: &state_context,
        compaction_summary: "summary".into(),
        system_reminder: None,
        summary_before_recent: false,
        transcript_hint: None,
        summary_count: 1,
    });
    h.handle.replace_conversation_for_compaction(compacted);
    for req_id in ["first-attempt", "retry"] {
        let built = h.handle.build_request(vec![], None, true, None, "conv".into(), req_id.into()).await.unwrap();
        let sources: Vec<_> = built.projected_membership().into_iter().map(|member| member.source).collect();
        assert!(sources.contains(&first));
        assert!(sources.contains(&second));
        assert!(sources.contains(&tool_result));
        let transformed_result = built.items.iter().find(|item| item.source() == tool_result).unwrap();
        assert_eq!(transformed_result.text_content(), "Tool call omitted...");
        assert!(transformed_result.evidence().transforms().contains(&xai_grok_sampling_types::ContentTransform::CompactionToolResultOmitted));
        assert_eq!(sources.iter().filter(|source| **source == first).count(), 1);
        assert_eq!(sources.iter().filter(|source| **source == second).count(), 1);
    }
    h.handle.restore_snapshot(snapshot);
    assert_eq!(h.handle.get_conversation().await.get(1).unwrap().source(), first);
}

#[tokio::test]
async fn deliberate_identical_reminders_have_distinct_injected_sources() {
    let h = TestHarness::new();
    for _ in 0..2 {
        h.handle.push_user_message(ConversationItem::system_reminder("identical reminder"));
    }
    let built = h.handle.build_request(vec![], None, true, None, "conv".into(), "request".into()).await.unwrap();
    let sources: Vec<_> = built.injected_records.iter().filter_map(|record| match record {
        crate::InjectedRequestRecord::Synthetic { text, source, .. } if text == "identical reminder" => Some(*source),
        _ => None,
    }).collect();
    assert_eq!(sources.len(), 2);
    assert_ne!(sources[0], sources[1]);
    let restored = TestHarness::with_conversation(serde_json::from_value(serde_json::to_value(h.handle.get_conversation().await).unwrap()).unwrap());
    assert_eq!(restored.handle.get_conversation().await.iter().map(ConversationItem::source).collect::<Vec<_>>(), sources);
}

#[tokio::test]
async fn projected_membership_excludes_policy_content_and_preview_does_not_admit() {
    let mut h = TestHarness::new();
    h.handle.push_user_message(ConversationItem::system("system"));
    h.handle.push_user_message(ConversationItem::system_reminder("excluded"));
    h.handle.push_user_message(ConversationItem::user("current"));
    let original = h.handle.get_conversation().await;
    let excluded = original.get(1).unwrap().source();
    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::RuntimeNotices(xai_grok_sampling_types::RuntimeNotice::SystemReminder), ContextSwitch::Excluded);
    h.handle.set_context_policy(policy).await.unwrap();
    h.drain_persistence();
    let before = serde_json::to_value(&original).unwrap();
    let preview = h.handle.preview_request(vec![], Some("preview-only".into()), "conv".into(), "preview".into()).await.unwrap();
    assert!(!preview.projected_membership().iter().any(|member| member.source == excluded));
    assert_eq!(serde_json::to_value(h.handle.get_conversation().await).unwrap(), before);
    assert!(h.drain_persistence().is_empty());
}

#[tokio::test]
async fn legacy_normalization_is_explicit_and_persists_without_historical_membership() {
    let legacy: Vec<ConversationItem> = serde_json::from_str(r#"[{"type":"user","content":[{"type":"text","text":"old"}]}]"#).unwrap();
    let (mock, mut persistence) = MockChatPersistence::new();
    let (events, _rx) = mpsc::unbounded_channel();
    let handle = ChatStateActor::spawn(legacy, test_config(), Box::new(mock), events, tokio_util::sync::CancellationToken::new());
    let loaded = handle.get_conversation().await;
    assert!(matches!(loaded.first().unwrap().source(), SourceOccurrence::LegacyNormalized { .. }));
    assert!(persistence.drain().is_empty());
    handle.preview_request(vec![], None, "conv".into(), "preview".into()).await.unwrap();
    assert!(persistence.drain().is_empty());
    assert_eq!(handle.get_conversation().await.first().unwrap().source(), loaded.first().unwrap().source());
    handle.build_request(vec![], None, false, None, "conv".into(), "actual".into()).await.unwrap();
    let records = persistence.drain();
    let persisted = records.iter().find_map(|record| match record {
        PersistenceRecord::ReplaceHistory(items) => Some(items),
        _ => None,
    }).unwrap();
    let restored = TestHarness::with_conversation(serde_json::from_value(serde_json::to_value(persisted).unwrap()).unwrap());
    assert_eq!(restored.handle.get_conversation().await.first().unwrap().source(), loaded.first().unwrap().source());
}


#[tokio::test]
async fn partial_policy_and_agent_label_are_independent_preview_facts() {
    use xai_grok_sampling_types::{ContentEvidence, ContentPart, ContentTransform, Sections};
    let mut agent = ConversationItem::agent_message("agent body");
    if let ConversationItem::User(user) = &mut agent {
        user.content.push(ContentPart::Text { text: "private rules".into() });
        user.sections = Sections::Split(vec![ContextCategory::OtherRuntime, ContextCategory::ProjectInstructions]);
    }
    let mut h = TestHarness::with_conversation(vec![agent, ConversationItem::user("current")]);
    let before = h.handle.get_conversation().await;
    let source = before[0].source();
    let mut policy = ContextPolicy::default();
    policy.set(ContextCategory::ProjectInstructions, ContextSwitch::Excluded);
    h.handle.set_context_policy(policy).await.unwrap();
    h.drain_persistence();
    for _ in 0..2 {
        let built = h.handle.preview_request(vec![], None, "c".into(), "p".into()).await.unwrap();
        let projected = built.items.iter().find(|item| item.source() == source).unwrap();
        assert!(projected.text_content().contains("agent body"));
        assert!(!projected.text_content().contains("private rules"));
        assert!(projected.evidence().transforms().contains(&ContentTransform::PolicyParts { kept: 1, total: 2 }));
        assert!(projected.evidence().transforms().contains(&ContentTransform::AgentLabelPrepended));
        assert_eq!(built.items.last().unwrap().evidence(), &ContentEvidence::Unknown);
    }
    assert_eq!(serde_json::to_value(h.handle.get_conversation().await).unwrap(), serde_json::to_value(before).unwrap());
    assert!(h.drain_persistence().is_empty());
}

#[tokio::test]
async fn canonical_head_rewrite_survives_build_snapshot_and_disk_restore() {
    use xai_grok_sampling_types::ContentTransform;
    let mut h = TestHarness::with_conversation(vec![ConversationItem::system("old head"), ConversationItem::user("current")]);
    let source = h.handle.get_conversation().await[0].source();
    assert_eq!(h.handle.replace_system_head("new head").await, Some(true));
    let snapshot = h.handle.snapshot().await.unwrap();
    let persisted = h.drain_persistence().into_iter().find_map(|record| match record {
        PersistenceRecord::ReplaceHistory(items) => Some(items),
        _ => None,
    }).unwrap();
    let restored: Vec<ConversationItem> = serde_json::from_value(serde_json::to_value(persisted).unwrap()).unwrap();
    for handle in [h.handle.clone(), TestHarness::with_conversation(restored).handle] {
        for _ in 0..2 {
            let built = handle.build_request(vec![], None, false, None, "c".into(), "r".into()).await.unwrap();
            let head = &built.items[0];
            assert_eq!(head.source(), source);
            assert_eq!(head.text_content(), "new head");
            assert!(head.evidence().transforms().contains(&ContentTransform::SystemHeadRewritten));
        }
        handle.restore_snapshot(snapshot.clone());
        let built = handle.preview_request(vec![], None, "c".into(), "p".into()).await.unwrap();
        assert!(built.items[0].evidence().transforms().contains(&ContentTransform::SystemHeadRewritten));
    }
}

#[tokio::test]
async fn summary_replacement_sets_exclude_retained_tail_and_survive_transitive_compaction() {
    use std::collections::HashSet;
    use xai_grok_sampling_types::{CompactionCoverage, ContentTransform};
    let mut h = TestHarness::new();
    h.handle.push_user_message(ConversationItem::user("old"));
    h.handle.push_model_output(ConversationItem::assistant("identical"));
    h.handle.push_model_output(ConversationItem::assistant("identical"));
    h.handle.push_user_message(ConversationItem::user("recent"));
    h.handle.push_model_output(ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
        id: "tail".into(), name: "read_file".into(), arguments: "{}".into(),
    }]));
    h.handle.push_tool_result(ConversationItem::tool_result("tail", "full result"));
    let original = h.handle.get_conversation().await;
    let tail = original.last().unwrap().source();
    let mut prior_coverage = HashSet::new();
    for round in 0..2 {
        let before = h.handle.get_conversation().await;
        let state = crate::compaction_utils::CompactionStateContext::build(&before, crate::compaction_utils::CompactionInputs::default()).await;
        let compacted = crate::compaction_utils::build_compacted_history(crate::compaction_utils::CompactedHistoryInput {
            system_message: ConversationItem::system("head"), user_message_prefix: "prefix".into(),
            agents_md_reminder: None, state_context: &state, compaction_summary: format!("summary {round}"),
            system_reminder: None, summary_before_recent: false, transcript_hint: None, summary_count: round + 1,
        });
        let retained: HashSet<_> = compacted.iter().filter_map(|item| item.source().id()).collect();
        let replaced: HashSet<_> = before.iter().filter_map(|item| item.source().id()).filter(|id| !retained.contains(id)).collect();
        h.handle.replace_conversation_for_compaction(compacted);
        let after = h.handle.get_conversation().await;
        let coverage = after.iter().find_map(|item| match item.evidence().compaction() {
            CompactionCoverage::Replaced { sources, inherited, unidentified, .. } => Some((sources, inherited, unidentified)),
            CompactionCoverage::Unknown | CompactionCoverage::AwaitingReplacement => None,
        }).unwrap();
        assert_eq!(coverage.0.iter().copied().collect::<HashSet<_>>(), replaced);
        assert_eq!(*coverage.2, 0);
        assert!(!coverage.0.contains(&tail.id().unwrap()));
        assert!(prior_coverage.iter().all(|id| coverage.1.contains(id)));
        prior_coverage.extend(coverage.0.iter().copied());
        prior_coverage.extend(coverage.1.iter().copied());
        let retained_tail = after.iter().find(|item| item.source() == tail).unwrap();
        assert_eq!(retained_tail.text_content(), "Tool call omitted...");
        assert!(retained_tail.evidence().transforms().contains(&ContentTransform::CompactionToolResultOmitted));
        let restored: Vec<ConversationItem> = serde_json::from_value(serde_json::to_value(&after).unwrap()).unwrap();
        h = TestHarness::with_conversation(restored);
        for _ in 0..2 {
            let built = h.handle.build_request(vec![], None, false, None, "c".into(), "r".into()).await.unwrap();
            assert_eq!(built.items.iter().find(|item| item.source() == tail).unwrap().evidence(), retained_tail.evidence());
        }
    }
}

#[tokio::test]
async fn missing_legacy_body_proof_stays_unknown_after_normalization() {
    use xai_grok_sampling_types::ContentEvidence;
    let items: Vec<ConversationItem> = serde_json::from_str(r#"[{"type":"user","content":[{"type":"text","text":"legacy"}]}]"#).unwrap();
    let h = TestHarness::with_conversation(items);
    let built = h.handle.build_request(vec![], None, false, None, "c".into(), "r".into()).await.unwrap();
    assert_eq!(built.projected_membership()[0].evidence, ContentEvidence::Unknown);
}


#[tokio::test]
async fn hard_and_soft_request_pruning_preserve_occurrence_not_body() {
    use xai_grok_sampling_types::ContentTransform;
    for (turns, expected) in [(5, ContentTransform::ToolResultSoftTrimmed), (12, ContentTransform::ToolResultHardCleared)] {
        let mut history = vec![ConversationItem::user("old"), ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
            id: "call".into(), name: "read_file".into(), arguments: "{}".into(),
        }]), ConversationItem::tool_result("call", "original body ".repeat(600))];
        history.extend((0..turns).map(|_| ConversationItem::user("next")));
        let h = TestHarness::with_config(history, test_config_with_window(1000));
        let canonical = h.handle.get_conversation().await;
        for _ in 0..2 {
            let projected = h.handle.apply_turn_request_pruning(canonical.clone()).await;
            assert_eq!(projected[2].source(), canonical[2].source());
            assert_ne!(projected[2].text_content(), canonical[2].text_content());
            assert!(projected[2].evidence().transforms().contains(&expected));
        }
        assert_eq!(serde_json::to_value(h.handle.get_conversation().await).unwrap(), serde_json::to_value(canonical).unwrap());
    }
}

#[tokio::test]
async fn local_evidence_changes_neither_provider_bodies_nor_image_or_token_budget() {
    use xai_grok_sampling_types::{ContentPart, ContentTransform, CompactionCoverage, CoverageKnowledge, SourceOccurrenceId};
    let plain = vec![ConversationItem::system("head"), ConversationItem::user_with_parts(vec![
        ContentPart::Text { text: "question".into() },
        ContentPart::Image { url: format!("data:image/png;base64,{}", "A".repeat(5000)).into() },
    ]), ConversationItem::Reasoning(xai_grok_sampling_types::rs::ReasoningItem {
        id: "reasoning".into(), summary: vec![], content: None,
        encrypted_content: Some("opaque".into()), status: None,
    }.into()), ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
        id: "call".into(), name: "read_file".into(), arguments: "{}".into(),
    }]), ConversationItem::tool_result("call", "result"),
    ConversationItem::BackendToolCall(xai_grok_sampling_types::BackendToolCallItem {
        source: Default::default(), evidence: Default::default(),
        kind: xai_grok_sampling_types::BackendToolKind::WebSearch(xai_grok_sampling_types::rs::WebSearchToolCall {
            id: "search".into(), status: xai_grok_sampling_types::rs::WebSearchToolCallStatus::Completed,
            action: xai_grok_sampling_types::rs::WebSearchToolCallAction::Search(xai_grok_sampling_types::rs::WebSearchActionSearch {
                query: "query".into(), sources: None,
            }),
        }),
    })];
    let mut annotated = plain.clone();
    for item in &mut annotated {
        item.evidence_mut().record(ContentTransform::SystemHeadRewritten);
        item.evidence_mut().set_compaction(CompactionCoverage::Replaced {
            sources: vec![SourceOccurrenceId([1; 16])], inherited: vec![SourceOccurrenceId([2; 16])],
            unidentified: 0, ancestry: CoverageKnowledge::Complete,
        });
    }
    assert_eq!(super::super::state::estimate_conversation_tokens(&plain), super::super::state::estimate_conversation_tokens(&annotated));
    for (trigger, target) in [(usize::MAX, usize::MAX), (1, 0)] {
        let left = crate::image_budget::apply_image_budget_with_limits(plain.clone(), trigger, target);
        let right = crate::image_budget::apply_image_budget_with_limits(annotated.clone(), trigger, target);
        assert_eq!(left.outcome, right.outcome);
        let left = xai_grok_sampling_types::ConversationRequest::from_items(left.items);
        let right = xai_grok_sampling_types::ConversationRequest::from_items(right.items);
        let chat_left: xai_grok_sampling_types::ChatCompletionRequest = left.clone().into();
        let chat_right: xai_grok_sampling_types::ChatCompletionRequest = right.clone().into();
        assert_eq!(serde_json::to_vec(&chat_left).unwrap(), serde_json::to_vec(&chat_right).unwrap());
        let responses_left: xai_grok_sampling_types::rs::CreateResponse = (&left).into();
        let responses_right: xai_grok_sampling_types::rs::CreateResponse = (&right).into();
        assert_eq!(serde_json::to_vec(&responses_left).unwrap(), serde_json::to_vec(&responses_right).unwrap());
        assert_eq!(serde_json::to_vec(&xai_grok_sampling_types::build_messages_request(&left)).unwrap(),
            serde_json::to_vec(&xai_grok_sampling_types::build_messages_request(&right)).unwrap());
        if trigger == 1 {
            assert!(right.items[1].evidence().transforms().contains(&ContentTransform::ImagesEvicted { kept: 0, total: 1 }));
            assert!(right.items[1].evidence().transforms().contains(&ContentTransform::SystemHeadRewritten));
        }
    }
}

#[tokio::test]
async fn output_acknowledgements_forward_the_actual_admission_and_keep_accounting_owners() {
    let mut h = TestHarness::new();
    let first = h.handle.push_assistant_response_and_ack(ConversationItem::assistant_tool_calls(vec![xai_grok_sampling_types::ToolCall {
        id: "actual-call".into(), name: "read_file".into(), arguments: "{}".into(),
    }])).await.unwrap();
    let result = h.handle.push_tool_result_and_ack(ConversationItem::tool_result("actual-call", "same output")).await.unwrap();
    let second = h.handle.push_model_output_and_ack(ConversationItem::assistant("same output")).await.unwrap();
    let third = h.handle.push_unreported_model_output_and_ack(ConversationItem::assistant("same output")).await.unwrap();
    let expected = vec![first.source, result.source, second.source, third.source];
    let history = h.handle.get_conversation().await;
    assert_eq!(history.iter().map(ConversationItem::source).collect::<Vec<_>>(), expected);
    assert_ne!(second.source, third.source);
    let persisted: Vec<_> = h.drain_persistence().into_iter().filter_map(|record| match record {
        PersistenceRecord::Message(item) => Some(item.source()),
        _ => None,
    }).collect();
    assert_eq!(persisted, expected);
    assert_eq!(h.handle.get_conversation().await.len(), 4);
    assert!(h.drain_persistence().is_empty());
}

#[tokio::test]
async fn output_ack_transport_keeps_mailbox_closed_distinct_from_reply_lost() {
    assert!(matches!(crate::ChatStateHandle::noop().push_model_output_and_ack(ConversationItem::assistant("not admitted")).await,
        Err(crate::MutationAdmissionError::MailboxClosed(_))));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = crate::ChatStateHandle::new(tx);
    let request = tokio::spawn(async move { handle.push_model_output_and_ack(ConversationItem::assistant("may have applied")).await });
    let command = rx.recv().await.unwrap();
    let crate::commands::ChatStateCommand::PushModelOutput { reply: crate::MutationReply::Observed(reply), .. } = command else { panic!("observed output command"); };
    drop(reply);
    assert!(matches!(request.await.unwrap(), Err(crate::MutationAdmissionError::ReplyLost(_))));
}
