use super::*;
use crate::acp::tracker::TurnActivity;
use crate::app::actions::Action;
use crate::scrollback::types::ToolCompletion;
use serde_json::json;
use xai_grok_shell::extensions::notification::{RetryState, SessionNotification};
use xai_grok_shell::extensions::sampling_lifecycle::{
    RequestAttemptId, RetryDeadlineProjection, SamplingLifecycle, SamplingOutcome, SamplingOwner,
    SamplingPhase,
};

const REQUEST_A: &str = "1c3cbaf4-4404-4798-87b9-291defb4aa18";
const REQUEST_B: &str = "0129ea38-66ba-429d-a8c2-c55f78bbcc55";

fn owner(prompt: &str, request: &str, attempt: u64) -> SamplingOwner {
    SamplingOwner {
        prompt_id: prompt.into(),
        request_id: request.into(),
        attempt: RequestAttemptId(attempt),
    }
}

fn unknown_deadline() -> RetryDeadlineProjection {
    serde_json::from_value(json!({
        "clock": {"clock": "unavailable", "reason": "domain_unavailable"},
        "wall_deadline_ms": 0,
    }))
    .unwrap()
}

fn stamp(sid: &str, sequence: u64, owner: &SamplingOwner) -> serde_json::Value {
    json!({"promptId": owner.prompt_id, "requestId": owner.request_id, "streamEpoch": owner.attempt,
        "samplingOwner": owner, "eventId": format!("{sid}-{sequence}")})
}

