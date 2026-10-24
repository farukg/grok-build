use super::support::{create_test_actor_ex, drain_gateway, drain_persistence, running_task_stub};
use super::*;

pub(crate) struct CompletionWakeReceiverFixture {
    pub(crate) actor: Arc<SessionActor>,
    pub(crate) sender: mpsc::UnboundedSender<SessionCommand>,
    _task: tokio_util::task::AbortOnDropHandle<()>,
}

pub(crate) async fn completion_wake_receiver_fixture() -> CompletionWakeReceiverFixture {
    let (gateway_tx, gateway_rx) = mpsc::unbounded_channel();
    drain_gateway(gateway_rx);
    let (persistence_tx, persistence_rx) = mpsc::unbounded_channel();
    drain_persistence(persistence_rx);
    let (actor, event_rx) = create_test_actor_ex(0, 256_000, 85, gateway_tx, persistence_tx).await;
    actor.state.lock().await.running_task = Some(running_task_stub("caller-busy"));
    let actor = Arc::new(actor);
    let (sender, commands) = mpsc::unbounded_channel();
    let (chat_sender, chat_events) = mpsc::unbounded_channel();
    let receiver_actor = actor.clone();
    let task = tokio::task::spawn_local(async move {
        let _chat_sender = chat_sender;
        run_session(
            receiver_actor,
            commands,
            chat_events,
            event_rx,
            None,
            Arc::new(parking_lot::Mutex::new(
                xai_grok_workspace::file_system::CodebaseIndexManager::new(),
            )),
            std::path::PathBuf::from("/tmp"),
            crate::session::fs_watch::FsWatchCapabilities::none(),
        )
        .await;
    });
    CompletionWakeReceiverFixture {
        actor,
        sender,
        _task: tokio_util::task::AbortOnDropHandle::new(task),
    }
}
