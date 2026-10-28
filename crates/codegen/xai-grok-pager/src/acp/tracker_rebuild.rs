use super::*;
use std::hash::{Hash, Hasher};

impl AcpUpdateTracker {
    pub(crate) fn reconcile_rebuilt_tools(&mut self, historical: &Self, history: &mut ScrollbackState, live: &mut ScrollbackState) {
        let replacements = historical.replay_entry_replacements(self);
        let mut targets: HashMap<_, _> = historical.tool_instances.entries.keys().map(|entry| {
            let target = replacements.get(entry).and_then(|entries| entries.iter()
                .filter_map(|id| live.index_of_id(*id).map(|index| (index, *id))).min_by_key(|(index, _)| *index))
                .map_or(*entry, |(_, entry)| entry);
            (*entry, target)
        }).collect();
        for (entry, context) in &historical.tool_context_ids {
            let target = targets.get(entry).copied().unwrap_or(*entry);
            self.tool_context_ids.entry(target).or_insert_with(|| context.clone());
        }
        let mut timings: HashMap<Uuid, Vec<ToolExecutionTiming>> = HashMap::new();
        for (execution, invocation) in &historical.tool_instances.executions {
            if let Some(call) = historical.tool_instances.calls.get(invocation)
                && let Some(timing) = call.clock.timing(*execution) {
                timings.entry(*invocation).or_default().push(timing);
            }
        }
        for (invocation, past) in &historical.tool_instances.calls {
            if self.tool_instances.calls.get(invocation).is_some_and(|current| current.identity != past.identity) { continue; }
            let existed = self.tool_instances.calls.contains_key(invocation);
            let clock = self.register_tool_identity(&past.identity);
            for task in &past.background_tasks { self.bind_background_tool(&past.identity, task, true); }
            if !existed { clock.close(); self.tool_instances.open.remove(invocation); }
            for timing in timings.get(invocation).into_iter().flatten() {
                let execution = match timing {
                    ToolExecutionTiming::Unknown => continue,
                    ToolExecutionTiming::Started { execution_id, .. } | ToolExecutionTiming::Finished { execution_id, .. } => *execution_id,
                };
                if self.tool_instances.executions.get(&execution).is_some_and(|owner| owner != invocation) { continue; }
                if clock.accepts(*timing) {
                    clock.apply(*timing, ToolClockAdmission::Unconfirmed);
                    self.tool_instances.executions.insert(execution, *invocation);
                }
            }
            let key = ToolKey::Registered(*invocation);
            if existed && matches!(self.tool_instances.calls.get(invocation).map(|call| &call.projection), Some(ToolProjection::AwaitingBase))
                && self.orphan_updates.contains_key(&key)
                && let Some(base) = historical.pending_tools.get(&key).map(|pending| pending.base.clone()) {
                let meta = NotificationMeta { is_replay: true, tool: ToolMetadata::Owned {
                    identity: past.identity.clone(), timing: ToolExecutionTiming::Unknown,
                }, ..Default::default() };
                self.handle_tool_observation(acp::SessionUpdate::ToolCall(base), &meta, live, None);
                if let (ToolProjection::Entry(history), Some(current)) = (&past.projection, self.registered_tool_entry(*invocation)) {
                    targets.insert(*history, current);
                    if *history != current { self.tool_context_ids.remove(history); }
                }
            }
            let projection = match past.projection {
                ToolProjection::Entry(entry) => ToolProjection::Entry(targets.get(&entry).copied().unwrap_or(entry)),
                ToolProjection::Hidden => ToolProjection::Hidden,
                ToolProjection::AwaitingBase => ToolProjection::AwaitingBase,
            };
            if past.clock.is_terminal() {
                clock.settle();
                self.pending_tools.remove(&key);
                self.suppressed_tools.remove(&key);
                self.blocking_waits.remove(&key);
                self.tool_instances.open.remove(invocation);
                if let (ToolProjection::Entry(past_entry), ToolProjection::Entry(entry)) = (&past.projection, &projection)
                    && let Some(past_entry) = history.get_by_id(*past_entry) {
                    let completion = past_entry.tool_completion();
                    if live.get_by_id(*entry).is_some_and(|entry| entry.tool_completion() == ToolCompletion::Unconfirmed) {
                        live.set_tool_completion(*entry, completion);
                        live.finish_running(*entry);
                    }
                }
            }
            if let ToolProjection::Entry(target) = projection {
                if let Some(previous) = self.registered_tool_entry(*invocation) && previous != target {
                    self.merge_tool_projections(target, previous);
                    self.tool_context_ids.remove(&previous);
                    self.pending_edit_hl.retain(|entry| *entry != previous);
                }
                self.note_tool_projection(&key, ToolProjection::Entry(target));
            } else if !existed { self.note_tool_projection(&key, projection); }
        }
        for (history_entry, target) in targets {
            let ids = self.tool_instances.entries.get(&target).cloned().unwrap_or_default();
            let mut clock = live.get_by_id(target).and_then(|entry| if let RenderBlock::ToolCall(tool) = &entry.block {
                Some(tool.execution_clock().clone())
            } else { None }).or_else(|| ids.first().and_then(|id| self.tool_instances.calls.get(id).map(|call| call.clock.clone())));
            if let Some(current) = &mut clock {
                for invocation in &ids {
                    if let Some(call) = self.tool_instances.calls.get(invocation)
                        && current.is_terminal() && call.clock.is_terminal() {
                        *current = current.merge_completed(&call.clock);
                    }
                }
            }
            let Some(historical_entry) = history.get_by_id_mut(history_entry) else { continue; };
            if let (Some(current), RenderBlock::ToolCall(tool)) = (&clock, &mut historical_entry.block) { tool.set_execution_clock(current.clone()); }
            let Some(entry) = live.get_by_id_mut(target) else { continue; };
            if let (RenderBlock::ToolCall(current), RenderBlock::ToolCall(past)) = (&mut entry.block, &historical_entry.block) {
                if let (ToolCallBlock::Edit(current), ToolCallBlock::Edit(past)) = (&mut *current, past) && current.path == past.path {
                    let mut hunks = Vec::new();
                    let mut seen: HashMap<u64, Vec<usize>> = HashMap::new();
                    for hunk in past.hunks.iter().chain(&current.hunks) {
                        let mut hash = std::collections::hash_map::DefaultHasher::new();
                        for line in hunk { (line.lo, line.ln, std::mem::discriminant(&line.tag), &line.text).hash(&mut hash); }
                        let key = hash.finish();
                        if seen.get(&key).is_some_and(|indices| indices.iter().any(|index| hunks.get(*index) == Some(hunk))) { continue; }
                        seen.entry(key).or_default().push(hunks.len());
                        hunks.push(hunk.clone());
                    }
                    let edit_count = ids.len().max(current.edit_count).max(past.edit_count);
                    let hunks = xai_grok_pager_diff::stitch_overlapping_hunks(hunks);
                    if current.hunks != hunks {
                        current.set_hunks(hunks);
                        current.highlight = EditHighlightPhase::HunkOnly;
                        if !self.pending_edit_hl.contains(&target) { self.pending_edit_hl.push(target); }
                    }
                    current.edit_count = edit_count;
                    if let Some(clock) = &clock { current.execution_clock = clock.clone(); }
                } else {
                    if historical.tool_instances.data_frontiers.get(&history_entry).is_some_and(|past|
                        self.tool_instances.data_frontiers.get(&target).is_none_or(|current| current <= past)) {
                        let clock = clock.clone().unwrap_or_else(|| current.execution_clock().clone());
                        *current = past.clone();
                        current.set_execution_clock(clock);
                    }
                }
            }
            entry.invalidate_cache();
            live.mark_structurally_dirty(target);
        }
    }
}
