//! Value types of the persisted F6/F7 panel settings and their two-layer resolution.

use std::collections::{BTreeMap, HashMap};

use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;

/// Which stored layer an edit targets. Global is the default edit layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum Layer {
    Global,
    Session,
}

/// Whether a tree node shows its children. Independent of any message [`DisplayForm`].
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, strum::EnumString, strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum DisclosureState {
    Collapsed,
    #[default]
    Expanded,
}

impl DisclosureState {
    pub const fn from_expanded(expanded: bool) -> Self {
        if expanded {
            Self::Expanded
        } else {
            Self::Collapsed
        }
    }

    pub const fn is_expanded(self) -> bool {
        matches!(self, Self::Expanded)
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Expanded => Self::Collapsed,
            Self::Collapsed => Self::Expanded,
        }
    }
}

/// Panel disclosure preferences. Tasks and Todo control only the global upper projection.
/// The snake_case names are the on-disk keys.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum SidebarSection {
    Context,
    MessageDisplay,
    Tasks,
    Todo,
    Dock,
    TodoSection,
    ContextInstructions,
    ContextConversation,
    ContextContinuation,
    ContextNotifications,
    ContextRetained,
    DisplayConversation,
    DisplayFiles,
    DisplayResearch,
    DisplayWork,
    DisplaySession,
    DisplayInjected,
}

impl SidebarSection {
    pub(super) const fn storage_layer(self, _requested: Layer) -> Layer {
        Layer::Global
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum CompletedItemsPane {
    Tasks,
    Todos,
}

impl CompletedItemsPane {
    pub const fn default_visibility(self) -> bool {
        match self {
            Self::Tasks => false,
            Self::Todos => true,
        }
    }
}

/// The conversation's prompt ordinal (`UserPromptBlock::prompt_index`): stable across restarts and replay,
/// unlike entry ids and scrollback positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptOrdinal(u32);

impl PromptOrdinal {
    pub fn from_prompt_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self)
    }

    pub fn get(self) -> u32 {
        self.0
    }

    pub(super) fn from_stored(value: u32) -> Self {
        Self(value)
    }
}

/// A session id that is safe to use as a file name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionKeyError {
    #[error("session id is empty")]
    Empty,
    #[error("session id is longer than {} bytes", SessionKey::MAX_LEN)]
    TooLong,
    #[error("session id contains {0:?}; only ASCII letters, digits, '-' and '_' are allowed")]
    InvalidChar(char),
}

impl SessionKey {
    const MAX_LEN: usize = 128;

    pub fn parse(id: &str) -> Result<Self, SessionKeyError> {
        if id.is_empty() {
            return Err(SessionKeyError::Empty);
        }
        if id.len() > Self::MAX_LEN {
            return Err(SessionKeyError::TooLong);
        }
        match id
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(*c, '-' | '_')))
        {
            Some(c) => Err(SessionKeyError::InvalidChar(c)),
            None => Ok(Self(id.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One layered value to store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// The form blocks of `kind` start in. Kinds outside [`MessageKind::settings_kinds`] are ignored.
    KindForm {
        kind: MessageKind,
        form: DisplayForm,
    },
    Section {
        section: SidebarSection,
        state: DisclosureState,
    },
    /// Default state of F6 turn rows.
    OutlineTurns(DisclosureState),
    DockPlacement(crate::views::dock::DockPlacement),
    ShowCompleted { pane: CompletedItemsPane, show: bool },
    DisplayScope(Layer),
}

/// Names one layered value without its content, for clearing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    KindForm(MessageKind),
    Section(SidebarSection),
    OutlineTurns,
    DockPlacement,
    ShowCompleted(CompletedItemsPane),
    DisplayScope,
}

/// Everything a layer file can change. Per-turn exceptions exist only in the session layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Edit {
    Set(Setting),
    Clear(SettingKey),
    /// `None` removes the turn's exception.
    Turn(PromptOrdinal, Option<DisclosureState>),
}

/// The values one layer holds. Absence means "not set here"; the next layer or the built-in default decides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerSettings {
    message_forms: HashMap<MessageKind, DisplayForm>,
    sections: BTreeMap<SidebarSection, DisclosureState>,
    outline_turns: Option<DisclosureState>,
    turn_exceptions: BTreeMap<PromptOrdinal, DisclosureState>,
    dock_placement: Option<crate::views::dock::DockPlacement>,
    completed_items: BTreeMap<CompletedItemsPane, bool>,
    display_scope: Option<Layer>,
}

impl LayerSettings {
    pub fn session_display_scope(&self) -> Layer {
        self.display_scope.unwrap_or_else(|| {
            if self.message_forms.is_empty() { Layer::Global } else { Layer::Session }
        })
    }

    pub fn show_completed(&self, pane: CompletedItemsPane) -> bool {
        self.completed_items.get(&pane).copied().unwrap_or_else(|| pane.default_visibility())
    }

