use super::AgentView;
use crate::acp::tracker::AcpUpdateTracker;
use crate::app::agent::AgentId;
use crate::app::app_view::AppView;
use crate::scrollback::state::ScrollbackState;

pub(crate) struct ReplayTranscript {
    pub(crate) scrollback: ScrollbackState,
    pub(crate) tracker: AcpUpdateTracker,
    pub(crate) applied_background_event_ids: std::collections::VecDeque<String>,
}

#[derive(Default)]
pub(crate) enum LoadReconstruction {
    #[default]
    Closed,
    Historical,
    Continuing {
        historical: Box<ReplayTranscript>,
        origin: ContinuationOrigin,
    },
    Continued {
        historical_tracker: Box<AcpUpdateTracker>,
    },
}

pub(crate) enum ContinuationOrigin {
    Fresh,
    Reload {
        original_floor: u64,
        retained: std::collections::HashSet<crate::scrollback::entry::EntryId>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum LoadDelivery {
    Historical,
    Live,
}

pub(crate) enum LoadCompletion {
    Historical,
    Unchanged,
    Continuing {
        live: ReplayTranscript,
        origin: ContinuationOrigin,
    },
    Continued {
        historical_tracker: Box<AcpUpdateTracker>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum LoadedTurn {
    Reconstructed,
    Continued,
}

pub(super) enum ReloadRetarget {
    Retargeted,
    Unchanged,
}

impl LoadCompletion {
    pub(super) fn turn(&self) -> LoadedTurn {
        match self {
            Self::Historical => LoadedTurn::Reconstructed,
            Self::Unchanged | Self::Continuing { .. } | Self::Continued { .. } => {
                LoadedTurn::Continued
            }
        }
    }
}

impl AgentView {
    pub(crate) fn accepts_background_delivery(
        &self,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> bool {
        let ids = if !meta.is_replay
            && matches!(self.load_reconstruction, LoadReconstruction::Historical)
            && let Some(reload) = &self.session_reload
            && reload.binding_epoch == self.session_binding_epoch
        {
            &reload.stash.transcript.applied_background_event_ids
        } else {
            &self.applied_background_event_ids
        };
        meta.event_id.as_ref().is_none_or(|id| !ids.contains(id))
    }

    pub(crate) fn active_transcript_entries(
        &self,
    ) -> std::collections::HashSet<crate::scrollback::entry::EntryId> {
        let mut entries = self.live_admission_tracker().active_projection_entries();
        if let Some(input) = self
            .session
            .in_flight_prompt
            .as_ref()
            .or(self.session.compact_held_prompt.as_ref())
        {
            entries.insert(input.scrollback_entry);
            entries.extend(input.combined_scrollback_entries.iter().copied());
        }
        entries.extend(
            self.session
                .bg_tasks
                .values()
                .filter(|task| task.status == crate::app::agent::BgTaskStatus::Running)
                .filter_map(|task| task.scrollback_entry_id),
        );
        entries
    }

    pub(crate) fn protect_live_entry(&mut self, entry: crate::scrollback::entry::EntryId) {
        if let LoadReconstruction::Continuing {
            origin: ContinuationOrigin::Reload { retained, .. },
            ..
        } = &mut self.load_reconstruction
        {
            retained.insert(entry);
        }
    }

    pub(crate) fn remember_current_prompt_projection(&mut self) {
        let Some(prompt) = self.session.current_prompt_id.as_deref() else {
            return;
        };
        let Some(input) = self
            .session
            .in_flight_prompt
            .as_ref()
            .or(self.session.compact_held_prompt.as_ref())
        else {
            return;
        };
        let mut entries = input.combined_scrollback_entries.clone();
        if !entries.contains(&input.scrollback_entry) {
            entries.push(input.scrollback_entry);
        }
        entries.retain(|entry| self.scrollback.get_by_id(*entry).is_some());
        self.session
            .tracker
            .retain_prompt_projection(prompt, entries);
    }

    pub(crate) fn has_stashed_tool_observation(
        &self,
        update: &agent_client_protocol::SessionUpdate,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> bool {
        !meta.is_replay
            && matches!(self.load_reconstruction, LoadReconstruction::Historical)
            && self.session_reload.as_ref().is_some_and(|reload| {
                reload.binding_epoch == self.session_binding_epoch
                    && reload
                        .stash
                        .transcript
                        .tracker
                        .knows_tool_observation(update, meta)
            })
    }

    pub(crate) fn apply_stashed_tool_observation(
        &mut self,
        update: agent_client_protocol::SessionUpdate,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> bool {
        if !self.has_stashed_tool_observation(&update, meta) {
            return false;
        }
        // Settling an existing entry does not start a live continuation.
        self.swap_stashed_tool_projection();
        let prompt = self.session.current_prompt_id.clone();
        let changed = self.handle_admitted_tool_observation(update, meta, prompt.as_deref());
        self.swap_stashed_tool_projection();
        changed
    }

    fn swap_stashed_tool_projection(&mut self) {
        if let Some(reload) = &mut self.session_reload {
            std::mem::swap(
                &mut self.scrollback,
                &mut reload.stash.transcript.scrollback,
            );
            std::mem::swap(
                &mut self.session.tracker,
                &mut reload.stash.transcript.tracker,
            );
        }
    }

    pub(crate) fn reconstructed_source_frontier(&self, incarnation: uuid::Uuid) -> Option<u64> {
        match &self.load_reconstruction {
            LoadReconstruction::Historical => {
                self.session.tracker.replay_source_frontier(incarnation)
            }
            LoadReconstruction::Continuing { historical, .. } => {
                historical.tracker.replay_source_frontier(incarnation)
            }
            LoadReconstruction::Continued { historical_tracker } => {
                historical_tracker.replay_source_frontier(incarnation)
            }
            LoadReconstruction::Closed => None,
        }
    }

    pub(crate) fn terminal_tool_entry(
        &self,
        update: &agent_client_protocol::ToolCallUpdate,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> Option<crate::scrollback::entry::EntryId> {
        if !matches!(
            meta.tool,
            crate::acp::tool_meta::ToolMetadata::Legacy
                | crate::acp::tool_meta::ToolMetadata::NotTool
        ) {
            return None;
        }
        let scrollback = if meta.is_replay {
            match &self.load_reconstruction {
                LoadReconstruction::Continuing { historical, .. } => &historical.scrollback,
                LoadReconstruction::Closed
                | LoadReconstruction::Historical
                | LoadReconstruction::Continued { .. } => &self.scrollback,
            }
        } else if matches!(self.load_reconstruction, LoadReconstruction::Historical)
            && let Some(reload) = &self.session_reload
            && reload.binding_epoch == self.session_binding_epoch
        {
            &reload.stash.transcript.scrollback
        } else {
            &self.scrollback
        };
        let tracker = self.admission_tracker(meta);
        tracker.drained_tool_entry(update, scrollback).or_else(|| {
            tracker
                .historical_pending_tool_entry(update, meta)
                .filter(|entry| scrollback.get_by_id(*entry).is_some())
        })
    }

    pub(crate) fn apply_terminal_tool_update(
        &mut self,
        update: &agent_client_protocol::ToolCallUpdate,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> bool {
        if self.terminal_tool_entry(update, meta).is_none() {
            return false;
        }
        let apply = |tracker: &mut AcpUpdateTracker, scrollback: &mut ScrollbackState| {
            tracker
                .settle_drained_tool_terminal(update, scrollback)
                .unwrap_or_else(|| {
                    tracker.handle_update(
                        agent_client_protocol::SessionUpdate::ToolCallUpdate(update.clone()),
                        meta,
                        scrollback,
                    )
                })
        };
        if meta.is_replay {
            match &mut self.load_reconstruction {
                LoadReconstruction::Continuing { historical, .. } => {
                    return apply(&mut historical.tracker, &mut historical.scrollback);
                }
                LoadReconstruction::Continued { historical_tracker } => {
                    return apply(historical_tracker, &mut self.scrollback);
                }
                LoadReconstruction::Closed | LoadReconstruction::Historical => {}
            }
        } else if matches!(self.load_reconstruction, LoadReconstruction::Historical)
            && let Some(reload) = &mut self.session_reload
            && reload.binding_epoch == self.session_binding_epoch
        {
            return apply(
                &mut reload.stash.transcript.tracker,
                &mut reload.stash.transcript.scrollback,
            );
        }
        apply(&mut self.session.tracker, &mut self.scrollback)
    }

    pub(crate) fn admission_tracker(
        &self,
        meta: &crate::acp::meta::NotificationMeta,
    ) -> &AcpUpdateTracker {
        if !meta.is_replay {
            return self.live_admission_tracker();
        }
        match &self.load_reconstruction {
            LoadReconstruction::Continuing { historical, .. } => &historical.tracker,
            LoadReconstruction::Continued { historical_tracker } => historical_tracker,
            LoadReconstruction::Closed | LoadReconstruction::Historical => &self.session.tracker,
        }
    }

    pub(crate) fn live_admission_tracker(&self) -> &AcpUpdateTracker {
        if matches!(self.load_reconstruction, LoadReconstruction::Historical)
            && let Some(reload) = self.session_reload.as_ref()
            && reload.binding_epoch == self.session_binding_epoch
        {
            return &reload.stash.transcript.tracker;
        }
        &self.session.tracker
    }

    pub(super) fn retarget_live_reload(&mut self, generation: u64) -> ReloadRetarget {
        let Some(reload) = self.session_reload.as_mut() else {
            return ReloadRetarget::Unchanged;
        };
        if reload.binding_epoch != self.session_binding_epoch {
            return ReloadRetarget::Unchanged;
        }
        match &mut self.load_reconstruction {
            LoadReconstruction::Continuing { historical, .. } => {
                if reload.generation == generation {
                    return ReloadRetarget::Retargeted;
                }
                self.last_seen_event_id = reload.last_seen_event_id.clone();
                self.last_seen_event_seq = reload.last_seen_event_seq;
                let mut scrollback = self.scrollback.fresh_continuation();
                self.loading_placeholder_id = Some(scrollback.push_block(
                    crate::scrollback::block::RenderBlock::system(
                        "Reloading session after reconnect...",
                    ),
                ));
                scrollback.begin_batch();
                **historical = ReplayTranscript {
                    scrollback,
                    tracker: AcpUpdateTracker::sharing_labels(
                        &self.session.tracker.subagent_labels,
                    ),
                    applied_background_event_ids: Default::default(),
                };
                reload.generation = generation;
                reload.saw_replay = false;
                reload.replayed_expiry_notices.clear();
                self.replayed_terminal_prompts.clear();
                self.replayed_visible_prompts.clear();
                self.replayed_bash_prompts.clear();
                self.session.loading_replay = true;
                self.late_replay_until = None;
                self.current_actions_generation = self.current_actions_generation.wrapping_add(1);
                self.retarget_current_actions_load();
                crate::memory_release::release_retained_memory("reload-retarget-history");
                ReloadRetarget::Retargeted
            }
            LoadReconstruction::Closed
            | LoadReconstruction::Historical
            | LoadReconstruction::Continued { .. } => ReloadRetarget::Unchanged,
        }
    }

    pub(super) fn take_replay_transcript(&mut self) -> ReplayTranscript {
        let fresh = self.scrollback.fresh_continuation();
        let tracker = AcpUpdateTracker::sharing_labels(&self.session.tracker.subagent_labels);
        ReplayTranscript {
            scrollback: std::mem::replace(&mut self.scrollback, fresh),
            tracker: std::mem::replace(&mut self.session.tracker, tracker),
            applied_background_event_ids: std::mem::take(&mut self.applied_background_event_ids),
        }
    }

    pub(super) fn restore_replay_transcript(&mut self, mut transcript: ReplayTranscript) {
        transcript
            .scrollback
            .raise_id_floor(self.scrollback.id_floor());
        transcript
            .scrollback
            .raise_invalidation_floor(self.scrollback.invalidation_generations());
        self.scrollback = transcript.scrollback;
        self.session.tracker = transcript.tracker;
        self.applied_background_event_ids = transcript.applied_background_event_ids;
    }

    fn swap_replay_transcript(&mut self, transcript: &mut ReplayTranscript) {
        transcript
            .scrollback
            .raise_id_floor(self.scrollback.id_floor());
        transcript
            .scrollback
            .raise_invalidation_floor(self.scrollback.invalidation_generations());
        std::mem::swap(&mut self.scrollback, &mut transcript.scrollback);
        std::mem::swap(&mut self.session.tracker, &mut transcript.tracker);
        std::mem::swap(
            &mut self.applied_background_event_ids,
            &mut transcript.applied_background_event_ids,
        );
    }

    pub(crate) fn open_live_load_delivery(&mut self) {
        if matches!(self.load_reconstruction, LoadReconstruction::Closed) {
            self.load_reconstruction = LoadReconstruction::Continued {
                historical_tracker: Box::new(AcpUpdateTracker::sharing_labels(
                    &self.session.tracker.subagent_labels,
                )),
            };
            return;
        }
        if !matches!(self.load_reconstruction, LoadReconstruction::Historical) {
            return;
        }
        let retained = self.active_transcript_entries();
        let historical = Box::new(self.take_replay_transcript());
        let origin = if let Some(reload) = self.session_reload.as_mut()
            && reload.binding_epoch == self.session_binding_epoch
        {
            let replacement = ReplayTranscript {
                scrollback: self.scrollback.fresh_continuation(),
                tracker: AcpUpdateTracker::sharing_labels(&self.session.tracker.subagent_labels),
                applied_background_event_ids: Default::default(),
            };
            let live = std::mem::replace(&mut reload.stash.transcript, replacement);
            let original_floor = live.scrollback.id_floor();
            self.restore_replay_transcript(live);
            ContinuationOrigin::Reload {
                original_floor,
                retained,
            }
        } else {
            ContinuationOrigin::Fresh
        };
        self.session.tracker.begin_live_reconstruction();
        self.load_reconstruction = LoadReconstruction::Continuing { historical, origin };
    }

    pub(super) fn take_load_completion(&mut self) -> LoadCompletion {
        self.remember_current_prompt_projection();
        match std::mem::take(&mut self.load_reconstruction) {
            LoadReconstruction::Closed => LoadCompletion::Unchanged,
            LoadReconstruction::Historical => LoadCompletion::Historical,
            LoadReconstruction::Continued { historical_tracker } => {
                LoadCompletion::Continued { historical_tracker }
            }
            LoadReconstruction::Continuing { historical, origin } => {
                let live = self.take_replay_transcript();
                self.restore_replay_transcript(*historical);
                LoadCompletion::Continuing { live, origin }
            }
        }
    }

    pub(super) fn complete_load_transcript(&mut self, completion: LoadCompletion) {
        self.complete_load_transcript_with_replay(completion, true);
    }

    pub(super) fn complete_load_transcript_with_replay(
        &mut self,
        completion: LoadCompletion,
        full_replay: bool,
    ) {
        match completion {
            LoadCompletion::Unchanged => {}
            LoadCompletion::Historical => {
                self.session.finish_turn(&mut self.scrollback);
                self.scrollback.finish_all_running();
                while self.scrollback.in_batch() {
                    self.scrollback.end_batch();
                }
                self.settle_loaded_turn();
            }
            LoadCompletion::Continuing { mut live, origin } => {
                let mut historical_tracker = std::mem::take(&mut self.session.tracker);
                if full_replay {
                    live.tracker.reconcile_rebuilt_tools(
                        &historical_tracker,
                        &mut self.scrollback,
                        &mut live.scrollback,
                    );
                    self.adopt_reconstructed_current_tools(&historical_tracker, &mut live);
                }
                let replacements = historical_tracker.replay_entry_replacements(&live.tracker);
                if full_replay {
                    let tails = live.tracker.replay_stream_tails(
                        &historical_tracker,
                        &replacements,
                        &live.scrollback,
                    );
                    self.scrollback
                        .merge_live_streams(&mut live.scrollback, &replacements, &tails);
                }
                historical_tracker.finish_turn(&mut self.scrollback);
                self.scrollback.finish_all_running();
                while self.scrollback.in_batch() {
                    self.scrollback.end_batch();
                }
                for event_id in live.applied_background_event_ids {
                    self.admit_background_lifecycle_event(Some(&event_id));
                }
                live.scrollback.raise_id_floor(self.scrollback.id_floor());
                match origin {
                    ContinuationOrigin::Fresh => {
                        self.scrollback.merge_live_reconstruction(
                            live.scrollback,
                            replacements,
                            0,
                            std::collections::HashSet::new(),
                        );
                    }
                    ContinuationOrigin::Reload {
                        original_floor,
                        retained,
                    } if full_replay => {
                        self.scrollback.merge_live_reconstruction(
                            live.scrollback,
                            replacements,
                            original_floor,
                            retained,
                        );
                    }
                    ContinuationOrigin::Reload { .. } => {
                        let historical = std::mem::replace(&mut self.scrollback, live.scrollback);
                        self.scrollback.raise_id_floor(historical.id_floor());
                        self.scrollback.append_entries_from(historical);
                    }
                }
                live.tracker.end_live_reconstruction();
                self.session.tracker = live.tracker;
                self.load_reconstruction = LoadReconstruction::Continued {
                    historical_tracker: Box::new(historical_tracker),
                };
            }
            LoadCompletion::Continued { historical_tracker } => {
                self.load_reconstruction = LoadReconstruction::Continued { historical_tracker };
            }
        }
    }

    pub(crate) fn finish_loaded_reconstruction(&mut self) -> LoadedTurn {
        let completion = self.take_load_completion();
        let turn = completion.turn();
        self.complete_load_transcript(completion);
        self.session.loading_replay = false;
        turn
    }
}

enum DeliveryReturn {
    Direct,
    Reconstructing {
        live: Box<ReplayTranscript>,
        origin: ContinuationOrigin,
    },
    Settled {
        live_tracker: Box<AcpUpdateTracker>,
    },
}

/// The caller classifies delivery after owner admission; historical handlers must not mutate live turn fields.
pub(crate) fn with_load_delivery<T>(
    app: &mut AppView,
    agent_id: AgentId,
    delivery: LoadDelivery,
    apply: impl FnOnce(&mut AppView) -> T,
) -> T {
    let Some(view) = app.agents.get_mut(&agent_id) else {
        return apply(app);
    };
    let binding_epoch = view.session_binding_epoch;
    let generation = view.session_reload.as_ref().map(|reload| reload.generation);
    let reconstruction = std::mem::take(&mut view.load_reconstruction);
    let returning = match (delivery, reconstruction) {
        (LoadDelivery::Live, LoadReconstruction::Historical) => {
            view.load_reconstruction = LoadReconstruction::Historical;
            view.open_live_load_delivery();
            DeliveryReturn::Direct
        }
        (LoadDelivery::Live, LoadReconstruction::Closed) => {
            view.open_live_load_delivery();
            DeliveryReturn::Direct
        }
        (
            LoadDelivery::Live,
            reconstruction @ (LoadReconstruction::Continuing { .. }
            | LoadReconstruction::Continued { .. }),
        ) => {
            view.load_reconstruction = reconstruction;
            DeliveryReturn::Direct
        }
        (
            LoadDelivery::Historical,
            LoadReconstruction::Continuing {
                mut historical,
                origin,
            },
        ) => {
            view.swap_replay_transcript(&mut historical);
            DeliveryReturn::Reconstructing {
                live: historical,
                origin,
            }
        }
        (
            LoadDelivery::Historical,
            LoadReconstruction::Continued {
                mut historical_tracker,
            },
        ) => {
            std::mem::swap(&mut view.session.tracker, &mut *historical_tracker);
            DeliveryReturn::Settled {
                live_tracker: historical_tracker,
            }
        }
        (LoadDelivery::Historical, LoadReconstruction::Closed) => {
            let mut historical_tracker = Box::new(AcpUpdateTracker::sharing_labels(
                &view.session.tracker.subagent_labels,
            ));
            std::mem::swap(&mut view.session.tracker, &mut *historical_tracker);
            DeliveryReturn::Settled {
                live_tracker: historical_tracker,
            }
        }
        (LoadDelivery::Historical, reconstruction @ LoadReconstruction::Historical) => {
            view.load_reconstruction = reconstruction;
            DeliveryReturn::Direct
        }
    };
    let loading = view.session.loading_replay;
    view.session.loading_replay = matches!(delivery, LoadDelivery::Historical);
    let result = apply(app);
    if let Some(view) = app.agents.get_mut(&agent_id)
        && view.session_binding_epoch == binding_epoch
        && view.session_reload.as_ref().map(|reload| reload.generation) == generation
    {
        match returning {
            DeliveryReturn::Direct => {}
            DeliveryReturn::Reconstructing { mut live, origin } => {
                view.swap_replay_transcript(&mut live);
                view.load_reconstruction = LoadReconstruction::Continuing {
                    historical: live,
                    origin,
                };
            }
            DeliveryReturn::Settled { mut live_tracker } => {
                std::mem::swap(&mut view.session.tracker, &mut *live_tracker);
                view.load_reconstruction = LoadReconstruction::Continued {
                    historical_tracker: live_tracker,
                };
            }
        }
        view.session.loading_replay = loading;
    }
    result
}
