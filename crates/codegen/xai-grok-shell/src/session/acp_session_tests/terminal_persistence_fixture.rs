use super::*;
use crate::session::storage::{SessionUpdate, StorageAdapter};

pub(super) fn attach_parent_admissions(actor: &mut SessionActor) {
    let capture = actor.notifications.persistence_tx.clone();
    let (tx, mut rx) = mpsc::unbounded_channel();
    actor.notifications.persistence_tx = tx;
    let info = actor.session_info.clone();
    tokio::spawn(async move {
        let directory = tempfile::tempdir().unwrap();
        let storage = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let (lease, _) = storage.claim_receiver(&info).await.unwrap();
        while let Some(message) = rx.recv().await {
            match message {
                PersistenceMsg::CancelParentMessages { message_ids, respond_to } => {
                    let result = storage.cancel_parent_messages(&info, &message_ids, crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await;
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::AdmitParentMessage { admission, authority, respond_to } => {
                    let result = storage.admit_parent_message(&info, &admission, authority, crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await;
                    let _ = respond_to.send(result);
                }
                other => { let _ = capture.send(other); }
            }
        }
    });
}

pub(super) fn attach(actor: &mut SessionActor, capture: mpsc::UnboundedSender<PersistenceMsg>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    actor.notifications.persistence_tx = tx;
    let info = actor.session_info.clone();
    tokio::task::spawn_local(async move {
        let directory = tempfile::tempdir().unwrap();
        let storage = crate::session::storage::jsonl::JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_path_buf());
        storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
        let (lease, _) = storage.claim_receiver(&info).await.unwrap();
        while let Some(message) = rx.recv().await {
            match message {
                PersistenceMsg::CancelParentMessages { message_ids, respond_to } => {
                    let result = storage.cancel_parent_messages(&info, &message_ids, crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await;
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::AdmitParentMessage { admission, authority, respond_to } => {
                    let result = storage.admit_parent_message(&info, &admission, authority, crate::session::receiver_execution::ReceiverAccess::Owner(&lease)).await;
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::SuppressTerminalCompletions { ids, reason, respond_to } => {
                    let result = storage.suppress_terminal_completions(&info, ids, reason).await;
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::GetCompletionState { respond_to } => {
                    let _ = respond_to.send(storage.read_completion_state(&info).await);
                }
                PersistenceMsg::EnrollTerminalCompletion { source, text, policy, respond_to } => {
                    let result = storage.enroll_terminal_completion(&info, source, text, policy).await;
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to }
                    if matches!(&update, SessionUpdate::Xai(notification) if matches!(&notification.update, XaiSessionUpdate::TurnCompleted { .. })) => {
                    let result = storage.append_update_durable_commit_aware(&info, &update).await;
                    let (observed_ack, _) = oneshot::channel();
                    let _ = capture.send(PersistenceMsg::AppendUpdateDurablyAndAck { update, respond_to: observed_ack });
                    let _ = respond_to.send(result);
                }
                PersistenceMsg::FinishDeliveryTurn { turn_id, parent_task, origin, progress, respond_to } => {
                    let result = storage.finish_delivery_turn(&info, &turn_id, &parent_task, &origin, &progress).await;
                    let (observed_ack, _) = oneshot::channel();
                    let _ = capture.send(PersistenceMsg::FinishDeliveryTurn { turn_id, parent_task, origin, progress, respond_to: observed_ack });
                    let _ = respond_to.send(result);
                }
                other => { let _ = capture.send(other); }
            }
        }
    });
}
