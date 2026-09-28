use crate::{ContextCategory, ConversationItem, RuntimeNotice, SyntheticReason};

pub fn classify_item(item: &ConversationItem) -> ContextCategory {
    match item {
        ConversationItem::System(system) => match &system.synthetic_reason {
            SyntheticReason::Primary => ContextCategory::CoreInstructions,
            SyntheticReason::Human | SyntheticReason::Unknown => ContextCategory::OtherRuntime,
            reason => category_of(reason),
        },
        ConversationItem::User(user) => match &user.synthetic_reason {
            SyntheticReason::Human => ContextCategory::UserTurns,
            SyntheticReason::Unknown => ContextCategory::OtherRuntime,
            reason => category_of(reason),
        },
        ConversationItem::Assistant(_) => ContextCategory::AssistantTurns,
        ConversationItem::ToolResult(_) | ConversationItem::BackendToolCall(_) => {
            ContextCategory::ToolExchanges
        }
        ConversationItem::Reasoning(_) => ContextCategory::Reasoning,
    }
}

fn category_of(reason: SyntheticReason) -> ContextCategory {
    match reason {
        SyntheticReason::CompactionMeta => ContextCategory::CompactionSummary,
        SyntheticReason::ProjectInstructions => ContextCategory::ProjectInstructions,
        SyntheticReason::SessionPrefix => ContextCategory::Environment,
        SyntheticReason::DirectBash | SyntheticReason::GoalSetup => ContextCategory::UserTurns,
        SyntheticReason::SystemReminder => ContextCategory::RuntimeNotices(RuntimeNotice::SystemReminder),
        SyntheticReason::LengthContinue => ContextCategory::RuntimeNotices(RuntimeNotice::LengthContinue),
        SyntheticReason::AutoContinue => ContextCategory::RuntimeNotices(RuntimeNotice::AutoContinue),
        SyntheticReason::AutoRecovery => ContextCategory::RuntimeNotices(RuntimeNotice::AutoRecovery),
        SyntheticReason::Interjection => ContextCategory::RuntimeNotices(RuntimeNotice::Interjection),
        SyntheticReason::AgentMessage | SyntheticReason::ParentHumanMessage => {
            ContextCategory::RuntimeNotices(RuntimeNotice::AgentMessage)
        }
        SyntheticReason::TaskCompleted => ContextCategory::RuntimeNotices(RuntimeNotice::TaskCompleted),
        SyntheticReason::SubagentCompleted => ContextCategory::RuntimeNotices(RuntimeNotice::SubagentCompleted),
        SyntheticReason::NotificationDrain => ContextCategory::RuntimeNotices(RuntimeNotice::NotificationDrain),
        SyntheticReason::GoalSummary => ContextCategory::RuntimeNotices(RuntimeNotice::GoalSummary),
        SyntheticReason::GoalClassifierNudge => ContextCategory::RuntimeNotices(RuntimeNotice::GoalClassifierNudge),
        SyntheticReason::SchedulerFired => ContextCategory::RuntimeNotices(RuntimeNotice::SchedulerFired),
        SyntheticReason::StopHookFeedback => ContextCategory::RuntimeNotices(RuntimeNotice::StopHookFeedback),
        SyntheticReason::WorkingDirectorySwitch => ContextCategory::RuntimeNotices(RuntimeNotice::WorkingDirectorySwitch),
        SyntheticReason::Primary => ContextCategory::CoreInstructions,
        SyntheticReason::Human | SyntheticReason::Unknown => ContextCategory::OtherRuntime
    }
}
