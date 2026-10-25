use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc;
use xai_message_delivery_core::{AgentId, AttemptId};

use super::*;
use crate::implementations::grok_build::task::coordinator::active_message::tests::{
    TestCoordinator, finish_child, finish_child_before_visibility, fixture, insert_child_with,
    insert_pending, promote_pending, recv_with_timeout, release_admission, response_outcome,
};
use crate::implementations::grok_build::task::coordinator::agent_targets::tests::holder;
use crate::implementations::grok_build::task::coordinator::{
    ActiveMessageAdmission, SendBoxFuture,
};
use crate::implementations::grok_build::task::coordinator_state::PendingDisposition;
use crate::implementations::grok_build::task::root_control::{
    RootReceiptSink, UserInputGeneration,
};
use crate::implementations::grok_build::task::types::{
    ActiveAgentMessageOperation, ActiveAgentMessageRequest, ActiveMessageSenderContext,
    ActiveMessageTarget, SubagentActiveMessageRequest, SubagentOwner,
};
use crate::implementations::grok_build::task::types::{
    InterruptionCause, SubagentActor, SubagentState,
};

const ROOT_A: &str = "019b0000-0000-7000-8000-000000000011";
const ROOT_B: &str = "019b0000-0000-7000-8000-000000000012";
const CHILD: &str = "019b0000-0000-7000-8000-000000000003";

thread_local! {
    static ROOTS: RefCell<HashMap<AgentId, TestRootControl>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub(in super::super) struct TestRootControl {
    agent_id: AgentId,
    session_id: &'static str,
    attempt_id: AttemptId,
    generation: AgentMessageGeneration,
    is_receipt_closed: bool,
    routes: mpsc::UnboundedSender<ActiveMessageRoute>,
    held_admissions: Option<mpsc::UnboundedSender<super::super::active_message::tests::AdmissionCall>>,
}

impl RootReceiptSink for bool {
    fn is_closed(&self) -> bool {
        *self
    }
}

impl RootControl for TestRootControl {
    type ReceiptSink = bool;

    fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    fn session_id(&self) -> &str {
        self.session_id
    }
    fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }
    fn generation(&self) -> AgentMessageGeneration {
        self.generation
    }
    fn user_input_generation(&self) -> UserInputGeneration {
        UserInputGeneration::new(7)
    }
    fn label(&self) -> &str {
        self.session_id
    }
    fn receipt_sink(&self) -> bool {
        self.is_receipt_closed
    }
    fn deliver(
        &self,
        delivery: ActiveAgentMessageDelivery,
    ) -> SendBoxFuture<ActiveMessageAdmission> {
        if let Some(admissions) = &self.held_admissions {
            use crate::implementations::grok_build::task::coordinator_state::ChildControl;
            return super::super::active_message::tests::TestControl {
                admissions: admissions.clone(),
            }
            .send_active_message(delivery);
        }
        let routes = self.routes.clone();
        Box::pin(async move {
            let route = delivery.route();
            let admitted = delivery
                .claim_admission()
                .map(|claim| claim.commit().try_handoff(|| routes.send(route)))
                .is_some_and(|result| result.is_ok());
            if admitted {
                ActiveMessageAdmission::Admitted
            } else {
                ActiveMessageAdmission::Rejected
            }
        })
    }
}

pub(in super::super) fn resolve_root(id: &AgentId) -> Option<TestRootControl> {
    ROOTS.with_borrow(|roots| roots.get(id).cloned())
}

pub(in super::super) fn resolve_root_session(session_id: &str) -> Option<TestRootControl> {
    ROOTS.with_borrow(|roots| {
        roots
            .values()
            .find(|root| root.session_id == session_id)
            .cloned()
    })
}

fn install_roots(routes: mpsc::UnboundedSender<ActiveMessageRoute>) {
    ROOTS.with_borrow_mut(|roots| {
        *roots = [(ROOT_A, "root-a"), (ROOT_B, "root-b")]
            .map(|(id, session_id)| {
                let agent_id = AgentId::parse(id).expect("root id");
                (
                    agent_id.clone(),
                    TestRootControl {
                        agent_id,
                        session_id,
                        attempt_id: AttemptId::mint(1),
                        generation: AgentMessageGeneration::mint(1),
                        is_receipt_closed: false,
                        routes: routes.clone(),
                        held_admissions: None,
                    },
                )
            })
            .into();
    });
}

