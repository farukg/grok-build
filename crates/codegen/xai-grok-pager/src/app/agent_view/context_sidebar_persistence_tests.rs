//! F7 settings survive a restart: section folds and message-display forms, globally and per session.
//! Every test gives its views a store under a tempdir; none touches the real grok home.

use super::{
    AgentView, InputOutcome, KeyCode, KeyModifiers, click_row, draw, open_sidebar, press,
    press_with, ready_report, screen, sidebar_row, test_fixtures, toast, focus_row,
};
use crate::scrollback::block::MessageKind;
use crate::scrollback::types::DisplayForm;
use agent_client_protocol as acp;
use std::path::Path;

fn view(root: &Path, session: Option<&str>) -> AgentView {
    let mut agent = test_fixtures::make_agent();
    agent.session.session_id = session.map(|id| id.to_owned().into());
    agent.open_panel_settings_in(root);
    agent
}

fn open_f7(agent: &mut AgentView) {
    let _ = press(agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    let _ = draw(agent);
}

fn toggle_display_scope(agent: &mut AgentView) -> InputOutcome {
    let _ = press(agent, KeyCode::End);
    press(agent, KeyCode::Char('g'))
}

fn flip(agent: &mut AgentView, label: &str) {
    focus_row(agent, label);
    let buf = draw(agent);
    assert!(matches!(
        click_row(agent, &buf, label),
        InputOutcome::Changed
    ));
}

fn form(agent: &AgentView, kind: MessageKind) -> Option<DisplayForm> {
    agent.scrollback.display_defaults().get(kind)
}

fn read_toml(path: &Path) -> toml::Value {
    let text = std::fs::read_to_string(path).expect("settings file exists");
    toml::from_str(&text).expect("settings file is valid TOML")
}

fn stored<'a>(file: &'a toml::Value, table: &str, key: &str) -> Option<&'a str> {
    file.get(table)?.get(key)?.as_str()
}

fn global_file(root: &Path) -> std::path::PathBuf {
    root.join("global.toml")
}

fn session_file(root: &Path, session: &str) -> std::path::PathBuf {
    root.join("sessions").join(format!("{session}.toml"))
}

#[test]
fn a_changed_form_comes_back_in_a_new_view_even_before_f7_opens() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    flip(&mut agent, "File reads");
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Expanded));
    drop(agent);

    let mut restored = view(&root, Some("sess-a"));
    assert_eq!(
        form(&restored, MessageKind::Read),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        form(&restored, MessageKind::Edit),
        None,
        "only the changed kind is restored"
    );
    open_f7(&mut restored);
    focus_row(&mut restored, "File reads");
    assert!(sidebar_row(&draw(&mut restored), "File reads").contains("full"));
}

#[test]
fn a_global_edit_reaches_other_sessions_and_a_session_edit_stays_in_its_session() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");

    let mut a = view(&root, Some("sess-a"));
    open_f7(&mut a);
    flip(&mut a, "File reads");
    assert!(
        !session_file(&root, "sess-a").exists(),
        "a global edit creates no session file"
    );

    let mut b = view(&root, Some("sess-b"));
    assert_eq!(
        form(&b, MessageKind::Read),
        Some(DisplayForm::Expanded),
        "global applies to every session"
    );
    open_f7(&mut b);
    let _ = toggle_display_scope(&mut b);
    flip(&mut b, "File edits");
    flip(&mut b, "File reads");
    assert_eq!(form(&b, MessageKind::Edit), Some(DisplayForm::Expanded));
    assert_eq!(form(&b, MessageKind::Read), Some(DisplayForm::Collapsed));

    let global = read_toml(&global_file(&root));
    assert_eq!(stored(&global, "message_display", "read"), Some("expanded"));
    assert_eq!(
        stored(&global, "message_display", "edit"),
        None,
        "a session edit stays out of global"
    );
    let session = read_toml(&session_file(&root, "sess-b"));
    assert_eq!(
        stored(&session, "message_display", "edit"),
        Some("expanded")
    );
    assert_eq!(
        stored(&session, "message_display", "read"),
        Some("collapsed"),
        "the session value differs from the global one"
    );

    let b_again = view(&root, Some("sess-b"));
    assert_eq!(
        form(&b_again, MessageKind::Edit),
        Some(DisplayForm::Expanded)
    );
    assert_eq!(
        form(&b_again, MessageKind::Read),
        Some(DisplayForm::Collapsed)
    );
    let a_again = view(&root, Some("sess-a"));
    assert_eq!(form(&a_again, MessageKind::Edit), None);
    assert_eq!(
        form(&a_again, MessageKind::Read),
        Some(DisplayForm::Expanded)
    );
    let c = view(&root, Some("sess-c"));
    assert_eq!(
        form(&c, MessageKind::Edit),
        None,
        "another session does not see sess-b's edit"
    );
    assert_eq!(form(&c, MessageKind::Read), Some(DisplayForm::Expanded));
}

