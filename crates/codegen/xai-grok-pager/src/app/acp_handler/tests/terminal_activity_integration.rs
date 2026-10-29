use super::*;
use crate::acp::tracker::TurnActivity;
use crate::app::actions::{Action, TaskResult};
use crate::scrollback::types::ToolCompletion;
use xai_grok_shell::extensions::notification::{RetryState, SessionNotification};

fn acp_event(app: &mut AppView, sid: &str, update: acp::SessionUpdate, meta: serde_json::Value) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::SessionNotification(xai_acp_lib::AcpArgs {
            request: acp::SessionNotification::new(acp::SessionId::new(sid), update)
                .meta(meta.as_object().cloned()),
            response_tx,
        }),
        app,
    );
}

fn xai_event(app: &mut AppView, sid: &str, update: XaiSessionUpdate, meta: serde_json::Value) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp_fixtures::ext_notification(
                "x.ai/session_notification",
                &SessionNotification {
                    session_id: acp::SessionId::new(sid),
                    update,
                    meta: Some(meta),
                },
            ),
            response_tx,
        }),
        app,
    );
}

fn root_turn(sid: &str) -> (AppView, String) {
    let mut app = make_app_with_agent(sid);
    crate::app::dispatch::dispatch(Action::SendPrompt("repair fixture".into()), &mut app);
    let prompt = test_agent(&app, AgentId(0))
        .session
        .current_prompt_id
        .clone()
        .unwrap();
    (app, prompt)
}

fn tool(id: &str, status: acp::ToolCallStatus) -> acp::ToolCall {
    acp::ToolCall::new(id.to_owned(), "/tmp/源.rs")
        .kind(acp::ToolKind::Read)
        .status(status)
        .raw_input(Some(serde_json::json!({"target_file": "/tmp/源.rs"})))
}

fn last_tool(app: &AppView) -> &crate::scrollback::entry::ScrollbackEntry {
    let sb = &test_agent(app, AgentId(0)).scrollback;
    (0..sb.len())
        .rev()
        .filter_map(|i| sb.get(i))
        .find(|entry| matches!(entry.block, RenderBlock::ToolCall(_)))
        .unwrap()
}

#[derive(Clone, Copy)]
enum TerminalDelivery {
    Initial,
    Update,
    Orphan,
}

#[test]
fn acp_ingress_keeps_completed_failed_cancelled_and_unconfirmed_distinct() {
    for replay in [false, true] {
        for delivery in [
            TerminalDelivery::Initial,
            TerminalDelivery::Update,
            TerminalDelivery::Orphan,
        ] {
            for (status, terminal_meta, expected) in [
                (
                    acp::ToolCallStatus::Completed,
                    serde_json::json!({}),
                    ToolCompletion::Completed,
                ),
                (
                    acp::ToolCallStatus::Failed,
                    serde_json::json!({}),
                    ToolCompletion::Failed,
                ),
                (
                    acp::ToolCallStatus::Failed,
                    serde_json::json!({"toolTerminalStatus": "cancelled"}),
                    ToolCompletion::Cancelled,
                ),
            ] {
                let (mut app, prompt) = root_turn("terminal-integration");
                if replay {
                    app.agents.get_mut(&AgentId(0)).unwrap().begin_session_reload(1);
                }
                let envelope = serde_json::json!({"promptId": prompt, "isReplay": replay});
                let terminal = || {
                    acp::SessionUpdate::ToolCallUpdate(
                        acp::ToolCallUpdate::new(
                            "terminal",
                            acp::ToolCallUpdateFields::new()
                                .status(Some(status))
                                .content(Some(vec![acp::ToolCallContent::from(
                                    acp::ContentBlock::Text(acp::TextContent::new(
                                        "cancelled: identical diagnostic, not a status oracle",
                                    )),
                                )])),
                        )
                        .meta(terminal_meta.as_object().cloned()),
                    )
                };
                match delivery {
                    TerminalDelivery::Initial => acp_event(
                        &mut app,
                        "terminal-integration",
                        acp::SessionUpdate::ToolCall(
                            tool("terminal", status).meta(terminal_meta.as_object().cloned()),
                        ),
                        envelope.clone(),
                    ),
                    TerminalDelivery::Update => {
                        acp_event(
                            &mut app,
                            "terminal-integration",
                            acp::SessionUpdate::ToolCall(tool(
                                "terminal",
                                acp::ToolCallStatus::InProgress,
                            )),
                            envelope.clone(),
                        );
                        acp_event(
                            &mut app,
                            "terminal-integration",
                            terminal(),
                            envelope.clone(),
                        );
                    }
                    TerminalDelivery::Orphan => {
                        acp_event(
                            &mut app,
                            "terminal-integration",
                            terminal(),
                            envelope.clone(),
                        );
                        acp_event(
                            &mut app,
                            "terminal-integration",
                            acp::SessionUpdate::ToolCallUpdate(
                                acp::ToolCallUpdate::new(
                                    "terminal",
                                    acp::ToolCallUpdateFields::new()
                                        .title(Some("late partial title".into()))
                                        .status(Some(acp::ToolCallStatus::InProgress)),
                                )
                                .meta(Some(
                                    serde_json::json!({"unrelated": "retained"})
                                        .as_object()
                                        .unwrap()
                                        .clone(),
                                )),
                            ),
                            envelope.clone(),
                        );
                        acp_event(
                            &mut app,
                            "terminal-integration",
                            acp::SessionUpdate::ToolCall(tool(
                                "terminal",
                                acp::ToolCallStatus::Pending,
                            )),
                            envelope.clone(),
                        );
                    }
                }
                let entry = last_tool(&app);
                assert!(!entry.is_running);
                assert_eq!(entry.tool_completion(), expected);
                let appearance = crate::appearance::AppearanceConfig::default();
                let ctx = entry.context(80, &appearance, None);
                assert_eq!(
                    entry.copy_visible_text_in_state(&ctx).as_deref(),
                    Some("/tmp/源.rs")
                );
                acp_event(
                    &mut app,
                    "terminal-integration",
                    acp::SessionUpdate::ToolCall(tool("unconfirmed", acp::ToolCallStatus::Pending)),
                    envelope,
                );
                let agent = app.agents.get_mut(&AgentId(0)).unwrap();
                agent.session.tracker.finish_turn(&mut agent.scrollback);
                assert!(!last_tool(&app).is_running);
                assert_eq!(
                    last_tool(&app).tool_completion(),
                    ToolCompletion::Unconfirmed
                );
            }
        }
    }
}

