//! Captured model-request inspector for one session.
//!
//! Shows the latest real request the shell captured for this session (`x.ai/session/request_snapshot`),
//! readable and raw, with the honest prepared/dispatch facts. Nothing here prepares, sends or
//! edits a request: it only reads evidence that already exists.

use agent_client_protocol as acp;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as RBlock, BorderType, Borders, Clear, Widget};
use xai_grok_shell::sampling::ApiBackend;
use xai_grok_shell::session::request_capture::{
    AttemptOutcome, BodyAvailability, CaptureFailure, NotCapturedReason,
    RequestSnapshot, RequestSnapshotId, SourceDurability,
};
use xai_grok_shell::session::storage::request_capture::PayloadRepresentation;
use std::sync::Arc;

mod body;
pub use body::{RequestBodyContent, RequestBodyKey, RequestBodyPresentation};

use crate::clipboard::SystemClipboard;
use crate::key;
use crate::render::color::dim_area;
use crate::render::scrollbar::SCROLLBAR_TOTAL_COLS;
use crate::theme::Theme;
use crate::views::block_viewer::ContentLine;
use crate::views::list_pane::{
    ListItem, ListPane, ListPaneConfig, ListPaneState, ListPaneStyle, WrapMode,
};
use crate::views::modal_window::ModalWindowState;
use crate::views::shortcuts_bar::HintItem;

use crate::app::agent_view::context_sidebar::label as context_category_label;

const SOURCE_DURABILITY_LABEL: &str = "unconfirmed (independent history channel)";

/// Why the inspector has no snapshot to show.
#[derive(Debug)]
pub(crate) enum RequestViewFailure {
    Capture(CaptureFailure),
    Encode(serde_json::Error),
    Decode(serde_json::Error),
    Rpc(acp::Error),
    UnexpectedShape,
}

impl From<crate::app::actions::RequestSnapshotTransportError> for RequestViewFailure {
    fn from(error: crate::app::actions::RequestSnapshotTransportError) -> Self {
        match error {
            crate::app::actions::RequestSnapshotTransportError::Encode(error) => Self::Encode(error),
            crate::app::actions::RequestSnapshotTransportError::Decode(error) => Self::Decode(error),
            crate::app::actions::RequestSnapshotTransportError::Rpc(error) => Self::Rpc(error),
            crate::app::actions::RequestSnapshotTransportError::Unavailable(cause) => Self::Capture(cause),
            crate::app::actions::RequestSnapshotTransportError::UnexpectedShape => Self::UnexpectedShape,
        }
    }
}

impl RequestViewFailure {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Capture(cause) => capture_failure_label(*cause),
            Self::Encode(_) => "request inspection encoding failed".to_owned(),
            Self::Decode(_) => "capture response decoding failed".to_owned(),
            Self::Rpc(_) => "capture RPC failed".to_owned(),
            Self::UnexpectedShape => "capture store answered an unexpected state".to_owned(),
        }
    }
}

/// What the readable tab shows for the session's latest captured request.
#[derive(Debug)]
pub(crate) enum RequestViewData {
    Loading,
    Absent,
    Loaded { snapshot: RequestSnapshot },
    Unavailable { failure: RequestViewFailure },
}

/// Raw tab content: the lazily fetched publication body of a prepared attempt.
#[derive(Debug)]
pub(crate) enum RawBodyState {
    NotRequested,
    Loading { key: RequestBodyKey, generation: u64 },
    Loaded { key: RequestBodyKey, presentation: RequestBodyPresentation },
    Unavailable { key: RequestBodyKey, availability: BodyAvailability },
    Failed { key: RequestBodyKey, failure: RequestViewFailure },
}