#[test]
fn g_switches_and_persists_the_effective_display_scope() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    assert!(screen(&draw(&mut agent)).contains("display: global"));

    assert!(matches!(
        toggle_display_scope(&mut agent),
        InputOutcome::Changed
    ));
    let buf = draw(&mut agent);
    assert!(screen(&buf).contains("display: this session"));
    assert!(!screen(&buf).contains("display: global"));
    assert!(matches!(
        toggle_display_scope(&mut agent),
        InputOutcome::Changed
    ));
    assert!(screen(&draw(&mut agent)).contains("display: global"));
    assert_eq!(stored(&read_toml(&session_file(&root, "sess-a")), "scope", "message_display"), Some("global"));
    let mut restored = view(&root, Some("sess-a"));
    open_f7(&mut restored);
    assert!(screen(&draw(&mut restored)).contains("display: global"));
    assert!(!global_file(&root).exists(), "switching scope changes no global value");
}

#[test]
fn g_says_why_a_view_without_a_session_cannot_save_to_one() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, None);
    open_f7(&mut agent);
    let _ = toggle_display_scope(&mut agent);
    assert!(
        toast(&agent).contains("no session is attached"),
        "{}",
        toast(&agent)
    );
    assert!(screen(&draw(&mut agent)).contains("display: global"));

    agent.bind_session_id(acp::SessionId::new("sess-late"));
    let _ = toggle_display_scope(&mut agent);
    assert!(screen(&draw(&mut agent)).contains("display: this session"));
}

#[test]
fn a_view_that_keeps_no_settings_says_so_and_still_works() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = open_sidebar();
    assert!(screen(&draw(&mut agent)).contains("not saved"));
    let _ = toggle_display_scope(&mut agent);
    assert!(toast(&agent).contains("not stored"), "{}", toast(&agent));
    flip(&mut agent, "File reads");
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Expanded));
}

#[test]
fn a_folded_section_stays_folded_after_a_restart() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    focus_row(&mut agent, "Context");
    let _ = press(&mut agent, KeyCode::Char(' '));
    assert!(!screen(&draw(&mut agent)).contains("Core instructions"));
    assert_eq!(
        stored(&read_toml(&global_file(&root)), "sidebar", "context"),
        Some("collapsed")
    );
    drop(agent);

    let mut restored = view(&root, Some("sess-other"));
    open_f7(&mut restored);
    let buf = draw(&mut restored);
    assert!(
        screen(&buf).contains("\u{25B8} Context"),
        "the stored fold applies to every session"
    );
    assert!(!screen(&buf).contains("Core instructions"));
    assert!(
        screen(&buf).contains("File reads"),
        "the other section stays open"
    );

    focus_row(&mut restored, "Context");
    let _ = press(&mut restored, KeyCode::Char(' '));
    let file = read_toml(&global_file(&root));
    assert_eq!(
        stored(&file, "sidebar", "context"),
        None,
        "an open section is the default and leaves no entry"
    );
}

