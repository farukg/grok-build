use std::ops::Range;

use super::super::common::{DEFAULT_COLS, DEFAULT_ROWS, PROMPT, PtyHarness};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Corners,
    Clipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SelectionFrame {
    body: Range<usize>,
    top: Edge,
    bottom: Edge,
}

struct Screen {
    cells: Vec<Vec<char>>,
    composer_top: usize,
}

impl Screen {
    fn read(harness: &PtyHarness) -> Option<Self> {
        let output = harness.screen_output();
        if output.size.cols != usize::from(DEFAULT_COLS)
            || output.size.rows != usize::from(DEFAULT_ROWS)
        {
            return None;
        }
        let mut cells = vec![vec![' '; output.size.cols]; output.size.rows];
        for (row, line) in output.lines.iter().enumerate() {
            let target = cells.get_mut(row)?;
            let mut col = 0;
            for ch in line.chars() {
                let width = ch.width().unwrap_or(0);
                if width == 0 {
                    continue;
                }
                *target.get_mut(col)? = ch;
                col += width;
            }
        }
        let composer_top = cells
            .iter()
            .rposition(|row| row[2] == '╭' && row[usize::from(DEFAULT_COLS) - 3] == '╮')?;
        let composer_bottom =
            cells
                .iter()
                .enumerate()
                .skip(composer_top + 2)
                .find_map(|(index, row)| {
                    (row[2] == '╰' && row[usize::from(DEFAULT_COLS) - 3] == '╯').then_some(index)
                })?;
        if !(composer_top + 1..composer_bottom)
            .all(|row| cells[row][2] == '│' && cells[row][usize::from(DEFAULT_COLS) - 3] == '│')
        {
            return None;
        }
        Some(Self {
            cells,
            composer_top,
        })
    }

    fn edges(&self, row: usize) -> Option<(char, char)> {
        let line = self.cells.get(row)?;
        Some((line[1], line[usize::from(DEFAULT_COLS) - 2]))
    }

    fn prompt(&self) -> Option<(u16, u16)> {
        // Default content starts after the accent and two padding columns; ten columns reserve timestamps.
        let end = usize::from(DEFAULT_COLS) - 14;
        (3..self.composer_top).find_map(|row| {
            let text: String = self.cells[row][5..end].iter().collect();
            (text.trim_end() == format!("❯ {PROMPT}")).then_some((row as u16, 7))
        })
    }

    fn frames(&self) -> Vec<SelectionFrame> {
        let mut frames = Vec::new();
        let mut row = 3;
        while row < self.composer_top {
            if !matches!(self.edges(row), Some(('│', '│') | ('┆', '┆'))) {
                row += 1;
                continue;
            }
            let start = row;
            while row < self.composer_top
                && matches!(self.edges(row), Some(('│', '│') | ('┆', '┆')))
            {
                row += 1;
            }
            let end = row;
            let pinned_header = self.cells[start - 3][5] == '❯'
                && (start - 2..start).all(|gap| {
                    self.cells[gap][2..usize::from(DEFAULT_COLS) - 2]
                        .iter()
                        .all(|ch| ch.is_whitespace())
                });
            let pushed_header = self.cells[start - 1][5] == '❯';
            let pushed_two_rows = self.cells[start - 2][5] == '❯'
                && self.cells[start - 1][2..usize::from(DEFAULT_COLS) - 2]
                    .iter()
                    .all(|ch| ch.is_whitespace());
            let top = if self.edges(start - 1) == Some(('┌', '┐')) {
                Edge::Corners
            } else if self.edges(start) == Some(('┆', '┆'))
                && (start == 3 || pinned_header || pushed_header || pushed_two_rows)
            {
                Edge::Clipped
            } else {
                continue;
            };
            let bottom = if end < self.composer_top && self.edges(end) == Some(('└', '┘')) {
                Edge::Corners
            } else if end + 1 == self.composer_top && self.edges(end - 1) == Some(('┆', '┆')) {
                Edge::Clipped
            } else {
                continue;
            };
            let valid_sides = (start..end).all(|body_row| {
                let clipped = (body_row == start && top == Edge::Clipped)
                    || (body_row + 1 == end && bottom == Edge::Clipped);
                let expected = if clipped {
                    ('┆', '┆')
                } else {
                    ('│', '│')
                };
                self.edges(body_row) == Some(expected)
            });
            let has_content = (start..end).any(|body_row| {
                self.cells[body_row][5..usize::from(DEFAULT_COLS) - 4]
                    .iter()
                    .any(|ch| !ch.is_whitespace())
            });
            if valid_sides && has_content {
                frames.push(SelectionFrame {
                    body: start..end,
                    top,
                    bottom,
                });
            }
        }
        frames
    }
}

pub(super) fn frames(harness: &PtyHarness) -> Vec<SelectionFrame> {
    Screen::read(harness).map_or_else(Vec::new, |screen| screen.frames())
}

pub(super) fn prompt_location(harness: &PtyHarness) -> Option<(u16, u16)> {
    Screen::read(harness)?.prompt()
}

pub(super) fn composer_location(harness: &PtyHarness) -> Option<(u16, u16)> {
    let screen = Screen::read(harness)?;
    Some(((screen.composer_top + 1) as u16, 5))
}

pub(super) fn composer_text(harness: &PtyHarness) -> Option<String> {
    let screen = Screen::read(harness)?;
    Some(
        screen.cells[screen.composer_top + 1][3..usize::from(DEFAULT_COLS) - 3]
            .iter()
            .collect(),
    )
}

pub(super) fn selected_prompt_frame(harness: &PtyHarness) -> Option<SelectionFrame> {
    let screen = Screen::read(harness)?;
    let (row, _) = screen.prompt()?;
    screen
        .frames()
        .into_iter()
        .find(|frame| frame.body.contains(&usize::from(row)))
}

pub(super) fn selected_prompt(harness: &PtyHarness) -> Option<(u16, u16)> {
    selected_prompt_frame(harness)?;
    prompt_location(harness)
}
