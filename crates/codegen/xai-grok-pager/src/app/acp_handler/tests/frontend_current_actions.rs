use super::*;
use xai_chat_state::source_version::SourceVersion;
use crate::acp::tracker::{TurnActivity, WaitingReason};
use crate::app::agent_view::current_actions::CurrentActionsContext;
use xai_grok_shell::session::current_actions::{
    BackendToolsObservation, CurrentActionTransition, CurrentActions, CurrentActionsEvent,
    CurrentActionsSnapshot, CurrentHook, CurrentInteraction, FrozenOwner, InteractionSource,
    TRANSITION_METHOD,
};

fn lifecycle(
    app: &mut AppView,
    sid: &str,
    sequence: u64,
    owner: &SamplingOwner,
    phase: SamplingPhase,
) {
    let mut meta = json!({"eventId": format!("{sid}-{sequence}"), "promptId": owner.prompt_id});
    stamp_fixture_delivery(app, sid, &mut meta);
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp_fixtures::ext_notification(
                "x.ai/session_notification",
                &SessionNotification {
                    session_id: acp::SessionId::new(sid),
                    update: XaiSessionUpdate::SamplingLifecycle {
                        lifecycle: SamplingLifecycle {
                            owner: owner.clone(),
                            phase,
                        },
                    },
                    meta: Some(meta),
                },
            ),
            response_tx,
        }),
        app,
    );
}

fn owner(prompt: &str) -> SamplingOwner {
    SamplingOwner {
        prompt_id: prompt.into(),
        request_id: REQUEST.into(),
        attempt: RequestAttemptId(1),
    }
}

fn loaded(app: &mut AppView, id: AgentId, sid: &str, prompt: &str) {
    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::SessionLoaded {
            agent_id: id,
            session_id: acp::SessionId::new(sid),
            models: None,
            modes: None,
            code_restored: false,
            restore_summary: None,
            restore_degree: None,
            running_prompt_id: Some(prompt.into()),
        }),
        app,
    );
}

fn bind_source(app: &mut AppView, id: AgentId) -> CurrentActionsContext {
    let view = app.agents.get_mut(&id).unwrap();
    let context = CurrentActionsContext {
        binding: view.current_actions_binding().unwrap(),
        incarnation: uuid::Uuid::new_v4(),
    };
    assert!(view.bind_current_actions(context.clone()));
    context
}

fn transition(
    app: &mut AppView,
    context: &CurrentActionsContext,
    sequence: u64,
    transition: CurrentActionTransition,
    replay: bool,
) {
    let event = CurrentActionsEvent {
        session_id: context.binding.session_id.clone(),
        version: SourceVersion {
            incarnation: context.incarnation,
            sequence,
        },
        transition,
    };
    let mut payload = serde_json::to_value(event).unwrap();
    let role = if replay {
        xai_grok_shell::extensions::current_actions::DeliveryRole::Historical {
            binding: context.binding.clone(),
        }
    } else {
        xai_grok_shell::extensions::current_actions::DeliveryRole::Live {
            context: xai_grok_shell::extensions::current_actions::PublicationContext {
                binding: context.binding.clone(),
                incarnation: context.incarnation,
            },
        }
    };
    let mut meta = acp::Meta::new();
    xai_grok_shell::extensions::current_actions::stamp_delivery(&mut meta, &role).unwrap();
    payload
        .as_object_mut()
        .unwrap()
        .insert("_meta".into(), serde_json::to_value(meta).unwrap());
    let (response_tx, _response) = tokio::sync::oneshot::channel();
    handle(
        AcpClientMessage::ExtNotification(xai_acp_lib::AcpArgs {
            request: acp_fixtures::ext_notification(TRANSITION_METHOD, &payload),
            response_tx,
        }),
        app,
    );
}

