use crate::implementations::grok_build::task::backend::SubagentBackendResource;
use crate::implementations::grok_build::task::types::{SubagentInspection, SubagentState};
use crate::types::tool::{ToolKind, ToolNamespace};

#[cfg(test)]
#[path = "list_subagents_tests.rs"]
mod tests;

pub const LIST_SUBAGENTS_TOOL_NAME: &str = "list_subagents";

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ListSubagentsInput {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListedSubagent {
    pub subagent_id: String,
    pub subagent_type: String,
    pub description: String,
    pub started_at_epoch_ms: u64,
    #[serde(default)]
    pub parent_session_id: String,
    #[serde(default)]
    pub child_session_id: String,
    pub fork_parent_prompt_id: Option<String>,
    pub resumed_from: Option<String>,
    pub persona: Option<String>,
    #[serde(flatten)]
    pub observation: ListedSubagentObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ListedSubagentObservation {
    Available { duration_ms: u64, state: SubagentState },
    Unavailable { unavailable: CustodyUnavailable },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustodyUnavailable {
    CustodyUnavailable,
}

impl From<SubagentInspection> for ListedSubagent {
    fn from(inspection: SubagentInspection) -> Self {
        let observation = match inspection.observation.state_duration() {
            Some((state, duration_ms)) => ListedSubagentObservation::Available {
                state,
                duration_ms,
            },
            None => ListedSubagentObservation::Unavailable {
                unavailable: CustodyUnavailable::CustodyUnavailable,
            },
        };
        Self {
            observation,
            subagent_id: inspection.subagent_id,
            subagent_type: inspection.subagent_type,
            description: inspection.description,
            started_at_epoch_ms: inspection.started_at_epoch_ms,
            parent_session_id: inspection.parent_session_id,
            child_session_id: inspection.child_session_id,
            fork_parent_prompt_id: inspection.fork_parent_prompt_id,
            resumed_from: inspection.resumed_from,
            persona: inspection.persona,
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
                "{separator}- {} [{}] {:?}",
                subagent.subagent_id,
                subagent.subagent_type,
                subagent.description,
            )?;
            match &subagent.observation {
                ListedSubagentObservation::Available { duration_ms, state } => {
                    write!(f, " ({:.1}s): {}", *duration_ms as f64 / 1000.0, state.model_text())?;
                }
                ListedSubagentObservation::Unavailable { .. } => {
                    f.write_str(": custody unavailable")?;
                }
            }
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
            .map_err(|error| xai_tool_runtime::ToolError::custom("coordinator_unavailable", error.to_string()))?
            .into_iter()
            .map(ListedSubagent::from)
            .collect();
        subagents.sort_by(|a, b| {
            (a.started_at_epoch_ms, &a.subagent_id).cmp(&(b.started_at_epoch_ms, &b.subagent_id))
        });
        Ok(ListSubagentsOutput { subagents })
    }
}
