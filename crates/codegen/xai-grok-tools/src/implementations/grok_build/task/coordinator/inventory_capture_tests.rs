use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use super::*;
use super::super::*;
use super::super::super::admission::{LimitBehavior, SubagentLimits};
use super::super::super::inventory::{
    AttemptEvidence, ChildEvidenceSource, InventoryCancelProof, InventoryTarget, RecordedSubagentOwner,
    RecordedSubagentSpawner, ResidentChildTarget,
};
use super::super::super::terminal::{ChildTerminalEvidence, SubagentTerminalState};
use super::super::super::types::SubagentSpawnRequest;

struct CaptureControl(Arc<AtomicUsize>);

impl ChildControl for CaptureControl {
    type ProgressFuture = std::future::Ready<SubagentProgress>;

    fn progress(&self) -> Self::ProgressFuture {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::ready(SubagentProgress {
            turn_count: 7, tool_call_count: 13, tokens_used: 173,
            context_window_tokens: 1000, context_usage_pct: 17,
            tools_used: vec!["read_file".into()], error_count: 2,
        })
    }

    fn cancel(&self, _: ChildCancellation) -> ChildCancelOutcome {
        ChildCancelOutcome::Accepted
    }
}

struct CaptureRunner;

impl ChildRunner for CaptureRunner {
    type Control = CaptureControl;
    type RootControl = super::super::super::root_control::NoRootControl;
    type CompletionData = ();
    type RunFuture = std::future::Pending<ChildRunOutput<()>>;
    type ValidateFuture = std::future::Pending<SubagentValidateTypeOutcome>;
    type DescribeFuture = std::future::Pending<SubagentDescribeOutcome>;

    fn run(&self, _: ChildRunRequest<CaptureControl>) -> Self::RunFuture {
        std::future::pending()
    }

    fn validate_type(&self, _: String, _: String) -> Self::ValidateFuture {
        std::future::pending()
    }

    fn describe_type(&self, _: String, _: Option<String>, _: String) -> Self::DescribeFuture {
        std::future::pending()
    }

    fn on_completed(&self, _: CompletionPreparation<'_, ()>, published: Box<dyn FnOnce(TerminalCommitResult) + Send>) {
        published(TerminalCommitResult::Committed);
    }

    fn on_published(&self, _: ChildCompletion<()>, published: Box<dyn FnOnce(TerminalVisibilityResult) + Send>) {
        published(TerminalVisibilityResult::Published);
    }
}

type Coordinator = SubagentCoordinator<CaptureRunner>;

fn fixture() -> Coordinator {
    let (_sender, receiver) = SubagentCoordinator::<CaptureRunner>::channel();
    let mut coordinator = SubagentCoordinator::from_channel(receiver, CaptureRunner, CoordinatorConfig {
        limits: SubagentLimits { max_concurrent: 1, behavior: LimitBehavior::Queue },
        ..CoordinatorConfig::default()
    });
    coordinator.graph.begin_inventory("parent").unwrap();
    coordinator
}

fn spawn(coordinator: &mut Coordinator, id: &str) {
    let (result_tx, _result) = oneshot::channel();
    let result_tx = super::super::super::result_delivery::SubagentResultSender::new(result_tx, None);
    coordinator.handle_spawn(SubagentSpawnRequest {
        request: Box::new(super::super::tests::request(id, true)),
        result_tx, registered_tx: None,
    });
}

fn promote(coordinator: &mut Coordinator, id: &str) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let (respond_to, mut response) = oneshot::channel();
    coordinator.handle_internal(InternalEvent::Started {
        subagent_id: id.into(), defer_admission: false, respond_to,
        child: StartedChild {
            child_session_id: id.into(), persona: Some("resolved".into()), resumed_from: None,
            child_cwd: "/project".into(), worktree_path: None, effective_model_id: "model".into(),
            definition_background: false, control: CaptureControl(Arc::clone(&calls)),
        },
    });
    assert!(response.try_recv().unwrap());
    calls
}