#[test]
fn first_new_live_callback_transfers_stash_and_full_replay_keeps_late_old_entry_settlement() {
    for id in [AgentId(0), AgentId(1)] {
        for outcome in 0..3 {
            let (mut app, sid, prompt) = target(id, true);
            let input = test_agent(&app, id)
                .session
                .in_flight_prompt
                .clone()
                .unwrap();
            let input_entry = input.scrollback_entry;
            let input_text = input.text;
            let sampling = owner(&prompt);
            let original = MonotonicPoint::at(tokio::time::Instant::from_std(
                Instant::now() - Duration::from_secs(30),
            ));
            lifecycle(
                &mut app,
                sid,
                1,
                &sampling,
                SamplingPhase::DispatchStarted {
                    started_at: original,
                },
            );
            let old = call_owner(&prompt, 700);
            let execution = ToolExecutionId::generate();
            deliver(&mut app, sid, &prompt, 2, base(&old), false);
            deliver(
                &mut app,
                sid,
                &prompt,
                3,
                update(
                    &old,
                    ToolExecutionTiming::Started {
                        execution_id: execution,
                        started_at: original,
                    },
                    Default::default(),
                ),
                false,
            );
            let entry = last_entry(&app, id).id;
            let retained = clock(&app, id, entry);
            let mapping = retained.mapping(execution);
            assert!(mapping.is_some());
            app.agents.get_mut(&id).unwrap().begin_session_reload(41);
            if outcome == 1 {
                deliver_meta(
                    &mut app,
                    sid,
                    acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                        acp::ContentBlock::Text(acp::TextContent::new(input_text)),
                    )),
                    json!({"isReplay":true,"promptId":prompt,"eventId":format!("{sid}-0")}),
                );
                deliver(&mut app, sid, &prompt, 2, base(&old), true);
                deliver(
                    &mut app,
                    sid,
                    &prompt,
                    3,
                    update(
                        &old,
                        ToolExecutionTiming::Started {
                            execution_id: execution,
                            started_at: original,
                        },
                        Default::default(),
                    ),
                    true,
                );
            }
            assert!(matches!(
                test_agent(&app, id).load_reconstruction,
                crate::app::agent_view::load_continuation::LoadReconstruction::Historical
            ));
            let next = call_owner(&prompt, 701);
            deliver(&mut app, sid, &prompt, 10, base(&next), false);
            assert!(matches!(
                test_agent(&app, id).load_reconstruction,
                crate::app::agent_view::load_continuation::LoadReconstruction::Continuing { .. }
            ));
            let next_entry = last_entry(&app, id).id;
            assert_ne!(entry, next_entry);
            assert_eq!(clock(&app, id, entry).mapping(execution), mapping);
            deliver(
                &mut app,
                sid,
                &prompt,
                11,
                update(
                    &old,
                    finished(
                        execution,
                        original,
                        2000,
                        ToolExecutionTermination::Returned,
                    ),
                    Default::default(),
                ),
                false,
            );
            deliver(
                &mut app,
                sid,
                &prompt,
                12,
                update(
                    &old,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                ),
                false,
            );
            assert!(
                app.agents
                    .get_mut(&id)
                    .unwrap()
                    .finish_session_reload(41, outcome != 2)
            );
            assert_eq!(clock(&app, id, entry).elapsed_ms(), Some(2000));
            assert_eq!(retained.elapsed_ms(), Some(2000));
            assert!(
                test_agent(&app, id)
                    .scrollback
                    .get_by_id(input_entry)
                    .is_some()
            );
            assert_eq!(
                test_agent(&app, id)
                    .scrollback
                    .get_by_id(entry)
                    .unwrap()
                    .tool_completion(),
                ToolCompletion::Completed
            );
            assert!(
                test_agent(&app, id)
                    .scrollback
                    .get_by_id(next_entry)
                    .unwrap()
                    .is_running
            );
            loaded(&mut app, id, sid, &prompt);
            assert!(
                test_agent(&app, id)
                    .scrollback
                    .get_by_id(next_entry)
                    .unwrap()
                    .is_running
            );
            assert_eq!(retained.elapsed_ms(), Some(2000));
        }
    }
}

#[test]
fn native_dispatch_without_tokens_keeps_the_original_phase_anchor_across_inactive_focus_and_reload()
{
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let sampling = owner(&prompt);
        let at = Instant::now() - Duration::from_secs(30);
        let point = MonotonicPoint::at(tokio::time::Instant::from_std(at));
        lifecycle(
            &mut app,
            sid,
            1,
            &sampling,
            SamplingPhase::DispatchStarted { started_at: point },
        );
        let clock = test_agent(&app, id).current_action_started_at();
        let LocalClockMapping::Correlated {
            earliest_at,
            latest_at,
        } = point.map_local()
        else {
            panic!("native domain required for this positive oracle");
        };
        assert!(earliest_at.into_std() <= at && at <= latest_at.into_std());
        assert!(clock.is_some());
        assert_eq!(
            test_agent(&app, id).resolve_turn_activity(),
            Some(TurnActivity::Waiting(WaitingReason::Model))
        );
        let _ = render(&mut app, id);
        app.active_view = ActiveView::Agent(id);
        let _ = render(&mut app, id);
        assert_eq!(test_agent(&app, id).current_action_started_at(), clock);
        app.agents.get_mut(&id).unwrap().begin_session_reload(42);
        lifecycle(
            &mut app,
            sid,
            2,
            &sampling,
            SamplingPhase::Finished {
                outcome: xai_grok_shell::extensions::sampling_lifecycle::SamplingOutcome::Completed,
            },
        );
        assert!(
            app.agents
                .get_mut(&id)
                .unwrap()
                .finish_session_reload(42, true)
        );
        assert_eq!(test_agent(&app, id).current_action_started_at(), None);
        loaded(&mut app, id, sid, &prompt);
        assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    }
}

#[test]
fn history_after_closed_uses_a_separate_model_owner_and_cannot_clear_the_live_phase_clock() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        let sampling = owner(&prompt);
        lifecycle(
            &mut app,
            sid,
            1,
            &sampling,
            SamplingPhase::DispatchStarted {
                started_at: MonotonicPoint::at(tokio::time::Instant::now()),
            },
        );
        let clock = test_agent(&app, id).current_action_started_at();
        app.agents.get_mut(&id).unwrap().arm_late_replay_grace();
        deliver_meta(
            &mut app,
            sid,
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("historical thought"),
            ))),
            json!({"isReplay":true,"promptId":"historical","eventId":format!("{sid}-90")}),
        );
        assert_eq!(test_agent(&app, id).current_action_started_at(), clock);
        assert_eq!(
            test_agent(&app, id).resolve_turn_activity(),
            Some(TurnActivity::Waiting(WaitingReason::Model))
        );
        assert_eq!(
            test_agent(&app, id).session.current_prompt_id.as_deref(),
            Some(prompt.as_str())
        );
    }
}

