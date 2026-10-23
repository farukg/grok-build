use std::collections::HashSet;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::super::codec::kind_name;
use super::super::{
    DisclosureState, Layer, LayerSettings, LoadProblem, LoadReport, PanelSettings,
    PanelSettingsStore, PromptOrdinal, SaveError, SaveOutcome, SessionKey, Setting, SettingKey,
    SidebarSection,
};
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;

fn key(id: &str) -> SessionKey {
    SessionKey::parse(id).expect("valid session id")
}

fn open(root: &Path, session: Option<&str>) -> LoadReport {
    PanelSettingsStore::open(root, session.map(key))
}

fn kind_form(kind: MessageKind, form: DisplayForm) -> Setting {
    Setting::KindForm { kind, form }
}

fn prompt(index: usize) -> PromptOrdinal {
    PromptOrdinal::from_prompt_index(index).expect("small index")
}

fn read_toml(path: &Path) -> toml::Value {
    let text = std::fs::read_to_string(path).expect("file exists");
    toml::from_str(&text).expect("file is valid TOML")
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir exists")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn work_visibility_is_global_even_from_session_edits_and_preserves_other_writers() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = open(dir.path(), Some("first")).store;
    let mut second = open(dir.path(), Some("second")).store;
    first.set(Layer::Session, Setting::Section {
        section: SidebarSection::Tasks, state: DisclosureState::Expanded,
    }).unwrap();
    second.set(Layer::Session, Setting::Section {
        section: SidebarSection::Todo, state: DisclosureState::Expanded,
    }).unwrap();
    let restarted = open(dir.path(), Some("first")).store;
    assert_eq!(restarted.global().section(SidebarSection::Tasks), Some(DisclosureState::Expanded));
    assert_eq!(restarted.global().section(SidebarSection::Todo), Some(DisclosureState::Expanded));
    assert!(restarted.session().unwrap().section(SidebarSection::Tasks).is_none());
    assert!(restarted.session().unwrap().section(SidebarSection::Todo).is_none());
    assert_eq!(read_toml(&dir.path().join("global.toml"))["sidebar"]["tasks"].as_str(), Some("expanded"));
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    std::fs::write(dir.path().join("sessions/first.toml"), "version = 1\n[sidebar]\ntasks = 'collapsed'\ntodo = 'collapsed'\n").unwrap();
    let report = open(dir.path(), Some("first"));
    assert_eq!(report.warnings.len(), 2);
    assert!(report.store.effective().section(SidebarSection::Tasks).is_expanded());
    assert!(report.store.effective().section(SidebarSection::Todo).is_expanded());
}

