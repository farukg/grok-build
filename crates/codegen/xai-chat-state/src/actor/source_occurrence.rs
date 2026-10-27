use xai_grok_sampling_types::{ConversationItem, SourceOccurrence, SourceOccurrenceId};

pub(super) enum IdentityNormalization {
    Current,
    RewritePending,
}

impl super::ChatStateActor {
    pub(super) fn persist_source_normalization(&mut self) {
        match self.state.identity_normalization {
            IdentityNormalization::Current => {}
            IdentityNormalization::RewritePending => {
                self.persistence.replace_history(&self.state.conversation);
                self.state.identity_normalization = IdentityNormalization::Current;
            }
        }
    }
}

pub(super) fn admit(item: &mut ConversationItem) {
    *item.source_mut() = SourceOccurrence::Admitted { id: next_id() };
}

#[derive(Clone, Copy)]
pub(super) enum Normalization {
    Legacy,
    Admission,
}

pub(super) fn normalize(items: &mut [ConversationItem], normalization: Normalization) -> usize {
    let mut assigned = 0;
    for item in items {
        if item.source().is_unadmitted() {
            *item.source_mut() = match normalization {
                Normalization::Legacy => SourceOccurrence::LegacyNormalized { id: next_id() },
                Normalization::Admission => SourceOccurrence::Admitted { id: next_id() },
            };
            assigned += 1;
        }
    }
    assigned
}

fn next_id() -> SourceOccurrenceId {
    SourceOccurrenceId(*uuid::Uuid::new_v4().as_bytes())
}

/// Called before persistence, while both sides of the actual replacement are owned.
pub(super) fn record_compaction_replacement(old: &[ConversationItem], new: &mut [ConversationItem]) {
    use std::collections::HashSet;
    use xai_grok_sampling_types::{CompactionCoverage, CoverageKnowledge, SyntheticReason};
    let retained: HashSet<_> = new.iter().filter_map(|item| item.source().id()).collect();
    let mut sources = Vec::new();
    let mut inherited = Vec::new();
    let mut seen = HashSet::new();
    let mut unidentified = 0;
    let mut ancestry = CoverageKnowledge::Complete;
    for item in old {
        if item.source().id().is_some_and(|id| retained.contains(&id)) {
            continue;
        }
        match item.source().id() {
            Some(id) => sources.push(id),
            None => unidentified += 1,
        }
        match item.evidence().compaction() {
            CompactionCoverage::Replaced { sources, inherited: prior, unidentified: missing, ancestry: knowledge } => {
                for id in sources.iter().chain(prior) {
                    if seen.insert(*id) { inherited.push(*id); }
                }
                if *missing > 0 || *knowledge == CoverageKnowledge::Unknown {
                    ancestry = CoverageKnowledge::Unknown;
                }
            }
            CompactionCoverage::Unknown => {
                if matches!(item, ConversationItem::User(user) if user.synthetic_reason == SyntheticReason::CompactionMeta) {
                    ancestry = CoverageKnowledge::Unknown;
                }
            }
            CompactionCoverage::AwaitingReplacement => ancestry = CoverageKnowledge::Unknown,
        }
    }
    for item in new {
        if *item.evidence().compaction() == CompactionCoverage::AwaitingReplacement {
            item.evidence_mut().set_compaction(CompactionCoverage::Replaced {
                sources: sources.clone(), inherited: inherited.clone(), unidentified, ancestry,
            });
        }
    }
}
