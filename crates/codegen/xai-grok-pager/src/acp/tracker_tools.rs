use super::*;
use crate::acp::tool_clock::{ToolClockAdmission, ToolExecutionClock};
use crate::acp::tool_meta::{ToolIdentity, ToolMetadata, ToolSource};
use uuid::Uuid;
use xai_grok_shell::extensions::tool_timing::{ToolExecutionId, ToolExecutionTiming};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ToolKey {
    Legacy(String),
    Registered(Uuid),
}

#[derive(Debug, Default)]
pub(super) enum ToolProjection {
    #[default]
    AwaitingBase,
    Hidden,
    Entry(EntryId),
}

#[derive(Debug)]
pub(super) struct ToolInstance {
    identity: ToolIdentity,
    clock: ToolExecutionClock,
    projection: ToolProjection,
    background_tasks: Vec<String>,
}

#[derive(Debug)]
enum LegacyProvider {
    Seen,
    Reused,
}

#[derive(Debug, Default)]
pub(super) struct ToolInstances {
    calls: HashMap<Uuid, ToolInstance>,
    providers: HashMap<String, Vec<Uuid>>,
    executions: HashMap<ToolExecutionId, Uuid>,
    open: HashSet<Uuid>,
    legacy_providers: HashMap<String, LegacyProvider>,
    entries: HashMap<EntryId, Vec<Uuid>>,
    data_frontiers: HashMap<EntryId, u64>,
}

pub(super) enum ToolProjectionAdmission {
    Current,
    Detached,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolAdmission {
    Reject,
    Turn,
    EntryOnly,
}

pub(crate) struct CurrentToolProjection {
    invocation: Uuid,
    phase: CurrentToolPhase,
}

enum CurrentToolPhase {
    Preparing,
    Executing,
    Settling,
}

impl CurrentToolProjection {
    pub(crate) fn from_current(
        tool: &xai_grok_shell::session::current_actions::CurrentTool,
    ) -> Option<Self> {
        use xai_grok_shell::extensions::tool_timing::ToolCallOwnership;
        let ToolCallOwnership::Registered { invocation_id, .. } = &tool.ownership else {
            return None;
        };
        let invocation = Uuid::parse_str(invocation_id)
            .ok()
            .filter(|id| !id.is_nil())?;
        let phase = if tool
            .executions
            .iter()
            .any(|span| matches!(span, ToolExecutionTiming::Started { .. }))
        {
            CurrentToolPhase::Executing
        } else if tool.executions.is_empty() {
            CurrentToolPhase::Preparing
        } else {
            CurrentToolPhase::Settling
        };
        Some(Self { invocation, phase })
    }

    pub(crate) fn activity(&self) -> TurnActivity {
        TurnActivity::Waiting(match self.phase {
            CurrentToolPhase::Preparing => WaitingReason::ToolPreparation,
            CurrentToolPhase::Executing => WaitingReason::ToolExecution,
            CurrentToolPhase::Settling => WaitingReason::ToolSettlement,
        })
    }
}

impl AcpUpdateTracker {
    pub(crate) fn accepts_known_tool_identity(&self, meta: &NotificationMeta) -> bool {
        match &meta.tool {
            ToolMetadata::Owned { identity, timing } => self
                .tool_instances
                .calls
                .get(&identity.invocation)
                .is_none_or(|known| known.identity == *identity && known.clock.accepts(*timing)),
            ToolMetadata::Malformed => false,
            ToolMetadata::Legacy | ToolMetadata::NotTool => true,
        }
    }

    pub(crate) fn current_tool_is_admissible(
        &self,
        tool: &xai_grok_shell::session::current_actions::CurrentTool,
    ) -> bool {
        let Some(identity) = ToolIdentity::from_ownership(&tool.ownership) else {
            return false;
        };
        self.tool_instances
            .calls
            .get(&identity.invocation)
            .is_none_or(|known| known.identity == identity && !known.clock.is_terminal())
    }

    pub(crate) fn adopt_replayed_current_tool_basis(
        &mut self,
        historical: &Self,
        tool: &xai_grok_shell::session::current_actions::CurrentTool,
        scrollback: &mut ScrollbackState,
    ) {
        let Some(identity) = ToolIdentity::from_ownership(&tool.ownership) else {
            return;
        };
        let key = ToolKey::Registered(identity.invocation);
        if self.pending_tools.contains_key(&key) {
            return;
        }
        let Some(base) = historical
            .pending_tools
            .get(&key)
            .map(|pending| pending.base.clone())
        else {
            return;
        };
        if !self.current_tool_is_admissible(tool) {
            return;
        }
        let meta = NotificationMeta {
            tool: ToolMetadata::Owned {
                identity,
                timing: ToolExecutionTiming::Unknown,
            },
            ..Default::default()
        };
        self.adopt_current_tool_call(base, &meta, tool, scrollback);
        self.observe_current_tool(tool, scrollback);
    }

