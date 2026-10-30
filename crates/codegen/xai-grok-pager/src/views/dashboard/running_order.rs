use super::row::{
    DashboardRow, activity_time_from_unix_ms, classify_top_level, roster_activity_to_state,
    sort_row_key_by_activity, top_level_last_change_at,
};
use super::state::{DashboardRowId, RowState};
use crate::app::agent::AgentId;
use crate::app::roster::RosterEntry;
use crate::app::session_views::SessionViews;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

#[derive(Default)]
pub(crate) struct RunningOrder {
    membership: RefCell<Membership>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum InitialIdentity {
    Unbound(AgentId),
    Session(String),
}

struct Entry {
    entered_at: u128,
    initial_activity: SystemTime,
    initial_identity: InitialIdentity,
}

#[derive(Default)]
struct Membership {
    revision: u128,
    sessions: HashMap<String, Entry>,
    local: HashMap<AgentId, Entry>,
}

pub(crate) fn observe_app(app: &crate::app::app_view::AppView) {
    if let Some(dashboard) = app.dashboard.as_ref() {
        dashboard.running_order.observe_sources(&app.agents, app.dashboard_roster());
    }
}

pub(crate) fn remove_session(app: &crate::app::app_view::AppView, session: &str) {
    if app.agents.find_by_session_id(session).is_none()
        && let Some(dashboard) = app.dashboard.as_ref()
    {
        dashboard.running_order.membership.borrow_mut().sessions.remove(session);
    }
}

pub(crate) fn observe_agent(app: &crate::app::app_view::AppView, id: AgentId) {
    if let Some(dashboard) = app.dashboard.as_ref()
        && let Some(agent) = app.agents.get(&id)
    {
        let mut membership = dashboard.running_order.membership.borrow_mut();
        let revision = membership.next_revision();
        membership.observe_agent(id, agent, revision);
    }
}

impl RunningOrder {
    pub(super) fn observe_sources(&self, agents: &SessionViews, roster: &[RosterEntry]) {
        let mut membership = self.membership.borrow_mut();
        let revision = membership.next_revision();
        let mut loaded: HashSet<&str> = HashSet::new();
        // Observe every lane before filtering; hiding a row is not leaving Working.
        for (id, agent) in agents.all() {
            if let Some(session) = agent.session.session_id.as_ref() {
                loaded.insert(session.0.as_ref());
            }
            membership.observe_agent(id, agent, revision);
        }
        for row in roster {
            // Loaded views own status, including when their roster twin is stale.
            if loaded.contains(row.session_id.as_str()) {
                continue;
            }
            match roster_activity_to_state(row.activity) {
                RowState::Working => {
                    if !membership.sessions.contains_key(&row.session_id) {
                        membership.sessions.insert(
                            row.session_id.clone(),
                            Entry {
                                entered_at: revision,
                                initial_activity: activity_time_from_unix_ms(row.last_change_unix_ms),
                                initial_identity: InitialIdentity::Session(row.session_id.clone()),
                            },
                        );
                    }
                }
                RowState::NeedsInput
                | RowState::Idle
                | RowState::Inactive
                | RowState::Completed
                | RowState::Failed => {
                    membership.sessions.remove(&row.session_id);
                }
            }
        }
    }

    pub(super) fn sort(
        &self,
        rows: &mut [DashboardRow],
        compare_inactive: impl Fn(&DashboardRow, &DashboardRow) -> std::cmp::Ordering,
    ) {
        let membership = self.membership.borrow();
        rows.sort_unstable_by(|left, right| {
            match (membership.entry(left), membership.entry(right)) {
                (Some(left_entry), Some(right_entry)) => right_entry
                    .entered_at
                    .cmp(&left_entry.entered_at)
                    .then_with(|| {
                        if left_entry.initial_activity == right_entry.initial_activity {
                            left_entry.initial_identity.cmp(&right_entry.initial_identity)
                        } else {
                            sort_row_key_by_activity(
                                left,
                                right,
                                left_entry.initial_activity,
                                right_entry.initial_activity,
                            )
                        }
                    }),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => compare_inactive(left, right),
            }
        });
    }
}

impl Membership {
    fn next_revision(&mut self) -> u128 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    fn observe_agent(&mut self, id: AgentId, agent: &crate::app::agent_view::AgentView, revision: u128) {
        let state = classify_top_level(agent);
        let session = agent.session.session_id.as_ref().map(|id| id.0.as_ref());
        match state {
            RowState::Working => match session {
                Some(session) => {
                    let provisional = self.local.remove(&id);
                    if !self.sessions.contains_key(session) {
                        let entry = provisional.unwrap_or_else(|| Entry {
                            entered_at: revision,
                            initial_activity: top_level_last_change_at(agent, state),
                            initial_identity: InitialIdentity::Session(session.to_owned()),
                        });
                        self.sessions.insert(session.to_owned(), entry);
                    }
                }
                None => {
                    self.local.entry(id).or_insert_with(|| Entry {
                        entered_at: revision,
                        initial_activity: top_level_last_change_at(agent, state),
                        initial_identity: InitialIdentity::Unbound(id),
                    });
                }
            },
            RowState::NeedsInput
            | RowState::Idle
            | RowState::Inactive
            | RowState::Completed
            | RowState::Failed => {
                // Replay's temporary idle state is not a new running-group entry.
                if agent.session_reload.is_some() || agent.loading_placeholder_id.is_some() {
                    return;
                }
                self.local.remove(&id);
                if let Some(session) = session {
                    self.sessions.remove(session);
                }
            }
        }
    }

    fn entry(&self, row: &DashboardRow) -> Option<&Entry> {
        match row.state {
            RowState::Working => row
                .session_id
                .as_deref()
                .and_then(|id| self.sessions.get(id))
                .or_else(|| match row.id {
                    DashboardRowId::TopLevel(id) => self.local.get(&id),
                    DashboardRowId::Roster { .. } | DashboardRowId::Workspace { .. } => None,
                }),
            RowState::NeedsInput
            | RowState::Idle
            | RowState::Inactive
            | RowState::Completed
            | RowState::Failed => None,
        }
    }
}
