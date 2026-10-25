//! The two side columns around the session view: the F6 timeline on the left and the F7 context switches on the right.
//! Both use the shared sidebar component, so header and footer heights and the column split come from one place.

use super::AgentView;
use crate::scrollback::state::ScrollbackState;
use crate::theme::Theme;
use crate::views::prompt_widget::PromptWidget;
use crate::views::sidebar::{
    HostedSection, Sidebar, SidebarContent, SidebarEdge, SidebarHeights, SidebarLayout,
    SidebarRender, SidebarSection, SidebarState, SidebarsOpen,
};
use crate::views::timeline::MIN_TERMINAL_WIDTH;
use crate::views::timeline_panel::footer_memo::FooterRowsKey;
use crate::views::timeline_panel::hint::{footer_lines, footer_rows, status_line};
use crate::views::timeline_panel::{PanelFocus, TimelinePanelState, render_panel};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;

/// Header and footer heights of the sidebars, measured once per column width.
#[derive(Debug, Default)]
pub(crate) struct SidebarHeightsCache(Option<(u16, SidebarHeights)>);

impl SidebarHeightsCache {
    pub(crate) fn heights_for(&mut self, width: u16) -> SidebarHeights {
        match self.0 {
            Some((cached, heights)) if cached == width => heights,
            _ => {
                let heights = SidebarHeights::from_shared_sources(&PromptWidget::new(), width);
                self.0 = Some((width, heights));
                heights
            }
        }
    }
}

/// The expanded timeline, hosted in the left column's body.
struct TimelineBody<'a> {
    panel: &'a TimelinePanelState,
    scrollback: &'a ScrollbackState,
    height: u16,
}

impl HostedSection for TimelineBody<'_> {
    fn desired_height(&self) -> u16 {
        self.height
    }

    fn render(&self, buf: &mut Buffer, _visible_area: Rect, _row_offset: u16, theme: &Theme) {
        render_panel(buf, self.panel, self.scrollback, theme);
    }
}

/// A dock ready to paint into the top rows of the F7 body.
pub(crate) struct PendingDock {
    pub(crate) data: crate::views::dock::DockData,
    pub(crate) rect: Rect,
}

pub(super) struct DockSlot<'a> {
    pub(super) data: Option<&'a std::cell::RefCell<crate::views::dock::DockData>>,
    pub(super) height: u16,
    pub(super) pointer: (u16, u16),
}

impl HostedSection for DockSlot<'_> {
    fn desired_height(&self) -> u16 {
        self.height
    }

    fn render(&self, buf: &mut Buffer, visible_area: Rect, row_offset: u16, theme: &Theme) {
        let Some(data) = self.data else { return; };
        let mut data = data.borrow_mut();
        data.hovered = visible_area.contains(self.pointer.into()).then(|| {
            crate::views::dock::DockLayout::with_cap(&data.counts(), self.height as usize)
                .item_at(self.pointer.1.saturating_sub(visible_area.y).saturating_add(row_offset))
        }).flatten();
        data.stop_hovered = crate::views::dock::hovered_stop_button_rect_with_offset(
            visible_area, row_offset, self.height, &data,
        ).is_some_and(|hit| hit.rect.contains(self.pointer.into()));
        if row_offset == 0 && visible_area.height == self.height {
            crate::views::dock::render(buf, visible_area, theme, &data);
            return;
        }
        let full = Rect::new(0, 0, visible_area.width, self.height);
        let mut rendered = Buffer::empty(full);
        crate::views::dock::render(&mut rendered, full, theme, &data);
        for y in 0..visible_area.height {
            for x in 0..visible_area.width {
                if let (Some(source), Some(target)) = (
                    rendered.cell((x, y.saturating_add(row_offset))),
                    buf.cell_mut((visible_area.x + x, visible_area.y + y)),
                ) {
                    *target = source.clone();
                }
            }
        }
    }
}

impl AgentView {
    /// Which side columns take space this frame. The timeline needs a terminal wide enough for its rail.
    pub(crate) fn sidebars_open(&self, area_width: u16) -> SidebarsOpen {
        let timeline = self.timeline_mode.panel().is_some() && area_width >= MIN_TERMINAL_WIDTH;
        let context = matches!(
            self.context_sidebar,
            super::context_sidebar::ContextSidebar::Open(_)
        );
        match (timeline, context) {
            (false, false) => SidebarsOpen::None,
            (true, false) => SidebarsOpen::Left,
            (false, true) => SidebarsOpen::Right,
            (true, true) => SidebarsOpen::Both,
        }
    }

    /// Draw the expanded timeline into the left column; runs after the session view so its layout is prepared.
    pub(super) fn draw_timeline_sidebar(&mut self, area: Rect, buf: &mut Buffer) {
        if self.timeline_mode.panel().is_none() {
            return;
        }
        let theme = Theme::current();
        let base = self.sidebar_heights.heights_for(area.width);
        let status = self.panel_persistence.saving_status();
        let rows_key = FooterRowsKey {
            column_width: area.width,
            column_height: area.height,
            status,
        };
        let footer_height = match self.timeline_mode.panel_mut() {
            Some(panel) => panel.footer_rows_for(rows_key, || {
                footer_rows(base, area.width, area.height, &theme, status)
            }),
            None => base.footer,
        };
        let heights = SidebarHeights {
            footer: footer_height,
            ..base
        };
        let body = SidebarLayout::compute(area, heights).body;
        self.sync_timeline_panel_frame(if area.width == 0 {
            Rect::default()
        } else {
            body
        });
        let context = self.timeline_footer_context();
        let Some(panel) = self.timeline_mode.panel() else {
            return;
        };
        if area.width == 0 {
            return;
        }
        let hosted = TimelineBody {
            panel,
            scrollback: &self.scrollback,
            height: body.height,
        };
        let sections = [SidebarSection::Hosted(&hosted)];
        let mut state = SidebarState::default();
        state.rebuild_layout(&sections);
        let header = [Line::from("Timeline")];
        let mut footer = footer_lines(area.width, &theme, &context);
        footer.extend(status_line(area.width, &theme, status));
        Sidebar::render(
            SidebarRender {
                area,
                content: SidebarContent {
                    header: &header,
                    sections: &sections,
                    footer: &footer,
                },
                heights,
                edge: SidebarEdge::Left,
                state: &state,
                hovered: None,
                focused: panel.focus == PanelFocus::Panel,
                theme: &theme,
            },
            buf,
        );
    }
}
