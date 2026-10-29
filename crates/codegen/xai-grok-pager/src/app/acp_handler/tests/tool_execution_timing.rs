use super::*;
use crate::acp::tool_clock::ToolExecutionClock;
use crate::app::actions::{Action, TaskResult};
use crate::scrollback::entry::{EntryId, ScrollbackEntry};
use crate::scrollback::types::ToolCompletion;
use serde_json::json;
use std::time::{Duration, Instant};
use xai_grok_shell::extensions::notification::{RetryState, SessionNotification};
use xai_grok_shell::extensions::sampling_lifecycle::{
    RequestAttemptId, RetryDeadlineProjection, SamplingLifecycle, SamplingOwner, SamplingPhase,
};
use xai_grok_shell::extensions::tool_timing::{
    LocalClockMapping, MonotonicPoint, ToolCallOwnership, ToolCallSource, ToolExecutionId,
    ToolExecutionTermination, ToolExecutionTiming, stamp_ownership, timing_meta,
};

const REQUEST: &str = "00000000-0000-4000-8000-000000000001";

fn target(id: AgentId, inactive: bool) -> (AppView, &'static str, String) {
    let mut app = make_app_viewing_child("execution-root", "execution-child");
    app.active_view = ActiveView::Agent(id);
    crate::app::dispatch::dispatch(
        Action::SendPrompt("measure actual execution".into()),
        &mut app,
    );
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
    (
        app,
        if id == AgentId(0) {
            "execution-root"
        } else {
            "execution-child"
        },
        prompt,
    )
}

fn call_owner(prompt: &str, invocation: u128) -> ToolCallOwnership {
    ToolCallOwnership::Registered {
        invocation_id: uuid::Uuid::from_u128(invocation).to_string(),
        provider_call_id: "reused-provider".into(),
        source: ToolCallSource::Sampling {
            prompt_id: prompt.into(),
            request_id: REQUEST.into(),
            attempt: 1,
        },
    }
}

fn inner(owner: &ToolCallOwnership, timing: ToolExecutionTiming) -> acp::Meta {
    let mut meta = timing_meta(timing).unwrap();
    stamp_ownership(&mut meta, owner).unwrap();
    meta
}

fn provider_id(owner: &ToolCallOwnership) -> acp::ToolCallId {
    match owner {
        ToolCallOwnership::Registered {
            provider_call_id, ..
        } => acp::ToolCallId::new(provider_call_id.clone()),
        ToolCallOwnership::Unknown => panic!("registered fixture owner required"),
    }
}

fn base(owner: &ToolCallOwnership) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(provider_id(owner), "run_terminal_command")
            .status(acp::ToolCallStatus::Pending)
            .kind(acp::ToolKind::Other)
            .meta(Some(inner(owner, ToolExecutionTiming::Unknown))),
    )
}

fn update(
    owner: &ToolCallOwnership,
    timing: ToolExecutionTiming,
    fields: acp::ToolCallUpdateFields,
) -> acp::SessionUpdate {
    acp::SessionUpdate::ToolCallUpdate(
        acp::ToolCallUpdate::new(provider_id(owner), fields).meta(Some(inner(owner, timing))),
    )
}

fn deliver(
    app: &mut AppView,
    sid: &str,
    prompt: &str,
    sequence: u64,
    update: acp::SessionUpdate,
    replay: bool,
) {
    deliver_meta(
        app,
        sid,
        update,
        json!({"promptId": prompt, "requestId": REQUEST, "streamEpoch": 1,
        "eventId": format!("{sid}-{sequence}"), "isReplay": replay}),
    );
}

fn stamp_fixture_delivery(app: &AppView, sid: &str, meta: &mut serde_json::Value) {
    if let Some(id) = app.agents.find_by_session_id(sid)
        && let Some(context) = app
            .agents
            .get(&id)
            .and_then(|view| view.current_actions_context())
    {
        use xai_grok_shell::extensions::current_actions::{
            DeliveryRole, PublicationContext, stamp_delivery,
        };
        let role = if meta["isReplay"].as_bool() == Some(true) {
            DeliveryRole::Historical {
                binding: context.binding.clone(),
            }
        } else {
            DeliveryRole::Live {
                context: PublicationContext {
                    binding: context.binding.clone(),
                    incarnation: context.incarnation,
                },
            }
        };
        stamp_delivery(meta.as_object_mut().unwrap(), &role).unwrap();
    }
}

