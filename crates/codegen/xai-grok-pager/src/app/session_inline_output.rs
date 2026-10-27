use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use super::SessionViews;
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentRole, inline_sources::{InlineReplay, TaskProjection, ChildProjection}};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::entry::{EntryId, ScrollbackEntry};
use crate::scrollback::entry::inline_output::{InlineProjection, OutputTree, SourceBody, SourceKey, EntryTree, SourceIdentity, TaskSource, OutputPhase};
use crate::scrollback::types::{BlockOutput, DisplayMode, RenderedBlockOutput,
    SelectionBoundaries, SelectionBoundary, SelectionBoundaryEntry, selectable_cols};

impl SessionViews {
    pub(crate) fn note_inline_content_changed(&mut self, id: AgentId) {
        if let Some(view) = self.get(&id) { view.inline_sources.signal.bump(); }
        let mut seen = HashSet::new();
        let mut next = Some(id);
        while let Some(child) = next.filter(|id| seen.insert(*id)) {
            next = self.parent_of(child);
            let link = self.link_and_parent(child).map(|(link, _)| link.subagent_id.clone());
            if let (Some(parent), Some(sid)) = (next, link)
                && let Some(view) = self.get_mut(&parent)
                && let Some(info) = view.subagent_sessions.get(&sid)
            {
                let rows = [info.attempt.scrollback_entry_id, info.attempt.terminal_entry_id];
                for row in rows.into_iter().flatten() {
                    view.scrollback.mark_inline_height_dirty(row);
                }
            }
        }
    }

    pub(crate) fn inline_copy_text(&mut self, owner: AgentId, id: EntryId) -> Option<String> {
        let view = self.get(&owner)?;
        let entry = view.scrollback.get_by_id(id)?;
        let mut dirty = Vec::new();
        let text = match &entry.block {
            RenderBlock::BgTask(row) => view.session.bg_tasks.get(&row.task_id)
                .filter(|task| !task.stdout.is_empty()).map(|task| task.stdout.clone()),
            RenderBlock::Subagent(row) => {
                let width = entry.inline_request_width()?;
                let key = self.source_key(owner, entry, &[owner])?;
                let body = self.project_child(owner, &row.child_session_id, &key, width,
                    true, &mut vec![owner], &mut dirty);
                match body {
                    SourceBody::Content(output) => output.copy_text(),
                    SourceBody::Available | SourceBody::Empty | SourceBody::Waiting | SourceBody::Loading
                    | SourceBody::Unavailable | SourceBody::Cycle => None,
                }
            }
            RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
            | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => None,
        };
        for (owner, id) in dirty {
            if let Some(view) = self.get_mut(&owner) { view.scrollback.mark_inline_height_dirty(id); }
        }
        text
    }

    fn inline_child(&self, parent: AgentId, sid: &str) -> Option<AgentId> {
        let child = self.find_by_session_id(sid)?;
        let parent_sid = self.get(&parent)?.session.session_id.as_ref()?;
        match &self.get(&child)?.role {
            AgentRole::Root => None,
            AgentRole::Child(link) => (link.subagent_id == sid
                && &link.parent_session_id == parent_sid
                && self.parent_of(child) == Some(parent)).then_some(child),
        }
    }

