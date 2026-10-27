//! Persistence regressions for hierarchical F7 preferences.

use super::super::{DisclosureState, Layer, PanelSettingsStore, SessionKey, Setting, SidebarSection};
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;

fn session(id: &str) -> SessionKey { SessionKey::parse(id).unwrap() }

#[test]
fn session_parent_outranks_global_child_but_session_child_outranks_parent() {
    let root = tempfile::tempdir().unwrap();
    let mut store = PanelSettingsStore::open(root.path(), Some(session("a"))).store;
    store.set(Layer::Global, Setting::KindForm { kind: MessageKind::InjectedNotification, form: DisplayForm::Expanded }).unwrap();
    store.set(Layer::Session, Setting::KindForm { kind: MessageKind::Injected, form: DisplayForm::Collapsed }).unwrap();
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Collapsed));
    store.set(Layer::Session, Setting::KindForm { kind: MessageKind::InjectedNotification, form: DisplayForm::Expanded }).unwrap();
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
    assert_eq!(store.effective().message_form(MessageKind::InjectedMemory), Some(DisplayForm::Collapsed));
    store.set(Layer::Session, Setting::DisplayScope(Layer::Global)).unwrap();
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
    assert_eq!(store.effective().message_form(MessageKind::InjectedMemory), None);
    store.set(Layer::Session, Setting::DisplayScope(Layer::Session)).unwrap();
    let reopened = PanelSettingsStore::open(root.path(), Some(session("a"))).store;
    assert_eq!(reopened.effective().message_form(MessageKind::InjectedMemory), Some(DisplayForm::Collapsed));
    assert_eq!(reopened.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
}

#[test]
fn legacy_injected_global_value_is_inherited_without_overwriting_explicit_children() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("global.toml"), "version = 1\n[message_display]\ninjected = 'expanded'\ninjected_memory = 'collapsed'\n").unwrap();
    let store = PanelSettingsStore::open(root.path(), Some(session("new"))).store;
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
    assert_eq!(store.effective().message_form(MessageKind::InjectedMemory), Some(DisplayForm::Collapsed));
    assert_eq!(store.effective().display_scope(), Layer::Global);
}

#[test]
fn every_top_and_semantic_group_fold_is_global_across_a_b_new_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let sections = [SidebarSection::Dock, SidebarSection::TodoSection, SidebarSection::Context,
        SidebarSection::MessageDisplay, SidebarSection::ContextInstructions, SidebarSection::ContextConversation,
        SidebarSection::ContextContinuation, SidebarSection::ContextNotifications, SidebarSection::ContextRetained,
        SidebarSection::DisplayConversation, SidebarSection::DisplayFiles, SidebarSection::DisplayResearch,
        SidebarSection::DisplayWork, SidebarSection::DisplaySession, SidebarSection::DisplayInjected];
    let mut a = PanelSettingsStore::open(root.path(), Some(session("a"))).store;
    let mut b = PanelSettingsStore::open(root.path(), Some(session("b"))).store;
    for section in sections {
        a.set(Layer::Session, Setting::Section { section, state: DisclosureState::Collapsed }).unwrap();
    }
    let reload = b.reload_global();
    assert!(reload.warnings.is_empty());
    assert!(reload.changed);
    for id in ["a", "b", "new"] {
        a.attach_session(Some(session(id)));
        let reopened = PanelSettingsStore::open(root.path(), Some(session(id))).store;
        for section in sections {
            assert_eq!(a.effective().section(section), DisclosureState::Collapsed);
            assert_eq!(b.effective().section(section), DisclosureState::Collapsed);
            assert_eq!(reopened.effective().section(section), DisclosureState::Collapsed);
            assert_eq!(reopened.session().unwrap().section(section), None);
        }
    }
}

#[test]
fn explicit_child_equal_to_global_is_not_elided_below_a_session_parent() {
    let root = tempfile::tempdir().unwrap();
    let mut store = PanelSettingsStore::open(root.path(), Some(session("a"))).store;
    store.set(Layer::Global, Setting::KindForm { kind: MessageKind::InjectedNotification, form: DisplayForm::Expanded }).unwrap();
    store.set(Layer::Session, Setting::KindForm { kind: MessageKind::Injected, form: DisplayForm::Collapsed }).unwrap();
    store.set_deviations(Layer::Session, &[Setting::KindForm { kind: MessageKind::InjectedNotification, form: DisplayForm::Expanded }]).unwrap();
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
    store.set(Layer::Global, Setting::KindForm { kind: MessageKind::InjectedNotification, form: DisplayForm::Collapsed }).unwrap();
    assert_eq!(store.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
    let reopened = PanelSettingsStore::open(root.path(), Some(session("a"))).store;
    assert_eq!(reopened.effective().message_form(MessageKind::InjectedNotification), Some(DisplayForm::Expanded));
}
