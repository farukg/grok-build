use sha2::{Digest, Sha256};
use xai_chat_state::source_version::SourceVersion;

use super::{SpawnGraph, SpawnNode};
use super::super::super::inventory::*;
use super::super::super::terminal::{ChildTerminalEvidence, RecordedSubagentTerminal, RecordedTerminalState, SubagentTerminalState};
use super::super::super::types::{ChildObservation, SubagentAccessError, SubagentInspection};

#[derive(Clone)]
pub(super) enum InventoryScope {
    Loading { base: SourceVersion, version: SourceVersion },
    Ready { version: SourceVersion },
    Failed { version: SourceVersion, issues: Vec<InventoryIssue> },
    Releasing { version: SourceVersion, target: ReleaseTarget },
}

#[derive(Clone)]
pub(super) enum ReleaseTarget {
    Root,
    Subscope { session: String },
}

impl InventoryScope {
    fn version(&self) -> SourceVersion {
        match self {
            Self::Loading { version, .. } | Self::Ready { version }
            | Self::Failed { version, .. } | Self::Releasing { version, .. } => *version,
        }
    }

    fn advance(&mut self) {
        let next = advance(self.version());
        match self {
            Self::Loading { version, .. } | Self::Ready { version }
            | Self::Failed { version, .. } | Self::Releasing { version, .. } => *version = next,
        }
    }
}