    pub(crate) fn current_tool_mappings(
        &self,
        tool: &xai_grok_shell::session::current_actions::CurrentTool,
    ) -> Vec<(
        xai_grok_shell::extensions::tool_timing::MonotonicPoint,
        xai_grok_shell::extensions::tool_timing::LocalClockMapping,
    )> {
        use xai_grok_shell::extensions::tool_timing::ToolCallOwnership;
        let ToolCallOwnership::Registered { invocation_id, .. } = &tool.ownership else {
            return Vec::new();
        };
        let Ok(invocation) = Uuid::parse_str(invocation_id) else {
            return Vec::new();
        };
        let Some(instance) = self.tool_instances.calls.get(&invocation) else {
            return Vec::new();
        };
        tool.executions
            .iter()
            .filter_map(|timing| match timing {
                ToolExecutionTiming::Started {
                    execution_id,
                    started_at,
                } => instance
                    .clock
                    .execution_mapping(*execution_id, *started_at)
                    .map(|clock| (*started_at, clock)),
                ToolExecutionTiming::Unknown | ToolExecutionTiming::Finished { .. } => None,
            })
            .collect()
    }

    pub(crate) fn observe_current_tool(
        &mut self,
        tool: &xai_grok_shell::session::current_actions::CurrentTool,
        scrollback: &mut ScrollbackState,
    ) -> bool {
        let Some(identity) = ToolIdentity::from_ownership(&tool.ownership) else {
            return false;
        };
        let invocation = identity.invocation;
        let key = ToolKey::Registered(invocation);
        if self
            .tool_instances
            .calls
            .get(&invocation)
            .is_some_and(|known| known.identity != identity || known.clock.is_terminal())
        {
            return false;
        }
        let clock = self.register_tool_identity(&identity);
        if !clock.confirm_pending() {
            return false;
        }
        if let Some(base) = self
            .pending_tools
            .get(&key)
            .map(|pending| pending.base.clone())
        {
            let meta = NotificationMeta {
                tool: ToolMetadata::Owned {
                    identity: identity.clone(),
                    timing: ToolExecutionTiming::Unknown,
                },
                ..Default::default()
            };
            if !self.adopt_current_tool_call(base, &meta, tool, scrollback) {
                return false;
            }
        }
        let Some(instance) = self.tool_instances.calls.get(&invocation) else {
            return false;
        };
        if instance.identity != identity {
            return false;
        }
        let clock = instance.clock.clone();
        let mut changed = false;
        for timing in &tool.executions {
            let execution = match timing {
                ToolExecutionTiming::Unknown => continue,
                ToolExecutionTiming::Started { execution_id, .. }
                | ToolExecutionTiming::Finished { execution_id, .. } => *execution_id,
            };
            if self
                .tool_instances
                .executions
                .get(&execution)
                .is_some_and(|known| *known != invocation)
                || !clock.accepts(*timing)
            {
                continue;
            }
            changed |= clock.apply(*timing, ToolClockAdmission::Current);
            if matches!(timing, ToolExecutionTiming::Started { .. }) {
                changed |= clock.confirm_current(execution);
            }
            self.tool_instances.executions.insert(execution, invocation);
        }
        if changed {
            self.invalidate_tool_projection(&key, scrollback);
        }
        changed
    }

    pub(crate) fn current_tool_activity(&self, tool: &CurrentToolProjection) -> TurnActivity {
        match tool.phase {
            CurrentToolPhase::Preparing | CurrentToolPhase::Settling => tool.activity(),
            CurrentToolPhase::Executing => self
                .pending_tools
                .get(&ToolKey::Registered(tool.invocation))
                .map_or_else(
                    || tool.activity(),
                    |pending| self.pending_tool_activity(pending),
                ),
        }
    }