#[test]
fn native_terminal_after_turn_drain_confirms_only_the_tool_it_names() {
    use crate::theme::color_support;
    for id in [AgentId(0), AgentId(1)] {
        for replay in [false, true] {
            for (status, terminal_meta, expected, label) in [
                (
                    acp::ToolCallStatus::Completed,
                    serde_json::json!({}),
                    ToolCompletion::Completed,
                    "done",
                ),
                (
                    acp::ToolCallStatus::Failed,
                    serde_json::json!({}),
                    ToolCompletion::Failed,
                    "failed",
                ),
                (
                    acp::ToolCallStatus::Failed,
                    serde_json::json!({"toolTerminalStatus": "cancelled"}),
                    ToolCompletion::Cancelled,
                    "cancelled",
                ),
            ] {
                let (mut app, sid, prompt) = target_turn(id);
                acp_event(
                    &mut app,
                    sid,
                    acp::SessionUpdate::ToolCall(tool("old-tool", acp::ToolCallStatus::InProgress)),
                    serde_json::json!({"promptId": prompt}),
                );
                let mut old_entry = test_agent(&app, id)
                    .session
                    .tracker
                    .pending_tool_entry_id("old-tool")
                    .unwrap();
                acp_event(
                    &mut app,
                    sid,
                    acp::SessionUpdate::ToolCall(tool("other-tool", acp::ToolCallStatus::InProgress)),
                    serde_json::json!({"promptId": prompt}),
                );
                let mut other_entry = test_agent(&app, id)
                    .session.tracker.pending_tool_entry_id("other-tool").unwrap();
                crate::app::dispatch::dispatch(
                    Action::TaskComplete(TaskResult::PromptResponse {
                        agent_id: id,
                        prompt_id: Some(prompt.clone()),
                        http_status: None,
                        result: Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
                    }),
                    &mut app,
                );
                assert_eq!(
                    test_agent(&app, id)
                        .scrollback
                        .get_by_id(old_entry)
                        .unwrap()
                        .tool_completion(),
                    ToolCompletion::Unconfirmed
                );
                if replay {
                    app.agents.get_mut(&id).unwrap().begin_session_reload(1);
                    for provider in ["old-tool", "other-tool"] {
                        acp_event(
                            &mut app,
                            sid,
                            acp::SessionUpdate::ToolCall(tool(provider, acp::ToolCallStatus::InProgress)),
                            serde_json::json!({"promptId": prompt, "isReplay": true}),
                        );
                    }
                    let historical = &test_agent(&app, id).session.tracker;
                    old_entry = historical.pending_tool_entry_id("old-tool").unwrap();
                    other_entry = historical.pending_tool_entry_id("other-tool").unwrap();
                }
                crate::app::dispatch::dispatch(
                    Action::SendPrompt("next fixture turn".into()),
                    &mut app,
                );
                xai_event(
                    &mut app,
                    sid,
                    XaiSessionUpdate::RetryState(RetryState::Retrying {
                        attempt: 1,
                        max_retries: 3,
                        reason: "new turn retry".into(),
                        error_type: None,
                        next_attempt_at_ms: None,
                    }),
                    serde_json::json!({}),
                );
                if replay {
                    assert!(app.agents.get_mut(&id).unwrap().finish_session_reload(1, true));
                    assert_eq!(
                        test_agent(&app, id).scrollback.get_by_id(old_entry).unwrap().tool_completion(),
                        ToolCompletion::Unconfirmed,
                    );
                }
                let agent = test_agent(&app, id);
                let retry = agent.session.tracker.retry_activity().cloned();
                assert!(matches!(
                    retry,
                    Some(TurnActivity::Retrying { attempt: 1, .. })
                ));
                assert_ne!(
                    agent.session.current_prompt_id.as_deref(),
                    Some(prompt.as_str())
                );
                let phase = agent.session.tracker.activity();
                let highwater = agent.last_applied_event_seq;
                let context = agent.context_state.as_ref().map(|state| state.used);
                let activity = agent.last_active_at;
                let first_activity = agent.first_activity_logged_for;
                let current_prompt = agent.session.current_prompt_id.clone();
                let clock = agent.turn_started_at;
                let anchor = agent.turn_start_ms;
                let ack = agent.prompt_ack.clone();
                let in_flight = agent
                    .session
                    .in_flight_prompt
                    .as_ref()
                    .map(|p| p.text.clone());
                let entries = agent.scrollback.len();
                for sequence in [1, 2, 2] {
                    let generation = test_agent(&app, id).scrollback.content_generation();
                    acp_event(
                        &mut app,
                        sid,
                        acp::SessionUpdate::ToolCallUpdate(
                            acp::ToolCallUpdate::new(
                                "old-tool",
                                acp::ToolCallUpdateFields::new().status(Some(status)),
                            )
                            .meta(terminal_meta.as_object().cloned()),
                        ),
                        serde_json::json!({"isReplay": replay, "eventId": format!("{sid}-{sequence}"),
                        "promptId": (sequence == 2).then_some(prompt.as_str()),
                        "turnStartMs": 1, "totalTokens": 999_999}),
                    );
                    let agent = test_agent(&app, id);
                    let entry = agent.scrollback.get_by_id(old_entry).unwrap();
                    assert_eq!(
                        entry.tool_completion(), expected,
                        "agent={id:?} replay={replay} sequence={sequence} status={status:?}"
                    );
                    assert!(!entry.is_running);
                    let unrelated = agent.scrollback.get_by_id(other_entry).unwrap();
                    assert_eq!(unrelated.tool_completion(), ToolCompletion::Unconfirmed);
                    assert!(!unrelated.is_running);
                    assert_eq!(agent.scrollback.len(), entries);
                    if sequence == 2 {
                        assert_eq!(agent.scrollback.content_generation(), generation);
                    }
                    assert_eq!(agent.session.tracker.retry_activity(), retry.as_ref());
                    assert_eq!(agent.session.tracker.activity(), phase);
                    assert_eq!(agent.last_applied_event_seq, highwater);
                    assert_eq!(
                        agent.context_state.as_ref().map(|state| state.used),
                        context
                    );
                    assert_eq!(agent.last_active_at, activity);
                    assert_eq!(agent.first_activity_logged_for, first_activity);
                    assert_eq!(agent.session.current_prompt_id, current_prompt);
                    assert_eq!(agent.turn_started_at, clock);
                    assert_eq!(agent.turn_start_ms, anchor);
                    assert_eq!(agent.prompt_ack, ack);
                    assert_eq!(
                        agent
                            .session
                            .in_flight_prompt
                            .as_ref()
                            .map(|p| p.text.clone()),
                        in_flight
                    );
                    color_support::set_level_for_test(color_support::ColorLevel::None);
                    let appearance = crate::appearance::AppearanceConfig::default();
                    let ctx = entry.context(80, &appearance, None);
                    let symbol = entry.message_symbol(&ctx).unwrap();
                    assert_eq!(
                        symbol.prefix(&ctx),
                        format!("{} [{label}] ", crate::glyphs::tool_read_file())
                    );
                    assert!(!symbol.style(&ctx).unwrap().animated);
                    color_support::set_level_for_test(color_support::ColorLevel::TrueColor);
                }
            }
        }
    }
}

