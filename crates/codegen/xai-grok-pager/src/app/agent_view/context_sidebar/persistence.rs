//! Saving what an F7 gesture changed (section folds, message-display forms) and restoring it.
//!
//! A gesture's changed forms use the effective Display scope; changed section folds use Global.
//! A form the user set is stored as chosen: a kind nobody set
//! renders by its block's own policy, which is not one form, so no stored form can be called redundant.
//! Context switches belong to the shell and are never touched here.

use super::gesture::SidebarCommand;
use super::groups::GroupId;
use super::outline::CursorTarget;
use super::RowTarget;
use super::{AgentView, ContextSidebar, SectionDisclosure, SectionId};
use crate::app::app_view::InputOutcome;
use crate::app::panel_settings::{DisclosureState, PanelSettings, Setting, SidebarSection};
use crate::scrollback::block::{DisplayDefaults, MessageKind};
use crate::scrollback::types::DisplayForm;
use strum::IntoEnumIterator;

/// The part of the F7 view that is stored.
struct StoredView {
    /// `None` while F7 is closed.
    disclosure: Option<SectionDisclosure>,
    /// One entry per [`MessageKind::settings_kinds`], in that order; `None` = the user never set the kind.
    forms: Vec<(MessageKind, Option<DisplayForm>)>,
}

const fn stored_section(section: SectionId) -> SidebarSection {
    match section {
        SectionId::Dock => SidebarSection::Dock,
        SectionId::Todo => SidebarSection::TodoSection,
        SectionId::Context => SidebarSection::Context,
        SectionId::Display => SidebarSection::MessageDisplay,
    }
}

fn restored_disclosure(settings: &PanelSettings) -> SectionDisclosure {
    let mut disclosure = SectionDisclosure::default();
    for section in SectionId::iter() {
        disclosure.set_expanded(
            section,
            settings.section(stored_section(section)).is_expanded(),
        );
    }
    for group in GroupId::all() {
        disclosure.set_group(group, settings.section(group.stored()).is_expanded());
    }
    disclosure
}

impl AgentView {
    fn stored_f7_view(&self) -> StoredView {
        let disclosure = match &self.context_sidebar {
            ContextSidebar::Open(open) => Some(open.disclosure()),
            ContextSidebar::Closed => None,
        };
        let defaults = self.scrollback.display_defaults();
        StoredView {
            disclosure,
            forms: MessageKind::settings_kinds()
                .iter()
                .map(|&kind| (kind, defaults.get(kind)))
                .collect(),
        }
    }

    /// Runs the gesture, then saves whatever it changed.
    pub(super) fn run_sidebar_command_saving(&mut self, command: SidebarCommand) -> InputOutcome {
        let forced = match (&self.context_sidebar, command) {
            (ContextSidebar::Open(open), SidebarCommand::Activate | SidebarCommand::Bulk { .. }) => {
                match open.sidebar.cursor.and_then(|line| CursorTarget::of(line, open.disclosure)) {
                    Some(CursorTarget::Row(RowTarget::Group(GroupId::Display(group)))) => group.kinds().collect(),
                    Some(CursorTarget::Row(RowTarget::Display(kind))) => if matches!(command, SidebarCommand::Activate) { vec![kind] } else { Vec::new() },
                    Some(CursorTarget::Title(_) | CursorTarget::Row(RowTarget::Group(GroupId::Context(_)) | RowTarget::Context(_) | RowTarget::DockPlacement)) | None => Vec::new(),
                }
            }
            (ContextSidebar::Closed, SidebarCommand::Activate | SidebarCommand::Bulk { .. })
            | (ContextSidebar::Open(_) | ContextSidebar::Closed,
                SidebarCommand::Tree(_) | SidebarCommand::ToggleDisplayScope
                | SidebarCommand::ToggleContextScope | SidebarCommand::Inspect
                | SidebarCommand::FocusChat | SidebarCommand::Close) => Vec::new(),
        };
        let before = self.stored_f7_view();
        let outcome = self.run_sidebar_command(command);
        let after = self.stored_f7_view();
        self.save_f7_changes(&before, &after, &forced);
        outcome
    }

    fn save_f7_changes(&mut self, before: &StoredView, after: &StoredView, forced: &[MessageKind]) {
        let forms: Vec<(MessageKind, DisplayForm)> = before
            .forms
            .iter()
            .zip(&after.forms)
            .filter(|(old, new)| old != new || forced.contains(&new.0))
            .filter_map(|(_, &(kind, form))| form.map(|form| (kind, form)))
            .collect();
        let sections: Vec<(SectionId, bool)> = match (before.disclosure, after.disclosure) {
            (Some(old), Some(new)) => SectionId::iter()
                .filter(|&section| old.is_expanded(section) != new.is_expanded(section))
                .map(|section| (section, new.is_expanded(section)))
                .collect(),
            (None, None) | (Some(_), None) | (None, Some(_)) => Vec::new(),
        };
        let groups: Vec<_> = match (before.disclosure, after.disclosure) {
            (Some(old), Some(new)) => GroupId::all()
                .filter(|&group| old.group_expanded(group) != new.group_expanded(group))
                .map(|group| Setting::Section { section: group.stored(), state: DisclosureState::from_expanded(new.group_expanded(group)) })
                .collect(),
            (None, None) | (Some(_), None) | (None, Some(_)) => Vec::new(),
        };
        if forms.is_empty() && sections.is_empty() && groups.is_empty() {
            return;
        }
        let mut section_settings: Vec<_> = sections.iter().map(|&(section, open)| Setting::Section {
            section: stored_section(section),
            state: DisclosureState::from_expanded(open),
        }).collect();
        section_settings.extend(groups);
        let form_settings: Vec<_> = forms.iter().map(|&(kind, form)| Setting::KindForm { kind, form }).collect();
        if !section_settings.is_empty() { self.save_global_panel_settings(&section_settings); }
        if !form_settings.is_empty() { self.save_display_settings(&form_settings); }
        if self.panel_persistence.store.is_some() {
            self.restore_f7_panel_settings();
        }
    }

    /// Applies the stored message-display forms to the scrollback, and the folds to an open F7.
    /// A kind the stored layers no longer set (another session's override) goes back to its block's own policy.
    pub(in crate::app::agent_view) fn restore_f7_panel_settings(&mut self) {
        self.apply_f7_panel_settings(false);
    }

    pub(in crate::app::agent_view) fn apply_f7_panel_settings(&mut self, reset_overrides: bool) {
        let stored = self.panel_persistence.effective();
        let mut defaults = DisplayDefaults::default();
        for &kind in MessageKind::settings_kinds() {
            if let Some(form) = stored.message_form(kind) {
                defaults.set(kind, form);
            }
        }
        self.scrollback.replace_display_defaults(defaults, reset_overrides);
        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
            open.apply_disclosure(restored_disclosure(&stored));
        }
    }

    /// The folds F7 starts with when it opens.
    pub(super) fn restored_section_disclosure(&self) -> SectionDisclosure {
        restored_disclosure(&self.panel_persistence.effective())
    }
}
