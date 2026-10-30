use super::*;
use super::tests::{painted, settle, source_text, task_views};
use crate::scrollback::selection_identity::SelectablePart;
use crate::views::block_viewer::BlockViewerPane;
use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Modifier}};

fn task_rows(views: &SessionViews, id: EntryId) -> Vec<String> {
    let tree = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap().cached_tree();
    (0..tree.len()).filter_map(|index| {
        let row = tree.line(index)?;
        matches!(row.range_id?.part, SelectablePart::TaskOutput(_)).then(|| {
            row.line.content.spans.iter().map(|span| span.content.as_ref()).collect()
        })
    }).collect()
}

fn modal(views: &SessionViews, id: EntryId, raw: &str, width: u16) -> Buffer {
    let entry = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap();
    let mut pane = BlockViewerPane::for_bg_task(id, "task", raw, false);
    let area = Rect::new(0, 0, width, 20);
    let mut buffer = Buffer::empty(area);
    pane.render_content(area, &mut buffer, entry, true, &[]);
    buffer
}

fn assert_visible(buffer: &Buffer, expected: &str) {
    assert!(painted(buffer).contains(expected), "missing {expected:?}");
    assert!(!painted(buffer).contains('\x1b'));
    assert!(!painted(buffer).contains("[31m"));
}

#[test]
fn chat_and_modal_resolve_sgr_cr_erase_tabs_and_osc_without_changing_source() {
    let raw = "\x1b[31;1mRED\x1b[0m normal\nobsolete progress\r\x1b[2KDONE\n\x1b]0;PRIVATE_TITLE\x07\x1b]52;c;PRIVATE_CLIPBOARD\x07\x1b]8;;https://example.invalid\x07LINK\x1b]8;;\x07\nA\t界";
    let (mut views, id) = task_views();
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.into());
    let chat = settle(&mut views, AgentId(0), 80);
    assert_eq!(task_rows(&views, id), ["RED normal", "DONE", "LINK", "A       界"]);
    let viewer = modal(&views, id, raw, 60);
    for buffer in [&chat, &viewer] {
        for expected in ["RED normal", "DONE", "LINK", "A       界"] {
            assert_visible(buffer, expected);
        }
        for hidden in ["obsolete", "PRIVATE_TITLE", "PRIVATE_CLIPBOARD", "https://"] {
            assert!(!painted(buffer).contains(hidden));
        }
        let red = buffer.content.windows(3).find(|cells| {
            cells.iter().map(|cell| cell.symbol()).collect::<String>() == "RED"
        }).unwrap();
        assert_eq!(red[0].fg, crate::theme::color_support::quantize(Color::Red));
        assert!(red[0].modifier.contains(Modifier::BOLD));
    }
    assert_eq!(source_text(&mut views, id).as_deref(), Some(raw));
    assert_eq!(views.get(&AgentId(0)).unwrap().session.bg_tasks["task"].stdout, raw);
}

#[test]
fn accumulated_stream_snapshots_keep_partial_sequences_and_replay_matches_live_cells() {
    let (mut views, id) = task_views();
    let mut raw = String::new();
    for (chunk, expected) in [
        ("\x1b[3", Vec::<&str>::new()),
        ("1mFIRST", vec!["FIRST"]),
        ("\r\x1b[", vec!["FIRST"]),
        ("2KFINAL\x1b[0m\n\x1b]52;c;HIDDEN", vec!["FINAL"]),
        ("\x07TAIL", vec!["FINAL", "TAIL"]),
    ] {
        raw.push_str(chunk);
        views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.clone());
        settle(&mut views, AgentId(0), 80);
        assert_eq!(task_rows(&views, id), expected);
    }
    let (mut replay, replay_id) = task_views();
    let root = replay.get_mut(&AgentId(0)).unwrap();
    root.session.bg_tasks.get_mut("task").unwrap().restored_from_replay = true;
    root.set_bg_task_stdout("task", raw.clone());
    let replay_buffer = settle(&mut replay, AgentId(0), 80);
    assert_eq!(task_rows(&replay, replay_id), ["FINAL", "TAIL"]);
    assert_visible(&replay_buffer, "FINAL");
    assert_visible(&modal(&replay, replay_id, &raw, 45), "TAIL");
    assert_eq!(source_text(&mut replay, replay_id), Some(raw));
}

