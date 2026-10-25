use super::{IdentityStamp, MvpAgent, SessionHost};
use crate::session::SessionCommand;
use crate::session::delivery_receiver::{DeliveryRequest, DeliveryResult};
use agent_client_protocol as acp;

pub(crate) enum DeliveryDispatch {
    NotResident,
    Refused(DeliveryResult),
    Receiving(tokio::sync::oneshot::Receiver<DeliveryResult>),
}

impl MvpAgent {
    pub(crate) fn dispatch_session_delivery(
        &self,
        id: &acp::SessionId,
        request: &DeliveryRequest,
    ) -> DeliveryDispatch {
        let entries = self.session_registry.sessions.borrow();
        let Some(entry) = entries.get(id) else {
            return DeliveryDispatch::NotResident;
        };
        match &entry.host {
            Some(SessionHost::Child(_)) => {
                return DeliveryDispatch::Refused(DeliveryResult::Rejected {
                    reason: "receiver requires a root session; child ingress retains its own receipt and Held policy".into(),
                });
            }
            Some(SessionHost::Root) => {}
            None => {
                return match entry
                    .presence
                    .as_ref()
                    .and_then(|presence| presence.hosted_handle())
                {
                    Some(_) => DeliveryDispatch::Refused(DeliveryResult::Indeterminate {
                        reason: "receiver host provenance is unavailable".into(),
                    }),
                    None => DeliveryDispatch::NotResident,
                };
            }
        }
        let Some(presence) = &entry.presence else {
            return DeliveryDispatch::NotResident;
        };
        let Some(handle) = presence.hosted_handle() else {
            return DeliveryDispatch::NotResident;
        };
        if let Some(installed) = presence.installed() {
            match *installed.stamp.borrow() {
                IdentityStamp::Durable => {}
                IdentityStamp::Pending | IdentityStamp::Failed => {
                    return DeliveryDispatch::Refused(DeliveryResult::Indeterminate {
                        reason: "receiver root identity is not durable".into(),
                    });
                }
            }
        }
        let (respond_to, receive) = tokio::sync::oneshot::channel();
        match handle.cmd_tx.send(SessionCommand::ReceiveDelivery {
            request: request.clone(),
            respond_to,
        }) {
            Ok(()) => DeliveryDispatch::Receiving(receive),
            Err(_) => DeliveryDispatch::Refused(DeliveryResult::Indeterminate {
                reason: "receiver actor closed".into(),
            }),
        }
    }
}
