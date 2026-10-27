use super::entry::EntryId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceBinding {
    Detached,
    Session { agent: usize, epoch: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntryIdentity {
    pub binding: SourceBinding,
    pub entry: EntryId,
    pub instance: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectablePart {
    Block,
    TaskOutput(u64),
    GroupHeader,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RangeIdentity {
    pub root: EntryIdentity,
    pub source: EntryIdentity,
    pub part: SelectablePart,
    pub local: usize,
}

impl RangeIdentity {
    pub fn new(source: EntryIdentity, part: SelectablePart, local: usize) -> Self {
        Self { root: source, source, part, local }
    }

    pub const fn overlay(local: usize) -> Self {
        let source = EntryIdentity { binding: SourceBinding::Detached,
            entry: EntryId::new(0), instance: 0 };
        Self { root: source, source, part: SelectablePart::Overlay, local }
    }
}
