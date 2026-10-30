use super::*;
use crate::actions::ActionRegistry;
use crate::scrollback::render::ScratchBuffer;
use crate::scrollback::blocks::tool::ToolCallBlock;
use ratatui::{buffer::Buffer, layout::Rect};
use serde_json::{Value, json};

struct Transcript(tempfile::TempDir);

impl Transcript {
    fn new(lines: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("sessions")
            .join(urlencoding::encode("/tmp").as_ref()).join("row-time-child");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("summary.json"), "{}").unwrap();
        std::fs::write(dir.join("updates.jsonl"), lines).unwrap();
        Self(home)
    }

    fn load(&self) -> (SessionViews, AgentId) {
        set_replay_grok_home_for_tests(Some(self.0.path().to_path_buf()));
        let mut info = make_info();
        info.child_session_id = "row-time-child".into();
        info.set_finished_for_test(true);
        info.transcript = ChildTranscript::DiskBacked;
        let (mut views, child) = session_views("row-time-child", make_min_child_view(), info);
        assert_eq!(replay_on_open(&mut views, child), ChildReplayOutcome::Replayed);
        (views, child)
    }
}

impl Drop for Transcript {
    fn drop(&mut self) {
        set_replay_grok_home_for_tests(None);
    }
}

fn draw(view: &mut AgentView) -> String {
    let area = Rect::new(0, 0, 140, 60);
    let mut buffer = Buffer::empty(area);
    view.draw(
        area, &mut buffer, &ActionRegistry::defaults(), &mut ScratchBuffer::new(),
        None, false, crate::app::agent_view::BannerSlotParams::none(), false,
        &mut Vec::new(), Default::default(),
    );
    (0..area.height).map(|y| {
        (0..area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n"
    }).collect()
}

fn row_times(view: &AgentView) -> Vec<Option<i64>> {
    (0..view.scrollback.len()).filter_map(|index| {
        let entry = view.scrollback.entry(index).unwrap();
        matches!(entry.block,
            RenderBlock::Thinking(_) | RenderBlock::AgentMessage(_) | RenderBlock::ToolCall(_)
        ).then(|| entry.created_at.map(|time| time.timestamp_millis()))
    }).collect()
}

fn line(update: acp::SessionUpdate, meta: Value) -> String {
    json!({"timestamp":1793393760000_i64,"method":"session/update","params":{
        "sessionId":"row-time-child","update":update,"_meta":meta
    }}).to_string() + "\n"
}

fn tool(id: &str) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(acp::ToolCallId::new(id.to_owned()), "write")
            .kind(acp::ToolKind::Other).status(acp::ToolCallStatus::Pending),
    )
}

fn tool_update(id: &str, fields: acp::ToolCallUpdateFields) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new(id.to_owned()), fields,
    ))
}

#[test]
fn persisted_child_draw_and_reload_keep_original_row_times() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    let fixture = Transcript::new(include_str!("fixtures/row_timestamp_child.jsonl"));
    for _ in 0..2 {
        let (mut views, child) = fixture.load();
        let view = views.get_mut(&child).unwrap();
        assert_eq!(row_times(view), vec![
            Some(1_793_367_926_179), Some(1_793_367_926_469), Some(1_793_367_966_934),
        ]);
        let thinking = (0..view.scrollback.len()).find_map(|index| {
            match &view.scrollback.entry(index).unwrap().block {
                RenderBlock::Thinking(block) => Some(block),
                _ => None,
            }
        }).unwrap();
        assert_eq!(thinking.elapsed_time_ms(), Some(37_088));
        let painted = draw(view);
        assert!(painted.contains("Let me rewrite the test file cleanly"), "{painted}");
        assert_eq!(row_times(view), vec![
            Some(1_793_367_926_179), Some(1_793_367_926_469), Some(1_793_367_966_934),
        ]);
    }
}