#[test]
fn transient_hook_clocks_are_observations_and_unrelated_source_versions_do_not_fence_other_owned_work()
 {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, _sid, prompt) = target(id, true);
        let context = bind_source(&mut app, id);
        let before = test_agent(&app, id).last_active_at;
        let ack = test_agent(&app, id).prompt_ack.is_some();
        let point = MonotonicPoint::at(tokio::time::Instant::from_std(
            Instant::now() - Duration::from_secs(30),
        ));
        let batch = uuid::Uuid::new_v4();
        let turn = if id == AgentId(0) {
            FrozenOwner::Root {
                prompt_id: prompt,
                task_id: "root-task-fixture".into(),
            }
        } else {
            FrozenOwner::Child {
                prompt_id: prompt,
                task_id: "child-task-fixture".into(),
            }
        };
        let registration = uuid::Uuid::new_v4();
        transition(
            &mut app,
            &context,
            10,
            CurrentActionTransition::HookStarted {
                hook: CurrentHook {
                    batch_id: batch,
                    turn: turn.clone(),
                    started_at: point,
                },
            },
            false,
        );
        let clock = test_agent(&app, id).current_action_started_at();
        assert!(clock.is_some());
        let _ = render(&mut app, id);
        assert_eq!(test_agent(&app, id).current_action_started_at(), clock);
        transition(
            &mut app,
            &context,
            30,
            CurrentActionTransition::InteractionStarted {
                interaction: CurrentInteraction {
                    tool_call_id: "permission-fixture".into(),
                    kind: xai_grok_shell::session::pending_interaction::PendingKind::Permission,
                    source: InteractionSource::Registered {
                        registration_id: registration,
                        turn: turn.clone(),
                        started_at: point,
                    },
                },
            },
            false,
        );
        transition(
            &mut app,
            &context,
            12,
            CurrentActionTransition::HookFinished { batch_id: batch },
            false,
        );
        transition(
            &mut app,
            &context,
            11,
            CurrentActionTransition::HookStarted {
                hook: CurrentHook {
                    batch_id: batch,
                    turn: turn.clone(),
                    started_at: point,
                },
            },
            false,
        );
        assert_eq!(test_agent(&app, id).last_active_at, before);
        assert_eq!(test_agent(&app, id).prompt_ack.is_some(), ack);
        assert!(
            test_agent(&app, id).permission_queue.is_empty(),
            "source observation must not issue or steal a reverse request"
        );
        transition(
            &mut app,
            &context,
            32,
            CurrentActionTransition::InteractionFinished {
                tool_call_id: "permission-fixture".into(),
                registration_id: registration,
            },
            false,
        );
        assert_eq!(test_agent(&app, id).current_action_started_at(), None);
        let next_hook = CurrentHook {
            batch_id: uuid::Uuid::new_v4(),
            turn,
            started_at: point,
        };
        transition(
            &mut app,
            &context,
            21,
            CurrentActionTransition::HookStarted {
                hook: next_hook.clone(),
            },
            false,
        );
        assert!(test_agent(&app, id).current_action_started_at().is_some());
        let next_clock = test_agent(&app, id).current_action_started_at();
        transition(
            &mut app,
            &context,
            33,
            CurrentActionTransition::HookStarted { hook: next_hook },
            true,
        );
        assert_eq!(test_agent(&app, id).current_action_started_at(), next_clock);
        assert_eq!(test_agent(&app, id).last_active_at, before);
    }
}

#[test]
fn stale_snapshot_and_wrong_binding_load_wrappers_are_rejected_before_any_owner_mutation() {
    let id = AgentId(0);
    let (mut app, sid, prompt) = target(id, false);
    let context = bind_source(&mut app, id);
    let old_snapshot = CurrentActionsSnapshot::Current(CurrentActions {
        binding: context.binding.clone(),
        version: SourceVersion {
            incarnation: context.incarnation,
            sequence: 1,
        },
        event_frontier: 50,
        turn: None,
        sampling: Vec::new(),
        tools: Vec::new(),
        backend_tools: BackendToolsObservation::Unavailable,
        compactions: Vec::new(),
        hooks: Vec::new(),
        interactions: Vec::new(),
    });
    deliver_meta(
        &mut app,
        sid,
        base(&call_owner(&prompt, 702)),
        json!({"promptId":prompt,"eventId":format!("{sid}-50"),
        "currentActionsVersion":SourceVersion { incarnation: context.incarnation, sequence: 2 }}),
    );
    let entry = last_entry(&app, id).id;
    let activity = test_agent(&app, id).last_active_at;
    assert!(!crate::app::acp_handler::current_actions::apply_snapshot(
        &mut app,
        id,
        &context,
        old_snapshot
    ));
    let mut stale = context;
    stale.binding.load_generation += 1;
    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::WithCurrentActions {
            agent_id: id,
            context: crate::app::agent_view::CurrentActionsReadContext::Observe(stale),
            snapshot: CurrentActionsSnapshot::Unavailable,
            result: Box::new(TaskResult::SessionLoaded {
                agent_id: id,
                session_id: acp::SessionId::new(sid),
                models: None,
                modes: None,
                code_restored: false,
                restore_summary: None,
                restore_degree: None,
                running_prompt_id: Some(prompt.clone()),
            }),
        }),
        &mut app,
    );
    assert!(
        test_agent(&app, id)
            .scrollback
            .get_by_id(entry)
            .unwrap()
            .is_running
    );
    assert_eq!(test_agent(&app, id).last_active_at, activity);
    assert_eq!(
        test_agent(&app, id).session.current_prompt_id.as_deref(),
        Some(prompt.as_str())
    );
}

