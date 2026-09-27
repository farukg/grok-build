use super::*;

fn parse(json: serde_json::Value) -> SubagentContext {
    serde_json::from_value(json).expect("wire context parses")
}

#[test]
fn replayed_flat_fields_map_to_the_typed_context() {
    let cases = [
        (serde_json::json!({}), SubagentContext::Unreported),
        (
            serde_json::json!({"effective_context_source": "new"}),
            SubagentContext::Fresh,
        ),
        (
            serde_json::json!({
                "effective_context_source": "new",
                "fork_copy_error": "no inheritable parent content after filtering"
            }),
            SubagentContext::ForkFailed(ForkFailure::NoInheritableContent),
        ),
        (
            serde_json::json!({
                "effective_context_source": "new",
                "fork_copy_error": "disk full"
            }),
            SubagentContext::ForkFailed(ForkFailure::CopyFailed("disk full".into())),
        ),
        (
            serde_json::json!({"effective_context_source": "forked"}),
            SubagentContext::Forked(ForkMode::Verbatim),
        ),
        (
            serde_json::json!({"effective_context_source": "forked", "context_normalized": true}),
            SubagentContext::Forked(ForkMode::Summarized),
        ),
        (
            serde_json::json!({"effective_context_source": "resumed"}),
            SubagentContext::Resumed,
        ),
    ];
    for (json, expected) in cases {
        assert_eq!(parse(json.clone()), expected, "{json}");
    }
}

#[test]
fn fork_failure_survives_a_persist_and_replay_round_trip() {
    let failed = SubagentContext::ForkFailed(ForkFailure::NoInheritableContent);
    let json = serde_json::to_value(&failed).expect("serializes");
    assert_eq!(
        json,
        serde_json::json!({
            "effective_context_source": "new",
            "fork_copy_error": "no inheritable parent content after filtering"
        })
    );
    assert_eq!(parse(json), failed);
}
