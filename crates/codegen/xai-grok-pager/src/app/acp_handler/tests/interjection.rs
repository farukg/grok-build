#![cfg_attr(rustfmt, rustfmt::skip)]
    use super::*;

    #[test]
    fn interjection_broadcast_mid_park_adds_no_marker() {
        use crate::app::agent_view::test_fixtures::{count_turn_markers, simulate_task_output_wait};

        let mut app = make_app_with_agent("sess-park");
        {
            let agent = app.agents.get_mut(&AgentId(0)).unwrap();
            agent.session.state = AgentState::TurnRunning;
            agent.session.current_prompt_id = Some("p1".into());
            insert_running_task(agent, "t10", "sleep 10");
            insert_running_task(agent, "t15", "sleep 15");
            simulate_task_output_wait(agent, "t15");
            assert!(agent.is_parked_on_sendable_wait());
            assert_eq!(count_turn_markers(agent), 0, "the park writes no row");
        }

        assert!(handle_ext_notification(
            &interjection_broadcast("sess-park", "queued follow-up"),
            &mut app,
        ));

        {
            let agent = app.agents.get_mut(&AgentId(0)).unwrap();
            assert_eq!(
                last_interjection_text(&agent.scrollback).as_deref(),
                Some("queued follow-up"),
            );
            assert_eq!(
                count_turn_markers(agent),
                0,
                "no 'Worked for …' marker around the interjection"
            );
        }

        handle_ext_notification(
            &make_task_completed_notif("sess-park", "t10", "sleep 10", Some(0)),
            &mut app,
        );
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        assert_eq!(
            count_turn_markers(agent),
            0,
            "no 'Worked for …' tick under the interjection"
        );
        assert!(agent.renders_parked(), "the parked chrome stays on");
    }

    #[test]
    fn parked_completions_push_chips_without_markers() {
        use crate::app::agent_view::test_fixtures::{count_turn_markers, simulate_task_output_wait};

        let mut app = make_app_with_agent("sess-park");
        {
            let agent = app.agents.get_mut(&AgentId(0)).unwrap();
            agent.session.state = AgentState::TurnRunning;
            agent.session.current_prompt_id = Some("p1".into());
            insert_running_task(agent, "t10", "sleep 10");
            insert_running_task(agent, "t15", "sleep 15");
            insert_running_task(agent, "t20", "sleep 20");
            simulate_task_output_wait(agent, "t20");
            assert!(agent.renders_parked());
        }

        handle_ext_notification(
            &make_task_completed_notif("sess-park", "t10", "sleep 10", Some(0)),
            &mut app,
        );
        // Duplicate completion for the same task: not a Running-to-Done edge
        handle_ext_notification(
            &make_task_completed_notif("sess-park", "t10", "sleep 10", Some(0)),
            &mut app,
        );
        handle_ext_notification(
            &make_task_completed_notif("sess-park", "t15", "sleep 15", Some(0)),
            &mut app,
        );
        handle_ext_notification(
            &make_task_completed_notif("sess-park", "t20", "sleep 20", Some(0)),
            &mut app,
        );

        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        assert_eq!(
            count_turn_markers(agent),
            0,
            "completions during a park never write a marker"
        );
        assert!(
            work_status_lines(&agent.scrollback).is_empty(),
            "no work-only status lines in the transcript"
        );
    }

    #[test]
    fn consecutive_subagent_finishes_stay_markerless() {
        use crate::app::agent_view::test_fixtures::count_turn_markers;

        let mut app = make_app_with_agent("sess-park");
        {
            let agent = app.agents.get_mut(&AgentId(0)).unwrap();
            park_on_subagents(agent, &["child-1", "child-2", "child-3"]);
        }

        for child in ["child-1", "child-1", "child-2", "child-3"] {
            handle(
                make_ext_session_notification("sess-park", test_subagent_finished(child)),
                &mut app,
            );
        }
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        assert_eq!(
            count_turn_markers(agent),
            0,
            "subagent finishes never write a marker mid-park"
        );
    }

    #[test]
    fn repark_after_parent_output_stays_markerless() {
        use crate::acp::meta::NotificationMeta;
        use crate::app::agent_view::test_fixtures::{
            complete_task_output_wait_call, count_turn_markers, simulate_task_output_wait_call,
        };

        let mut app = make_app_with_agent("sess-park");
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.session.state = AgentState::TurnRunning;
        agent.session.current_prompt_id = Some("p1".into());
        insert_running_task(agent, "t10", "sleep 10");

        simulate_task_output_wait_call(agent, "wait-1", "t10", 30_000);
        assert!(agent.renders_parked());
        assert_eq!(count_turn_markers(agent), 0);

        complete_task_output_wait_call(agent, "wait-1");
        assert!(agent.session.tracker.handle_update(
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                acp::ContentBlock::Text(acp::TextContent::new("between-parks content")),
            )),
            &NotificationMeta::default(),
            &mut agent.scrollback,
        ));

        simulate_task_output_wait_call(agent, "wait-2", "t10", 30_000);
        assert!(agent.renders_parked(), "the re-park renders parked again");
        assert_eq!(count_turn_markers(agent), 0, "and still writes no marker");
    }

    #[test]
    fn interjection_notification_pushes_block_to_matching_session() {
        // Multi-client: an interjection typed in one pane is broadcast by the shell as x.ai/session/interjection
        // EVERY attached pane (incl. the originator, which no longer pushes a local block) renders it.
        let mut app = make_app_with_agent("sess-view");
        let affected =
            handle_ext_notification(&interjection_ext("sess-view", "also add tests"), &mut app);
        assert!(affected, "rendering into the active agent should redraw");

        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(
            last_interjection_text(&agent.scrollback).as_deref(),
            Some("also add tests"),
            "the interjection block must be pushed from the broadcast"
        );
    }

    #[test]
    fn interjection_notification_for_unknown_session_is_ignored() {
        let mut app = make_app_with_agent("sess-view");
        let affected = handle_ext_notification(&interjection_ext("sess-other", "stray"), &mut app);
        assert!(!affected, "an unmatched session must be a no-op");

        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(
            last_interjection_text(&agent.scrollback).is_none(),
            "no interjection block must be pushed for an unknown session"
        );
    }

    #[test]
    fn interjection_notification_renders_for_a_viewer() {
        // A viewer (attached_as_viewer) watching another client's session must also render interjections broadcast for that session
        let mut app = make_app_with_agent("sess-view");
        app.agents.get_mut(&AgentId(0)).unwrap().attached_as_viewer = true;
        let affected =
            handle_ext_notification(&interjection_ext("sess-view", "viewer sees this"), &mut app);
        assert!(affected);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(
            last_interjection_text(&agent.scrollback).as_deref(),
            Some("viewer sees this"),
            "a viewer must render interjections broadcast for its session"
        );
    }

    #[test]
    fn interjection_notification_dedups_originators_own_echo() {
        // The originator rendered an optimistic block in dispatch_interject and recorded the id
        // Its own broadcast echo must be dropped (no dup) and the id forgotten
        let mut app = make_app_with_agent("sess-view");
        app.agents
            .get_mut(&AgentId(0))
            .unwrap()
            .self_interjection_ids
            .insert("ij-1".to_string());

        let affected = handle_ext_notification(
            &interjection_ext_with_id("sess-view", "my own", Some("ij-1")),
            &mut app,
        );
        assert!(
            !affected,
            "an originator's own echo must be a no-op (already rendered locally)"
        );
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(
            last_interjection_text(&agent.scrollback).is_none(),
            "the echo must not push a duplicate block"
        );
        assert!(
            !agent.self_interjection_ids.contains("ij-1"),
            "the id must be forgotten after dedup"
        );
    }

    #[test]
    fn goal_send_now_notification_claims_optimistic_prompt_block() {
        let mut app = make_app_with_agent("sess-view");
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.note_self_originated_prompt("prompt-1");
        let entry_id = agent
            .scrollback
            .push_block(RenderBlock::user_prompt("steer the goal".to_string()));
        agent
            .send_now_painted_blocks
            .insert("prompt-1".to_string(), (entry_id, false));

        let affected = handle_ext_notification(
            &interjection_ext_with_id("sess-view", "steer the goal", Some("prompt-1")),
            &mut app,
        );
        assert!(!affected, "the optimistic block already represents the message");
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert!(!agent.send_now_painted_blocks.contains_key("prompt-1"));
        assert!(agent.expect_send_now_cancel.is_none());
        assert_eq!(last_interjection_text(&agent.scrollback).as_deref(), Some("steer the goal"));
    }

    #[test]
    fn interjection_does_not_rewrite_earlier_same_text_bubble() {
        // Self-originated id plus matching text must not restyle an earlier bubble.
        let mut app = make_app_with_agent("sess-view");
        let agent = app.agents.get_mut(&AgentId(0)).unwrap();
        agent.note_self_originated_prompt("queue-pid");
        agent
            .scrollback
            .push_block(RenderBlock::user_prompt("try again".to_string()));

        let affected = handle_ext_notification(
            &interjection_ext_with_id("sess-view", "try again", Some("queue-pid")),
            &mut app,
        );
        assert!(affected, "must render a new interjection, not claim the old bubble");
        let agent = app.agents.get(&AgentId(0)).unwrap();
        let prompts: Vec<_> = (0..agent.scrollback.len())
            .filter_map(|i| match agent.scrollback.entry(i).map(|e| &e.block) {
                Some(RenderBlock::UserPrompt(b)) => Some((b.text.clone(), b.is_interjection)),
                _ => None,
            })
            .collect();
        assert_eq!(
            prompts,
            vec![("try again".into(), false), ("try again".into(), true)]
        );
    }

    #[test]
    fn interjection_notification_with_foreign_id_renders() {
        // A broadcast carrying an id this client did NOT mint (another pane's interjection) must render; only the originator dedups by its own id
        let mut app = make_app_with_agent("sess-view");
        let affected = handle_ext_notification(
            &interjection_ext_with_id("sess-view", "from another pane", Some("other-id")),
            &mut app,
        );
        assert!(affected);
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(
            last_interjection_text(&agent.scrollback).as_deref(),
            Some("from another pane"),
            "an interjection from another pane must render"
        );
    }

    #[derive(Clone, Copy)]
    enum InterjectionProjection {
        Echo,
        Injected,
        ReplayChunk,
    }

    fn deliver_identity_projection(
        app: &mut AppView,
        sid: &str,
        id: &str,
        text: &str,
        projection: InterjectionProjection,
        replay: bool,
    ) {
        deliver_identity_projection_with_meta(app, sid, id, text, projection, serde_json::json!({"isReplay": replay}));
    }

    fn deliver_identity_projection_with_meta(
        app: &mut AppView,
        sid: &str,
        id: &str,
        text: &str,
        projection: InterjectionProjection,
        mut envelope: serde_json::Value,
    ) {
        let identity = serde_json::json!({"id": id, "display_text": text});
        match projection {
            InterjectionProjection::Echo => {
                handle_ext_notification(&acp_fixtures::ext_notification("x.ai/session/interjection", &serde_json::json!({
                    "sessionId": sid, "text": text, "interjectionId": id, "meta": envelope,
                })), app);
            }
            InterjectionProjection::Injected => {
                let (response_tx, _rx) = tokio::sync::oneshot::channel();
                let notification = SessionNotification {
                    session_id: acp::SessionId::new(sid),
                    update: XaiSessionUpdate::InjectedContextRecord {
                        kind: "interjection".into(),
                        text: format!("The user sent a message while you were working:\n<user_query>\n{text}\n</user_query>"),
                    },
                    meta: Some({
                        envelope["interjectionIdentity"] = identity;
                        envelope
                    }),
                };
                handle(AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                    request: acp_fixtures::ext_notification("x.ai/session_notification", &notification),
                    response_tx,
                }), app);
            }
            InterjectionProjection::ReplayChunk => {
                let (response_tx, _rx) = tokio::sync::oneshot::channel();
                let meta = serde_json::json!({"interjection": true, "interjectionIdentity": identity});
                handle(AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
                    request: acp::SessionNotification::new(acp::SessionId::new(sid),
                        acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                            acp::ContentBlock::Text(acp::TextContent::new("framed model text"))
                        ).meta(meta.as_object().cloned())))
                        .meta(envelope.as_object().cloned()),
                    response_tx,
                }), app);
            }
        }
    }

    fn interjection_rows(app: &AppView) -> Vec<(EntryId, String)> {
        let agent = app.agents.get(&AgentId(0)).unwrap();
        (0..agent.scrollback.len()).filter_map(|index| {
            let entry = agent.scrollback.get(index)?;
            match &entry.block {
                RenderBlock::UserPrompt(block) if block.is_interjection => Some((entry.id, block.text.clone())),
                RenderBlock::Injected(block) => panic!("identified interjection produced an extra injected row: {block:?}"),
                _ => None,
            }
        }).collect()
    }

    #[test]
    fn submitted_interjection_keeps_entry_through_all_projection_orders_and_retries() {
        use crate::app::actions::{Action, Effect};
        use crate::app::dispatch::router::dispatch;
        use InterjectionProjection::{Echo, Injected, ReplayChunk};
        for order in [
            [Echo, Injected, ReplayChunk], [Echo, ReplayChunk, Injected],
            [Injected, Echo, ReplayChunk], [Injected, ReplayChunk, Echo],
            [ReplayChunk, Echo, Injected], [ReplayChunk, Injected, Echo],
        ] {
            let mut app = make_app_with_agent("identity-session");
            let text = "once [Image #1]";
            let mut image = crate::prompt_images::from_clipboard_data(&crate::clipboard::ImageData {
                data: vec![1, 2, 3], mime_type: "image/png".into(),
            });
            image.display_number = 1;
            let effects = dispatch(Action::Interject { text: text.into(), images: vec![image] }, &mut app);
            let [Effect::SendInterject { interjection_id, blocks: Some(blocks), .. }] = effects.as_slice() else {
                panic!("real input must dispatch an image-bearing interjection");
            };
            assert!(blocks.iter().any(|block| matches!(block, acp::ContentBlock::Image(_))));
            let before = interjection_rows(&app);
            assert_eq!(before.len(), 1);
            for projection in order.into_iter().chain(order) {
                deliver_identity_projection(&mut app, "identity-session", interjection_id, text, projection, false);
                assert_eq!(interjection_rows(&app), before, "the original EntryId and content must survive");
            }
            deliver_identity_projection(&mut app, "other-session", interjection_id, "foreign", Echo, false);
            assert_eq!(interjection_rows(&app), before);
        }
    }

    #[test]
    fn second_client_and_restarted_client_join_equal_text_by_id_not_text() {
        use InterjectionProjection::{Echo, Injected, ReplayChunk};
        for replay in [false, true] {
            for order in [
                [Echo, Injected, ReplayChunk], [Echo, ReplayChunk, Injected],
                [Injected, Echo, ReplayChunk], [Injected, ReplayChunk, Echo],
                [ReplayChunk, Echo, Injected], [ReplayChunk, Injected, Echo],
            ] {
                let mut app = make_app_with_agent("identity-session");
                let agent = app.agents.get_mut(&AgentId(0)).unwrap();
                agent.attached_as_viewer = true;
                agent.session.loading_replay = replay;
                for (index, id) in ["first", "second"].into_iter().enumerate() {
                    for projection in order {
                        deliver_identity_projection(&mut app, "identity-session", id, "intentionally equal", projection, replay);
                        assert_eq!(interjection_rows(&app).len(), index + 1);
                    }
                    for projection in order {
                        deliver_identity_projection(&mut app, "identity-session", id, "intentionally equal", projection, replay);
                    }
                }
                let rows = interjection_rows(&app);
                assert_eq!(rows.len(), 2);
                assert_ne!(rows[0].0, rows[1].0);
                assert_eq!(rows[0].1, "intentionally equal");
                assert_eq!(rows[1].1, "intentionally equal");
            }
        }
    }

    #[test]
    fn accepted_source_survives_rpc_failure_before_or_after_echo() {
        use crate::app::actions::{Action, Effect, TaskResult};
        use crate::app::dispatch::dispatch;
        for echo_first in [false, true] {
            let mut app = make_app_with_agent("identity-session");
            let text = "accepted despite lost response";
            let effects = dispatch(Action::Interject { text: text.into(), images: vec![] }, &mut app);
            let [Effect::SendInterject { interjection_id, binding, .. }] = effects.as_slice() else {
                panic!("expected interjection ingress");
            };
            let original = interjection_rows(&app);
            if echo_first {
                deliver_identity_projection(&mut app, "identity-session", interjection_id, text, InterjectionProjection::Echo, false);
            }
            deliver_identity_projection(&mut app, "identity-session", interjection_id, text, InterjectionProjection::Injected, false);
            dispatch(Action::TaskComplete(TaskResult::InterjectFailed {
                agent_id: AgentId(0), binding: binding.clone(), error: "response lost".into(),
                remaining: vec![(text.into(), interjection_id.clone(), None)],
            }), &mut app);
            assert_eq!(interjection_rows(&app), original);
            assert!(app.agents.get(&AgentId(0)).unwrap().session.pending_prompts.is_empty());
            deliver_identity_projection(&mut app, "identity-session", interjection_id, text, InterjectionProjection::Echo, false);
            assert_eq!(interjection_rows(&app), original);
        }
    }

    #[test]
    fn rejected_ingress_removes_optimistic_entry_and_preserves_retry() {
        use crate::app::actions::{Action, Effect, TaskResult};
        use crate::app::dispatch::dispatch;
        let mut app = make_app_with_agent("identity-session");
        let effects = dispatch(Action::Interject { text: "rejected".into(), images: vec![] }, &mut app);
        let [Effect::SendInterject { interjection_id, binding, .. }] = effects.as_slice() else {
            panic!("expected interjection ingress");
        };
        dispatch(Action::TaskComplete(TaskResult::InterjectFailed {
            agent_id: AgentId(0), binding: binding.clone(), error: "ingress rejected".into(),
            remaining: vec![("rejected".into(), interjection_id.clone(), None)],
        }), &mut app);
        assert!(interjection_rows(&app).is_empty());
        let agent = app.agents.get(&AgentId(0)).unwrap();
        assert_eq!(agent.session.pending_prompts.len(), 1);
        assert_eq!(agent.session.pending_prompts.front().unwrap().text, "rejected");
    }

    #[test]
    fn stale_or_unversioned_echo_cannot_claim_bound_interjection() {
        use crate::app::actions::{Action, Effect};
        use crate::app::dispatch::dispatch;
        use crate::app::agent_view::current_actions::CurrentActionsContext;
        let mut app = make_app_with_agent("identity-session");
        let view = app.agents.get_mut(&AgentId(0)).unwrap();
        let current = CurrentActionsContext {
            binding: view.current_actions_binding().unwrap(),
            incarnation: uuid::Uuid::new_v4(),
        };
        assert!(view.bind_current_actions(current.clone()));
        let effects = dispatch(Action::Interject { text: "new binding".into(), images: vec![] }, &mut app);
        let [Effect::SendInterject { interjection_id, .. }] = effects.as_slice() else {
            panic!("expected interjection ingress");
        };
        let original = interjection_rows(&app);
        for meta in [serde_json::json!({}), serde_json::json!({
            "currentActionsVersion": {"incarnation": uuid::Uuid::new_v4(), "sequence": 1}
        })] {
            let echo = acp_fixtures::ext_notification("x.ai/session/interjection", &serde_json::json!({
                "sessionId": "identity-session", "text": "old binding", "interjectionId": interjection_id,
                "meta": meta,
            }));
            assert!(!handle_ext_notification(&echo, &mut app));
            assert_eq!(interjection_rows(&app), original);
            assert!(app.agents.get(&AgentId(0)).unwrap().self_interjection_ids.contains(interjection_id));
        }
    }

    #[test]
    fn identityless_legacy_interjections_are_not_guessed_from_equal_text() {
        let mut app = make_app_with_agent("legacy-session");
        for _ in 0..2 {
            handle_ext_notification(&interjection_broadcast("legacy-session", "equal legacy text"), &mut app);
        }
        assert_eq!(interjection_rows(&app).len(), 2);
    }

    #[test]
    fn rpc_error_and_original_source_retire_only_the_matching_retry_in_both_orders() {
        use crate::app::actions::{Action, Effect, TaskResult};
        use crate::app::dispatch::dispatch;
        for projection in [InterjectionProjection::Injected, InterjectionProjection::ReplayChunk] {
            for source_first in [false, true] {
                let mut app = make_app_with_agent("identity-session");
                let text = "same text";
                let first = dispatch(Action::Interject { text: text.into(), images: vec![] }, &mut app);
                let second = dispatch(Action::Interject { text: text.into(), images: vec![] }, &mut app);
                let [Effect::SendInterject { interjection_id: first_id, binding, .. }] = first.as_slice() else { panic!("first dispatch"); };
                let [Effect::SendInterject { interjection_id: second_id, .. }] = second.as_slice() else { panic!("second dispatch"); };
                assert_ne!(first_id, second_id);
                app.agents.get_mut(&AgentId(0)).unwrap().session.enqueue_prompt("unrelated tail".into());
                let failure = |id: &String| TaskResult::InterjectFailed {
                    agent_id: AgentId(0), binding: binding.clone(), error: "lost response".into(),
                    remaining: vec![(text.into(), id.clone(), None)],
                };
                dispatch(Action::TaskComplete(failure(second_id)), &mut app);
                let unrelated_retry = app.agents.get(&AgentId(0)).unwrap().session.pending_prompts.front().unwrap().id;
                if source_first {
                    deliver_identity_projection(&mut app, "identity-session", first_id, text, projection, false);
                }
                dispatch(Action::TaskComplete(failure(first_id)), &mut app);
                dispatch(Action::TaskComplete(failure(first_id)), &mut app);
                if !source_first {
                    let pending = &app.agents.get(&AgentId(0)).unwrap().session.pending_prompts;
                    assert_eq!(pending.len(), 3);
                    assert_eq!(pending.front().unwrap().interjection.as_ref().unwrap().id, *first_id);
                    deliver_identity_projection(&mut app, "identity-session", first_id, text, projection, false);
                }
                let accepted = interjection_rows(&app);
                assert_eq!(accepted.len(), 1);
                assert_eq!(accepted[0].1, text);
                for repeated in [projection, InterjectionProjection::Echo, projection] {
                    deliver_identity_projection(&mut app, "identity-session", first_id, text, repeated, false);
                }
                dispatch(Action::TaskComplete(failure(first_id)), &mut app);
                assert_eq!(interjection_rows(&app), accepted);
                let pending = &app.agents.get(&AgentId(0)).unwrap().session.pending_prompts;
                assert_eq!(pending.len(), 2);
                assert_eq!(pending[0].id, unrelated_retry);
                assert_eq!(pending[0].interjection.as_ref().unwrap().id, *second_id);
                assert_eq!(pending[1].text, "unrelated tail");
            }
        }
    }

    #[test]
    fn image_retry_queue_promotion_keeps_original_wire_identity() {
        use crate::app::actions::{Action, Effect, TaskResult};
        use crate::app::dispatch::dispatch;
        let mut app = make_app_with_agent("identity-session");
        let image = crate::prompt_images::from_clipboard_data(&crate::clipboard::ImageData {
            data: vec![1, 2, 3], mime_type: "image/png".into(),
        });
        let text = "original image input";
        let effects = dispatch(Action::Interject { text: text.into(), images: vec![image] }, &mut app);
        let [Effect::SendInterject { interjection_id, binding, blocks: Some(original), .. }] = effects.as_slice() else { panic!("image dispatch"); };
        deliver_identity_projection(&mut app, "identity-session", interjection_id, text, InterjectionProjection::Echo, false);
        dispatch(Action::TaskComplete(TaskResult::InterjectFailed {
            agent_id: AgentId(0), binding: binding.clone(), error: "lost ACK".into(),
            remaining: vec![(text.into(), interjection_id.clone(), Some(original.clone()))],
        }), &mut app);
        let queued = app.agents.get(&AgentId(0)).unwrap().session.pending_prompts.front().unwrap();
        assert_eq!(queued.interjection.as_ref().unwrap().id, *interjection_id);
        assert_eq!(queued.interjection.as_ref().unwrap().display_text, text);
        assert_eq!(queued.images.len(), 1);
        let promoted = dispatch(Action::DrainQueue, &mut app);
        let [Effect::SendInterject { interjection_id: retry_id, blocks: Some(blocks), .. }] = promoted.as_slice() else { panic!("retry must reuse interject ingress, including idle fallback"); };
        assert_eq!(retry_id, interjection_id);
        assert!(blocks.iter().any(|block| matches!(block, acp::ContentBlock::Image(_))));
        let before = interjection_rows(&app);
        assert_eq!(before.len(), 1);
        deliver_identity_projection(&mut app, "identity-session", interjection_id, text, InterjectionProjection::ReplayChunk, false);
        assert_eq!(interjection_rows(&app), before);
        assert!(app.agents.get(&AgentId(0)).unwrap().session.pending_prompts.is_empty());
    }

    #[test]
    fn reload_binding_rejects_late_old_effect_echo_ack_and_sources_in_same_incarnation() {
        use crate::app::actions::{Action, Effect, TaskResult};
        use crate::app::dispatch::dispatch;
        use crate::app::agent_view::current_actions::CurrentActionsContext;
        use xai_grok_shell::extensions::current_actions::{DeliveryRole, PublicationContext, DELIVERY_META_KEY};
        let mut app = make_app_with_agent("identity-session");
        let view = app.agents.get_mut(&AgentId(0)).unwrap();
        let incarnation = uuid::Uuid::new_v4();
        let original_binding = view.current_actions_binding().unwrap();
        assert!(view.bind_current_actions(CurrentActionsContext { binding: original_binding.clone(), incarnation }));
        let effects = dispatch(Action::Interject { text: "original".into(), images: vec![] }, &mut app);
        let [Effect::SendInterject { interjection_id, binding, .. }] = effects.as_slice() else { panic!("original dispatch"); };
        let failure = || TaskResult::InterjectFailed {
            agent_id: AgentId(0), binding: binding.clone(), error: "late old failure".into(),
            remaining: vec![("original".into(), interjection_id.clone(), None)],
        };
        dispatch(Action::TaskComplete(failure()), &mut app);
        app.agents.get_mut(&AgentId(0)).unwrap().begin_replay_window();
        let loaded = dispatch(Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: AgentId(0), session_id: acp::SessionId::new("identity-session"),
            models: None, modes: None, code_restored: false, restore_summary: None,
            restore_degree: None, running_prompt_id: None,
        }), &mut app);
        let view = app.agents.get(&AgentId(0)).unwrap();
        let new_binding = view.current_actions_binding().unwrap();
        assert_ne!(new_binding, original_binding);
        assert_eq!(view.current_actions_context().unwrap().incarnation, incarnation);
        assert!(loaded.iter().any(|effect| matches!(effect,
            Effect::SendInterject { interjection_id: id, binding, .. }
                if id == interjection_id && binding == &new_binding)));
        let current_rows = interjection_rows(&app);
        assert_eq!(current_rows.len(), 1);
        let make_meta = |binding| {
            let mut meta = serde_json::json!({"currentActionsVersion": {"incarnation": incarnation, "sequence": 1}});
            meta[DELIVERY_META_KEY] = serde_json::to_value(DeliveryRole::Live {
                context: PublicationContext { binding, incarnation },
            }).unwrap();
            meta
        };
        dispatch(Action::TaskComplete(failure()), &mut app);
        dispatch(Action::TaskComplete(TaskResult::InterjectQueued {
            agent_id: AgentId(0), binding: original_binding.clone(),
        }), &mut app);
        for projection in [InterjectionProjection::Echo, InterjectionProjection::Injected, InterjectionProjection::ReplayChunk] {
            deliver_identity_projection_with_meta(&mut app, "identity-session", interjection_id, "late old source", projection, make_meta(original_binding.clone()));
            assert_eq!(interjection_rows(&app), current_rows);
            let view = app.agents.get(&AgentId(0)).unwrap();
            assert!(view.self_interjection_ids.contains(interjection_id));
            assert!(view.session.pending_prompts.is_empty());
        }
        let mut unbound = make_meta(new_binding.clone());
        unbound[DELIVERY_META_KEY] = serde_json::to_value(DeliveryRole::LiveUnbound {
            session_id: "identity-session".into(), incarnation,
        }).unwrap();
        deliver_identity_projection_with_meta(&mut app, "identity-session", interjection_id, "unknown authority", InterjectionProjection::Echo, unbound);
        assert!(app.agents.get(&AgentId(0)).unwrap().self_interjection_ids.contains(interjection_id));
        deliver_identity_projection_with_meta(&mut app, "identity-session", interjection_id, "original", InterjectionProjection::Injected, make_meta(new_binding.clone()));
        deliver_identity_projection_with_meta(&mut app, "identity-session", interjection_id, "original", InterjectionProjection::Echo, make_meta(new_binding));
        assert_eq!(interjection_rows(&app), current_rows);
        assert!(!app.agents.get(&AgentId(0)).unwrap().self_interjection_ids.contains(interjection_id));
    }

    #[test]
    fn bound_new_client_without_optimistic_row_joins_by_id_with_publication_proof() {
        use crate::app::agent_view::current_actions::CurrentActionsContext;
        use xai_grok_shell::extensions::current_actions::{DeliveryRole, PublicationContext, DELIVERY_META_KEY};
        for order in [
            [InterjectionProjection::Echo, InterjectionProjection::Injected, InterjectionProjection::ReplayChunk],
            [InterjectionProjection::Injected, InterjectionProjection::ReplayChunk, InterjectionProjection::Echo],
            [InterjectionProjection::ReplayChunk, InterjectionProjection::Echo, InterjectionProjection::Injected],
        ] {
            let mut app = make_app_with_agent("identity-session");
            let view = app.agents.get_mut(&AgentId(0)).unwrap();
            view.attached_as_viewer = true;
            let context = CurrentActionsContext {
                binding: view.current_actions_binding().unwrap(), incarnation: uuid::Uuid::new_v4(),
            };
            assert!(view.bind_current_actions(context.clone()));
            let mut meta = serde_json::json!({"currentActionsVersion": {"incarnation": context.incarnation, "sequence": 1}});
            meta[DELIVERY_META_KEY] = serde_json::to_value(DeliveryRole::Live {
                context: PublicationContext { binding: context.binding, incarnation: context.incarnation },
            }).unwrap();
            assert!(interjection_rows(&app).is_empty());
            for (index, id) in ["first-id", "second-id"].into_iter().enumerate() {
                for projection in order.into_iter().chain(order) {
                    deliver_identity_projection_with_meta(&mut app, "identity-session", id, "same text", projection, meta.clone());
                    assert_eq!(interjection_rows(&app).len(), index + 1);
                }
            }
            let rows = interjection_rows(&app);
            assert_ne!(rows[0].0, rows[1].0);
            assert_eq!(rows[0].1, rows[1].1);
        }
    }