#[test]
fn late_completion_and_eof_keep_registration_not_terminal_time() {
    let lines = line(tool("late"), json!({"agentTimestampMs":1793367966934_i64}))
        + &line(tool_update("late", acp::ToolCallUpdateFields::new()
            .title(Some("renamed write".into()))), json!({"agentTimestampMs":1793370000000_i64}))
        + &line(tool_update("late", acp::ToolCallUpdateFields::new()
            .status(Some(acp::ToolCallStatus::Completed))), json!({"agentTimestampMs":1793390000000_i64}))
        + &line(tool("eof"), json!({"agentTimestampMs":1793368000000_i64}))
        + &line(tool_update("eof", acp::ToolCallUpdateFields::new()
            .title(Some("unfinished write".into()))), json!({"agentTimestampMs":1793391000000_i64}));
    let fixture = Transcript::new(&lines);
    for _ in 0..2 {
        let (mut views, child) = fixture.load();
        let view = views.get_mut(&child).unwrap();
        draw(view);
        assert_eq!(row_times(view), vec![Some(1_793_367_966_934), Some(1_793_368_000_000)]);
    }
}

#[test]
fn legacy_and_invalid_historical_origins_stay_unknown() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    for meta in [json!({}), json!({"agentTimestampMs":"invalid"}),
        json!({"agentTimestampMs":i64::MAX})] {
        let lines = line(acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
            acp::ContentBlock::from("legacy thought"))), meta.clone())
            + &line(acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                acp::ContentBlock::from("legacy answer"))), meta.clone())
            + &line(tool("legacy"), meta)
            + &line(tool_update("legacy", acp::ToolCallUpdateFields::new()
                .status(Some(acp::ToolCallStatus::Completed))), json!({"agentTimestampMs":1793390000000_i64}));
        let fixture = Transcript::new(&lines);
        let (mut views, child) = fixture.load();
        let view = views.get_mut(&child).unwrap();
        draw(view);
        assert_eq!(row_times(view), vec![None, None, None]);
    }
}

#[test]
fn live_fallback_and_followup_updates_do_not_rebind_rows() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    let mut view = make_min_child_view();
    let before = chrono::Local::now().timestamp_millis();
    let live = NotificationMeta::default();
    let thought = || acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
        acp::ContentBlock::from("thinking")));
    let answer = || acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
        acp::ContentBlock::from("answer")));
    view.session.handle_update(thought(), &live, &mut view.scrollback);
    let first_thought = row_times(&view)[0];
    let later = NotificationMeta { agent_timestamp_ms: Some(1_893_390_000_000), ..Default::default() };
    view.session.handle_update(thought(), &later, &mut view.scrollback);
    view.session.handle_update(answer(), &live, &mut view.scrollback);
    let first_answer = row_times(&view)[1];
    view.session.handle_update(answer(), &later, &mut view.scrollback);
    view.session.handle_update(tool("live"), &live, &mut view.scrollback);
    let registered = row_times(&view)[2];
    view.session.handle_update(tool_update("live", acp::ToolCallUpdateFields::new()
        .title(Some("new title".into()))), &later, &mut view.scrollback);
    view.session.handle_update(tool_update("live", acp::ToolCallUpdateFields::new()
        .status(Some(acp::ToolCallStatus::Completed))), &later, &mut view.scrollback);
    let after = chrono::Local::now().timestamp_millis();
    assert_eq!(row_times(&view), vec![first_thought, first_answer, registered]);
    for time in row_times(&view) {
        let time = time.unwrap();
        assert!(time >= before && time <= after);
    }
    draw(&mut view);
}

#[test]
fn streaming_origins_survive_deltas_and_hidden_tool_failure() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    let mut view = make_min_child_view();
    let thought = || acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
        acp::ContentBlock::from("thought delta")));
    let answer = || acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
        acp::ContentBlock::from("answer delta")));
    let original = NotificationMeta { agent_timestamp_ms: Some(1_793_367_926_179),
        is_replay: true, ..Default::default() };
    let later = NotificationMeta { agent_timestamp_ms: Some(1_793_390_000_000),
        is_replay: true, ..Default::default() };
    view.session.handle_update(thought(), &original, &mut view.scrollback);
    view.session.handle_update(thought(), &later, &mut view.scrollback);
    let original = NotificationMeta { agent_timestamp_ms: Some(1_793_367_926_469),
        is_replay: true, ..Default::default() };
    view.session.handle_update(answer(), &original, &mut view.scrollback);
    view.session.handle_update(answer(), &later, &mut view.scrollback);
    let hidden = acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(acp::ToolCallId::new("hidden"), "todo_write")
            .status(acp::ToolCallStatus::Pending),
    );
    let original = NotificationMeta { agent_timestamp_ms: Some(1_793_367_966_934),
        is_replay: true, ..Default::default() };
    view.session.handle_update(hidden, &original, &mut view.scrollback);
    assert_eq!(row_times(&view).len(), 2);
    view.session.handle_update(tool_update("hidden", acp::ToolCallUpdateFields::new()
        .status(Some(acp::ToolCallStatus::Failed))), &later, &mut view.scrollback);
    assert_eq!(row_times(&view), vec![Some(1_793_367_926_179),
        Some(1_793_367_926_469), Some(1_793_367_966_934)]);
    draw(&mut view);
}

