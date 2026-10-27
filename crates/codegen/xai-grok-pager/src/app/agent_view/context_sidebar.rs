//! The F7 sidebar: which context categories the session's next requests send.
//! The shell owns the policy; this state shows it, asks for a change and waits for the shell's answer.

use super::AgentView;
use crate::app::actions::Action;
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use crate::theme::Theme;
use crate::views::context_bar::fmt_tokens;
use crate::views::row_style::{SidebarRowClass, sidebar_style};
use crate::views::sidebar::{
    RowIdx, SectionIdx, Sidebar, SidebarContent, SidebarEdge, SidebarHit, SidebarLayout,
    SidebarLine, SidebarRender, SidebarRow, SidebarSection, SidebarState,
};
use crossterm::event::{Event, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;
use xai_grok_shell::sampling::{ContextCategory, ContextSwitch, RuntimeNotice, all_categories};

mod commands;
mod footer;
mod gesture;
mod groups;
mod group_commands;
mod group_rows;
mod lines;
mod outline;
mod persistence;
mod policy;

use outline::{CursorTarget, Outline};
pub(crate) use outline::{SectionDisclosure, SectionId};

const DOCK_SECTION: SectionIdx = SectionIdx(0);
const TODO_SECTION: SectionIdx = SectionIdx(2);
const CONTEXT_SECTION: SectionIdx = SectionIdx(3);
const DISPLAY_SECTION: SectionIdx = SectionIdx(4);

/// What a row of the sidebar changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowTarget {
    DockPlacement,
    Context(ContextCategory),
    Display(MessageKind),
    Group(groups::GroupId),
}

fn row_target(section: SectionIdx, RowIdx(index): RowIdx, disclosure: SectionDisclosure) -> Option<RowTarget> {
    if section == DOCK_SECTION {
        (index == 0).then_some(RowTarget::DockPlacement)
    } else {
        let section = SectionId::of_index(section)?;
        outline::section_targets(disclosure, section).get(index).copied()
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
    disclosure: SectionDisclosure,
    todo_count: usize,
    /// The next draw scrolls the cursor line into view.
    reveal_pending: bool,
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
    Confirming { shown: ContextPolicyReport, confirmation: policy::Confirmation },
    ExpiredConfirmation(policy::ExpiredConfirmation),
    /// A change is with the shell; `shown` stays on screen until it answers.
    Applying {
        shown: ContextPolicyReport,
    },
    Unavailable,
}

impl PolicyState {
    fn shown(&self) -> Option<&ContextPolicyReport> {
        match self {
            Self::Ready(report) | Self::Applying { shown: report } | Self::Confirming { shown: report, .. } => Some(report),
            Self::ExpiredConfirmation(expired) => expired.shown(),
            Self::Loading | Self::Unavailable => None,
        }
    }
}

pub(crate) fn label(category: ContextCategory) -> &'static str {
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
            RuntimeNotice::SystemReminder => "reminders",
            RuntimeNotice::LengthContinue => "length continue",
            RuntimeNotice::AutoContinue => "auto continue",
            RuntimeNotice::AutoRecovery => "auto recovery",
            RuntimeNotice::Interjection => "interjections",
            RuntimeNotice::AgentMessage => "agent messages",
            RuntimeNotice::TaskCompleted => "task completed",
            RuntimeNotice::SubagentCompleted => "subagent completed",
            RuntimeNotice::NotificationDrain => "notifications",
            RuntimeNotice::GoalSummary => "goal summary",
            RuntimeNotice::GoalClassifierNudge => "goal nudge",
            RuntimeNotice::SchedulerFired => "scheduled task",
            RuntimeNotice::StopHookFeedback => "stop hook",
            RuntimeNotice::WorkingDirectorySwitch => "directory switch",
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


fn form_label(form: DisplayForm) -> &'static str {
    match form {
        DisplayForm::Collapsed => "1 line",
        DisplayForm::Expanded => "full",
    }
}