    fn source_key(&self, owner: AgentId, entry: &ScrollbackEntry, path: &[AgentId])
        -> Option<SourceKey>
    {
        let view = self.get(&owner)?;
        let identity = match &entry.block {
            RenderBlock::BgTask(task) => {
                let state = match view.session.bg_tasks.get(&task.task_id) {
                    None => TaskSource::Missing,
                    Some(task) => TaskSource::Present(match task.status {
                        crate::app::agent::BgTaskStatus::Running => OutputPhase::Running,
                        crate::app::agent::BgTaskStatus::Done | crate::app::agent::BgTaskStatus::Failed => OutputPhase::Final,
                    }),
                };
                SourceIdentity::Task { owner: owner.0, binding: view.session_binding_epoch,
                    id: task.task_id.clone(), revision: view.inline_sources.task_revision(&task.task_id), state }
            }
            RenderBlock::Subagent(row) => {
                if let Some(child) = self.inline_child(owner, &row.child_session_id) {
                    let child_view = self.get(&child)?;
                    SourceIdentity::Child { owner: child.0, binding: child_view.session_binding_epoch,
                        id: row.child_session_id.clone(), ancestry: path.iter().map(|id| id.0).collect(),
                        revision: child_view.inline_sources.signal.revision(),
                        content: child_view.scrollback.content_generation() }
                } else {
                    SourceIdentity::MissingChild { parent: owner.0, binding: view.session_binding_epoch,
                        id: row.child_session_id.clone(), revision: view.inline_sources.signal.revision() }
                }
            }
            RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
            | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => return None,
        };
        Some(SourceKey { identity, theme: crate::theme::cache::current_kind(),
            thinking_visible: crate::appearance::cache::load_show_thinking_blocks() })
    }

    pub(crate) fn prepare_inline_output(&mut self, owner: AgentId) -> bool {
        let requests = self.get(&owner).map(|view| view.scrollback.inline_render_requests())
            .unwrap_or_default();
        let mut dirty = Vec::new();
        for (id, width) in requests {
            let pending_child = self.get(&owner).and_then(|view| {
                let entry = view.scrollback.get_by_id(id)?;
                let key = self.source_key(owner, entry, &[owner])?;
                if entry.inline_source_matches(&key, width) { return None; }
                match &entry.block {
                    RenderBlock::Subagent(row) if entry.display_mode != DisplayMode::Collapsed =>
                        Some(row.child_session_id.clone()),
                    RenderBlock::Subagent(_) | RenderBlock::BgTask(_)
                    | RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
                    | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
                    | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
                    | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => None,
                }
            });
            if let Some(sid) = pending_child {
                self.prepare_passive_child(owner, &sid, &mut vec![owner]);
            }
            if let Some(view) = self.get(&owner)
                && let Some(entry) = view.scrollback.get_by_id(id)
            {
                self.project_inline_entry(owner, entry, width, &mut vec![owner], &mut dirty);
            }
        }
        let changed = !dirty.is_empty();
        let mut touched = HashSet::new();
        for (agent, id) in dirty {
            touched.insert(agent);
            if let Some(view) = self.get_mut(&agent) {
                view.scrollback.mark_inline_height_dirty(id);
                if let Some(entry) = view.scrollback.get_by_id(id)
                    && let RenderBlock::BgTask(task) = &entry.block
                {
                    let rows = view.inline_sources.observed_tasks.entry(task.task_id.clone()).or_default();
                    if !rows.contains(&id) { rows.push(id); }
                }
            }
        }
        let pending: Vec<_> = touched.into_iter().flat_map(|owner| {
            self.get(&owner).map(|view| view.inline_sources.pending_children.borrow_mut().drain()
                .map(|sid| (owner, sid)).collect::<Vec<_>>()).unwrap_or_default()
        }).collect();
        for (owner, sid) in pending {
            self.prepare_passive_child(owner, &sid, &mut vec![owner]);
        }
        changed
    }

    fn prepare_passive_child(&mut self, parent: AgentId, sid: &str, path: &[AgentId]) {
        let Some(child) = self.inline_child(parent, sid).filter(|id| !path.contains(id)) else { return };
        if self.get(&child).is_some_and(|view| view.session.loading_replay) { return; }
        let unrequested = self.get(&child).is_some_and(|view|
            matches!(view.inline_sources.replay, InlineReplay::Unrequested));
        if unrequested {
            let outcome = crate::app::subagent::replay_on_open(self, child);
            if let Some(view) = self.get_mut(&child) {
                view.inline_sources.replay = InlineReplay::Attempted(outcome);
            }
            self.note_inline_content_changed(child);
        }
    }

