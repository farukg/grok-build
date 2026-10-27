use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::render::wrapping::{RtOptions, WrapSemantics, word_wrap_lines_with_joiners};
use crate::scrollback::types::{BlockContext, BlockLine, DisplayMode, Selectable};
use crate::theme::Theme;

pub(crate) fn wrapped_body(
    text: &str,
    style: Style,
    width: usize,
    indent: &'static str,
) -> Vec<BlockLine> {
    wrapped_lines(
        text.split('\n').map(|line| Line::from(Span::styled(line.to_owned(), style))),
        width, indent,
    )
}

pub(crate) fn wrapped_lines(
    lines: impl IntoIterator<Item = Line<'static>>,
    width: usize,
    indent: &'static str,
) -> Vec<BlockLine> {
    if width == 0 {
        return Vec::new();
    }
    let indent = &indent[..indent.len().min(width.saturating_sub(1))];
    let body_width = width.saturating_sub(indent.len()).max(1);
    let options = RtOptions::new(body_width).semantics(WrapSemantics::PlainContent);
    let (rows, joiners) = word_wrap_lines_with_joiners(lines, options);
    rows.into_iter().zip(joiners).map(|(row, joiner)| {
        let mut spans = vec![Span::raw(indent)];
        spans.extend(row.spans);
        let end = spans.len();
        BlockLine {
            selectable: Selectable::Spans(1..end),
            content: Line::from(spans),
            joiner,
            ..Default::default()
        }
    }).collect()
}

pub(super) fn push_content_box(
    lines: &mut Vec<BlockLine>,
    text: &str,
    theme: &Theme,
    ctx: &BlockContext,
) {
    let max_rows = match ctx.mode {
        DisplayMode::Truncated => 3,
        DisplayMode::Collapsed | DisplayMode::Expanded => usize::MAX,
    };
    lines.push(BlockLine::separator(Line::from("")).with_panel_background(theme.bg_dark));
    let mut shown = 0;
    'body: for source_line in text.split('\n') {
        for row in wrapped_body(source_line, theme.primary(), ctx.content_width(), "  ") {
            if shown == max_rows {
                lines.push(BlockLine::separator(Line::from(Span::styled("  …", theme.muted())))
                    .with_panel_background(theme.bg_dark));
                break 'body;
            }
            lines.push(row.with_panel_background(theme.bg_dark));
            shown += 1;
        }
    }
    lines.push(BlockLine::separator(Line::from("")).with_panel_background(theme.bg_dark));
}
