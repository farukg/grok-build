use super::{AgentView, CurrentActionsContext};
use xai_grok_shell::extensions::current_actions::{DeliveryRole, GenerationEnd, LoadCut};
use xai_grok_shell::session::current_actions::{CurrentActions, SnapshotBinding};

#[derive(Default)]
pub(crate) enum PublicationLoad {
    #[default]
    Closed,
    Awaiting {
        binding: SnapshotBinding,
    },
    Marked {
        binding: SnapshotBinding,
        end: GenerationEnd,
    },
}

impl AgentView {
    pub(crate) fn accept_interjection_projection(&mut self, id: &str) -> bool {
        if !self.session.tracker.accepted_interjection(id) {
            return false;
        }
        self.self_interjection_ids.remove(id);
        self.interjection_painted_blocks.remove(id);
        self.interjection_retry_images.remove(id);
        self.session.retire_interjection_retry(id)
    }


    pub(crate) fn begin_publication_load(&mut self) -> Option<SnapshotBinding> {
        let binding = self.current_actions_binding()?;
        self.publication_load = PublicationLoad::Awaiting {
            binding: binding.clone(),
        };
        Some(binding)
    }

    pub(crate) fn receive_generation_end(&mut self, end: GenerationEnd) -> bool {
        if self.current_actions_binding().as_ref() != Some(&end.snapshot.binding) {
            return false;
        }
        match &mut self.publication_load {
            PublicationLoad::Awaiting { binding } if *binding == end.snapshot.binding => {
                self.publication_load = PublicationLoad::Marked {
                    binding: binding.clone(),
                    end,
                };
                true
            }
            PublicationLoad::Marked {
                binding,
                end: known,
            } => *binding == end.snapshot.binding && *known == end,
            PublicationLoad::Closed | PublicationLoad::Awaiting { .. } => false,
        }
    }

    pub(crate) fn complete_publication_load(
        &mut self,
        binding: &SnapshotBinding,
        snapshot: &CurrentActions,
        cut: &LoadCut,
    ) -> bool {
        if self.current_actions_binding().as_ref() != Some(binding)
            || self.session.loading_replay
            || self.session_reload.is_some()
        {
            return false;
        }
        let matches = match &self.publication_load {
            PublicationLoad::Marked {
                binding: original,
                end,
            } => {
                original == binding
                    && snapshot.binding == *binding
                    && end.snapshot == *snapshot
                    && matches!(cut, LoadCut::Settled)
            }
            PublicationLoad::Closed | PublicationLoad::Awaiting { .. } => false,
        };
        self.publication_load = PublicationLoad::Closed;
        if !matches || !self.has_current_actions_capture(snapshot) {
            return false;
        }
        let context = CurrentActionsContext {
            binding: binding.clone(),
            incarnation: snapshot.version.incarnation,
        };
        self.finish_current_actions_replay(&context, snapshot.event_frontier)
    }

    pub(crate) fn accepts_interjection_publication(&self, meta: &crate::acp::meta::NotificationMeta) -> bool {
        if !self.accepts_current_source(meta) {
            return false;
        }
        match &meta.delivery_role {
            DeliveryRole::Live { context } => {
                self.current_actions_binding().as_ref() == Some(&context.binding)
                    && matches!(meta.current_actions_version, crate::acp::meta::CurrentSourceMetadata::Version(version)
                        if version.incarnation == context.incarnation)
            }
            DeliveryRole::Historical { binding } => {
                meta.is_replay && self.current_actions_binding().as_ref() == Some(binding)
            }
            DeliveryRole::Unavailable => {
                self.current_actions_context().is_none()
                    && matches!(meta.current_actions_version, crate::acp::meta::CurrentSourceMetadata::Legacy)
            }
            DeliveryRole::LiveUnbound { .. } => false,
        }
    }

    pub(crate) fn accepts_publication_role(&self, role: &DeliveryRole) -> bool {
        match role {
            DeliveryRole::Unavailable => false,
            DeliveryRole::Historical { binding } => {
                self.current_actions_binding().as_ref() == Some(binding)
            }
            DeliveryRole::Live { context } => {
                self.current_actions_binding().as_ref() == Some(&context.binding)
                    && self
                        .current_actions_context()
                        .is_some_and(|known| known.incarnation == context.incarnation)
            }
            DeliveryRole::LiveUnbound {
                session_id,
                incarnation,
            } => self.current_actions_context().is_some_and(|known| {
                known.binding.session_id == *session_id && known.incarnation == *incarnation
            }),
        }
    }
}
