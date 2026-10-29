//! Updates addressed to a child session land on the child's own view; its parent only mirrors them on the child's subagent row.
use super::session_notification::terminal_meta_str;
use super::*;
use crate::app::agent_view::AgentRole;
use crate::app::session_views::SessionViews;

/// What a child session's update changes on its parent's subagent row.
pub(super) enum ChildObservation {
    /// The child compacted its context down to this many tokens.
    Compacted {
        tokens_after: u64,
    },
    /// The child reported its context usage.
    ContextUsed {
        used: u64,
        window: u64,
    },
    Activity(Option<String>),
}

pub(super) fn is_child_view(app: &AppView, id: AgentId) -> bool {
    app.agents.get(&id).is_some_and(|view| match view.role {
        AgentRole::Root => false,
        AgentRole::Child(_) => true,
    })
}

/// The child's own view and its parent's (which paints the child's row) both redraw.
fn child_or_parent_active(app: &AppView, child: AgentId) -> bool {
    is_matched_agent_active(app, child)
        || app
            .agents
            .parent_of(child)
            .is_some_and(|parent| is_matched_agent_active(app, parent))
}

pub(super) fn observe_child(
    views: &mut SessionViews,
    child: AgentId,
    observation: ChildObservation,
) {
    let Some((link, parent)) = views.link_and_parent_mut(child) else {
        return;
    };
    match observation {
        ChildObservation::Compacted { tokens_after } => {
            if let Some(info) = parent.subagent_sessions.get_mut(&link.subagent_id) {
                info.attempt.tokens_used = Some(tokens_after);
                if let Some(cw) = info.attempt.context_window_tokens.filter(|&cw| cw > 0) {
                    info.attempt.context_usage_pct =
                        Some(xai_token_estimation::usage_percentage_u8(tokens_after, cw));
                }
            }
        }
        ChildObservation::ContextUsed { used, window } => {
            if let Some(info) = parent.subagent_sessions.get_mut(&link.subagent_id) {
                info.attempt.tokens_used = Some(used);
                if window > 0 {
                    info.attempt.context_window_tokens = Some(window);
                    info.attempt.context_usage_pct =
                        Some(xai_token_estimation::usage_percentage_u8(used, window));
                }
            }
        }
        ChildObservation::Activity(label) => {
            sync_subagent_activity(parent, &link.subagent_id, label);
        }
    }
}

