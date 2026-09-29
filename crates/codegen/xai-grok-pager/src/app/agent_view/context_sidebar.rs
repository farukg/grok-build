//! The F7 sidebar: which context categories the session's next requests send.
//! The shell owns the policy; this state shows it, asks for a change and waits for the shell's answer.

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::theme::Theme;
use crate::views::prompt_widget::PromptWidget;
use crate::views::sidebar::{
    RowIdx, Sidebar, SidebarContent, SidebarEdge, SidebarHeights, SidebarHit, SidebarLayout,
    SidebarLine, SidebarRender, SidebarRow, SidebarSection, SidebarState, SidebarsOpen,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use xai_grok_shell::sampling::{
    ContextCategory, ContextPolicy, ContextSwitch, RuntimeNotice, switchable_categories,
};

const SECTION: SidebarLine = SidebarLine::Row(crate::views::sidebar::SectionIdx(0), RowIdx(0));

#[derive(Debug, Default)]
pub(crate) enum ContextSidebar {
    #[default]
    Closed,
    Open(OpenContextSidebar),
}

#[derive(Debug)]
pub(crate) struct OpenContextSidebar {
    focus: SidebarFocus,
    sidebar: SidebarState,
    layout: Option<SidebarLayout>,
    policy: PolicyState,
    heights: Option<(u16, SidebarHeights)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarFocus {
    Chat,
    Sidebar,
}

/// What the sidebar knows about the session's policy. A row shows the shell's answer, never a hope.
#[derive(Debug)]
pub(crate) enum PolicyState {
    Loading,
    Ready(ContextPolicy),
    /// A change is with the shell; `shown` stays on screen until it answers.
    Applying { shown: ContextPolicy },
    Unavailable,
}

impl PolicyState {
    fn shown(&self) -> Option<&ContextPolicy> {
        match self {
            Self::Ready(policy) | Self::Applying { shown: policy } => Some(policy),
            Self::Loading | Self::Unavailable => None,
        }
    }
}

fn label(category: ContextCategory) -> &'static str {
    match category {
        ContextCategory::CoreInstructions => "Core instructions",
        ContextCategory::ProjectInstructions => "Project rules",
        ContextCategory::Environment => "Environment",
        ContextCategory::SkillsWorkflows => "Skills",
        ContextCategory::McpCatalog => "MCP catalog",
        ContextCategory::ToolDefinitions => "Tool definitions",
        ContextCategory::UserTurns => "Your messages",
        ContextCategory::AssistantTurns => "Agent replies",
        ContextCategory::Reasoning => "Reasoning",
        ContextCategory::ToolExchanges => "Tool calls and results",
        ContextCategory::RuntimeNotices(notice) => match notice {
            RuntimeNotice::SystemReminder => "Notice: reminders",
            RuntimeNotice::LengthContinue => "Notice: length continue",
            RuntimeNotice::AutoContinue => "Notice: auto continue",
            RuntimeNotice::AutoRecovery => "Notice: auto recovery",
            RuntimeNotice::Interjection => "Notice: interjections",
            RuntimeNotice::AgentMessage => "Notice: agent messages",
            RuntimeNotice::TaskCompleted => "Notice: task completed",
            RuntimeNotice::SubagentCompleted => "Notice: subagent completed",
            RuntimeNotice::NotificationDrain => "Notice: notifications",
            RuntimeNotice::GoalSummary => "Notice: goal summary",
            RuntimeNotice::GoalClassifierNudge => "Notice: goal nudge",
            RuntimeNotice::SchedulerFired => "Notice: scheduled task",
            RuntimeNotice::StopHookFeedback => "Notice: stop hook",
            RuntimeNotice::WorkingDirectorySwitch => "Notice: directory switch",
        },
        ContextCategory::Memory => "Memory",
        ContextCategory::CompactionSummary => "Compaction summary",
        ContextCategory::OtherRuntime => "Other runtime context",
    }
}

fn consequence(category: ContextCategory) -> &'static str {
    match category {
        ContextCategory::CoreInstructions => "The agent loses its base behavior and tool guidance.",
        ContextCategory::ProjectInstructions => "AGENTS.md rules stop applying.",
        ContextCategory::Environment => "Working directory, OS and git status are unknown.",
        ContextCategory::SkillsWorkflows => "Skills are no longer listed.",
        ContextCategory::McpCatalog => "MCP servers are no longer listed.",
        ContextCategory::ToolDefinitions => "Tools are unavailable; earlier tool exchanges go too.",
        ContextCategory::UserTurns => "Earlier messages are forgotten; the current one is sent.",
        ContextCategory::AssistantTurns => "Earlier replies are forgotten.",
        ContextCategory::Reasoning => "Reasoning continuity is lost; the prompt cache resets.",
        ContextCategory::ToolExchanges => "Earlier tool calls and their results are forgotten.",
        ContextCategory::RuntimeNotices(_) => "The agent no longer sees these runtime notices.",
        ContextCategory::Memory => "Memory is not injected.",
        ContextCategory::CompactionSummary => "The summary of compacted history is dropped.",
        ContextCategory::OtherRuntime => "Other injected context is dropped.",
    }
}

