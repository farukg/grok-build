use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::actions::ActionRegistry;
use crate::app::agent_view::{AgentView, test_fixtures};
use crate::app::app_view::InputOutcome;
use crate::clipboard::ClipboardProvider;

#[derive(Debug)]
struct RecordingClipboard(Rc<RefCell<Vec<String>>>);

impl ClipboardProvider for RecordingClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.borrow().last().cloned()
    }

    fn set(&mut self, text: &str) {
        self.0.borrow_mut().push(text.to_owned());
    }
}

fn press(agent: &mut AgentView, code: KeyCode) -> InputOutcome {
    agent.handle_input(
        &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        &ActionRegistry::defaults(),
    )
}

fn render(agent: &mut AgentView) {
    let area = Rect::new(0, 0, 140, 30);
    let mut buffer = Buffer::empty(area);
    agent.request_viewer.as_mut().unwrap().render(area, &mut buffer);
}

#[test]
fn line_and_visual_y_use_the_list_clipboard_without_touching_the_editor() {
    let _theme = crate::theme::cache::pin_theme();
    let mut agent = test_fixtures::make_agent();
    agent.session.session_id = Some(agent_client_protocol::SessionId::new(
        "11111111-1111-1111-1111-111111111111",
    ));
    agent.prompt.set_text("editor stays independent");
    let (session, _, generation) = agent.open_request_viewer().unwrap();
    agent.request_snapshot_loaded(&session, generation, Ok(None));
    let copies = Rc::new(RefCell::new(Vec::new()));
    agent.request_viewer.as_mut().unwrap().list_state.set_clipboard_provider(
        Box::new(RecordingClipboard(Rc::clone(&copies))),
    );
    render(&mut agent);
    assert!(matches!(press(&mut agent, KeyCode::Char('y')), InputOutcome::Changed));
    assert_eq!(copies.borrow().len(), 1);
    assert_eq!(copies.borrow().last().map(String::as_str),
        Some("The latest model request captured for this session, as the shell recorded it."));
    assert!(matches!(press(&mut agent, KeyCode::Char('v')), InputOutcome::Changed));
    assert!(agent.request_viewer.as_ref().unwrap().list_state.visual_mode);
    press(&mut agent, KeyCode::Down);
    press(&mut agent, KeyCode::Down);
    render(&mut agent);
    assert!(matches!(press(&mut agent, KeyCode::Char('y')), InputOutcome::Changed));
    assert_eq!(copies.borrow().last().unwrap(),
        "The latest model request captured for this session, as the shell recorded it.\n\nNo captured request for this session yet.");
    assert!(!agent.request_viewer.as_ref().unwrap().list_state.visual_mode);
    press(&mut agent, KeyCode::Char('v'));
    press(&mut agent, KeyCode::Esc);
    assert!(agent.request_viewer.is_some(), "Esc exits visual selection first");
    assert!(!agent.request_viewer.as_ref().unwrap().list_state.visual_mode);
    assert_eq!(agent.prompt.text(), "editor stays independent");
    press(&mut agent, KeyCode::Esc);
    assert!(agent.request_viewer.is_none());
    assert_eq!(agent.prompt.text(), "editor stays independent");
}
