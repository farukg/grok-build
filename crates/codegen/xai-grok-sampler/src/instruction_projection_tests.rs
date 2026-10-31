use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use xai_grok_sampling_types::instruction_projection::{
    InstructionSelection, ProviderInstructionPlan, RequestCapability,
};
use xai_grok_sampling_types::{
    ApiBackend, ContentPart, ContentTransform, ContextCategory, ConversationItem,
    ConversationRequest, ConversationToolChoice, HostedTool, Sections, SourceOccurrence,
    SourceOccurrenceId, SyntheticReason, ToolCall, ToolSpec, UserItem, WebSearchOptions,
};

use crate::client::ClientDefaults;
use crate::instruction_projection::*;
use crate::request_body::{self, BodyMode};
use crate::request_capture::{CaptureAttempt, LocalRequestBody, LocalRequestEvidence, RequestAttemptId, RequestBodyCache};
use crate::{RequestId, SamplerConfig, SamplingEvent};

fn fixture() -> ConversationRequest {
    let synthetic = |text: &str, synthetic_reason| ConversationItem::User(UserItem {
        content: vec![ContentPart::Text { text: text.into() }], synthetic_reason, ..Default::default()
    });
    let mut items = vec![
        ConversationItem::system("core α"),
        ConversationItem::user("core α"),
        ConversationItem::assistant_tool_calls(vec![
            ToolCall { id: "a".into(), name: "Lookup".into(), arguments: "{}".into() },
            ToolCall { id: "b".into(), name: "Lookup".into(), arguments: "{}".into() },
        ]),
        ConversationItem::tool_result("a", "first"),
        ConversationItem::tool_result("b", "second"),
        synthetic("project", SyntheticReason::ProjectInstructions),
        ConversationItem::session_prefix("environment"),
        synthetic("skills", SyntheticReason::SkillCatalog),
        synthetic("mcp", SyntheticReason::McpCatalog),
        ConversationItem::User(UserItem {
            content: vec![
                ContentPart::Text { text: "ordinary".into() },
                ContentPart::Text { text: "mixed β".into() },
                ContentPart::Text { text: "unclassified project instructions tools".into() },
            ],
            sections: Sections::Split(vec![ContextCategory::UserTurns, ContextCategory::ProjectInstructions]),
            ..Default::default()
        }),
        ConversationItem::system("tail core"),
        ConversationItem::user("ordinary skills mcp system developer"),
    ];
    for (index, item) in items.iter_mut().enumerate() {
        *item.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([u8::try_from(index).unwrap(); 16]) };
    }
    items.get_mut(5).unwrap().evidence_mut().record(ContentTransform::PolicyParts { kept: 1, total: 2 });
    ConversationRequest {
        model: Some("fixture-model".into()), items,
        tools: vec![
            ToolSpec { name: "Lookup".into(), description: Some("lookup schema".into()), parameters: json!({"type":"object","properties":{"q":{"type":"string"}}}) },
            ToolSpec { name: "web_search".into(), description: None, parameters: json!({"type":"object"}) },
        ],
        hosted_tools: vec![
            HostedTool::WebSearch { options: Some(WebSearchOptions { allowed_domains: None, excluded_domains: Some(vec!["example.invalid".into()]) }) },
            HostedTool::XSearch { options: None },
        ],
        tool_choice: Some(ConversationToolChoice::Required),
        json_schema: Some(json!({"type":"object","properties":{"answer":{"type":"string"}}})),
        ..Default::default()
    }
}

