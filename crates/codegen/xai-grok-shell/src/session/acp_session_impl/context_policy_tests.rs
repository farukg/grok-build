use super::*;
use crate::session::acp_session::support::{
    create_test_actor, test_agent_backend_search, test_grok_build_agent_with_todo,
};
use xai_grok_sampling_types::{ContextCategory, ContextPolicy, ContextSwitch};

#[tokio::test(flavor = "current_thread")]
async fn context_policy_report_counts_function_schemas_outside_history() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (gateway, _events) = tokio::sync::mpsc::unbounded_channel();
            let (persistence, _persisted) = tokio::sync::mpsc::unbounded_channel();
            let actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
            *actor.agent.borrow_mut() = test_grok_build_agent_with_todo().await;
            let definitions = actor.prepare_tool_definitions_inner().await;
            assert!(!definitions.is_empty());
            let report = actor.context_policy_report().await.unwrap();
            let tool_tokens = |report: &crate::extensions::context_policy::ContextPolicyReport| {
                report
                    .usage
                    .iter()
                    .find(|usage| usage.category == ContextCategory::ToolDefinitions)
                    .expect("function schema category")
                    .tokens
            };
            assert!(tool_tokens(&report) > 0);
            assert!(actor.chat_state_handle.get_conversation().await.is_empty());

            let mut policy = ContextPolicy::default();
            policy.set(ContextCategory::ToolDefinitions, ContextSwitch::Excluded);
            actor.chat_state_handle.set_context_policy(policy).await.unwrap();
            let excluded = actor.context_policy_report().await.unwrap();
            assert!(!excluded.policy.includes(ContextCategory::ToolDefinitions));
            assert_eq!(tool_tokens(&excluded), tool_tokens(&report));
        })
        .await;
}

#[derive(Clone, Copy)]
enum SchemaTransport {
    Native,
    Tool,
}

