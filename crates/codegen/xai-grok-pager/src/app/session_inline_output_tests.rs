use super::*;
use crate::actions::ActionRegistry;
use crate::app::agent_test_fixtures::running_bg_task;
use crate::app::agent_view::{AgentView, AppRenderParams, BannerSlotParams, ChildLink};
use crate::app::session_views::test_support::link_child;
use crate::scrollback::blocks::{BgTaskBlock, SubagentBlock};
use crate::scrollback::render::ScratchBuffer;
use ratatui::{buffer::Buffer, layout::Rect};
use std::path::PathBuf;
use std::time::Instant;

pub(super) fn view(sid: &str) -> AgentView {
    crate::app::agent_view::test_agent_view(Some(sid), PathBuf::from("/tmp"))
}

pub(super) fn settle(views: &mut SessionViews, owner: AgentId, width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, 60);
    for _ in 0..8 {
        views.prepare_inline_output(owner);
        let mut buffer = Buffer::empty(area);
        views.get_mut(&owner).unwrap().draw(area, &mut buffer, &ActionRegistry::defaults(),
            &mut ScratchBuffer::new(), None, false, BannerSlotParams::none(), false,
            &mut Vec::new(), AppRenderParams::default());
        if !views.prepare_inline_output(owner) { return buffer; }
    }
    panic!("inline source layout did not settle");
}

pub(super) fn painted(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect::<String>()
}

pub(super) fn task_views() -> (SessionViews, EntryId) {
    let mut root = view("inline-root");
    let mut task = running_bg_task("task", false);
    task.command = "ls".into();
    root.session.bg_tasks.insert("task".into(), task);
    let id = root.scrollback.push_block(RenderBlock::BgTask(BgTaskBlock::started("ls", "task")));
    root.scrollback.get_by_id_mut(id).unwrap().set_display_mode(DisplayMode::Expanded);
    (SessionViews::from([(AgentId(0), root)]), id)
}

pub(super) fn child_views(sid: &str, child: AgentView) -> (SessionViews, EntryId) {
    let mut root = view("inline-root");
    let id = root.scrollback.push_block(RenderBlock::Subagent(SubagentBlock::started(
        "delegation is not the transcript", sid, "explore", None, None, None, false)));
    root.scrollback.get_by_id_mut(id).unwrap().set_display_mode(DisplayMode::Expanded);
    let mut info = crate::app::subagent::test_support::make_info();
    info.subagent_id = sid.into();
    info.child_session_id = sid.into();
    info.attempt.scrollback_entry_id = Some(id);
    root.subagent_sessions.insert(sid.into(), info);
    let mut views = SessionViews::from([(AgentId(0), root)]);
    link_child(&mut views, AgentId(0), AgentId(1), child, Instant::now());
    (views, id)
}

pub(super) fn source_text(views: &mut SessionViews, id: EntryId) -> Option<String> {
    match views.inline_copy_text(AgentId(0), id) {
        super::InlineCopyOutcome::Ready(text) => Some(text),
        super::InlineCopyOutcome::Empty | super::InlineCopyOutcome::Loading
        | super::InlineCopyOutcome::Unavailable | super::InlineCopyOutcome::Failed => None,
    }
}

#[test]
fn one_line_command_uses_real_stdout_for_render_fold_height_and_selection() {
    let (mut views, id) = task_views();
    let source = "  | stdout 界 > raw\n\tsecond line  \n";
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", source.into());
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("stdout 界"));
    let root = views.get(&AgentId(0)).unwrap();
    assert!(root.scrollback.get_by_id(id).unwrap().is_foldable());
    let lines: Vec<_> = root.last_scrollback_selection_model.ranges.iter()
        .flat_map(|range| &range.lines).collect();
    let mut selected = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 { selected.push_str(line.joiner_to_previous.as_deref().unwrap_or("\n")); }
        selected.push_str(&line.text);
    }
    assert_eq!(selected, source);
    assert_eq!(source_text(&mut views, id).as_deref(), Some(source));
}