/// A child's `session/update`: the subagent runner drives its turns, so prompt ids are adopted, never gated like a root's.
pub(super) fn apply_child_acp_update(
    app: &mut AppView,
    child: AgentId,
    update: acp::SessionUpdate,
    meta: &NotificationMeta,
) -> bool {
    let affected = child_or_parent_active(app, child);
    let Some(child_view) = app.agents.get(&child) else {
        return false;
    };
    if meta.is_replay
        && !child_view
            .live_admission_tracker()
            .accepts_known_tool_identity(meta)
    {
        return false;
    }
    if !child_view.accepts_current_source(meta)
        || !child_view
            .admission_tracker(meta)
            .accepts_sampling_acp(&update, meta)
    {
        return false;
    }
    match child_view.admission_tracker(meta).tool_admission(
        &update,
        meta,
        child_view.session.current_prompt_id.as_deref(),
    ) {
        crate::acp::tracker::ToolAdmission::Reject => return false,
        crate::acp::tracker::ToolAdmission::EntryOnly => {
            if meta.is_replay && !child_view.accepts_replayed_update() {
                return false;
            }
            let current = child_view.session.current_prompt_id.clone();
            let current_tool = child_view
                .admission_tracker(meta)
                .tool_observation_is_current(&update, meta, current.as_deref());
            if child_view.has_stashed_tool_observation(&update, meta) {
                let Some(view) = app.agents.get_mut(&child) else {
                    return false;
                };
                return view.apply_stashed_tool_observation(update, meta) && affected;
            }
            if !meta.is_replay && !current_tool {
                let Some(view) = app.agents.get_mut(&child) else {
                    return false;
                };
                return view.handle_admitted_tool_observation(update, meta, current.as_deref())
                    && affected;
            }
            let delivery = if meta.is_replay {
                super::super::agent_view::load_continuation::LoadDelivery::Historical
            } else {
                super::super::agent_view::load_continuation::LoadDelivery::Live
            };
            return super::super::agent_view::load_continuation::with_load_delivery(
                app,
                child,
                delivery,
                |app| {
                    let Some(view) = app.agents.get_mut(&child) else {
                        return false;
                    };
                    view.handle_admitted_tool_observation(update, meta, current.as_deref())
                        && affected
                },
            );
        }
        crate::acp::tracker::ToolAdmission::Turn => {}
    }
    if !admission::admits_delivery(child_view, meta) {
        return false;
    }
    if child_view.has_stashed_tool_observation(&update, meta) {
        let Some(view) = app.agents.get_mut(&child) else {
            return false;
        };
        return view.apply_stashed_tool_observation(update, meta) && affected;
    }
    let Some(child_view) = app.agents.get(&child) else {
        return false;
    };
    if !admission::admits_delivery(child_view, meta) {
        return false;
    }
    let scope = admission::update_scope(child_view, &update, meta);
    let prompt_admission = if meta.is_replay || matches!(scope, admission::UpdateScope::ToolTerminal)
    {
        prompt_origin::ChildPromptAdmission::Apply
    } else {
        prompt_origin::child_prompt_admission(
            child_view,
            meta.prompt_id.as_deref(),
            meta.turn_start_ms,
        )
    };
    match prompt_admission {
        prompt_origin::ChildPromptAdmission::Drop => return false,
        prompt_origin::ChildPromptAdmission::Closed(pid) => {
            // Retain the identity learned for a previously nameless closed turn, not its metadata.
            if let Some(pid) = pid
                && let Some(child_view) = app.agents.get_mut(&child)
            {
                child_view.ended_child_prompt_ids.insert(pid.to_owned());
                if child_view.unidentified_child_turn_closed_prompt.is_none() {
                    child_view.unidentified_child_turn_closed_prompt = Some(pid.to_owned());
                }
            }
            return false;
        }
        prompt_origin::ChildPromptAdmission::Apply => {}
    }
    if matches!(scope, admission::UpdateScope::ToolTerminal) {
        let Some(child_view) = app.agents.get_mut(&child) else {
            return false;
        };
        let changed = match &update {
            acp::SessionUpdate::ToolCallUpdate(tool) => {
                child_view.apply_terminal_tool_update(tool, meta)
            }
            _ => false,
        };
        return changed && affected;
    }
    let delivery = delivery::AcpDelivery::classify(&update, meta);
    delivery.apply(app, child, |app| {
        if !meta.is_replay {
            crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
        }
        let Some(child_view) = app.agents.get_mut(&child) else {
            return false;
        };
        admission::commit_metadata(child_view, meta);
        let detached_tool = !matches!(scope, admission::UpdateScope::Turn);
        if !detached_tool {
            ack_prompt_from_update(child_view, meta);
            if let Some(tokens) = meta.total_tokens {
                confirm_context_used(child_view, tokens);
            }
        }
        let context_used = match &update {
            acp::SessionUpdate::UsageUpdate(usage) => {
                child_view.apply_context_used(usage.used, usage.size);
                (usage.used > 0).then_some(ChildObservation::ContextUsed {
                    used: usage.used,
                    window: usage.size,
                })
            }
            _ => None,
        };
        let is_live = !meta.is_replay;
        if is_live && !detached_tool {
            note_child_live_prompt(child_view, meta.prompt_id.as_deref());
            if let Some(ts) = meta.turn_start_ms {
                let named = meta.prompt_id.as_deref().is_some_and(|pid| !pid.is_empty());
                if named
                    || child_view.turn_start_ms_prompt.is_none()
                    || child_view.turn_start_ms == Some(ts)
                {
                    child_view.turn_start_ms = Some(ts);
                    if named {
                        child_view.turn_start_ms_prompt = meta.prompt_id.clone();
                    }
                }
            }
            backdate_child_turn_clock(child_view);
        }
        if is_live
            && !detached_tool
            && (admission::has_text_fragment(&update)
                || (!child_view.session.loading_replay
                    && (crate::acp::tracker::is_agent_output_update(&update)
                        || matches!(&update, acp::SessionUpdate::UserMessageChunk(_)))))
        {
            child_view.last_active_at = Some(std::time::Instant::now());
        }
        let content_before = child_view.scrollback.content_generation();
        let source_before = child_view.inline_sources.signal.revision();
        let routed_stdout = if let acp::SessionUpdate::ToolCallUpdate(ref tcu) = update {
            route_bg_task_stdout(tcu, child_view, meta)
        } else {
            false
        };
        if !routed_stdout {
            child_view.handle_admitted_acp_update(update, meta);
        }
        let content_applied = child_view.scrollback.content_generation() != content_before
            || child_view.inline_sources.signal.revision() != source_before;
        for entry_id in child_view.session.tracker.take_pending_edit_hl() {
            child_view.submit_edit_highlight(entry_id);
        }
        let label = subagent_activity_label(child_view);
        if content_applied {
            app.agents.note_inline_content_changed(child);
        }
        if let Some(observation) = context_used {
            observe_child(&mut app.agents, child, observation);
        }
        if !meta.is_replay {
            observe_child(&mut app.agents, child, ChildObservation::Activity(label));
        }
        affected
    })
}

/// The xAI updates whose meaning differs for a child: its turn ends through the runner, and its row mirrors activity and context.
pub(super) fn is_child_turn_update(update: &XaiSessionUpdate) -> bool {
    matches!(
        update,
        XaiSessionUpdate::AutoCompactStarted { .. }
            | XaiSessionUpdate::AutoCompactCompleted { .. }
            | XaiSessionUpdate::AutoCompactFailed { .. }
            | XaiSessionUpdate::AutoCompactCancelled { .. }
            | XaiSessionUpdate::RetryState(_)
            | XaiSessionUpdate::SamplingLifecycle { .. }
            | XaiSessionUpdate::MemoryFlushCompleted { .. }
            | XaiSessionUpdate::MemoryDreamCompleted { .. }
            | XaiSessionUpdate::MemorySessionSaved { .. }
            | XaiSessionUpdate::HookAnnotation { .. }
            | XaiSessionUpdate::ToolCallDeltaChunk { .. }
            | XaiSessionUpdate::TurnCompleted { .. }
            | XaiSessionUpdate::ModelServed { .. }
    )
}

