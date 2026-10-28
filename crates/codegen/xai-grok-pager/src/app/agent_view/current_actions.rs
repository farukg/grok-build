use super::AgentView;
use crate::acp::meta::NotificationMeta;
use crate::acp::tracker::{CurrentToolProjection, TurnActivity, WaitingReason};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use xai_grok_shell::extensions::tool_timing::{
    LocalClockMapping, MonotonicPoint, ToolExecutionTiming,
};
use xai_grok_shell::session::current_actions::{
    CompactOwner, CompactStage, CurrentActionTransition, CurrentActions, CurrentActionsEvent,
    CurrentActionsSnapshot, CurrentCompact, CurrentHook, CurrentInteraction, CurrentSampling,
    CurrentSamplingPhase, CurrentTool, CurrentTurn, CurrentTurnPhase, FrozenOwner,
    InteractionSource, SnapshotBinding, SourceVersion,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentActionsContext {
    pub(crate) binding: SnapshotBinding,
    pub(crate) incarnation: uuid::Uuid,
}

#[derive(Debug, Clone)]
pub enum CurrentActionsReadContext {
    Attach(CurrentActionsContext),
    Observe(CurrentActionsContext),
    Unavailable(SnapshotBinding),
}

impl CurrentActionsReadContext {
    pub(crate) fn binding(&self) -> &SnapshotBinding {
        match self {
            Self::Attach(context) | Self::Observe(context) => &context.binding,
            Self::Unavailable(binding) => binding,
        }
    }
}

pub(crate) enum CurrentActionsProjection {
    Unavailable {
        latest_live: Option<SourceVersion>,
        native_frontier: Option<u64>,
        settlements: PendingPromptSettlements,
    },
    Bound(Box<BoundSource>),
}

impl Default for CurrentActionsProjection {
    fn default() -> Self {
        Self::Unavailable {
            latest_live: None,
            native_frontier: None,
            settlements: PendingPromptSettlements::None,
        }
    }
}

#[derive(Default)]
pub(crate) enum PendingPromptSettlements {
    #[default]
    None,
    OriginalRpc {
        binding: SnapshotBinding,
        prompts: HashSet<String>,
    },
}

#[derive(PartialEq, Eq, Hash)]
enum TransitionOwner {
    Task(String),
    Hook(uuid::Uuid),
    ManualCompact(uuid::Uuid),
    AutomaticCompact(uuid::Uuid),
    InlineCompact,
    Interaction(String),
}

impl TransitionOwner {
    fn compact(owner: &CompactOwner) -> Self {
        match owner {
            CompactOwner::Inline => Self::InlineCompact,
            CompactOwner::Manual { request_id, .. } => Self::ManualCompact(*request_id),
            CompactOwner::Automatic { invocation } => Self::AutomaticCompact(invocation.id.0),
        }
    }
    fn turn(owner: &FrozenOwner) -> Self {
        match owner {
            FrozenOwner::Root { task_id, .. }
            | FrozenOwner::Child { task_id, .. }
            | FrozenOwner::Wake { task_id, .. } => Self::Task(task_id.clone()),
            FrozenOwner::Unavailable => Self::Task(String::new()),
        }
    }
    fn event(event: &CurrentActionsEvent) -> Self {
        match &event.transition {
            CurrentActionTransition::TurnRegistered { turn }
            | CurrentActionTransition::TurnAnchored { turn }
            | CurrentActionTransition::TurnStarted { turn }
            | CurrentActionTransition::TurnBackendFinished { turn } => Self::turn(&turn.owner),
            CurrentActionTransition::TurnFinished { owner } => Self::turn(owner),
            CurrentActionTransition::HookStarted { hook } => Self::Hook(hook.batch_id),
            CurrentActionTransition::HookFinished { batch_id } => Self::Hook(*batch_id),
            CurrentActionTransition::CompactChanged { compact } => Self::compact(&compact.owner),
            CurrentActionTransition::CompactFinished { owner } => Self::compact(owner),
            CurrentActionTransition::InteractionStarted { interaction } => {
                Self::Interaction(interaction.tool_call_id.clone())
            }
            CurrentActionTransition::InteractionFinished { tool_call_id, .. } => {
                Self::Interaction(tool_call_id.clone())
            }
        }
    }
}

#[derive(Default)]
enum Coverage {
    #[default]
    Unavailable,
    Transitions,
    Complete,
}

pub(crate) struct BoundSource {
    context: CurrentActionsContext,
    latest_live: Option<SourceVersion>,
    native_frontier: Option<u64>,
    facts: Facts,
    selection: Selection,
    task_clock: PhaseClock,
    coverage: Coverage,
    snapshot_version: Option<SourceVersion>,
    snapshot_frontier: Option<u64>,
    turn_sequence: Option<u64>,
    transitions: HashMap<TransitionOwner, u64>,
    settled_prompts: HashSet<String>,
    clocks: Vec<(MonotonicPoint, LocalClockMapping)>,
}

enum ToolPublication {
    None,
    Base(agent_client_protocol::ToolCallStatus),
    Update(Option<agent_client_protocol::ToolCallStatus>),
}

impl ToolPublication {
    fn from_update(update: &agent_client_protocol::SessionUpdate) -> Self {
        match update {
            agent_client_protocol::SessionUpdate::ToolCall(call) => Self::Base(call.status),
            agent_client_protocol::SessionUpdate::ToolCallUpdate(call) => {
                Self::Update(call.fields.status)
            }
            _ => Self::None,
        }
    }
}

#[derive(Default)]
struct Facts {
    turn: Option<CurrentTurn>,
    sampling: Vec<CurrentSampling>,
    tools: Vec<CurrentTool>,
    compactions: Vec<CurrentCompact>,
    hooks: Vec<ObservedHook>,
    interactions: Vec<CurrentInteraction>,
}

struct ObservedHook {
    current: CurrentHook,
    // Presentation reveal only; an unmappable backend clock stays unknown.
    reveal_since: Instant,
}

#[derive(Default)]
enum Selection {
    #[default]
    Unavailable,
    Idle,
    Model(CurrentSampling, PhaseClock),
    Retry(
        CurrentSampling,
        xai_grok_shell::extensions::sampling_lifecycle::RetryClockMapping,
    ),
    Turn(CurrentTurn, PhaseClock),
    Tool(CurrentToolProjection, PhaseClock),
    Compact(CurrentCompact, PhaseClock),
    Hook {
        clock: PhaseClock,
        reveal_since: Instant,
    },
    Interaction(PhaseClock),
}

struct PhaseClock {
    mapping: LocalClockMapping,
}

impl PhaseClock {
    fn new(point: MonotonicPoint) -> Self {
        Self {
            mapping: point.map_local(),
        }
    }
    fn started_at(&self) -> Option<Instant> {
        match self.mapping {
            LocalClockMapping::Correlated { latest_at, .. } => Some(latest_at.into_std()),
            LocalClockMapping::Uncorrelated => None,
        }
    }
}

impl Selection {
    fn label(&self) -> &'static str {
        match self {
            Self::Unavailable => "source_unavailable",
            Self::Idle => "idle",
            Self::Model(_, _) => "model",
            Self::Retry(_, _) => "retry_backoff",
            Self::Turn(_, _) => "backend_response",
            Self::Tool(_, _) => "tool",
            Self::Compact(compact, _) => match compact.stage {
                CompactStage::Queued => "compaction_queued",
                CompactStage::Executing => "auto_compacting",
                CompactStage::AwaitingPersistence => "compaction_persistence",
                CompactStage::AwaitingResponse => "compaction_response",
            },
            Self::Hook { .. } => "hooks",
            Self::Interaction(_) => "user_input",
        }
    }

    fn clock(&self) -> Option<Instant> {
        match self {
            Self::Unavailable | Self::Idle | Self::Retry(_, _) => None,
            Self::Model(_, clock)
            | Self::Turn(_, clock)
            | Self::Tool(_, clock)
            | Self::Compact(_, clock)
            | Self::Interaction(clock) => clock.started_at(),
            Self::Hook {
                clock,
                reveal_since,
            } => (reveal_since.elapsed() >= crate::acp::tracker::HOOK_REVEAL_DELAY)
                .then(|| clock.started_at())
                .flatten(),
        }
    }
}