fn deliver_meta(
    app: &mut AppView,
    sid: &str,
    update: acp::SessionUpdate,
    mut meta: serde_json::Value,
) {
    stamp_fixture_delivery(app, sid, &mut meta);
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

fn last_entry(app: &AppView, id: AgentId) -> &ScrollbackEntry {
    let sb = &test_agent(app, id).scrollback;
    (0..sb.len())
        .rev()
        .filter_map(|i| sb.get(i))
        .find(|entry| matches!(entry.block, RenderBlock::ToolCall(_)))
        .unwrap()
}

fn clock(app: &AppView, id: AgentId, entry: EntryId) -> ToolExecutionClock {
    match &test_agent(app, id)
        .scrollback
        .get_by_id(entry)
        .unwrap()
        .block
    {
        RenderBlock::ToolCall(tool) => tool.execution_clock().clone(),
        _ => panic!("expected tool projection"),
    }
}

fn finished(
    id: ToolExecutionId,
    start: MonotonicPoint,
    duration_ms: u64,
    termination: ToolExecutionTermination,
) -> ToolExecutionTiming {
    ToolExecutionTiming::Finished {
        execution_id: id,
        started_at: start,
        duration_ms,
        termination,
    }
}

fn finish_turn(app: &mut AppView, id: AgentId, prompt: &str) {
    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::PromptResponse {
            agent_id: id,
            prompt_id: Some(prompt.into()),
            http_status: None,
            result: Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)),
        }),
        app,
    );
}

fn render(app: &mut AppView, id: AgentId) -> String {
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

#[test]
fn permission_thirty_seconds_execution_two_seconds_and_postflight_use_one_clock_on_every_view() {
    for id in [AgentId(0), AgentId(1)] {
        for inactive in [false, true] {
            let (mut app, sid, prompt) = target(id, inactive);
            let owner = call_owner(&prompt, 1);
            deliver_meta(
                &mut app,
                sid,
                base(&owner),
                json!({"promptId": prompt,
                "eventId": format!("{sid}-1"), "agentTimestampMs": 32000}),
            );
            let entry = last_entry(&app, id).id;
            let permission_start = Instant::now() - Duration::from_secs(32);
            assert_eq!(
                clock(&app, id, entry).elapsed_at(permission_start + Duration::from_secs(30)),
                None,
                "thirty seconds of permission/resource waiting are not execution"
            );
            let turn_anchor = test_agent(&app, id).turn_started_at;
            let actual_start = Instant::now() - Duration::from_secs(2);
            let point = MonotonicPoint::at(tokio::time::Instant::from_std(actual_start));
            let execution = ToolExecutionId::generate();
            deliver(
                &mut app,
                sid,
                &prompt,
                2,
                update(
                    &owner,
                    ToolExecutionTiming::Started {
                        execution_id: execution,
                        started_at: point,
                    },
                    Default::default(),
                ),
                false,
            );
            let mapped = clock(&app, id, entry).mapping(execution).unwrap();
            let LocalClockMapping::Correlated {
                earliest_at,
                latest_at,
            } = mapped
            else {
                panic!("same host must correlate");
            };
            assert!(earliest_at.into_std() <= actual_start && actual_start <= latest_at.into_std());
            let projection_at = latest_at.into_std() + Duration::from_secs(2);
            assert_eq!(clock(&app, id, entry).elapsed_at(projection_at), Some(2000));
            deliver(
                &mut app,
                sid,
                &prompt,
                3,
                update(
                    &owner,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new()
                        .kind(Some(acp::ToolKind::Execute))
                        .raw_input(Some(
                            json!({"variant": "Bash", "command": "printf measured"}),
                        )),
                ),
                false,
            );
            assert_eq!(
                last_entry(&app, id).id,
                entry,
                "kind refinement retains the real clock and copy identity"
            );
            assert_eq!(clock(&app, id, entry).mapping(execution), Some(mapped));
            deliver(
                &mut app,
                sid,
                &prompt,
                4,
                update(
                    &owner,
                    finished(execution, point, 2000, ToolExecutionTermination::Returned),
                    Default::default(),
                ),
                false,
            );
            assert_eq!(
                clock(&app, id, entry).elapsed_at(projection_at + Duration::from_secs(30)),
                Some(2000)
            );
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Unconfirmed
            );
            assert!(
                last_entry(&app, id).is_running,
                "postflight still owns canonical pending work"
            );
            assert!(matches!(
                test_agent(&app, id).session.tracker.activity(),
                Some(crate::acp::tracker::TurnActivity::ToolRunning { .. })
            ));
            let raw_postflight_done = xai_grok_tools::types::output::ToolOutput::Bash(
                xai_grok_tools::types::output::BashOutput {
                    output: b"measured".to_vec(),
                    output_for_prompt: "measured".into(),
                    exit_code: 0,
                    command: "printf measured".into(),
                    truncated: false,
                    signal: None,
                    timed_out: false,
                    description: None,
                    current_dir: String::new(),
                    output_file: String::new(),
                    total_bytes: 8,
                    output_delta: None,
                    was_bare_echo: false,
                },
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                5,
                update(
                    &owner,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new()
                        .raw_output(Some(serde_json::to_value(raw_postflight_done).unwrap())),
                ),
                false,
            );
            assert_eq!(
                clock(&app, id, entry).elapsed_ms(),
                Some(2000),
                "postflight output cannot restart or replace execution timing"
            );
            assert_eq!(test_agent(&app, id).turn_started_at, turn_anchor);
            app.active_view = ActiveView::Agent(id);
            assert!(render(&mut app, id).contains("printf measured"));
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            deliver(
                &mut app,
                sid,
                &prompt,
                6,
                update(
                    &owner,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                ),
                false,
            );
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Completed
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            assert_eq!(
                test_agent(&app, id)
                    .session
                    .tracker
                    .context_tool_call_id_for_entry(entry)
                    .unwrap()
                    .as_str(),
                "reused-provider"
            );
        }
    }
}

