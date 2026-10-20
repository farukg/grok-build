use serde::{Deserialize, Serialize};

use crate::{ContextCategory, ConversationItem, RuntimeNotice, SyntheticReason, UserItem};

/// The context categories inside one user item whose parts come from different sources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sections {
    /// Every part belongs to the item's own category.
    #[default]
    Whole,
    /// Part `i` belongs to category `i`; parts past the list belong to the item's own category.
    Split(Vec<ContextCategory>),
}

impl Sections {
    pub fn is_whole(&self) -> bool {
        matches!(self, Self::Whole)
    }
}

impl UserItem {
    /// The context category of each content part, in order.
    pub fn part_categories(&self) -> impl Iterator<Item = ContextCategory> + '_ {
        let split = match &self.sections {
            Sections::Whole => &[][..],
            Sections::Split(categories) => categories.as_slice(),
        };
        split
            .iter()
            .copied()
            .chain(std::iter::repeat(self.synthetic_reason.context_category()))
            .take(self.content.len())
    }
}

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
    pub(crate) fn context_category(&self) -> ContextCategory {
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
            Self::SkillCatalog => ContextCategory::SkillsWorkflows,
            Self::McpCatalog => ContextCategory::McpCatalog,
            Self::Unknown => ContextCategory::OtherRuntime,
        }
    }
}
