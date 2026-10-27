//! The F7 footer: a dim hint for the cursor line, and the scope line of its own section.

use crate::app::agent_view::sidebars::SidebarHeightsCache;
use crate::theme::Theme;
use crate::views::sidebar::SidebarHeights;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

const FOOTER_ROWS: u16 = 2;

/// Border cells around the column, on each axis.
const BORDER: u16 = 2;

/// Body rows that stay in view however much the footer wants.
const MIN_BODY_ROWS: u16 = 5;

/// Header and footer heights of the F7 column: two footer lines when the column is tall enough to keep
/// [`MIN_BODY_ROWS`], else the shared footer height.
pub(super) fn heights(cache: &mut SidebarHeightsCache, column: Rect) -> SidebarHeights {
    let base = cache.heights_for(column.width);
    let room = column
        .height
        .saturating_sub(BORDER + base.header + MIN_BODY_ROWS);
    SidebarHeights {
        footer: FOOTER_ROWS.min(room).max(base.footer),
        ..base
    }
}

/// The footer lines: the keyboard hint for the cursor line, and the scope line of its section.
pub(super) fn lines(status: &'static str, scope: Option<&'static str>, theme: &Theme) -> [Line<'static>; 2] {
    let dim = theme.muted();
    [
        Line::from(Span::styled(status, dim)),
        scope.map_or_else(Line::default, |scope| Line::from(Span::styled(scope, dim))),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tall_column_gets_two_footer_rows_and_a_short_one_keeps_its_body() {
        let mut cache = SidebarHeightsCache::default();
        let base = cache.heights_for(56);

        let tall = heights(&mut cache, Rect::new(0, 0, 56, 40));
        assert_eq!(tall.footer, 2);
        assert_eq!(tall.header, base.header);

        let short = heights(&mut cache, Rect::new(0, 0, 56, 9));
        assert_eq!(
            short.footer, base.footer,
            "the hint rows give way to the body"
        );
        let body = 9 - BORDER - short.header - short.footer;
        assert!(body >= 4, "{body}");
    }
}