#[test]
fn authentication_attempts_sum_only_actual_spans_and_old_terminals_cannot_reset_the_current_attempt()
 {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let owner = call_owner(&prompt, 2);
        deliver(&mut app, sid, &prompt, 1, base(&owner), false);
        let entry = last_entry(&app, id).id;
        let first = ToolExecutionId::generate();
        let second = ToolExecutionId::generate();
        let first_point = MonotonicPoint::at(tokio::time::Instant::from_std(
            Instant::now() - Duration::from_secs(3),
        ));
        let second_point = MonotonicPoint::at(tokio::time::Instant::now());
        let first_end = finished(first, first_point, 1000, ToolExecutionTermination::Returned);
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            update(&owner, first_end, Default::default()),
            false,
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            3,
            update(
                &owner,
                ToolExecutionTiming::Started {
                    execution_id: second,
                    started_at: second_point,
                },
                Default::default(),
            ),
            false,
        );
        let mapped = clock(&app, id, entry).mapping(second).unwrap();
        let LocalClockMapping::Correlated { latest_at, .. } = mapped else {
            panic!("expected actual shared mapping");
        };
        let at = latest_at.into_std() + Duration::from_secs(5);
        assert_eq!(clock(&app, id, entry).elapsed_at(at), Some(6000));
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            update(&owner, first_end, Default::default()),
            false,
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            4,
            update(
                &owner,
                ToolExecutionTiming::Started {
                    execution_id: first,
                    started_at: first_point,
                },
                Default::default(),
            ),
            false,
        );
        assert_eq!(clock(&app, id, entry).mapping(second), Some(mapped));
        assert_eq!(clock(&app, id, entry).elapsed_at(at), Some(6000));
        deliver(
            &mut app,
            sid,
            &prompt,
            5,
            update(
                &owner,
                finished(
                    second,
                    second_point,
                    2000,
                    ToolExecutionTermination::Returned,
                ),
                Default::default(),
            ),
            false,
        );
        assert_eq!(clock(&app, id, entry).elapsed_at(at), Some(3000));
        deliver(
            &mut app,
            sid,
            &prompt,
            900,
            update(
                &owner,
                finished(first, first_point, 9999, ToolExecutionTermination::Returned),
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            clock(&app, id, entry).elapsed_at(at),
            Some(3000),
            "conflicting duplicate is rejected, not accumulated"
        );
        assert_eq!(
            test_agent(&app, id).last_applied_event_seq,
            Some(1),
            "timing rail owns no generic highwater"
        );
    }
}

