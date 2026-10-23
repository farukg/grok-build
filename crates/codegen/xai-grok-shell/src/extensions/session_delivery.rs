use super::*;
use crate::session::delivery_receiver::{DeliveryAction, DeliveryRequest, DeliveryResult};
use crate::session::storage::StorageAdapter;

pub(crate) async fn handle(agent: &crate::agent::MvpAgent, args: &acp::ExtRequest) -> ExtResult {
    let request: DeliveryRequest = parse_params(args)?;
    if let Err(reason) = request.validate() { return to_raw_response(&DeliveryResult::Rejected { reason: reason.into() }); }
    let sid = acp::SessionId::new(request.session_id.clone());
    if let Some(handle) = agent.session_handle_waiting_for_load(&sid).await {
        if agent.child_writer_is_active(&sid) {
            return to_raw_response(&DeliveryResult::Rejected { reason: "receiver requires a root session; child ingress retains its own receipt and Held policy".into() });
        }
        return receive_resident(&handle.cmd_tx, request).await;
    }
    let lookup_id = request.session_id.clone();
    let summary = match tokio::task::spawn_blocking(move || crate::session::persistence::find_summary_by_session_id_result(&lookup_id)).await {
        Ok(Ok(Some(summary))) => summary,
        Ok(Ok(None)) => return to_raw_response(&match request.action {
            DeliveryAction::Observe => DeliveryResult::Absent,
            DeliveryAction::Admit => DeliveryResult::Rejected { reason: "target session is absent".into() },
        }),
        Ok(Err(error)) => return to_raw_response(&DeliveryResult::Indeterminate { reason: error.to_string() }),
        Err(error) => return to_raw_response(&DeliveryResult::Indeterminate { reason: error.to_string() }),
    };
    let storage = crate::session::storage::jsonl::JsonlStorageAdapter::default();
    let mut observe = request.clone(); observe.action = DeliveryAction::Observe;
    let observed = match storage.receive_delivery(&summary.info, &observe, crate::session::receiver_execution::ReceiverAccess::Observe).await {
        Ok(observed) => observed,
        Err(error) => return to_raw_response(&DeliveryResult::Indeterminate { reason: error.to_string() }),
    };
    let result = match (request.action, observed) {
        (DeliveryAction::Observe, result) => result,
        (DeliveryAction::Admit, DeliveryResult::Absent) => DeliveryResult::Rejected { reason: "target session must be loaded by its owning leader before new delivery admission".into() },
        (DeliveryAction::Admit, result @ (DeliveryResult::Admitted { .. } | DeliveryResult::Conflict { .. } | DeliveryResult::Rejected { .. } | DeliveryResult::Indeterminate { .. })) => result,
    };
    to_raw_response(&result)
}

async fn receive_resident(sender: &tokio::sync::mpsc::UnboundedSender<crate::session::SessionCommand>, request: DeliveryRequest) -> ExtResult {
    let (respond_to, receive) = tokio::sync::oneshot::channel();
    if sender.send(crate::session::SessionCommand::ReceiveDelivery { request, respond_to }).is_err() {
        return to_raw_response(&DeliveryResult::Indeterminate { reason: "receiver actor closed".into() });
    }
    let result = receive.await.unwrap_or_else(|_| DeliveryResult::Indeterminate { reason: "receiver response lost; observe the same identity".into() });
    to_raw_response(&result)
}
