//! The F7 tree: Dock, Todos, Context, Message display.

use super::{CONTEXT_SECTION, DISPLAY_SECTION, DOCK_SECTION, TODO_SECTION, RowTarget, row_target};
use crate::scrollback::block::MessageKind;
use crate::views::sidebar::{RowIdx, SectionIdx, SidebarLine};
use crate::views::tree_nav::{TreeRow, visible_anchor};
use strum::IntoEnumIterator;
use xai_grok_shell::sampling::all_categories;

/// A foldable section of the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub(crate) enum SectionId {
    Dock,
    Todo,
    Context,
    Display,
}

impl SectionId {
    pub(crate) const fn index(self) -> SectionIdx {
        match self {
            Self::Dock => DOCK_SECTION,
            Self::Todo => TODO_SECTION,
            Self::Context => CONTEXT_SECTION,
            Self::Display => DISPLAY_SECTION,
        }
    }

    pub(super) fn of_index(index: SectionIdx) -> Option<Self> {
        Self::iter().find(|section| section.index() == index)
    }

    pub(super) const fn title(self) -> &'static str {
        match self {
            Self::Dock => "Dock",
            Self::Todo => "Todos",
            Self::Context => "Context",
            Self::Display => "Message display",
        }
    }

    pub(super) fn row_count(self, todo_count: usize) -> usize {
        match self {
            Self::Dock => 1,
            Self::Todo => todo_count,
            Self::Context => all_categories().len(),
            Self::Display => MessageKind::settings_kinds().len(),
        }
    }

    pub(crate) fn title_line(self) -> SidebarLine {
        SidebarLine::SectionTitle(self.index())
    }

    pub(crate) fn row_line(self, row: usize) -> SidebarLine {
        SidebarLine::Row(self.index(), RowIdx(row))
    }
}

/// Which sections are open; every section starts open. Small and copyable so it can be stored and restored.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SectionDisclosure {
    dock: crate::app::panel_settings::DisclosureState,
    todo: crate::app::panel_settings::DisclosureState,
    context_collapsed: bool,
    display_collapsed: bool,
}

impl SectionDisclosure {
    pub(crate) fn is_expanded(self, section: SectionId) -> bool {
        match section {
            SectionId::Dock => self.dock.is_expanded(),
            SectionId::Todo => self.todo.is_expanded(),
            SectionId::Context => !self.context_collapsed,
            SectionId::Display => !self.display_collapsed,
        }
    }

    pub(crate) fn set_expanded(&mut self, section: SectionId, expanded: bool) {
        match section {
            SectionId::Dock => self.dock = crate::app::panel_settings::DisclosureState::from_expanded(expanded),
            SectionId::Todo => self.todo = crate::app::panel_settings::DisclosureState::from_expanded(expanded),
            SectionId::Context => self.context_collapsed = !expanded,
            SectionId::Display => self.display_collapsed = !expanded,
        }
    }

    /// The rows a section contributes to the layout: none while it is collapsed.
    pub(super) fn visible<T>(self, section: SectionId, rows: &[T]) -> &[T] {
        if self.is_expanded(section) { rows } else { &[] }
    }
}

/// What the cursor line is, as far as activation and gestures care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorTarget {
    Title(SectionId),
    Row(RowTarget),
}

impl CursorTarget {
    /// `None` for the hosted dock slot and for lines outside the tree.
    pub(super) fn of(line: SidebarLine) -> Option<Self> {
        match line {
            SidebarLine::SectionTitle(index) => SectionId::of_index(index).map(Self::Title),
            SidebarLine::Row(section, row) => row_target(section, row).map(Self::Row),
            SidebarLine::Hosted(_) => None,
        }
    }
}

/// Every line of the tree in order, hidden or not, with the tree shape `tree_nav` navigates.
pub(super) struct Outline {
    rows: Vec<TreeRow>,
    lines: Vec<SidebarLine>,
}

impl Outline {
    pub(super) fn build(disclosure: SectionDisclosure, todo_count: usize) -> Self {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        for section in SectionId::iter() {
            rows.push(TreeRow::branch(0, disclosure.is_expanded(section)));
            lines.push(section.title_line());
            for row in 0..section.row_count(todo_count) {
                rows.push(TreeRow::leaf(1));
                lines.push(section.row_line(row));
            }
        }
        Self { rows, lines }
    }

    pub(super) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(super) fn index_of(&self, line: SidebarLine) -> Option<usize> {
        self.lines.iter().position(|&candidate| candidate == line)
    }

    pub(super) fn line(&self, index: usize) -> Option<SidebarLine> {
        self.lines.get(index).copied()
    }

    /// The section whose title is tree row `index`.
    pub(super) fn section_of_title(&self, index: usize) -> Option<SectionId> {
        match self.line(index)? {
            SidebarLine::SectionTitle(section) => SectionId::of_index(section),
            SidebarLine::Hosted(_) | SidebarLine::Row(..) => None,
        }
    }

    /// Tree index of the title that owns `line` (the title itself for a title).
    pub(super) fn owner_title(&self, line: SidebarLine) -> Option<usize> {
        let section = match line {
            SidebarLine::SectionTitle(section) | SidebarLine::Row(section, _) => section,
            SidebarLine::Hosted(_) => return None,
        };
        self.index_of(SidebarLine::SectionTitle(section))
    }

    /// The line to start on: the first context switch, or its title while that section is collapsed.
    pub(super) fn default_cursor(&self) -> SidebarLine {
        let first = SectionId::Context.row_line(0);
        self.index_of(first)
            .and_then(|index| visible_anchor(&self.rows, index))
            .and_then(|anchor| self.line(anchor))
            .unwrap_or(first)
    }

    /// The nearest visible line at or above `line`.
    pub(super) fn visible_line(&self, line: SidebarLine) -> Option<SidebarLine> {
        let index = self.index_of(line)?;
        self.line(visible_anchor(&self.rows, index)?)
    }
}
