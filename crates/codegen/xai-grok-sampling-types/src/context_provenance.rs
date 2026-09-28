use crate::{ContextCategory, ConversationItem, RuntimeNotice, SyntheticReason};

pub fn classify_item(item: &ConversationItem) -> ContextCategory {
    match item {
        ConversationItem::System(system) => match system.synthetic_reason {
            SyntheticReason::Primary => ContextCategory::CoreInstructions,
            SyntheticReason::ProjectInstructions => ContextCategory::ProjectInstructions,
            SyntheticReason::CompactionMeta => ContextCategory::CompactionSummary,
            SyntheticReason::Human
            | SyntheticReason::Unknown
            | SyntheticReason::SessionPrefix
            | SyntheticReason::DirectBash
            | SyntheticReason::GoalSetup => ContextCategory::OtherRuntime,
            SyntheticReason::SystemReminder => notice(RuntimeNotice::SystemReminder),
            SyntheticReason::LengthContinue => notice(RuntimeNotice::LengthContinue),
            SyntheticReason::AutoContinue => notice(RuntimeNotice::AutoContinue),
            SyntheticReason::AutoRecovery => notice(RuntimeNotice::AutoRecovery),
            SyntheticReason::Interjection => notice(RuntimeNotice::Interjection),
            SyntheticReason::AgentMessage | SyntheticReason::ParentHumanMessage => {
                notice(RuntimeNotice::AgentMessage)
            }
            SyntheticReason::TaskCompleted => notice(RuntimeNotice::TaskCompleted),
            SyntheticReason::SubagentCompleted => notice(RuntimeNotice::SubagentCompleted),
            SyntheticReason::NotificationDrain => notice(RuntimeNotice::NotificationDrain),
            SyntheticReason::GoalSummary => notice(RuntimeNotice::GoalSummary),
            SyntheticReason::GoalClassifierNudge => notice(RuntimeNotice::GoalClassifierNudge),
            SyntheticReason::SchedulerFired => notice(RuntimeNotice::SchedulerFired),
            SyntheticReason::StopHookFeedback => notice(RuntimeNotice::StopHookFeedback),
            SyntheticReason::WorkingDirectorySwitch => notice(RuntimeNotice::WorkingDirectorySwitch),
        },
        ConversationItem::User(user) => match user.synthetic_reason {
            SyntheticReason::Human | SyntheticReason::DirectBash | SyntheticReason::GoalSetup => {
                ContextCategory::UserTurns
            }
            SyntheticReason::ProjectInstructions => ContextCategory::ProjectInstructions,
            SyntheticReason::SessionPrefix => ContextCategory::Environment,
            SyntheticReason::AgentMessage | SyntheticReason::ParentHumanMessage => {
                notice(RuntimeNotice::AgentMessage)
            }
            SyntheticReason::CompactionMeta => ContextCategory::CompactionSummary,
            SyntheticReason::SystemReminder => notice(RuntimeNotice::SystemReminder),
            SyntheticReason::LengthContinue => notice(RuntimeNotice::LengthContinue),
            SyntheticReason::AutoContinue => notice(RuntimeNotice::AutoContinue),
            SyntheticReason::AutoRecovery => notice(RuntimeNotice::AutoRecovery),
            SyntheticReason::Interjection => notice(RuntimeNotice::Interjection),
            SyntheticReason::TaskCompleted => notice(RuntimeNotice::TaskCompleted),
            SyntheticReason::SubagentCompleted => notice(RuntimeNotice::SubagentCompleted),
            SyntheticReason::NotificationDrain => notice(RuntimeNotice::NotificationDrain),
            SyntheticReason::GoalSummary => notice(RuntimeNotice::GoalSummary),
            SyntheticReason::GoalClassifierNudge => notice(RuntimeNotice::GoalClassifierNudge),
            SyntheticReason::SchedulerFired => notice(RuntimeNotice::SchedulerFired),
            SyntheticReason::StopHookFeedback => notice(RuntimeNotice::StopHookFeedback),
            SyntheticReason::WorkingDirectorySwitch => notice(RuntimeNotice::WorkingDirectorySwitch),
            SyntheticReason::Primary => ContextCategory::CoreInstructions,
            SyntheticReason::Unknown => ContextCategory::OtherRuntime,
        },
        ConversationItem::Assistant(_) => ContextCategory::AssistantTurns,
        ConversationItem::ToolResult(_) | ConversationItem::BackendToolCall(_) => {
            ContextCategory::ToolExchanges
        }
        ConversationItem::Reasoning(_) => ContextCategory::Reasoning,
    }
}

fn notice(reason: RuntimeNotice) -> ContextCategory {
    ContextCategory::RuntimeNotices(reason)
}
