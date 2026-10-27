//! What the F7 sidebar does for a [`SidebarCommand`]: tree navigation, activation and bulk gestures.

use super::gesture::{Goal, SidebarCommand, Spread};
use super::outline::{CursorTarget, Outline};
use super::{AgentView, ContextSidebar, OpenContextSidebar, PolicyState, RowTarget, SidebarFocus};
use crate::app::app_view::InputOutcome;
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use crate::views::tree_nav::{
    BulkAnchor, BulkDirection, BulkScope, TreeKey, TreeStep, bulk_set, bulk_toggle, navigate,
    toggle,
};
use xai_grok_shell::extensions::context_policy::ContextPolicyReport;

/// What a bulk gesture reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reached {
    Sections,
    Kinds,
    Both,
}

impl Reached {
    fn of(spread: Spread, target: CursorTarget) -> Self {
        match (spread, target) {
            (Spread::Everything, _) => Self::Both,
            (Spread::Peers, CursorTarget::Row(RowTarget::Display(_))) => Self::Kinds,
            (Spread::Peers, CursorTarget::Title(_) | CursorTarget::Row(RowTarget::Context(_) | RowTarget::DockPlacement)) => {
                Self::Sections
            }
        }
    }

    fn sections(self) -> bool {
        matches!(self, Self::Sections | Self::Both)
    }

    fn kinds(self) -> bool {
        matches!(self, Self::Kinds | Self::Both)
    }

    /// Said when a bulk gesture finds nothing to change.
    fn already(self, direction: BulkDirection) -> &'static str {
        match (self, direction) {
            (Self::Sections, BulkDirection::Expand) => "All sections are already open",
            (Self::Sections, BulkDirection::Collapse) => "All sections are already closed",
            (Self::Kinds, BulkDirection::Expand) => "All message kinds already show full",
            (Self::Kinds, BulkDirection::Collapse) => "All message kinds already show one line",
            (Self::Both, BulkDirection::Expand) => "Everything is already open and full",
            (Self::Both, BulkDirection::Collapse) => "Everything is already closed and on one line",
        }
    }
}

impl PolicyState {
    /// The report a change starts from, or why none can be sent now.
    pub(super) fn changeable(&self) -> Result<&ContextPolicyReport, &'static str> {
        match self {
            Self::Ready(report) => Ok(report),
            Self::Loading => {
                Err("Context switches are still loading from the session; try again in a moment")
            }
            Self::Applying { .. } => {
                Err("The previous context change is still being applied; wait for it to finish")
            }
            Self::Unavailable => {
                Err("The session's context policy could not be read, so ON/OFF cannot be changed")
            }
        }
    }
}

impl OpenContextSidebar {
    /// Which sections are open, to store and restore.
    pub(crate) fn disclosure(&self) -> super::SectionDisclosure {
        self.disclosure
    }

    pub(crate) fn apply_disclosure(&mut self, disclosure: super::SectionDisclosure) {
        self.disclosure = disclosure;
        self.repair_cursor();
    }

    #[cfg(test)]
    pub(crate) fn cursor(&self) -> Option<crate::views::sidebar::SidebarLine> {
        self.sidebar.cursor
    }

    /// Move the cursor off a line that a fold just hid, and scroll to it on the next draw.
    fn repair_cursor(&mut self) {
        let outline = Outline::build(self.disclosure, self.todo_count);
        if self.sidebar.cursor.is_some() {
            self.sidebar.cursor = self
                .sidebar
                .cursor
                .and_then(|line| outline.visible_line(line))
                .or_else(|| Some(outline.default_cursor()));
        }
        self.reveal_pending = true;
    }

    fn apply_step(&mut self, outline: &Outline, step: TreeStep) -> InputOutcome {
        match step {
            TreeStep::Stay => InputOutcome::Unchanged,
            TreeStep::MoveTo(index) => match outline.line(index) {
                Some(line) => {
                    self.sidebar.cursor = Some(line);
                    self.reveal_pending = true;
                    InputOutcome::Changed
                }
                None => InputOutcome::Unchanged,
            },
            TreeStep::Expand(index) => self.set_section(outline, index, true),
            TreeStep::Collapse(index) => self.set_section(outline, index, false),
        }
    }

    fn set_section(&mut self, outline: &Outline, index: usize, expanded: bool) -> InputOutcome {
        let Some(section) = outline.section_of_title(index) else {
            return InputOutcome::Unchanged;
        };
        self.disclosure.set_expanded(section, expanded);
        self.repair_cursor();
        InputOutcome::Changed
    }
}

impl AgentView {
    pub(super) fn run_sidebar_command(&mut self, command: SidebarCommand) -> InputOutcome {
        match command {
            SidebarCommand::Tree(key) => self.navigate_sidebar(key),
            SidebarCommand::Activate => self.activate_cursor(),
            SidebarCommand::Bulk {
                spread,
                goal,
                anchor,
            } => self.bulk_sidebar(spread, goal, anchor),
            SidebarCommand::ToggleDisplayScope => {
                self.toggle_message_display_scope();
                InputOutcome::Changed
            }
            SidebarCommand::FocusChat => {
                if let ContextSidebar::Open(open) = &mut self.context_sidebar {
                    open.focus = SidebarFocus::Chat;
                }
                InputOutcome::Changed
            }
            SidebarCommand::Close => {
                self.context_sidebar = ContextSidebar::Closed;
                InputOutcome::Changed
            }
        }
    }

