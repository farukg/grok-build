use super::*;

#[derive(Debug, Default)]
pub(super) enum UserProjection {
    #[default]
    Absent,
    AwaitingOwner(Vec<EntryId>),
    Bound { prompt: String },
}

pub(super) enum TranscriptUpdate { Message, Thought, User, Other }

impl TranscriptUpdate {
    pub(super) fn classify(update: &acp::SessionUpdate) -> Self {
        match update {
            acp::SessionUpdate::AgentMessageChunk(_) => Self::Message,
            acp::SessionUpdate::AgentThoughtChunk(_) => Self::Thought,
            acp::SessionUpdate::UserMessageChunk(_) => Self::User,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Default)]
pub(super) enum LiveStreamReconstruction {
    #[default]
    Inactive,
    Active(HashMap<EntryId, StreamAppends>),
}

#[derive(Debug)]
pub(super) enum StreamBase { New, Through(u64) }

#[derive(Debug)]
pub(super) enum StreamAppends {
    Correlated { base: StreamBase, appends: Vec<(u64, usize)> },
    Uncorrelated,
}

impl AcpUpdateTracker {
    pub(crate) fn begin_live_reconstruction(&mut self) {
        if matches!(self.live_stream_reconstruction, LiveStreamReconstruction::Active(_)) { return; }
        let entries = self.current_agent_msg.into_iter().chain(self.current_thinking).map(|entry| {
            let state = match self.stream_frontiers.get(&entry) {
                Some(sequence) => StreamAppends::Correlated { base: StreamBase::Through(*sequence), appends: Vec::new() },
                None => StreamAppends::Uncorrelated,
            };
            (entry, state)
        }).collect();
        self.live_stream_reconstruction = LiveStreamReconstruction::Active(entries);
    }

    pub(crate) fn end_live_reconstruction(&mut self) { self.live_stream_reconstruction = LiveStreamReconstruction::Inactive; }

    pub(crate) fn note_replayed_source_frontier(&mut self, meta: &NotificationMeta) {
        if meta.is_replay && let Some(sequence) = meta.event_seq
            && let crate::acp::meta::CurrentSourceMetadata::Version(version) = meta.current_actions_version {
            self.replay_source_frontiers.entry(version.incarnation).and_modify(|old| *old = (*old).max(sequence)).or_insert(sequence);
        }
    }

    pub(crate) fn replay_source_frontier(&self, incarnation: uuid::Uuid) -> Option<u64> {
        self.replay_source_frontiers.get(&incarnation).copied()
    }

    pub(super) fn note_stream_append(&mut self, entry: EntryId, meta: &NotificationMeta, offset: Option<usize>) {
        if let Some(sequence) = meta.event_seq { self.stream_frontiers.entry(entry).and_modify(|old| *old = (*old).max(sequence)).or_insert(sequence); }
        if meta.is_replay { return; }
        let LiveStreamReconstruction::Active(entries) = &mut self.live_stream_reconstruction else { return; };
        let Some((sequence, offset)) = meta.event_seq.zip(offset) else { entries.insert(entry, StreamAppends::Uncorrelated); return; };
        let state = entries.entry(entry).or_insert_with(|| StreamAppends::Correlated { base: StreamBase::New, appends: Vec::new() });
        match state {
            StreamAppends::Correlated { appends, .. } => appends.push((sequence, offset)),
            StreamAppends::Uncorrelated => {}
        }
    }

    pub(crate) fn replay_stream_tails(&self, historical: &Self, replacements: &HashMap<EntryId, Vec<EntryId>>, scrollback: &ScrollbackState) -> HashMap<EntryId, usize> {
        let mut tails = HashMap::new();
        for (history, current) in replacements {
            let Some(frontier) = historical.stream_frontiers.get(history) else { continue; };
            for entry in current {
                if self.stream_frontiers.get(entry).is_some_and(|sequence| sequence <= frontier) {
                    if let Some(offset) = scrollback.stream_source_len(*entry) { tails.insert(*entry, offset); }
                    continue;
                }
                if let LiveStreamReconstruction::Active(entries) = &self.live_stream_reconstruction
                    && let Some(StreamAppends::Correlated { base, appends }) = entries.get(entry) {
                    if matches!(base, StreamBase::Through(sequence) if sequence > frontier) { continue; }
                    if let Some((_, offset)) = appends.iter().find(|(sequence, _)| sequence > frontier) { tails.insert(*entry, *offset); }
                }
            }
        }
        tails
    }

    pub(crate) fn retain_prompt_projection(&mut self, prompt: &str, entries: Vec<EntryId>) {
        if !entries.is_empty() { self.prompt_entries.insert(prompt.to_owned(), entries); }
    }

    pub(super) fn bind_user_projection(&mut self, prompt: &str) {
        match std::mem::take(&mut self.user_projection) {
            UserProjection::AwaitingOwner(entries) => {
                self.prompt_entries.insert(prompt.to_owned(), entries);
                self.user_projection = UserProjection::Bound { prompt: prompt.to_owned() };
            }
            UserProjection::Bound { prompt: previous } if previous == prompt => {
                self.user_projection = UserProjection::Bound { prompt: previous };
            }
            UserProjection::Absent | UserProjection::Bound { .. } => {}
        }
    }

    fn note_user_projection(&mut self, meta: &NotificationMeta, entries: Vec<EntryId>) {
        if entries.is_empty() { return; }
        self.user_projection = UserProjection::AwaitingOwner(entries);
        if let Some(prompt) = &meta.prompt_id { self.bind_user_projection(prompt); }
    }

    pub(super) fn note_optimistic_user_echo(&mut self, meta: &NotificationMeta, count: usize, scrollback: &ScrollbackState) {
        let mut entries: Vec<_> = (0..scrollback.len()).rev().filter_map(|index| scrollback.get(index))
            .filter(|entry| matches!(&entry.block, RenderBlock::UserPrompt(block) if !block.is_interjection))
            .take(count).map(|entry| entry.id).collect();
        entries.reverse();
        if let Some(event) = &meta.event_id {
            for entry in &entries { self.transcript_events.entry(*entry).or_insert_with(|| event.clone()); }
        }
        self.note_user_projection(meta, entries);
    }

    pub(super) fn record_transcript_update(&mut self, kind: TranscriptUpdate, meta: &NotificationMeta, floor: u64, changed: bool, scrollback: &ScrollbackState) {
        if !changed { return; }
        let entries = match kind {
            TranscriptUpdate::Message => self.current_agent_msg.into_iter().collect(),
            TranscriptUpdate::Thought => self.current_thinking.into_iter().collect(),
            TranscriptUpdate::User | TranscriptUpdate::Other => scrollback.entry_ids_since(floor),
        };
        if let Some(event) = &meta.event_id {
            for entry in &entries { self.transcript_events.entry(*entry).or_insert_with(|| event.clone()); }
        }
        if matches!(kind, TranscriptUpdate::User) {
            let prompts = entries.into_iter().filter(|id| scrollback.get_by_id(*id)
                .is_some_and(|entry| matches!(&entry.block, RenderBlock::UserPrompt(block) if !block.is_interjection))).collect();
            self.note_user_projection(meta, prompts);
        }
    }
}
