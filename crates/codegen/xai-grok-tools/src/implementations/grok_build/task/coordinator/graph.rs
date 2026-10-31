//! Parent-local identity and compact evidence; runtime maps retain custody.

use std::collections::HashMap;

#[path = "graph/inventory.rs"]
mod inventory;
pub(crate) use inventory::RuntimeInventory;

/// Identity survives runtime-output eviction until explicit scope release.
#[derive(Default, Clone)]
pub(super) struct SpawnGraph {
    nodes: HashMap<String, SpawnNode>,
    scopes: HashMap<String, inventory::InventoryScope>,
}

#[derive(Clone)]
struct SpawnNode {
    records: Vec<inventory::CompactRecord>,
    root: String,
    /// Spawner sessions between the root and this child, outermost first.
    spawner_chain: Lineage,
    /// Host capability only; independent of the lineage above.
    advertise_to_spawner: bool,
    /// The ingress `surface_completion`, which reparent clears on the request itself.
    surface_to_spawner: bool,
}

#[derive(Clone)]
enum Lineage {
    KnownChain(Vec<String>),
    RootAudienceOnly,
}

impl Lineage {
    fn chain(&self) -> &[String] {
        match self {
            Self::KnownChain(chain) => chain,
            Self::RootAudienceOnly => &[],
        }
    }
}

pub(super) struct NestedSpawner {
    pub(super) child_id: String,
    /// The pre-reparent `parent_session_id`, which lineage checks compare against.
    pub(super) session_id: String,
    pub(super) surface_completion: bool,
}

/// The spawner has no node, so the child's ancestry cannot be completed.
#[derive(Debug)]
pub(super) struct MissingSpawnerNode;

impl SpawnGraph {
    pub(super) fn insert_root_child(&mut self, child_id: &str, root: &str) {
        if self.validate_lineage_insert(child_id, root).is_err() { return; }
        self.insert(
            child_id,
            SpawnNode {
                records: Vec::new(),
                root: root.to_owned(),
                spawner_chain: Lineage::KnownChain(Vec::new()),
                advertise_to_spawner: false,
                surface_to_spawner: false,
            },
        );
    }

    /// Fails closed rather than recording a chain with missing ancestors.
    pub(super) fn insert_nested(
        &mut self,
        child_id: &str,
        root: &str,
        spawner: NestedSpawner,
    ) -> Result<(), MissingSpawnerNode> {
        self.validate_lineage_insert(child_id, root)?;
        let parent = self.nodes.get(&spawner.child_id).ok_or(MissingSpawnerNode)?;
        if parent.root != root || spawner.child_id == child_id
            || spawner.session_id == root || spawner.session_id == child_id
            || parent.spawner_chain.chain().contains(&spawner.session_id)
            || parent.spawner_chain.chain().iter().any(|id| id == child_id)
        {
            return Err(MissingSpawnerNode);
        }
        let mut spawner_chain = match &parent.spawner_chain {
            Lineage::KnownChain(chain) => chain.clone(),
            Lineage::RootAudienceOnly => return Err(MissingSpawnerNode),
        };
        spawner_chain.push(spawner.session_id);
        self.insert(
            child_id,
            SpawnNode {
                records: Vec::new(),
                root: root.to_owned(),
                spawner_chain: Lineage::KnownChain(spawner_chain),
                advertise_to_spawner: true,
                surface_to_spawner: spawner.surface_completion,
            },
        );
        Ok(())
    }

    fn validate_lineage_insert(&self, child_id: &str, root: &str) -> Result<(), MissingSpawnerNode> {
        for scope in [Some(root), self.root_session(child_id)].into_iter().flatten() {
            match self.scopes.get(scope) {
                Some(inventory::InventoryScope::Releasing { .. }) => return Err(MissingSpawnerNode),
                Some(inventory::InventoryScope::Loading { .. }) | Some(inventory::InventoryScope::Ready { .. })
                | Some(inventory::InventoryScope::Failed { .. }) | None => {}
            }
        }
        Ok(())
    }

    fn insert(&mut self, child_id: &str, mut node: SpawnNode) {
        if let Some(previous) = self.nodes.remove(child_id) {
            node.records = previous.records;
        }
        self.nodes.insert(child_id.to_owned(), node);
    }

    pub(super) fn surface_target(&self, child_id: &str) -> Option<&str> {
        self.nodes
            .get(child_id)?
            .surface_to_spawner
            .then(|| self.direct_spawner(child_id))
            .flatten()
    }

    pub(super) fn remove(&mut self, child_id: &str) {
        if self.nodes.get(child_id).is_some_and(|node| node.records.is_empty()) {
            self.nodes.remove(child_id);
        } else {
            self.drop_advertise(child_id);
        }
    }

    pub(super) fn contains(&self, child_id: &str) -> bool {
        self.nodes.contains_key(child_id)
    }

    /// A missing node is unreachable.
    pub(super) fn is_reachable_from(&self, child_id: &str, session_id: &str) -> bool {
        self.nodes.get(child_id).is_some_and(|node| {
            node.root == session_id || node.spawner_chain.chain().iter().any(|id| id == session_id)
        })
    }

    pub(super) fn is_ancestor(&self, child_id: &str, ancestor_id: &str) -> bool {
        self.nodes
            .get(child_id)
            .is_some_and(|node| node.spawner_chain.chain().iter().any(|id| id == ancestor_id))
    }

    pub(super) fn root_session(&self, child_id: &str) -> Option<&str> {
        self.nodes.get(child_id).map(|node| node.root.as_str())
    }

    pub(super) fn direct_spawner(&self, child_id: &str) -> Option<&str> {
        self.nodes
            .get(child_id)?
            .spawner_chain
            .chain()
            .last()
            .map(String::as_str)
    }

    pub(super) fn advertise_target(&self, child_id: &str) -> Option<&str> {
        self.nodes
            .get(child_id)?
            .advertise_to_spawner
            .then(|| self.direct_spawner(child_id))
            .flatten()
    }

    pub(super) fn drop_advertise(&mut self, child_id: &str) {
        if let Some(node) = self.nodes.get_mut(child_id) {
            node.advertise_to_spawner = false;
        }
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
