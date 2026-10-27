use super::*;
use super::tests::{child_views, painted, settle, source_text, task_views, view};
use crate::scrollback::selection_identity::{RangeIdentity, SelectablePart};
use crate::scrollback::text_selection::ResolvedSelectableLine;

fn line_containing(views: &SessionViews, text: &str) -> ResolvedSelectableLine {
    views.get(&AgentId(0)).unwrap().last_scrollback_selection_model.ranges.iter()
        .flat_map(|range| &range.lines).find(|line| line.text.contains(text)).unwrap().clone()
}

#[test]
fn seventy_thousand_real_child_ranges_keep_tail_identity_and_bounded_caches() {
    let mut child = view("inline-seventy-thousand");
    child.scrollback.begin_batch();
    let mut expected = String::new();
    let mut ids = Vec::new();
    for index in 0..70_000 {
        let text = format!("SELECTABLE_{index:05}");
        if !expected.is_empty() { expected.push('\n'); }
        expected.push_str(&text);
        ids.push(child.scrollback.push_block(RenderBlock::agent_message(text)));
    }
    child.scrollback.end_batch();
    let (mut views, root) = child_views("inline-seventy-thousand", child);
    settle(&mut views, AgentId(0), 80);
    views.get_mut(&AgentId(0)).unwrap().scrollback.goto_top();
    settle(&mut views, AgentId(0), 80);
    let first = line_containing(&views, "SELECTABLE_00000");
    views.get_mut(&AgentId(0)).unwrap().scrollback.goto_bottom();
    let buffer = settle(&mut views, AgentId(0), 80);
    assert!(painted(&buffer).contains("SELECTABLE_69999"));
    let tail = line_containing(&views, "SELECTABLE_69999");
    assert_eq!(tail.range_id.source.entry, *ids.last().unwrap());
    assert_eq!(first.range_id.source.entry, ids[0]);
    assert_ne!(first.range_id, tail.range_id);
    assert_eq!(first.range_id.root, tail.range_id.root);
    assert!(tail.range_id.source.entry.value() > 65_534);
    let source = views.get(&AgentId(1)).unwrap();
    let identities: HashSet<_> = source.scrollback.iter_entries().map(|(_, entry)|
        RangeIdentity::new(entry.selection_identity(), SelectablePart::Block, 0)).collect();
    assert_eq!(identities.len(), 70_000);
    let cached = source.scrollback.iter_entries().filter(|(_, entry)| entry.has_cached_output()).count();
    assert!(cached <= 80, "only the physical visible window may hold native render caches: {cached}");
    assert_eq!(source_text(&mut views, root).as_deref(), Some(expected.as_str()));
    let source = views.get(&AgentId(1)).unwrap();
    assert_eq!(source.scrollback.iter_entries().filter(|(_, entry)| entry.has_cached_output()).count(), cached,
        "explicit canonical copy must not populate off-screen render caches");
    let root_view = views.get_mut(&AgentId(0)).unwrap();
    root_view.scrollback.scroll_up(0);
    root_view.scrollback.set_selected(None);
    let offset = root_view.scrollback.scroll_offset();
    root_view.scrollback.select_prev();
    assert_eq!(root_view.scrollback.scroll_offset(), offset);
    assert_eq!(root_view.scrollback.selected(), Some(0));
}

#[test]
fn earlier_part_insert_remove_live_delta_and_resize_do_not_retarget_source_bytes() {
    let mut child = view("inline-stable-parts");
    let earlier = child.scrollback.push_block(RenderBlock::agent_message("EARLIER"));
    let target = child.scrollback.push_block(RenderBlock::agent_message("KEEP_THIS_ENDPOINT"));
    let (mut views, root) = child_views("inline-stable-parts", child);
    settle(&mut views, AgentId(0), 80);
    let line = line_containing(&views, "KEEP_THIS_ENDPOINT");
    let position = line.position(4);
    let inserted = {
        let child = views.get_mut(&AgentId(1)).unwrap();
        child.scrollback.push_chunk_to_agent(earlier, &" extra preceding content".repeat(20));
        child.scrollback.insert_block_before(target, RenderBlock::agent_message("INSERTED_PART"))
    };
    views.note_inline_content_changed(AgentId(1));
    settle(&mut views, AgentId(0), 40);
    let current = line_containing(&views, "KEEP_THIS_ENDPOINT");
    assert_eq!(line.range_id, current.range_id);
    assert_eq!(position, current.position(4));
    let tree = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(root).unwrap().cached_tree();
    let leaf = tree.selection_leaf(line.range_id).unwrap();
    let (row, column) = leaf.resolve_position(line.range_id.local, position).unwrap();
    assert_eq!(column, 4);
    assert_eq!(crate::scrollback::types::derive_selection_text(leaf.line(row).unwrap().line), "KEEP_THIS_ENDPOINT");
    views.get_mut(&AgentId(1)).unwrap().scrollback.remove_entry(inserted);
    views.note_inline_content_changed(AgentId(1));
    settle(&mut views, AgentId(0), 80);
    assert_eq!(line.range_id, line_containing(&views, "KEEP_THIS_ENDPOINT").range_id);
    views.get_mut(&AgentId(1)).unwrap().scrollback.remove_entry(target);
    views.note_inline_content_changed(AgentId(1));
    settle(&mut views, AgentId(0), 80);
    let tree = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(root).unwrap().cached_tree();
    assert!(tree.selection_leaf(line.range_id).is_none());
}