#[test]
fn source_unavailable_and_legacy_dispatch_never_create_phase_or_history_clocks() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, false);
        lifecycle(
            &mut app,
            sid,
            1,
            &owner(&prompt),
            SamplingPhase::DispatchStarted {
                started_at: Default::default(),
            },
        );
        assert_eq!(test_agent(&app, id).current_action_started_at(), None);
        assert_eq!(
            xai_grok_shell::extensions::current_actions::load_snapshot(None).unwrap(),
            CurrentActionsSnapshot::Unavailable
        );
        let context = bind_source(&mut app, id);
        let activity = test_agent(&app, id).last_active_at;
        assert!(crate::app::acp_handler::current_actions::apply_snapshot(
            &mut app,
            id,
            &context,
            CurrentActionsSnapshot::Unavailable
        ));
        assert_eq!(test_agent(&app, id).current_action_started_at(), None);
        assert_eq!(
            test_agent(&app, id).resolve_turn_activity(),
            Some(TurnActivity::Waiting(
                WaitingReason::ActionSourceUnavailable
            ))
        );
        assert_eq!(test_agent(&app, id).last_active_at, activity);
    }
}

#[test]
fn coalesced_entry_handles_and_all_invocation_spans_survive_full_replay_and_a_new_model_owner() {
    let collapsed = crate::appearance::cache::load_collapsed_edit_blocks();
    crate::appearance::cache::set_collapsed_edit_blocks(true);
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let owners: Vec<_> = ["edit-a", "edit-b"]
            .into_iter()
            .enumerate()
            .map(|(index, provider)| ToolCallOwnership::Registered {
                invocation_id: uuid::Uuid::from_u128(800 + index as u128).to_string(),
                provider_call_id: provider.into(),
                source: ToolCallSource::Sampling {
                    prompt_id: prompt.clone(),
                    request_id: REQUEST.into(),
                    attempt: 1,
                },
            })
            .collect();
        let start = |index: usize| {
            acp::SessionUpdate::ToolCall(
                acp::ToolCall::new(provider_id(&owners[index]), "search_replace")
                    .kind(acp::ToolKind::Edit)
                    .status(acp::ToolCallStatus::Pending)
                    .meta(Some(inner(&owners[index], ToolExecutionTiming::Unknown))),
            )
        };
        let terminal = |index: usize| {
            let line = 5 + 35 * index;
            let diff = acp::Diff::new("retained.rs", format!("new_{line}"))
                .old_text(Some(format!("old_{line}")))
                .meta(
                    json!({"old_line":line,"new_line":line})
                        .as_object()
                        .cloned(),
                );
            update(
                &owners[index],
                ToolExecutionTiming::Unknown,
                acp::ToolCallUpdateFields::new()
                    .kind(Some(acp::ToolKind::Edit))
                    .title(Some("retained.rs".into()))
                    .raw_input(Some(json!({"file_path":"retained.rs"})))
                    .content(Some(vec![acp::ToolCallContent::Diff(diff)]))
                    .status(Some(acp::ToolCallStatus::Completed)),
            )
        };
        for index in 0..2 {
            deliver(
                &mut app,
                sid,
                &prompt,
                1 + index as u64,
                start(index),
                false,
            );
        }
        for index in [1, 0] {
            deliver(
                &mut app,
                sid,
                &prompt,
                4 - index as u64,
                terminal(index),
                false,
            );
        }
        let survivor = last_entry(&app, id).id;
        let retained = clock(&app, id, survivor);
        app.agents.get_mut(&id).unwrap().begin_session_reload(43);
        for index in 0..2 {
            deliver(&mut app, sid, &prompt, 1 + index as u64, start(index), true);
        }
        for index in [1, 0] {
            deliver(
                &mut app,
                sid,
                &prompt,
                4 - index as u64,
                terminal(index),
                true,
            );
        }
        let next = SamplingOwner {
            prompt_id: prompt.clone(),
            request_id: "next-real-request".into(),
            attempt: RequestAttemptId(1),
        };
        lifecycle(
            &mut app,
            sid,
            20,
            &next,
            SamplingPhase::DispatchStarted {
                started_at: MonotonicPoint::at(tokio::time::Instant::now()),
            },
        );
        let phase = test_agent(&app, id).current_action_started_at();
        assert!(phase.is_some());
        for index in 0..2 {
            deliver(
                &mut app,
                sid,
                &prompt,
                21 + index as u64,
                update(
                    &owners[index],
                    finished(
                        ToolExecutionId::generate(),
                        Default::default(),
                        2000 + 1000 * index as u64,
                        ToolExecutionTermination::Returned,
                    ),
                    Default::default(),
                ),
                false,
            );
        }
        assert!(
            app.agents
                .get_mut(&id)
                .unwrap()
                .finish_session_reload(43, true)
        );
        assert_eq!(retained.elapsed_ms(), Some(5000));
        assert_eq!(clock(&app, id, survivor).elapsed_ms(), Some(5000));
        assert_eq!(test_agent(&app, id).current_action_started_at(), phase);
        assert_eq!(
            test_agent(&app, id)
                .session
                .tracker
                .context_tool_call_id_for_entry(survivor)
                .unwrap()
                .as_str(),
            "edit-a"
        );
        let entry = test_agent(&app, id).scrollback.get_by_id(survivor).unwrap();
        let RenderBlock::ToolCall(crate::scrollback::blocks::tool::ToolCallBlock::Edit(edit)) =
            &entry.block
        else {
            panic!("original edit survivor required");
        };
        assert_eq!(edit.edit_count, 2);
    }
    crate::appearance::cache::set_collapsed_edit_blocks(collapsed);
}