fn output(id: &str) -> ChildRunOutput<()> {
    let mut result = SubagentResult::completed(id, id, "actual output");
    result.duration_ms = 193;
    result.tool_calls = 11;
    result.turns = 5;
    result.tokens_used = 971;
    ChildRunOutput { result, completion_data: (), snapshot_ref: None }
}

fn settle(coordinator: &mut Coordinator) {
    while let Ok(publication) = coordinator.terminal_published_rx.try_recv() {
        coordinator.handle_terminal_publication(publication);
    }
}

async fn capture(coordinator: &Coordinator, id: &str) -> RuntimeInventory {
    coordinator.capture_runtime_inventory("parent", &[id.into()]).await.unwrap().runtime.pop().unwrap()
}

fn target(inspection: &SubagentInspection) -> ResidentChildTarget {
    match &inspection.observation {
        ChildObservation::Resident { target, .. } => target.clone(),
        ChildObservation::Terminal { .. } | ChildObservation::IdentityOnly { .. }
        | ChildObservation::Uncustodied { .. } => panic!("resident target required"),
    }
}

#[tokio::test]
async fn actual_enqueue_has_a_target_before_any_read_and_no_execution_attempt() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "pending");
    spawn(&mut coordinator, "queued");
    assert!(coordinator.pending.contains_key("pending"));
    assert!(coordinator.queued.contains_id("queued"));
    let before = coordinator.target_incarnation_for_inventory("queued").unwrap();
    let first = capture(&coordinator, "queued").await;
    let second = capture(&coordinator, "queued").await;
    assert_eq!(before, coordinator.target_incarnation_for_inventory("queued").unwrap());
    assert_eq!(target(&first.inspection), target(&second.inspection));
    assert_eq!(first.inspection.attempt, AttemptEvidence::NotStarted);
    assert_eq!(first.inspection.child_session_id, None);
    assert_eq!(first.inspection.started_at_epoch_ms, None);
    assert_eq!(first.inspection.description.as_deref(), Some("test child"));
    assert_eq!(first.inspection.subagent_type.as_deref(), Some("explore"));
    assert_eq!(first.inspection.recorded_spawner, Some(RecordedSubagentSpawner::Root));
    assert_eq!(first.control_owner, Some(RecordedSubagentOwner::Task));
    assert_eq!(AttemptEvidence::from_recorded(None).unwrap(), AttemptEvidence::Legacy);
    assert_ne!(AttemptEvidence::NotStarted, AttemptEvidence::Legacy);
    assert!(matches!(first.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Initializing, ..
    }));
    let pending = capture(&coordinator, "pending").await;
    assert_eq!(pending.inspection.child_session_id, None);
    assert_eq!(pending.inspection.attempt.known(), Some(&coordinator.pending.get("pending").unwrap().attempt_id));
}

