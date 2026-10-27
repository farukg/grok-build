use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use crate::scrollback::entry::EntryId;
use crate::scrollback::entry::inline_output::{OutputTree, SourceKey, RevisionSignal, EntryTree};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum InlineReplay {
    #[default]
    Unrequested,
    Attempted(crate::app::subagent::ChildReplayOutcome),
}

pub(crate) struct TaskProjection {
    pub key: SourceKey,
    pub width: u16,
    pub source: crate::scrollback::selection_identity::EntryIdentity,
    pub output: Weak<OutputTree>,
}

pub(crate) struct ChildProjection {
    pub key: SourceKey,
    pub width: u16,
    pub structure: u64,
    pub indices: Rc<HashMap<EntryId, usize>>,
    pub ids: Vec<EntryId>,
    pub layouts: Vec<crate::scrollback::state::EntryLayoutInfo>,
    pub groups: Vec<crate::scrollback::state::groups::GroupSpan>,
    pub materialized: HashSet<EntryId>,
    pub tree: Rc<EntryTree>,
}

#[derive(Default)]
pub(crate) struct InlineSources {
    pub signal: Rc<RevisionSignal>,
    task_revisions: RefCell<HashMap<String, Rc<RevisionSignal>>>,
    pub observed_tasks: HashMap<String, Vec<EntryId>>,
    pub replay: InlineReplay,
    pub transcript: RefCell<Option<ChildProjection>>,
    pub dirty_entries: Rc<RefCell<HashSet<EntryId>>>,
    pub pending_children: RefCell<HashSet<String>>,
    pub task_outputs: RefCell<HashMap<String, TaskProjection>>,
}

impl InlineSources {
    pub fn task_revision(&self, id: &str) -> u64 {
        self.task_revisions.borrow().get(id).map_or(0, |signal| signal.revision())
    }

    pub(crate) fn task_signal(&self, id: &str) -> Rc<RevisionSignal> {
        Rc::clone(self.task_revisions.borrow_mut().entry(id.to_owned()).or_default())
    }

    pub fn note_task(&mut self, id: &str) {
        self.signal.bump();
        self.task_outputs.borrow_mut().remove(id);
        self.task_signal(id).bump();
    }
}

impl super::AgentView {
    pub(crate) fn note_bg_stdout_changed(&mut self, task_id: &str) {
        self.inline_sources.note_task(task_id);
        if let Some(entries) = self.inline_sources.observed_tasks.get(task_id) {
            for id in entries {
                self.inline_sources.dirty_entries.borrow_mut().insert(*id);
                self.scrollback.mark_inline_height_dirty(*id);
            }
        }
    }
}
