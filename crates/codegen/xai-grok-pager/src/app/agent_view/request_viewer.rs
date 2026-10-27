//! Request inspector ownership: open/close, query correlation and applying capture answers.
//!
//! One viewer per agent view, bound to the session it was opened for. Answers apply only when
//! the session identity and the query generation still match, so a stale reply can never paint
//! another session's (or an earlier open's) evidence.

use super::AgentView;
use crate::app::actions::{
    Action, RequestSnapshotQuery, RequestSnapshotTransportError,
};
use crate::app::app_view::InputOutcome;
use crate::views::request_viewer::{
    PendingSnapshotRefresh, RawBodyState, RequestBodyContent, RequestBodyKey,
    RequestViewTab, RequestViewerState, SnapshotQueryState,
};
use agent_client_protocol as acp;
use crossterm::event::KeyEvent;
use xai_grok_shell::session::request_capture::{
    RequestSnapshot, RequestSnapshotNotification,
};

/// A capture query the viewer wants dispatched next.
pub(crate) type RequestSnapshotFetch = (acp::SessionId, RequestSnapshotQuery, u64);

impl AgentView {
    /// F3: close the inspector, or ask dispatch to open it for the active session.
    pub(crate) fn toggle_request_viewer(&mut self) -> InputOutcome {
        if self.request_viewer.take().is_some() {
            InputOutcome::Changed
        } else {
            InputOutcome::Action(Action::OpenRequestInspector)
        }
    }

    /// Open the viewer for the current session and arm the first snapshot query.
    pub(crate) fn open_request_viewer(&mut self) -> Option<RequestSnapshotFetch> {
        let session_id = self.session.session_id.clone()?;
        self.block_viewer = None;
        let generation = self.request_snapshot_generation.checked_add(1)?;
        self.request_snapshot_generation = generation;
        self.request_viewer = Some(RequestViewerState::new(session_id.clone(), generation));
        Some((session_id, RequestSnapshotQuery::Latest, generation))
    }

    /// Arm a raw-body query when the raw tab is showing and its body is not in flight or loaded.
    pub(crate) fn request_body_query_started(&mut self) -> Option<RequestSnapshotFetch> {
        let viewer = self.request_viewer.as_mut()?;
        if viewer.tab != RequestViewTab::Raw {
            return None;
        }
        match viewer.snapshot_query {
            SnapshotQueryState::InFlight { .. } => return None,
            SnapshotQueryState::Idle => {}
        }
        match &viewer.raw {
            RawBodyState::NotRequested => {}
            RawBodyState::Loading { .. } | RawBodyState::Loaded { .. }
            | RawBodyState::Unavailable { .. } | RawBodyState::Failed { .. } => return None,
        }
        let key = viewer.body_key()?;
        let session_id = viewer.session_id.clone();
        let generation = self.request_snapshot_generation.checked_add(1)?;
        self.request_snapshot_generation = generation;
        viewer.mark_raw_loading(key.clone(), generation);
        Some((session_id, RequestSnapshotQuery::Body { key }, generation))
    }

    /// The shell answered the latest-request query. Returns a follow-up raw-body query when the
    /// raw tab now needs one for the (possibly new) request.
    pub(crate) fn request_snapshot_loaded(
        &mut self,
        session_id: &acp::SessionId,
        generation: u64,
        result: Result<Option<RequestSnapshot>, RequestSnapshotTransportError>,
    ) -> Option<RequestSnapshotFetch> {
        let viewer = self.request_viewer.as_mut()?;
        if self.session.session_id.as_ref() != Some(session_id)
            || viewer.session_id != *session_id
        {
            return None;
        }
        let refresh = match viewer.snapshot_query {
            SnapshotQueryState::Idle => return None,
            SnapshotQueryState::InFlight { generation: active, refresh } => {
                if active != generation {
                    return None;
                }
                refresh
            }
        };
        viewer.snapshot_query = SnapshotQueryState::Idle;
        match refresh {
            PendingSnapshotRefresh::Pending => self.start_request_snapshot_refresh(),
            PendingSnapshotRefresh::Clear => {
                match result {
                    Ok(Some(snapshot)) if snapshot.origin.session_id != session_id.0.as_ref() =>
                        viewer.set_failure(RequestSnapshotTransportError::UnexpectedShape.into()),
                    Ok(snapshot) => viewer.set_snapshot(snapshot),
                    Err(error) => viewer.set_failure(error.into()),
                }
                self.request_body_query_started()
            }
        }
    }

