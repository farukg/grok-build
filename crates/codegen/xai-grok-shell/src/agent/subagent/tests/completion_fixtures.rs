use super::*;
use crate::session::persistence::durable_update_tests::{ActorGuard, test_actor};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};

pub(super) fn completion_data_with_initial_meta(
    ctx: &SubagentSpawnContext,
    metadata: SubagentMeta,
    meta_dir: &Path,
) -> ShellCompletionData {
    assert_eq!(metadata.status, SubagentMetaStatus::Running);
    assert!(metadata.completed_at.is_none());
    assert!(write_subagent_meta(meta_dir, &metadata));
    let data = ShellCompletionData::from_context(ctx, None);
    data.state.lock().terminal_artifacts = TerminalArtifacts::StartPublished {
        dir: meta_dir.to_owned(),
        metadata: Box::new(metadata),
    };
    data
}

pub(super) struct DurableCompletionFixture {
    pub(super) directory: tempfile::TempDir,
    pub(super) storage: Arc<JsonlStorageAdapter>,
    pub(super) info: SessionInfo,
    persistence: ActorGuard,
    _transport: tokio_util::task::AbortOnDropHandle<()>,
}

impl Drop for DurableCompletionFixture {
    fn drop(&mut self) {
        self.persistence.task.abort();
    }
}

impl DurableCompletionFixture {
    pub(super) async fn new(ctx: &mut SubagentSpawnContext) -> Self {
        let directory = tempfile::tempdir().expect("durable completion fixture");
        let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(
            directory.path().to_path_buf(),
        ));
        let info = SessionInfo {
            id: acp::SessionId::new(ctx.parent_session_id.clone()),
            cwd: ctx.parent_cwd.to_string_lossy().into_owned(),
        };
        storage
            .init_session(&info, ctx.model_id.clone())
            .await
            .expect("durable parent session");
        let persistence = test_actor(info.clone(), storage.clone());
        let persistence_tx = persistence.handle.tx.clone();
        ctx.parent_persistence_tx = Some(persistence_tx.clone());
        ctx.parent_session_info = Some(info.clone());
        let (sender, mut receive) = mpsc::unbounded_channel();
        ctx.parent_cmd_tx = Some(sender);
        let transport = tokio::spawn(async move {
            while let Some(command) = receive.recv().await {
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
                }
            }
        });
        Self {
            directory,
            storage,
            info,
            persistence,
            _transport: tokio_util::task::AbortOnDropHandle::new(transport),
        }
    }
}
