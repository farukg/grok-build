use super::*;
use crate::extensions::notification::{SessionNotification as XaiNotification, SessionUpdate};

fn delta(prompt: &str, arguments: &str) -> XaiNotification {
    XaiNotification {
        session_id: acp::SessionId::new("buffer-progress"),
        update: SessionUpdate::ToolCallDeltaChunk {
            tool_call_id: None, tool_index: 0, name: Some("edit_file".into()),
            arguments_delta: Some(arguments.into()),
        },
        meta: Some(serde_json::json!({ "promptId": prompt })),
    }
}

fn delta_in_stream(prompt: &str, request: &str, epoch: u64, arguments: &str) -> XaiNotification {
    XaiNotification {
        session_id: acp::SessionId::new("buffer-progress"),
        update: SessionUpdate::ToolCallDeltaChunk {
            tool_call_id: None, tool_index: 0, name: Some("edit_file".into()),
            arguments_delta: Some(arguments.into()),
        },
        meta: Some(serde_json::json!({ "promptId": prompt, "requestId": request, "streamEpoch": epoch })),
    }
}

#[test]
fn buffering_preserves_prompt_identity_and_never_concatenates_deltas_across_turns() {
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings {
        max_items: 100, max_bytes: 100_000, max_duration_ms: 10,
    }));
    assert!(buffer.consume_chunk(delta("first-prompt", "{\"path\":")).is_none());
    assert!(buffer.consume_chunk(delta("first-prompt", "\"first.rs\"}")).is_none());
    let (first, second) = buffer.consume_chunk(delta("next-prompt", "{\"path\":\"next.rs\"}")).unwrap();
    let SessionNotification::Xai(first) = first else { panic!("expected tool progress"); };
    let SessionNotification::Xai(second) = second.unwrap() else { panic!("expected next-turn tool progress"); };
    assert_eq!(first.meta.as_ref().unwrap().get("promptId").and_then(serde_json::Value::as_str), Some("first-prompt"));
    assert_eq!(second.meta.as_ref().unwrap().get("promptId").and_then(serde_json::Value::as_str), Some("next-prompt"));
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: first_arguments, .. } = &first.update else {
        panic!("expected tool arguments");
    };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: second_arguments, .. } = &second.update else {
        panic!("expected next-turn tool arguments");
    };
    assert_eq!(first_arguments.as_deref(), Some("{\"path\":\"first.rs\"}"));
    assert_eq!(second_arguments.as_deref(), Some("{\"path\":\"next.rs\"}"));
    assert!(buffer.flush().is_none());
}

#[test]
fn attempt_and_request_boundaries_split_same_index_id_less_deltas() {
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings {
        max_items: 100, max_bytes: 100_000, max_duration_ms: 10,
    }));
    // Attempt 1 streams partial arguments, then the scheduler discards it (Retrying bumps
    // the epoch) and attempt 2 restreams the same id-less tool index from scratch.
    assert!(buffer.consume_chunk(delta_in_stream("p", "req-1", 0, "{\"path\":\"at")).is_none());
    let (first, second) = buffer
        .consume_chunk(delta_in_stream("p", "req-1", 1, "{\"path\":\"attempt-2.rs\"}"))
        .unwrap();
    let SessionNotification::Xai(first) = first else { panic!("expected discarded-attempt progress"); };
    let SessionNotification::Xai(second) = second.unwrap() else { panic!("expected fresh-attempt progress"); };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: discarded, .. } = &first.update else {
        panic!("expected discarded-attempt arguments");
    };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: fresh, .. } = &second.update else {
        panic!("expected fresh-attempt arguments");
    };
    assert_eq!(discarded.as_deref(), Some("{\"path\":\"at"));
    assert_eq!(fresh.as_deref(), Some("{\"path\":\"attempt-2.rs\"}"));
    assert_eq!(first.meta.as_ref().unwrap().get("streamEpoch"), Some(&serde_json::json!(0)));
    assert_eq!(second.meta.as_ref().unwrap().get("streamEpoch"), Some(&serde_json::json!(1)));
    assert!(buffer.flush().is_none());

    // A parallel request (e.g. prefire) sharing the epoch never concatenates either.
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings {
        max_items: 100, max_bytes: 100_000, max_duration_ms: 10,
    }));
    assert!(buffer.consume_chunk(delta_in_stream("p", "main", 0, "{\"a\"")).is_none());
    let (main, prefire) = buffer
        .consume_chunk(delta_in_stream("p", "prefire", 0, "{\"b\"}"))
        .unwrap();
    let SessionNotification::Xai(main) = main else { panic!("expected main progress"); };
    let SessionNotification::Xai(prefire) = prefire.unwrap() else { panic!("expected prefire progress"); };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: main_args, .. } = &main.update else {
        panic!("expected main arguments");
    };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: prefire_args, .. } = &prefire.update else {
        panic!("expected prefire arguments");
    };
    assert_eq!(main_args.as_deref(), Some("{\"a\""));
    assert_eq!(prefire_args.as_deref(), Some("{\"b\"}"));
    assert!(buffer.flush().is_none());

    // Same provenance still merges: the boundary must not disable buffering.
    let mut buffer = ReplayBuffer::new(Some(BufferingSettings {
        max_items: 100, max_bytes: 100_000, max_duration_ms: 10,
    }));
    assert!(buffer.consume_chunk(delta_in_stream("p", "req-1", 1, "{\"path\":\"")).is_none());
    assert!(buffer.consume_chunk(delta_in_stream("p", "req-1", 1, "same.rs\"}")).is_none());
    let merged = buffer.flush().unwrap();
    let SessionNotification::Xai(merged) = merged else { panic!("expected merged progress"); };
    let SessionUpdate::ToolCallDeltaChunk { arguments_delta: merged_args, .. } = &merged.update else {
        panic!("expected merged arguments");
    };
    assert_eq!(merged_args.as_deref(), Some("{\"path\":\"same.rs\"}"));
}
