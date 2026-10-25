use super::*;
use xai_grok_shell::tools::{TodoItem, TodoStatus};

fn item(content: &str, status: TodoStatus) -> TodoItem {
    TodoItem { content: content.into(), status, priority: Default::default(), meta: None }
}

fn draw_short(agent: &mut AgentView) -> Buffer {
    let area = Rect::new(0, 0, 160, 20);
    let mut buf = Buffer::empty(area);
    agent.draw(
        area, &mut buf, &ActionRegistry::defaults(), &mut ScratchBuffer::new(),
        None, false, BannerSlotParams::none(), false, &mut Vec::new(),
        AppRenderParams::default(),
    );
    buf
}

fn control(agent: &mut AgentView, code: char) {
    let _ = agent.handle_input(
        &Event::Key(KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)),
        &ActionRegistry::defaults(),
    );
}

fn text_style(buf: &Buffer, text: &str) -> Vec<(ratatui::style::Color, ratatui::style::Modifier)> {
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let painted: String = (x..buf.area.width)
                .filter_map(|col| buf.cell((col, y)).map(|cell| cell.symbol()))
                .collect();
            if painted.starts_with(text) {
                return (x..x + text.chars().count() as u16)
                    .map(|col| {
                        let cell = buf.cell((col, y)).unwrap();
                        (cell.fg, cell.modifier)
                    })
                    .collect();
            }
        }
    }
    panic!("missing painted text: {text}");
}

#[test]
fn f7_hidden_todos_reuse_preparation_and_materialize_only_painted_rows() {
    let _theme = crate::theme::cache::pin_theme();
    let root = tempfile::tempdir().unwrap();
    let mut agent = test_fixtures::make_agent();
    agent.open_panel_settings_in(root.path());
    agent.todo.overlay.visible = false;
    agent.todo.update_todos((0..64).flat_map(|_| [
        item("Literal pending work", TodoStatus::Pending),
        item("Literal active work", TodoStatus::InProgress),
        item("Literal finished work", TodoStatus::Completed),
        item("Literal cancelled work", TodoStatus::Cancelled),
    ]).collect());
    assert_eq!(agent.todo.todos().len(), 256);
    let prepared = agent.todo.preparation_counts().0;
    let _ = press(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    let _ = draw(&mut agent);
    let _ = press(&mut agent, KeyCode::Home);
    let before = agent.todo.preparation_counts();
    let buf = draw(&mut agent);
    assert!(screen(&buf).contains("□ Literal pending work"));
    assert_eq!(agent.todo.preparation_counts().0, prepared);
    assert!(agent.todo.preparation_counts().1 - before.1 <= buf.area.height as usize);
    for _ in 0..8 {
        let before = agent.todo.preparation_counts();
        let _ = draw(&mut agent);
        let after = agent.todo.preparation_counts();
        assert_eq!(after.0, before.0);
        assert!(after.1 - before.1 <= buf.area.height as usize);
    }

    let _ = click_row(&mut agent, &buf, "\u{25BE} Todos");
    let collapsed = draw(&mut agent);
    assert!(screen(&collapsed).contains("\u{25B8} Todos"));
    assert!(!screen(&collapsed).contains("Literal pending work"));
    let hidden = agent.todo.preparation_counts();
    for _ in 0..8 {
        let _ = press(&mut agent, KeyCode::End);
        let _ = draw_short(&mut agent);
        let _ = press(&mut agent, KeyCode::Home);
        let _ = draw(&mut agent);
    }
    assert_eq!(agent.todo.preparation_counts(), hidden);
    assert!(!agent.todo.overlay.visible);

    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25B8} Todos");
    let _ = draw_short(&mut agent);
    let _ = press(&mut agent, KeyCode::End);
    let offscreen = draw_short(&mut agent);
    assert!(!screen(&offscreen).contains("Literal "));
    let hidden = agent.todo.preparation_counts();
    for _ in 0..8 {
        let _ = press(&mut agent, KeyCode::Up);
        let _ = draw_short(&mut agent);
        let _ = agent.handle_input(
            &Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: offscreen.area.right() - 12,
                row: 8,
                modifiers: KeyModifiers::NONE,
            }),
            &ActionRegistry::defaults(),
        );
        let _ = press(&mut agent, KeyCode::End);
        let _ = draw_short(&mut agent);
    }
    assert_eq!(agent.todo.preparation_counts(), hidden);

    let _ = press(&mut agent, KeyCode::Home);
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25BE} Todos");
    let _ = draw(&mut agent);
    let before_update = agent.todo.preparation_counts();
    agent.todo.update_todos(vec![
        item("New pending", TodoStatus::Pending),
        item("New active", TodoStatus::InProgress),
        item("New finished", TodoStatus::Completed),
        item("New cancelled", TodoStatus::Cancelled),
    ]);
    let updated = agent.todo.preparation_counts();
    assert_eq!(updated.0, before_update.0 + 1);
    assert_eq!(updated.1, before_update.1);
    for _ in 0..8 { let _ = draw(&mut agent); }
    assert_eq!(agent.todo.preparation_counts(), updated);
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25B8} Todos");
    let sidebar = draw(&mut agent);
    let expected = [
        "□ New pending".to_string(),
        "▶ New active".to_string(),
        format!("{} New finished", crate::glyphs::check_mark()),
        format!("{} New cancelled", crate::glyphs::ballot_x()),
    ];
    let painted = screen(&sidebar);
    for text in &expected {
        assert!(painted.contains(text.as_str()));
    }
    assert!(!painted.contains("Literal "));
    assert!(painted.find("Dock").unwrap() < painted.find("Todos").unwrap());
    assert!(painted.find("Todos").unwrap() < painted.find("Context").unwrap());

    control(&mut agent, 't');
    assert!(agent.todo.overlay.visible);
    let shared = screen(&draw(&mut agent));
    for text in &expected {
        assert_eq!(shared.matches(text.as_str()).count(), 2);
    }
    assert_eq!(agent.todo.preparation_counts().0, updated.0);
    let _ = press(&mut agent, KeyCode::F(7));
    let standalone = draw(&mut agent);
    for text in &expected {
        assert!(screen(&standalone).contains(text.as_str()));
        assert_eq!(text_style(&sidebar, text), text_style(&standalone, text));
    }
    assert_eq!(agent.todo.preparation_counts().0, updated.0);
    let _ = press(&mut agent, KeyCode::Char('h'));
    let filtered = screen(&draw(&mut agent));
    assert!(filtered.contains("□ New pending"));
    assert!(filtered.contains("▶ New active"));
    assert!(!filtered.contains("New finished"));
    assert!(!filtered.contains("New cancelled"));
    control(&mut agent, 't');
    let _ = press(&mut agent, KeyCode::F(7));
    let _ = draw(&mut agent);
    let _ = press(&mut agent, KeyCode::Home);
    let filtered = screen(&draw(&mut agent));
    assert!(filtered.contains("□ New pending"));
    assert!(filtered.contains("▶ New active"));
    assert!(!filtered.contains("New finished"));
    assert!(!filtered.contains("New cancelled"));
    agent.todo.update_todos(vec![item("Only finished", TodoStatus::Completed)]);
    assert!(screen(&draw(&mut agent)).contains("All done."));
    agent.todo.update_todos(Vec::new());
    assert!(screen(&draw(&mut agent)).contains("No todo items."));
}

