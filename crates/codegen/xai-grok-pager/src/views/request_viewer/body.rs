use std::sync::Arc;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use xai_grok_shell::session::request_capture::{
    BodyAvailability, ProtectedBodyRef, RequestBodyRead, RequestObservation, RequestSnapshot,
    RequestSnapshotNotification,
};
use xai_grok_shell::session::storage::request_capture::PayloadRepresentation;

use crate::app::actions::RequestSnapshotTransportError;
use crate::theme::Theme;
use crate::views::block_viewer::ContentLine;

#[derive(Debug, Clone)]
pub struct RequestBodyKey(pub(crate) ProtectedBodyRef);

impl PartialEq for RequestBodyKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.snapshot.session_id == other.0.snapshot.session_id
            && self.0.snapshot.request == other.0.snapshot.request
            && self.0.attempt == other.0.attempt
            && self.0.digest == other.0.digest
    }
}

impl Eq for RequestBodyKey {}

impl RequestBodyKey {
    pub(crate) fn from_snapshot(snapshot: &RequestSnapshot) -> Option<Self> {
        let entry = last_prepared(snapshot)?;
        match &entry.observation {
            RequestObservation::Prepared { body, .. } => Some(Self(ProtectedBodyRef {
                snapshot: xai_grok_shell::session::request_capture::RequestSnapshotRef {
                    session_id: snapshot.origin.session_id.clone(),
                    request: snapshot.origin.request,
                },
                attempt: entry.attempt,
                digest: body.digest,
            })),
            RequestObservation::DispatchStarted
            | RequestObservation::ResponseHeaders { .. }
            | RequestObservation::Finished { .. } => None,
        }
    }

    pub(crate) fn superseded_by(&self, notification: &RequestSnapshotNotification) -> bool {
        if self.0.snapshot.request != notification.snapshot.request {
            return true;
        }
        match &notification.evidence.observation {
            RequestObservation::Prepared { body, .. } => {
                self.0.attempt != notification.evidence.attempt || self.0.digest != body.digest
            }
            RequestObservation::DispatchStarted
            | RequestObservation::ResponseHeaders { .. }
            | RequestObservation::Finished { .. } => false,
        }
    }
}

pub(super) fn last_prepared(
    snapshot: &RequestSnapshot,
) -> Option<&xai_grok_shell::session::request_capture::AttemptObservation> {
    snapshot.observations.iter().rev().find(|entry| matches!(
        entry.observation, RequestObservation::Prepared { .. }
    ))
}

pub struct RequestBodyPresentation {
    pub(super) json: String,
    pub(crate) items: Arc<[ContentLine]>,
}

impl std::fmt::Debug for RequestBodyPresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestBodyPresentation").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum RequestBodyContent {
    Available(RequestBodyPresentation),
    Unavailable(BodyAvailability),
}

impl RequestBodyContent {
    pub(crate) fn prepare(
        key: &RequestBodyKey,
        body: RequestBodyRead,
    ) -> Result<Self, RequestSnapshotTransportError> {
        match body {
            RequestBodyRead::Available { json, representation, protected_local } => {
                if RequestBodyKey(protected_local) != *key {
                    return Err(RequestSnapshotTransportError::UnexpectedShape);
                }
                let parsed: serde_json::Value =
                    serde_json::from_str(&json).map_err(RequestSnapshotTransportError::Decode)?;
                let pretty = serde_json::to_string_pretty(&parsed)
                    .map_err(RequestSnapshotTransportError::Encode)?;
                let theme = Theme::current();
                let dim = Style::default().fg(theme.gray_dim);
                let value = Style::default().fg(theme.text_primary);
                let mut lines = vec![
                    Span::styled(
                        format!("Publication view  ·  {}  ·  formatted for display", super::representation_label(&representation)),
                        dim,
                    ).into(),
                    Span::styled(representation_description(representation), dim).into(),
                    Line::default(),
                ];
                lines.extend(pretty.lines().map(|line| Span::styled(line.to_owned(), value).into()));
                Ok(Self::Available(RequestBodyPresentation {
                    json,
                    items: ContentLine::from_lines(&lines).into(),
                }))
            }
            RequestBodyRead::Unavailable { availability } => Ok(Self::Unavailable(availability)),
        }
    }
}

fn representation_description(representation: PayloadRepresentation) -> &'static str {
    match representation {
        PayloadRepresentation::ExactLocalJson =>
            "Unchanged local JSON publication; display is pretty-printed, copy retains publication bytes. Headers are not retained; provider processing is not claimed.",
        PayloadRepresentation::Redacted { .. } =>
            "Redacted publication, not the protected original serialization; display is pretty-printed, copy retains publication bytes. Headers are not retained; provider processing is not claimed.",
    }
}
