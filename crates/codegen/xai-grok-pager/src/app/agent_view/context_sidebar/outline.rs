//! The F7 tree: Dock, Todos, Context, Message display.

use super::groups::GroupId;
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
    groups: [crate::app::panel_settings::DisclosureState; GroupId::COUNT],
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

    pub(super) fn group_expanded(self, group: GroupId) -> bool {
        self.groups[group.index()].is_expanded()
    }

    pub(super) fn set_group(&mut self, group: GroupId, expanded: bool) {
        self.groups[group.index()] = crate::app::panel_settings::DisclosureState::from_expanded(expanded);
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
    pub(super) fn of(line: SidebarLine, disclosure: SectionDisclosure) -> Option<Self> {
        match line {
            SidebarLine::SectionTitle(index) => SectionId::of_index(index).map(Self::Title),
            SidebarLine::Row(section, row) => row_target(section, row, disclosure).map(Self::Row),
            SidebarLine::Hosted(_) => None,
        }
    }
}

/// Every line of the tree in order, hidden or not, with the tree shape `tree_nav` navigates.
pub(super) struct Outline {
    rows: Vec<TreeRow>,
    lines: Vec<Option<SidebarLine>>,
    targets: Vec<CursorTarget>,
}

impl Outline {
    pub(super) fn build(disclosure: SectionDisclosure, todo_count: usize) -> Self {
        let mut rows = Vec::new();
        let mut lines = Vec::new();
        let mut targets = Vec::new();
        for section in SectionId::iter() {
            let expanded = disclosure.is_expanded(section);
            rows.push(TreeRow::branch(0, expanded));
            lines.push(Some(section.title_line()));
            targets.push(CursorTarget::Title(section));
            let mut visible_index = 0;
            let groups: Vec<_> = GroupId::all().filter(|group| group.section() == section).collect();
            if groups.is_empty() {
                for row in 0..section.row_count(todo_count) {
                    rows.push(TreeRow::leaf(1));
                    lines.push(expanded.then_some(section.row_line(row)));
                    targets.push(CursorTarget::Row(RowTarget::DockPlacement));
                }
            } else {
                for group in groups {
                    rows.push(TreeRow::branch(1, disclosure.group_expanded(group)));
                    lines.push(expanded.then_some(section.row_line(visible_index)));
                    targets.push(CursorTarget::Row(RowTarget::Group(group)));
                    visible_index += 1;
                    for child in group.children() {
                        rows.push(TreeRow::leaf(2));
                        let visible = disclosure.group_expanded(group);
                        lines.push((expanded && visible).then_some(section.row_line(visible_index)));
                        targets.push(CursorTarget::Row(child));
                        if visible { visible_index += 1; }
                    }
                }
            }
        }
        Self { rows, lines, targets }
    }

    pub(super) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(super) fn index_of(&self, line: SidebarLine) -> Option<usize> {
        self.lines.iter().position(|&candidate| candidate == Some(line))
    }

    pub(super) fn line(&self, index: usize) -> Option<SidebarLine> {
        self.lines.get(index).copied().flatten()
    }

    pub(super) fn target(&self, index: usize) -> Option<CursorTarget> {
        self.targets.get(index).copied()
    }

    pub(super) fn line_for_target(&self, target: CursorTarget) -> Option<SidebarLine> {
        let index = self.targets.iter().position(|&candidate| candidate == target)?;
        self.line(visible_anchor(&self.rows, index)?)
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
        let first = xai_grok_shell::sampling::all_categories().into_iter().next();
        first.and_then(|category| self.line_for_target(CursorTarget::Row(RowTarget::Context(category))))
            .unwrap_or_else(|| SectionId::Context.title_line())
    }

    /// The nearest visible line at or above `line`.
    pub(super) fn visible_line(&self, line: SidebarLine) -> Option<SidebarLine> {
        let index = self.index_of(line)?;
        self.line(visible_anchor(&self.rows, index)?)
    }
}

pub(super) fn section_targets(disclosure: SectionDisclosure, section: SectionId) -> Vec<RowTarget> {
    GroupId::all().filter(|group| group.section() == section).flat_map(|group| {
        std::iter::once(RowTarget::Group(group)).chain(
            group.children().into_iter().filter(move |_| disclosure.group_expanded(group))
        )
    }).collect()
}
