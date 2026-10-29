use super::*;
use crate::render::SafeBuf;
use crate::render::osc8::LinkOverlay;
use crate::scrollback::blocks::{AgentMessageBlock, ThinkingBlock};
use crate::scrollback::selection_identity::RangeIdentity;
use crate::scrollback::selection_position::SourcePosition;
use crate::scrollback::text_selection::{
    ActiveTextDrag, RangeHit, SelectionKind, reconstruct_full_selection_text,
};
use crate::scrollback::types::BlockLine;
use ratatui::{buffer::Buffer, layout::Rect};

fn hit(row: usize, col: u16) -> RangeHit {
    RangeHit {
        position: SourcePosition::Overlay { row, column: col },
        entry_idx: 0,
        range_id: RangeIdentity::overlay(MARKDOWN_BODY_RANGE),
        block_line_idx: row,
        col_within_range: col,
    }
}

fn select(lines: &[BlockLine], anchor: (usize, u16), head: (usize, u16)) -> String {
    reconstruct_full_selection_text(
        lines,
        &ActiveTextDrag {
            anchor: hit(anchor.0, anchor.1),
            head: hit(head.0, head.1),
            kind: SelectionKind::Linear,
            anchor_content_width: None,
        },
    )
    .unwrap()
}

#[test]
fn markdown_preview_keeps_projected_tabs_math_and_character_cap() {
    let source = "left\t\\( x \\)";
    let markdown = MarkdownContent::new(source);
    assert_eq!(markdown.preview(), "left    $x$");
    assert_eq!(markdown.text(), source);

    let prefix = "a".repeat(118);
    let capped = MarkdownContent::new(format!("{prefix}\tz"));
    assert_eq!(capped.preview(), format!("{prefix} …"));

    let mut streaming = MarkdownContent::streaming();
    streaming.push_chunk("left\t\\");
    assert_eq!(streaming.preview(), "left");
    assert_eq!(streaming.text(), "left\t\\");
    streaming.finish();
    assert_eq!(streaming.preview(), "left    \\");
}

#[test]
fn blank_chunks_remain_original_source_until_and_after_finish() {
    let mut markdown = MarkdownContent::streaming();
    markdown.push_chunk(" \t\n");
    assert_eq!(markdown.text().as_bytes(), b" \t\n");
    markdown.with_source(|source| assert_eq!(source.len(), 3));
    markdown.finish();
    assert_eq!(markdown.text().as_bytes(), b" \t\n");
    assert_eq!(MarkdownContent::new(" \t\n").text().as_bytes(), b" \t\n");
}

#[test]
fn source_bytes_survive_complete_streamed_and_deferred_markdown() {
    let chunks = [" \t\n", "界", "\t", "[é](https://x.test)", "\n\n\\", "( x \\)"];
    let source = " \t\n界\t[é](https://x.test)\n\n\\( x \\)";
    let complete = MarkdownContent::new(source);
    let faithful = MarkdownContent::new_source_faithful(source, None);
    assert_eq!(complete.text().as_bytes(), source.as_bytes());
    assert_eq!(faithful.text().as_bytes(), source.as_bytes());
    for deferred in [false, true] {
        let mut markdown = MarkdownContent::streaming();
        let mut expected = String::new();
        for chunk in chunks {
            expected.push_str(chunk);
            if deferred {
                markdown.push_chunk_deferred(chunk);
            } else {
                markdown.push_chunk(chunk);
                let _ = markdown.output(12);
            }
            markdown.with_source(|source| assert_eq!(source.as_bytes(), expected.as_bytes()));
        }
        markdown.finish();
        assert_eq!(markdown.text(), source);
        assert_eq!(markdown.rendered_plain_text(), complete.rendered_plain_text());
        for width in [4, 12, 80] {
            assert_eq!(
                markdown
                    .output(width)
                    .lines
                    .iter()
                    .map(|line| line.content.to_string())
                    .collect::<Vec<_>>(),
                complete
                    .output(width)
                    .lines
                    .iter()
                    .map(|line| line.content.to_string())
                    .collect::<Vec<_>>(),
            );
        }
    }
}

#[test]
fn raw_and_pretty_copy_keep_distinct_source_and_display_oracles() {
    let source = "界\t[é](https://x.test)";
    let mut agent = AgentMessageBlock::new(source);
    let mut thinking = ThinkingBlock::new(source);
    for raw in [false, true, false] {
        agent.set_raw_mode(raw);
        thinking.set_raw_mode(raw);
        assert_eq!(agent.copy_text(true), source);
        assert_eq!(thinking.copy_text(true), source);
        let painted = if raw {
            "界    [é](https://x.test)"
        } else {
            "界    é (https://x.test)"
        };
        assert_eq!(agent.copy_text(false), painted);
        assert_eq!(thinking.copy_text(false), painted);
        assert_eq!(
            agent.content().pre_wrap_lines().first().unwrap().to_string(),
            painted
        );
    }
}

