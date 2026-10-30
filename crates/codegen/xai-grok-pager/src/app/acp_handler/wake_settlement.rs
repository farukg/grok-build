use super::*;
use xai_grok_shell::session::{WakeCancelRequest, WakeCancelResponse, WakeCancelSettlement};

pub(crate) fn apply_wake_cancel_settlement(
    agent: &mut AgentView,
    request: &WakeCancelRequest,
    response: WakeCancelResponse,
) {
    let context = crate::app::agent_view::CurrentActionsContext {
        binding: request.context.binding.clone(),
        incarnation: request.context.incarnation,
    };
    if response.context != request.context
        || response.prompt_id != request.prompt_id
        || !agent.accepts_current_actions_context(&context)
        || agent.finished_wake_prompts.contains(&request.prompt_id)
        || !agent.running_wake_turn.as_ref().is_some_and(|wake| {
            wake.prompt_id == request.prompt_id && wake.cancel_sent
        })
    {
        return;
    }
    let terminal = match response.settlement {
        WakeCancelSettlement::Unresolved { reason } => {
            tracing::warn!(?reason, prompt_id = %request.prompt_id, "wake cancel remains unresolved");
            return;
        }
        WakeCancelSettlement::RemovedFromQueue => None,
        WakeCancelSettlement::Terminal { notification } => {
            if notification.session_id.0.as_ref() != request.context.binding.session_id {
                return;
            }
            let XaiSessionUpdate::TurnCompleted { ref prompt_id, .. } = notification.update else {
                return;
            };
            if prompt_id != &request.prompt_id {
                return;
            }
            Some(notification)
        }
    };
    if agent.pending_cancel_resend.as_ref().is_some_and(|pending| {
        pending.prompt_id.as_deref() == Some(request.prompt_id.as_str())
    }) {
        agent.pending_cancel_resend = None;
    }
    agent.settle_current_actions_prompt(&request.prompt_id);
    if agent.session.state.is_busy() {
        agent.running_wake_turn = None;
        agent.finished_wake_prompts.insert(request.prompt_id.clone());
        return;
    }
    match terminal {
        Some(notification) => {
            let XaiSessionUpdate::TurnCompleted { stop_reason, agent_result, .. } =
                notification.update else {
                    return;
                };
            let meta_str = |key: &str| notification.meta.as_ref()
                .and_then(|meta| meta.get(key)).and_then(serde_json::Value::as_str);
            super::prompt_origin::close_wake_turn_from_terminal(
                agent,
                &request.prompt_id,
                super::prompt_origin::WakeTerminal {
                    stop_reason: &stop_reason,
                    agent_result: agent_result.as_deref(),
                    cancel_trigger: meta_str(crate::app::turn_completion::CANCEL_TRIGGER_KEY),
                    cancellation_category: meta_str(
                        crate::app::turn_completion::CANCELLATION_CATEGORY_KEY,
                    ),
                },
            );
        }
        None => {
            agent.running_wake_turn = None;
            agent.finished_wake_prompts.insert(request.prompt_id.clone());
            agent.session.tracker.finish_turn(&mut agent.scrollback);
            crate::app::turn_completion::push_turn_terminal_marker(agent, None);
        }
    }
}
