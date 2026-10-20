use xai_grok_tools::implementations::grok_build::{ControlSubagentTool, ListSubagentsTool};
use xai_grok_tools::registry::types::{ToolConfig, ToolServerConfig};

/// Root-only subagent control rides the active-agent-messages grant: children never get it, and
/// a definition that does not inject default tools keeps its own choice while the grant holds.
pub(crate) fn offer(tool_config: &mut ToolServerConfig, granted: bool, inject_default_tools: bool) {
    let tools = [
        ToolConfig::for_tool::<ControlSubagentTool>(),
        ToolConfig::for_tool::<ListSubagentsTool>(),
    ];
    for tool in tools {
        let same = |existing: &ToolConfig| existing.id == tool.id || existing.kind == tool.kind;
        let present = tool_config.tools.iter().any(same);
        match (granted, inject_default_tools) {
            (true, true) if !present => tool_config.tools.push(tool),
            (true, _) => {}
            (false, _) => tool_config.tools.retain(|existing| !same(existing)),
        }
    }
}

#[cfg(test)]
#[path = "subagent_control_tools_tests.rs"]
mod tests;