    fn project_inline_entry(&self, owner: AgentId, entry: &ScrollbackEntry, width: u16,
        path: &mut Vec<AgentId>, dirty: &mut Vec<(AgentId, EntryId)>)
    {
        let Some(key) = self.source_key(owner, entry, path) else { return };
        if entry.inline_source_matches(&key, width) { return; }
        let expanded = entry.display_mode != DisplayMode::Collapsed;
        let body = match &entry.block {
            RenderBlock::BgTask(task) => self.project_task(owner, &task.task_id, &key, width, expanded),
            RenderBlock::Subagent(row) => {
                if let Some(child) = self.inline_child(owner, &row.child_session_id)
                    && let (Some(child_view), Some(parent_view)) = (self.get(&child), self.get(&owner))
                {
                    child_view.inline_sources.signal.subscribe_entry(entry.id, &parent_view.inline_sources.dirty_entries);
                }
                self.project_child(owner, &row.child_session_id, &key, width, expanded, path, dirty)
            }
            RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
            | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => return,
        };
        entry.set_inline_source(InlineProjection { key, width, expanded, body });
        dirty.push((owner, entry.id));
    }

    fn project_task(&self, owner: AgentId, id: &str, key: &SourceKey, width: u16, expanded: bool) -> SourceBody {
        let Some(view) = self.get(&owner) else { return SourceBody::Unavailable };
        let Some(task) = view.session.bg_tasks.get(id) else { return SourceBody::Unavailable };
        if task.stdout.is_empty() {
            return match task.status {
                crate::app::agent::BgTaskStatus::Running => SourceBody::Waiting,
                crate::app::agent::BgTaskStatus::Done | crate::app::agent::BgTaskStatus::Failed => SourceBody::Empty,
            };
        }
        if !expanded { return SourceBody::Available; }
        if let Some(cached) = view.inline_sources.task_outputs.borrow().get(id)
            && cached.key == *key && cached.width == width
            && let Some(output) = cached.output.upgrade()
        {
            return SourceBody::Content(output);
        }
        let lines: Vec<_> = crate::scrollback::blocks::tool::body::wrapped_body(&task.stdout,
            crate::theme::Theme::current().primary(), usize::from(width), "")
            .into_iter().map(|line| line.with_selection_range(Some(0))).collect();
        let boundaries = SelectionBoundaries::from_entries(lines.iter().enumerate()
            .filter(|(_, line)| selectable_cols(&line.content, &line.selectable)
                .is_some_and(|cols| cols.is_empty()))
            .map(|(line_index, _)| SelectionBoundaryEntry { line_index,
                boundary: Arc::new(SelectionBoundary::empty_row_anchor(String::new(), String::new())),
            }).collect());
        let output = OutputTree::leaf(Rc::new(RenderedBlockOutput { output: BlockOutput { lines }, boundaries }));
        view.inline_sources.task_outputs.borrow_mut().insert(id.to_owned(), TaskProjection {
            key: key.clone(), width, output: Rc::downgrade(&output),
        });
        SourceBody::Content(output)
    }