#[test]
fn empty_streaming_final_replay_and_equal_length_stdout_replace_the_same_source() {
    let (mut views, id) = task_views();
    settle(&mut views, AgentId(0), 80);
    assert!(!views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap().is_foldable());
    assert_eq!(source_text(&mut views, id), None);
    for (text, replay) in [("AAAA", false), ("BBBB", false), ("CCCC", true)] {
        let root = views.get_mut(&AgentId(0)).unwrap();
        root.session.bg_tasks.get_mut("task").unwrap().restored_from_replay = replay;
        root.set_bg_task_stdout("task", text.into());
        let buffer = settle(&mut views, AgentId(0), 80);
        assert!(painted(&buffer).contains(text));
        assert_eq!(source_text(&mut views, id).as_deref(), Some(text));
    }
    views.get_mut(&AgentId(0)).unwrap().append_bg_task_stdout("task", "final line");
    views.get_mut(&AgentId(0)).unwrap().session.bg_tasks.get_mut("task").unwrap().status =
        crate::app::agent::BgTaskStatus::Done;
    settle(&mut views, AgentId(0), 80);
    assert_eq!(source_text(&mut views, id).as_deref(), Some("CCCC\nfinal line"));
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", String::new());
    settle(&mut views, AgentId(0), 80);
    assert_eq!(source_text(&mut views, id), None);
    assert!(!views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap().is_foldable());
}

#[test]
fn long_single_line_stdout_is_complete_at_each_width_and_unchanged_draws_reuse_sources() {
    let (mut views, id) = task_views();
    let source = format!("BEGIN {} END", "objective界 ".repeat(30));
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", source.clone());
    for width in [80, 52, 110, 80] {
        settle(&mut views, AgentId(0), width);
        assert_eq!(source_text(&mut views, id), Some(source.clone()));
        assert!(!views.prepare_inline_output(AgentId(0)));
        let root = views.get(&AgentId(0)).unwrap();
        let entry = root.scrollback.get_by_id(id).unwrap();
        assert!(entry.cached_tree().len() > 3);
        let output_width = entry.inline_request_width().unwrap();
        assert!(entry.cached_tree().lines().all(|line| line.content.width() <= usize::from(output_width)));
    }
}

#[test]
fn child_projection_reads_actual_view_and_tracks_live_equal_length_and_resize_changes() {
    let mut child = view("inline-child-live");
    let message = child.scrollback.push_block(RenderBlock::user_prompt("FIRST transcript body"));
    let (mut views, id) = child_views("inline-child-live", child);
    let before_reads = crate::app::subagent::test_support::transcript_reads();
    settle(&mut views, AgentId(0), 80);
    assert!(source_text(&mut views, id).unwrap().contains("FIRST transcript body"));
    assert!(!source_text(&mut views, id).unwrap().contains("delegation is not"));
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before_reads);
    {
        let child = views.get_mut(&AgentId(1)).unwrap();
        let entry = child.scrollback.get_by_id_mut(message).unwrap();
        entry.block = RenderBlock::user_prompt("OTHER transcript body");
        entry.invalidate_cache();
        child.scrollback.mark_height_dirty(message);
    }
    for width in [80, 52, 110] {
        settle(&mut views, AgentId(0), width);
        let text = source_text(&mut views, id).unwrap();
        assert!(text.contains("OTHER transcript body"));
        assert!(!text.contains("FIRST transcript body"));
    }
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before_reads);
}