#[test]
fn two_turns_reusing_a_provider_id_keep_old_timing_and_canonical_outcomes_off_the_new_call_and_retry()
 {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, first_prompt) = target(id, false);
        let first_owner = call_owner(&first_prompt, 3);
        let first_execution = ToolExecutionId::generate();
        let point = MonotonicPoint::at(tokio::time::Instant::now());
        deliver(&mut app, sid, &first_prompt, 1, base(&first_owner), false);
        let old_entry = last_entry(&app, id).id;
        deliver(
            &mut app,
            sid,
            &first_prompt,
            2,
            update(
                &first_owner,
                ToolExecutionTiming::Started {
                    execution_id: first_execution,
                    started_at: point,
                },
                Default::default(),
            ),
            false,
        );
        let cancel = acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "reused-provider",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Failed)),
            )
            .meta(Some(
                json!({"toolTerminalStatus":"cancelled"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )),
        );
        deliver(&mut app, sid, &first_prompt, 3, cancel, false);
        finish_turn(&mut app, id, &first_prompt);
        assert_eq!(
            clock(&app, id, old_entry).elapsed_ms(),
            None,
            "missing terminal timing is not completion-receipt duration"
        );
        crate::app::dispatch::dispatch(Action::SendPrompt("next real turn".into()), &mut app);
        let second_prompt = test_agent(&app, id)
            .session
            .current_prompt_id
            .clone()
            .unwrap();
        assert_ne!(first_prompt, second_prompt);
        let second_owner = call_owner(&second_prompt, 4);
        deliver(&mut app, sid, &second_prompt, 4, base(&second_owner), false);
        let new_entry = last_entry(&app, id).id;
        let second_execution = ToolExecutionId::generate();
        deliver(
            &mut app,
            sid,
            &second_prompt,
            5,
            update(
                &second_owner,
                ToolExecutionTiming::Started {
                    execution_id: second_execution,
                    started_at: point,
                },
                Default::default(),
            ),
            false,
        );
        let mapping = clock(&app, id, new_entry).mapping(second_execution);
        let reserved = SamplingOwner {
            prompt_id: second_prompt.clone(),
            request_id: REQUEST.into(),
            attempt: RequestAttemptId(2),
        };
        let deadline: RetryDeadlineProjection = serde_json::from_value(json!({"clock": {"clock":"unavailable", "reason":"domain_unavailable"}, "wall_deadline_ms":0})).unwrap();
        let lifecycle = SamplingLifecycle {
            owner: reserved.clone(),
            phase: SamplingPhase::RetryScheduled { deadline },
        };
        let (response_tx, _response) = tokio::sync::oneshot::channel();
        handle(
            AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
                request: acp_fixtures::ext_notification(
                    "x.ai/session/update",
                    &SessionNotification {
                        session_id: acp::SessionId::new(sid),
                        update: XaiSessionUpdate::RetryState(RetryState::Retrying {
                            attempt: 1,
                            max_retries: 5,
                            reason: "retry fixture".into(),
                            error_type: Some("api".into()),
                            next_attempt_at_ms: None,
                        }),
                        meta: Some(
                            json!({"promptId": second_prompt, "samplingOwner": reserved, "samplingLifecycle": lifecycle, "eventId": format!("{sid}-6")}),
                        ),
                    },
                ),
                response_tx,
            }),
            &mut app,
        );
        let retry = test_agent(&app, id).session.tracker.activity();
        let highwater = test_agent(&app, id).last_applied_event_seq;
        let turn_anchor = test_agent(&app, id).turn_started_at;
        deliver(
            &mut app,
            sid,
            &first_prompt,
            1000,
            update(
                &first_owner,
                finished(
                    first_execution,
                    point,
                    2000,
                    ToolExecutionTermination::Returned,
                ),
                Default::default(),
            ),
            false,
        );
        let old_status = acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "reused-provider",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Failed)),
            )
            .meta(Some(
                json!({"toolTerminalStatus":"cancelled"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )),
        );
        deliver(&mut app, sid, &first_prompt, 1001, old_status, false);
        assert_eq!(clock(&app, id, old_entry).elapsed_ms(), Some(2000));
        assert_eq!(
            test_agent(&app, id)
                .scrollback
                .get_by_id(old_entry)
                .unwrap()
                .tool_completion(),
            ToolCompletion::Cancelled
        );
        assert_eq!(
            test_agent(&app, id)
                .scrollback
                .get_by_id(new_entry)
                .unwrap()
                .tool_completion(),
            ToolCompletion::Unconfirmed
        );
        assert!(
            test_agent(&app, id)
                .scrollback
                .get_by_id(new_entry)
                .unwrap()
                .is_running
        );
        assert_eq!(
            clock(&app, id, new_entry).mapping(second_execution),
            mapping
        );
        assert_eq!(test_agent(&app, id).session.tracker.activity(), retry);
        assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
        assert_eq!(test_agent(&app, id).turn_started_at, turn_anchor);
        assert_eq!(
            test_agent(&app, id).session.current_prompt_id.as_deref(),
            Some(second_prompt.as_str())
        );
    }
}

#[test]
fn terminal_before_base_is_typed_idempotent_and_never_confirms_returned_as_success() {
    for id in [AgentId(0), AgentId(1)] {
        for termination in [
            ToolExecutionTermination::Returned,
            ToolExecutionTermination::DispatchCancelled,
            ToolExecutionTermination::StreamEndedWithoutTerminal,
        ] {
            let (mut app, sid, prompt) = target(id, true);
            let owner = call_owner(&prompt, 5);
            let execution = ToolExecutionId::generate();
            let point = MonotonicPoint::default();
            let terminal = finished(execution, point, 2000, termination);
            deliver(
                &mut app,
                sid,
                &prompt,
                3,
                update(&owner, terminal, Default::default()),
                false,
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                1,
                update(
                    &owner,
                    ToolExecutionTiming::Started {
                        execution_id: execution,
                        started_at: point,
                    },
                    Default::default(),
                ),
                false,
            );
            deliver(&mut app, sid, &prompt, 4, base(&owner), false);
            let entry = last_entry(&app, id).id;
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            assert_eq!(
                clock(&app, id, entry).termination(execution),
                Some(termination)
            );
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Unconfirmed
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                5,
                update(&owner, terminal, Default::default()),
                false,
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            finish_turn(&mut app, id, &prompt);
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Unconfirmed
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            let stats = crate::tool_usage::ToolUsageStats::from_scrollback(
                &test_agent(&app, id).scrollback,
            );
            assert!(matches!(
                &last_entry(&app, id).block,
                RenderBlock::ToolCall(crate::scrollback::blocks::tool::ToolCallBlock::Execute(_))
            ));
            assert_eq!(
                stats.categories[&crate::tool_usage::ToolCategory::Execute].unconfirmed_count(),
                1,
                "returning and draining do not confirm success in the duration consumer"
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                6,
                acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                    "reused-provider",
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::InProgress)),
                )),
                false,
            );
            assert!(
                !clock(&app, id, entry).is_terminal(),
                "late progress is not a canonical terminal proof"
            );
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Unconfirmed
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
        }
    }
}