fn advance(version: SourceVersion) -> SourceVersion {
    match version.sequence.checked_add(1) {
        Some(sequence) => SourceVersion { sequence, ..version },
        None => SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 },
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct CompactRecord {
    record: ChildEvidenceRecord,
    output_digest: Option<[u8; 32]>,
    producer_payload_digest: Option<[u8; 32]>,
}

impl CompactRecord {
    fn new(mut record: ChildEvidenceRecord) -> Self {
        let producer_payload_digest = if let ChildEvidence::Terminal { terminal: ChildTerminalEvidence::Producer { result, .. } } = &record.value {
            let mut digest = Sha256::new();
            digest.update(result.tool_calls.to_le_bytes());
            digest.update(result.turns.to_le_bytes());
            digest.update(result.duration_ms.to_le_bytes());
            digest.update(result.tokens_used.to_le_bytes());
            digest.update(result.output.as_bytes());
            Some(digest.finalize().into())
        } else { None };
        let mut output_digest = None;
        if let ChildEvidence::Terminal { terminal } = &mut record.value {
            let output = match terminal {
                ChildTerminalEvidence::Producer { result, .. } => Some(result.output.as_ref()),
                ChildTerminalEvidence::ProducerFact { recorded, .. } => recorded.as_ref().and_then(|r| r.output.as_deref()),
                ChildTerminalEvidence::Recorded { result } => result.output.as_deref(),
            };
            if let Some(output) = output {
                output_digest = Some(Sha256::digest(output.as_bytes()).into());
            }
            *terminal = compact_terminal(terminal);
        }
        Self { record, output_digest, producer_payload_digest }
    }
}

fn compact_terminal(terminal: &ChildTerminalEvidence) -> ChildTerminalEvidence {
    match terminal {
        ChildTerminalEvidence::Producer { completion_id, result, sources } => {
            let state = match &result.state {
                SubagentTerminalState::Completed => RecordedTerminalState::Completed,
                SubagentTerminalState::Failed { message } => RecordedTerminalState::Failed { message: Some(message.clone()) },
                SubagentTerminalState::Interrupted { cause } => RecordedTerminalState::Cancelled { cause: Some(cause.clone()), message: None },
            };
            ChildTerminalEvidence::ProducerFact {
                completion_id: completion_id.clone(), state: result.state.clone(), sources: sources.clone(),
                recorded: Some(RecordedSubagentTerminal {
                    attempt: terminal.attempt(), state, completed_at: None,
                    duration_ms: Some(result.duration_ms), tool_calls: Some(result.tool_calls),
                    turns: Some(result.turns), tokens_used: Some(result.tokens_used),
                    output: None, sources: sources.clone(),
                }),
            }
        }
        ChildTerminalEvidence::ProducerFact { completion_id, state, recorded, sources } => {
            let mut recorded = recorded.clone();
            if let Some(result) = &mut recorded { result.output = None; }
            ChildTerminalEvidence::ProducerFact { completion_id: completion_id.clone(), state: state.clone(), recorded, sources: sources.clone() }
        }
        ChildTerminalEvidence::Recorded { result } => {
            let mut result = result.clone();
            result.output = None;
            ChildTerminalEvidence::Recorded { result }
        }
    }
}

/// A response-time projection of actual custody, never stored in the graph.
pub(crate) struct RuntimeInventory {
    pub(crate) inspection: SubagentInspection,
    pub(crate) control_owner: Option<RecordedSubagentOwner>,
}

impl SpawnGraph {
    pub(crate) fn begin_inventory(&mut self, parent: &str) -> Result<SourceVersion, SubagentAccessError> {
        validate_id(parent).map_err(invalid)?;
        let version = match self.scopes.get(parent) {
            Some(InventoryScope::Releasing { .. }) => return Err(SubagentAccessError::InventoryReleasing),
            Some(scope) => advance(scope.version()),
            None => SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 },
        };
        self.scopes.insert(parent.to_owned(), InventoryScope::Loading { base: version, version });
        Ok(version)
    }

    pub(crate) fn inventory_source_version(&self, parent: &str) -> Result<SourceVersion, SubagentAccessError> {
        match self.scopes.get(parent) {
            Some(InventoryScope::Releasing { .. }) => Err(SubagentAccessError::InventoryReleasing),
            Some(scope) => Ok(scope.version()),
            None => Err(SubagentAccessError::InventorySuperseded),
        }
    }

    fn validate_runtime(&self, parent: &str, runtime: &[RuntimeInventory]) -> Result<(), SubagentAccessError> {
        let mut ids = std::collections::HashSet::new();
        for value in runtime {
            value.inspection.validate().map_err(invalid)?;
            if value.inspection.parent_session_id != parent || !ids.insert(&value.inspection.subagent_id)
                || self.nodes.get(&value.inspection.subagent_id).is_none_or(|node| node.root != parent)
            {
                return Err(invalid(InventoryValidationError::IdentityMismatch));
            }
            validate_owner(value.control_owner.as_ref()).map_err(invalid)?;
        }
        Ok(())
    }

    pub(crate) fn inventory_version(&self, parent: &str) -> Result<SourceVersion, SubagentAccessError> {
        match self.scopes.get(parent) {
            Some(InventoryScope::Ready { version }) => Ok(*version),
            Some(InventoryScope::Failed { issues, .. }) => Err(SubagentAccessError::InventoryUnreadable { issues: issues.clone() }),
            Some(InventoryScope::Releasing { .. }) => Err(SubagentAccessError::InventoryReleasing),
            Some(InventoryScope::Loading { .. }) | None => Err(SubagentAccessError::InventorySuperseded),
        }
    }

    pub(crate) fn import_inventory(&mut self, batch: ChildInventoryImport, runtime: &[RuntimeInventory]) -> Result<ChildInventorySnapshot, SubagentAccessError> {
        batch.validate().map_err(invalid)?;
        self.validate_runtime(&batch.parent_session_id, runtime)?;
        let version = match self.scopes.get(&batch.parent_session_id) {
            Some(InventoryScope::Loading { base, version }) if *base == batch.base && base.incarnation == version.incarnation => *version,
            Some(InventoryScope::Releasing { .. }) => return Err(SubagentAccessError::InventoryReleasing),
            Some(InventoryScope::Loading { .. }) | Some(InventoryScope::Ready { .. })
            | Some(InventoryScope::Failed { .. }) | None => return Err(SubagentAccessError::InventorySuperseded),
        };
        for record in &batch.records {
            self.validate_root(&batch.parent_session_id, &record.subagent_id)?;
        }
        let mut candidate = self.clone();
        for record in batch.records {
            candidate.merge_record(&batch.parent_session_id, CompactRecord::new(record));
        }
        candidate.bind_lineage(&batch.parent_session_id)?;
        self.nodes = candidate.nodes;
        let version = advance(version);
        let scope = if batch.issues.is_empty() {
            InventoryScope::Ready { version }
        } else {
            InventoryScope::Failed { version, issues: batch.issues }
        };
        self.scopes.insert(batch.parent_session_id.clone(), scope);
        self.inventory_snapshot(&batch.parent_session_id, runtime)
    }

    fn bind_lineage(&mut self, root: &str) -> Result<(), SubagentAccessError> {
        let mut chains = Vec::new();
        for (id, node) in &self.nodes {
            if node.root != root { continue; }
            let mut visiting = std::collections::HashSet::new();
            chains.push((id.clone(), self.resolve_lineage(id, root, &mut visiting)?));
        }
        for (id, chain) in chains {
            if let Some(node) = self.nodes.get_mut(&id) { node.spawner_chain = chain; }
        }
        Ok(())
    }

    fn resolve_lineage(&self, id: &str, root: &str, visiting: &mut std::collections::HashSet<String>) -> Result<super::Lineage, SubagentAccessError> {
        if id == root || !visiting.insert(id.to_owned()) {
            return Err(invalid(InventoryValidationError::IdentityMismatch));
        }
        let node = self.nodes.get(id).ok_or_else(|| invalid(InventoryValidationError::IdentityMismatch))?;
        if node.root != root { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
        let records: Vec<_> = node.records.iter().map(|r| &r.record).collect();
        let attempt = select_attempt(&records).ok();
        let selected: Vec<_> = records.into_iter().filter(|r| Some(&r.attempt) == attempt.as_ref()).collect();
        for record in &selected {
            let session = match &record.value {
                ChildEvidence::Identity { child_session_id, .. } => child_session_id.as_ref(),
                ChildEvidence::SummaryIdentity { child_session_id, .. } => Some(child_session_id),
                ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None,
            };
            if session.is_some_and(|session| session == root) { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
        }
        let mut spawner = None;
        for record in &selected {
            match &record.value {
                ChildEvidence::Identity { recorded_spawner: Some(value), .. } => {
                    if spawner.is_some_and(|old| old != value) {
                        return Err(invalid(InventoryValidationError::IdentityMismatch));
                    }
                    spawner = Some(value);
                }
                ChildEvidence::Identity { recorded_spawner: None, .. } | ChildEvidence::SummaryIdentity { .. }
                | ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => {}
            }
        }
        let chain = match spawner {
            None => node.spawner_chain.clone(),
            Some(RecordedSubagentSpawner::Root) => {
                if !node.spawner_chain.chain().is_empty() { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
                super::Lineage::KnownChain(Vec::new())
            }
            Some(RecordedSubagentSpawner::Child { subagent_id, session_id }) => {
                if session_id == root || session_id == id || subagent_id == id {
                    return Err(invalid(InventoryValidationError::IdentityMismatch));
                }
                let Some(parent) = self.nodes.get(subagent_id) else {
                    visiting.remove(id);
                    return Ok(super::Lineage::RootAudienceOnly);
                };
                if parent.root != root { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
                let parent_records: Vec<_> = parent.records.iter().map(|r| &r.record).collect();
                let Ok(parent_attempt) = select_attempt(&parent_records) else {
                    visiting.remove(id);
                    return Ok(super::Lineage::RootAudienceOnly);
                };
                let mut actual_session = None;
                for record in parent_records.iter().filter(|r| r.attempt == parent_attempt) {
                    let session = match &record.value {
                        ChildEvidence::Identity { child_session_id, .. } => child_session_id.as_ref(),
                        ChildEvidence::SummaryIdentity { child_session_id, .. } => Some(child_session_id),
                        ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None,
                    };
                    if let Some(session) = session {
                        if actual_session.is_some_and(|old| old != session) { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
                        actual_session = Some(session);
                    }
                }
                let Some(actual_session) = actual_session else {
                    visiting.remove(id);
                    return Ok(super::Lineage::RootAudienceOnly);
                };
                if actual_session != session_id { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
                match self.resolve_lineage(subagent_id, root, visiting)? {
                    super::Lineage::RootAudienceOnly => super::Lineage::RootAudienceOnly,
                    super::Lineage::KnownChain(mut chain) => {
                        if chain.contains(session_id) || chain.iter().any(|session| session == id) {
                            return Err(invalid(InventoryValidationError::IdentityMismatch));
                        }
                        chain.push(session_id.clone());
                        if !node.spawner_chain.chain().is_empty() && node.spawner_chain.chain() != chain.as_slice() {
                            return Err(invalid(InventoryValidationError::IdentityMismatch));
                        }
                        super::Lineage::KnownChain(chain)
                    }
                }
            }
        };
        for record in selected {
            let session = match &record.value {
                ChildEvidence::Identity { child_session_id, .. } => child_session_id.as_ref(),
                ChildEvidence::SummaryIdentity { child_session_id, .. } => Some(child_session_id),
                ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None,
            };
            if session.is_some_and(|session| session == root || chain.chain().contains(session)) {
                return Err(invalid(InventoryValidationError::IdentityMismatch));
            }
        }
        visiting.remove(id);
        Ok(chain)
    }

    fn validate_root(&self, parent: &str, child: &str) -> Result<(), SubagentAccessError> {
        if parent == child { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
        if let Some(node) = self.nodes.get(child) && node.root != parent {
            return Err(invalid(InventoryValidationError::IdentityMismatch));
        }
        Ok(())
    }

    fn merge_record(&mut self, parent: &str, record: CompactRecord) {
        let node = self.nodes.entry(record.record.subagent_id.clone()).or_insert_with(|| SpawnNode {
            root: parent.to_owned(), spawner_chain: super::Lineage::RootAudienceOnly, advertise_to_spawner: false,
            surface_to_spawner: false, records: Vec::new(),
        });
        if !node.records.contains(&record) { node.records.push(record); }
    }

    /// Call only at real runtime/commit transitions, with the existing scope version.
    pub(crate) fn record_runtime_evidence(&mut self, parent: &str, record: ChildEvidenceRecord) -> Result<SourceVersion, SubagentAccessError> {
        record.validate().map_err(invalid)?;
        self.validate_root(parent, &record.subagent_id)?;
        let scope = self.scopes.get(parent).ok_or(SubagentAccessError::InventorySuperseded)?;
        match scope {
            InventoryScope::Releasing { .. } => return Err(SubagentAccessError::InventoryReleasing),
            InventoryScope::Loading { .. } | InventoryScope::Ready { .. } | InventoryScope::Failed { .. } => {}
        }
        match &record.source {
            ChildEvidenceSource::Coordinator { version } if *version == scope.version() => {}
            ChildEvidenceSource::Coordinator { .. } => return Err(SubagentAccessError::InventoryChanged),
            ChildEvidenceSource::ParentMeta { .. } | ChildEvidenceSource::ParentJournal { .. }
            | ChildEvidenceSource::ParentSummary { .. } | ChildEvidenceSource::ChildSummary { .. } => return Err(invalid(InventoryValidationError::InvalidSource)),
        }
        let child = record.subagent_id.clone();
        match &record.value {
            ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. } => {
                let previous = self.nodes.get(&child).cloned();
                self.merge_record(parent, CompactRecord::new(record));
                if let Err(error) = self.bind_lineage(parent) {
                    match previous {
                        Some(node) => { self.nodes.insert(child, node); }
                        None => { self.nodes.remove(&child); }
                    }
                    return Err(error);
                }
            }
            ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => self.merge_record(parent, CompactRecord::new(record)),
        }
        let scope = self.scopes.get_mut(parent).ok_or(SubagentAccessError::InventorySuperseded)?;
        scope.advance();
        Ok(scope.version())
    }

    pub(crate) fn inventory_snapshot(&self, parent: &str, runtime: &[RuntimeInventory]) -> Result<ChildInventorySnapshot, SubagentAccessError> {
        let version = self.inventory_version(parent)?;
        self.validate_runtime(parent, runtime)?;
        let mut entries = Vec::new();
        for (id, node) in &self.nodes {
            if node.root == parent {
                let resident = runtime.iter().find(|r| r.inspection.subagent_id == *id && r.inspection.parent_session_id == parent);
                require_capture(id, node, resident)?;
                entries.push(project(id, node, resident));
            }
        }
        entries.sort_by(|a, b| a.subagent_id().cmp(b.subagent_id()));
        Ok(ChildInventorySnapshot { parent_session_id: parent.to_owned(), version, entries })
    }

    /// Exact runtime reads remain available when a full historical read failed.
    pub(crate) fn inventory_entry(&self, parent: &str, child: &str, runtime: Option<&RuntimeInventory>) -> Result<Option<ChildInventoryEntry>, SubagentAccessError> {
        if let Some(runtime) = runtime {
            runtime.inspection.validate().map_err(invalid)?;
            if runtime.inspection.parent_session_id != parent || runtime.inspection.subagent_id != child {
                return Err(invalid(InventoryValidationError::IdentityMismatch));
            }
            if let Some(InventoryScope::Releasing { .. }) = self.scopes.get(parent) {
                return Err(SubagentAccessError::InventoryReleasing);
            }
        } else {
            self.inventory_version(parent)?;
        }
        self.nodes.get(child).filter(|n| n.root == parent).map(|n| {
            require_capture(child, n, runtime)?;
            Ok(project(child, n, runtime))
        }).transpose()
    }

    pub(crate) fn begin_inventory_release(&mut self, parent: &str) -> Result<SourceVersion, SubagentAccessError> {
        let scope = self.scopes.get(parent).ok_or(SubagentAccessError::InventorySuperseded)?;
        let version = match scope {
            InventoryScope::Releasing { version, target: ReleaseTarget::Root } => *version,
            InventoryScope::Releasing { target: ReleaseTarget::Subscope { .. }, .. } => return Err(SubagentAccessError::InventoryReleasing),
            InventoryScope::Loading { .. } | InventoryScope::Ready { .. } | InventoryScope::Failed { .. } => advance(scope.version()),
        };
        self.scopes.insert(parent.to_owned(), InventoryScope::Releasing { version, target: ReleaseTarget::Root });
        Ok(version)
    }

    /// The caller must first settle the real runtime/publication drain.
    pub(crate) fn release_inventory(&mut self, parent: &str, version: SourceVersion) -> Result<(), SubagentAccessError> {
        match self.scopes.get(parent) {
            Some(InventoryScope::Releasing { version: current, target: ReleaseTarget::Root }) if *current == version => {}
            Some(InventoryScope::Loading { .. }) | Some(InventoryScope::Ready { .. })
            | Some(InventoryScope::Failed { .. }) | Some(InventoryScope::Releasing { .. }) | None => return Err(SubagentAccessError::InventorySuperseded),
        }
        self.nodes.retain(|_, node| node.root != parent);
        self.scopes.remove(parent);
        Ok(())
    }

    pub(crate) fn begin_inventory_subscope_release(&mut self, root: &str, session: &str) -> Result<SourceVersion, SubagentAccessError> {
        validate_id(session).map_err(invalid)?;
        if session == root { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
        let known_session = self.nodes.values().filter(|node| node.root == root).any(|node| {
            node.spawner_chain.chain().iter().any(|ancestor| ancestor == session)
                || node.records.iter().any(|record| match &record.record.value {
                    ChildEvidence::Identity { child_session_id, .. } => child_session_id.as_deref() == Some(session),
                    ChildEvidence::SummaryIdentity { child_session_id, .. } => child_session_id == session,
                    ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => false,
                })
        });
        if !known_session { return Err(invalid(InventoryValidationError::IdentityMismatch)); }
        let scope = self.scopes.get(root).ok_or(SubagentAccessError::InventorySuperseded)?;
        match scope {
            InventoryScope::Releasing { version, target: ReleaseTarget::Subscope { session: current, .. } } if current == session => return Ok(*version),
            InventoryScope::Releasing { .. } => return Err(SubagentAccessError::InventoryReleasing),
            InventoryScope::Loading { .. } | InventoryScope::Failed { .. } => return Err(SubagentAccessError::InventorySuperseded),
            InventoryScope::Ready { .. } => {}
        }
        let version = advance(scope.version());
        self.scopes.insert(root.to_owned(), InventoryScope::Releasing {
            version, target: ReleaseTarget::Subscope { session: session.to_owned() },
        });
        Ok(version)
    }

    /// The caller must settle the selected subtree's runtime/publication drain first.
    pub(crate) fn release_inventory_subscope(&mut self, root: &str, session: &str, version: SourceVersion) -> Result<(), SubagentAccessError> {
        match self.scopes.get(root) {
            Some(InventoryScope::Releasing { version: current, target: ReleaseTarget::Subscope { session: selected } }) if *current == version && selected == session => {},
            Some(InventoryScope::Loading { .. }) | Some(InventoryScope::Ready { .. }) | Some(InventoryScope::Failed { .. }) | Some(InventoryScope::Releasing { .. }) | None => return Err(SubagentAccessError::InventorySuperseded),
        };
        self.nodes.retain(|_, node| node.root != root || !node.spawner_chain.chain().iter().any(|ancestor| ancestor == session));
        // A fresh incarnation fences subtree proofs and imports without losing siblings.
        self.scopes.insert(root.to_owned(), InventoryScope::Ready { version: SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 } });
        Ok(())
    }

    pub(crate) fn matches_inventory_proof(&self, parent: &str, proof: &InventoryCancelProof, current: &InventoryTarget) -> bool {
        let Some(scope) = self.scopes.get(parent) else { return false; };
        match scope {
            InventoryScope::Releasing { .. } => false,
            InventoryScope::Loading { .. } | InventoryScope::Ready { .. } | InventoryScope::Failed { .. } => {
                let version = scope.version();
                proof.version.incarnation == version.incarnation && proof.version.sequence <= version.sequence
                    && &proof.target == current
            }
        }
    }
}

fn invalid(reason: InventoryValidationError) -> SubagentAccessError {
    SubagentAccessError::InvalidInventory { reason }
}

fn require_capture(id: &str, node: &SpawnNode, runtime: Option<&RuntimeInventory>) -> Result<(), SubagentAccessError> {
    if node.records.is_empty() && runtime.is_none() {
        return Err(SubagentAccessError::InventoryUnreadable { issues: vec![InventoryIssue::RuntimeCaptureUnavailable { subagent_id: id.to_owned() }] });
    }
    Ok(())
}

fn project(id: &str, node: &SpawnNode, runtime: Option<&RuntimeInventory>) -> ChildInventoryEntry {
    let records: Vec<_> = node.records.iter().map(|r| &r.record).collect();
    let attempt = match runtime {
        Some(runtime) => Ok(runtime.inspection.attempt.clone()),
        None => select_attempt(&records),
    };
    let attempt = match attempt {
        Ok(attempt) => attempt,
        Err(issue) => return ChildInventoryEntry::Conflict { subagent_id: id.to_owned(), issues: vec![issue] },
    };
    let selected: Vec<_> = records.iter().copied().filter(|r| r.attempt == attempt).collect();
    let mut issues = identity_issues(&selected);
    let observation = terminal_observation(&selected, &node.records, &mut issues);
    if let Some(runtime) = runtime {
        let mut inspection = runtime.inspection.clone();
        if let Some(owner) = &runtime.control_owner {
            if selected.iter().filter_map(|r| match &r.value { ChildEvidence::Identity { recorded_owner, .. } => recorded_owner.as_ref(), ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None }).any(|recorded| recorded != owner) {
                issues.push(InventoryIssue::OwnerConflict { sources: selected.iter().map(|r| r.source.clone()).collect() });
            }
        }
        // Custody includes the publication barrier, even beside historical terminal evidence.
        inspection.observation = runtime.inspection.observation.clone();
        return ChildInventoryEntry::Child { inspection, control_owner: runtime.control_owner.clone(), issues };
    }
    if observation.is_none() && issues.iter().any(|i| matches!(i, InventoryIssue::TerminalConflict { .. })) {
        return ChildInventoryEntry::Conflict { subagent_id: id.to_owned(), issues };
    }
    if !issues.is_empty() {
        let producer = selected.iter().filter_map(|r| terminal_value(&r.value)).any(|terminal| producer_identity(terminal).is_some());
        if !producer || issues.iter().any(|i| !matches!(i, InventoryIssue::TerminalConflict { .. })) {
            return ChildInventoryEntry::Conflict { subagent_id: id.to_owned(), issues };
        }
    }
    let identity = selected.iter().find_map(|r| match &r.value { ChildEvidence::Identity { .. } => owned_identity(r), ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None });
    let summary = selected.iter().find_map(|r| match &r.value { ChildEvidence::SummaryIdentity { .. } => Some(*r), ChildEvidence::Identity { .. } | ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None });
    let mut sources = Vec::new();
    union_sources(&mut sources, &selected.iter().map(|r| r.source.clone()).collect::<Vec<_>>());
    let observation = observation.unwrap_or_else(|| {
        match selected.iter().rev().find_map(|r| match &r.value { ChildEvidence::Nonterminal { phase } => Some(*phase), ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Terminal { .. } => None }) {
            Some(last) => ChildObservation::Uncustodied { attempt: attempt.clone(), last, sources },
            None => ChildObservation::IdentityOnly { attempt: attempt.clone(), sources },
        }
    });
    let mut inspection = SubagentInspection {
        subagent_id: id.to_owned(), parent_session_id: node.root.clone(), attempt,
        child_session_id: None, description: None, subagent_type: None, started_at_epoch_ms: None,
        persona: None, resumed_from: None, fork_parent_prompt_id: None,
        recorded_owner: None, recorded_spawner: None, observation,
    };
    if let Some(record) = identity {
        if let ChildEvidence::Identity { child_session_id, description, subagent_type, started_at_epoch_ms, persona, resumed_from, fork_parent_prompt_id, recorded_owner, recorded_spawner } = &record.value {
            inspection.child_session_id = child_session_id.clone();
            inspection.description = Some(description.clone());
            inspection.subagent_type = Some(subagent_type.clone());
            inspection.started_at_epoch_ms = *started_at_epoch_ms;
            inspection.persona = persona.clone(); inspection.resumed_from = resumed_from.clone();
            inspection.fork_parent_prompt_id = fork_parent_prompt_id.clone();
            inspection.recorded_owner = recorded_owner.clone(); inspection.recorded_spawner = recorded_spawner.clone();
        }
    } else if let Some(record) = summary {
        if let ChildEvidence::SummaryIdentity { child_session_id, fork_parent_prompt_id } = &record.value {
            inspection.child_session_id = Some(child_session_id.clone());
            inspection.fork_parent_prompt_id = fork_parent_prompt_id.clone();
        }
    } else {
        return ChildInventoryEntry::Reference { subagent_id: id.to_owned(), child_session_id: None, records: records.into_iter().cloned().collect() };
    }
    for record in &selected {
        if let ChildEvidence::Identity { child_session_id, started_at_epoch_ms, persona, resumed_from, fork_parent_prompt_id, recorded_owner, recorded_spawner, .. } = &record.value {
            inspection.child_session_id = inspection.child_session_id.or_else(|| child_session_id.clone());
            inspection.started_at_epoch_ms = inspection.started_at_epoch_ms.or(*started_at_epoch_ms);
            inspection.persona = inspection.persona.or_else(|| persona.clone());
            inspection.resumed_from = inspection.resumed_from.or_else(|| resumed_from.clone());
            inspection.fork_parent_prompt_id = inspection.fork_parent_prompt_id.or_else(|| fork_parent_prompt_id.clone());
            inspection.recorded_owner = inspection.recorded_owner.or_else(|| recorded_owner.clone());
            inspection.recorded_spawner = inspection.recorded_spawner.or_else(|| recorded_spawner.clone());
        }
        if let ChildEvidence::SummaryIdentity { child_session_id, fork_parent_prompt_id } = &record.value {
            inspection.child_session_id = inspection.child_session_id.or_else(|| Some(child_session_id.clone()));
            inspection.fork_parent_prompt_id = inspection.fork_parent_prompt_id.or_else(|| fork_parent_prompt_id.clone());
        }
    }
    ChildInventoryEntry::Child { control_owner: inspection.recorded_owner.clone(), inspection, issues }
}

fn owned_identity(record: &ChildEvidenceRecord) -> Option<&ChildEvidenceRecord> {
    match &record.source {
        ChildEvidenceSource::ParentMeta { .. } | ChildEvidenceSource::Coordinator { .. } => Some(record),
        ChildEvidenceSource::ParentJournal { .. } | ChildEvidenceSource::ParentSummary { .. } | ChildEvidenceSource::ChildSummary { .. } => None,
    }
}

fn terminal_value(value: &ChildEvidence) -> Option<&ChildTerminalEvidence> {
    match value {
        ChildEvidence::Terminal { terminal } => Some(terminal),
        ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Nonterminal { .. } => None,
    }
}

fn producer_identity(terminal: &ChildTerminalEvidence) -> Option<&xai_chat_state::SubagentCompletionId> {
    match terminal {
        ChildTerminalEvidence::Producer { completion_id, .. } | ChildTerminalEvidence::ProducerFact { completion_id, .. } => Some(completion_id),
        ChildTerminalEvidence::Recorded { .. } => None,
    }
}

fn select_attempt(records: &[&ChildEvidenceRecord]) -> Result<AttemptEvidence, InventoryIssue> {
    if let Some(record) = records.iter().rev().find_map(|r| match &r.source { ChildEvidenceSource::Coordinator { .. } => Some(*r), ChildEvidenceSource::ParentMeta { .. } | ChildEvidenceSource::ParentJournal { .. } | ChildEvidenceSource::ParentSummary { .. } | ChildEvidenceSource::ChildSummary { .. } => None }) {
        return Ok(record.attempt.clone());
    }
    let sources = || records.iter().map(|r| r.source.clone()).collect();
    let mut anchors: Vec<AttemptEvidence> = Vec::new();
    for record in records {
        let anchor = match &record.value {
            ChildEvidence::SummaryIdentity { .. } => Some(*record),
            ChildEvidence::Identity { .. } => owned_identity(record),
            ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. } => None,
        };
        if let Some(record) = anchor {
            if !anchors.contains(&record.attempt) { anchors.push(record.attempt.clone()); }
        }
    }
    let latest_spawn = records.iter().filter_map(|r| match (&r.value, &r.source) {
        (ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Spawned }, ChildEvidenceSource::ParentJournal { line, .. }) => Some((*line, &r.attempt)),
        (ChildEvidence::Nonterminal { phase: HistoricalChildPhase::Running }, ChildEvidenceSource::ParentJournal { .. }) => None,
        (ChildEvidence::Nonterminal { .. }, ChildEvidenceSource::ParentMeta { .. } | ChildEvidenceSource::ParentSummary { .. } | ChildEvidenceSource::ChildSummary { .. } | ChildEvidenceSource::Coordinator { .. }) => None,
        (ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Terminal { .. }, ChildEvidenceSource::ParentMeta { .. } | ChildEvidenceSource::ParentJournal { .. } | ChildEvidenceSource::ParentSummary { .. } | ChildEvidenceSource::ChildSummary { .. } | ChildEvidenceSource::Coordinator { .. }) => None,
    }).max_by_key(|(line, _)| *line);
    if let Some((_, attempt)) = latest_spawn {
        if anchors.iter().any(|anchor| anchor != attempt) {
            return Err(InventoryIssue::AttemptConflict { sources: sources() });
        }
        return Ok(attempt.clone());
    }
    if anchors.len() == 1 { return Ok(anchors.remove(0)); }
    if anchors.len() > 1 { return Err(InventoryIssue::AttemptConflict { sources: sources() }); }
    let mut attempts: Vec<_> = records.iter().map(|r| r.attempt.clone()).collect();
    attempts.dedup();
    match attempts.first() {
        Some(first) if attempts.iter().all(|a| a == first) => Ok(first.clone()),
        Some(_) | None => Err(InventoryIssue::AttemptConflict { sources: sources() }),
    }
}

fn identity_issues(records: &[&ChildEvidenceRecord]) -> Vec<InventoryIssue> {
    let mut issues = Vec::new();
    for (index, a) in records.iter().enumerate() {
        for b in records.iter().skip(index + 1) {
            match (&a.value, &b.value) {
                (ChildEvidence::Identity { child_session_id: ac, description: ad, subagent_type: at, recorded_owner: ao, recorded_spawner: asp, started_at_epoch_ms: ast, persona: ap, resumed_from: ar, fork_parent_prompt_id: af, .. }, ChildEvidence::Identity { child_session_id: bc, description: bd, subagent_type: bt, recorded_owner: bo, recorded_spawner: bsp, started_at_epoch_ms: bst, persona: bp, resumed_from: br, fork_parent_prompt_id: bf, .. }) => {
                    let sources = vec![a.source.clone(), b.source.clone()];
                    if ad != bd || at != bt || (ac.is_some() && bc.is_some() && ac != bc)
                        || (ast.is_some() && bst.is_some() && ast != bst)
                        || (ap.is_some() && bp.is_some() && ap != bp)
                        || (ar.is_some() && br.is_some() && ar != br)
                        || (af.is_some() && bf.is_some() && af != bf)
                    { issues.push(InventoryIssue::IdentityMismatch { source: b.source.clone() }); }
                    if ao.is_some() && bo.is_some() && ao != bo { issues.push(InventoryIssue::OwnerConflict { sources: sources.clone() }); }
                    if asp.is_some() && bsp.is_some() && asp != bsp { issues.push(InventoryIssue::LineageConflict { sources }); }
                }
                (ChildEvidence::Identity { child_session_id, fork_parent_prompt_id, .. }, ChildEvidence::SummaryIdentity { child_session_id: child, fork_parent_prompt_id: prompt })
                | (ChildEvidence::SummaryIdentity { child_session_id: child, fork_parent_prompt_id: prompt }, ChildEvidence::Identity { child_session_id, fork_parent_prompt_id, .. }) => {
                    if child_session_id.as_ref().is_some_and(|id| id != child)
                        || fork_parent_prompt_id.as_ref().zip(prompt.as_ref()).is_some_and(|(a, b)| a != b) {
                        issues.push(InventoryIssue::IdentityMismatch { source: b.source.clone() });
                    }
                }
                (ChildEvidence::SummaryIdentity { child_session_id: ac, fork_parent_prompt_id: af }, ChildEvidence::SummaryIdentity { child_session_id: bc, fork_parent_prompt_id: bf }) => {
                    if ac != bc || af.as_ref().zip(bf.as_ref()).is_some_and(|(a, b)| a != b) {
                        issues.push(InventoryIssue::IdentityMismatch { source: b.source.clone() });
                    }
                }
                (ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. }, ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. } | ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. })
                | (ChildEvidence::Identity { .. } | ChildEvidence::SummaryIdentity { .. }, ChildEvidence::Nonterminal { .. } | ChildEvidence::Terminal { .. }) => {}
            }
        }
    }
    issues
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum TerminalRank {
    Producer,
    Recorded,
}

fn terminal_rank(terminal: &ChildTerminalEvidence) -> TerminalRank {
    match terminal {
        ChildTerminalEvidence::Producer { .. } | ChildTerminalEvidence::ProducerFact { .. } => TerminalRank::Producer,
        ChildTerminalEvidence::Recorded { .. } => TerminalRank::Recorded,
    }
}

fn terminal_observation(records: &[&ChildEvidenceRecord], compact: &[CompactRecord], issues: &mut Vec<InventoryIssue>) -> Option<ChildObservation> {
    let terminals: Vec<_> = records.iter().filter_map(|r| terminal_value(&r.value).map(|terminal| (*r, terminal))).collect();
    let mut selected = terminals.iter().find_map(|(_, terminal)| match terminal {
        ChildTerminalEvidence::ProducerFact { .. } => Some((*terminal).clone()),
        ChildTerminalEvidence::Producer { .. } | ChildTerminalEvidence::Recorded { .. } => None,
    }).or_else(|| terminals.first().map(|(_, terminal)| (*terminal).clone()));
    let mut producer_conflict = false;
    for rank in [TerminalRank::Producer, TerminalRank::Recorded] {
        let original = selected.clone();
        let issue_count = issues.len();
        for (record, terminal) in terminals.iter().filter(|(_, terminal)| terminal_rank(terminal) == rank) {
            let Some(current) = &mut selected else { continue; };
            let compatible = match (&*current, *terminal) {
                (ChildTerminalEvidence::ProducerFact { completion_id: a, state: ast, .. }, ChildTerminalEvidence::ProducerFact { completion_id: b, state: bst, .. }) => a == b && ast == bst,
                (ChildTerminalEvidence::ProducerFact { state, .. }, ChildTerminalEvidence::Recorded { result })
                | (ChildTerminalEvidence::Recorded { result }, ChildTerminalEvidence::ProducerFact { state, .. }) => result.state.matches_producer(state),
                (ChildTerminalEvidence::Recorded { result: a }, ChildTerminalEvidence::Recorded { result: b }) => {
                    let mut state = a.state.clone();
                    merge_state(&mut state, &b.state)
                }
                (ChildTerminalEvidence::Producer { .. }, ChildTerminalEvidence::Producer { .. } | ChildTerminalEvidence::ProducerFact { .. } | ChildTerminalEvidence::Recorded { .. })
                | (ChildTerminalEvidence::ProducerFact { .. } | ChildTerminalEvidence::Recorded { .. }, ChildTerminalEvidence::Producer { .. }) => false,
            };
            if !compatible {
                if producer_identity(current).is_some() && producer_identity(terminal).is_some() { producer_conflict = true; }
                issues.push(InventoryIssue::TerminalConflict { sources: current.sources().iter().cloned().chain(std::iter::once(record.source.clone())).collect() });
                continue;
            }
            let before = issues.len();
            match current {
                ChildTerminalEvidence::ProducerFact { recorded, sources, .. } => {
                    union_sources(sources, terminal.sources());
                    union_sources(sources, std::slice::from_ref(&record.source));
                    if let Some(addition) = terminal.recorded() {
                        match recorded {
                            Some(base) => merge_recorded(base, addition, issues),
                            None => *recorded = Some(addition.clone()),
                        }
                    }
                    if producer_identity(terminal).is_some() && issues.len() != before { producer_conflict = true; }
                }
                ChildTerminalEvidence::Recorded { result } => {
                    if let Some(addition) = terminal.recorded() { merge_recorded(result, addition, issues); }
                    union_sources(&mut result.sources, std::slice::from_ref(&record.source));
                }
                ChildTerminalEvidence::Producer { .. } => {}
            }
        }
        if rank == TerminalRank::Recorded && issues.len() != issue_count {
            if let (Some(ChildTerminalEvidence::ProducerFact { recorded, .. }), Some(ChildTerminalEvidence::ProducerFact { recorded: original, .. })) = (&mut selected, original) {
                *recorded = original;
            }
        }
    }
    for (i, a) in compact.iter().enumerate().filter(|(_, r)| records.iter().any(|record| *record == &r.record)) {
        for b in compact.iter().skip(i + 1).filter(|r| records.iter().any(|record| *record == &r.record)) {
            if let (Some(a_digest), Some(b_digest)) = (a.producer_payload_digest, b.producer_payload_digest) {
                if a_digest != b_digest {
                    producer_conflict = true;
                    issues.push(InventoryIssue::TerminalConflict { sources: vec![a.record.source.clone(), b.record.source.clone()] });
                }
            }
            if let (Some(a_digest), Some(b_digest)) = (a.output_digest, b.output_digest) {
                if a_digest != b_digest {
                    issues.push(InventoryIssue::TerminalConflict { sources: vec![a.record.source.clone(), b.record.source.clone()] });
                    producer_conflict |= terminal_value(&a.record.value).and_then(producer_identity).is_some() && terminal_value(&b.record.value).and_then(producer_identity).is_some();
                }
            }
        }
    }
    if producer_conflict { return None; }
    selected.map(|terminal| ChildObservation::Terminal { terminal })
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SourceKey<'a> {
    ParentMeta(&'a str, &'a str),
    ParentJournal(u64, u64, u64, Option<&'a str>, &'a str),
    ParentSummary(&'a str, &'a str, &'a str),
    ChildSummary(&'a str, &'a str),
    Coordinator(uuid::Uuid, u64),
}

fn source_key(source: &ChildEvidenceSource) -> SourceKey<'_> {
    match source {
        ChildEvidenceSource::ParentMeta { subagent_id, digest } => SourceKey::ParentMeta(subagent_id, digest),
        ChildEvidenceSource::ParentJournal { line, byte_offset, byte_len, event_id, digest } => SourceKey::ParentJournal(*line, *byte_offset, *byte_len, event_id.as_deref(), digest),
        ChildEvidenceSource::ParentSummary { digest, completion_id } => SourceKey::ParentSummary(&completion_id.subagent_id, completion_id.attempt_id.as_str(), digest),
        ChildEvidenceSource::ChildSummary { child_session_id, digest } => SourceKey::ChildSummary(child_session_id, digest),
        ChildEvidenceSource::Coordinator { version } => SourceKey::Coordinator(version.incarnation, version.sequence),
    }
}

fn union_sources(base: &mut Vec<ChildEvidenceSource>, other: &[ChildEvidenceSource]) {
    for source in other { if !base.contains(source) { base.push(source.clone()); } }
    base.sort_by(|a, b| source_key(a).cmp(&source_key(b)));
}

fn merge_optional<T: Clone + PartialEq>(base: &mut Option<T>, other: &Option<T>) -> bool {
    match (&*base, other) {
        (Some(a), Some(b)) => a == b,
        (None, Some(value)) => { *base = Some(value.clone()); true }
        (Some(_), None) | (None, None) => true,
    }
}

fn merge_state(base: &mut RecordedTerminalState, other: &RecordedTerminalState) -> bool {
    match (base, other) {
        (RecordedTerminalState::Completed, RecordedTerminalState::Completed) => true,
        (RecordedTerminalState::Failed { message: a }, RecordedTerminalState::Failed { message: b }) => merge_optional(a, b),
        (RecordedTerminalState::Cancelled { cause: ac, message: am }, RecordedTerminalState::Cancelled { cause: bc, message: bm }) => {
            let cause = merge_optional(ac, bc);
            let message = merge_optional(am, bm);
            cause && message
        }
        (RecordedTerminalState::Completed, RecordedTerminalState::Failed { .. } | RecordedTerminalState::Cancelled { .. })
        | (RecordedTerminalState::Failed { .. }, RecordedTerminalState::Completed | RecordedTerminalState::Cancelled { .. })
        | (RecordedTerminalState::Cancelled { .. }, RecordedTerminalState::Completed | RecordedTerminalState::Failed { .. }) => false,
    }
}

fn merge_recorded(base: &mut RecordedSubagentTerminal, other: &RecordedSubagentTerminal, issues: &mut Vec<InventoryIssue>) {
    if base == other { return; }
    if base.attempt != other.attempt {
        issues.push(InventoryIssue::TerminalConflict { sources: base.sources.iter().chain(&other.sources).cloned().collect() });
        return;
    }
    match &base.attempt {
        AttemptEvidence::Known(_) => {}
        AttemptEvidence::Legacy | AttemptEvidence::NotStarted => {
            let mut comparable = other.clone();
            comparable.sources = base.sources.clone();
            if *base != comparable {
                issues.push(InventoryIssue::TerminalConflict { sources: base.sources.iter().chain(&other.sources).cloned().collect() });
            } else {
                union_sources(&mut base.sources, &other.sources);
            }
            return;
        }
    }
    macro_rules! merge {
        ($field:ident) => {
            match (&base.$field, &other.$field) {
                (Some(a), Some(b)) if a != b => issues.push(InventoryIssue::TerminalConflict { sources: base.sources.iter().chain(&other.sources).cloned().collect() }),
                (None, Some(value)) => base.$field = Some(value.clone()),
                (Some(_), Some(_)) | (Some(_), None) | (None, None) => {}
            }
        };
    }
    if !merge_state(&mut base.state, &other.state) {
        issues.push(InventoryIssue::TerminalConflict { sources: base.sources.iter().chain(&other.sources).cloned().collect() });
    }
    merge!(completed_at); merge!(duration_ms); merge!(tool_calls); merge!(turns); merge!(tokens_used);
    union_sources(&mut base.sources, &other.sources);
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