#[tokio::test]
async fn active_capture_asks_the_real_control_once_and_preserves_its_counters() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "active");
    let before = coordinator.target_incarnation_for_inventory("active").unwrap();
    let calls = promote(&mut coordinator, "active");
    assert_eq!(before, coordinator.target_incarnation_for_inventory("active").unwrap());
    let captured = capture(&coordinator, "active").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(captured.inspection.persona.as_deref(), Some("resolved"));
    assert_eq!(captured.inspection.child_session_id.as_deref(), Some("active"));
    assert!(matches!(captured.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Running {
            turn_count: 7, tool_call_count: 13, tokens_used: 173,
            context_window_tokens: 1000, context_usage_pct: 17,
            error_count: 2, ref tools_used,
        }, ..
    } if tools_used == &["read_file"]));
    capture(&coordinator, "active").await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn finishing_committing_publishing_are_custody_and_only_published_is_terminal() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    promote(&mut coordinator, "child");
    coordinator.terminal_outputs.insert("child".into(), TerminalOutput::Finishing(output("child")));
    let finishing = capture(&coordinator, "child").await;
    assert!(matches!(finishing.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Settling }, ..
    }));
    coordinator.terminal_outputs.remove("child");
    coordinator.finish_child("child", output("child"));
    let committing = capture(&coordinator, "child").await;
    assert!(matches!(committing.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Writing }, ..
    }));
    let prepared = coordinator.terminal_published_rx.try_recv().unwrap();
    coordinator.handle_terminal_publication(prepared);
    coordinator.completed.get_mut("child").unwrap().publication =
        super::super::super::coordinator_state::TerminalPublicationState::Published;
    let publishing = capture(&coordinator, "child").await;
    assert!(matches!(publishing.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Publishing }, ..
    }));
    settle(&mut coordinator);
    assert!(coordinator.terminal_outputs.is_empty());
    let finished = capture(&coordinator, "child").await;
    match finished.inspection.observation {
        ChildObservation::Terminal { terminal: ChildTerminalEvidence::Producer { result, sources, completion_id } } => {
            assert_eq!(result.output.as_ref(), "actual output");
            assert_eq!((result.tool_calls, result.turns, result.tokens_used, result.duration_ms), (11, 5, 971, 193));
            assert_eq!(result.state, SubagentTerminalState::Completed);
            assert_eq!(completion_id.attempt_id, coordinator.completed.get("child").unwrap().attempt_id);
            assert_eq!(sources, vec![ChildEvidenceSource::Coordinator {
                version: coordinator.graph.inventory_source_version("parent").unwrap(),
            }]);
        }
        other => panic!("published producer required: {other:?}"),
    }
}

#[tokio::test]
async fn current_finalization_beats_a_stale_completed_record_for_the_same_child_id() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    promote(&mut coordinator, "child");
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    let stale = coordinator.completed.remove("child").unwrap();
    coordinator.start_child(super::super::tests::request("child", true), None, None,
        StartOrigin::Direct, None, None, None, None);
    promote(&mut coordinator, "child");
    let newer = coordinator.active.get("child").unwrap().attempt_id.clone();
    coordinator.finish_child("child", output("child"));
    coordinator.completed.insert("child".into(), stale);
    let current = capture(&coordinator, "child").await;
    assert_eq!(current.inspection.attempt, AttemptEvidence::Known(newer));
    assert!(matches!(current.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Writing }, ..
    }));
}

#[tokio::test]
async fn absent_publishing_metadata_is_an_error_not_a_terminal_or_a_minted_target() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    promote(&mut coordinator, "child");
    coordinator.finish_child("child", output("child"));
    let prepared = coordinator.terminal_published_rx.try_recv().unwrap();
    coordinator.handle_terminal_publication(prepared);
    let before = coordinator.target_incarnation_for_inventory("child").unwrap();
    coordinator.completed.remove("child");
    assert!(matches!(coordinator.capture_runtime_inventory("parent", &["child".into()]).await,
        Err(SubagentAccessError::InventoryUnreadable { issues })
        if issues == vec![InventoryIssue::RuntimeCaptureUnavailable { subagent_id: "child".into() }]));
    assert_eq!(before, coordinator.target_incarnation_for_inventory("child").unwrap());
}

#[tokio::test]
async fn output_eviction_keeps_real_producer_facts_without_invented_output_or_completion_time() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    let child = coordinator.completed.get_mut("child").unwrap();
    child.persisted_output_ref = Some("resident-reference-only".into());
    child.result.output = Arc::from("");
    match capture(&coordinator, "child").await.inspection.observation {
        ChildObservation::Terminal { terminal: ChildTerminalEvidence::ProducerFact { recorded: Some(recorded), .. } } => {
            assert_eq!(recorded.output, None);
            assert_eq!(recorded.completed_at, None);
            assert_eq!(recorded.duration_ms, Some(193));
            assert_eq!(recorded.tool_calls, Some(11));
            assert_eq!(recorded.turns, Some(5));
            assert_eq!(recorded.tokens_used, Some(971));
        }
        other => panic!("compact producer fact required: {other:?}"),
    }
}

