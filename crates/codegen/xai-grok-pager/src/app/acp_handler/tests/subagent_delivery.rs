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

    fn observed_progress(
        delivery: SubagentDelivery,
        completion: xai_grok_shell::extensions::notification::SubagentCompletion,
        caller: xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCaller,
    ) -> AcpClientMessage {
        let mut update = test_subagent_progress("sess-parent", "sess-child");
        let XaiSessionUpdate::SubagentProgress {
            attempt_id, delivery: reported_delivery, completion: reported_completion,
            caller: reported_caller, ..
        } = &mut update else { unreachable!() };
        *attempt_id = None;
        *reported_delivery = delivery;
        *reported_completion = completion;
        *reported_caller = caller;
        make_ext_session_notification("sess-parent", update)
    }

    #[test]
    fn progress_observations_drive_dock_warning_and_do_not_change_on_navigation_or_prompt() {
        use xai_grok_shell::extensions::notification::SubagentCompletion::{Working, Settling, Pending};
        use xai_grok_tools::implementations::grok_build::task::coordinator::SubagentCaller::{NotWaiting, Waiting};
        use crate::app::actions::TreeStep;
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        for (completion, caller, expected) in [
            (Working, NotWaiting, "MANUAL — child working; open child, F5: auto"),
            (Working, Waiting, "MANUAL — caller waiting; child working; open child, F5: auto"),
            (Settling, Waiting, "MANUAL — caller waiting; follow-ups settling; open child, F5: auto"),
            (Settling, NotWaiting, "MANUAL — follow-ups settling; open child, F5: auto"),
            (Pending, NotWaiting, "MANUAL — answer ready, delivery blocked; open child, F5: auto"),
            (Pending, Waiting, "MANUAL — CALLER BLOCKED: answer ready; open child, F5: auto"),
        ] {
            let _ = handle(observed_progress(SubagentDelivery::Held, completion, caller), &mut app);
            let info = app.agents.child_info(AgentId(1)).unwrap();
            assert_eq!((info.attempt.completion, info.attempt.caller), (completion, caller));
            assert_eq!(info.delivery_warning(), Some(expected));
            let parent_id = app.agents.parent_of(AgentId(1)).unwrap();
            assert_eq!(app.agents.get(&parent_id).unwrap().dock_subagent_rows()[0].2.warning, Some(expected));
            for step in [TreeStep::Parent, TreeStep::LatestChild] {
                let effects = dispatch(Action::NavigateTree(step), &mut app);
                assert!(!effects.iter().any(|effect| matches!(effect, Effect::DeliverSubagent { .. })));
            }
            assert_eq!(app.agents.child_info(AgentId(1)).unwrap().delivery_warning(), Some(expected));
        }
        let effects = dispatch(Action::SendPrompt("keep going".into()), &mut app);
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::DeliverSubagent { .. })));
        assert_eq!(app.agents.child_info(AgentId(1)).unwrap().attempt.delivery, SubagentDelivery::Held);
        let _ = handle(observed_progress(SubagentDelivery::OnTurnEnd, Pending, Waiting), &mut app);
        assert!(app.agents.child_info(AgentId(1)).unwrap().delivery_warning().is_none());
    }

    #[test]
    fn hidden_child_steers_render_in_order_once_across_navigation_and_replayed_delivery() {
        use crate::app::actions::TreeStep;
        fn user_chunk(text: &str, event_id: &str, replay: bool) -> AcpClientMessage {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let request = acp::SessionNotification::new(
                acp::SessionId::new("sess-child"),
                acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                    acp::ContentBlock::Text(acp::TextContent::new(text)),
                )),
            ).meta(serde_json::json!({
                "promptId": "child-running-turn", "eventId": event_id,
                "agentTimestampMs": 1, "isReplay": replay,
            }).as_object().cloned());
            AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs { request, response_tx: tx })
        }
        fn user_lines(app: &AppView) -> Vec<String> {
            let child = app.agents.get(&AgentId(1)).unwrap();
            child.scrollback.entries_in_range(0..child.scrollback.len()).iter()
                .filter_map(|entry| match &entry.block {
                    RenderBlock::UserPrompt(block) => Some(block.text.clone()),
                    _ => None,
                }).collect()
        }
        for delivery in [SubagentDelivery::OnTurnEnd, SubagentDelivery::Held] {
            let mut app = make_app_viewing_child("sess-parent", "sess-child");
            let _ = handle(progress(delivery), &mut app);
            let _ = dispatch(Action::NavigateTree(TreeStep::Parent), &mut app);
            assert_eq!(app.active_view, ActiveView::Agent(AgentId(0)));
            let _ = handle(make_agent_chunk_meta("sess-child", "working", "child-running-turn", Some("sess-child-1"), false), &mut app);
            for (text, id) in [("steer first", "sess-child-2"), ("steer second", "sess-child-3")] {
                let _ = handle(user_chunk(text, id, false), &mut app);
            }
            assert_eq!(user_lines(&app), ["steer first", "steer second"]);
            let _ = dispatch(Action::NavigateTree(TreeStep::LatestChild), &mut app);
            assert_eq!(app.active_view, ActiveView::Agent(AgentId(1)));
            assert_eq!(user_lines(&app), ["steer first", "steer second"]);
            for replay in [false, true] {
                for (text, id) in [("steer first", "sess-child-2"), ("steer second", "sess-child-3")] {
                    let _ = handle(user_chunk(text, id, replay), &mut app);
                }
                let _ = dispatch(Action::NavigateTree(TreeStep::Parent), &mut app);
                let _ = dispatch(Action::NavigateTree(TreeStep::LatestChild), &mut app);
                assert_eq!(user_lines(&app), ["steer first", "steer second"]);
                assert_eq!(app.agents.child_info(AgentId(1)).unwrap().attempt.delivery, delivery);
            }
            let parent = app.agents.get(&AgentId(0)).unwrap();
            assert!(!parent.scrollback.entries_in_range(0..parent.scrollback.len()).iter().any(|entry|
                matches!(&entry.block, RenderBlock::UserPrompt(block) if block.text.starts_with("steer "))));
            with_replay_disk_home(|home| {
                let lines: Vec<String> = [("steer first", "sess-child-2"), ("steer second", "sess-child-3")]
                    .into_iter().map(|(text, id)| {
                        let AcpClientMessage::SessionNotification(args) = user_chunk(text, id, false) else { unreachable!() };
                        serde_json::json!({ "method": "session/update", "params": args.request }).to_string()
                    }).collect();
                write_child_updates_jsonl(home, "sess-child", &(lines.join("\n") + "\n"));
                let _ = handle(make_ext_session_notification("sess-parent", test_subagent_finished_for_attempt("sess-child", None)), &mut app);
                for _ in 0..2 {
                    leave_child_view(&mut app);
                    open_child_view(&mut app, "sess-child");
                    assert_eq!(user_lines(&app), ["steer first", "steer second"]);
                }
            });
        }
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
    fn prompting_a_finished_child_ends_in_its_continuation_view() {
        use crate::app::actions::TaskResult;
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let sent = dispatch(Action::SendPrompt("look again".into()), &mut app);
        assert!(
            sent.iter().any(|effect| matches!(
                effect,
                Effect::SendPrompt { target: PromptTarget::ChildOf(parent), .. } if parent.0.as_ref() == "sess-parent"
            )),
            "{sent:?}"
        );

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
                prompt_id: None,
            }),
            &mut app,
        );
        let mut spawned = test_subagent_spawned("sess-parent", "sess-continued");
        let XaiSessionUpdate::SubagentSpawned { resumed_from, .. } = &mut spawned else {
            unreachable!("test_subagent_spawned builds a spawn update")
        };
        *resumed_from = Some("sess-child".into());
        let _ = handle(
            make_ext_session_notification_with_method("sess-parent", "x.ai/session/update", spawned),
            &mut app,
        );

        let ActiveView::Agent(opened) = app.active_view else {
            panic!("an agent view is active");
        };
        assert_eq!(
            app.agents.get(&opened).and_then(|view| view.session.session_id.as_ref()).map(|sid| sid.0.as_ref()),
            Some("sess-continued")
        );
    }

    #[test]
    fn entering_and_leaving_a_child_preserves_automatic_and_manual_delivery() {
        use crate::app::actions::TreeStep;
        for delivery in [SubagentDelivery::OnTurnEnd, SubagentDelivery::Held] {
            let mut app = make_app_viewing_child("sess-parent", "sess-child");
            let _ = handle(progress(delivery), &mut app);
            for step in [TreeStep::Parent, TreeStep::LatestChild, TreeStep::Parent, TreeStep::LatestChild] {
                let effects = dispatch(Action::NavigateTree(step), &mut app);
                assert!(!effects.iter().any(|effect| matches!(effect, Effect::DeliverSubagent { .. })));
                assert_eq!(app.agents.child_info(AgentId(1)).unwrap().attempt.delivery, delivery);
            }
            assert_eq!(app.active_view, ActiveView::Agent(AgentId(1)));
        }
    }

    #[test]
    fn finished_child_cannot_change_delivery_mode() {
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let _ = handle(progress(SubagentDelivery::Held), &mut app);
        let (link, parent) = app.agents.link_and_parent_mut(AgentId(1)).unwrap();
        parent.subagent_sessions.get_mut(&link.subagent_id).unwrap().set_finished_for_test(true);
        assert!(press_deliver_key(&mut app).is_empty());
    }

    #[test]
    fn delivery_changes_are_explicit_and_failed_requests_can_retry() {
        use crate::app::actions::TaskResult;
        let mut app = make_app_viewing_child("sess-parent", "sess-child");
        let _ = handle(progress(SubagentDelivery::OnTurnEnd), &mut app);
        assert!(matches!(press_deliver_key(&mut app).as_slice(), [Effect::DeliverSubagent {
            child_session_id, delivery: SubagentDelivery::Held,
        }] if child_session_id.0.as_ref() == "sess-child"));
        let _ = dispatch(Action::TaskComplete(TaskResult::DeliverSubagentComplete { outcome: None }), &mut app);
        assert!(matches!(press_deliver_key(&mut app).as_slice(), [Effect::DeliverSubagent {
            delivery: SubagentDelivery::Held, ..
        }]));
        let _ = handle(progress(SubagentDelivery::Held), &mut app);
        assert!(matches!(press_deliver_key(&mut app).as_slice(), [Effect::DeliverSubagent {
            delivery: SubagentDelivery::OnTurnEnd, ..
        }]));
        let _ = dispatch(Action::TaskComplete(TaskResult::DeliverSubagentComplete { outcome: None }), &mut app);
        assert!(matches!(press_deliver_key(&mut app).as_slice(), [Effect::DeliverSubagent {
            delivery: SubagentDelivery::OnTurnEnd, ..
        }]));
        let _ = handle(progress(SubagentDelivery::OnTurnEnd), &mut app);
        assert!(matches!(press_deliver_key(&mut app).as_slice(), [Effect::DeliverSubagent {
            delivery: SubagentDelivery::Held, ..
        }]));
    }
