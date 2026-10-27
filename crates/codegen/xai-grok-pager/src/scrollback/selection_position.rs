use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePosition {
    Text { byte: usize, character: char },
    Empty { byte: usize },
    Overlay { row: usize, column: u16 },
}

impl SourcePosition {
    pub(crate) fn at(text: &str, start: usize, column: u16) -> Self {
        let column = crate::render::bidi::visual_col_to_logical_col(text, usize::from(column));
        let mut cells = 0usize;
        for (byte, grapheme) in text.grapheme_indices(true) {
            let width = grapheme.width();
            if cells + width > column {
                if let Some(character) = grapheme.chars().next() {
                    return Self::Text { byte: start + byte, character };
                }
            }
            cells += width;
        }
        Self::Empty { byte: start + text.len() }
    }

    pub(crate) fn byte(self) -> Option<usize> {
        match self {
            Self::Text { byte, .. } | Self::Empty { byte } => Some(byte),
            Self::Overlay { .. } => None,
        }
    }

    pub(crate) fn column(self, text: &str, start: usize) -> Option<u16> {
        match self {
            Self::Text { byte, character } => {
                let offset = byte.checked_sub(start)?;
                let tail = text.get(offset..)?;
                if tail.chars().next()? != character { return None; }
                let logical = text.get(..offset)?.width();
                let visual = crate::render::bidi::logical_cols_to_visual(text, logical, logical + 1)
                    .into_iter().next()?.0;
                u16::try_from(visual).ok()
            }
            Self::Empty { byte } => (byte == start + text.len())
                .then(|| u16::try_from(text.width()).ok()).flatten(),
            Self::Overlay { column, .. } => Some(column),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionPositions {
    pub anchor: SourcePosition,
    pub head: SourcePosition,
}