impl BoundSource {
    fn clock(&mut self, point: MonotonicPoint) -> PhaseClock {
        if let Some((_, mapping)) = self.clocks.iter().find(|(known, _)| *known == point) {
            return PhaseClock { mapping: *mapping };
        }
        let clock = PhaseClock::new(point);
        self.clocks.push((point, clock.mapping));
        clock
    }

    fn seed_tracker_clocks(&mut self, tracker: &crate::acp::tracker::AcpUpdateTracker) {
        for request in &self.facts.sampling {
            if let CurrentSamplingPhase::Submitted { started_at } = request.phase
                && let Some(mapping) = tracker.current_sampling_mapping(&request.owner, started_at)
                && !self.clocks.iter().any(|(point, _)| *point == started_at)
            {
                self.clocks.push((started_at, mapping));
            }
        }
        for tool in &self.facts.tools {
            for (point, mapping) in tracker.current_tool_mappings(tool) {
                if !self.clocks.iter().any(|(known, _)| *known == point) {
                    self.clocks.push((point, mapping));
                }
            }
        }
    }

    fn select(&mut self) {
        let mut points = Vec::new();
        if let Some(turn) = &self.facts.turn {
            points.push(turn.started_at);
            points.push(match turn.phase {
                CurrentTurnPhase::Admitted => turn.started_at,
                CurrentTurnPhase::Running { started_at }
                | CurrentTurnPhase::AwaitingResponse { started_at } => started_at,
            });
        }
        for request in &self.facts.sampling {
            match &request.phase {
                CurrentSamplingPhase::Submitted { started_at } => points.push(*started_at),
                CurrentSamplingPhase::RetryScheduled { deadline } => points.push(deadline.clock),
                CurrentSamplingPhase::Reserved => {}
            }
        }
        for compact in &self.facts.compactions {
            points.push(compact.admitted_at);
            points.push(compact.stage_started_at);
        }
        for hook in &self.facts.hooks {
            points.push(hook.current.started_at);
        }
        for interaction in &self.facts.interactions {
            if let InteractionSource::Registered { started_at, .. } = interaction.source {
                points.push(started_at);
            }
        }
        for tool in &self.facts.tools {
            points.push(tool.registered_at);
            for execution in &tool.executions {
                if let ToolExecutionTiming::Started { started_at, .. } = execution {
                    points.push(*started_at);
                }
            }
        }
        self.clocks.retain(|(point, _)| points.contains(point));
        for point in points {
            self.clock(point);
        }
        self.task_clock = match self.facts.turn.as_ref().map(|turn| turn.started_at) {
            Some(point) => self.clock(point),
            None => PhaseClock {
                mapping: LocalClockMapping::Uncorrelated,
            },
        };
        let next = if let Some(interaction) = self
            .facts
            .interactions
            .iter()
            .find(|interaction| matches!(interaction.source, InteractionSource::Registered { .. }))
            .cloned()
        {
            let point = match interaction.source {
                InteractionSource::Registered { started_at, .. } => started_at,
                InteractionSource::Unavailable => MonotonicPoint::default(),
            };
            Selection::Interaction(self.clock(point))
        } else if let Some(compact) = self
            .facts
            .compactions
            .iter()
            .find(|compact| compact.stage != CompactStage::Queued)
            .cloned()
        {
            Selection::Compact(compact.clone(), self.clock(compact.stage_started_at))
        } else if let Some((point, reveal_since)) = self
            .facts
            .hooks
            .first()
            .map(|hook| (hook.current.started_at, hook.reveal_since))
        {
            Selection::Hook {
                clock: self.clock(point),
                reveal_since,
            }
        } else if let Some(tool) = self.facts.tools.first().cloned() {
            let point = tool
                .executions
                .iter()
                .rev()
                .find_map(|execution| match execution {
                    ToolExecutionTiming::Started { started_at, .. } => Some(*started_at),
                    ToolExecutionTiming::Unknown | ToolExecutionTiming::Finished { .. } => None,
                })
                .unwrap_or_else(|| {
                    if tool.executions.is_empty() {
                        tool.registered_at
                    } else {
                        MonotonicPoint::default()
                    }
                });
            match CurrentToolProjection::from_current(&tool) {
                Some(projection) => Selection::Tool(projection, self.clock(point)),
                None => Selection::Unavailable,
            }
        } else if let Some(request) = self.facts.sampling.first().cloned() {
            match &request.phase {
                CurrentSamplingPhase::Submitted { started_at } => {
                    Selection::Model(request.clone(), self.clock(*started_at))
                }
                CurrentSamplingPhase::Reserved => {
                    Selection::Model(request.clone(), self.clock(MonotonicPoint::default()))
                }
                CurrentSamplingPhase::RetryScheduled { deadline } => {
                    use xai_grok_shell::extensions::sampling_lifecycle::RetryClockMapping;
                    let mapping = match self.clock(deadline.clock).mapping {
                        LocalClockMapping::Correlated {
                            earliest_at,
                            latest_at,
                        } => RetryClockMapping::Correlated {
                            earliest_due_at: earliest_at,
                            latest_due_at: latest_at,
                        },
                        LocalClockMapping::Uncorrelated => RetryClockMapping::Uncorrelated,
                    };
                    Selection::Retry(request.clone(), mapping)
                }
            }
        } else if let Some(turn) = self.facts.turn.clone() {
            let point = match turn.phase {
                CurrentTurnPhase::Admitted => turn.started_at,
                CurrentTurnPhase::Running { started_at }
                | CurrentTurnPhase::AwaitingResponse { started_at } => started_at,
            };
            Selection::Turn(turn.clone(), self.clock(point))
        } else if let Some(compact) = self.facts.compactions.first().cloned() {
            Selection::Compact(compact.clone(), self.clock(compact.stage_started_at))
        } else {
            match self.coverage {
                Coverage::Complete => Selection::Idle,
                Coverage::Unavailable | Coverage::Transitions => Selection::Unavailable,
            }
        };
        if self.facts.turn.is_none() {
            self.task_clock = match &next {
                Selection::Compact(compact, _) => self.clock(compact.admitted_at),
                Selection::Tool(_, _) => {
                    match self.facts.tools.first().map(|tool| tool.registered_at) {
                        Some(point) => self.clock(point),
                        None => PhaseClock {
                            mapping: LocalClockMapping::Uncorrelated,
                        },
                    }
                }
                Selection::Model(_, clock)
                | Selection::Interaction(clock)
                | Selection::Hook { clock, .. } => PhaseClock {
                    mapping: clock.mapping,
                },
                Selection::Turn(_, _)
                | Selection::Retry(_, _)
                | Selection::Unavailable
                | Selection::Idle => PhaseClock {
                    mapping: LocalClockMapping::Uncorrelated,
                },
            };
        }
        if self.selection.label() != next.label() || self.selection.clock() != next.clock() {
            crate::unified_log::debug(
                "turn.phase_transition",
                Some(&self.context.binding.session_id),
                Some(serde_json::json!({
                    "from": self.selection.label(), "to": next.label(),
                    "phase_elapsed_ms": self.selection.clock().map(|start| start.elapsed().as_millis()),
                })),
            );
        }
        self.selection = next;
    }
}