#[test]
fn execute_chat_modal_and_source_copy_keep_terminal_and_source_roles_separate() {
    use crate::scrollback::blocks::tool::{ExecuteToolCallBlock, ToolCallBlock};
    let raw = "\x1b[31mFAIL\x1b[0m\nold\r\x1b[2KOK";
    let block = ExecuteToolCallBlock::new("printf fixture").with_output(raw);
    assert_eq!(block.copy_text(), raw);
    let mut root = super::tests::view("terminal-execute");
    let id = root.scrollback.push_block(RenderBlock::ToolCall(ToolCallBlock::Execute(block)));
    root.scrollback.get_by_id_mut(id).unwrap().set_display_mode(DisplayMode::Expanded);
    let mut views = SessionViews::from([(AgentId(0), root)]);
    let chat = settle(&mut views, AgentId(0), 80);
    let entry = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(id).unwrap();
    assert_eq!(entry.block.copy_text(false).as_deref(), Some(raw));
    let mut pane = BlockViewerPane::for_execute(id, entry).unwrap();
    let area = Rect::new(0, 0, 60, 20);
    let mut viewer = Buffer::empty(area);
    pane.render_content(area, &mut viewer, entry, true, &[]);
    for buffer in [&chat, &viewer] {
        assert_visible(buffer, "FAIL");
        assert_visible(buffer, "OK");
        assert!(!painted(buffer).contains("old"));
    }
}

#[test]
fn chat_and_modal_overwrite_wide_and_combining_cells_at_terminal_columns() {
    let raw = "\x1b[0m界x\x1b[2D!\ne\u{301}x\x08!\na界z\x1b[2D\x1b[1K";
    let (mut views, id) = task_views();
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.into());
    let chat = settle(&mut views, AgentId(0), 80);
    assert_eq!(task_rows(&views, id), [" !x", "e\u{301}!", "   z"]);
    let viewer = modal(&views, id, raw, 60);
    for buffer in [&chat, &viewer] {
        assert_visible(buffer, " !x");
        assert_visible(buffer, "e\u{301}!");
        assert_visible(buffer, "   z");
        assert!(!painted(buffer).contains('界'));
    }
    assert_eq!(source_text(&mut views, id).as_deref(), Some(raw));
}

#[test]
fn chat_selection_copies_resolved_cells_while_task_copy_preserves_bytes() {
    use crate::scrollback::text_selection::{ActiveTextDrag, SelectionKind, reconstruct_selection_text};

    for (raw, expected_rows) in [
        ("ab\rX", vec!["Xb"]),
        ("a\tb", vec!["a       b"]),
        ("a\r\nb  \r\n", vec!["a", "b"]),
    ] {
        let (mut views, id) = task_views();
        views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.into());
        settle(&mut views, AgentId(0), 80);
        assert_eq!(task_rows(&views, id), expected_rows);
        let model = &views.get(&AgentId(0)).unwrap().last_scrollback_selection_model;
        let lines: Vec<_> = model.ranges.iter().filter(|range| {
            matches!(range.range_id.part, SelectablePart::TaskOutput(_))
        }).flat_map(|range| &range.lines).collect();
        let select = |row: usize, start, end| {
            let line = lines[row];
            let x = line.screen_x + line.selectable_cols.start;
            let anchor = model.hit_test_text_exact(x + start, line.screen_y).unwrap();
            let head = model.hit_test_text_exact(x + end, line.screen_y).unwrap();
            reconstruct_selection_text(model, &ActiveTextDrag {
                anchor, head, kind: SelectionKind::Linear, anchor_content_width: None,
            })
        };
        match raw {
            "ab\rX" => {
                assert_eq!(select(0, 0, 0).as_deref(), Some("X"));
                assert_eq!(select(0, 1, 1).as_deref(), Some("b"));
                assert_eq!(select(0, 0, 1).as_deref(), Some("Xb"));
            }
            "a\tb" => {
                assert_eq!(select(0, 1, 7).as_deref(), Some("       "));
                assert_eq!(select(0, 0, 8).as_deref(), Some("a       b"));
            }
            "a\r\nb  \r\n" => {
                assert_eq!(select(0, 0, 0).as_deref(), Some("a"));
                assert_eq!(select(1, 0, 0).as_deref(), Some("b"));
            }
            _ => unreachable!("explicit fixtures"),
        }
        let first = lines.first().unwrap();
        let last = lines.last().unwrap();
        let anchor = model.hit_test_text_exact(
            first.screen_x + first.selectable_cols.start, first.screen_y,
        ).unwrap();
        let head = model.hit_test_text_exact(
            last.screen_x + last.selectable_cols.end - 1, last.screen_y,
        ).unwrap();
        assert_eq!(reconstruct_selection_text(model, &ActiveTextDrag {
            anchor, head, kind: SelectionKind::Linear, anchor_content_width: None,
        }), Some(expected_rows.join("\n")));
        assert_eq!(source_text(&mut views, id).as_deref(), Some(raw));
    }
}

