use crate::implementations::grok_build::task::backend::SubagentBackendResource;
use crate::implementations::grok_build::task::types::{SubagentInspection, SubagentState};
use crate::types::tool::{ToolKind, ToolNamespace};

pub const LIST_SUBAGENTS_TOOL_NAME: &str = "list_subagents";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ListSubagentsInput {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListedSubagent {
    pub subagent_id: String,
    pub subagent_type: String,
    pub description: String,
    pub started_at_epoch_ms: u64,
    pub duration_ms: u64,
    pub state: SubagentState,
}

impl From<SubagentInspection> for ListedSubagent {
    fn from(inspection: SubagentInspection) -> Self {
        let snapshot = inspection.snapshot;
        Self {
            state: snapshot.status.state(),
            subagent_id: snapshot.subagent_id,
            subagent_type: snapshot.subagent_type,
            description: snapshot.description,
            started_at_epoch_ms: snapshot.started_at_epoch_ms,
            duration_ms: snapshot.duration_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListSubagentsOutput {
    pub subagents: Vec<ListedSubagent>,
}

impl xai_tool_runtime::ToolOutput for ListSubagentsOutput {}

impl std::fmt::Display for ListSubagentsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.subagents.is_empty() {
            return f.write_str("This session has no subagents.");
        }
        let mut separator = "";
        for subagent in &self.subagents {
            write!(
                f,
                "{separator}- {} [{}] {:?} ({:.1}s): {}",
                subagent.subagent_id,
                subagent.subagent_type,
                subagent.description,
                subagent.duration_ms as f64 / 1000.0,
                subagent.state.model_text(),
            )?;
            separator = "\n";
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ListSubagentsTool;

impl crate::types::tool_metadata::ToolMetadata for ListSubagentsTool {
    fn kind(&self) -> ToolKind {
        ToolKind::SubagentList
    }

    fn tool_namespace(&self) -> ToolNamespace {
        ToolNamespace::GrokBuild
    }

    fn description_template(&self) -> &str {
        "List the subagents this session owns, oldest first: running, queued, finished, paused or \
         stopped, each with its ID, type, description, elapsed time and state. A paused subagent \
         names who paused it. Use it to find a subagent's ID or to check which subagents are \
         still working."
    }
}

impl xai_tool_runtime::Tool for ListSubagentsTool {
    type Args = ListSubagentsInput;
    type Output = ListSubagentsOutput;

    fn id(&self) -> xai_tool_protocol::ToolId {
        xai_tool_protocol::ToolId::new(LIST_SUBAGENTS_TOOL_NAME).expect("valid tool id")
    }

    fn description(
        &self,
        _ctx: &xai_tool_runtime::ListToolsContext,
    ) -> xai_tool_types::ToolDescription {
        xai_tool_types::ToolDescription::new(
            LIST_SUBAGENTS_TOOL_NAME,
            crate::types::tool_metadata::ToolMetadata::sanitized_description_template(self),
        )
    }

    fn capabilities(&self) -> xai_tool_protocol::ToolCapabilities {
        xai_tool_protocol::ToolCapabilities {
            is_read_only: true,
            tool_scope: Some(xai_tool_protocol::ToolScope::Read),
            ..Default::default()
        }
    }

    #[tracing::instrument(name = "tool.list_subagents", skip_all)]
    async fn run(
        &self,
        ctx: xai_tool_runtime::ToolCallContext,
        _input: ListSubagentsInput,
    ) -> Result<ListSubagentsOutput, xai_tool_runtime::ToolError> {
        let resources = crate::types::tool_metadata::shared_resources(&ctx)?;
        let backend = resources
            .lock()
            .await
            .require::<SubagentBackendResource>()?
            .clone();
        let mut subagents: Vec<ListedSubagent> = backend
            .backend()
            .list_owned()
            .await
            .into_iter()
            .map(ListedSubagent::from)
            .collect();
        subagents.sort_by(|a, b| {
            (a.started_at_epoch_ms, &a.subagent_id).cmp(&(b.started_at_epoch_ms, &b.subagent_id))
        });
        Ok(ListSubagentsOutput { subagents })
    }
}
