use super::*;
use agent_client_protocol as acp;
use crate::agent::update_chunk_merge::{BufferingSettings, ReplayBuffer};
use crate::session::storage::{JsonlStorageAdapter, StorageAdapter, SessionUpdate};

fn origin(id: &str) -> OutputSourceOrigin {
    OutputSourceOrigin { session_id: "binding-session".into(), request_id: "actual-request".into(),
        reference: xai_grok_sampling_types::OutputOrigin::Item { reference: xai_grok_sampling_types::ProviderOutputRef::Item(id.into()) } }
}

fn chunk(text: &str, origin: Option<OutputSourceOrigin>) -> acp::SessionNotification {
    acp::SessionNotification::new(acp::SessionId::new("binding-session"), acp::SessionUpdate::AgentMessageChunk(
        acp::ContentChunk::new(acp::ContentBlock::from(text)).meta(origin.map(OutputSourceOrigin::into_meta)),
    ))
}

fn assert_aggregate(notification: &acp::SessionNotification) {
    let acp::SessionUpdate::AgentMessageChunk(chunk) = &notification.update else { panic!("text chunk"); };
    let acp::ContentBlock::Text(text) = &chunk.content else { panic!("text body"); };
    assert_eq!(text.text, "ABC");
    let wire = chunk.meta.as_ref().unwrap().get(OUTPUT_ORIGIN_META).unwrap();
    let StreamOutputOrigin::Aggregate { origins, unidentified } = serde_json::from_value(wire.clone()).unwrap() else { panic!("merged chunks cannot retain a false single-item identity"); };
    assert_eq!(origins, vec![origin("one"), origin("two")]);
    assert_eq!(unidentified, 1);
}

#[test]
fn live_chunk_buffer_preserves_all_structural_origins_and_missing_origin_uncertainty() {
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings { max_items: 100, max_bytes: 100_000, max_duration_ms: 100 }));
    for notification in [chunk("A", Some(origin("one"))), chunk("B", Some(origin("two"))), chunk("C", None)] {
        assert!(buffer.consume_chunk(notification).is_none());
    }
    assert_aggregate(&buffer.flush().unwrap().into_acp());
    assert!(buffer.flush().is_none());
}

#[test]
fn structural_metadata_keeps_thinking_tool_thinking_order() {
    let thought = |text: &str, id: &str| acp::SessionNotification::new(acp::SessionId::new("binding-session"), acp::SessionUpdate::AgentThoughtChunk(
        acp::ContentChunk::new(acp::ContentBlock::from(text)).meta(Some(origin(id).into_meta())),
    ));
    let tool = acp::SessionNotification::new(acp::SessionId::new("binding-session"), acp::SessionUpdate::ToolCall(
        acp::ToolCall::new(acp::ToolCallId::new("actual-call"), "read_file"),
    ));
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings { max_items: 100, max_bytes: 100_000, max_duration_ms: 100 }));
    let mut emitted = Vec::new();
    for notification in [thought("before", "reasoning-one"), tool, thought("after", "reasoning-two")] {
        if let Some((first, second)) = buffer.consume_chunk(notification) {
            emitted.push(first.into_acp());
            if let Some(second) = second { emitted.push(second.into_acp()); }
        }
    }
    if let Some(last) = buffer.flush() { emitted.push(last.into_acp()); }
    assert_eq!(emitted.len(), 3);
    assert!(matches!(emitted[0].update, acp::SessionUpdate::AgentThoughtChunk(_)));
    assert!(matches!(emitted[1].update, acp::SessionUpdate::ToolCall(_)));
    assert!(matches!(emitted[2].update, acp::SessionUpdate::AgentThoughtChunk(_)));
}

#[tokio::test]
async fn persistence_batching_keeps_aggregate_origins_without_turning_each_token_into_a_write() {
    let directory = tempfile::tempdir().unwrap();
    let storage = std::sync::Arc::new(JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned()));
    let info = crate::session::info::Info { id: acp::SessionId::new("binding-session"), cwd: "/tmp".into() };
    storage.init_session(&info, acp::ModelId::new("test")).await.unwrap();
    let persistence = crate::session::persistence::durable_update_tests::test_actor(info.clone(), storage);
    let _persistence = tokio_util::task::AbortOnDropHandle::new(persistence.task);
    for notification in [chunk("A", Some(origin("one"))), chunk("B", Some(origin("two"))), chunk("C", None)] {
        persistence.handle.tx.send(crate::session::persistence::PersistenceMsg::Update(SessionUpdate::Acp(Box::new(notification)))).unwrap();
    }
    let (respond_to, response) = tokio::sync::oneshot::channel();
    persistence.handle.tx.send(crate::session::persistence::PersistenceMsg::FlushAndAck { respond_to }).unwrap();
    response.await.unwrap().unwrap();
    let reopened = JsonlStorageAdapter::with_explicit_session_dir(directory.path().to_owned());
    let restored = reopened.load_session(&info).await.unwrap();
    assert_eq!(restored.updates.len(), 1);
    let SessionUpdate::Acp(notification) = &restored.updates[0] else { panic!("persisted ACP chunk"); };
    assert_aggregate(notification);
    assert!(restored.chat_history.is_empty());
}
