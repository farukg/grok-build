//! F6 turn folds survive a restart: per session, and as a default for how a fresh panel starts.
//! Every test gives its views a store under a tempdir; none touches the real grok home.

use super::{
    click, column_text, conversation, draw_buffer, draw_frame, go_to, id_at, number_prompts,
    open_panel, panel_text, press, press_with, toast, turn_expanded,
};
use crate::app::agent_view::AgentView;
use crate::app::agent_view::test_fixtures::make_agent;
use crate::app::panel_settings::Layer;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::state::TimelineRowKey;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::{Path, PathBuf};

/// A view of `conversation()` whose two prompts carry the shell numbers 3 and 4, with its store under `root`.
fn restarted(root: &Path, session: Option<&str>) -> AgentView {
    let mut agent = conversation();
    number_prompts(&mut agent);
    agent.session.session_id = session.map(|id| id.to_owned().into());
    agent.open_panel_settings_in(root);
    agent
}

/// Prompts carrying exactly the shell numbers `ordinals`, each followed by one answer, laid out and scrolled to the top.
fn push_numbered_turns(agent: &mut AgentView, ordinals: &[usize]) {
    for &ordinal in ordinals {
        let id = agent
            .scrollback
            .push_block(RenderBlock::user_prompt(format!("Q{ordinal}")));
        if let Some(entry) = agent.scrollback.get_by_id_mut(id)
            && let RenderBlock::UserPrompt(prompt) = &mut entry.block
        {
            prompt.prompt_index = Some(ordinal);
        }
        agent
            .scrollback
            .push_block(RenderBlock::agent_message(format!("answer {ordinal}")));
    }
    draw_frame(agent);
    agent.scrollback.goto_top();
    draw_frame(agent);
}

/// A view whose prompts carry exactly the shell numbers `ordinals`, with its store under `root`.
fn numbered_agent(ordinals: &[usize], root: &Path, session: &str) -> AgentView {
    let mut agent = make_agent();
    push_numbered_turns(&mut agent, ordinals);
    agent.session.session_id = Some(session.to_owned().into());
    agent.open_panel_settings_in(root);
    agent
}

fn global_file(root: &Path) -> PathBuf {
    root.join("global.toml")
}

fn session_file(root: &Path, session: &str) -> PathBuf {
    root.join("sessions").join(format!("{session}.toml"))
}

/// The string at `keys` in the TOML file, read independently of the store.
fn stored(path: &Path, keys: &[&str]) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: toml::Value = toml::from_str(&text).expect("settings file is valid TOML");
    let value = keys.iter().try_fold(&file, |table, key| table.get(*key))?;
    value.as_str().map(str::to_owned)
}

fn exception(root: &Path, session: &str, ordinal: &str) -> Option<String> {
    stored(
        &session_file(root, session),
        &["outline", "turn_exceptions", ordinal],
    )
}

fn root_in(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("panel-settings")
}

#[test]
fn a_folded_turn_stays_folded_after_a_restart_in_its_session_only() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);

    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 6)));
    press(&mut agent, KeyCode::Left);
    assert!(!turn_expanded(&agent, id_at(&ids, 6)));
    assert_eq!(
        exception(&root, "sess-a", "4").as_deref(),
        Some("collapsed")
    );
    assert!(
        !global_file(&root).exists(),
        "a turn's fold is never a global setting"
    );
    drop(agent);

    let mut again = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut again);
    assert!(!turn_expanded(&again, id_at(&ids, 6)));
    assert!(turn_expanded(&again, id_at(&ids, 0)));
    let buf = draw_buffer(&mut again);
    let text = panel_text(&again, &buf);
    assert!(
        text.contains("Turn 2") && text.contains("Thinking pondering"),
        "{text}"
    );
    assert!(
        !text.contains("second paragraph"),
        "the folded turn's rows are not drawn: {text}"
    );

    let mut other = restarted(&root, Some("sess-b"));
    let ids = open_panel(&mut other);
    assert!(
        turn_expanded(&other, id_at(&ids, 6)),
        "another session does not see the fold"
    );
}

