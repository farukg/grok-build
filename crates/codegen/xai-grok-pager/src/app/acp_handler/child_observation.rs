use super::*;
use crate::app::agent_view::{AgentRole, ChildLink};

#[derive(Debug, Clone)]
pub(super) enum ChildObservation {
    Activity(Option<String>),
    Context { tokens_used: Option<u64>, context_window_tokens: Option<u64>, context_usage_pct: Option<u8> },
    Finished(Option<String>),
}

impl ChildObservation {
    pub(super) fn from_update(update: &XaiSessionUpdate, child_view: &AgentView) -> Option<Self> {
        match update {
            XaiSessionUpdate::ToolCallDeltaChunk { .. } => Some(Self::Activity(subagent_activity_label(child_view))),
            XaiSessionUpdate::SubagentProgress { tokens_used, context_window_tokens, context_usage_pct, .. } => Some(Self::Context {
                tokens_used: Some(*tokens_used),
                context_window_tokens: Some(*context_window_tokens),
                context_usage_pct: Some(*context_usage_pct),
            }),
            XaiSessionUpdate::AutoCompactCompleted { tokens_after, .. } => Some(Self::Context {
                tokens_used: Some(*tokens_after),
                context_window_tokens: None,
                context_usage_pct: None,
            }),
            XaiSessionUpdate::TurnCompleted { .. } => Some(Self::Finished(subagent_activity_label(child_view))),
            _ => None,
        }
    }
}

pub(super) fn observe_child(views: &mut crate::app::session_views::SessionViews, child: AgentId, observation: ChildObservation) {
    let Some(child_view) = views.get(&child) else { return };
    let AgentRole::Child(ChildLink { parent, .. }) = &child_view.role else { return };
    let parent = *parent;
    let child_sid = child_view.session.session_id.as_ref().map(|sid| sid.0.as_ref());
    let Some(child_sid) = child_sid else { return };
    let Some(parent_view) = views.get_mut(&parent) else { return };
    let Some(info) = parent_view.subagent_sessions.get_mut(child_sid) else { return };
    match observation {
        ChildObservation::Activity(label) | ChildObservation::Finished(label) => {
            info.attempt.activity_label = label;
        }
        ChildObservation::Context { tokens_used, context_window_tokens, context_usage_pct } => {
            if let Some(tokens) = tokens_used { info.attempt.tokens_used = Some(tokens); }
            if let Some(window) = context_window_tokens { info.attempt.context_window_tokens = Some(window); }
            if let Some(pct) = context_usage_pct { info.attempt.context_usage_pct = Some(pct); }
        }
    }
}
