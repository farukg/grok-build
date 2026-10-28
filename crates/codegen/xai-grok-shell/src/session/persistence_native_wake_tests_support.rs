use super::*;

#[derive(Debug)]
pub(crate) struct NativeAdoptionFaultProbe {
    pub(crate) owner: mpsc::UnboundedSender<PersistenceMsg>,
    pub(crate) release: tokio::sync::oneshot::Sender<()>,
    pub(crate) cancellation: tokio_util::sync::CancellationToken,
    pub(crate) pause_owner: tokio::sync::oneshot::Sender<NativeOwnerTransportCut>,
}

#[derive(Debug)]
pub(crate) struct NativeOwnerTransportCut {
    pub(crate) reached: tokio::sync::oneshot::Sender<()>,
    pub(crate) release: tokio::sync::oneshot::Receiver<()>,
}

#[derive(Debug)]
pub(crate) struct NativeProjectionFaultProbe {
    pub(crate) owner: mpsc::UnboundedSender<PersistenceMsg>,
    pub(crate) release: tokio::sync::oneshot::Sender<()>,
    pub(crate) cancellation: tokio_util::sync::CancellationToken,
}

#[derive(Debug)]
pub(crate) enum NativeDeferredCommit {
    Accepted,
    Rejected,
    GateDropped,
}

#[derive(Debug)]
pub(crate) enum NativeAttemptEndCut {
    BeforeTerminal { release: tokio::sync::oneshot::Sender<()> },
    RunnerSnapshot {
        ownership: crate::session::receiver_execution::InitialMessageTerminalOwnership,
        release: tokio::sync::oneshot::Sender<()>,
    },
}

#[derive(Clone, Debug)]
pub(crate) enum ParentInputFault {
    ReleasedAttemptEnd {
        observe: mpsc::UnboundedSender<NativeProjectionFaultProbe>,
        actor: mpsc::UnboundedSender<crate::session::SessionHandle>,
        terminal: mpsc::UnboundedSender<NativeAttemptEndCut>,
    },
    BeforeAdoptedProjection {
        observe: mpsc::UnboundedSender<NativeProjectionFaultProbe>,
        actor: mpsc::UnboundedSender<crate::session::SessionHandle>,
    },
    PublishedReconfirm {
        observe: mpsc::UnboundedSender<NativeAdoptionFaultProbe>,
        actor: mpsc::UnboundedSender<crate::session::SessionHandle>,
        commit: mpsc::UnboundedSender<NativeDeferredCommit>,
    },
    AtomicWrite(crate::session::storage::AtomicWriteFault),
    LoseAdmissionAck,
    LoseReservationAck,
    ReservationDigestConflict,
    ShutdownAfterAdmission,
    ShutdownAfterProjection,
    ShutdownAfterWakeStart,
    ShutdownBeforeProjection,
}

pub(super) fn configure_storage(
    storage: JsonlStorageAdapter,
    fault: Option<ParentInputFault>,
) -> JsonlStorageAdapter {
    match fault {
        Some(ParentInputFault::AtomicWrite(fault)) => {
            let armed = std::sync::atomic::AtomicBool::new(true);
            storage.with_parent_admission_probe(move || {
                if armed.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    crate::session::storage::fail_next_atomic_write(fault);
                }
                Ok(())
            })
        }
        Some(ParentInputFault::BeforeAdoptedProjection { .. } | ParentInputFault::ReleasedAttemptEnd { .. } | ParentInputFault::PublishedReconfirm { .. } | ParentInputFault::LoseAdmissionAck | ParentInputFault::LoseReservationAck
            | ParentInputFault::ReservationDigestConflict | ParentInputFault::ShutdownAfterAdmission
            | ParentInputFault::ShutdownAfterProjection | ParentInputFault::ShutdownAfterWakeStart | ParentInputFault::ShutdownBeforeProjection)
        | None => storage,
    }
}

