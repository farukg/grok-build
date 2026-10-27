//! TOML mapping of [`LayerSettings`]: names on disk, parsing at the file boundary, and in-place edits.
//!
//! Decoding reads a document into typed values and reports what it ignored. Encoding patches the parsed document,
//! so keys this version does not know survive a read-modify-write.

use toml_edit::{DocumentMut, Item, Table, TableLike};

use super::error::LoadProblem;
use super::types::{
    CompletedItemsPane, DisclosureState, Edit, Layer, LayerSettings, PromptOrdinal, Setting, SettingKey, SidebarSection,
};
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;

pub(super) const FORMAT_VERSION: i64 = 1;

const VERSION: &str = "version";
const MESSAGE_DISPLAY: &str = "message_display";
const SIDEBAR: &str = "sidebar";
const OUTLINE: &str = "outline";
const OUTLINE_TURNS: &str = "turns";
const TURN_EXCEPTIONS: &str = "turn_exceptions";

/// The stable on-disk name of a storable kind; `None` for kinds the settings never offer.
pub(super) fn kind_name(kind: MessageKind) -> Option<&'static str> {
    match kind {
        MessageKind::UserPrompt => Some("user_prompt"),
        MessageKind::AgentMessage => Some("agent_message"),
        MessageKind::Execute => Some("execute"),
        MessageKind::Read => Some("read"),
        MessageKind::Edit => Some("edit"),
        MessageKind::ListDir => Some("list_dir"),
        MessageKind::Search => Some("search"),
        MessageKind::WebFetch => Some("web_fetch"),
        MessageKind::WebSearch => Some("web_search"),
        MessageKind::IntegrationSearch => Some("integration_search"),
        MessageKind::UseTool => Some("use_tool"),
        MessageKind::MemorySearch => Some("memory_search"),
        MessageKind::SentMessage => Some("sent_message"),
        MessageKind::Skill => Some("skill"),
        MessageKind::OtherTool => Some("other_tool"),
        MessageKind::Thinking => Some("thinking"),
        MessageKind::System => Some("system"),
        MessageKind::SessionEvent => Some("session_event"),
        MessageKind::BgTask => Some("bg_task"),
        MessageKind::Subagent => Some("subagent"),
        MessageKind::Workflow => Some("workflow"),
        MessageKind::Btw => Some("btw"),
        MessageKind::ContextInfo => Some("context_info"),
        MessageKind::MemoryCapture => Some("memory_capture"),
        MessageKind::Injected => Some("injected"),
        MessageKind::InjectedOther => Some("injected_other"),
        MessageKind::InjectedInstructions => Some("injected_instructions"),
        MessageKind::InjectedContinuation => Some("injected_continuation"),
        MessageKind::InjectedNotification => Some("injected_notification"),
        MessageKind::InjectedMemory => Some("injected_memory"),
        MessageKind::InjectedSummary => Some("injected_summary"),
        MessageKind::InjectedEnvironment => Some("injected_environment"),
        MessageKind::InjectedConversation => Some("injected_conversation"),

        MessageKind::Stub => None,
    }
}

fn kind_from_name(name: &str) -> Option<MessageKind> {
    MessageKind::settings_kinds()
        .iter()
        .copied()
        .find(|kind| kind_name(*kind) == Some(name))
}

fn form_name(form: DisplayForm) -> &'static str {
    match form {
        DisplayForm::Collapsed => "collapsed",
        DisplayForm::Expanded => "expanded",
    }
}

fn form_from_name(name: &str) -> Option<DisplayForm> {
    match name {
        "collapsed" => Some(DisplayForm::Collapsed),
        "expanded" => Some(DisplayForm::Expanded),
        _ => None,
    }
}

fn state_name(state: DisclosureState) -> &'static str {
    state.into()
}

fn section_name(section: SidebarSection) -> &'static str {
    section.into()
}