#[test]
fn old_child_binding_cannot_resolve_an_identical_replacement_endpoint() {
    let mut child = view("inline-binding-target");
    child.scrollback.push_block(RenderBlock::agent_message("SAME_TEXT"));
    let (mut views, root) = child_views("inline-binding-target", child);
    settle(&mut views, AgentId(0), 80);
    let line = line_containing(&views, "SAME_TEXT");
    let tree = views.get(&AgentId(0)).unwrap().scrollback.get_by_id(root).unwrap().cached_tree();
    assert!(tree.selection_leaf(line.range_id).is_some());
    views.get_mut(&AgentId(1)).unwrap().bind_session_id(agent_client_protocol::SessionId::new("new-binding"));
    assert!(tree.selection_leaf(line.range_id).is_none());
    assert_eq!(views.inline_copy_text(AgentId(0), root), InlineCopyOutcome::Unavailable);
}

#[test]
fn task_copy_is_canonical_without_a_prior_draw_or_render_width() {
    let (mut views, root) = task_views();
    let source = "  stdout\n\nwide 界 and trailing spaces  \n";
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", source.into());
    assert_eq!(views.inline_copy_text(AgentId(0), root), InlineCopyOutcome::Ready(source.into()));
    assert!(!views.get(&AgentId(0)).unwrap().scrollback.get_by_id(root).unwrap().has_cached_output());
}

#[test]
fn composite_overflow_is_typed_unavailable_and_not_a_saturated_tail() {
    use crate::scrollback::entry::inline_output::OutputError;
    let entry = ScrollbackEntry::new(RenderBlock::agent_message("source"));
    let tree = OutputTree::branch([
        OutputTree::deferred(entry.selection_identity(), usize::MAX), OutputTree::notice("real tail"),
    ]);
    assert_eq!(tree.unavailable_reason(), Some(OutputError::GeometryOverflow));
    assert!(matches!(tree.line(0).unwrap().line.selectable, crate::scrollback::types::Selectable::None));
    assert!(tree.line(0).unwrap().range_id.is_none());
}

fn root_pane(views: &mut SessionViews) -> (ratatui::buffer::Buffer, crate::scrollback::RenderOutput) {
    let area = ratatui::layout::Rect::new(0, 0, 100, 60);
    for _ in 0..8 {
        views.prepare_inline_output(AgentId(0));
        let root = views.get_mut(&AgentId(0)).unwrap();
        root.scrollback.prepare_layout(area.width, area.height);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        let output = crate::scrollback::ScrollbackPane::new().render_with_scratch(area, &mut buffer,
            &root.scrollback, &mut crate::scrollback::ScratchBuffer::new());
        if !views.prepare_inline_output(AgentId(0)) { return (buffer, output); }
    }
    panic!("native inline parts did not settle");
}

