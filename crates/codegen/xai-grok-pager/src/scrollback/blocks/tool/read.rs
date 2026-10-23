use std::path::Path;
use std::sync::Arc;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};

use super::{LineRange, TOOL_HEADER_RANGE};
use crate::appearance::TerminalColoring;
use crate::prompt_images::ScrollbackImageRef;
use crate::render::content_kind::{ContentKind, terminal_rows};
use crate::render::wrapping::word_wrap_lines_with_joiners;
use crate::scrollback::block::BlockContent;
use crate::scrollback::types::{
    AccentStyle, BlockBackground, BlockContext, BlockLine, BlockOutput, DisplayMode, Selectable,
};
use crate::syntax::get_syntect;
use crate::theme::{Theme, ThemeKind};

const FIRST_LINES: usize = 5;
const LAST_LINES: usize = 3;

use crate::appearance::AppearanceConfig;
use xai_grok_tools::implementations::skills::types::skill_name_from_path;

/// What kind of non-text media this read produced.
#[derive(Debug, Clone)]
pub enum ReadMediaKind {
    /// Image file (PNG, JPEG, etc.)
    Image,
    /// PDF rendered as page images.
    Pdf { pages: usize },
}

/// Styled rows for a Read's whole content, baked off-thread under `theme` and `coloring`.
#[derive(Debug, Clone)]
pub struct CachedReadRows {
    pub rows: Arc<Vec<Vec<Span<'static>>>>,
    pub theme: ThemeKind,
    pub coloring: TerminalColoring,
}

/// Where a large Read stands in the off-thread highlight pipeline.
#[derive(Debug, Clone, Default)]
pub enum ReadHighlightPhase {
    /// No job yet; the preview styles only its visible rows.
    #[default]
    Unrequested,
    /// A worker job is outstanding.
    Pending { job_id: u64 },
    /// The whole content is styled; expanding reads from here.
    Cached(CachedReadRows),
    /// No worker result will come: the content is under the row threshold or over the worker caps. The preview styles its own rows.
    Skipped,
}

#[derive(Debug, Clone)]
pub struct ReadToolCallBlock {
    /// Path to the file being read.
    pub path: String,
    /// Line range if specified: [start, end] (1-based, inclusive).
    pub line_range: Option<LineRange>,
    /// Error message if the tool call failed (None means success).
    pub error: Option<String>,
    /// When the tool started running.
    pub started_at: Option<std::time::Instant>,
    /// Elapsed time in ms after completion.
    pub elapsed_ms: Option<i64>,
    /// Raw file content (unformatted). `None` for errors, images, PDFs.
    pub content: Option<String>,
    /// Total number of lines in the file (from `FileContent.total_lines`).
    pub total_lines: Option<usize>,
    /// Inline image reference (for image file reads).
    pub image_ref: Option<ScrollbackImageRef>,
    /// Non-text media kind (image, PDF).
    pub media_kind: Option<ReadMediaKind>,
    /// Whether this ordinary read targets a memory v2 scope.
    pub is_memory_activity: bool,
    /// Off-thread highlight state for large content.
    pub highlight: ReadHighlightPhase,
}

