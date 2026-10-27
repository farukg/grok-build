//! Live propagation of panel-settings changes into a view: another window's
//! global and session writes, the equality gate for own/duplicate events, and the
//! guarantees a reload must keep (selection, no traffic to the agent, no writes).

use crate::app::agent_view::test_fixtures::make_agent;
use crate::app::agent_view::AgentView;
use crate::app::panel_settings::{DisclosureState, Setting, SidebarSection};
use crate::scrollback::block::{MessageKind, RenderBlock};
use crate::scrollback::types::DisplayForm;
use std::path::Path;

fn view(root: &Path, session: Option<&str>) -> AgentView {
    let mut agent = make_agent();
    agent.session.session_id = session.map(|id| id.to_owned().into());
    agent.open_panel_settings_in(root);
    agent
}

fn kind_form(kind: MessageKind, form: DisplayForm) -> Setting {
    Setting::KindForm { kind, form }
}

fn global_file(root: &Path) -> std::path::PathBuf {
    root.join("global.toml")
}

#[test]
fn another_windows_global_write_applies_and_the_echo_does_not_reapply() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut writer = view(&root, Some("sess-a"));
    let mut reader = view(&root, Some("sess-b"));

    writer.save_global_panel_settings(&[
        Setting::Section {
            section: SidebarSection::Context,
            state: DisclosureState::Collapsed,
        },
        kind_form(MessageKind::Read, DisplayForm::Expanded),
    ]);
    assert!(
        reader.reload_global_panel_settings(),
        "the reader applies the foreign global write"
    );
    assert_eq!(
        reader.scrollback.display_defaults().get(MessageKind::Read),
        Some(DisplayForm::Expanded)
    );
    assert!(!reader.reload_global_panel_settings());

    let echo = writer.reload_global_panel_settings();
    assert!(
        !echo,
        "the writer's own save echoing through the watcher changes nothing"
    );
    assert!(
        writer.toast.is_none(),
        "an absorbed event does not warn about anything"
    );
}

#[test]
fn a_session_file_write_reaches_only_views_of_that_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut writer = view(&root, Some("sess-a"));
    let mut peer = view(&root, Some("sess-a"));
    let mut other = view(&root, Some("sess-b"));

    writer.toggle_message_display_scope();
    writer.save_display_settings(&[kind_form(MessageKind::Edit, DisplayForm::Expanded)]);
    assert!(root.join("sessions").join("sess-a.toml").exists());

    assert!(peer.reload_session_panel_settings("sess-a"));
    assert_eq!(
        peer.scrollback.display_defaults().get(MessageKind::Edit),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        peer.panel_persistence.effective().display_scope(),
        crate::app::panel_settings::Layer::Session
    );

    assert!(
        !other.reload_session_panel_settings("sess-a"),
        "a view of another session keeps its state"
    );
    assert_eq!(
        other.scrollback.display_defaults().get(MessageKind::Edit),
        None
    );
    assert!(!peer.reload_session_panel_settings("sess-b"));
}

#[test]
fn a_reload_keeps_selection_and_sends_nothing_to_the_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    agent.scrollback.push_block(RenderBlock::read("a.rs", None));
    agent.scrollback.push_block(RenderBlock::read("b.rs", None));
    agent.scrollback.push_block(RenderBlock::agent_message("done"));
    agent.scrollback.set_selected(Some(1));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.session.acp_tx = tx;

    let mut writer = view(&root, Some("sess-b"));
    writer.save_global_panel_settings(&[kind_form(MessageKind::Read, DisplayForm::Expanded)]);
    assert!(agent.reload_global_panel_settings());

    assert_eq!(
        agent.scrollback.selected(),
        Some(1),
        "an unrelated preference change keeps the selection"
    );
    assert!(
        rx.try_recv().is_err(),
        "applying preferences sends nothing to the agent"
    );
    assert!(
        !agent.reload_global_panel_settings(),
        "the duplicate echo is absorbed and sends nothing either"
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_damaged_global_file_keeps_the_state_and_names_the_cause() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut writer = view(&root, Some("sess-a"));
    writer.save_global_panel_settings(&[kind_form(MessageKind::Read, DisplayForm::Expanded)]);
    let mut reader = view(&root, Some("sess-b"));

    std::fs::write(global_file(&root), "not toml [").unwrap();
    assert!(
        reader.reload_global_panel_settings(),
        "the warning itself must surface even when nothing changes"
    );
    assert_eq!(
        reader.scrollback.display_defaults().get(MessageKind::Read),
        Some(DisplayForm::Expanded),
        "the last-known value stays in force"
    );
    let toast = reader.toast.as_ref().expect("the failure is toasted").0.clone();
    assert!(
        toast.contains("Panel settings") && toast.contains("TOML"),
        "the toast names the source and the cause: {toast}"
    );
    assert!(
        toast.contains("kept the last loaded values"),
        "the reload says what it did instead of claiming defaults: {toast}"
    );
}

