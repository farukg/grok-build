use tokio::sync::mpsc;

use super::*;
use crate::implementations::grok_build::task::TaskTool;
use crate::implementations::grok_build::task::backend::{ChannelBackend, SubagentBackend};
use crate::implementations::grok_build::task::coordinator::{
    ActiveMessageAdmission, ChildCompletion, ChildControl, ChildRunOutput, ChildRunRequest,
    ChildRunner, SendBoxFuture, StartedChild, SubagentCoordinator, SubagentProgress,
};
use crate::implementations::grok_build::task::types::{
    ActiveAgentMessageDelivery, CurrentPromptIdResource, SessionIdResource, SubagentDepthCounter,
    SubagentDescribeOutcome, SubagentResult, SubagentValidateTypeOutcome, TaskModelValidator,
};
use crate::types::output::ToolOutput;
use crate::types::resources::Resources;
use crate::types::tool_metadata::test_ctx;

const TEST_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
const PARENT: &str = "parent";
const HOLD: &str = "hold";

async fn completes<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(TEST_WAIT, future)
        .await
        .expect("resume test operation timed out")
}

struct RecordingControl {
    admitted: mpsc::UnboundedSender<(ActiveAgentMessageOperation, String)>,
}

impl ChildControl for RecordingControl {
    type ProgressFuture = std::future::Ready<SubagentProgress>;

    fn progress(&self) -> Self::ProgressFuture {
        std::future::ready(SubagentProgress::default())
    }

    fn send_active_message(
        &self,
        delivery: ActiveAgentMessageDelivery,
    ) -> SendBoxFuture<ActiveMessageAdmission> {
        let record = (delivery.operation(), delivery.message().text.to_string());
        let admitted = delivery.commit_admission(|| self.admitted.send(record).is_ok());
        Box::pin(std::future::ready(if admitted == Some(true) {
            ActiveMessageAdmission::Admitted
        } else {
            ActiveMessageAdmission::Rejected
        }))
    }

    fn cancel(&self) {}
}

/// A child prompted with [`HOLD`] stays running; any other child finishes at once.
struct RecordingRunner {
    runs: mpsc::UnboundedSender<SubagentRequest>,
    started: mpsc::UnboundedSender<String>,
    admitted: mpsc::UnboundedSender<(ActiveAgentMessageOperation, String)>,
}

impl ChildRunner for RecordingRunner {
    type Control = RecordingControl;
    type RootControl = crate::implementations::grok_build::task::root_control::NoRootControl;
    type CompletionData = ();
    type RunFuture = SendBoxFuture<ChildRunOutput<()>>;
    type ValidateFuture = SendBoxFuture<SubagentValidateTypeOutcome>;
    type DescribeFuture = SendBoxFuture<SubagentDescribeOutcome>;

    fn run(&self, run: ChildRunRequest<Self::Control>) -> Self::RunFuture {
        let runs = self.runs.clone();
        let started = self.started.clone();
        let admitted = self.admitted.clone();
        Box::pin(async move {
            let request = run.request;
            let _ = runs.send(request.clone());
            let _ = run
                .reporter
                .started(StartedChild {
                    child_session_id: request.id.clone(),
                    persona: None,
                    resumed_from: request.resume_from.clone(),
                    child_cwd: String::new(),
                    worktree_path: None,
                    effective_model_id: "test-model".to_owned(),
                    definition_background: false,
                    control: RecordingControl { admitted },
                })
                .await;
            let _ = started.send(request.id.clone());
            if request.prompt == HOLD {
                std::future::pending::<()>().await;
            }
            ChildRunOutput {
                result: SubagentResult {
                    success: true,
                    output: "done".into(),
                    subagent_id: request.id.clone(),
                    child_session_id: request.id.clone(),
                    ..Default::default()
                },
                completion_data: (),
                snapshot_ref: None,
            }
        })
    }

    fn validate_type(&self, _: String, _: String) -> Self::ValidateFuture {
        Box::pin(std::future::ready(SubagentValidateTypeOutcome::Ok))
    }

    fn describe_type(&self, _: String, _: Option<String>, _: String) -> Self::DescribeFuture {
        Box::pin(std::future::ready(SubagentDescribeOutcome::Unavailable))
    }

    fn supports_wake(&self) -> bool {
        true
    }

    fn on_completed(
        &self,
        _: ChildCompletion<Self::CompletionData>,
        terminal_published: Box<dyn FnOnce() + Send>,
    ) {
        terminal_published();
    }
}

struct Harness {
    sender: crate::implementations::grok_build::task::backend::SubagentCoordinatorSender,
    backend: ChannelBackend,
    runs: mpsc::UnboundedReceiver<SubagentRequest>,
    started: mpsc::UnboundedReceiver<String>,
    admitted: mpsc::UnboundedReceiver<(ActiveAgentMessageOperation, String)>,
    actor: tokio::task::JoinHandle<()>,
}

fn harness() -> Harness {
    let (sender, receiver) = SubagentCoordinator::<RecordingRunner>::channel();
    let (runs_tx, runs) = mpsc::unbounded_channel();
    let (started_tx, started) = mpsc::unbounded_channel();
    let (admitted_tx, admitted) = mpsc::unbounded_channel();
    let actor = tokio::spawn(
        SubagentCoordinator::from_channel(
            receiver,
            RecordingRunner {
                runs: runs_tx,
                started: started_tx,
                admitted: admitted_tx,
            },
            Default::default(),
        )
        .run(),
    );
    Harness {
        backend: ChannelBackend::for_coordinator_session(sender.clone(), PARENT),
        sender,
        runs,
        started,
        admitted,
        actor,
    }
}