#[tokio::test]
async fn a_queued_observation_cannot_match_the_later_execution_even_with_the_same_child_id() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "blocker");
    spawn(&mut coordinator, "child");
    let observed = capture(&coordinator, "child").await;
    let proof = InventoryCancelProof {
        version: coordinator.graph.inventory_source_version("parent").unwrap(),
        target: InventoryTarget::Resident { target: target(&observed.inspection) },
    };
    coordinator.finish_child("blocker", output("blocker"));
    let current = capture(&coordinator, "child").await;
    assert!(current.inspection.attempt.known().is_some());
    assert!(!coordinator.graph.matches_inventory_proof("parent", &proof,
        &InventoryTarget::Resident { target: target(&current.inspection) }));
    assert!(!coordinator.pending.get("child").unwrap().cancellation.is_cancelled());
}

#[tokio::test]
async fn authorized_wake_replaces_the_target_and_rollback_restores_the_original_identity() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    let original = coordinator.target_incarnation_for_inventory("child").unwrap();
    assert!(matches!(coordinator.resolve_owned_child_id("child", "foreign", None),
        super::super::wake::ResolvedSend::Fail(_)));
    assert_eq!(original, coordinator.target_incarnation_for_inventory("child").unwrap());
    let proposed = coordinator.begin_wake_incarnation("child");
    assert_eq!(original, coordinator.target_incarnation_for_inventory("child").unwrap());
    let displaced = coordinator.completed.remove("child").unwrap();
    coordinator.start_child(super::super::tests::request("child", true), None, None,
        StartOrigin::Direct, None, None, None,
        Some(DisplacedCompletedChild { completed: Box::new(displaced) }));
    let wake = capture(&coordinator, "child").await;
    assert_ne!(target(&wake.inspection).incarnation, original);
    assert_eq!(coordinator.begin_wake_incarnation("child"), proposed);
    coordinator.finish_child("child", ChildRunOutput {
        result: SubagentResult::failed("child", "child", "host refused wake"),
        completion_data: (), snapshot_ref: None,
    });
    assert_eq!(original, coordinator.target_incarnation_for_inventory("child").unwrap());
    assert!(coordinator.completed.get("child").unwrap().publication.is_published());
}

#[tokio::test]
async fn absent_target_is_not_repaired_by_capture_and_a_foreign_parent_is_rejected() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    coordinator.clear_target_incarnation("child");
    assert!(matches!(coordinator.capture_runtime_inventory("parent", &["child".into()]).await,
        Err(SubagentAccessError::InventoryUnreadable { .. })));
    assert_eq!(coordinator.target_incarnation_for_inventory("child"), None);
    coordinator.graph.begin_inventory("foreign").unwrap();
    let _ = coordinator.current_target_incarnation("child");
    assert!(matches!(coordinator.capture_runtime_inventory("foreign", &["child".into()]).await,
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::IdentityMismatch })));
}