impl AgentView {
    pub(crate) fn retarget_current_actions_load(&mut self) {
        if let CurrentActionsProjection::Bound(source) = &mut self.current_actions {
            source.context.binding.load_generation = self.current_actions_generation;
            source.snapshot_frontier = None;
        }
    }

    fn protect_tool_observation(&mut self, meta: &NotificationMeta) {
        if meta.is_replay {
            return;
        }
        if let crate::acp::tool_meta::ToolMetadata::Owned { identity, .. } = &meta.tool
            && let Some(entry) = self
                .session
                .tracker
                .registered_tool_entry(identity.invocation)
        {
            self.protect_live_entry(entry);
        }
    }

    pub(crate) fn handle_admitted_acp_update(
        &mut self,
        update: agent_client_protocol::SessionUpdate,
        meta: &NotificationMeta,
    ) -> bool {
        self.observe_current_source(meta);
        let publication = ToolPublication::from_update(&update);
        self.protect_tool_observation(meta);
        let changed = self
            .session
            .handle_update(update, meta, &mut self.scrollback);
        self.protect_tool_observation(meta);
        self.observe_current_acp_tool(publication, meta);
        if !meta.is_replay {
            self.apply_pending_current_tools();
        }
        changed
    }

    pub(crate) fn observe_background_tool_update(
        &mut self,
        update: &agent_client_protocol::ToolCallUpdate,
        meta: &NotificationMeta,
    ) {
        self.observe_current_source(meta);
        self.session
            .tracker
            .settle_background_tool_update(update, meta, &mut self.scrollback);
        self.observe_current_acp_tool(ToolPublication::Update(update.fields.status), meta);
    }

    pub(crate) fn handle_admitted_tool_observation(
        &mut self,
        update: agent_client_protocol::SessionUpdate,
        meta: &NotificationMeta,
        prompt: Option<&str>,
    ) -> bool {
        self.observe_current_source(meta);
        if let agent_client_protocol::SessionUpdate::ToolCallUpdate(call) = &update
            && crate::app::acp_handler::route_bg_task_stdout(call, self, meta)
        {
            self.observe_background_tool_update(call, meta);
            return true;
        }
        let publication = ToolPublication::from_update(&update);
        self.protect_tool_observation(meta);
        let changed = self.session.tracker.handle_tool_observation(
            update,
            meta,
            &mut self.scrollback,
            prompt,
        );
        self.protect_tool_observation(meta);
        self.observe_current_acp_tool(publication, meta);
        if !meta.is_replay {
            self.apply_pending_current_tools();
        }
        changed
    }