fn acp_delivery(app: &mut AppView, sid: &str, update: acp::SessionUpdate, meta: serde_json::Value) {
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

fn xai_delivery(app: &mut AppView, sid: &str, update: XaiSessionUpdate, meta: serde_json::Value) {
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp_fixtures::ext_notification(
                "x.ai/session/update",
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

fn phase(app: &mut AppView, sid: &str, sequence: u64, owner: &SamplingOwner, phase: SamplingPhase) {
    let lifecycle = SamplingLifecycle {
        owner: owner.clone(),
        phase,
    };
    let mut meta = stamp(sid, sequence, owner);
    meta.as_object_mut().unwrap().insert(
        "samplingLifecycle".into(),
        serde_json::to_value(&lifecycle).unwrap(),
    );
    xai_delivery(
        app,
        sid,
        XaiSessionUpdate::SamplingLifecycle { lifecycle },
        meta,
    );
}

fn retry(
    app: &mut AppView,
    sid: &str,
    sequence: u64,
    owner: &SamplingOwner,
    display: u32,
    deadline: RetryDeadlineProjection,
) {
    let lifecycle = SamplingLifecycle {
        owner: owner.clone(),
        phase: SamplingPhase::RetryScheduled { deadline },
    };
    let mut meta = stamp(sid, sequence, owner);
    meta.as_object_mut().unwrap().insert(
        "samplingLifecycle".into(),
        serde_json::to_value(&lifecycle).unwrap(),
    );
    xai_delivery(
        app,
        sid,
        XaiSessionUpdate::RetryState(RetryState::Retrying {
            attempt: display,
            max_retries: 5,
            reason: "API error (status 503): overloaded".into(),
            error_type: Some("api".into()),
            next_attempt_at_ms: Some(0),
        }),
        meta,
    );
}

fn fragment(thought: bool, text: &str) -> acp::SessionUpdate {
    let content = acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(text)));
    if thought {
        acp::SessionUpdate::AgentThoughtChunk(content)
    } else {
        acp::SessionUpdate::AgentMessageChunk(content)
    }
}

fn arguments(name: &str) -> XaiSessionUpdate {
    XaiSessionUpdate::ToolCallDeltaChunk {
        tool_call_id: Some("call-lifecycle".into()),
        tool_index: 0,
        name: Some(name.into()),
        arguments_delta: Some("{\"target_file\":".into()),
    }
}

fn target(id: AgentId, inactive: bool) -> (AppView, &'static str, String) {
    let mut app = make_app_viewing_child("sampling-root", "sampling-child");
    app.active_view = ActiveView::Agent(id);
    crate::app::dispatch::dispatch(Action::SendPrompt("repair lifecycle".into()), &mut app);
    let prompt = test_agent(&app, id)
        .session
        .current_prompt_id
        .clone()
        .unwrap();
    if inactive {
        app.active_view = ActiveView::Agent(if id == AgentId(0) {
            AgentId(1)
        } else {
            AgentId(0)
        });
    }
    let sid = if id == AgentId(0) {
        "sampling-root"
    } else {
        "sampling-child"
    };
    (app, sid, prompt)
}

fn activity(app: &AppView, id: AgentId) -> Option<TurnActivity> {
    test_agent(app, id).session.tracker.activity()
}

#[test]
fn retry_retires_old_acp_and_argument_bytes_before_the_first_next_delta_on_every_view() {
    for id in [AgentId(0), AgentId(1)] {
        for inactive in [false, true] {
            for thought in [false, true] {
                let (mut app, sid, prompt) = target(id, inactive);
                let first = owner(&prompt, REQUEST_A, 1);
                let next = owner(&prompt, REQUEST_A, 2);
                phase(
                    &mut app,
                    sid,
                    1,
                    &first,
                    SamplingPhase::DispatchStarted {
                        started_at: Default::default(),
                    },
                );
                acp_delivery(
                    &mut app,
                    sid,
                    fragment(thought, "first output"),
                    stamp(sid, 2, &first),
                );
                xai_delivery(&mut app, sid, arguments("read_file"), stamp(sid, 3, &first));
                retry(&mut app, sid, 4, &next, 1, unknown_deadline());
                let view = test_agent(&app, id);
                let generation = view.scrollback.content_generation();
                let active = view.last_active_at;
                let started = view.turn_started_at;
                let turn_start_ms = view.turn_start_ms;
                let context_used = view.context_state.as_ref().map(|state| state.used);
                let ack = view.prompt_ack.clone();
                let acp_highwater = view.last_applied_event_seq;
                let xai_highwater = view.last_applied_xai_event_seq;
                let retry_phase = activity(&app, id);
                assert!(matches!(
                    retry_phase,
                    Some(TurnActivity::Retrying {
                        attempt: 1,
                        due_at: None,
                        ..
                    })
                ));
                let mut stale = stamp(sid, 100, &first);
                stale.as_object_mut().unwrap().extend(
                    json!({"turnStartMs": 1, "totalTokens": 999})
                        .as_object()
                        .unwrap()
                        .clone(),
                );
                acp_delivery(
                    &mut app,
                    sid,
                    fragment(thought, "discarded old bytes"),
                    stale,
                );
                xai_delivery(
                    &mut app,
                    sid,
                    arguments("discarded_tool"),
                    stamp(sid, 101, &first),
                );
                acp_delivery(
                    &mut app,
                    sid,
                    fragment(thought, "not dispatched yet"),
                    stamp(sid, 102, &next),
                );
                phase(
                    &mut app,
                    sid,
                    103,
                    &first,
                    SamplingPhase::DispatchStarted {
                        started_at: Default::default(),
                    },
                );
                retry(&mut app, sid, 104, &next, 1, unknown_deadline());
                let view = test_agent(&app, id);
                assert_eq!(view.scrollback.content_generation(), generation);
                assert_eq!(view.last_active_at, active);
                assert_eq!(view.turn_started_at, started);
                assert_eq!(view.turn_start_ms, turn_start_ms);
                assert_eq!(
                    view.context_state.as_ref().map(|state| state.used),
                    context_used
                );
                assert_eq!(view.prompt_ack, ack);
                assert_eq!(view.last_applied_event_seq, acp_highwater);
                assert_eq!(view.last_applied_xai_event_seq, xai_highwater);
                assert_eq!(activity(&app, id), retry_phase);
                assert_eq!(
                    view.session.current_prompt_id.as_deref(),
                    Some(prompt.as_str())
                );
                phase(
                    &mut app,
                    sid,
                    5,
                    &next,
                    SamplingPhase::DispatchStarted {
                        started_at: Default::default(),
                    },
                );
                assert!(
                    test_agent(&app, id)
                        .session
                        .tracker
                        .retry_activity()
                        .is_none(),
                    "dispatch, not token delivery, ends backoff"
                );
                assert!(test_agent(&app, id).session.state.is_turn_running());
                let before = Instant::now();
                acp_delivery(
                    &mut app,
                    sid,
                    fragment(thought, "next output"),
                    stamp(sid, 6, &next),
                );
                assert!(test_agent(&app, id).last_active_at.unwrap() >= before);
                xai_delivery(&mut app, sid, arguments("edit_file"), stamp(sid, 7, &next));
                assert!(
                    matches!(activity(&app, id), Some(TurnActivity::WritingToolCall(writing))
                    if writing.tool_name.as_deref() == Some("edit_file") && writing.ordinal.get() == 1)
                );
                let doom = owner(&prompt, REQUEST_A, 3);
                retry(&mut app, sid, 8, &doom, 1, unknown_deadline());
                let doom_retry = activity(&app, id);
                acp_delivery(
                    &mut app,
                    sid,
                    fragment(thought, "discarded second attempt"),
                    stamp(sid, 200, &next),
                );
                xai_delivery(
                    &mut app,
                    sid,
                    arguments("discarded_second_tool"),
                    stamp(sid, 201, &next),
                );
                assert_eq!(activity(&app, id), doom_retry);
                assert_eq!(test_agent(&app, id).last_applied_xai_event_seq, Some(8));
                phase(
                    &mut app,
                    sid,
                    9,
                    &doom,
                    SamplingPhase::DispatchStarted {
                        started_at: Default::default(),
                    },
                );
                xai_delivery(&mut app, sid, arguments("search"), stamp(sid, 10, &doom));
                assert!(
                    matches!(activity(&app, id), Some(TurnActivity::WritingToolCall(writing))
                    if writing.tool_name.as_deref() == Some("search") && writing.ordinal.get() == 1)
                );
            }
        }
    }
}

#[test]
fn lower_display_budget_on_a_new_request_and_foreign_terminals_do_not_steal_retry_ownership() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        phase(
            &mut app,
            sid,
            1,
            &owner(&prompt, REQUEST_A, 1),
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        for attempt in 2..=4 {
            retry(
                &mut app,
                sid,
                attempt * 2,
                &owner(&prompt, REQUEST_A, attempt),
                u32::try_from(attempt - 1).unwrap(),
                unknown_deadline(),
            );
            if attempt < 4 {
                phase(
                    &mut app,
                    sid,
                    attempt * 2 + 1,
                    &owner(&prompt, REQUEST_A, attempt),
                    SamplingPhase::DispatchStarted {
                        started_at: Default::default(),
                    },
                );
            }
        }
        assert!(matches!(
            activity(&app, id),
            Some(TurnActivity::Retrying { attempt: 3, .. })
        ));
        let next = owner(&prompt, REQUEST_B, 1);
        retry(&mut app, sid, 9, &next, 1, unknown_deadline());
        let retry_phase = activity(&app, id);
        assert!(matches!(
            retry_phase,
            Some(TurnActivity::Retrying { attempt: 1, .. })
        ));
        assert!(test_agent(&app, id).session.in_flight_prompt.is_some());
        let first = owner(&prompt, REQUEST_A, 4);
        phase(
            &mut app,
            sid,
            10,
            &first,
            SamplingPhase::Finished {
                outcome: SamplingOutcome::Failed,
            },
        );
        assert_eq!(activity(&app, id), retry_phase);
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::RetryState(RetryState::Exhausted {
                attempts: 3,
                reason: "old request exhausted".into(),
                is_rate_limited: true,
            }),
            json!({"promptId": prompt, "eventId": format!("{sid}-11")}),
        );
        assert_eq!(activity(&app, id), retry_phase);
        assert!(!test_agent(&app, id).session.rate_limited);
        acp_delivery(
            &mut app,
            sid,
            acp::SessionUpdate::ToolCall(
                acp::ToolCall::new("late-provider-tool", "Read file")
                    .kind(acp::ToolKind::Read)
                    .status(acp::ToolCallStatus::InProgress),
            ),
            stamp(sid, 12, &first),
        );
        acp_delivery(
            &mut app,
            sid,
            acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                "late-provider-tool",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
            )),
            stamp(sid, 13, &first),
        );
        assert_eq!(activity(&app, id), retry_phase);
        let view = test_agent(&app, id);
        let tool = (0..view.scrollback.len())
            .filter_map(|index| view.scrollback.get(index))
            .find(|entry| matches!(entry.block, RenderBlock::ToolCall(_)))
            .unwrap();
        assert_eq!(tool.tool_completion(), ToolCompletion::Completed);
        let active = view.last_active_at;
        let in_flight_after_tool = view
            .session
            .in_flight_prompt
            .as_ref()
            .map(|prompt| prompt.text.clone());
        phase(
            &mut app,
            sid,
            100,
            &first,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert_eq!(test_agent(&app, id).last_applied_xai_event_seq, Some(11));
        assert_eq!(test_agent(&app, id).last_active_at, active);
        phase(
            &mut app,
            sid,
            14,
            &next,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
        assert_eq!(
            test_agent(&app, id)
                .session
                .in_flight_prompt
                .as_ref()
                .map(|prompt| prompt.text.clone()),
            in_flight_after_tool
        );
        assert!(test_agent(&app, id).session.state.is_turn_running());
        xai_delivery(&mut app, sid, arguments("read_file"), stamp(sid, 15, &next));
        assert!(matches!(
            activity(&app, id),
            Some(TurnActivity::WritingToolCall(_))
        ));
    }
}

