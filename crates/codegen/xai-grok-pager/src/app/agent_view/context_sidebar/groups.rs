//! Semantic F7 parents and their aggregate values.

use super::outline::SectionId;
use super::RowTarget;
use crate::app::panel_settings::SidebarSection;
use crate::scrollback::block::{DisplayDefaults, MessageGroup};
use crate::scrollback::types::DisplayForm;
use strum::IntoEnumIterator;
use xai_grok_shell::sampling::{ContextCategory, ContextSwitch, RuntimeNotice, all_categories};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter, strum::EnumCount)]
pub(super) enum ContextGroup {
    Instructions,
    Conversation,
    Continuation,
    Notifications,
    Retained,
}

impl ContextGroup {
    pub(super) fn of(category: ContextCategory) -> Self {
        match category {
            ContextCategory::CoreInstructions | ContextCategory::ProjectInstructions
            | ContextCategory::Environment | ContextCategory::SkillsWorkflows
            | ContextCategory::McpCatalog | ContextCategory::ToolDefinitions => Self::Instructions,
            ContextCategory::UserTurns | ContextCategory::AssistantTurns
            | ContextCategory::Reasoning | ContextCategory::ToolExchanges => Self::Conversation,
            ContextCategory::RuntimeNotices(notice) => match notice {
                RuntimeNotice::SystemReminder | RuntimeNotice::LengthContinue
                | RuntimeNotice::AutoContinue | RuntimeNotice::AutoRecovery
                | RuntimeNotice::GoalSummary | RuntimeNotice::GoalClassifierNudge => Self::Continuation,
                RuntimeNotice::Interjection | RuntimeNotice::AgentMessage
                | RuntimeNotice::TaskCompleted | RuntimeNotice::SubagentCompleted
                | RuntimeNotice::NotificationDrain | RuntimeNotice::SchedulerFired
                | RuntimeNotice::StopHookFeedback | RuntimeNotice::WorkingDirectorySwitch => Self::Notifications,
            },
            ContextCategory::Memory | ContextCategory::CompactionSummary | ContextCategory::OtherRuntime => Self::Retained,
        }
    }

    pub(super) fn categories(self) -> Vec<ContextCategory> {
        all_categories().into_iter().filter(|&category| Self::of(category) == self).collect()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Instructions => "Instructions / capabilities",
            Self::Conversation => "Conversation",
            Self::Continuation => "Continuation / recovery",
            Self::Notifications => "Notifications / activity",
            Self::Retained => "Memory / retained context",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GroupId {
    Context(ContextGroup),
    Display(MessageGroup),
}

impl GroupId {
    pub(super) const COUNT: usize = <ContextGroup as strum::EnumCount>::COUNT + <MessageGroup as strum::EnumCount>::COUNT;

    pub(super) fn all() -> impl Iterator<Item = Self> {
        ContextGroup::iter().map(Self::Context).chain(MessageGroup::iter().map(Self::Display))
    }

    pub(super) fn index(self) -> usize {
        match self {
            Self::Context(group) => group as usize,
            Self::Display(group) => <ContextGroup as strum::EnumCount>::COUNT + group as usize,
        }
    }

    pub(super) fn section(self) -> SectionId {
        match self {
            Self::Context(_) => SectionId::Context,
            Self::Display(_) => SectionId::Display,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Context(group) => group.label(),
            Self::Display(group) => group.label(),
        }
    }

    pub(super) fn children(self) -> Vec<RowTarget> {
        match self {
            Self::Context(group) => group.categories().into_iter().map(RowTarget::Context).collect(),
            Self::Display(group) => group.kinds().map(RowTarget::Display).collect(),
        }
    }

    pub(super) fn stored(self) -> SidebarSection {
        match self {
            Self::Context(ContextGroup::Instructions) => SidebarSection::ContextInstructions,
            Self::Context(ContextGroup::Conversation) => SidebarSection::ContextConversation,
            Self::Context(ContextGroup::Continuation) => SidebarSection::ContextContinuation,
            Self::Context(ContextGroup::Notifications) => SidebarSection::ContextNotifications,
            Self::Context(ContextGroup::Retained) => SidebarSection::ContextRetained,
            Self::Display(MessageGroup::Conversation) => SidebarSection::DisplayConversation,
            Self::Display(MessageGroup::Files) => SidebarSection::DisplayFiles,
            Self::Display(MessageGroup::Research) => SidebarSection::DisplayResearch,
            Self::Display(MessageGroup::Work) => SidebarSection::DisplayWork,
            Self::Display(MessageGroup::Session) => SidebarSection::DisplaySession,
            Self::Display(MessageGroup::Injected) => SidebarSection::DisplayInjected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Aggregate<T> {
    Uniform(T),
    Mixed,
    Unavailable,
}

impl<T: Copy + Eq> Aggregate<T> {
    pub(super) fn of(values: impl IntoIterator<Item = T>) -> Self {
        let mut values = values.into_iter();
        match values.next() {
            Some(first) if values.all(|value| value == first) => Self::Uniform(first),
            Some(_) => Self::Mixed,
            None => Self::Unavailable,
        }
    }
}

pub(super) fn display_value(group: MessageGroup, defaults: &DisplayDefaults) -> Aggregate<DisplayForm> {
    Aggregate::of(group.kinds().map(|kind| defaults.shown(kind)))
}

pub(super) fn next_display(value: Aggregate<DisplayForm>) -> DisplayForm {
    match value {
        Aggregate::Uniform(DisplayForm::Collapsed) => DisplayForm::Expanded,
        Aggregate::Uniform(DisplayForm::Expanded) | Aggregate::Mixed | Aggregate::Unavailable => DisplayForm::Collapsed,
    }
}

pub(super) fn next_context(value: Aggregate<ContextSwitch>) -> ContextSwitch {
    match value {
        Aggregate::Uniform(ContextSwitch::Excluded) => ContextSwitch::Included,
        Aggregate::Uniform(ContextSwitch::Included) | Aggregate::Mixed | Aggregate::Unavailable => ContextSwitch::Excluded,
    }
}
