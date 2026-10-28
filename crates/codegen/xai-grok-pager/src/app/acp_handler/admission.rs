use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum UpdateScope {
    Turn,
    DrainedTool,
    BackgroundTool,
}

#[derive(Debug)]
pub(super) enum RootAdmission {
    Drop,
    Buffer,
    Apply,
    AdoptViewer,
}

pub(super) fn admits_delivery(view: &AgentView, meta: &NotificationMeta) -> bool {
    if !view.accepts_current_source(meta) { return false; }
    if meta.is_replay {
        return view.accepts_replayed_update();
    }
    !meta.event_seq.is_some_and(|seq| view.last_applied_event_seq.is_some_and(|last| seq <= last))
}

pub(super) fn root_admission(
    view: &AgentView,
    meta: &NotificationMeta,
    pending_adoption: Option<&str>,
    scope: UpdateScope,
) -> RootAdmission {
    if !admits_delivery(view, meta) {
        return RootAdmission::Drop;
    }
    if meta.is_replay || matches!(scope, UpdateScope::DrainedTool) {
        return RootAdmission::Apply;
    }
    let Some(pid) = meta.prompt_id.as_deref() else {
        if let (Some(start), Some(current)) = (meta.turn_start_ms, view.session.current_prompt_id.as_deref())
            && view.turn_start_ms_prompt.as_deref() == Some(current)
            && view.turn_start_ms.is_some_and(|live| live != start)
        {
            return RootAdmission::Drop;
        }
        return RootAdmission::Apply;
    };
    if view.is_rewound_prompt(pid) || view.finished_wake_prompts.contains(pid) {
        return RootAdmission::Drop;
    }
    if view.session.current_prompt_id.as_deref() == Some(pid) {
        return RootAdmission::Apply;
    }
    if !view.attached_as_viewer && pending_adoption == Some(pid) {
        if view.pending_adoption_updates.iter().any(|(_, _, pending)| {
            meta.event_seq.is_some_and(|seq| pending.event_seq == Some(seq))
                || meta.event_id.as_ref().is_some_and(|id| pending.event_id.as_ref() == Some(id))
        }) {
            return RootAdmission::Drop;
        }
        return RootAdmission::Buffer;
    }
    if view.is_self_originated_prompt(pid) || view.live_admission_tracker().has_admitted_live_prompt(pid) {
        return RootAdmission::Drop;
    }
    if !is_server_initiated_prompt(pid) {
        return RootAdmission::AdoptViewer;
    }
    if view.attached_as_viewer || view.session.current_prompt_id.is_none()
        || view.session.current_prompt_id.as_deref().is_some_and(is_server_initiated_prompt)
    {
        return RootAdmission::Apply;
    }
    RootAdmission::Drop
}

pub(super) fn commit_metadata(view: &mut AgentView, meta: &NotificationMeta) {
    view.observe_current_source(meta);
    if meta.is_replay {
        view.mark_reload_replay_seen();
    } else {
        view.late_replay_until = None;
        if let Some(seq) = meta.event_seq {
            view.last_applied_event_seq = Some(seq);
        }
    }
}

pub(super) fn update_scope(view: &AgentView, update: &acp::SessionUpdate) -> UpdateScope {
    if let acp::SessionUpdate::ToolCallUpdate(tool) = update {
        if view.session.bg_tool_call_to_task.contains_key(tool.tool_call_id.0.as_ref()) {
            return UpdateScope::BackgroundTool;
        }
        if view.session.tracker.drained_tool_entry(tool, &view.scrollback).is_some() {
            return UpdateScope::DrainedTool;
        }
    }
    UpdateScope::Turn
}

pub(super) fn has_text_fragment(update: &acp::SessionUpdate) -> bool {
    match update {
        acp::SessionUpdate::AgentMessageChunk(chunk) | acp::SessionUpdate::AgentThoughtChunk(chunk) => {
            matches!(&chunk.content, acp::ContentBlock::Text(text) if !text.text.is_empty())
        }
        _ => false,
    }
}