    pub fn background_pending_tool(
        &mut self,
        tool_call_id: &str,
        task_id: &str,
        meta: &NotificationMeta,
    ) {
        let Some(key) = self.tool_key(tool_call_id, meta) else {
            return;
        };
        if let ToolKey::Registered(id) = &key
            && let Some(call) = self.tool_instances.calls.get_mut(id)
            && !call.background_tasks.iter().any(|task| task == task_id)
        {
            call.background_tasks.push(task_id.to_owned());
        }
        self.note_tool_projection(&key, ToolProjection::Hidden);
        if let Some(pending) = self.pending_tools.get_mut(&key)
            && let Some(entry) = pending.entry_id.take()
        {
            self.tool_context_ids.remove(&entry);
        }
    }

    pub(crate) fn background_tool_entry_id(
        &self,
        provider: &str,
        meta: &NotificationMeta,
    ) -> Option<EntryId> {
        if let ToolMetadata::Owned { identity, .. } = &meta.tool {
            return self
                .pending_tools
                .get(&ToolKey::Registered(identity.invocation))
                .and_then(|pending| pending.entry_id);
        }
        if self
            .tool_instances
            .providers
            .get(provider)
            .is_some_and(|ids| ids.len() != 1)
            || matches!(
                self.tool_instances.legacy_providers.get(provider),
                Some(LegacyProvider::Reused)
            )
        {
            return None;
        }
        self.pending_tool_entry_id(provider)
    }

    pub(crate) fn background_tasks_for_tool(
        &self,
        provider: &str,
        meta: &NotificationMeta,
    ) -> Option<&[String]> {
        let ToolKey::Registered(id) = self.tool_key(provider, meta)? else {
            return None;
        };
        let tasks = &self.tool_instances.calls.get(&id)?.background_tasks;
        (!tasks.is_empty()).then_some(tasks.as_slice())
    }

    pub(crate) fn has_foreground_tool_projection(
        &self,
        provider: &str,
        meta: &NotificationMeta,
    ) -> bool {
        self.tool_key(provider, meta).is_some_and(|key| {
            self.pending_tools
                .get(&key)
                .is_some_and(|pending| pending.entry_id.is_some())
        })
    }

    pub(crate) fn legacy_background_tool_is_unambiguous(
        &self,
        provider: &str,
        meta: &NotificationMeta,
    ) -> bool {
        matches!(meta.tool, ToolMetadata::Legacy | ToolMetadata::NotTool)
            && !self.tool_instances.providers.contains_key(provider)
            && !matches!(
                self.tool_instances.legacy_providers.get(provider),
                Some(LegacyProvider::Reused)
            )
    }

    pub(crate) fn bind_background_tool(
        &mut self,
        identity: &ToolIdentity,
        task_id: &str,
        historical: bool,
    ) -> bool {
        if self
            .tool_instances
            .calls
            .get(&identity.invocation)
            .is_some_and(|known| known.identity != *identity)
        {
            return false;
        }
        let existed = self.tool_instances.calls.contains_key(&identity.invocation);
        let clock = self.register_tool_identity(identity);
        if historical && !existed {
            clock.close();
            self.tool_instances.open.remove(&identity.invocation);
        }
        let Some(call) = self.tool_instances.calls.get_mut(&identity.invocation) else {
            return false;
        };
        if !call.background_tasks.iter().any(|id| id == task_id) {
            call.background_tasks.push(task_id.to_owned());
        }
        true
    }

    pub(crate) fn settle_background_tool_update(
        &mut self,
        update: &acp::ToolCallUpdate,
        meta: &NotificationMeta,
        scrollback: &mut ScrollbackState,
    ) {
        if !matches!(
            update.fields.status,
            Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)
        ) {
            return;
        }
        let Some(key) = self.tool_key(update.tool_call_id.0.as_ref(), meta) else {
            return;
        };
        if self
            .pending_tools
            .get(&key)
            .is_some_and(|pending| pending.entry_id.is_some())
        {
            self.handle_tool_observation(
                acp::SessionUpdate::ToolCallUpdate(update.clone()),
                meta,
                scrollback,
                None,
            );
            return;
        }
        self.pending_tools.remove(&key);
        self.suppressed_tools.remove(&key);
        self.blocking_waits.remove(&key);
        if let ToolKey::Registered(id) = key {
            if let Some(call) = self.tool_instances.calls.get(&id) {
                call.clock.settle();
            }
            self.tool_instances.open.remove(&id);
        }
    }

    pub(crate) fn active_projection_entries(&self) -> HashSet<EntryId> {
        self.current_agent_msg
            .into_iter()
            .chain(self.current_thinking)
            .chain(
                self.pending_tools
                    .values()
                    .filter_map(|pending| pending.entry_id),
            )
            .collect()
    }