#[test]
fn canonical_terminal_before_base_survives_turn_drain_and_late_base_cannot_reopen_it() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let owner = call_owner(&prompt, 6);
        let execution = ToolExecutionId::generate();
        deliver(
            &mut app,
            sid,
            &prompt,
            1,
            update(
                &owner,
                finished(
                    execution,
                    Default::default(),
                    2000,
                    ToolExecutionTermination::DispatchCancelled,
                ),
                Default::default(),
            ),
            false,
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                "reused-provider",
                acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Failed)),
            )),
            false,
        );
        finish_turn(&mut app, id, &prompt);
        deliver(&mut app, sid, &prompt, 3, base(&owner), false);
        assert_eq!(
            last_entry(&app, id).tool_completion(),
            ToolCompletion::Failed
        );
        assert!(!last_entry(&app, id).is_running);
        assert_eq!(
            clock(&app, id, last_entry(&app, id).id).elapsed_ms(),
            Some(2000)
        );
    }
}

#[test]
fn replay_unknown_domain_missing_start_and_missing_owner_never_start_a_receipt_clock() {
    for id in [AgentId(0), AgentId(1)] {
        for replay in [false, true] {
            let (mut app, sid, prompt) = target(id, false);
            if replay {
                app.agents.get_mut(&id).unwrap().arm_late_replay_grace();
            }
            let owner = call_owner(&prompt, 7);
            deliver(&mut app, sid, &prompt, 1, base(&owner), replay);
            let entry = last_entry(&app, id).id;
            let execution = ToolExecutionId::generate();
            let point = if replay {
                MonotonicPoint::at(tokio::time::Instant::now())
            } else {
                MonotonicPoint::default()
            };
            deliver(
                &mut app,
                sid,
                &prompt,
                2,
                update(
                    &owner,
                    ToolExecutionTiming::Started {
                        execution_id: execution,
                        started_at: point,
                    },
                    Default::default(),
                ),
                replay,
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), None);
            assert_eq!(
                clock(&app, id, entry).mapping(execution),
                Some(LocalClockMapping::Uncorrelated)
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                3,
                update(
                    &owner,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                ),
                replay,
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), None);
        }
        let (mut app, sid, prompt) = target(id, false);
        let unknown = ToolCallOwnership::Registered {
            invocation_id: uuid::Uuid::from_u128(8).to_string(),
            provider_call_id: "reused-provider".into(),
            source: ToolCallSource::Unknown,
        };
        deliver(&mut app, sid, &prompt, 1, base(&unknown), false);
        let entry = last_entry(&app, id).id;
        let execution = ToolExecutionId::generate();
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            update(
                &unknown,
                ToolExecutionTiming::Started {
                    execution_id: execution,
                    started_at: MonotonicPoint::at(tokio::time::Instant::now()),
                },
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            clock(&app, id, entry).elapsed_ms(),
            None,
            "current prompt does not repair Unknown original source"
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            3,
            update(
                &unknown,
                finished(
                    execution,
                    MonotonicPoint::default(),
                    2000,
                    ToolExecutionTermination::Returned,
                ),
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            clock(&app, id, entry).elapsed_ms(),
            None,
            "conflicting start point is rejected"
        );
    }
}

