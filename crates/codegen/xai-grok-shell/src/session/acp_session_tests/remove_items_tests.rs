//! `x.ai/session/remove_items` handler (`acp_session_impl/remove_items.rs`): turn guard, per-ref outcome, persistence order.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::support::*;
use super::*;
use crate::session::{
    ContextItemRef, RemoveContextItemsOutcome, RemoveItemsRequest, SkipReason, SkippedItem,
};
use xai_grok_sampling_types::{ConversationItem, ToolCall};

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "read_file".into(),
        arguments: "{}".into(),
    }
}

fn user_at(text: &str, prompt_index: usize) -> ConversationItem {
    let mut item = ConversationItem::user(text);
    item.set_prompt_index(prompt_index);
    item
}

fn exchange(id: &str) -> ContextItemRef {
    ContextItemRef::ToolExchange {
        tool_call_id: id.into(),
    }
}

fn turn(prompt_index: usize) -> ContextItemRef {
    ContextItemRef::Turn { prompt_index }
}

fn request(items: Vec<ContextItemRef>) -> RemoveItemsRequest {
    RemoveItemsRequest { items }
}

fn seed() -> Vec<ConversationItem> {
    vec![
        ConversationItem::system("system"),
        user_at("first", 0),
        ConversationItem::assistant_tool_calls(vec![call("call_a")]),
        ConversationItem::tool_result("call_a", "a"),
        ConversationItem::assistant("done"),
        user_at("second", 1),
        ConversationItem::assistant_tool_calls(vec![call("call_b")]),
    ]
}

