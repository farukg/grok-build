//! How captured text (file reads, command output) is classified and colored.
//!
//! [`ContentKind::classify`] decides once per content whether terminal control data can be present.
//! [`terminal_rows`] then styles terminal-kind content according to the [`TerminalColoring`] setting.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::Span;

use crate::appearance::TerminalColoring;
use crate::render::terminal_output::{looks_like_terminal_output, render_terminal_lines};
use crate::syntax::{Syntect, highlight_line};

/// Extensions that routinely hold captured terminal output, even though a grammar exists for some of them.
const CAPTURE_EXTENSIONS: [&str; 4] = ["log", "txt", "out", "ansi"];

/// What a piece of captured text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// The path names a language grammar, so the content is source text. It is never scanned for escapes.
    Source,
    /// A capture-type or unknown path whose content carries ESC bytes or bare carriage returns.
    Terminal,
    /// A capture-type or unknown path without terminal control data.
    Plain,
}

impl ContentKind {
    pub fn classify(syntect: &Syntect, path: &Path, content: &str) -> Self {
        let capture_type = match path.extension().and_then(|ext| ext.to_str()) {
            None => true,
            Some(ext) => {
                CAPTURE_EXTENSIONS
                    .iter()
                    .any(|capture| capture.eq_ignore_ascii_case(ext))
                    || syntect.find_syntax_by_file_path(path).is_none()
            }
        };
        if !capture_type {
            Self::Source
        } else if looks_like_terminal_output(content) {
            Self::Terminal
        } else {
            Self::Plain
        }
    }
}

/// One styled terminal row: its spans and the de-escaped text a terminal would show.
#[derive(Debug, Clone)]
pub struct StyledRow {
    pub spans: Vec<Span<'static>>,
    pub plain: String,
}

/// Styled rows of terminal-kind `content`.
/// `path` selects the grammar for the syntax colors (DSL when it names none).
pub fn terminal_rows(
    syntect: &Syntect,
    path: &Path,
    content: &str,
    coloring: TerminalColoring,
    base: Style,
) -> Vec<StyledRow> {
    let rendered = render_terminal_lines(content, base);
    if coloring == TerminalColoring::Terminal {
        return rendered
            .into_iter()
            .map(|row| StyledRow {
                spans: row.line.spans,
                plain: row.plain,
            })
            .collect();
    }
    let mut highlighter = syntect
        .highlight_lines_by_file_path(path)
        .or_else(|| syntect.highlight_lines_for_fence_info(""));
    rendered
        .into_iter()
        .map(|row| {
            let syntax = highlight_line(&row.plain, &mut highlighter, syntect, base);
            let spans = match coloring {
                TerminalColoring::Terminal => row.line.spans,
                TerminalColoring::Syntax => syntax,
                TerminalColoring::Both => overlay_unstyled(row.line.spans, syntax, base),
            };
            StyledRow {
                spans,
                plain: row.plain,
            }
        })
        .collect()
}

/// Styled rows of the whole `content` (one span list per row), for off-thread caching.
/// Terminal data runs through the emulator and the [`TerminalColoring`] setting; source and plain text use the path's grammar.
pub fn styled_rows(
    syntect: &Syntect,
    path: &Path,
    content: &str,
    coloring: TerminalColoring,
    base: Style,
) -> Vec<Vec<Span<'static>>> {
    if ContentKind::classify(syntect, path, content) == ContentKind::Terminal {
        return terminal_rows(syntect, path, content, coloring, base)
            .into_iter()
            .map(|row| row.spans)
            .collect();
    }
    let mut highlighter = syntect.highlight_lines_by_file_path(path);
    content
        .lines()
        .map(|text| highlight_line(text, &mut highlighter, syntect, base))
        .collect()
}