#[test]
fn the_stored_turn_default_decides_how_a_fresh_panel_starts() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(
        global_file(&root),
        "version = 1\n\n[outline]\nturns = \"collapsed\"\n",
    )
    .expect("write global settings");

    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    assert!(!turn_expanded(&agent, id_at(&ids, 0)));
    assert!(!turn_expanded(&agent, id_at(&ids, 6)));
    let buf = draw_buffer(&mut agent);
    let text = panel_text(&agent, &buf);
    assert!(text.contains("Turn 1") && text.contains("Turn 2"), "{text}");
    assert!(
        !text.contains("Thinking pondering") && !text.contains("second paragraph"),
        "no turn shows its rows: {text}"
    );

    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::Right);
    assert_eq!(
        exception(&root, "sess-a", "3").as_deref(),
        Some("expanded"),
        "opening one turn against the default is this session's exception"
    );
    assert_eq!(exception(&root, "sess-a", "4"), None);
    drop(agent);

    let mut again = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut again);
    assert!(turn_expanded(&again, id_at(&ids, 0)));
    assert!(!turn_expanded(&again, id_at(&ids, 6)));
}

#[test]
fn t_stores_the_turn_default_globally_and_every_other_session_follows_it() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);

    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('t'));
    assert_eq!(toast(&agent), Some("Turns start folded"));
    assert!(!turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)));
    assert_eq!(
        stored(&global_file(&root), &["outline", "turns"]).as_deref(),
        Some("collapsed")
    );
    assert!(
        !session_file(&root, "sess-a").exists(),
        "a global edit creates no session file"
    );

    let mut other = restarted(&root, Some("sess-b"));
    let other_ids = open_panel(&mut other);
    assert!(
        !turn_expanded(&other, id_at(&other_ids, 0))
            && !turn_expanded(&other, id_at(&other_ids, 6))
    );

    press(&mut agent, KeyCode::Char('t'));
    assert_eq!(toast(&agent), Some("Turns start open"));
    assert!(turn_expanded(&agent, id_at(&ids, 0)) && turn_expanded(&agent, id_at(&ids, 6)));
    assert_eq!(
        stored(&global_file(&root), &["outline", "turns"]),
        None,
        "the built-in default needs no entry"
    );
    let mut third = restarted(&root, Some("sess-c"));
    let third_ids = open_panel(&mut third);
    assert!(
        turn_expanded(&third, id_at(&third_ids, 0)) && turn_expanded(&third, id_at(&third_ids, 6))
    );
}

#[test]
fn the_shared_save_target_sends_the_turn_default_to_the_session_file_only() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);

    let mut agent = restarted(&root, Some("sess-a"));
    open_panel(&mut agent);
    let buf = draw_buffer(&mut agent);
    assert!(column_text(&agent, &buf).contains("saving: global (g)"));

    press(&mut agent, KeyCode::Char('g'));
    assert_eq!(agent.panel_persistence.edit_layer(), Layer::Session);
    let buf = draw_buffer(&mut agent);
    assert!(column_text(&agent, &buf).contains("saving: this session (g)"));
    assert!(!root.exists(), "choosing a target writes nothing");

    press(&mut agent, KeyCode::Char('t'));
    assert_eq!(
        stored(&session_file(&root, "sess-a"), &["outline", "turns"]).as_deref(),
        Some("collapsed")
    );
    assert!(!global_file(&root).exists());

    let mut same = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut same);
    assert!(!turn_expanded(&same, id_at(&ids, 0)) && !turn_expanded(&same, id_at(&ids, 6)));
    let mut other = restarted(&root, Some("sess-b"));
    let ids = open_panel(&mut other);
    assert!(turn_expanded(&other, id_at(&ids, 0)) && turn_expanded(&other, id_at(&ids, 6)));
}

#[test]
fn g_says_why_a_view_without_a_session_cannot_save_to_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let mut agent = restarted(&root, None);
    open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('g'));
    assert_eq!(agent.panel_persistence.edit_layer(), Layer::Global);
    let message = toast(&agent).expect("a toast");
    assert!(message.contains("no session yet"), "{message}");
}