#[test]
fn malformed_and_conflicting_ownership_are_rejected_before_ack_highwater_activity_or_clock_mutation()
 {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let owner = call_owner(&prompt, 9);
        deliver(&mut app, sid, &prompt, 1, base(&owner), false);
        let entry = last_entry(&app, id).id;
        let activity = test_agent(&app, id).session.tracker.activity();
        let anchor = test_agent(&app, id).turn_started_at;
        let ack = test_agent(&app, id)
            .session
            .in_flight_prompt
            .as_ref()
            .map(|prompt| prompt.text.clone());
        let highwater = test_agent(&app, id).last_applied_event_seq;
        for invalid in [
            json!({"toolCallOwnership":null}),
            json!({"toolExecutionTiming":{"state":"started","executionId":"not-a-uuid"}}),
            json!({"toolExecutionTiming":{"state":"started", "executionId": ToolExecutionId::generate(),
                "startedAt":{"clock":"linux_monotonic", "domain":{"boot_id":uuid::Uuid::from_u128(90),
                    "namespace_device":1,"namespace_inode":1,"monotonic_offset_ns":0}, "earliest_ns":2,"latest_ns":1}}}),
            json!({"toolCallOwnership":{"ownership":"registered","invocationId":uuid::Uuid::nil(),"providerCallId":"reused-provider","source":{"source":"unknown"}}}),
            json!({"toolCallOwnership":{"ownership":"registered","invocationId":uuid::Uuid::from_u128(9),"providerCallId":"different","source":{"source":"unknown"}}}),
            json!({"toolCallOwnership":{"ownership":"registered","invocationId":uuid::Uuid::from_u128(9),"providerCallId":"reused-provider","source":{"source":"unknown"}}}),
        ] {
            let bad = acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new(
                    "reused-provider",
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                )
                .meta(invalid.as_object().cloned()),
            );
            deliver(&mut app, sid, &prompt, 9999, bad, false);
            assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
            assert_eq!(test_agent(&app, id).session.tracker.activity(), activity);
            assert_eq!(test_agent(&app, id).turn_started_at, anchor);
            assert_eq!(
                test_agent(&app, id)
                    .session
                    .in_flight_prompt
                    .as_ref()
                    .map(|prompt| prompt.text.clone()),
                ack
            );
            assert_eq!(
                last_entry(&app, id).tool_completion(),
                ToolCompletion::Unconfirmed
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), None);
        }
    }
}

#[test]
fn legacy_finished_without_real_ownership_and_raw_call_duration_remain_unknown() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let legacy = acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("legacy-provider", "legacy execution")
                .kind(acp::ToolKind::Execute)
                .status(acp::ToolCallStatus::Pending),
        );
        deliver(&mut app, sid, &prompt, 1, legacy, false);
        let entry = last_entry(&app, id).id;
        let legacy_timing = timing_meta(finished(
            ToolExecutionId::generate(),
            Default::default(),
            2000,
            ToolExecutionTermination::Returned,
        ))
        .unwrap();
        let terminal = acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                "legacy-provider",
                acp::ToolCallUpdateFields::new()
                    .status(Some(acp::ToolCallStatus::Completed))
                    .raw_output(Some(json!({"duration_ms":32000}))),
            )
            .meta(Some(legacy_timing)),
        );
        deliver(&mut app, sid, &prompt, 2, terminal, false);
        assert_eq!(
            last_entry(&app, id).tool_completion(),
            ToolCompletion::Completed
        );
        assert_eq!(
            clock(&app, id, entry).elapsed_ms(),
            None,
            "a provider id is not a real invocation owner"
        );
    }
}

#[test]
fn foreign_clock_domain_never_ticks_but_finished_without_a_wire_start_keeps_its_exact_duration() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let owner = call_owner(&prompt, 10);
        deliver(&mut app, sid, &prompt, 1, base(&owner), false);
        let entry = last_entry(&app, id).id;
        let execution = ToolExecutionId::generate();
        let mut point =
            serde_json::to_value(MonotonicPoint::at(tokio::time::Instant::now())).unwrap();
        assert_eq!(point["clock"], "linux_monotonic");
        point["domain"]["boot_id"] = json!(uuid::Uuid::from_u128(991));
        let foreign: MonotonicPoint = serde_json::from_value(point).unwrap();
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            update(
                &owner,
                ToolExecutionTiming::Started {
                    execution_id: execution,
                    started_at: foreign,
                },
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            clock(&app, id, entry).mapping(execution),
            Some(LocalClockMapping::Uncorrelated)
        );
        assert_eq!(clock(&app, id, entry).elapsed_ms(), None);
        deliver(
            &mut app,
            sid,
            &prompt,
            3,
            update(
                &owner,
                finished(execution, foreign, 2000, ToolExecutionTermination::Returned),
                Default::default(),
            ),
            false,
        );
        assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
        let (mut app, sid, prompt) = target(id, false);
        let owner = call_owner(&prompt, 11);
        deliver(&mut app, sid, &prompt, 1, base(&owner), false);
        let entry = last_entry(&app, id).id;
        let mut meta = inner(&owner, ToolExecutionTiming::Unknown);
        meta.insert(
            "toolExecutionTiming".into(),
            json!({"state":"finished", "executionId":ToolExecutionId::generate(),
            "durationMs":2000, "termination":{"kind":"returned"}}),
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new("reused-provider", Default::default()).meta(Some(meta)),
            ),
            false,
        );
        assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
        assert_eq!(
            last_entry(&app, id).tool_completion(),
            ToolCompletion::Unconfirmed
        );
    }
}

