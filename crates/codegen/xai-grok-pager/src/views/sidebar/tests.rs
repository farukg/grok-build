use super::layout::part;
use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

struct TestHosted {
    height: u16,
    calls: std::cell::Cell<(u16, u16)>,
}

impl HostedSection for TestHosted {
    fn desired_height(&self) -> u16 {
        self.height
    }

    fn render(&self, buf: &mut Buffer, area: Rect, row_offset: u16, _theme: &Theme) {
        self.calls.set((area.y, row_offset));
        for row in 0..area.height {
            buf.set_string(
                area.x,
                area.y + row,
                "host",
                ratatui::style::Style::default(),
            );
        }
    }
}

#[test]
fn columns_split_deterministically_and_cover_the_area() {
    for width in [180, 140, 120, 60] {
        let area = Rect::new(7, 3, width, 30);
        for open in [
            SidebarsOpen::None,
            SidebarsOpen::Left,
            SidebarsOpen::Right,
            SidebarsOpen::Both,
        ] {
            let columns = screen_columns(area, open);
            assert_eq!(
                columns.left.width + columns.center.width + columns.right.width,
                width
            );
            assert_eq!(columns.left.x, area.x);
            assert_eq!(columns.right.right(), area.right());
            match open {
                SidebarsOpen::None => assert_eq!(columns.center.width, width),
                SidebarsOpen::Left => assert_eq!(
                    (columns.left.width, columns.center.width),
                    (part(width, 35), part(width, 65))
                ),
                SidebarsOpen::Right => assert_eq!(
                    (columns.center.width, columns.right.width),
                    (part(width, 65), part(width, 35))
                ),
                SidebarsOpen::Both => assert_eq!(
                    (
                        columns.left.width,
                        columns.center.width,
                        columns.right.width
                    ),
                    (
                        part(width, 25),
                        width - 2 * part(width, 25),
                        part(width, 25)
                    )
                ),
            }
        }
    }
}

#[test]
fn hosted_sections_clip_at_viewport_and_keep_scroll_row_offset() {
    let hosted = TestHosted {
        height: 6,
        calls: std::cell::Cell::new((0, 0)),
    };
    let sections = [SidebarSection::Hosted(&hosted)];
    let content = SidebarContent {
        header: &[],
        sections: &sections,
        footer: &[],
    };
    let area = Rect::new(0, 0, 20, 7);
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    state.scroll = 2;
    let mut buffer = Buffer::empty(area);
    let layout = Sidebar::render(
        SidebarRender {
            area,
            content,
            heights: SidebarHeights {
                header: 0,
                footer: 0,
            },
            edge: SidebarEdge::Left,
            state: &state,
            hovered: None,
            focused: false,
            theme: &Theme::current(),
        },
        &mut buffer,
    );
    assert_eq!(hosted.calls.get(), (layout.body.y, 2));
    assert_eq!(layout.body.height, 5);
    assert_eq!(buffer[(layout.body.x, layout.body.y)].symbol(), "h");
}

#[test]
fn row_sections_stack_and_detail_index_is_typed() {
    let rows = [SidebarRow {
        left: Line::from("Context row"),
        right: Some(Line::from("ON")),
        detail: &[Line::from("Context detail")],
    }];
    let sections = [SidebarSection::Rows {
        title: Line::from("Context"),
        rows: &rows,
    }];
    let content = SidebarContent {
        header: &[],
        sections: &sections,
        footer: &[],
    };
    let area = Rect::new(0, 0, 30, 8);
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    assert_eq!(
        state.line_at(0),
        Some(SidebarLine::SectionTitle(SectionIdx(0)))
    );
    assert_eq!(
        state.line_at(1),
        Some(SidebarLine::Row(SectionIdx(0), RowIdx(0)))
    );
    assert_eq!(
        detail_for(
            content.sections,
            Some(SidebarLine::SectionTitle(SectionIdx(0)))
        ),
        None
    );
    assert_eq!(
        detail_for(
            content.sections,
            Some(SidebarLine::Row(SectionIdx(0), RowIdx(0)))
        ),
        Some((SidebarLine::Row(SectionIdx(0), RowIdx(0)), rows[0].detail))
    );
    let layout = SidebarLayout::compute(
        area,
        SidebarHeights {
            header: 0,
            footer: 0,
        },
    );
    assert_eq!(
        layout.hit(layout.body.x, layout.body.y + 1, &state),
        Some(SidebarHit::Line(SidebarLine::Row(SectionIdx(0), RowIdx(0))))
    );
}