#[test]
fn cold_child_replay_is_passive_and_cached_without_queue_or_session_activation() {
    let home = tempfile::tempdir().unwrap();
    let sid = "inline-child-cold";
    let dir = home.path().join("sessions").join(urlencoding::encode("/tmp").as_ref()).join(sid);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("summary.json"), "{}").unwrap();
    let record = serde_json::json!({"method":"session/update", "params": {
        "sessionId": sid, "update": {"sessionUpdate":"user_message_chunk",
            "content":{"type":"text", "text":"persisted actual child content"}}}});
    std::fs::write(dir.join("updates.jsonl"), format!("{record}\n")).unwrap();
    crate::app::subagent::set_replay_grok_home_for_tests(Some(home.path().to_owned()));
    let mut child = view(sid);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    child.session.acp_tx = tx;
    let (mut views, id) = child_views(sid, child);
    let before = crate::app::subagent::test_support::transcript_reads();
    settle(&mut views, AgentId(0), 80);
    let text = source_text(&mut views, id).unwrap();
    assert!(text.contains("persisted actual child content"));
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before + 1);
    settle(&mut views, AgentId(0), 52);
    settle(&mut views, AgentId(0), 80);
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before + 1);
    assert!(rx.try_recv().is_err());
    let child = views.get(&AgentId(1)).unwrap();
    assert!(matches!(child.session.state, crate::app::agent::AgentState::Idle));
    assert!(child.session.pending_prompts.is_empty());
    assert!(child.session.current_prompt_id.is_none());
    crate::app::subagent::set_replay_grok_home_for_tests(None);
}

#[test]
fn wrong_identity_and_missing_child_never_render_delegation_as_content() {
    let (mut views, id) = child_views("inline-child-missing", view("inline-child-missing"));
    views.get_mut(&AgentId(1)).unwrap().role = AgentRole::Root;
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("Output unavailable"));
    assert_eq!(source_text(&mut views, id), None);
    assert!(!views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap().is_foldable());
}

#[test]
fn real_child_cycle_stops_at_the_identity_path() {
    let mut child = view("inline-child-cycle");
    let back = child.scrollback.push_block(RenderBlock::Subagent(SubagentBlock::completed(
        "back edge", "inline-root", std::time::Duration::ZERO)));
    child.scrollback.get_by_id_mut(back).unwrap().set_display_mode(DisplayMode::Expanded);
    let (mut views, id) = child_views("inline-child-cycle", child);
    views.get_mut(&AgentId(0)).unwrap().role = AgentRole::Child(ChildLink {
        parent: Some(AgentId(1)), parent_session_id: agent_client_protocol::SessionId::new("inline-child-cycle"),
        subagent_id: "inline-root".into(), started_at: Instant::now(),
    });
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("Cyclic child link"));
    let _ = source_text(&mut views, id);
    views.note_inline_content_changed(AgentId(1));
    settle(&mut views, AgentId(0), 80);
}

#[test]
fn child_selection_and_copy_preserve_distinct_real_entry_ranges() {
    let mut child = view("inline-child-ranges");
    child.scrollback.push_block(RenderBlock::user_prompt("first real child entry"));
    let update: agent_client_protocol::SessionUpdate = serde_json::from_value(serde_json::json!({
        "sessionUpdate":"agent_message_chunk", "content":{"type":"text", "text":"second real child entry"}
    })).unwrap();
    child.session.handle_update(update, &crate::acp::meta::NotificationMeta::default(), &mut child.scrollback);
    let (mut views, id) = child_views("inline-child-ranges", child);
    settle(&mut views, AgentId(0), 80);
    let ranges = &views.get(&AgentId(0)).unwrap().last_scrollback_selection_model.ranges;
    let first = ranges.iter().find(|range| range.lines.iter().any(|line| line.text.contains("first real child"))).unwrap();
    let second = ranges.iter().find(|range| range.lines.iter().any(|line| line.text.contains("second real child"))).unwrap();
    assert_ne!(first.range_id, second.range_id);
    let text = source_text(&mut views, id).unwrap();
    assert!(text.contains("first real child entry"));
    assert!(text.contains("second real child entry"));
}

#[test]
fn loading_empty_child_is_not_foldable_and_does_not_read_storage() {
    let mut child = view("inline-child-loading");
    child.session.loading_replay = true;
    let (mut views, id) = child_views("inline-child-loading", child);
    let before = crate::app::subagent::test_support::transcript_reads();
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("Loading output"));
    assert!(!views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap().is_foldable());
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before);
    assert_eq!(source_text(&mut views, id), None);
}

