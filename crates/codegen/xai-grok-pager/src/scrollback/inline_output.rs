use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use super::super::types::{
    BlockLine, BlockOutput, RenderedBlockOutput, Selectable, SelectionBoundary,
};

#[derive(Debug, Default)]
pub(crate) struct RevisionSignal {
    revision: Cell<u64>,
    dependents: RefCell<Vec<Weak<RevisionSignal>>>,
    dirty_sinks: RefCell<Vec<(super::EntryId, Weak<RefCell<HashSet<super::EntryId>>>)>>,
}

impl RevisionSignal {
    pub(crate) fn revision(&self) -> u64 { self.revision.get() }

    pub(crate) fn subscribe(&self, dependent: &Rc<Self>) {
        let mut dependents = self.dependents.borrow_mut();
        if !dependents.iter().any(|weak| weak.ptr_eq(&Rc::downgrade(dependent))) {
            dependents.push(Rc::downgrade(dependent));
        }
    }

    pub(crate) fn subscribe_entry(&self, id: super::EntryId, sink: &Rc<RefCell<HashSet<super::EntryId>>>) {
        let mut sinks = self.dirty_sinks.borrow_mut();
        let weak = Rc::downgrade(sink);
        if !sinks.iter().any(|(entry, existing)| *entry == id && existing.ptr_eq(&weak)) {
            sinks.push((id, weak));
        }
    }

    fn mark_dirty(&self) {
        self.dirty_sinks.borrow_mut().retain(|(id, weak)| {
            if let Some(sink) = weak.upgrade() {
                sink.borrow_mut().insert(*id);
                true
            } else { false }
        });
    }

    pub(crate) fn bump(&self) {
        if self.dependents.borrow().is_empty() {
            self.revision.set(self.revision.get().wrapping_add(1));
            self.mark_dirty();
            return;
        }
        self.bump_seen(&mut HashSet::new());
    }