    fn observe_current_acp_tool(&mut self, publication: ToolPublication, meta: &NotificationMeta) {
        if !self.accepts_publication_role(&meta.delivery_role) {
            return;
        }
        use crate::acp::tool_meta::{ToolMetadata, ToolSource};
        use agent_client_protocol::ToolCallStatus;
        use xai_grok_shell::extensions::tool_timing::{ToolCallOwnership, ToolCallSource};
        if meta.is_replay {
            return;
        }
        let ToolMetadata::Owned { identity, timing } = &meta.tool else {
            return;
        };
        let status = match publication {
            ToolPublication::Base(status) => Some(status),
            ToolPublication::Update(status) => status,
            ToolPublication::None => return,
        };
        let CurrentActionsProjection::Bound(source) = &mut self.current_actions else {
            return;
        };
        let invocation = identity.invocation.to_string();
        let index = source.facts.tools.iter().position(|tool| {
            matches!(&tool.ownership,
            ToolCallOwnership::Registered { invocation_id, .. } if *invocation_id == invocation)
        });
        if matches!(
            status,
            Some(ToolCallStatus::Completed | ToolCallStatus::Failed)
        ) {
            if let Some(index) = index {
                source.facts.tools.remove(index);
                source.seed_tracker_clocks(&self.session.tracker);
                source.select();
            }
            return;
        }
        let index = if let Some(index) = index {
            index
        } else {
            let ToolSource::Sampling(owner) = &identity.source else {
                return;
            };
            if self.session.current_prompt_id.as_deref() != Some(owner.prompt_id.as_str())
                || !matches!(publication, ToolPublication::Base(_))
                || source.settled_prompts.contains(&owner.prompt_id)
                || source.snapshot_version.is_some_and(|snapshot|
                    !matches!(meta.current_actions_version, crate::acp::meta::CurrentSourceMetadata::Version(version) if version.sequence > snapshot.sequence)) { return; }
            let turn = source
                .facts
                .turn
                .as_ref()
                .filter(|turn| turn.owner.prompt_id() == Some(owner.prompt_id.as_str()))
                .map_or(FrozenOwner::Unavailable, |turn| turn.owner.clone());
            source.facts.tools.push(CurrentTool {
                ownership: ToolCallOwnership::Registered {
                    invocation_id: invocation,
                    provider_call_id: identity.provider.clone(),
                    source: ToolCallSource::Sampling {
                        prompt_id: owner.prompt_id.clone(),
                        request_id: owner.request_id.as_str().to_owned(),
                        attempt: owner.attempt.0,
                    },
                },
                turn,
                registered_at: MonotonicPoint::default(),
                executions: Vec::new(),
            });
            source.facts.tools.len() - 1
        };
        let executions = &mut source.facts.tools[index].executions;
        let id = match timing {
            ToolExecutionTiming::Unknown => {
                source.seed_tracker_clocks(&self.session.tracker);
                source.select();
                return;
            }
            ToolExecutionTiming::Started { execution_id, .. }
            | ToolExecutionTiming::Finished { execution_id, .. } => execution_id,
        };
        if let Some(known) = executions.iter_mut().find(|known| match known {
            ToolExecutionTiming::Unknown => false,
            ToolExecutionTiming::Started { execution_id, .. }
            | ToolExecutionTiming::Finished { execution_id, .. } => execution_id == id,
        }) {
            if !matches!(known, ToolExecutionTiming::Finished { .. }) {
                *known = *timing;
            }
        } else {
            executions.push(*timing);
        }
        source.seed_tracker_clocks(&self.session.tracker);
        source.select();
    }

    pub(crate) fn apply_pending_current_tools(&mut self) {
        if let CurrentActionsProjection::Bound(source) = &self.current_actions {
            for tool in &source.facts.tools {
                self.session
                    .tracker
                    .observe_current_tool(tool, &mut self.scrollback);
            }
        }
    }

    pub(crate) fn observe_current_sampling(
        &mut self,
        lifecycle: &xai_grok_shell::extensions::sampling_lifecycle::SamplingLifecycle,
        meta: &NotificationMeta,
    ) {
        if !self.accepts_publication_role(&meta.delivery_role) {
            return;
        }
        use xai_grok_shell::extensions::sampling_lifecycle::SamplingPhase;
        let CurrentActionsProjection::Bound(source) = &mut self.current_actions else {
            return;
        };
        if let Some(snapshot) = source.snapshot_version
            && !matches!(meta.current_actions_version, crate::acp::meta::CurrentSourceMetadata::Version(version) if version.sequence > snapshot.sequence)
        {
            return;
        }
        if matches!(source.coverage, Coverage::Unavailable) {
            source.coverage = Coverage::Transitions;
        }
        source
            .facts
            .sampling
            .retain(|request| request.owner.request_id != lifecycle.owner.request_id);
        let phase = match &lifecycle.phase {
            SamplingPhase::RetryScheduled { deadline } => {
                Some(CurrentSamplingPhase::RetryScheduled {
                    deadline: deadline.clone(),
                })
            }
            SamplingPhase::DispatchStarted { started_at } => {
                Some(CurrentSamplingPhase::Submitted {
                    started_at: *started_at,
                })
            }
            SamplingPhase::Finished { .. } => None,
        };
        if let Some(phase) = phase {
            let turn = source
                .facts
                .turn
                .as_ref()
                .filter(|turn| turn.owner.prompt_id() == Some(lifecycle.owner.prompt_id.as_str()))
                .map_or(FrozenOwner::Unavailable, |turn| turn.owner.clone());
            source.facts.sampling.push(CurrentSampling {
                owner: lifecycle.owner.clone(),
                turn,
                phase,
            });
        }
        source.seed_tracker_clocks(&self.session.tracker);
        source.select();
    }

    pub(crate) fn settle_current_actions_compact(
        &mut self,
        invocation: xai_grok_shell::session::acp_types::CompactInvocationId,
    ) {
        if let CurrentActionsProjection::Bound(source) = &mut self.current_actions {
            source.facts.compactions.retain(|compact| !matches!(compact.owner, CompactOwner::Manual { invocation_id: Some(id), .. } if id == invocation));
            source.seed_tracker_clocks(&self.session.tracker);
            source.select();
        }
    }

    pub(crate) fn settle_current_actions_prompt_rpc(&mut self, prompt: &str) {
        if self.session.current_prompt_id.as_deref() != Some(prompt) {
            return;
        }
        let Some(binding) = self.current_actions_binding() else {
            return;
        };
        if let CurrentActionsProjection::Unavailable { settlements, .. } = &mut self.current_actions
        {
            match settlements {
                PendingPromptSettlements::OriginalRpc {
                    binding: prior,
                    prompts,
                } if prior.session_id == binding.session_id
                    && prior.binding_epoch == binding.binding_epoch =>
                {
                    prompts.insert(prompt.to_owned());
                }
                PendingPromptSettlements::None | PendingPromptSettlements::OriginalRpc { .. } => {
                    *settlements = PendingPromptSettlements::OriginalRpc {
                        binding,
                        prompts: HashSet::from([prompt.to_owned()]),
                    };
                }
            }
        }
        self.settle_current_actions_prompt(prompt);
    }

