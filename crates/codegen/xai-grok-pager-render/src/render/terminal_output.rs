//! Bash/terminal tool output arrives as a raw PTY byte stream.
//! It can contain ANSI SGR (colors/styles), cursor movement, line erases, and carriage returns (progress bars rewriting a line).
//! ratatui paints text verbatim and does not interpret these, so without this module the scrollback shows literal escape codes like `[1m[36m`.
//!
//! [`render_terminal_lines`] feeds the stream through a minimal, line-oriented VTE emulator (built on the `vte` parser).
//! It produces styled [`Line`]s plus de-escaped plain text: what a terminal would actually display.
//! Unlike a screen/grid emulator it keeps an unbounded, fully-styled transcript that maps onto the pager's line model.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vte::{Params, Parser, Perform};

use crate::theme::color_support::quantize;

/// Bound sparse allocation from cursor jumps, not naturally printed transcript length.
const MAX_COLS: usize = 8_192;

/// A single rendered transcript line: styled spans plus de-escaped plain text.
pub struct RenderedLine {
    pub line: Line<'static>,
    pub plain: String,
}

/// Fresh emulator per call, so render and height-cache paths stay deterministic and idempotent.
pub fn render_terminal_lines(raw: &str, base: Style) -> Vec<RenderedLine> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut sink = TermSink::new(base);
    let mut parser = Parser::new();
    parser.advance(&mut sink, raw.as_bytes());
    sink.finish()
}

/// De-escaped, cursor-resolved plain text of a terminal stream, for clipboard/search.
/// Lines are joined with `\n`.
pub fn render_terminal_plain(raw: &str) -> String {
    render_terminal_lines(raw, Style::default())
        .into_iter()
        .map(|rl| rl.plain)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether `raw` carries terminal control data: an ESC byte (SGR, OSC, cursor movement) or a `\r` that is not part of a `\r\n` line ending (progress bars).
pub fn looks_like_terminal_output(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        b == 0x1b || (b == b'\r' && bytes.get(i + 1).is_some_and(|&next| next != b'\n'))
    })
}

#[derive(Clone)]
enum CellText {
    Scalar(char),
    Grapheme(String),
}

impl CellText {
    fn push_to(&self, out: &mut String) {
        match self {
            Self::Scalar(ch) => out.push(*ch),
            Self::Grapheme(text) => out.push_str(text),
        }
    }

    fn width(&self) -> usize {
        match self {
            Self::Scalar(ch) => ch.width().unwrap_or(0),
            Self::Grapheme(text) => text.width(),
        }
    }

    fn append(&mut self, ch: char) -> bool {
        if matches!(self, Self::Scalar(previous) if previous.is_ascii() && ch.is_ascii()) {
            return false;
        }
        match self {
            Self::Scalar(previous) => {
                let mut text = previous.to_string();
                let boundary = text.len();
                text.push(ch);
                if GraphemeCursor::new(boundary, text.len(), true)
                    .is_boundary(&text, 0) != Ok(false)
                {
                    return false;
                }
                *self = Self::Grapheme(text);
            }
            Self::Grapheme(text) => {
                let boundary = text.len();
                text.push(ch);
                if GraphemeCursor::new(boundary, text.len(), true)
                    .is_boundary(text, 0) != Ok(false)
                {
                    text.truncate(boundary);
                    return false;
                }
            }
        }
        true
    }

    fn extended(&self, next: &Self) -> Option<Self> {
        if matches!((self, next), (Self::Scalar(previous), Self::Scalar(ch)) if previous.is_ascii() && ch.is_ascii()) {
            return None;
        }
        let mut text = String::new();
        self.push_to(&mut text);
        next.push_to(&mut text);
        (text.graphemes(true).count() == 1).then_some(Self::Grapheme(text))
    }
}

#[derive(Clone)]
enum Cell {
    Blank,
    Glyph { text: CellText, style: Style },
    Continuation,
}

fn glyph_start(line: &[Cell], col: usize) -> Option<usize> {
    let cell = line.get(col)?;
    match cell {
        Cell::Blank | Cell::Glyph { .. } => Some(col),
        Cell::Continuation => line.get(..col)?.iter().rposition(|cell| {
            matches!(cell, Cell::Blank | Cell::Glyph { .. })
        }),
    }
}

