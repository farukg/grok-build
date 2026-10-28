use super::super::SessionUpdate;
use crate::extensions::notification::SessionUpdate as XaiUpdate;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum SummaryActivity {
    #[default]
    Metadata,
    Observation,
    Occurred,
}

impl SummaryActivity {
    pub(crate) fn from_recorded(recorded: bool) -> Self {
        if recorded { Self::Occurred } else { Self::Metadata }
    }

    pub(crate) fn for_update(update: &SessionUpdate) -> Self {
        let replay = match update {
            SessionUpdate::Acp(notification) => notification.meta.as_ref()
                .and_then(|meta| meta.get("isReplay")),
            SessionUpdate::Xai(notification) => notification.meta.as_ref()
                .and_then(|meta| meta.get("isReplay")),
        }.and_then(serde_json::Value::as_bool).unwrap_or(false);
        if replay {
            return Self::Observation;
        }
        match update {
            SessionUpdate::Acp(notification) => match &notification.update {
                agent_client_protocol::SessionUpdate::UserMessageChunk(_)
                | agent_client_protocol::SessionUpdate::AgentMessageChunk(_)
                | agent_client_protocol::SessionUpdate::AgentThoughtChunk(_)
                | agent_client_protocol::SessionUpdate::ToolCall(_)
                | agent_client_protocol::SessionUpdate::ToolCallUpdate(_) => Self::Occurred,
                _ => Self::Observation,
            },
            SessionUpdate::Xai(notification) => match &notification.update {
                XaiUpdate::BackgroundTasks { .. } => match
                    crate::session::acp_types::SessionActivityProvenance::from_snapshot_meta(notification.meta.as_ref())
                {
                    crate::session::acp_types::SessionActivityProvenance::Live => Self::Occurred,
                    crate::session::acp_types::SessionActivityProvenance::Observation => Self::Observation,
                },
                XaiUpdate::RequestSourceObserved { .. }
                | XaiUpdate::RelaySyncStatus(_)
                | XaiUpdate::HooksChanged { .. }
                | XaiUpdate::PluginsChanged { .. }
                | XaiUpdate::PluginUpdatesInstalled { .. }
                | XaiUpdate::SessionStatus(_)
                | XaiUpdate::SessionSummaryGenerated { .. }
                | XaiUpdate::SessionRecap { .. }
                | XaiUpdate::SessionRecapUnavailable
                | XaiUpdate::LastTurnSummary { .. }
                | XaiUpdate::CompactionCheckpoint(_)
                | XaiUpdate::ModelChanged { .. }
                | XaiUpdate::ServedModel { .. }
                | XaiUpdate::ModelServed { .. }
                | XaiUpdate::MemoryFiles { .. }
                | XaiUpdate::WorkflowUpdated { .. }
                | XaiUpdate::GoalUpdated { .. }
                | XaiUpdate::PendingInteraction { .. }
                | XaiUpdate::TurnUsage { .. }
                | XaiUpdate::Unknown => Self::Observation,
                XaiUpdate::DiffReview { .. }
                | XaiUpdate::RetryState(_)
                | XaiUpdate::SamplingLifecycle { .. }
                | XaiUpdate::AutoCompactStarted { .. }
                | XaiUpdate::AutoCompactCompleted { .. }
                | XaiUpdate::AutoCompactFailed { .. }
                | XaiUpdate::AutoCompactCancelled { .. }
                | XaiUpdate::AutoContinueCompleted { .. }
                | XaiUpdate::MemoryFlushStarted
                | XaiUpdate::MemoryFlushCompleted { .. }
                | XaiUpdate::MemoryCaptureActivity { .. }
                | XaiUpdate::MemoryDreamQueued
                | XaiUpdate::MemoryDreamStarted { .. }
                | XaiUpdate::MemoryDreamCompleted { .. }
                | XaiUpdate::MemorySessionSaved { .. }
                | XaiUpdate::FeedbackRequest(_)
                | XaiUpdate::AutoRecoveryStarted { .. }
                | XaiUpdate::AutoRecoveryExhausted { .. }
                | XaiUpdate::HookAnnotation { .. }
                | XaiUpdate::HookRunStarted { .. }
                | XaiUpdate::HookExecution { .. }
                | XaiUpdate::RewindMarker { .. }
                | XaiUpdate::ContextItemsRemoved { .. }
                | XaiUpdate::TaskCompleted { .. }
                | XaiUpdate::SubagentSpawned { .. }
                | XaiUpdate::SubagentProgress { .. }
                | XaiUpdate::SubagentFinished { .. }
                | XaiUpdate::TaskBackgrounded { .. }
                | XaiUpdate::ScheduledTaskCreated { .. }
                | XaiUpdate::ScheduledTaskFired { .. }
                | XaiUpdate::ScheduledTaskDeleted { .. }
                | XaiUpdate::MonitorEvent { .. }
                | XaiUpdate::ModelAutoSwitched { .. }
                | XaiUpdate::ToolCallDeltaChunk { .. }
                | XaiUpdate::ImageCompressed { .. }
                | XaiUpdate::ImageDropped { .. }
                | XaiUpdate::InjectedContextRecord { .. }
                | XaiUpdate::InteractionResolved { .. }
                | XaiUpdate::PlanKept { .. }
                | XaiUpdate::PlanCleared
                | XaiUpdate::PlanExecuting
                | XaiUpdate::TurnCompleted { .. }
                | XaiUpdate::ResponseStarted { .. }
                | XaiUpdate::ReasoningCompleted { .. }
                | XaiUpdate::ResponseCompleted { .. } => Self::Occurred,
            },
        }
    }
}
