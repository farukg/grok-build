use super::*;
use crate::acp::tool_meta::{ToolIdentity, ToolMetadata, ToolSource};
use uuid::Uuid;
use xai_grok_shell::extensions::tool_timing::ToolExecutionTiming;

fn owned_meta(is_replay: bool) -> NotificationMeta {
    NotificationMeta {
        is_replay,
        tool: ToolMetadata::Owned {
            identity: ToolIdentity {
                invocation: Uuid::from_u128(10987),
                provider: "historical-tool".into(),
                source: ToolSource::Unknown,
            },
            timing: ToolExecutionTiming::Unknown,
        },
        ..Default::default()
    }
}

#[test]
fn registered_replay_tool_closes_the_historical_thought_without_live_authority() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    let mut tracker = AcpUpdateTracker::new();
    let mut sb = ScrollbackState::new();
    let replay = NotificationMeta {
        is_replay: true,
        ..meta_stream(1000)
    };
    tracker.handle_update(thought_chunk("before"), &replay, &mut sb);
    let epoch = tracker.agent_output_epoch;
    tracker.handle_update(
        tool_call("historical-tool", acp::ToolKind::Read, "source.rs"),
        &owned_meta(true),
        &mut sb,
    );
    assert!(!sb.get(0).unwrap().is_running);
    assert!(tracker.current_thinking.is_none());
    assert_eq!(tracker.agent_output_epoch, epoch);
    tracker.handle_update(thought_chunk("after"), &replay, &mut sb);
    assert_eq!(sb.len(), 3);
    assert!(matches!(&sb.get(0).unwrap().block, RenderBlock::Thinking(t) if t.text() == "before"));
    assert!(matches!(&sb.get(1).unwrap().block, RenderBlock::ToolCall(_)));
    assert!(matches!(&sb.get(2).unwrap().block, RenderBlock::Thinking(t) if t.text() == "after"));
}

#[test]
fn detached_live_tool_does_not_close_the_current_thought() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    let mut tracker = AcpUpdateTracker::new();
    let mut sb = ScrollbackState::new();
    tracker.handle_update(thought_chunk("current"), &meta(), &mut sb);
    let thought = tracker.current_thinking;
    let activity = tracker.activity();
    tracker.handle_update(
        tool_call("historical-tool", acp::ToolKind::Read, "source.rs"),
        &owned_meta(false),
        &mut sb,
    );
    assert_eq!(tracker.current_thinking, thought);
    assert!(sb.get_by_id(thought.unwrap()).unwrap().is_running);
    assert_eq!(tracker.activity(), activity);
}

#[test]
fn drained_legacy_terminal_settles_only_the_named_entry_without_changing_activity() {
    crate::appearance::cache::set_show_thinking_blocks(true);
    for is_replay in [false, true] {
        for (status, terminal_meta, completion) in [
            (acp::ToolCallStatus::Completed, None, ToolCompletion::Completed),
            (acp::ToolCallStatus::Failed, None, ToolCompletion::Failed),
            (
                acp::ToolCallStatus::Failed,
                Some(serde_json::json!({"toolTerminalStatus": "cancelled"}).as_object().unwrap().clone()),
                ToolCompletion::Cancelled,
            ),
        ] {
            let mut tracker = AcpUpdateTracker::new();
            let mut sb = ScrollbackState::new();
            tracker.handle_update(tool_call("old", acp::ToolKind::Read, "old.rs"), &meta(), &mut sb);
            let old = sb.get(0).unwrap().id;
            tracker.handle_update(tool_call("other", acp::ToolKind::Read, "other.rs"), &meta(), &mut sb);
            let other = sb.get(1).unwrap().id;
            tracker.finish_turn(&mut sb);
            tracker.handle_update(thought_chunk("next turn"), &meta(), &mut sb);
            let thought = tracker.current_thinking;
            let activity = tracker.activity();
            let epoch = tracker.agent_output_epoch;
            let terminal = acp::SessionUpdate::ToolCallUpdate(
                acp::ToolCallUpdate::new("old", acp::ToolCallUpdateFields::new().status(Some(status)))
                    .meta(terminal_meta),
            );
            let metadata = NotificationMeta { is_replay, ..Default::default() };
            assert!(tracker.handle_update(terminal.clone(), &metadata, &mut sb));
            assert!(!tracker.handle_update(terminal, &metadata, &mut sb));
            assert_eq!(sb.get_by_id(old).unwrap().tool_completion(), completion);
            assert!(!sb.get_by_id(old).unwrap().is_running);
            assert_eq!(sb.get_by_id(other).unwrap().tool_completion(), ToolCompletion::Unconfirmed);
            assert!(!sb.get_by_id(other).unwrap().is_running);
            assert_eq!(tracker.current_thinking, thought);
            assert!(sb.get_by_id(thought.unwrap()).unwrap().is_running);
            assert_eq!(tracker.activity(), activity);
            assert_eq!(tracker.agent_output_epoch, epoch);
        }
    }
}