#[test]
fn native_child_pixels_gaps_media_diagrams_and_source_cwd_survive_inline_composition() {
    use crate::render::Renderable;
    use crate::scrollback::blocks::tool::{OtherToolCallBlock, ToolCallBlock};
    use crate::scrollback::wrappers::EntryRenderer;
    crate::appearance::cache::set_render_mermaid(crate::appearance::RenderMermaid::On);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native.png");
    image::RgbaImage::new(2, 2).save(&path).unwrap();
    let mut child = view("inline-native-paint");
    child.session.cwd = directory.path().to_path_buf();
    child.scrollback.set_cwd(Some(directory.path().to_path_buf()));
    child.scrollback.push_block(RenderBlock::user_prompt("native prompt\nsecond line"));
    child.scrollback.push_block(RenderBlock::agent_message("intro\n\n```mermaid\nA-->B\n```\n\nbye"));
    let waiting = child.scrollback.push_block(RenderBlock::read(
        directory.path().join("native.txt").to_string_lossy(), None));
    child.scrollback.set_entry_running(waiting, true);
    child.scrollback.set_pending_user_input(waiting, true);
    child.scrollback.push_block(RenderBlock::ToolCall(ToolCallBlock::Other(
        OtherToolCallBlock::new("Render", "image").with_media_ref(path.clone(), false))));
    for entry in child.scrollback.entries_mut() { entry.set_display_mode(DisplayMode::Expanded); }
    let (mut views, root_id) = child_views("inline-native-paint", child);
    let (buffer, output) = root_pane(&mut views);
    assert_eq!(output.diagram_affordances.len(), 1);
    assert_eq!(output.diagram_affordances[0].source.trim(), "A-->B");
    assert!(output.inline_media.iter().any(|placement| placement.info.path == path));
    let origin = output.selection_model.visible_blocks.first().unwrap().content_area;
    let root = views.get(&AgentId(0)).unwrap();
    let tree = root.scrollback.get_by_id(root_id).unwrap().cached_tree();
    let child = views.get(&AgentId(1)).unwrap();
    let theme = crate::theme::Theme::current();
    let mut previous_end = None;
    let mut parts = 0;
    tree.visit_native(0..tree.len(), 0, &mut |offset, part| {
        if let Some(end) = previous_end { assert_eq!(offset, end + 1, "native inter-entry gap"); }
        let entry = child.scrollback.get_by_id(part.entry.id).unwrap();
        let renderer = EntryRenderer::new(entry, &theme).with_appearance_ref(child.scrollback.appearance())
            .with_cwd(child.scrollback.cwd()).with_tick(child.scrollback.current_tick())
            .with_groupable(entry.block.is_groupable());
        let height = renderer.desired_height(part.width);
        assert_eq!(part.layout.height, height);
        let area = ratatui::layout::Rect::new(origin.x, origin.y + u16::try_from(offset).unwrap(),
            part.width, u16::try_from(height).unwrap());
        let mut native = ratatui::buffer::Buffer::empty(buffer.area);
        renderer.render(area, &mut native);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                assert_eq!(buffer.cell((x, y)), native.cell((x, y)), "native source pixel at {x},{y}");
            }
        }
        previous_end = Some(offset + height);
        parts += 1;
    });
    assert_eq!(parts, 4);
}

#[test]
fn first_up_selecting_a_huge_visible_inline_root_does_not_reveal_its_header() {
    let (mut views, root) = task_views();
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", "VISIBLE_SOURCE\n".repeat(70_000));
    views.get_mut(&AgentId(0)).unwrap().scrollback.push_block(RenderBlock::agent_message("AFTER_ROOT"));
    settle(&mut views, AgentId(0), 80);
    let view = views.get_mut(&AgentId(0)).unwrap();
    view.scrollback.goto_bottom();
    view.scrollback.scroll_up(30);
    view.scrollback.set_selected(Some(1));
    let before = view.scrollback.scroll_offset();
    assert!(before > 65_535);
    view.scrollback.select_prev();
    assert_eq!(view.scrollback.scroll_offset(), before);
    assert_eq!(view.scrollback.selected().and_then(|index| view.scrollback.entry(index)).map(|entry| entry.id), Some(root));
}

#[test]
fn canonical_inline_tail_is_searchable_beyond_terminal_rows() {
    let (mut views, root) = task_views();
    let source = format!("{}INLINE_SEARCH_TAIL", "source row\n".repeat(70_000));
    views.get_mut(&AgentId(0)).unwrap().set_bg_task_stdout("task", source);
    settle(&mut views, AgentId(0), 80);
    let view = views.get(&AgentId(0)).unwrap();
    let mut index = crate::scrollback::ScrollbackSearchIndex::new();
    index.sync(&view.scrollback);
    let matches = index.find(&crate::search::TextMatcher::new("INLINE_SEARCH_TAIL", crate::search::QueryKind::Regex));
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].entry_id, root);
    assert!(matches[0].line_in_entry >= 70_000);
}