    pub(crate) fn settle_current_actions_prompt(&mut self, prompt: &str) {
        let CurrentActionsProjection::Bound(source) = &mut self.current_actions else {
            return;
        };
        source.settled_prompts.insert(prompt.to_owned());
        if source
            .facts
            .turn
            .as_ref()
            .is_some_and(|turn| turn.owner.prompt_id() == Some(prompt))
        {
            source.facts.turn = None;
        }
        source
            .facts
            .sampling
            .retain(|request| request.owner.prompt_id != prompt);
        source.facts.tools.retain(|tool| tool.turn.prompt_id() != Some(prompt)
            && !matches!(&tool.ownership, xai_grok_shell::extensions::tool_timing::ToolCallOwnership::Registered {
                source: xai_grok_shell::extensions::tool_timing::ToolCallSource::Sampling { prompt_id, .. }, ..
            } if prompt_id == prompt));
        source
            .facts
            .hooks
            .retain(|hook| hook.current.turn.prompt_id() != Some(prompt));
        source.seed_tracker_clocks(&self.session.tracker);
        source.select();
    }

    pub(crate) fn current_actions_binding(&self) -> Option<SnapshotBinding> {
        Some(SnapshotBinding {
            session_id: self.session.session_id.as_ref()?.0.to_string(),
            binding_epoch: u64::from(self.session_binding_epoch),
            load_generation: self.current_actions_generation,
        })
    }

    pub(crate) fn bind_current_actions(&mut self, context: CurrentActionsContext) -> bool {
        if context.incarnation.is_nil()
            || self.current_actions_binding().as_ref() != Some(&context.binding)
        {
            return false;
        }
        if matches!(&self.current_actions, CurrentActionsProjection::Bound(source) if source.context.incarnation != context.incarnation)
        {
            self.last_applied_event_seq = None;
            self.last_applied_xai_event_seq = None;
            self.last_seen_event_seq = None;
        }
        let prior = match &self.current_actions {
            CurrentActionsProjection::Unavailable {
                latest_live,
                native_frontier,
                ..
            } if latest_live.is_none_or(|version| version.incarnation == context.incarnation) => {
                (*latest_live, *native_frontier)
            }
            CurrentActionsProjection::Unavailable { .. } | CurrentActionsProjection::Bound(_) => {
                (None, None)
            }
        };
        match &mut self.current_actions {
            CurrentActionsProjection::Bound(source)
                if source.context.incarnation == context.incarnation =>
            {
                source.context = context;
            }
            CurrentActionsProjection::Unavailable { .. } | CurrentActionsProjection::Bound(_) => {
                let settled_prompts = match &mut self.current_actions {
                    // Reload generations change read authority, not original RPC ownership.
                    CurrentActionsProjection::Unavailable { settlements, .. } => {
                        match std::mem::take(settlements) {
                            PendingPromptSettlements::OriginalRpc { binding, prompts }
                                if binding.session_id == context.binding.session_id
                                    && binding.binding_epoch == context.binding.binding_epoch =>
                            {
                                prompts
                            }
                            PendingPromptSettlements::None
                            | PendingPromptSettlements::OriginalRpc { .. } => HashSet::new(),
                        }
                    }
                    CurrentActionsProjection::Bound(_) => HashSet::new(),
                };
                self.current_actions = CurrentActionsProjection::Bound(Box::new(BoundSource {
                    context,
                    latest_live: prior.0,
                    native_frontier: prior.1,
                    facts: Facts::default(),
                    selection: Selection::Unavailable,
                    task_clock: PhaseClock {
                        mapping: LocalClockMapping::Uncorrelated,
                    },
                    coverage: Coverage::Unavailable,
                    snapshot_version: None,
                    snapshot_frontier: None,
                    turn_sequence: None,
                    transitions: HashMap::new(),
                    settled_prompts,
                    clocks: Vec::new(),
                }));
            }
        }
        true
    }

    pub(crate) fn accepts_current_source(&self, meta: &NotificationMeta) -> bool {
        use xai_grok_shell::extensions::current_actions::DeliveryRole;
        match &meta.delivery_role {
            DeliveryRole::Unavailable => {}
            DeliveryRole::Historical { .. }
            | DeliveryRole::Live { .. }
            | DeliveryRole::LiveUnbound { .. } => {
                if !self.accepts_publication_role(&meta.delivery_role) {
                    // Native admission retains its own custody before the first trusted read.
                    let same_binding = match &meta.delivery_role {
                        DeliveryRole::Live { context } => {
                            self.current_actions_binding().as_ref() == Some(&context.binding)
                        }
                        DeliveryRole::LiveUnbound { session_id, .. } => self
                            .current_actions_binding()
                            .is_some_and(|binding| binding.session_id == *session_id),
                        DeliveryRole::Unavailable | DeliveryRole::Historical { .. } => false,
                    };
                    return self.current_actions_context().is_none()
                        && !meta.is_replay
                        && same_binding;
                }
            }
        }
        if meta.is_replay {
            return true;
        }
        match (&self.current_actions, &meta.current_actions_version) {
            (_, crate::acp::meta::CurrentSourceMetadata::Malformed) => false,
            (
                CurrentActionsProjection::Bound(source),
                crate::acp::meta::CurrentSourceMetadata::Version(version),
            ) => {
                self.current_actions_binding().as_ref() == Some(&source.context.binding)
                    && version.incarnation == source.context.incarnation
            }
            (
                CurrentActionsProjection::Unavailable { .. },
                crate::acp::meta::CurrentSourceMetadata::Version(_),
            )
            | (_, crate::acp::meta::CurrentSourceMetadata::Legacy) => true,
        }
    }