fn serialize(backend: ApiBackend, request: ConversationRequest, mode: BodyMode) -> (Vec<u8>, ProviderInstructionPlan) {
    let defaults = ClientDefaults::from(&SamplerConfig { model: "fixture-model".into(), api_backend: backend.clone(), stream_tool_calls: true, ..Default::default() });
    match backend {
        ApiBackend::ChatCompletions => {
            let (payload, plan) = xai_grok_sampling_types::chat_request_with_instructions(request);
            (request_body::chat_body(&payload, mode).unwrap(), plan)
        }
        ApiBackend::Responses => {
            let (mut payload, plan) = request_body::responses_request(request, mode);
            request_body::apply_response_defaults(&defaults, &mut payload).unwrap();
            payload.inner.stream = match mode { BodyMode::Streaming => Some(true), BodyMode::NonStreaming => None };
            (request_body::responses_body(&defaults, &mut payload, mode).unwrap(), plan)
        }
        ApiBackend::Messages => {
            let (mut payload, plan) = request_body::messages_request(request, mode);
            request_body::apply_message_defaults(&defaults, &mut payload).unwrap();
            payload.inner.stream = match mode { BodyMode::Streaming => Some(true), BodyMode::NonStreaming => None };
            (request_body::json_body(&payload.inner).unwrap(), plan)
        }
    }
}

fn capture(backend: ApiBackend, bytes: &[u8], plan: Option<&ProviderInstructionPlan>) -> Arc<LocalRequestBody> {
    let (events, mut rx) = mpsc::unbounded_channel();
    CaptureAttempt {
        request_id: RequestId::from("projection-fixture"), attempt: RequestAttemptId(7), backend,
        membership: Vec::new().into(), cache: Arc::new(RequestBodyCache::default()), events,
    }.prepared(bytes, plan);
    let SamplingEvent::LocalRequest { request_id, attempt, evidence: LocalRequestEvidence::Prepared { body, .. } } = rx.try_recv().unwrap() else { panic!("Prepared evidence"); };
    assert_eq!(request_id.as_str(), "projection-fixture");
    assert_eq!(attempt, RequestAttemptId(7));
    assert!(rx.try_recv().is_err());
    body
}

fn complete(body: &LocalRequestBody) -> InstructionPublicationView {
    let InstructionPublication::Prepared { binding, evidence: InstructionProjectionEvidence::Complete { projection } } = body.instruction_publication().unwrap() else { panic!("complete prepared projection"); };
    assert_eq!(binding.request_id.as_str(), "projection-fixture");
    assert_eq!(binding.attempt, RequestAttemptId(7));
    assert_eq!(binding.body_digest, *body.digest().as_bytes());
    projection
}

fn text(value: &Value) -> &str {
    value.as_str().or_else(|| value.get("text").and_then(Value::as_str)).unwrap()
}