#[tokio::test(flavor = "current_thread")]
async fn sampling_uses_built_policy_even_when_policy_changes_during_publication() {
    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for initial in [ContextSwitch::Included, ContextSwitch::Excluded] {
                for transport in [SchemaTransport::Native, SchemaTransport::Tool] {
                    let (gateway, _events) = tokio::sync::mpsc::unbounded_channel();
                    let (persistence, mut persisted) = tokio::sync::mpsc::unbounded_channel();
                    let actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
                    *actor.agent.borrow_mut() = test_agent_backend_search(vec![
                        xai_grok_sampling_types::HostedTool::WebSearch { options: None },
                    ]).await;
                    actor.supports_backend_search.set(true);
                    assert_eq!(actor.hosted_tools_for_turn().len(), 1);
                    let mut policy = ContextPolicy::default();
                    policy.set(ContextCategory::ToolDefinitions, initial);
                    actor.chat_state_handle.set_context_policy(policy.clone()).await.unwrap();
                    let schema = serde_json::json!({"type": "object", "required": ["answer"],
                        "properties": {"answer": {"type": "string"}}});
                    let tools = match transport {
                        SchemaTransport::Native => Vec::new(),
                        SchemaTransport::Tool => vec![xai_grok_sampling_types::ToolSpec {
                            name: "StructuredOutput".into(), description: None, parameters: schema.clone(),
                        }],
                    };
                    let native_schema = match transport {
                        SchemaTransport::Native => Some(schema.clone()),
                        SchemaTransport::Tool => None,
                    };
                    let built = actor.chat_state_handle.build_request(tools, Some(
                        "<memory-context>request memory</memory-context>".into(),
                    ), false, None, "c".into(), "r".into()).await.unwrap();
                    assert_eq!(built.context_policy, policy);
                    assert!(!built.injected_records.is_empty());
                    let mut published = Vec::new();
                    let prepare = actor.prepare_request_for_sampling(built, native_schema, &mut published);
                    let change_policy = async {
                        let PersistenceMsg::AppendUpdatesAndAck { respond_to, .. } = persisted.recv().await.unwrap() else {
                            panic!("request publication must await persistence");
                        };
                        policy.set(ContextCategory::ToolDefinitions, match initial {
                            ContextSwitch::Included => ContextSwitch::Excluded,
                            ContextSwitch::Excluded => ContextSwitch::Included,
                        });
                        assert_eq!(actor.chat_state_handle.set_context_policy(policy.clone()).await.unwrap(), policy);
                        respond_to.send(Ok(())).unwrap();
                    };
                    let (request, ()) = tokio::join!(prepare, change_policy);
                    let request = request.unwrap();
                    match initial {
                        ContextSwitch::Included => {
                            assert_eq!(request.hosted_tools.len(), 1);
                            match transport {
                                SchemaTransport::Native => assert_eq!(request.json_schema, Some(schema)),
                                SchemaTransport::Tool => {
                                    assert!(request.json_schema.is_none());
                                    assert_eq!(request.tools.len(), 1);
                                    assert_eq!(request.tools.first().unwrap().name, "StructuredOutput");
                                }
                            }
                        }
                        ContextSwitch::Excluded => {
                            assert!(request.hosted_tools.is_empty());
                            assert!(request.tools.is_empty());
                            assert!(request.json_schema.is_none());
                        }
                    }
                }
            }
        }).await.expect("policy changes cannot strand request publication");
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn context_policy_report_rejects_a_missing_chat_actor() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, _events) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, _persisted) = tokio::sync::mpsc::unbounded_channel();
        let mut actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
        actor.chat_state_handle = xai_chat_state::ChatStateHandle::noop();
        assert!(matches!(actor.context_policy_report().await,
            Err(crate::session::helpers::context_policy_store::ContextPolicyError::Admission(xai_chat_state::ContextPolicyAdmissionError::MailboxClosed(_)))));
        assert!(matches!(actor.set_context_policy(ContextPolicy::default()).await,
            Err(crate::session::helpers::context_policy_store::ContextPolicyError::Admission(xai_chat_state::ContextPolicyAdmissionError::MailboxClosed(_)))));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_not_yet_registered_actor_admits_global_revision_before_its_next_snapshot() {
    tokio::task::LocalSet::new().run_until(async {
        use xai_grok_sampling_types::{ContextPolicyEdit, ContextPolicyMutation, ContextScope};
        let (gateway, _events) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, _persisted) = tokio::sync::mpsc::unbounded_channel();
        let actor = create_test_actor(0, 256_000, 85, gateway, persistence).await;
        let id = actor.session_info.id.0.as_ref();
        let old = actor.chat_state_handle.build_request(Vec::new(), None, false, None,
            id.to_owned(), "before".into()).await.unwrap();
        {
            let mut store = actor.context_policy_binding.authority.write().await;
            let expected = store.selection(id, &actor.context_policy_binding.incarnation).unwrap().version;
            store.edit(id, &actor.context_policy_binding.incarnation, &ContextPolicyEdit {
                expected,
                mutation: ContextPolicyMutation::SetCategories {
                    categories: vec![ContextCategory::Memory].try_into().unwrap(), value: ContextSwitch::Excluded, target: ContextScope::Global,
                },
            }).unwrap();
        }
        let guard = actor.admit_context_policy_for_request().await.unwrap();
        let next = actor.chat_state_handle.build_request(Vec::new(), Some("memory that must not be sent".into()), false, None,
            id.to_owned(), "after".into()).await.unwrap();
        drop(guard);
        assert!(old.context_policy.includes(ContextCategory::Memory));
        assert!(!next.context_policy.includes(ContextCategory::Memory));
        assert!(!next.items.iter().any(|item| item.text_content().contains("memory that must not be sent")));
    }).await;
}

#[tokio::test(flavor = "current_thread")]
async fn independent_request_admissions_can_build_concurrently() {
    tokio::task::LocalSet::new().run_until(async {
        let (gateway, _events) = tokio::sync::mpsc::unbounded_channel();
        let (persistence, _persisted) = tokio::sync::mpsc::unbounded_channel();
        let first = create_test_actor(0, 256_000, 85, gateway.clone(), persistence.clone()).await;
        let mut second = create_test_actor(0, 256_000, 85, gateway, persistence).await;
        second.session_info.id = agent_client_protocol::SessionId::new("second-context-session");
        second.context_policy_binding = crate::session::helpers::context_policy_store::ContextPolicyBinding::new(
            first.context_policy_binding.authority.clone(),
        );
        let first_admission = first.admit_context_policy_for_request().await.unwrap();
        let built = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let second_admission = second.admit_context_policy_for_request().await.unwrap();
            let built = second.chat_state_handle.build_request(Vec::new(), None, false, None,
                "second".into(), "concurrent".into()).await.unwrap();
            drop(second_admission);
            built
        }).await.expect("one session's snapshot must not serialize another session's build");
        assert!(built.context_policy.is_unrestricted());
        drop(first_admission);
    }).await;
}
