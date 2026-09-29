#![cfg_attr(rustfmt, rustfmt::skip)]
    use super::*;
    use crate::app::actions::{Action, Effect};
    use crate::app::agent_view::PromptTarget;
    use crate::app::app_view::InputOutcome;
    use crate::app::dispatch::dispatch;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use xai_grok_shell::extensions::notification::SubagentDelivery;

    fn progress(delivery: SubagentDelivery) -> AcpClientMessage {
        let mut update = test_subagent_progress("sess-parent", "sess-child");
        let XaiSessionUpdate::SubagentProgress { attempt_id, delivery: reported, .. } = &mut update else {
            unreachable!("test_subagent_progress builds a progress update")
        };
        // The fixture child runs a legacy (id-less) attempt.
        *attempt_id = None;
        *reported = delivery;
        make_ext_session_notification("sess-parent", update)
    }

    fn press_deliver_key(app: &mut AppView) -> Vec<Effect> {
        match app.handle_input(&Event::Key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE))) {
            InputOutcome::Action(action) => dispatch(action, app),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_prompt_from_a_child_view_names_its_parent() {
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let effects = dispatch(Action::SendPrompt("look again".into()), &mut app);
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::SendPrompt { session_id, target: PromptTarget::ChildOf(parent), .. }
                    if session_id.0.as_ref() == "sess-child" && parent.0.as_ref() == "sess-parent"
            )),
            "{effects:?}"
        );
    }

    #[test]
    fn deliver_key_releases_only_a_held_child() {
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        assert!(press_deliver_key(&mut app).is_empty(), "nothing is held before a human prompt");

        let _ = handle(progress(SubagentDelivery::Held), &mut app);
        let effects = press_deliver_key(&mut app);
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::DeliverSubagent { child_session_id }] if child_session_id.0.as_ref() == "sess-child"
            ),
            "{effects:?}"
        );

        let _ = handle(progress(SubagentDelivery::OnTurnEnd), &mut app);
        assert!(press_deliver_key(&mut app).is_empty(), "a delivered child has nothing left to release");
    }
