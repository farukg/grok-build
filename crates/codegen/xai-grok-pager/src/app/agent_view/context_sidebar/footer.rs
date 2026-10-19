//! The F7 footer: the hint for the cursor line, where edits are saved, and the standing note.

use crate::app::agent_view::sidebars::SidebarHeightsCache;
use crate::views::sidebar::SidebarHeights;
use ratatui::layout::Rect;
use ratatui::text::Line;

const FOOTER_ROWS: u16 = 3;

/// Border cells around the column, on each axis.
const BORDER: u16 = 2;

/// Body rows that stay in view however much the footer wants.
const MIN_BODY_ROWS: u16 = 5;

/// Header and footer heights of the F7 column: three footer lines when the column is tall enough to keep
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

/// The footer lines; with the shared two rows only the first two are shown, so the saving line is second.
pub(super) fn lines(status: &'static str, saving: &'static str) -> [Line<'static>; 3] {
    [
        Line::from(status),
        Line::from(saving),
        Line::from("Your current message is always sent."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tall_column_gets_three_footer_rows_and_a_short_one_keeps_its_body() {
        let mut cache = SidebarHeightsCache::default();
        let base = cache.heights_for(56);

        let tall = heights(&mut cache, Rect::new(0, 0, 56, 40));
        assert_eq!(tall.footer, 3);
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
