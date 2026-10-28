use agent_client_protocol as acp;

use super::MvpAgent;
use crate::extensions::current_actions::{CutUnavailable, LoadCut, PublicationContext};
use crate::session::current_actions::{CurrentActionsSnapshot, SnapshotBinding};

impl MvpAgent {
    fn publication_attach_is_current(
        &self,
        binding: &SnapshotBinding,
        waiter: &tokio::sync::watch::Receiver<bool>,
    ) -> bool {
        self.session_registry.attach_waiter(&acp::SessionId::new(binding.session_id.clone()))
            .is_some_and(|current| current.same_channel(waiter))
    }

    pub(super) async fn ordered_current_actions_load(
        &self,
        binding: SnapshotBinding,
        context: Option<PublicationContext>,
        waiter: &tokio::sync::watch::Receiver<bool>,
    ) -> (CurrentActionsSnapshot, LoadCut) {
        let unavailable = |reason| (
            self.current_actions_snapshot(binding.clone()), LoadCut::Unavailable { reason },
        );
        if !self.publication_attach_is_current(&binding, waiter) {
            return unavailable(CutUnavailable::Superseded);
        }
        let session_id = acp::SessionId::new(binding.session_id.clone());
        let Some(handle) = self.resident_handle(&session_id) else {
            return unavailable(CutUnavailable::SourceUnavailable);
        };
        let Some(reader) = &handle.current_actions else {
            return unavailable(CutUnavailable::SourceUnavailable);
        };
        let context = match context {
            Some(context) => context,
            None => match reader.bind_publication(binding.clone()) {
                Ok(context) => context,
                Err(reason) => return unavailable(reason),
            },
        };
        let prepared = match reader.prepare_cut(&context).await {
            Ok(prepared) => prepared,
            Err(reason) => return unavailable(reason),
        };
        #[cfg(test)]
        super::test_hooks::pause_at(super::test_hooks::AttachPause::PublicationEnd).await;
        let current = || self.publication_attach_is_current(&binding, waiter)
            && self.resident_handle(&session_id).is_some_and(|resident| {
                resident.cmd_tx.same_channel(&handle.cmd_tx)
                    && resident.current_actions.as_ref().and_then(|reader| reader.publication_context()).as_ref() == Some(&context)
            });
        if !current() {
            return unavailable(CutUnavailable::Superseded);
        }
        match prepared.finish().await {
            Ok(end) if current() => (CurrentActionsSnapshot::Current(end.snapshot), LoadCut::Settled),
            Ok(_) => unavailable(CutUnavailable::Superseded),
            Err(reason) => unavailable(reason),
        }
    }

    pub(super) async fn stamp_ordered_current_actions_load(
        &self,
        meta: &mut Option<acp::Meta>,
        binding: Option<SnapshotBinding>,
        context: Option<PublicationContext>,
        waiter: &tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), acp::Error> {
        if let Some(binding) = binding {
            let (snapshot, cut) = self.ordered_current_actions_load(binding, context, waiter).await;
            let meta = meta.get_or_insert_with(acp::Meta::new);
            crate::extensions::current_actions::stamp_load_snapshot(meta, &snapshot)
                .and_then(|()| crate::extensions::current_actions::stamp_load_cut(meta, &cut))
                .map_err(|error| acp::Error::internal_error().data(error.to_string()))?;
        }
        Ok(())
    }
}