fn rows<'a>(
    policy: Option<&ContextPolicy>,
    categories: &[ContextCategory],
    consequences: &'a [[Line<'static>; 1]],
) -> Vec<SidebarRow<'a>> {
    categories
        .iter()
        .zip(consequences)
        .map(|(&category, consequence)| {
            let included = policy.is_none_or(|policy| policy.includes(category));
            SidebarRow {
                left: Line::from(label(category)),
                right: policy.map(|_| Line::from(if included { "ON" } else { "OFF" })),
                detail: if included { &[] } else { consequence },
            }
        })
        .collect()
}

impl AgentView {
    pub(crate) fn sidebars_open(&self) -> SidebarsOpen {
        match self.context_sidebar {
            ContextSidebar::Closed => SidebarsOpen::None,
            ContextSidebar::Open(_) => SidebarsOpen::Right,
        }
    }

    /// F7: open the sidebar and ask the shell for the policy, or close it.
    pub(crate) fn toggle_context_sidebar(&mut self) -> InputOutcome {
        if self.is_minimal_mode() {
            return InputOutcome::Unchanged;
        }
        match self.context_sidebar {
            ContextSidebar::Closed => {
                self.context_sidebar = ContextSidebar::Open(OpenContextSidebar {
                    focus: SidebarFocus::Sidebar,
                    sidebar: SidebarState::default(),
                    layout: None,
                    policy: PolicyState::Loading,
                    heights: None,
                });
                InputOutcome::Action(Action::FetchContextPolicy)
            }
            ContextSidebar::Open(_) => {
                self.context_sidebar = ContextSidebar::Closed;
                InputOutcome::Changed
            }
        }
    }