#[test]
fn only_the_reserved_owner_can_dispatch_or_cancel_a_tokenless_retry() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let reserved = owner(&prompt, REQUEST_A, 2);
        retry(&mut app, sid, 1, &reserved, 1, unknown_deadline());
        let projection = activity(&app, id);
        let view = test_agent(&app, id);
        let in_flight = view.session.in_flight_prompt.as_ref().unwrap().text.clone();
        let turn = view.turn_started_at;
        let foreign = owner(&prompt, REQUEST_B, 1);
        phase(
            &mut app,
            sid,
            2,
            &foreign,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        phase(
            &mut app,
            sid,
            3,
            &foreign,
            SamplingPhase::Finished {
                outcome: SamplingOutcome::Completed,
            },
        );
        assert_eq!(activity(&app, id), projection);
        let lifecycle = SamplingLifecycle {
            owner: reserved.clone(),
            phase: SamplingPhase::Finished {
                outcome: SamplingOutcome::Cancelled,
            },
        };
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::SamplingLifecycle { lifecycle },
            json!({"eventId": format!("{sid}-4")}),
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
        assert!(test_agent(&app, id).session.state.is_turn_running());
        assert_eq!(
            test_agent(&app, id)
                .session
                .in_flight_prompt
                .as_ref()
                .unwrap()
                .text,
            in_flight
        );
        assert_eq!(test_agent(&app, id).turn_started_at, turn);
        phase(
            &mut app,
            sid,
            100,
            &reserved,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        acp_delivery(
            &mut app,
            sid,
            fragment(false, "cancelled owner cannot resume"),
            stamp(sid, 101, &reserved),
        );
        assert_eq!(test_agent(&app, id).last_applied_xai_event_seq, Some(4));
        assert_eq!(test_agent(&app, id).last_applied_event_seq, None);
    }
}

