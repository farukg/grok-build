//! Request inspector dispatch: opens the viewer and fetches its capture evidence.

use crate::app::actions::{Effect, RequestSnapshotQuery};
use crate::app::app_view::{ActiveView, AppView};

pub(super) fn dispatch_open_request_viewer(app: &mut AppView) -> Vec<Effect> {
    let ActiveView::Agent(agent_id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    if agent.request_viewer.is_some() {
        return vec![];
    }
    match agent.open_request_viewer() {
        Some((session_id, query, generation)) => vec![Effect::RequestSnapshotQuery {
            agent_id,
            session_id,
            query,
            generation,
        }],
        None => {
            app.show_toast("This session has no id yet; the request view opens once the session starts.");
            vec![]
        }
    }
}

pub(super) fn dispatch_fetch_request_body(app: &mut AppView) -> Vec<Effect> {
    let ActiveView::Agent(agent_id) = app.active_view else {
        return vec![];
    };
    let Some(agent) = app.agents.get_mut(&agent_id) else {
        return vec![];
    };
    match agent.request_body_query_started() {
        Some((session_id, query @ RequestSnapshotQuery::Body { .. }, generation)) => {
            vec![Effect::RequestSnapshotQuery { agent_id, session_id, query, generation }]
        }
        Some((_, RequestSnapshotQuery::Latest, _)) | None => vec![],
    }
}
