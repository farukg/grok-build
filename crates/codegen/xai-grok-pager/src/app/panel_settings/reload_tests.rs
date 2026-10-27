//! Event-driven reload of the two layer files: what another process sharing the
//! root wrote, and what a damaged file may and may not do to held state.

use std::path::Path;

use super::super::{
    DisclosureState, Layer, LoadProblem, LoadReport, PanelSettingsStore, SessionKey, Setting,
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

fn global_file(root: &Path) -> std::path::PathBuf {
    root.join("global.toml")
}

fn session_file(root: &Path, session: &str) -> std::path::PathBuf {
    root.join("sessions").join(format!("{session}.toml"))
}

/// Every file under `root` with its bytes, so a test can prove a reload wrote nothing.
fn tree_snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut snapshot = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).expect("readable file");
                snapshot.push((path, bytes));
            }
        }
    }
    snapshot.sort_by(|a, b| a.0.cmp(&b.0));
    snapshot
}

#[test]
fn another_clients_global_write_applies_and_duplicate_or_own_events_are_absorbed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    let mut reader = open(root, Some("sess-b")).store;

    writer
        .set(
            Layer::Global,
            Setting::Section {
                section: SidebarSection::Context,
                state: DisclosureState::Collapsed,
            },
        )
        .unwrap();

    let applied = reader.reload_global();
    assert!(applied.changed, "the other window's write is adopted");
    assert!(
        applied.warnings.is_empty(),
        "a healthy file warns about nothing"
    );
    assert_eq!(
        reader.effective().section(SidebarSection::Context),
        DisclosureState::Collapsed
    );

    let duplicate = reader.reload_global();
    assert!(!duplicate.changed, "the same event again changes nothing");
    assert!(duplicate.warnings.is_empty());

    let own_echo = writer.reload_global();
    assert!(
        !own_echo.changed,
        "the writer's own save echoing back changes nothing"
    );
    assert!(own_echo.warnings.is_empty());
}

#[test]
fn a_reloaded_global_layer_keeps_effective_session_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    let mut reader = open(root, Some("sess-b")).store;

    reader
        .set(Layer::Session, kind_form(MessageKind::Read, DisplayForm::Collapsed))
        .unwrap();
    writer
        .set(Layer::Global, kind_form(MessageKind::Read, DisplayForm::Expanded))
        .unwrap();

    assert!(reader.reload_global().changed);
    assert_eq!(
        reader.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Collapsed),
        "the session override survives the global change"
    );
    assert_eq!(
        reader.global().message_form(MessageKind::Read),
        Some(DisplayForm::Expanded),
        "the reloaded global itself did adopt the write"
    );
}

#[test]
fn a_session_file_change_reloads_only_that_sessions_layer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut holder = open(root, Some("sess-a")).store;
    let mut same_session = open(root, Some("sess-a")).store;
    let mut other = open(root, Some("sess-b")).store;

    holder
        .set(Layer::Session, kind_form(MessageKind::Read, DisplayForm::Expanded))
        .unwrap();
    assert!(session_file(root, "sess-a").exists());

    let applied = same_session.reload_session();
    assert!(applied.changed, "the other window's session write is adopted");
    assert!(applied.warnings.is_empty());
    assert_eq!(
        same_session.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Expanded)
    );

    let untouched = other.reload_session();
    assert!(
        !untouched.changed,
        "another session's file change is not this session's change"
    );

    let no_session = PanelSettingsStore::open(root, None)
        .store
        .reload_session();
    assert!(!no_session.changed && no_session.warnings.is_empty());
}

#[test]
fn a_removed_session_file_is_an_empty_layer_without_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    let mut reader = open(root, Some("sess-a")).store;
    writer
        .set(Layer::Session, kind_form(MessageKind::Read, DisplayForm::Expanded))
        .unwrap();
    assert!(reader.reload_session().changed);

    std::fs::remove_file(session_file(root, "sess-a")).unwrap();
    let removed = reader.reload_session();
    assert!(removed.changed, "absence really removes the overrides");
    assert!(
        removed.warnings.is_empty(),
        "a deleted file is absence, not a failure"
    );
    assert_eq!(
        reader.effective().message_form(MessageKind::Read),
        None,
        "no override is left; the kind renders by its block's own policy"
    );
}

