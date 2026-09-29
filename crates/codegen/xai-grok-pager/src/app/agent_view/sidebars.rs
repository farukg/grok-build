//! The two side columns around the session view: the F6 timeline on the left and the F7 context switches on the right.
//! Both use the shared sidebar component, so header and footer heights and the column split come from one place.

use super::AgentView;
use crate::views::prompt_widget::PromptWidget;
use crate::views::sidebar::{
    HostedSection, Sidebar, SidebarContent, SidebarEdge, SidebarHeights, SidebarLayout,
    SidebarRender, SidebarSection, SidebarState, SidebarsOpen,
};
use crate::views::timeline::MIN_TERMINAL_WIDTH;
use crate::views::timeline_panel::{TimelinePanelState, render_panel};
use crate::scrollback::state::ScrollbackState;
use crate::theme::Theme;
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

impl AgentView {
    /// Which side columns take space this frame. The timeline needs a terminal wide enough for its rail.
    pub(crate) fn sidebars_open(&self, area_width: u16) -> SidebarsOpen {
        let timeline = self.timeline_mode.panel().is_some() && area_width >= MIN_TERMINAL_WIDTH;
        let context = matches!(self.context_sidebar, super::context_sidebar::ContextSidebar::Open(_));
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
        let heights = self.sidebar_heights.heights_for(area.width);
        let body = SidebarLayout::compute(area, heights).body;
        self.sync_timeline_panel_frame(if area.width == 0 { Rect::default() } else { body });
        let Some(panel) = self.timeline_mode.panel() else {
            return;
        };
        if area.width == 0 {
            return;
        }
        let theme = Theme::current();
        let hosted = TimelineBody {
            panel,
            scrollback: &self.scrollback,
            height: body.height,
        };
        let sections = [SidebarSection::Hosted(&hosted)];
        let mut state = SidebarState::default();
        state.rebuild_layout(&sections);
        let header = [Line::from("Timeline")];
        let footer = [
            Line::from("j/k move · Enter jump · e fold"),
            Line::from("R rewind · d remove · Esc close"),
        ];
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
                theme: &theme,
            },
            buf,
        );
    }
}