#[test]
fn section_folds_stay_global_while_display_values_use_session_scope() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    let _ = toggle_display_scope(&mut agent);
    focus_row(&mut agent, "Message display");
    let _ = press(&mut agent, KeyCode::Char(' '));
    assert!(!screen(&draw(&mut agent)).contains("File reads"));
    assert_eq!(
        stored(&read_toml(&global_file(&root)), "sidebar", "message_display"),
        Some("collapsed")
    );
    assert_eq!(
        stored(&read_toml(&session_file(&root, "sess-a")), "sidebar", "message_display"),
        None
    );
    drop(agent);

    let mut same = view(&root, Some("sess-a"));
    open_f7(&mut same);
    assert!(!screen(&draw(&mut same)).contains("File reads"));
    let mut other = view(&root, Some("sess-b"));
    open_f7(&mut other);
    assert!(!screen(&draw(&mut other)).contains("File reads"));
}

fn message_display_table(file: &Path) -> toml::map::Map<String, toml::Value> {
    read_toml(file)
        .get("message_display")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn an_explicit_one_line_choice_for_edits_survives_a_restart() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    flip(&mut agent, "File edits");
    flip(&mut agent, "File edits");
    assert_eq!(
        form(&agent, MessageKind::Edit),
        Some(DisplayForm::Collapsed)
    );
    assert_eq!(
        stored(&read_toml(&global_file(&root)), "message_display", "edit"),
        Some("collapsed"),
        "a form the user chose is stored even when it equals the row's starting form"
    );
    drop(agent);

    let mut restored = view(&root, Some("sess-a"));
    assert_eq!(
        form(&restored, MessageKind::Edit),
        Some(DisplayForm::Collapsed),
        "the choice is applied, not left to the block's own policy"
    );
    open_f7(&mut restored);
    focus_row(&mut restored, "File edits");
    assert!(sidebar_row(&draw(&mut restored), "File edits").contains("1 line"));
}

#[test]
fn a_bulk_gesture_stores_every_bulk_kind_and_never_the_chat_messages() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    let _ = press(&mut agent, KeyCode::End);

    let _ = press(&mut agent, KeyCode::Char('-'));
    let table = message_display_table(&global_file(&root));
    assert_eq!(
        table.len(),
        MessageKind::bulk_kinds().len(),
        "only the bulk kinds are stored: {table:?}"
    );
    assert!(
        table
            .values()
            .all(|value| value.as_str() == Some("collapsed"))
    );
    assert_eq!(
        table.get("edit").and_then(toml::Value::as_str),
        Some("collapsed")
    );
    assert!(
        table.get("agent_message").is_none() && table.get("user_prompt").is_none(),
        "user and assistant messages are never stored by a bulk gesture"
    );

    let _ = press(&mut agent, KeyCode::Char('+'));
    let table = message_display_table(&global_file(&root));
    assert_eq!(table.len(), MessageKind::bulk_kinds().len());
    assert!(
        table
            .values()
            .all(|value| value.as_str() == Some("expanded"))
    );
    drop(agent);

    let restored = view(&root, Some("sess-a"));
    for &kind in MessageKind::bulk_kinds() {
        assert_eq!(
            form(&restored, kind),
            Some(DisplayForm::Expanded),
            "{kind:?}"
        );
    }
    for &kind in &[MessageKind::UserPrompt, MessageKind::AgentMessage] {
        assert_eq!(form(&restored, kind), None, "{kind:?} was never persisted");
    }
}