#[test]
fn golden_final_fields_roles_order_sources_and_capabilities_for_all_backends() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        for mode in [BodyMode::NonStreaming, BodyMode::Streaming] {
            let (bytes, plan) = serialize(backend.clone(), fixture(), mode);
            let before: Value = serde_json::from_slice(&bytes).unwrap();
            let body = capture(backend.clone(), &bytes, Some(&plan));
            let view = complete(&body);
            assert_eq!(body.sensitive_local_bytes(), bytes);
            assert_eq!(serde_json::from_slice::<Value>(body.sensitive_local_bytes()).unwrap(), before);
            assert_eq!(view.representation, InstructionRepresentation::ExactLocalValues);
            let instructions: Vec<_> = view.fragments.iter().filter_map(|fragment| match &fragment.source.origin {
                PreparedInstructionOrigin::Conversation { role, provenance } => Some((
                    fragment.source.reference.path.json_pointer(), *role, provenance.category,
                    provenance.source, text(&fragment.value).to_owned(),
                )),
                PreparedInstructionOrigin::ProviderRequest { .. } => None,
            }).collect();
            let user = ProviderInstructionRole::User;
            let system = ProviderInstructionRole::System;
            let (pointers, source_order) = match backend {
                ApiBackend::ChatCompletions => (vec!["/messages/0/content", "/messages/5/content", "/messages/6/content", "/messages/7/content", "/messages/8/content", "/messages/9/content", "/messages/10/content"], vec![0, 5, 6, 7, 8, 9, 10]),
                ApiBackend::Responses => (vec!["/input/0/content", "/input/6/content", "/input/7/content", "/input/8/content", "/input/9/content", "/input/10/content/1", "/input/11/content"], vec![0, 5, 6, 7, 8, 9, 10]),
                ApiBackend::Messages => (vec!["/system/0", "/system/1", "/messages/3/content/0", "/messages/4/content/0", "/messages/5/content/0", "/messages/6/content/0", "/messages/7/content/1"], vec![0, 10, 5, 6, 7, 8, 9]),
            };
            let expected: Vec<_> = pointers.into_iter().zip(source_order).map(|(pointer, source)| {
                let (role, category, value) = match source {
                    0 => (system, ContextCategory::CoreInstructions, "core α"),
                    5 => (user, ContextCategory::ProjectInstructions, "project"),
                    6 => (user, ContextCategory::Environment, "environment"),
                    7 => (user, ContextCategory::SkillsWorkflows, "skills"),
                    8 => (user, ContextCategory::McpCatalog, "mcp"),
                    9 => (user, ContextCategory::ProjectInstructions, "mixed β"),
                    10 => (system, ContextCategory::CoreInstructions, "tail core"),
                    _ => panic!("golden source"),
                };
                (pointer.to_owned(), role, category, SourceOccurrence::Admitted { id: SourceOccurrenceId([source; 16]) }, value.to_owned())
            }).collect();
            assert_eq!(instructions, expected);
            let capabilities: Vec<_> = view.fragments.iter().filter(|fragment| matches!(fragment.source.origin, PreparedInstructionOrigin::ProviderRequest { .. })).collect();
            let tool_count = match backend { ApiBackend::Responses => 3, ApiBackend::ChatCompletions | ApiBackend::Messages => 2 };
            assert_eq!(capabilities.len(), tool_count + 2);
            assert_eq!(view.fragments.len(), 7 + tool_count + 2);
            for fragment in &capabilities {
                assert_eq!(Some(&fragment.value), before.pointer(&fragment.source.reference.path.json_pointer()));
            }
            let native_pointer = match backend {
                ApiBackend::ChatCompletions => "/tools/0/function",
                ApiBackend::Responses | ApiBackend::Messages => "/tools/0",
            };
            assert_eq!(before.pointer(&format!("{native_pointer}/name")), Some(&json!("Lookup")));
            let schema_field = match backend { ApiBackend::Messages => "input_schema", ApiBackend::ChatCompletions | ApiBackend::Responses => "parameters" };
            assert_eq!(before.pointer(&format!("{native_pointer}/{schema_field}")), Some(&json!({"type":"object","properties":{"q":{"type":"string"}}})));
            let (choice, format_pointer) = match backend {
                ApiBackend::ChatCompletions => (json!("required"), "/response_format/json_schema/schema"),
                ApiBackend::Responses => (json!("required"), "/text/format/schema"),
                ApiBackend::Messages => (json!({"type":"any"}), "/output_config/format/schema"),
            };
            assert_eq!(before.get("tool_choice"), Some(&choice));
            assert_eq!(before.pointer(format_pointer), Some(&json!({"type":"object","properties":{"answer":{"type":"string"}}})));
            match backend {
                ApiBackend::Responses => {
                    assert_eq!(before.pointer("/tools/1"), Some(&json!({"type":"web_search","filters":{"excluded_domains":["example.invalid"]}})));
                    assert_eq!(before.pointer("/tools/2"), Some(&json!({"type":"x_search"})));
                    assert_eq!(before.get("stream_tool_calls").and_then(Value::as_bool), match mode { BodyMode::Streaming => Some(true), BodyMode::NonStreaming => None });
                }
                ApiBackend::Messages => {
                    assert_eq!(before.pointer("/system/1/cache_control"), Some(&json!({"type":"ephemeral"})));
                    assert_eq!(before.pointer("/tools/1/name"), Some(&json!("web_search")));
                }
                ApiBackend::ChatCompletions => assert_eq!(before.pointer("/tools/1/function/name"), Some(&json!("web_search"))),
            }
            let project = view.fragments.iter().find_map(|fragment| match &fragment.source.origin {
                PreparedInstructionOrigin::Conversation { provenance, .. } if provenance.source == (SourceOccurrence::Admitted { id: SourceOccurrenceId([5; 16]) }) => Some(provenance),
                PreparedInstructionOrigin::Conversation { .. } | PreparedInstructionOrigin::ProviderRequest { .. } => None,
            }).unwrap();
            assert_eq!(project.evidence.transforms(), &[ContentTransform::PolicyParts { kept: 1, total: 2 }]);
        }
    }
}

