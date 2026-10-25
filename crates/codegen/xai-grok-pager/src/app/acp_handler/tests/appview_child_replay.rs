//! Live child notifications and fresh disk replay through AppView activation.
use super::*;
use crate::app::actions::{Action, TreeStep};
use crate::app::agent_view::AgentRole;
use crate::app::app_view::{ActiveView, InputOutcome};
use crate::scrollback::blocks::ToolCallBlock;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use xai_grok_shell::session::info::Info;
use xai_grok_shell::session::storage::{SessionUpdate as StoredUpdate, StorageAdapter};
use xai_grok_shell::session::storage::jsonl::JsonlStorageAdapter;

const PARENT: &str = "replay-oracle-parent";
const CHILD: &str = "replay-oracle-child";
const BEFORE: &str = "Before reading the evidence.";
const RESULT: &str = "independent evidence\n";
const AFTER: &str = "The evidence confirms the answer.";

struct ReplayHome;

impl ReplayHome {
    fn install(path: &std::path::Path) -> Self {
        crate::app::subagent::set_replay_grok_home_for_tests(Some(path.to_path_buf()));
        Self
    }
}

impl Drop for ReplayHome {
    fn drop(&mut self) {
        crate::app::subagent::set_replay_grok_home_for_tests(None);
    }
}

fn event_meta(sid: &str, sequence: u64) -> acp::Meta {
    serde_json::json!({
        "eventId": format!("{sid}-{sequence}"),
        "agentTimestampMs": 1_750_000_000_000_i64 + sequence as i64,
    })
    .as_object()
    .unwrap()
    .clone()
}

fn lifecycle(update: XaiSessionUpdate, sequence: u64) -> StoredUpdate {
    StoredUpdate::Xai(Box::new(SessionNotification {
        session_id: acp::SessionId::new(PARENT),
        update,
        meta: Some(event_meta(PARENT, sequence)),
    }))
}

fn spawn_record() -> StoredUpdate {
    lifecycle(test_subagent_spawned(PARENT, CHILD), 1)
}

fn terminal_record() -> StoredUpdate {
    let mut update = test_subagent_finished(CHILD);
    let XaiSessionUpdate::SubagentFinished { tool_calls, .. } = &mut update else {
        panic!("terminal fixture must be SubagentFinished");
    };
    *tool_calls = 1;
    lifecycle(update, 2)
}

fn child_records() -> Vec<StoredUpdate> {
    let updates = [
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp_fixtures::text_block(BEFORE),
        )),
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(acp::ToolCallId::new("read-evidence"), "Read evidence.txt")
                .kind(acp::ToolKind::Read)
                .status(acp::ToolCallStatus::InProgress)
                .raw_input(serde_json::json!({"file_path": "evidence.txt"})),
        ),
        acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
            acp::ToolCallId::new("read-evidence"),
            acp::ToolCallUpdateFields::new()
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                    acp_fixtures::text_block(RESULT),
                ))]),
        )),
        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
            acp_fixtures::text_block(AFTER),
        )),
    ];
    updates
        .into_iter()
        .enumerate()
        .map(|(index, update)| {
            StoredUpdate::Acp(Box::new(
                acp_fixtures::session_notification(CHILD, update)
                    .meta(Some(event_meta(CHILD, index as u64 + 1))),
            ))
        })
        .collect()
}

fn deliver(app: &mut AppView, record: StoredUpdate, replay: bool) -> bool {
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let message = match record {
        StoredUpdate::Acp(mut notification) => {
            if replay {
                notification.meta.get_or_insert_default()
                    .insert("isReplay".into(), serde_json::Value::Bool(true));
            }
            AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
                request: *notification,
                response_tx: tx,
            })
        }
        StoredUpdate::Xai(mut notification) => {
            if replay {
                notification.meta.get_or_insert_default()
                    .insert("isReplay".into(), serde_json::Value::Bool(true));
            }
            AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: acp_fixtures::ext_notification("x.ai/session_notification", &*notification),
                response_tx: tx,
            })
        }
    };
    handle(message, app)
}

