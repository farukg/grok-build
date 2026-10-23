use super::*;
use crate::session::chat_persistence::ChannelChatPersistence;
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter};
use xai_chat_state::{
    ChatPersistence, CompletionProjectionAck, CompletionProjectionError, InputProjection,
    StrictAppendAck, StrictAppendError,
};

struct DurableChatFixture {
    channel: ChannelChatPersistence,
    actor: crate::session::persistence::durable_update_tests::ActorGuard,
    _directory: tempfile::TempDir,
}
impl Drop for DurableChatFixture {
    fn drop(&mut self) {
        self.actor.task.abort();
    }
}
impl ChatPersistence for DurableChatFixture {
    fn persist_message(&mut self, item: &ConversationItem) {
        self.channel.persist_message(item);
    }
    fn persist_working_directory_switch_and_ack(
        &mut self,
        item: &ConversationItem,
    ) -> oneshot::Receiver<Result<StrictAppendAck, StrictAppendError>> {
        self.channel.persist_working_directory_switch_and_ack(item)
    }
    fn persist_input_and_ack(
        &mut self,
        projection: &InputProjection,
        admission: &xai_chat_state::InputAdmission,
    ) -> oneshot::Receiver<Result<CompletionProjectionAck, CompletionProjectionError>> {
        self.channel.persist_input_and_ack(projection, admission)
    }
    fn replace_history(&mut self, items: &[ConversationItem]) {
        self.channel.replace_history(items);
    }
    fn replace_history_for_strip_and_ack(
        &mut self,
        items: &[ConversationItem],
    ) -> oneshot::Receiver<std::io::Result<()>> {
        self.channel.replace_history_for_strip_and_ack(items)
    }
    fn flush(&mut self) {
        self.channel.flush();
    }
}

pub(crate) async fn create_completion_test_actor(
    total_tokens: u64,
    context_window: u64,
    threshold_percent: u8,
    gateway_tx: mpsc::UnboundedSender<xai_acp_lib::AcpClientMessage>,
) -> SessionActor {
    let directory = tempfile::tempdir().expect("durable chat fixture");
    let storage = Arc::new(JsonlStorageAdapter::with_explicit_session_dir(
        directory.path().to_path_buf(),
    ));
    let info = SessionInfo {
        id: acp::SessionId::new("test-actor"),
        cwd: "/tmp".into(),
    };
    storage
        .init_session(&info, acp::ModelId::new("test"))
        .await
        .expect("fixture session");
    let actor = crate::session::persistence::durable_update_tests::test_actor(info, storage);
    let channel = ChannelChatPersistence::new(actor.handle.tx.clone());
    create_test_actor_with_chat_persistence(
        total_tokens,
        context_window,
        threshold_percent,
        gateway_tx,
        actor.handle.tx.clone(),
        Box::new(DurableChatFixture {
            channel,
            actor,
            _directory: directory,
        }),
    )
    .await
    .0
}