impl RawBodyState {
    pub(crate) fn key(&self) -> Option<&RequestBodyKey> {
        match self {
            Self::NotRequested => None,
            Self::Loading { key, .. }
            | Self::Loaded { key, .. }
            | Self::Unavailable { key, .. }
            | Self::Failed { key, .. } => Some(key),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PendingSnapshotRefresh {
    Clear,
    Pending,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnapshotQueryState {
    Idle,
    InFlight { generation: u64, refresh: PendingSnapshotRefresh },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestViewTab {
    Readable,
    Raw,
}

/// Result of routing one key into the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestViewerKey {
    Consumed,
    Bubble,
    Close,
    ToggleTab(RequestViewTab),
    CopyFull,
}

/// Result of routing one mouse event into the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestViewerMouse {
    Consumed,
    Unchanged,
}

pub(crate) struct RequestViewerState {
    pub(crate) session_id: acp::SessionId,
    pub(crate) snapshot_query: SnapshotQueryState,
    pub(crate) tab: RequestViewTab,
    pub(crate) data: RequestViewData,
    pub(crate) raw: RawBodyState,
    pub(crate) modal: ModalWindowState,
    list_state: ListPaneState,
    items: Arc<[ContentLine]>,
    last_content_area: Rect,
}

impl RequestViewerState {
    pub(crate) fn new(session_id: acp::SessionId, snapshot_generation: u64) -> Self {
        let config = ListPaneConfig {
            follow_enabled: false,
            wrap_toggle_enabled: true,
            search_enabled: true,
            copy_enabled: true,
            show_selection_when_unfocused: false,
            visual_select_enabled: true,
            filter_enabled: false,
            goto_line_enabled: false,
        };
        let mut list_state = ListPaneState::new_with_config(WrapMode::Wrap, false, config);
        list_state.set_clipboard_provider(Box::new(SystemClipboard));
        let mut viewer = Self {
            session_id,
            snapshot_query: SnapshotQueryState::InFlight {
                generation: snapshot_generation,
                refresh: PendingSnapshotRefresh::Clear,
            },
            tab: RequestViewTab::Readable,
            data: RequestViewData::Loading,
            raw: RawBodyState::NotRequested,
            modal: ModalWindowState::new(),
            list_state,
            items: Arc::from([]),
            last_content_area: Rect::default(),
        };
        viewer.rebuild_items();
        viewer
    }

    pub(crate) fn body_key(&self) -> Option<RequestBodyKey> {
        match &self.data {
            RequestViewData::Loaded { snapshot } => RequestBodyKey::from_snapshot(snapshot),
            RequestViewData::Loading | RequestViewData::Absent | RequestViewData::Unavailable { .. } => None,
        }
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: Option<RequestSnapshot>) {
        self.data = match snapshot {
            Some(snapshot) => RequestViewData::Loaded { snapshot },
            None => RequestViewData::Absent,
        };
        let key = self.body_key();
        if self.raw.key() != key.as_ref() {
            self.raw = RawBodyState::NotRequested;
        }
        self.rebuild_items();
    }

    pub(crate) fn set_failure(&mut self, failure: RequestViewFailure) {
        self.data = RequestViewData::Unavailable { failure };
        self.raw = RawBodyState::NotRequested;
        self.rebuild_items();
    }

    pub(crate) fn set_raw_body(
        &mut self,
        key: RequestBodyKey,
        result: Result<RequestBodyContent, RequestViewFailure>,
    ) {
        self.raw = match result {
            Ok(RequestBodyContent::Available(presentation)) => RawBodyState::Loaded { key, presentation },
            Ok(RequestBodyContent::Unavailable(availability)) => RawBodyState::Unavailable { key, availability },
            Err(failure) => RawBodyState::Failed { key, failure },
        };
        if self.tab == RequestViewTab::Raw {
            self.rebuild_items();
        }
    }

    pub(crate) fn invalidate_raw(&mut self) {
        self.raw = RawBodyState::NotRequested;
        if self.tab == RequestViewTab::Raw {
            self.rebuild_items();
        }
    }

    pub(crate) fn mark_raw_loading(&mut self, key: RequestBodyKey, generation: u64) {
        self.raw = RawBodyState::Loading { key, generation };
        if self.tab == RequestViewTab::Raw {
            self.rebuild_items();
        }
    }

    pub(crate) fn show_tab(&mut self, tab: RequestViewTab) {
        self.tab = tab;
        self.rebuild_items();
    }

    /// The latest prepared observation, whose body the raw tab asks for.
    pub(crate) fn last_prepared(&self) -> Option<&xai_grok_shell::session::request_capture::AttemptObservation> {
        let RequestViewData::Loaded { snapshot } = &self.data else {
            return None;
        };
        body::last_prepared(snapshot)
    }

    pub(crate) fn handle_scroll(&mut self, lines: i32) {
        self.list_state.scroll_lines(lines, &self.items);
    }

    pub(crate) fn handle_paste(&mut self, text: &str) -> bool {
        self.list_state.handle_paste(text, &self.items)
    }

    pub(crate) fn handle_key(&mut self, key: &KeyEvent) -> RequestViewerKey {
        if self.list_state.input_mode().is_some() {
            self.list_state.handle_key_event(key, &self.items);
            return RequestViewerKey::Consumed;
        }
        let idle = !self.list_state.visual_mode;
        if idle && key!(Esc).matches(key) {
            return RequestViewerKey::Close;
        }
        if idle && key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE {
            return RequestViewerKey::Close;
        }
        // Enter never reaches the composer from the inspector.
        if key.code == KeyCode::Enter {
            return RequestViewerKey::Consumed;
        }
        if idle && key!('r').matches(key) {
            return RequestViewerKey::ToggleTab(match self.tab {
                RequestViewTab::Readable => RequestViewTab::Raw,
                RequestViewTab::Raw => RequestViewTab::Readable,
            });
        }
        if idle && key!('Y').matches(key) {
            return RequestViewerKey::CopyFull;
        }
        if self.list_state.handle_key_event(key, &self.items) {
            RequestViewerKey::Consumed
        } else {
            RequestViewerKey::Bubble
        }
    }

    pub(crate) fn handle_mouse(&mut self, mouse: &MouseEvent) -> RequestViewerMouse {
        use crossterm::event::{MouseButton, MouseEventKind};
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.list_state.handle_scroll_event(3, mouse.column, mouse.row, &self.items);
                RequestViewerMouse::Consumed
            }
            MouseEventKind::ScrollUp => {
                self.list_state.handle_scroll_event(-3, mouse.column, mouse.row, &self.items);
                RequestViewerMouse::Consumed
            }
            MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::Drag(MouseButton::Left)
            | MouseEventKind::Up(MouseButton::Left) => {
                self.list_state.handle_mouse_event(
                    mouse.kind,
                    mouse.column,
                    mouse.row,
                    self.last_content_area,
                    &self.items,
                );
                RequestViewerMouse::Consumed
            }
            MouseEventKind::Moved => RequestViewerMouse::Unchanged,
            _ => RequestViewerMouse::Unchanged,
        }
    }

    pub(crate) fn full_copy_text(&self) -> Option<String> {
        match self.tab {
            RequestViewTab::Raw => match &self.raw {
                RawBodyState::Loaded { presentation, .. } => return Some(presentation.json.clone()),
                RawBodyState::NotRequested | RawBodyState::Loading { .. }
                | RawBodyState::Unavailable { .. } | RawBodyState::Failed { .. } => {}
            },
            RequestViewTab::Readable => {}
        }
        let joined = self
            .items
            .iter()
            .map(|item| item.search_text())
            .collect::<Vec<_>>()
            .join("\n");
        (!joined.is_empty()).then_some(joined)
    }

    pub(crate) fn shortcuts_hints(&self) -> Vec<HintItem> {
        vec![
            HintItem::new(key!(Esc), "close"),
            HintItem::new(key!('/'), "search"),
            HintItem::new(key!('v'), "select"),
            HintItem::new(key!('w'), "wrap"),
            HintItem::new(
                key!('r'),
                match self.tab {
                    RequestViewTab::Readable => "raw",
                    RequestViewTab::Raw => "readable",
                },
            ),
            HintItem::new(key!('Y'), "copy all"),
        ]
    }

    fn rebuild_items(&mut self) {
        let items = match self.tab {
            RequestViewTab::Raw => match &self.raw {
                RawBodyState::Loaded { presentation, .. } => Arc::clone(&presentation.items),
                RawBodyState::NotRequested | RawBodyState::Loading { .. }
                | RawBodyState::Unavailable { .. } | RawBodyState::Failed { .. } =>
                    ContentLine::from_lines(&self.raw_lines()).into(),
            },
            RequestViewTab::Readable => ContentLine::from_lines(&self.readable_lines()).into(),
        };
        if Arc::ptr_eq(&self.items, &items) {
            return;
        }
        let selected = self.list_state.selected_id();
        self.items = items;
        self.list_state.invalidate_layout();
        if let Some(id) = selected.filter(|id| self.items.iter().any(|item| item.stable_id() == *id)) {
            self.list_state.select_by_id(id);
        } else if let Some(first) = self.items.first() {
            self.list_state.select_by_id(first.stable_id());
        }
    }

    fn title_line(&self, theme: &Theme) -> Line<'static> {
        let mut spans = vec![Span::styled(
            "Model request".to_owned(),
            Style::default().fg(theme.accent_user).add_modifier(Modifier::BOLD),
        )];
        let tab = match self.tab {
            RequestViewTab::Readable => "readable",
            RequestViewTab::Raw => "raw",
        };
        spans.push(Span::styled(
            format!("  ·  {tab}"),
            Style::default().fg(theme.text_secondary),
        ));
        if matches!(self.snapshot_query, SnapshotQueryState::InFlight { .. }) {
            spans.push(Span::styled(
                "  ·  refreshing…".to_owned(),
                Style::default().fg(theme.warning),
            ));
        }
        Line::from(spans)
    }

