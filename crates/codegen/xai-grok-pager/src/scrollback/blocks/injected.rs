//! Runtime-injected context entries: auto-wake prompt echoes and request-only records.
//! Collapsed by default (kind default, Read-style); expanded renders the full body through the
//! same content-styling path Read uses so what the model saw is what the user can inspect.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::appearance::AppearanceConfig;
use crate::render::content_kind::styled_rows;
use crate::render::wrapping::word_wrap_lines_with_joiners;
use crate::scrollback::block::{BlockContent, MessageKind};
use crate::scrollback::types::{
    AccentStyle, BlockContext, BlockLine, BlockOutput, DisplayMode,
};
use crate::syntax::get_syntect;
use crate::theme::Theme;

#[derive(Debug, Clone)]
pub struct InjectedBlock {
    kind: MessageKind,
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
            kind: MessageKind::Injected,
            label: label.into(),
            text: text.into(),
            starts_turn: false,
        }
    }

    pub fn from_reason(reason: &str, text: impl Into<String>) -> Self {
        let (kind, label) = decode_reason(reason);
        Self { kind, label, text: text.into(), starts_turn: false }
    }

    pub fn message_kind(&self) -> MessageKind { self.kind }

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

    fn header_line(&self, theme: &Theme, muted: bool, width: usize) -> Line<'static> {
        let label_style = if muted {
            theme.muted()
        } else {
            Style::default().fg(theme.gray_bright)
        };
        let mut spans = vec![Span::styled(self.label.clone(), label_style)];
        let preview = self.first_body_line();
        if muted && !preview.is_empty() {
            let used = unicode_width::UnicodeWidthStr::width(self.label.as_str())
                + unicode_width::UnicodeWidthStr::width(" — ");
            let preview = crate::render::line_utils::truncate_str(
                preview, width.saturating_sub(used),
            );
            spans.push(Span::styled(" \u{2014} ".to_string(), theme.muted()));
            spans.push(Span::styled(preview, theme.muted()));
        }
        if muted {
            crate::render::line_utils::truncate_line(Line::from(spans), width)
        } else {
            Line::from(spans)
        }
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
        let (wrapped, joiners) = word_wrap_lines_with_joiners(body, ctx.width as usize);
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
                lines: vec![BlockLine::styled(self.header_line(
                    &theme,
                    true,
                    ctx.width as usize,
                ))
                .with_selection_range(Some(0))],
            },
            DisplayMode::Expanded => {
                let mut lines: Vec<_> = super::tool::body::wrapped_lines(
                    [self.header_line(&theme, false, ctx.width as usize)],
                    ctx.width as usize, "",
                ).into_iter().map(|line| line.with_selection_range(Some(0))).collect();
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
    decode_reason(reason).1
}

fn decode_reason(reason: &str) -> (MessageKind, String) {
    use serde::Deserialize;
    use serde::de::value::StrDeserializer;
    use xai_grok_shell::sampling::SyntheticReason;

    match reason {
        "memory_reminder" => return (MessageKind::InjectedMemory, "Memory reminder".to_string()),
        "context_view" => return (MessageKind::InjectedSummary, "Context view".to_string()),
        _ => {}
    }
    let decoded = SyntheticReason::deserialize(StrDeserializer::<serde::de::value::Error>::new(reason));
    let Ok(decoded) = decoded else {
        return (MessageKind::InjectedOther, format!("Injected ({reason})"));
    };
    let (kind, label) = match decoded {
        SyntheticReason::Human => (MessageKind::InjectedConversation, "Your message"),
        SyntheticReason::Primary => (MessageKind::InjectedInstructions, "Core instructions"),
        SyntheticReason::CompactionMeta => (MessageKind::InjectedSummary, "Compaction summary"),
        SyntheticReason::SystemReminder => (MessageKind::InjectedContinuation, "System reminder"),
        SyntheticReason::LengthContinue => (MessageKind::InjectedContinuation, "Length continue"),
        SyntheticReason::ProjectInstructions => (MessageKind::InjectedInstructions, "Project rules"),
        SyntheticReason::AutoContinue => (MessageKind::InjectedContinuation, "Auto-continue"),
        SyntheticReason::AutoRecovery => (MessageKind::InjectedContinuation, "Auto-recovery"),
        SyntheticReason::Interjection => (MessageKind::InjectedConversation, "Interjection"),
        SyntheticReason::AgentMessage => (MessageKind::InjectedConversation, "Agent message"),
        SyntheticReason::ParentHumanMessage => (MessageKind::InjectedConversation, "Parent message"),
        SyntheticReason::TaskCompleted => (MessageKind::InjectedNotification, "Task completed"),
        SyntheticReason::SubagentCompleted => (MessageKind::InjectedNotification, "Subagent completed"),
        SyntheticReason::NotificationDrain => (MessageKind::InjectedNotification, "Notification drain"),
        SyntheticReason::GoalSummary => (MessageKind::InjectedSummary, "Goal continuation"),
        SyntheticReason::GoalClassifierNudge => (MessageKind::InjectedContinuation, "Goal check"),
        SyntheticReason::SchedulerFired => (MessageKind::InjectedNotification, "Scheduled task"),
        SyntheticReason::StopHookFeedback => (MessageKind::InjectedNotification, "Stop-hook feedback"),
        SyntheticReason::WorkingDirectorySwitch => (MessageKind::InjectedEnvironment, "Directory switch"),
        SyntheticReason::SessionPrefix => (MessageKind::InjectedEnvironment, "Session context"),
        SyntheticReason::DirectBash => (MessageKind::InjectedConversation, "Direct command"),
        SyntheticReason::GoalSetup => (MessageKind::InjectedContinuation, "Goal setup"),
        SyntheticReason::SkillCatalog => (MessageKind::InjectedInstructions, "Skill catalog"),
        SyntheticReason::McpCatalog => (MessageKind::InjectedInstructions, "MCP catalog"),
        SyntheticReason::Unknown => return (MessageKind::InjectedOther, format!("Injected ({reason})")),
    };
    (kind, label.to_string())
}
