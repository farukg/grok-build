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
    crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
    let affected = child_or_parent_active(app, child);
    let Some(child_view) = app.agents.get_mut(&child) else {
        return false;
    };
    ack_prompt_from_update(child_view, meta);
    if let Some(tokens) = meta.total_tokens {
        confirm_context_used(child_view, tokens);
    }
    if let acp::SessionUpdate::UsageUpdate(ref usage) = update {
        child_view.apply_context_used(usage.used, usage.size);
    }
    let ended = !meta.is_replay
        && meta.prompt_id.as_deref().is_some_and(|pid| {
            child_view.ended_child_prompt_ids.contains(pid)
                || child_view.superseded_child_prompt_ids.contains(pid)
        });
    if !ended {
        let is_live = !meta.is_replay && !child_view.session.loading_replay;
        let apply = !is_live
            || note_child_live_prompt(
                child_view,
                meta.prompt_id.as_deref(),
                meta.turn_start_ms,
                meta.is_replay,
            );
        if apply {
            if is_live {
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
            child_view
                .session
                .handle_update(update, meta, &mut child_view.scrollback);
            for entry_id in child_view.session.tracker.take_pending_edit_hl() {
                child_view.submit_edit_highlight(entry_id);
            }
        }
    }
    let label = subagent_activity_label(child_view);
    observe_child(&mut app.agents, child, ChildObservation::Activity(label));
    affected
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
            | XaiSessionUpdate::MemoryFlushCompleted { .. }
            | XaiSessionUpdate::MemoryDreamCompleted { .. }
            | XaiSessionUpdate::MemorySessionSaved { .. }
            | XaiSessionUpdate::ToolCallDeltaChunk { .. }
            | XaiSessionUpdate::TurnCompleted { .. }
            | XaiSessionUpdate::ModelServed { .. }
    )
}

/// Events like compaction, retry, and memory flush are emitted by the child's `acp_session` with the *child's* `session_id`.
pub(super) fn apply_child_xai_update(
    app: &mut AppView,
    child: AgentId,
    update: XaiSessionUpdate,
    is_api_key_auth: bool,
    meta: Option<&serde_json::Value>,
) -> bool {
    let affected = child_or_parent_active(app, child);
    match update {
        XaiSessionUpdate::AutoCompactStarted { .. }
        | XaiSessionUpdate::AutoCompactCompleted { .. }
        | XaiSessionUpdate::AutoCompactFailed { .. }
        | XaiSessionUpdate::AutoCompactCancelled { .. }
        | XaiSessionUpdate::RetryState(_)
        | XaiSessionUpdate::MemoryFlushCompleted { .. }
        | XaiSessionUpdate::MemoryDreamCompleted { .. }
        | XaiSessionUpdate::MemorySessionSaved { .. } => {
            crate::app::subagent::hydrate_resumed_child(&mut app.agents, child);
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
            let changed = apply_child_view_session_event(child_view, &update, is_api_key_auth);
            if let XaiSessionUpdate::AutoCompactCompleted { tokens_after, .. } = update {
                observe_child(
                    &mut app.agents,
                    child,
                    ChildObservation::Compacted { tokens_after },
                );
            }
            changed && affected
        }
        XaiSessionUpdate::ToolCallDeltaChunk {
            ref name,
            tool_index,
            ..
        } => {
            let row_live = app
                .agents
                .link_and_parent(child)
                .and_then(|(link, parent)| parent.subagent_sessions.get(&link.subagent_id))
                .is_some_and(|info| info.is_running());
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
            if child_view.session.loading_replay || !row_live {
                return false;
            }
            if !child_view
                .session
                .tracker
                .note_tool_call_arguments_delta(name.as_deref(), tool_index)
            {
                return false;
            }
            let label = subagent_activity_label(child_view);
            observe_child(&mut app.agents, child, ChildObservation::Activity(label));
            affected
        }
        XaiSessionUpdate::TurnCompleted {
            prompt_id,
            stop_reason,
            agent_result,
            error_kind,
            elapsed_ms,
            ..
        } => {
            if NotificationMeta::from_json(meta.and_then(|v| v.as_object())).is_replay {
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
                let label = subagent_activity_label(child_view);
                observe_child(&mut app.agents, child, ChildObservation::Activity(label));
            }
            finished && affected
        }
        XaiSessionUpdate::ModelServed { route } => {
            let Some(child_view) = app.agents.get_mut(&child) else {
                return false;
            };
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
