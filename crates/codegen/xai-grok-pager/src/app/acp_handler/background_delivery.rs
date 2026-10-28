use super::*;
use crate::app::agent_view::load_continuation::{LoadDelivery, with_load_delivery};

enum BackgroundKind { Started, Completed }

pub(super) fn apply(notification: SessionNotification, mut meta: NotificationMeta, app: &mut AppView) -> bool {
    let (kind, task) = if let XaiSessionUpdate::TaskBackgrounded { task_id, .. } = &notification.update {
        (BackgroundKind::Started, task_id.clone())
    } else if let XaiSessionUpdate::TaskCompleted { task_snapshot, .. } = &notification.update {
        (BackgroundKind::Completed, task_snapshot.task_id.clone())
    } else { return false; };
    let ownership = match xai_grok_shell::extensions::tool_timing::ownership_from_meta(notification.meta.as_ref().and_then(serde_json::Value::as_object)) {
        Ok(ownership) => ownership,
        Err(_) => return false,
    };
    let identity = match ownership {
        xai_grok_shell::extensions::tool_timing::ToolCallOwnership::Unknown => None,
        ownership @ xai_grok_shell::extensions::tool_timing::ToolCallOwnership::Registered { .. } => {
            let Some(identity) = crate::acp::tool_meta::ToolIdentity::from_ownership(&ownership) else { return false; };
            if let XaiSessionUpdate::TaskBackgrounded { tool_call_id, .. } = &notification.update
                && identity.provider != *tool_call_id { return false; }
            Some(identity)
        }
    };
    if let Some(identity) = &identity {
        if let crate::acp::tool_meta::ToolSource::Sampling(owner) = &identity.source
            && (meta.prompt_id.as_ref().is_some_and(|prompt| prompt != &owner.prompt_id)
                || meta.sampling_owner().is_some_and(|stamped| stamped != owner)) { return false; }
        meta.tool = crate::acp::tool_meta::ToolMetadata::Owned {
            identity: identity.clone(), timing: xai_grok_shell::extensions::tool_timing::ToolExecutionTiming::Unknown,
        };
    }
    let Some(SessionMatch(id)) = find_session_match(app, &notification.session_id) else { return false; };
    if !app.agents.get(&id).is_some_and(|view| view.accepts_current_source(&meta) && view.accepts_background_delivery(&meta)
        && view.live_admission_tracker().accepts_known_tool_identity(&meta)) { return false; }
    let delivery = if meta.is_replay { LoadDelivery::Historical } else { LoadDelivery::Live };
    with_load_delivery(app, id, delivery, |app| {
        let changed = match kind {
            BackgroundKind::Started => background::apply_task_backgrounded(notification, &meta, app),
            BackgroundKind::Completed => background::apply_task_completed(notification, &meta, app),
        };
        if let Some(identity) = &identity && let Some(view) = app.agents.get_mut(&id) {
            view.session.tracker.bind_background_tool(identity, &task, meta.is_replay);
        }
        if !meta.is_replay && let Some(view) = app.agents.get_mut(&id) {
            view.observe_current_source(&meta);
            if let Some(entry) = view.session.bg_tasks.get(&task).and_then(|task| task.scrollback_entry_id) {
                view.protect_live_entry(entry);
            }
        }
        changed
    })
}
