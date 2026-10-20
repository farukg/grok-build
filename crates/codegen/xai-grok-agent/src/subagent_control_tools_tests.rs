use std::sync::Arc;

use xai_grok_tools::computer::local::LocalTerminalBackend;
use xai_grok_tools::implementations::grok_build::CONTROL_SUBAGENT_TOOL_NAME;
use xai_grok_tools::notification::ToolNotificationHandle;

use crate::AgentBuilder;
use crate::prompt::context::PromptAudience;

async fn offers_control(enabled: bool, audience: PromptAudience) -> bool {
    AgentBuilder::new(
        std::env::temp_dir(),
        Arc::new(LocalTerminalBackend::new()),
        ToolNotificationHandle::noop(),
    )
    .from_definition(crate::config::AgentDefinition::default_grok_build())
    .with_prompt_audience(audience)
    .with_active_agent_messages_enabled(enabled)
    .build()
    .await
    .expect("agent should build")
    .tool_definitions()
    .await
    .iter()
    .any(|definition| definition.function.name == CONTROL_SUBAGENT_TOOL_NAME)
}

#[tokio::test]
async fn only_a_root_with_agent_messages_gets_subagent_control() {
    assert!(offers_control(true, PromptAudience::Primary).await);
    assert!(!offers_control(false, PromptAudience::Primary).await);
    assert!(!offers_control(true, PromptAudience::Subagent).await);
}
