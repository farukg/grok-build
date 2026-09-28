use super::test_support::make_info;
use super::*;
use crate::acp::meta::NotificationMeta;
use crate::acp::model_state::ModelState;
use crate::acp::tracker::AcpUpdateTracker;
use crate::app::agent::{AgentId, AgentSession, AgentState};
use crate::app::agent_view::AgentView;
use crate::app::session_views::SessionViews;
use crate::scrollback::block::RenderBlock;
use crate::scrollback::state::ScrollbackState;
use agent_client_protocol as acp;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
fn make_min_child_view() -> AgentView {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let session = AgentSession {
        id: AgentId(0),
        acp_tx: tx,
        session_id: Some(acp::SessionId::new(Arc::from("child"))),
        models: ModelState::default(),
        state: AgentState::Idle,
        tracker: AcpUpdateTracker::new(),
        cwd: PathBuf::from("/tmp"),
        is_worktree: false,
        forked_from: None,
        pending_prompts: VecDeque::new(),
        next_queue_id: 0,
        yolo_mode: false,
        auto_mode: false,
        prompt_history: Vec::new(),
        prompt_history_loading: false,
        loading_replay: false,
        restore_degree: None,
        rate_limited: false,
        model_incompatible: false,
        credit_limit_blocked: false,
        free_usage_blocked: false,
        available_commands: Vec::new(),
        available_commands_generation: 0,
        available_tools: None,
        model_switch_pending: false,
        hook_block_hold: false,
        blocked_prompt: None,
        user_model_preference: None,
        deferred_model_switch: None,
        bg_tasks: BTreeMap::new(),
        bg_tool_call_to_task: HashMap::new(),
        scheduled_tasks: HashMap::new(),
        in_flight_prompt: None,
        compact_held_prompt: None,
        current_prompt_id: None,
        created_via_new: false,
    };
    AgentView::new(session, ScrollbackState::new())
}
fn session_views(sid: &str, child: AgentView, info: SubagentInfo) -> (SessionViews, AgentId) {
    let parent_id = AgentId(0);
    let child_id = AgentId(1);
    let mut views = SessionViews::new();
    let mut parent = make_min_child_view();
    parent.session.id = parent_id;
    parent.session.session_id = Some(acp::SessionId::new("parent"));
    parent.subagent_sessions.insert(sid.to_owned(), info);
    views.insert(parent_id, parent);
    let mut child = child;
    child.session.session_id = Some(acp::SessionId::new(sid));
    crate::app::session_views::test_support::link_child(
        &mut views,
        parent_id,
        child_id,
        child,
        std::time::Instant::now(),
    );
    (views, child_id)
}
fn seed_tool_call(view: &mut AgentView) {
    view.session.tracker.handle_update(
        acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(acp::ToolCallId::new(Arc::from("tc1")), "Read foo")
                .kind(acp::ToolKind::Other)
                .status(acp::ToolCallStatus::Pending)
                .content(vec![])
                .locations(vec![]),
        ),
        &NotificationMeta::default(),
        &mut view.scrollback,
    );
}
#[test]
fn scrollback_is_footer_only_classifies_content() {
    let empty = make_min_child_view();
    assert!(scrollback_is_footer_only(&empty.scrollback), "empty");
    let mut footer = make_min_child_view();
    footer.scrollback.push_block(RenderBlock::session_event(
        crate::scrollback::blocks::SessionEvent::TurnCompleted { elapsed: None },
    ));
    assert!(scrollback_is_footer_only(&footer.scrollback), "footer");
    let mut prompt = make_min_child_view();
    prompt
        .scrollback
        .push_block(RenderBlock::user_prompt("scan src/"));
    assert!(
        !scrollback_is_footer_only(&prompt.scrollback),
        "echoed prompt is content"
    );
    let mut tool = make_min_child_view();
    seed_tool_call(&mut tool);
    assert!(!scrollback_is_footer_only(&tool.scrollback), "tool call");
}
#[test]
fn a_running_child_whose_view_holds_only_the_echoed_prompt_is_not_replayed() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-echo-holds";
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let echo = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"scan src/"}}}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), echo + "\n").unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut child = make_min_child_view();
    child.session.session_id = Some(acp::SessionId::new(child_sid));
    child.session.tracker.handle_update(
        acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
            acp::TextContent::new("scan src/"),
        ))),
        &NotificationMeta::default(),
        &mut child.scrollback,
    );
    assert_eq!(child.scrollback.len(), 1);
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    let (mut views, child_id) = session_views(child_sid, child, info);
    let before = test_support::transcript_reads();
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::ViewHoldsLiveBlocks,
        "a live echo closes the replay window like any other live block"
    );
    assert_eq!(test_support::transcript_reads(), before, "disk is not read");
    let child = views.get(&child_id).unwrap();
    assert_eq!(
        child.scrollback.len(),
        1,
        "the prompt is painted exactly once"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn finished_child_evicted_on_leave_and_replayed_on_open() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-evict-replay";
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let line = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"persisted"}}}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), format!("{line}\n")).unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    info.set_finished_for_test(true);
    let child = make_min_child_view();
    let (mut views, child_id) = session_views(child_sid, child, info);
    assert_eq!(replay_on_open(&mut views, child_id), ChildReplayOutcome::Replayed);
    assert_eq!(evict_on_leave(&mut views, child_id), EvictOutcome::Evicted);
    assert!(
        views.get(&child_id).is_some_and(|child| child.scrollback.is_empty()),
        "eviction drops the rebuilt content, never the session view"
    );
    assert_eq!(replay_on_open(&mut views, child_id), ChildReplayOutcome::Replayed);
    assert_eq!(views.get(&child_id).unwrap().scrollback.len(), 1);
    set_replay_grok_home_for_tests(None);
}

