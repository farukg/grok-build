//! Saved F6/F7 settings of one view, and the layer (global or this session) an edit is saved to.
//!
//! Both panels share one [`PanelPersistence`]: a save goes to its edit layer, which the user flips with `g`.
//! The edit layer starts as global and returns to global whenever the view's session changes.

use std::error::Error as _;
use std::path::Path;

use super::AgentView;
use crate::app::panel_settings::{
    Layer, LoadWarning, PanelSettings, PanelSettingsStore, SaveError, SaveOutcome, SessionKey,
    Setting,
};

/// Said when a global edit does not show because this session holds its own value for the setting. It claims no
/// write: a value equal to the built-in default needs no global entry, so nothing may have been written.
pub(super) const OVERRULED: &str = "The change does not show here: this session has its own setting for it; press g to change that one";

/// Why the edit layer cannot be flipped.
#[derive(Debug, thiserror::Error)]
enum EditLayerError {
    #[error("Panel settings are not stored in this view, so there is no layer to choose")]
    Unavailable,
    #[error("This view has no session yet, so edits are saved globally")]
    NoSession,
}

/// The settings store of a view; absent (nothing is saved or restored) until [`AgentView::open_panel_settings_in`].
pub(crate) struct PanelPersistence {
    store: Option<PanelSettingsStore>,
    edit_layer: Layer,
}

impl Default for PanelPersistence {
    fn default() -> Self {
        Self {
            store: None,
            edit_layer: Layer::Global,
        }
    }
}

impl PanelPersistence {
    /// Reads the global layer and, for a valid `session` id, that session's layer. Creates nothing on disk.
    fn open(root: &Path, session: Option<&str>) -> (Self, Vec<LoadWarning>) {
        let report = PanelSettingsStore::open(root, session.and_then(session_key));
        let persistence = Self {
            store: Some(report.store),
            edit_layer: Layer::Global,
        };
        (persistence, report.warnings)
    }

    /// Session override, else global, else built-in default.
    pub(crate) fn effective(&self) -> PanelSettings {
        self.store
            .as_ref()
            .map(PanelSettingsStore::effective)
            .unwrap_or_default()
    }

    /// The layer edits are saved to.
    pub(crate) fn edit_layer(&self) -> Layer {
        self.edit_layer
    }

    /// For edits that are not layered, such as a turn's own exception (session only).
    pub(crate) fn store_mut(&mut self) -> Option<&mut PanelSettingsStore> {
        self.store.as_mut()
    }

    /// Where the footer says edits go.
    pub(crate) fn saving_status(&self) -> &'static str {
        match (&self.store, self.edit_layer) {
            (None, _) => "not saved",
            (Some(_), Layer::Global) => "saving: global (g)",
            (Some(_), Layer::Session) => "saving: this session (g)",
        }
    }

    fn toggle_edit_layer(&mut self) -> Result<Layer, EditLayerError> {
        let Some(store) = &self.store else {
            return Err(EditLayerError::Unavailable);
        };
        let next = match self.edit_layer {
            Layer::Global => Layer::Session,
            Layer::Session => Layer::Global,
        };
        if next == Layer::Session && store.session_key().is_none() {
            return Err(EditLayerError::NoSession);
        }
        self.edit_layer = next;
        Ok(next)
    }

    /// Stores the values in the edit layer; a value the layers beneath already hold is not repeated there.
    /// `None` when this view keeps no settings.
    fn save(&mut self, settings: &[Setting]) -> Option<Result<SaveOutcome, SaveError>> {
        let layer = self.edit_layer;
        self.store
            .as_mut()
            .map(|store| store.set_deviations(layer, settings))
    }

    /// Follows the view to `session`. `None` when the session did not change; else what could not be loaded.
    fn sync_session(&mut self, session: Option<&str>) -> Option<Vec<LoadWarning>> {
        let store = self.store.as_mut()?;
        let key = session.and_then(session_key);
        if store.session_key() == key.as_ref() {
            return None;
        }
        let warnings = store.attach_session(key);
        self.edit_layer = Layer::Global;
        Some(warnings)
    }
}

fn session_key(id: &str) -> Option<SessionKey> {
    match SessionKey::parse(id) {
        Ok(key) => Some(key),
        Err(error) => {
            tracing::warn!(%error, "session settings are not stored for this session id");
            None
        }
    }
}

/// The first problem in the toast (it names the file and the cause), the rest only in the log.
fn load_warning_text(warnings: &[LoadWarning]) -> Option<String> {
    for warning in warnings {
        tracing::warn!(%warning, "panel settings");
    }
    let (first, rest) = warnings.split_first()?;
    let more = match rest.len() {
        0 => String::new(),
        count => format!(" (and {count} more)"),
    };
    Some(format!(
        "Panel settings partly unusable, defaults apply: {first}{more}"
    ))
}

