//! AppView fan-out of live panel-settings changes into every loaded view.

use crate::app::agent::AgentId;
use crate::app::agent_view::test_fixtures::make_agent;
use crate::app::app_view::AppView;
use crate::acp::model_state::ModelState;
use crate::app::panel_settings::Setting;
use crate::render::draw::EscapeWriter;
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use agent_client_protocol as acp;
use std::path::Path;

fn view_with(session: &str, root: &Path) -> crate::app::agent_view::AgentView {
    let mut agent = make_agent();
    agent.session.session_id = Some(acp::SessionId::from(session.to_owned()));
    agent.open_panel_settings_in(root);
    agent
}

fn app_with_views(root: &Path) -> AppView {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(tx, ModelState::default(), Vec::new(), EscapeWriter::disconnected());
    app.panel_settings_root = Some(root.to_path_buf());
    app.agents.insert(AgentId(0), view_with("sess-a", root));
    app.agents.insert(AgentId(1), view_with("sess-b", root));
    app
}

fn form_of(app: &AppView, id: AgentId, kind: MessageKind) -> Option<DisplayForm> {
    app.agents
        .get(&id)
        .expect("view is loaded")
        .scrollback
        .display_defaults()
        .get(kind)
}

#[test]
fn a_global_change_fans_out_to_every_loaded_view() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut app = app_with_views(&root);

    let mut writer = view_with("sess-writer", &root);
    writer.save_global_panel_settings(&[Setting::KindForm {
        kind: MessageKind::Read,
        form: DisplayForm::Expanded,
    }]);

    assert!(app.reload_global_panel_settings());
    for id in [AgentId(0), AgentId(1)] {
        assert_eq!(
            form_of(&app, id, MessageKind::Read),
            Some(DisplayForm::Expanded),
            "view {id:?} applied the foreign global write"
        );
    }
    assert!(
        !app.reload_global_panel_settings(),
        "the duplicate echo fans out to nothing"
    );
}

#[test]
fn a_session_change_fans_out_only_to_views_of_that_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut app = app_with_views(&root);

    let mut writer = view_with("sess-a", &root);
    writer.toggle_message_display_scope();
    writer.save_display_settings(&[Setting::KindForm {
        kind: MessageKind::Edit,
        form: DisplayForm::Expanded,
    }]);

    assert!(app.reload_session_panel_settings("sess-a"));
    assert_eq!(
        form_of(&app, AgentId(0), MessageKind::Edit),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        form_of(&app, AgentId(1), MessageKind::Edit),
        None,
        "a view of another session is untouched by sess-a's file"
    );
    assert!(!app.reload_session_panel_settings("sess-none"));
}
