use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarsOpen {
    None,
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenColumns {
    pub left: Rect,
    pub center: Rect,
    pub right: Rect,
}

pub fn screen_columns(area: Rect, open: SidebarsOpen) -> ScreenColumns {
    let (left_width, center_width, right_width) = match open {
        SidebarsOpen::None => (0, area.width, 0),
        SidebarsOpen::Left => (part(area.width, 35), part(area.width, 65), 0),
        SidebarsOpen::Right => (0, part(area.width, 65), part(area.width, 35)),
        SidebarsOpen::Both => (part(area.width, 25), part(area.width, 50), 0),
    };
    let (left_width, center_width, right_width) = if open == SidebarsOpen::Both {
        let left = part(area.width, 25).max(1).min(area.width);
        let right = part(area.width, 25)
            .max(1)
            .min(area.width.saturating_sub(left));
        (left, area.width.saturating_sub(left + right), right)
    } else {
        (left_width, center_width, right_width)
    };
    let (left_width, center_width, right_width) = match open {
        SidebarsOpen::Left if area.width >= 2 => {
            let left = left_width.max(1).min(area.width - 1);
            (left, area.width - left, 0)
        }
        SidebarsOpen::Right if area.width >= 2 => {
            let right = right_width.max(1).min(area.width - 1);
            (0, area.width - right, right)
        }
        SidebarsOpen::Left => (area.width, 0, 0),
        SidebarsOpen::Right => (0, 0, area.width),
        _ => (left_width, center_width, right_width),
    };
    let left = Rect::new(area.x, area.y, left_width, area.height);
    let center_x = area.x + left_width;
    let center = Rect::new(center_x, area.y, center_width, area.height);
    let right = Rect::new(center_x + center_width, area.y, right_width, area.height);
    ScreenColumns {
        left,
        center,
        right,
    }
}

pub(super) fn part(width: u16, percent: u16) -> u16 {
    ((u32::from(width) * u32::from(percent) + 50) / 100) as u16
}
