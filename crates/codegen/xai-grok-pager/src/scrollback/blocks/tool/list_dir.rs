use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{
    AccentStyle, BlockBackground, BlockContext, BlockLine, BlockOutput, DisplayMode, Selectable,
};
use crate::theme::Theme;

use super::TOOL_HEADER_RANGE;
use crate::appearance::AppearanceConfig;

#[derive(Debug, Clone)]
pub struct ListDirToolCallBlock {
    pub path: String,
    pub output: String,
    pub error: Option<String>,
    pub(crate) execution_clock: crate::acp::tool_clock::ToolExecutionClock,
    /// Whether this ordinary list targets a memory v2 scope.
    pub is_memory_activity: bool,
}

impl ListDirToolCallBlock {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            output: String::new(),
            error: None,
            execution_clock: Default::default(),
            is_memory_activity: false,
        }
    }

    pub fn with_memory_activity(mut self) -> Self {
        self.is_memory_activity = true;
        self
    }

    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = output.into();
        self
    }

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    pub fn finish(&mut self) {
        self.execution_clock.close();
    }

    pub fn elapsed_ms(&self) -> Option<i64> {
        self.execution_clock.elapsed_ms()
    }

    pub fn set_output(&mut self, output: impl Into<String>) {
        self.output = output.into();
    }

    fn collapsed_line(&self, theme: &Theme, muted: bool, width: Option<usize>) -> Line<'static> {
        let text_style = if muted {
            theme.muted()
        } else {
            theme.primary()
        };
        let bold_style = text_style.add_modifier(Modifier::BOLD);
        let path_style = if muted {
            theme.muted()
        } else {
            theme.fg(theme.path)
        };

        let prefix = String::new();
        let entry_count = self.output.lines().filter(|l| !l.trim().is_empty()).count();
        let suffix = if self.error.is_none() && entry_count > 0 {
            let s = if entry_count == 1 { "y" } else { "ies" };
            format!(" ({entry_count} entr{s})")
        } else {
            String::new()
        };
        let prefix_width = unicode_width::UnicodeWidthStr::width(prefix.as_str());
        let suffix_fits = width.is_none_or(|w| prefix_width + suffix.len() < w);
        let effective_suffix = if suffix_fits { suffix.as_str() } else { "" };

        let path_budget = width
            .map(|w| {
                w.saturating_sub(prefix_width)
                    .saturating_sub(effective_suffix.len())
            })
            .unwrap_or(usize::MAX);
        let path = crate::render::tool_paths::shorten_path(&self.path, path_budget);

        let mut spans = vec![
            Span::styled(prefix, bold_style),
            Span::styled(path, path_style),
        ];
        if !effective_suffix.is_empty() {
            spans.push(Span::styled(effective_suffix.to_string(), theme.muted()));
        }
        Line::from(spans)
    }

    /// Header line with only the path span selectable (exclude "List " prefix).
    fn header_block_line(&self, line: Line<'static>) -> BlockLine {
        let path_end = 2.min(line.spans.len()).max(1);
        BlockLine {
            selectable: Selectable::Spans(1..path_end),
            selection_range: Some(TOOL_HEADER_RANGE),
            selection_text: Some(self.path.clone()),
            content: line,
            ..Default::default()
        }
    }
}

impl BlockContent for ListDirToolCallBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let muted_collapsed =
            ctx.mute_when_collapsed(ctx.appearance.scrollback.blocks.tool.muted_collapsed);
        let terminal_bg = ctx.appearance.scrollback.blocks.list_dir.terminal_bg;

        match ctx.mode {
            DisplayMode::Collapsed => BlockOutput {
                lines: vec![self.header_block_line(self.collapsed_line(
                    &theme,
                    muted_collapsed,
                    Some(ctx.content_width()),
                ))],
            },
            DisplayMode::Truncated | DisplayMode::Expanded => {
                let mut lines: Vec<BlockLine> =
                    vec![self.header_block_line(self.collapsed_line(&theme, false, None))];

                if !self.output.is_empty() {
                    lines.push(BlockLine::separator(Line::from("")));

                    for row in crate::render::content_kind::terminal_rows(
                        crate::syntax::get_syntect(),
                        std::path::Path::new(""),
                        &self.output,
                        ctx.appearance.scrollback.blocks.tool.terminal_coloring,
                        theme.primary(),
                    ) {
                        // Indent output by 2 spaces
                        let mut spans = vec![Span::styled("  ".to_string(), theme.primary())];
                        spans.extend(row.spans);
                        let mut block_line: BlockLine = Line::from(spans).into();
                        if terminal_bg {
                            block_line = block_line.with_panel_background(theme.bg_dark);
                        }
                        lines.push(block_line);
                    }
                }

                BlockOutput { lines }
            }
        }
    }

    fn accent(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        None // ListDir blocks never have an accent line
    }

    fn has_vpad_for(&self, _appearance: &AppearanceConfig) -> bool {
        false
    }

    fn background(&self, _ctx: &BlockContext) -> BlockBackground {
        BlockBackground::None
    }

    fn has_raw_mode(&self) -> bool {
        false
    }

    fn is_foldable(&self) -> bool {
        // Not foldable if failed
        if self.error.is_some() {
            return false;
        }
        !self.output.is_empty()
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Collapsed
    }

    fn next_fold_mode(&self, current: DisplayMode, _is_running: bool) -> DisplayMode {
        match current {
            DisplayMode::Collapsed => DisplayMode::Expanded,
            _ => DisplayMode::Collapsed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrollback::types::BlockContext;

    fn ctx() -> BlockContext {
        BlockContext {
            width: 80,
            mode: DisplayMode::Collapsed,
            is_running: false,
            raw: false,
            max_lines: None,
            appearance: Default::default(),
            is_selected: false,
            cwd: None,
        }
    }

    fn header_text(block: &ListDirToolCallBlock) -> String {
        let output = block.output(&ctx());
        let Some(line) = output.lines.first() else {
            panic!("expected a header line");
        };
        line.content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn collapsed_header_shows_entry_count() {
        let block = ListDirToolCallBlock::new("src").with_output("a.rs\nb.rs\nsub/\n");
        assert_eq!(header_text(&block), "src (3 entries)".to_owned());

        let single = ListDirToolCallBlock::new("src").with_output("lonely.rs\n");
        assert_eq!(header_text(&single), "src (1 entry)".to_owned());
    }

    #[test]
    fn collapsed_header_omits_count_when_empty_or_failed() {
        let empty = ListDirToolCallBlock::new("src");
        assert_eq!(header_text(&empty), "src".to_owned());

        let failed = ListDirToolCallBlock::new("gone").with_error("no such directory");
        assert_eq!(header_text(&failed), "gone".to_owned());
    }
}
