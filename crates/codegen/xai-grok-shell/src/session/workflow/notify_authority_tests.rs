use super::*;
use crate::session::workflow::tracker::WorkflowTracker;
use tokio::sync::mpsc;

#[tokio::test]
async fn episode_notifier_cannot_reclassify_a_successor_snapshot_as_its_own_publication() {
    let mut tracker = WorkflowTracker::default();
    let first = tracker.start_run("wf_notifier".into(), "notifier".into(), "objective".into(), Vec::new(), Some(10), None);
    let (persistence, mut requests) = mpsc::unbounded_channel();
    let (gateway, _notifications) = mpsc::unbounded_channel();
    let store = WorkflowRunStore::new(None, persistence.clone());
    store.register(&first.run_id, "complete(1);", &serde_json::json!({}), None).unwrap();
    let notify = WorkflowNotifySender::new(
        agent_client_protocol::SessionId::new("test-session"),
        xai_acp_lib::AcpAgentGatewaySender::new(gateway), persistence, store,
    );
    let old_host = notify.for_episode(first.episode());
    tracker.pause_user(&first.run_id, None).unwrap();
    let successor = tracker.resume_run(&first.run_id, None).unwrap();
    old_host.emit(&successor, 0, 0);
    old_host.emit_ephemeral(&successor, 0, 0);
    old_host.broadcast(&successor, 0, 0, true);
    assert!(requests.try_recv().is_err());
    notify.for_episode(successor.episode()).emit_ephemeral(&successor, 0, 0);
    let PersistenceMsg::WorkflowRunState(manifest) = requests.try_recv().unwrap() else {
        panic!("the genuine successor retains publication authority");
    };
    assert_eq!(manifest.state.episode(), successor.episode());
    assert!(requests.try_recv().is_err());
    old_host.emit_ephemeral(&first, 0, 0);
    let PersistenceMsg::WorkflowRunState(manifest) = requests.try_recv().unwrap() else {
        panic!("an old snapshot retains its actual episode identity");
    };
    assert_eq!(manifest.state.episode(), first.episode());
}
