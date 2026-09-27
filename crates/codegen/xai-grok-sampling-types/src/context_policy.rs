use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNotice {
    CompactionMeta,
    SystemReminder,
    LengthContinue,
    AutoContinue,
    AutoRecovery,
    Interjection,
    AgentMessage,
    TaskCompleted,
    SubagentCompleted,
    NotificationDrain,
    GoalSummary,
    GoalClassifierNudge,
    SchedulerFired,
    StopHookFeedback,
    WorkingDirectorySwitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "category", content = "notice", rename_all = "snake_case")]
pub enum ContextCategory {
    CoreInstructions,
    ProjectInstructions,
    Environment,
    SkillsWorkflows,
    McpCatalog,
    ToolDefinitions,
    UserTurns,
    AssistantTurns,
    Reasoning,
    ToolExchanges,
    RuntimeNotices(RuntimeNotice),
    Memory,
    CompactionSummary,
    OtherRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSwitch {
    Included,
    Excluded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicy {
    switches: Vec<(ContextCategory, ContextSwitch)>,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        Self {
            switches: all_categories()
                .into_iter()
                .map(|category| (category, ContextSwitch::Included))
                .collect(),
        }
    }
}

impl ContextPolicy {
    pub fn switch(&self, category: ContextCategory) -> ContextSwitch {
        self.switches
            .iter()
            .find_map(|(key, value)| match *key == category {
                true => Some(*value),
                false => None,
            })
            .unwrap_or(ContextSwitch::Included)
    }

    pub fn set(&mut self, category: ContextCategory, switch: ContextSwitch) {
        if let Some((_, value)) = self.switches.iter_mut().find(|(key, _)| *key == category) {
            *value = switch;
        } else {
            self.switches.push((category, switch));
        }
    }

    pub fn includes(&self, category: ContextCategory) -> bool {
        self.switch(category) == ContextSwitch::Included
    }
}

pub fn all_categories() -> Vec<ContextCategory> {
    use ContextCategory::*;
    vec![
        CoreInstructions,
        ProjectInstructions,
        Environment,
        SkillsWorkflows,
        McpCatalog,
        ToolDefinitions,
        UserTurns,
        AssistantTurns,
        Reasoning,
        ToolExchanges,
        RuntimeNotices(RuntimeNotice::CompactionMeta),
        RuntimeNotices(RuntimeNotice::SystemReminder),
        RuntimeNotices(RuntimeNotice::LengthContinue),
        RuntimeNotices(RuntimeNotice::AutoContinue),
        RuntimeNotices(RuntimeNotice::AutoRecovery),
        RuntimeNotices(RuntimeNotice::Interjection),
        RuntimeNotices(RuntimeNotice::AgentMessage),
        RuntimeNotices(RuntimeNotice::TaskCompleted),
        RuntimeNotices(RuntimeNotice::SubagentCompleted),
        RuntimeNotices(RuntimeNotice::NotificationDrain),
        RuntimeNotices(RuntimeNotice::GoalSummary),
        RuntimeNotices(RuntimeNotice::GoalClassifierNudge),
        RuntimeNotices(RuntimeNotice::SchedulerFired),
        RuntimeNotices(RuntimeNotice::StopHookFeedback),
        RuntimeNotices(RuntimeNotice::WorkingDirectorySwitch),
        Memory,
        CompactionSummary,
        OtherRuntime,
    ]
}
