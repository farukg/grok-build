//! Runtime-injected context entries: auto-wake prompt echoes and request-only records.
//! Collapsed by default (kind default, Read-style); expanded renders the full body through the
//! same content-styling path Read uses so what the model saw is what the user can inspect.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::appearance::AppearanceConfig;
use crate::render::content_kind::styled_rows;
use crate::render::wrapping::word_wrap_lines_with_joiners;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{
    AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode,
};
use crate::syntax::get_syntect;
use crate::theme::Theme;

/// Char cap on the collapsed header's inline body preview; the full body lives behind the fold.
const COLLAPSED_PREVIEW_CHARS: usize = 80;

#[derive(Debug, Clone)]
pub struct InjectedBlock {
    /// Short surface label ("Task completed", "Memory reminder", …), derived from the
    /// `SyntheticReason` wire name or the request-record kind.
    pub label: String,
    /// Full injected body exactly as it entered the request.
    pub text: String,
    /// Whether this entry started a numbered prompt turn (an auto-wake echo); mid-turn records
    /// never split the turn outline.
    pub starts_turn: bool,
}

impl InjectedBlock {
    pub fn new(label: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            text: text.into(),
            starts_turn: false,
        }
    }

    /// Mark the entry as the start of a numbered prompt turn (auto-wake echo).
    pub fn with_starts_turn(mut self, starts_turn: bool) -> Self {
        self.starts_turn = starts_turn;
        self
    }

    /// First non-empty body line for the collapsed header preview.
    fn first_body_line(&self) -> &str {
        self.text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
    }

    fn header_line(&self, theme: &Theme, muted: bool) -> Line<'static> {
        let label_style = if muted {
            theme.muted()
        } else {
            Style::default().fg(theme.gray_bright)
        };
        let mut spans = vec![Span::styled(self.label.clone(), label_style)];
        let preview = self.first_body_line();
        if muted && !preview.is_empty() {
            let preview = if preview.chars().count() > COLLAPSED_PREVIEW_CHARS {
                let truncated: String = preview.chars().take(COLLAPSED_PREVIEW_CHARS).collect();
                format!("{truncated}\u{2026}")
            } else {
                preview.to_string()
            };
            spans.push(Span::styled(" \u{2014} ".to_string(), theme.muted()));
            spans.push(Span::styled(preview, theme.muted()));
        }
        Line::from(spans)
    }

    /// Expanded body rows: same grammar-less styling path Read uses for plain content.
    fn body_lines(&self, ctx: &BlockContext, theme: &Theme) -> Vec<BlockLine> {
        if self.text.is_empty() {
            return Vec::new();
        }
        let text_style = theme.dim();
        let rows = styled_rows(
            get_syntect(),
            Path::new(""),
            &self.text,
            ctx.appearance.scrollback.blocks.tool.terminal_coloring,
            text_style,
        );
        let body: Vec<Line<'static>> = rows.into_iter().map(Line::from).collect();
        let (wrapped, joiners) = word_wrap_lines_with_joiners(body, ctx.content_width() as usize);
        wrapped
            .into_iter()
            .zip(joiners)
            .map(|(line, joiner)| {
                BlockLine::styled(line)
                    .with_selection_range(Some(0))
                    .with_joiner(joiner)
            })
            .collect()
    }
}

impl BlockContent for InjectedBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        match ctx.mode {
            DisplayMode::Collapsed | DisplayMode::Truncated => BlockOutput {
                lines: vec![BlockLine::styled(self.header_line(&theme, true))
                    .with_selection_range(Some(0))],
            },
            DisplayMode::Expanded => {
                let mut lines =
                    vec![BlockLine::styled(self.header_line(&theme, false))
                        .with_selection_range(Some(0))];
                if !self.text.is_empty() {
                    lines.push(BlockLine::separator(Line::from("")));
                    lines.extend(self.body_lines(ctx, &theme));
                }
                BlockOutput { lines }
            }
        }
    }

    fn accent(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        None
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        false
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        !self.text.is_empty()
    }

    fn is_groupable(&self) -> bool {
        true
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Collapsed
    }
}

#[cfg(test)]
#[path = "injected_tests.rs"]
mod tests;


/// Human label for a `SyntheticReason` wire name (the chunk-meta string) or a request-record
/// kind wire name. Unknown future reasons fall back to a generic label; they still render.
pub fn injected_label(reason: &str) -> String {
    use serde::Deserialize;
    use serde::de::value::StrDeserializer;
    use xai_grok_shell::sampling::SyntheticReason;

    match reason {
        "memory_reminder" => return "Memory reminder".to_string(),
        "context_view" => return "Context view".to_string(),
        _ => {}
    }
    let decoded = SyntheticReason::deserialize(StrDeserializer::<serde::de::value::Error>::new(reason));
    let Ok(decoded) = decoded else {
        return format!("Injected ({reason})");
    };
    match decoded {
        SyntheticReason::Human => "Your message",
        SyntheticReason::Primary => "Core instructions",
        SyntheticReason::CompactionMeta => "Compaction summary",
        SyntheticReason::SystemReminder => "System reminder",
        SyntheticReason::LengthContinue => "Length continue",
        SyntheticReason::ProjectInstructions => "Project rules",
        SyntheticReason::AutoContinue => "Auto-continue",
        SyntheticReason::AutoRecovery => "Auto-recovery",
        SyntheticReason::Interjection => "Interjection",
        SyntheticReason::AgentMessage => "Agent message",
        SyntheticReason::ParentHumanMessage => "Parent message",
        SyntheticReason::TaskCompleted => "Task completed",
        SyntheticReason::SubagentCompleted => "Subagent completed",
        SyntheticReason::NotificationDrain => "Notification drain",
        SyntheticReason::GoalSummary => "Goal continuation",
        SyntheticReason::GoalClassifierNudge => "Goal check",
        SyntheticReason::SchedulerFired => "Scheduled task",
        SyntheticReason::StopHookFeedback => "Stop-hook feedback",
        SyntheticReason::WorkingDirectorySwitch => "Directory switch",
        SyntheticReason::SessionPrefix => "Session context",
        SyntheticReason::DirectBash => "Direct command",
        SyntheticReason::GoalSetup => "Goal setup",
        SyntheticReason::SkillCatalog => "Skill catalog",
        SyntheticReason::McpCatalog => "MCP catalog",
        SyntheticReason::Unknown => return format!("Injected ({reason})"),
    }.to_string()
}