    pub(crate) fn observe_current_frontier(&mut self, meta: &NotificationMeta) {
        if !self.accepts_publication_role(&meta.delivery_role) {
            return;
        }
        if meta.is_replay {
            self.session.tracker.note_replayed_source_frontier(meta);
            return;
        }
        let native_frontier = match &mut self.current_actions {
            CurrentActionsProjection::Bound(source) => &mut source.native_frontier,
            CurrentActionsProjection::Unavailable {
                native_frontier, ..
            } => native_frontier,
        };
        if let Some(seq) = meta.event_seq {
            *native_frontier = Some(native_frontier.map_or(seq, |old| old.max(seq)));
        }
    }

    pub(crate) fn observe_current_source(&mut self, meta: &NotificationMeta) {
        self.observe_current_frontier(meta);
        if meta.is_replay || !self.accepts_publication_role(&meta.delivery_role) {
            return;
        }
        let latest_live = match &mut self.current_actions {
            CurrentActionsProjection::Bound(source) => &mut source.latest_live,
            CurrentActionsProjection::Unavailable { latest_live, .. } => latest_live,
        };
        if let crate::acp::meta::CurrentSourceMetadata::Version(version) =
            meta.current_actions_version
        {
            *latest_live = Some(
                latest_live
                    .filter(|old| old.incarnation == version.incarnation)
                    .map_or(version, |old| SourceVersion {
                        sequence: old.sequence.max(version.sequence),
                        ..version
                    }),
            );
        }
    }

    pub(super) fn adopt_reconstructed_current_tools(
        &mut self,
        historical: &crate::acp::tracker::AcpUpdateTracker,
        live: &mut super::load_continuation::ReplayTranscript,
    ) {
        if let CurrentActionsProjection::Bound(source) = &mut self.current_actions {
            source
                .facts
                .tools
                .retain(|tool| live.tracker.current_tool_is_admissible(tool));
            for tool in &source.facts.tools {
                live.tracker.adopt_replayed_current_tool_basis(
                    historical,
                    tool,
                    &mut live.scrollback,
                );
            }
            source.seed_tracker_clocks(&live.tracker);
            source.select();
        }
    }

    pub(crate) fn has_current_actions_capture(&self, current: &CurrentActions) -> bool {
        matches!(&self.current_actions, CurrentActionsProjection::Bound(source)
            if source.context.binding == current.binding && source.context.incarnation == current.version.incarnation
                && source.snapshot_version == Some(current.version) && source.snapshot_frontier == Some(current.event_frontier))
    }

    pub(crate) fn finish_current_actions_replay(
        &mut self,
        context: &CurrentActionsContext,
        frontier: u64,
    ) -> bool {
        if !self.accepts_current_actions_context(context)
            || self.session.loading_replay
            || self.session_reload.is_some()
            || self
                .reconstructed_source_frontier(context.incarnation)
                .is_some_and(|sequence| sequence >= frontier)
        {
            return false;
        }
        if !matches!(&self.current_actions, CurrentActionsProjection::Bound(source) if source.snapshot_frontier == Some(frontier))
        {
            return false;
        }
        self.late_replay_until = None;
        if matches!(
            self.load_reconstruction,
            super::load_continuation::LoadReconstruction::Continued { .. }
        ) {
            self.load_reconstruction = super::load_continuation::LoadReconstruction::Closed;
        }
        true
    }

    pub(crate) fn current_actions_unavailable_for_binding(
        &mut self,
        binding: &SnapshotBinding,
    ) -> bool {
        if self.current_actions_binding().as_ref() != Some(binding) {
            return false;
        }
        if let CurrentActionsProjection::Bound(source) = &mut self.current_actions {
            source.facts = Facts::default();
            source.selection = Selection::Unavailable;
            source.task_clock = PhaseClock {
                mapping: LocalClockMapping::Uncorrelated,
            };
            source.coverage = Coverage::Unavailable;
            source.snapshot_frontier = None;
            source.clocks.clear();
        }
        true
    }

    pub(crate) fn current_actions_unavailable(&mut self, context: &CurrentActionsContext) -> bool {
        if !self.accepts_current_actions_context(context) {
            return false;
        }
        self.current_actions_unavailable_for_binding(&context.binding)
    }

    pub(crate) fn current_actions_context(&self) -> Option<&CurrentActionsContext> {
        match &self.current_actions {
            CurrentActionsProjection::Bound(source)
                if self.current_actions_binding().as_ref() == Some(&source.context.binding) =>
            {
                Some(&source.context)
            }
            CurrentActionsProjection::Bound(_) | CurrentActionsProjection::Unavailable { .. } => {
                None
            }
        }
    }

    pub(crate) fn accepts_current_actions_context(&self, context: &CurrentActionsContext) -> bool {
        self.current_actions_binding().as_ref() == Some(&context.binding)
            && matches!(&self.current_actions, CurrentActionsProjection::Bound(source) if source.context == *context)
    }

    pub(crate) fn accepts_current_actions_snapshot(
        &self,
        context: &CurrentActionsContext,
        snapshot: &CurrentActionsSnapshot,
    ) -> bool {
        let CurrentActionsProjection::Bound(source) = &self.current_actions else {
            return false;
        };
        context == &source.context
            && self.current_actions_binding().as_ref() == Some(&context.binding)
            && match snapshot {
                CurrentActionsSnapshot::Unavailable => false,
                CurrentActionsSnapshot::Current(current) => {
                    current.version.incarnation == context.incarnation
                }
            }
            && snapshot.is_current_for(
                &context.binding,
                source.latest_live,
                source
                    .native_frontier
                    .max(self.reconstructed_source_frontier(context.incarnation)),
            )
    }

