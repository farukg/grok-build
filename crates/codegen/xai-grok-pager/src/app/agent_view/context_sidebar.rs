//! The F7 sidebar: which context categories the session's next requests send.
//! The shell owns the policy; this state shows it, asks for a change and waits for the shell's answer.

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::theme::Theme;
use crate::views::context_bar::fmt_tokens;
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use crate::views::sidebar::{
    RowIdx, SectionIdx, Sidebar, SidebarContent, SidebarEdge, SidebarHit, SidebarLayout, SidebarLine,
    SidebarRender, SidebarRow, SidebarSection, SidebarState,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;
use xai_grok_shell::sampling::{ContextCategory, ContextSwitch, RuntimeNotice, switchable_categories};

/// Section 0 is the blank slot the dock paints over, then the context switches, then message display.
const CONTEXT_SECTION: SectionIdx = SectionIdx(1);
const DISPLAY_SECTION: SectionIdx = SectionIdx(2);
const FIRST_SWITCH: SidebarLine = SidebarLine::Row(CONTEXT_SECTION, RowIdx(0));

/// What a row of the sidebar changes.
enum RowTarget {
    Context(ContextCategory),
    Display(MessageKind),
}

fn row_target(section: SectionIdx, RowIdx(index): RowIdx) -> Option<RowTarget> {
    if section == CONTEXT_SECTION {
        switchable_categories().get(index).copied().map(RowTarget::Context)
    } else if section == DISPLAY_SECTION {
        MessageKind::settings_kinds().get(index).copied().map(RowTarget::Display)
    } else {
        None
    }
}

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
    Ready(ContextPolicyReport),
    /// A change is with the shell; `shown` stays on screen until it answers.
    Applying { shown: ContextPolicyReport },
    Unavailable,
}

impl PolicyState {
    fn shown(&self) -> Option<&ContextPolicyReport> {
        match self {
            Self::Ready(report) | Self::Applying { shown: report } => Some(report),
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

fn kind_label(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::UserPrompt => "Your messages",
        MessageKind::AgentMessage => "Agent replies",
        MessageKind::Execute => "Commands",
        MessageKind::Read => "File reads",
        MessageKind::Edit => "File edits",
        MessageKind::ListDir => "Directory listings",
        MessageKind::Search => "Code searches",
        MessageKind::WebFetch => "Web fetches",
        MessageKind::WebSearch => "Web searches",
        MessageKind::IntegrationSearch => "Integration searches",
        MessageKind::UseTool => "Integration tools",
        MessageKind::MemorySearch => "Memory searches",
        MessageKind::SentMessage => "Sent messages",
        MessageKind::Skill => "Skills",
        MessageKind::OtherTool => "Other tools",
        MessageKind::Thinking => "Reasoning",
        MessageKind::System => "System messages",
        MessageKind::SessionEvent => "Session events",
        MessageKind::BgTask => "Background tasks",
        MessageKind::Subagent => "Subagents",
        MessageKind::Workflow => "Workflows",
        MessageKind::Btw => "Side questions",
        MessageKind::ContextInfo => "Context info",
        MessageKind::MemoryCapture => "Memory captures",
        MessageKind::Stub => "Stub",
    }
}

fn form_label(form: DisplayForm) -> &'static str {
    match form {
        DisplayForm::Collapsed => "1 line",
        DisplayForm::Expanded => "full",
    }
}

fn display_rows(
    kinds: &[MessageKind],
    defaults: &crate::scrollback::block::DisplayDefaults,
) -> Vec<SidebarRow<'static>> {
    kinds
        .iter()
        .map(|&kind| SidebarRow {
            left: Line::from(kind_label(kind)),
            right: Some(Line::from(form_label(defaults.shown(kind)))),
            detail: &[],
        })
        .collect()
}

fn rows<'a>(
    report: Option<&ContextPolicyReport>,
    categories: &[ContextCategory],
    consequences: &'a [[Line<'static>; 1]],
) -> Vec<SidebarRow<'a>> {
    categories
        .iter()
        .zip(consequences)
        .map(|(&category, consequence)| {
            let included = report.is_none_or(|report| report.policy.includes(category));
            SidebarRow {
                left: Line::from(label(category)),
                right: report.map(|report| {
                    let tokens = report
                        .usage
                        .iter()
                        .find(|usage| usage.category == category)
                        .map_or(0, |usage| usage.tokens);
                    let state = if included { "ON " } else { "OFF" };
                    Line::from(format!("~{:<4} {state}", fmt_tokens(tokens)))
                }),
                detail: if included { &[] } else { consequence },
            }
        })
        .collect()
}

impl AgentView {
    /// Body rect of the F7 column, or empty while it is closed.
    pub(super) fn context_sidebar_body(&mut self, column: Rect) -> Rect {
        if !matches!(self.context_sidebar, ContextSidebar::Open(_)) || column.width == 0 {
            return Rect::default();
        }
        let heights = self.sidebar_heights.heights_for(column.width);
        SidebarLayout::compute(column, heights).body
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
    pub(crate) fn context_policy_answered(&mut self, answer: Result<ContextPolicyReport, String>) {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return;
        };
        open.policy = match answer {
            Ok(report) => PolicyState::Ready(report),
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
                    KeyCode::Char(' ') | KeyCode::Enter => return Some(self.toggle_cursor_row()),
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
                        Some(self.toggle_cursor_row())
                    }
                    Some(SidebarHit::Line(SidebarLine::Hosted(_))) => None,
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

    fn toggle_cursor_row(&mut self) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let Some(SidebarLine::Row(section, row)) = open.sidebar.cursor else {
            return InputOutcome::Unchanged;
        };
        match row_target(section, row) {
            Some(RowTarget::Context(category)) => self.toggle_context_category(category),
            Some(RowTarget::Display(kind)) => {
                let form = match self.scrollback.display_defaults().shown(kind) {
                    DisplayForm::Collapsed => DisplayForm::Expanded,
                    DisplayForm::Expanded => DisplayForm::Collapsed,
                };
                self.scrollback.set_kind_default(kind, form);
                InputOutcome::Changed
            }
            None => InputOutcome::Unchanged,
        }
    }

    fn toggle_context_category(&mut self, category: ContextCategory) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let Some(shown) = open.policy.shown().cloned() else {
            return InputOutcome::Unchanged;
        };
        let mut requested = shown.policy.clone();
        requested.set(
            category,
            if shown.policy.includes(category) {
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
        let heights = self.sidebar_heights.heights_for(area.width);
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
        let display = display_rows(MessageKind::settings_kinds(), self.scrollback.display_defaults());
        let dock_slot = super::sidebars::DockSlot(self.dock_rows_in_sidebar);
        let sections = [
            SidebarSection::Hosted(&dock_slot),
            SidebarSection::Rows {
                title: title.clone(),
                rows: &rows,
            },
            SidebarSection::Rows {
                title: Line::from(Span::styled(
                    "Message display",
                    Style::default().fg(theme.text_primary),
                )),
                rows: &display,
            },
        ];
        open.sidebar.rebuild_layout(&sections);
        if open.sidebar.cursor.is_none() && open.focus == SidebarFocus::Sidebar {
            open.sidebar.cursor = Some(FIRST_SWITCH);
        }
        let status = match open.policy {
            PolicyState::Loading => "Loading…",
            PolicyState::Applying { .. } => "Applying…",
            PolicyState::Unavailable => "Unavailable for this session",
            PolicyState::Ready(_) => "Space changes · Esc closes",
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