fn agent(id: &str) -> ActiveMessageTarget {
    ActiveMessageTarget::Agent {
        agent_id: AgentId::parse(id).expect("agent id"),
    }
}

fn begin(
    coordinator: &mut TestCoordinator,
    tx: &crate::implementations::grok_build::task::backend::SubagentCoordinatorSender,
    target: ActiveMessageTarget,
    sender_context: ActiveMessageSenderContext,
) -> tokio::sync::oneshot::Receiver<ActiveAgentMessageOutcome> {
    let (respond_to, response) = tokio::sync::oneshot::channel();
    tx.try_send_active_message(SubagentActiveMessageRequest {
        request: ActiveAgentMessageRequest::try_from_parts(
            target,
            "hello",
            ActiveAgentMessageOperation::Steer,
        )
        .expect("request"),
        sender_context,
        respond_to,
    })
    .expect("ingress open");
    let ingress = coordinator
        .active_message_ingress
        .as_mut()
        .expect("paired ingress")
        .try_recv()
        .expect("request queued");
    coordinator.handle_send_active_message(ingress);
    response
}

async fn finish(
    coordinator: &mut TestCoordinator,
    routes: &mut mpsc::UnboundedReceiver<ActiveMessageRoute>,
    response: tokio::sync::oneshot::Receiver<ActiveAgentMessageOutcome>,
) -> ActiveMessageRoute {
    let completion = coordinator
        .active_messages
        .next()
        .await
        .expect("completion");
    let route = routes.recv().await.expect("root admission");
    coordinator.finish_active_message(completion);
    assert!(matches!(
        response.await.expect("response"),
        ActiveAgentMessageOutcome::Accepted { .. }
    ));
    route
}

