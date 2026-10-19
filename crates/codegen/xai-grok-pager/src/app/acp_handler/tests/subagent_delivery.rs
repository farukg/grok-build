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
    fn a_legacy_shell_refuses_prompts_from_a_child_view() {
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        app.shell_child_support = crate::acp::ShellChildSupport::Legacy;
        let effects = dispatch(Action::SendPrompt("look again".into()), &mut app);
        assert!(effects.is_empty(), "{effects:?}");
    }

    #[test]
    fn a_prompt_answered_with_a_resume_marks_the_continuation_to_follow() {
        use crate::app::actions::TaskResult;
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let answer = acp::PromptResponse::new(acp::StopReason::EndTurn).meta(
            serde_json::json!({ "childResume": { "kind": "resumed", "sourceId": "sess-child" } })
                .as_object()
                .cloned(),
        );
        let _ = dispatch(
            Action::TaskComplete(TaskResult::PromptResponse {
                agent_id: AgentId(1),
                result: Ok(answer),
                http_status: None,
                prompt_id: Some("p-1".into()),
            }),
            &mut app,
        );
        assert_eq!(app.follow_resumed_child.as_deref(), Some("sess-child"));
    }

    #[test]
    fn a_resumed_child_opens_when_the_user_resumed_it_from_its_finished_view() {
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let resumed = |source: &str, continued: &str| {
            let mut spawned = test_subagent_spawned("sess-parent", continued);
            let XaiSessionUpdate::SubagentSpawned { resumed_from, .. } = &mut spawned else {
                unreachable!("test_subagent_spawned builds a spawn update")
            };
            *resumed_from = Some(source.into());
            make_ext_session_notification_with_method("sess-parent", "x.ai/session/update", spawned)
        };

        let _ = handle(resumed("sess-child", "sess-continued-1"), &mut app);
        assert_eq!(
            app.active_view,
            ActiveView::Agent(AgentId(1)),
            "a continuation nobody asked for stays in the background"
        );

        app.follow_resumed_child = Some("sess-other".into());
        let _ = handle(resumed("sess-child", "sess-continued-2"), &mut app);
        assert_eq!(app.active_view, ActiveView::Agent(AgentId(1)));

        app.follow_resumed_child = Some("sess-child".into());
        let _ = handle(resumed("sess-child", "sess-continued-3"), &mut app);
        let ActiveView::Agent(opened) = app.active_view else {
            panic!("an agent view is active");
        };
        assert_eq!(
            app.agents.get(&opened).and_then(|view| view.session.session_id.as_ref()).map(|sid| sid.0.as_ref()),
            Some("sess-continued-3")
        );
        assert_eq!(app.follow_resumed_child, None);
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