#[test]
fn recycled_legacy_provider_cannot_confirm_either_drained_entry() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, first_prompt) = target_turn(id);
        let mut entries = Vec::new();
        for turn in 0..2 {
            if turn == 1 {
                crate::app::dispatch::dispatch(Action::SendPrompt("reuse provider".into()), &mut app);
            }
            let prompt = test_agent(&app, id).session.current_prompt_id.clone().unwrap();
            acp_event(
                &mut app,
                sid,
                acp::SessionUpdate::ToolCall(tool("recycled", acp::ToolCallStatus::InProgress)),
                serde_json::json!({"promptId": prompt}),
            );
            entries.push(test_agent(&app, id).session.tracker
                .pending_tool_entry_id("recycled").unwrap());
            crate::app::dispatch::dispatch(
                Action::TaskComplete(TaskResult::PromptResponse {
                    agent_id: id,
                    prompt_id: Some(prompt),
                    http_status: None,
                    result: Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
                }),
                &mut app,
            );
        }
        assert_ne!(entries[0], entries[1]);
        crate::app::dispatch::dispatch(Action::SendPrompt("unrelated next turn".into()), &mut app);
        let agent = test_agent(&app, id);
        let current = agent.session.current_prompt_id.clone();
        let ack = agent.prompt_ack.clone();
        let activity = agent.last_active_at;
        let highwater = agent.last_applied_event_seq;
        acp_event(
            &mut app,
            sid,
            acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                "recycled",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
            )),
            serde_json::json!({"promptId": first_prompt, "eventId": format!("{sid}-999")}),
        );
        let agent = test_agent(&app, id);
        for entry in entries {
            let entry = agent.scrollback.get_by_id(entry).unwrap();
            assert_eq!(entry.tool_completion(), ToolCompletion::Unconfirmed);
            assert!(!entry.is_running);
        }
        assert_eq!(agent.session.current_prompt_id, current);
        assert_eq!(agent.prompt_ack, ack);
        assert_eq!(agent.last_active_at, activity);
        assert_eq!(agent.last_applied_event_seq, highwater);
    }
}

#[test]
fn registered_terminal_cannot_confirm_a_drained_legacy_entry_with_the_same_provider() {
    use xai_grok_shell::extensions::tool_timing::{
        ToolCallOwnership, ToolCallSource, stamp_ownership,
    };
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target_turn(id);
        acp_event(
            &mut app,
            sid,
            acp::SessionUpdate::ToolCall(tool("legacy", acp::ToolCallStatus::InProgress)),
            serde_json::json!({"promptId": prompt}),
        );
        let original = test_agent(&app, id).session.tracker
            .pending_tool_entry_id("legacy").unwrap();
        crate::app::dispatch::dispatch(
            Action::TaskComplete(TaskResult::PromptResponse {
                agent_id: id,
                prompt_id: Some(prompt.clone()),
                http_status: None,
                result: Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
            }),
            &mut app,
        );
        crate::app::dispatch::dispatch(Action::SendPrompt("registered next turn".into()), &mut app);
        let current = test_agent(&app, id).session.current_prompt_id.clone();
        let ack = test_agent(&app, id).prompt_ack.clone();
        let mut metadata = acp::Meta::new();
        stamp_ownership(&mut metadata, &ToolCallOwnership::Registered {
            invocation_id: uuid::Uuid::from_u128(10987).to_string(),
            provider_call_id: "legacy".into(),
            source: ToolCallSource::Unknown,
        }).unwrap();
        acp_event(
            &mut app,
            sid,
            acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                "legacy",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
            ).meta(Some(metadata))),
            serde_json::json!({"promptId": prompt}),
        );
        let agent = test_agent(&app, id);
        assert_eq!(agent.scrollback.get_by_id(original).unwrap().tool_completion(), ToolCompletion::Unconfirmed);
        assert_eq!(agent.session.current_prompt_id, current);
        assert_eq!(agent.prompt_ack, ack);
    }
}