/// Events like compaction, retry, and memory flush are emitted by the child's `acp_session` with the *child's* `session_id`.
pub(super) fn apply_child_xai_update(
    app: &mut AppView,
    child: AgentId,
    notification: SessionNotification,
    metadata: &NotificationMeta,
) -> bool {
    let is_api_key_auth = app.is_api_key_auth;
    let update = notification.update;
    let meta = notification.meta.as_ref();
    if let Some(changed) = sampling::handle_sampling_update(app, child, &update, metadata) {
        return changed;
    }
    let affected = child_or_parent_active(app, child);
    match update {
        XaiSessionUpdate::AutoCompactStarted { .. }
        | XaiSessionUpdate::AutoCompactCompleted { .. }
        | XaiSessionUpdate::AutoCompactFailed { .. }
        | XaiSessionUpdate::AutoCompactCancelled { .. }
        | XaiSessionUpdate::MemoryFlushCompleted { .. }
        | XaiSessionUpdate::MemoryDreamCompleted { .. }
        | XaiSessionUpdate::MemorySessionSaved { .. }
        | XaiSessionUpdate::HookAnnotation { .. } => {
            if !metadata.is_replay {
                crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
            }
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
            let historical = metadata.is_replay;
            if !historical
                && (metadata.event_seq.is_some_and(|seq| {
                    child_view
                        .last_applied_xai_event_seq
                        .is_some_and(|last| seq <= last)
                }) || (metadata.prompt_id.is_some()
                    && metadata.prompt_id != child_view.session.current_prompt_id))
            {
                return false;
            }
            child_view.observe_current_source(metadata);
            if !historical && let Some(seq) = metadata.event_seq {
                child_view.last_applied_xai_event_seq = Some(seq);
            }
            let last_active_at = child_view.last_active_at;
            let changed = super::session_notification::apply_child_view_session_event_with_replay(
                child_view,
                &update,
                is_api_key_auth,
                historical,
            );
            if historical {
                child_view.last_active_at = last_active_at;
            }
            if let XaiSessionUpdate::AutoCompactCompleted { tokens_after, .. } = update {
                observe_child(
                    &mut app.agents,
                    child,
                    ChildObservation::Compacted { tokens_after },
                );
            }
            if changed {
                app.agents.note_inline_content_changed(child);
            }
            changed && affected
        }
        XaiSessionUpdate::TurnCompleted {
            prompt_id,
            stop_reason,
            agent_result,
            error_kind,
            elapsed_ms,
            ..
        } => {
            if metadata.is_replay {
                return false;
            }
            crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
            let finished = super::super::turn_completion::finalize_child_view_turn(
                child_view,
                super::super::turn_completion::TerminalSignal {
                    prompt_id: Some(&prompt_id),
                    stop_reason: Some(&stop_reason),
                    agent_result: agent_result.as_deref(),
                    cancel_trigger: terminal_meta_str(
                        meta,
                        super::super::turn_completion::CANCEL_TRIGGER_KEY,
                    ),
                    cancellation_category: terminal_meta_str(
                        meta,
                        super::super::turn_completion::CANCELLATION_CATEGORY_KEY,
                    ),
                    cancellation_context: meta.and_then(|m| {
                        m.get(super::super::turn_completion::CANCELLATION_CONTEXT_KEY)
                    }),
                    error_kind: crate::app::error_display::wire_error_kind(error_kind.as_deref()),
                },
                elapsed_ms,
            );
            if finished {
                child_view.settle_current_actions_prompt(&prompt_id);
                let label = subagent_activity_label(child_view);
                observe_child(&mut app.agents, child, ChildObservation::Activity(label));
                app.agents.note_inline_content_changed(child);
            }
            finished && affected
        }
        XaiSessionUpdate::ModelServed { route } => {
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
            if metadata.is_replay {
                return false;
            }
            child_view.observe_current_source(metadata);
            child_view.session.models.served_route = Some(route);
            affected
        }
        _ => false,
    }
}

/// A child's `x.ai/session/prompt_complete`: the runner's terminal for the child's turn.
pub(super) fn complete_child_prompt(
    app: &mut AppView,
    child: AgentId,
    signal: super::super::turn_completion::TerminalSignal<'_>,
) -> bool {
    crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
    let affected = child_or_parent_active(app, child);
    let Some(child_view) = app.agents.get_mut(&child) else {
        return false;
    };
    let finished =
        super::super::turn_completion::finalize_child_view_turn(child_view, signal, None);
    if finished {
        let label = subagent_activity_label(child_view);
        observe_child(&mut app.agents, child, ChildObservation::Activity(label));
    }
    finished && affected
}
