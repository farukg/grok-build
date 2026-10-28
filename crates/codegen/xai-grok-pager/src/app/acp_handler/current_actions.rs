use super::*;
use crate::app::agent_view::current_actions::CurrentActionsContext;
use crate::app::agent_view::load_continuation::{LoadDelivery, with_load_delivery};
use xai_grok_shell::session::current_actions::{CurrentActionsEvent, CurrentActionsSnapshot};

pub(super) fn handle_transition(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    use xai_grok_shell::extensions::current_actions::{DeliveryRole, transition_delivery};
    let Ok((event, role)) = transition_delivery(notif) else { return false; };
    let delivery = match &role {
        DeliveryRole::Unavailable | DeliveryRole::Historical { .. } => return false,
        DeliveryRole::Live { .. } | DeliveryRole::LiveUnbound { .. } => LoadDelivery::Live,
    };
    let Some(id) = app.agents.find_by_session_id(&event.session_id) else { return false; };
    let Some(view) = app.agents.get(&id) else { return false; };
    if !view.accepts_publication_role(&role) { return false; }
    let Some(context) = view.current_actions_context().cloned() else { return false; };
    apply_transition(app, id, &context, delivery, event)
}

pub(super) fn handle_generation_end(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    let Ok(end) = serde_json::from_str::<xai_grok_shell::extensions::current_actions::GenerationEnd>(notif.params.get()) else { return false; };
    let Some(id) = app.agents.find_by_session_id(&end.snapshot.binding.session_id) else { return false; };
    if let Some(view) = app.agents.get_mut(&id) { view.receive_generation_end(end); }
    false
}

pub(super) fn handle_discard(notif: &acp::ExtNotification, app: &mut AppView) -> bool {
    use xai_grok_shell::extensions::current_actions::{DiscardedNotification, DiscardedRecord};
    let Ok(record) = serde_json::from_str::<DiscardedRecord>(notif.params.get()) else { return false; };
    let (session, mut meta) = match &record.notification {
        DiscardedNotification::Acp(notification) => (notification.session_id.0.as_ref(), NotificationMeta::from_json(notification.meta.as_ref())),
        DiscardedNotification::Xai(notification) => (notification.session_id.0.as_ref(), NotificationMeta::from_xai(notification.meta.as_ref(), &notification.update)),
    };
    let Some(id) = app.agents.find_by_session_id(session) else { return false; };
    let Some(view) = app.agents.get_mut(&id) else { return false; };
    if !view.accepts_publication_role(&record.role) { return false; }
    meta.is_replay = matches!(record.role, xai_grok_shell::extensions::current_actions::DeliveryRole::Historical { .. });
    meta.delivery_role = record.role;
    if !view.accepts_current_source(&meta) { return false; }
    view.observe_current_frontier(&meta);
    false
}

pub(crate) fn apply_transition(
    app: &mut AppView, id: AgentId, context: &CurrentActionsContext,
    delivery: LoadDelivery, event: CurrentActionsEvent,
) -> bool {
    if matches!(delivery, LoadDelivery::Historical) { return false; }
    if !app.agents.get(&id).is_some_and(|view| view.accepts_current_actions_context(context)
        && view.accepts_current_actions_event(&event)) { return false; }
    let changed = with_load_delivery(app, id, LoadDelivery::Live, |app| {
        app.agents.get_mut(&id).is_some_and(|view| view.apply_current_actions_event(event))
    });
    if changed && is_child_view(app, id) {
        let label = app.agents.get(&id).and_then(subagent_activity_label);
        observe_child(&mut app.agents, id, ChildObservation::Activity(label));
    }
    changed && (is_matched_agent_active(app, id)
        || app.agents.parent_of(id).is_some_and(|parent| is_matched_agent_active(app, parent)))
}

pub(crate) fn apply_load_publication(app: &mut AppView, id: AgentId, publication: &crate::app::effects::current_actions::LoadPublication) -> bool {
    use crate::app::agent_view::CurrentActionsReadContext;
    if !app.agents.get(&id).is_some_and(|view| view.current_actions_binding().as_ref() == Some(&publication.binding)) { return false; }
    match publication.context() {
        CurrentActionsReadContext::Attach(context) => {
            if !app.agents.get_mut(&id).is_some_and(|view| view.bind_current_actions(context.clone())) { return false; }
            apply_snapshot(app, id, &context, publication.snapshot.clone())
        }
        CurrentActionsReadContext::Observe(context) => apply_snapshot(app, id, &context, publication.snapshot.clone()),
        CurrentActionsReadContext::Unavailable(binding) => app.agents.get_mut(&id).is_some_and(|view| view.current_actions_unavailable_for_binding(&binding)),
    }
}

pub(crate) fn apply_snapshot(app: &mut AppView, id: AgentId, context: &CurrentActionsContext, snapshot: CurrentActionsSnapshot) -> bool {
    if matches!(snapshot, CurrentActionsSnapshot::Unavailable) {
        return app.agents.get_mut(&id).is_some_and(|view| view.current_actions_unavailable(context));
    }
    if !app.agents.get(&id).is_some_and(|view| view.accepts_current_actions_snapshot(context, &snapshot)) { return false; }
    with_load_delivery(app, id, LoadDelivery::Live, |app| {
        app.agents.get_mut(&id).is_some_and(|view| view.apply_current_actions_snapshot(context, snapshot))
    })
}
