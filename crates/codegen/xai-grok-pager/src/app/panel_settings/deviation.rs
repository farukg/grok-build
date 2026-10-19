//! Storing a wanted value as the smallest correct edit against the layers beneath a layer.

use super::error::{SaveError, SaveOutcome};
use super::store::PanelSettingsStore;
use super::types::{Edit, Layer, LayerSettings, PanelSettings, Setting, SettingKey};

impl PanelSettingsStore {
    /// Makes `layer` yield `wanted`, in one write. A value the layers beneath already hold removes the layer's own
    /// entry; any other value is stored.
    ///
    /// A message-display form has no built-in baseline to compare with: a kind nobody set renders by its block's own
    /// policy (and, for edit blocks, by a user configuration), which is not one [`DisplayForm`]. So a form is
    /// "held beneath" only when the global layer stores that same form for the kind; in the global layer it is
    /// always stored. Section and outline folds have a fixed built-in default and compare against it.
    ///
    /// [`DisplayForm`]: crate::scrollback::types::DisplayForm
    pub fn set_deviations(
        &mut self,
        layer: Layer,
        wanted: &[Setting],
    ) -> Result<SaveOutcome, SaveError> {
        let nothing = LayerSettings::default();
        let beneath = match layer {
            Layer::Global => &nothing,
            Layer::Session => self.global(),
        };
        let edits: Vec<Edit> = wanted
            .iter()
            .map(|&setting| {
                if held_beneath(beneath, setting) {
                    Edit::Clear(key_of(setting))
                } else {
                    Edit::Set(setting)
                }
            })
            .collect();
        self.save(layer, &edits)
    }
}

/// Whether the layers beneath already produce `setting`'s value.
fn held_beneath(beneath: &LayerSettings, setting: Setting) -> bool {
    let defaults = PanelSettings::default();
    match setting {
        Setting::KindForm { kind, form } => beneath.message_form(kind) == Some(form),
        Setting::Section { section, state } => {
            beneath
                .section(section)
                .unwrap_or_else(|| defaults.section(section))
                == state
        }
        Setting::OutlineTurns(state) => {
            beneath
                .outline_turns()
                .unwrap_or_else(|| defaults.outline_turns())
                == state
        }
    }
}

fn key_of(setting: Setting) -> SettingKey {
    match setting {
        Setting::KindForm { kind, .. } => SettingKey::KindForm(kind),
        Setting::Section { section, .. } => SettingKey::Section(section),
        Setting::OutlineTurns(_) => SettingKey::OutlineTurns,
    }
}
