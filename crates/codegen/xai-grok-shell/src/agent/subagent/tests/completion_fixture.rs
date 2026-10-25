use super::*;
use crate::session::persistence::{ExplicitSessionOpen, PersistenceHandle};
use crate::session::storage::JsonlStorageAdapter;

pub(crate) fn completion_data_with_initial_meta(
    ctx: &SubagentSpawnContext,
    metadata: SubagentMeta,
    meta_dir: &Path,
) -> ShellCompletionData {
    assert!(matches!(metadata.status, SubagentMetaStatus::Running));
    assert!(write_subagent_meta(meta_dir, &metadata));
    let completion_data = ShellCompletionData::from_context(ctx, None);
    completion_data.state.lock().terminal_artifacts = TerminalArtifacts::StartPublished {
        dir: meta_dir.to_owned(),
        metadata: Box::new(metadata),
    };
    completion_data
}

pub(crate) struct DurableCompletionFixture {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) storage: Arc<JsonlStorageAdapter>,
    pub(crate) info: SessionInfo,
    pub(crate) observed: mpsc::UnboundedReceiver<SessionCommand>,
    _persistence: PersistenceHandle,
    _transport: tokio_util::task::AbortOnDropHandle<()>,
}

impl DurableCompletionFixture {
    pub(crate) async fn new(ctx: &mut SubagentSpawnContext) -> Self {
        let directory = tempfile::tempdir().expect("durable completion fixture");
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(
            directory.path().to_path_buf(),
        ));
        ctx.parent_cwd = directory.path().to_path_buf();
        let info = SessionInfo {
            id: acp::SessionId::new(ctx.parent_session_id.clone()),
            cwd: ctx.parent_cwd.to_string_lossy().into_owned(),
        };
        let client = crate::sampling::Client::new(ctx.sampling_config.clone())
            .expect("fixture sampling client");
        let persistence = crate::session::persistence::new_with_explicit_dir(
            &info,
            directory.path().to_path_buf(),
            ctx.model_id.clone(),
            client,
            ctx.sampling_config.model.clone(),
            ExplicitSessionOpen::New {
                identity: None,
                next_trace_turn: None,
                parent_session_id: None,
            },
        )
        .await
        .expect("durable parent session");
        let persistence_tx = persistence.tx.clone();
        ctx.parent_persistence_tx = Some(persistence_tx.clone());
        ctx.parent_session_info = Some(info.clone());
        let (parent_tx, mut parent_rx) = mpsc::unbounded_channel();
        let (observed_tx, observed) = mpsc::unbounded_channel();
        ctx.parent_cmd_tx = Some(parent_tx);
        let transport = tokio::spawn(async move {
            while let Some(command) = parent_rx.recv().await {
                if let SessionCommand::PublishSubagentCompletion {
                    event,
                    will_wake,
                    respond_to,
                } = command
                {
                    let _ = persistence_tx.send(
                        crate::session::persistence::PersistenceMsg::PublishSubagentCompletion {
                            event,
                            will_wake,
                            respond_to,
                        },
                    );
                } else {
                    let _ = observed_tx.send(command);
                }
            }
        });
        Self {
            directory,
            storage,
            info,
            observed,
            _persistence: persistence,
            _transport: tokio_util::task::AbortOnDropHandle::new(transport),
        }
    }
}