    fn readable_lines(&self) -> Vec<Line<'static>> {
        let theme = Theme::current();
        let dim = Style::default().fg(theme.gray_dim);
        let label = Style::default().fg(theme.text_secondary);
        let value = Style::default().fg(theme.text_primary);
        let mut lines = vec![
            Span::styled(
                "The latest model request captured for this session, as the shell recorded it.",
                dim,
            )
            .into(),
            Line::default(),
        ];
        match &self.data {
            RequestViewData::Loading => {
                lines.push(Span::styled("Reading the captured request…", value).into());
            }
            RequestViewData::Absent => {
                lines.push(Span::styled(
                    "No captured request for this session yet.",
                    value,
                )
                .into());
                lines.push(
                    Span::styled(
                        "The next real model request appears here; nothing is prepared in advance.",
                        dim,
                    )
                    .into(),
                );
            }
            RequestViewData::Unavailable { failure } => {
                lines.push(Span::styled(
                    format!("Capture unavailable: {}", failure.label()),
                    Style::default().fg(theme.accent_error),
                )
                .into());
            }
            RequestViewData::Loaded { snapshot } => {
                let origin = &snapshot.origin;
                lines.push(
                    Span::styled(
                        format!(
                            "request {}  ·  turn {}  ·  generation {}",
                            origin_short_id(&origin.request),
                            origin.turn,
                            origin.generation
                        ),
                        dim,
                    )
                    .into(),
                );
                let prompt = origin.prompt_id.as_deref().map(short_token).unwrap_or("—");
                let conversation = origin
                    .conversation_id
                    .as_deref()
                    .map(short_token)
                    .unwrap_or("—");
                let logical = origin
                    .logical_request_id
                    .as_deref()
                    .map(short_token)
                    .unwrap_or("—");
                lines.push(
                    Span::styled(
                        format!("prompt {prompt}  ·  conversation {conversation}  ·  logical {logical}"),
                        dim,
                    )
                    .into(),
                );
                lines.push(Line::default());
                for entry in &snapshot.observations {
                    lines.extend(attempt_lines(entry, &label, &value, &dim, &theme));
                }
                if let Some(prepared) = self.last_prepared()
                    && let xai_grok_shell::session::request_capture::RequestObservation::Prepared {
                        membership, ..
                    } = &prepared.observation
                {
                    lines.push(Line::default());
                    lines.push(Span::styled("Members by context category", label).into());
                    let mut counts: Vec<(&'static str, usize)> = Vec::new();
                    for member in membership {
                        let category_label = context_category_label(member.category);
                        match counts.iter_mut().find(|(known, _)| *known == category_label) {
                            Some((_, count)) => *count += 1,
                            None => counts.push((category_label, 1)),
                        }
                    }
                    if counts.is_empty() {
                        lines.push(Span::styled("none", dim).into());
                    } else {
                        let parts: Vec<String> = counts
                            .into_iter()
                            .map(|(category_label, count)| format!("{category_label} {count}"))
                            .collect();
                        lines.push(Span::styled(parts.join("  ·  "), value).into());
                    }
                    let with_id = membership
                        .iter()
                        .filter(|member| member.source.id().is_some())
                        .count();
                    let without = membership.len().saturating_sub(with_id);
                    lines.push(
                        Span::styled(
                            format!("sources: {with_id} with local id  ·  {without} without"),
                            dim,
                        )
                        .into(),
                    );
                }
            }
        }
        lines.push(Line::default());
        lines.push(
            Span::styled(
                "Prepared bodies are local captures; dispatch and response facts do not prove what the model consumed.",
                dim,
            )
            .into(),
        );
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let theme = Theme::current();
        let dim = Style::default().fg(theme.gray_dim);
        let value = Style::default().fg(theme.text_primary);
        let mut lines = Vec::new();
        match &self.raw {
            RawBodyState::NotRequested => lines.push(
                Span::styled("Raw body not requested yet.", value).into(),
            ),
            RawBodyState::Loading { .. } => lines.push(
                Span::styled("Loading the captured body…", value).into(),
            ),
            RawBodyState::Loaded { .. } => {}
            RawBodyState::Unavailable { availability, .. } => {
                lines.push(
                    Span::styled(
                        format!("Body unavailable: {}", body_availability_label(availability)),
                        Style::default().fg(theme.accent_error),
                    )
                    .into(),
                );
            }
            RawBodyState::Failed { failure, .. } => {
                lines.push(
                    Span::styled(
                        format!("Body read failed: {}", failure.label()),
                        Style::default().fg(theme.accent_error),
                    )
                    .into(),
                );
            }
        }
        match &self.data {
            RequestViewData::Loading => lines.push(
                Span::styled("Reading the captured request…", dim).into(),
            ),
            RequestViewData::Absent => lines.push(
                Span::styled("No captured request for this session yet.", dim).into(),
            ),
            RequestViewData::Unavailable { failure } => lines.push(
                Span::styled(
                    format!("Capture unavailable: {}", failure.label()),
                    dim,
                )
                .into(),
            ),
            RequestViewData::Loaded { .. } => {}
        }
        lines
    }

