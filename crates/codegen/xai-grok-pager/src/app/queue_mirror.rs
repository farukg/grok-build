use super::AppView;
use crate::app::agent_view::queue::QueueTransport;
use xai_grok_shell::extensions::interject::{QueueOperation, QueueResponse};

impl AppView {
    pub(crate) fn apply_queue_send_now_reply(
        &mut self,
        operation: QueueOperation,
        binding_epoch: u64,
        result: Result<QueueResponse, QueueTransport>,
    ) {
        if let Some(agent_id) = self.agents.find_by_session_id(&operation.session_id)
            && let Some(agent) = self.agents.get_mut(&agent_id)
        {
            let session_id = operation.session_id.clone();
            if let Some(row) = agent.apply_queue_send_now_result(operation, binding_epoch, result) {
                self.prepare_queue_mirror_binding(&session_id);
                let queue = self.shared_prompt_queues.entry(session_id).or_default();
                match queue.iter_mut().find(|current| current.id == row.id) {
                    Some(current) => { *current = row; }
                    None => queue.push(row),
                }
            }
        }
    }

    /// Reconcile the shared prompt queue for a session from a `x.ai/queue/changed` broadcast.
    /// Returns `(old_id, new_id)` for echoes retired via the kind-and-text fallback (re-keyed: the old id never appears in any broadcast).
    /// The caller routes these through `AgentView::note_queue_echo_rekeyed` so per-agent state moves with the message instead of leaking.
    pub fn apply_queue_changed(
        &mut self,
        changed: crate::app::prompt_queue::QueueChanged,
    ) -> Vec<(String, String)> {
        let crate::app::prompt_queue::QueueChanged {
            session_id,
            mut entries,
            running_prompt_id,
            running_text: _,
            running_kind: _,
            running_combined_texts: _,
        } = changed;
        self.prepare_queue_mirror_binding(&session_id);
        entries.retain(|row| running_prompt_id.as_deref() != Some(row.id.as_str()));
        if let Some(previous) = self.shared_prompt_queues.get(&session_id) {
            let versions: std::collections::HashMap<_, _> = previous.iter()
                .map(|row| (row.id.as_str(), row)).collect();
            for row in &mut entries {
                if let Some(known) = versions.get(row.id.as_str())
                    && known.version > row.version
                {
                    let position = row.position;
                    *row = (*known).clone();
                    row.position = position;
                }
            }
        }
        let mut rekeyed_echo_ids: Vec<(String, String)> = Vec::new();
        let running_row: Option<(String, String)> = running_prompt_id.as_ref().and_then(|pid| {
            self.shared_prompt_queues
                .get(&session_id)
                .and_then(|q| q.iter().find(|e| &e.id == pid))
                .map(|e| (e.kind.clone(), e.text.clone()))
        });
        if let Some(opt) = self.optimistic_prompt_echoes.get_mut(&session_id) {
            opt.retain(|e| {
                let id_matches_running = running_prompt_id.as_deref() == Some(e.id.as_str());
                let id_matches_entry = entries.iter().any(|x| x.id == e.id);
                let content_match_id = running_row
                    .as_ref()
                    .filter(|(kind, text)| *kind == e.kind && *text == e.text)
                    .and_then(|_| running_prompt_id.clone())
                    .or_else(|| {
                        entries
                            .iter()
                            .find(|x| x.kind == e.kind && x.text == e.text)
                            .map(|x| x.id.clone())
                    });
                let retired = id_matches_running || id_matches_entry || content_match_id.is_some();
                if retired
                    && !id_matches_running
                    && !id_matches_entry
                    && let Some(new_id) = content_match_id
                {
                    rekeyed_echo_ids.push((e.id.clone(), new_id));
                }
                !retired
            });
            for e in opt.iter() {
                if !entries.iter().any(|x| x.id == e.id) {
                    let mut pinned = e.clone();
                    pinned.position = entries.len();
                    entries.push(pinned);
                }
            }
            if opt.is_empty() {
                self.optimistic_prompt_echoes.remove(&session_id);
            }
        }
        if entries.is_empty() {
            self.shared_prompt_queues.remove(&session_id);
            self.queue_mirror_bindings.remove(&session_id);
        } else {
            self.shared_prompt_queues.insert(session_id, entries);
        }
        rekeyed_echo_ids
    }

    fn prepare_queue_mirror_binding(&mut self, session_id: &str) {
        let binding = self.agents.find_by_session_id(session_id)
            .and_then(|id| self.agents.get(&id).map(|agent| (id, agent.session_binding_epoch)));
        if self.queue_mirror_bindings.get(session_id).is_some_and(|previous| *previous != binding) {
            self.shared_prompt_queues.remove(session_id);
            self.optimistic_prompt_echoes.remove(session_id);
        }
        self.queue_mirror_bindings.insert(session_id.to_owned(), binding);
    }
    /// Push an optimistic echo row for a server-authoritative prompt the pager just sent.
    /// (A plain prompt or agent-bound kind typed while a turn is running.)
    /// The row is keyed by `prompt_id` so the authoritative `x.ai/queue/changed` broadcast replaces it (matched by `id`) rather than duplicating it.
    pub fn push_optimistic_prompt_echo(
        &mut self,
        session_id: &str,
        prompt_id: &str,
        text: &str,
        kind: &str,
    ) {
        self.prepare_queue_mirror_binding(session_id);
        let entry = crate::app::prompt_queue::QueueEntryWire {
            id: prompt_id.to_string(),
            version: 0,
            owner: None,
            last_editor: None,
            kind: kind.to_string(),
            text: text.to_string(),
            combined_texts: None,
            position: 0,
        };
        let opt = self
            .optimistic_prompt_echoes
            .entry(session_id.to_string())
            .or_default();
        if !opt.iter().any(|e| e.id == entry.id) {
            opt.push(entry.clone());
        }
        let shared = self
            .shared_prompt_queues
            .entry(session_id.to_string())
            .or_default();
        if !shared.iter().any(|e| e.id == entry.id) {
            let mut e = entry;
            e.position = shared.len();
            shared.push(e);
        }
    }
    /// The shared (server-authoritative) prompt queue for a session, if any.
    pub fn shared_prompt_queue(
        &self,
        session_id: &str,
    ) -> Option<&Vec<crate::app::prompt_queue::QueueEntryWire>> {
        self.shared_prompt_queues.get(session_id)
    }
}
