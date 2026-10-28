use std::borrow::Cow;

use super::{decode_event, decode_event_entry};
use crate::types::{Event, ToolCompletedSource, ToolOutcome, TurnOutcomeLabel};

#[test]
fn tool_completion_accepts_writer_omissions() {
    let event = decode_event(
        r#"{"type":"tool_completed","tool_name":"bash","duration_ms":10,"outcome":"success"}"#,
    )
    .unwrap();
    let Event::ToolCompleted {
        tool_call_id,
        source,
        rewriting_hook,
        ..
    } = event
    else {
        panic!("expected tool completion");
    };
    assert!(tool_call_id.is_empty());
    assert_eq!(source, ToolCompletedSource::Shell);
    assert_eq!(rewriting_hook, None);
}

#[test]
fn entry_accepts_legacy_tool_completion_omissions() {
    let entry = decode_event_entry(
        r#"{"ts":"2026-10-28T00:00:00Z","type":"tool_completed","tool_name":"bash","duration_ms":10,"outcome":"success"}"#,
    )
    .unwrap();
    let Event::ToolCompleted {
        tool_call_id,
        source,
        rewriting_hook,
        ..
    } = entry.event
    else {
        panic!("expected tool completion");
    };
    assert!(tool_call_id.is_empty());
    assert_eq!(source, ToolCompletedSource::Shell);
    assert_eq!(rewriting_hook, None);
}

#[test]
fn tool_completion_preserves_explicit_workspace_fields() {
    let event = decode_event(
        r#"{"type":"tool_completed","tool_name":"edit","duration_ms":42,"outcome":"hook_denied","tool_call_id":"call-7","source":"workspace","rewriting_hook":"rewrite"}"#,
    )
    .unwrap();
    let Event::ToolCompleted {
        tool_call_id,
        source,
        outcome,
        rewriting_hook,
        ..
    } = event
    else {
        panic!("expected tool completion");
    };
    assert_eq!(tool_call_id, "call-7");
    assert_eq!(source, ToolCompletedSource::Workspace);
    assert_eq!(outcome, ToolOutcome::HookDenied);
    assert_eq!(rewriting_hook.as_deref(), Some("rewrite"));
}

#[test]
fn mcp_initialization_accepts_omitted_empty_failures() {
    let event = decode_event(
        r#"{"type":"mcp_init_completed","total_servers":1,"succeeded":1,"failed":0,"auth_required":0,"total_tools":3,"duration_ms":12,"is_reinit":false}"#,
    )
    .unwrap();
    let Event::McpInitCompleted { failed_servers, .. } = event else {
        panic!("expected MCP initialization");
    };
    assert!(failed_servers.is_empty());
}

