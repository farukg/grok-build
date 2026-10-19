//! Hidden shell turns recorded from user chunks.
use super::*;
use crate::scrollback::block::RenderBlock;

fn user_chunk(meta: Option<serde_json::Value>) -> acp::ContentChunk {
    acp::ContentChunk::new(acp::ContentBlock::Text(acp::TextContent::new(
        "wake".to_owned(),
    )))
    .meta(meta.and_then(|value| value.as_object().cloned()))
}

#[test]
fn numbered_hidden_chunk_ends_the_visible_turn_at_the_newest_entry() {
    let mut scrollback = ScrollbackState::new();
    scrollback.push_block(RenderBlock::user_prompt("Q"));
    let reply = scrollback.push_block(RenderBlock::agent_message("A"));
    let mut ledger = TurnLedger::default();
    ledger.note_hidden_user_chunk(
        &user_chunk(Some(serde_json::json!({ "promptIndex": 4 }))),
        &scrollback,
    );
    assert!(ledger.turn_ends_after(reply));
}

#[test]
fn unnumbered_chunk_and_empty_scrollback_record_nothing() {
    let mut scrollback = ScrollbackState::new();
    let mut ledger = TurnLedger::default();
    ledger.note_hidden_user_chunk(
        &user_chunk(Some(serde_json::json!({ "promptIndex": 0 }))),
        &scrollback,
    );
    let first = scrollback.push_block(RenderBlock::user_prompt("Q"));
    ledger.note_hidden_user_chunk(&user_chunk(None), &scrollback);
    assert!(!ledger.turn_ends_after(first));
}

#[test]
fn boundary_of_a_removed_entry_moves_to_the_nearest_survivor() {
    let mut scrollback = ScrollbackState::new();
    let prompt = scrollback.push_block(RenderBlock::user_prompt("Q"));
    let reply = scrollback.push_block(RenderBlock::agent_message("A"));
    let tail = scrollback.push_block(RenderBlock::agent_message("B"));
    let mut ledger = TurnLedger::default();
    ledger.note_hidden_user_chunk(
        &user_chunk(Some(serde_json::json!({ "promptIndex": 1 }))),
        &scrollback,
    );
    assert!(ledger.turn_ends_after(tail));
    ledger.carry_to_survivors(&scrollback, &HashSet::from([tail]));
    assert!(!ledger.turn_ends_after(tail));
    assert!(ledger.turn_ends_after(reply));
    assert!(!ledger.turn_ends_after(prompt));
}