fn navigate(app: &mut AppView, step: TreeStep) {
    let code = match step {
        TreeStep::Parent => KeyCode::Up,
        TreeStep::LatestChild => KeyCode::Down,
    };
    let outcome = app.handle_input(&Event::Key(KeyEvent::new(
        code,
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    )));
    let InputOutcome::Action(Action::NavigateTree(actual)) = outcome else {
        panic!("tree navigation must remain available: {outcome:?}");
    };
    assert_eq!(actual, step);
    let _ = crate::app::dispatch::dispatch(Action::NavigateTree(actual), app);
}

fn assert_transcript(app: &AppView) {
    let child_id = app.agents.find_by_session_id(CHILD).expect("registered child session");
    assert_eq!(app.active_view, ActiveView::Agent(child_id));
    let child = app.agents.get(&child_id).unwrap();
    assert_eq!(child.session.session_id.as_ref().unwrap().0.as_ref(), CHILD);
    let AgentRole::Child(link) = &child.role else {
        panic!("child activation must not turn the child into a root");
    };
    assert_eq!(link.parent, Some(AgentId(0)));
    assert_eq!(link.parent_session_id.0.as_ref(), PARENT);
    assert_eq!(link.subagent_id, CHILD);
    assert!(child.session.state.is_idle());
    assert_eq!(child.scrollback.len(), 4, "two messages, one read result, one completion");
    assert!(child.scrollback.iter_entries().all(|(_, entry)| !entry.is_running));
    let first = &child.scrollback.entry(0).unwrap().block;
    assert!(matches!(first, RenderBlock::AgentMessage(_)));
    assert_eq!(first.copy_text(true).as_deref(), Some(BEFORE));
    let RenderBlock::ToolCall(ToolCallBlock::Read(read)) = &child.scrollback.entry(1).unwrap().block else {
        panic!("read result must remain between the two assistant messages");
    };
    assert_eq!(read.path, "evidence.txt");
    assert_eq!(read.content.as_deref(), Some(RESULT));
    assert_eq!(read.error, None);
    let last = &child.scrollback.entry(2).unwrap().block;
    assert!(matches!(last, RenderBlock::AgentMessage(_)));
    assert_eq!(last.copy_text(true).as_deref(), Some(AFTER));
    assert!(matches!(
        &child.scrollback.entry(3).unwrap().block,
        RenderBlock::SessionEvent(block)
            if matches!(&block.event, SessionEvent::TurnCompleted { elapsed }
                if *elapsed == Some(std::time::Duration::from_millis(500)))
    ));
    let parent = test_agent(app, AgentId(0));
    assert_eq!(parent.session.session_id.as_ref().unwrap().0.as_ref(), PARENT);
    let info = parent.subagent_sessions.get(CHILD).unwrap();
    assert!(info.is_finished());
    assert_eq!(info.attempt.status.as_deref(), Some("completed"));
    assert_eq!(info.attempt.tool_calls, Some(1));
    assert_eq!(info.attempt.turns, Some(1));
    assert_eq!(parent.scrollback.iter_entries().filter(|(_, entry)| matches!(
        &entry.block, RenderBlock::Subagent(block)
            if block.child_session_id == CHILD
                && matches!(&block.kind, SubagentBlockKind::Completed { .. })
    )).count(), 1, "terminal redelivery must not duplicate the parent's completion row");
}