#[test]
fn malformed_provenance_is_not_legacy_and_replay_does_not_settle_live_owners() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let reserved = owner(&prompt, REQUEST_A, 2);
        retry(&mut app, sid, 1, &reserved, 1, unknown_deadline());
        let original = activity(&app, id);
        let in_flight = test_agent(&app, id)
            .session
            .in_flight_prompt
            .as_ref()
            .unwrap()
            .text
            .clone();
        for malformed in [
            json!({"samplingOwner": null}),
            json!({"samplingOwner": {"promptId": prompt, "requestId": REQUEST_A, "attempt": "2"}}),
            json!({"samplingOwner": {"promptId": prompt, "requestId": REQUEST_A, "attempt": 0}}),
            json!({"samplingOwner": reserved, "requestId": REQUEST_B}),
            json!({"samplingOwner": reserved, "samplingLifecycle": {"owner": reserved, "phase": "retry_scheduled", "deadline": {"clock": "broken"}}}),
        ] {
            let mut meta = malformed.as_object().unwrap().clone();
            meta.insert("eventId".into(), json!(format!("{sid}-100")));
            acp_delivery(
                &mut app,
                sid,
                fragment(false, "malformed"),
                serde_json::Value::Object(meta.clone()),
            );
            xai_delivery(
                &mut app,
                sid,
                arguments("malformed"),
                serde_json::Value::Object(meta.clone()),
            );
            xai_delivery(
                &mut app,
                sid,
                XaiSessionUpdate::RetryState(RetryState::Failed {
                    error_type: "encrypted_content_mismatch".into(),
                    message: "malformed".into(),
                }),
                serde_json::Value::Object(meta),
            );
            assert_eq!(activity(&app, id), original);
            assert_eq!(test_agent(&app, id).last_applied_xai_event_seq, Some(1));
            assert_eq!(test_agent(&app, id).last_applied_event_seq, None);
            assert!(!test_agent(&app, id).session.model_incompatible);
        }
        app.agents.get_mut(&id).unwrap().arm_late_replay_grace();
        let historical = SamplingLifecycle {
            owner: reserved.clone(),
            phase: SamplingPhase::Finished {
                outcome: SamplingOutcome::Cancelled,
            },
        };
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::SamplingLifecycle {
                lifecycle: historical,
            },
            json!({"isReplay": true, "eventId": format!("{sid}-200")}),
        );
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::RetryState(RetryState::Exhausted {
                attempts: 4,
                reason: "historical overload".into(),
                is_rate_limited: true,
            }),
            json!({"isReplay": true, "eventId": format!("{sid}-201")}),
        );
        assert_eq!(activity(&app, id), original);
        assert!(!test_agent(&app, id).session.rate_limited);
        assert_eq!(
            test_agent(&app, id)
                .session
                .in_flight_prompt
                .as_ref()
                .unwrap()
                .text,
            in_flight
        );
        assert_eq!(test_agent(&app, id).last_applied_xai_event_seq, Some(1));
        let mismatch = SamplingLifecycle {
            owner: reserved.clone(),
            phase: SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        };
        let mut meta = stamp(sid, 300, &owner(&prompt, REQUEST_B, 1));
        meta.as_object_mut().unwrap().insert(
            "samplingLifecycle".into(),
            serde_json::to_value(&mismatch).unwrap(),
        );
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::SamplingLifecycle {
                lifecycle: mismatch,
            },
            meta,
        );
        assert_eq!(activity(&app, id), original);
        phase(
            &mut app,
            sid,
            2,
            &reserved,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
    }
}

