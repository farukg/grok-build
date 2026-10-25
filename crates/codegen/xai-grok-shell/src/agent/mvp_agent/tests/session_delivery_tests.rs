use super::{
    AttachPause, assert_withdrawn, build_minimal_agent_for_tests, drain_persistence,
    make_live_session_handle, new_root_session, run_local_for_bridge_test,
    supersede_with_invalid_load, with_pause_at,
};
use crate::agent::MvpAgent;
use crate::agent::mvp_agent::session_setup::AttachOperation;
use crate::agent::mvp_agent::{IdentityStamp, SessionHost, StampResolution};
use crate::session::delivery_receiver::{DeliveryAction, DeliveryRequest, DeliveryResult};
use crate::session::storage::StorageAdapter;
use acp::Agent as _;
use agent_client_protocol as acp;

const DELIVERY_ID: &str = "stamp-gated-delivery";
const PAYLOAD: &str = "  exact pending-stamp delivery\n";

async fn delivery(agent: &MvpAgent, sid: &acp::SessionId, action: &str) -> DeliveryResult {
    let request = acp::ExtRequest::new(
        "x.ai/session/delivery",
        std::sync::Arc::from(
            serde_json::value::to_raw_value(&serde_json::json!({
                "sessionId": sid.0, "deliveryId": DELIVERY_ID, "payload": PAYLOAD, "action": action,
            }))
            .unwrap(),
        ),
    );
    let response =
        tokio::time::timeout(std::time::Duration::from_secs(1), agent.ext_method(request))
            .await
            .expect("delivery must not wait for the installer it is racing")
            .unwrap();
    serde_json::from_str(response.0.get()).unwrap()
}

async fn assert_pending_delivery_is_blocked(agent: &MvpAgent, sid: &acp::SessionId) {
    let (_, stamp) = agent.session_registry.pending_identity_stamp(sid).unwrap();
    assert_eq!(*stamp.borrow(), IdentityStamp::Pending);
    let (admit, observe) = tokio::join!(
        delivery(agent, sid, "admit"),
        delivery(agent, sid, "observe"),
    );
    for result in [admit, observe] {
        assert!(
            matches!(result, DeliveryResult::Indeterminate { .. }),
            "{result:?}"
        );
    }
    assert_eq!(*stamp.borrow(), IdentityStamp::Pending);
    assert_delivery_was_not_persisted(sid).await;
}

async fn assert_delivery_was_not_persisted(sid: &acp::SessionId) {
    let summary = crate::session::persistence::find_summary_by_session_id_result(sid.0.as_ref())
        .unwrap()
        .expect("the root fixture has a persisted summary");
    let request = DeliveryRequest {
        session_id: sid.0.to_string(),
        delivery_id: DELIVERY_ID.into(),
        payload: PAYLOAD.into(),
        action: DeliveryAction::Observe,
    };
    let storage = crate::session::storage::jsonl::JsonlStorageAdapter::default();
    assert!(matches!(
        storage
            .receive_delivery(
                &summary.info,
                &request,
                crate::session::receiver_execution::ReceiverAccess::Observe,
            )
            .await
            .unwrap(),
        DeliveryResult::Absent
    ));
}

async fn attach_root(
    agent: &MvpAgent,
    sid: &acp::SessionId,
    cwd: &std::path::Path,
    operation: AttachOperation,
) -> Result<(), acp::Error> {
    match operation {
        AttachOperation::Load => agent
            .load_session_inner(acp::LoadSessionRequest::new(sid.clone(), cwd.to_path_buf()))
            .await
            .map(|_| ()),
        AttachOperation::Resume => agent
            .resume_session_inner(acp::ResumeSessionRequest::new(
                sid.clone(),
                cwd.to_path_buf(),
            ))
            .await
            .map(|_| ()),
        AttachOperation::ManageChild => unreachable!("root attach fixture"),
    }
}

#[test]
fn resident_without_host_provenance_cannot_admit_or_observe() {
    run_local_for_bridge_test(|| async {
        use crate::session::delivery_receiver::DeliveryResult;
        use acp::Agent as _;
        let agent = build_minimal_agent_for_tests();
        let sid = acp::SessionId::new("unclassified-delivery-receiver");
        let (handle, _cmd_tx, _cmd_rx) = make_live_session_handle(&sid, None);
        agent.insert_resident(&sid, handle);
        for action in ["admit", "observe"] {
            let request = acp::ExtRequest::new("x.ai/session/delivery", std::sync::Arc::from(serde_json::value::to_raw_value(&serde_json::json!({
                "sessionId": sid.0, "deliveryId": "unknown-host", "payload": "exact unknown delivery", "action": action,
            })).unwrap()));
            let response =
                tokio::time::timeout(std::time::Duration::from_secs(1), agent.ext_method(request))
                    .await
                    .unwrap()
                    .unwrap();
            assert!(
                matches!(serde_json::from_str::<DeliveryResult>(response.0.get()).unwrap(), DeliveryResult::Indeterminate { reason } if reason == "receiver host provenance is unavailable")
            );
        }
        agent.remove_session(&sid);
    });
}