#[test]
fn original_append_offsets_reconcile_both_replayed_markdown_consumers() {
    let mut agent = AgentMessageBlock::streaming();
    let mut thinking = ThinkingBlock::streaming();
    agent.push_chunk("é\t\\");
    thinking.push_chunk("é\t\\");
    assert_eq!(agent.source_len(), 4);
    assert_eq!(thinking.source_len(), 4);
    agent.push_chunk("( x \\)\t界");
    thinking.push_chunk("( x \\)\t界");
    assert_eq!(agent.source_tail(4).as_deref(), Some("( x \\)\t界"));
    assert_eq!(thinking.source_tail(4).as_deref(), Some("( x \\)\t界"));
    assert_eq!(agent.source_tail(1), None);
    assert_eq!(thinking.source_tail(1), None);
    assert!(agent.merge_replayed_source(&AgentMessageBlock::new("historical\t\\"), 4));
    assert!(thinking.merge_replayed_source(&ThinkingBlock::new("historical\t\\"), 4));
    agent.finish();
    thinking.finish();
    assert_eq!(agent.copy_text(true), "historical\t\\( x \\)\t界");
    assert_eq!(thinking.copy_text(true), "historical\t\\( x \\)\t界");
    assert_eq!(agent.copy_text(false), "historical    x    界");
    assert_eq!(thinking.copy_text(false), "historical    x    界");
}

#[test]
fn append_to_cached_completed_markdown_reopens_the_original_delimiter() {
    let mut markdown = MarkdownContent::new("界\t\\");
    let _ = markdown.output(8);
    markdown.push_chunk("( x \\)\tend");
    let output = markdown.output(8);
    let last = output.lines.len() - 1;
    let last_col = output.lines.get(last).unwrap().content.width().saturating_sub(1) as u16;
    assert_eq!(select(&output.lines, (0, 0), (last, last_col)), "界    x    end");
    assert_eq!(markdown.text(), "界\t\\( x \\)\tend");
    markdown.finish();
    assert_eq!(markdown.rendered_plain_text(), "界    x    end");
}

#[test]
fn unicode_graphemes_after_tabs_keep_paint_and_selection_cells() {
    let markdown = MarkdownContent::new("界\te\u{301}");
    let output = markdown.output(80);
    let line = output.lines.first().unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 1));
    buffer.set_line_safe_bidi(0, 0, &line.content, 80);
    assert_eq!(line.content.width(), 7);
    assert_eq!(buffer.cell((6, 0)).unwrap().symbol(), "e\u{301}");
    assert_eq!(select(&output.lines, (0, 6), (0, 6)), "e\u{301}");
    assert_eq!(select(&output.lines, (0, 1), (0, 6)), "界    e\u{301}");
    assert_eq!(markdown.text(), "界\te\u{301}");
}

#[test]
fn tab_paint_selection_and_wrapped_link_hitboxes_share_display_cells() {
    let mut markdown = MarkdownContent::new("界\t[é](https://x.test)");
    for (raw, expected) in [
        (false, "界    é (https://x.test)"),
        (true, "界    [é](https://x.test)"),
    ] {
        markdown.set_raw_mode(raw);
        let wide = markdown.output(80);
        let first = wide.lines.first().unwrap();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 1));
        buffer.set_line_safe_bidi(0, 0, &first.content, 80);
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "界");
        for column in 2..6 {
            assert_eq!(buffer.cell((column, 0)).unwrap().symbol(), " ");
        }
        let label_column = if raw { 7 } else { 6 };
        assert_eq!(buffer.cell((label_column, 0)).unwrap().symbol(), "é");
        assert_eq!(select(&wide.lines, (0, 2), (0, 5)), "    ");
        assert_eq!(select(&wide.lines, (0, label_column), (0, label_column)), "é");
        assert_eq!(select(&wide.lines, (0, 1), (0, 1)), "界");
        for width in [4, 12, 80] {
            let output = markdown.output(width);
            assert!(output.lines.iter().all(|line| line.content.width() <= width));
            let last = output.lines.len() - 1;
            let last_col = output.lines.get(last).unwrap().content.width().saturating_sub(1) as u16;
            assert_eq!(select(&output.lines, (0, 0), (last, last_col)), expected);
            let mut overlay = LinkOverlay::new();
            markdown.with_hyperlinks(|links| {
                crate::scrollback::render::map_hyperlinks_to_overlay(
                    links, &output, 0, 0, 100, 0, 0, &[], None, &mut overlay,
                );
            });
            let mut labels = String::new();
            let mut urls = String::new();
            for link in overlay.links() {
                let selected = select(
                    &output.lines,
                    (usize::from(link.screen_row), link.col_start),
                    (usize::from(link.screen_row), link.col_end - 1),
                );
                match link.id {
                    Some(0) => labels.push_str(&selected),
                    Some(1) => urls.push_str(&selected),
                    other => panic!("unexpected link id {other:?}"),
                }
            }
            assert_eq!(labels, "é");
            assert_eq!(urls, "https://x.test");
        }
    }
}
