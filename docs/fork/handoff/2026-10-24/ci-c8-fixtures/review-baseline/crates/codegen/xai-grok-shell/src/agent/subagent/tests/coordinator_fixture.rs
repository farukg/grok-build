use super::*;
use std::cell::RefCell;
use xai_grok_tools::implementations::grok_build::task::backend::ChannelBackend;
use xai_grok_tools::implementations::grok_build::task::coordinator::{
    ChildRunRequest, ChildRunner, CompletionPreparation, CoordinatorConfig, LocalBoxFuture,
    SubagentCoordinator, TerminalCommitResult, TerminalVisibilityResult,
};
use xai_grok_tools::implementations::grok_build::task::root_control::NoRootControl;

pub(super) struct CoordinatorFixture {
    pub(super) backend: ChannelBackend,
    pub(super) classified: mpsc::UnboundedReceiver<ChildCompletion<ShellCompletionData>>,
    pub(super) visible: mpsc::UnboundedReceiver<TerminalVisibilityResult>,
    _task: tokio_util::task::AbortOnDropHandle<()>,
}

impl CoordinatorFixture {
    pub(super) fn new(
        ctx: SubagentSpawnContext,
        state: SubagentState,
        meta_dir: PathBuf,
        gateway: GatewaySender,
    ) -> Self {
        let parent = ctx.parent_session_id.clone();
        let (sender, receiver) = SubagentCoordinator::<Runner>::channel();
        let (classified_tx, classified) = mpsc::unbounded_channel();
        let (visible_tx, visible) = mpsc::unbounded_channel();
        let runner = Runner {
            ctx: RefCell::new(Some(ctx)),
            state,
            meta_dir,
            gateway,
            classified: classified_tx,
            visible: visible_tx,
        };
        let task = tokio::task::spawn_local(
            SubagentCoordinator::from_channel(receiver, runner, CoordinatorConfig::default()).run(),
        );
        Self {
            backend: ChannelBackend::for_coordinator_session(sender, parent),
            classified,
            visible,
            _task: tokio_util::task::AbortOnDropHandle::new(task),
        }
    }
}

struct Runner {
    ctx: RefCell<Option<SubagentSpawnContext>>,
    state: SubagentState,
    meta_dir: PathBuf,
    gateway: GatewaySender,
    classified: mpsc::UnboundedSender<ChildCompletion<ShellCompletionData>>,
    visible: mpsc::UnboundedSender<TerminalVisibilityResult>,
}

impl ChildRunner for Runner {
    type Control = ShellChildRuntime;
    type RootControl = NoRootControl;
    type CompletionData = ShellCompletionData;
    type RunFuture = LocalBoxFuture<ChildRunOutput<ShellCompletionData>>;
    type ValidateFuture = LocalBoxFuture<SubagentValidateTypeOutcome>;
    type DescribeFuture = LocalBoxFuture<SubagentDescribeOutcome>;

    fn run(&self, run: ChildRunRequest<Self::Control>) -> Self::RunFuture {
        let ctx = self
            .ctx
            .borrow_mut()
            .take()
            .expect("single outcome fixture");
        let completion_data = pending_shell_completion_fixture(
            &ctx,
            xai_chat_state::SubagentCompletionId {
                subagent_id: run.request.id.clone(),
                attempt_id: run.attempt_id,
            },
            &self.meta_dir,
        );
        Box::pin(std::future::ready(ChildRunOutput {
            result: SubagentResult {
                state: self.state.clone(),
                subagent_id: run.request.id.clone(),
                child_session_id: run.request.id,
                ..Default::default()
            },
            completion_data,
            snapshot_ref: None,
        }))
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
        completion: CompletionPreparation<'_, Self::CompletionData>,
        committed: Box<dyn FnOnce(TerminalCommitResult) + Send>,
    ) {
        let completion = completion.snapshot();
        self.classified
            .send(completion.clone())
            .expect("classification observer");
        tokio::task::spawn_local(async move {
            committed(match prepare_completion_admission(&completion).await {
                Ok(()) => TerminalCommitResult::Committed,
                Err(error) => TerminalCommitResult::Rejected {
                    error: error.to_string(),
                },
            });
        });
    }

    fn on_published(
        &self,
        completion: ChildCompletion<Self::CompletionData>,
        published: Box<dyn FnOnce(TerminalVisibilityResult) + Send>,
    ) {
        let gateway = self.gateway.clone();
        let visible = self.visible.clone();
        tokio::task::spawn_local(async move {
            let result = publish_committed_completion(completion, gateway).await;
            published(result.clone());
            visible.send(result).expect("publication observer");
        });
    }
}