#[test]
fn a_malformed_global_file_keeps_last_known_state_and_then_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    writer
        .set(Layer::Global, kind_form(MessageKind::Read, DisplayForm::Expanded))
        .unwrap();
    let mut reader = open(root, Some("sess-b")).store;
    assert_eq!(
        reader.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Expanded)
    );

    std::fs::write(global_file(root), "not toml [").unwrap();
    let broken = reader.reload_global();
    assert!(!broken.changed, "a damaged file changes no held value");
    assert_eq!(broken.warnings.len(), 1);
    assert!(matches!(
        broken.warnings[0].problem,
        LoadProblem::Malformed(_)
    ));
    assert_eq!(
        reader.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Expanded),
        "the last-known value stays in force"
    );

    let mut healed = open(root, Some("sess-a")).store;
    healed
        .set(Layer::Global, kind_form(MessageKind::Read, DisplayForm::Collapsed))
        .unwrap();
    let recovered = reader.reload_global();
    assert!(recovered.changed, "a healthy file is adopted again");
    assert!(recovered.warnings.is_empty());
    assert_eq!(
        reader.effective().message_form(MessageKind::Read),
        Some(DisplayForm::Collapsed)
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_global_file_keeps_last_known_state_and_names_the_cause() {
    if unsafe { libc::geteuid() } == 0 {
        // Root reads through 0o000; the permission oracle would not hold.
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    writer
        .set(
            Layer::Global,
            Setting::Section {
                section: SidebarSection::Dock,
                state: DisclosureState::Collapsed,
            },
        )
        .unwrap();
    let mut reader = open(root, Some("sess-b")).store;

    let file = global_file(root);
    let mut permissions = std::fs::metadata(&file).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o000);
    std::fs::set_permissions(&file, permissions).unwrap();

    let blocked = reader.reload_global();
    assert!(!blocked.changed);
    assert_eq!(blocked.warnings.len(), 1);
    assert!(matches!(
        blocked.warnings[0].problem,
        LoadProblem::Unreadable { .. }
    ));
    assert_eq!(
        reader.effective().section(SidebarSection::Dock),
        DisclosureState::Collapsed,
        "the held value survives the failed read"
    );

    let mut permissions = std::fs::metadata(&file).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o600);
    std::fs::set_permissions(&file, permissions).unwrap();
    let readable_again = reader.reload_global();
    assert!(!readable_again.changed && readable_again.warnings.is_empty());
}

#[test]
fn reloads_never_write_any_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = open(root, Some("sess-a")).store;
    writer
        .set(
            Layer::Global,
            Setting::Section {
                section: SidebarSection::Dock,
                state: DisclosureState::Collapsed,
            },
        )
        .unwrap();
    writer
        .set(Layer::Session, kind_form(MessageKind::Read, DisplayForm::Expanded))
        .unwrap();
    std::fs::write(global_file(root), "not toml [").unwrap();

    let mut reader = open(root, Some("sess-a")).store;
    let _ = reader.reload_global();
    let _ = reader.reload_session();
    std::fs::remove_file(session_file(root, "sess-a")).unwrap();
    let before_removal = tree_snapshot(root);
    let _ = reader.reload_session();

    let after = tree_snapshot(root);
    assert_eq!(
        after, before_removal,
        "a reload after deletion creates no replacement file"
    );
    let global_after = after
        .into_iter()
        .find(|(path, _)| path == &global_file(root))
        .expect("the damaged global file is still there");
    assert_eq!(
        global_after.1, b"not toml ["[..],
        "the damaged file is preserved exactly; repair belongs to the next save"
    );
}
