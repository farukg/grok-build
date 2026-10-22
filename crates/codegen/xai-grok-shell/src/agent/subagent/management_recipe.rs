//! Durable child-only inputs not carried by the conversation or selected agent definition.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ChildManagementRecipe {
    pub(crate) role_instructions: Option<String>,
    pub(crate) persona_instructions: Option<String>,
    pub(crate) inherited_mcp_servers: Vec<String>,
}

impl ChildManagementRecipe {
    pub(crate) fn inherited_pool(
        &self,
        pool: Option<crate::session::mcp_servers::SharedMcpPool>,
    ) -> Option<crate::session::mcp_servers::SharedMcpPool> {
        super::resolve_inherited_mcp_pool(
            pool,
            &xai_grok_agent::config::McpInheritance::Named(self.inherited_mcp_servers.clone()),
        )
    }

    pub(crate) fn save(&self, directory: &Path) -> std::io::Result<()> {
        let content = serde_json::to_string(self).map_err(std::io::Error::other)?;
        crate::util::config::atomic_write_string(&directory.join("child_management.json"), &content)
    }

    pub(crate) fn load(directory: &Path) -> std::io::Result<Option<Self>> {
        match std::fs::read(directory.join("child_management.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(std::io::Error::other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_child_mcp_snapshot_never_admits_new_parent_servers() {
        use crate::session::mcp_servers::{McpClient, McpState, SharedMcpPool};
        let dir = tempfile::tempdir().unwrap();
        let recipe = ChildManagementRecipe {
            role_instructions: Some("original role".into()),
            persona_instructions: Some("original persona".into()),
            inherited_mcp_servers: vec!["allowed".into()],
        };
        recipe.save(dir.path()).unwrap();
        let restored = ChildManagementRecipe::load(dir.path()).unwrap().unwrap();
        assert_eq!(restored.role_instructions.as_deref(), Some("original role"));
        assert_eq!(
            restored.persona_instructions.as_deref(),
            Some("original persona")
        );
        let mut state = McpState::new(vec![]);
        for name in ["allowed", "added-after-child-finished"] {
            state
                .owned_clients
                .insert(name.into(), std::sync::Arc::new(McpClient::stub(name)));
        }
        let pool = restored
            .inherited_pool(Some(SharedMcpPool::from_state(&state)))
            .unwrap();
        assert_eq!(pool.server_names().collect::<Vec<_>>(), vec!["allowed"]);
    }
}