#[test]
fn reconnect_admission_reads_the_genuine_stashed_invocation_before_the_first_continuation() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let original = call_owner(&prompt, 12);
        deliver(&mut app, sid, &prompt, 1, base(&original), false);
        let entry = last_entry(&app, id).id;
        let execution = ToolExecutionId::generate();
        let point = MonotonicPoint::at(tokio::time::Instant::now());
        deliver(
            &mut app,
            sid,
            &prompt,
            2,
            update(
                &original,
                ToolExecutionTiming::Started {
                    execution_id: execution,
                    started_at: point,
                },
                Default::default(),
            ),
            false,
        );
        let original_clock = clock(&app, id, entry);
        let mapped = original_clock.mapping(execution);
        app.agents.get_mut(&id).unwrap().begin_session_reload(81);
        let generation = test_agent(&app, id).scrollback.content_generation();
        let highwater = test_agent(&app, id).last_applied_event_seq;
        let active = test_agent(&app, id).last_active_at;
        let conflicting = ToolCallOwnership::Registered {
            invocation_id: uuid::Uuid::from_u128(12).to_string(),
            provider_call_id: "reused-provider".into(),
            source: ToolCallSource::Unknown,
        };
        deliver(
            &mut app,
            sid,
            &prompt,
            999,
            update(
                &conflicting,
                finished(execution, point, 2000, ToolExecutionTermination::Returned),
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            test_agent(&app, id).scrollback.content_generation(),
            generation
        );
        assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
        assert_eq!(test_agent(&app, id).last_active_at, active);
        assert_eq!(original_clock.mapping(execution), mapped);
        assert!(test_agent(&app, id).session.loading_replay);
        deliver(
            &mut app,
            sid,
            &prompt,
            1000,
            update(
                &original,
                finished(execution, point, 2000, ToolExecutionTermination::Returned),
                Default::default(),
            ),
            false,
        );
        assert_eq!(
            original_clock.elapsed_ms(),
            Some(2000),
            "the existing stashed clock settles in place"
        );
        assert_eq!(
            test_agent(&app, id).scrollback.content_generation(),
            generation
        );
        assert_eq!(test_agent(&app, id).last_applied_event_seq, highwater);
        assert_eq!(test_agent(&app, id).last_active_at, active);
        assert!(matches!(
            test_agent(&app, id).load_reconstruction,
            crate::app::agent_view::load_continuation::LoadReconstruction::Historical
        ));
        assert!(test_agent(&app, id).session.loading_replay);
    }
}

#[test]
fn termination_evidence_never_reclassifies_canonical_completed_failed_or_cancelled() {
    for id in [AgentId(0), AgentId(1)] {
        for termination in [
            ToolExecutionTermination::Returned,
            ToolExecutionTermination::DispatchCancelled,
            ToolExecutionTermination::StreamEndedWithoutTerminal,
        ] {
            for (status, cancelled, expected) in [
                (
                    acp::ToolCallStatus::Completed,
                    false,
                    ToolCompletion::Completed,
                ),
                (acp::ToolCallStatus::Failed, false, ToolCompletion::Failed),
                (acp::ToolCallStatus::Failed, true, ToolCompletion::Cancelled),
            ] {
                let (mut app, sid, prompt) = target(id, true);
                let owner = call_owner(&prompt, 13);
                deliver(&mut app, sid, &prompt, 1, base(&owner), false);
                let entry = last_entry(&app, id).id;
                let execution = ToolExecutionId::generate();
                deliver(
                    &mut app,
                    sid,
                    &prompt,
                    2,
                    update(
                        &owner,
                        finished(execution, Default::default(), 2000, termination),
                        Default::default(),
                    ),
                    false,
                );
                assert_eq!(
                    last_entry(&app, id).tool_completion(),
                    ToolCompletion::Unconfirmed
                );
                let mut meta = acp::Meta::new();
                if cancelled {
                    meta.insert("toolTerminalStatus".into(), json!("cancelled"));
                }
                deliver(
                    &mut app,
                    sid,
                    &prompt,
                    3,
                    acp::SessionUpdate::ToolCallUpdate(
                        acp::ToolCallUpdate::new(
                            "reused-provider",
                            acp::ToolCallUpdateFields::new().status(Some(status)),
                        )
                        .meta(Some(meta)),
                    ),
                    false,
                );
                assert_eq!(last_entry(&app, id).tool_completion(), expected);
                assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
                assert!(!last_entry(&app, id).is_running);
                assert!(!render(&mut app, id).is_empty());
                assert_eq!(last_entry(&app, id).tool_completion(), expected);
            }
        }
    }
}