#[test]
fn a_global_default_the_session_overrules_says_so_and_changes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('g'));
    press(&mut agent, KeyCode::Char('t'));
    assert!(!turn_expanded(&agent, id_at(&ids, 0)));
    press(&mut agent, KeyCode::Char('g'));
    press(&mut agent, KeyCode::Char('t'));
    let message = toast(&agent).expect("a toast");
    assert!(message.contains("has its own setting"), "{message}");
    assert!(
        !turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)),
        "the screen keeps what a restart would show"
    );
}

#[test]
fn a_view_without_a_store_still_folds_and_says_it_saves_nothing() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = conversation();
    let ids = open_panel(&mut agent);
    press(&mut agent, KeyCode::Char('t'));
    assert_eq!(toast(&agent), Some("Turns start folded"));
    assert!(!turn_expanded(&agent, id_at(&ids, 0)) && !turn_expanded(&agent, id_at(&ids, 6)));
    let buf = draw_buffer(&mut agent);
    assert!(column_text(&agent, &buf).contains("not saved"));
}

#[test]
fn every_gesture_that_folds_a_turn_is_saved() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    let saved = |ordinal: &str| exception(&root, "sess-a", ordinal);

    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 0)));
    press(&mut agent, KeyCode::Char(' '));
    assert_eq!(saved("3").as_deref(), Some("collapsed"), "Space");
    press(&mut agent, KeyCode::Char(' '));
    assert_eq!(saved("3"), None, "opening it again leaves no exception");

    click(&mut agent, 4, 0, KeyModifiers::NONE);
    assert_eq!(
        saved("4").as_deref(),
        Some("collapsed"),
        "click on the glyph"
    );

    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert_eq!(
        (saved("3"), saved("4")),
        (Some("collapsed".into()), Some("collapsed".into())),
        "Shift+Enter closes the other turn and preserves the collapsed anchor"
    );
    press_with(&mut agent, KeyCode::Enter, KeyModifiers::SHIFT);
    assert_eq!(
        (saved("3").as_deref(), saved("4").as_deref()),
        (None, Some("collapsed")),
        "Shift+Enter reopens the other turn and preserves the collapsed anchor"
    );

    press(&mut agent, KeyCode::Char('+'));
    assert_eq!((saved("3"), saved("4")), (None, None), "+");
    press(&mut agent, KeyCode::Char('*'));
    press(&mut agent, KeyCode::Char('*'));
    assert_eq!(
        (saved("3").as_deref(), saved("4").as_deref()),
        (Some("collapsed"), Some("collapsed")),
        "* closes every branch"
    );
    drop(agent);

    let mut again = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut again);
    assert!(!turn_expanded(&again, id_at(&ids, 0)) && !turn_expanded(&again, id_at(&ids, 6)));
}

#[test]
fn a_turn_without_a_prompt_number_is_not_saved() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let mut agent = conversation();
    agent.session.session_id = Some("sess-a".to_owned().into());
    agent.open_panel_settings_in(&root);
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 6)));
    press(&mut agent, KeyCode::Left);
    assert!(
        !turn_expanded(&agent, id_at(&ids, 6)),
        "the fold still applies on screen"
    );
    assert!(
        !root.exists(),
        "nothing identifies the turn across a restart"
    );
    assert_eq!(toast(&agent), None);
}

#[test]
fn opening_a_group_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Group(id_at(&ids, 2)));
    press(&mut agent, KeyCode::Right);
    assert!(
        !root.exists(),
        "group folds are live scrollback state shared with the chat"
    );
}

#[test]
fn a_failed_save_names_the_file_and_the_reason_and_the_fold_still_applies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("a-file");
    std::fs::write(&blocker, b"not a directory").expect("write blocker");
    let root = blocker.join("panel-settings");
    let file = session_file(&root, "sess-a");
    let reason = std::fs::read_to_string(&file)
        .expect_err("a path below a regular file cannot be read")
        .to_string();

    let mut agent = restarted(&root, Some("sess-a"));
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 6)));
    press(&mut agent, KeyCode::Left);

    let message = toast(&agent).expect("a toast");
    assert!(message.contains("not saved"), "{message}");
    assert!(message.contains(&file.display().to_string()), "{message}");
    assert!(message.contains(&reason), "{message}");
    assert!(!turn_expanded(&agent, id_at(&ids, 6)));
}