    /// Paint the inspector popup over `overlay_area` (the area above the shortcuts bar).
    pub(crate) fn render(&mut self, overlay_area: Rect, buf: &mut Buffer) {
        let theme = Theme::current();
        let popup_w = ((overlay_area.width as f32 * 0.95) as u16)
            .max(60)
            .min(overlay_area.width);
        let popup_h = ((overlay_area.height as f32 * 0.92) as u16)
            .max(12)
            .min(overlay_area.height.saturating_sub(2));
        let h_pad = 2u16;
        let inner_w = popup_w.saturating_sub(2);
        if popup_h.saturating_sub(2) < 3 || inner_w < 10 {
            return;
        }
        let popup_x = overlay_area.x + (overlay_area.width.saturating_sub(popup_w)) / 2;
        let popup_y = overlay_area.y + (overlay_area.height.saturating_sub(popup_h)) / 2;
        let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);
        let base_style = Style::default().fg(theme.text_primary).bg(theme.bg_base);
        let border = RBlock::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.gray_dim))
            .style(base_style);
        let inner = border.inner(popup_area);
        if inner.height < 3 || inner.width < 10 {
            return;
        }
        dim_area(buf, overlay_area, theme.bg_base, 0.5);
        Clear.render(popup_area, buf);
        buf.set_style(popup_area, base_style);
        border.render(popup_area, buf);
        self.title_line(&theme).render(
            Rect::new(inner.x + h_pad, inner.y, inner.width.saturating_sub(h_pad + 5), 1),
            buf,
        );
        let close_chars: &[char] = &['[', 'x', ']'];
        let close_w = close_chars.len() as u16;
        let close_x = inner.x + inner.width.saturating_sub(close_w + 1);
        let close_rect = Rect::new(close_x, inner.y, close_w, 1);
        for (i, &ch) in close_chars.iter().enumerate() {
            if let Some(cell) = buf.cell_mut((close_x + i as u16, inner.y)) {
                cell.set_char(ch);
                cell.set_style(if self.modal.close_hovered {
                    Style::default().fg(theme.text_primary).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.gray_dim)
                });
            }
        }
        self.modal.close_button_rect = Some(close_rect);
        self.modal.popup_area = Some(popup_area);
        let content_area = Rect {
            x: inner.x + h_pad,
            y: inner.y + 1,
            width: inner.width.saturating_sub(h_pad * 2),
            height: inner.height.saturating_sub(1),
        };
        self.last_content_area = content_area;
        if content_area.height == 0 || content_area.width == 0 || self.items.is_empty() {
            return;
        }
        let likely_scrollbar = self.items.len() > content_area.height as usize;
        let render_area = if likely_scrollbar {
            Rect {
                width: content_area.width + SCROLLBAR_TOTAL_COLS.min(2),
                ..content_area
            }
        } else {
            content_area
        };
        self.list_state
            .prepare_layout(&self.items, render_area.width, render_area.height);
        if !likely_scrollbar && self.list_state.total_height() > render_area.height as usize {
            let wider = Rect {
                width: content_area.width + SCROLLBAR_TOTAL_COLS.min(2),
                ..content_area
            };
            self.list_state
                .prepare_layout(&self.items, wider.width, wider.height);
        }
        let style = ListPaneStyle {
            uniform_visual_bg: true,
            ..ListPaneStyle::default()
        };
        ListPane::new(&self.items)
            .focused(true)
            .style(style)
            .render(render_area, buf, &mut self.list_state);
    }
}

