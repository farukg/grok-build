//! Scrollback entries for the subagent lifecycle.
//!
//! Compact by default, with an animated running bullet and a terminal status bullet.
//! Enter / Ctrl-F opens the subagent view.
//!
//! Blocking calls update one lifecycle row; background calls add a separate terminal row.

use std::time::Duration;

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::app::subagent::format_subagent_meta;
use crate::appearance::AppearanceConfig;
use crate::render::line_utils::truncate_str;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode};
use crate::theme::Theme;
use crate::util::format_duration;

/// What kind of subagent lifecycle event this block represents.
#[derive(Debug, Clone)]
pub enum SubagentBlockKind {
    /// Subagent is running (or was running; `finish_running` stops animation).
    Started,
    /// Subagent completed successfully.
    Completed { elapsed: Duration },
    /// Subagent failed.
    Failed {
        elapsed: Duration,
        error: Option<String>,
    },
    /// Subagent was cancelled.
    Cancelled { elapsed: Duration },
}

/// Collapsed one-liner by default; a row with description, delegation metadata or a failure
/// reason folds open to them. Groupable and selectable.
/// Enter / Ctrl-F opens the subagent view.
#[derive(Debug, Clone)]
pub struct SubagentBlock {
    /// Human-readable description of the task.
    pub description: String,
    /// Child session ID (for opening the subagent view).
    pub child_session_id: String,
    /// Subagent type (e.g. "general-purpose", "explore").
    pub subagent_type: String,
    /// Named persona applied to this subagent, if any.
    pub persona: Option<String>,
    /// Role that supplied defaults for this subagent, if any.
    pub role: Option<String>,
    /// Effective model ID used by the subagent, if available.
    pub model: Option<String>,
    /// Whether the subagent was launched in background mode.
    pub is_background: bool,
    /// Lifecycle kind.
    pub kind: SubagentBlockKind,
    /// Live activity label from the child session's turn tracker. The user sees interactive progress without opening
    /// the subagent view.
    pub activity_label: Option<String>,
}

impl SubagentBlock {
    /// Create a "Subagent started" block (for both sync and async).
    pub fn started(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        subagent_type: impl Into<String>,
        persona: Option<String>,
        role: Option<String>,
        model: Option<String>,
        is_background: bool,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: subagent_type.into(),
            persona,
            role,
            model,
            is_background,
            kind: SubagentBlockKind::Started,
            activity_label: None,
        }
    }

    /// Create a "Subagent completed" block (background mode only).
    pub fn completed(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Completed { elapsed },
            activity_label: None,
        }
    }

    /// Create a "Subagent failed" block (background mode only).
    pub fn failed(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
        error: Option<String>,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Failed { elapsed, error },
            activity_label: None,
        }
    }

    /// Create a "Subagent cancelled" block (background mode only).
    pub fn cancelled(
        description: impl Into<String>,
        child_session_id: impl Into<String>,
        elapsed: Duration,
    ) -> Self {
        Self {
            description: description.into(),
            child_session_id: child_session_id.into(),
            subagent_type: String::new(),
            persona: None,
            role: None,
            model: None,
            is_background: true,
            kind: SubagentBlockKind::Cancelled { elapsed },
            activity_label: None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.kind, SubagentBlockKind::Started)
    }

    /// Delegation preamble; the child transcript remains owned by its session view.
    fn expanded_body(&self, ctx: &BlockContext, theme: &Theme) -> Vec<BlockLine> {
        let mut lines = Vec::new();
        let width = ctx.content_width();

        let description = self.description.trim();
        if !description.is_empty() {
            lines.push(BlockLine::separator(Line::from("")));
            lines.extend(super::tool::body::wrapped_body(
                &self.description, theme.primary(), width, "",
            ));
        }

        let meta = [
            (!self.subagent_type.is_empty()).then(|| self.subagent_type.as_str()),
            self.persona.as_deref().filter(|p| !p.is_empty()),
            self.role.as_deref().filter(|r| !r.is_empty()),
            self.model.as_deref().filter(|m| !m.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" \u{00b7} ");
        if !meta.is_empty() {
            lines.push(BlockLine::separator(Line::from("")));
            lines.extend(super::tool::body::wrapped_body(
                &meta, theme.muted(), width, "  ",
            ));
        }

        if let Some(activity) = self.activity_label.as_deref().filter(|s| !s.is_empty()) {
            lines.extend(super::tool::body::wrapped_body(
                activity, theme.muted(), width, "  ",
            ));
        }

        if let SubagentBlockKind::Failed { error: Some(err), .. } = &self.kind
            && !err.trim().is_empty()
        {
            lines.push(BlockLine::separator(Line::from("")));
            lines.extend(super::tool::body::wrapped_body(
                err, theme.fg(theme.accent_error), width, "",
            ));
        }

        lines
    }
}