impl ReadToolCallBlock {
    /// Pre-completed blocks have no meaningful local timing, so `started_at` is `None`.
    /// Timing is only set for blocks that enter a running UI state (via `set_last_running(true)` in `ScrollbackState`).
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            line_range: None,
            error: None,
            started_at: None,
            elapsed_ms: None,
            content: None,
            total_lines: None,
            image_ref: None,
            media_kind: None,
            is_memory_activity: false,
            highlight: ReadHighlightPhase::Unrequested,
        }
    }

    pub fn with_memory_activity(mut self) -> Self {
        self.is_memory_activity = true;
        self
    }

    pub fn with_line_range(mut self, range: LineRange) -> Self {
        self.line_range = Some(range);
        self
    }

    pub fn with_content(mut self, content: String, total_lines: usize) -> Self {
        self.content = Some(content);
        self.total_lines = Some(total_lines);
        self
    }

    /// Set error (marks as failed).
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    /// Whether the block has non-empty text content to display.
    pub fn has_content(&self) -> bool {
        self.content.as_ref().is_some_and(|c| !c.is_empty())
    }

    /// Skill name when this read targets a skill definition (`SKILL.md`).
    /// All skill-read detection goes through here.
    pub fn skill_name(&self) -> Option<&str> {
        skill_name_from_path(&self.path)
    }

    /// Whether this read targets a skill definition rather than a plain file.
    pub fn is_skill_read(&self) -> bool {
        self.skill_name().is_some()
    }

    /// Set the error, computing elapsed time if not already set.
    pub fn set_error(&mut self, error: Option<String>) {
        if self.elapsed_ms.is_none()
            && let Some(start) = self.started_at
        {
            self.elapsed_ms = Some(start.elapsed().as_millis() as i64);
        }
        self.error = error;
    }

    /// Finalize elapsed time from `started_at`.
    ///
    /// Idempotent: no-op if `started_at` is `None` (pre-completed block) or if `elapsed_ms` is already set (already finalized).
    pub fn finish(&mut self) {
        if self.elapsed_ms.is_some() {
            return;
        }
        if let Some(start) = self.started_at {
            self.elapsed_ms = Some(start.elapsed().as_millis() as i64);
        }
    }

    pub fn elapsed_ms(&self) -> Option<i64> {
        match self.elapsed_ms {
            Some(ms) => Some(ms),
            None => self
                .started_at
                .map(|start| start.elapsed().as_millis() as i64),
        }
    }

    /// Render header line: `Read path (start-end)`.
    fn collapsed_line(
        &self,
        theme: &Theme,
        muted: bool,
        dim_details: bool,
        surface: crate::render::tool_paths::ToolPathSurface,
        cwd: Option<&std::path::Path>,
        width: Option<usize>,
    ) -> Line<'static> {
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
        let detail_style = if dim_details {
            theme.dim()
        } else {
            theme.muted()
        };

        // SKILL.md reads render as "Skill {skill_name}".
        if let Some(skill) = self.skill_name() {
            return Line::from(vec![
                Span::styled("Skill ", bold_style),
                Span::styled(skill.to_owned(), path_style),
            ]);
        }

        let prefix = "Read ";
        let range_suffix = self
            .line_range
            .map(|r| {
                if let Some(total) = self.total_lines
                    && total > r.end.saturating_sub(r.start) + 1
                {
                    format!(" ({} of {total})", r)
                } else {
                    format!(" ({})", r)
                }
            })
            .unwrap_or_default();
        // Extra suffix for errors or empty content
        let extra_suffix = if self.content.as_ref().is_some_and(|c| c.is_empty()) {
            " (empty)".to_string()
        } else if let Some(media) = &self.media_kind {
            match media {
                ReadMediaKind::Image => " (image)".to_string(),
                ReadMediaKind::Pdf { pages } => format!(" ({pages} pages)"),
            }
        } else {
            String::new()
        };
        let total_suffix_len = range_suffix.len() + extra_suffix.len();
        let path = crate::render::tool_paths::path_for_tool_surface(
            &self.path,
            surface,
            cwd,
            width,
            prefix.len() + total_suffix_len,
        );

        let mut spans = vec![
            Span::styled(prefix, bold_style),
            Span::styled(path, path_style),
        ];

        if !range_suffix.is_empty() {
            spans.push(Span::styled(range_suffix, detail_style));
        }

        if !extra_suffix.is_empty() {
            spans.push(Span::styled(extra_suffix, detail_style));
        }

        Line::from(spans)
    }

    /// Header line with only the path (or skill name) span selectable.
    fn header_block_line(&self, line: Line<'static>, cwd: Option<&std::path::Path>) -> BlockLine {
        let path_end = 2.min(line.spans.len()).max(1);
        let link_target = if self.skill_name().is_some() {
            None
        } else {
            crate::render::osc8::tool_path_file_target(&self.path, cwd)
        };
        BlockLine {
            selectable: Selectable::Spans(1..path_end),
            selection_range: Some(TOOL_HEADER_RANGE),
            content: line,
            link_target,
            ..Default::default()
        }
    }

    /// Render content lines with absolute line numbers in the gutter.
    ///
    /// Wraps all lines first, then applies truncation, matching ExecuteToolCallBlock.
    fn render_content_lines(
        &self,
        theme: &Theme,
        width: usize,
        truncate: Option<(usize, usize)>,
        coloring: TerminalColoring,
    ) -> Vec<BlockLine> {
        let Some(content) = &self.content else {
            return Vec::new();
        };

        let base_line = self.line_range.map_or(1, |r| r.start);

        // Use Theme::dim/primary so terminal-native (minimal) maps grays to SGR dim / default fg instead of raw gray_dim slots
        let gutter_style = theme.dim();
        let text_style = theme.primary();

        let syntect = get_syntect();
        let cached = match &self.highlight {
            ReadHighlightPhase::Cached(cached)
                if cached.theme == Theme::current_kind() && cached.coloring == coloring =>
            {
                Some(Arc::clone(&cached.rows))
            }
            _ => None,
        };
        let (total_rows, body) = if let Some(rows) = cached {
            let ranges = visible_row_ranges(rows.len(), truncate);
            let visible: Vec<(usize, Vec<Span<'static>>)> = rows
                .iter()
                .enumerate()
                .filter(|(i, _)| ranges.iter().any(|range| range.contains(i)))
                .map(|(i, spans)| (i, spans.clone()))
                .collect();
            (rows.len(), visible)
        } else if ContentKind::classify(syntect, Path::new(&self.path), content)
            == ContentKind::Terminal
        {
            let rendered = terminal_rows(
                syntect,
                Path::new(&self.path),
                content,
                coloring,
                text_style,
            );
            let total_rows = rendered.len();
            let ranges = visible_row_ranges(total_rows, truncate);
            let rows: Vec<(usize, Vec<Span<'static>>)> = rendered
                .into_iter()
                .enumerate()
                .filter(|(i, _)| ranges.iter().any(|range| range.contains(i)))
                .map(|(i, row)| (i, row.spans))
                .collect();
            (total_rows, rows)
        } else {
            let source: Vec<&str> = content.lines().collect();
            let ranges = if source.len() >= PARTIAL_HIGHLIGHT_MIN_ROWS {
                visible_row_ranges(source.len(), truncate)
            } else {
                vec![0..source.len()]
            };
            let mut rows = Vec::new();
            for range in &ranges {
                let mut highlighter = syntect.highlight_lines_by_file_path(Path::new(&self.path));
                let warmup_start = range.start.saturating_sub(TAIL_WARMUP_LINES);
                for (i, text) in source.iter().enumerate().take(range.end).skip(warmup_start) {
                    let spans =
                        crate::syntax::highlight_line(text, &mut highlighter, syntect, text_style);
                    if i >= range.start {
                        rows.push((i, spans));
                    }
                }
            }
            (source.len(), rows)
        };

        let gutter_width = digit_count(base_line + total_rows.saturating_sub(1));
        let content_width = width.saturating_sub(gutter_width + 2).max(20);

        let styled_rows: Vec<(usize, Line<'static>)> = body
            .into_iter()
            .map(|(i, body_spans)| {
                let gutter = format!("{:>w$}  ", base_line + i, w = gutter_width);
                let mut spans = vec![Span::styled(gutter, gutter_style)];
                spans.extend(body_spans);
                (i, Line::from(spans))
            })
            .collect();

        let mut lines = Vec::new();

        let (wrapped, joiners) = word_wrap_lines_with_joiners(
            styled_rows.into_iter().map(|(_, line)| line).collect(),
            content_width,
        );
        let total = wrapped.len();

        if let Some((first, last)) = truncate {
            let threshold = first + last;
            // A partially styled body holds only head and tail rows, so more source rows than the budget means a hidden middle even when no row wraps.
            if total > threshold || total_rows > threshold {
                for (wrapped_line, joiner) in wrapped.iter().zip(joiners.iter()).take(first) {
                    lines.push(
                        BlockLine::styled(wrapped_line.clone())
                            .with_panel_background(theme.bg_dark)
                            .with_joiner(joiner.clone()),
                    );
                }
                lines.push(
                    BlockLine::separator(Line::from(Span::styled("\u{2026}", theme.muted())))
                        .with_panel_background(theme.bg_dark),
                );
                for (wrapped_line, joiner) in wrapped.iter().zip(joiners.iter()).skip(total - last)
                {
                    lines.push(
                        BlockLine::styled(wrapped_line.clone())
                            .with_panel_background(theme.bg_dark)
                            .with_joiner(joiner.clone()),
                    );
                }
            } else {
                for (wrapped_line, joiner) in wrapped.into_iter().zip(joiners) {
                    lines.push(
                        BlockLine::styled(wrapped_line)
                            .with_panel_background(theme.bg_dark)
                            .with_joiner(joiner),
                    );
                }
            }
        } else {
            for (wrapped_line, joiner) in wrapped.into_iter().zip(joiners) {
                lines.push(
                    BlockLine::styled(wrapped_line)
                        .with_panel_background(theme.bg_dark)
                        .with_joiner(joiner),
                );
            }
        }

        lines
    }
}

/// Source rows highlighted ahead of a truncated tail so open strings and block comments reach it in the right state.
const TAIL_WARMUP_LINES: usize = 64;

/// Source files up to this many rows are highlighted whole, so the preview tail is exact. Larger files highlight only the preview rows.
pub const PARTIAL_HIGHLIGHT_MIN_ROWS: usize = 400;

/// Row ranges a read preview shows: the head and tail when truncation hides the middle, otherwise every row.
/// More than `first + last` rows always wrap to more than that many lines, so the hidden middle never reaches the screen.
fn visible_row_ranges(
    total_rows: usize,
    truncate: Option<(usize, usize)>,
) -> Vec<std::ops::Range<usize>> {
    match truncate {
        Some((first, last)) if total_rows > first + last => {
            vec![0..first, total_rows - last..total_rows]
        }
        _ => vec![0..total_rows],
    }
}

/// Count decimal digits in a number (for gutter width).
fn digit_count(n: usize) -> usize {
    n.checked_ilog10().map_or(1, |d| d as usize + 1)
}

impl BlockContent for ReadToolCallBlock {
    fn output(&self, ctx: &BlockContext) -> BlockOutput {
        let theme = Theme::current();
        let tool_cfg = &ctx.appearance.scrollback.blocks.tool;
        let muted_collapsed = ctx.mute_when_collapsed(tool_cfg.muted_collapsed);

        let cwd = ctx.cwd.as_deref();
        match ctx.mode {
            DisplayMode::Collapsed => BlockOutput {
                lines: vec![self.header_block_line(
                    self.collapsed_line(
                        &theme,
                        muted_collapsed,
                        tool_cfg.dim_details,
                        crate::render::tool_paths::ToolPathSurface::Collapsed,
                        cwd,
                        Some(ctx.content_width()),
                    ),
                    cwd,
                )],
            },
            DisplayMode::Truncated | DisplayMode::Expanded => {
                let truncate = if ctx.mode == DisplayMode::Truncated {
                    Some((FIRST_LINES, LAST_LINES))
                } else {
                    None
                };
                let header = self.collapsed_line(
                    &theme,
                    false,
                    tool_cfg.dim_details,
                    crate::render::tool_paths::ToolPathSurface::Expanded,
                    cwd,
                    None,
                );
                let mut lines: Vec<BlockLine> = vec![self.header_block_line(header, cwd)];
                if self.has_content() {
                    lines.push(BlockLine::separator(Line::from("")));
                    lines.extend(self.render_content_lines(
                        &theme,
                        ctx.width as usize,
                        truncate,
                        tool_cfg.terminal_coloring,
                    ));
                } else if let Some(err) = &self.error {
                    lines.push(BlockLine::separator(Line::from("")));
                    let error_style = Style::default().fg(theme.accent_error);
                    for line in err.lines() {
                        lines.push(BlockLine::separator(Line::from(Span::styled(
                            line.to_string(),
                            error_style,
                        ))));
                    }
                }
                BlockOutput { lines }
            }
        }
    }

    fn accent(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        None
    }

    fn bullet(&self, _ctx: &BlockContext) -> Option<AccentStyle> {
        if self.error.is_some() {
            let theme = Theme::current();
            Some(AccentStyle::static_color(theme.accent_error))
        } else {
            None
        }
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
        self.has_content()
    }

    fn default_display_mode(&self) -> DisplayMode {
        DisplayMode::Collapsed
    }

    fn finished_display_mode(&self) -> Option<DisplayMode> {
        Some(DisplayMode::Collapsed)
    }

    fn next_fold_mode(&self, current: DisplayMode, _is_running: bool) -> DisplayMode {
        match current {
            DisplayMode::Collapsed => DisplayMode::Truncated,
            DisplayMode::Truncated | DisplayMode::Expanded => DisplayMode::Collapsed,
        }
    }

    fn image_references(&self) -> &[ScrollbackImageRef] {
        match &self.image_ref {
            Some(r) => std::slice::from_ref(r),
            None => &[],
        }
    }

    fn preamble(&self, ctx: &BlockContext) -> Option<Text<'static>> {
        let theme = Theme::current();
        let dim_details = ctx.appearance.scrollback.blocks.tool.dim_details;
        Some(Text::from(self.collapsed_line(
            &theme,
            false,
            dim_details,
            crate::render::tool_paths::ToolPathSurface::Fullscreen,
            ctx.cwd.as_deref(),
            None,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrollback::types::{BlockContext, DisplayMode};

    fn first_line(
        output: &crate::scrollback::types::BlockOutput,
    ) -> &crate::scrollback::types::BlockLine {
        output
            .lines
            .first()
            .unwrap_or_else(|| panic!("expected a line, got {:?}", output.lines))
    }

    fn span_text(line: &crate::scrollback::types::BlockLine, i: usize) -> &str {
        line.content
            .spans
            .get(i)
            .unwrap_or_else(|| panic!("expected span {i}, got {:?}", line.content.spans))
            .content
            .as_ref()
    }

    fn make_ctx() -> BlockContext {
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

    #[test]
    fn terminal_capture_log_renders_without_literal_escapes() {
        let capture = "All file: dependencies are valid.\n\x1b[0;31mpre-commit: codegen drift detected\x1b[0m\n\x1b[0;33m  diagnose: sig codegen verify\x1b[0m\n";
        let block = ReadToolCallBlock::new("/home/u/.grok/sessions/x/terminal/call_1.log")
            .with_content(capture.into(), 3);
        let mut ctx = make_ctx();
        ctx.mode = DisplayMode::Expanded;
        let output = block.output(&ctx);
        let body: String = output
            .lines
            .iter()
            .flat_map(|l| l.content.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!body.contains('\u{1b}'), "{body:?}");
        assert!(!body.contains("[0;31m"), "{body:?}");
        assert!(
            body.contains("pre-commit: codegen drift detected"),
            "{body:?}"
        );
    }

    #[test]
    fn source_extension_skips_the_terminal_scan() {
        let syntect = get_syntect();
        let colored = "\x1b[31mred\x1b[0m";
        let kind =
            |path: &str, content: &str| ContentKind::classify(syntect, Path::new(path), content);
        assert_eq!(kind("call.log", colored), ContentKind::Terminal);
        assert_eq!(kind("capture", colored), ContentKind::Terminal);
        assert_eq!(
            kind("main.rs", "let s = \"\x1b[31m\";"),
            ContentKind::Source
        );
        assert_eq!(kind("call.log", "plain text\n"), ContentKind::Plain);
    }

    #[test]
    fn visible_rows_cover_head_and_tail_only_when_truncation_hides_the_middle() {
        assert_eq!(visible_row_ranges(100, Some((5, 3))), vec![0..5, 97..100]);
        assert_eq!(visible_row_ranges(8, Some((5, 3))), vec![0..8]);
        assert_eq!(visible_row_ranges(100, None), vec![0..100]);
    }

    #[test]
    fn large_truncated_read_shows_head_tail_and_hides_the_middle() {
        let content = (1..=2000)
            .map(|i| format!("let value_{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let block = ReadToolCallBlock::new("big.rs")
            .with_line_range(LineRange::new(1, 2000))
            .with_content(content, 2000);
        let ctx = BlockContext {
            mode: DisplayMode::Truncated,
            ..make_ctx()
        };
        let text: Vec<String> = block
            .output(&ctx)
            .lines
            .iter()
            .map(|l| l.content.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let joined = text.join("\n");
        assert!(joined.contains("value_1 = 1;"), "{joined}");
        assert!(joined.contains("value_5 = 5;"), "{joined}");
        assert!(!joined.contains("value_6 = 6;"), "{joined}");
        assert!(!joined.contains("value_1000 = 1000;"), "{joined}");
        assert!(joined.contains("value_1998 = 1998;"), "{joined}");
        assert!(joined.contains("value_2000 = 2000;"), "{joined}");
        assert!(!joined.contains("value_1997 = 1997;"), "{joined}");
        assert!(joined.contains('\u{2026}'), "{joined}");
        assert!(joined.contains("2000"), "gutter shows absolute numbers");
    }

    #[test]
    fn cached_rows_replace_a_fresh_highlight_pass() {
        let content = (1..=2000)
            .map(|i| format!("let value_{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut block = ReadToolCallBlock::new("big.rs")
            .with_line_range(LineRange::new(1, 2000))
            .with_content(content, 2000);
        let marker = Span::raw("CACHED-ROW");
        let rows: Vec<Vec<Span<'static>>> = (0..2000).map(|_| vec![marker.clone()]).collect();
        block.highlight = ReadHighlightPhase::Cached(CachedReadRows {
            rows: Arc::new(rows),
            theme: Theme::current_kind(),
            coloring: TerminalColoring::Terminal,
        });
        let ctx = BlockContext {
            mode: DisplayMode::Expanded,
            ..make_ctx()
        };
        let joined: String = block
            .output(&ctx)
            .lines
            .iter()
            .flat_map(|l| l.content.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(joined.contains("CACHED-ROW"), "cache must supply the rows");
        assert!(!joined.contains("value_1 = 1;"), "no fresh highlight pass");
    }

    #[test]
    fn cache_baked_under_another_coloring_is_ignored() {
        let content = (1..=2000)
            .map(|i| format!("let value_{i} = {i};"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut block = ReadToolCallBlock::new("big.rs")
            .with_line_range(LineRange::new(1, 2000))
            .with_content(content, 2000);
        let rows: Vec<Vec<Span<'static>>> = (0..2000).map(|_| vec![Span::raw("STALE")]).collect();
        block.highlight = ReadHighlightPhase::Cached(CachedReadRows {
            rows: Arc::new(rows),
            theme: Theme::current_kind(),
            coloring: TerminalColoring::Both,
        });
        let ctx = BlockContext {
            mode: DisplayMode::Truncated,
            ..make_ctx()
        };
        let joined: String = block
            .output(&ctx)
            .lines
            .iter()
            .flat_map(|l| l.content.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!joined.contains("STALE"), "{joined}");
        assert!(joined.contains("value_1 = 1;"), "{joined}");
    }

    #[test]
    fn cache_baked_under_another_theme_is_ignored() {
        let mut block =
            ReadToolCallBlock::new("main.rs").with_content("let fresh = 1;".to_owned(), 1);
        let other_theme = ThemeKind::ALL
            .iter()
            .copied()
            .find(|kind| *kind != Theme::current_kind())
            .expect("another theme");
        block.highlight = ReadHighlightPhase::Cached(CachedReadRows {
            rows: Arc::new(vec![vec![Span::raw("STALE-THEME")]]),
            theme: other_theme,
            coloring: TerminalColoring::Terminal,
        });
        let ctx = BlockContext {
            mode: DisplayMode::Expanded,
            ..make_ctx()
        };
        let joined: String = block
            .output(&ctx)
            .lines
            .iter()
            .flat_map(|line| line.content.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert!(joined.contains("let fresh = 1;"), "{joined}");
        assert!(!joined.contains("STALE-THEME"), "{joined}");
    }

    #[test]
    fn skill_md_renders_as_skill_label() {
        let block = ReadToolCallBlock::new("/home/user/.grok/skills/deploy/SKILL.md");
        let output = block.output(&make_ctx());
        let text: String = first_line(&output)
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "Skill deploy");
    }

    #[test]
    fn regular_file_renders_as_read() {
        let block = ReadToolCallBlock::new("src/main.rs");
        let output = block.output(&make_ctx());
        let text: String = first_line(&output)
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            text.starts_with("Read "),
            "expected 'Read ...' got '{text}'"
        );
        assert!(text.contains("main.rs"));
    }

    #[test]
    fn collapsed_header_shows_basename_only() {
        let block = ReadToolCallBlock::new("/Users/me/project/src/main.rs")
            .with_line_range(LineRange::new(1, 10));
        let output = block.output(&make_ctx());
        let text: String = first_line(&output)
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(text, "Read main.rs (1-10)");
    }

    #[test]
    fn expanded_shows_relative_when_under_cwd_preamble_absolute() {
        let abs = "/Users/me/project/src/main.rs";
        let cwd = std::path::PathBuf::from("/Users/me/project");
        let block = ReadToolCallBlock::new(abs).with_content("hello".into(), 1);
        let mut ctx = make_ctx();
        ctx.mode = DisplayMode::Expanded;
        ctx.cwd = Some(cwd.clone());
        let output = block.output(&ctx);
        let header: String = first_line(&output)
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(header, "Read src/main.rs");

        let preamble = block.preamble(&ctx).unwrap();
        let preamble_text: String = preamble
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(preamble_text, "Read /Users/me/project/src/main.rs");
    }

    #[test]
    fn content_preview_shading_is_marked_panel() {
        // The preview's bg_dark band is decoration, not meaningful shading
        // It must be flagged `background_is_panel` so minimal mode's flat rendering can drop it (EntryRenderer::flat_background)
        let block = ReadToolCallBlock::new("notes.txt").with_content("alpha\nbravo".to_string(), 2);
        let mut ctx = make_ctx();
        ctx.mode = DisplayMode::Expanded;
        let output = block.output(&ctx);
        let shaded: Vec<_> = output
            .lines
            .iter()
            .filter(|l| l.background.is_some())
            .collect();
        assert!(
            !shaded.is_empty(),
            "expanded read preview must shade its content lines"
        );
        assert!(
            shaded.iter().all(|l| l.background_is_panel),
            "read preview shading must be marked panel"
        );
    }

    #[test]
    fn header_only_path_is_selectable() {
        use crate::scrollback::types::{Selectable, derive_selection_text};

        let block = ReadToolCallBlock::new("/Users/me/project/src/main.rs")
            .with_line_range(LineRange::new(1, 10));
        let output = block.output(&make_ctx());
        let header = first_line(&output);

        assert!(
            matches!(&header.selectable, Selectable::Spans(r) if *r == (1..2)),
            "only path span should be selectable, got {:?}",
            header.selectable
        );
        // Collapsed: copy the painted basename, not a full-path override.
        assert_eq!(
            derive_selection_text(header),
            "main.rs",
            "copy/highlight should match the painted path span, not 'Read …'"
        );
        assert_eq!(span_text(header, 0), "Read ");
        assert!(header.selection_text.is_none());
    }

    #[test]
    fn expanded_header_selection_matches_relative_path() {
        use crate::scrollback::types::derive_selection_text;

        let block = ReadToolCallBlock::new("/Users/me/project/src/main.rs");
        let mut ctx = make_ctx();
        ctx.mode = DisplayMode::Expanded;
        ctx.cwd = Some(std::path::PathBuf::from("/Users/me/project"));
        let output = block.output(&ctx);
        let header = first_line(&output);
        assert_eq!(derive_selection_text(header), "src/main.rs");
    }

    #[test]
    fn header_link_target_is_absolute_file_for_collapsed_and_expanded() {
        let abs = "/Users/me/project/src/main.rs";
        let block = ReadToolCallBlock::new(abs);
        let mut ctx = make_ctx();
        ctx.cwd = Some(std::path::PathBuf::from("/Users/me/project"));

        let collapsed = block.output(&ctx);
        let collapsed_header = first_line(&collapsed);
        let target = collapsed_header.link_target.as_ref().expect("link target");
        assert_eq!(
            target,
            &crate::render::osc8::LinkTarget::File(
                std::sync::Arc::from(std::path::Path::new(abs),)
            )
        );
        assert_eq!(
            crate::render::osc8::resolve_link_target(target)
                .unwrap()
                .osc8_url
                .unwrap()
                .as_ref(),
            "file:///Users/me/project/src/main.rs"
        );
        assert_eq!(span_text(collapsed_header, 1), "main.rs");

        ctx.mode = DisplayMode::Expanded;
        let expanded = block.output(&ctx);
        let expanded_header = first_line(&expanded);
        assert_eq!(span_text(expanded_header, 1), "src/main.rs");
        assert_eq!(expanded_header.link_target.as_ref(), Some(target));
    }

    #[test]
    fn skill_header_selects_skill_name_only() {
        use crate::scrollback::types::{Selectable, derive_selection_text};

        let block = ReadToolCallBlock::new("/home/user/.grok/skills/deploy/SKILL.md");
        let output = block.output(&make_ctx());
        let header = first_line(&output);

        assert!(matches!(&header.selectable, Selectable::Spans(r) if *r == (1..2)));
        assert_eq!(derive_selection_text(header), "deploy");
        assert_eq!(span_text(header, 0), "Skill ");
    }

    #[test]
    fn is_skill_read_only_for_skill_paths() {
        assert!(ReadToolCallBlock::new("/x/skills/deploy/SKILL.md").is_skill_read());
        assert!(!ReadToolCallBlock::new("src/main.rs").is_skill_read());
        assert!(!ReadToolCallBlock::new("/x/skills/deploy/README.md").is_skill_read());
    }

    #[test]
    fn foldable_when_content_present() {
        let block = ReadToolCallBlock::new("f.rs").with_content("hello\nworld".into(), 2);
        assert!(block.is_foldable());
    }

    #[test]
    fn not_foldable_when_no_content() {
        let block = ReadToolCallBlock::new("f.rs");
        assert!(!block.is_foldable());
    }

    #[test]
    fn fold_cycles_collapsed_truncated() {
        let block = ReadToolCallBlock::new("f.rs").with_content("a\nb\nc".into(), 3);
        assert_eq!(
            block.next_fold_mode(DisplayMode::Collapsed, false),
            DisplayMode::Truncated
        );
        assert_eq!(
            block.next_fold_mode(DisplayMode::Truncated, false),
            DisplayMode::Collapsed
        );
    }

    #[test]
    fn truncated_output_includes_content_lines() {
        let content = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let block = ReadToolCallBlock::new("f.rs")
            .with_line_range(LineRange::new(1, 20))
            .with_content(content, 20);
        let ctx = BlockContext {
            mode: DisplayMode::Truncated,
            ..make_ctx()
        };
        let output = block.output(&ctx);
        // Header, blank separator, FIRST_LINES, ellipsis, LAST_LINES
        assert!(
            output.lines.len() > 1,
            "truncated should have content lines"
        );
        let all_text: String = output
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
            .join("\n");
        assert!(all_text.contains("line 1"), "should contain first line");
        assert!(all_text.contains("line 20"), "should contain last line");
        assert!(all_text.contains("\u{2026}"), "should contain ellipsis");
    }

    #[test]
    fn absolute_line_numbers_with_offset() {
        let block = ReadToolCallBlock::new("f.rs")
            .with_line_range(LineRange::new(50, 52))
            .with_content("fn foo() {}\nfn bar() {}\nfn baz() {}".into(), 100);
        let ctx = BlockContext {
            mode: DisplayMode::Truncated,
            ..make_ctx()
        };
        let output = block.output(&ctx);
        let all_text: String = output
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
            .join("\n");
        assert!(all_text.contains("50"), "should show line 50");
        assert!(all_text.contains("52"), "should show line 52");
    }
}