#[test]
fn legacy_bytes_cannot_clear_a_correlated_retry_or_invent_its_next_owner() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let reserved = owner(&prompt, REQUEST_A, 2);
        retry(&mut app, sid, 1, &reserved, 1, unknown_deadline());
        let projection = activity(&app, id);
        acp_delivery(
            &mut app,
            sid,
            fragment(false, "legacy history has no scheduler evidence"),
            json!({"promptId": prompt}),
        );
        xai_delivery(
            &mut app,
            sid,
            arguments("read_file"),
            json!({"promptId": prompt, "eventId": format!("{sid}-2")}),
        );
        assert_eq!(activity(&app, id), projection);
        phase(
            &mut app,
            sid,
            3,
            &reserved,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
    }
}

#[test]
fn reconnect_stash_remains_sampling_admission_authority_before_the_first_live_delta() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let first = owner(&prompt, REQUEST_A, 1);
        let reserved = owner(&prompt, REQUEST_A, 2);
        phase(
            &mut app,
            sid,
            1,
            &first,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        retry(&mut app, sid, 2, &reserved, 1, unknown_deadline());
        app.agents.get_mut(&id).unwrap().begin_session_reload(17);
        let generation = test_agent(&app, id).scrollback.content_generation();
        let active = test_agent(&app, id).last_active_at;
        let highwater = test_agent(&app, id).last_applied_event_seq;
        acp_delivery(
            &mut app,
            sid,
            fragment(false, "retired bytes during reconnect"),
            stamp(sid, 100, &first),
        );
        assert_eq!(
            test_agent(&app, id).scrollback.content_generation(),
            generation
        );
        assert_eq!(test_agent(&app, id).last_active_at, active);
        assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
        phase(
            &mut app,
            sid,
            3,
            &reserved,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
        xai_delivery(
            &mut app,
            sid,
            arguments("read_file"),
            stamp(sid, 4, &reserved),
        );
        assert!(matches!(
            activity(&app, id),
            Some(TurnActivity::WritingToolCall(_))
        ));
    }
}

