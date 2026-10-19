use crate::app::app_view::AppView;
use crate::views::dashboard::DashboardRowId;
use xai_grok_telemetry::events::{
    DashboardAgentAttached, DashboardAgentLaunched, DashboardClosed, DashboardOpened,
};
use xai_grok_telemetry::session_ctx::log_event;

pub(super) fn log_dashboard_opened(app: &AppView) {
    let subagents: usize = app
        .agents
        .roots()
        .map(|(_, a)| a.subagent_sessions.len())
        .sum();
    log_event(DashboardOpened {
        agents: app.agents.roots().count(),
        subagents,
        leader_mode: app.leader_mode,
    });
}

pub(super) fn log_dashboard_closed(app: &AppView) {
    log_event(DashboardClosed {
        agents: app.agents.roots().count(),
    });
}

pub(super) fn log_dashboard_attached(id: &DashboardRowId) {
    let kind = match id {
        DashboardRowId::TopLevel(_) => "top_level",
        DashboardRowId::Roster { .. } => "roster",
        DashboardRowId::Workspace { .. } => "workspace",
    };
    log_event(DashboardAgentAttached { kind });
}

pub(super) fn log_dashboard_launched(source: &'static str) {
    log_event(DashboardAgentLaunched { source });
}