    pub(crate) fn apply_current_actions_snapshot(
        &mut self,
        context: &CurrentActionsContext,
        snapshot: CurrentActionsSnapshot,
    ) -> bool {
        if !self.accepts_current_actions_snapshot(context, &snapshot) {
            return false;
        }
        let CurrentActionsSnapshot::Current(current) = snapshot else {
            return false;
        };
        let mut current = current;
        if let CurrentActionsProjection::Bound(source) = &self.current_actions {
            current
                .sampling
                .retain(|request| !source.settled_prompts.contains(&request.owner.prompt_id));
            if current.turn.as_ref().is_some_and(|turn| {
                turn.owner
                    .prompt_id()
                    .is_some_and(|prompt| source.settled_prompts.contains(prompt))
            }) {
                current.turn = None;
            }
            current.tools.retain(|tool| !tool.turn.prompt_id().is_some_and(|prompt| source.settled_prompts.contains(prompt))
                && !matches!(&tool.ownership, xai_grok_shell::extensions::tool_timing::ToolCallOwnership::Registered {
                    source: xai_grok_shell::extensions::tool_timing::ToolCallSource::Sampling { prompt_id, .. }, ..
                } if source.settled_prompts.contains(prompt_id)));
            current.hooks.retain(|hook| {
                !hook
                    .turn
                    .prompt_id()
                    .is_some_and(|prompt| source.settled_prompts.contains(prompt))
            });
        }
        current
            .compactions
            .retain(|compact| !self.current_compact_rpc_settled(&compact.owner));
        current
            .tools
            .retain(|tool| self.session.tracker.current_tool_is_admissible(tool));
        for request in &current.sampling {
            self.session
                .tracker
                .adopt_current_sampling(request, &mut self.scrollback);
        }
        for tool in &current.tools {
            self.session
                .tracker
                .observe_current_tool(tool, &mut self.scrollback);
        }
        let CurrentActions {
            turn,
            sampling,
            tools,
            compactions,
            hooks,
            interactions,
            version,
            event_frontier,
            ..
        } = current;
        if let CurrentActionsProjection::Bound(source) = &mut self.current_actions {
            let previous_hooks = std::mem::take(&mut source.facts.hooks);
            let hooks = hooks
                .into_iter()
                .map(|current| {
                    let reveal_since = previous_hooks
                        .iter()
                        .find(|previous| previous.current.batch_id == current.batch_id)
                        .map(|previous| previous.reveal_since)
                        .unwrap_or_else(|| {
                            source
                                .clock(current.started_at)
                                .started_at()
                                .unwrap_or_else(Instant::now)
                        });
                    ObservedHook {
                        current,
                        reveal_since,
                    }
                })
                .collect();
            source.facts = Facts {
                turn,
                sampling,
                tools,
                compactions,
                hooks,
                interactions,
            };
            source.latest_live = Some(version);
            source.snapshot_version = Some(version);
            source.snapshot_frontier = Some(event_frontier);
            source.turn_sequence = Some(version.sequence);
            source.coverage = Coverage::Complete;
            source.seed_tracker_clocks(&self.session.tracker);
            source.select();
        }
        true
    }

    fn current_compact_rpc_settled(&self, owner: &CompactOwner) -> bool {
        match owner {
            CompactOwner::Manual {
                invocation_id: Some(id),
                ..
            } => self.compact_invocations.get(id).is_some_and(|invocation| {
                invocation.binding_epoch == self.session_binding_epoch
                    && invocation.settlement
                        == crate::app::agent::CompactInvocationSettlement::RpcSettled
            }),
            CompactOwner::Manual {
                invocation_id: None,
                ..
            }
            | CompactOwner::Inline
            | CompactOwner::Automatic { .. } => false,
        }
    }

    pub(crate) fn accepts_current_actions_event(&self, event: &CurrentActionsEvent) -> bool {
        if let CurrentActionTransition::CompactChanged { compact } = &event.transition
            && self.current_compact_rpc_settled(&compact.owner)
        {
            return false;
        }
        let CurrentActionsProjection::Bound(source) = &self.current_actions else {
            return false;
        };
        event.validate().is_ok()
            && event.session_id == source.context.binding.session_id
            && self.current_actions_binding().as_ref() == Some(&source.context.binding)
            && event.version.incarnation == source.context.incarnation
            && source
                .snapshot_version
                .is_none_or(|snapshot| event.version.sequence > snapshot.sequence)
            && source
                .transitions
                .get(&TransitionOwner::event(event))
                .is_none_or(|last| event.version.sequence > *last)
            && (!matches!(TransitionOwner::event(event), TransitionOwner::Task(_))
                || source
                    .turn_sequence
                    .is_none_or(|last| event.version.sequence > last))
    }

    pub(crate) fn apply_current_actions_event(&mut self, event: CurrentActionsEvent) -> bool {
        if !self.accepts_current_actions_event(&event) {
            return false;
        }
        let CurrentActionsProjection::Bound(source) = &mut self.current_actions else {
            return false;
        };
        source.latest_live = Some(
            source
                .latest_live
                .map_or(event.version, |old| SourceVersion {
                    sequence: old.sequence.max(event.version.sequence),
                    ..event.version
                }),
        );
        if matches!(TransitionOwner::event(&event), TransitionOwner::Task(_)) {
            source.turn_sequence = Some(event.version.sequence);
        }
        source
            .transitions
            .insert(TransitionOwner::event(&event), event.version.sequence);
        if matches!(source.coverage, Coverage::Unavailable) {
            source.coverage = Coverage::Transitions;
        }
        match event.transition {
            CurrentActionTransition::TurnRegistered { turn }
            | CurrentActionTransition::TurnAnchored { turn }
            | CurrentActionTransition::TurnStarted { turn }
            | CurrentActionTransition::TurnBackendFinished { turn } => {
                if !turn
                    .owner
                    .prompt_id()
                    .is_some_and(|prompt| source.settled_prompts.contains(prompt))
                {
                    source.facts.turn = Some(turn);
                }
            }
            CurrentActionTransition::TurnFinished { owner } => {
                if source
                    .facts
                    .turn
                    .as_ref()
                    .is_some_and(|turn| turn.owner == owner)
                {
                    source.facts.turn = None;
                }
            }
            CurrentActionTransition::HookStarted { hook } => {
                if let Some(previous) = source
                    .facts
                    .hooks
                    .iter_mut()
                    .find(|previous| previous.current.batch_id == hook.batch_id)
                {
                    previous.current = hook;
                } else {
                    let reveal_since = source
                        .clock(hook.started_at)
                        .started_at()
                        .unwrap_or_else(Instant::now);
                    source.facts.hooks.push(ObservedHook {
                        current: hook,
                        reveal_since,
                    });
                }
            }
            CurrentActionTransition::HookFinished { batch_id } => source
                .facts
                .hooks
                .retain(|hook| hook.current.batch_id != batch_id),
            CurrentActionTransition::CompactChanged { compact } => {
                source
                    .facts
                    .compactions
                    .retain(|old| old.owner != compact.owner);
                source.facts.compactions.push(compact);
            }
            CurrentActionTransition::CompactFinished { owner } => {
                source
                    .facts
                    .compactions
                    .retain(|compact| compact.owner != owner);
                if let CompactOwner::Manual {
                    invocation_id: Some(id),
                    ..
                } = owner
                    && let Some(invocation) = self.compact_invocations.get_mut(&id)
                    && invocation.binding_epoch == self.session_binding_epoch
                    && invocation.settlement
                        != crate::app::agent::CompactInvocationSettlement::RpcSettled
                {
                    invocation.settlement =
                        crate::app::agent::CompactInvocationSettlement::LifecycleObserved;
                }
            }
            CurrentActionTransition::InteractionStarted { interaction } => {
                source
                    .facts
                    .interactions
                    .retain(|old| old.tool_call_id != interaction.tool_call_id);
                source.facts.interactions.push(interaction);
            }
            CurrentActionTransition::InteractionFinished {
                tool_call_id,
                registration_id,
            } => {
                source.facts.interactions.retain(|interaction| interaction.tool_call_id != tool_call_id
                    || !matches!(interaction.source, InteractionSource::Registered { registration_id: current, .. } if current == registration_id));
            }
        }
        source.seed_tracker_clocks(&self.session.tracker);
        source.select();
        true
    }