#[test]
fn late_history_after_local_live_work_does_not_replace_the_current_model_owner() {
    for id in [AgentId(0), AgentId(1)] {
        for thought in [false, true] {
            let (mut app, sid, prompt) = target(id, true);
            let current = owner(&prompt, REQUEST_A, 1);
            phase(
                &mut app,
                sid,
                1,
                &current,
                SamplingPhase::DispatchStarted {
                    started_at: Default::default(),
                },
            );
            acp_delivery(
                &mut app,
                sid,
                fragment(thought, "current output"),
                stamp(sid, 2, &current),
            );
            let projection = activity(&app, id);
            assert_eq!(
                projection,
                Some(if thought {
                    TurnActivity::Thinking
                } else {
                    TurnActivity::Responding
                })
            );
            let active = test_agent(&app, id).last_active_at;
            let highwater = test_agent(&app, id).last_applied_event_seq;
            app.agents.get_mut(&id).unwrap().arm_late_replay_grace();
            let mut historical = stamp(sid, 100, &owner("historical-prompt", REQUEST_B, 1));
            historical
                .as_object_mut()
                .unwrap()
                .insert("isReplay".into(), json!(true));
            acp_delivery(
                &mut app,
                sid,
                fragment(!thought, "late history"),
                historical,
            );
            assert_eq!(activity(&app, id), projection);
            assert_eq!(test_agent(&app, id).last_active_at, active);
            assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
            phase(
                &mut app,
                sid,
                3,
                &current,
                SamplingPhase::Finished {
                    outcome: SamplingOutcome::Completed,
                },
            );
            assert_eq!(activity(&app, id), None);
        }
    }
}