#[test]
fn owned_child_replay_keeps_registration_separate_from_execution_duration() {
    use xai_grok_shell::extensions::tool_timing::{
        ToolCallOwnership, ToolCallSource, ToolExecutionId, ToolExecutionTermination,
        ToolExecutionTiming, stamp_ownership, timing_meta,
    };
    let owner = ToolCallOwnership::Registered {
        invocation_id: "9b699562-00ba-4011-94f8-0cc8c19e7322".into(),
        provider_call_id: "owned".into(), source: ToolCallSource::Unknown,
    };
    let mut registration = timing_meta(ToolExecutionTiming::Unknown).unwrap();
    stamp_ownership(&mut registration, &owner).unwrap();
    let call = acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(acp::ToolCallId::new("owned"), "write")
            .kind(acp::ToolKind::Other).status(acp::ToolCallStatus::Pending)
            .meta(Some(registration)),
    );
    let completed = acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
        acp::ToolCallId::new("owned"), acp::ToolCallUpdateFields::new()
            .status(Some(acp::ToolCallStatus::Completed)),
    ).meta(Some(timing_meta(ToolExecutionTiming::Finished {
        execution_id: ToolExecutionId::generate(), started_at: Default::default(),
        duration_ms: 42_000, termination: ToolExecutionTermination::Returned,
    }).unwrap())));
    let fixture = Transcript::new(&(line(call, json!({"agentTimestampMs":1793367966934_i64}))
        + &line(completed, json!({"agentTimestampMs":1793390000000_i64}))));
    let (mut views, child) = fixture.load();
    let view = views.get_mut(&child).unwrap();
    assert_eq!(row_times(view), vec![Some(1_793_367_966_934)]);
    let clock = (0..view.scrollback.len()).find_map(|index| {
        match &view.scrollback.entry(index).unwrap().block {
            RenderBlock::ToolCall(block) => Some(block.execution_clock()),
            _ => None,
        }
    }).unwrap();
    assert_eq!(clock.elapsed_ms(), Some(42_000));
    draw(view);
}

#[test]
fn persisted_coalesced_edits_keep_surviving_earlier_registration() {
    crate::appearance::cache::set_collapsed_edit_blocks(true);
    let mut lines = String::new();
    for (id, start, line_number) in [("first", 1_793_367_966_934_i64, 5), ("second", 1_793_368_000_000, 40)] {
        let call = acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(acp::ToolCallId::new(id), "search_replace")
                .kind(acp::ToolKind::Edit).status(acp::ToolCallStatus::Pending),
        );
        lines += &line(call, json!({"agentTimestampMs":start}));
        let diff = acp::ToolCallContent::Diff(acp::Diff::new("fixture.rs", "new")
            .old_text(Some("old".into()))
            .meta(json!({"old_line":line_number,"new_line":line_number}).as_object().cloned()));
        lines += &line(tool_update(id, acp::ToolCallUpdateFields::new()
            .kind(Some(acp::ToolKind::Edit)).title(Some("fixture.rs".into()))
            .raw_input(Some(json!({"file_path":"fixture.rs"}))).content(Some(vec![diff]))
            .status(Some(acp::ToolCallStatus::Completed))), json!({"agentTimestampMs":1793390000000_i64}));
    }
    let fixture = Transcript::new(&lines);
    let (mut views, child) = fixture.load();
    let view = views.get_mut(&child).unwrap();
    assert_eq!(row_times(view), vec![Some(1_793_367_966_934)]);
    let edits: Vec<_> = (0..view.scrollback.len()).filter_map(|index| {
        match &view.scrollback.entry(index).unwrap().block {
            RenderBlock::ToolCall(ToolCallBlock::Edit(edit)) => Some(edit),
            _ => None,
        }
    }).collect();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].edit_count, 2);
    assert_eq!(edits[0].hunks.len(), 2);
    draw(view);
}