    pub(crate) fn registered_tool_entry(&self, invocation: Uuid) -> Option<EntryId> {
        match self.tool_instances.calls.get(&invocation)?.projection {
            ToolProjection::Entry(entry) => Some(entry),
            ToolProjection::AwaitingBase | ToolProjection::Hidden => None,
        }
    }

    pub(crate) fn replay_entry_replacements(&self, live: &Self) -> HashMap<EntryId, Vec<EntryId>> {
        let mut live_events: HashMap<&String, Vec<EntryId>> = HashMap::new();
        for (entry, event) in &live.transcript_events {
            live_events.entry(event).or_default().push(*entry);
        }
        let mut replacements: HashMap<EntryId, Vec<EntryId>> = HashMap::new();
        for (historical, event) in &self.transcript_events {
            if let Some(current) = live_events.get(event) {
                replacements.entry(*historical).or_default().extend(current);
            }
        }
        for (prompt, historical) in &self.prompt_entries {
            if let Some(current) = live.prompt_entries.get(prompt) {
                for historical in historical {
                    replacements.entry(*historical).or_default().extend(current);
                }
            }
        }
        for entries in replacements.values_mut() {
            entries.sort_by_key(|entry| entry.value());
            entries.dedup();
        }
        for (id, past) in &self.tool_instances.calls {
            if let ToolProjection::Entry(historical) = past.projection
                && let Some(ToolInstance {
                    identity,
                    projection: ToolProjection::Entry(current),
                    ..
                }) = live.tool_instances.calls.get(id)
                && identity == &past.identity
            {
                let entries = replacements.entry(historical).or_default();
                if !entries.contains(current) {
                    entries.push(*current);
                }
            }
        }
        replacements
    }

    pub(crate) fn knows_tool_observation(
        &self,
        update: &acp::SessionUpdate,
        meta: &NotificationMeta,
    ) -> bool {
        let provider = match update {
            acp::SessionUpdate::ToolCall(call) => call.tool_call_id.0.as_ref(),
            acp::SessionUpdate::ToolCallUpdate(call) => call.tool_call_id.0.as_ref(),
            _ => return false,
        };
        match self.tool_key(provider, meta) {
            Some(ToolKey::Registered(id)) => self.tool_instances.calls.contains_key(&id),
            Some(ToolKey::Legacy(_)) | None => false,
        }
    }

    pub(crate) fn tool_observation_is_current(
        &self,
        update: &acp::SessionUpdate,
        meta: &NotificationMeta,
        prompt: Option<&str>,
    ) -> bool {
        let provider = match update {
            acp::SessionUpdate::ToolCall(call) => call.tool_call_id.0.as_ref(),
            acp::SessionUpdate::ToolCallUpdate(call) => call.tool_call_id.0.as_ref(),
            _ => return false,
        };
        let source = match &meta.tool {
            ToolMetadata::Owned { identity, .. } => Some(&identity.source),
            ToolMetadata::NotTool | ToolMetadata::Legacy => match self.tool_key(provider, meta) {
                Some(ToolKey::Registered(id)) => self
                    .tool_instances
                    .calls
                    .get(&id)
                    .map(|call| &call.identity.source),
                Some(ToolKey::Legacy(_)) | None => None,
            },
            ToolMetadata::Malformed => None,
        };
        match source {
            Some(ToolSource::Sampling(owner)) => prompt == Some(owner.prompt_id.as_str()),
            Some(ToolSource::Unknown) | None => false,
        }
    }

    pub(super) fn note_legacy_tool_registration(&mut self, key: &ToolKey) {
        if let ToolKey::Legacy(provider) = key {
            self.tool_instances
                .legacy_providers
                .entry(provider.clone())
                .and_modify(|state| *state = LegacyProvider::Reused)
                .or_insert(LegacyProvider::Seen);
        }
    }

    fn register_tool_identity(&mut self, identity: &ToolIdentity) -> ToolExecutionClock {
        let ToolInstances {
            calls,
            providers,
            open,
            ..
        } = &mut self.tool_instances;
        calls
            .entry(identity.invocation)
            .or_insert_with(|| {
                providers
                    .entry(identity.provider.clone())
                    .or_default()
                    .push(identity.invocation);
                open.insert(identity.invocation);
                ToolInstance {
                    identity: identity.clone(),
                    clock: ToolExecutionClock::new(identity.invocation),
                    projection: ToolProjection::AwaitingBase,
                    background_tasks: Vec::new(),
                }
            })
            .clock
            .clone()
    }