#[test]
fn terminal_during_reload_cannot_cross_live_and_historical_legacy_owners() {
    for id in [AgentId(0), AgentId(1)] {
        for replay in [false, true] {
            let (mut app, sid, prompt) = target_turn(id);
            acp_event(
                &mut app,
                sid,
                acp::SessionUpdate::ToolCall(tool("same-provider", acp::ToolCallStatus::InProgress)),
                serde_json::json!({"promptId": prompt}),
            );
            let original = test_agent(&app, id).session.tracker
                .pending_tool_entry_id("same-provider").unwrap();
            crate::app::dispatch::dispatch(
                Action::TaskComplete(TaskResult::PromptResponse {
                    agent_id: id,
                    prompt_id: Some(prompt.clone()),
                    http_status: None,
                    result: Ok(acp::PromptResponse::new(acp::StopReason::Cancelled)),
                }),
                &mut app,
            );
            assert_eq!(
                test_agent(&app, id).scrollback.get_by_id(original).unwrap().tool_completion(),
                ToolCompletion::Unconfirmed,
            );
            crate::app::dispatch::dispatch(Action::SendPrompt("live next turn".into()), &mut app);
            app.agents.get_mut(&id).unwrap().begin_session_reload(1);
            acp_event(
                &mut app,
                sid,
                acp::SessionUpdate::ToolCall(tool("same-provider", acp::ToolCallStatus::InProgress)),
                serde_json::json!({"promptId": prompt, "isReplay": true}),
            );
            let historical = test_agent(&app, id).session.tracker
                .pending_tool_entry_id("same-provider").unwrap();
            assert_ne!(original, historical);
            let agent = test_agent(&app, id);
            let current = agent.session.current_prompt_id.clone();
            let ack = agent.prompt_ack.clone();
            let activity = agent.last_active_at;
            let highwater = agent.last_applied_event_seq;
            let clock = agent.turn_started_at;
            let context = agent.context_state.as_ref().map(|state| state.used);
            for sequence in [1, 2, 2] {
                acp_event(
                    &mut app,
                    sid,
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        "same-provider",
                        acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                    )),
                    serde_json::json!({
                        "isReplay": replay, "promptId": prompt,
                        "eventId": format!("{sid}-{sequence}"),
                        "turnStartMs": 1, "totalTokens": 999_999,
                    }),
                );
                let agent = test_agent(&app, id);
                let entry = agent.scrollback.get_by_id(historical).unwrap();
                assert_eq!(entry.tool_completion(), if replay {
                    ToolCompletion::Completed
                } else {
                    ToolCompletion::Unconfirmed
                }, "agent={id:?} replay={replay} sequence={sequence}");
                assert_eq!(entry.is_running, !replay);
                assert_eq!(agent.session.current_prompt_id, current);
                assert_eq!(agent.prompt_ack, ack);
                assert_eq!(agent.last_active_at, activity);
                assert_eq!(agent.last_applied_event_seq, highwater);
                assert_eq!(agent.turn_started_at, clock);
                assert_eq!(agent.context_state.as_ref().map(|state| state.used), context);
            }
            assert!(app.agents.get_mut(&id).unwrap().finish_session_reload(1, false));
            let original = test_agent(&app, id).scrollback.get_by_id(original).unwrap();
            assert_eq!(original.tool_completion(), if replay {
                ToolCompletion::Unconfirmed
            } else {
                ToolCompletion::Completed
            });
            assert!(!original.is_running);
        }
    }
}

fn fragment(index: u32, name: Option<&str>) -> XaiSessionUpdate {
    XaiSessionUpdate::ToolCallDeltaChunk {
        tool_call_id: None,
        tool_index: index,
        name: name.map(str::to_owned),
        arguments_delta: Some("new bytes".into()),
    }
}

#[test]
fn root_argument_fragments_count_during_load_but_duplicates_replay_and_stale_streams_do_not() {
    let (mut app, prompt) = root_turn("root-fragments");
    app.agents
        .get_mut(&AgentId(0))
        .unwrap()
        .session
        .loading_replay = true;
    let meta = |seq, request, epoch| {
        serde_json::json!({
            "promptId": prompt, "eventId": format!("root-fragments-{seq}"),
            "requestId": request, "streamEpoch": epoch,
        })
    };
    for seq in [1, 2] {
        let before = Instant::now();
        let stamp = meta(seq, "request-1", 1);
        xai_event(
            &mut app,
            "root-fragments",
            fragment(0, Some("read_file")),
            stamp.clone(),
        );
        let activity = test_agent(&app, AgentId(0)).last_active_at.unwrap();
        assert!(activity >= before);
        xai_event(
            &mut app,
            "root-fragments",
            fragment(0, Some("read_file")),
            stamp,
        );
        assert_eq!(test_agent(&app, AgentId(0)).last_active_at, Some(activity));
    }
    xai_event(
        &mut app,
        "root-fragments",
        fragment(0, Some("read_file")),
        meta(3, "request-1", 2),
    );
    let admitted = test_agent(&app, AgentId(0)).last_active_at;
    for rejected in [
        serde_json::json!({"promptId": prompt, "isReplay": true, "eventId": "root-fragments-90"}),
        serde_json::json!({"promptId": "old-prompt", "eventId": "root-fragments-91"}),
        meta(92, "request-1", 1),
    ] {
        xai_event(
            &mut app,
            "root-fragments",
            fragment(0, Some("edit_file")),
            rejected,
        );
        assert_eq!(test_agent(&app, AgentId(0)).last_active_at, admitted);
    }
    let before = Instant::now();
    xai_event(
        &mut app,
        "root-fragments",
        fragment(0, None),
        meta(4, "request-1", 2),
    );
    assert!(
        test_agent(&app, AgentId(0)).last_active_at.unwrap() >= before,
        "rejected stale-bound high sequences must not poison the live highwater"
    );
}