    pub(crate) fn current_action_activity(&self) -> Option<Option<TurnActivity>> {
        let CurrentActionsProjection::Bound(source) = &self.current_actions else {
            return None;
        };
        let waiting = |reason| Some(Some(TurnActivity::Waiting(reason)));
        match &source.selection {
            Selection::Unavailable => match source.coverage {
                Coverage::Unavailable => waiting(WaitingReason::ActionSourceUnavailable),
                Coverage::Transitions | Coverage::Complete => None,
            },
            Selection::Idle => Some(None),
            Selection::Model(request, _) => match request.phase {
                CurrentSamplingPhase::Reserved => waiting(WaitingReason::ModelAdmission),
                CurrentSamplingPhase::RetryScheduled { .. } => Some(Some(
                    self.session
                        .tracker
                        .current_retry_activity(&request.owner)
                        .unwrap_or(TurnActivity::Waiting(WaitingReason::Model)),
                )),
                CurrentSamplingPhase::Submitted { .. } => Some(Some(
                    self.session
                        .tracker
                        .current_model_activity(&request.owner)
                        .unwrap_or(TurnActivity::Waiting(WaitingReason::Model)),
                )),
            },
            Selection::Retry(request, mapping) => {
                use xai_grok_shell::extensions::sampling_lifecycle::RetryClockMapping;
                let due_at = match mapping {
                    RetryClockMapping::Correlated { latest_due_at, .. } => {
                        Some(latest_due_at.into_std())
                    }
                    RetryClockMapping::Uncorrelated => None,
                };
                Some(Some(
                    self.session
                        .tracker
                        .current_retry_activity(&request.owner)
                        .unwrap_or(TurnActivity::Waiting(WaitingReason::RetryBackoff {
                            due_at,
                        })),
                ))
            }
            Selection::Turn(turn, _) => waiting(match turn.phase {
                CurrentTurnPhase::Admitted => WaitingReason::TaskStart,
                CurrentTurnPhase::Running { .. } => WaitingReason::BackendResponse,
                CurrentTurnPhase::AwaitingResponse { .. } => WaitingReason::ResponseDelivery,
            }),
            Selection::Tool(tool, _) => {
                Some(Some(self.session.tracker.current_tool_activity(tool)))
            }
            Selection::Compact(compact, _) => Some(Some(match compact.stage {
                CompactStage::Queued => TurnActivity::Waiting(WaitingReason::CompactionQueued),
                CompactStage::Executing => TurnActivity::AutoCompacting,
                CompactStage::AwaitingPersistence | CompactStage::AwaitingResponse => {
                    TurnActivity::Waiting(WaitingReason::CompactionSettlement)
                }
            })),
            Selection::Hook { reveal_since, .. } => Some(
                (reveal_since.elapsed() >= crate::acp::tracker::HOOK_REVEAL_DELAY)
                    .then_some(TurnActivity::Waiting(WaitingReason::HookBatch)),
            ),
            Selection::Interaction(_) => waiting(WaitingReason::UserInput),
        }
    }

    pub(crate) fn current_actions_display_state(
        &self,
    ) -> Option<&'static crate::app::agent::AgentState> {
        if !self.session.state.is_idle() {
            return None;
        }
        match &self.current_actions {
            CurrentActionsProjection::Bound(source) => match &source.selection {
                Selection::Unavailable | Selection::Idle => None,
                Selection::Model(_, _)
                | Selection::Retry(_, _)
                | Selection::Turn(_, _)
                | Selection::Tool(_, _)
                | Selection::Compact(_, _)
                | Selection::Interaction(_) => Some(&crate::app::agent::AgentState::TurnRunning),
                Selection::Hook { reveal_since, .. } => (reveal_since.elapsed()
                    >= crate::acp::tracker::HOOK_REVEAL_DELAY)
                    .then_some(&crate::app::agent::AgentState::TurnRunning),
            },
            CurrentActionsProjection::Unavailable { .. } => None,
        }
    }

    pub(crate) fn current_source_turn_elapsed(&self) -> Option<std::time::Duration> {
        match &self.current_actions {
            CurrentActionsProjection::Bound(source) => {
                source.task_clock.started_at().map(|start| start.elapsed())
            }
            CurrentActionsProjection::Unavailable { .. } => None,
        }
    }

    pub(crate) fn current_action_started_at(&self) -> Option<Instant> {
        match &self.current_actions {
            CurrentActionsProjection::Unavailable { .. } => match self.session.tracker.activity() {
                None
                | Some(
                    TurnActivity::Thinking
                    | TurnActivity::Responding
                    | TurnActivity::WritingToolCall(_)
                    | TurnActivity::Waiting(WaitingReason::Model),
                ) => self.session.tracker.current_sampling_started_at(),
                Some(
                    TurnActivity::ToolRunning { .. }
                    | TurnActivity::AutoCompacting
                    | TurnActivity::Retrying { .. }
                    | TurnActivity::Waiting(_),
                ) => None,
            },
            CurrentActionsProjection::Bound(source) => source.selection.clock(),
        }
    }
}