    /// The caller validates the snapshot binding, load generation and ordered frontier first.
    pub(crate) fn adopt_current_tool_call(
        &mut self,
        call: acp::ToolCall,
        meta: &NotificationMeta,
        proof: &xai_grok_shell::session::current_actions::CurrentTool,
        scrollback: &mut ScrollbackState,
    ) -> bool {
        if meta.is_replay
            || !matches!(
                call.status,
                acp::ToolCallStatus::Pending | acp::ToolCallStatus::InProgress
            )
        {
            return false;
        }
        let ToolMetadata::Owned { identity, timing } = &meta.tool else {
            return false;
        };
        if ToolIdentity::from_ownership(&proof.ownership).as_ref() != Some(identity)
            || call.tool_call_id.0.as_ref() != identity.provider
        {
            return false;
        }
        if let Some(existing) = self.tool_instances.calls.get(&identity.invocation)
            && (existing.identity != *identity
                || existing.clock.is_terminal()
                || !existing.clock.accepts(*timing))
        {
            return false;
        }
        let execution = match timing {
            ToolExecutionTiming::Unknown => None,
            ToolExecutionTiming::Started { execution_id, .. }
            | ToolExecutionTiming::Finished { execution_id, .. } => Some(*execution_id),
        };
        if execution.is_some_and(|id| {
            self.tool_instances
                .executions
                .get(&id)
                .is_some_and(|owner| *owner != identity.invocation)
        }) {
            return false;
        }
        let key = ToolKey::Registered(identity.invocation);
        let clock = self.register_tool_identity(identity);
        if !clock.confirm_pending() {
            return false;
        }
        clock.apply(*timing, ToolClockAdmission::Current);
        if let ToolExecutionTiming::Started { execution_id, .. } = timing {
            clock.confirm_current(*execution_id);
        }
        if let Some(execution) = execution {
            self.tool_instances
                .executions
                .insert(execution, identity.invocation);
        }
        self.tool_instances.open.insert(identity.invocation);
        if let Some(pending) = self.pending_tools.get(&key) {
            if let Some(entry) = pending.entry_id {
                scrollback.set_entry_running(entry, true);
            }
            return true;
        }
        if let Some(entry) = self
            .tool_projection(&key)
            .filter(|id| scrollback.get_by_id(*id).is_some())
        {
            let block = tool_call_with_clock(
                &call,
                self.session_cwd.as_deref(),
                &self.subagent_labels.borrow(),
                &clock,
            );
            scrollback.replace_tool_block(entry, block, Some(clock.clone()));
            scrollback.set_entry_running(entry, true);
            self.pending_tools.insert(
                key,
                PendingTool {
                    entry_id: Some(entry),
                    context_id: xai_grok_shell::session::ToolCallId::new(
                        identity.provider.as_str(),
                    ),
                    base: call,
                    utf8_decoder: Utf8Decoder::default(),
                    execution_clock: clock,
                },
            );
            true
        } else {
            self.handle_tool_call(
                call,
                scrollback,
                false,
                key,
                clock,
                ToolProjectionAdmission::Detached,
            )
        }
    }

    pub(super) fn close_tool_clocks(&mut self) {
        for id in self.tool_instances.open.drain() {
            if let Some(call) = self.tool_instances.calls.get(&id) {
                call.clock.close();
            }
        }
    }

    pub(super) fn has_registered_provider(&self, provider: &str) -> bool {
        self.tool_instances.providers.contains_key(provider)
    }

    pub(super) fn is_registered_tool_update(
        &self,
        update: &acp::SessionUpdate,
        meta: &NotificationMeta,
    ) -> bool {
        match &meta.tool {
            ToolMetadata::Owned { .. } => true,
            ToolMetadata::Malformed => false,
            ToolMetadata::NotTool | ToolMetadata::Legacy => match update {
                acp::SessionUpdate::ToolCallUpdate(call) => {
                    self.has_registered_provider(call.tool_call_id.0.as_ref())
                }
                _ => false,
            },
        }
    }