fn disclosure_of(item: &Item) -> Option<DisclosureState> {
    item.as_str().and_then(|text| text.parse().ok())
}

pub(super) struct Decoded {
    pub settings: LayerSettings,
    pub problems: Vec<LoadProblem>,
}

impl Decoded {
    pub(super) fn has_invalid_known(&self) -> bool {
        self.invalid_known_key().is_some()
    }

    pub(super) fn invalid_known_key(&self) -> Option<&str> {
        self.problems.iter().find_map(|problem| match problem {
            LoadProblem::InvalidKnown { key } => Some(key.as_str()),
            LoadProblem::Unreadable { .. } | LoadProblem::Malformed(_)
            | LoadProblem::NewerVersion { .. } | LoadProblem::IgnoredEntry { .. } => None,
        })
    }
}

#[derive(Clone, Copy)]
enum EntryLocation {
    Active,
    Legacy,
}

impl EntryLocation {
    fn for_layer(layer: Layer, stored: Layer) -> Self {
        if layer == stored { Self::Active } else { Self::Legacy }
    }
}

/// Reads every recognized value of `doc`; anything else is reported and left alone.
pub(super) fn decode(layer: Layer, doc: &DocumentMut) -> Decoded {
    let mut decoded = Decoded {
        settings: LayerSettings::default(),
        problems: Vec::new(),
    };
    decode_version(doc, &mut decoded);
    for (name, item) in entries(doc, &["dock"], &mut decoded.problems, EntryLocation::for_layer(layer, Layer::Global)) {
        match (name, item.as_str().and_then(|value| value.parse::<crate::views::dock::DockPlacement>().ok()), layer) {
            ("placement", Some(placement), Layer::Global) => { decoded.settings.apply(&Edit::Set(Setting::DockPlacement(placement))); }
            ("placement", None, Layer::Global) => decoded.problems.push(LoadProblem::InvalidKnown { key: "dock.placement".to_owned() }),
            _ => decoded.problems.push(LoadProblem::IgnoredEntry { key: format!("dock.{name}") }),
        }
    }
    for (name, item) in entries(doc, &["scope"], &mut decoded.problems, EntryLocation::for_layer(layer, Layer::Session)) {
        match (name, layer, item.as_str().and_then(|value| value.parse::<Layer>().ok())) {
            ("message_display", Layer::Session, Some(scope)) => {
                decoded.settings.apply(&Edit::Set(Setting::DisplayScope(scope)));
            }
            ("message_display", Layer::Session, None) => decoded.problems.push(LoadProblem::InvalidKnown { key: "scope.message_display".to_owned() }),
            _ => decoded.problems.push(LoadProblem::IgnoredEntry { key: format!("scope.{name}") }),
        }
    }
    for (name, item) in entries(doc, &["completed_items"], &mut decoded.problems, EntryLocation::for_layer(layer, Layer::Global)) {
        let show = match item.as_str() {
            Some("show") => Some(true),
            Some("hide") => Some(false),
            _ => None,
        };
        match (layer, name.parse::<CompletedItemsPane>().ok(), show) {
            (Layer::Global, Some(pane), Some(show)) => {
                decoded.settings.apply(&Edit::Set(Setting::ShowCompleted { pane, show }));
            }
            (Layer::Global, Some(_), None) => decoded.problems.push(LoadProblem::InvalidKnown { key: format!("completed_items.{name}") }),
            _ => decoded.problems.push(LoadProblem::IgnoredEntry { key: format!("completed_items.{name}") }),
        }
    }
    for (name, item) in entries(doc, &[MESSAGE_DISPLAY], &mut decoded.problems, EntryLocation::Active) {
        let key = format!("{MESSAGE_DISPLAY}.{name}");
        match (kind_from_name(name), item.as_str().and_then(form_from_name)) {
            (Some(kind), Some(form)) => {
                decoded
                    .settings
                    .apply(&Edit::Set(Setting::KindForm { kind, form }));
            }
            (None, _) => decoded.problems.push(LoadProblem::IgnoredEntry { key }),
            (Some(_), None) => decoded.problems.push(LoadProblem::InvalidKnown { key }),
        }
    }
    for (name, item) in entries(doc, &[SIDEBAR], &mut decoded.problems, EntryLocation::Active) {
        let key = format!("{SIDEBAR}.{name}");
        match (name.parse::<SidebarSection>().ok(), disclosure_of(item)) {
            (Some(section), Some(state)) => {
                if section.storage_layer(layer) != layer {
                    decoded.problems.push(LoadProblem::IgnoredEntry { key });
                } else {
                    decoded
                        .settings
                        .apply(&Edit::Set(Setting::Section { section, state }));
                }
            }
            (None, _) => decoded.problems.push(LoadProblem::IgnoredEntry { key }),
            (Some(section), None) => decoded.problems.push(if section.storage_layer(layer) == layer {
                LoadProblem::InvalidKnown { key }
            } else { LoadProblem::IgnoredEntry { key } }),
        }
    }
    for (name, item) in entries(doc, &[OUTLINE], &mut decoded.problems, EntryLocation::Active) {
        match (name, disclosure_of(item)) {
            (OUTLINE_TURNS, Some(state)) => {
                decoded
                    .settings
                    .apply(&Edit::Set(Setting::OutlineTurns(state)));
            }
            (OUTLINE_TURNS, None) => decoded.problems.push(LoadProblem::InvalidKnown { key: format!("{OUTLINE}.{OUTLINE_TURNS}") }),
            (TURN_EXCEPTIONS, _) => {}
            _ => decoded.problems.push(LoadProblem::IgnoredEntry {
                key: format!("{OUTLINE}.{name}"),
            }),
        }
    }
    match layer {
        Layer::Global => {}
        Layer::Session => decode_turn_exceptions(doc, &mut decoded),
    }
    decoded
}