fn normalized_frame(app: &mut AppView) -> Buffer {
    let (mut terminal, _frames) = crate::test_util::test_terminal();
    app.draw(&mut terminal);
    let mut frame = terminal.completed_buffer().clone();
    assert_eq!(frame.area, ratatui::layout::Rect::new(0, 0, 80, 24));
    let clock = regex::Regex::new(r"\b(?:[01][0-9]|2[0-3]):[0-5][0-9](?::[0-5][0-9])?\b").unwrap();
    for y in 0..frame.area.height {
        let symbols: Vec<String> = (0..frame.area.width)
            .map(|x| frame.cell((x, y)).unwrap().symbol().to_owned())
            .collect();
        let row = symbols.concat();
        // Completion timestamps are local wall-clock time; content and styles stay exact.
        for time in clock.find_iter(&row) {
            let mut byte = 0;
            for (x, symbol) in symbols.iter().enumerate() {
                if (time.start()..time.end()).contains(&byte)
                    && symbol.chars().all(|c| c.is_ascii_digit())
                {
                    frame.cell_mut((x as u16, y)).unwrap().set_symbol("0");
                }
                byte += symbol.len();
            }
        }
    }
    let text: String = frame.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains(BEFORE), "the captured frame must paint the first message");
    assert!(text.contains(AFTER), "the captured frame must paint the final message");
    frame
}

#[tokio::test(flavor = "current_thread")]
#[serial_test::serial(MEMORY_RELEASE_DEFER)]
async fn appview_live_child_matches_fresh_disk_replay_and_reopen() {
    let _theme = crate::theme::cache::pin_theme();
    let home = tempfile::tempdir().unwrap();
    let _replay_home = ReplayHome::install(home.path());
    let mut live = make_app_with_agent(PARENT);
    assert!(deliver(&mut live, spawn_record(), false));
    navigate(&mut live, TreeStep::LatestChild);
    for record in child_records() {
        assert!(deliver(&mut live, record, false));
    }
    assert!(deliver(&mut live, terminal_record(), false));
    assert_transcript(&live);
    let live_frame = normalized_frame(&mut live);

    let storage = JsonlStorageAdapter::with_root(home.path().to_path_buf());
    let parent_info = Info { id: acp::SessionId::new(PARENT), cwd: "/tmp".into() };
    let child_info = Info { id: acp::SessionId::new(CHILD), cwd: "/tmp".into() };
    for info in [&parent_info, &child_info] {
        storage.init_session(info, acp::ModelId::new("test-model")).await.unwrap();
    }
    // Fresh records, not serialization of the already-mutated live view.
    for record in [spawn_record(), terminal_record()] {
        storage.append_update(&parent_info, &record).await.unwrap();
    }
    for record in child_records() {
        storage.append_update(&child_info, &record).await.unwrap();
    }
    let parent_records = storage.load_session(&parent_info).await.unwrap().updates;
    assert_eq!(parent_records.len(), 2);
    let terminal = parent_records.last().unwrap().clone();
    let mut replayed = make_app_with_agent(PARENT);
    replayed.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = true;
    for record in parent_records {
        deliver(&mut replayed, record, true);
    }
    replayed.agents.get_mut(&AgentId(0)).unwrap().session.loading_replay = false;
    let reads = crate::app::subagent::test_support::transcript_reads();
    navigate(&mut replayed, TreeStep::LatestChild);
    assert!(crate::app::subagent::test_support::transcript_reads() > reads,
        "fresh AppView activation must actually read the persisted child");
    assert_transcript(&replayed);
    assert_eq!(normalized_frame(&mut replayed), live_frame);

    navigate(&mut replayed, TreeStep::Parent);
    assert_eq!(replayed.active_view, ActiveView::Agent(AgentId(0)));
    let child_id = replayed.agents.find_by_session_id(CHILD).unwrap();
    assert!(replayed.agents.get(&child_id).unwrap().scrollback.is_empty(),
        "leaving the disk-backed finished child must exercise eviction");
    let reads = crate::app::subagent::test_support::transcript_reads();
    navigate(&mut replayed, TreeStep::LatestChild);
    assert!(crate::app::subagent::test_support::transcript_reads() > reads,
        "reopening must exercise replay, not a retained copy");
    assert_transcript(&replayed);
    assert_eq!(normalized_frame(&mut replayed), live_frame);
    deliver(&mut replayed, terminal, false);
    assert_transcript(&replayed);
    assert_eq!(normalized_frame(&mut replayed), live_frame);
}