#[test]
fn cursor_remains_visible_when_moving_in_both_directions() {
    let rows = [
        SidebarRow {
            left: Line::from("1"),
            right: None,
            detail: &[],
        },
        SidebarRow {
            left: Line::from("2"),
            right: None,
            detail: &[],
        },
        SidebarRow {
            left: Line::from("3"),
            right: None,
            detail: &[],
        },
    ];
    let sections = [SidebarSection::Rows {
        title: Line::from("Rows"),
        rows: &rows,
    }];
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    state.move_cursor(1, 2);
    assert_eq!(state.cursor, Some(SidebarLine::SectionTitle(SectionIdx(0))));
    state.move_cursor(3, 2);
    assert_eq!(state.scroll, 2);
    assert_eq!(
        state.cursor,
        Some(SidebarLine::Row(SectionIdx(0), RowIdx(2)))
    );
    state.move_cursor(-2, 2);
    assert_eq!(state.scroll, 1);
}

#[test]
fn fixed_rows_stay_put_and_scrollbar_tracks_overflow() {
    let rows = [
        SidebarRow {
            left: Line::from(vec![Span::raw("one")]),
            right: None,
            detail: &[],
        },
        SidebarRow {
            left: Line::from(vec![Span::raw("two")]),
            right: None,
            detail: &[],
        },
    ];
    let sections = [SidebarSection::Rows {
        title: Line::from("Rows"),
        rows: &rows,
    }];
    let header = [Line::from("fixed header")];
    let footer = [Line::from("fixed footer")];
    let content = SidebarContent {
        header: &header,
        sections: &sections,
        footer: &footer,
    };
    let area = Rect::new(0, 0, 24, 8);
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    let heights = SidebarHeights {
        header: 1,
        footer: 1,
    };
    let mut buffer = Buffer::empty(area);
    let layout = Sidebar::render(
        SidebarRender {
            area,
            content,
            heights,
            edge: SidebarEdge::Right,
            state: &state,
            hovered: None,
            focused: false,
            theme: &Theme::current(),
        },
        &mut buffer,
    );
    assert_eq!(buffer[(layout.header.x, layout.header.y)].symbol(), "f");
    assert_eq!(buffer[(layout.footer.x, layout.footer.y)].symbol(), "f");
    assert_eq!(layout.body.height, 4);
    let one_row = [SidebarSection::Rows {
        title: Line::from("Only"),
        rows: &rows[..1],
    }];
    let mut short_state = SidebarState::default();
    short_state.rebuild_layout(&one_row);
    let mut no_overflow = Buffer::empty(area);
    Sidebar::render(
        SidebarRender {
            area,
            content: SidebarContent {
                header: &header,
                sections: &one_row,
                footer: &footer,
            },
            heights,
            edge: SidebarEdge::Right,
            state: &short_state,
            hovered: None,
            focused: false,
            theme: &Theme::current(),
        },
        &mut no_overflow,
    );
    assert!(
        no_overflow
            .content()
            .iter()
            .all(|cell| cell.symbol() != "█")
    );
}

#[test]
fn sidebar_fixed_sizes_follow_shared_height_sources() {
    let composer = PromptWidget::default();
    let width = 34;
    let heights = SidebarHeights::from_shared_sources(&composer, width);
    assert_eq!(
        heights.header,
        crate::views::agent::SESSION_HEADER_ROW_HEIGHT
    );
    assert_eq!(
        heights.footer,
        composer.desired_height(width, &PromptStyle::default(), false, u16::MAX)
    );
}

fn styled_rows() -> [SidebarRow<'static>; 2] {
    let row = |label: &'static str, value: &'static str| SidebarRow {
        left: Line::from(Span::styled(label, Style::default().fg(Color::Red))),
        right: Some(Line::from(Span::styled(
            value,
            Style::default().fg(Color::Green),
        ))),
        detail: &[],
    };
    [row("first", "ON"), row("second", "OFF")]
}