#[test]
fn removing_a_middle_turn_keeps_the_fold_on_the_same_prompt() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);

    let mut agent = numbered_agent(&[0, 1, 2], &root, "sess-a");
    let ids = open_panel(&mut agent);
    go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, 4)));
    press(&mut agent, KeyCode::Left);
    assert_eq!(
        exception(&root, "sess-a", "2").as_deref(),
        Some("collapsed")
    );

    agent.scrollback.remove_entry(id_at(&ids, 2));
    agent.scrollback.remove_entry(id_at(&ids, 3));
    let buf = draw_buffer(&mut agent);
    let text = panel_text(&agent, &buf);
    assert!(turn_expanded(&agent, id_at(&ids, 0)));
    assert!(!turn_expanded(&agent, id_at(&ids, 4)));
    assert!(
        text.contains("answer 0") && !text.contains("answer 2"),
        "{text}"
    );
    drop(agent);

    let mut again = numbered_agent(&[0, 2], &root, "sess-a");
    let ids = open_panel(&mut again);
    assert!(turn_expanded(&again, id_at(&ids, 0)));
    assert!(
        !turn_expanded(&again, id_at(&ids, 2)),
        "prompt 2 is still the folded one although it is now the second turn"
    );
    let buf = draw_buffer(&mut again);
    let text = panel_text(&again, &buf);
    assert!(
        text.contains("answer 0") && !text.contains("answer 2"),
        "{text}"
    );
}

#[test]
fn a_rewind_drops_the_saved_folds_of_the_discarded_prompt_numbers() {
    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);

    let mut agent = numbered_agent(&[0, 1, 2], &root, "sess-a");
    let ids = open_panel(&mut agent);
    for turn in [0, 2, 4] {
        go_to(&mut agent, TimelineRowKey::Turn(id_at(&ids, turn)));
        press(&mut agent, KeyCode::Left);
    }
    for ordinal in ["0", "1", "2"] {
        assert_eq!(
            exception(&root, "sess-a", ordinal).as_deref(),
            Some("collapsed"),
            "prompt {ordinal}"
        );
    }

    agent.clear_turn_folds_from(1);

    assert_eq!(
        exception(&root, "sess-a", "0").as_deref(),
        Some("collapsed"),
        "the kept prompt keeps its fold"
    );
    assert_eq!(exception(&root, "sess-a", "1"), None);
    assert_eq!(exception(&root, "sess-a", "2"), None);
}

#[test]
fn a_new_prompt_that_reuses_a_rewound_number_starts_open() {
    use crate::app::actions::{Action, TaskResult};
    use crate::app::agent::AgentId;
    use crate::views::rewind::RewindResponse;

    let _theme = crate::theme::cache::pin_theme();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = root_in(&dir);
    let id = AgentId(0);

    let mut app = crate::app::app_view::tests::test_app_with_agent();
    {
        let agent = app.agents.get_mut(&id).expect("test agent");
        push_numbered_turns(agent, &[0, 1, 2]);
        agent.open_panel_settings_in(&root);
        let ids = open_panel(agent);
        go_to(agent, TimelineRowKey::Turn(id_at(&ids, 4)));
        press(agent, KeyCode::Left);
    }
    assert_eq!(
        exception(&root, "test-session", "2").as_deref(),
        Some("collapsed")
    );

    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::RewindExecuteComplete {
            agent_id: id,
            response: RewindResponse {
                success: true,
                target_prompt_index: 1,
                reverted_files: Vec::new(),
                clean_files: Vec::new(),
                conflicts: Vec::new(),
                error: None,
                mode: Some("conversation_only".into()),
                prompt_text: None,
            },
        }),
        &mut app,
    );
    assert_eq!(exception(&root, "test-session", "2"), None);

    // The shell hands out number 2 again to the next prompt; a restart must show that turn open.
    let mut again = numbered_agent(&[0, 1, 2], &root, "test-session");
    let ids = open_panel(&mut again);
    assert!(turn_expanded(&again, id_at(&ids, 4)));
}