#[test]
fn coalesced_edits_keep_all_real_spans_late_terminal_projection_and_the_earlier_copy_identity() {
    let collapsed = crate::appearance::cache::load_collapsed_edit_blocks();
    crate::appearance::cache::set_collapsed_edit_blocks(true);
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let owners = ["edit-a", "edit-b"]
            .into_iter()
            .enumerate()
            .map(|(index, provider)| ToolCallOwnership::Registered {
                invocation_id: uuid::Uuid::from_u128(20 + index as u128).to_string(),
                provider_call_id: provider.into(),
                source: ToolCallSource::Sampling {
                    prompt_id: prompt.clone(),
                    request_id: REQUEST.into(),
                    attempt: 1,
                },
            })
            .collect::<Vec<_>>();
        for (index, owner) in owners.iter().enumerate() {
            let start = acp::ToolCall::new(provider_id(owner), "search_replace")
                .kind(acp::ToolKind::Edit)
                .status(acp::ToolCallStatus::Pending)
                .meta(Some(inner(owner, ToolExecutionTiming::Unknown)));
            deliver(
                &mut app,
                sid,
                &prompt,
                1 + index as u64,
                acp::SessionUpdate::ToolCall(start),
                false,
            );
        }
        let sb = &test_agent(&app, id).scrollback;
        let survivor = (0..sb.len())
            .filter_map(|index| sb.get(index))
            .find(|entry| matches!(entry.block, RenderBlock::ToolCall(_)))
            .unwrap()
            .id;
        for index in [1, 0] {
            let line = 5 + 35 * index;
            let diff = acp::Diff::new("timed.rs", format!("new_{line}"))
                .old_text(Some(format!("old_{line}")))
                .meta(
                    json!({"old_line":line,"new_line":line})
                        .as_object()
                        .cloned(),
                );
            let fields = acp::ToolCallUpdateFields::new()
                .kind(Some(acp::ToolKind::Edit))
                .title(Some("timed.rs".into()))
                .raw_input(Some(json!({"file_path":"timed.rs"})))
                .content(Some(vec![acp::ToolCallContent::Diff(diff)]))
                .status(Some(acp::ToolCallStatus::Completed));
            let terminal = acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                provider_id(&owners[index]),
                fields,
            ));
            deliver(&mut app, sid, &prompt, 4 - index as u64, terminal, false);
        }
        assert_eq!(last_entry(&app, id).id, survivor);
        match &last_entry(&app, id).block {
            RenderBlock::ToolCall(crate::scrollback::blocks::tool::ToolCallBlock::Edit(edit)) => {
                assert_eq!(edit.edit_count, 2)
            }
            _ => panic!("expected surviving coalesced edit"),
        }
        assert_eq!(clock(&app, id, survivor).elapsed_ms(), None);
        let first = ToolExecutionId::generate();
        let second = ToolExecutionId::generate();
        let second_end = finished(
            second,
            Default::default(),
            3000,
            ToolExecutionTermination::Returned,
        );
        let generation = test_agent(&app, id).scrollback.content_generation();
        deliver(
            &mut app,
            sid,
            &prompt,
            5,
            update(&owners[1], second_end, Default::default()),
            false,
        );
        assert!(test_agent(&app, id).scrollback.content_generation() > generation);
        assert_eq!(
            clock(&app, id, survivor).elapsed_ms(),
            None,
            "a missing invocation is not a zero-duration contribution"
        );
        deliver(
            &mut app,
            sid,
            &prompt,
            6,
            update(
                &owners[0],
                finished(
                    first,
                    Default::default(),
                    2000,
                    ToolExecutionTermination::Returned,
                ),
                Default::default(),
            ),
            false,
        );
        assert_eq!(clock(&app, id, survivor).elapsed_ms(), Some(5000));
        deliver(
            &mut app,
            sid,
            &prompt,
            7,
            update(&owners[1], second_end, Default::default()),
            false,
        );
        assert_eq!(clock(&app, id, survivor).elapsed_ms(), Some(5000));
        assert_eq!(
            test_agent(&app, id)
                .session
                .tracker
                .context_tool_call_id_for_entry(survivor)
                .unwrap()
                .as_str(),
            "edit-a"
        );
        let stats =
            crate::tool_usage::ToolUsageStats::from_scrollback(&test_agent(&app, id).scrollback);
        assert_eq!(
            stats.categories[&crate::tool_usage::ToolCategory::Edit].total_time_ms,
            5000
        );
    }
    crate::appearance::cache::set_collapsed_edit_blocks(collapsed);
}

#[path = "frontend_current_actions.rs"]
mod frontend_current_actions;
#[path = "frontend_publication.rs"]
mod frontend_publication;