fn clear_cells(line: &mut [Cell], cols: std::ops::Range<usize>) {
    if cols.is_empty() {
        return;
    }
    let start = glyph_start(line, cols.start).unwrap_or(line.len());
    let last = cols.end.min(line.len()).saturating_sub(1);
    let end = match glyph_start(line, last).and_then(|col| line.get(col).map(|cell| (col, cell))) {
        Some((col, Cell::Glyph { text, .. })) => col + text.width(),
        Some((col, Cell::Blank | Cell::Continuation)) => col + 1,
        None => line.len(),
    };
    if let Some(cells) = line.get_mut(start..end) {
        cells.fill(Cell::Blank);
    }
}

// Batch printable extensions across SGR changes before measuring their width.
enum PendingGlyph {
    Empty,
    Glyph { text: CellText, style: Style },
}

struct TermSink {
    pending: PendingGlyph,
    base: Style,
    cur: Style,
    rows: Vec<Vec<Cell>>,
    row: usize,
    col: usize,
}

impl TermSink {
    fn new(base: Style) -> Self {
        Self {
            pending: PendingGlyph::Empty,
            base,
            cur: base,
            rows: vec![Vec::new()],
            row: 0,
            col: 0,
        }
    }

    fn ensure_row(&mut self) {
        while self.rows.len() <= self.row {
            self.rows.push(Vec::new());
        }
    }

    fn put(&mut self, ch: char) {
        if let PendingGlyph::Glyph { text, .. } = &mut self.pending
            && text.append(ch)
        {
            return;
        }
        self.flush_glyph();
        self.pending = PendingGlyph::Glyph { text: CellText::Scalar(ch), style: self.cur };
    }

    fn flush_glyph(&mut self) {
        let PendingGlyph::Glyph { text: incoming, style: incoming_style } =
            std::mem::replace(&mut self.pending, PendingGlyph::Empty)
        else {
            return;
        };
        self.ensure_row();
        let Some(line) = self.rows.get_mut(self.row) else {
            return;
        };
        let previous = self.col.checked_sub(1).and_then(|col| glyph_start(line, col));
        if let Some(start) = previous
            && let Some(Cell::Glyph { text, style }) = line.get(start)
            && start + text.width() == self.col
            && let Some(extended) = text.extended(&incoming)
        {
            let style = *style;
            let end = start + extended.width();
            line.resize(line.len().max(end), Cell::Blank);
            clear_cells(line, start..end.max(self.col));
            if let Some(slot) = line.get_mut(start) {
                *slot = Cell::Glyph { text: extended, style };
            }
            if let Some(tail) = line.get_mut(start + 1..end) {
                tail.fill(Cell::Continuation);
            }
            self.col = end;
            return;
        }
        let width = incoming.width();
        if width == 0 {
            return;
        }
        let end = self.col + width;
        line.resize(line.len().max(end), Cell::Blank);
        clear_cells(line, self.col..end);
        if let Some(slot) = line.get_mut(self.col) {
            *slot = Cell::Glyph { text: incoming, style: incoming_style };
        }
        if let Some(tail) = line.get_mut(self.col + 1..end) {
            tail.fill(Cell::Continuation);
        }
        self.col = end;
    }

    fn newline(&mut self) {
        self.row += 1;
        self.col = 0;
        self.ensure_row();
    }