    pub(crate) fn request_body_loaded(
        &mut self,
        session_id: &acp::SessionId,
        generation: u64,
        key: &RequestBodyKey,
        result: Result<RequestBodyContent, RequestSnapshotTransportError>,
    ) {
        let Some(viewer) = self.request_viewer.as_mut() else { return; };
        if self.session.session_id.as_ref() != Some(session_id)
            || viewer.session_id != *session_id
        {
            return;
        }
        match &viewer.raw {
            RawBodyState::Loading { key: active, generation: active_generation } => {
                if active != key || *active_generation != generation {
                    return;
                }
            }
            RawBodyState::NotRequested | RawBodyState::Loaded { .. }
            | RawBodyState::Unavailable { .. } | RawBodyState::Failed { .. } => return,
        }
        viewer.set_raw_body(key.clone(), result.map_err(Into::into));
    }

    /// A real capture observation arrived for this session; refresh the open viewer.
    pub(crate) fn request_snapshot_observed(
        &mut self,
        notification: &RequestSnapshotNotification,
    ) -> Option<RequestSnapshotFetch> {
        let viewer = self.request_viewer.as_mut()?;
        if self.session.session_id.as_ref() != Some(&viewer.session_id)
            || viewer.session_id.0.as_ref() != notification.snapshot.session_id.as_str()
        {
            return None;
        }
        if viewer.raw.key().is_some_and(|key| key.superseded_by(notification)) {
            viewer.invalidate_raw();
        }
        match &mut viewer.snapshot_query {
            SnapshotQueryState::InFlight { refresh, .. } => {
                *refresh = PendingSnapshotRefresh::Pending;
                None
            }
            SnapshotQueryState::Idle => self.start_request_snapshot_refresh(),
        }
    }

    fn start_request_snapshot_refresh(&mut self) -> Option<RequestSnapshotFetch> {
        let viewer = self.request_viewer.as_mut()?;
        let generation = self.request_snapshot_generation.checked_add(1)?;
        self.request_snapshot_generation = generation;
        viewer.snapshot_query = SnapshotQueryState::InFlight {
            generation,
            refresh: PendingSnapshotRefresh::Clear,
        };
        Some((viewer.session_id.clone(), RequestSnapshotQuery::Latest, generation))
    }

    /// The agent view's session changed under the viewer; drop it rather than show foreign data.
    pub(crate) fn close_foreign_request_viewer(&mut self) {
        if let Some(viewer) = &self.request_viewer
            && self.session.session_id.as_ref() != Some(&viewer.session_id)
        {
            self.request_viewer = None;
        }
    }

    pub(super) fn handle_request_viewer_key(&mut self, key: &KeyEvent) -> InputOutcome {
        use crate::views::request_viewer::RequestViewerKey;
        let Some(viewer) = self.request_viewer.as_mut() else {
            return InputOutcome::Unchanged;
        };
        match viewer.handle_key(key) {
            RequestViewerKey::Close => {
                self.request_viewer = None;
                InputOutcome::Changed
            }
            RequestViewerKey::Consumed => InputOutcome::Changed,
            RequestViewerKey::Bubble => InputOutcome::Unchanged,
            RequestViewerKey::ToggleTab(tab) => {
                viewer.show_tab(tab);
                match tab {
                    RequestViewTab::Raw => InputOutcome::Action(Action::FetchRequestBody),
                    RequestViewTab::Readable => InputOutcome::Changed,
                }
            }
            RequestViewerKey::CopyFull => {
                if let Some(text) = self
                    .request_viewer
                    .as_ref()
                    .and_then(|viewer| viewer.full_copy_text())
                {
                    self.copy_to_clipboard(&text);
                }
                InputOutcome::Changed
            }
        }
    }

    pub(in crate::app) fn handle_request_viewer_mouse(
        &mut self,
        mouse: &crossterm::event::MouseEvent,
    ) -> InputOutcome {
        use crate::views::modal_window::{ModalWindowOutcome, handle_modal_mouse};
        use crate::views::request_viewer::RequestViewerMouse;
        let Some(viewer) = self.request_viewer.as_mut() else {
            return InputOutcome::Changed;
        };
        match handle_modal_mouse(&mut viewer.modal, mouse.kind, mouse.column, mouse.row) {
            ModalWindowOutcome::CloseRequested => {
                self.request_viewer = None;
                return InputOutcome::Changed;
            }
            ModalWindowOutcome::Unhandled => {}
            ModalWindowOutcome::Handled | ModalWindowOutcome::TabChanged(_)
            | ModalWindowOutcome::ShortcutActivated(_) | ModalWindowOutcome::CollapseGroup
            | ModalWindowOutcome::ExpandGroup | ModalWindowOutcome::CollapseDetails
            | ModalWindowOutcome::ExpandDetails | ModalWindowOutcome::JumpToParent(_) =>
                return InputOutcome::Changed,
        }
        match viewer.handle_mouse(mouse) {
            RequestViewerMouse::Consumed => InputOutcome::Changed,
            RequestViewerMouse::Unchanged => InputOutcome::Unchanged,
        }
    }
}

#[cfg(test)]
#[path = "request_viewer_tests.rs"]
mod tests;
