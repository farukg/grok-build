use super::*;
use crate::acp::meta::SamplingMetadata;
use crate::app::agent_view::load_continuation::{LoadDelivery, with_load_delivery};
use xai_grok_shell::extensions::notification::RetryState;
use xai_grok_shell::extensions::sampling_lifecycle::{SamplingLifecycle, SamplingPhase};

enum SamplingUpdate<'a> {
    LegacyRetry(&'a RetryState),
    Lifecycle {
        lifecycle: &'a SamplingLifecycle,
        retry: Option<&'a RetryState>,
    },
    Malformed,
}

fn classify<'a>(
    update: &'a XaiSessionUpdate,
    meta: &'a NotificationMeta,
) -> Option<SamplingUpdate<'a>> {
    let retry = if let XaiSessionUpdate::RetryState(retry) = update {
        Some(retry)
    } else if matches!(update, XaiSessionUpdate::SamplingLifecycle { .. }) {
        None
    } else {
        return None;
    };
    Some(match &meta.sampling {
        SamplingMetadata::Malformed => SamplingUpdate::Malformed,
        SamplingMetadata::Legacy => match retry {
            Some(retry) => SamplingUpdate::LegacyRetry(retry),
            None => SamplingUpdate::Malformed,
        },
        SamplingMetadata::Owner(_) => SamplingUpdate::Malformed,
        SamplingMetadata::Lifecycle(lifecycle) => {
            let consistent = match (&lifecycle.phase, retry) {
                (SamplingPhase::RetryScheduled { .. }, Some(RetryState::Retrying { .. }))
                | (
                    SamplingPhase::Finished { .. },
                    Some(RetryState::Failed { .. } | RetryState::Exhausted { .. }),
                )
                | (
                    SamplingPhase::RetryScheduled { .. }
                    | SamplingPhase::DispatchStarted { .. }
                    | SamplingPhase::Finished { .. },
                    None,
                ) => true,
                (
                    SamplingPhase::RetryScheduled { .. },
                    Some(RetryState::Failed { .. } | RetryState::Exhausted { .. }),
                )
                | (
                    SamplingPhase::DispatchStarted { .. },
                    Some(
                        RetryState::Retrying { .. }
                        | RetryState::Failed { .. }
                        | RetryState::Exhausted { .. },
                    ),
                )
                | (SamplingPhase::Finished { .. }, Some(RetryState::Retrying { .. })) => false,
            };
            if consistent {
                SamplingUpdate::Lifecycle { lifecycle, retry }
            } else {
                SamplingUpdate::Malformed
            }
        }
    })
}

pub(super) fn handle_sampling_update(
    app: &mut AppView,
    id: AgentId,
    update: &XaiSessionUpdate,
    meta: &NotificationMeta,
) -> Option<bool> {
    if let XaiSessionUpdate::ToolCallDeltaChunk {
        name,
        tool_index,
        arguments_delta,
        ..
    } = update
    {
        return Some(handle_arguments_update(
            app,
            id,
            meta,
            ArgumentDelta {
                name: name.as_deref(),
                index: *tool_index,
                fragment: arguments_delta.as_deref(),
            },
        ));
    }
    let update = classify(update, meta)?;
    if matches!(update, SamplingUpdate::Malformed) {
        return Some(false);
    }
    let starts_inference = matches!(
        &update,
        SamplingUpdate::Lifecycle {
            lifecycle: SamplingLifecycle {
                phase: SamplingPhase::RetryScheduled { .. } | SamplingPhase::DispatchStarted { .. },
                ..
            },
            ..
        }
    );
    let child = is_child_view(app, id);
    let active = is_matched_agent_active(app, id)
        || app
            .agents
            .parent_of(id)
            .is_some_and(|parent| is_matched_agent_active(app, parent));
    let view = app.agents.get(&id)?;
    if !view.accepts_current_source(meta) {
        return Some(false);
    }
    if meta.is_replay {
        if !view.accepts_replayed_update() {
            return Some(false);
        }
    } else {
        if !starts_inference
            && meta.prompt_id.is_some()
            && meta.prompt_id != view.session.current_prompt_id
        {
            return Some(false);
        }
        if meta.event_seq.is_some_and(|seq| {
            view.last_applied_xai_event_seq
                .is_some_and(|last| seq <= last)
        }) {
            return Some(false);
        }
        if let SamplingUpdate::Lifecycle { lifecycle, .. } = &update
            && !view
                .live_admission_tracker()
                .accepts_sampling_lifecycle(lifecycle)
        {
            return Some(false);
        }
    }
    let admission = if meta.is_replay {
        admission::RootAdmission::Apply
    } else if child {
        match prompt_origin::child_prompt_admission(
            view,
            meta.prompt_id.as_deref(),
            meta.turn_start_ms,
        ) {
            prompt_origin::ChildPromptAdmission::Apply => admission::RootAdmission::Apply,
            prompt_origin::ChildPromptAdmission::Drop
            | prompt_origin::ChildPromptAdmission::Closed(_) => return Some(false),
        }
    } else {
        let prompt_meta = NotificationMeta {
            event_seq: None,
            ..meta.clone()
        };
        admission::root_admission(
            view,
            &prompt_meta,
            app.pending_running_adoptions
                .get(&id)
                .map(|pending| pending.prompt_id.as_str()),
            admission::UpdateScope::Turn,
        )
    };
    match admission {
        admission::RootAdmission::Drop | admission::RootAdmission::Buffer => return Some(false),
        admission::RootAdmission::Apply | admission::RootAdmission::AdoptViewer => {}
    }
    let delivery = if meta.is_replay {
        LoadDelivery::Historical
    } else {
        LoadDelivery::Live
    };
    with_load_delivery(app, id, delivery, |app| {
        if child && starts_inference && !meta.is_replay {
            crate::app::subagent::hydrate_resumed_child(&mut app.agents, id);
        }
        let view = app.agents.get_mut(&id)?;
        view.observe_current_source(meta);
        if meta.is_replay {
            view.mark_reload_replay_seen();
        } else {
            if matches!(admission, admission::RootAdmission::AdoptViewer) {
                view.attached_as_viewer = true;
                view.session.current_prompt_id = meta.prompt_id.clone();
                view.clear_follow_ups();
                if let Some(prompt) = meta.prompt_id.as_deref() {
                    view.flush_pending_follow_ups(prompt);
                }
            }
            if starts_inference {
                ack_prompt_from_update(view, meta);
                if child {
                    note_child_live_prompt(view, meta.prompt_id.as_deref());
                } else if let Some(prompt) = meta.prompt_id.as_deref() {
                    if is_wake_prompt(prompt) {
                        view.note_streaming_wake_turn(prompt);
                    } else if should_adopt_running_prompt(prompt)
                        && view.attached_as_viewer
                        && !view.session.state.is_turn_running()
                        && view.session.state.command_in_flight().is_none()
                        && !matches!(view.session.state, AgentState::TurnCancelling)
                    {
                        view.session.state = AgentState::TurnRunning;
                    }
                }
                if let Some(start) = meta.turn_start_ms {
                    view.turn_start_ms = Some(start);
                    view.turn_start_ms_prompt = meta.prompt_id.clone();
                    if view.turn_started_at.is_none() {
                        view.turn_started_at = Some(viewer_turn_anchor(Some(start)));
                    }
                }
            }
            if let Some(prompt) = meta.prompt_id.as_deref() {
                view.session.tracker.note_admitted_live_prompt(prompt);
            }
        }
        let retry = match update {
            SamplingUpdate::LegacyRetry(retry) => Some(retry),
            SamplingUpdate::Lifecycle { lifecycle, retry } => {
                if !meta.is_replay {
                    view.session.tracker.apply_sampling_lifecycle(
                        lifecycle,
                        retry,
                        &mut view.scrollback,
                    );
                    view.observe_current_sampling(lifecycle, meta);
                }
                retry
            }
            SamplingUpdate::Malformed => return Some(false),
        };
        if let Some(retry) = retry {
            super::session_notification::apply_retry_state(
                retry,
                &mut view.session,
                &mut view.scrollback,
                app.is_api_key_auth,
                meta.is_replay,
            );
        }
        if !meta.is_replay
            && let Some(seq) = meta.event_seq
        {
            view.last_applied_xai_event_seq = Some(seq);
        }
        if let Some(event) = meta.event_id.as_ref() {
            view.advance_last_seen_event_id(event.clone(), meta.event_seq);
        }
        let label = (!meta.is_replay && child).then(|| subagent_activity_label(view));
        app.agents.note_inline_content_changed(id);
        if let Some(label) = label {
            observe_child(&mut app.agents, id, ChildObservation::Activity(label));
        }
        Some(active)
    })
}