#[test]
fn child_argument_streams_reset_ordinals_and_wire_replay_does_not_own_live_retry() {
    let mut app = make_app_with_parent_and_child("child-stream-parent", "child-stream");
    let parent_activity = test_agent(&app, AgentId(0)).last_active_at;
    let child = app.agents.get_mut(&AgentId(1)).unwrap();
    child.session.current_prompt_id = Some("child-prompt".into());
    child.session.state = AgentState::TurnRunning;
    child.session.loading_replay = true;
    let meta = |seq, request, epoch| {
        serde_json::json!({
            "promptId": "child-prompt", "eventId": format!("child-stream-{seq}"),
            "requestId": request, "streamEpoch": epoch,
        })
    };
    for (seq, index, request, epoch, ordinal) in [
        (1, 0, "request-1", 1, 1),
        (2, 1, "request-1", 1, 2),
        (3, 1, "request-1", 2, 1),
        (4, 1, "request-2", 0, 1),
    ] {
        let before = Instant::now();
        xai_event(
            &mut app,
            "child-stream",
            fragment(index, Some("read_file")),
            meta(seq, request, epoch),
        );
        let child = test_agent(&app, AgentId(1));
        assert!(child.last_active_at.unwrap() >= before);
        match child.session.tracker.activity() {
            Some(TurnActivity::WritingToolCall(writing)) => {
                assert_eq!(writing.ordinal.get(), ordinal)
            }
            other => panic!("expected writing activity, got {other:?}"),
        }
    }
    let admitted = test_agent(&app, AgentId(1)).last_active_at;
    for rejected in [
        meta(4, "request-2", 0),
        meta(90, "request-1", 2),
        serde_json::json!({"promptId": "child-prompt", "isReplay": true, "eventId": "child-stream-91"}),
    ] {
        xai_event(
            &mut app,
            "child-stream",
            fragment(0, Some("edit_file")),
            rejected,
        );
        assert_eq!(test_agent(&app, AgentId(1)).last_active_at, admitted);
    }
    let retry = || {
        XaiSessionUpdate::RetryState(RetryState::Retrying {
            attempt: 1,
            max_retries: 3,
            reason: "fixture retry".into(),
            error_type: None,
            next_attempt_at_ms: None,
        })
    };
    xai_event(
        &mut app,
        "child-stream",
        retry(),
        serde_json::json!({"isReplay": true}),
    );
    assert!(
        test_agent(&app, AgentId(1))
            .session
            .tracker
            .retry_activity()
            .is_none()
    );
    xai_event(&mut app, "child-stream", retry(), meta(5, "request-2", 0));
    assert!(matches!(
        test_agent(&app, AgentId(1))
            .session
            .tracker
            .retry_activity(),
        Some(TurnActivity::Retrying { attempt: 1, .. })
    ));
    let live_retry = test_agent(&app, AgentId(1))
        .session
        .tracker
        .retry_activity()
        .cloned();
    app.agents
        .get_mut(&AgentId(1))
        .unwrap()
        .session
        .loading_replay = false;
    for terminal in [
        XaiSessionUpdate::RetryState(RetryState::Exhausted {
            attempts: 1,
            reason: "historical failure".into(),
            is_rate_limited: false,
        }),
        XaiSessionUpdate::RetryState(RetryState::Failed {
            error_type: "api".into(),
            message: "historical failure".into(),
        }),
    ] {
        xai_event(
            &mut app,
            "child-stream",
            terminal,
            serde_json::json!({"isReplay": true}),
        );
        assert_eq!(
            test_agent(&app, AgentId(1))
                .session
                .tracker
                .retry_activity(),
            live_retry.as_ref()
        );
        assert_eq!(test_agent(&app, AgentId(1)).last_active_at, admitted);
    }
    assert_eq!(test_agent(&app, AgentId(0)).last_active_at, parent_activity);
}

#[test]
fn proven_live_background_snapshots_count_after_admission_even_during_load() {
    let mut app = make_app_with_parent_and_child("snapshot-live-parent", "snapshot-live-child");
    crate::app::dispatch::dispatch(Action::SendPrompt("wait for model".into()), &mut app);
    for (id, sid) in [
        (AgentId(0), "snapshot-live-parent"),
        (AgentId(1), "snapshot-live-child"),
    ] {
        app.agents.get_mut(&id).unwrap().session.loading_replay = true;
        let original = test_agent(&app, id).last_active_at;
        let waiting_for_prompt = test_agent(&app, id).session.in_flight_prompt.is_some();
        let snapshot = || XaiSessionUpdate::BackgroundTasks {
            tasks: Vec::new(),
            truncated: false,
        };
        xai_event(
            &mut app,
            sid,
            snapshot(),
            serde_json::json!({"activityProvenance": "observation"}),
        );
        assert_eq!(test_agent(&app, id).last_active_at, original);
        xai_event(
            &mut app,
            sid,
            snapshot(),
            serde_json::json!({"activityProvenance": "live", "isReplay": true}),
        );
        assert_eq!(test_agent(&app, id).last_active_at, original);
        let before = Instant::now();
        let stamp =
            serde_json::json!({"activityProvenance": "live", "eventId": format!("{sid}-10")});
        xai_event(&mut app, sid, snapshot(), stamp.clone());
        let accepted = test_agent(&app, id).last_active_at.unwrap();
        assert!(accepted >= before);
        xai_event(&mut app, sid, snapshot(), stamp);
        xai_event(
            &mut app,
            sid,
            snapshot(),
            serde_json::json!({"activityProvenance": "live", "eventId": format!("{sid}-9")}),
        );
        assert_eq!(test_agent(&app, id).last_active_at, Some(accepted));
        assert_eq!(
            test_agent(&app, id).session.in_flight_prompt.is_some(),
            waiting_for_prompt
        );
    }
}