#[test]
fn final_role_and_patched_capabilities_not_internal_role_or_inventory_are_authoritative() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses] {
        let (bytes, plan) = serialize(backend.clone(), fixture(), BodyMode::Streaming);
        let mut final_body: Value = serde_json::from_slice(&bytes).unwrap();
        let role_pointer = match backend { ApiBackend::ChatCompletions => "/messages/0/role", ApiBackend::Responses => "/input/0/role", ApiBackend::Messages => unreachable!() };
        *final_body.pointer_mut(role_pointer).unwrap() = json!("developer");
        *final_body.get_mut("tools").unwrap() = match backend {
            ApiBackend::ChatCompletions => json!([{"type":"function","function":{"name":"FinalOnly","parameters":{"type":"object"}}}]),
            ApiBackend::Responses => json!([{"type":"web_search","filters":{"allowed_domains":["final.invalid"]}}]),
            ApiBackend::Messages => unreachable!(),
        };
        let bytes = serde_json::to_vec(&final_body).unwrap();
        let view = complete(&capture(backend, &bytes, Some(&plan)));
        assert!(matches!(view.fragments.first().unwrap().source.origin, PreparedInstructionOrigin::Conversation { role: ProviderInstructionRole::Developer, .. }));
        let tools: Vec<_> = view.fragments.iter().filter(|fragment| matches!(fragment.source.origin, PreparedInstructionOrigin::ProviderRequest { capability: RequestCapability::Tool })).collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(&tools.first().unwrap().value, final_body.pointer("/tools/0").unwrap());
    }
}

#[test]
fn source_annotations_do_not_change_provider_bytes_or_typed_json() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        let annotated = fixture();
        let mut plain = annotated.clone();
        for item in &mut plain.items {
            *item.source_mut() = SourceOccurrence::Unadmitted;
            *item.evidence_mut() = Default::default();
        }
        for mode in [BodyMode::Streaming, BodyMode::NonStreaming] {
            let (annotated_bytes, _) = serialize(backend.clone(), annotated.clone(), mode);
            let (plain_bytes, _) = serialize(backend.clone(), plain.clone(), mode);
            assert_eq!(annotated_bytes, plain_bytes);
            assert_eq!(serde_json::from_slice::<Value>(&annotated_bytes).unwrap(), serde_json::from_slice::<Value>(&plain_bytes).unwrap());
        }
    }
}

#[test]
fn malformed_instruction_fields_are_partial_for_each_backend() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        let (bytes, plan) = serialize(backend.clone(), fixture(), BodyMode::Streaming);
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        let reference = &plan.instructions.first().unwrap().reference;
        *value.pointer_mut(&reference.path.json_pointer()).unwrap() = Value::Null;
        let body = capture(backend, &serde_json::to_vec(&value).unwrap(), Some(&plan));
        let InstructionPublication::Prepared { evidence: InstructionProjectionEvidence::Partial { projection, issues }, .. } = body.instruction_publication().unwrap() else { panic!("malformed source fragment is not complete"); };
        assert!(issues.iter().any(|issue| matches!(issue, InstructionProjectionIssue::InvalidReference { .. })));
        assert_eq!(projection.fragments.iter().filter(|fragment| matches!(fragment.source.origin, PreparedInstructionOrigin::Conversation { .. })).count(), 6);
    }
}