    fn erase_line(&mut self, mode: u16) {
        self.ensure_row();
        let Some(line) = self.rows.get_mut(self.row) else {
            return;
        };
        match mode {
            0 => {
                let end = line.len();
                clear_cells(line, self.col..end);
                line.truncate(self.col.min(end));
            }
            1 => clear_cells(line, 0..self.col.saturating_add(1)),
            2 => line.clear(),
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                self.rows.truncate(self.row + 1);
            }
            2 | 3 => {
                self.rows.clear();
                self.rows.push(Vec::new());
                self.row = 0;
                self.col = 0;
            }
            _ => {}
        }
    }

    fn apply_sgr(&mut self, params: &Params) {
        if params.is_empty() {
            self.cur = self.base;
            return;
        }
        let groups: Vec<&[u16]> = params.iter().collect();
        let mut i = 0;
        while i < groups.len() {
            let code = groups.get(i).and_then(|g| g.first()).copied().unwrap_or(0);
            match code {
                0 => self.cur = self.base,
                1 => self.cur = self.cur.add_modifier(Modifier::BOLD),
                2 => self.cur = self.cur.add_modifier(Modifier::DIM),
                3 => self.cur = self.cur.add_modifier(Modifier::ITALIC),
                4 => self.cur = self.cur.add_modifier(Modifier::UNDERLINED),
                7 => self.cur = self.cur.add_modifier(Modifier::REVERSED),
                22 => self.cur = self.cur.remove_modifier(Modifier::BOLD | Modifier::DIM),
                23 => self.cur = self.cur.remove_modifier(Modifier::ITALIC),
                24 => self.cur = self.cur.remove_modifier(Modifier::UNDERLINED),
                27 => self.cur = self.cur.remove_modifier(Modifier::REVERSED),
                30..=37 => self.cur.fg = Some(quantize(ansi16(code - 30))),
                39 => self.cur.fg = self.base.fg,
                40..=47 => self.cur.bg = Some(quantize(ansi16(code - 40))),
                49 => self.cur.bg = self.base.bg,
                90..=97 => self.cur.fg = Some(quantize(ansi16_bright(code - 90))),
                100..=107 => self.cur.bg = Some(quantize(ansi16_bright(code - 100))),
                38 => {
                    if let Some(c) = ext_color(&groups, &mut i) {
                        self.cur.fg = Some(quantize(c));
                    }
                }
                48 => {
                    if let Some(c) = ext_color(&groups, &mut i) {
                        self.cur.bg = Some(quantize(c));
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn finish(mut self) -> Vec<RenderedLine> {
        self.flush_glyph();
        // `str::lines()` ignores a single trailing newline; mirror that so a command ending in `\n` does not gain a spurious blank line
        if self.rows.last().is_some_and(|r| r.is_empty()) {
            self.rows.pop();
        }
        let base = self.base;
        self.rows
            .into_iter()
            .map(|cells| row_to_line(cells, base))
            .collect()
    }
}

impl Perform for TermSink {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        if matches!(byte, b'\n' | 0x0b | 0x0c | b'\r' | b'\t' | 0x08) {
            self.flush_glyph();
        }
        match byte {
            b'\n' | 0x0b | 0x0c => self.newline(),
            b'\r' => self.col = 0,
            b'\t' => self.col = (self.col / 8 + 1) * 8,
            0x08 => self.col = self.col.saturating_sub(1),
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if matches!(action, 'K' | 'J' | 'A' | 'B' | 'C' | 'D' | 'G') {
            self.flush_glyph();
        }
        match action {
            'm' => self.apply_sgr(params),
            'K' => self.erase_line(first_param(params, 0)),
            'J' => self.erase_display(first_param(params, 0)),
            'A' => self.row = self.row.saturating_sub(first_param(params, 1) as usize),
            'B' => {
                let n = first_param(params, 1) as usize;
                self.row = (self.row + n).min(self.rows.len().saturating_sub(1));
            }
            'C' => self.col = (self.col + first_param(params, 1) as usize).min(MAX_COLS),
            'D' => self.col = self.col.saturating_sub(first_param(params, 1) as usize),
            'G' => {
                self.col = (first_param(params, 1) as usize)
                    .saturating_sub(1)
                    .min(MAX_COLS)
            }
            _ => {}
        }
    }
}

/// First parameter value, substituting `default` for a missing or `0` value (CSI cursor ops treat `0` as `1`; erase ops pass `0` as the default).
fn first_param(params: &Params, default: u16) -> u16 {
    match params.iter().next().and_then(|p| p.first().copied()) {
        Some(0) | None => default,
        Some(v) => v,
    }
}

/// Map a 0-7 ANSI color index to a named ratatui color.
fn ansi16(n: u16) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

/// Map a 0-7 bright ANSI color index to a named ratatui color.
fn ansi16_bright(n: u16) -> Color {
    match n {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

/// Resolve an extended color (`38`/`48`) in either `;` (advancing `i` over the consumed groups) or `:` subparameter form.
/// Returns an un-quantized color.
fn ext_color(groups: &[&[u16]], i: &mut usize) -> Option<Color> {
    let g = groups.get(*i).copied()?;
    if g.len() >= 2 {
        let rest = g.get(1..)?;
        return parse_ext(rest);
    }
    match groups.get(*i + 1).and_then(|p| p.first().copied())? {
        5 => {
            let idx = groups.get(*i + 2).and_then(|p| p.first().copied())?;
            *i += 2;
            Some(Color::Indexed(idx as u8))
        }
        2 => {
            let r = groups.get(*i + 2).and_then(|p| p.first().copied())?;
            let g = groups.get(*i + 3).and_then(|p| p.first().copied())?;
            let b = groups.get(*i + 4).and_then(|p| p.first().copied())?;
            *i += 4;
            Some(Color::Rgb(r as u8, g as u8, b as u8))
        }
        _ => None,
    }
}

/// Parse the subparameter form of an extended color, e.g. `[5, n]` (256) or `[2, r, g, b]` (with an optional leading colorspace id).
/// Returns an un-quantized color.
fn parse_ext(sub: &[u16]) -> Option<Color> {
    match sub.first().copied()? {
        5 => sub.get(1).map(|n| Color::Indexed(*n as u8)),
        2 => {
            let vals = sub.get(1..)?;
            let (r, g, b) = match vals {
                [r, g, b] => (*r, *g, *b),
                [.., r, g, b] => (*r, *g, *b),
                _ => return None,
            };
            Some(Color::Rgb(r as u8, g as u8, b as u8))
        }
        _ => None,
    }
}

fn row_to_line(cells: Vec<Cell>, base: Style) -> RenderedLine {
    let end = cells.iter().rposition(|cell| match cell {
        Cell::Blank => false,
        Cell::Glyph { text: CellText::Scalar(' '), style } => *style != base,
        Cell::Glyph { .. } | Cell::Continuation => true,
    }).map_or(0, |col| col + 1);
    let mut plain = String::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut style = base;
    for cell in cells.into_iter().take(end) {
        let (text, next_style) = match cell {
            Cell::Blank => (CellText::Scalar(' '), base),
            Cell::Glyph { text, style } => (text, style),
            Cell::Continuation => continue,
        };
        if next_style != style {
            if !buf.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            style = next_style;
        }
        text.push_to(&mut plain);
        text.push_to(&mut buf);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, style));
    }
    RenderedLine {
        line: Line::from(spans),
        plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nth<T>(xs: &[T], i: usize) -> &T {
        let Some(x) = xs.get(i) else {
            panic!("expected item {i}, got {} items", xs.len());
        };
        x
    }

    fn plain(raw: &str) -> String {
        render_terminal_plain(raw)
    }

    fn lines(raw: &str) -> Vec<String> {
        render_terminal_lines(raw, Style::default())
            .into_iter()
            .map(|rl| rl.plain)
            .collect()
    }

    #[test]
    fn detects_escapes_and_bare_carriage_returns() {
        assert!(looks_like_terminal_output("\x1b[0;31mfailed\x1b[0m"));
        assert!(looks_like_terminal_output("10%\r50%\r100%\n"));
        assert!(looks_like_terminal_output("\x1b]8;;http://x\x07link"));
    }

    #[test]
    fn plain_text_and_crlf_are_not_terminal_output() {
        assert!(!looks_like_terminal_output(""));
        assert!(!looks_like_terminal_output("fn main() {}\n"));
        assert!(!looks_like_terminal_output("a\r\nb\r\n"));
        assert!(!looks_like_terminal_output("trailing cr\r"));
    }

    #[test]
    fn strips_sgr_to_plain_text() {
        assert_eq!(plain("\x1b[1m\x1b[36mbazel\x1b[0m"), "bazel");
    }

    #[test]
    fn carriage_return_overwrites_in_place() {
        assert_eq!(plain("aaaa\rbb"), "bbaa");
    }

    #[test]
    fn progress_bar_collapses_to_final_state() {
        assert_eq!(plain("10%\r50%\r100%\n"), "100%");
    }

    #[test]
    fn newline_splits_lines() {
        assert_eq!(lines("a\nb"), vec!["a", "b"]);
    }

    #[test]
    fn trailing_newline_adds_no_blank_line() {
        assert_eq!(lines("a\n"), vec!["a"]);
        assert_eq!(lines("a\n\n"), vec!["a", ""]);
    }

    #[test]
    fn cursor_up_then_carriage_return_and_erase() {
        // Write two lines, move up, overwrite the start, erase to end of line.
        assert_eq!(lines("line1\nline2\x1b[A\rXX\x1b[K"), vec!["XX", "line2"]);
    }

    #[test]
    fn tab_advances_to_next_stop() {
        assert_eq!(plain("a\tb"), "a       b");
    }

    #[test]
    fn malformed_escape_does_not_panic() {
        assert_eq!(plain("\x1b[38;5mhi"), "hi");
        assert!(plain("\x1b[99999999999m\x1b[mok").contains("ok"));
    }

    #[test]
    fn sgr_splits_into_styled_spans() {
        let rendered = render_terminal_lines("plain \x1b[31mred\x1b[0m", Style::default());
        assert_eq!(rendered.len(), 1);
        let spans = &nth(&rendered, 0).line.spans;
        assert_eq!(spans.len(), 2);
        let [a, b] = spans.as_slice() else {
            panic!("expected two spans: {spans:?}");
        };
        assert_eq!(a.content.as_ref(), "plain ");
        assert_eq!(b.content.as_ref(), "red");
        assert!(b.style.fg.is_some());
        assert_eq!(nth(&rendered, 0).plain, "plain red");
    }

    #[test]
    fn idempotent_line_count() {
        let raw = "a\nb\x1b[32mc\x1b[0m\rd\ne";
        let first = render_terminal_lines(raw, Style::default()).len();
        let second = render_terminal_lines(raw, Style::default()).len();
        assert_eq!(first, second);
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(render_terminal_lines("", Style::default()).is_empty());
    }

    #[test]
    fn ansi16_mapping() {
        assert_eq!(ansi16(1), Color::Red);
        assert_eq!(ansi16(6), Color::Cyan);
        assert_eq!(ansi16_bright(2), Color::LightGreen);
        assert_eq!(ansi16_bright(7), Color::White);
    }

    #[test]
    fn ext_color_subparam_forms() {
        assert_eq!(parse_ext(&[5, 42]), Some(Color::Indexed(42)));
        assert_eq!(parse_ext(&[2, 10, 20, 30]), Some(Color::Rgb(10, 20, 30)));
        assert_eq!(parse_ext(&[2, 0, 10, 20, 30]), Some(Color::Rgb(10, 20, 30)));
        assert_eq!(parse_ext(&[2, 1]), None);
    }

    // Pipe capture is plain text plus line endings, never a ConPTY screen stream.
    // CRLF and unsupported controls (DEC modes, OSC, cursor save) must be ignored without corrupting text.

    #[test]
    fn windows_crlf_line_endings() {
        assert_eq!(lines("a\r\nb\r\nc\r\n"), vec!["a", "b", "c"]);
    }

    #[test]
    fn ignores_dec_private_modes_and_osc() {
        let raw = "\x1b[?25l\x1b]0;window title\x07hello\x1b[?1049h world\x1b[?25h";
        assert_eq!(plain(raw), "hello world");
    }

    #[test]
    fn ignores_cursor_save_restore_and_absolute_positioning() {
        assert_eq!(plain("\x1b7\x1b[10;5Hkept\x1b8"), "kept");
    }

    #[test]
    fn forced_sgr_over_crlf_renders_styled() {
        let rendered =
            render_terminal_lines("\x1b[01;31mmatch\x1b[0m\r\nplain\r\n", Style::default());
        assert_eq!(rendered.len(), 2);
        assert_eq!(nth(&rendered, 0).plain, "match");
        assert_eq!(nth(&rendered, 1).plain, "plain");
        assert!(
            nth(&rendered, 0)
                .line
                .spans
                .iter()
                .any(|s| s.style.fg.is_some())
        );
    }

    // Real Windows shell output samples
    // Each pins a distinct parser behavior exercised by a sequence these shells actually emit on the wire

    // Git Bash / GNU `grep --color=always` wraps the match in a bold-red SGR with an interleaved EL (`\x1b[K`)
    // An empty-param reset (`\x1b[m`) closes it
    // The EL must not truncate already-printed text, and `\x1b[m` must restore the base style for the trailing run
    #[test]
    fn git_bash_gnu_grep_color() {
        let rendered =
            render_terminal_lines("\x1b[01;31m\x1b[Kfoo\x1b[m\x1b[Kbar\n", Style::default());
        assert_eq!(rendered.len(), 1);
        assert_eq!(nth(&rendered, 0).plain, "foobar");
        let spans = &nth(&rendered, 0).line.spans;
        assert_eq!(spans.len(), 2);
        let [a, b] = spans.as_slice() else {
            panic!("expected two spans: {spans:?}");
        };
        assert_eq!(a.content.as_ref(), "foo");
        assert!(a.style.fg.is_some());
        assert!(a.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(b.content.as_ref(), "bar");
        assert_eq!(b.style, Style::default());
    }

    // PowerShell 7 (`$PSStyle`) emits 24-bit color in the semicolon form `\x1b[38;2;R;G;Bm`
    // That drives the `ext_color` branch that consumes the following groups and advances `i`
    // If that advance were wrong the trailing `0` param would reset and drop the color
    #[test]
    fn powershell_truecolor_psstyle() {
        let rendered = render_terminal_lines(
            "\x1b[38;2;255;128;0mWARNING\x1b[0m: low disk\n",
            Style::default(),
        );
        assert_eq!(rendered.len(), 1);
        assert_eq!(nth(&rendered, 0).plain, "WARNING: low disk");
        let spans = &nth(&rendered, 0).line.spans;
        let [a, b, ..] = spans.as_slice() else {
            panic!("expected two spans: {spans:?}");
        };
        assert_eq!(a.content.as_ref(), "WARNING");
        assert!(a.style.fg.is_some());
        assert_eq!(b.style, Style::default());
    }

    // Progress output (cargo/npm/pip style under cmd/PowerShell) wipes the status line with EL mode 2 (`\x1b[2K`) regardless of cursor column
    // The line is then rewritten, so the transcript collapses to the final line
    #[test]
    fn progress_erase_entire_line_collapses() {
        assert_eq!(lines("loading 99%\x1b[2K\rdone\n"), vec!["done"]);
    }

    #[test]
    fn known_terminal_input_without_escapes_still_resolves_tabs_crlf_and_controls() {
        assert_eq!(lines("a\tb\r\nc\x00d\x07\r\n"), ["a       b", "cd"]);
        assert_eq!(lines("\t\r\n"), [""]);
        assert_eq!(lines("text  \r\n"), ["text"]);
    }

    #[test]
    fn cursor_overwrite_uses_display_cells_for_wide_glyphs() {
        assert_eq!(plain("界x\r!"), "! x");
        assert_eq!(plain("界x\x1b[2D!"), " !x");
        assert_eq!(plain("a界z\x1b[2D!"), "a !z");
        assert_eq!(plain("界\t!"), "界      !");
    }

    #[test]
    fn erases_clear_entire_intersecting_wide_glyphs() {
        assert_eq!(plain("界x\x1b[2D\x1b[K"), "");
        assert_eq!(plain("a界z\x1b[2D\x1b[1K"), "   z");
        assert_eq!(plain("界x\x1b[2D\x1b[J"), "");
    }

    #[test]
    fn combining_marks_do_not_advance_the_cursor() {
        assert_eq!(plain("e\u{301}x\x08!"), "e\u{301}!");
        assert_eq!(plain("e\u{301}x\r!"), "!x");
        assert_eq!(plain("e\u{301}x\x1b[2D\x1b[K"), "");
        assert_eq!(plain("界\u{301}x\x1b[2D!"), " !x");
    }

    #[test]
    fn grapheme_extension_preserves_style_and_cell_width() {
        let rows = render_terminal_lines("\x1b[31me\x1b[0m\u{301}x\x08!", Style::default());
        let row = nth(&rows, 0);
        assert_eq!(row.plain, "e\u{301}!");
        assert_eq!(row.line.spans.first().map(|span| span.content.as_ref()), Some("e\u{301}"));
        assert_eq!(row.line.spans.first().and_then(|span| span.style.fg), Some(quantize(Color::Red)));
        assert_eq!(plain("⚠\u{fe0f}x\x1b[2D!"), " !x");
    }

    #[test]
    fn long_combining_runs_preserve_text_style_and_cursor_cells() {
        let marks = "\u{301}".repeat(32_768);
        let glyph = format!("e{marks}");
        let expected = format!("{glyph}!");
        assert_eq!(plain(&format!("e{marks}x\x08!")), expected);
        let interleaved = "\x1b[0m\u{301}".repeat(32_768);
        let rows = render_terminal_lines(
            &format!("\x1b[31me{interleaved}x\x08!"),
            Style::default(),
        );
        let row = nth(&rows, 0);
        assert_eq!(row.plain, expected);
        assert_eq!(row.line.spans.first().map(|span| span.content.as_ref()), Some(glyph.as_str()));
        assert_eq!(row.line.spans.first().and_then(|span| span.style.fg), Some(quantize(Color::Red)));
        assert_eq!(plain(&format!("界{marks}x\x1b[2D!")), " !x");
        assert_eq!(plain(&format!("e{marks}x\r!")), "!x");
    }

    #[test]
    fn cursor_interrupted_combining_growth_preserves_text_style_and_display_cells() {
        let count = 4_096;
        let glyph = format!("e{}", "\u{301}".repeat(count));
        let interrupted = "\u{301}\x1b[1D\x1b[1C".repeat(count);
        assert_eq!(plain(&format!("e{interrupted}x\x08!")), format!("{glyph}!"));
        let mut raw = String::from("\x1b[31me");
        for _ in 0..count {
            raw.push_str("\x1b[0m\u{301}\x1b[1D\x1b[1C");
        }
        raw.push_str("x\x08!");
        let rows = render_terminal_lines(&raw, Style::default());
        assert_eq!(rows.len(), 1);
        let row = nth(&rows, 0);
        assert_eq!(row.plain, format!("{glyph}!"));
        assert_eq!(row.line.width(), 2);
        let [accented, overwritten] = row.line.spans.as_slice() else {
            panic!("expected accented glyph and overwritten cell");
        };
        assert_eq!(accented.content.as_ref(), glyph.as_str());
        assert_eq!(accented.style, Style::default().fg(quantize(Color::Red)));
        assert_eq!(overwritten.content.as_ref(), "!");
        assert_eq!(overwritten.style, Style::default());
    }

    #[test]
    fn interrupted_combining_growth_keeps_cursor_erase_and_wide_overwrite_semantics() {
        let count = 128;
        let glyph = format!("e{}", "\u{301}".repeat(count));
        for interruption in [
            "\x1b[1D\x1b[1C",
            "\x08\x1b[1C",
            "\r\x1b[2G",
            "\x1b[1A\x1b[1B",
            "\x1b[2G\x1b[K",
        ] {
            let mut raw = String::from("e");
            for _ in 0..count {
                raw.push('\u{301}');
                raw.push_str(interruption);
            }
            assert_eq!(plain(&format!("{raw}x\x08!")), format!("{glyph}!"));
            assert_eq!(plain(&format!("{raw}x\r!")), "!x");
            assert_eq!(plain(&format!("{raw}x\r\x1b[K")), "");
        }
        let mut wide = String::from("界");
        for _ in 0..count {
            wide.push_str("\u{301}\x1b[1D\x1b[1C");
        }
        let rows = render_terminal_lines(&format!("{wide}x\x1b[2D!"), Style::default());
        assert_eq!(rows.len(), 1);
        let row = nth(&rows, 0);
        assert_eq!(row.plain, " !x");
        assert_eq!(row.line.width(), 3);
        assert_eq!(plain(&format!("{wide}x\x1b[2D\x1b[K")), "");
    }

    #[test]
    fn printed_rows_and_columns_are_not_cursor_jump_limits() {
        let raw = format!("\x1b[32m{}TAIL", "row\n".repeat(70_000));
        let rows = lines(&raw);
        assert_eq!(rows.len(), 70_001);
        assert_eq!(rows.last().map(String::as_str), Some("TAIL"));
        let long = format!("\x1b[31m{}END", "x".repeat(10_000));
        assert_eq!(plain(&long), format!("{}END", "x".repeat(10_000)));
    }
}