#[test]
fn global_settings_survive_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    store
        .set(
            Layer::Global,
            kind_form(MessageKind::Execute, DisplayForm::Collapsed),
        )
        .expect("save");
    store
        .set(
            Layer::Global,
            kind_form(MessageKind::AgentMessage, DisplayForm::Expanded),
        )
        .expect("save");
    store
        .set(
            Layer::Global,
            Setting::Section {
                section: SidebarSection::MessageDisplay,
                state: DisclosureState::Collapsed,
            },
        )
        .expect("save");
    store
        .set(
            Layer::Global,
            Setting::OutlineTurns(DisclosureState::Collapsed),
        )
        .expect("save");
    drop(store);

    let report = open(dir.path(), None);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let settings = report.store.effective();
    assert_eq!(
        settings.message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(
        settings.message_form(MessageKind::AgentMessage),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(settings.message_form(MessageKind::Read), None);
    assert_eq!(
        settings.section(SidebarSection::MessageDisplay),
        DisclosureState::Collapsed
    );
    assert_eq!(
        settings.section(SidebarSection::Context),
        DisclosureState::Expanded
    );
    assert_eq!(settings.outline_turns(), DisclosureState::Collapsed);
}

#[test]
fn global_file_holds_the_documented_keys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    store
        .set(
            Layer::Global,
            kind_form(MessageKind::WebFetch, DisplayForm::Collapsed),
        )
        .expect("save");
    store
        .set(
            Layer::Global,
            Setting::Section {
                section: SidebarSection::Context,
                state: DisclosureState::Collapsed,
            },
        )
        .expect("save");
    store
        .set(
            Layer::Global,
            Setting::OutlineTurns(DisclosureState::Collapsed),
        )
        .expect("save");

    let doc = read_toml(&dir.path().join("global.toml"));
    let text = |table: &str, name: &str| {
        doc.get(table)
            .and_then(|t| t.get(name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    assert_eq!(doc.get("version").and_then(|v| v.as_integer()), Some(1));
    assert_eq!(
        text("message_display", "web_fetch").as_deref(),
        Some("collapsed")
    );
    assert_eq!(text("sidebar", "context").as_deref(), Some("collapsed"));
    assert_eq!(text("outline", "turns").as_deref(), Some("collapsed"));
}

#[test]
fn session_overrides_survive_reopen_and_stay_per_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut first = open(dir.path(), Some("0199c0e5-7a3b-7c3e-9f00-000000000001")).store;
    first
        .set(
            Layer::Session,
            kind_form(MessageKind::Thinking, DisplayForm::Collapsed),
        )
        .expect("save");
    first
        .set_turn_deviations(&[(prompt(3), DisclosureState::Collapsed)])
        .expect("save");
    drop(first);

    let mut second = open(dir.path(), Some("0199c0e5-7a3b-7c3e-9f00-000000000002")).store;
    assert_eq!(second.effective(), PanelSettings::default());
    second
        .set(
            Layer::Session,
            kind_form(MessageKind::Thinking, DisplayForm::Expanded),
        )
        .expect("save");
    drop(second);

    let first = open(dir.path(), Some("0199c0e5-7a3b-7c3e-9f00-000000000001")).store;
    let settings = first.effective();
    assert_eq!(
        settings.message_form(MessageKind::Thinking),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(settings.turn(prompt(3)), DisclosureState::Collapsed);
    assert_eq!(settings.turn(prompt(4)), DisclosureState::Expanded);

    let second = open(dir.path(), Some("0199c0e5-7a3b-7c3e-9f00-000000000002")).store;
    let settings = second.effective();
    assert_eq!(
        settings.message_form(MessageKind::Thinking),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(settings.turn(prompt(3)), DisclosureState::Expanded);

    let global = open(dir.path(), None).store;
    assert_eq!(global.effective(), PanelSettings::default());
}

#[test]
fn effective_value_is_session_then_global_then_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), Some("s1")).store;
    assert_eq!(store.effective().message_form(MessageKind::Execute), None);
    assert_eq!(store.effective().outline_turns(), DisclosureState::Expanded);

    store
        .set(
            Layer::Global,
            kind_form(MessageKind::Execute, DisplayForm::Collapsed),
        )
        .expect("save");
    assert_eq!(
        store.effective().message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );

    store
        .set(
            Layer::Session,
            kind_form(MessageKind::Execute, DisplayForm::Expanded),
        )
        .expect("save");
    assert_eq!(
        store.effective().message_form(MessageKind::Execute),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        store.global().message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );

    store
        .clear(Layer::Session, SettingKey::KindForm(MessageKind::Execute))
        .expect("save");
    assert_eq!(
        store.effective().message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );

    store
        .clear(Layer::Global, SettingKey::KindForm(MessageKind::Execute))
        .expect("save");
    assert_eq!(store.effective().message_form(MessageKind::Execute), None);
}

#[test]
fn turn_exception_beats_turn_default() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), Some("s1")).store;
    store
        .set(
            Layer::Global,
            Setting::OutlineTurns(DisclosureState::Collapsed),
        )
        .expect("save");
    store
        .set_turn_deviations(&[(prompt(2), DisclosureState::Expanded)])
        .expect("save");
    assert_eq!(store.effective().turn(prompt(2)), DisclosureState::Expanded);
    assert_eq!(
        store.effective().turn(prompt(5)),
        DisclosureState::Collapsed
    );

    store
        .set_turn_deviations(&[(prompt(2), DisclosureState::Collapsed)])
        .expect("save");
    assert_eq!(
        store.effective().turn(prompt(2)),
        DisclosureState::Collapsed,
        "a state equal to the turn default drops the exception"
    );
}