fn decode_version(doc: &DocumentMut, decoded: &mut Decoded) {
    let Some(item) = doc.get(VERSION) else {
        return;
    };
    match item.as_integer() {
        Some(found) if found > FORMAT_VERSION => {
            decoded.problems.push(LoadProblem::NewerVersion { found });
        }
        Some(found) if found >= 1 => {}
        Some(_) | None => decoded.problems.push(LoadProblem::InvalidKnown {
            key: VERSION.to_owned(),
        }),
    }
}

fn decode_turn_exceptions(doc: &DocumentMut, decoded: &mut Decoded) {
    for (name, item) in entries(doc, &[OUTLINE, TURN_EXCEPTIONS], &mut decoded.problems, EntryLocation::Active) {
        match (name.parse::<u32>().ok(), disclosure_of(item)) {
            (Some(prompt), Some(state)) => {
                let prompt = PromptOrdinal::from_stored(prompt);
                decoded.settings.apply(&Edit::Turn(prompt, Some(state)));
            }
            (None, _) => decoded.problems.push(LoadProblem::IgnoredEntry {
                key: format!("{OUTLINE}.{TURN_EXCEPTIONS}.{name}"),
            }),
            (Some(_), None) => decoded.problems.push(LoadProblem::InvalidKnown {
                key: format!("{OUTLINE}.{TURN_EXCEPTIONS}.{name}"),
            }),
        }
    }
}

/// The key/value pairs of the table at `path`; a missing table is empty, a non-table value is reported.
fn entries<'a>(
    doc: &'a DocumentMut,
    path: &[&str],
    problems: &mut Vec<LoadProblem>,
    location: EntryLocation,
) -> Vec<(&'a str, &'a Item)> {
    let mut current: &dyn TableLike = doc.as_table();
    let mut dotted = String::new();
    for key in path {
        if !dotted.is_empty() {
            dotted.push('.');
        }
        dotted.push_str(key);
        let Some(item) = current.get(key) else {
            return Vec::new();
        };
        match item.as_table_like() {
            Some(next) => current = next,
            None => {
                problems.push(match location {
                    EntryLocation::Active => LoadProblem::InvalidKnown { key: dotted },
                    EntryLocation::Legacy => LoadProblem::IgnoredEntry { key: dotted },
                });
                return Vec::new();
            }
        }
    }
    current.iter().collect()
}