#[test]
fn only_confirmed_completed_edits_coalesce_through_acp_ingress() {
    std::thread::spawn(|| {
        crate::appearance::cache::set_collapsed_edit_blocks(true);
        for confirmed in [false, true] {
            let mut app = make_app_with_agent("coalesce-terminal");
            app.agents
                .get_mut(&AgentId(0))
                .unwrap()
                .session
                .loading_replay = true;
            for (id, line, status) in [
                (
                    "first",
                    5,
                    if confirmed {
                        acp::ToolCallStatus::Completed
                    } else {
                        acp::ToolCallStatus::InProgress
                    },
                ),
                ("second", 40, acp::ToolCallStatus::Completed),
            ] {
                let diff = acp::Diff::new("/tmp/edit.rs", format!("new_{line}"))
                    .old_text(Some(format!("old_{line}")))
                    .meta(
                        serde_json::json!({"old_line": line, "new_line": line})
                            .as_object()
                            .cloned(),
                    );
                acp_event(
                    &mut app,
                    "coalesce-terminal",
                    acp::SessionUpdate::ToolCall(
                        acp::ToolCall::new(id, "/tmp/edit.rs")
                            .kind(acp::ToolKind::Edit)
                            .status(status)
                            .raw_input(Some(serde_json::json!({"file_path": "/tmp/edit.rs"})))
                            .content(vec![acp::ToolCallContent::Diff(diff)]),
                    ),
                    serde_json::json!({"isReplay": true}),
                );
                let agent = app.agents.get_mut(&AgentId(0)).unwrap();
                agent.session.tracker.finish_turn(&mut agent.scrollback);
            }
            let sb = &test_agent(&app, AgentId(0)).scrollback;
            assert_eq!(sb.len(), if confirmed { 1 } else { 2 });
            assert_eq!(
                sb.get(0).unwrap().tool_completion(),
                if confirmed {
                    ToolCompletion::Completed
                } else {
                    ToolCompletion::Unconfirmed
                }
            );
            assert_eq!(last_tool(&app).tool_completion(), ToolCompletion::Completed);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn child_wire_live_compaction_during_load_is_not_historical_and_replay_cannot_finish_it() {
    use xai_grok_shell::extensions::notification::{
        CompactPhase, MODEL_FAMILY_SWITCH_COMPACT_BANNER,
    };
    let mut app = make_app_with_parent_and_child("compact-wire-parent", "compact-wire-child");
    app.agents
        .get_mut(&AgentId(1))
        .unwrap()
        .session
        .loading_replay = true;
    xai_event(
        &mut app,
        "compact-wire-child",
        XaiSessionUpdate::AutoCompactStarted {
            phase: CompactPhase::Executing,
            tokens_used: 12_000,
            context_window: 100_000,
            percentage: 12,
            reason: MODEL_FAMILY_SWITCH_COMPACT_BANNER.into(),
        },
        serde_json::json!({"eventId": "compact-wire-child-1"}),
    );
    assert!(
        test_agent(&app, AgentId(1))
            .session
            .state
            .is_switch_model_compact()
    );
    let active = test_agent(&app, AgentId(1)).last_active_at;
    app.agents
        .get_mut(&AgentId(1))
        .unwrap()
        .session
        .loading_replay = false;
    let completed = || XaiSessionUpdate::AutoCompactCompleted {
        tokens_before: Some(12_000),
        tokens_after: 4_000,
        elapsed_ms: Some(100),
        summary_preview: None,
    };
    xai_event(
        &mut app,
        "compact-wire-child",
        completed(),
        serde_json::json!({"isReplay": true}),
    );
    assert!(
        test_agent(&app, AgentId(1))
            .session
            .state
            .is_switch_model_compact()
    );
    assert_eq!(test_agent(&app, AgentId(1)).last_active_at, active);
    xai_event(
        &mut app,
        "compact-wire-child",
        completed(),
        serde_json::json!({"eventId": "compact-wire-child-2"}),
    );
    assert!(test_agent(&app, AgentId(1)).session.state.is_idle());
}

#[derive(Clone, Copy)]
enum TextFragment {
    Message,
    Thought,
}

fn text_chunk(kind: TextFragment, text: &str) -> acp::SessionUpdate {
    let chunk = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)));
    match kind {
        TextFragment::Message => acp::SessionUpdate::AgentMessageChunk(chunk),
        TextFragment::Thought => acp::SessionUpdate::AgentThoughtChunk(chunk),
    }
}

fn target_turn(id: AgentId) -> (AppView, &'static str, String) {
    let mut app = make_app_viewing_child("admission-root", "admission-child");
    app.active_view = ActiveView::Agent(id);
    crate::app::dispatch::dispatch(Action::SendPrompt("turn A".into()), &mut app);
    let prompt = test_agent(&app, id)
        .session
        .current_prompt_id
        .clone()
        .unwrap();
    let sid = if id == AgentId(0) {
        "admission-root"
    } else {
        "admission-child"
    };
    (app, sid, prompt)
}

#[test]
fn root_and_child_rejected_high_sequences_and_duplicates_cannot_ack_or_poison_current_turn() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, old) = target_turn(id);
        acp_event(
            &mut app,
            sid,
            text_chunk(TextFragment::Message, "A output"),
            serde_json::json!({"promptId": old, "eventId": format!("{sid}-10"), "turnStartMs": 10}),
        );
        crate::app::dispatch::dispatch(
            Action::TaskComplete(TaskResult::PromptResponse {
                agent_id: id,
                prompt_id: Some(old.clone()),
                http_status: None,
                result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
            }),
            &mut app,
        );
        crate::app::dispatch::dispatch(Action::SendPrompt("turn B".into()), &mut app);
        let view = app.agents.get_mut(&id).unwrap();
        let current = view.session.current_prompt_id.clone().unwrap();
        assert_ne!(old, current);
        view.late_replay_until = None;
        view.turn_start_ms = Some(20);
        view.turn_start_ms_prompt = Some(current.clone());
        view.apply_context_used(42, 1000);
        let ack = view.prompt_ack.clone();
        assert!(ack.is_some());
        let in_flight = view.session.in_flight_prompt.as_ref().unwrap().text.clone();
        let started = view.turn_started_at;
        let activity = view.last_active_at;
        let generation = view.scrollback.content_generation();
        for update in [
            text_chunk(TextFragment::Message, "late A"),
            acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(999, 1000)),
        ] {
            acp_event(
                &mut app,
                sid,
                update,
                serde_json::json!({
                    "promptId": old, "eventId": format!("{sid}-100"), "turnStartMs": 1, "totalTokens": 999,
                }),
            );
            let view = test_agent(&app, id);
            assert_eq!(view.last_applied_event_seq, Some(10));
            assert_eq!(view.session.current_prompt_id.as_ref(), Some(&current));
            assert_eq!(view.prompt_ack, ack);
            assert_eq!(
                view.session.in_flight_prompt.as_ref().unwrap().text,
                in_flight
            );
            assert_eq!(view.context_state.as_ref().unwrap().used, 42);
            assert_eq!(view.turn_start_ms, Some(20));
            assert_eq!(view.turn_start_ms_prompt.as_ref(), Some(&current));
            assert_eq!(view.turn_started_at, started);
            assert_eq!(view.last_active_at, activity);
            assert_eq!(view.scrollback.content_generation(), generation);
        }
        for rejected in [
            serde_json::json!({"eventId": format!("{sid}-100"), "turnStartMs": 10, "totalTokens": 999}),
            serde_json::json!({"promptId": current, "eventId": format!("{sid}-100"), "isReplay": true,
                "turnStartMs": 1, "totalTokens": 999}),
        ] {
            acp_event(
                &mut app,
                sid,
                text_chunk(TextFragment::Message, "rejected turn/replay"),
                rejected,
            );
            let view = test_agent(&app, id);
            assert_eq!(view.prompt_ack, ack);
            assert_eq!(view.last_applied_event_seq, Some(10));
            assert_eq!(
                view.session.in_flight_prompt.as_ref().unwrap().text,
                in_flight
            );
            assert_eq!(view.context_state.as_ref().unwrap().used, 42);
            assert_eq!(view.turn_start_ms, Some(20));
            assert_eq!(view.turn_started_at, started);
            assert_eq!(view.last_active_at, activity);
        }
        acp_event(
            &mut app,
            sid,
            text_chunk(TextFragment::Message, "duplicate pretending to be B"),
            serde_json::json!({
                "promptId": current, "eventId": format!("{sid}-10"), "turnStartMs": 1, "totalTokens": 999,
            }),
        );
        let view = test_agent(&app, id);
        assert_eq!(view.prompt_ack, ack);
        assert_eq!(view.last_applied_event_seq, Some(10));
        assert_eq!(view.context_state.as_ref().unwrap().used, 42);
        assert_eq!(view.turn_start_ms, Some(20));
        assert_eq!(view.turn_started_at, started);
        assert_eq!(view.last_active_at, activity);
        let before = Instant::now();
        acp_event(
            &mut app,
            sid,
            text_chunk(TextFragment::Message, "valid B"),
            serde_json::json!({
                "promptId": current, "eventId": format!("{sid}-11"), "turnStartMs": 20, "totalTokens": 43,
            }),
        );
        let view = test_agent(&app, id);
        assert_eq!(view.last_applied_event_seq, Some(11));
        assert!(view.prompt_ack.is_none());
        assert_eq!(view.context_state.as_ref().unwrap().used, 43);
        assert!(view.last_active_at.unwrap() >= before);
        assert!(view.scrollback.iter_entries().any(|(_, entry)| {
            entry
                .block
                .copy_text(true)
                .is_some_and(|text| text.contains("valid B"))
        }));
    }
}