#[test]
fn clearing_turns_from_a_prompt_drops_that_prompt_and_every_later_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), Some("s1")).store;
    store
        .set_turn_deviations(&[
            (prompt(0), DisclosureState::Collapsed),
            (prompt(1), DisclosureState::Collapsed),
            (prompt(4), DisclosureState::Collapsed),
        ])
        .expect("save");

    assert!(matches!(
        store.clear_turns_from(prompt(1)),
        Ok(SaveOutcome::Written)
    ));
    drop(store);

    let settings = open(dir.path(), Some("s1")).store.effective();
    assert_eq!(settings.turn(prompt(0)), DisclosureState::Collapsed);
    assert_eq!(settings.turn(prompt(1)), DisclosureState::Expanded);
    assert_eq!(settings.turn(prompt(4)), DisclosureState::Expanded);

    let mut store = open(dir.path(), Some("s1")).store;
    assert!(matches!(
        store.clear_turns_from(prompt(1)),
        Ok(SaveOutcome::Unchanged)
    ));
}

#[test]
fn clearing_turns_without_a_session_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    assert!(matches!(
        store.clear_turns_from(prompt(0)),
        Ok(SaveOutcome::Unchanged)
    ));
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[test]
fn selecting_the_session_layer_copies_nothing_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut global = open(dir.path(), None).store;
    global
        .set(
            Layer::Global,
            kind_form(MessageKind::Read, DisplayForm::Collapsed),
        )
        .expect("save");
    drop(global);
    let before = file_names(dir.path());

    let store = open(dir.path(), Some("s1")).store;

    assert_eq!(store.session(), Some(&LayerSettings::default()));
    assert_eq!(
        store.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(file_names(dir.path()), before);
    assert!(!dir.path().join("sessions").exists());
}

#[test]
fn session_edits_need_an_attached_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    let result = store.set(
        Layer::Session,
        kind_form(MessageKind::Read, DisplayForm::Collapsed),
    );
    assert!(matches!(result, Err(SaveError::NoSession)));
    assert!(matches!(
        store.set_turn_deviations(&[(prompt(1), DisclosureState::Collapsed)]),
        Err(SaveError::NoSession)
    ));
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[test]
fn repeating_a_stored_value_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    let setting = Setting::OutlineTurns(DisclosureState::Collapsed);
    assert!(matches!(
        store.set(Layer::Global, setting),
        Ok(SaveOutcome::Written)
    ));
    assert!(matches!(
        store.set(Layer::Global, setting),
        Ok(SaveOutcome::Unchanged)
    ));
    assert!(matches!(
        store.clear(Layer::Global, SettingKey::Section(SidebarSection::Context)),
        Ok(SaveOutcome::Unchanged)
    ));
}

#[test]
fn a_corrupt_file_yields_defaults_and_stays_untouched_until_the_next_save() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("global.toml");
    let corrupt = b"[message_display\nexecute = ";
    std::fs::write(&path, corrupt).expect("write");

    let report = open(dir.path(), None);
    assert_eq!(report.store.effective(), PanelSettings::default());
    let [warning] = report.warnings.as_slice() else {
        panic!("expected one warning, got {:?}", report.warnings);
    };
    assert_eq!(warning.layer, Layer::Global);
    assert_eq!(warning.path, path);
    assert!(matches!(warning.problem, LoadProblem::Malformed(_)));
    assert_eq!(std::fs::read(&path).expect("read"), corrupt);

    let mut store = report.store;
    let outcome = store
        .set(
            Layer::Global,
            kind_form(MessageKind::Search, DisplayForm::Collapsed),
        )
        .expect("save");
    let backup = dir.path().join("global.toml.bad");
    assert!(matches!(outcome, SaveOutcome::Repaired { backup: ref moved } if *moved == backup));
    assert_eq!(std::fs::read(&backup).expect("backup"), corrupt);
    drop(store);

    let report = open(dir.path(), None);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(
        report.store.effective().message_form(MessageKind::Search),
        Some(DisplayForm::Collapsed)
    );
}