#[test]
fn original_rpc_terminal_before_reload_result_stays_settled_for_every_reload_outcome() {
    for id in [AgentId(0), AgentId(1)] {
        for outcome in 0..3 {
            let (mut app, sid, prompt) = target(id, true);
            let call = call_owner(&prompt, 803);
            deliver(&mut app, sid, &prompt, 1, base(&call), false);
            let entry = last_entry(&app, id).id;
            app.agents.get_mut(&id).unwrap().begin_session_reload(44);
            if outcome == 1 {
                deliver(&mut app, sid, &prompt, 1, base(&call), true);
            }
            deliver(
                &mut app,
                sid,
                &prompt,
                2,
                update(
                    &call,
                    ToolExecutionTiming::Unknown,
                    acp::ToolCallUpdateFields::new().status(Some(acp::ToolCallStatus::Completed)),
                ),
                false,
            );
            finish_turn(&mut app, id, &prompt);
            assert!(
                app.agents
                    .get_mut(&id)
                    .unwrap()
                    .finish_session_reload(44, outcome != 2)
            );
            loaded(&mut app, id, sid, &prompt);
            assert!(test_agent(&app, id).session.state.is_idle());
            assert!(test_agent(&app, id).session.current_prompt_id.is_none());
            assert_eq!(
                test_agent(&app, id)
                    .scrollback
                    .get_by_id(entry)
                    .unwrap()
                    .tool_completion(),
                ToolCompletion::Completed
            );
            assert!(
                !test_agent(&app, id)
                    .scrollback
                    .get_by_id(entry)
                    .unwrap()
                    .is_running
            );
        }
    }
}

#[test]
fn actor_incarnations_are_only_trusted_attach_identities_and_never_lexical_order() {
    let id = AgentId(0);
    let (mut app, _sid, prompt) = target(id, false);
    let mut old = bind_source(&mut app, id);
    old.incarnation = uuid::Uuid::from_u128(u128::MAX);
    assert!(
        app.agents
            .get_mut(&id)
            .unwrap()
            .bind_current_actions(old.clone())
    );
    let mut new = old.clone();
    new.incarnation = uuid::Uuid::from_u128(1);
    assert!(
        app.agents
            .get_mut(&id)
            .unwrap()
            .bind_current_actions(new.clone())
    );
    let point = MonotonicPoint::at(tokio::time::Instant::from_std(
        Instant::now() - Duration::from_secs(1),
    ));
    let hook = CurrentHook {
        batch_id: uuid::Uuid::new_v4(),
        turn: FrozenOwner::Root {
            prompt_id: prompt,
            task_id: "new-actor-task".into(),
        },
        started_at: point,
    };
    transition(
        &mut app,
        &old,
        100,
        CurrentActionTransition::HookStarted { hook: hook.clone() },
        false,
    );
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    transition(
        &mut app,
        &new,
        1,
        CurrentActionTransition::HookStarted { hook },
        false,
    );
    assert!(test_agent(&app, id).current_action_started_at().is_some());
}

#[test]
fn native_history_context_cannot_promote_an_unstamped_transient_current_action() {
    let id = AgentId(0);
    let (mut app, _sid, prompt) = target(id, false);
    let context = bind_source(&mut app, id);
    let before = test_agent(&app, id).last_active_at;
    let event = CurrentActionsEvent {
        session_id: context.binding.session_id.clone(),
        version: SourceVersion {
            incarnation: context.incarnation,
            sequence: 1,
        },
        transition: CurrentActionTransition::HookStarted {
            hook: CurrentHook {
                batch_id: uuid::Uuid::new_v4(),
                turn: FrozenOwner::Root {
                    prompt_id: prompt,
                    task_id: "history-only-task".into(),
                },
                started_at: MonotonicPoint::at(tokio::time::Instant::now()),
            },
        },
    };
    assert!(!crate::app::acp_handler::current_actions::apply_transition(
        &mut app,
        id,
        &context,
        crate::app::agent_view::load_continuation::LoadDelivery::Historical,
        event
    ));
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    assert_eq!(test_agent(&app, id).last_active_at, before);
}