#[test]
fn missing_malformed_and_mismatched_evidence_remains_typed() {
    let backend = ApiBackend::ChatCompletions;
    for (bytes, reason) in [(b"not json".as_slice(), InstructionProjectionUnavailable::MalformedJson), (b"[]".as_slice(), InstructionProjectionUnavailable::InvalidRoot)] {
        let body = capture(backend.clone(), bytes, None);
        assert!(matches!(body.instructions(), CapturedInstructions::Prepared { evidence: InstructionProjectionEvidence::Unavailable { reason: actual }, .. } if *actual == reason));
        assert!(matches!(body.instruction_publication().unwrap(), InstructionPublication::Prepared { evidence: InstructionProjectionEvidence::Unavailable { .. }, .. }));
    }
    let (bytes, plan) = serialize(backend.clone(), fixture(), BodyMode::Streaming);
    let missing = capture(backend.clone(), &bytes, None);
    let CapturedInstructions::Prepared { evidence: InstructionProjectionEvidence::Partial { projection, issues }, .. } = missing.instructions() else { panic!("partial source evidence"); };
    assert!(issues.contains(&InstructionProjectionIssue::MissingConversionEvidence));
    assert!(issues.iter().any(|issue| matches!(issue, InstructionProjectionIssue::UnmappedInstruction { .. })));
    assert!(projection.iter().all(|fragment| matches!(fragment.origin, PreparedInstructionOrigin::ProviderRequest { .. })));
    let mut malformed = plan.clone();
    malformed.instructions.first_mut().unwrap().reference.selection = InstructionSelection::TextRange { start: 6, end: 7 };
    let malformed_body = capture(backend.clone(), &bytes, Some(&malformed));
    let CapturedInstructions::Prepared { evidence: InstructionProjectionEvidence::Partial { issues, .. }, .. } = malformed_body.instructions() else { panic!("invalid UTF-8 reference"); };
    assert!(issues.iter().any(|issue| matches!(issue, InstructionProjectionIssue::InvalidReference { .. })));
    malformed.backend = ApiBackend::Messages;
    assert!(matches!(capture(backend.clone(), &bytes, Some(&malformed)).instructions(), CapturedInstructions::Prepared { evidence: InstructionProjectionEvidence::Partial { issues, .. }, .. } if issues.contains(&InstructionProjectionIssue::BackendMismatch)));
    let body = capture(backend, br#"{"messages":[],"tools":[false],"tool_choice":3,"response_format":[]}"#, Some(&ProviderInstructionPlan::new(ApiBackend::ChatCompletions)));
    assert!(matches!(body.instructions(), CapturedInstructions::Prepared { evidence: InstructionProjectionEvidence::Partial { issues, .. }, .. } if issues.len() == 3));
    assert!(matches!(LocalRequestBody::from_serialized(bytes).instructions(), CapturedInstructions::Unavailable { reason: InstructionProjectionUnavailable::NotPrepared }));
}

#[test]
fn content_digest_is_not_occurrence_identity_and_capture_never_globally_deduplicates() {
    let (events, mut rx) = mpsc::unbounded_channel();
    let cache = Arc::new(RequestBodyCache::default());
    let mut digests = Vec::new();
    for (index, content) in ["A", "B", "A"].into_iter().enumerate() {
        let mut item = ConversationItem::system(content);
        *item.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([u8::try_from(index).unwrap(); 16]) };
        let (bytes, plan) = serialize(ApiBackend::ChatCompletions, ConversationRequest::from_items(vec![item]), BodyMode::Streaming);
        CaptureAttempt {
            request_id: RequestId::from(format!("transition-{index}")), attempt: RequestAttemptId(1), backend: ApiBackend::ChatCompletions,
            membership: Vec::new().into(), cache: Arc::clone(&cache), events: events.clone(),
        }.prepared(&bytes, Some(&plan));
        let SamplingEvent::LocalRequest { request_id, evidence: LocalRequestEvidence::Prepared { body, .. }, .. } = rx.try_recv().unwrap() else { panic!("every transition captured"); };
        let InstructionPublication::Prepared { binding, evidence: InstructionProjectionEvidence::Complete { projection } } = body.instruction_publication().unwrap() else { panic!("complete"); };
        assert_eq!(binding.request_id, request_id);
        digests.push(projection.content_digest);
    }
    assert_eq!(digests.first(), digests.last());
    assert_ne!(digests.first(), digests.get(1));
    assert!(rx.try_recv().is_err());
}