    pub fn dock_placement(&self) -> Option<crate::views::dock::DockPlacement> { self.dock_placement }

    pub fn message_form(&self, kind: MessageKind) -> Option<DisplayForm> {
        self.message_forms.get(&kind).copied()
            .or_else(|| kind.parent().and_then(|parent| self.message_form(parent)))
    }

    pub fn section(&self, section: SidebarSection) -> Option<DisclosureState> {
        self.sections.get(&section).copied()
    }

    pub fn outline_turns(&self) -> Option<DisclosureState> {
        self.outline_turns
    }

    pub fn turn(&self, prompt: PromptOrdinal) -> Option<DisclosureState> {
        self.turn_exceptions.get(&prompt).copied()
    }

    /// Prompt numbers from `from` upwards that hold a turn exception.
    pub(super) fn turns_from(&self, from: PromptOrdinal) -> Vec<PromptOrdinal> {
        self.turn_exceptions
            .range(from..)
            .map(|(&prompt, _)| prompt)
            .collect()
    }

    /// Applies `edit`; `false` when it changed nothing.
    pub(super) fn apply(&mut self, edit: &Edit) -> bool {
        match *edit {
            Edit::Set(Setting::DisplayScope(scope)) => self.display_scope.replace(scope) != Some(scope),
            Edit::Clear(SettingKey::DisplayScope) => self.display_scope.take().is_some(),
            Edit::Set(Setting::ShowCompleted { pane, show }) => {
                self.completed_items.insert(pane, show) != Some(show)
            }
            Edit::Clear(SettingKey::ShowCompleted(pane)) => self.completed_items.remove(&pane).is_some(),
            Edit::Set(Setting::KindForm { kind, form }) => {
                MessageKind::settings_kinds().contains(&kind)
                    && self.message_forms.insert(kind, form) != Some(form)
            }
            Edit::Set(Setting::Section { section, state }) => {
                self.sections.insert(section, state) != Some(state)
            }
            Edit::Set(Setting::DockPlacement(placement)) => self.dock_placement.replace(placement) != Some(placement),
            Edit::Clear(SettingKey::DockPlacement) => self.dock_placement.take().is_some(),
            Edit::Set(Setting::OutlineTurns(state)) => {
                self.outline_turns.replace(state) != Some(state)
            }
            Edit::Clear(SettingKey::KindForm(kind)) => self.message_forms.remove(&kind).is_some(),
            Edit::Clear(SettingKey::Section(section)) => self.sections.remove(&section).is_some(),
            Edit::Clear(SettingKey::OutlineTurns) => self.outline_turns.take().is_some(),
            Edit::Turn(prompt, Some(state)) => {
                self.turn_exceptions.insert(prompt, state) != Some(state)
            }
            Edit::Turn(prompt, None) => self.turn_exceptions.remove(&prompt).is_some(),
        }
    }

    /// `self` with every value `top` holds replacing its own.
    pub(super) fn overlaid_with(&self, top: &Self) -> Self {
        let mut merged = self.clone();
        let display_scope = top.session_display_scope();
        merged.display_scope = Some(display_scope);
        match display_scope {
            Layer::Global => {}
            Layer::Session => {
                for &kind in MessageKind::settings_kinds() {
                    if let Some(form) = top.message_form(kind).or_else(|| self.message_form(kind)) {
                        merged.message_forms.insert(kind, form);
                    }
                }
            }
        }
        merged.outline_turns = top.outline_turns.or(self.outline_turns);
        merged.turn_exceptions.extend(&top.turn_exceptions);
        merged.dock_placement = top.dock_placement.or(self.dock_placement);
        merged
    }
}

/// The settings in force: session override, else global, else the built-in default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSettings(LayerSettings);

impl Default for PanelSettings {
    fn default() -> Self { Self::new(LayerSettings::default()) }
}

impl PanelSettings {
    pub fn display_scope(&self) -> Layer {
        self.0.display_scope.unwrap_or(Layer::Global)
    }

    pub(super) fn new(mut merged: LayerSettings) -> Self {
        merged.display_scope = Some(merged.display_scope.unwrap_or(Layer::Global));
        Self(merged)
    }

    /// The user's form for `kind`; `None` keeps the block's own display policy.
    pub fn message_form(&self, kind: MessageKind) -> Option<DisplayForm> {
        self.0.message_form(kind)
    }

    pub fn section(&self, section: SidebarSection) -> DisclosureState {
        self.0.section(section).unwrap_or_default()
    }

    pub fn outline_turns(&self) -> DisclosureState {
        self.0.outline_turns().unwrap_or_default()
    }

    /// A turn's own exception, else the turn default.
    pub fn turn(&self, prompt: PromptOrdinal) -> DisclosureState {
        self.0.turn(prompt).unwrap_or_else(|| self.outline_turns())
    }
}
