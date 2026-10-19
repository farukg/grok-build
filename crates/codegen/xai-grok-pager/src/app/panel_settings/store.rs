//! The two-layer store: global settings plus the attached session's overrides.

use std::path::{Path, PathBuf};

use super::error::{LoadWarning, SaveError, SaveOutcome};
use super::layer_file::LayerFile;
use super::types::{Edit, Layer, LayerSettings, PanelSettings, SessionKey, Setting, SettingKey};

/// Below the grok home.
const ROOT_DIR: &str = "panel-settings";
const GLOBAL_FILE: &str = "global.toml";
const SESSIONS_DIR: &str = "sessions";
const SESSION_EXTENSION: &str = "toml";

/// Global settings and, once a session is attached, that session's overrides.
///
/// Every edit is written through to its layer file before the call returns. Loading never fails: an unusable
/// file yields an empty layer plus a [`LoadWarning`], and the file is left as it was until the next edit of that layer.
pub struct PanelSettingsStore {
    root: PathBuf,
    global: LayerFile,
    session: Option<AttachedSession>,
}

struct AttachedSession {
    key: SessionKey,
    file: LayerFile,
}

/// A freshly opened store and what could not be used while loading it.
pub struct LoadReport {
    pub store: PanelSettingsStore,
    pub warnings: Vec<LoadWarning>,
}

impl PanelSettingsStore {
    /// Where the app keeps the store.
    pub fn default_root() -> PathBuf {
        xai_grok_config::grok_home().join(ROOT_DIR)
    }

    /// Opens the store under `root` (files: `global.toml`, `sessions/<session id>.toml`).
    /// Reads only; creates no file or directory.
    pub fn open(root: &Path, session: Option<SessionKey>) -> LoadReport {
        let (global, mut warnings) = LayerFile::load(Layer::Global, root.join(GLOBAL_FILE));
        let mut store = Self {
            root: root.to_path_buf(),
            global,
            session: None,
        };
        warnings.extend(store.attach_session(session));
        LoadReport { store, warnings }
    }

    /// Switches to `session`'s overrides, or to none. Use it when the session id becomes known after startup.
    pub fn attach_session(&mut self, session: Option<SessionKey>) -> Vec<LoadWarning> {
        let Some(key) = session else {
            self.session = None;
            return Vec::new();
        };
        let (file, warnings) = LayerFile::load(Layer::Session, self.session_path(&key));
        self.session = Some(AttachedSession { key, file });
        warnings
    }

    pub fn session_key(&self) -> Option<&SessionKey> {
        self.session.as_ref().map(|session| &session.key)
    }

    /// Session override, else global, else the built-in default.
    pub fn effective(&self) -> PanelSettings {
        let merged = match &self.session {
            Some(session) => self
                .global
                .settings()
                .overlaid_with(session.file.settings()),
            None => self.global.settings().clone(),
        };
        PanelSettings::new(merged)
    }

    pub fn global(&self) -> &LayerSettings {
        self.global.settings()
    }

    /// `None` while no session is attached. An attached session without edits holds an empty layer.
    pub fn session(&self) -> Option<&LayerSettings> {
        self.session.as_ref().map(|session| session.file.settings())
    }

    /// Stores `setting` in `layer`. Selecting the session layer stores nothing; only this call creates an override.
    pub fn set(&mut self, layer: Layer, setting: Setting) -> Result<SaveOutcome, SaveError> {
        self.save(layer, &[Edit::Set(setting)])
    }

    /// Removes `key` from `layer`; in the session layer the global value shows through again.
    pub fn clear(&mut self, layer: Layer, key: SettingKey) -> Result<SaveOutcome, SaveError> {
        self.save(layer, &[Edit::Clear(key)])
    }

    pub(super) fn save(&mut self, layer: Layer, edits: &[Edit]) -> Result<SaveOutcome, SaveError> {
        match layer {
            Layer::Global => self.global.save(edits),
            Layer::Session => match &mut self.session {
                Some(session) => session.file.save(edits),
                None => Err(SaveError::NoSession),
            },
        }
    }

    fn session_path(&self, key: &SessionKey) -> PathBuf {
        self.root
            .join(SESSIONS_DIR)
            .join(key.as_str())
            .with_extension(SESSION_EXTENSION)
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