#[test]
fn dock_override_removal_recomputes_real_default_chain() {
    use crate::app::panel_settings::{Layer, PanelSettingsStore, SettingKey};
    use crate::views::dock::{self, DockPlacement};
    const CHILD: &str = "GROK_PANEL_DOCK_REGRESSION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // A fresh test process isolates OnceLock-backed home resolution.
        let dir = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("dock_override_removal_recomputes_real_default_chain")
            .env_clear()
            .env(CHILD, "1")
            .env("HOME", dir.path())
            .env("GROK_HOME", dir.path())
            .current_dir(dir.path())
            .status().unwrap();
        assert!(status.success());
        return;
    }
    let home = xai_grok_config::grok_home();
    let root = home.join("panel-settings");
    assert_eq!(crate::app::dock_requirement_pin(), None, "fixture requires no system dock pin");
    dock::set_enabled_for_test(false);
    for (config, env, pin, remote, expected) in [
        (None, None, None, None, false),
        (None, None, None, Some(true), true),
        (Some(false), None, None, Some(true), false),
        (Some(true), None, None, Some(false), true),
        (Some(true), Some(false), None, Some(true), false),
        (Some(false), Some(true), None, Some(false), true),
        (Some(false), Some(true), Some(false), Some(true), false),
        (Some(false), Some(false), Some(true), Some(false), true),
    ] {
        let config_path = home.join("config.toml");
        let pin_path = home.join("requirements.toml");
        for (path, value) in [(&config_path, config), (&pin_path, pin)] {
            match value {
                Some(value) => std::fs::write(path, format!("[features]\ndock = {value}\n")).unwrap(),
                None => { if path.exists() { std::fs::remove_file(path).unwrap(); } }
            }
        }
        unsafe {
            match env {
                Some(value) => std::env::set_var("GROK_DOCK", if value { "true" } else { "false" }),
                None => std::env::remove_var("GROK_DOCK"),
            }
        }
        crate::app::confirm_dock_enabled(remote);
        let mut reader = view(&root, Some("reader"));
        let mut writer = PanelSettingsStore::open(&root, None).store;
        for delete_file in [false, true] {
            writer.set(Layer::Global, Setting::DockPlacement(DockPlacement::Top)).unwrap();
            reader.reload_global_panel_settings();
            if pin.or(env).is_none_or(|visible| visible) {
                assert_eq!(dock::placement(), DockPlacement::Top);
            } else {
                assert!(!dock::enabled(), "explicit visibility pins cannot be overruled");
            }
            if delete_file {
                std::fs::remove_file(global_file(&root)).unwrap();
            } else {
                writer.clear(Layer::Global, SettingKey::DockPlacement).unwrap();
            }
            assert!(reader.reload_global_panel_settings());
            assert_eq!(dock::enabled(), expected, "removal uses current Pin/Env/Config/Remote/Builtin inputs");
        }
    }
    std::fs::remove_file(home.join("requirements.toml")).unwrap();
    unsafe { std::env::remove_var("GROK_DOCK"); }
    crate::app::confirm_dock_enabled(Some(false));
    let mut reader = view(&root, Some("reader"));
    let mut writer = PanelSettingsStore::open(&root, None).store;
    writer.set(Layer::Global, Setting::DockPlacement(DockPlacement::Top)).unwrap();
    reader.reload_global_panel_settings();
    std::fs::write(home.join("config.toml"), "[features]\ndock = true\n").unwrap();
    writer.clear(Layer::Global, SettingKey::DockPlacement).unwrap();
    assert!(reader.reload_global_panel_settings());
    assert!(dock::enabled(), "a config edit made while the override was active must be read now");
}

#[test]
fn semantic_damage_keeps_view_state_with_a_safe_cause_then_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut writer = view(root, None);
    writer.save_global_panel_settings(&[kind_form(MessageKind::Read, DisplayForm::Expanded)]);
    let mut reader = view(root, None);
    std::fs::write(global_file(root), "[message_display]\nread = \"invalid-choice-marker\"\n").unwrap();
    assert!(reader.reload_global_panel_settings());
    assert_eq!(reader.scrollback.display_defaults().get(MessageKind::Read), Some(DisplayForm::Expanded));
    let toast = &reader.toast.as_ref().unwrap().0;
    assert!(toast.contains("invalid value for known setting `message_display.read`"));
    assert!(toast.contains("kept the last loaded values"));
    assert!(!toast.contains("invalid-choice-marker"));
    std::fs::write(global_file(root), "[message_display]\nread = \"collapsed\"\n").unwrap();
    assert!(reader.reload_global_panel_settings());
    assert_eq!(reader.scrollback.display_defaults().get(MessageKind::Read), Some(DisplayForm::Collapsed));
}

#[test]
fn default_constructors_leave_preferences_unattached_and_reload_passive() {
    use crate::acp::model_state::ModelState;
    use crate::app::app_view::AppView;
    use crate::render::draw::EscapeWriter;
    let mut agent = make_agent();
    assert!(agent.panel_persistence.store.is_none());
    assert!(!agent.reload_global_panel_settings());
    assert!(!agent.reload_session_panel_settings("unattached"));
    assert!(agent.panel_persistence.store.is_none());
    assert!(agent.toast.is_none());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(tx, ModelState::default(), Vec::new(), EscapeWriter::disconnected());
    assert!(app.panel_settings_root.is_none());
    assert!(!app.reload_all_panel_settings());
    assert!(app.panel_settings_root.is_none());
    assert!(rx.try_recv().is_err());
}