    pub(crate) fn sidebar_end_is_stay(&self) -> bool {
        let ContextSidebar::Open(open) = &self.context_sidebar else {
            return false;
        };
        let outline = Outline::build(open.disclosure, open.todo_count);
        if outline.rows().is_empty() {
            return true;
        }
        let Some(at) = open.sidebar.cursor.and_then(|line| outline.index_of(line)) else {
            return false;
        };
        matches!(navigate(outline.rows(), at, TreeKey::End), TreeStep::Stay)
    }

    fn navigate_sidebar(&mut self, key: TreeKey) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let outline = Outline::build(open.disclosure, open.todo_count);
        let Some(at) = open.sidebar.cursor.and_then(|line| outline.index_of(line)) else {
            open.sidebar.cursor = Some(outline.default_cursor());
            open.reveal_pending = true;
            return InputOutcome::Changed;
        };
        let step = navigate(outline.rows(), at, key);
        open.apply_step(&outline, step)
    }

    /// Enter, Space and a plain click on the cursor line.
    fn activate_cursor(&mut self) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let Some(line) = open.sidebar.cursor else {
            return InputOutcome::Unchanged;
        };
        match CursorTarget::of(line) {
            Some(CursorTarget::Title(_)) => {
                let outline = Outline::build(open.disclosure, open.todo_count);
                let Some(at) = outline.index_of(line) else {
                    return InputOutcome::Unchanged;
                };
                let step = toggle(outline.rows(), at);
                open.apply_step(&outline, step)
            }
            Some(CursorTarget::Row(RowTarget::DockPlacement)) => {
                self.set_dock_placement(crate::views::dock::placement().next());
                InputOutcome::Changed
            }
            Some(CursorTarget::Row(RowTarget::Context(category))) => {
                self.toggle_context_category(category)
            }
            Some(CursorTarget::Row(RowTarget::Display(kind))) => {
                let form = match self.scrollback.display_defaults().shown(kind) {
                    DisplayForm::Collapsed => DisplayForm::Expanded,
                    DisplayForm::Expanded => DisplayForm::Collapsed,
                };
                self.scrollback.choose_kind_default(kind, form);
                InputOutcome::Changed
            }
            None => InputOutcome::Unchanged,
        }
    }

    /// Open or close sections and message kinds together. Context switches are never part of it.
    /// User and assistant message rows are never reached, and a modifier activation leaves the
    /// cursor line's own section and kind alone.
    fn bulk_sidebar(&mut self, spread: Spread, goal: Goal, anchor: BulkAnchor) -> InputOutcome {
        let ContextSidebar::Open(open) = &mut self.context_sidebar else {
            return InputOutcome::Unchanged;
        };
        let outline = Outline::build(open.disclosure, open.todo_count);
        let Some(line) = open.sidebar.cursor else {
            return InputOutcome::Unchanged;
        };
        let (Some(target), Some(at)) = (CursorTarget::of(line), outline.owner_title(line)) else {
            return InputOutcome::Unchanged;
        };
        let reached = Reached::of(spread, target);
        let scope = match spread {
            Spread::Peers => BulkScope::Siblings,
            Spread::Everything => BulkScope::All,
        };
        // The kinds this gesture may flip: the bulk kinds, without the cursor line's own kind
        // when the gesture leaves that line alone.
        let anchor_kind = match (anchor, target) {
            (BulkAnchor::Exclude, CursorTarget::Row(RowTarget::Display(kind))) => Some(kind),
            _ => None,
        };
        let kinds = MessageKind::bulk_kinds()
            .iter()
            .copied()
            .filter(|&kind| Some(kind) != anchor_kind);
        let direction = match goal {
            Goal::Set(direction) => direction,
            Goal::Toggle => {
                let section_closed = reached.sections()
                    && bulk_toggle(outline.rows(), at, scope, anchor).direction
                        == BulkDirection::Expand;
                let kind_closed = reached.kinds()
                    && kinds.clone().any(|kind| {
                        self.scrollback.display_defaults().shown(kind) == DisplayForm::Collapsed
                    });
                if section_closed || kind_closed {
                    BulkDirection::Expand
                } else {
                    BulkDirection::Collapse
                }
            }
        };
        let mut changed = false;
        if reached.sections() {
            let expanded = direction == BulkDirection::Expand;
            for &index in &bulk_set(outline.rows(), at, scope, direction, anchor).indices {
                if let Some(section) = outline.section_of_title(index) {
                    open.disclosure.set_expanded(section, expanded);
                    changed = true;
                }
            }
        }
        if reached.kinds() {
            let form = match direction {
                BulkDirection::Expand => DisplayForm::Expanded,
                BulkDirection::Collapse => DisplayForm::Collapsed,
            };
            for kind in kinds {
                // An unset kind renders by its block's own policy, so it is set even when `shown` already says `form`.
                if self.scrollback.display_defaults().get(kind) != Some(form) {
                    self.scrollback.choose_kind_default(kind, form);
                    changed = true;
                }
            }
        }
        if changed {
            open.repair_cursor();
        } else {
            self.show_toast(reached.already(direction));
        }
        InputOutcome::Changed
    }
}