#[tokio::test(flavor = "current_thread")]
async fn root_resolution_routes_and_completion_are_exact() {
    let (mut coordinator, tx, admission_tx, _admissions) = fixture();
    let (routes_tx, mut routes) = mpsc::unbounded_channel();
    install_roots(routes_tx);
    let sender = holder(&mut coordinator, admission_tx, None);
    coordinator.graph.remove(CHILD);
    coordinator.graph.insert_root_child(CHILD, "root-a");
    let granted = ActiveMessageSenderContext::GrantedChild {
        holder: sender.holder().clone(),
    };
    for (index, (target, context, route)) in [
        (
            ActiveMessageTarget::Parent,
            granted.clone(),
            ActiveMessageRoute::DescendantToParent,
        ),
        (
            agent(ROOT_A),
            granted.clone(),
            ActiveMessageRoute::DescendantToParent,
        ),
        (agent(ROOT_B), granted, ActiveMessageRoute::Peer),
        (
            agent(ROOT_B),
            ActiveMessageSenderContext::RootSession {
                session_id: Arc::from("root-a"),
            },
            ActiveMessageRoute::Peer,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = begin(&mut coordinator, &tx, target, context);
        assert!(!coordinator.active_messages.is_empty(), "case {index}");
        assert_eq!(route, finish(&mut coordinator, &mut routes, response).await);
    }
    assert!(coordinator.root_active_messages.is_empty());
    coordinator.terminal_outputs.insert(
        CHILD.to_owned(),
        crate::implementations::grok_build::task::coordinator_state::TerminalOutput::Finishing(
            crate::implementations::grok_build::task::coordinator::ChildRunOutput {
                result: crate::implementations::grok_build::task::types::SubagentResult {
                    state: SubagentState::Completed,
                    subagent_id: CHILD.to_owned(),
                    output: "sentinel".into(),
                    ..Default::default()
                },
                completion_data: (),
                snapshot_ref: None,
            },
        ),
    );
    let response = begin(
        &mut coordinator,
        &tx,
        agent(ROOT_B),
        ActiveMessageSenderContext::RootSession {
            session_id: Arc::from("root-a"),
        },
    );
    let _ = finish(&mut coordinator, &mut routes, response).await;
    assert!(coordinator.terminal_outputs.contains_key(CHILD));
    for (target, context) in [
        (
            agent(CHILD),
            ActiveMessageSenderContext::GrantedChild {
                holder: sender.holder().clone(),
            },
        ),
        (
            agent(ROOT_A),
            ActiveMessageSenderContext::RootSession {
                session_id: Arc::from("root-a"),
            },
        ),
    ] {
        assert_eq!(
            ActiveAgentMessageOutcome::NotFoundOrNotOwned,
            begin(&mut coordinator, &tx, target, context)
                .await
                .expect("self response"),
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn closed_receipt_fails_before_delivery() {
    let (mut coordinator, tx, _admission_tx, _admissions) = fixture();
    let (routes_tx, mut routes) = mpsc::unbounded_channel();
    install_roots(routes_tx);
    ROOTS.with_borrow_mut(|roots| {
        roots
            .get_mut(&AgentId::parse(ROOT_A).expect("root id"))
            .expect("root")
            .is_receipt_closed = true;
    });
    assert_eq!(
        ActiveAgentMessageOutcome::ChannelClosed,
        begin(
            &mut coordinator,
            &tx,
            agent(ROOT_A),
            ActiveMessageSenderContext::RootSession {
                session_id: Arc::from("root-b")
            },
        )
        .await
        .expect("closed response"),
    );
    assert!(routes.try_recv().is_err());
    assert!(coordinator.root_active_messages.is_empty());
    ROOTS.with_borrow_mut(|roots| {
        roots
            .get_mut(&AgentId::parse(ROOT_A).expect("root id"))
            .expect("root")
            .is_receipt_closed = false;
    });
    let response = begin(
        &mut coordinator,
        &tx,
        agent(ROOT_A),
        ActiveMessageSenderContext::RootSession {
            session_id: Arc::from("root-b"),
        },
    );
    assert_eq!(
        ActiveMessageRoute::Peer,
        finish(&mut coordinator, &mut routes, response).await,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn peer_root_reaches_another_roots_child_in_every_state() {
    for state in "active|pending|completed|completed_unpublished|completed_killed".split('|') {
        let (mut c, tx, admission_tx, mut admissions) = fixture();
        let (routes_tx, _routes) = mpsc::unbounded_channel();
        install_roots(routes_tx);
        if state == "pending" {
            insert_pending(&mut c, CHILD, "root-b");
        } else {
            insert_child_with(
                &mut c,
                admission_tx.clone(),
                CHILD,
                "root-b",
                None,
                SubagentOwner::Task,
            );
        }
        if state == "completed_killed" {
            c.active.get_mut(CHILD).unwrap().disposition =
                PendingDisposition::Interrupted(InterruptionCause::ExplicitStop {
                    actor: SubagentActor::Human,
                });
        }
        if state == "completed_unpublished" {
            finish_child_before_visibility(&mut c, CHILD);
        } else if state.starts_with("completed") {
            finish_child(&mut c, CHILD);
        }
        let sender = ActiveMessageSenderContext::RootSession {
            session_id: Arc::from("root-a"),
        };
        let mut response = begin(&mut c, &tx, agent(CHILD), sender.clone());
        if state == "completed_killed" {
            assert_eq!(
                ActiveAgentMessageOutcome::NotActiveOrFinalizing,
                response_outcome(response).await
            );
            assert!(!c.pending.contains_key(CHILD) && admissions.try_recv().is_err());
            continue;
        }
        assert!(response.try_recv().is_err(), "{state}");
        if state == "completed_unpublished" {
            assert_eq!(1, c.pending_wakes.get(CHILD).unwrap().len(), "{state}");
            let publication = c
                .terminal_published_rx
                .try_recv()
                .expect("host visibility callback");
            c.handle_terminal_publication(publication);
        }
        // A wake folds its first send into the prompt, so a second send exposes the route.
        let wake = state.starts_with("completed").then(|| {
            assert!(c.pending.contains_key(CHILD), "{state}");
            std::mem::replace(&mut response, begin(&mut c, &tx, agent(CHILD), sender))
        });
        if state != "active" {
            promote_pending(&mut c, admission_tx, CHILD);
        }
        let call = recv_with_timeout(&mut admissions).await;
        assert_eq!(
            (ActiveMessageRoute::Peer, "root-a"),
            (
                call.late_delivery.route(),
                call.late_delivery.message().sender_session_id.as_str(),
            ),
            "{state}"
        );
        release_admission(&mut c, call, ActiveMessageAdmission::Admitted).await;
        for response in wake.into_iter().chain([response]) {
            assert!(
                matches!(
                    response_outcome(response).await,
                    ActiveAgentMessageOutcome::Accepted { .. }
                ),
                "{state}"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn root_deadline_arbitrates_real_claim_before_any_handoff() {
    for claimed in [false, true] {
        let (mut coordinator, tx, admission_tx, mut admissions) = fixture();
        let (routes_tx, mut routes) = mpsc::unbounded_channel();
        install_roots(routes_tx);
        ROOTS.with_borrow_mut(|roots| {
            roots.get_mut(&AgentId::parse(ROOT_A).unwrap()).unwrap().held_admissions =
                Some(admission_tx);
        });
        let response = begin(
            &mut coordinator,
            &tx,
            agent(ROOT_A),
            ActiveMessageSenderContext::RootSession { session_id: Arc::from("root-b") },
        );
        let held = recv_with_timeout(&mut admissions).await;
        let claim = claimed.then(|| held.late_delivery.clone().claim_admission().unwrap());
        assert!(routes.try_recv().is_err());
        tokio::time::advance(ACTIVE_MESSAGE_ADMISSION_TIMEOUT).await;
        let completion = coordinator.active_messages.next().await.unwrap();
        coordinator.finish_active_message(completion);
        let expected = if claimed {
            ActiveAgentMessageOutcome::AdmissionUncertain
        } else {
            ActiveAgentMessageOutcome::NotAcceptedBeforeDeadline
        };
        assert_eq!(expected, response_outcome(response).await);
        assert!(held.late_delivery.claim_admission().is_none());
        assert!(held.release.send(ActiveMessageAdmission::Admitted).is_err());
        assert!(coordinator.root_active_messages.is_empty());
        assert!(routes.try_recv().is_err());
        if let Some(claim) = claim {
            assert_eq!("root-b", claim.delivery().message().sender_session_id);
            assert_eq!("hello", claim.delivery().message().text.as_ref());
            assert_eq!(ActiveAgentMessageOperation::Steer, claim.delivery().operation());
            let (receipt, mut receipts) = mpsc::unbounded_channel();
            claim.commit().handoff(|| receipt.send("hello").unwrap());
            assert_eq!(Some("hello"), receipts.recv().await);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn root_observer_drop_cannot_revoke_owned_claim() {
    for claimed in [false, true] {
        let (mut coordinator, tx, admission_tx, mut admissions) = fixture();
        let (routes_tx, mut routes) = mpsc::unbounded_channel();
        install_roots(routes_tx);
        ROOTS.with_borrow_mut(|roots| {
            roots.get_mut(&AgentId::parse(ROOT_A).unwrap()).unwrap().held_admissions =
                Some(admission_tx);
        });
        let response = begin(
            &mut coordinator,
            &tx,
            agent(ROOT_A),
            ActiveMessageSenderContext::RootSession { session_id: Arc::from("root-b") },
        );
        let held = recv_with_timeout(&mut admissions).await;
        let claim = claimed.then(|| held.late_delivery.clone().claim_admission().unwrap());
        drop(coordinator);
        let expected = if claimed {
            ActiveAgentMessageOutcome::AdmissionUncertain
        } else {
            ActiveAgentMessageOutcome::ChannelClosed
        };
        assert_eq!(expected, response_outcome(response).await);
        assert!(held.late_delivery.claim_admission().is_none());
        assert!(routes.try_recv().is_err());
        if let Some(claim) = claim {
            claim.commit().handoff(|| ());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn root_admitted_requires_completed_durable_proof_handoff() {
    use super::super::active_message::tests::HandoffCut;

    for cut in [
        HandoffCut::Withheld,
        HandoffCut::Dropped,
        HandoffCut::Failed,
        HandoffCut::Panicked,
        HandoffCut::Completed,
    ] {
        let (mut coordinator, tx, admission_tx, mut admissions) = fixture();
        let (routes_tx, mut routes) = mpsc::unbounded_channel();
        install_roots(routes_tx);
        ROOTS.with_borrow_mut(|roots| {
            roots.get_mut(&AgentId::parse(ROOT_A).unwrap()).unwrap().held_admissions =
                Some(admission_tx);
        });
        let response = begin(
            &mut coordinator,
            &tx,
            agent(ROOT_A),
            ActiveMessageSenderContext::RootSession {
                session_id: Arc::from("root-b"),
            },
        );
        let held = recv_with_timeout(&mut admissions).await;
        let proof = held.late_delivery.clone().claim_admission().unwrap().commit();
        assert_eq!(held.message_id, proof.delivery().message().message_id);
        assert_eq!("root-b", proof.delivery().message().sender_session_id);
        assert_eq!("hello", proof.delivery().message().text.as_ref());
        assert_eq!(ActiveAgentMessageOperation::Steer, proof.delivery().operation());
        let retained = cut.apply(proof);
        assert!(held.late_delivery.claim_admission().is_none());
        let (finalize_tx, mut finalize_rx) = tokio::sync::oneshot::channel();
        coordinator
            .root_active_messages
            .values_mut()
            .next()
            .unwrap()
            .active_messages
            .begin_finalizing(finalize_tx);
        assert!(finalize_rx.try_recv().is_err());
        held.report.send(ActiveMessageAdmission::Admitted).unwrap();
        let completion = coordinator.active_messages.next().await.unwrap();
        coordinator.finish_active_message(completion);
        let completed = matches!(cut, HandoffCut::Completed);
        let expected = if completed {
            ActiveAgentMessageOutcome::Accepted {
                message_id: held.message_id,
            }
        } else {
            ActiveAgentMessageOutcome::AdmissionUncertain
        };
        assert_eq!(expected, response_outcome(response).await, "{cut:?}");
        assert_eq!(completed, finalize_rx.await.unwrap(), "{cut:?}");
        assert!(coordinator.active_messages.is_empty());
        assert!(routes.try_recv().is_err());
        if let Some(proof) = retained {
            proof.handoff(|| ());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn root_lost_claim_and_ack_remain_uncertain() {
    let (mut coordinator, tx, admission_tx, mut admissions) = fixture();
    let (routes_tx, mut routes) = mpsc::unbounded_channel();
    install_roots(routes_tx);
    ROOTS.with_borrow_mut(|roots| {
        roots.get_mut(&AgentId::parse(ROOT_A).unwrap()).unwrap().held_admissions =
            Some(admission_tx);
    });
    let response = begin(
        &mut coordinator,
        &tx,
        agent(ROOT_A),
        ActiveMessageSenderContext::RootSession { session_id: Arc::from("root-b") },
    );
    let held = recv_with_timeout(&mut admissions).await;
    drop(held.late_delivery.clone().claim_admission().unwrap());
    assert!(held.late_delivery.claim_admission().is_none());
    drop(held.release);
    let completion = coordinator.active_messages.next().await.unwrap();
    coordinator.finish_active_message(completion);
    assert_eq!(ActiveAgentMessageOutcome::AdmissionUncertain, response_outcome(response).await);
    assert!(routes.try_recv().is_err());
    assert!(coordinator.root_active_messages.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn default_root_seam_replays_existing_park_wake_and_promotion_paths() {
    ROOTS.with_borrow_mut(HashMap::clear);
    super::super::agent_targets::tests::replay_root_and_granted_wakes().await;
    super::super::active_message::tests::replay_owned_queued_child_promotion().await;
    super::super::agent_targets::tests::replay_target_resolution_table().await;
    super::super::agent_targets::tests::replay_resolved_routes().await;
}