#[test]
fn hosted_chat_search_is_final_request_evidence_not_a_conversation_occurrence() {
    let bytes = br#"{"messages":[],"search_parameters":{"mode":"auto","sources":[{"type":"web"}]}}"#;
    let view = complete(&capture(ApiBackend::ChatCompletions, bytes,
        Some(&ProviderInstructionPlan::new(ApiBackend::ChatCompletions))));
    assert_eq!(view.fragments.len(), 1);
    let fragment = view.fragments.first().unwrap();
    assert_eq!(fragment.source.reference.path.json_pointer(), "/search_parameters");
    assert_eq!(fragment.value, json!({"mode":"auto","sources":[{"type":"web"}]}));
    assert_eq!(fragment.source.origin, PreparedInstructionOrigin::ProviderRequest { capability: RequestCapability::HostedSearch });
}

#[test]
fn retry_binding_changes_without_another_raw_byte_owner() {
    let (bytes, plan) = serialize(ApiBackend::ChatCompletions, fixture(), BodyMode::Streaming);
    let (events, mut rx) = mpsc::unbounded_channel();
    let cache = Arc::new(RequestBodyCache::default());
    let mut bodies = Vec::new();
    for attempt in [1, 2] {
        CaptureAttempt {
            request_id: RequestId::from("same-request"), attempt: RequestAttemptId(attempt), backend: ApiBackend::ChatCompletions,
            membership: Vec::new().into(), cache: Arc::clone(&cache), events: events.clone(),
        }.prepared(&bytes, Some(&plan));
        let SamplingEvent::LocalRequest { evidence: LocalRequestEvidence::Prepared { body, .. }, .. } = rx.try_recv().unwrap() else { panic!("prepared retry"); };
        assert!(matches!(body.instructions(), CapturedInstructions::Prepared { binding, .. } if binding.attempt == RequestAttemptId(attempt) && binding.request_id.as_str() == "same-request"));
        bodies.push(body);
    }
    assert_eq!(bodies.first().unwrap().sensitive_local_bytes().as_ptr(), bodies.get(1).unwrap().sensitive_local_bytes().as_ptr());
}

#[test]
fn reasoning_folds_and_image_parts_keep_their_real_converted_locations() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        let reasoning = xai_grok_sampling_types::rs::ReasoningItem {
            id: "reasoning".into(), summary: vec![], content: None,
            encrypted_content: None, status: None,
        };
        let mut instruction = ConversationItem::User(UserItem {
            synthetic_reason: SyntheticReason::ProjectInstructions,
            content: vec![
                ContentPart::Text { text: "image rules".into() },
                ContentPart::Image { url: "https://image.invalid/rules.png".into() },
            ],
            ..Default::default()
        });
        *instruction.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([42; 16]) };
        let request = ConversationRequest::from_items(vec![
            ConversationItem::Reasoning(reasoning.into()),
            ConversationItem::user("ordinary"), ConversationItem::assistant("answer"), instruction,
        ]);
        let (bytes, plan) = serialize(backend.clone(), request, BodyMode::Streaming);
        let view = complete(&capture(backend.clone(), &bytes, Some(&plan)));
        assert_eq!(view.fragments.len(), 2);
        let prefix = match backend {
            ApiBackend::ChatCompletions => "/messages/2/content",
            ApiBackend::Responses => "/input/3/content",
            ApiBackend::Messages => "/messages/2/content",
        };
        assert_eq!(view.fragments.first().unwrap().source.reference.path.json_pointer(), format!("{prefix}/0"));
        assert_eq!(view.fragments.get(1).unwrap().source.reference.path.json_pointer(), format!("{prefix}/1"));
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        for fragment in &view.fragments {
            assert_eq!(Some(&fragment.value), body.pointer(&fragment.source.reference.path.json_pointer()));
            assert!(matches!(&fragment.source.origin, PreparedInstructionOrigin::Conversation {
                role: ProviderInstructionRole::User,
                provenance: xai_grok_sampling_types::instruction_projection::ConversationInstructionSource {
                    source: SourceOccurrence::Admitted { id: SourceOccurrenceId(id) }, category: ContextCategory::ProjectInstructions, ..
                },
            } if *id == [42; 16]));
        }
        let image_pointer = match backend {
            ApiBackend::ChatCompletions => format!("{prefix}/1/image_url/url"),
            ApiBackend::Responses => format!("{prefix}/1/image_url"),
            ApiBackend::Messages => format!("{prefix}/1/source/url"),
        };
        assert_eq!(body.pointer(&image_pointer), Some(&json!("https://image.invalid/rules.png")));
    }
}

