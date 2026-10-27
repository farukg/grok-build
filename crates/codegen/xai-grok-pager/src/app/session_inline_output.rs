use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use super::SessionViews;
use crate::app::agent::AgentId;
use crate::app::agent_view::{AgentRole, inline_sources::{InlineReplay, TaskProjection, ChildProjection}};
use crate::scrollback::block::{BlockContent, RenderBlock};
use crate::scrollback::entry::{EntryId, ScrollbackEntry};
use crate::scrollback::entry::inline_output::{InlineProjection, OutputTree, SourceBody, SourceKey, EntryTree, SourceIdentity, TaskSource, OutputPhase};
use crate::scrollback::types::{BlockOutput, DisplayMode, RenderedBlockOutput,
    SelectionBoundaries, SelectionBoundary, SelectionBoundaryEntry, selectable_cols};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InlineCopyOutcome { Ready(String), Empty, Loading, Unavailable, Failed }

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

    pub(crate) fn inline_copy_text(&mut self, owner: AgentId, id: EntryId) -> InlineCopyOutcome {
        self.inline_copy_from(owner, id, &mut vec![owner])
    }

    fn inline_copy_from(&mut self, owner: AgentId, id: EntryId, path: &mut Vec<AgentId>) -> InlineCopyOutcome {
        let Some(view) = self.get(&owner) else { return InlineCopyOutcome::Unavailable };
        let Some(entry) = view.scrollback.get_by_id(id) else { return InlineCopyOutcome::Unavailable };
        entry.bind_selection_source(crate::scrollback::selection_identity::SourceBinding::Session { agent: owner.0, epoch: view.session_binding_epoch });
        let origin = entry.selection_identity();
        let child_sid = match &entry.block {
            RenderBlock::Subagent(row) => Some(row.child_session_id.clone()),
            RenderBlock::BgTask(task) => return match view.session.bg_tasks.get(&task.task_id) {
                Some(task) if !task.stdout.is_empty() => InlineCopyOutcome::Ready(task.stdout.clone()),
                Some(_) => InlineCopyOutcome::Empty,
                None => InlineCopyOutcome::Unavailable,
            },
            RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
            | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => None,
        };
        let Some(sid) = child_sid else { return InlineCopyOutcome::Unavailable };
        let target = self.inline_child(owner, &sid).and_then(|child|
            self.get(&child).map(|view| (child, view.session_binding_epoch)));
        let Some(target) = target else { return InlineCopyOutcome::Unavailable };
        if path.contains(&target.0) { return InlineCopyOutcome::Unavailable; }
        self.prepare_passive_child(owner, &sid, path);
        let Some(view) = self.get(&owner) else { return InlineCopyOutcome::Unavailable };
        let Some(entry) = view.scrollback.get_by_id(id) else { return InlineCopyOutcome::Unavailable };
        if entry.selection_identity() != origin
            || self.inline_child(owner, &sid).and_then(|child|
                self.get(&child).map(|view| (child, view.session_binding_epoch))) != Some(target)
        { return InlineCopyOutcome::Unavailable; }
        let Some(child_view) = self.get(&target.0) else { return InlineCopyOutcome::Unavailable };
        if child_view.session.loading_replay { return InlineCopyOutcome::Loading; }
        match child_view.inline_sources.replay {
            InlineReplay::Unrequested => return InlineCopyOutcome::Loading,
            InlineReplay::Attempted(crate::app::subagent::ChildReplayOutcome::ReadFailed) => return InlineCopyOutcome::Failed,
            InlineReplay::Attempted(crate::app::subagent::ChildReplayOutcome::UnknownChild) => return InlineCopyOutcome::Unavailable,
            InlineReplay::Attempted(crate::app::subagent::ChildReplayOutcome::Replayed
                | crate::app::subagent::ChildReplayOutcome::FoundNothingOnDisk
                | crate::app::subagent::ChildReplayOutcome::NothingToRead
                | crate::app::subagent::ChildReplayOutcome::ViewHoldsLiveBlocks) => {},
        }
        let entries: Vec<_> = child_view.scrollback.iter_entries().map(|(id, _)| id).collect();
        path.push(target.0);
        let mut text = String::new();
        let mut result = InlineCopyOutcome::Empty;
        for id in entries {
            let Some(view) = self.get(&target.0).filter(|view| view.session_binding_epoch == target.1) else {
                result = InlineCopyOutcome::Unavailable; break;
            };
            let Some(entry) = view.scrollback.get_by_id(id) else {
                result = InlineCopyOutcome::Unavailable; break;
            };
            let output = if entry.block.is_bg_task() || entry.block.is_subagent() {
                self.inline_copy_from(target.0, id, path)
            } else {
                match entry.block.copy_text(entry.raw) {
                    Some(text) => InlineCopyOutcome::Ready(text), None => InlineCopyOutcome::Empty,
                }
            };
            match output {
                InlineCopyOutcome::Ready(next) => {
                    if !next.is_empty() {
                        let Some(additional) = next.len().checked_add(usize::from(!text.is_empty())) else {
                            result = InlineCopyOutcome::Failed; break;
                        };
                        if text.try_reserve(additional).is_err() { result = InlineCopyOutcome::Failed; break; }
                        if !text.is_empty() { text.push('\n'); }
                        text.push_str(&next);
                    }
                }
                InlineCopyOutcome::Empty => {},
                InlineCopyOutcome::Loading | InlineCopyOutcome::Unavailable | InlineCopyOutcome::Failed => {
                    result = output; break;
                }
            }
        }
        path.pop();
        if !matches!(result, InlineCopyOutcome::Empty) { return result; }
        if text.is_empty() { InlineCopyOutcome::Empty } else { InlineCopyOutcome::Ready(text) }
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
        let bookmark = self.get(&owner).and_then(|view| view.scrollback.inline_settlement_bookmark());
        let requests = self.get(&owner).map(|view| view.scrollback.inline_render_requests())
            .unwrap_or_default();
        let mut dirty = Vec::new();
        for (id, width, window) in requests {
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
                self.project_inline_entry(owner, entry, width, window, &mut vec![owner], &mut dirty);
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
        if changed && let Some(view) = self.get_mut(&owner) { view.scrollback.settle_inline_bookmark(bookmark); }
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
        window: std::ops::Range<usize>, path: &mut Vec<AgentId>, dirty: &mut Vec<(AgentId, EntryId)>)
    {
        let Some(view) = self.get(&owner) else { return };
        entry.bind_selection_source(crate::scrollback::selection_identity::SourceBinding::Session { agent: owner.0, epoch: view.session_binding_epoch });
        let Some(key) = self.source_key(owner, entry, path) else { return };
        if entry.inline_source_matches(&key, width) && !entry.inline_window_pending(window.clone()) { return; }
        let expanded = entry.display_mode != DisplayMode::Collapsed;
        let body = match &entry.block {
            RenderBlock::BgTask(task) => self.project_task(owner, &task.task_id, entry.selection_identity(), &key, width, expanded),
            RenderBlock::Subagent(row) => {
                if let Some(child) = self.inline_child(owner, &row.child_session_id)
                    && let (Some(child_view), Some(parent_view)) = (self.get(&child), self.get(&owner))
                {
                    child_view.inline_sources.signal.subscribe_entry(entry.id, &parent_view.inline_sources.dirty_entries);
                }
                self.project_child(owner, &row.child_session_id, &key, width, expanded, entry.inline_body_window(window), path, dirty)
            }
            RenderBlock::Stub(_) | RenderBlock::UserPrompt(_) | RenderBlock::AgentMessage(_)
            | RenderBlock::ToolCall(_) | RenderBlock::Thinking(_) | RenderBlock::System(_)
            | RenderBlock::SessionEvent(_) | RenderBlock::Workflow(_) | RenderBlock::Btw(_)
            | RenderBlock::ContextInfo(_) | RenderBlock::MemoryCapture(_) | RenderBlock::Injected(_) => return,
        };
        entry.set_inline_source(InlineProjection { key, width, expanded, body });
        dirty.push((owner, entry.id));
    }

    fn project_task(&self, owner: AgentId, id: &str, source: crate::scrollback::selection_identity::EntryIdentity, key: &SourceKey, width: u16, expanded: bool) -> SourceBody {
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
            && cached.key == *key && cached.width == width && cached.source == source
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
        let output = OutputTree::tracked(Rc::new(RenderedBlockOutput { output: BlockOutput { lines }, boundaries }),
            source, crate::scrollback::selection_identity::SelectablePart::TaskOutput(view.inline_sources.task_signal(id).identity()), view.inline_sources.task_signal(id), Rc::downgrade(&view.inline_sources.signal));
        view.inline_sources.task_outputs.borrow_mut().insert(id.to_owned(), TaskProjection {
            key: key.clone(), width, source, output: Rc::downgrade(&output),
        });
        SourceBody::Content(output)
    }

    fn project_child(&self, parent: AgentId, sid: &str, key: &SourceKey, width: u16,
        expanded: bool, window: std::ops::Range<usize>, path: &mut Vec<AgentId>, dirty: &mut Vec<(AgentId, EntryId)>) -> SourceBody
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
            && !cached.tree.output().has_deferred(window.clone())
        {
            let output = OutputTree::transcript(Rc::clone(&cached.tree), Rc::clone(&cached.indices),
                crate::scrollback::selection_identity::SourceBinding::Session { agent: child.0, epoch: view.session_binding_epoch },
                cached.materialized.iter().filter_map(|id| cached.indices.get(id).copied()).collect());
            return if output.is_empty() { SourceBody::Empty } else { SourceBody::Content(output) };
        }
        path.push(child);
        let changed: Vec<_> = view.inline_sources.dirty_entries.borrow_mut().drain().collect();
        let cached = view.inline_sources.transcript.take();
        let show_thinking = crate::appearance::cache::load_show_thinking_blocks();
        let mut projection = match cached {
            Some(cache) if cache.key.same_child_owner(key)
                && cache.structure == view.scrollback.outline_generation()
                && cache.indices.len() == view.scrollback.len() => Some(cache),
            Some(cache) => {
                for id in cache.materialized {
                    if let Some(entry) = view.scrollback.get_by_id(id) { entry.evict_render_cache(); }
                }
                None
            }
            None => None,
        };
        if let Some(cache) = projection.as_mut() {
            let reset = cache.width != width || cache.key.theme != key.theme
                || cache.key.thinking_visible != key.thinking_visible;
            let invalid: HashSet<_> = changed.into_iter().chain(if reset {
                cache.materialized.iter().copied().collect::<Vec<_>>()
            } else { Vec::new() }).collect();
            for id in invalid {
                if let Some(index) = cache.indices.get(&id).copied()
                    && let Some(entry) = view.scrollback.get_by_id(id)
                    && let Some(old) = cache.tree.part(index)
                    && let Some(tree) = cache.tree.replace(index,
                        OutputTree::deferred(entry.selection_identity(), old.len()))
                {
                    cache.tree = tree;
                    cache.materialized.remove(&id);
                }
            }
            cache.key = key.clone();
            cache.width = width;
        }
        if projection.is_none() {
            let mut indices = std::collections::HashMap::new();
            let mut ids = Vec::new();
            let mut parts = Vec::new();
            let mut estimates = Vec::new();
            for i in 0..view.scrollback.len() {
                let Some(entry) = view.scrollback.entry(i) else { continue };
                entry.bind_selection_source(crate::scrollback::selection_identity::SourceBinding::Session { agent: child.0, epoch: view.session_binding_epoch });
                entry.subscribe_output_changes(&view.inline_sources.signal, &view.inline_sources.dirty_entries);
                indices.insert(entry.id, parts.len());
                ids.push(entry.id);
                let estimate = if entry.is_hidden_thinking(show_thinking) { 0 } else {
                    entry.cached_estimate_lines(width).unwrap_or(1)
                        + 2 * usize::from(entry.block.has_vpad_for(view.scrollback.appearance())) + 1
                };
                estimates.push(estimate.saturating_sub(usize::from(estimate > 0)));
                parts.push(OutputTree::deferred(entry.selection_identity(), estimate));
            }
            let (layouts, groups) = view.scrollback.inline_part_layouts(&estimates);
            for (index, layout) in layouts.iter().enumerate() {
                if let Some(id) = ids.get(index)
                    && let Some(entry) = view.scrollback.get_by_id(*id)
                    && let Some(part) = parts.get_mut(index)
                { *part = OutputTree::deferred(entry.selection_identity(), layout.height + usize::from(layout.gap_after)); }
            }
            projection = EntryTree::build(&parts).map(|tree| ChildProjection {
                key: key.clone(), width, structure: view.scrollback.outline_generation(),
                indices: Rc::new(indices), ids, layouts, groups, materialized: HashSet::new(), tree,
            });
        }
        if let Some(cache) = projection.as_mut() {
            let visible = cache.tree.parts_in(window.clone());
            let keep: HashSet<_> = visible.iter().filter_map(|index| cache.ids.get(*index).copied()).collect();
            let evict: Vec<_> = cache.materialized.difference(&keep).copied().collect();
            for id in evict {
                if let Some(index) = cache.indices.get(&id).copied()
                    && let Some(entry) = view.scrollback.get_by_id(id)
                    && let Some(old) = cache.tree.part(index)
                    && let Some(tree) = cache.tree.replace(index, OutputTree::deferred(entry.selection_identity(), old.len()))
                {
                    cache.tree = tree;
                    cache.materialized.remove(&id);
                    entry.evict_render_cache();
                }
            }
            for index in visible {
                let Some(id) = cache.ids.get(index).copied() else { continue };
                let Some(entry) = view.scrollback.get_by_id(id) else { continue };
                let Some(row) = cache.tree.row_of_part(index) else { continue };
                let local = window.start.saturating_sub(row)..window.end.saturating_sub(row);
                if cache.materialized.contains(&id) && !entry.inline_window_pending(local.clone()) { continue; }
                entry.bind_selection_source(crate::scrollback::selection_identity::SourceBinding::Session { agent: child.0, epoch: view.session_binding_epoch });
                if entry.is_hidden_thinking(show_thinking) { continue; }
                let theme = crate::theme::Theme::current();
                let content_width = width.saturating_sub(crate::scrollback::wrappers::EntryRenderer::new(entry, &theme)
                    .with_appearance_ref(view.scrollback.appearance()).chrome_width_for_inline());
                entry.ensure_cached(content_width, view.scrollback.appearance(), false, view.scrollback.cwd());
                self.project_inline_entry(child, entry, content_width, local, path, dirty);
                let renderer = crate::scrollback::wrappers::EntryRenderer::new(entry, &theme)
                    .with_appearance_ref(view.scrollback.appearance()).with_cwd(view.scrollback.cwd());
                let Some(mut layout) = cache.layouts.get(index).copied() else { continue };
                if layout.height == 0 { continue; }
                if !layout.is_group_header() || layout.is_expanded_verb_header() {
                    layout.height = layout.with_verb_header_row(crate::render::Renderable::desired_height(&renderer, width));
                }
                let header_label = view.scrollback.inline_group_header_label(index, layout, &cache.groups, &theme);
                let part = OutputTree::native(crate::scrollback::entry::inline_output::NativePart {
                    entry: Rc::new(entry.inline_snapshot()), output: entry.cached_tree(),
                    authority: entry.output_authority(), clock: view.scrollback.animation_clock(),
                    signal: entry.output_signal(), revision: entry.output_signal().revision(),
                    binding_lease: Rc::downgrade(&view.inline_sources.signal), appearance: view.scrollback.appearance().clone(),
                    cwd: view.scrollback.cwd().map(std::path::Path::to_path_buf),
                    media_paths: Rc::new(view.media_link_paths.clone()), width,
                    layout, header_label,
                });
                if let Some(tree) = cache.tree.replace(index, part) {
                    cache.tree = tree;
                    cache.materialized.insert(id);
                }
            }
        }
        path.pop();
        let Some(projection) = projection else { return SourceBody::Empty };
        let output = OutputTree::transcript(Rc::clone(&projection.tree), Rc::clone(&projection.indices),
            crate::scrollback::selection_identity::SourceBinding::Session { agent: child.0, epoch: view.session_binding_epoch },
            projection.materialized.iter().filter_map(|id| projection.indices.get(id).copied()).collect());
        view.inline_sources.transcript.replace(Some(projection));
        if output.is_empty() { SourceBody::Empty } else { SourceBody::Content(output) }
    }
}

#[cfg(test)]
#[path = "session_inline_output_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_inline_output_completion_tests.rs"]
mod completion_tests;