fn display_rows(
    kinds: &[MessageKind],
    defaults: &crate::scrollback::block::DisplayDefaults,
    theme: &Theme,
) -> Vec<SidebarRow<'static>> {
    kinds
        .iter()
        .map(|&kind| {
            let form = defaults.shown(kind);
            let class = SidebarRowClass::from(form);
            let style = sidebar_style(class, theme);
            SidebarRow {
                left: lines::leaf_label(class, style, kind.label()),
                right: Some(Line::from(Span::styled(form_label(form), style.value))),
                detail: &[],
            }
        })
        .collect()
}

fn rows<'a>(
    report: Option<&ContextPolicyReport>,
    categories: &[ContextCategory],
    consequences: &'a [[Line<'static>; 1]],
    theme: &Theme,
) -> Vec<SidebarRow<'a>> {
    categories
        .iter()
        .zip(consequences)
        .map(|(&category, consequence)| {
            let included = report.is_none_or(|report| report.policy.includes(category));
            let class = match (report, included) {
                (None, _) => SidebarRowClass::Unavailable,
                (Some(_), true) => SidebarRowClass::SwitchOn,
                (Some(_), false) => SidebarRowClass::SwitchOff,
            };
            let style = sidebar_style(class, theme);
            SidebarRow {
                left: lines::leaf_label(class, style, label(category)),
                right: report.map(|report| {
                    let tokens = report.usage.iter().find(|usage| usage.category == category)
                        .map_or_else(|| "?".to_owned(), |usage| fmt_tokens(usage.tokens));
                    let state = if included { "ON " } else { "OFF" };
                    Line::from(vec![
                        Span::styled(format!("~{tokens:<4} "), theme.muted()),
                        Span::styled(state, style.value),
                    ])
                }),
                detail: if included { &[] } else { consequence },
            }
        })
        .collect()
}

impl AgentView {
    pub(crate) fn context_dock_geometry(&self) -> Option<(Rect, u16)> {
        let ContextSidebar::Open(open) = &self.context_sidebar else { return None; };
        open.layout?.hosted_rect(SectionIdx(1), &open.sidebar)
    }

    pub(crate) fn context_dock_expanded(&self) -> bool {
        match &self.context_sidebar {
            ContextSidebar::Open(open) => open.disclosure.is_expanded(SectionId::Dock),
            ContextSidebar::Closed => false,
        }
    }

    /// Body rect of the F7 column, or empty while it is closed.
    pub(super) fn context_sidebar_body(&mut self, column: Rect) -> Rect {
        if !matches!(self.context_sidebar, ContextSidebar::Open(_)) || column.width == 0 {
            return Rect::default();
        }
        let heights = footer::heights(&mut self.sidebar_heights, column);
        SidebarLayout::compute(column, heights).body
    }

    pub(crate) fn sidebar_open_flags(&self) -> (bool, bool) {
        (
            self.timeline_mode.panel().is_some(),
            matches!(self.context_sidebar, ContextSidebar::Open(_)),
        )
    }

    pub(crate) fn f6_panel_focused(&self) -> bool {
        self.timeline_mode
            .panel()
            .is_some_and(|panel| panel.focus == crate::views::timeline_panel::PanelFocus::Panel)
    }