fn labels(conversation: &[ConversationItem]) -> Vec<String> {
    conversation
        .iter()
        .map(|item| match item {
            ConversationItem::System(_) => "system".to_string(),
            ConversationItem::User(_) => format!("user:{}", item.text_content()),
            ConversationItem::Assistant(assistant) => format!(
                "assistant:{}:{}",
                assistant.content,
                assistant
                    .tool_calls
                    .iter()
                    .map(|call| call.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            ConversationItem::ToolResult(result) => format!("result:{}", result.tool_call_id),
            ConversationItem::BackendToolCall(call) => format!("backend:{}", call.id()),
            ConversationItem::Reasoning(reasoning) => format!("reasoning:{}", reasoning.id),
        })
        .collect()
}

/// Persistence messages in arrival order; every flush barrier is answered with the next scripted result.
struct Persistence {
    seen: Rc<RefCell<Vec<&'static str>>>,
    script: Rc<RefCell<VecDeque<std::io::Result<()>>>>,
}

fn spawn_persistence(
    mut persistence_rx: tokio::sync::mpsc::UnboundedReceiver<PersistenceMsg>,
) -> Persistence {
    let persistence = Persistence {
        seen: Rc::default(),
        script: Rc::default(),
    };
    let log = Rc::clone(&persistence.seen);
    let script = Rc::clone(&persistence.script);
    tokio::task::spawn_local(async move {
        while let Some(message) = persistence_rx.recv().await {
            match message {
                PersistenceMsg::ReplaceChatHistory(_) => log.borrow_mut().push("replace"),
                PersistenceMsg::FlushAndAck { respond_to } => {
                    log.borrow_mut().push("flush");
                    let _ = respond_to.send(script.borrow_mut().pop_front().unwrap_or(Ok(())));
                }
                PersistenceMsg::Update(crate::session::storage::SessionUpdate::Xai(
                    notification,
                )) if matches!(
                    notification.update,
                    crate::extensions::notification::SessionUpdate::ContextItemsRemoved { .. }
                ) =>
                {
                    log.borrow_mut().push("marker");
                }
                _ => {}
            }
        }
    });
    persistence
}

/// An actor holding `conversation`, with the seeding traffic already drained from the persistence log.
async fn actor_with(
    conversation: Vec<ConversationItem>,
    flush_results: Vec<std::io::Result<()>>,
) -> (Arc<SessionActor>, Persistence) {
    let (gateway_tx, _gateway_rx) = tokio::sync::mpsc::unbounded_channel();
    let (persistence_tx, persistence_rx) = tokio::sync::mpsc::unbounded_channel();
    let persistence = spawn_persistence(persistence_rx);
    let (actor, _events) = create_test_actor_with_chat_persistence(
        0,
        256_000,
        85,
        gateway_tx,
        persistence_tx.clone(),
        Box::new(crate::session::chat_persistence::ChannelChatPersistence::new(persistence_tx)),
    )
    .await;
    let actor = Arc::new(actor);
    actor.chat_state_handle.replace_conversation(conversation);
    actor.chat_state_handle.get_conversation().await;
    let (barrier_tx, barrier_rx) = tokio::sync::oneshot::channel();
    let sent = actor
        .notifications
        .persistence_tx
        .send(PersistenceMsg::FlushAndAck {
            respond_to: barrier_tx,
        });
    assert!(sent.is_ok(), "persistence log is open");
    assert!(barrier_rx.await.is_ok(), "seeding traffic drained");
    persistence.seen.borrow_mut().clear();
    persistence.script.borrow_mut().extend(flush_results);
    (actor, persistence)
}

#[tokio::test(flavor = "current_thread")]
async fn removal_during_an_active_turn_is_refused_and_changes_nothing() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence) = actor_with(seed(), vec![]).await;
            actor.session_turn_active.store(true, Ordering::SeqCst);

            let outcome = actor
                .handle_remove_context_items(request(vec![turn(0)]))
                .await;

            assert_eq!(outcome, RemoveContextItemsOutcome::TurnRunning);
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(
                labels(&conversation),
                [
                    "system",
                    "user:first",
                    "assistant::call_a",
                    "result:call_a",
                    "assistant:done:",
                    "user:second",
                    "assistant::call_b",
                ]
            );
            assert!(
                persistence.seen.borrow().is_empty(),
                "nothing may be persisted: {:?}",
                persistence.seen.borrow()
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn idle_removal_reports_exactly_what_left_and_persists_rewrite_before_marker() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence) = actor_with(seed(), vec![]).await;

            let outcome = actor
                .handle_remove_context_items(request(vec![turn(0)]))
                .await;

            assert_eq!(
                outcome,
                RemoveContextItemsOutcome::Removed {
                    removed: vec![turn(0)],
                    skipped: vec![]
                }
            );
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(
                labels(&conversation),
                ["system", "user:second", "assistant::call_b"]
            );
            assert_eq!(
                *persistence.seen.borrow(),
                ["replace", "flush", "marker", "flush"]
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn mixed_request_removes_the_answered_exchange_and_reports_the_unanswered_one() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence) = actor_with(seed(), vec![]).await;

            let outcome = actor
                .handle_remove_context_items(request(vec![
                    exchange("call_a"),
                    exchange("call_b"),
                    turn(9),
                ]))
                .await;

            assert_eq!(
                outcome,
                RemoveContextItemsOutcome::Removed {
                    removed: vec![exchange("call_a")],
                    skipped: vec![
                        SkippedItem {
                            item: exchange("call_b"),
                            reason: SkipReason::CallWithoutResult
                        },
                        SkippedItem {
                            item: turn(9),
                            reason: SkipReason::NotInContext
                        },
                    ],
                }
            );
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(
                labels(&conversation),
                [
                    "system",
                    "user:first",
                    "assistant:done:",
                    "user:second",
                    "assistant::call_b"
                ]
            );
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn only_unanswered_call_is_refused_without_changes() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence) = actor_with(seed(), vec![]).await;

            let outcome = actor
                .handle_remove_context_items(request(vec![exchange("call_b")]))
                .await;

            assert_eq!(outcome, RemoveContextItemsOutcome::WouldOrphan);
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(conversation.len(), 7);
            assert!(persistence.seen.borrow().is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn turn_below_the_compaction_floor_rejects_the_whole_request() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, persistence) = actor_with(seed(), vec![]).await;
            actor.chat_state_handle.record_compaction_at(1);

            let outcome = actor
                .handle_remove_context_items(request(vec![turn(0), exchange("call_a")]))
                .await;

            assert_eq!(outcome, RemoveContextItemsOutcome::BeforeCompaction);
            assert_eq!(actor.chat_state_handle.get_conversation().await.len(), 7);
            assert!(persistence.seen.borrow().is_empty());
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn failed_rewrite_flush_is_persist_failed_and_writes_no_marker() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let failure = std::io::Error::new(std::io::ErrorKind::StorageFull, "disk full");
            let (actor, persistence) = actor_with(seed(), vec![Err(failure)]).await;

            let outcome = actor
                .handle_remove_context_items(request(vec![turn(0)]))
                .await;

            assert_eq!(
                outcome,
                RemoveContextItemsOutcome::PersistFailed {
                    removed: vec![turn(0)]
                }
            );
            assert_eq!(*persistence.seen.borrow(), ["replace", "flush"]);
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_turn_ref_repeated_in_one_request_is_removed_once() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (actor, _persistence) = actor_with(seed(), vec![]).await;

            let outcome = actor
                .handle_remove_context_items(request(vec![turn(0), turn(0)]))
                .await;

            assert_eq!(
                outcome,
                RemoveContextItemsOutcome::Removed {
                    removed: vec![turn(0)],
                    skipped: vec![]
                }
            );
            let conversation = actor.chat_state_handle.get_conversation().await;
            assert_eq!(
                labels(&conversation),
                ["system", "user:second", "assistant::call_b"]
            );
        })
        .await;
}
