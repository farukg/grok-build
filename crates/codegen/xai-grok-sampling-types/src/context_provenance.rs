use crate::{ContextCategory, ConversationItem, RuntimeNotice, SyntheticReason};

impl ConversationItem {
    /// The context category a whole item belongs to, derived from its variant and synthetic origin.
    pub fn context_category(&self) -> ContextCategory {
        match self {
            Self::System(system) => match &system.synthetic_reason {
                SyntheticReason::Human => ContextCategory::OtherRuntime,
                reason => reason.context_category(),
            },
            Self::User(user) => user.synthetic_reason.context_category(),
            Self::Assistant(_) => ContextCategory::AssistantTurns,
            Self::ToolResult(_) | Self::BackendToolCall(_) => ContextCategory::ToolExchanges,
            Self::Reasoning(_) => ContextCategory::Reasoning,
        }
    }
}

impl SyntheticReason {
    fn context_category(&self) -> ContextCategory {
        let notice = ContextCategory::RuntimeNotices;
        match self {
            Self::Human | Self::ParentHumanMessage | Self::DirectBash | Self::GoalSetup => {
                ContextCategory::UserTurns
            }
            Self::Primary => ContextCategory::CoreInstructions,
            Self::ProjectInstructions => ContextCategory::ProjectInstructions,
            Self::SessionPrefix => ContextCategory::Environment,
            Self::CompactionMeta => ContextCategory::CompactionSummary,
            Self::SystemReminder => notice(RuntimeNotice::SystemReminder),
            Self::LengthContinue => notice(RuntimeNotice::LengthContinue),
            Self::AutoContinue => notice(RuntimeNotice::AutoContinue),
            Self::AutoRecovery => notice(RuntimeNotice::AutoRecovery),
            Self::Interjection => notice(RuntimeNotice::Interjection),
            Self::AgentMessage => notice(RuntimeNotice::AgentMessage),
            Self::TaskCompleted => notice(RuntimeNotice::TaskCompleted),
            Self::SubagentCompleted => notice(RuntimeNotice::SubagentCompleted),
            Self::NotificationDrain => notice(RuntimeNotice::NotificationDrain),
            Self::GoalSummary => notice(RuntimeNotice::GoalSummary),
            Self::GoalClassifierNudge => notice(RuntimeNotice::GoalClassifierNudge),
            Self::SchedulerFired => notice(RuntimeNotice::SchedulerFired),
            Self::StopHookFeedback => notice(RuntimeNotice::StopHookFeedback),
            Self::WorkingDirectorySwitch => notice(RuntimeNotice::WorkingDirectorySwitch),
            Self::Unknown => ContextCategory::OtherRuntime,
        }
    }
}