#[test]
fn queued_manual_source_does_not_end_foreground_and_persistence_wait_ends_only_at_original_rpc() {
    use xai_grok_shell::extensions::notification::CompactPhase;
    use xai_grok_shell::session::current_actions::{CompactOwner, CompactStage, CurrentCompact};
    let id = AgentId(0);
    let (mut app, sid, prompt) = target(id, true);
    let context = bind_source(&mut app, id);
    lifecycle(
        &mut app,
        sid,
        1,
        &owner(&prompt),
        SamplingPhase::DispatchStarted {
            started_at: MonotonicPoint::at(tokio::time::Instant::from_std(
                Instant::now() - Duration::from_secs(30),
            )),
        },
    );
    let foreground_clock = test_agent(&app, id).current_action_started_at();
    let inactive_view = app.active_view;
    app.active_view = ActiveView::Agent(id);
    let effects = crate::app::dispatch::dispatch(Action::SendPrompt("/compact".into()), &mut app);
    app.active_view = inactive_view;
    let invocation = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Compact {
                agent_id,
                invocation_id,
                ..
            } => {
                assert_eq!(*agent_id, id);
                Some(*invocation_id)
            }
            _ => None,
        })
        .expect("original compact RPC effect");
    let admitted = MonotonicPoint::at(tokio::time::Instant::from_std(
        Instant::now() - Duration::from_secs(20),
    ));
    let mut compact = CurrentCompact {
        owner: CompactOwner::Manual {
            request_id: uuid::Uuid::new_v4(),
            invocation_id: Some(invocation),
        },
        turn: FrozenOwner::Root {
            prompt_id: prompt.clone(),
            task_id: "foreground-task-fixture".into(),
        },
        stage: CompactStage::Queued,
        admitted_at: admitted,
        stage_started_at: admitted,
    };
    transition(
        &mut app,
        &context,
        10,
        CurrentActionTransition::CompactChanged {
            compact: compact.clone(),
        },
        false,
    );
    assert!(test_agent(&app, id).session.state.is_turn_running());
    assert_eq!(
        test_agent(&app, id).current_action_started_at(),
        foreground_clock
    );
    finish_turn(&mut app, id, &prompt);
    let manual = |app: &mut AppView, update| {
        let mut meta = json!({"compactInvocationId":invocation});
        stamp_fixture_delivery(app, sid, &mut meta);
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
    };
    manual(
        &mut app,
        XaiSessionUpdate::AutoCompactStarted {
            phase: CompactPhase::Executing,
            tokens_used: 90,
            context_window: 100,
            percentage: 90,
            reason: "manual".into(),
        },
    );
    compact.stage = CompactStage::Executing;
    transition(
        &mut app,
        &context,
        11,
        CurrentActionTransition::CompactChanged {
            compact: compact.clone(),
        },
        false,
    );
    app.agents.get_mut(&id).unwrap().begin_session_reload(45);
    manual(
        &mut app,
        XaiSessionUpdate::AutoCompactCompleted {
            tokens_before: Some(100),
            tokens_after: 50,
            elapsed_ms: Some(2000),
            summary_preview: None,
        },
    );
    compact.stage = CompactStage::AwaitingPersistence;
    compact.stage_started_at = MonotonicPoint::at(tokio::time::Instant::from_std(
        Instant::now() - Duration::from_secs(3),
    ));
    let mut current_context = context.clone();
    current_context.binding = test_agent(&app, id).current_actions_binding().unwrap();
    transition(
        &mut app,
        &current_context,
        12,
        CurrentActionTransition::CompactChanged {
            compact: compact.clone(),
        },
        false,
    );
    let persistence_clock = test_agent(&app, id).current_action_started_at();
    assert!(persistence_clock.is_some());
    assert_eq!(
        test_agent(&app, id).resolve_turn_activity(),
        Some(TurnActivity::Waiting(WaitingReason::CompactionSettlement))
    );
    assert!(
        app.agents
            .get_mut(&id)
            .unwrap()
            .finish_session_reload(45, true)
    );
    let _ = render(&mut app, id);
    assert_eq!(
        test_agent(&app, id).current_action_started_at(),
        persistence_clock
    );
    assert_ne!(
        test_agent(&app, id).compact_invocations[&invocation].settlement,
        crate::app::agent::CompactInvocationSettlement::RpcSettled
    );
    crate::app::dispatch::dispatch(
        Action::TaskComplete(TaskResult::CompactComplete {
            agent_id: id,
            invocation_id: invocation,
            result: Ok(()),
        }),
        &mut app,
    );
    assert_eq!(
        test_agent(&app, id).compact_invocations[&invocation].settlement,
        crate::app::agent::CompactInvocationSettlement::RpcSettled
    );
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    transition(
        &mut app,
        &current_context,
        13,
        CurrentActionTransition::CompactChanged {
            compact: compact.clone(),
        },
        false,
    );
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    assert!(
        test_agent(&app, id)
            .current_actions_display_state()
            .is_none()
    );
    compact.stage = CompactStage::AwaitingResponse;
    transition(
        &mut app,
        &current_context,
        14,
        CurrentActionTransition::CompactChanged {
            compact: compact.clone(),
        },
        false,
    );
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    app.active_view = ActiveView::Agent(id);
    let next_effects =
        crate::app::dispatch::dispatch(Action::SendPrompt("/compact".into()), &mut app);
    app.active_view = inactive_view;
    let next_invocation = next_effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Compact { invocation_id, .. } => Some(*invocation_id),
            _ => None,
        })
        .expect("independent original compact RPC effect");
    assert_ne!(next_invocation, invocation);
    compact.owner = CompactOwner::Manual {
        request_id: uuid::Uuid::new_v4(),
        invocation_id: Some(next_invocation),
    };
    transition(
        &mut app,
        &current_context,
        15,
        CurrentActionTransition::CompactChanged { compact },
        false,
    );
    assert!(test_agent(&app, id).current_action_started_at().is_some());
    assert_eq!(
        test_agent(&app, id).resolve_turn_activity(),
        Some(TurnActivity::Waiting(WaitingReason::CompactionSettlement))
    );
}

