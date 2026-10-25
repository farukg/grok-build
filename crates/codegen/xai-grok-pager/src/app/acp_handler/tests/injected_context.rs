use super::*;
use crate::scrollback::block::{BlockContent, MessageKind, RenderBlock};
use crate::scrollback::types::{BlockContext, DisplayForm, DisplayMode};

#[test]
fn injected_notification_replay_is_lossless_foldable_and_obeys_kind_defaults() {
    let _theme = crate::theme::cache::pin_theme();
    let mut app = make_app_with_agent("injected-session");
    let text = format!("{}\nVISIBLE_REQUEST_TAIL", "ä".repeat(5000));
    let payload = xai_grok_shell::extensions::notification::SessionNotification {
        session_id: acp::SessionId::new("injected-session"),
        update: XaiSessionUpdate::InjectedContextRecord { kind: "memory_reminder".into(), text: text.clone() },
        meta: Some(serde_json::json!({ "eventId": "injected-session-1", "isReplay": false })),
    };
    let live = acp::ExtNotification::new("x.ai/session/update", serde_json::value::to_raw_value(&payload).unwrap().into());
    handle_ext_notification(&live, &mut app);
    let before_replay = test_agent(&app, AgentId(0)).scrollback.len();
    let mut replay_payload = payload.clone();
    replay_payload.meta = Some(serde_json::json!({ "eventId": "injected-session-1", "isReplay": true }));
    let replay = acp::ExtNotification::new("x.ai/session/update", serde_json::value::to_raw_value(&replay_payload).unwrap().into());
    handle_ext_notification(&replay, &mut app);
    assert_eq!(test_agent(&app, AgentId(0)).scrollback.len(), before_replay);
    let agent = app.agents.get_mut(&AgentId(0)).unwrap();
    let index = agent.scrollback.len() - 1;
    let entry = agent.scrollback.get_mut(index).unwrap();
    assert_eq!(entry.display_mode, DisplayMode::Collapsed);
    assert!(matches!(&entry.block, RenderBlock::Injected(block) if block.text == text));
    entry.toggle_fold();
    assert_eq!(entry.display_mode, DisplayMode::Expanded);
    let output = entry.block.output(&BlockContext {
        mode: entry.display_mode, width: 80, is_running: false, raw: false,
        max_lines: None, appearance: Default::default(), is_selected: false, cwd: None,
    });
    assert!(output.lines.iter().any(|line| line.content.to_string().contains("VISIBLE_REQUEST_TAIL")));
    agent.scrollback.set_kind_default(MessageKind::Injected, DisplayForm::Expanded);
    let mut next_payload = payload;
    next_payload.meta = Some(serde_json::json!({ "eventId": "injected-session-2" }));
    let next = acp::ExtNotification::new("x.ai/session/update", serde_json::value::to_raw_value(&next_payload).unwrap().into());
    handle_ext_notification(&next, &mut app);
    let agent = test_agent(&app, AgentId(0));
    assert_eq!(agent.scrollback.get(agent.scrollback.len() - 1).unwrap().display_mode, DisplayMode::Expanded);
}