#[test]
fn live_retry_and_retired_attempts_survive_actual_loaded_task_after_interleaved_history() {
    use crate::app::actions::TaskResult;
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let view = app.agents.get_mut(&id).unwrap();
        let started = view.turn_started_at;
        view.begin_replay_window();
        let historical = owner("historical-prompt", REQUEST_B, 1);
        let mut history_meta = stamp(sid, 1, &historical);
        history_meta
            .as_object_mut()
            .unwrap()
            .insert("isReplay".into(), json!(true));
        history_meta
            .as_object_mut()
            .unwrap()
            .insert("turnStartMs".into(), json!(1));
        acp_delivery(
            &mut app,
            sid,
            fragment(false, "historical response"),
            history_meta.clone(),
        );
        assert_eq!(test_agent(&app, id).turn_started_at, started);
        let reserved = owner(&prompt, REQUEST_A, 2);
        retry(&mut app, sid, 2, &reserved, 1, unknown_deadline());
        let projection = activity(&app, id);
        let active = test_agent(&app, id).last_active_at;
        acp_delivery(
            &mut app,
            sid,
            fragment(true, "historical thought"),
            history_meta,
        );
        xai_delivery(
            &mut app,
            sid,
            XaiSessionUpdate::RetryState(RetryState::Exhausted {
                attempts: 4,
                reason: "historical failure".into(),
                is_rate_limited: true,
            }),
            json!({"isReplay": true}),
        );
        crate::app::dispatch::dispatch(
            Action::TaskComplete(TaskResult::SessionLoaded {
                agent_id: id,
                session_id: acp::SessionId::new(sid),
                models: None,
                modes: None,
                code_restored: false,
                restore_summary: None,
                restore_degree: None,
                running_prompt_id: Some(prompt.clone()),
            }),
            &mut app,
        );
        assert_eq!(activity(&app, id), projection);
        assert_eq!(test_agent(&app, id).last_active_at, active);
        assert_eq!(test_agent(&app, id).turn_started_at, started);
        assert_eq!(
            test_agent(&app, id).session.current_prompt_id.as_ref(),
            Some(&prompt)
        );
        assert!(!test_agent(&app, id).session.rate_limited);
        assert!(test_agent(&app, id).session.state.is_turn_running());
        let generation = test_agent(&app, id).scrollback.content_generation();
        acp_delivery(
            &mut app,
            sid,
            fragment(false, "retired before load finished"),
            stamp(sid, 100, &owner(&prompt, REQUEST_A, 1)),
        );
        assert_eq!(
            test_agent(&app, id).scrollback.content_generation(),
            generation
        );
        phase(
            &mut app,
            sid,
            3,
            &reserved,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
        assert!(test_agent(&app, id).session.state.is_turn_running());
    }
}

#[cfg(target_os = "linux")]
fn native_deadline_after(seconds: i64) -> RetryDeadlineProjection {
    use std::os::unix::fs::MetadataExt;
    let namespace = std::fs::metadata("/proc/thread-self/ns/time").unwrap();
    let offsets = std::fs::read_to_string("/proc/thread-self/timens_offsets").unwrap();
    let fields: Vec<_> = offsets
        .lines()
        .find(|line| line.split_whitespace().next() == Some("monotonic"))
        .unwrap()
        .split_whitespace()
        .collect();
    let offset =
        fields[1].parse::<i64>().unwrap() * 1_000_000_000 + fields[2].parse::<i64>().unwrap();
    let now = nix::time::clock_gettime(nix::time::ClockId::CLOCK_MONOTONIC).unwrap();
    let deadline = i128::from(now.tv_sec()) * 1_000_000_000
        + i128::from(now.tv_nsec())
        + i128::from(seconds) * 1_000_000_000;
    let deadline = i64::try_from(deadline).unwrap();
    serde_json::from_value(json!({
        "clock": {"clock": "linux_monotonic", "domain": {
            "boot_id": std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim(),
            "namespace_device": namespace.dev(), "namespace_inode": namespace.ino(), "monotonic_offset_ns": offset,
        }, "earliest_ns": deadline, "latest_ns": deadline},
        "wall_deadline_ms": 0,
    })).unwrap()
}

