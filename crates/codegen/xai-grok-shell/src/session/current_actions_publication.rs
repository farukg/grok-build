use super::{CurrentActions, CurrentActionsSnapshot, Reader, SnapshotBinding, Source};
use crate::extensions::current_actions::{
    CutUnavailable, DeliveryRole, GenerationEnd, PublicationContext, GENERATION_END_METHOD,
};

pub(crate) struct PreparedCut {
    snapshot: CurrentActions,
    source: Source,
}

impl Source {
    pub(crate) fn attach_replay_queue(
        &self,
        queue: tokio::sync::mpsc::UnboundedSender<crate::session::replay_events::SessionEvent>,
    ) {
        let _ = self.0.replay_queue.set(queue);
    }

    pub(crate) fn begin_publication(&self) -> Option<xai_acp_lib::PublicationGuard> {
        self.0.emitter.get().map(|emitter| emitter.gateway.begin_publication())
    }

    pub(crate) fn delivery_role(&self) -> DeliveryRole {
        let role = self.0.delivery.lock().clone();
        match role {
            DeliveryRole::Unavailable => match (self.0.emitter.get(), self.version()) {
                (Some(emitter), Some(version)) => DeliveryRole::LiveUnbound {
                    session_id: emitter.session_id.0.to_string(), incarnation: version.incarnation,
                },
                (Some(_), None) | (None, Some(_)) | (None, None) => DeliveryRole::Unavailable,
            },
            DeliveryRole::Live { .. } | DeliveryRole::LiveUnbound { .. } | DeliveryRole::Historical { .. } => role,
        }
    }

    pub(crate) fn stamp_delivery(&self, meta: &mut agent_client_protocol::Meta) {
        if let Err(error) = crate::extensions::current_actions::stamp_delivery(meta, &self.delivery_role()) {
            if let Some(emitter) = self.0.emitter.get() {
                emitter.gateway.fail_publication();
            }
            tracing::error!(%error, "current-actions delivery serialization failed");
        }
    }
}

impl Reader {
    pub(crate) fn bind_publication(&self, binding: SnapshotBinding) -> Result<PublicationContext, CutUnavailable> {
        if binding.session_id != self.session_id {
            return Err(CutUnavailable::SourceUnavailable);
        }
        let state = self.source.lock();
        let version = state.version.ok_or(CutUnavailable::SourceUnavailable)?;
        let context = PublicationContext { binding, incarnation: version.incarnation };
        *self.source.0.delivery.lock() = DeliveryRole::Live { context: context.clone() };
        Ok(context)
    }

    pub(crate) fn publication_context(&self) -> Option<PublicationContext> {
        match self.source.delivery_role() {
            DeliveryRole::Live { context } => Some(context),
            DeliveryRole::Unavailable | DeliveryRole::LiveUnbound { .. } | DeliveryRole::Historical { .. } => None,
        }
    }

    pub(crate) async fn prepare_cut(&self, context: &PublicationContext) -> Result<PreparedCut, CutUnavailable> {
        let emitter = self.source.0.emitter.get().ok_or(CutUnavailable::SourceUnavailable)?;
        if emitter.gateway.receiver_settlement() != xai_acp_lib::ReceiverSettlement::AppliedOrDiscarded {
            return Err(CutUnavailable::UnsupportedTransport);
        }
        if self.publication_context().as_ref() != Some(context) {
            return Err(CutUnavailable::Superseded);
        }
        let snapshot = match self.snapshot(context.binding.clone()) {
            CurrentActionsSnapshot::Current(snapshot) if snapshot.version.incarnation == context.incarnation => snapshot,
            CurrentActionsSnapshot::Current(_) | CurrentActionsSnapshot::Unavailable => return Err(CutUnavailable::SourceUnavailable),
        };
        let producers = emitter.gateway.producer_prefix();
        if producers.settle().await.is_err() {
            return Err(CutUnavailable::ProducerLost);
        }
        let queue = self.source.0.replay_queue.get().ok_or(CutUnavailable::SourceUnavailable)?;
        crate::session::replay_events::flush_replay_actor(queue).await
            .map_err(|_| CutUnavailable::ReplayFlushFailed)?;
        #[cfg(test)]
        crate::agent::mvp_agent::test_hooks::pause_at(crate::agent::mvp_agent::test_hooks::AttachPause::PublicationDrain).await;
        emitter.gateway.delivery_prefix().settle().await.map_err(|error| match error {
            xai_acp_lib::PublicationFailure::ProducerLost => CutUnavailable::ProducerLost,
            xai_acp_lib::PublicationFailure::ReceiverFailed => CutUnavailable::ReceiverFailed,
            xai_acp_lib::PublicationFailure::IncompletePublication => CutUnavailable::PublicationIncomplete,
        })?;
        if self.publication_context().as_ref() != Some(context) {
            return Err(CutUnavailable::Superseded);
        }
        Ok(PreparedCut { snapshot, source: self.source.clone() })
    }
}

impl PreparedCut {
    fn context(&self) -> PublicationContext {
        PublicationContext { binding: self.snapshot.binding.clone(), incarnation: self.snapshot.version.incarnation }
    }

    pub(crate) async fn finish(self) -> Result<GenerationEnd, CutUnavailable> {
        let context = self.context();
        let emitter = self.source.0.emitter.get().ok_or(CutUnavailable::SourceUnavailable)?;
        let end = GenerationEnd { snapshot: self.snapshot };
        let completion = {
            let delivery = self.source.0.delivery.lock();
            if *delivery != (DeliveryRole::Live { context }) {
                return Err(CutUnavailable::Superseded);
            }
            let params = serde_json::value::to_raw_value(&end).map_err(|_| CutUnavailable::SourceUnavailable)?;
            emitter.gateway.forward_with_completion(agent_client_protocol::ExtNotification::new(GENERATION_END_METHOD, params.into()))
        };
        match completion.await {
            Ok(Ok(())) => Ok(end),
            Ok(Err(_)) | Err(_) => Err(CutUnavailable::ReceiverFailed),
        }
    }
}
