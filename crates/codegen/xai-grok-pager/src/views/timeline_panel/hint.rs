//! Footer hint of the outline column: the keys and gestures that work on the cursor row, packed into as few rows as the column width allows.

use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::scrollback::state::TimelineRowKind;
use crate::theme::Theme;
use crate::views::sidebar::SidebarHeights;

const SEPARATOR: &str = " \u{00B7} ";

/// Border cells around the column, on each axis.
const BORDER: u16 = 2;

const MAX_FOOTER_ROWS: usize = 10;

/// Rows of the outline that stay in view however much the footer wants.
const MIN_BODY_ROWS: u16 = 5;

/// What decides which hints are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterContext {
    /// Kind of the row under the cursor.
    pub cursor: Option<TimelineRowKind>,
    /// Whether `d` can remove the cursor row from the model context.
    pub removable: bool,
}

impl FooterContext {
    /// The context that offers every hint, for sizing the footer once for all cursor rows.
    pub fn widest() -> Self {
        Self {
            cursor: Some(TimelineRowKind::Entry {
                id: crate::scrollback::entry::EntryId::new(0),
            }),
            removable: true,
        }
    }
}

/// Key and action, most important first: when the column is too narrow for everything, the tail is left out.
/// Copy and viewer act on a single entry, so a group row does not offer them.
fn hints(context: &FooterContext) -> Vec<(&'static str, &'static str)> {
    let single_entry = !matches!(context.cursor, Some(TimelineRowKind::Group { .. }));
    let mut hints = vec![
        ("\u{2191}\u{2193}", "move"),
        ("\u{2190}\u{2192}", "close/open"),
        ("Space", "fold"),
        ("Enter", "jump"),
        ("*", "fold all"),
        ("Shift+Enter", "toggle siblings"),
        ("Ctrl+Enter", "toggle subtree"),
        ("+/-", "open/close siblings"),
        ("e", "fold entry"),
    ];
    if single_entry {
        hints.extend([("y", "copy"), ("Y", "copy meta"), ("v", "viewer")]);
    }
    hints.push(("R", "rewind"));
    if context.removable {
        hints.push(("d", "remove"));
    }
    hints.push(("t", "turn default"));
    hints.push(("Esc", "close"));
    hints
}

struct PackedRow {
    spans: Vec<Span<'static>>,
    width: usize,
}

pub fn footer_lines(
    column_width: u16,
    theme: &Theme,
    context: &FooterContext,
) -> Vec<Line<'static>> {
    let inner = usize::from(column_width.saturating_sub(BORDER));
    let separator_width = SEPARATOR.width();
    let mut rows: Vec<PackedRow> = Vec::new();
    for (key, action) in hints(context) {
        let item_width = key.width() + 1 + action.width();
        if item_width > inner {
            continue;
        }
        if let Some(row) = rows
            .last_mut()
            .filter(|row| row.width + separator_width + item_width <= inner)
        {
            row.spans.push(Span::styled(SEPARATOR, theme.dim()));
            row.spans.extend(item_spans(key, action, theme));
            row.width += separator_width + item_width;
        } else if rows.len() < MAX_FOOTER_ROWS {
            rows.push(PackedRow {
                spans: item_spans(key, action, theme).into(),
                width: item_width,
            });
        }
    }
    rows.into_iter().map(|row| Line::from(row.spans)).collect()
}

/// Where settings are saved, as the last footer row (the first to go when the footer is cut); `None` when the column is too narrow to show it whole.
pub fn status_line(column_width: u16, theme: &Theme, status: &str) -> Option<Line<'static>> {
    let inner = usize::from(column_width.saturating_sub(BORDER));
    (status.width() <= inner).then(|| Line::from(Span::styled(status.to_owned(), theme.muted())))
}

/// Footer height for a column of `column_height` rows: what the widest hint set and the saving status need, never below
/// the shared footer height and never leaving fewer than [`MIN_BODY_ROWS`] for the outline.
pub fn footer_rows(
    base: SidebarHeights,
    column_width: u16,
    column_height: u16,
    theme: &Theme,
    status: &str,
) -> u16 {
    let wanted = footer_lines(column_width, theme, &FooterContext::widest()).len()
        + usize::from(status_line(column_width, theme, status).is_some());
    let room = column_height.saturating_sub(BORDER + base.header + MIN_BODY_ROWS);
    u16::try_from(wanted)
        .unwrap_or(u16::MAX)
        .min(room)
        .max(base.footer)
}

fn item_spans(key: &'static str, action: &'static str, theme: &Theme) -> [Span<'static>; 2] {
    [
        Span::styled(key, theme.fg(theme.accent_system)),
        Span::styled(format!(" {action}"), theme.muted()),
    ]
}