/// Applies `edit` to `doc`, keeping all unrelated content.
pub(super) fn encode(doc: &mut DocumentMut, edit: &Edit) {
    let root: &mut dyn TableLike = doc.as_table_mut();
    match edit {
        Edit::Set(Setting::DisplayScope(scope)) => put(root, &["scope"], "message_display", (*scope).into()),
        Edit::Clear(SettingKey::DisplayScope) => remove(root, &["scope"], "message_display"),
        Edit::Set(Setting::ShowCompleted { pane, show }) => {
            put(root, &["completed_items"], (*pane).into(), if *show { "show" } else { "hide" });
        }
        Edit::Clear(SettingKey::ShowCompleted(pane)) => {
            remove(root, &["completed_items"], (*pane).into());
        }
        Edit::Set(Setting::DockPlacement(placement)) => put(root, &["dock"], "placement", (*placement).into()),
        Edit::Clear(SettingKey::DockPlacement) => remove(root, &["dock"], "placement"),
        Edit::Set(Setting::KindForm { kind, form }) => {
            if let Some(name) = kind_name(*kind) {
                put(root, &[MESSAGE_DISPLAY], name, form_name(*form));
            }
        }
        Edit::Set(Setting::Section { section, state }) => {
            put(root, &[SIDEBAR], section_name(*section), state_name(*state));
        }
        Edit::Set(Setting::OutlineTurns(state)) => {
            put(root, &[OUTLINE], OUTLINE_TURNS, state_name(*state));
        }
        Edit::Clear(SettingKey::KindForm(kind)) => {
            if let Some(name) = kind_name(*kind) {
                remove(root, &[MESSAGE_DISPLAY], name);
            }
        }
        Edit::Clear(SettingKey::Section(section)) => {
            remove(root, &[SIDEBAR], section_name(*section));
        }
        Edit::Clear(SettingKey::OutlineTurns) => remove(root, &[OUTLINE], OUTLINE_TURNS),
        Edit::Turn(prompt, Some(state)) => put(
            root,
            &[OUTLINE, TURN_EXCEPTIONS],
            &prompt.get().to_string(),
            state_name(*state),
        ),
        Edit::Turn(prompt, None) => {
            remove(root, &[OUTLINE, TURN_EXCEPTIONS], &prompt.get().to_string())
        }
    }
}

/// Adds the format version to a document that has none; an existing one, including a newer one, stays.
pub(super) fn stamp_version(doc: &mut DocumentMut) {
    if !doc.contains_key(VERSION) {
        doc.insert(VERSION, toml_edit::value(FORMAT_VERSION));
    }
}

fn new_table() -> Item {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
}

fn put(table: &mut dyn TableLike, path: &[&str], key: &str, text: &str) {
    match path.split_first() {
        None => {
            table.insert(key, toml_edit::value(text));
        }
        Some((head, rest)) => {
            let slot = table.entry(head).or_insert_with(new_table);
            if !slot.is_table_like() {
                *slot = new_table();
            }
            if let Some(child) = slot.as_table_like_mut() {
                put(child, rest, key, text);
            }
        }
    }
}

fn remove(table: &mut dyn TableLike, path: &[&str], key: &str) {
    match path.split_first() {
        None => {
            table.remove(key);
        }
        Some((head, rest)) => {
            let Some(child) = table
                .get_mut(head)
                .and_then(|item| item.as_table_like_mut())
            else {
                return;
            };
            remove(child, rest, key);
            let emptied = child.is_empty();
            if emptied {
                table.remove(head);
            }
        }
    }
}
