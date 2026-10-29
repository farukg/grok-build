//! Source-byte coordinates for derived parser input.

use std::borrow::Cow;
use std::ops::Range;

/// Replaces each tab with a fixed number of spaces, not tab stops.
pub fn expand_tabs(text: &str, width: u8) -> Cow<'_, str> {
    if width == 0 || !text.contains('\t') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace('\t', &" ".repeat(usize::from(width))))
}

#[derive(Debug, Clone)]
struct Edit {
    input: Range<usize>,
    output: Range<usize>,
}

/// Equal-length runs need no entries; only byte-length changes are stored.
#[derive(Debug, Clone, Default)]
pub(crate) struct OffsetMap {
    input_len: usize,
    output_len: usize,
    edits: Vec<Edit>,
}

impl OffsetMap {
    pub(crate) fn append(&mut self, input_len: usize, output_len: usize) {
        if input_len != output_len {
            self.edits.push(Edit {
                input: self.input_len..self.input_len + input_len,
                output: self.output_len..self.output_len + output_len,
            });
        }
        self.input_len += input_len;
        self.output_len += output_len;
    }

    /// Interior bytes of a replacement belong to its original start boundary.
    pub(crate) fn original_offset(&self, offset: usize) -> usize {
        let preceding = self.edits.partition_point(|edit| edit.output.end <= offset);
        if let Some(edit) = self.edits.get(preceding)
            && offset >= edit.output.start
        {
            return edit.input.start;
        }
        match preceding.checked_sub(1).and_then(|i| self.edits.get(i)) {
            Some(edit) => edit.input.end + (offset - edit.output.end),
            None => offset,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LineOffsets {
    starts: Vec<usize>,
    len: usize,
}

impl Default for LineOffsets {
    fn default() -> Self {
        Self {
            starts: vec![0],
            len: 0,
        }
    }
}

impl LineOffsets {
    pub(crate) fn append(&mut self, text: &str) {
        let base = self.len;
        self.starts.extend(
            text.bytes()
                .enumerate()
                .filter_map(|(i, byte)| (byte == b'\n').then_some(base + i + 1)),
        );
        self.len += text.len();
    }

    pub(crate) fn line_at(&self, byte: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }

    pub(crate) fn offset(&self, line: usize) -> usize {
        self.starts.get(line).copied().unwrap_or(self.len)
    }

    pub(crate) fn line_count(&self) -> usize {
        self.starts.len()
    }
}

/// The parser sees expanded tabs; callers retain byte-exact source coordinates.
#[derive(Debug, Clone, Default)]
pub(crate) struct TabProjection {
    offsets: OffsetMap,
}

impl TabProjection {
    pub(crate) fn push<'a>(&mut self, chunk: &'a str, width: u8) -> Cow<'a, str> {
        let expanded = expand_tabs(chunk, width);
        if matches!(expanded, Cow::Borrowed(_)) {
            self.offsets.append(chunk.len(), chunk.len());
            return expanded;
        }
        for part in chunk.split_inclusive('\t') {
            let (literal, tab) = match part.strip_suffix('\t') {
                Some(literal) => (literal, "\t"),
                None => (part, ""),
            };
            self.offsets.append(literal.len(), literal.len());
            if !tab.is_empty() {
                self.offsets.append(tab.len(), usize::from(width));
            }
        }
        expanded
    }

    pub(crate) fn original_offset(&self, offset: usize) -> usize {
        self.offsets.original_offset(offset)
    }
}