#[test]
fn invalid_entries_are_reported_and_valid_ones_still_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("global.toml");
    std::fs::write(
        &path,
        "version = 1\n[message_display]\nexecute = \"hologram\"\nbogus_kind = \"expanded\"\nread = \"collapsed\"\n[sidebar]\ncontext = \"collapsed\"\n",
    )
    .expect("write");

    let report = open(dir.path(), None);

    let mut ignored: Vec<String> = report
        .warnings
        .iter()
        .filter_map(|warning| match &warning.problem {
            LoadProblem::IgnoredEntry { key } => Some(key.clone()),
            LoadProblem::Unreadable { .. }
            | LoadProblem::Malformed(_)
            | LoadProblem::NewerVersion { .. } => None,
        })
        .collect();
    ignored.sort();
    assert_eq!(
        ignored,
        ["message_display.bogus_kind", "message_display.execute"]
    );
    let settings = report.store.effective();
    assert_eq!(
        settings.message_form(MessageKind::Read),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(settings.message_form(MessageKind::Execute), None);
    assert_eq!(
        settings.section(SidebarSection::Context),
        DisclosureState::Collapsed
    );
}

#[test]
fn unknown_keys_and_comments_survive_a_save() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("global.toml");
    std::fs::write(
        &path,
        "# keep me\nversion = 2\nfuture_key = \"x\"\n\n[tools]\nbash = true\n\n[message_display]\nread = \"collapsed\"\n",
    )
    .expect("write");

    let report = open(dir.path(), None);
    assert!(matches!(
        report.warnings.as_slice(),
        [warning] if warning.problem == LoadProblem::NewerVersion { found: 2 }
    ));
    let mut store = report.store;
    store
        .set(
            Layer::Global,
            kind_form(MessageKind::Execute, DisplayForm::Expanded),
        )
        .expect("save");

    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("# keep me"), "{text}");
    let doc: toml::Value = toml::from_str(&text).expect("valid");
    assert_eq!(doc.get("version").and_then(|v| v.as_integer()), Some(2));
    assert_eq!(doc.get("future_key").and_then(|v| v.as_str()), Some("x"));
    assert_eq!(
        doc.get("tools")
            .and_then(|t| t.get("bash"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    let display = doc.get("message_display").expect("table");
    assert_eq!(
        display.get("read").and_then(|v| v.as_str()),
        Some("collapsed")
    );
    assert_eq!(
        display.get("execute").and_then(|v| v.as_str()),
        Some("expanded")
    );
}

#[test]
fn a_save_leaves_no_temp_file_and_a_parsable_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), Some("s1")).store;
    for round in 0..5 {
        let form = if round % 2 == 0 {
            DisplayForm::Collapsed
        } else {
            DisplayForm::Expanded
        };
        store
            .set(Layer::Global, kind_form(MessageKind::Skill, form))
            .expect("save");
        store
            .set(Layer::Session, kind_form(MessageKind::Skill, form))
            .expect("save");
    }

    assert_eq!(file_names(dir.path()), ["global.toml", "sessions"]);
    assert_eq!(file_names(&dir.path().join("sessions")), ["s1.toml"]);
    let doc = read_toml(&dir.path().join("global.toml"));
    assert_eq!(
        doc.get("message_display")
            .and_then(|t| t.get("skill"))
            .and_then(|v| v.as_str()),
        Some("collapsed")
    );
    read_toml(&dir.path().join("sessions").join("s1.toml"));
}

#[test]
fn every_settings_kind_round_trips_under_a_unique_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    let kinds = MessageKind::settings_kinds();
    let form_for = |index: usize| {
        if index % 2 == 0 {
            DisplayForm::Collapsed
        } else {
            DisplayForm::Expanded
        }
    };
    for (index, kind) in kinds.iter().enumerate() {
        store
            .set(Layer::Global, kind_form(*kind, form_for(index)))
            .expect("save");
    }
    drop(store);

    let settings = open(dir.path(), None).store.effective();
    for (index, kind) in kinds.iter().enumerate() {
        assert_eq!(
            settings.message_form(*kind),
            Some(form_for(index)),
            "{kind:?}"
        );
    }

    let doc = read_toml(&dir.path().join("global.toml"));
    let table = doc
        .get("message_display")
        .and_then(|t| t.as_table())
        .expect("table");
    let names: HashSet<&str> = table.keys().map(String::as_str).collect();
    assert_eq!(table.len(), kinds.len());
    assert_eq!(names.len(), kinds.len());
    for pinned in [
        "user_prompt",
        "agent_message",
        "web_fetch",
        "other_tool",
        "memory_capture",
    ] {
        assert!(names.contains(pinned), "{pinned}");
    }
}