fn attempt_lines(
    entry: &xai_grok_shell::session::request_capture::AttemptObservation,
    label: &Style,
    value: &Style,
    dim: &Style,
    theme: &Theme,
) -> Vec<Line<'static>> {
    use xai_grok_shell::session::request_capture::RequestObservation;
    let mut lines = vec![Span::styled(format!("attempt {}", entry.attempt.0), *label).into()];
    match &entry.observation {
        RequestObservation::Prepared { backend, membership, body, source_durability } => {
            lines.push(
                Span::styled(
                    format!(
                        "  prepared  ·  backend {}  ·  {} members",
                        api_backend_label(backend),
                        membership.len()
                    ),
                    *value,
                )
                .into(),
            );
            lines.push(
                Span::styled(
                    format!(
                        "  body {}  ·  digest {}",
                        body_state_label(body),
                        digest_prefix(&body.digest)
                    ),
                    *dim,
                )
                .into(),
            );
            let durability = match source_durability {
                SourceDurability::UnconfirmedIndependentHistoryChannel => SOURCE_DURABILITY_LABEL,
            };
            lines.push(
                Span::styled(format!("  source publication durability: {durability}"), *dim)
                    .into(),
            );
        }
        RequestObservation::DispatchStarted => lines.push(
            Span::styled("  dispatch started (local execution began)", *value).into(),
        ),
        RequestObservation::ResponseHeaders { status } => lines.push(
            Span::styled(format!("  response HTTP {status}"), *value).into(),
        ),
        RequestObservation::Finished { outcome } => lines.push(
            Span::styled(
                format!("  finished: {}", attempt_outcome_label(*outcome)),
                Style::default().fg(theme.text_primary),
            )
            .into(),
        ),
    }
    lines
}