    fn bump_seen(&self, seen: &mut HashSet<*const Self>) {
        if !seen.insert(self as *const Self) { return; }
        self.revision.set(self.revision.get().wrapping_add(1));
        self.mark_dirty();
        let mut dependents = self.dependents.borrow_mut();
        dependents.retain(|weak| weak.strong_count() > 0);
        let live: Vec<_> = dependents.iter().filter_map(Weak::upgrade).collect();
        drop(dependents);
        for dependent in live { dependent.bump_seen(seen); }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputPhase { Running, Final }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskSource { Missing, Present(OutputPhase) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceIdentity {
    Task { owner: usize, binding: u32, id: String, revision: u64, state: TaskSource },
    Child { owner: usize, binding: u32, id: String, ancestry: Vec<usize>, revision: u64, content: u64 },
    MissingChild { parent: usize, binding: u32, id: String, revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceKey {
    pub identity: SourceIdentity,
    pub theme: crate::theme::ThemeKind,
    pub thinking_visible: bool,
}

impl SourceKey {
    pub(crate) fn same_child_owner(&self, other: &Self) -> bool {
        match &self.identity {
            SourceIdentity::Child { owner, binding, id, ancestry, .. } => match &other.identity {
                SourceIdentity::Child { owner: next_owner, binding: next_binding, id: next_id,
                    ancestry: next_ancestry, .. } => owner == next_owner && binding == next_binding
                    && id == next_id && ancestry == next_ancestry,
                SourceIdentity::Task { .. } | SourceIdentity::MissingChild { .. } => false,
            },
            SourceIdentity::Task { .. } | SourceIdentity::MissingChild { .. } => false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SourceBody {
    Content(Rc<OutputTree>),
    Available,
    Empty,
    Waiting,
    Loading,
    Unavailable,
    Cycle,
}

impl SourceBody {
    pub(crate) fn is_foldable(&self) -> bool {
        match self {
            Self::Content(tree) => tree.len() > 0,
            Self::Available => true,
            Self::Empty | Self::Waiting | Self::Loading | Self::Unavailable | Self::Cycle => false,
        }
    }

    pub(crate) fn output(&self) -> Rc<OutputTree> {
        match self {
            Self::Content(tree) => Rc::clone(tree),
            Self::Empty => OutputTree::notice("No output"),
            Self::Waiting => OutputTree::notice("No output yet"),
            Self::Loading | Self::Available => OutputTree::notice("Loading output"),
            Self::Unavailable => OutputTree::notice("Output unavailable"),
            Self::Cycle => OutputTree::notice("Cyclic child link"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InlineProjection {
    pub key: SourceKey,
    pub width: u16,
    pub expanded: bool,
    pub body: SourceBody,
}

#[derive(Debug, Clone)]
struct Part {
    end: usize,
    ranges_before: usize,
    output: Rc<OutputTree>,
}

#[derive(Debug, Clone)]
enum TreeData {
    Leaf(Rc<RenderedBlockOutput>),
    Branch(Vec<Part>),
}

#[derive(Debug, Clone)]
pub(crate) struct OutputTree {
    data: TreeData,
    len: usize,
    ranges: usize,
}

pub(crate) struct OutputLine<'a> {
    pub line: &'a BlockLine,
    pub range_id: Option<u16>,
    pub boundary: Option<&'a Arc<SelectionBoundary>>,
}

impl OutputTree {
    pub(crate) fn leaf(output: Rc<RenderedBlockOutput>) -> Rc<Self> {
        let ranges = output.output.lines.iter().filter_map(|line| line.selection_range)
            .max().map_or(0, |id| usize::from(id) + 1);
        Rc::new(Self { len: output.output.len(), ranges, data: TreeData::Leaf(output) })
    }

    pub(crate) fn branch(outputs: impl IntoIterator<Item = Rc<Self>>) -> Rc<Self> {
        let mut len = 0usize;
        let mut ranges = 0usize;
        let parts = outputs.into_iter().map(|output| {
            len = len.saturating_add(output.len);
            let part = Part { end: len, ranges_before: ranges, output };
            ranges = ranges.saturating_add(part.output.ranges);
            part
        }).collect();
        Rc::new(Self { data: TreeData::Branch(parts), len, ranges })
    }

    pub(crate) fn notice(text: &str) -> Rc<Self> {
        Self::leaf(Rc::new(RenderedBlockOutput::from(BlockOutput {
            lines: vec![BlockLine::separator(ratatui::text::Line::from(text.to_owned()))],
        })))
    }

    pub(crate) fn len(&self) -> usize { self.len }
    pub(crate) fn is_empty(&self) -> bool { self.len == 0 }

    pub(crate) fn line(&self, index: usize) -> Option<OutputLine<'_>> {
        match &self.data {
            TreeData::Leaf(output) => {
                let line = output.output.lines.get(index)?;
                Some(OutputLine { line, range_id: line.selection_range,
                    boundary: output.boundaries.get(index) })
            }
            TreeData::Branch(parts) => {
                let pos = parts.partition_point(|part| part.end <= index);
                let part = parts.get(pos)?;
                let start = if pos == 0 { 0 } else { parts.get(pos - 1)?.end };
                let mut line = part.output.line(index.checked_sub(start)?)?;
                line.range_id = line.range_id.and_then(|id| {
                    u16::try_from(part.ranges_before.saturating_add(usize::from(id))).ok()
                        .filter(|id| *id != u16::MAX)
                });
                Some(line)
            }
        }
    }

    pub(crate) fn lines(&self) -> impl DoubleEndedIterator<Item = &BlockLine> {
        (0..self.len).filter_map(|i| self.line(i).map(|line| line.line))
    }

    pub(crate) fn lines_from(&self, start: usize) -> impl Iterator<Item = &BlockLine> {
        (start..self.len).filter_map(|i| self.line(i).map(|line| line.line))
    }

    pub(crate) fn copy_text(&self) -> Option<String> {
        let mut text = String::new();
        let mut wrote = false;
        for index in 0..self.len {
            let Some(source) = self.line(index) else { continue };
            if matches!(source.line.selectable, Selectable::None) { continue; }
            if wrote { text.push_str(source.line.joiner.as_deref().unwrap_or("\n")); }
            let selected = super::super::types::derive_selection_text(source.line);
            if let Some(boundary) = source.boundary {
                text.push_str(&boundary.apply(selected, true, true));
            } else { text.push_str(&selected); }
            wrote = true;
        }
        wrote.then_some(text)
    }

    pub(crate) fn materialize(&self) -> BlockOutput {
        BlockOutput { lines: (0..self.len).filter_map(|i| self.line(i)).map(|source| {
            let mut line = source.line.clone();
            line.selection_range = source.range_id;
            line
        }).collect() }
    }


}

#[derive(Debug)]
pub(crate) enum EntryTree {
    Leaf(Rc<OutputTree>),
    Branch { left: Rc<EntryTree>, right: Rc<EntryTree>, entries: usize, output: Rc<OutputTree> },
}

impl EntryTree {
    fn join(left: Rc<Self>, right: Rc<Self>) -> Rc<Self> {
        let entries = left.entries() + right.entries();
        let output = OutputTree::branch([left.output(), right.output()]);
        Rc::new(Self::Branch { left, right, entries, output })
    }

    pub(crate) fn build(parts: &[Rc<OutputTree>]) -> Option<Rc<Self>> {
        match parts.len() {
            0 => None,
            1 => parts.first().map(|part| Rc::new(Self::Leaf(Rc::clone(part)))),
            len => {
                let (left, right) = parts.split_at(len / 2);
                Some(Self::join(Self::build(left)?, Self::build(right)?))
            }
        }
    }

    pub(crate) fn output(&self) -> Rc<OutputTree> {
        match self {
            Self::Leaf(output) | Self::Branch { output, .. } => Rc::clone(output),
        }
    }

    fn entries(&self) -> usize {
        match self { Self::Leaf(_) => 1, Self::Branch { entries, .. } => *entries }
    }

    pub(crate) fn replace(&self, index: usize, output: Rc<OutputTree>) -> Option<Rc<Self>> {
        match self {
            Self::Leaf(_) => (index == 0).then(|| Rc::new(Self::Leaf(output))),
            Self::Branch { left, right, .. } => {
                if index < left.entries() {
                    Some(Self::join(left.replace(index, output)?, Rc::clone(right)))
                } else {
                    Some(Self::join(Rc::clone(left), right.replace(index - left.entries(), output)?))
                }
            }
        }
    }
}