#[test]
fn cold_task_events_render_original_task_and_phase_clocks_without_acquiring_rpc_custody() {
    use xai_grok_shell::session::current_actions::{CurrentTurn, CurrentTurnPhase};
    let id = AgentId(0);
    let (mut app, _sid, previous) = target(id, false);
    finish_turn(&mut app, id, &previous);
    let context = bind_source(&mut app, id);
    let original = Instant::now() - Duration::from_secs(30);
    let phase = Instant::now() - Duration::from_secs(10);
    let mut turn = CurrentTurn {
        owner: FrozenOwner::Root {
            prompt_id: "remote-current-prompt".into(),
            task_id: "remote-current-task".into(),
        },
        started_at: MonotonicPoint::at(tokio::time::Instant::from_std(original)),
        phase: CurrentTurnPhase::Admitted,
    };
    transition(
        &mut app,
        &context,
        1,
        CurrentActionTransition::TurnRegistered { turn: turn.clone() },
        false,
    );
    turn.phase = CurrentTurnPhase::Running {
        started_at: MonotonicPoint::at(tokio::time::Instant::from_std(phase)),
    };
    transition(
        &mut app,
        &context,
        2,
        CurrentActionTransition::TurnStarted { turn: turn.clone() },
        false,
    );
    let phase_clock = test_agent(&app, id).current_action_started_at().unwrap();
    let task_elapsed = test_agent(&app, id).turn_elapsed().unwrap();
    assert!(task_elapsed > phase_clock.elapsed() + Duration::from_secs(15));
    let screen = render(&mut app, id);
    assert!(screen.contains("Waiting for backend response"));
    assert_eq!(
        test_agent(&app, id).current_action_started_at(),
        Some(phase_clock)
    );
    assert!(test_agent(&app, id).session.state.is_idle());
    assert!(test_agent(&app, id).session.current_prompt_id.is_none());
    assert!(test_agent(&app, id).permission_queue.is_empty());
    transition(
        &mut app,
        &context,
        3,
        CurrentActionTransition::TurnFinished { owner: turn.owner },
        false,
    );
    assert_eq!(test_agent(&app, id).current_action_started_at(), None);
    assert!(test_agent(&app, id).turn_elapsed().is_none());
}

#[test]
fn full_replay_reconstructs_missed_text_and_keeps_only_the_post_frontier_live_suffix_once() {
    for id in [AgentId(0), AgentId(1)] {
        let (mut app, sid, prompt) = target(id, true);
        let sampling = owner(&prompt);
        lifecycle(
            &mut app,
            sid,
            0,
            &sampling,
            SamplingPhase::DispatchStarted {
                started_at: MonotonicPoint::at(tokio::time::Instant::now()),
            },
        );
        let chunk = |text: &str| {
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new(text),
            )))
        };
        let meta = |sequence, replay| json!({"promptId":prompt,"samplingOwner":sampling,"eventId":format!("{sid}-{sequence}"),"isReplay":replay});
        deliver_meta(&mut app, sid, chunk("prefix "), meta(1, false));
        let sb = &test_agent(&app, id).scrollback;
        let entry = (0..sb.len())
            .filter_map(|index| sb.get(index))
            .find(|entry| matches!(entry.block, RenderBlock::AgentMessage(_)))
            .unwrap()
            .id;
        let phase = test_agent(&app, id).current_action_started_at();
        app.agents.get_mut(&id).unwrap().begin_session_reload(46);
        deliver_meta(&mut app, sid, chunk("prefix "), meta(1, true));
        deliver_meta(&mut app, sid, chunk("missed "), meta(2, true));
        deliver_meta(&mut app, sid, chunk("covered-live "), meta(3, false));
        deliver_meta(&mut app, sid, chunk("covered-live "), meta(3, true));
        deliver_meta(&mut app, sid, chunk("post-cut"), meta(4, false));
        assert!(
            app.agents
                .get_mut(&id)
                .unwrap()
                .finish_session_reload(46, true)
        );
        let original = test_agent(&app, id).scrollback.get_by_id(entry).unwrap();
        let RenderBlock::AgentMessage(message) = &original.block else {
            panic!("original streaming projection required");
        };
        assert_eq!(message.text(), "prefix missed covered-live post-cut");
        assert!(original.is_running);
        assert_eq!(test_agent(&app, id).current_action_started_at(), phase);
    }
}

