use super::*;
use ratatui::style::Color;

#[test]
fn virtual_height_search_and_tail_paint_keep_rows_beyond_terminal_coordinates() {
    let theme = Theme::current();
    let source = (0..70_000).map(|row| format!("ROW{row:05}"))
        .collect::<Vec<_>>().join("\n");
    let entry = ScrollbackEntry::new(RenderBlock::stub(source, Color::Blue));
    let renderer = EntryRenderer::new(&entry, &theme);
    assert_eq!(renderer.desired_height(40), 70_002);
    assert_eq!(renderer.rendered_row_of_logical_line(40, 65_536), 65_537);
    assert_eq!(renderer.logical_line_of_rendered_row(40, 65_537), 65_536);
    let renderer = renderer.with_skip_rows(65_537);
    let area = Rect::new(0, 0, 40, 3);
    let mut buffer = Buffer::empty(area);
    renderer.render(area, &mut buffer);
    for row in 0..area.height {
        let text = (0..area.width).filter_map(|x| buffer.cell((x, row)))
            .map(|cell| cell.symbol()).collect::<String>();
        assert!(text.contains(&format!("ROW{:05}", 65_536 + usize::from(row))));
    }
    let renderer = renderer.with_skip_rows(69_998);
    let mut buffer = Buffer::empty(area);
    renderer.render(area, &mut buffer);
    let painted = buffer.content.iter().map(|cell| cell.symbol()).collect::<String>();
    assert!(painted.contains("ROW69999"));
}

#[test]
fn native_wrap_continuation_survives_a_physical_batch_boundary() {
    let theme = Theme::current();
    let mut appearance = AppearanceConfig::default();
    appearance.show_timestamps = false;
    let entry = ScrollbackEntry::new(RenderBlock::user_prompt("a".repeat(700_000)));
    let renderer = EntryRenderer::new(&entry, &theme).with_appearance(appearance);
    assert!(renderer.desired_height(12) > 65_535);
    let renderer = renderer.with_skip_rows(65_536);
    let area = Rect::new(0, 0, 12, 2);
    let mut buffer = Buffer::empty(area);
    renderer.render(area, &mut buffer);
    assert_eq!(renderer.row_soft_wraps(area.height), vec![true, true]);
}