struct ArgumentDelta<'a> {
    name: Option<&'a str>,
    index: u32,
    fragment: Option<&'a str>,
}

fn handle_arguments_update(
    app: &mut AppView,
    id: AgentId,
    meta: &NotificationMeta,
    delta: ArgumentDelta<'_>,
) -> bool {
    if meta.is_replay || matches!(&meta.sampling, SamplingMetadata::Malformed) {
        return false;
    }
    let child = is_child_view(app, id);
    let active = is_matched_agent_active(app, id)
        || app
            .agents
            .parent_of(id)
            .is_some_and(|parent| is_matched_agent_active(app, parent));
    let Some(view) = app.agents.get(&id) else {
        return false;
    };
    if !view.accepts_current_source(meta) {
        return false;
    }
    if meta.event_seq.is_some_and(|seq| {
        view.last_applied_xai_event_seq
            .is_some_and(|last| seq <= last)
    }) || (meta.prompt_id.is_some() && meta.prompt_id != view.session.current_prompt_id)
    {
        return false;
    }
    if child {
        let row_live = app
            .agents
            .link_and_parent(id)
            .and_then(|(link, parent)| parent.subagent_sessions.get(&link.subagent_id))
            .is_some_and(|info| info.is_running());
        if !row_live {
            return false;
        }
    } else if view.running_wake_turn.is_some() || !view.session.state.is_turn_running() {
        return false;
    }
    let stream = meta.writing_stream();
    if !view
        .live_admission_tracker()
        .accepts_tool_arguments_stream(&stream)
    {
        return false;
    }
    with_load_delivery(app, id, LoadDelivery::Live, |app| {
        let Some(view) = app.agents.get_mut(&id) else {
            return false;
        };
        view.observe_current_source(meta);
        let changed = view
            .session
            .tracker
            .note_tool_call_arguments_delta_in_stream(delta.name, delta.index, stream);
        let has_fragment = delta.fragment.is_some_and(|fragment| !fragment.is_empty());
        if has_fragment {
            if child {
                view.last_active_at = Some(std::time::Instant::now());
            } else {
                note_first_turn_activity(view);
            }
        }
        if let Some(seq) = meta.event_seq {
            view.last_applied_xai_event_seq = Some(seq);
        }
        if let Some(event) = meta.event_id.as_ref() {
            view.advance_last_seen_event_id(event.clone(), meta.event_seq);
        }
        let label = subagent_activity_label(view);
        if child && (changed || has_fragment) {
            observe_child(&mut app.agents, id, ChildObservation::Activity(label));
        }
        (changed || has_fragment) && active
    })
}