#[test]
fn root_and_child_live_text_thought_and_stdout_count_during_load_with_identical_labels() {
    for id in [AgentId(0), AgentId(1)] {
        for kind in [TextFragment::Message, TextFragment::Thought] {
            let (mut app, sid, prompt) = target_turn(id);
            let view = app.agents.get_mut(&id).unwrap();
            view.session.loading_replay = true;
            view.note_self_originated_prompt("stale-A");
            view.ended_child_prompt_ids.insert("stale-A".into());
            view.session.bg_tasks.insert(
                "background".into(),
                crate::app::agent_test_fixtures::running_bg_task("background", false),
            );
            view.session
                .bg_tool_call_to_task
                .insert("bg-call".into(), "background".into());
            for sequence in [1, 2] {
                let ack = test_agent(&app, id).prompt_ack.clone();
                let first = test_agent(&app, id).first_activity_logged_for;
                let in_flight = test_agent(&app, id)
                    .session
                    .in_flight_prompt
                    .as_ref()
                    .map(|p| p.text.clone());
                let stdout = || {
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        "bg-call",
                        acp::ToolCallUpdateFields::new().raw_output(Some(serde_json::json!({
                            "type": "Bash", "output_for_prompt": "same stdout",
                        }))),
                    ))
                };
                let before = Instant::now();
                let stamp = serde_json::json!({"eventId": format!("{sid}-{sequence}")});
                acp_event(&mut app, sid, stdout(), stamp.clone());
                let active = test_agent(&app, id).last_active_at;
                assert!(active.unwrap() >= before);
                assert_eq!(test_agent(&app, id).prompt_ack, ack);
                assert_eq!(test_agent(&app, id).first_activity_logged_for, first);
                assert_eq!(
                    test_agent(&app, id)
                        .session
                        .in_flight_prompt
                        .as_ref()
                        .map(|p| p.text.clone()),
                    in_flight
                );
                assert_eq!(
                    test_agent(&app, id)
                        .session
                        .bg_tasks
                        .get("background")
                        .unwrap()
                        .stdout,
                    "same stdout"
                );
                acp_event(&mut app, sid, stdout(), stamp);
                assert_eq!(test_agent(&app, id).last_active_at, active);
                acp_event(
                    &mut app,
                    sid,
                    stdout(),
                    serde_json::json!({"isReplay": true}),
                );
                assert_eq!(test_agent(&app, id).last_active_at, active);
                acp_event(
                    &mut app,
                    sid,
                    stdout(),
                    serde_json::json!({
                        "promptId": "stale-A", "eventId": format!("{sid}-100"),
                    }),
                );
                assert_eq!(test_agent(&app, id).last_active_at, active);
                acp_event(
                    &mut app,
                    sid,
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        "bg-call",
                        acp::ToolCallUpdateFields::new().raw_output(Some(serde_json::json!({
                            "type": "Bash", "output_for_prompt": "",
                        }))),
                    )),
                    serde_json::json!({}),
                );
                assert_eq!(test_agent(&app, id).last_active_at, active);
            }
            for sequence in [3, 4] {
                let before = Instant::now();
                let stamp =
                    serde_json::json!({"promptId": prompt, "eventId": format!("{sid}-{sequence}")});
                acp_event(
                    &mut app,
                    sid,
                    text_chunk(kind, "same fragment"),
                    stamp.clone(),
                );
                let active = test_agent(&app, id).last_active_at;
                assert!(active.unwrap() >= before);
                let label = match kind {
                    TextFragment::Message => Some(TurnActivity::Responding),
                    TextFragment::Thought => Some(TurnActivity::Thinking),
                };
                assert_eq!(test_agent(&app, id).session.tracker.activity(), label);
                acp_event(&mut app, sid, text_chunk(kind, "same fragment"), stamp);
                assert_eq!(test_agent(&app, id).last_active_at, active);
                for rejected in [
                    serde_json::json!({"promptId": "stale-A", "eventId": format!("{sid}-100")}),
                    serde_json::json!({"promptId": prompt, "isReplay": true}),
                ] {
                    acp_event(&mut app, sid, text_chunk(kind, "same fragment"), rejected);
                    assert_eq!(test_agent(&app, id).last_active_at, active);
                }
                assert_eq!(test_agent(&app, id).session.tracker.activity(), label);
            }
        }
    }
}