/// Terminal spans keep their escape styling; characters still in the `base` style take the syntax style at the same position.
fn overlay_unstyled(
    terminal: Vec<Span<'static>>,
    syntax: Vec<Span<'static>>,
    base: Style,
) -> Vec<Span<'static>> {
    let mut syntax_styles = syntax
        .iter()
        .flat_map(|span| span.content.chars().map(move |_| span.style));
    let mut out: Vec<Span<'static>> = Vec::new();
    for span in terminal {
        for ch in span.content.chars() {
            let from_syntax = syntax_styles.next().unwrap_or(base);
            let style = if span.style == base {
                from_syntax
            } else {
                span.style
            };
            match out.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push(ch),
                _ => out.push(Span::styled(ch.to_string(), style)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::get_syntect;

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn source_extension_is_never_scanned() {
        let syntect = get_syntect();
        let escaped = "let s = \"\x1b[31m\";";
        assert_eq!(
            ContentKind::classify(syntect, Path::new("main.rs"), escaped),
            ContentKind::Source
        );
    }

    #[test]
    fn capture_paths_split_on_terminal_data() {
        let syntect = get_syntect();
        let colored = "\x1b[0;31mfailed\x1b[0m\n";
        for path in ["call.log", "notes.txt", "capture", "x.unknownext"] {
            assert_eq!(
                ContentKind::classify(syntect, Path::new(path), colored),
                ContentKind::Terminal,
                "{path}"
            );
            assert_eq!(
                ContentKind::classify(syntect, Path::new(path), "plain\n"),
                ContentKind::Plain,
                "{path}"
            );
        }
    }

    #[test]
    fn terminal_coloring_keeps_only_escape_colors() {
        let syntect = get_syntect();
        let base = Style::default();
        let rows = terminal_rows(
            syntect,
            Path::new("a.rs"),
            "\x1b[31mlet\x1b[0m x = 1;",
            TerminalColoring::Terminal,
            base,
        );
        let [row] = rows.as_slice() else {
            panic!("one row expected: {}", rows.len());
        };
        assert_eq!(text(&row.spans), "let x = 1;");
        assert_eq!(row.spans.last().map(|span| span.style), Some(base));
    }

    #[test]
    fn syntax_coloring_drops_escapes_and_paints_the_grammar() {
        let syntect = get_syntect();
        let base = Style::default();
        let rows = terminal_rows(
            syntect,
            Path::new("a.rs"),
            "\x1b[31mlet\x1b[0m x = 1;",
            TerminalColoring::Syntax,
            base,
        );
        let [row] = rows.as_slice() else {
            panic!("one row expected: {}", rows.len());
        };
        assert_eq!(text(&row.spans), "let x = 1;");
        assert_eq!(row.plain, "let x = 1;");
        assert!(
            row.spans.iter().any(|span| span.style.fg.is_some()),
            "{row:?}"
        );
    }

    #[test]
    fn both_keeps_escape_colors_and_colors_the_rest_by_grammar() {
        let syntect = get_syntect();
        let base = Style::default();
        let content = "\x1b[31mlet\x1b[0m x = 1;";
        let terminal = terminal_rows(
            syntect,
            Path::new("a.rs"),
            content,
            TerminalColoring::Terminal,
            base,
        );
        let syntax = terminal_rows(
            syntect,
            Path::new("a.rs"),
            content,
            TerminalColoring::Syntax,
            base,
        );
        let both = terminal_rows(
            syntect,
            Path::new("a.rs"),
            content,
            TerminalColoring::Both,
            base,
        );
        let (Some(t), Some(s), Some(b)) = (terminal.first(), syntax.first(), both.first()) else {
            panic!("one row each");
        };
        assert_eq!(text(&b.spans), "let x = 1;");
        assert_eq!(
            b.spans.first().map(|span| span.style),
            t.spans.first().map(|span| span.style)
        );
        assert_eq!(
            b.spans.last().map(|span| span.style),
            s.spans.last().map(|span| span.style)
        );
    }

    #[test]
    fn styled_rows_cover_every_line_of_source_and_terminal_content() {
        let syntect = get_syntect();
        let base = Style::default();
        let source = styled_rows(
            syntect,
            Path::new("a.rs"),
            "fn a() {}\nfn b() {}\n",
            TerminalColoring::Terminal,
            base,
        );
        assert_eq!(source.len(), 2);
        let terminal = styled_rows(
            syntect,
            Path::new("call.log"),
            "\x1b[31mfailed\x1b[0m\nok\n",
            TerminalColoring::Terminal,
            base,
        );
        assert_eq!(terminal.len(), 2);
        assert_eq!(
            terminal.first().map(|row| text(row)),
            Some("failed".to_owned())
        );
    }

    #[test]
    fn dsl_colors_unknown_paths_in_syntax_mode() {
        let syntect = get_syntect();
        let rows = terminal_rows(
            syntect,
            Path::new("capture"),
            "\x1b[1mplain words\x1b[0m\n",
            TerminalColoring::Syntax,
            Style::default(),
        );
        assert_eq!(
            rows.first().map(|row| row.plain.as_str()),
            Some("plain words")
        );
    }
}