pub(super) fn configure_transport(
    mut handle: PersistenceHandle,
    mut fault: Option<ParentInputFault>,
) -> PersistenceHandle {
    match fault {
        Some(ParentInputFault::AtomicWrite(_)) | None => return handle,
        Some(ParentInputFault::BeforeAdoptedProjection { .. } | ParentInputFault::ReleasedAttemptEnd { .. } | ParentInputFault::PublishedReconfirm { .. } | ParentInputFault::LoseAdmissionAck | ParentInputFault::LoseReservationAck
            | ParentInputFault::ReservationDigestConflict | ParentInputFault::ShutdownAfterAdmission
            | ParentInputFault::ShutdownAfterProjection | ParentInputFault::ShutdownAfterWakeStart | ParentInputFault::ShutdownBeforeProjection) => {}
    }
    let (proxy, mut requests) = mpsc::unbounded_channel();
    let real = std::mem::replace(&mut handle.tx, proxy);
    tokio::spawn(async move {
        let mut owner_cut: Option<tokio::sync::oneshot::Receiver<NativeOwnerTransportCut>> = None;
        while let Some(message) = requests.recv().await {
            if let Some(mut pending) = owner_cut.take() {
                match pending.try_recv() {
                    Ok(cut) => {
                        let _ = cut.reached.send(());
                        let _ = cut.release.await;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => owner_cut = Some(pending),
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {}
                }
            }
            match fault.clone() {
                Some(ParentInputFault::ReleasedAttemptEnd { terminal, .. })
                    if matches!(&message, PersistenceMsg::CancelParentMessages { .. } | PersistenceMsg::ResolveParentMessageTerminal { .. }) => {
                    let (release, released) = tokio::sync::oneshot::channel();
                    terminal.send(NativeAttemptEndCut::BeforeTerminal { release }).unwrap();
                    let _ = released.await;
                    let _ = real.send(message);
                    fault = None;
                }
                Some(ParentInputFault::BeforeAdoptedProjection { observe, .. }
                    | ParentInputFault::ReleasedAttemptEnd { observe, .. }) => {
                    if let PersistenceMsg::AppendInputAndAck { projection, admission, respond_to } = message {
                        if matches!(&projection, xai_chat_state::InputProjection::Delivery(delivery) if delivery.binding.delivery_id.starts_with("parent-message/")) {
                            let (release, released) = tokio::sync::oneshot::channel();
                            observe.send(NativeProjectionFaultProbe { owner: real.clone(), release, cancellation: admission.cancellation.clone() }).unwrap();
                            let owner = real.clone();
                            tokio::spawn(async move {
                                let _ = released.await;
                                let _ = owner.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to });
                            });
                            if matches!(&fault, Some(ParentInputFault::BeforeAdoptedProjection { .. })) {
                                fault = None;
                            }
                        } else {
                            let _ = real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to });
                        }
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(ParentInputFault::PublishedReconfirm { observe, .. }) => {
                    if let PersistenceMsg::ReconfirmPublishedParentMessage { reservation, launch, respond_to } = message {
                        let cancellation = super::parent_message::tests_support::cancellation(&reservation);
                        let (reply, response) = tokio::sync::oneshot::channel();
                        real.send(PersistenceMsg::ReconfirmPublishedParentMessage { reservation, launch, respond_to: reply }).unwrap();
                        let result = response.await.unwrap();
                        assert!(result.is_ok(), "cut requires a real successful reconfirm: {result:?}");
                        let (release, released) = tokio::sync::oneshot::channel();
                        let (pause_owner, paused) = tokio::sync::oneshot::channel();
                        owner_cut = Some(paused);
                        observe.send(NativeAdoptionFaultProbe { owner: real.clone(), release, cancellation, pause_owner }).unwrap();
                        tokio::spawn(async move {
                            if released.await.is_ok() {
                                let _ = respond_to.send(result);
                            }
                        });
                        fault = None;
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(cut @ (ParentInputFault::LoseAdmissionAck | ParentInputFault::ShutdownAfterAdmission)) => {
                    if let PersistenceMsg::AdmitParentMessage { admission, authority, respond_to } = message {
                        let (reply, response) = tokio::sync::oneshot::channel();
                        real.send(PersistenceMsg::AdmitParentMessage { admission, authority, respond_to: reply }).unwrap();
                        let result = response.await.unwrap();
                        assert!(result.is_ok(), "cut requires a real durable admission: {result:?}");
                        drop(respond_to);
                        match cut {
                            ParentInputFault::ShutdownAfterAdmission => break,
                            ParentInputFault::LoseAdmissionAck => fault = None,
                            ParentInputFault::BeforeAdoptedProjection { .. } | ParentInputFault::ReleasedAttemptEnd { .. } | ParentInputFault::PublishedReconfirm { .. } | ParentInputFault::AtomicWrite(_) | ParentInputFault::LoseReservationAck
                            | ParentInputFault::ReservationDigestConflict | ParentInputFault::ShutdownAfterProjection | ParentInputFault::ShutdownAfterWakeStart | ParentInputFault::ShutdownBeforeProjection => unreachable!(),
                        }
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(cut @ (ParentInputFault::LoseReservationAck | ParentInputFault::ReservationDigestConflict)) => {
                    if let PersistenceMsg::ReserveParentMessageTurn { mut reservation, respond_to } = message {
                        if matches!(cut, ParentInputFault::ReservationDigestConflict) {
                            reservation.payload_digest = "conflicting-digest".into();
                        }
                        let (reply, response) = tokio::sync::oneshot::channel();
                        real.send(PersistenceMsg::ReserveParentMessageTurn { reservation, respond_to: reply }).unwrap();
                        let result = response.await.unwrap();
                        match cut {
                            ParentInputFault::LoseReservationAck => {
                                assert!(result.is_ok(), "cut requires a genuine reservation: {result:?}");
                                drop(respond_to);
                            }
                            ParentInputFault::ReservationDigestConflict => {
                                assert!(result.is_err(), "durable owner must reject the conflicting digest");
                                let _ = respond_to.send(result);
                            }
                            ParentInputFault::BeforeAdoptedProjection { .. } | ParentInputFault::ReleasedAttemptEnd { .. } | ParentInputFault::PublishedReconfirm { .. } | ParentInputFault::AtomicWrite(_) | ParentInputFault::LoseAdmissionAck
                            | ParentInputFault::ShutdownAfterAdmission | ParentInputFault::ShutdownAfterProjection | ParentInputFault::ShutdownAfterWakeStart | ParentInputFault::ShutdownBeforeProjection => unreachable!(),
                        }
                        fault = None;
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(ParentInputFault::ShutdownAfterWakeStart) => {
                    if let PersistenceMsg::WakeStart { start, abort, respond_to } = message {
                        let (reply, response) = tokio::sync::oneshot::channel();
                        real.send(PersistenceMsg::WakeStart { start, abort, respond_to: reply }).unwrap();
                        let result = response.await.unwrap();
                        assert!(result.is_ok(), "cut requires a genuine committed WakeStart: {result:?}");
                        drop(respond_to);
                        break;
                    }
                    let _ = real.send(message);
                }
                Some(ParentInputFault::ShutdownBeforeProjection) => {
                    if let PersistenceMsg::AppendInputAndAck { projection, admission, mut respond_to } = message {
                        if matches!(&projection, xai_chat_state::InputProjection::Delivery(projection) if projection.binding.delivery_id.starts_with("parent-message/")) {
                            drop(real);
                            respond_to.closed().await;
                            break;
                        }
                        let _ = real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to });
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(ParentInputFault::ShutdownAfterProjection) => {
                    if let PersistenceMsg::AppendInputAndAck { projection, admission, respond_to } = message {
                        let is_parent = matches!(&projection, xai_chat_state::InputProjection::Delivery(projection) if projection.binding.delivery_id.starts_with("parent-message/"));
                        if is_parent {
                            let (reply, response) = tokio::sync::oneshot::channel();
                            real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to: reply }).unwrap();
                            let result = response.await.unwrap();
                            assert!(matches!(result, Ok(xai_chat_state::CompletionProjectionAck::Appended)), "cut requires a genuine strict projection: {result:?}");
                            drop(respond_to);
                            break;
                        }
                        let _ = real.send(PersistenceMsg::AppendInputAndAck { projection, admission, respond_to });
                    } else {
                        let _ = real.send(message);
                    }
                }
                Some(ParentInputFault::AtomicWrite(_)) | None => { let _ = real.send(message); }
            }
        }
    });
    handle
}

pub(crate) fn wire_reference_fixture() -> xai_grok_tools::implementations::grok_build::task::native_recovery::MessageRecoveryRef {
    use xai_grok_tools::implementations::grok_build::task::{native_recovery::MessageRecoveryRef, root_control::AgentMessageGeneration, types::{ActiveAgentMessageOperation, ActiveMessageReceiver}};
    MessageRecoveryRef {
        delivery_id: "parent-message/message".into(), message_id: "message".into(), prompt_id: "parent-message-message".into(),
        payload_digest: "ab".repeat(32), operation: ActiveAgentMessageOperation::Queue,
        receiver: ActiveMessageReceiver { session_id: "receiver".into(), generation: AgentMessageGeneration::mint(42) },
    }
}