    pub(super) fn tool_key(&self, provider: &str, meta: &NotificationMeta) -> Option<ToolKey> {
        match &meta.tool {
            ToolMetadata::Owned { identity, .. } => Some(ToolKey::Registered(identity.invocation)),
            ToolMetadata::Malformed => None,
            ToolMetadata::NotTool | ToolMetadata::Legacy => {
                let Some(ids) = self.tool_instances.providers.get(provider) else {
                    return Some(ToolKey::Legacy(provider.to_owned()));
                };
                if ids.len() != 1 || self.tool_instances.legacy_providers.contains_key(provider) {
                    return None;
                }
                let mut candidates = ids.iter().filter(|id| {
                    self.tool_instances.calls.get(id).is_some_and(|call| {
                        match &call.identity.source {
                            ToolSource::Sampling(owner) => {
                                meta.prompt_id.as_ref() == Some(&owner.prompt_id)
                            }
                            ToolSource::Unknown => ids.len() == 1,
                        }
                    })
                });
                let id = *candidates.next()?;
                if candidates.next().is_some() {
                    return None;
                }
                Some(ToolKey::Registered(id))
            }
        }
    }

    pub(crate) fn tool_admission(
        &self,
        update: &acp::SessionUpdate,
        meta: &NotificationMeta,
        current_prompt: Option<&str>,
    ) -> ToolAdmission {
        let provider = match update {
            acp::SessionUpdate::ToolCall(call) => call.tool_call_id.0.as_ref(),
            acp::SessionUpdate::ToolCallUpdate(call) => call.tool_call_id.0.as_ref(),
            _ => return ToolAdmission::Turn,
        };
        let Some(key) = self.tool_key(provider, meta) else {
            return ToolAdmission::Reject;
        };
        if matches!(meta.tool, ToolMetadata::NotTool | ToolMetadata::Legacy)
            && matches!(update, acp::SessionUpdate::ToolCallUpdate(_))
            && matches!(
                self.tool_instances.legacy_providers.get(provider),
                Some(LegacyProvider::Reused)
            )
        {
            return ToolAdmission::Reject;
        }
        match &meta.tool {
            ToolMetadata::Malformed => ToolAdmission::Reject,
            ToolMetadata::Owned { identity, timing } => {
                if let Some(call) = self.tool_instances.calls.get(&identity.invocation) {
                    if call.identity != *identity || !call.clock.accepts(*timing) {
                        return ToolAdmission::Reject;
                    }
                    if matches!(update, acp::SessionUpdate::ToolCall(_))
                        && !matches!(call.projection, ToolProjection::AwaitingBase)
                    {
                        return ToolAdmission::Reject;
                    }
                } else if !meta.is_replay && self.retired_tool_source(&identity.source) {
                    return ToolAdmission::Reject;
                }
                let execution = match timing {
                    ToolExecutionTiming::Unknown => None,
                    ToolExecutionTiming::Started { execution_id, .. }
                    | ToolExecutionTiming::Finished { execution_id, .. } => Some(execution_id),
                };
                if execution.is_some_and(|id| {
                    self.tool_instances
                        .executions
                        .get(id)
                        .is_some_and(|owner| *owner != identity.invocation)
                }) {
                    return ToolAdmission::Reject;
                }
                if execution.is_some() {
                    return ToolAdmission::EntryOnly;
                }
                match &identity.source {
                    ToolSource::Unknown => ToolAdmission::EntryOnly,
                    ToolSource::Sampling(_) if self.retired_tool_source(&identity.source) => {
                        ToolAdmission::EntryOnly
                    }
                    ToolSource::Sampling(owner)
                        if current_prompt != Some(owner.prompt_id.as_str())
                            && (self.tool_instances.calls.contains_key(&identity.invocation)
                                || self.has_admitted_live_prompt(&owner.prompt_id)) =>
                    {
                        ToolAdmission::EntryOnly
                    }
                    ToolSource::Sampling(_) => ToolAdmission::Turn,
                }
            }
            ToolMetadata::NotTool | ToolMetadata::Legacy => match key {
                ToolKey::Legacy(_) => ToolAdmission::Turn,
                ToolKey::Registered(id) => {
                    let Some(call) = self.tool_instances.calls.get(&id) else {
                        return ToolAdmission::Reject;
                    };
                    match &call.identity.source {
                        ToolSource::Sampling(owner)
                            if current_prompt == Some(owner.prompt_id.as_str()) =>
                        {
                            ToolAdmission::Turn
                        }
                        ToolSource::Sampling(_) | ToolSource::Unknown => ToolAdmission::EntryOnly,
                    }
                }
            },
        }
    }

    fn retired_tool_source(&self, source: &ToolSource) -> bool {
        match source {
            ToolSource::Unknown => false,
            ToolSource::Sampling(owner) => self.tool_source_is_retired(owner),
        }
    }