/// Truncate description and wrap in quotes for display.
fn quoted_desc(desc: &str, max_width: usize) -> String {
    // Reserve 2 chars for quotes
    if max_width <= 2 {
        return "\u{201C}\u{2026}\u{201D}".to_string(); // "…"
    }
    let inner = truncate_str(desc, max_width - 2);
    format!("\u{201C}{inner}\u{201D}")
}

impl BlockContent for SubagentBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let bold = if ctx.is_selected {
            theme.primary().add_modifier(Modifier::BOLD)
        } else {
            theme.muted().add_modifier(Modifier::BOLD)
        };
        let muted = theme.muted();
        // Bullet-aware budget: the bullet glyph `RenderBlock::output` prepends occupies the
        // first cells of the row, so the summary must fit inside the content width.
        let w = ctx.content_width();

        let line = match (&self.kind, self.is_background) {
            (SubagentBlockKind::Started, bg) => {
                let verb = if bg { "started: " } else { "running: " };
                let activity_suffix: String = self
                    .activity_label
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|a| format!(" \u{00b7} {a}"))
                    .unwrap_or_default();
                let meta = format_subagent_meta(
                    self.persona.as_deref(),
                    self.role.as_deref(),
                    self.model.as_deref(),
                );
                let overhead = crate::glyphs::history_subagent().width() + 1 + verb.width()
                    + meta.width() + activity_suffix.width();
                let desc = quoted_desc(&self.description, w.saturating_sub(overhead));
                let mut spans = vec![
                    Span::styled(format!("{} ", crate::glyphs::history_subagent()), bold),
                    Span::styled(verb, muted),
                    Span::styled(desc, muted),
                ];
                if !activity_suffix.is_empty() {
                    spans.push(Span::styled(activity_suffix, muted));
                }
                spans.push(Span::styled(meta, muted));
                Line::from(spans)
            }
            // Completed: Subagent completed in Xs: "description"
            (SubagentBlockKind::Completed { elapsed }, _) => {
                let time_str = format_duration(*elapsed);
                let prefix_len = crate::glyphs::history_subagent().width() + 1
                    + format!("completed in {time_str}: ").width();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                Line::from(vec![
                    Span::styled(format!("{} ", crate::glyphs::history_subagent()), bold),
                    Span::styled(format!("completed in {time_str}: "), muted),
                    Span::styled(desc, muted),
                ])
            }
            // Failed: Subagent failed in Xs: "description"
            (SubagentBlockKind::Failed { elapsed, error }, _) => {
                let time_str = format_duration(*elapsed);
                let detail = error
                    .as_deref()
                    .map(|e| format!(" ({e})"))
                    .unwrap_or_default();
                let prefix_len = crate::glyphs::history_subagent().width() + 1
                    + format!("failed in {time_str}{detail}: ").width();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                Line::from(vec![
                    Span::styled(format!("{} ", crate::glyphs::history_subagent()), bold),
                    Span::styled(format!("failed in {time_str}{detail}: "), muted),
                    Span::styled(desc, muted),
                ])
            }
            // Cancelled: Subagent cancelled in Xs: "description"
            (SubagentBlockKind::Cancelled { elapsed }, _) => {
                let time_str = format_duration(*elapsed);
                let prefix_len = crate::glyphs::history_subagent().width() + 1
                    + format!("cancelled in {time_str}: ").width();
                let desc = quoted_desc(&self.description, w.saturating_sub(prefix_len));
                Line::from(vec![
                    Span::styled(format!("{} ", crate::glyphs::history_subagent()), bold),
                    Span::styled(format!("cancelled in {time_str}: "), muted),
                    Span::styled(desc, muted),
                ])
            }
        };

        let line = Line::from(line.spans.into_iter().map(|span| {
            Span::styled(span.content.replace('\n', " "), span.style)
        }).collect::<Vec<_>>());
        let line = crate::render::line_utils::truncate_line(line, w);
        // Collapsed stays the one-line summary; Expanded appends the body the one-liner
        // truncates away (full description, delegation metadata, failure reason).
        if ctx.mode == DisplayMode::Collapsed {
            return BlockOutput {
                lines: vec![line.into()],
            };
        }
        let mut lines = vec![line.into()];
        lines.extend(self.expanded_body(ctx, &theme));
        BlockOutput { lines }
    }

    fn accent(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.kind {
            SubagentBlockKind::Started if ctx.is_running => {
                Some(AccentStyle::static_color(theme.accent_running))
            }
            _ => None,
        }
    }

    fn bullet(&self, ctx: &BlockContext) -> Option<AccentStyle> {
        let theme = Theme::current();
        match &self.kind {
            SubagentBlockKind::Started => {
                if ctx.is_running {
                    Some(AccentStyle::animated_running(ctx, &theme))
                } else {
                    // Finished: gray bullet (same as bg task "started" after completion)
                    None
                }
            }
            SubagentBlockKind::Completed { .. } => {
                Some(AccentStyle::static_color(theme.accent_success))
            }
            SubagentBlockKind::Failed { .. } | SubagentBlockKind::Cancelled { .. } => {
                Some(AccentStyle::static_color(theme.accent_error))
            }
        }
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        false
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        // The fold reveals the untruncated description, the delegation metadata or the
        // failure reason; a row with none of those offers no fake expand.
        !self.description.trim().is_empty()
            || self.activity_label.as_deref().is_some_and(|a| !a.is_empty())
            || !self.subagent_type.is_empty()
            || self.persona.as_deref().is_some_and(|p| !p.is_empty())
            || self.role.as_deref().is_some_and(|r| !r.is_empty())
            || self.model.as_deref().is_some_and(|m| !m.is_empty())
            || matches!(&self.kind, SubagentBlockKind::Failed { error: Some(err), .. } if !err.trim().is_empty())
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Collapsed
    }

    fn is_selectable(&self) -> bool {
        true
    }

    fn has_bullet(&self, _ctx: &BlockContext) -> bool {
        true
    }

    fn is_groupable(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::AppearanceConfig;

    fn ctx(mode: DisplayMode) -> BlockContext {
        BlockContext {
            mode,
            is_running: false,
            width: 100,
            raw: false,
            max_lines: None,
            appearance: AppearanceConfig::default(),
            is_selected: false,
            cwd: None,
        }
    }

    fn rendered_text(block: &SubagentBlock, mode: DisplayMode) -> String {
        block
            .output(&ctx(mode))
            .lines
            .iter()
            .map(|l| {
                l.content
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn failed_subagent_expands_to_its_error_body() {
        let block = SubagentBlock::failed(
            "investigate flaky test",
            "child-1",
            Duration::from_secs(3),
            Some("panicked at assert".into()),
        );

        assert!(block.is_foldable());
        let expanded = block.output(&ctx(DisplayMode::Expanded));
        assert!(
            expanded.lines.len() > 1,
            "the failure reason must live inline, not only in the viewer"
        );
        let text = rendered_text(&block, DisplayMode::Expanded);
        assert!(text.contains("panicked at assert"));
    }

    #[test]
    fn collapsed_stays_the_one_line_summary() {
        let block = SubagentBlock::failed(
            "investigate flaky test",
            "child-1",
            Duration::from_secs(3),
            Some("panicked at assert".into()),
        );
        let collapsed = rendered_text(&block, DisplayMode::Collapsed);
        assert_eq!(collapsed.lines().count(), 1);
    }

    #[test]
    fn started_expands_to_delegation_metadata() {
        let block = SubagentBlock::started(
            "check the failing build",
            "child-2",
            "explore",
            Some("scout".into()),
            None,
            Some("grok-test".into()),
            false,
        );
        let expanded = rendered_text(&block, DisplayMode::Expanded);
        assert!(
            expanded.contains("explore · scout · grok-test"),
            "expanded:\n{expanded}"
        );
    }

    #[test]
    fn a_row_without_body_offers_no_fold() {
        let block = SubagentBlock::completed("", "child-3", Duration::from_secs(1));
        assert!(!block.is_foldable());
    }
}
