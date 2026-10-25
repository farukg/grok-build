use crate::app::agent_view::AgentRole;
use crate::app::app_view::AppView;
use crate::views::dashboard::child_layout::ChildLayoutUpdate;
use crate::views::dashboard::{DashboardRowId, PersistedRowId};
use std::collections::BTreeMap;

pub(super) fn child_rows(app: &AppView) -> BTreeMap<String, DashboardRowId> {
    let mut rows = BTreeMap::new();
    for entry in app.dashboard_roster() {
        if entry.session_kind.as_deref() == Some(crate::app::roster::SESSION_KIND_SUBAGENT) {
            let id = app
                .agents
                .find_by_session_id(&entry.session_id)
                .map(DashboardRowId::TopLevel)
                .unwrap_or_else(|| DashboardRowId::Roster {
                    session_id: entry.session_id.clone(),
                });
            rows.insert(
                entry.session_id.clone(),
                id,
            );
        }
    }
    for (id, view) in app.agents.all() {
        match &view.role {
            AgentRole::Root => {}
            AgentRole::Child(_) => {
                if let Some(session) = &view.session.session_id {
                    rows.insert(
                        session.0.to_string(),
                        DashboardRowId::TopLevel(id),
                    );
                }
            }
        }
    }
    rows
}

pub(super) fn refresh(app: &mut AppView) {
    let attached: Vec<_> = app
        .dashboard_roster()
        .iter()
        .filter(|entry| {
            entry.session_kind.as_deref() == Some(crate::app::roster::SESSION_KIND_SUBAGENT)
        })
        .filter_map(|entry| {
            app.agents.find_by_session_id(&entry.session_id).map(|id| {
                (id, entry.session_id.clone(), entry.parent_session_id.clone())
            })
        })
        .collect();
    for (id, session_id, parent) in attached {
        crate::app::agent_view::stamp_child_role_for_session(
            &mut app.agents, id, &session_id, parent.as_deref(),
        );
    }
    if app.dashboard.is_none() {
        return;
    }
    let rows = child_rows(app);
    let saved = super::persisted_dashboard(&mut app.dashboard_persisted);
    let row_for = |key: &PersistedRowId| match key {
        PersistedRowId::TopLevel { session_id } => rows.get(session_id).cloned(),
    };
    let pins: Vec<_> = saved.pinned.iter().filter_map(&row_for).collect();
    let order: Vec<_> = saved.reorder.iter().filter_map(row_for).collect();
    let scoped = |id: &DashboardRowId| match id {
        DashboardRowId::TopLevel(_) => rows.values().any(|row| row == id),
        DashboardRowId::Roster { session_id } | DashboardRowId::Workspace { session_id } => {
            rows.contains_key(session_id)
        }
    };
    if let Some(dashboard) = app.dashboard.as_mut() {
        dashboard.pinned.retain(|id| !scoped(id));
        dashboard.pinned.extend(pins);
        dashboard.reorder.retain(|id| !scoped(id));
        dashboard.reorder.extend(order);
        dashboard.child_rows = rows;
    }
}

pub(super) fn snapshot(app: &AppView) -> Option<ChildLayoutUpdate> {
    let dashboard = app.dashboard.as_ref()?;
    let rows = &dashboard.child_rows;
    let key_for = |row: &DashboardRowId| {
        rows.iter().find_map(|(session_id, candidate)| {
            (candidate == row).then(|| PersistedRowId::TopLevel { session_id: session_id.clone() })
        })
    };
    Some(ChildLayoutUpdate {
        scope: rows.keys().map(|session_id| PersistedRowId::TopLevel { session_id: session_id.clone() }).collect(),
        pinned: dashboard.pinned.iter().filter_map(&key_for).collect(),
        reorder: dashboard.reorder.iter().filter_map(key_for).collect(),
    })
}