#[test]
fn execute_chat_partial_selection_uses_overwritten_cells() {
    use crate::scrollback::blocks::tool::{ExecuteToolCallBlock, ToolCallBlock};
    use crate::scrollback::text_selection::{ActiveTextDrag, SelectionKind, reconstruct_selection_text};

    let raw = "ab\rX\nA\tB\r\n";
    let mut root = super::tests::view("execute-selection");
    let id = root.scrollback.push_block(RenderBlock::ToolCall(ToolCallBlock::Execute(
        ExecuteToolCallBlock::new("printf fixture").with_output(raw),
    )));
    root.scrollback.get_by_id_mut(id).unwrap().set_display_mode(DisplayMode::Expanded);
    let mut views = SessionViews::from([(AgentId(0), root)]);
    let buffer = settle(&mut views, AgentId(0), 80);
    assert_visible(&buffer, "Xb");
    assert_visible(&buffer, "A       B");
    let root = views.get(&AgentId(0)).unwrap();
    let model = &root.last_scrollback_selection_model;
    let line = model.ranges.iter().flat_map(|range| &range.lines)
        .find(|line| line.text == "Xb").unwrap();
    let x = line.screen_x + line.selectable_cols.start;
    for (start, end, expected) in [(0, 0, "X"), (1, 1, "b"), (0, 1, "Xb")] {
        let anchor = model.hit_test_text_exact(x + start, line.screen_y).unwrap();
        let head = model.hit_test_text_exact(x + end, line.screen_y).unwrap();
        assert_eq!(reconstruct_selection_text(model, &ActiveTextDrag {
            anchor, head, kind: SelectionKind::Linear, anchor_content_width: None,
        }).as_deref(), Some(expected));
    }
    assert_eq!(root.scrollback.get_by_id(id).unwrap().block.copy_text(false).as_deref(), Some(raw));
}

#[test]
fn control_only_task_output_has_no_raw_selection_bytes() {
    for (raw, rows) in [("\t", Vec::<&str>::new()), ("\r\n", vec![""])] {
        let (mut views, id) = task_views();
        views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.into());
        settle(&mut views, AgentId(0), 80);
        assert_eq!(task_rows(&views, id), rows);
        let model = &views.get(&AgentId(0)).unwrap().last_scrollback_selection_model;
        let selected: String = model.ranges.iter().filter(|range| {
            matches!(range.range_id.part, SelectablePart::TaskOutput(_))
        }).flat_map(|range| &range.lines).map(|line| line.text.as_str()).collect();
        assert_eq!(selected, "");
        assert_eq!(source_text(&mut views, id).as_deref(), Some(raw));
    }
}

#[test]
fn terminal_projection_wrap_resize_collapse_and_tail_use_resolved_rows() {
    let raw = format!("\x1b[32m{}\x1b[0m\nTAIL", "Z".repeat(100));
    let (mut views, id) = task_views();
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", raw.clone());
    for width in [42, 65, 42] {
        settle(&mut views, AgentId(0), width);
        let root = views.get(&AgentId(0)).unwrap();
        let entry = root.scrollback.get_by_id(id).unwrap();
        let available = usize::from(entry.inline_request_width().unwrap());
        let rows = task_rows(&views, id);
        assert_eq!(rows.concat(), format!("{}TAIL", "Z".repeat(100)));
        assert_eq!(rows.last().map(String::as_str), Some("TAIL"));
        assert!(rows.iter().all(|row| unicode_width::UnicodeWidthStr::width(row.as_str()) <= available));
        assert_eq!(rows.len(), 100_usize.div_ceil(available) + 1);
        assert!(!views.prepare_inline_output(AgentId(0)));
        assert_eq!(source_text(&mut views, id).as_deref(), Some(raw.as_str()));
    }
    views.get_mut(&AgentId(0)).unwrap().scrollback.get_by_id_mut(id).unwrap()
        .set_display_mode(DisplayMode::Collapsed);
    let collapsed = settle(&mut views, AgentId(0), 42);
    assert!(!painted(&collapsed).contains("TAIL"));
    assert!(task_rows(&views, id).is_empty());
    views.get_mut(&AgentId(0)).unwrap().scrollback.get_by_id_mut(id).unwrap()
        .set_display_mode(DisplayMode::Expanded);
    settle(&mut views, AgentId(0), 42);
    assert_eq!(task_rows(&views, id).last().map(String::as_str), Some("TAIL"));
}