#[tokio::test]
async fn cancelled_queue_keeps_positive_no_start_evidence_without_a_fake_execution_or_time() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "blocker");
    spawn(&mut coordinator, "queued");
    let before = coordinator.target_incarnation_for_inventory("queued").unwrap();
    let cause = InterruptionCause::Error { message: "withdrawn before admission".into() };
    assert_eq!(coordinator.remove_queued(&cause, |request| request.id == "queued"), 1);
    assert_eq!(before, coordinator.target_incarnation_for_inventory("queued").unwrap());
    let committing = capture(&coordinator, "queued").await;
    assert_eq!(committing.inspection.attempt, AttemptEvidence::NotStarted);
    assert_eq!(committing.inspection.child_session_id, None);
    assert_eq!(committing.inspection.started_at_epoch_ms, None);
    settle(&mut coordinator);
    let completed = capture(&coordinator, "queued").await;
    assert_eq!(completed.inspection.attempt, AttemptEvidence::NotStarted);
    assert_eq!(completed.inspection.child_session_id, None);
    assert_eq!(completed.inspection.started_at_epoch_ms, None);
    match completed.inspection.observation {
        ChildObservation::Terminal { terminal: ChildTerminalEvidence::Recorded { result } } => {
            assert_eq!(result.attempt, AttemptEvidence::NotStarted);
            assert_eq!(result.completed_at, None);
            assert_eq!(result.state, super::super::super::terminal::RecordedTerminalState::Cancelled {
                cause: Some(cause), message: None,
            });
        }
        other => panic!("admission-only terminal evidence required: {other:?}"),
    }
}

#[tokio::test]
async fn a_new_execution_fences_an_observed_stop_without_changing_the_child_id() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    let before = capture(&coordinator, "child").await;
    let proof = InventoryCancelProof {
        version: coordinator.graph.inventory_source_version("parent").unwrap(),
        target: InventoryTarget::Resident { target: target(&before.inspection) },
    };
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    coordinator.completed.remove("child");
    coordinator.start_child(super::super::tests::request("child", true), None, None,
        StartOrigin::Direct, None, None, None, None);
    let current = coordinator.runtime_inventory_target("child").unwrap().unwrap();
    assert_ne!(current.attempt, before.inspection.attempt);
    assert!(!coordinator.graph.matches_inventory_proof("parent", &proof,
        &InventoryTarget::Resident { target: current }));
    assert!(!coordinator.pending.get("child").unwrap().cancellation.is_cancelled());
}

#[tokio::test]
async fn keyed_queue_retains_admission_order_and_only_promotes_with_capacity() {
    let mut coordinator = fixture();
    for id in ["blocker", "first", "second"] {
        spawn(&mut coordinator, id);
    }
    assert!(coordinator.queued.get("first").is_some());
    assert!(coordinator.queued.get("second").is_some());
    coordinator.finish_child("blocker", output("blocker"));
    assert!(coordinator.pending.contains_key("first"));
    assert!(coordinator.queued.get("first").is_none());
    assert!(coordinator.queued.get("second").is_some());
    coordinator.finish_child("first", output("first"));
    assert!(coordinator.pending.contains_key("second"));
    assert!(coordinator.queued.is_empty());
}

#[tokio::test]
async fn nested_workflow_custody_keeps_the_root_audience_and_real_spawner() {
    let mut coordinator = fixture();
    let mut request = super::super::tests::request("spawner", true);
    request.owner = super::super::super::types::SubagentOwner::workflow("run");
    coordinator.graph.insert_root_child("spawner", "parent");
    coordinator.start_child(request, None, None, StartOrigin::Direct, None, None, None, None);
    promote(&mut coordinator, "spawner");
    let mut nested = super::super::tests::request("nested", true);
    nested.parent_session_id = "spawner".into();
    let (result_tx, _result) = oneshot::channel();
    coordinator.handle_spawn(SubagentSpawnRequest {
        request: Box::new(nested), registered_tx: None,
        result_tx: super::super::super::result_delivery::SubagentResultSender::new(result_tx, None),
    });
    let actual = capture(&coordinator, "nested").await;
    assert_eq!(actual.inspection.parent_session_id, "parent");
    assert_eq!(actual.inspection.recorded_spawner, Some(RecordedSubagentSpawner::Child {
        subagent_id: "spawner".into(), session_id: "spawner".into(),
    }));
    assert_eq!(actual.control_owner, Some(RecordedSubagentOwner::Workflow { run_id: "run".into() }));
}

