use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNotice {
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
        match self.switches.iter().find(|(key, _)| *key == category) {
            Some((_, switch)) => *switch,
            None => ContextSwitch::Included,
        }
    }

    pub fn set(&mut self, category: ContextCategory, switch: ContextSwitch) {
        match self.switches.iter_mut().find(|(key, _)| *key == category) {
            Some((_, value)) => *value = switch,
            None => self.switches.push((category, switch)),
        }
    }

    pub fn includes(&self, category: ContextCategory) -> bool {
        self.switch(category) == ContextSwitch::Included
    }

    pub fn is_unrestricted(&self) -> bool {
        self.switches
            .iter()
            .all(|(_, switch)| *switch == ContextSwitch::Included)
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