#[test]
fn missing_source_is_partial_but_never_replaced_with_a_minted_occurrence() {
    let (bytes, plan) = serialize(ApiBackend::Messages,
        ConversationRequest::from_items(vec![ConversationItem::system("unadmitted")]), BodyMode::Streaming);
    let body = capture(ApiBackend::Messages, &bytes, Some(&plan));
    let InstructionPublication::Prepared { evidence: InstructionProjectionEvidence::Partial { projection, issues }, .. } = body.instruction_publication().unwrap() else { panic!("missing source"); };
    assert_eq!(issues.len(), 1);
    assert!(matches!(issues.first().unwrap(), InstructionProjectionIssue::MissingSource { .. }));
    assert_eq!(projection.fragments.len(), 1);
    assert!(matches!(projection.fragments.first().unwrap().source.origin,
        PreparedInstructionOrigin::Conversation { provenance: xai_grok_sampling_types::instruction_projection::ConversationInstructionSource { source: SourceOccurrence::Unadmitted, .. }, .. }));
}

#[test]
fn ordinary_appends_source_ids_and_cache_breakpoint_moves_do_not_change_content_digest() {
    for backend in [ApiBackend::ChatCompletions, ApiBackend::Responses, ApiBackend::Messages] {
        let mut previous = None;
        for append in [false, true] {
            let mut instruction = ConversationItem::session_prefix("unchanged active environment");
            *instruction.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([u8::from(append); 16]) };
            let mut items = vec![instruction];
            if append {
                items.insert(0, ConversationItem::user("ordinary preceding conversation"));
                items.push(ConversationItem::user("ordinary appended conversation"));
            }
            let (bytes, plan) = serialize(backend.clone(), ConversationRequest::from_items(items), BodyMode::Streaming);
            let view = complete(&capture(backend.clone(), &bytes, Some(&plan)));
            if let Some(digest) = previous { assert_eq!(view.content_digest, digest); }
            previous = Some(view.content_digest);
        }
    }
}

#[test]
fn publication_redacts_selected_values_without_rewriting_provider_bytes() {
    let sensitive = ["sk-", &"q".repeat(32)].concat();
    let mut item = ConversationItem::system(sensitive.clone());
    *item.source_mut() = SourceOccurrence::Admitted { id: SourceOccurrenceId([1; 16]) };
    let (bytes, plan) = serialize(ApiBackend::ChatCompletions, ConversationRequest::from_items(vec![item]), BodyMode::Streaming);
    let body = capture(ApiBackend::ChatCompletions, &bytes, Some(&plan));
    let view = complete(&body);
    assert!(matches!(view.representation, InstructionRepresentation::Redacted { original_body_digest } if original_body_digest == *body.digest().as_bytes()));
    assert!(!serde_json::to_string(&view).unwrap().contains(&sensitive));
    assert!(!format!("{body:?} {view:?}").contains(&sensitive));
    assert_eq!(body.sensitive_local_bytes(), bytes);
}