#[tokio::test]
async fn owned_progress_capture_does_not_borrow_the_actor_and_keeps_its_real_source_fence() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    let calls = promote(&mut coordinator, "child");
    let version = coordinator.graph.inventory_source_version("parent").unwrap();
    let pending = coordinator.capture_runtime_inventory("parent", &["child".into()]);
    coordinator.graph.begin_inventory_release("parent").unwrap();
    let captured = pending.await.unwrap();
    assert_eq!(captured.version, version);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(coordinator.graph.inventory_entry("parent", "child", captured.runtime.first()),
        Err(SubagentAccessError::InventoryReleasing)));
    assert!(matches!(coordinator.capture_runtime_inventory("parent", &["child".into()]).await,
        Err(SubagentAccessError::InventoryReleasing)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_successful_pending_wake_keeps_the_new_identity_when_no_started_callback_arrives() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    let original = coordinator.target_incarnation_for_inventory("child").unwrap();
    coordinator.begin_wake_incarnation("child");
    let previous = coordinator.completed.remove("child").unwrap();
    coordinator.start_child(super::super::tests::request("child", true), None, None,
        StartOrigin::Direct, None, None, None,
        Some(DisplacedCompletedChild { completed: Box::new(previous) }));
    let pending = coordinator.target_incarnation_for_inventory("child").unwrap();
    assert_ne!(pending, original);
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    assert_eq!(coordinator.target_incarnation_for_inventory("child"), Some(pending));
    assert!(coordinator.completed.get("child").unwrap().publication.is_published());
}

#[tokio::test]
async fn a_progress_response_from_an_old_attempt_cannot_become_a_new_attempt_observation() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    promote(&mut coordinator, "child");
    let old_attempt = coordinator.active.get("child").unwrap().attempt_id.clone();
    let pending = coordinator.capture_runtime_inventory("parent", &["child".into()]);
    coordinator.finish_child("child", output("child"));
    settle(&mut coordinator);
    coordinator.completed.remove("child");
    coordinator.start_child(super::super::tests::request("child", true), None, None,
        StartOrigin::Direct, None, None, None, None);
    let captured = pending.await.unwrap();
    let old = captured.runtime.first().unwrap();
    assert_eq!(old.inspection.attempt, AttemptEvidence::Known(old_attempt));
    let proof = InventoryCancelProof {
        version: captured.version,
        target: InventoryTarget::Resident { target: target(&old.inspection) },
    };
    assert!(!coordinator.graph.matches_inventory_proof("parent", &proof, &InventoryTarget::Resident {
        target: coordinator.runtime_inventory_target("child").unwrap().unwrap(),
    }));
}

#[tokio::test]
async fn a_running_result_in_the_published_map_is_not_invented_as_a_failed_terminal() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "child");
    let mut invalid_output = output("child");
    invalid_output.result.state = SubagentState::Running;
    coordinator.finish_child("child", invalid_output);
    settle(&mut coordinator);
    assert!(matches!(coordinator.capture_runtime_inventory("parent", &["child".into()]).await,
        Err(SubagentAccessError::InvalidInventory { reason: InventoryValidationError::InvalidEvidence })));
}

#[tokio::test]
async fn committing_custody_carries_no_start_origin_even_without_the_completed_map() {
    let mut coordinator = fixture();
    spawn(&mut coordinator, "blocker");
    spawn(&mut coordinator, "queued");
    let cause = InterruptionCause::Error { message: "withdrawn".into() };
    coordinator.remove_queued(&cause, |request| request.id == "queued");
    coordinator.completed.remove("queued");
    let captured = capture(&coordinator, "queued").await;
    assert_eq!(captured.inspection.attempt, AttemptEvidence::NotStarted);
    assert_eq!(captured.inspection.child_session_id, None);
    assert_eq!(captured.inspection.started_at_epoch_ms, None);
    assert!(matches!(captured.inspection.observation, ChildObservation::Resident {
        status: SubagentSnapshotStatus::Finalizing { publication: SubagentFinalization::Writing }, ..
    }));
}
