//! F6 on the left and F7 on the right split the screen 25/50/25; either alone takes 35%.
use super::{AgentView, AppRenderParams, BannerSlotParams, test_fixtures};
use crate::actions::ActionRegistry;
use crate::scrollback::render::ScratchBuffer;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const WIDTH: u16 = 160;

fn draw(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, WIDTH, 40);
    let mut buf = Buffer::empty(area);
    agent.draw(
        area,
        &mut buf,
        &ActionRegistry::defaults(),
        &mut ScratchBuffer::new(),
        None,
        false,
        BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        AppRenderParams::default(),
    );
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .filter_map(|x| buf.cell((x, y)).map(|cell| cell.symbol().to_string()))
        .collect()
}

/// Column of the first cell of `needle` on any of the first rows.
fn column_of(buf: &Buffer, needle: &str) -> Option<usize> {
    (0..4).find_map(|y| {
        let row = row_text(buf, y);
        row.find(needle).map(|byte| row[..byte].chars().count())
    })
}

fn press(agent: &mut AgentView, code: KeyCode) {
    let _ = agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    );
}

#[test]
fn the_side_columns_follow_which_panels_are_open() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = test_fixtures::make_agent();
    assert_eq!(column_of(&draw(&mut agent), "Timeline"), None);

    press(&mut agent, KeyCode::F(6));
    let timeline_only = draw(&mut agent);
    assert!(column_of(&timeline_only, "Timeline").is_some_and(|x| x < 4));
    assert_eq!(column_of(&timeline_only, "Context sent"), None);

    press(&mut agent, KeyCode::F(7));
    let both = draw(&mut agent);
    let context_x = column_of(&both, "Context sent").unwrap_or_else(|| panic!("F7 header: {}", row_text(&both, 1)));
    assert!(
        (usize::from(WIDTH) * 3 / 4 - 2..=usize::from(WIDTH) * 3 / 4 + 3).contains(&context_x),
        "the right column starts at 75%: {context_x}"
    );
    assert!(column_of(&both, "Timeline").is_some());

    press(&mut agent, KeyCode::F(6));
    let context_only = draw(&mut agent);
    assert_eq!(column_of(&context_only, "Timeline"), None);
    let context_x = column_of(&context_only, "Context sent").expect("F7 header");
    assert!(
        (usize::from(WIDTH) * 65 / 100 - 2..=usize::from(WIDTH) * 65 / 100 + 3).contains(&context_x),
        "the right column starts at 65%: {context_x}"
    );
}