#[test]
fn cold_root_load_and_resume_restore_delivery_provenance() {
    run_local_for_bridge_test(|| async {
        use crate::session::delivery_receiver::DeliveryResult;
        use acp::Agent as _;
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        for operation in [AttachOperation::Load, AttachOperation::Resume] {
            agent.remove_session(&sid);
            assert!(agent.session_host(&sid).is_none());
            match operation {
                AttachOperation::Load => {
                    agent
                        .load_session_inner(acp::LoadSessionRequest::new(
                            sid.clone(),
                            cwd.path().to_path_buf(),
                        ))
                        .await
                        .unwrap();
                }
                AttachOperation::Resume => {
                    agent
                        .resume_session_inner(acp::ResumeSessionRequest::new(
                            sid.clone(),
                            cwd.path().to_path_buf(),
                        ))
                        .await
                        .unwrap();
                }
                AttachOperation::ManageChild => {
                    unreachable!("root attach fixture")
                }
            }
            assert!(matches!(agent.session_host(&sid), Some(SessionHost::Root)));
            assert_eq!(agent.session_registry.thread_is_finished(&sid), Some(false));
            let response = agent.ext_method(acp::ExtRequest::new("x.ai/session/delivery", std::sync::Arc::from(serde_json::value::to_raw_value(&serde_json::json!({
                "sessionId": sid.0, "deliveryId": "cold-root-observe", "payload": "exact root delivery", "action": "observe",
            })).unwrap()))).await.unwrap();
            assert!(matches!(
                serde_json::from_str::<DeliveryResult>(response.0.get()).unwrap(),
                DeliveryResult::Absent
            ));
        }
        agent.remove_session(&sid);
    });
}

#[test]
fn pending_root_stamp_blocks_concurrent_admit_and_observe_for_load_and_resume() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        for operation in [AttachOperation::Load, AttachOperation::Resume] {
            agent.remove_session(&sid);
            let (attach, reached, release) = with_pause_at(
                AttachPause::AfterInstall,
                attach_root(&agent, &sid, cwd.path(), operation),
            );
            let probe = async {
                reached
                    .await
                    .expect("the root is installed before its stamp");
                assert!(agent.session_host(&sid).is_none());
                assert_pending_delivery_is_blocked(&agent, &sid).await;
                assert!(agent.session_host(&sid).is_none());
                release.send(()).unwrap();
            };
            let (result, ()) = tokio::join!(attach, probe);
            result.expect("validated root attach succeeds");
            drain_persistence(&agent, &sid).await;
            assert!(matches!(
                delivery(&agent, &sid, "observe").await,
                DeliveryResult::Absent
            ));
            assert_delivery_was_not_persisted(&sid).await;
        }
        agent.remove_session(&sid);
    });
}

#[test]
fn retained_root_provenance_cannot_bypass_a_pending_reload_stamp() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        let evict = serde_json::value::to_raw_value(&serde_json::json!({
            "sessionIds": [sid.0],
        }))
        .unwrap();
        agent.handle_evict_sessions(&evict).await;
        assert!(!agent.is_resident(&sid));
        assert!(matches!(agent.session_host(&sid), Some(SessionHost::Root)));
        let (attach, reached, release) = with_pause_at(
            AttachPause::AfterInstall,
            agent.load_session_inner(acp::LoadSessionRequest::new(
                sid.clone(),
                cwd.path().to_path_buf(),
            )),
        );
        let probe = async {
            reached.await.unwrap();
            assert!(matches!(agent.session_host(&sid), Some(SessionHost::Root)));
            assert_pending_delivery_is_blocked(&agent, &sid).await;
            assert!(matches!(delivery(&agent, &sid, "observe").await,
                DeliveryResult::Indeterminate { reason } if reason == "receiver root identity is not durable"));
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(attach, probe);
        result.expect("the retained root reload validates successfully");
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        agent.remove_session(&sid);
    });
}

#[test]
fn failed_root_stamp_never_admits_delivery() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        agent.remove_session(&sid);
        let (attach, reached, release) = with_pause_at(
            AttachPause::AfterInstall,
            agent.load_session_inner(acp::LoadSessionRequest::new(
                sid.clone(),
                cwd.path().to_path_buf(),
            )),
        );
        let probe = async {
            reached.await.unwrap();
            assert_pending_delivery_is_blocked(&agent, &sid).await;
            crate::session::persistence::FAIL_NEXT_STAMP.set(true);
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(attach, probe);
        result.expect_err("failed stamp fails closed");
        assert_withdrawn(&agent, &sid);
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        assert!(matches!(
            delivery(&agent, &sid, "admit").await,
            DeliveryResult::Rejected { .. }
        ));
        assert_delivery_was_not_persisted(&sid).await;
        attach_root(&agent, &sid, cwd.path(), AttachOperation::Load)
            .await
            .unwrap();
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        agent.remove_session(&sid);
    });
}