#[test]
fn legacy_turns_accept_omitted_optional_metadata() {
    let event = decode_event(
        r#"{"type":"turn_started","session_id":"session","turn_number":1,"model_id":"model","yolo_mode":false,"conversation_message_count":0,"session_relationship":"primary","schema_version":"1.0"}"#,
    )
    .unwrap();
    assert!(matches!(
        event,
        Event::TurnStarted {
            redirect_kind: None,
            ..
        }
    ));
    let event = decode_event(r#"{"type":"turn_ended","outcome":"interrupted"}"#).unwrap();
    assert!(matches!(
        event,
        Event::TurnEnded {
            outcome: TurnOutcomeLabel::Interrupted,
            cancellation_category: None,
            cancellation_context: None,
        }
    ));
}

#[test]
fn old_event_variants_remain_decodable() {
    for wire in [
        r#"{"type":"first_token"}"#,
        r#"{"type":"loop_started","loop_index":0}"#,
        r#"{"type":"phase_changed","phase":"streaming_reasoning"}"#,
        r#"{"type":"permission_resolved","tool_name":"bash","decision":"followup","wait_ms":3}"#,
        r#"{"type":"goal_auto_paused","reason":"back_off"}"#,
        r#"{"type":"todo_gate_fired","fires":1,"pending":2,"in_progress":0,"reason":"in_flight"}"#,
        r#"{"type":"laziness_classifier_fired","model_id":"model","category":"stalled_narration","confidence":0.9}"#,
        r#"{"type":"laziness_nudge_fired","model_id":"model","category":"stalled_narration","nudges_remaining":2}"#,
        r#"{"type":"laziness_classifier_aborted","reason":"model_switch"}"#,
        r#"{"type":"goal_classifier_fail_open","reason":"timeout","attempt":1,"latency_ms":2}"#,
        r#"{"type":"goal_classifier_fail_closed","reason":"concurrent_in_flight","attempt":1}"#,
        r#"{"type":"goal_planner_fail_closed","reason":"missing_plan_file","attempt":1,"latency_ms":2}"#,
        r#"{"type":"goal_strategist_failed","reason":"missing_strategy_file","attempt":1,"consecutive_failures":2,"latency_ms":3}"#,
        r#"{"type":"goal_strategist_contract_restore_failed","reason":"symlink_tamper","attempt":1}"#,
        r#"{"type":"goal_summarizer_fail_open","reason":"empty_summary","attempt":1,"latency_ms":2}"#,
        r#"{"type":"goal_role_model_resolved","role":"skeptic","model_id":"model","agent_type":"general-purpose","source":"remote"}"#,
        r#"{"type":"goal_role_model_fail_open","role":"planner","reason":"harness_flavor_unsupported"}"#,
        r#"{"type":"goal_verifier_skeptic_verdict","attempt":1,"skeptic_idx":0,"refuted":true,"confidence":"high","latency_ms":2}"#,
        r#"{"type":"goal_premature_stop_detected","pattern":"giving_up"}"#,
        r#"{"type":"mcp_config_resolved","servers":[{"name":"server","transport":"stdio","source":"project"}],"disabled":[]}"#,
        r#"{"type":"mcp_oauth_discovery_timeout","server_name":"server","url":"https://example.invalid"}"#,
        r#"{"type":"mcp_oauth_probe_resolved","server_name":"server","verdict":"ready"}"#,
    ] {
        assert!(decode_event(wire).is_ok(), "valid legacy event was refused");
    }
}

#[test]
fn decoded_strings_outlive_input_including_escaped_labels() {
    let entry = {
        let input = String::from(
            r#"{"ts":"2026-10-28T00:00:00Z","type":"goal_classifier_fail_open","reason":"legacy\nlabel","attempt":1,"latency_ms":2}"#,
        );
        decode_event_entry(&input).unwrap()
    };
    assert_eq!(entry.ts, "2026-10-28T00:00:00Z");
    let Event::GoalClassifierFailOpen { reason, .. } = entry.event else {
        panic!("expected classifier failure");
    };
    assert_eq!(reason, "legacy\nlabel");
    assert!(matches!(reason, Cow::Owned(_)));
}

#[test]
fn unknown_variants_and_malformed_known_payloads_are_refused() {
    for wire in [
        r#"{"type":"future_event"}"#,
        r#"{"type":"tool_started"}"#,
        r#"{"type":"tool_started","tool_name":7}"#,
        r#"{"type":"loop_started","loop_index":-1}"#,
        r#"{"type":"loop_started","loop_index":4294967296}"#,
        r#"{"type":"tool_completed","tool_name":"bash","duration_ms":1,"outcome":"unknown"}"#,
        r#"{"type":"tool_completed","tool_name":"bash","duration_ms":1,"outcome":"success","tool_call_id":null}"#,
        r#"{"type":"tool_completed","tool_name":"bash","duration_ms":1,"outcome":"success","source":null}"#,
        r#"{"type":"tool_completed","tool_name":"bash","duration_ms":1,"outcome":"success","source":"unknown"}"#,
        r#"{"type":"goal_classifier_fail_open","reason":null,"attempt":1,"latency_ms":2}"#,
        r#"{"type":"goal_classifier_fail_open","reason":[],"attempt":1,"latency_ms":2}"#,
        r#"{"type":"phase_changed","phase":"unknown"}"#,
        r#"{"type":"permission_resolved","tool_name":"bash","decision":"unknown","wait_ms":0}"#,
        r#"{"type":"goal_auto_paused","reason":"unknown"}"#,
        r#"{"type":"mcp_init_completed","total_servers":1,"succeeded":1,"failed":0,"auth_required":0,"total_tools":3,"duration_ms":12,"is_reinit":false,"failed_servers":null}"#,
        r#"{"type":"tool_started","tool_name":"bash","tool_name":"edit"}"#,
        r#"{"type":"turn_ended","outcome":"unknown"}"#,
        r#"{"type":"turn_ended","outcome":"cancelled","cancellation_category":"unknown"}"#,
        r#"{"type":"mcp_config_resolved","servers":[{"name":"server","transport":"stdio"}],"disabled":[]}"#,
        r#"{"type":"first_token"} {"type":"first_token"}"#,
        r#"{"type":"tool_started","tool_name":"#,
        "null",
        "[]",
        "",
    ] {
        assert!(decode_event(wire).is_err(), "malformed event was accepted");
    }
}

#[test]
fn entry_requires_string_timestamp_and_flattened_native_event() {
    let entry = decode_event_entry(
        r#"{"ts":"2026-10-28T00:00:00+00:00","type":"first_token","additive_metadata":3}"#,
    )
    .unwrap();
    assert!(matches!(entry.event, Event::FirstToken));
    for wire in [
        r#"{"type":"first_token"}"#,
        r#"{"ts":1,"type":"first_token"}"#,
        r#"{"ts":null,"type":"first_token"}"#,
        r#"{"ts":"2026-10-28T00:00:00Z","event":{"type":"first_token"}}"#,
        r#"{"ts":"2026-10-28T00:00:00Z","type":"future_event"}"#,
        r#"{"ts":"2026-10-28T00:00:00Z","type":"tool_started","tool_name":false}"#,
    ] {
        assert!(
            decode_event_entry(wire).is_err(),
            "malformed entry was accepted"
        );
    }
}

#[test]
fn static_producer_labels_stay_borrowed_with_unchanged_wire() {
    let event = Event::GoalClassifierFailOpen {
        reason: "timeout".into(),
        attempt: 1,
        latency_ms: 2,
    };
    assert_eq!(
        serde_json::to_string(&event).unwrap(),
        r#"{"type":"goal_classifier_fail_open","reason":"timeout","attempt":1,"latency_ms":2}"#,
    );
    let Event::GoalClassifierFailOpen { reason, .. } = event else {
        panic!("expected classifier failure");
    };
    assert!(matches!(reason, Cow::Borrowed("timeout")));
}