impl Harness {
    /// Spawn a child with `id`; a [`HOLD`] prompt keeps it running.
    async fn spawn_child(&mut self, id: &str, prompt: &str) {
        let mut request =
            human_resume_request(String::new(), prompt.to_owned(), PARENT.to_owned());
        request.id = id.to_owned();
        request.resume_from = None;
        let backend = self.backend.clone();
        let spawn = tokio::spawn(async move { backend.spawn(request, None).await });
        let _ = completes(self.runs.recv()).await;
        assert_eq!(completes(self.started.recv()).await.as_deref(), Some(id));
        if prompt != HOLD {
            completes(spawn).await.expect("join").expect("spawn");
        }
    }

    async fn run_task(
        &self,
        resume_from: &str,
        prompt: &str,
    ) -> Result<ToolOutput, xai_tool_runtime::ToolError> {
        let mut resources = Resources::new();
        resources.insert(self.backend.clone().into_resource());
        resources.insert(SubagentDepthCounter(0));
        resources.insert(SessionIdResource(PARENT.to_owned()));
        resources.insert(CurrentPromptIdResource("prompt-1".to_owned()));
        resources.insert(TaskModelValidator::new(|_| None));
        let mut input: xai_tool_types::TaskToolInput = serde_json::from_value(serde_json::json!({
            "description": "continue",
            "prompt": prompt,
            "run_in_background": false,
        }))
        .expect("task input");
        input.resume_from = Some(resume_from.to_owned());
        completes(xai_tool_runtime::Tool::run(
            &TaskTool,
            test_ctx(resources.into_shared()),
            input,
        ))
        .await
    }
}

const RUNNING_ID: &str = "019b1111-aaaa-7000-8000-000000000001";
const FINISHED_ID: &str = "019b2222-bbbb-7000-8000-000000000002";
const TWIN_A: &str = "019b3333-cccc-7000-8000-00000000000a";
const TWIN_B: &str = "019b3333-cccc-7000-8000-00000000000b";

#[tokio::test]
async fn resume_running_target_delivers_as_queued_message() {
    let mut harness = harness();
    harness.spawn_child(RUNNING_ID, HOLD).await;

    let output = harness
        .run_task(RUNNING_ID, "also check the tests")
        .await
        .expect("a running target takes the prompt");

    assert!(matches!(output, ToolOutput::Text(_)), "{output:?}");
    assert_eq!(
        completes(harness.admitted.recv()).await,
        Some((
            ActiveAgentMessageOperation::Queue,
            "also check the tests".to_owned()
        ))
    );
    assert!(
        harness.runs.try_recv().is_err(),
        "delivering to a running child must not spawn another"
    );
    harness.actor.abort();
}

#[tokio::test]
async fn resume_by_unique_prefix_wakes_the_finished_child_under_its_own_id() {
    let mut harness = harness();
    harness.spawn_child(FINISHED_ID, "work").await;
    harness.spawn_child(RUNNING_ID, HOLD).await;

    let prefix = &FINISHED_ID[..MIN_SUBAGENT_ID_PREFIX_LEN + 2];
    let output = harness
        .run_task(prefix, "continue")
        .await
        .expect("a unique prefix resumes");

    assert!(matches!(output, ToolOutput::Text(_)), "{output:?}");
    let woken = completes(harness.runs.recv()).await.expect("wake run");
    assert_eq!(woken.id, FINISHED_ID, "the same subagent continues");
    assert_eq!(woken.prompt, "continue");
    assert!(
        harness.runs.try_recv().is_err(),
        "waking a finished child must not start another"
    );
    harness.actor.abort();
}

#[tokio::test]
async fn resume_ambiguous_prefix_is_typed_error() {
    let mut harness = harness();
    harness.spawn_child(TWIN_A, "work").await;
    harness.spawn_child(TWIN_B, "work").await;

    let shared = &TWIN_A[..MIN_SUBAGENT_ID_PREFIX_LEN + 4];
    assert_eq!(
        harness.backend.resolve_resume(shared, PARENT).await,
        Err(SubagentResumeError::Ambiguous {
            reference: shared.to_owned(),
            candidates: vec![TWIN_A.to_owned(), TWIN_B.to_owned()],
        })
    );
    let error = harness
        .run_task(shared, "continue")
        .await
        .expect_err("an ambiguous prefix must not resume");
    assert!(error.detail.contains(TWIN_A) && error.detail.contains(TWIN_B));
    assert!(harness.runs.try_recv().is_err());
    harness.actor.abort();
}

#[tokio::test]
async fn resume_of_another_parents_child_is_foreign() {
    let mut harness = harness();
    harness.spawn_child(FINISHED_ID, "work").await;

    let unbound = ChannelBackend::from_coordinator(harness.sender.clone());
    assert_eq!(
        unbound.resolve_resume(FINISHED_ID, "someone-else").await,
        Err(SubagentResumeError::ForeignParent {
            subagent_id: FINISHED_ID.to_owned(),
        })
    );
    harness.actor.abort();
}