    pub(crate) fn focus_context_sidebar(&mut self) {
        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
            open.focus = SidebarFocus::Sidebar;
            open.reveal_pending = true;
        }
    }

    pub(crate) fn clear_context_sidebar_focus(&mut self) {
        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
            open.focus = SidebarFocus::Chat;
        }
    }

    fn clear_timeline_panel_focus(&mut self) {
        if let Some(panel) = self.timeline_mode.panel_mut() {
            panel.focus = crate::views::timeline_panel::PanelFocus::Chat;
        }
    }

    pub(crate) fn f7_sidebar_focused(&self) -> bool {
        matches!(
            &self.context_sidebar,
            ContextSidebar::Open(open) if open.focus == SidebarFocus::Sidebar
        )
    }

    /// Match this session to the pager-wide F6/F7 open state. Contents stay this session's.
    /// `true` means F7 was just opened and needs a policy read.
    pub(crate) fn adopt_global_sidebars(
        &mut self,
        f6_open: bool,
        f7_open: bool,
        f6_focused: bool,
        f7_focused: bool,
    ) -> bool {
        if self.is_minimal_mode() {
            return false;
        }
        let f6_focused = f6_focused && !f7_focused;
        if f6_open {
            if self.timeline_mode.panel().is_none() {
                let _ = self.open_timeline_panel_at(self.scrollback.timeline_follow_index());
            }
            if let Some(panel) = self.timeline_mode.panel_mut() {
                panel.focus = if f6_focused {
                    crate::views::timeline_panel::PanelFocus::Panel
                } else {
                    crate::views::timeline_panel::PanelFocus::Chat
                };
            }
        } else if self.timeline_mode.panel().is_some() {
            self.timeline_mode = crate::views::timeline_panel::TimelineMode::Rail;
        }
        if f7_open {
            let focus = if f7_focused {
                SidebarFocus::Sidebar
            } else {
                SidebarFocus::Chat
            };
            if matches!(self.context_sidebar, ContextSidebar::Closed) {
                self.context_sidebar = ContextSidebar::Open(OpenContextSidebar {
                    focus,
                    sidebar: SidebarState::default(),
                    layout: None,
                    policy: PolicyState::Loading,
                    disclosure: self.restored_section_disclosure(),
                    todo_count: self.todo.sidebar_row_count(),
                    reveal_pending: f7_focused,
                });
            } else if let ContextSidebar::Open(open) = &mut self.context_sidebar {
                open.focus = focus;
                open.policy = PolicyState::Loading;
            }
            true
        } else {
            if matches!(self.context_sidebar, ContextSidebar::Open(_)) {
                self.context_sidebar = ContextSidebar::Closed;
            }
            false
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
                    disclosure: self.restored_section_disclosure(),
                    todo_count: self.todo.sidebar_row_count(),
                    reveal_pending: false,
                });
                self.clear_timeline_panel_focus();
                InputOutcome::Action(Action::FetchContextPolicy)
            }
            ContextSidebar::Open(_) => {
                self.context_sidebar = ContextSidebar::Closed;
                InputOutcome::Changed
            }
        }
    }

    /// The shell answered a get or set.
    pub(crate) fn context_policy_answered(&mut self, answer: Result<ContextPolicyReport, crate::app::actions::ContextPolicyTransportError>) {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else { return; };
        open.policy = match answer {
            Ok(report) => {
                if let Some(previous) = open.policy.shown() {
                    if let (Some(old), Some(new)) = (&previous.selection, &report.selection) {
                        if new.version.revision < old.version.revision { return; }
                    }
                }
                match std::mem::replace(&mut open.policy, PolicyState::Unavailable) {
                    PolicyState::Confirming { confirmation, .. } => {
                        if confirmation.still_current(&report) {
                            PolicyState::Confirming { shown: report, confirmation }
                        } else {
                            PolicyState::ExpiredConfirmation(policy::ExpiredConfirmation::Confirmed(report))
                        }
                    }
                    PolicyState::ExpiredConfirmation(_) => PolicyState::ExpiredConfirmation(policy::ExpiredConfirmation::Confirmed(report)),
                    PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Applying { .. } | PolicyState::Unavailable => PolicyState::Ready(report),
                }
            }
            Err(error) => {
                tracing::warn!(%error, "context policy request failed");
                match open.policy {
                    PolicyState::Confirming { .. } | PolicyState::ExpiredConfirmation(_) => PolicyState::ExpiredConfirmation(policy::ExpiredConfirmation::Unavailable),
                    PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Applying { .. } | PolicyState::Unavailable => PolicyState::Unavailable,
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
            Event::Key(key)
                if key.kind != KeyEventKind::Release && open.focus == SidebarFocus::Sidebar =>
            {
                let viewport = open.layout.map_or(1, |layout| layout.body.height as usize);
                let command = gesture::command_for(key, viewport)?;
                Some(self.run_sidebar_command_saving(command))
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let hit = open
                    .layout
                    .and_then(|layout| layout.hit(mouse.column, mouse.row, &open.sidebar));
                match hit {
                    Some(SidebarHit::Line(
                        line @ (SidebarLine::Row(..) | SidebarLine::SectionTitle(_)),
                    )) => {
                        open.focus = SidebarFocus::Sidebar;
                        open.sidebar.cursor = Some(line);
                        if let Some(panel) = self.timeline_mode.panel_mut() {
                            panel.focus = crate::views::timeline_panel::PanelFocus::Chat;
                        }
                        let in_context = match line {
                            SidebarLine::Row(section, _) | SidebarLine::SectionTitle(section) => {
                                section == CONTEXT_SECTION
                            }
                            SidebarLine::Hosted(_) => false,
                        };
                        if in_context {
                            Some(InputOutcome::Changed)
                        } else {
                            Some(self.run_sidebar_command_saving(gesture::activation(mouse.modifiers)))
                        }
                    }
                    Some(SidebarHit::Line(SidebarLine::Hosted(_))) => {
                        open.focus = SidebarFocus::Chat;
                        None
                    }
                    Some(SidebarHit::Header | SidebarHit::Footer) => {
                        open.focus = SidebarFocus::Sidebar;
                        if let Some(panel) = self.timeline_mode.panel_mut() {
                            panel.focus = crate::views::timeline_panel::PanelFocus::Chat;
                        }
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

    fn toggle_context_category(&mut self, category: ContextCategory) -> InputOutcome {
        let ContextSidebar::Open(open) = &self.context_sidebar else { return InputOutcome::Unchanged; };
        let shown = match open.policy.changeable() {
            Ok(shown) => shown,
            Err(reason) => { self.show_toast(reason); return InputOutcome::Changed; }
        };
        let Some(selection) = &shown.selection else {
            self.show_toast("Context scope has not been confirmed by the Shell.");
            return InputOutcome::Changed;
        };
        self.request_context_change(xai_grok_shell::sampling::ContextPolicyMutation::SetCategories {
            categories: xai_grok_shell::sampling::ContextPolicyCategories::one(category),
            value: if shown.policy.includes(category) { ContextSwitch::Excluded } else { ContextSwitch::Included },
            target: selection.scope,
        })
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
        let heights = footer::heights(&mut self.sidebar_heights, area);
        let categories = all_categories();
        let consequences: Vec<[Line<'static>; 1]> = categories
            .iter()
            .map(|&category| [Line::from(consequence(category))])
            .collect();
        let defaults = self.scrollback.display_defaults();
        let disclosure = open.disclosure;
        let rows = group_rows::grouped(SectionId::Context, disclosure,
            rows(open.policy.shown(), &categories, &consequences, &theme),
            &categories, open.policy.shown(), defaults, &theme);
        let display = group_rows::grouped(SectionId::Display, disclosure,
            display_rows(MessageKind::settings_kinds(), defaults, &theme),
            &categories, open.policy.shown(), defaults, &theme);
        let context_title = lines::title_line(
            SectionId::Context,
            disclosure.is_expanded(SectionId::Context),
            lines::context_summary(open.policy.shown(), &categories),
            &theme,
        );
        let display_title = lines::title_line(
            SectionId::Display,
            disclosure.is_expanded(SectionId::Display),
            Some(lines::display_summary(defaults)),
            &theme,
        );
        self.todo.refresh_entries();
        let todo_count = self.todo.sidebar_row_count();
        if open.todo_count != todo_count {
            open.todo_count = todo_count;
            open.apply_disclosure(open.disclosure);
        }
        let pending_dock = self.pending_dock.take();
        let dock_height = if disclosure.is_expanded(SectionId::Dock) {
            pending_dock.as_ref().map_or(0, |dock| dock.rect.height)
        } else { 0 };
        let dock_data = pending_dock.map(|dock| std::cell::RefCell::new(dock.data));
        let dock_slot = super::sidebars::DockSlot {
            data: dock_data.as_ref(),
            height: dock_height,
            pointer: self.last_mouse_pos,
        };
        let dock_controls = [SidebarRow {
            left: Line::from("  Placement"),
            right: Some(Line::from(crate::views::dock::placement().label())),
            detail: &[],
        }];
        let sections = [
            SidebarSection::Rows {
                title: lines::title_line(SectionId::Dock, disclosure.is_expanded(SectionId::Dock), None, &theme),
                rows: disclosure.visible(SectionId::Dock, &dock_controls),
            },
            SidebarSection::Hosted(&dock_slot),
            SidebarSection::LazyRows {
                title: lines::title_line(SectionId::Todo, disclosure.is_expanded(SectionId::Todo), None, &theme),
                count: if disclosure.is_expanded(SectionId::Todo) { todo_count } else { 0 },
                source: &self.todo,
            },
            SidebarSection::Rows {
                title: context_title,
                rows: disclosure.visible(SectionId::Context, &rows),
            },
            SidebarSection::Rows {
                title: display_title,
                rows: disclosure.visible(SectionId::Display, &display),
            },
        ];
        open.sidebar.rebuild_layout(&sections);
        let viewport = SidebarLayout::compute(area, heights).body.height as usize;
        open.sidebar.clamp_scroll(viewport);
        if open.sidebar.cursor.is_none() && open.focus == SidebarFocus::Sidebar {
            open.sidebar.cursor =
                Some(Outline::build(disclosure, open.todo_count).default_cursor());
            open.reveal_pending = true;
        }
        if open.reveal_pending {
            open.sidebar.reveal_cursor(viewport);
            open.reveal_pending = false;
        }
        let cursor_target = open.sidebar.cursor.and_then(|line| CursorTarget::of(line, open.disclosure));
        let status = match open.policy {
            PolicyState::Loading => "Loading…",
            PolicyState::Applying { .. } => "Applying…",
            PolicyState::Confirming { .. } => "Context confirmation · Enter cancels initially",
            PolicyState::ExpiredConfirmation(_) => "Context confirmation expired · dismiss first",
            PolicyState::Unavailable => "Unavailable for this session",
            PolicyState::Ready(_) => gesture::hint_for(cursor_target),
        };
        let header = [Line::from("Session overview")];
        let saving = self.panel_persistence.display_saving_status();
        let footer = footer::lines(status, saving);
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
                focused: open.focus == SidebarFocus::Sidebar,
                theme: &theme,
            },
            buf,
        ));
        match &open.policy {
            PolicyState::Confirming { confirmation, .. } => confirmation.render(area, buf, &theme),
            PolicyState::ExpiredConfirmation(expired) => expired.render(area, buf, &theme),
            PolicyState::Loading | PolicyState::Ready(_) | PolicyState::Applying { .. } | PolicyState::Unavailable => {},
        }
        let dock_rect = open.layout.and_then(|layout| layout.hosted_rect(SectionIdx(1), &open.sidebar));
        if self.f7_body.contains((self.pane_areas.dock.x, self.pane_areas.dock.y).into()) {
            self.pane_areas.dock = dock_rect.map(|(rect, _)| rect).unwrap_or_default();
        }
        let standalone = self.pane_areas.dock.area() > 0
            && dock_rect.is_none_or(|(rect, _)| rect != self.pane_areas.dock);
        self.dock_shown = standalone || dock_rect.is_some();
        if self.active_pane == super::AgentPane::Dock {
            if !self.dock_shown {
                self.set_active_pane(super::AgentPane::Scrollback, false);
            } else {
                self.dock_focus_in_sidebar = !standalone
                    || (self.dock_focus_in_sidebar && dock_rect.is_some());
            }
        }
        if self.dock_shown {
            self.clamp_dock_overflow();
        }
        if let (Some((rect, _)), Some(data)) = (dock_rect, dock_data.as_ref()) {
            let data = data.borrow();
            if rect.contains(self.last_mouse_pos.into()) {
                self.dock_hovered = data.hovered;
                self.cache_dock_stop_at(rect, &data);
            }
        }
    }
}

#[cfg(test)]
#[path = "context_sidebar/hierarchy_tests.rs"]
mod hierarchy_tests;