fn body_state_label(body: &xai_grok_shell::session::request_capture::PreparedBody) -> String {
    match &body.availability {
        BodyAvailability::Captured { representation, .. } => {
            format!("captured ({} B, {})", body.byte_len, representation_label(representation))
        }
        availability @ (BodyAvailability::Expired
        | BodyAvailability::NotCaptured { .. }
        | BodyAvailability::Failed { .. }
        | BodyAvailability::Unknown) => body_availability_label(availability),
    }
}

fn api_backend_label(backend: &ApiBackend) -> &'static str {
    match backend {
        ApiBackend::ChatCompletions => "chat completions",
        ApiBackend::Responses => "responses",
        ApiBackend::Messages => "messages",
    }
}

fn attempt_outcome_label(outcome: AttemptOutcome) -> &'static str {
    match outcome {
        AttemptOutcome::Completed => "completed",
        AttemptOutcome::Empty => "empty",
        AttemptOutcome::Failed => "failed",
        AttemptOutcome::Cancelled => "cancelled",
        AttemptOutcome::InitializationFailed => "initialization failed",
    }
}

pub(crate) fn capture_failure_label(failure: CaptureFailure) -> String {
    match failure {
        CaptureFailure::Storage => "capture storage failed".to_owned(),
        CaptureFailure::Io { kind, os_code } => match os_code {
            Some(code) => format!("capture I/O error ({kind:?}, os {code})"),
            None => format!("capture I/O error ({kind:?})"),
        },
        CaptureFailure::ReplyLost => "capture reply lost".to_owned(),
        CaptureFailure::MailboxClosed => "session mailbox closed".to_owned(),
        CaptureFailure::OriginUnavailable => "capture origin unavailable".to_owned(),
        CaptureFailure::InvalidEvidence => "capture evidence invalid".to_owned(),
        CaptureFailure::WorkerLost => "capture worker lost".to_owned(),
    }
}

fn body_availability_label(availability: &BodyAvailability) -> String {
    match availability {
        BodyAvailability::Captured { representation, .. } => {
            format!("captured ({})", representation_label(representation))
        }
        BodyAvailability::Expired => "expired (older body blobs were evicted)".to_owned(),
        BodyAvailability::NotCaptured { reason } => match reason {
            NotCapturedReason::Oversize => "not captured (body over the size cap)".to_owned(),
        },
        BodyAvailability::Failed { cause } => capture_failure_label(*cause),
        BodyAvailability::Unknown => "unknown".to_owned(),
    }
}

fn representation_label(representation: &PayloadRepresentation) -> &'static str {
    match representation {
        PayloadRepresentation::ExactLocalJson => "exact local json",
        PayloadRepresentation::Redacted { .. } => "redacted",
    }
}

fn origin_short_id(id: &RequestSnapshotId) -> String {
    format!("{:.8}", id.0)
}

fn short_token(token: &str) -> &str {
    let end = token.char_indices().nth(8).map_or(token.len(), |(idx, _)| idx);
    &token[..end]
}

fn digest_prefix(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for byte in digest.iter().take(6) {
        let _ = write!(out, "{byte:02x}");
    }
    out.push('…');
    out
}
