//! Fullscreen subagent takeover for [`AgentView`]: opening and closing the child view, drawing its framed transcript,
//! routing input to it, and fetching a child for a live update (which hydrates a resumed child's transcript first).
//!
//! Invariants: `active_subagent` is the sole "takeover open" signal, and every close routes through
//! `close_subagent_fullscreen` so `evict_finished_child_view` runs exactly once. While open, the intercept runs as
//! step 0 of `handle_input_inner`, before any parent routing, and the child's `pending_effects` are hoisted because
//! `AppView` drains only the top-level view's queue. Ctrl+Q is never consumed here; it always bubbles to the global quit.
use crate::actions::ActionRegistry;
use crate::app::agent_view::child_action_filter::filter_child_outcome;
use crate::app::agent_view::viewer::IdleEnterQuote;
use crate::app::agent_view::{AgentView, AppRenderParams, OverlayHeader};
use crate::app::app_view::InputOutcome;
use crate::key;
use crate::render::SafeBuf;
use crate::scrollback::render::ScratchBuffer;
use crate::theme::Theme;
use crate::views::agent;
use crate::views::shortcuts_bar::PendingHint;
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
/// What a subagent's fullscreen takeover inherits from its parent. The header keeps describing the parent, so its session
/// switcher (and, in the dashboard overlay, its title) come from it. `stop_label` is the parent's `Ctrl+X` action and is
/// `Some` exactly when the parent sits in the dashboard overlay (its pending confirmation already arrives as `pending_hint`).
#[derive(Clone, Copy)]
pub(super) struct InheritedOverlay<'a> {
    pub(super) header: OverlayHeader<'a>,
    pub(super) stop_label: Option<&'static str>,
}
/// Sibling switch direction for the Ctrl+Alt+Left/Right chords.
#[derive(Clone, Copy)]
pub(in crate::app) enum Direction {
    Prev,
    Next,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum CycleOutcome {
    Switched,
    NoSibling,
}
/// A Ctrl+Alt+Arrow subagent-tree navigation chord.
#[derive(Clone, Copy)]
pub(in crate::app) enum TreeChord {
    Parent,
    LatestChild,
}
/// Ctrl+Alt+Up/Down stay raw for tree navigation while a takeover is open; Left/Right resolve as cycle actions.
pub(in crate::app) fn tree_chord(ev: &Event) -> Option<TreeChord> {
    let Event::Key(key) = ev else {
        return None;
    };
    if key.kind == KeyEventKind::Release
        || !key
            .modifiers
            .contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
        || key.modifiers.contains(KeyModifiers::SHIFT)
    {
        return None;
    }
    match key.code {
        KeyCode::Up => Some(TreeChord::Parent),
        KeyCode::Down => Some(TreeChord::LatestChild),
        _ => None,
    }
}
const NO_CHILD_TOAST: &str = "No child subagent to open";
pub(in crate::app) const NO_SIBLING_TOAST: &str = "No sibling subagent";
/// A node of the subagent tree this root view owns. Every child view, at any depth, is registered on the root with
/// its real parent link; a child whose parent has no view here hangs off the root session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeNode<'a> {
    Root,
    Child(&'a str),
}
impl AgentView {
    fn tree_parent(&self, child_sid: &str) -> Option<TreeNode<'_>> {
        let parent_sid = self
            .subagent_views
            .get(child_sid)?
            .child_link()?
            .parent_session_id()
            .0
            .as_ref();
        Some(
            self.subagent_views
                .get_key_value(parent_sid)
                .map_or(TreeNode::Root, |(sid, _)| TreeNode::Child(sid.as_str())),
        )
    }
    /// Direct children of `node`, earliest started first.
    fn child_order_key(&self, sid: &str) -> (Option<std::time::Instant>, &str) {
        (
            self.subagent_sessions
                .get(sid)
                .map(|info| info.attempt.started_at),
            sid,
        )
    }
    fn children_of(&self, node: TreeNode<'_>) -> Vec<String> {
        let mut children: Vec<&str> = self
            .subagent_views
            .keys()
            .map(String::as_str)
            .filter(|sid| self.tree_parent(sid) == Some(node))
            .collect();
        children.sort_by_key(|sid| self.child_order_key(sid));
        children.into_iter().map(str::to_owned).collect()
    }
    pub(in crate::app) fn sibling_position(&self, child_sid: &str) -> Option<(usize, usize)> {
        let Some(parent) = self.tree_parent(child_sid) else {
            return None;
        };
        let target_key = self.child_order_key(child_sid);
        let mut sibling_count = 0;
        let mut sibling_rank = 0;
        for sid in self.subagent_views.keys().map(String::as_str) {
            if self.tree_parent(sid) == Some(parent) {
                sibling_count += 1;
                if self.child_order_key(sid) < target_key {
                    sibling_rank += 1;
                }
            }
        }
        (sibling_count > 1).then_some((sibling_rank + 1, sibling_count))
    }
    /// Open the most recently started child of `node`; on a leaf, tell the user on the view they are looking at.
    fn open_latest_child(&mut self, node: TreeNode<'_>) -> InputOutcome {
        if let Some(latest) = self.children_of(node).pop() {
            self.open_subagent_fullscreen(latest);
            return InputOutcome::Changed;
        }
        match node {
            TreeNode::Root => self.show_toast(NO_CHILD_TOAST),
            TreeNode::Child(sid) => {
                if let Some(child) = self.subagent_views.get_mut(sid) {
                    child.show_toast(NO_CHILD_TOAST);
                }
            }
        }
        InputOutcome::Changed
    }
    /// Ctrl+Alt+Down with nothing blocking the keyboard and no open takeover enters the root's latest child. Up has no
    /// parent here, and Left/Right stay free for session cycling, so neither is claimed.
    pub(super) fn intercept_root_tree_input(&mut self, ev: &Event) -> Option<InputOutcome> {
        match tree_chord(ev)? {
            TreeChord::LatestChild => Some(self.open_latest_child(TreeNode::Root)),
            TreeChord::Parent => None,
        }
    }
    /// Switch the takeover to the previous/next sibling of `child_sid` (same
    /// parent, start order, wrapping). Returns the new child sid when a switch happened.
    pub(in crate::app) fn cycle_sibling(
        &mut self,
        child_sid: &str,
        order: &[String],
        dir: Direction,
    ) -> CycleOutcome {
        if order.len() < 2 {
            return CycleOutcome::NoSibling;
        }
        let Some(idx) = order.iter().position(|sid| sid == child_sid) else {
            return CycleOutcome::NoSibling;
        };
        let next = match dir {
            Direction::Prev => (idx + order.len() - 1) % order.len(),
            Direction::Next => (idx + 1) % order.len(),
        };
        if let Some(next) = order.get(next) {
            self.open_subagent_fullscreen(next.clone());
            CycleOutcome::Switched
        } else {
            CycleOutcome::NoSibling
        }
    }
    /// Open the fullscreen subagent view for `child_sid`, replaying child `updates.jsonl` when the child scrollback is still empty (or the child finished).
    pub(crate) fn open_subagent_fullscreen(&mut self, child_sid: String) {
        let Some(child) = self.subagent_views.get(&child_sid) else {
            return;
        };
        let parent_session_id = child
            .child_link()
            .map(|link| link.parent_session_id().clone());
        if self.active_subagent.as_deref() != Some(child_sid.as_str()) {
            self.close_subagent_fullscreen();
        }
        let replay_outcome = crate::app::subagent::ensure_subagent_child_replayed(self, &child_sid);
        tracing::debug!(
            child_sid = %child_sid,
            ?parent_session_id,
            ?replay_outcome,
            "opened subagent fullscreen"
        );
        self.active_subagent = Some(child_sid);
    }
    /// Open the takeover for the child the selected row links to (`RenderBlock::child_session_id`). `false`, so the
    /// caller keeps its routing, on a group header, an unlinked row, or a child this view does not own (a row
    /// replayed inside a child transcript).
    pub(crate) fn try_open_child_from_selected_row(&mut self) -> bool {
        let child_sid = self.selected_linked_child();
        self.open_owned_child(child_sid)
    }
    /// The child the selected row links to, owned here or not; `None` on a group header or an unlinked row.
    fn selected_linked_child(&self) -> Option<String> {
        if self.scrollback.is_selected_group_header() {
            return None;
        }
        self.scrollback
            .selected()
            .and_then(|idx| self.scrollback.entry(idx))
            .and_then(|entry| entry.block.child_session_id())
            .map(str::to_owned)
    }
    /// Open the takeover for `child_sid` when this view owns it, whatever its state (running, finished, evicted → replay).
    fn open_owned_child(&mut self, child_sid: Option<String>) -> bool {
        let Some(child_sid) = child_sid.filter(|sid| self.subagent_views.contains_key(sid)) else {
            return false;
        };
        self.open_subagent_fullscreen(child_sid);
        true
    }
    /// Select the scrollback row at screen `row` and name the child it links to, owned here or not (a row replayed
    /// inside a child transcript links a child the root owns).
    pub(crate) fn linked_child_at_scrollback_row(&mut self, row: u16) -> Option<String> {
        let idx = self
            .scrollback
            .entry_index_at_screen_row(row, self.pane_areas.scrollback)?;
        self.scrollback.set_selected(Some(idx));
        self.selected_linked_child()
    }
    /// Ctrl+Alt+Click: open the owned child `child_sid`. The press hands the pointer to the takeover, so this view's
    /// click gesture ends here.
    pub(crate) fn open_child_from_click(&mut self, child_sid: Option<String>) -> bool {
        if !self.open_owned_child(child_sid) {
            return false;
        }
        self.left_mouse_down = false;
        true
    }
    /// Close the fullscreen subagent takeover (if any), evicting the closed child when finished.
    /// See [`crate::app::subagent::evict_finished_child_view`] for rationale and guards.
    /// All close sites route through here.
    pub(crate) fn close_subagent_fullscreen(&mut self) {
        if let Some(child_sid) = self.active_subagent.take() {
            let _ = crate::app::subagent::evict_finished_child_view(self, &child_sid);
        }
    }
    /// Fetch a child view for applying a live update, hydrating a resumed child's inherited transcript first.
    /// The incoming block then never closes the replay window (see [`crate::app::subagent::replay_resumed_child_before_live_block`]).
    /// The funnel for every apply that can be a resumed child's *first* live block: the ACP and xAI child ingresses and the finish-path finalize.
    pub(crate) fn child_view_for_live_update_mut(
        &mut self,
        child_sid: &str,
    ) -> Option<&mut AgentView> {
        crate::app::subagent::replay_resumed_child_before_live_block(self, child_sid);
        self.subagent_views.get_mut(child_sid).map(|v| &mut **v)
    }
    /// Paint the takeover frame (title row with status, elapsed, and the `[\u{2717}]` close button) around the child view's own `draw`.
    /// Returns the child's cursor and post-flush so the parent frame forwards them unchanged.
    #[expect(clippy::too_many_arguments)]
    pub(super) fn draw_subagent_fullscreen(
        &mut self,
        child_sid: &str,
        area: Rect,
        buf: &mut Buffer,
        registry: &ActionRegistry,
        scratch: &mut ScratchBuffer,
        pending_hint: Option<PendingHint>,
        theme: &Theme,
        overlay: InheritedOverlay<'_>,
    ) -> (
        Option<(u16, u16)>,
        Option<crate::terminal::overlay::PostFlush>,
    ) {
        use crate::app::subagent::{format_context_badge, format_subagent_label};
        use ratatui::style::Modifier;
        use unicode_width::UnicodeWidthStr;
        let appearance = self.scrollback.appearance().clone();
        let layout_cfg = &appearance.scrollback.layout;
        let compact = appearance.prompt.compact;
        agent::fill_background(buf, area, layout_cfg, compact, theme);
        let padded = Rect {
            x: area.x + layout_cfg.eff_hpad_left(compact),
            y: area.y + layout_cfg.eff_outer_vpad(compact),
            width: area.width.saturating_sub(
                layout_cfg.eff_hpad_left(compact) + layout_cfg.eff_hpad_right(compact),
            ),
            height: area
                .height
                .saturating_sub(layout_cfg.eff_outer_vpad(compact) * 2),
        };
        if padded.width < 10 || padded.height < 5 {
            return (None, crate::terminal::overlay::clear().map(Into::into));
        }
        let border_color = theme.selection_border;
        let frame = match crate::views::picker::render_bordered_frame(
            buf,
            padded,
            border_color,
            theme.bg_base,
        ) {
            Some(f) => f,
            None => {
                return (None, crate::terminal::overlay::clear().map(Into::into));
            }
        };
        let title_y = frame.title_row.y;
        let _title_row = frame.title_row;
        let inner = frame.content;
        let _border_style = Style::default().fg(border_color);
        let info = self.subagent_sessions.get(child_sid);
        let raw_description = info.map(|s| s.description.as_ref()).unwrap_or("subagent");
        let is_running = info.is_some_and(|s| s.is_running());
        let elapsed = info
            .map(|s| crate::util::format_duration(s.display_elapsed()))
            .unwrap_or_default();
        let (type_label, description): (String, String) = match info {
            Some(s) => format_subagent_label(s),
            None => (String::new(), raw_description.to_string()),
        };
        let icon = if is_running {
            let spinner_frames = crate::glyphs::dot_spinner_frames();
            let tick = self.tasks.tick_count();
            match spinner_frames.len() {
                0 => "",
                n => spinner_frames
                    .get((tick / 4) as usize % n)
                    .copied()
                    .unwrap_or(""),
            }
        } else if info.and_then(|s| s.attempt.status.as_deref()) == Some("completed") {
            crate::glyphs::check_mark()
        } else {
            crate::glyphs::ballot_x()
        };
        let icon_color = if is_running {
            theme.accent_running
        } else if info.and_then(|s| s.attempt.status.as_deref()) == Some("completed") {
            theme.accent_success
        } else {
            theme.accent_error
        };
        let label_color = if info.is_some_and(|s| s.attempt.pending_kill) {
            theme.accent_error
        } else if is_running {
            theme.accent_running
        } else if info.and_then(|s| s.attempt.status.as_deref()) == Some("completed") {
            theme.accent_success
        } else {
            theme.accent_error
        };
        let model = info
            .and_then(|s| s.attempt.model.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|requested| match self.subagent_views.get(child_sid) {
                Some(child) => child.session.models.served_label_for(requested),
                None => requested.to_owned(),
            });
        let tokens = info.and_then(|s| s.attempt.token_usage_label());
        let meta = match (model, tokens) {
            (Some(model), Some(tokens)) => format!("{model} \u{b7} {tokens}"),
            (Some(model), None) => model,
            (None, Some(tokens)) => tokens,
            (None, None) => String::new(),
        };
        let badge = info.map(format_context_badge).unwrap_or_default();
        let activity_label: Option<String> = if is_running {
            self.subagent_views.get(child_sid).and_then(|cv| {
                cv.resolve_turn_activity()
                    .map(|a| crate::app::subagent::format_activity_label(&a))
                    .or_else(|| cv.session.state.is_busy().then(|| "Waiting".to_string()))
            })
        } else {
            None
        };
        let title_x = padded.x + 1;
        buf.set_span_safe(
            title_x,
            title_y,
            &Span::styled(format!(" {icon}"), Style::default().fg(icon_color)),
            3,
        );
        let close_text = "[\u{2717}]";
        let close_width: u16 = close_text.width() as u16;
        let elapsed_text = elapsed.clone();
        let right_margin: u16 = 1;
        let badge_width = if badge.is_empty() {
            0
        } else {
            badge.width() as u16 + 1
        };
        let activity_width: u16 = activity_label
            .as_deref()
            .map(|s| s.width() as u16 + 3)
            .unwrap_or(0);
        let right_width = activity_width
            + elapsed_text.width() as u16
            + 1
            + close_width
            + right_margin
            + badge_width;
        let desc_start_x = title_x + 3;
        let avail = padded.width.saturating_sub(5 + right_width) as usize;
        let type_text = if type_label.is_empty() {
            String::new()
        } else if description.is_empty() {
            type_label.clone()
        } else {
            format!("{type_label} ")
        };
        let meta_text = if meta.is_empty() {
            String::new()
        } else {
            format!(" {meta}")
        };
        let overhead = type_text.width() + meta_text.width();
        let desc_max = avail.saturating_sub(overhead);
        let desc_display = crate::render::line_utils::truncate_str(&description, desc_max);
        if !type_text.is_empty() {
            buf.set_span_safe(
                desc_start_x,
                title_y,
                &Span::styled(&type_text, Style::default().fg(label_color)),
                type_text.width() as u16,
            );
        }
        let desc_x = desc_start_x + type_text.width() as u16;
        buf.set_span_safe(
            desc_x,
            title_y,
            &Span::styled(
                &desc_display,
                Style::default()
                    .fg(theme.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            desc_display.width() as u16,
        );
        let after_desc_x = desc_x + desc_display.width() as u16;
        if !meta_text.is_empty() {
            buf.set_span_safe(
                after_desc_x,
                title_y,
                &Span::styled(&meta_text, Style::default().fg(theme.gray)),
                meta_text.width() as u16,
            );
        }
        let mut rx = padded.x + padded.width.saturating_sub(right_margin + close_width + 1);
        let close_style = if self.hit_subagent_frame_close.hovered {
            Style::default()
                .fg(theme.text_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.gray)
        };
        buf.set_span_safe(
            rx,
            title_y,
            &Span::styled(close_text, close_style),
            close_width,
        );
        self.hit_subagent_frame_close.rect = Some(Rect::new(rx, title_y, close_width, 1));
        rx = rx.saturating_sub(elapsed_text.width() as u16 + 1);
        buf.set_span_safe(
            rx,
            title_y,
            &Span::styled(&elapsed_text, Style::default().fg(theme.gray)),
            elapsed_text.width() as u16,
        );
        if let Some(activity) = activity_label.as_deref() {
            let segment = format!("{activity} \u{00b7} ");
            let w = segment.width() as u16;
            rx = rx.saturating_sub(w);
            buf.set_span_safe(
                rx,
                title_y,
                &Span::styled(segment, Style::default().fg(theme.gray)),
                w,
            );
        }
        if !badge.is_empty() {
            rx = rx.saturating_sub(badge.width() as u16 + 1);
            buf.set_span_safe(
                rx,
                title_y,
                &Span::styled(badge.as_ref(), Style::default().fg(theme.gray_dim)),
                badge.width() as u16,
            );
        }
        let mut child_cursor = None;
        let mut child_post_flush = None;
        if inner.width > 5
            && inner.height > 3
            && let Some(child_view) = self.subagent_views.get_mut(child_sid)
        {
            let (cursor, post_flush) = child_view.draw(
                inner,
                buf,
                registry,
                scratch,
                pending_hint,
                false,
                crate::app::agent_view::BannerSlotParams::none(),
                overlay.stop_label.is_some(),
                &mut Vec::new(),
                AppRenderParams {
                    overlay_header: overlay.header,
                    overlay_stop_label: overlay.stop_label,
                    ..AppRenderParams::default()
                },
            );
            child_cursor = cursor;
            child_post_flush = post_flush;
        }
        (child_cursor, child_post_flush)
    }
    /// `None` when no takeover is open, so the caller continues its normal routing. Otherwise all input goes to the
    /// child view; `q`/`Esc` from bare scrollback closes the view and Ctrl+Q always bubbles to the global quit.
    /// Rung order is fixed: Ctrl+Q, `[✗]` click, hover, bare-scrollback close, idle-quote, forward + filter.
    pub(super) fn intercept_takeover_input(
        &mut self,
        ev: &Event,
        registry: &ActionRegistry,
        prompt_paging: bool,
    ) -> Option<InputOutcome> {
        let child_sid = self.active_subagent.clone()?;
        let key = match ev {
            Event::Key(key) if key.kind != KeyEventKind::Release => Some(key),
            _ => None,
        };
        if key.is_some_and(|key| key!('q', CONTROL).matches(key)) {
            return Some(InputOutcome::Unchanged);
        }
        // Tree navigation chords, handled raw before any child routing.
        if let Some(chord) = tree_chord(ev) {
            return Some(match chord {
                TreeChord::Parent => match self.tree_parent(&child_sid) {
                    Some(TreeNode::Child(parent)) => {
                        let parent = parent.to_owned();
                        self.open_subagent_fullscreen(parent);
                        InputOutcome::Changed
                    }
                    // Direct child of the root: leave the takeover.
                    Some(TreeNode::Root) | None => {
                        self.close_subagent_fullscreen();
                        InputOutcome::Changed
                    }
                },
                TreeChord::LatestChild => self.open_latest_child(TreeNode::Child(&child_sid)),
            });
        }
        if let Event::Mouse(mouse) = ev
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self
                .hit_subagent_frame_close
                .contains(mouse.column, mouse.row)
        {
            self.close_subagent_fullscreen();
            return Some(InputOutcome::Changed);
        }
        if let Event::Mouse(mouse) = ev
            && matches!(mouse.kind, MouseEventKind::Moved)
            && self
                .hit_subagent_frame_close
                .update_hover(mouse.column, mouse.row)
        {
            return Some(InputOutcome::Changed);
        }
        // A subagent row inside the child's transcript links a grandchild, which only this root view owns.
        if let Event::Mouse(mouse) = ev
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && super::is_open_child_click(mouse.modifiers)
        {
            let linked = self.subagent_views.get_mut(&child_sid).and_then(|child| {
                child
                    .pane_areas
                    .scrollback
                    .contains((mouse.column, mouse.row).into())
                    .then(|| child.linked_child_at_scrollback_row(mouse.row))
                    .flatten()
            });
            if self.open_child_from_click(linked) {
                return Some(InputOutcome::Changed);
            }
        }
        let child_in_scrollback = self
            .subagent_views
            .get(&child_sid)
            .is_some_and(|c| c.is_bare_scrollback());
        if child_in_scrollback
            && key.is_some_and(|key| key!('q').matches(key) || key.code == KeyCode::Esc)
        {
            self.close_subagent_fullscreen();
            return Some(InputOutcome::Changed);
        }
        let child_quote = key.and_then(|key| {
            self.subagent_views
                .get_mut(&child_sid)
                .map(|child| child.try_take_idle_enter_quote(key))
        });
        match child_quote {
            Some(IdleEnterQuote::Quoted(quoted)) => {
                self.close_subagent_fullscreen();
                self.insert_quoted_reply(&quoted);
                return Some(InputOutcome::Changed);
            }
            Some(IdleEnterQuote::ConsumedEmpty) => return Some(InputOutcome::Changed),
            Some(IdleEnterQuote::NotHandled) | None => {}
        }
        let Some(child_view) = self.subagent_views.get_mut(&child_sid) else {
            return Some(InputOutcome::Unchanged);
        };
        let outcome = child_view.handle_input_inner(ev, registry, prompt_paging);
        let mut child_effects = std::mem::take(&mut child_view.pending_effects);
        self.pending_effects.append(&mut child_effects);
        Some(filter_child_outcome(outcome))
    }
}
#[cfg(test)]
#[path = "subagent_takeover_tests.rs"]
mod tests;