#[test]
fn f7_todo_theme_changes_restyle_once_without_materializing_collapsed_rows() {
    let _theme = crate::theme::cache::pin_theme();
    crate::theme::cache::set(crate::theme::ThemeKind::GrokNight);
    let root = tempfile::tempdir().unwrap();
    let mut agent = test_fixtures::make_agent();
    agent.open_panel_settings_in(root.path());
    agent.todo.overlay.visible = false;
    agent.todo.update_todos(vec![item("Theme active", TodoStatus::InProgress)]);
    let _ = press(&mut agent, KeyCode::F(7));
    agent.context_policy_answered(Ok(ready_report()));
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25BE} Todos");
    let _ = draw(&mut agent);
    let before = agent.todo.preparation_counts();
    crate::theme::cache::set(crate::theme::ThemeKind::GrokDay);
    let _ = draw(&mut agent);
    assert_eq!(agent.todo.preparation_counts(), (before.0 + 1, before.1));
    for _ in 0..8 {
        let _ = draw(&mut agent);
    }
    assert_eq!(agent.todo.preparation_counts(), (before.0 + 1, before.1));
    let buf = draw(&mut agent);
    let _ = click_row(&mut agent, &buf, "\u{25B8} Todos");
    let sidebar = draw(&mut agent);
    let theme = crate::theme::Theme::current();
    let style = text_style(&sidebar, "▶ Theme active");
    assert_eq!(style[0].0, theme.warning);
    assert!(style[2..].iter().all(|(fg, modifier)| {
        *fg == theme.text_primary && modifier.contains(ratatui::style::Modifier::BOLD)
    }));
    let _ = press(&mut agent, KeyCode::F(7));
    control(&mut agent, 't');
    assert_eq!(text_style(&draw(&mut agent), "▶ Theme active"), style);
    assert_eq!(agent.todo.preparation_counts().0, before.0 + 1);
}

#[test]
fn todo_preparation_refreshes_accepted_filter_after_same_length_data_update() {
    let _theme = crate::theme::cache::pin_theme();
    let root = tempfile::tempdir().unwrap();
    let mut agent = test_fixtures::make_agent();
    agent.open_panel_settings_in(root.path());
    agent.todo.overlay.visible = false;
    agent.todo.update_todos(vec![
        item("keep old", TodoStatus::Pending),
        item("skip old", TodoStatus::InProgress),
    ]);
    control(&mut agent, 't');
    let _ = draw(&mut agent);
    let _ = press(&mut agent, KeyCode::Char('f'));
    for ch in "keep".chars() {
        let _ = press(&mut agent, KeyCode::Char(ch));
    }
    let _ = press(&mut agent, KeyCode::Enter);
    let filtered = screen(&draw(&mut agent));
    assert!(filtered.contains("□ keep old"));
    assert!(!filtered.contains("skip old"));
    agent.todo.update_todos(vec![
        item("skip new", TodoStatus::Pending),
        item("keep new", TodoStatus::InProgress),
    ]);
    let updated = screen(&draw(&mut agent));
    assert!(updated.contains("▶ keep new"));
    assert!(!updated.contains("skip new"));
    assert!(!updated.contains("keep old"));
    let prepared = agent.todo.preparation_counts().0;
    for _ in 0..8 {
        let _ = draw(&mut agent);
    }
    assert_eq!(agent.todo.preparation_counts().0, prepared);
}
