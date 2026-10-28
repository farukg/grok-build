//! In-memory continuation facts. No transcript, request payloads, or replay adoption.
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};
use serde::{Deserialize, Serialize};
use xai_monotonic_clock::MonotonicPoint;

use crate::extensions::sampling_lifecycle::{RequestAttemptId, RequestId, RetryDeadlineProjection, SamplingOwner};
use crate::extensions::tool_timing::{ToolCallOwnership, ToolCallSource, ToolExecutionTiming};
use super::acp_types::{AutomaticCompactInvocation, CompactInvocationId};
use super::pending_interaction::{PendingInteractions, PendingKind};

pub const CAPABILITY: &str = "x.ai/current_actions";
pub const VERSION_META_KEY: &str = "currentActionsVersion";
pub const TRANSITION_METHOD: &str = "x.ai/current_actions/transition";

#[path = "current_actions_publication.rs"]
mod publication;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceVersion {
    #[serde(deserialize_with = "deserialize_incarnation")]
    pub incarnation: uuid::Uuid,
    pub sequence: u64,
}

fn deserialize_incarnation<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<uuid::Uuid, D::Error> {
    let incarnation = uuid::Uuid::deserialize(deserializer)?;
    if incarnation.is_nil() { return Err(serde::de::Error::custom("nil current-actions incarnation")); }
    Ok(incarnation)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "owner", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FrozenOwner {
    Unavailable,
    Root { prompt_id: String, task_id: String },
    Child { prompt_id: String, task_id: String },
    Wake { prompt_id: String, task_id: String },
}

impl FrozenOwner {
    pub fn prompt_id(&self) -> Option<&str> {
        match self {
            Self::Unavailable => None,
            Self::Root { prompt_id, .. } | Self::Child { prompt_id, .. }
            | Self::Wake { prompt_id, .. } => Some(prompt_id),
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Unavailable => true,
            Self::Root { prompt_id, task_id } | Self::Child { prompt_id, task_id }
            | Self::Wake { prompt_id, task_id } => !prompt_id.is_empty() && !task_id.is_empty(),
        }
    }

    fn task_id(&self) -> Option<&str> {
        match self {
            Self::Unavailable => None,
            Self::Root { task_id, .. } | Self::Child { task_id, .. }
            | Self::Wake { task_id, .. } => Some(task_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentTurn {
    pub owner: FrozenOwner,
    /// Projected on first poll from the original task anchor, outside the admission lock.
    pub started_at: MonotonicPoint,
    pub phase: CurrentTurnPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CurrentTurnPhase {
    Admitted,
    Running { started_at: MonotonicPoint },
    AwaitingResponse { started_at: MonotonicPoint },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CurrentSamplingPhase {
    Reserved,
    RetryScheduled { deadline: RetryDeadlineProjection },
    Submitted { #[serde(default)] started_at: MonotonicPoint },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSampling {
    pub owner: SamplingOwner,
    pub turn: FrozenOwner,
    #[serde(flatten)]
    pub phase: CurrentSamplingPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentTool {
    pub ownership: ToolCallOwnership,
    pub turn: FrozenOwner,
    pub registered_at: MonotonicPoint,
    pub executions: Vec<ToolExecutionTiming>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CompactOwner {
    Inline,
    Manual { request_id: uuid::Uuid, invocation_id: Option<CompactInvocationId> },
    Automatic { invocation: AutomaticCompactInvocation },
}

impl CompactOwner {
    fn valid(&self) -> bool {
        match self {
            Self::Inline => true,
            Self::Manual { request_id, invocation_id } => !request_id.is_nil() && invocation_id.is_none_or(|id| !id.0.is_nil()),
            Self::Automatic { invocation } => !invocation.id.0.is_nil() && invocation.prompt_id.as_ref().is_none_or(|id| !id.is_empty()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactStage {
    Queued,
    Executing,
    AwaitingPersistence,
    AwaitingResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCompact {
    pub owner: CompactOwner,
    pub turn: FrozenOwner,
    pub stage: CompactStage,
    pub admitted_at: MonotonicPoint,
    pub stage_started_at: MonotonicPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentHook {
    pub batch_id: uuid::Uuid,
    pub turn: FrozenOwner,
    pub started_at: MonotonicPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum InteractionSource {
    Unavailable,
    Registered { registration_id: uuid::Uuid, turn: FrozenOwner, started_at: MonotonicPoint },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentInteraction {
    pub tool_call_id: String,
    pub kind: PendingKind,
    pub source: InteractionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotBinding {
    pub session_id: String,
    pub binding_epoch: u64,
    pub load_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentActions {
    pub binding: SnapshotBinding,
    pub version: SourceVersion,
    /// Exclusive process-wide native event cursor at capture.
    pub event_frontier: u64,
    pub turn: Option<CurrentTurn>,
    pub sampling: Vec<CurrentSampling>,
    pub tools: Vec<CurrentTool>,
    pub backend_tools: BackendToolsObservation,
    pub compactions: Vec<CurrentCompact>,
    pub hooks: Vec<CurrentHook>,
    pub interactions: Vec<CurrentInteraction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum BackendToolsObservation { Unavailable }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum CurrentActionsSnapshot {
    Unavailable,
    Current(CurrentActions),
}

#[derive(Deserialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
enum SnapshotDocument { Unavailable, Current(CurrentActions) }

impl<'de> Deserialize<'de> for CurrentActionsSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let snapshot = match SnapshotDocument::deserialize(deserializer)? {
            SnapshotDocument::Unavailable => Self::Unavailable,
            SnapshotDocument::Current(current) => Self::Current(current),
        };
        snapshot.validate().map_err(serde::de::Error::custom)?;
        Ok(snapshot)
    }
}

impl Default for CurrentActionsSnapshot {
    fn default() -> Self { Self::Unavailable }
}

impl CurrentActionsSnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        let Self::Current(current) = self else { return Ok(()); };
        if current.binding.session_id.is_empty() || current.version.incarnation.is_nil() {
            return Err("invalid current-actions binding or source incarnation");
        }
        if let Some(turn) = &current.turn
            && (turn.owner == FrozenOwner::Unavailable || !turn.owner.valid())
        { return Err("invalid current turn owner"); }
        let mut requests = std::collections::HashSet::new();
        for request in &current.sampling {
            if request.owner.prompt_id.is_empty() || request.owner.request_id.as_str().is_empty()
                || request.owner.attempt.0 == 0 || !request.turn.valid()
                || request.turn.prompt_id().is_some_and(|id| id != request.owner.prompt_id)
                || !requests.insert(&request.owner.request_id)
            { return Err("invalid or duplicate current sampling owner"); }
        }
        let mut invocations = std::collections::HashSet::new();
        for tool in &current.tools {
            let ToolCallOwnership::Registered { invocation_id, source, .. } = &tool.ownership
                else { return Err("unregistered current tool"); };
            if invocation_id.is_empty() || !tool.turn.valid()
                || !invocations.insert(invocation_id)
            { return Err("invalid or duplicate current tool"); }
            match source {
                ToolCallSource::Unknown => {}
                ToolCallSource::Sampling { prompt_id, request_id, attempt } => {
                    if prompt_id.is_empty() || request_id.is_empty() || *attempt == 0
                        || tool.turn.prompt_id().is_some_and(|id| id != prompt_id)
                    { return Err("invalid tool sampling source"); }
                }
            }
            let mut executions = std::collections::HashSet::new();
            for span in &tool.executions {
                let id = match span {
                    ToolExecutionTiming::Unknown => return Err("unknown execution in current span set"),
                    ToolExecutionTiming::Started { execution_id, .. }
                    | ToolExecutionTiming::Finished { execution_id, .. } => execution_id,
                };
                if !executions.insert(id) { return Err("duplicate current execution"); }
            }
        }
        let mut compactions = std::collections::HashSet::new();
        for compact in &current.compactions {
            if !compact.turn.valid() || !compactions.insert(CompactKey::of(&compact.owner)) {
                return Err("invalid or duplicate current compaction");
            }
            match &compact.owner {
                CompactOwner::Inline => {}
                CompactOwner::Manual { request_id, invocation_id } => {
                    if request_id.is_nil() || invocation_id.is_some_and(|id| id.0.is_nil()) { return Err("invalid manual compaction identity"); }
                }
                CompactOwner::Automatic { invocation } => {
                    if invocation.id.0.is_nil() || invocation.prompt_id.as_ref().is_some_and(String::is_empty)
                        || compact.turn.prompt_id().is_some_and(|id| Some(id) != invocation.prompt_id.as_deref())
                    { return Err("invalid automatic compaction identity"); }
                }
            }
        }
        let mut hooks = std::collections::HashSet::new();
        for hook in &current.hooks {
            if hook.batch_id.is_nil() || !hook.turn.valid() || !hooks.insert(hook.batch_id) {
                return Err("invalid or duplicate current hook");
            }
        }
        let mut interactions = std::collections::HashSet::new();
        for interaction in &current.interactions {
            if interaction.tool_call_id.is_empty() || !interactions.insert(&interaction.tool_call_id) {
                return Err("invalid or duplicate current interaction");
            }
            match &interaction.source {
                InteractionSource::Unavailable => {}
                InteractionSource::Registered { registration_id, turn, .. } => {
                    if registration_id.is_nil() || !turn.valid() { return Err("invalid interaction registration"); }
                }
            }
        }
        Ok(())
    }

    pub fn is_current_for(&self, binding: &SnapshotBinding, latest_live: Option<SourceVersion>, latest_event_sequence: Option<u64>) -> bool {
        if self.validate().is_err() { return false; }
        match self {
            Self::Unavailable => false,
            Self::Current(current) => current.binding == *binding
                && latest_event_sequence.is_none_or(|sequence| sequence < current.event_frontier)
                && match latest_live {
                    None => true,
                    Some(live) => current.version.incarnation == live.incarnation && current.version.sequence >= live.sequence,
                },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transition", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum CurrentActionTransition {
    TurnRegistered { turn: CurrentTurn },
    TurnAnchored { turn: CurrentTurn },
    TurnStarted { turn: CurrentTurn },
    TurnBackendFinished { turn: CurrentTurn },
    TurnFinished { owner: FrozenOwner },
    HookStarted { hook: CurrentHook },
    HookFinished { batch_id: uuid::Uuid },
    CompactChanged { compact: CurrentCompact },
    CompactFinished { owner: CompactOwner },
    InteractionStarted { interaction: CurrentInteraction },
    InteractionFinished { tool_call_id: String, registration_id: uuid::Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentActionsEvent {
    pub session_id: String,
    pub version: SourceVersion,
    #[serde(flatten)]
    pub transition: CurrentActionTransition,
}

impl CurrentActionsEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.is_empty() || self.version.incarnation.is_nil() { return Err("invalid source event binding"); }
        let phase_matches = match &self.transition {
            CurrentActionTransition::TurnRegistered { turn } | CurrentActionTransition::TurnAnchored { turn } => matches!(turn.phase, CurrentTurnPhase::Admitted),
            CurrentActionTransition::TurnStarted { turn } => matches!(turn.phase, CurrentTurnPhase::Running { .. }),
            CurrentActionTransition::TurnBackendFinished { turn } => matches!(turn.phase, CurrentTurnPhase::AwaitingResponse { .. }),
            CurrentActionTransition::TurnFinished { .. } | CurrentActionTransition::HookStarted { .. }
            | CurrentActionTransition::HookFinished { .. } | CurrentActionTransition::CompactChanged { .. }
            | CurrentActionTransition::CompactFinished { .. } | CurrentActionTransition::InteractionStarted { .. }
            | CurrentActionTransition::InteractionFinished { .. } => true,
        };
        if !phase_matches { return Err("source transition does not match its phase"); }
        match &self.transition {
            CurrentActionTransition::TurnRegistered { turn } | CurrentActionTransition::TurnAnchored { turn }
            | CurrentActionTransition::TurnStarted { turn } | CurrentActionTransition::TurnBackendFinished { turn } => {
                if turn.owner.prompt_id().is_none_or(str::is_empty) || turn.owner.task_id().is_none_or(str::is_empty) {
                    return Err("invalid source turn");
                }
            }
            CurrentActionTransition::TurnFinished { owner } => {
                if owner.prompt_id().is_none_or(str::is_empty) || owner.task_id().is_none_or(str::is_empty) { return Err("invalid source terminal"); }
            }
            CurrentActionTransition::HookStarted { hook } => {
                if hook.batch_id.is_nil() || !hook.turn.valid() { return Err("invalid hook registration"); }
            }
            CurrentActionTransition::HookFinished { batch_id } => {
                if batch_id.is_nil() { return Err("invalid hook terminal"); }
            }
            CurrentActionTransition::CompactChanged { compact } => {
                if !compact.turn.valid() || !compact.owner.valid() { return Err("invalid compaction source"); }
            }
            CurrentActionTransition::CompactFinished { owner } => {
                if !owner.valid() { return Err("invalid compaction terminal"); }
            }
            CurrentActionTransition::InteractionStarted { interaction } => {
                if interaction.tool_call_id.is_empty() || !matches!(&interaction.source, InteractionSource::Registered { registration_id, turn, .. } if !registration_id.is_nil() && turn.valid()) {
                    return Err("invalid interaction registration");
                }
            }
            CurrentActionTransition::InteractionFinished { tool_call_id, registration_id } => {
                if tool_call_id.is_empty() || registration_id.is_nil() { return Err("invalid interaction terminal"); }
            }
        }
        Ok(())
    }
}

pub(crate) enum SamplingProgress {
    Reserved(RequestAttemptId),
    RetryScheduled(RequestAttemptId, RetryDeadlineProjection),
    Submitted(RequestAttemptId, MonotonicPoint),
    Completed(RequestAttemptId),
    Discarded(RequestAttemptId),
}

impl SamplingProgress {
    pub(crate) fn attempt(&self) -> RequestAttemptId {
        match self {
            Self::Reserved(attempt) | Self::RetryScheduled(attempt, _)
            | Self::Submitted(attempt, _) | Self::Completed(attempt)
            | Self::Discarded(attempt) => *attempt,
        }
    }

    pub(crate) fn active(&self) -> bool {
        match self {
            Self::Reserved(_) | Self::RetryScheduled(_, _) | Self::Submitted(_, _) => true,
            Self::Completed(_) | Self::Discarded(_) => false,
        }
    }
}

pub(crate) struct SamplingRequest {
    pub(crate) prompt_id: String,
    pub(crate) turn: FrozenOwner,
    pub(crate) progress: SamplingProgress,
}

struct ToolState {
    current: CurrentTool,
    status: ToolStatus,
}

enum ToolStatus { Active, Settled }

enum TurnObservation { Unregistered, Active(CurrentTurn), Settled(FrozenOwner) }

pub(crate) struct State {
    version: Option<SourceVersion>,
    turn: TurnObservation,
    pub(crate) requests: HashMap<RequestId, SamplingRequest>,
    tools: HashMap<String, ToolState>,
    compactions: HashMap<CompactKey, CurrentCompact>,
    hooks: HashMap<uuid::Uuid, CurrentHook>,
    interactions: HashMap<String, InteractionSource>,
}

impl State {
    pub(crate) fn changed(&mut self) {
        self.version = self.version.and_then(|version| version.sequence.checked_add(1)
            .map(|sequence| SourceVersion { sequence, ..version }));
    }

    pub(crate) fn owner(&self) -> FrozenOwner {
        match &self.turn {
            TurnObservation::Active(turn) => turn.owner.clone(),
            TurnObservation::Unregistered | TurnObservation::Settled(_) => FrozenOwner::Unavailable,
        }
    }

    pub(crate) fn sampling_turn_owner(&self, prompt_id: &str) -> FrozenOwner {
        match &self.turn {
            TurnObservation::Active(turn) if turn.owner.prompt_id() == Some(prompt_id) => turn.owner.clone(),
            TurnObservation::Settled(owner) if owner.prompt_id() == Some(prompt_id) => owner.clone(),
            TurnObservation::Active(_) | TurnObservation::Unregistered | TurnObservation::Settled(_) => FrozenOwner::Unavailable,
        }
    }

    fn observes(&self, owner: &FrozenOwner) -> bool {
        match &self.turn {
            TurnObservation::Active(turn) => owner == &FrozenOwner::Unavailable || *owner == turn.owner,
            TurnObservation::Unregistered => true,
            TurnObservation::Settled(_) => false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Source(Arc<Inner>);

struct Inner {
    state: Mutex<State>,
    emitter: std::sync::OnceLock<Emitter>,
    replay_queue: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<super::replay_events::SessionEvent>>,
    delivery: Mutex<crate::extensions::current_actions::DeliveryRole>,
}

struct Emitter {
    gateway: xai_acp_lib::AcpAgentGatewaySender,
    enabled: Arc<std::sync::atomic::AtomicBool>,
    session_id: agent_client_protocol::SessionId,
}

impl Default for Source {
    fn default() -> Self {
        Self(Arc::new(Inner { state: Mutex::new(State {
            version: Some(SourceVersion { incarnation: uuid::Uuid::new_v4(), sequence: 0 }),
            turn: TurnObservation::Unregistered, requests: HashMap::new(), tools: HashMap::new(),
            compactions: HashMap::new(), hooks: HashMap::new(), interactions: HashMap::new(),
        }), emitter: std::sync::OnceLock::new(), replay_queue: std::sync::OnceLock::new(),
            delivery: Mutex::new(Default::default()) }))
    }
}

impl Source {
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> { self.0.state.lock() }

    pub(crate) fn attach_emitter(&self, gateway: xai_acp_lib::AcpAgentGatewaySender, enabled: Arc<std::sync::atomic::AtomicBool>, session_id: agent_client_protocol::SessionId) {
        let _ = self.0.emitter.set(Emitter { gateway, enabled, session_id });
    }

    fn publish(&self, version: Option<SourceVersion>, transition: CurrentActionTransition) {
        let Some(emitter) = self.0.emitter.get() else { return; };
        if !emitter.enabled.load(std::sync::atomic::Ordering::Relaxed) { return; }
        let Some(version) = version else { return; };
        let event = CurrentActionsEvent { session_id: emitter.session_id.0.to_string(), version, transition };
        match crate::extensions::current_actions::transition_notification(&event, &self.delivery_role()) {
            Ok(notification) => { emitter.gateway.forward_fire_and_forget(notification); }
            Err(error) => {
                emitter.gateway.fail_publication();
                tracing::error!(%error, "current-actions transition serialization failed");
            }
        }
    }

    pub(crate) fn version(&self) -> Option<SourceVersion> { self.lock().version }

    pub(crate) fn scoped_owner(&self) -> FrozenOwner {
        let state = self.lock();
        match TURN_PUBLICATION.try_with(Clone::clone).ok() {
            Some(task_id) if state.owner().task_id() == Some(task_id.as_str()) => state.owner(),
            Some(_) | None => FrozenOwner::Unavailable,
        }
    }

    pub(crate) fn reader(&self, pending: PendingInteractions, session_id: String) -> Reader {
        Reader { source: self.clone(), pending, session_id }
    }

    pub(crate) fn start_turn(&self, owner: FrozenOwner, started_at: MonotonicPoint) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let turn = CurrentTurn { owner, started_at, phase: CurrentTurnPhase::Admitted };
        state.turn = TurnObservation::Active(turn.clone());
        state.tools.retain(|_, tool| matches!(tool.status, ToolStatus::Active));
        state.changed();
        let version = state.version;
        drop(state);
        self.publish(version, CurrentActionTransition::TurnRegistered { turn });
    }

    fn project_turn_anchor(&self, task_id: &str, started_at: MonotonicPoint) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        match &mut state.turn {
            TurnObservation::Active(turn) if turn.owner.task_id() == Some(task_id) => {
                match turn.phase {
                    CurrentTurnPhase::Admitted => turn.started_at = started_at,
                    CurrentTurnPhase::Running { .. } | CurrentTurnPhase::AwaitingResponse { .. } => return,
                }
            }
            TurnObservation::Active(_) | TurnObservation::Unregistered | TurnObservation::Settled(_) => return,
        }
        state.changed();
        let version = state.version;
        let TurnObservation::Active(turn) = &state.turn else { return; };
        let turn = turn.clone();
        drop(state);
        self.publish(version, CurrentActionTransition::TurnAnchored { turn });
    }

    fn begin_turn_execution(&self, task_id: &str, started_at: MonotonicPoint) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        match &mut state.turn {
            TurnObservation::Active(turn) if turn.owner.task_id() == Some(task_id) => {
                match turn.phase {
                    CurrentTurnPhase::Admitted => turn.phase = CurrentTurnPhase::Running { started_at },
                    CurrentTurnPhase::Running { .. } | CurrentTurnPhase::AwaitingResponse { .. } => return,
                }
            }
            TurnObservation::Active(_) | TurnObservation::Unregistered | TurnObservation::Settled(_) => return,
        }
        state.changed();
        let version = state.version;
        let TurnObservation::Active(turn) = &state.turn else { return; };
        let turn = turn.clone();
        drop(state);
        self.publish(version, CurrentActionTransition::TurnStarted { turn });
    }

    fn finish_turn_backend(&self, task_id: &str, started_at: MonotonicPoint) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        match &mut state.turn {
            TurnObservation::Active(turn) if turn.owner.task_id() == Some(task_id) => {
                match turn.phase {
                    CurrentTurnPhase::Admitted | CurrentTurnPhase::Running { .. } => turn.phase = CurrentTurnPhase::AwaitingResponse { started_at },
                    CurrentTurnPhase::AwaitingResponse { .. } => return,
                }
            }
            TurnObservation::Active(_) | TurnObservation::Unregistered | TurnObservation::Settled(_) => return,
        }
        state.changed();
        let version = state.version;
        let TurnObservation::Active(turn) = &state.turn else { return; };
        let turn = turn.clone();
        drop(state);
        self.publish(version, CurrentActionTransition::TurnBackendFinished { turn });
    }

    pub(crate) fn finish_turn(&self, task_id: &str) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let owner = match &state.turn {
            TurnObservation::Active(turn) if turn.owner.task_id() == Some(task_id) => turn.owner.clone(),
            TurnObservation::Active(_) | TurnObservation::Unregistered | TurnObservation::Settled(_) => return,
        };
        state.turn = TurnObservation::Settled(owner.clone());
        for tool in state.tools.values_mut() {
            if tool.current.turn.task_id() == Some(task_id) { tool.status = ToolStatus::Settled; }
        }
        state.hooks.retain(|_, hook| hook.turn.task_id() != Some(task_id));
        state.changed();
        let version = state.version;
        drop(state);
        self.publish(version, CurrentActionTransition::TurnFinished { owner });
    }

    pub(crate) fn register_tool(&self, ownership: ToolCallOwnership, registered_at: MonotonicPoint) -> FrozenOwner {
        let ToolCallOwnership::Registered { invocation_id, source, .. } = &ownership else { return FrozenOwner::Unavailable; };
        let _publication = self.begin_publication();
        let mut state = self.lock();
        if let Some(tool) = state.tools.get(invocation_id) { return tool.current.turn.clone(); }
        let task = TURN_PUBLICATION.try_with(Clone::clone).ok();
        let turn = match source {
            ToolCallSource::Sampling { prompt_id, request_id, attempt } => state.requests.get(&RequestId::from(request_id.as_str()))
                .filter(|request| request.prompt_id == *prompt_id && request.progress.attempt().0 == *attempt)
                .map(|request| request.turn.clone()).unwrap_or(FrozenOwner::Unavailable),
            ToolCallSource::Unknown => match task.as_deref() {
                Some(task_id) if state.owner().task_id() == Some(task_id) => state.owner(),
                Some(_) | None => FrozenOwner::Unavailable,
            },
        };
        let status = if task.as_deref().is_some_and(|task_id| state.owner().task_id() != Some(task_id)) {
            ToolStatus::Settled
        } else { ToolStatus::Active };
        let key = invocation_id.clone();
        state.tools.insert(key, ToolState {
            current: CurrentTool { ownership, turn: turn.clone(), registered_at, executions: Vec::new() },
            status,
        });
        state.changed();
        turn
    }

    pub(crate) fn tool_execution(&self, ownership: &ToolCallOwnership, timing: ToolExecutionTiming) {
        let ToolCallOwnership::Registered { invocation_id, .. } = ownership else { return; };
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let Some(tool) = state.tools.get_mut(invocation_id) else { return; };
        if tool.current.ownership != *ownership || matches!(tool.status, ToolStatus::Settled) { return; }
        let execution_id = match timing {
            ToolExecutionTiming::Unknown => return,
            ToolExecutionTiming::Started { execution_id, .. }
            | ToolExecutionTiming::Finished { execution_id, .. } => execution_id,
        };
        let existing = tool.current.executions.iter_mut().find(|span| match span {
            ToolExecutionTiming::Unknown => false,
            ToolExecutionTiming::Started { execution_id: id, .. }
            | ToolExecutionTiming::Finished { execution_id: id, .. } => *id == execution_id,
        });
        match existing {
            Some(ToolExecutionTiming::Finished { .. }) => return,
            Some(span) => *span = timing,
            None => tool.current.executions.push(timing),
        }
        state.changed();
    }

    pub(crate) fn settle_tool(&self, invocation_id: &str) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let Some(tool) = state.tools.get_mut(invocation_id) else { return; };
        if matches!(tool.status, ToolStatus::Settled) { return; }
        tool.status = ToolStatus::Settled;
        state.changed();
    }

    pub(crate) fn start_hook(&self, batch_id: uuid::Uuid, prompt_id: Option<&str>, started_at: MonotonicPoint) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let scope = TURN_PUBLICATION.try_with(Clone::clone).ok();
        if scope.as_deref().is_some_and(|task_id| state.owner().task_id() != Some(task_id)) { return; }
        let turn = match scope {
            Some(_) if prompt_id.is_none() || state.owner().prompt_id() == prompt_id => state.owner(),
            Some(_) | None => FrozenOwner::Unavailable,
        };
        let hook = CurrentHook { batch_id, turn, started_at };
        state.hooks.insert(batch_id, hook.clone());
        state.changed();
        let version = state.version;
        drop(state);
        self.publish(version, CurrentActionTransition::HookStarted { hook });
    }

    pub(crate) fn finish_hook(&self, batch_id: uuid::Uuid) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        if state.hooks.remove(&batch_id).is_none() { return; }
        state.changed();
        let version = state.version;
        drop(state);
        self.publish(version, CurrentActionTransition::HookFinished { batch_id });
    }

    pub(crate) fn compact_stage(&self, owner: CompactOwner, stage: CompactStage, started_at: MonotonicPoint) {
        let id = CompactKey::of(&owner);
        let _publication = self.begin_publication();
        let mut state = self.lock();
        if TURN_PUBLICATION.try_with(Clone::clone).ok().as_deref()
            .is_some_and(|task_id| state.owner().task_id() != Some(task_id))
        { return; }
        if let Some(compact) = state.compactions.get_mut(&id) {
            if compact.owner != owner || compact.stage == stage { return; }
            compact.stage = stage;
            compact.stage_started_at = started_at;
        } else {
            let turn = match &owner {
                CompactOwner::Inline => state.owner(),
                CompactOwner::Manual { .. } => FrozenOwner::Unavailable,
                CompactOwner::Automatic { invocation } => {
                    let turn = state.owner();
                    if turn.prompt_id() == invocation.prompt_id.as_deref() { turn } else { FrozenOwner::Unavailable }
                }
            };
            state.compactions.insert(id, CurrentCompact {
                owner, turn, stage, admitted_at: started_at, stage_started_at: started_at,
            });
        }
        state.changed();
        let version = state.version;
        let Some(compact) = state.compactions.get(&id).cloned() else { return; };
        drop(state);
        self.publish(version, CurrentActionTransition::CompactChanged { compact });
    }

    pub(crate) fn finish_compact(&self, id: CompactKey) {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let Some(compact) = state.compactions.remove(&id) else { return; };
        state.changed();
        let version = state.version;
        drop(state);
        self.publish(version, CurrentActionTransition::CompactFinished { owner: compact.owner });
    }

    pub(crate) fn start_interaction(&self, pending: &PendingInteractions, tool_call_id: &str, kind: PendingKind, turn: FrozenOwner, started_at: MonotonicPoint) -> uuid::Uuid {
        let registration_id = uuid::Uuid::new_v4();
        let _publication = self.begin_publication();
        let mut state = self.lock();
        let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
        pending.insert(tool_call_id.to_owned(), kind);
        let source = InteractionSource::Registered { registration_id, turn, started_at };
        state.interactions.insert(tool_call_id.to_owned(), source.clone());
        state.changed();
        let version = state.version;
        drop(pending);
        drop(state);
        self.publish(version, CurrentActionTransition::InteractionStarted {
            interaction: CurrentInteraction { tool_call_id: tool_call_id.to_owned(), kind, source },
        });
        registration_id
    }

    pub(crate) fn finish_interaction(&self, pending: &PendingInteractions, tool_call_id: &str, registration_id: uuid::Uuid) -> bool {
        let _publication = self.begin_publication();
        let mut state = self.lock();
        if !matches!(state.interactions.get(tool_call_id), Some(InteractionSource::Registered { registration_id: current, .. }) if *current == registration_id) { return false; }
        let removed = pending.lock().unwrap_or_else(|error| error.into_inner()).remove(tool_call_id).is_some();
        state.interactions.remove(tool_call_id);
        if removed { state.changed(); }
        let version = state.version;
        drop(state);
        if removed {
            self.publish(version, CurrentActionTransition::InteractionFinished { tool_call_id: tool_call_id.to_owned(), registration_id });
        }
        removed
    }
}

#[derive(Clone)]
pub struct Reader {
    source: Source,
    pending: PendingInteractions,
    session_id: String,
}

impl Reader {
    pub fn snapshot(&self, binding: SnapshotBinding) -> CurrentActionsSnapshot {
        if binding.session_id != self.session_id { return CurrentActionsSnapshot::Unavailable; }
        let state = self.source.lock();
        let Some(version) = state.version else { return CurrentActionsSnapshot::Unavailable; };
        let pending = self.pending.lock().unwrap_or_else(|error| error.into_inner());
        let sampling = state.requests.iter().filter_map(|(request_id, request)| {
            if !state.observes(&request.turn) { return None; }
            let phase = match &request.progress {
                SamplingProgress::Reserved(_) => CurrentSamplingPhase::Reserved,
                SamplingProgress::RetryScheduled(_, deadline) => CurrentSamplingPhase::RetryScheduled { deadline: deadline.clone() },
                SamplingProgress::Submitted(_, started_at) => CurrentSamplingPhase::Submitted { started_at: *started_at },
                SamplingProgress::Completed(_) | SamplingProgress::Discarded(_) => return None,
            };
            Some(CurrentSampling {
                owner: SamplingOwner { prompt_id: request.prompt_id.clone(), request_id: request_id.clone(), attempt: request.progress.attempt() },
                turn: request.turn.clone(), phase,
            })
        }).collect();
        CurrentActionsSnapshot::Current(CurrentActions {
            binding, version, event_frontier: crate::util::event_id::next_event_sequence(), turn: match &state.turn {
                TurnObservation::Active(turn) => Some(turn.clone()),
                TurnObservation::Unregistered | TurnObservation::Settled(_) => None,
            }, sampling,
            tools: state.tools.values().filter_map(|tool| match tool.status {
                ToolStatus::Active if tool.current.turn.task_id().is_none() || state.observes(&tool.current.turn) => Some(tool.current.clone()),
                ToolStatus::Active | ToolStatus::Settled => None,
            }).collect(),
            backend_tools: BackendToolsObservation::Unavailable,
            compactions: state.compactions.values().filter(|compact| compact.turn.task_id().is_none() || state.observes(&compact.turn)).cloned().collect(),
            hooks: state.hooks.values().filter(|hook| hook.turn.task_id().is_none() || state.observes(&hook.turn)).cloned().collect(),
            interactions: pending.iter().filter_map(|(id, kind)| {
                let source = state.interactions.get(id).cloned().unwrap_or(InteractionSource::Unavailable);
                if let InteractionSource::Registered { turn, .. } = &source
                    && turn.task_id().is_some() && !state.observes(turn)
                { return None; }
                Some(CurrentInteraction { tool_call_id: id.clone(), kind: *kind, source })
            }).collect(),
        })
    }
}

pub(crate) struct TurnRegistration { source: Source, task_id: String, state: TurnRegistrationState }

enum TurnRegistrationState { Executing, AwaitingResponse }

#[derive(Default)]
pub(crate) enum TurnOwnership {
    #[default]
    Unavailable,
    Registered { source: Source, task_id: String },
}

impl TurnOwnership {
    pub(crate) fn new(source: Source, task_id: String) -> Self { Self::Registered { source, task_id } }

    pub(crate) fn finish(&self) {
        match self {
            Self::Unavailable => {}
            Self::Registered { source, task_id } => source.finish_turn(task_id),
        }
    }
}

impl Drop for TurnOwnership {
    fn drop(&mut self) { self.finish(); }
}

impl TurnRegistration {
    pub(crate) fn new(source: Source, task_id: String) -> Self { Self { source, task_id, state: TurnRegistrationState::Executing } }

    pub(crate) fn backend_finished(&mut self) {
        self.source.finish_turn_backend(&self.task_id, MonotonicPoint::at(tokio::time::Instant::now()));
        self.state = TurnRegistrationState::AwaitingResponse;
    }

    pub(crate) fn project_anchor(&self, started_at: std::time::Instant) {
        self.source.project_turn_anchor(&self.task_id, MonotonicPoint::at(tokio::time::Instant::from_std(started_at)));
    }

    pub(crate) fn begin_execution(&self) {
        self.source.begin_turn_execution(&self.task_id, MonotonicPoint::at(tokio::time::Instant::now()));
    }
}

impl Drop for TurnRegistration {
    fn drop(&mut self) {
        match self.state {
            TurnRegistrationState::Executing => self.source.finish_turn(&self.task_id),
            TurnRegistrationState::AwaitingResponse => {}
        }
    }
}

#[derive(Clone, Default)]
pub(crate) enum ToolRegistration {
    #[default]
    Unavailable,
    Registered(Arc<RegisteredTool>),
}

pub(crate) struct RegisteredTool {
    source: Source,
    invocation_id: String,
    provider_call_id: String,
    call_source: ToolCallSource,
    owner: FrozenOwner,
}

impl std::fmt::Debug for ToolRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Unavailable => f.write_str("Unavailable"), Self::Registered(_) => f.write_str("Registered") }
    }
}

impl ToolRegistration {
    pub(crate) fn new(source: Source, ownership: ToolCallOwnership, started_at: MonotonicPoint) -> Self {
        let ToolCallOwnership::Registered { invocation_id, provider_call_id, source: call_source } = &ownership else { return Self::Unavailable; };
        let invocation_id = invocation_id.clone();
        let provider_call_id = provider_call_id.clone();
        let call_source = call_source.clone();
        let owner = source.register_tool(ownership, started_at);
        Self::Registered(Arc::new(RegisteredTool { source, invocation_id, provider_call_id, call_source, owner }))
    }

    pub(crate) fn owner(&self) -> FrozenOwner {
        match self {
            Self::Unavailable => FrozenOwner::Unavailable,
            Self::Registered(tool) => tool.owner.clone(),
        }
    }

    pub(crate) fn finish(&self) {
        match self {
            Self::Unavailable => {}
            Self::Registered(tool) => tool.source.settle_tool(&tool.invocation_id),
        }
    }
}

impl Drop for RegisteredTool {
    fn drop(&mut self) { self.source.settle_tool(&self.invocation_id); }
}

#[derive(Clone, Default)]
pub(crate) enum HookRegistration {
    #[default]
    Unavailable,
    Registered(Arc<RegisteredHook>),
}

pub(crate) struct RegisteredHook { source: Source, batch_id: uuid::Uuid }

impl std::fmt::Debug for HookRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Unavailable => f.write_str("Unavailable"), Self::Registered(hook) => std::fmt::Debug::fmt(&hook.batch_id, f) }
    }
}

impl PartialEq for HookRegistration {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Unavailable, Self::Unavailable) => true,
            (Self::Registered(left), Self::Registered(right)) => left.batch_id == right.batch_id,
            (Self::Unavailable, Self::Registered(_)) | (Self::Registered(_), Self::Unavailable) => false,
        }
    }
}
impl Eq for HookRegistration {}

impl HookRegistration {
    pub(crate) fn new(source: Source, prompt_id: Option<&str>, started_at: MonotonicPoint) -> Self {
        let batch_id = uuid::Uuid::new_v4();
        source.start_hook(batch_id, prompt_id, started_at);
        Self::Registered(Arc::new(RegisteredHook { source, batch_id }))
    }

    pub(crate) fn finish(&self) {
        match self {
            Self::Unavailable => {}
            Self::Registered(hook) => hook.source.finish_hook(hook.batch_id),
        }
    }
}

impl Drop for RegisteredHook {
    fn drop(&mut self) { self.source.finish_hook(self.batch_id); }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CompactKey { Inline, Manual(uuid::Uuid), Automatic(uuid::Uuid) }

impl CompactKey {
    fn of(owner: &CompactOwner) -> Self {
        match owner {
            CompactOwner::Inline => Self::Inline,
            CompactOwner::Manual { request_id, .. } => Self::Manual(*request_id),
            CompactOwner::Automatic { invocation } => Self::Automatic(invocation.id.0),
        }
    }
}

pub(crate) struct CompactRegistration { source: Source, id: CompactKey }

impl CompactRegistration {
    pub(crate) fn new(source: Source, owner: CompactOwner, stage: CompactStage, started_at: MonotonicPoint) -> Self {
        let id = CompactKey::of(&owner);
        source.compact_stage(owner, stage, started_at);
        Self { source, id }
    }

    pub(crate) fn respond(&self, respond_to: tokio::sync::oneshot::Sender<agent_client_protocol::Result<()>>, result: agent_client_protocol::Result<()>) {
        let mut state = self.source.lock();
        let _ = respond_to.send(result);
        let Some(compact) = state.compactions.remove(&self.id) else { return; };
        state.changed();
        let version = state.version;
        drop(state);
        self.source.publish(version, CurrentActionTransition::CompactFinished { owner: compact.owner });
    }

    pub(crate) fn stage(&self, stage: CompactStage) {
        let point = MonotonicPoint::at(tokio::time::Instant::now());
        let mut state = self.source.lock();
        let Some(compact) = state.compactions.get_mut(&self.id) else { return; };
        if compact.stage == stage { return; }
        compact.stage = stage;
        compact.stage_started_at = point;
        let compact = compact.clone();
        state.changed();
        let version = state.version;
        drop(state);
        self.source.publish(version, CurrentActionTransition::CompactChanged { compact });
    }
}

impl Drop for CompactRegistration {
    fn drop(&mut self) { self.source.finish_compact(self.id); }
}

tokio::task_local! {
    static TOOL_PUBLICATION: ToolRegistration;
    static TURN_PUBLICATION: String;
}

impl TurnRegistration {
    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        TURN_PUBLICATION.scope(self.task_id.clone(), future).await
    }
}

impl ToolRegistration {
    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        TOOL_PUBLICATION.scope(self.clone(), future).await
    }

    fn ownership(&self) -> Option<ToolCallOwnership> {
        match self {
            Self::Unavailable => None,
            Self::Registered(tool) => Some(ToolCallOwnership::Registered {
                invocation_id: tool.invocation_id.clone(),
                provider_call_id: tool.provider_call_id.clone(),
                source: tool.call_source.clone(),
            }),
        }
    }
}

pub(crate) fn observe_tool_publication(update: &mut agent_client_protocol::SessionUpdate) -> Result<(), serde_json::Error> {
    let agent_client_protocol::SessionUpdate::ToolCallUpdate(update) = update else { return Ok(()); };
    let ownership = TOOL_PUBLICATION.try_with(|registration| {
        let ownership = registration.ownership();
        if matches!(update.fields.status, Some(agent_client_protocol::ToolCallStatus::Completed | agent_client_protocol::ToolCallStatus::Failed)) {
            registration.finish();
        }
        ownership
    }).ok().flatten();
    if let Some(ownership) = ownership {
        crate::extensions::tool_timing::stamp_ownership(update.meta.get_or_insert_with(agent_client_protocol::Meta::new), &ownership)?;
    }
    Ok(())
}

pub(crate) fn tool_publication_owner() -> FrozenOwner {
    TOOL_PUBLICATION.try_with(ToolRegistration::owner).unwrap_or(FrozenOwner::Unavailable)
}
