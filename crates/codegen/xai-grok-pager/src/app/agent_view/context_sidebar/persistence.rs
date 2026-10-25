//! Saving what an F7 gesture changed (section folds, message-display forms) and restoring it.
//!
//! A gesture runs unchanged; the stored view is read before and after, and what differs goes to the
//! shared edit layer (`panel_persistence`). A form the user set is stored as chosen: a kind nobody set
//! renders by its block's own policy, which is not one form, so no stored form can be called redundant.
//! Context switches belong to the shell and are never touched here.

use super::gesture::SidebarCommand;
use super::{AgentView, ContextSidebar, SectionDisclosure, SectionId};
use crate::app::agent_view::panel_persistence::OVERRULED;
use crate::app::app_view::InputOutcome;
use crate::app::panel_settings::{DisclosureState, PanelSettings, Setting, SidebarSection};
use crate::scrollback::block::MessageKind;
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
        let before = self.stored_f7_view();
        let outcome = self.run_sidebar_command(command);
        let after = self.stored_f7_view();
        self.save_f7_changes(&before, &after);
        outcome
    }

    fn save_f7_changes(&mut self, before: &StoredView, after: &StoredView) {
        let forms: Vec<(MessageKind, DisplayForm)> = before
            .forms
            .iter()
            .zip(&after.forms)
            .filter(|(old, new)| old != new)
            .filter_map(|(_, &(kind, form))| form.map(|form| (kind, form)))
            .collect();
        let sections: Vec<(SectionId, bool)> = match (before.disclosure, after.disclosure) {
            (Some(old), Some(new)) => SectionId::iter()
                .filter(|&section| old.is_expanded(section) != new.is_expanded(section))
                .map(|section| (section, new.is_expanded(section)))
                .collect(),
            _ => Vec::new(),
        };
        if forms.is_empty() && sections.is_empty() {
            return;
        }
        let settings: Vec<Setting> = forms
            .iter()
            .map(|&(kind, form)| Setting::KindForm { kind, form })
            .chain(sections.iter().map(|&(section, open)| Setting::Section {
                section: stored_section(section),
                state: DisclosureState::from_expanded(open),
            }))
            .collect();
        if self.save_panel_settings(&settings) {
            self.show_what_is_in_force(&forms, &sections);
        }
    }

    /// A save to the global layer changes nothing while this session holds its own value for the same
    /// setting. Show the value in force and say so, so the screen matches what a restart will bring.
    fn show_what_is_in_force(
        &mut self,
        forms: &[(MessageKind, DisplayForm)],
        sections: &[(SectionId, bool)],
    ) {
        let in_force = self.panel_persistence.effective();
        let mut overruled = false;
        for &(kind, form) in forms {
            if let Some(stored) = in_force.message_form(kind)
                && stored != form
            {
                self.scrollback.set_kind_default(kind, stored);
                overruled = true;
            }
        }
        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
            let mut disclosure = open.disclosure();
            for &(section, open_now) in sections {
                let stored = in_force.section(stored_section(section)).is_expanded();
                if stored != open_now {
                    disclosure.set_expanded(section, stored);
                    overruled = true;
                }
            }
            if disclosure != open.disclosure() {
                open.apply_disclosure(disclosure);
            }
        }
        if overruled {
            self.show_toast(OVERRULED);
        }
    }

    /// Applies the stored message-display forms to the scrollback, and the folds to an open F7.
    /// A kind the stored layers no longer set (another session's override) goes back to its block's own policy.
    pub(in crate::app::agent_view) fn restore_f7_panel_settings(&mut self) {
        let stored = self.panel_persistence.effective();
        for &kind in MessageKind::settings_kinds() {
            match stored.message_form(kind) {
                Some(form) => self.scrollback.set_kind_default(kind, form),
                None => self.scrollback.clear_kind_default(kind),
            }
        }
        if let ContextSidebar::Open(open) = &mut self.context_sidebar {
            open.apply_disclosure(restored_disclosure(&stored));
        }
    }

    /// The folds F7 starts with when it opens.
    pub(super) fn restored_section_disclosure(&self) -> SectionDisclosure {
        restored_disclosure(&self.panel_persistence.effective())
    }
}
