use super::*;
use crate::app::agent_view::{AgentRole, ChildLink};

#[derive(Debug, Clone)]
pub(super) enum ChildObservation {
    ContextTokens { tokens_used: u64, context_usage_pct: Option<u8> },
    Activity(Option<String>),
    TurnFinished(Option<String>),
}

pub(super) fn classify(update: &XaiSessionUpdate, child: &AgentView) -> Option<ChildObservation> {
    match update {
        XaiSessionUpdate::AutoCompactCompleted { tokens_after, .. } => {
            let context_usage_pct = child.context_state.as_ref().map(|context| context.usage_pct);
            Some(ChildObservation::ContextTokens { tokens_used: *tokens_after, context_usage_pct })
        }
        XaiSessionUpdate::ToolCallDeltaChunk { .. } => Some(ChildObservation::Activity(
            subagent_activity_label(child),
        )),
        XaiSessionUpdate::TurnCompleted { .. } => Some(ChildObservation::TurnFinished(
            subagent_activity_label(child),
        )),
        _ => None,
    }
}

pub(super) fn observe_child(
    views: &mut crate::app::session_views::SessionViews,
    child: AgentId,
    observation: ChildObservation,
) {
    let Some(child_view) = views.get(&child) else { return };
    let AgentRole::Child(ChildLink { parent, .. }) = &child_view.role else { return };
    let Some(child_sid) = child_view.session.session_id.as_ref().map(|sid| sid.0.as_ref()) else {
        return;
    };
    let parent = *parent;
    let Some(parent_view) = views.get_mut(&parent) else { return };
    match observation {
        ChildObservation::ContextTokens { tokens_used, context_usage_pct } => {
            if let Some(info) = parent_view.subagent_sessions.get_mut(child_sid) {
                info.attempt.tokens_used = Some(tokens_used);
                if let Some(pct) = context_usage_pct {
                    info.attempt.context_usage_pct = Some(pct);
                }
            }
        }
        ChildObservation::Activity(label) | ChildObservation::TurnFinished(label) => {
            super::subagent_activity::sync_child_activity(parent_view, child_sid, label);
        }
    }
}
