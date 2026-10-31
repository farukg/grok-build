use super::*;

#[test]
fn restored_execution_identity_is_durable_and_legacy_manifests_start_at_the_original_epoch() {
    let mut tracker = WorkflowTracker::default();
    let first = tracker.start_run("wf_identity".into(), "identity".into(), "objective".into(), Vec::new(), Some(10), None);
    let mut legacy = serde_json::to_value(&first).unwrap();
    legacy.as_object_mut().unwrap().remove("execution_epoch");
    let legacy: WorkflowRunState = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.episode(), first.episode());
    for epoch in 1..=3 {
        tracker.pause_user(&first.run_id, None).unwrap();
        assert_eq!(tracker.resume_run(&first.run_id, None).unwrap().execution_epoch, epoch);
    }
    let paused = tracker.pause_user(&first.run_id, None).unwrap();
    let bytes = serde_json::to_vec(&paused).unwrap();
    let mut restored = WorkflowTracker::from_snapshot(vec![serde_json::from_slice(&bytes).unwrap()]);
    assert_eq!(restored.execution_epoch(&first.run_id), Some(3));
    let successor = restored.resume_run(&first.run_id, None).unwrap();
    assert_eq!(successor.execution_epoch, 4);
    assert_ne!(successor.episode(), paused.episode());
}

#[test]
fn exhausted_execution_identity_refuses_resume_without_mutating_the_run() {
    let mut tracker = WorkflowTracker::default();
    let first = tracker.start_run("wf_exhausted".into(), "exhausted".into(), "objective".into(), Vec::new(), Some(10), None);
    let mut paused = tracker.pause_user(&first.run_id, None).unwrap();
    paused.execution_epoch = u64::MAX;
    let mut tracker = WorkflowTracker::from_snapshot(vec![paused.clone()]);
    assert!(tracker.resume_run(&first.run_id, Some(20)).is_none());
    let retained = tracker.get(&first.run_id).unwrap();
    assert_eq!(retained.episode(), paused.episode());
    assert_eq!(retained.status, paused.status);
    assert_eq!(retained.revision, paused.revision);
    assert_eq!(retained.agent_budget, paused.agent_budget);
}