#[test]
fn a_kind_the_settings_do_not_offer_is_not_stored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = open(dir.path(), None).store;
    let outcome = store
        .set(
            Layer::Global,
            kind_form(MessageKind::Stub, DisplayForm::Collapsed),
        )
        .expect("no write error");
    assert!(matches!(outcome, SaveOutcome::Unchanged));
    assert_eq!(kind_name(MessageKind::Stub), None);
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[test]
fn a_failed_write_keeps_the_previous_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"").expect("write");

    let mut store = open(&blocker, None).store;
    let result = store.set(
        Layer::Global,
        kind_form(MessageKind::Read, DisplayForm::Collapsed),
    );

    assert!(matches!(result, Err(SaveError::Io { .. })));
    assert_eq!(store.effective().message_form(MessageKind::Read), None);
}

#[test]
fn session_ids_must_be_file_name_safe() {
    for bad in ["", "../escape", "a/b", "a.b", "a b"] {
        assert!(SessionKey::parse(bad).is_err(), "{bad:?}");
    }
    assert!(SessionKey::parse(&"x".repeat(129)).is_err());
    assert!(SessionKey::parse("0199c0e5-7a3b-7c3e-9f00-1a2b3c4d5e6f").is_ok());
}

#[test]
fn two_stores_changing_different_keys_keep_both_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut first = open(dir.path(), Some("s1")).store;
    let mut second = open(dir.path(), Some("s1")).store;

    first
        .set(
            Layer::Global,
            kind_form(MessageKind::Execute, DisplayForm::Collapsed),
        )
        .expect("save");
    second
        .set(
            Layer::Global,
            kind_form(MessageKind::Read, DisplayForm::Collapsed),
        )
        .expect("save");
    first
        .set(
            Layer::Session,
            kind_form(MessageKind::Edit, DisplayForm::Collapsed),
        )
        .expect("save");
    second
        .set_turn_deviations(&[(prompt(7), DisclosureState::Collapsed)])
        .expect("save");

    let reopened = open(dir.path(), Some("s1")).store;
    let settings = reopened.effective();
    assert_eq!(
        settings.message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(
        settings.message_form(MessageKind::Read),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(
        settings.message_form(MessageKind::Edit),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(settings.turn(prompt(7)), DisclosureState::Collapsed);
    assert_eq!(
        second.effective().message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed),
        "a save also refreshes the values the writer holds"
    );
}

#[test]
fn a_stale_store_still_writes_its_choice_over_another_processes_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut stale = open(dir.path(), None).store;
    let mut other = open(dir.path(), None).store;
    let collapsed = kind_form(MessageKind::Execute, DisplayForm::Collapsed);

    stale.set(Layer::Global, collapsed).expect("save");
    other
        .set(
            Layer::Global,
            kind_form(MessageKind::Execute, DisplayForm::Expanded),
        )
        .expect("save");
    let outcome = stale.set(Layer::Global, collapsed).expect("save");

    assert!(matches!(outcome, SaveOutcome::Written));
    let reopened = open(dir.path(), None).store;
    assert_eq!(
        reopened.effective().message_form(MessageKind::Execute),
        Some(DisplayForm::Collapsed)
    );
}

#[cfg(unix)]
#[test]
fn settings_files_and_directories_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut store = open(&root, Some("s1")).store;
    store
        .set(
            Layer::Global,
            kind_form(MessageKind::Read, DisplayForm::Collapsed),
        )
        .expect("save");
    store
        .set(
            Layer::Session,
            kind_form(MessageKind::Read, DisplayForm::Expanded),
        )
        .expect("save");

    let mode = |path: &Path| {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(&root), 0o700);
    assert_eq!(mode(&root.join("sessions")), 0o700);
    assert_eq!(mode(&root.join("global.toml")), 0o600);
    assert_eq!(mode(&root.join("sessions").join("s1.toml")), 0o600);
}