/// The error and the causes below it, so the toast names the file and the operating system's reason.
fn save_failure_text(error: &SaveError) -> String {
    let mut text = format!("Panel settings not saved: {error}");
    let mut cause = error.source();
    while let Some(inner) = cause {
        text = format!("{text}: {inner}");
        cause = inner.source();
    }
    text
}

impl AgentView {
    pub(crate) fn restore_dock_placement(&self) {
        if let Some(placement) = self.panel_persistence.store.as_ref().and_then(|store| store.global().dock_placement())
            && crate::app::dock_requirement_pin().or_else(crate::app::dock_env_override)
                .is_none_or(|pin| pin == placement.is_visible())
        {
            crate::views::dock::set_placement(placement);
        }
    }

    pub(crate) fn set_dock_placement(&mut self, placement: crate::views::dock::DockPlacement) -> bool {
        if crate::app::dock_requirement_pin().is_some_and(|pin| pin != placement.is_visible()) {
            self.show_toast("Dock visibility is pinned by requirements.toml");
            return false;
        }
        if let Some(store) = self.panel_persistence.store_mut() {
            let result = store.set(Layer::Global, Setting::DockPlacement(placement));
            if !self.report_panel_save(result) { return false; }
        }
        crate::views::dock::set_placement(placement);
        true
    }

    /// At view start: opens the store under `root` and applies it.
    pub(crate) fn open_panel_settings_in(&mut self, root: &Path) {
        let session = self.session.session_id.as_ref().map(|id| &*id.0);
        let (persistence, warnings) = PanelPersistence::open(root, session);
        self.panel_persistence = persistence;
        self.restore_panel_settings();
        self.restore_dock_placement();
        if let Some(store) = self.panel_persistence.store_mut() {
            use crate::app::panel_settings::SidebarSection;
            self.tasks.overlay.visible = store
                .global()
                .section(SidebarSection::Tasks)
                .map_or(false, |state| state.is_expanded());
            self.todo.overlay.visible = store
                .global()
                .section(SidebarSection::Todo)
                .map_or(false, |state| state.is_expanded());
        }
        self.toast_load_warnings(&warnings);
    }

    pub(crate) fn save_global_work_visibility(&mut self, tasks: Option<bool>, todo: Option<bool>) {
        use crate::app::panel_settings::{DisclosureState, SidebarSection};
        for (section, visible) in [(SidebarSection::Tasks, tasks), (SidebarSection::Todo, todo)] {
            if let Some(visible) = visible {
                let result = self.panel_persistence.store_mut().map(|store| {
                    store.set(
                        Layer::Global,
                        Setting::Section {
                            section,
                            state: DisclosureState::from_expanded(visible),
                        },
                    )
                });
                if let Some(result) = result {
                    self.report_panel_save(result);
                }
            }
        }
    }

    /// After the view's session id was bound, changed or dropped.
    pub(crate) fn sync_panel_settings(&mut self) {
        let session = self.session.session_id.as_ref().map(|id| &*id.0);
        if let Some(warnings) = self.panel_persistence.sync_session(session) {
            self.restore_panel_settings();
            self.toast_load_warnings(&warnings);
        }
    }

    fn toast_load_warnings(&mut self, warnings: &[LoadWarning]) {
        if let Some(text) = load_warning_text(warnings) {
            self.show_toast(&text);
        }
    }

    /// Flips the edit layer between global and this session; says why when it cannot.
    pub(crate) fn toggle_panel_edit_layer(&mut self) {
        if let Err(refusal) = self.panel_persistence.toggle_edit_layer() {
            self.show_toast(&refusal.to_string());
        }
    }

    /// Saves into the edit layer. A failure, or a damaged file that was replaced, is toasted with its reason.
    /// Returns whether the store now holds the settings (`false` too when this view keeps none).
    pub(crate) fn save_panel_settings(&mut self, settings: &[Setting]) -> bool {
        match self.panel_persistence.save(settings) {
            None => false,
            Some(result) => self.report_panel_save(result),
        }
    }

    /// Toasts what a save did that the user must know; whether the store now holds the values.
    pub(super) fn report_panel_save(&mut self, result: Result<SaveOutcome, SaveError>) -> bool {
        match result {
            Ok(SaveOutcome::Written | SaveOutcome::Unchanged) => true,
            Ok(SaveOutcome::Repaired { backup }) => {
                self.show_toast(&format!(
                    "A damaged panel settings file was replaced; the old one is kept as {}",
                    backup.display()
                ));
                true
            }
            Err(error) => {
                self.show_toast(&save_failure_text(&error));
                false
            }
        }
    }

    /// Applies the stored settings of every panel to the live view.
    fn restore_panel_settings(&mut self) {
        self.restore_f7_panel_settings();
        self.restore_f6_panel_settings();
    }
}