#[test]
fn a_disk_backed_child_is_not_replayed_again() {
    let child_sid = "child-skip";
    let mut child = make_min_child_view();
    child.session.session_id = Some(acp::SessionId::new(child_sid));
    child
        .scrollback
        .push_block(RenderBlock::user_prompt("task only"));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    info.transcript = ChildTranscript::DiskBacked;
    let (mut views, child_id) = session_views(child_sid, child, info);
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::NothingToRead
    );
    let child = views.get(&child_id).unwrap();
    assert_eq!(child.scrollback.len(), 1);
    assert!(matches!(
        child.scrollback.entry(0).unwrap().block,
        RenderBlock::UserPrompt(_)
    ));
}
#[test]
fn enter_on_a_subagent_row_opens_the_child_session_view() {
    use crate::app::agent_view::test_fixtures::make_agent;
    use crate::app::app_view::{ActiveView, AppView};
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = AppView::new(
        tx,
        crate::acp::model_state::ModelState::default(),
        Vec::new(),
        crate::render::draw::EscapeWriter::disconnected(),
    );
    let parent_id = AgentId(0);
    let child_id = AgentId(1);
    let child_sid = "linked-child";
    let mut parent = make_agent();
    parent.session.id = parent_id;
    parent.session.session_id = Some(acp::SessionId::new("parent"));
    parent.scrollback.push_block(RenderBlock::Subagent(
        crate::scrollback::blocks::SubagentBlock::started(
            "child output",
            child_sid,
            "explore",
            None,
            None,
            None,
            false,
        ),
    ));
    let mut info = make_info();
    info.transcript = ChildTranscript::DiskBacked;
    parent.subagent_sessions.insert(child_sid.to_owned(), info);
    app.agents.insert(parent_id, parent);
    let mut child = make_agent();
    child.session.session_id = Some(acp::SessionId::new(child_sid));
    crate::app::session_views::test_support::link_child(
        &mut app.agents,
        parent_id,
        child_id,
        child,
        std::time::Instant::now(),
    );
    app.active_view = ActiveView::Agent(parent_id);
    let parent = app.agents.get_mut(&parent_id).unwrap();
    parent.set_active_pane(crate::app::agent_view::AgentPane::Scrollback, false);
    parent.scrollback.set_selected(Some(0));
    let enter = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    let crate::app::app_view::InputOutcome::Action(action) = app.handle_input(&enter) else {
        panic!("Enter on a subagent row must request an action");
    };
    let _ = crate::app::dispatch::dispatch(action, &mut app);
    assert_eq!(app.active_view, ActiveView::Agent(child_id));
}