fn screen(app: &mut AppView, id: AgentId) -> String {
    let area = ratatui::layout::Rect::new(0, 0, 120, 35);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    app.agents.get_mut(&id).unwrap().draw(
        area,
        &mut buffer,
        &crate::actions::ActionRegistry::defaults(),
        &mut crate::scrollback::render::ScratchBuffer::new(),
        None,
        false,
        crate::app::agent_view::BannerSlotParams::none(),
        false,
        &mut Vec::new(),
        crate::app::agent_view::AppRenderParams::default(),
    );
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

#[cfg(target_os = "linux")]
#[test]
fn twenty_minute_native_clock_and_due_display_are_not_dispatch_observations() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let reserved = owner(&prompt, REQUEST_A, 2);
        app.agents.get_mut(&id).unwrap().begin_replay_window();
        let before = Instant::now();
        retry(&mut app, sid, 1, &reserved, 1, native_deadline_after(1200));
        let Some(TurnActivity::Retrying {
            due_at: Some(due), ..
        }) = activity(&app, id)
        else {
            panic!("same-host deadline must correlate");
        };
        assert!(due.duration_since(before) >= std::time::Duration::from_secs(1190));
        assert!(
            due.saturating_duration_since(Instant::now()) < std::time::Duration::from_secs(1201)
        );
        let original_projection = activity(&app, id);
        crate::app::dispatch::dispatch(
            Action::TaskComplete(crate::app::actions::TaskResult::SessionLoaded {
                agent_id: id,
                session_id: acp::SessionId::new(sid),
                models: None,
                modes: None,
                code_restored: false,
                restore_summary: None,
                restore_degree: None,
                running_prompt_id: Some(prompt.clone()),
            }),
            &mut app,
        );
        assert_eq!(activity(&app, id), original_projection);
        assert!(!screen(&mut app, id).contains("attempt due"));
        let due_owner = owner(&prompt, REQUEST_A, 3);
        retry(&mut app, sid, 2, &due_owner, 2, native_deadline_after(-5));
        assert!(screen(&mut app, id).contains("attempt due"));
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_some()
        );
        assert!(test_agent(&app, id).session.in_flight_prompt.is_some());
        assert!(test_agent(&app, id).session.state.is_turn_running());
        phase(
            &mut app,
            sid,
            3,
            &due_owner,
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert!(
            test_agent(&app, id)
                .session
                .tracker
                .retry_activity()
                .is_none()
        );
        assert_eq!(activity(&app, id), None);
    }
}

#[test]
fn unknown_native_clock_stays_unknown_even_when_wall_projection_is_past() {
    let (mut app, sid, prompt) = target(AgentId(0), false);
    retry(
        &mut app,
        sid,
        1,
        &owner(&prompt, REQUEST_A, 2),
        1,
        unknown_deadline(),
    );
    assert!(matches!(
        activity(&app, AgentId(0)),
        Some(TurnActivity::Retrying { due_at: None, .. })
    ));
    let rendered = screen(&mut app, AgentId(0));
    assert!(!rendered.contains("attempt due"));
    assert!(!rendered.contains("next try in"));
    assert!(test_agent(&app, AgentId(0)).session.state.is_turn_running());
}

#[test]
fn usage_observation_does_not_suppress_authoritative_loaded_prompt_adoption() {
    for id in [AgentId(0), AgentId(1)] {
        let mut app = make_app_viewing_child("observation-root", "observation-child");
        app.active_view = ActiveView::Agent(if id == AgentId(0) {
            AgentId(1)
        } else {
            AgentId(0)
        });
        let sid = if id == AgentId(0) {
            "observation-root"
        } else {
            "observation-child"
        };
        app.agents.get_mut(&id).unwrap().begin_replay_window();
        acp_delivery(
            &mut app,
            sid,
            acp::SessionUpdate::UsageUpdate(acp::UsageUpdate::new(42, 1000)),
            json!({"eventId": format!("{sid}-1")}),
        );
        assert!(test_agent(&app, id).session.state.is_idle());
        crate::app::dispatch::dispatch(
            Action::TaskComplete(crate::app::actions::TaskResult::SessionLoaded {
                agent_id: id,
                session_id: acp::SessionId::new(sid),
                models: None,
                modes: None,
                code_restored: false,
                restore_summary: None,
                restore_degree: None,
                running_prompt_id: Some("observed-running-prompt".into()),
            }),
            &mut app,
        );
        assert_eq!(
            test_agent(&app, id).session.current_prompt_id.as_deref(),
            Some("observed-running-prompt")
        );
        assert!(test_agent(&app, id).session.state.is_turn_running());
    }
}