    /// The shell answered a get or set.
    pub(crate) fn context_policy_answered(&mut self, answer: Result<ContextPolicy, String>) {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return;
        };
        open.policy = match answer {
            Ok(policy) => PolicyState::Ready(policy),
            Err(error) => {
                tracing::warn!(%error, "context policy request failed");
                match std::mem::replace(&mut open.policy, PolicyState::Unavailable) {
                    PolicyState::Applying { shown } => PolicyState::Ready(shown),
                    PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Unavailable => {
                        PolicyState::Unavailable
                    }
                }
            }
        };
    }

    /// Keys while the sidebar has focus and clicks on it; `None` lets the event continue.
    pub(super) fn route_context_sidebar_input(&mut self, ev: &Event) -> Option<InputOutcome> {
        if !self.no_input_overlay_pending() {
            return None;
        }
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return None;
        };
        match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release && open.focus == SidebarFocus::Sidebar => {
                let viewport = open.layout.map_or(1, |layout| layout.body.height as usize);
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => open.sidebar.move_cursor(-1, viewport),
                    KeyCode::Down | KeyCode::Char('j') => open.sidebar.move_cursor(1, viewport),
                    KeyCode::Char(' ') | KeyCode::Enter => return Some(self.toggle_cursor_category()),
                    KeyCode::Esc => {
                        self.context_sidebar = ContextSidebar::Closed;
                    }
                    KeyCode::Tab => open.focus = SidebarFocus::Chat,
                    _ => return None,
                }
                Some(InputOutcome::Changed)
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let hit = open
                    .layout
                    .and_then(|layout| layout.hit(mouse.column, mouse.row, &open.sidebar));
                match hit {
                    Some(SidebarHit::Line(line @ SidebarLine::Row(..))) => {
                        open.focus = SidebarFocus::Sidebar;
                        open.sidebar.cursor = Some(line);
                        Some(self.toggle_cursor_category())
                    }
                    Some(SidebarHit::Header | SidebarHit::Footer | SidebarHit::Line(_)) => {
                        open.focus = SidebarFocus::Sidebar;
                        Some(InputOutcome::Changed)
                    }
                    None => {
                        open.focus = SidebarFocus::Chat;
                        None
                    }
                }
            }
            _ => None,
        }
    }

    /// Wheel over the sidebar scrolls it; returns whether it consumed the wheel.
    pub(super) fn scroll_context_sidebar(&mut self, lines: i32, col: u16, row: u16) -> bool {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return false;
        };
        let Some(layout) = open.layout else {
            return false;
        };
        if !layout.outer.contains((col, row).into()) {
            return false;
        }
        open.sidebar
            .scroll_rows(lines.signum() as isize, layout.body.height as usize);
        true
    }

    fn toggle_cursor_category(&mut self) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let (Some(SidebarLine::Row(_, RowIdx(index))), Some(shown)) =
            (open.sidebar.cursor, open.policy.shown().cloned())
        else {
            return InputOutcome::Unchanged;
        };
        let Some(&category) = switchable_categories().get(index) else {
            return InputOutcome::Unchanged;
        };
        let mut requested = shown.clone();
        requested.set(
            category,
            if shown.includes(category) {
                ContextSwitch::Excluded
            } else {
                ContextSwitch::Included
            },
        );
        open.policy = PolicyState::Applying { shown };
        InputOutcome::Action(Action::SetContextPolicy(requested))
    }

    pub(super) fn draw_context_sidebar(&mut self, area: Rect, buf: &mut Buffer) {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            open.layout = None;
            return;
        }
        let theme = Theme::current();
        let heights = match open.heights {
            Some((width, heights)) if width == area.width => heights,
            _ => {
                let heights = SidebarHeights::from_shared_sources(&PromptWidget::new(), area.width);
                open.heights = Some((area.width, heights));
                heights
            }
        };
        let categories = switchable_categories();
        let consequences: Vec<[Line<'static>; 1]> = categories
            .iter()
            .map(|&category| [Line::from(consequence(category))])
            .collect();
        let rows = rows(open.policy.shown(), &categories, &consequences);
        let title = Line::from(Span::styled(
            "Context",
            Style::default().fg(theme.text_primary),
        ));
        let sections = [SidebarSection::Rows {
            title: title.clone(),
            rows: &rows,
        }];
        open.sidebar.rebuild_layout(&sections);
        if open.sidebar.cursor.is_none() && open.focus == SidebarFocus::Sidebar {
            open.sidebar.cursor = Some(SECTION);
        }
        let status = match open.policy {
            PolicyState::Loading => "Loading…",
            PolicyState::Applying { .. } => "Applying…",
            PolicyState::Unavailable => "Unavailable for this session",
            PolicyState::Ready(_) => "Space toggles · Esc closes",
        };
        let header = [Line::from("Context sent to the model")];
        let footer = [
            Line::from(status),
            Line::from("Your current message is always sent."),
        ];
        open.layout = Some(Sidebar::render(
            SidebarRender {
                area,
                content: SidebarContent {
                    header: &header,
                    sections: &sections,
                    footer: &footer,
                },
                heights,
                edge: SidebarEdge::Right,
                state: &open.sidebar,
                hovered: None,
                theme: &theme,
            },
            buf,
        ));
    }
}
