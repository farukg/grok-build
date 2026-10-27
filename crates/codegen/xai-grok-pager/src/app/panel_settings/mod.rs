//! Persistent F6/F7 panel settings in two layers: global, and per session.
//!
//! Files under a caller-supplied root (`$GROK_HOME/panel-settings` in the app):
//! `global.toml` and `sessions/<session id>.toml`. The effective value of a setting is the session override,
//! else the global value, else the built-in default. Selecting the session layer stores nothing; an override
//! exists only after an explicit session-layer edit.
//!
//! Saving is a read-modify-write against the file as it is on disk at that moment, so several pager and leader
//! processes can edit different keys of one file and each keeps the others' changes. Two edits in the same instant
//! can still lose one (no file lock: the window is the read-to-rename gap of a user-triggered edit, and the
//! rename keeps the file valid). Files are created owner-only.

mod codec;
mod deviation;
mod error;
mod layer_file;
mod store;
mod turn_deviation;
mod types;

pub use error::{LoadProblem, LoadWarning, SaveError, SaveOutcome};
pub use store::{LayerReload, LoadReport, PanelSettingsStore};
pub use types::{
    CompletedItemsPane, DisclosureState, Layer, LayerSettings, PanelSettings, PromptOrdinal, SessionKey,
    SessionKeyError, Setting, SettingKey, SidebarSection,
};