#[test]
fn cancelled_root_stamp_never_admits_delivery() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        agent.remove_session(&sid);
        let (attach, reached, _release) = with_pause_at(
            AttachPause::AfterInstall,
            agent.load_session_inner(acp::LoadSessionRequest::new(
                sid.clone(),
                cwd.path().to_path_buf(),
            )),
        );
        let mut attach = Box::pin(attach);
        tokio::select! {
            reached = reached => reached.expect("root installed before cancellation"),
            result = &mut attach => panic!("attach finished before its pause: {result:?}"),
        }
        assert_pending_delivery_is_blocked(&agent, &sid).await;
        drop(attach);
        assert_withdrawn(&agent, &sid);
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        assert!(matches!(
            delivery(&agent, &sid, "admit").await,
            DeliveryResult::Rejected { .. }
        ));
        assert_delivery_was_not_persisted(&sid).await;
        attach_root(&agent, &sid, cwd.path(), AttachOperation::Load)
            .await
            .unwrap();
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        agent.remove_session(&sid);
    });
}

async fn delivery_after_superseding_guard_settles(fail_stamp: bool) {
    let agent = build_minimal_agent_for_tests();
    let cwd = tempfile::tempdir().unwrap();
    let sid = new_root_session(&agent, cwd.path()).await;
    agent.remove_session(&sid);
    let (attach, reached, release) = with_pause_at(
        AttachPause::AfterInstall,
        agent.load_session_inner(acp::LoadSessionRequest::new(
            sid.clone(),
            cwd.path().to_path_buf(),
        )),
    );
    let probe = async {
        reached.await.unwrap();
        supersede_with_invalid_load(&agent, &sid).await;
        assert_pending_delivery_is_blocked(&agent, &sid).await;
        crate::session::persistence::FAIL_NEXT_STAMP.set(fail_stamp);
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(attach, probe);
    if fail_stamp {
        result.expect_err("failed stamp withdraws the settled resident");
        assert_withdrawn(&agent, &sid);
        assert!(matches!(
            delivery(&agent, &sid, "admit").await,
            DeliveryResult::Rejected { .. }
        ));
    } else {
        result.expect("matching successful stamp validates the settled resident");
        assert!(agent.session_registry.root_for_session(&sid).is_some());
    }
    assert!(matches!(
        delivery(&agent, &sid, "observe").await,
        DeliveryResult::Absent
    ));
    assert_delivery_was_not_persisted(&sid).await;
    agent.remove_session(&sid);
}

#[test]
fn settled_resident_cannot_deliver_until_its_matching_stamp_lands() {
    run_local_for_bridge_test(|| delivery_after_superseding_guard_settles(false));
}

#[test]
fn settled_resident_with_a_failed_matching_stamp_never_admits_delivery() {
    run_local_for_bridge_test(|| delivery_after_superseding_guard_settles(true));
}

#[test]
fn superseded_stamp_outcomes_cannot_validate_or_withdraw_the_current_receiver() {
    run_local_for_bridge_test(|| async {
        let agent = build_minimal_agent_for_tests();
        let cwd = tempfile::tempdir().unwrap();
        let sid = new_root_session(&agent, cwd.path()).await;
        agent.remove_session(&sid);
        let request = || acp::LoadSessionRequest::new(sid.clone(), cwd.path().to_path_buf());
        let (first, first_reached, first_release) = with_pause_at(
            AttachPause::AfterInstall,
            agent.load_session_inner(request()),
        );
        let replacement = async {
            first_reached.await.unwrap();
            let (stale_id, _) = agent.session_registry.pending_identity_stamp(&sid).unwrap();
            assert_pending_delivery_is_blocked(&agent, &sid).await;
            agent.remove_session(&sid);
            let (second, second_reached, second_release) = with_pause_at(
                AttachPause::AfterInstall,
                agent.load_session_inner(request()),
            );
            let probe = async {
                second_reached.await.unwrap();
                let (current_id, _) = agent.session_registry.pending_identity_stamp(&sid).unwrap();
                assert_ne!(stale_id, current_id);
                for outcome in [IdentityStamp::Durable, IdentityStamp::Failed] {
                    assert!(matches!(
                        agent
                            .session_registry
                            .resolve_identity_stamp(&sid, stale_id, outcome,),
                        StampResolution::Superseded
                    ));
                    assert!(agent.session_host(&sid).is_none());
                    assert_pending_delivery_is_blocked(&agent, &sid).await;
                }
                second_release.send(()).unwrap();
            };
            let (result, ()) = tokio::join!(second, probe);
            result.expect("only the replacement's own stamp validates it");
            assert!(matches!(
                agent.session_registry.resolve_identity_stamp(
                    &sid,
                    stale_id,
                    IdentityStamp::Failed,
                ),
                StampResolution::Superseded
            ));
            assert!(matches!(
                delivery(&agent, &sid, "observe").await,
                DeliveryResult::Absent
            ));
            first_release.send(()).unwrap();
        };
        let (first_result, ()) = tokio::join!(first, replacement);
        first_result.expect_err("the original installer was superseded");
        assert!(agent.is_resident(&sid));
        assert!(agent.session_registry.root_for_session(&sid).is_some());
        assert!(matches!(
            delivery(&agent, &sid, "observe").await,
            DeliveryResult::Absent
        ));
        assert_delivery_was_not_persisted(&sid).await;
        agent.remove_session(&sid);
    });
}