#[test]
fn nested_child_output_changes_invalidate_the_real_ancestor_projection() {
    let mut middle = view("inline-middle");
    let nested_row = middle.scrollback.push_block(RenderBlock::Subagent(SubagentBlock::completed(
        "nested delegation", "inline-leaf", std::time::Duration::ZERO)));
    middle.scrollback.get_by_id_mut(nested_row).unwrap().set_display_mode(DisplayMode::Expanded);
    let (mut views, id) = child_views("inline-middle", middle);
    let mut leaf = view("inline-leaf");
    let leaf_row = leaf.scrollback.push_block(RenderBlock::user_prompt("leaf before"));
    link_child(&mut views, AgentId(1), AgentId(2), leaf, Instant::now());
    settle(&mut views, AgentId(0), 80);
    assert!(source_text(&mut views, id).unwrap().contains("leaf before"));
    {
        let leaf = views.get_mut(&AgentId(2)).unwrap();
        let entry = leaf.scrollback.get_by_id_mut(leaf_row).unwrap();
        entry.block = RenderBlock::user_prompt("leaf after!");
        entry.invalidate_cache();
        leaf.scrollback.mark_height_dirty(leaf_row);
    }
    settle(&mut views, AgentId(0), 80);
    let text = source_text(&mut views, id).unwrap();
    assert!(text.contains("leaf after!"));
    assert!(!text.contains("leaf before"));
}

#[test]
fn full_inline_stdout_tail_remains_reachable_beyond_terminal_coordinate_range() {
    let (mut views, _) = task_views();
    let source = format!("{}INLINE_REAL_TAIL", "source line\n".repeat(65_540));
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", source);
    settle(&mut views, AgentId(0), 80);
    views.get_mut(&AgentId(0)).unwrap().scrollback.goto_bottom();
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("INLINE_REAL_TAIL"));
}

#[test]
fn left_collapses_empty_output_and_late_data_preserves_the_selected_mode() {
    let (mut views, id) = task_views();
    let root = views.get_mut(&AgentId(0)).unwrap();
    root.session.bg_tasks.get_mut("task").unwrap().status = crate::app::agent::BgTaskStatus::Done;
    root.scrollback.set_selected(Some(0));
    settle(&mut views, AgentId(0), 80);
    let root = views.get_mut(&AgentId(0)).unwrap();
    assert!(!root.scrollback.get_by_id(id).unwrap().is_foldable());
    root.scrollback.collapse_selected();
    assert_eq!(root.scrollback.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
    root.scrollback.expand_selected();
    assert_eq!(root.scrollback.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
    root.set_bg_task_stdout("task", "late canonical body".into());
    settle(&mut views, AgentId(0), 80);
    let root = views.get(&AgentId(0)).unwrap();
    assert_eq!(root.scrollback.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
    assert_eq!(root.scrollback.selected(), Some(0));
}

#[test]
fn explicit_cold_expand_and_loading_collapse_do_not_activate_a_child() {
    let sid = "inline-loading-fold";
    let mut child = view(sid);
    child.session.loading_replay = true;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    child.session.acp_tx = tx;
    let (mut views, id) = child_views(sid, child);
    let before = crate::app::subagent::test_support::transcript_reads();
    let root = views.get_mut(&AgentId(0)).unwrap();
    root.scrollback.set_selected(Some(0));
    root.scrollback.get_by_id_mut(id).unwrap().set_display_mode(DisplayMode::Collapsed);
    root.scrollback.expand_selected();
    assert_eq!(root.scrollback.get_by_id(id).unwrap().display_mode, DisplayMode::Expanded);
    settle(&mut views, AgentId(0), 80);
    let root = views.get_mut(&AgentId(0)).unwrap();
    assert!(!root.scrollback.get_by_id(id).unwrap().is_foldable());
    root.scrollback.collapse_selected();
    assert_eq!(root.scrollback.get_by_id(id).unwrap().display_mode, DisplayMode::Collapsed);
    assert_eq!(crate::app::subagent::test_support::transcript_reads(), before);
    assert!(rx.try_recv().is_err());
}