#[test]
fn a_session_value_equal_to_the_global_one_leaves_no_session_entry() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    flip(&mut agent, "File reads");
    let global_before = std::fs::read(global_file(&root)).expect("global file");

    let _ = toggle_display_scope(&mut agent);
    flip(&mut agent, "File reads");
    assert_eq!(
        stored(
            &read_toml(&session_file(&root, "sess-a")),
            "message_display",
            "read"
        ),
        Some("collapsed")
    );
    flip(&mut agent, "File reads");
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Expanded));
    assert_eq!(
        stored(
            &read_toml(&session_file(&root, "sess-a")),
            "message_display",
            "read"
        ),
        None,
        "back at the global value, the session holds nothing of its own"
    );
    assert_eq!(
        std::fs::read(global_file(&root)).expect("global file"),
        global_before,
        "session edits never rewrite the global file"
    );
}

#[test]
fn switching_scope_applies_values_immediately_and_preserves_the_other_scope() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    flip(&mut agent, "File reads");
    let _ = toggle_display_scope(&mut agent);
    flip(&mut agent, "File reads");
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Collapsed));
    let _ = toggle_display_scope(&mut agent);
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Expanded));
    flip(&mut agent, "File reads");
    flip(&mut agent, "File reads");
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Expanded));
    let _ = toggle_display_scope(&mut agent);
    assert_eq!(form(&agent, MessageKind::Read), Some(DisplayForm::Collapsed));
    focus_row(&mut agent, "File reads");
    assert!(sidebar_row(&draw(&mut agent), "File reads").contains("1 line"));
    assert_eq!(stored(&read_toml(&global_file(&root)), "message_display", "read"), Some("expanded"));
}

#[test]
fn moving_a_view_to_another_session_applies_that_sessions_settings() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    let mut b = view(&root, Some("sess-b"));
    open_f7(&mut b);
    flip(&mut b, "File reads");
    let _ = toggle_display_scope(&mut b);
    flip(&mut b, "File edits");
    drop(b);

    let mut agent = view(&root, None);
    assert_eq!(form(&agent, MessageKind::Edit), None);
    agent.bind_session_id(acp::SessionId::new("sess-b"));
    assert_eq!(
        form(&agent, MessageKind::Edit),
        Some(DisplayForm::Expanded),
        "late binding loads the session"
    );

    agent.bind_session_id(acp::SessionId::new("sess-c"));
    assert_eq!(
        form(&agent, MessageKind::Edit),
        None,
        "another session's override does not follow the view"
    );
    assert_eq!(
        form(&agent, MessageKind::Read),
        Some(DisplayForm::Expanded),
        "global stays"
    );
    open_f7(&mut agent);
    assert!(screen(&draw(&mut agent)).contains("display: global"));
}

#[test]
fn an_unreadable_settings_file_is_reported_when_the_view_opens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("panel-settings");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(global_file(&root), "this is = = not toml").expect("write");

    let agent = view(&root, Some("sess-a"));
    let message = toast(&agent);
    assert!(message.contains("defaults apply"), "{message}");
    assert!(message.contains("global.toml"), "{message}");
    assert_eq!(form(&agent, MessageKind::Read), None);
    assert_eq!(
        std::fs::read_to_string(global_file(&root)).expect("read"),
        "this is = = not toml",
        "opening leaves the file as it was"
    );
}

#[test]
fn a_failed_save_reports_its_cause_and_restores_the_effective_form() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("a-file");
    std::fs::write(&blocker, b"not a directory").expect("write blocker");
    let root = blocker.join("panel-settings");
    let reason = std::fs::read_to_string(global_file(&root))
        .expect_err("a path below a regular file cannot be read")
        .to_string();

    let mut agent = view(&root, Some("sess-a"));
    open_f7(&mut agent);
    flip(&mut agent, "File reads");

    let message = toast(&agent);
    assert!(message.contains("not saved"), "{message}");
    assert!(
        message.contains(&global_file(&root).display().to_string()),
        "{message}"
    );
    assert!(message.contains(&reason), "{message}");
    assert_eq!(
        form(&agent, MessageKind::Read),
        None,
        "the failed preference is not shown as effective"
    );
    assert!(sidebar_row(&draw(&mut agent), "File reads").contains("1 line"));
}