    pub(crate) fn handle_tool_observation(
        &mut self,
        update: acp::SessionUpdate,
        meta: &NotificationMeta,
        scrollback: &mut ScrollbackState,
        current_prompt: Option<&str>,
    ) -> bool {
        if matches!(
            self.tool_admission(&update, meta, current_prompt),
            ToolAdmission::Reject
        ) {
            return false;
        }
        let provider = match &update {
            acp::SessionUpdate::ToolCall(call) => call.tool_call_id.0.as_ref(),
            acp::SessionUpdate::ToolCallUpdate(call) => call.tool_call_id.0.as_ref(),
            _ => return false,
        };
        let Some(key) = self.tool_key(provider, meta) else {
            return false;
        };
        let prompt = match &meta.tool {
            ToolMetadata::Owned {
                identity:
                    ToolIdentity {
                        source: ToolSource::Sampling(owner),
                        ..
                    },
                ..
            } => Some(owner.prompt_id.as_str()),
            ToolMetadata::Owned {
                identity:
                    ToolIdentity {
                        source: ToolSource::Unknown,
                        ..
                    },
                ..
            }
            | ToolMetadata::Legacy
            | ToolMetadata::NotTool
            | ToolMetadata::Malformed => meta.prompt_id.as_deref(),
        };
        if let Some(prompt) = prompt {
            self.bind_user_projection(prompt);
        }
        let clock = match &meta.tool {
            ToolMetadata::Owned { identity, .. } => Some(self.register_tool_identity(identity)),
            ToolMetadata::Legacy | ToolMetadata::NotTool => match &key {
                ToolKey::Registered(id) => self
                    .tool_instances
                    .calls
                    .get(id)
                    .map(|call| call.clock.clone()),
                ToolKey::Legacy(_) => None,
            },
            ToolMetadata::Malformed => return false,
        }
        .unwrap_or_default();
        let mut timing_changed = false;
        if let ToolMetadata::Owned { identity, timing } = &meta.tool {
            let live = !meta.is_replay
                && match &identity.source {
                    ToolSource::Unknown => false,
                    ToolSource::Sampling(owner) => current_prompt == Some(owner.prompt_id.as_str()),
                };
            timing_changed = clock.apply(
                *timing,
                if live {
                    ToolClockAdmission::Current
                } else {
                    ToolClockAdmission::Unconfirmed
                },
            );
            match timing {
                ToolExecutionTiming::Unknown => {}
                ToolExecutionTiming::Started { execution_id, .. }
                | ToolExecutionTiming::Finished { execution_id, .. } => {
                    self.tool_instances
                        .executions
                        .insert(*execution_id, identity.invocation);
                }
            }
        }
        // Timing-only publications must not replay the base, stream metadata or phase transitions.
        if let acp::SessionUpdate::ToolCallUpdate(call) = &update
            && call.fields.kind.is_none()
            && call.fields.status.is_none()
            && call.fields.title.is_none()
            && call.fields.name.is_none()
            && call.fields.content.is_none()
            && call.fields.locations.is_none()
            && call.fields.raw_input.is_none()
            && call.fields.raw_output.is_none()
        {
            if timing_changed {
                self.invalidate_tool_projection(&key, scrollback);
            }
            return timing_changed;
        }
        if let acp::SessionUpdate::ToolCallUpdate(call) = &update
            && matches!(
                call.fields.status,
                Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)
            )
        {
            clock.settle();
            if let ToolKey::Registered(id) = &key {
                self.tool_instances.open.remove(id);
            }
        }
        let changed = match update {
            acp::SessionUpdate::ToolCall(call) => {
                if let ToolKey::Registered(id) = &key
                    && self.tool_instances.calls.get(id).is_some_and(|call| {
                        !matches!(call.projection, ToolProjection::AwaitingBase)
                    })
                {
                    return timing_changed;
                }
                let affects_turn = !meta.is_replay
                    && match &meta.tool {
                        ToolMetadata::Owned { identity, .. } => match &identity.source {
                            ToolSource::Sampling(owner) => {
                                current_prompt == Some(owner.prompt_id.as_str())
                                    && !self.tool_source_is_retired(owner)
                            }
                            ToolSource::Unknown => false,
                        },
                        ToolMetadata::NotTool | ToolMetadata::Legacy | ToolMetadata::Malformed => {
                            false
                        }
                    };
                self.handle_tool_call(
                    call,
                    scrollback,
                    meta.is_replay,
                    key.clone(),
                    clock.clone(),
                    if affects_turn {
                        ToolProjectionAdmission::Current
                    } else {
                        ToolProjectionAdmission::Detached
                    },
                )
            }
            acp::SessionUpdate::ToolCallUpdate(call) => {
                if let Some(id) = self.tool_projection(&key) {
                    if !self.pending_tools.contains_key(&key)
                        && !self.suppressed_tools.contains_key(&key)
                    {
                        let completion = call.fields.status.and_then(|status| {
                            match crate::acp::meta::tool_completion(status, call.meta.as_ref()) {
                                ToolCompletion::Unconfirmed => None,
                                completion @ (ToolCompletion::Completed
                                | ToolCompletion::Failed
                                | ToolCompletion::Cancelled) => Some(completion),
                            }
                        });
                        let outcome_changed = if let Some(completion) = completion {
                            clock.settle();
                            if scrollback.get_by_id(id).is_some_and(|entry| {
                                entry.tool_completion() == ToolCompletion::Unconfirmed
                            }) {
                                scrollback.set_tool_completion(id, completion);
                                scrollback.finish_running(id);
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if timing_changed || outcome_changed {
                            self.invalidate_tool_projection(&key, scrollback);
                        }
                        return timing_changed || outcome_changed;
                    }
                }
                self.handle_tool_call_update(
                    call,
                    scrollback,
                    meta.is_replay,
                    key.clone(),
                    clock.clone(),
                )
            }
            _ => false,
        };
        self.invalidate_tool_projection(&key, scrollback);
        if changed
            && let Some(entry) = self.tool_projection(&key)
            && let Some(sequence) = meta.event_seq
        {
            self.tool_instances
                .data_frontiers
                .entry(entry)
                .and_modify(|old| *old = (*old).max(sequence))
                .or_insert(sequence);
        }
        if changed
            && !meta.is_replay
            && meta.prompt_id.as_deref() == current_prompt
            && current_prompt.is_some()
        {
            self.bump_agent_output_epoch();
        }
        changed || timing_changed
    }

    fn tool_projection(&self, key: &ToolKey) -> Option<EntryId> {
        match key {
            ToolKey::Legacy(_) => None,
            ToolKey::Registered(id) => match &self.tool_instances.calls.get(id)?.projection {
                ToolProjection::Entry(id) => Some(*id),
                ToolProjection::AwaitingBase | ToolProjection::Hidden => None,
            },
        }
    }

    pub(super) fn note_tool_projection(&mut self, key: &ToolKey, projection: ToolProjection) {
        let ToolKey::Registered(id) = key else {
            return;
        };
        let Some(call) = self.tool_instances.calls.get_mut(id) else {
            return;
        };
        if let ToolProjection::Entry(previous) = call.projection
            && let Some(ids) = self.tool_instances.entries.get_mut(&previous)
        {
            ids.retain(|owner| owner != id);
        }
        if let ToolProjection::Entry(entry) = projection {
            self.tool_instances
                .entries
                .entry(entry)
                .or_default()
                .push(*id);
        }
        call.projection = projection;
        if call.clock.is_terminal() {
            self.tool_instances.open.remove(id);
        }
    }

    pub(super) fn merge_tool_projections(&mut self, survivor: EntryId, removed: EntryId) {
        if let Some(sequence) = self.tool_instances.data_frontiers.remove(&removed) {
            self.tool_instances
                .data_frontiers
                .entry(survivor)
                .and_modify(|old| *old = (*old).max(sequence))
                .or_insert(sequence);
        }
        if let Some(ids) = self.tool_instances.entries.remove(&removed) {
            for id in &ids {
                if let Some(call) = self.tool_instances.calls.get_mut(id) {
                    call.projection = ToolProjection::Entry(survivor);
                }
            }
            self.tool_instances
                .entries
                .entry(survivor)
                .or_default()
                .extend(ids);
        }
    }

    fn invalidate_tool_projection(&self, key: &ToolKey, scrollback: &mut ScrollbackState) {
        if let Some(id) = self.tool_projection(key) {
            scrollback.mark_structurally_dirty(id);
            if let Some(entry) = scrollback.get_by_id_mut(id) {
                entry.invalidate_cache();
            }
        }
    }

    pub(super) fn provider_key(&self, provider: &str) -> ToolKey {
        match self.tool_instances.providers.get(provider).and_then(|ids| {
            ids.iter()
                .rev()
                .find(|id| self.pending_tools.contains_key(&ToolKey::Registered(**id)))
        }) {
            Some(id) => ToolKey::Registered(*id),
            None => ToolKey::Legacy(provider.to_owned()),
        }
    }
}

#[path = "tracker_rebuild.rs"]
mod rebuild;