fn render_rows(
    rows: &[SidebarRow<'_>],
    cursor: Option<SidebarLine>,
    hovered: Option<SidebarLine>,
) -> (Buffer, SidebarLayout) {
    let sections = [SidebarSection::Rows {
        title: Line::from(Span::styled("Title", Style::default().fg(Color::Blue))),
        rows,
    }];
    let area = Rect::new(0, 0, 30, 8);
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    state.cursor = cursor;
    let mut buffer = Buffer::empty(area);
    let layout = Sidebar::render(
        SidebarRender {
            area,
            content: SidebarContent {
                header: &[],
                sections: &sections,
                footer: &[],
            },
            heights: SidebarHeights {
                header: 0,
                footer: 0,
            },
            edge: SidebarEdge::Right,
            state: &state,
            hovered,
            focused: cursor.is_some(),
            theme: &Theme::current(),
        },
        &mut buffer,
    );
    (buffer, layout)
}

#[test]
fn span_colours_survive_without_an_overlay() {
    let _theme = crate::theme::cache::pin_theme();
    let rows = styled_rows();
    let (buffer, layout) = render_rows(&rows, None, None);
    let first = layout.body.y + 1;
    assert_eq!(buffer[(layout.body.x, first)].fg, Color::Red);
    let right_x = layout.body.right() - 1 - 2;
    assert_eq!(buffer[(right_x, first)].fg, Color::Green);
    assert_eq!(buffer[(layout.body.x, layout.body.y)].fg, Color::Blue);
}

#[test]
fn cursor_and_hover_overlays_add_a_background_and_keep_span_colours() {
    let _theme = crate::theme::cache::pin_theme();
    let theme = Theme::current();
    let rows = styled_rows();
    let first = SidebarLine::Row(SectionIdx(0), RowIdx(0));
    let second = SidebarLine::Row(SectionIdx(0), RowIdx(1));
    let title = SidebarLine::SectionTitle(SectionIdx(0));
    let (buffer, layout) = render_rows(&rows, Some(first), Some(second));
    let (x, y) = (layout.body.x, layout.body.y);
    assert_eq!(buffer[(x, y + 1)].fg, Color::Red);
    assert_eq!(buffer[(x, y + 2)].fg, Color::Red);
    assert_eq!(Some(buffer[(x, y + 1)].bg), theme.selection_overlay().bg);
    assert_eq!(Some(buffer[(x, y + 2)].bg), theme.hover_overlay().bg);
    assert_eq!(buffer[(x, y)].bg, theme.bg_base);
    let right_x = layout.body.right() - 1 - 2;
    assert_eq!(buffer[(right_x, y + 1)].fg, Color::Green);
    assert_eq!(
        Some(buffer[(right_x, y + 1)].bg),
        theme.selection_overlay().bg
    );

    let (buffer, layout) = render_rows(&rows, Some(title), None);
    assert_eq!(buffer[(layout.body.x, layout.body.y)].fg, Color::Blue);
    assert_eq!(
        Some(buffer[(layout.body.x, layout.body.y)].bg),
        theme.selection_overlay().bg,
        "a section title shows the cursor"
    );
}

#[test]
fn a_section_without_rows_keeps_only_its_title_line() {
    let rows = styled_rows();
    let sections = [
        SidebarSection::Rows {
            title: Line::from("Folded"),
            rows: &[],
        },
        SidebarSection::Rows {
            title: Line::from("Open"),
            rows: &rows,
        },
    ];
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    assert_eq!(
        state.line_at(0),
        Some(SidebarLine::SectionTitle(SectionIdx(0)))
    );
    assert_eq!(
        state.line_at(1),
        Some(SidebarLine::SectionTitle(SectionIdx(1)))
    );
    assert_eq!(
        state.line_at(2),
        Some(SidebarLine::Row(SectionIdx(1), RowIdx(0)))
    );
    assert_eq!(state.line_at(4), None);
    let layout = SidebarLayout::compute(
        Rect::new(0, 0, 30, 8),
        SidebarHeights {
            header: 0,
            footer: 0,
        },
    );
    assert_eq!(
        layout.hit(layout.body.x, layout.body.y + 2, &state),
        Some(SidebarHit::Line(SidebarLine::Row(SectionIdx(1), RowIdx(0))))
    );
}

#[test]
fn reveal_cursor_scrolls_the_cursor_line_into_view() {
    let rows = styled_rows();
    let sections = [SidebarSection::Rows {
        title: Line::from("Rows"),
        rows: &rows,
    }];
    let mut state = SidebarState::default();
    state.rebuild_layout(&sections);
    state.cursor = Some(SidebarLine::Row(SectionIdx(0), RowIdx(1)));
    state.reveal_cursor(2);
    assert_eq!(state.scroll, 1, "line 2 is the last of a two-line viewport");
    state.cursor = Some(SidebarLine::SectionTitle(SectionIdx(0)));
    state.reveal_cursor(2);
    assert_eq!(state.scroll, 0);
    state.scroll = 2;
    state.clamp_scroll(2);
    assert_eq!(
        state.scroll, 1,
        "three lines in a two-line viewport scroll at most one"
    );
}
