use crate::implementations::grok_build::SEND_SUBAGENT_MESSAGE_TOOL_NAME;
use crate::implementations::grok_build::task::backend::SubagentBackendResource;
use crate::implementations::grok_build::task::types::{
    SubagentActor, SubagentCancelDisposition, SubagentCancelOutcome,
};
use crate::types::resources::OwnerSessionId;
use crate::types::tool::{ToolKind, ToolNamespace};

pub const CONTROL_SUBAGENT_TOOL_NAME: &str = "control_subagent";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ControlSubagentInput {
    /// The ID of the subagent to pause or stop.
    pub subagent_id: String,
    /// `pause` keeps the subagent resumable under the same ID; `stop` ends it.
    pub action: SubagentCancelDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlSubagentOutput {
    pub subagent_id: String,
    #[serde(flatten)]
    pub outcome: SubagentCancelOutcome,
}

impl ControlSubagentOutput {
    /// The subagent whose terminal notice this call answers; `None` when it found no subagent.
    pub fn consumed_completion_id(&self) -> Option<&str> {
        match self.outcome {
            SubagentCancelOutcome::Cancelled { .. }
            | SubagentCancelOutcome::AlreadyFinished { .. } => Some(&self.subagent_id),
            SubagentCancelOutcome::NotFound => None,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.outcome, SubagentCancelOutcome::NotFound)
    }
}

impl xai_tool_runtime::ToolOutput for ControlSubagentOutput {}

impl std::fmt::Display for ControlSubagentOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.outcome {
            SubagentCancelOutcome::Cancelled { cause } => f.write_str(&cause.model_text()),
            SubagentCancelOutcome::AlreadyFinished { state } => {
                write!(f, "Subagent had already ended. {}", state.model_text())
            }
            SubagentCancelOutcome::NotFound => {
                f.write_str("Subagent not found or not owned by this session.")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct ControlSubagentTool;

impl crate::types::tool_metadata::ToolMetadata for ControlSubagentTool {
    fn kind(&self) -> ToolKind {
        ToolKind::SubagentControl
    }

    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }

    fn description_template(&self) -> &str {
        static DESC: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
            format!(
                "Pause or stop a subagent that this session owns. \
                 `pause` interrupts its current turn and keeps it resumable under the same ID: \
                 send it a message with {SEND_SUBAGENT_MESSAGE_TOOL_NAME} to continue. \
                 `stop` ends it; it can only be continued as a new subagent via `resume_from`. \
                 Either action aborts the tool call in flight, so files may hold partial edits. \
                 Use it only when the subagent's work must wait or is no longer wanted; \
                 otherwise let the subagent run to completion."
            )
        });
        &DESC
    }
}

impl xai_tool_runtime::Tool for ControlSubagentTool {
    type Args = ControlSubagentInput;
    type Output = ControlSubagentOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new(CONTROL_SUBAGENT_TOOL_NAME).expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            CONTROL_SUBAGENT_TOOL_NAME,
            crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: false,
            tool_scope: Some(xai_tool_protocol::ToolScope::Write),
            ..Default::default()
        }
    }

    #[tracing::instrument(
        name = "tool.control_subagent",
        skip_all,
        fields(subagent_id = %input.subagent_id)
    )]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        input: ControlSubagentInput,
    ) -> Result<ControlSubagentOutput, xai_tool_runtime::ToolError> {
        let resources = crate::types::tool_metadata::shared_resources(&ctx)?;
        let (backend, owner) = {
            let res = resources.lock().await;
            (
                res.require::<SubagentBackendResource>()?.clone(),
                res.get::<OwnerSessionId>().map(|owner| owner.0.clone()),
            )
        };
        let outcome = backend
            .backend()
            .cancel(
                &input.subagent_id,
                SubagentActor::model_tool(owner),
                input.action,
            )
            .await;
        Ok(ControlSubagentOutput {
            subagent_id: input.subagent_id,
            outcome,
        })
    }
}

#[cfg(test)]
#[path = "control_subagent_tests.rs"]
mod tests;
