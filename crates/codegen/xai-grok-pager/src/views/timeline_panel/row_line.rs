//! One styled outline row: indent, disclosure glyph and label.

use ratatui::text::{Line, Span};

use crate::render::line_utils::truncate_line;
use crate::theme::Theme;
use crate::views::row_style::{Disclosure, OutlineClass, RowState, outline_style};

/// Columns each tree level is indented by.
pub(super) const INDENT_WIDTH: u16 = 2;

/// Text and semantics of a row, before layout.
#[derive(Debug)]
pub(super) struct RowLabel {
    pub(super) class: OutlineClass,
    pub(super) state: RowState,
    pub(super) text: String,
}

impl RowLabel {
    pub(super) fn plain(class: OutlineClass, text: String) -> Self {
        Self {
            class,
            state: RowState::default(),
            text,
        }
    }
}

pub(super) fn outline_line(
    depth: u8,
    branch: Option<bool>,
    label: RowLabel,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let style = outline_style(label.class, label.state, theme);
    let glyph = Disclosure::of_branch(branch).glyph();
    let indent = " ".repeat(usize::from(depth) * usize::from(INDENT_WIDTH));
    truncate_line(
        Line::from(vec![
            Span::raw(indent),
            Span::styled(glyph.to_string(), style.glyph),
            Span::raw(" "),
            Span::styled(label.text, style.label),
        ]),
        width,
    )
}