    fn project_child(&self, parent: AgentId, sid: &str, key: &SourceKey, width: u16,
        expanded: bool, path: &mut Vec<AgentId>, dirty: &mut Vec<(AgentId, EntryId)>) -> SourceBody
    {
        let Some(child) = self.inline_child(parent, sid) else { return SourceBody::Unavailable };
        if path.contains(&child) { return SourceBody::Cycle; }
        let Some(view) = self.get(&child) else { return SourceBody::Unavailable };
        if let Some(parent_view) = self.get(&parent) {
            view.inline_sources.signal.subscribe(&parent_view.inline_sources.signal);
        }
        if view.session.loading_replay { return SourceBody::Loading; }
        if expanded && matches!(view.inline_sources.replay, InlineReplay::Unrequested)
            && let Some(parent_view) = self.get(&parent)
        {
            parent_view.inline_sources.pending_children.borrow_mut().insert(sid.to_owned());
        }
        if view.scrollback.is_empty() {
            return match view.inline_sources.replay {
                InlineReplay::Unrequested => SourceBody::Loading,
                InlineReplay::Attempted(crate::app::subagent::ChildReplayOutcome::ReadFailed
                    | crate::app::subagent::ChildReplayOutcome::UnknownChild) => SourceBody::Unavailable,
                InlineReplay::Attempted(crate::app::subagent::ChildReplayOutcome::Replayed
                    | crate::app::subagent::ChildReplayOutcome::FoundNothingOnDisk
                    | crate::app::subagent::ChildReplayOutcome::NothingToRead
                    | crate::app::subagent::ChildReplayOutcome::ViewHoldsLiveBlocks) => {
                        if self.child_info(child).is_some_and(|info| info.is_running()) {
                            SourceBody::Waiting
                        } else { SourceBody::Empty }
                    },
            };
        }
        if !expanded { return SourceBody::Available; }
        if let Some(cached) = view.inline_sources.transcript.borrow().as_ref()
            && cached.key == *key && cached.width == width
        {
            let output = cached.tree.output();
            return if output.is_empty() { SourceBody::Empty } else { SourceBody::Content(output) };
        }
        path.push(child);
        let changed: Vec<_> = view.inline_sources.dirty_entries.borrow_mut().drain().collect();
        let cached = view.inline_sources.transcript.take();
        let show_thinking = crate::appearance::cache::load_show_thinking_blocks();
        let mut incremental = cached.filter(|cache| cache.width == width && cache.key.same_child_owner(key)
            && cache.structure == view.scrollback.outline_generation()
            && cache.indices.len() == view.scrollback.len()
            && cache.key.theme == key.theme && cache.key.thinking_visible == key.thinking_visible
            && !changed.is_empty());
        let mut complete = true;
        if let Some(cache) = incremental.as_mut() {
            for id in &changed {
                let updated = cache.indices.get(id).copied().and_then(|index| {
                    let entry = view.scrollback.get_by_id(*id)?;
                    self.project_inline_entry(child, entry, width, path, dirty);
                    let output = if entry.is_hidden_thinking(show_thinking) {
                        OutputTree::branch([])
                    } else {
                        entry.ensure_cached(width, view.scrollback.appearance(), false, view.scrollback.cwd());
                        entry.cached_tree()
                    };
                    cache.tree.replace(index, output)
                });
                if let Some(tree) = updated { cache.tree = tree; }
                else { complete = false; break; }
            }
        }
        if !complete { incremental = None; }
        let projection = if let Some(mut cached) = incremental {
            cached.key = key.clone();
            Some(cached)
        } else {
            let mut indices = std::collections::HashMap::new();
            let mut parts = Vec::new();
            for i in 0..view.scrollback.len() {
                let Some(entry) = view.scrollback.entry(i) else { continue };
                entry.subscribe_output_changes(&view.inline_sources.signal, &view.inline_sources.dirty_entries);
                self.project_inline_entry(child, entry, width, path, dirty);
                indices.insert(entry.id, parts.len());
                parts.push(if entry.is_hidden_thinking(show_thinking) {
                    OutputTree::branch([])
                } else {
                    entry.ensure_cached(width, view.scrollback.appearance(), false, view.scrollback.cwd());
                    entry.cached_tree()
                });
            }
            EntryTree::build(&parts).map(|tree| ChildProjection {
                key: key.clone(), width, structure: view.scrollback.outline_generation(), indices, tree,
            })
        };
        path.pop();
        let Some(projection) = projection else { return SourceBody::Empty };
        let output = projection.tree.output();
        view.inline_sources.transcript.replace(Some(projection));
        if output.is_empty() { SourceBody::Empty } else { SourceBody::Content(output) }
    }
}

#[cfg(test)]
#[path = "session_inline_output_tests.rs"]
mod tests;