#[test]
fn original_prompt_rpc_before_first_attach_rejects_correctly_tagged_stale_work() {
    use crate::app::agent_view::CurrentActionsReadContext;
    use xai_grok_shell::session::current_actions::{
        CurrentSampling, CurrentSamplingPhase, CurrentTool, CurrentTurn, CurrentTurnPhase,
    };
    for id in [AgentId(0), AgentId(1)] {
        for outcome in 0..4 {
            let (mut app, sid, prompt) = target(id, true);
            let call = call_owner(&prompt, 804);
            deliver(&mut app, sid, &prompt, 1, base(&call), false);
            let entry = last_entry(&app, id).id;
            if outcome != 0 {
                app.agents.get_mut(&id).unwrap().begin_session_reload(47);
            }
            if outcome == 2 {
                deliver(&mut app, sid, &prompt, 1, base(&call), true);
            }
            let context = CurrentActionsContext {
                binding: test_agent(&app, id).current_actions_binding().unwrap(),
                incarnation: uuid::Uuid::new_v4(),
            };
            let point = MonotonicPoint::at(tokio::time::Instant::now());
            let turn_owner = if id == AgentId(0) {
                FrozenOwner::Root {
                    prompt_id: prompt.clone(),
                    task_id: "stale-root-task".into(),
                }
            } else {
                FrozenOwner::Child {
                    prompt_id: prompt.clone(),
                    task_id: "stale-child-task".into(),
                }
            };
            // Negative stale-read fixture, not a current Reader/Producer oracle.
            let stale_read = CurrentActionsSnapshot::Current(CurrentActions {
                binding: context.binding.clone(),
                version: SourceVersion {
                    incarnation: context.incarnation,
                    sequence: 1,
                },
                event_frontier: 100,
                turn: Some(CurrentTurn {
                    owner: turn_owner.clone(),
                    started_at: point,
                    phase: CurrentTurnPhase::Running { started_at: point },
                }),
                sampling: vec![CurrentSampling {
                    owner: owner(&prompt),
                    turn: turn_owner.clone(),
                    phase: CurrentSamplingPhase::Submitted { started_at: point },
                }],
                tools: vec![CurrentTool {
                    ownership: call,
                    turn: turn_owner,
                    registered_at: point,
                    executions: Vec::new(),
                }],
                backend_tools: BackendToolsObservation::Unavailable,
                compactions: Vec::new(),
                hooks: Vec::new(),
                interactions: Vec::new(),
            });
            finish_turn(&mut app, id, &prompt);
            if outcome != 0 {
                assert!(
                    app.agents
                        .get_mut(&id)
                        .unwrap()
                        .finish_session_reload(47, outcome != 3)
                );
            }
            crate::app::dispatch::dispatch(
                Action::TaskComplete(TaskResult::WithCurrentActions {
                    agent_id: id,
                    context: CurrentActionsReadContext::Attach(context.clone()),
                    snapshot: stale_read,
                    result: Box::new(TaskResult::SessionLoaded {
                        agent_id: id,
                        session_id: acp::SessionId::new(sid),
                        models: None,
                        modes: None,
                        code_restored: false,
                        restore_summary: None,
                        restore_degree: None,
                        running_prompt_id: Some(prompt.clone()),
                    }),
                }),
                &mut app,
            );
            let view = test_agent(&app, id);
            assert_eq!(
                view.current_actions_context(),
                Some(&context),
                "the first attach must actually pass its binding gates"
            );
            assert!(view.session.state.is_idle());
            assert!(view.session.current_prompt_id.is_none());
            assert_eq!(view.current_action_activity(), Some(None));
            assert_eq!(view.current_action_started_at(), None);
            assert!(view.current_actions_display_state().is_none());
            assert!(!view.scrollback.get_by_id(entry).unwrap().is_running);
            let other = CurrentTurn {
                owner: FrozenOwner::Root {
                    prompt_id: "independent-prompt".into(),
                    task_id: "independent-task".into(),
                },
                started_at: point,
                phase: CurrentTurnPhase::Admitted,
            };
            transition(
                &mut app,
                &context,
                2,
                CurrentActionTransition::TurnRegistered { turn: other },
                false,
            );
            assert!(test_agent(&app, id).current_action_started_at().is_some());
            assert!(test_agent(&app, id).session.current_prompt_id.is_none());
        }
    }
}

#[test]
fn prompt_rpc_fence_is_scoped_to_first_source_and_actual_session_binding() {
    use xai_grok_shell::session::current_actions::{CurrentTurn, CurrentTurnPhase};
    for rebind_before_attach in [false, true] {
        let id = AgentId(0);
        let (mut app, _sid, prompt) = target(id, false);
        finish_turn(&mut app, id, &prompt);
        let old_binding = test_agent(&app, id).current_actions_binding().unwrap();
        if rebind_before_attach {
            app.agents
                .get_mut(&id)
                .unwrap()
                .bind_session_id("independent-session".into());
        }
        let context = bind_source(&mut app, id);
        if rebind_before_attach {
            assert_ne!(context.binding.session_id, old_binding.session_id);
            assert_ne!(context.binding.binding_epoch, old_binding.binding_epoch);
        } else {
            let mut replacement = context;
            replacement.incarnation = uuid::Uuid::new_v4();
            assert!(
                app.agents
                    .get_mut(&id)
                    .unwrap()
                    .bind_current_actions(replacement)
            );
        }
        let context = test_agent(&app, id)
            .current_actions_context()
            .unwrap()
            .clone();
        let point = MonotonicPoint::at(tokio::time::Instant::now());
        transition(
            &mut app,
            &context,
            1,
            CurrentActionTransition::TurnRegistered {
                turn: CurrentTurn {
                    owner: FrozenOwner::Root {
                        prompt_id: prompt,
                        task_id: "independent-actor-task".into(),
                    },
                    started_at: point,
                    phase: CurrentTurnPhase::Admitted,
                },
            },
            false,
        );
        assert_eq!(
            test_agent(&app, id).current_action_activity(),
            Some(Some(TurnActivity::Waiting(WaitingReason::TaskStart)))
        );
        assert!(test_agent(&app, id).current_action_started_at().is_some());
        assert!(test_agent(&app, id).session.current_prompt_id.is_none());
    }
}

#[test]
fn legacy_thinking_without_native_measurement_never_uses_receipt_or_finish_time() {
    let mut thought = crate::scrollback::blocks::ThinkingBlock::streaming();
    thought.push_chunk("legacy content");
    assert_eq!(thought.elapsed_time_ms(), None);
    thought.finish();
    assert_eq!(thought.elapsed_time_ms(), None);
    thought.set_elapsed_time_ms(Some(2000));
    assert_eq!(thought.elapsed_time_ms(), Some(2000));
}
