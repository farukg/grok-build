use super::*;
use crate::session::acp_session::support::{create_test_actor, test_grok_build_agent_with_todo};
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
            let report = actor.context_policy_report().await;
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
            actor.chat_state_handle.set_context_policy(policy);
            let excluded = actor.context_policy_report().await;
            assert!(!excluded.policy.includes(ContextCategory::ToolDefinitions));
            assert_eq!(tool_tokens(&excluded), tool_tokens(&report));
        })
        .await;
}