#[test]
fn late_completed_edit_does_not_coalesce_across_a_new_prompt() {
    std::thread::spawn(|| {
        crate::appearance::cache::set_collapsed_edit_blocks(true);
        for replay in [false, true] {
            let (mut app, prompt) = root_turn("late-edit");
            let edit = |id, line, status| {
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new(id, "/tmp/edit.rs")
                        .kind(acp::ToolKind::Edit)
                        .status(status)
                        .raw_input(Some(serde_json::json!({"file_path": "/tmp/edit.rs"})))
                        .content(vec![acp::ToolCallContent::Diff(
                            acp::Diff::new("/tmp/edit.rs", format!("new_{line}"))
                                .old_text(Some(format!("old_{line}")))
                                .meta(
                                    serde_json::json!({"old_line": line, "new_line": line})
                                        .as_object()
                                        .cloned(),
                                ),
                        )]),
                )
            };
            acp_event(
                &mut app,
                "late-edit",
                edit("old-edit", 5, acp::ToolCallStatus::InProgress),
                serde_json::json!({"promptId": prompt}),
            );
            let old = last_tool(&app).id;
            crate::app::dispatch::dispatch(
                Action::TaskComplete(TaskResult::PromptResponse {
                    agent_id: AgentId(0),
                    prompt_id: Some(prompt),
                    http_status: None,
                    result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
                }),
                &mut app,
            );
            crate::app::dispatch::dispatch(Action::SendPrompt("new edit turn".into()), &mut app);
            let current = test_agent(&app, AgentId(0))
                .session
                .current_prompt_id
                .clone();
            acp_event(
                &mut app,
                "late-edit",
                edit("new-edit", 40, acp::ToolCallStatus::Completed),
                serde_json::json!({"promptId": current}),
            );
            let new = last_tool(&app).id;
            assert_ne!(old, new);
            let entries = test_agent(&app, AgentId(0)).scrollback.len();
            app.agents
                .get_mut(&AgentId(0))
                .unwrap()
                .session
                .loading_replay = replay;
            for status in [
                acp::ToolCallStatus::Completed,
                acp::ToolCallStatus::Completed,
                acp::ToolCallStatus::Failed,
            ] {
                acp_event(
                    &mut app,
                    "late-edit",
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        "old-edit",
                        acp::ToolCallUpdateFields::new().status(Some(status)),
                    )),
                    serde_json::json!({"isReplay": replay}),
                );
                let view = test_agent(&app, AgentId(0));
                assert_eq!(view.scrollback.len(), entries);
                assert_eq!(
                    view.scrollback.get_by_id(old).unwrap().tool_completion(),
                    ToolCompletion::Completed
                );
                assert_eq!(
                    view.scrollback.get_by_id(new).unwrap().tool_completion(),
                    ToolCompletion::Completed
                );
                assert_eq!(view.session.current_prompt_id, current);
            }
        }
    })
    .join()
    .unwrap();
}

#[test]
fn unknown_terminal_stays_an_orphan_until_its_real_tool_call_arrives() {
    for replay in [false, true] {
        let (mut app, prompt) = root_turn("unknown-terminal");
        app.agents
            .get_mut(&AgentId(0))
            .unwrap()
            .session
            .loading_replay = replay;
        let before = test_agent(&app, AgentId(0)).scrollback.len();
        let stamp = serde_json::json!({"isReplay": replay, "promptId": prompt});
        acp_event(
            &mut app,
            "unknown-terminal",
            acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                "unknown",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
            )),
            stamp.clone(),
        );
        assert_eq!(test_agent(&app, AgentId(0)).scrollback.len(), before);
        acp_event(
            &mut app,
            "unknown-terminal",
            acp::SessionUpdate::ToolCall(tool("unknown", acp::ToolCallStatus::Pending)),
            stamp,
        );
        assert_eq!(last_tool(&app).tool_completion(), ToolCompletion::Completed);
        assert!(!last_tool(&app).is_running);
    }
}

#[test]
fn explicit_viewer_adoption_does_not_readmit_a_superseded_viewer_prompt() {
    let mut app = make_app_with_agent("viewer-admission");
    for (seq, prompt) in [(10, "external-A"), (11, "external-B")] {
        acp_event(
            &mut app,
            "viewer-admission",
            text_chunk(TextFragment::Message, prompt),
            serde_json::json!({
                "promptId": prompt, "eventId": format!("viewer-admission-{seq}"),
            }),
        );
        assert!(test_agent(&app, AgentId(0)).attached_as_viewer);
        assert_eq!(
            test_agent(&app, AgentId(0))
                .session
                .current_prompt_id
                .as_deref(),
            Some(prompt)
        );
    }
    let activity = test_agent(&app, AgentId(0)).last_active_at;
    acp_event(
        &mut app,
        "viewer-admission",
        text_chunk(TextFragment::Message, "late A"),
        serde_json::json!({
            "promptId": "external-A", "eventId": "viewer-admission-100", "turnStartMs": 1,
        }),
    );
    assert_eq!(
        test_agent(&app, AgentId(0)).last_applied_event_seq,
        Some(11)
    );
    assert_eq!(test_agent(&app, AgentId(0)).last_active_at, activity);
    acp_event(
        &mut app,
        "viewer-admission",
        text_chunk(TextFragment::Message, "valid B"),
        serde_json::json!({
            "promptId": "external-B", "eventId": "viewer-admission-12",
        }),
    );
    assert_eq!(
        test_agent(&app, AgentId(0)).last_applied_event_seq,
        Some(12)
    );
    assert_eq!(
        test_agent(&app, AgentId(0))
            .session
            .current_prompt_id
            .as_deref(),
        Some("external-B")
    );
}

#[test]
fn pending_viewer_adoption_buffers_once_without_committing_current_turn_metadata() {
    let (mut app, current) = root_turn("staged-admission");
    app.pending_running_adoptions.insert(
        AgentId(0),
        PendingRunningAdoption {
            prompt_id: "next-external".into(),
            text: None,
            combined_texts: None,
            kind: "prompt".into(),
            turn_ended: false,
        },
    );
    let view = test_agent(&app, AgentId(0));
    let ack = view.prompt_ack.clone();
    let activity = view.last_active_at;
    let clock = view.turn_started_at;
    let anchor = view.turn_start_ms;
    for _ in 0..2 {
        acp_event(
            &mut app,
            "staged-admission",
            text_chunk(TextFragment::Message, "buffered next output"),
            serde_json::json!({"promptId": "next-external", "eventId": "staged-admission-100",
                "turnStartMs": 1, "totalTokens": 999}),
        );
        let view = test_agent(&app, AgentId(0));
        assert_eq!(view.pending_adoption_updates.len(), 1);
        assert_eq!(view.last_applied_event_seq, None);
        assert_eq!(view.prompt_ack, ack);
        assert_eq!(view.last_active_at, activity);
        assert_eq!(view.turn_started_at, clock);
        assert_eq!(view.turn_start_ms, anchor);
        assert_eq!(view.session.current_prompt_id.as_ref(), Some(&current));
        assert!(view.context_state.is_none());
        assert!(!view.attached_as_viewer);
    }
}
