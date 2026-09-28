mod layout;

#[cfg(test)]
mod tests;

pub use layout::{ScreenColumns, SidebarsOpen, screen_columns};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::render::scrollbar::render_scrollbar_styled;
use crate::theme::Theme;
use crate::views::prompt_widget::{PromptStyle, PromptWidget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarHeights {
    pub header: u16,
    pub footer: u16,
}

impl SidebarHeights {
    pub fn from_shared_sources(empty_composer: &PromptWidget, column_width: u16) -> Self {
        Self {
            header: crate::views::agent::SESSION_HEADER_ROW_HEIGHT,
            footer: empty_composer.desired_height(
                column_width,
                &PromptStyle::default(),
                false,
                u16::MAX,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarEdge {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionIdx(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowIdx(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarLine {
    Hosted(SectionIdx),
    SectionTitle(SectionIdx),
    Row(SectionIdx, RowIdx),
}

pub trait HostedSection {
    fn desired_height(&self) -> u16;
    fn render(&self, buf: &mut Buffer, visible_area: Rect, row_offset: u16, theme: &Theme);
}

pub enum SidebarSection<'a> {
    Hosted(&'a dyn HostedSection),
    Rows {
        title: Line<'a>,
        rows: &'a [SidebarRow<'a>],
    },
}

#[derive(Debug, Clone)]
pub struct SidebarRow<'a> {
    pub left: Line<'a>,
    pub right: Option<Line<'a>>,
    pub detail: &'a [Line<'a>],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionLayout {
    Hosted {
        index: SectionIdx,
        start: usize,
        height: usize,
    },
    Rows {
        index: SectionIdx,
        title: usize,
        rows_start: usize,
        rows: usize,
    },
}

#[derive(Debug, Default)]
pub struct SidebarState {
    pub scroll: usize,
    pub cursor: Option<SidebarLine>,
    layout: Vec<SectionLayout>,
    total_height: usize,
}

impl SidebarState {
    pub fn rebuild_layout(&mut self, sections: &[SidebarSection<'_>]) {
        self.layout.clear();
        let mut y = 0usize;
        for (index, section) in sections.iter().enumerate() {
            let index = SectionIdx(index);
            match section {
                SidebarSection::Hosted(widget) => {
                    let height = widget.desired_height() as usize;
                    self.layout.push(SectionLayout::Hosted {
                        index,
                        start: y,
                        height,
                    });
                    y = y.saturating_add(height);
                }
                SidebarSection::Rows { rows, .. } => {
                    let title = y;
                    y = y.saturating_add(1);
                    let rows_start = y;
                    self.layout.push(SectionLayout::Rows {
                        index,
                        title,
                        rows_start,
                        rows: rows.len(),
                    });
                    y = y.saturating_add(rows.len());
                }
            }
        }
        self.total_height = y;
        self.scroll = self.scroll.min(y.saturating_sub(1));
        if let Some(cursor) = self.cursor {
            if !self.contains_line(cursor) {
                self.cursor = None;
            }
        }
    }

    pub fn move_cursor(&mut self, delta: isize, viewport_height: usize) {
        let Some(current) = self.cursor.and_then(|line| self.line_position(line)) else {
            self.cursor = self.line_at(0);
            return;
        };
        let next = current
            .saturating_add_signed(delta)
            .min(self.total_height.saturating_sub(1));
        self.cursor = self.line_at(next);
        if next < self.scroll {
            self.scroll = next;
        } else if next >= self.scroll.saturating_add(viewport_height) {
            self.scroll = next.saturating_add(1).saturating_sub(viewport_height);
        }
    }

    pub fn scroll_rows(&mut self, delta: isize, viewport: usize) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.total_height.saturating_sub(viewport));
    }

    fn line_position(&self, line: SidebarLine) -> Option<usize> {
        self.layout
            .iter()
            .find_map(|section| match (*section, line) {
                (SectionLayout::Hosted { index, start, .. }, SidebarLine::Hosted(target))
                    if index == target =>
                {
                    Some(start)
                }
                (SectionLayout::Rows { index, title, .. }, SidebarLine::SectionTitle(target))
                    if index == target =>
                {
                    Some(title)
                }
                (
                    SectionLayout::Rows {
                        index,
                        rows_start,
                        rows,
                        ..
                    },
                    SidebarLine::Row(target, row),
                ) if index == target && row.0 < rows => Some(rows_start + row.0),
                _ => None,
            })
    }

    fn line_at(&self, position: usize) -> Option<SidebarLine> {
        self.layout.iter().find_map(|section| match *section {
            SectionLayout::Hosted {
                index,
                start,
                height,
            } if (start..start.saturating_add(height)).contains(&position) => {
                Some(SidebarLine::Hosted(index))
            }
            SectionLayout::Rows { index, title, .. } if position == title => {
                Some(SidebarLine::SectionTitle(index))
            }
            SectionLayout::Rows {
                index,
                rows_start,
                rows,
                ..
            } if (rows_start..rows_start.saturating_add(rows)).contains(&position) => {
                Some(SidebarLine::Row(index, RowIdx(position - rows_start)))
            }
            _ => None,
        })
    }

    fn contains_line(&self, line: SidebarLine) -> bool {
        self.line_position(line).is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarHit {
    Header,
    Line(SidebarLine),
    Footer,
}

pub struct SidebarContent<'a> {
    pub header: &'a [Line<'a>],
    pub sections: &'a [SidebarSection<'a>],
    pub footer: &'a [Line<'a>],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarLayout {
    pub outer: Rect,
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub scrollbar: Rect,
}

impl SidebarLayout {
    pub fn compute(area: Rect, heights: SidebarHeights) -> Self {
        let outer = Block::default().borders(Borders::ALL).inner(area);
        let header_height = heights.header.min(outer.height);
        let footer_height = heights
            .footer
            .min(outer.height.saturating_sub(header_height));
        let header = Rect::new(outer.x, outer.y, outer.width, header_height);
        let footer_y = outer.y + outer.height.saturating_sub(footer_height);
        let footer = Rect::new(outer.x, footer_y, outer.width, footer_height);
        let body_y = header.bottom();
        let body = Rect::new(
            outer.x,
            body_y,
            outer.width,
            footer.y.saturating_sub(body_y),
        );
        let scrollbar = Rect::new(
            body.right().saturating_sub(1),
            body.y,
            u16::from(body.width > 0),
            body.height,
        );
        Self {
            outer: area,
            header,
            body,
            footer,
            scrollbar,
        }
    }

    pub fn hit(&self, col: u16, row: u16, state: &SidebarState) -> Option<SidebarHit> {
        if !self.outer.contains((col, row).into()) {
            return None;
        }
        if self.header.contains((col, row).into()) {
            return Some(SidebarHit::Header);
        }
        if self.footer.contains((col, row).into()) {
            return Some(SidebarHit::Footer);
        }
        let position = state
            .scroll
            .checked_add(row.checked_sub(self.body.y)? as usize)?;
        state.line_at(position).map(SidebarHit::Line)
    }

    pub fn line_rect(&self, line: SidebarLine, state: &SidebarState) -> Option<Rect> {
        let position = state.line_position(line)?;
        let row = position.checked_sub(state.scroll)?;
        (row < self.body.height as usize)
            .then(|| Rect::new(self.body.x, self.body.y + row as u16, self.body.width, 1))
    }
}

pub struct Sidebar;

pub struct SidebarRender<'a> {
    pub area: Rect,
    pub content: SidebarContent<'a>,
    pub heights: SidebarHeights,
    pub edge: SidebarEdge,
    pub state: &'a SidebarState,
    pub hovered: Option<SidebarLine>,
    pub theme: &'a Theme,
}

impl Sidebar {
    pub fn render(params: SidebarRender<'_>, buf: &mut Buffer) -> SidebarLayout {
        let SidebarRender {
            area,
            content,
            heights,
            edge,
            state,
            hovered,
            theme,
        } = params;
        let layout = SidebarLayout::compute(area, heights);
        let bg = theme.bg_base;
        buf.set_style(area, Style::default().fg(theme.text_primary).bg(bg));
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(
                Style::default()
                    .fg(if state.cursor.is_some() {
                        theme.selection_border
                    } else {
                        theme.gray_dim
                    })
                    .bg(bg),
            )
            .style(Style::default().bg(bg))
            .render(area, buf);
        render_lines(buf, layout.header, content.header);
        render_lines(buf, layout.footer, content.footer);
        let visible_start = state.scroll;
        let visible_end = state.scroll.saturating_add(layout.body.height as usize);
        for section in &state.layout {
            match *section {
                SectionLayout::Hosted {
                    index,
                    start,
                    height,
                } => {
                    let visible_start = start.max(state.scroll);
                    let visible_end = start
                        .saturating_add(height)
                        .min(state.scroll.saturating_add(layout.body.height as usize));
                    if visible_start < visible_end {
                        if let Some(SidebarSection::Hosted(widget)) = content.sections.get(index.0)
                        {
                            let visible_area = Rect::new(
                                layout.body.x,
                                layout.body.y + (visible_start - state.scroll) as u16,
                                layout.body.width,
                                (visible_end - visible_start) as u16,
                            );
                            widget.render(buf, visible_area, (visible_start - start) as u16, theme);
                        }
                    }
                }
                SectionLayout::Rows {
                    index,
                    title,
                    rows_start,
                    rows,
                } => {
                    let section_end = rows_start.saturating_add(rows);
                    if (visible_start..visible_end).contains(&title)
                        || (rows_start < visible_end && section_end > visible_start)
                    {
                        let section_title = SidebarLine::SectionTitle(index);
                        let row_params = RowLineRender {
                            body: layout.body,
                            state,
                            hovered,
                            content: &content,
                            theme,
                        };
                        render_row_line(buf, &row_params, section_title, title);
                        let first_row = visible_start.saturating_sub(rows_start).min(rows);
                        let last_row = visible_end.saturating_sub(rows_start).min(rows);
                        for row in first_row..last_row {
                            render_row_line(
                                buf,
                                &row_params,
                                SidebarLine::Row(index, RowIdx(row)),
                                rows_start + row,
                            );
                        }
                    }
                }
            }
        }
        render_scrollbar_styled(
            buf,
            Some(layout.scrollbar),
            state.total_height.min(u16::MAX as usize) as u16,
            layout.body.height,
            state.scroll.min(u16::MAX as usize) as u16,
            Style::default().bg(theme.scrollbar_bg),
            Style::default()
                .fg(theme.scrollbar_fg)
                .bg(theme.scrollbar_bg),
        );
        if let Some((line, detail)) = detail_for(content.sections, hovered.or(state.cursor)) {
            render_detail(buf, layout.body, edge, line, state, detail, theme);
        }
        layout
    }
}

fn render_lines(buf: &mut Buffer, area: Rect, lines: &[Line<'_>]) {
    for (index, line) in lines.iter().take(area.height as usize).enumerate() {
        buf.set_line(area.x, area.y + index as u16, line, area.width);
    }
}

struct RowLineRender<'a> {
    body: Rect,
    state: &'a SidebarState,
    hovered: Option<SidebarLine>,
    content: &'a SidebarContent<'a>,
    theme: &'a Theme,
}

fn render_row_line(
    buf: &mut Buffer,
    params: &RowLineRender<'_>,
    line_id: SidebarLine,
    position: usize,
) {
    let Some(y_offset) = position.checked_sub(params.state.scroll) else {
        return;
    };
    if y_offset >= params.body.height as usize {
        return;
    }
    let section_index = match line_id {
        SidebarLine::SectionTitle(section) | SidebarLine::Row(section, _) => section.0,
        SidebarLine::Hosted(_) => return,
    };
    let Some(SidebarSection::Rows { title, rows }) = params.content.sections.get(section_index)
    else {
        return;
    };
    let y = params.body.y + y_offset as u16;
    let style = if params.state.cursor == Some(line_id) {
        params.theme.selection_overlay()
    } else if params.hovered == Some(line_id) {
        params.theme.hover_overlay()
    } else {
        Style::default()
            .fg(params.theme.text_primary)
            .bg(params.theme.bg_base)
    };
    match line_id {
        SidebarLine::SectionTitle(_) => {
            buf.set_line(params.body.x, y, title, params.body.width.saturating_sub(1));
        }
        SidebarLine::Row(_, row) => {
            if let Some(item) = rows.get(row.0) {
                render_row(buf, params.body, y, item, style, params.theme);
            }
        }
        SidebarLine::Hosted(_) => {}
    }
}

fn render_row(
    buf: &mut Buffer,
    body: Rect,
    y: u16,
    row: &SidebarRow<'_>,
    style: Style,
    theme: &Theme,
) {
    let width = body.width.saturating_sub(1);
    let right_width = row
        .right
        .as_ref()
        .map_or(0, |line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .min(width);
    let left_width = width.saturating_sub(right_width);
    buf.set_line(body.x, y, &row.left, left_width);
    if let Some(right) = &row.right {
        let right_width = u16::try_from(right.width()).unwrap_or(u16::MAX).min(width);
        let x = body.x.saturating_add(width.saturating_sub(right_width));
        let right = right
            .clone()
            .style(Style::default().fg(theme.gray).bg(theme.bg_base));
        buf.set_line(x, y, &right, right_width);
    }
    buf.set_style(Rect::new(body.x, y, width, 1), style);
}

fn detail_for<'a>(
    sections: &'a [SidebarSection<'a>],
    line: Option<SidebarLine>,
) -> Option<(SidebarLine, &'a [Line<'a>])> {
    let line = line?;
    match line {
        SidebarLine::Hosted(_) | SidebarLine::SectionTitle(_) => None,
        SidebarLine::Row(section, row) => {
            let SidebarSection::Rows { rows, .. } = sections.get(section.0)? else {
                return None;
            };
            Some((line, &rows.get(row.0)?.detail))
        }
    }
}

fn render_detail(
    buf: &mut Buffer,
    bounds: Rect,
    edge: SidebarEdge,
    line: SidebarLine,
    state: &SidebarState,
    detail: &[Line<'_>],
    theme: &Theme,
) {
    if bounds.width < 8 || bounds.height < 3 || detail.is_empty() {
        return;
    }
    let content_width = detail
        .iter()
        .map(Line::width)
        .max()
        .map_or(0, |width| u16::try_from(width).unwrap_or(u16::MAX));
    let width = content_width
        .saturating_add(4)
        .min(bounds.width.saturating_sub(1));
    let height = (detail.len() as u16).saturating_add(2).min(bounds.height);
    let anchor = state
        .line_position(line)
        .and_then(|position| position.checked_sub(state.scroll))
        .map(|row| bounds.y + row as u16)
        .unwrap_or(bounds.y);
    let x = match edge {
        SidebarEdge::Left => bounds.right().saturating_sub(width),
        SidebarEdge::Right => bounds.x,
    };
    let y = anchor
        .saturating_sub(height / 2)
        .max(bounds.y)
        .min(bounds.bottom().saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    Widget::render(Clear, area, buf);
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.gray).bg(theme.bg_base))
        .render(area, buf);
    for (index, line) in detail
        .iter()
        .take(height.saturating_sub(2) as usize)
        .enumerate()
    {
        buf.set_line(
            area.x + 1,
            area.y + 1 + index as u16,
            line,
            area.width.saturating_sub(2),
        );
    }
}
