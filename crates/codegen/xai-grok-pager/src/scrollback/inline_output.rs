use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use super::super::types::{
    BlockLine, BlockOutput, RenderedBlockOutput, Selectable, SelectionBoundary,
};

#[derive(Debug)]
pub(crate) struct RevisionSignal {
    identity: u64,
    revision: Cell<u64>,
    dependents: RefCell<Vec<Weak<RevisionSignal>>>,
    dirty_sinks: RefCell<Vec<(super::EntryId, Weak<RefCell<HashSet<super::EntryId>>>)>>,
}

static NEXT_SIGNAL_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Default for RevisionSignal {
    fn default() -> Self {
        Self { identity: NEXT_SIGNAL_IDENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            revision: Cell::new(0), dependents: RefCell::new(Vec::new()), dirty_sinks: RefCell::new(Vec::new()) }
    }
}

impl RevisionSignal {
    pub(crate) fn identity(&self) -> u64 { self.identity }

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
    output: Rc<OutputTree>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputError { GeometryOverflow }

#[derive(Debug, Clone)]
enum SourceRevision {
    Static,
    Tracked { signal: Rc<RevisionSignal>, revision: u64, lease: Weak<RevisionSignal> },
}

impl SourceRevision {
    fn current(&self) -> bool {
        match self { Self::Static => true, Self::Tracked { signal, revision, lease } => lease.strong_count() > 0 && signal.revision() == *revision }
    }
}

#[derive(Debug, Clone)]
enum TreeData {
    Leaf { output: Rc<RenderedBlockOutput>, starts: Vec<usize>, rows: std::collections::HashMap<usize, Vec<usize>> },
    Branch(Vec<Part>),
    Identified { output: Rc<OutputTree>, source: super::super::selection_identity::EntryIdentity, part: super::super::selection_identity::SelectablePart, revision: SourceRevision },
    Rooted { output: Rc<OutputTree>, root: super::super::selection_identity::EntryIdentity },
    Deferred { source: super::super::selection_identity::EntryIdentity },
    Native(Rc<NativePart>),
    Unavailable { reason: OutputError, notice: Rc<OutputTree> },
    Transcript { tree: Rc<EntryTree>, indices: Rc<std::collections::HashMap<super::EntryId, usize>>,
        binding: super::super::selection_identity::SourceBinding, materialized: Vec<usize> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorStep {
    Entry(super::super::selection_identity::EntryIdentity, usize),
    Branch(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorPosition {
    Text { range: super::super::selection_identity::RangeIdentity,
        position: super::super::selection_position::SourcePosition },
    Row(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutputAnchor {
    pub path: Vec<AnchorStep>,
    pub position: AnchorPosition,
}

#[derive(Debug, Clone)]
pub(crate) struct NativePart {
    pub entry: Rc<super::ScrollbackEntry>,
    pub output: Rc<OutputTree>,
    pub signal: Rc<RevisionSignal>,
    pub revision: u64,
    pub binding_lease: Weak<RevisionSignal>,
    pub authority: Weak<()>,
    pub clock: Rc<Cell<u64>>,
    pub appearance: crate::appearance::AppearanceConfig,
    pub cwd: Option<std::path::PathBuf>,
    pub media_paths: Rc<Vec<std::path::PathBuf>>,
    pub width: u16,
    pub layout: super::super::state::EntryLayoutInfo,
    pub header_label: Option<super::super::state::verb_group::GroupHeaderLabel>,
}

#[derive(Debug, Clone)]
pub(crate) struct OutputTree {
    data: TreeData,
    len: usize,
}

pub(crate) struct OutputLine<'a> {
    pub line: &'a BlockLine,
    pub range_id: Option<super::super::selection_identity::RangeIdentity>,
    pub local_row: usize,
    pub byte_start: usize,
    pub native: bool,
    pub boundary: Option<&'a Arc<SelectionBoundary>>,
}

impl OutputTree {
    pub(crate) fn leaf(output: Rc<RenderedBlockOutput>) -> Rc<Self> {
        let mut offsets = std::collections::HashMap::<usize, usize>::new();
        let mut rows = std::collections::HashMap::<usize, Vec<usize>>::new();
        let mut starts = Vec::with_capacity(output.output.len());
        for (index, line) in output.output.lines.iter().enumerate() {
            let Some(range) = line.selection_range else { starts.push(0); continue };
            rows.entry(range).or_default().push(index);
            let seen = offsets.contains_key(&range);
            let offset = offsets.entry(range).or_default();
            let joiner = if seen { line.joiner.as_deref().unwrap_or("\n").len() } else { 0 };
            let boundary = output.boundaries.get(index);
            let prefix = boundary.map_or(0, |boundary| boundary.prefix_len());
            let suffix = boundary.map_or(0, |boundary| boundary.suffix_len());
            let Some(start) = offset.checked_add(joiner).and_then(|offset| offset.checked_add(prefix)) else {
                return Self::unavailable(OutputError::GeometryOverflow);
            };
            let Some(end) = start.checked_add(super::super::types::derive_selection_text(line).len())
                .and_then(|end| end.checked_add(suffix)) else {
                return Self::unavailable(OutputError::GeometryOverflow);
            };
            starts.push(start);
            *offset = end;
        }
        Rc::new(Self { len: output.output.len(), data: TreeData::Leaf { output, starts, rows } })
    }

    pub(crate) fn identified(output: Rc<RenderedBlockOutput>, source: super::super::selection_identity::EntryIdentity,
        part: super::super::selection_identity::SelectablePart) -> Rc<Self>
    {
        let output = Self::leaf(output);
        Rc::new(Self { len: output.len, data: TreeData::Identified { output, source, part, revision: SourceRevision::Static } })
    }

    pub(crate) fn tracked(output: Rc<RenderedBlockOutput>, source: super::super::selection_identity::EntryIdentity,
        part: super::super::selection_identity::SelectablePart, signal: Rc<RevisionSignal>, lease: Weak<RevisionSignal>) -> Rc<Self>
    {
        let output = Self::leaf(output);
        let revision = SourceRevision::Tracked { revision: signal.revision(), signal, lease };
        Rc::new(Self { len: output.len, data: TreeData::Identified { output, source, part, revision } })
    }

    pub(crate) fn rooted(output: Rc<Self>, root: super::super::selection_identity::EntryIdentity) -> Rc<Self> {
        Rc::new(Self { len: output.len, data: TreeData::Rooted { output, root } })
    }

    pub(crate) fn branch(outputs: impl IntoIterator<Item = Rc<Self>>) -> Rc<Self> {
        let mut len = 0usize;
        let mut parts = Vec::new();
        for output in outputs {
            if let Some(reason) = output.unavailable_reason() { return Self::unavailable(reason); }
            let Some(end) = len.checked_add(output.len) else { return Self::unavailable(OutputError::GeometryOverflow) };
            len = end;
            parts.push(Part { end, output });
        }
        Rc::new(Self { data: TreeData::Branch(parts), len })
    }

    fn unavailable(reason: OutputError) -> Rc<Self> {
        let notice = Self::notice("Output exceeds addressable virtual geometry");
        Rc::new(Self { len: notice.len(), data: TreeData::Unavailable { reason, notice } })
    }

    pub(crate) fn unavailable_reason(&self) -> Option<OutputError> {
        match &self.data {
            TreeData::Unavailable { reason, .. } => Some(*reason),
            TreeData::Rooted { output, .. } | TreeData::Identified { output, .. } => output.unavailable_reason(),
            TreeData::Transcript { tree, .. } => tree.output_ref().unavailable_reason(),
            TreeData::Native(part) => part.output.unavailable_reason(),
            TreeData::Leaf { .. } | TreeData::Branch(_) | TreeData::Deferred { .. } => None,
        }
    }

    pub(crate) fn transcript(tree: Rc<EntryTree>, indices: Rc<std::collections::HashMap<super::EntryId, usize>>,
        binding: super::super::selection_identity::SourceBinding, materialized: Vec<usize>) -> Rc<Self>
    {
        Rc::new(Self { len: tree.output().len(), data: TreeData::Transcript { tree, indices, binding, materialized } })
    }

    pub(crate) fn deferred(source: super::super::selection_identity::EntryIdentity, len: usize) -> Rc<Self> {
        Rc::new(Self { len, data: TreeData::Deferred { source } })
    }

    pub(crate) fn native(part: NativePart) -> Rc<Self> {
        let Some(len) = part.layout.height.checked_add(usize::from(part.layout.gap_after)) else {
            return Self::unavailable(OutputError::GeometryOverflow);
        };
        Rc::new(Self { len,
            data: TreeData::Native(Rc::new(part)) })
    }

    pub(crate) fn has_deferred(&self, window: std::ops::Range<usize>) -> bool {
        if window.start >= self.len || window.is_empty() { return false; }
        match &self.data {
            TreeData::Unavailable { .. } => false,
            TreeData::Deferred { .. } => true,
            TreeData::Leaf { .. } => false,
            TreeData::Transcript { tree, .. } => tree.output().has_deferred(window),
            TreeData::Native(part) => {
                if (part.authority.strong_count() == 0 || part.binding_lease.strong_count() == 0 || part.signal.revision() != part.revision) { return true; }
                let vpad = usize::from(part.entry.block.has_vpad_for(&part.appearance));
                part.entry.cached_tree().has_deferred(window.start.saturating_sub(vpad)..window.end.saturating_sub(vpad))
            },
            TreeData::Identified { output, revision, .. } => !revision.current() || output.has_deferred(window),
            TreeData::Rooted { output, .. } => output.has_deferred(window),
            TreeData::Branch(parts) => {
                let first = parts.partition_point(|part| part.end <= window.start);
                (first..parts.len()).take_while(|&i| i == 0 || parts[i - 1].end < window.end)
                    .any(|i| {
                        let start = if i == 0 { 0 } else { parts[i - 1].end };
                        parts[i].output.has_deferred(window.start.saturating_sub(start)..window.end.saturating_sub(start))
                    })
            }
        }
    }

    pub(crate) fn visit_native(&self, window: std::ops::Range<usize>, base: usize,
        visit: &mut impl FnMut(usize, &NativePart))
    {
        if window.start >= self.len || window.is_empty() { return; }
        match &self.data {
            TreeData::Native(part) => visit(base, part),
            TreeData::Transcript { tree, .. } => tree.output().visit_native(window, base, visit),
            TreeData::Leaf { .. } | TreeData::Deferred { .. } | TreeData::Unavailable { .. } => {},
            TreeData::Identified { output, .. } | TreeData::Rooted { output, .. } => output.visit_native(window, base, visit),
            TreeData::Branch(parts) => {
                let first = parts.partition_point(|part| part.end <= window.start);
                for i in (first..parts.len()).take_while(|&i| i == 0 || parts[i - 1].end < window.end) {
                    let start = if i == 0 { 0 } else { parts[i - 1].end };
                    parts[i].output.visit_native(window.start.saturating_sub(start)..window.end.saturating_sub(start),
                        base + start, visit);
                }
            }
        }
    }

    pub(crate) fn anchor_at(&self, row: usize) -> Option<OutputAnchor> {
        if row >= self.len { return None; }
        match &self.data {
            TreeData::Unavailable { .. } => None,
            TreeData::Leaf { .. } => {
                let line = self.line(row)?;
                let position = match line.range_id {
                    Some(range) => AnchorPosition::Text { range,
                        position: super::super::selection_position::SourcePosition::at(
                            &super::super::types::derive_selection_text(line.line), line.byte_start, 0) },
                    None => AnchorPosition::Row(row),
                };
                Some(OutputAnchor { path: Vec::new(), position })
            }
            TreeData::Deferred { source } => Some(OutputAnchor {
                path: vec![AnchorStep::Entry(*source, row)], position: AnchorPosition::Row(row),
            }),
            TreeData::Native(part) => {
                let vpad = usize::from(part.entry.block.has_vpad_for(&part.appearance));
                let mut anchor = row.checked_sub(vpad).and_then(|row| part.entry.cached_tree().anchor_at(row))
                    .unwrap_or(OutputAnchor { path: Vec::new(), position: AnchorPosition::Row(row) });
                anchor.path.insert(0, AnchorStep::Entry(part.entry.selection_identity(), row));
                Some(anchor)
            }
            TreeData::Transcript { tree, .. } => {
                let (index, local) = tree.part_at_row(row)?;
                tree.part(index)?.anchor_at(local)
            }
            TreeData::Identified { output, source, part, .. } => {
                let mut anchor = output.anchor_at(row)?;
                if let AnchorPosition::Text { range, .. } = &mut anchor.position {
                    *range = super::super::selection_identity::RangeIdentity::new(*source, *part, range.local);
                }
                Some(anchor)
            }
            TreeData::Rooted { output, root } => {
                let mut anchor = output.anchor_at(row)?;
                if let AnchorPosition::Text { range, .. } = &mut anchor.position { range.root = *root; }
                Some(anchor)
            }
            TreeData::Branch(parts) => {
                let index = parts.partition_point(|part| part.end <= row);
                let start = if index == 0 { 0 } else { parts.get(index - 1)?.end };
                let mut anchor = parts.get(index)?.output.anchor_at(row - start)?;
                anchor.path.insert(0, AnchorStep::Branch(index));
                Some(anchor)
            }
        }
    }

    pub(crate) fn anchor_needs_preparation(&self, anchor: &OutputAnchor) -> bool {
        self.deferred_anchor(&anchor.path)
    }

    fn deferred_anchor(&self, path: &[AnchorStep]) -> bool {
        match &self.data {
            TreeData::Deferred { source } => matches!(path.first(), Some(AnchorStep::Entry(expected, _)) if expected == source),
            TreeData::Transcript { tree, indices, .. } => match path.first() {
                Some(AnchorStep::Entry(source, _)) => indices.get(&source.entry).and_then(|index| tree.part(*index))
                    .is_some_and(|part| part.deferred_anchor(path)),
                Some(AnchorStep::Branch(_)) | None => false,
            },
            TreeData::Branch(parts) => match path.first() {
                Some(AnchorStep::Branch(index)) => parts.get(*index).is_some_and(|part|
                    part.output.deferred_anchor(path.get(1..).unwrap_or(&[]))),
                Some(AnchorStep::Entry(_, _)) | None => false,
            },
            TreeData::Native(part) => match path.first() {
                Some(AnchorStep::Entry(source, _)) if *source == part.entry.selection_identity() =>
                    part.output.deferred_anchor(path.get(1..).unwrap_or(&[])),
                Some(AnchorStep::Entry(_, _) | AnchorStep::Branch(_)) | None => false,
            },
            TreeData::Identified { output, .. } | TreeData::Rooted { output, .. } => output.deferred_anchor(path),
            TreeData::Leaf { .. } | TreeData::Unavailable { .. } => false,
        }
    }

    pub(crate) fn row_for_anchor(&self, anchor: &OutputAnchor) -> Option<usize> {
        self.resolve_anchor(&anchor.path, &anchor.position)
    }

    fn resolve_anchor(&self, path: &[AnchorStep], position: &AnchorPosition) -> Option<usize> {
        match &self.data {
            TreeData::Unavailable { .. } => None,
            TreeData::Transcript { tree, indices, .. } => {
                let AnchorStep::Entry(source, _) = path.first()? else { return None };
                let index = *indices.get(&source.entry)?;
                tree.row_of_part(index)?.checked_add(tree.part(index)?.resolve_anchor(path, position)?)
            }
            TreeData::Branch(parts) => {
                let AnchorStep::Branch(index) = path.first()? else { return None };
                let start = if *index == 0 { 0 } else { parts.get(index - 1)?.end };
                start.checked_add(parts.get(*index)?.output.resolve_anchor(path.get(1..)?, position)?)
            }
            TreeData::Native(part) => {
                let AnchorStep::Entry(source, _) = path.first()? else { return None };
                if *source != part.entry.selection_identity() { return None; }
                let tail = path.get(1..)?;
                match position {
                    AnchorPosition::Row(row) if tail.is_empty() => (*row < self.len).then_some(*row),
                    AnchorPosition::Row(_) | AnchorPosition::Text { .. } => {
                        let vpad = usize::from(part.entry.block.has_vpad_for(&part.appearance));
                        part.entry.cached_tree().resolve_anchor(tail, position)?.checked_add(vpad)
                    }
                }
            }
            TreeData::Deferred { source } => {
                let AnchorStep::Entry(expected, row) = path.first()? else { return None };
                (*expected == *source && *row < self.len).then_some(*row)
            }
            TreeData::Identified { output, source, part, .. } => {
                if let AnchorPosition::Text { range, .. } = position
                    && (range.source != *source || range.part != *part) { return None; }
                output.resolve_anchor(path, position)
            }
            TreeData::Rooted { output, .. } => output.resolve_anchor(path, position),
            TreeData::Leaf { .. } => match position {
                AnchorPosition::Text { range, position } => self.resolve_position(range.local, *position).map(|(row, _)| row),
                AnchorPosition::Row(row) => (*row < self.len).then_some(*row),
            },
        }
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
            TreeData::Unavailable { notice, .. } => notice.line(index),
            TreeData::Deferred { .. } => None,
            TreeData::Native(part) => {
                let vpad = usize::from(part.entry.block.has_vpad_for(&part.appearance));
                let mut line = part.output.line(index.checked_sub(vpad)?)?;
                line.native = true;
                Some(line)
            },
            TreeData::Transcript { tree, .. } => tree.output_ref().line(index),
            TreeData::Leaf { output, starts, .. } => {
                let line = output.output.lines.get(index)?;
                Some(OutputLine { line, native: false, local_row: index, byte_start: *starts.get(index)?, range_id: line.selection_range.map(super::super::selection_identity::RangeIdentity::overlay),
                    boundary: output.boundaries.get(index) })
            }
            TreeData::Branch(parts) => {
                let pos = parts.partition_point(|part| part.end <= index);
                let part = parts.get(pos)?;
                let start = if pos == 0 { 0 } else { parts.get(pos - 1)?.end };
                part.output.line(index.checked_sub(start)?)
            }
            TreeData::Identified { output, source, part, .. } => {
                let mut line = output.line(index)?;
                line.range_id = line.range_id.map(|range| super::super::selection_identity::RangeIdentity::new(*source, *part, range.local));
                Some(line)
            }
            TreeData::Rooted { output, root } => {
                let mut line = output.line(index)?;
                if let Some(range) = line.range_id.as_mut() { range.root = *root; }
                Some(line)
            }
        }
    }

    pub(crate) fn selection_owner(&self, range: super::super::selection_identity::RangeIdentity) -> Option<&NativePart> {
        match &self.data {
            TreeData::Native(part) => {
                if (part.authority.strong_count() == 0 || part.binding_lease.strong_count() == 0 || part.signal.revision() != part.revision) { return None; }
                if part.entry.selection_identity() == range.source { Some(part) }
                else { part.output.selection_owner(range) }
            }
            TreeData::Transcript { tree, indices, binding, materialized } => {
                if *binding == range.source.binding {
                    tree.part_ref(*indices.get(&range.source.entry)?)?.selection_owner(range)
                } else { materialized.iter().find_map(|index| tree.part_ref(*index)?.selection_owner(range)) }
            }
            TreeData::Rooted { output, .. } | TreeData::Identified { output, .. } => output.selection_owner(range),
            TreeData::Branch(parts) => parts.iter().find_map(|part| part.output.selection_owner(range)),
            TreeData::Leaf { .. } | TreeData::Deferred { .. } | TreeData::Unavailable { .. } => None,
        }
    }

    pub(crate) fn selection_leaf(&self, range: super::super::selection_identity::RangeIdentity) -> Option<Rc<OutputTree>> {
        match &self.data {
            TreeData::Leaf { .. } | TreeData::Deferred { .. } | TreeData::Unavailable { .. } => None,
            TreeData::Native(part) => ((part.authority.strong_count() > 0 && part.binding_lease.strong_count() > 0 && part.signal.revision() == part.revision))
                .then(|| part.output.selection_leaf(range)).flatten(),
            TreeData::Transcript { tree, indices, binding, materialized } => {
                if range.source.binding == *binding {
                    tree.part(*indices.get(&range.source.entry)?)?.selection_leaf(range)
                } else {
                    materialized.iter().find_map(|index| tree.part(*index)?.selection_leaf(range))
                }
            },
            TreeData::Branch(parts) => parts.iter().find_map(|part| part.output.selection_leaf(range)),
            TreeData::Identified { output, source, part, revision } => {
                (revision.current() && *source == range.source && *part == range.part).then(|| Rc::clone(output))
            }
            TreeData::Rooted { output, .. } => output.selection_leaf(range)
        }
    }

    pub(crate) fn resolve_position(&self, local: usize, position: super::super::selection_position::SourcePosition)
        -> Option<(usize, u16)>
    {
        use super::super::selection_position::SourcePosition;
        if let SourcePosition::Overlay { row, column } = position { return Some((row, column)); }
        let TreeData::Leaf { output, starts, rows } = &self.data else { return None };
        let rows = rows.get(&local)?;
        let byte = position.byte()?;
        let at = rows.partition_point(|&index| starts.get(index).is_some_and(|start| *start <= byte));
        let index = *rows.get(at.checked_sub(1)?)?;
        let line = output.output.lines.get(index)?;
        let text = super::super::types::derive_selection_text(line);
        position.column(&text, *starts.get(index)?).map(|column| (index, column))
    }

    pub(crate) fn rendered_leaf(&self) -> Option<Rc<RenderedBlockOutput>> {
        match &self.data {
            TreeData::Leaf { output, .. } => Some(Rc::clone(output)),
            TreeData::Branch(_) | TreeData::Identified { .. } | TreeData::Rooted { .. }
            | TreeData::Native(_) | TreeData::Deferred { .. } | TreeData::Transcript { .. } | TreeData::Unavailable { .. } => None,
        }
    }

    pub(crate) fn lines(&self) -> impl DoubleEndedIterator<Item = &BlockLine> {
        (0..self.len).filter_map(|i| self.line(i).map(|line| line.line))
    }

    pub(crate) fn lines_from(&self, start: usize) -> impl Iterator<Item = &BlockLine> {
        (start..self.len).filter_map(|i| self.line(i).filter(|line| !line.native).map(|line| line.line))
    }

    pub(crate) fn materialize(&self) -> BlockOutput {
        BlockOutput { lines: (0..self.len).filter_map(|i| self.line(i)).map(|source| {
            let mut line = source.line.clone();
            line.selection_range = source.range_id.map(|range| range.local);
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

    fn output_ref(&self) -> &OutputTree {
        match self { Self::Leaf(output) | Self::Branch { output, .. } => output }
    }

    pub(crate) fn output(&self) -> Rc<OutputTree> {
        match self {
            Self::Leaf(output) | Self::Branch { output, .. } => Rc::clone(output),
        }
    }

    fn entries(&self) -> usize {
        match self { Self::Leaf(_) => 1, Self::Branch { entries, .. } => *entries }
    }

    pub(crate) fn part_at_row(&self, row: usize) -> Option<(usize, usize)> {
        match self {
            Self::Leaf(output) => (row < output.len()).then_some((0, row)),
            Self::Branch { left, right, .. } => {
                let left_len = left.output().len();
                if row < left_len { left.part_at_row(row) }
                else { right.part_at_row(row - left_len).map(|(i, local)| (left.entries() + i, local)) }
            }
        }
    }

    fn part_ref(&self, index: usize) -> Option<&OutputTree> {
        match self {
            Self::Leaf(output) => (index == 0).then_some(output.as_ref()),
            Self::Branch { left, right, .. } => if index < left.entries() { left.part_ref(index) }
                else { right.part_ref(index - left.entries()) },
        }
    }

    pub(crate) fn part(&self, index: usize) -> Option<Rc<OutputTree>> {
        match self {
            Self::Leaf(output) => (index == 0).then(|| Rc::clone(output)),
            Self::Branch { left, right, .. } => if index < left.entries() { left.part(index) }
                else { right.part(index - left.entries()) },
        }
    }

    pub(crate) fn row_of_part(&self, index: usize) -> Option<usize> {
        match self {
            Self::Leaf(_) => (index == 0).then_some(0),
            Self::Branch { left, right, .. } => if index < left.entries() { left.row_of_part(index) }
                else { right.row_of_part(index - left.entries()).map(|row| left.output().len() + row) },
        }
    }

    pub(crate) fn parts_in(&self, window: std::ops::Range<usize>) -> Vec<usize> {
        let mut parts = Vec::new();
        let mut row = window.start;
        while row < window.end {
            let Some((index, _)) = self.part_at_row(row) else { break };
            parts.push(index);
            let Some(start) = self.row_of_part(index) else { break };
            let Some(part) = self.part(index) else { break };
            let Some(next) = start.checked_add(part.len()) else { break };
            if next <= row { break; }
            row = next;
        }
        parts
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