#[test]
fn replay_reports_live_blocks_and_unknown_children_distinctly() {
    let child_sid = "child-live-blocks";
    let mut child = make_min_child_view();
    child.session.session_id = Some(acp::SessionId::new(child_sid));
    seed_tool_call(&mut child);
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    let (mut views, child_id) = session_views(child_sid, child, info);
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::ViewHoldsLiveBlocks
    );
    assert_eq!(
        replay_on_open(&mut views, AgentId(99)),
        ChildReplayOutcome::UnknownChild
    );
}
#[test]
fn empty_read_of_a_running_child_is_cached_until_it_finishes() {
    let child_sid = "child-empty-cache";
    let child = make_min_child_view();
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    let (mut views, child_id) = session_views(child_sid, child, info);
    let home = tempfile::tempdir().unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let before = test_support::transcript_reads();
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::FoundNothingOnDisk
    );
    assert_eq!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .map(|s| &s.transcript),
        Some(&ChildTranscript::DiskEmptyWhileRunning)
    );
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::NothingToRead
    );
    assert_eq!(
        test_support::transcript_reads(),
        before + 1,
        "the cached empty result must not re-read the transcript"
    );
    views
        .get_mut(&AgentId(0))
        .unwrap()
        .subagent_sessions
        .get_mut(child_sid)
        .unwrap()
        .transcript
        .retry_disk_after_finish();
    assert_eq!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .map(|s| &s.transcript),
        Some(&ChildTranscript::NeedsReplay),
        "the finish must allow one more read for a late persistence flush"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn an_empty_read_of_a_running_resumed_child_stays_needs_replay_and_retries() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-resumed-empty";
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    info.attempt.context = xai_grok_shell::extensions::subagent_context::SubagentContext::Resumed;
    let (mut views, child_id) = session_views(child_sid, make_min_child_view(), info);
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::FoundNothingOnDisk
    );
    assert_eq!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .map(|s| &s.transcript),
        Some(&ChildTranscript::NeedsReplay),
        "a resumed child's empty-while-running read must not settle: its inherited history is expected on disk"
    );
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let tool_line = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call","toolCallId":"tc1","title":"Read foo","kind":"read","locations":[{{"path":"/tmp/foo"}}]}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), tool_line + "\n").unwrap();
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::Replayed,
        "the retry after the transcript flushes must replay the inherited prefix"
    );
    assert_eq!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .map(|s| &s.transcript),
        Some(&ChildTranscript::DiskBacked)
    );
    let child = views.get(&child_id).unwrap();
    let tools = (0..child.scrollback.len())
        .filter(|i| {
            child
                .scrollback
                .entry(*i)
                .is_some_and(|e| matches!(e.block, RenderBlock::ToolCall(_)))
        })
        .count();
    assert_eq!(
        tools, 1,
        "the inherited tool call must appear after the retry"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn a_child_replay_releases_retained_memory_only_once() {
    use crate::memory_release::test_support;
    test_support::install_counting_hook();
    let child_sid = "child-purge-real";
    let home = tempfile::tempdir().unwrap();
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let tool_line = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call","toolCallId":"tc1","title":"Read foo","kind":"read","locations":[{{"path":"/tmp/foo"}}]}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), tool_line + "\n").unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    let child = make_min_child_view();
    let (mut views, child_id) = session_views(child_sid, child, info);
    let before = test_support::calls();
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::Replayed,
        "an emitting replay returns Replayed"
    );
    assert_eq!(
        test_support::calls(),
        before + 1,
        "a real replay must purge after the parsed transient drops"
    );
    assert!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .is_some_and(|s| !s.transcript.needs_replay()),
        "fixture sanity: the emitting replay must record the disk copy"
    );
    let before = test_support::calls();
    let _ = replay_on_open(&mut views, child_id);
    assert_eq!(
        test_support::calls(),
        before,
        "the skip path allocates nothing and must not purge"
    );
    let ghost_sid = "child-purge-ghost";
    let ghost = make_info();
    let ghost_child = make_min_child_view();
    let (mut ghost_views, ghost_id) = session_views(ghost_sid, ghost_child, ghost);
    let before = test_support::calls();
    let _ = replay_on_open(&mut ghost_views, ghost_id);
    assert_eq!(
        test_support::calls(),
        before,
        "a no-op replay (missing transcript) must not purge"
    );
    let empty_sid = "child-purge-empty";
    let empty_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(empty_sid);
    std::fs::create_dir_all(&empty_dir).unwrap();
    std::fs::write(empty_dir.join("summary.json"), "{}").unwrap();
    std::fs::write(empty_dir.join("updates.jsonl"), "").unwrap();
    let empty = make_info();
    let empty_child = make_min_child_view();
    let (mut empty_views, empty_id) = session_views(empty_sid, empty_child, empty);
    let before = test_support::calls();
    let _ = replay_on_open(&mut empty_views, empty_id);
    assert_eq!(
        test_support::calls(),
        before,
        "an empty replay (zero updates parsed) must not purge"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn rebuilt_child_transcript_keeps_persisted_timestamps_not_the_rebuild_time() {
    use chrono::TimeZone;
    let child_sid = "child-timestamps";
    let prompt_ms: i64 = 1_700_000_000_000;
    let msg_ms: i64 = 1_700_000_060_000;
    let home = tempfile::tempdir().unwrap();
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let echo = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"scan src/"}}}},"_meta":{{"agentTimestampMs":{prompt_ms},"turnStartMs":{prompt_ms}}}}}}}"#
    );
    let msg = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"done"}}}},"_meta":{{"agentTimestampMs":{msg_ms}}}}}}}"#
    );
    std::fs::write(
        session_dir.join("updates.jsonl"),
        format!("{echo}\n{msg}\n"),
    )
    .unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    info.set_finished_for_test(true);
    info.attempt.duration_ms = Some(1_000);
    let (mut views, child_id) = session_views(child_sid, make_min_child_view(), info);
    let _ = replay_on_open(&mut views, child_id);
    let expected = |ms: i64| {
        chrono::Utc
            .timestamp_millis_opt(ms)
            .single()
            .unwrap()
            .with_timezone(&chrono::Local)
    };
    let child = views.get(&child_id).unwrap();
    let mut prompts_seen = 0;
    let mut msg_seen = false;
    for i in 0..child.scrollback.len() {
        let entry = child.scrollback.entry(i).unwrap();
        match &entry.block {
            RenderBlock::UserPrompt(_) => {
                assert_eq!(
                    entry.created_at,
                    Some(expected(prompt_ms)),
                    "replayed task prompt must carry the persisted turn start, not the rebuild time"
                );
                prompts_seen += 1;
            }
            RenderBlock::AgentMessage(_) => {
                assert_eq!(
                    entry.created_at,
                    Some(expected(msg_ms)),
                    "replayed agent message must carry the persisted timestamp, not the rebuild time"
                );
                msg_seen = true;
            }
            _ => {}
        }
    }
    assert_eq!(
        prompts_seen, 1,
        "the persisted echo is the one writer of the task prompt"
    );
    assert!(msg_seen, "fixture must produce an agent message entry");
    set_replay_grok_home_for_tests(None);
}
#[test]
fn a_replayed_transcript_collapses_a_tool_call_and_its_updates() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-batch";
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let user = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"go"}}}}}}}}"#
    );
    let tool = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call","toolCallId":"t1","title":"bash","kind":"execute","status":"pending"}}}}}}"#
    );
    let ip = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"in_progress","content":[{{"type":"text","text":"out"}}]}}}}}}"#
    );
    let done = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{{"type":"text","text":"out"}}]}}}}}}"#
    );
    let agent_msg = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"ok"}}}}}}}}"#
    );
    std::fs::write(
        session_dir.join("updates.jsonl"),
        format!("{user}\n{tool}\n{ip}\n{done}\n{agent_msg}\n"),
    )
    .unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut view = make_min_child_view();
    assert!(matches!(
        replay_inherited_updates(
            &mut view,
            child_sid,
            std::path::Path::new("/tmp"),
            None,
            ReplayLookupFallback::Relocation,
        ),
        Ok(ReplayEmission::Emitted)
    ));
    assert!(
        !view.scrollback.in_batch(),
        "end_batch must run after streamed apply"
    );
    assert_eq!(
        view.scrollback.turn_count(),
        1,
        "end_batch must rebuild turns once after the stream"
    );
    let tools = (0..view.scrollback.len())
        .filter(|i| {
            view.scrollback
                .entry(*i)
                .is_some_and(|e| matches!(e.block, RenderBlock::ToolCall(_)))
        })
        .count();
    assert_eq!(tools, 1, "ToolCall+updates must collapse to one block");
    set_replay_grok_home_for_tests(None);
}
#[test]
fn a_read_error_reports_read_failed_and_closes_the_scrollback_batch() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-read-err";
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    std::fs::create_dir(session_dir.join("updates.jsonl")).unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut view = make_min_child_view();
    assert!(
        replay_inherited_updates(
            &mut view,
            child_sid,
            std::path::Path::new("/tmp"),
            None,
            ReplayLookupFallback::Relocation,
        )
        .is_err()
    );
    assert!(
        !view.scrollback.in_batch(),
        "end_batch must run after a read error"
    );
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    let (mut views, child_id) = session_views(child_sid, view, info);
    assert_eq!(
        replay_on_open(&mut views, child_id),
        ChildReplayOutcome::ReadFailed,
        "a broken transcript surfaces as ReadFailed so the next open retries"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn a_replay_locates_the_transcript_via_the_child_cwd_hint() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-wt-hint";
    let child_cwd = "/work/wt";
    let session_dir = home
        .path()
        .join("sessions")
        .join(xai_grok_config::encode_cwd_dirname(child_cwd))
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let user = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"from-wt"}}}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), format!("{user}\n")).unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut view = make_min_child_view();
    assert!(matches!(
        replay_inherited_updates(
            &mut view,
            child_sid,
            std::path::Path::new("/tmp"),
            Some(std::path::Path::new(child_cwd)),
            ReplayLookupFallback::Relocation,
        ),
        Ok(ReplayEmission::Emitted)
    ));
    assert_ne!(
        view.scrollback.len(),
        0,
        "child_cwd hint must locate the worktree transcript"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn child_view_for_live_update_hydrates_a_resumed_child_before_returning_it() {
    let home = tempfile::tempdir().unwrap();
    let child_sid = "child-live-update-hydrate";
    let session_dir = home
        .path()
        .join("sessions")
        .join(urlencoding::encode("/tmp").as_ref())
        .join(child_sid);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("summary.json"), "{}").unwrap();
    let tool_line = format!(
        r#"{{"method":"session/update","params":{{"sessionId":"{child_sid}","update":{{"sessionUpdate":"tool_call","toolCallId":"tc1","title":"Read foo","kind":"read","locations":[{{"path":"/tmp/foo"}}]}}}}}}"#
    );
    std::fs::write(session_dir.join("updates.jsonl"), tool_line + "\n").unwrap();
    set_replay_grok_home_for_tests(Some(home.path().to_path_buf()));
    let mut info = make_info();
    info.child_session_id = child_sid.into();
    info.attempt.context = xai_grok_shell::extensions::subagent_context::SubagentContext::Resumed;
    let (mut views, child_id) = session_views(child_sid, make_min_child_view(), info);
    assert_eq!(replay_on_open(&mut views, child_id), ChildReplayOutcome::Replayed);
    {
        let view = views.get(&child_id).unwrap();
        let tools = (0..view.scrollback.len())
            .filter(|i| {
                view.scrollback
                    .entry(*i)
                    .is_some_and(|e| matches!(e.block, RenderBlock::ToolCall(_)))
            })
            .count();
        assert_eq!(
            tools, 1,
            "the accessor must replay a resumed child before handing back its view for a live block"
        );
    }
    assert_eq!(
        views
            .get(&AgentId(0))
            .and_then(|parent| parent.subagent_sessions.get(child_sid))
            .map(|s| &s.transcript),
        Some(&ChildTranscript::DiskBacked),
        "the hydrate records the proven disk copy"
    );
    set_replay_grok_home_for_tests(None);
}
#[test]
fn accepted_attempt_invalidates_prior_disk_proof() {
    for transcript in [
        ChildTranscript::DiskBacked,
        ChildTranscript::DiskEmptyWhileRunning,
    ] {
        let mut prior = make_info();
        prior.transcript = transcript;
        let child = SubagentChildInfo {
            subagent_id: prior.subagent_id.to_string(),
            child_session_id: prior.child_session_id.to_string(),
            description: prior.description.to_string(),
            subagent_type: prior.subagent_type.to_string(),
        };
        let replacement = make_info().attempt;
        let info = SubagentInfo::from_spawn(Some(prior), child, replacement, true);
        assert_eq!(info.transcript, ChildTranscript::NeedsReplay);
    }
}
